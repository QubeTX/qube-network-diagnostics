//! Hidden Windows-installer bookkeeping.
//!
//! MSI marker writes are commit custom actions so an older product cannot
//! remove the newly selected channel's marker during `RemoveExistingProducts`.
//! The Global MSI also repairs the pre-v3.1 PATH move after commit because the
//! historical package reused one component GUID at two different directories.

use crate::cli::InstallMaintenanceArgs;
#[cfg(windows)]
use crate::cli::{InstallMaintenanceAction, MigrateInstallOrigin};

pub fn run(args: InstallMaintenanceArgs) -> i32 {
    #[cfg(windows)]
    {
        let result = match args.action {
            InstallMaintenanceAction::SetMarker => set_marker(args.origin),
            InstallMaintenanceAction::ClearMarker => clear_marker_if_current(args.origin),
            InstallMaintenanceAction::RepairGlobalPath => repair_global_path(&args),
        };
        match result {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("ND-300 installer maintenance failed: {error}");
                2
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = args;
        eprintln!("install-maintenance is a Windows-installer-only command");
        2
    }
}

#[cfg(windows)]
fn marker_value(origin: MigrateInstallOrigin) -> &'static str {
    match origin {
        MigrateInstallOrigin::MsiGlobal => "msi-global",
        MigrateInstallOrigin::MsiCorporate => "msi-corporate",
        MigrateInstallOrigin::ExeGlobal => "exe-global",
        MigrateInstallOrigin::ExeCorporate => "exe-corporate",
        MigrateInstallOrigin::MacosPkg => "macos-pkg",
    }
}

#[cfg(windows)]
fn set_marker(origin: MigrateInstallOrigin) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey("Software\\ND300")
        .map_err(|error| format!("open HKCU\\Software\\ND300: {error}"))?;
    key.set_value("InstallSource", &marker_value(origin))
        .map_err(|error| format!("write InstallSource marker: {error}"))
}

#[cfg(windows)]
fn clear_marker_if_current(origin: MigrateInstallOrigin) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey_with_flags(
        "Software\\ND300",
        winreg::enums::KEY_READ | winreg::enums::KEY_WRITE,
    ) else {
        return Ok(());
    };
    let current = key.get_value::<String, _>("InstallSource").ok();
    if current.as_deref() == Some(marker_value(origin)) {
        key.delete_value("InstallSource")
            .map_err(|error| format!("clear matching InstallSource marker: {error}"))?;
    }
    Ok(())
}

#[cfg(windows)]
fn repair_global_path(args: &InstallMaintenanceArgs) -> Result<(), String> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};
    use winreg::RegKey;

    if args.origin != MigrateInstallOrigin::MsiGlobal {
        return Err("legacy machine PATH repair is valid only for msi-global".to_string());
    }
    let current = validated_path_arg(args.current_path.as_deref(), "current-path")?;
    let legacy = validated_path_arg(args.legacy_path.as_deref(), "legacy-path")?;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey_with_flags(
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
            KEY_READ | KEY_WRITE,
        )
        .map_err(|error| format!("open machine Environment key: {error}"))?;
    let mut raw = key
        .get_raw_value("Path")
        .map_err(|error| format!("read machine PATH: {error}"))?;
    if raw.vtype != REG_SZ && raw.vtype != REG_EXPAND_SZ {
        return Err(format!(
            "machine PATH has unsupported registry type {:?}",
            raw.vtype
        ));
    }
    let existing = decode_registry_string(&raw.bytes)?;
    let (updated, changed) = migrate_path_value(&existing, &legacy, &current);
    if changed {
        raw.bytes = encode_registry_string(&updated);
        key.set_raw_value("Path", &raw)
            .map_err(|error| format!("write repaired machine PATH: {error}"))?;
    }
    Ok(())
}

#[cfg(windows)]
fn validated_path_arg(value: Option<&str>, label: &str) -> Result<std::path::PathBuf, String> {
    let value = value.ok_or_else(|| format!("missing --{label}"))?;
    if value.contains(';') {
        return Err(format!("--{label} contains a PATH separator"));
    }
    let path = std::path::PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("--{label} must be absolute"));
    }
    Ok(path)
}

#[cfg(any(windows, test))]
fn migrate_path_value(
    existing: &str,
    legacy: &std::path::Path,
    current: &std::path::Path,
) -> (String, bool) {
    let legacy = normalized_path(legacy.to_string_lossy().as_ref());
    let current_normalized = normalized_path(current.to_string_lossy().as_ref());
    let mut entries = Vec::new();
    let mut current_present = false;
    for entry in existing.split(';').filter(|entry| !entry.trim().is_empty()) {
        let normalized = normalized_path(entry);
        if normalized == legacy {
            continue;
        }
        if normalized == current_normalized {
            if current_present {
                continue;
            }
            current_present = true;
        }
        entries.push(entry.to_string());
    }
    if !current_present {
        entries.push(current.display().to_string());
    }
    let updated = entries.join(";");
    (updated.clone(), updated != existing)
}

#[cfg(any(windows, test))]
fn normalized_path(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn decode_registry_string(bytes: &[u8]) -> Result<String, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err("machine PATH registry bytes have odd length".to_string());
    }
    let mut wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    while wide.last() == Some(&0) {
        wide.pop();
    }
    String::from_utf16(&wide).map_err(|error| format!("decode machine PATH: {error}"))
}

#[cfg(windows)]
fn encode_registry_string(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn legacy_machine_path_is_replaced_once_without_rewriting_other_entries() {
        let existing = r"C:\Tools;C:\Program Files\nd-300\bin;C:\Windows\System32";
        let (updated, changed) = migrate_path_value(
            existing,
            Path::new(r"C:\Program Files\nd-300\bin"),
            Path::new(r"C:\Program Files\nd300\bin"),
        );
        assert!(changed);
        assert_eq!(
            updated,
            r"C:\Tools;C:\Windows\System32;C:\Program Files\nd300\bin"
        );
    }

    #[test]
    fn current_machine_path_is_idempotent_and_deduplicated() {
        let current = Path::new(r"C:\Program Files\nd300\bin");
        let (updated, changed) = migrate_path_value(
            r"C:\Program Files\nd300\bin;C:\Tools;c:\program files\ND300\bin\",
            Path::new(r"C:\Program Files\nd-300\bin"),
            current,
        );
        assert!(changed);
        assert_eq!(updated, r"C:\Program Files\nd300\bin;C:\Tools");

        let (second, second_changed) =
            migrate_path_value(&updated, Path::new(r"C:\Program Files\nd-300\bin"), current);
        assert!(!second_changed);
        assert_eq!(second, updated);
    }
}
