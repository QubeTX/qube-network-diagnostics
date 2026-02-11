use std::process::Output;
use std::time::Duration;
use tokio::process::Command;

pub const TIMEOUT_QUICK: Duration = Duration::from_secs(15);
pub const TIMEOUT_MEDIUM: Duration = Duration::from_secs(30);
pub const TIMEOUT_SLOW: Duration = Duration::from_secs(60);

pub async fn run_cmd(mut cmd: Command, timeout: Duration) -> Result<Output, String> {
    let label = format!("{:?}", cmd.as_std().get_program());
    match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(format!("{} failed: {}", label, e)),
        Err(_) => Err(format!("{} timed out after {}s", label, timeout.as_secs())),
    }
}
