/// ARP cache flush — all platforms.
pub async fn flush_arp() -> Result<String, String> {
    #[cfg(windows)]
    {
        match tokio::process::Command::new("arp")
            .args(["-d", "*"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                Ok("ARP cache flushed".to_string())
            }
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(format!("Failed to run arp: {}", e)),
        }
    }

    #[cfg(target_os = "macos")]
    {
        match tokio::process::Command::new("arp")
            .args(["-d", "-a"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                Ok("ARP cache flushed".to_string())
            }
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(format!("Failed to run arp: {}", e)),
        }
    }

    #[cfg(target_os = "linux")]
    {
        match tokio::process::Command::new("ip")
            .args(["neigh", "flush", "all"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                Ok("ARP cache flushed".to_string())
            }
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(format!("Failed to run ip neigh flush: {}", e)),
        }
    }
}
