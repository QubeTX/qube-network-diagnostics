use super::cmd::{run_cmd, TIMEOUT_QUICK};
use crate::actions::trusted_executable;

/// ARP cache flush — all platforms.
pub async fn flush_arp() -> Result<String, String> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new(trusted_executable("arp"));
        cmd.args(["-d", "*"]);
        match run_cmd(cmd, TIMEOUT_QUICK).await {
            Ok(output) if output.status.success() => Ok("ARP cache flushed".to_string()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = tokio::process::Command::new(trusted_executable("arp"));
        cmd.args(["-d", "-a"]);
        match run_cmd(cmd, TIMEOUT_QUICK).await {
            Ok(output) if output.status.success() => Ok("ARP cache flushed".to_string()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = tokio::process::Command::new(trusted_executable("ip"));
        cmd.args(["neigh", "flush", "all"]);
        match run_cmd(cmd, TIMEOUT_QUICK).await {
            Ok(output) if output.status.success() => Ok("ARP cache flushed".to_string()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }
}
