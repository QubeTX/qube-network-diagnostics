//! Hidden Windows fresh-install channel takeover.
//!
//! `nd300 update` preserves its proven installation channel. A user who
//! deliberately launches a different current installer has expressed a newer
//! intent: that installer becomes authoritative. MSI packages call this helper
//! before `InstallInitialize` to remove conflicting Inno products; Inno calls it
//! before copying files to remove conflicting MSI products and the other Inno
//! edition. No registry command is shell-expanded.

use crate::cli::InstallTakeoverArgs;

#[cfg(windows)]
const MANUAL_INSTALL_URL: &str = "https://reports.qubetx.com/nd300#install";

pub fn run(args: InstallTakeoverArgs) -> i32 {
    #[cfg(windows)]
    {
        match run_windows(args) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("ND-300 could not complete the fresh-install channel takeover: {error}");
                eprintln!("The new installer has not claimed ownership.");
                eprintln!(
                    "Any prior uninstaller that already completed remains completed; review the install help before retrying."
                );
                eprintln!("Install help: {MANUAL_INSTALL_URL}");
                2
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = args;
        eprintln!("install-takeover is a Windows-installer-only command");
        2
    }
}

#[cfg(windows)]
fn run_windows(args: InstallTakeoverArgs) -> Result<(), String> {
    use crate::cli::InstallTakeoverScope;

    let target_machine_scope = target_uses_machine_scope(args.target);
    let owners: Vec<_> = super::update::registered_install_owners_for_takeover()?
        .into_iter()
        .filter(|owner| match args.scope {
            InstallTakeoverScope::All => true,
            InstallTakeoverScope::User => !owner.machine_hive,
            InstallTakeoverScope::Machine => owner.machine_hive,
        })
        .collect();

    // Never execute a per-user uninstaller from an elevated machine installer,
    // or a machine uninstaller from a per-user/standalone installer. Apart from
    // Windows Installer's cross-context upgrade limitation, an HKCU uninstaller
    // path is user-writable and must never cross an elevation boundary.
    if let Some(owner) = owners
        .iter()
        .find(|owner| owner.machine_hive != target_machine_scope)
    {
        return Err(format!(
            "a registered {} installation uses the opposite Windows install scope; uninstall it first or choose a current installer in the same scope",
            owner.origin.json_id()
        ));
    }

    let conflicts: Vec<_> = owners
        .into_iter()
        .filter(|owner| owner_conflicts_with_target(owner, args.target))
        .collect();

    for owner in conflicts {
        uninstall_registered_owner_and_wait(&owner)?;
    }
    Ok(())
}

#[cfg(windows)]
fn target_uses_machine_scope(target: crate::cli::InstallTakeoverTarget) -> bool {
    use crate::cli::InstallTakeoverTarget;
    matches!(
        target,
        InstallTakeoverTarget::MsiGlobal | InstallTakeoverTarget::ExeGlobal
    )
}

#[cfg(windows)]
fn owner_conflicts_with_target(
    owner: &super::update::RegisteredInstallOwner,
    target: crate::cli::InstallTakeoverTarget,
) -> bool {
    use super::update::{InstallOrigin, RegisteredUninstall};
    use crate::cli::InstallTakeoverTarget;

    match target {
        // Windows Installer's Upgrade table removes an older product with the
        // target package's own UpgradeCode and context transactionally. The
        // opposite context was refused above. Calling msiexec from an MSI custom
        // action would be a forbidden nested installation, so this helper handles
        // only the same-scope conflicting Inno product for MSI targets.
        InstallTakeoverTarget::MsiGlobal | InstallTakeoverTarget::MsiCorporate => {
            matches!(owner.uninstall, RegisteredUninstall::Inno { .. })
        }
        // Inno is not an MSI transaction, so it can synchronously remove every
        // conflicting MSI and the other Inno edition before file replacement.
        InstallTakeoverTarget::ExeGlobal => match owner.uninstall {
            RegisteredUninstall::Msi { .. } => true,
            RegisteredUninstall::Inno { .. } => owner.origin != InstallOrigin::ExeGlobal,
        },
        InstallTakeoverTarget::ExeCorporate => match owner.uninstall {
            RegisteredUninstall::Msi { .. } => true,
            RegisteredUninstall::Inno { .. } => owner.origin != InstallOrigin::ExeCorporate,
        },
        // The versionless PowerShell installer is a deliberate standalone fresh
        // install unless its parent is an old ND-300 updater. In fresh-install
        // mode it removes every registered Windows channel after staging the new
        // standalone pair.
        InstallTakeoverTarget::Standalone => true,
    }
}

#[cfg(windows)]
fn uninstall_registered_owner_and_wait(
    owner: &super::update::RegisteredInstallOwner,
) -> Result<(), String> {
    use super::update::RegisteredUninstall;
    use std::process::Command;
    use std::time::{Duration, Instant};

    let status = match &owner.uninstall {
        RegisteredUninstall::Msi { product_code } => {
            Command::new(super::uninstall::system_msiexec_path()?)
                .args(["/x", product_code, "/qn", "/norestart"])
                .status()
                .map_err(|error| format!("could not start MSI uninstaller: {error}"))?
        }
        RegisteredUninstall::Inno { executable } => Command::new(executable)
            .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"])
            .status()
            .map_err(|error| {
                format!(
                    "could not start validated Inno uninstaller {}: {error}",
                    executable.display()
                )
            })?,
    };

    let code = status.code().unwrap_or(-1);
    let accepted = status.success()
        || matches!(
            owner.uninstall,
            RegisteredUninstall::Msi { .. }
                if matches!(code, 1605 | 3010)
        );
    if !accepted {
        return Err(format!(
            "registered {} uninstaller exited {code}",
            owner.origin.json_id()
        ));
    }

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if !super::update::registered_install_owners_for_takeover()?.contains(owner) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!(
        "registered {} owner remained after its uninstaller exited",
        owner.origin.json_id()
    ))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::actions::update::{InstallOrigin, RegisteredInstallOwner, RegisteredUninstall};
    use crate::cli::InstallTakeoverTarget;
    use std::path::PathBuf;

    fn owner(origin: InstallOrigin, uninstall: RegisteredUninstall) -> RegisteredInstallOwner {
        RegisteredInstallOwner {
            origin,
            install_location: PathBuf::from(r"C:\fixture"),
            uninstall,
            machine_hive: matches!(origin, InstallOrigin::MsiGlobal | InstallOrigin::ExeGlobal),
        }
    }

    #[test]
    fn update_target_keeps_same_inno_channel_but_same_scope_msi_takes_over() {
        let global_exe = owner(
            InstallOrigin::ExeGlobal,
            RegisteredUninstall::Inno {
                executable: PathBuf::from(r"C:\fixture\unins000.exe"),
            },
        );
        assert!(!owner_conflicts_with_target(
            &global_exe,
            InstallTakeoverTarget::ExeGlobal
        ));
        assert!(owner_conflicts_with_target(
            &global_exe,
            InstallTakeoverTarget::MsiGlobal
        ));
    }

    #[test]
    fn msi_targets_never_launch_a_nested_msi_uninstall() {
        let global_msi = owner(
            InstallOrigin::MsiGlobal,
            RegisteredUninstall::Msi {
                product_code: "{A1111111-B222-4CCC-8DDD-E55555555555}".to_string(),
            },
        );
        assert!(!owner_conflicts_with_target(
            &global_msi,
            InstallTakeoverTarget::MsiGlobal
        ));
        assert!(owner_conflicts_with_target(
            &global_msi,
            InstallTakeoverTarget::ExeGlobal
        ));
    }

    #[test]
    fn machine_and_user_installer_scopes_are_never_crossed() {
        assert!(target_uses_machine_scope(InstallTakeoverTarget::MsiGlobal));
        assert!(target_uses_machine_scope(InstallTakeoverTarget::ExeGlobal));
        assert!(!target_uses_machine_scope(
            InstallTakeoverTarget::MsiCorporate
        ));
        assert!(!target_uses_machine_scope(
            InstallTakeoverTarget::ExeCorporate
        ));
        assert!(!target_uses_machine_scope(
            InstallTakeoverTarget::Standalone
        ));
    }
}
