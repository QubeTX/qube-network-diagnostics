//! Serialize WPA-Personal credentials without interpolating configuration syntax.

use std::num::NonZeroU32;

/// Ordinary quoted wpa_supplicant fields do not decode C-style escapes. Use
/// its native hex SSID and 256-bit PSK forms instead, preserving the SSID bytes
/// and printable password punctuation without permitting injected directives.
pub(super) fn network_block(ssid: &str, passphrase: &str) -> Result<String, String> {
    if !(1..=32).contains(&ssid.len()) {
        return Err("Wi-Fi network name must contain 1 to 32 bytes".to_string());
    }
    if !(8..=63).contains(&passphrase.len())
        || !passphrase.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        return Err("Wi-Fi passphrase must contain 8 to 63 printable ASCII characters".to_string());
    }

    let mut psk = [0u8; 32];
    // These parameters are mandated by WPA-PSK, not a password-storage policy.
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA1,
        NonZeroU32::new(4096).expect("WPA iteration count is nonzero"),
        ssid.as_bytes(),
        passphrase.as_bytes(),
        &mut psk,
    );
    // The leading newline also separates this block from a file without a final
    // newline (including a trailing comment).
    Ok(format!(
        "\nnetwork={{\n  ssid={}\n  psk={}\n}}\n",
        hex(ssid.as_bytes()),
        hex(&psk)
    ))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        value.push(DIGITS[(byte >> 4) as usize] as char);
        value.push(DIGITS[(byte & 15) as usize] as char);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpa_published_known_answer() {
        // IEEE WPA-PSK test vector: password="password", SSID="IEEE".
        assert_eq!(
            network_block("IEEE", "password").unwrap(),
            "\nnetwork={\n  ssid=49454545\n  psk=f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e\n}\n"
        );
    }

    #[test]
    fn hostile_ssids_remain_one_hex_value() {
        for ssid in [
            "\"\n}\nnetwork={\n#",
            "slash\\quote\"",
            "\0\r\t\u{7f}",
            "café📡",
        ] {
            let config = network_block(ssid, "a\"b\\c#de").unwrap();
            let lines: Vec<_> = config.lines().collect();
            assert_eq!(lines.len(), 5);
            assert_eq!(lines[0], "");
            assert_eq!(lines[1], "network={");
            assert_eq!(lines[4], "}");
            let encoded = lines[2].strip_prefix("  ssid=").unwrap();
            let decoded: Vec<_> = encoded
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            assert_eq!(decoded, ssid.as_bytes());
            let key = lines[3].strip_prefix("  psk=").unwrap();
            assert_eq!(key.len(), 64);
            assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert!(!config.contains("a\"b\\c#de"));
        }
    }

    #[test]
    fn rejects_invalid_lengths_and_non_ascii_passphrases_without_echoing_them() {
        for ssid in [String::new(), "a".repeat(33), "é".repeat(17)] {
            assert!(network_block(&ssid, "password").is_err());
        }
        for password in [
            "".to_string(),
            "a".repeat(7),
            "a".repeat(64),
            "password\n}".to_string(),
            "password\r".to_string(),
            "password\t".to_string(),
            "password\0".to_string(),
            "password\u{7f}".to_string(),
            "passwörd".to_string(),
        ] {
            assert_eq!(
                network_block("SSID", &password).unwrap_err(),
                "Wi-Fi passphrase must contain 8 to 63 printable ASCII characters"
            );
        }
    }

    #[test]
    fn accepts_byte_boundaries_and_all_printable_password_punctuation() {
        assert!(network_block("a", "a".repeat(8).as_str()).is_ok());
        assert!(network_block(&"é".repeat(16), &"a".repeat(63)).is_ok());
        for byte in 0x20u8..=0x7e {
            let password = format!("prefix{}suffix", byte as char);
            assert!(network_block("SSID", &password).is_ok());
        }
    }

    /// CI explicitly installs wpasupplicant and runs this ignored test. An
    /// unsupported driver makes the real parser exit before it can open any
    /// network device; fixtures use only synthetic credentials.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires the real Linux wpasupplicant package; run explicitly in CI"]
    async fn real_parser_preserves_credentials() {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        async fn parse(config: &str) -> String {
            let path = std::env::temp_dir().join(format!(
                "nd300-wpa-parser-{}-{}.conf",
                std::process::id(),
                rand::random::<u64>()
            ));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .unwrap();
            file.write_all(config.as_bytes()).unwrap();
            drop(file);
            let mut command = crate::platform::system_command("wpa_supplicant");
            command
                .args(["-dd", "-Dnd300-no-device", "-ind300-unused", "-c"])
                .arg(&path)
                .env("LC_ALL", "C");
            let result = crate::diagnostics::util::run_with_timeout(
                command,
                std::time::Duration::from_secs(5),
            )
            .await;
            std::fs::remove_file(&path).unwrap();
            let output =
                result.expect("wpa_supplicant must be installed and exit within five seconds");
            let log = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.status.success(), "unsupported driver must fail");
            assert!(
                log.contains("Unsupported driver 'nd300-no-device'"),
                "{log}"
            );
            assert!(
                !log.contains("Failed to read or parse configuration"),
                "{log}"
            );
            log
        }

        let hostile_ssid = "x\"\n  disabled=1\n#";
        // Positive control: prove that the former interpolation really changes
        // a parsed setting, rather than merely looking dangerous as text.
        let old = format!("network={{\n  ssid=\"{hostile_ssid}\"\n  psk=\"password\"\n}}\n");
        let has_disabled_directive = |log: &str| {
            log.lines()
                .any(|line| line.trim_start().starts_with("disabled=1 ("))
        };
        assert!(has_disabled_directive(&parse(&old).await));

        for ssid in [hostile_ssid, "slash\\quote\"", "café📡", "\0\r\t\u{7f}"] {
            let config = network_block(ssid, "a\"b\\c#de").unwrap();
            let log = parse(&config).await;
            assert!(!has_disabled_directive(&log), "{log}");
            assert!(
                log.contains(&format!("ssid - hexdump_ascii(len={})", ssid.len())),
                "{log}"
            );
        }
    }
}
