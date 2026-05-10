use indicatif::ProgressBar;

use super::dns;

const RETRY_ATTEMPTS: u32 = 3;
const WAIT_SECONDS: u64 = 30;

/// Retry connectivity check up to 3 times with a 30-second countdown between each attempt.
/// Updates the spinner with a visible countdown so the user knows the tool isn't frozen.
pub async fn wait_for_connectivity(spinner: &ProgressBar) -> bool {
    for attempt in 1..=RETRY_ATTEMPTS {
        // Countdown wait
        for remaining in (1..=WAIT_SECONDS).rev() {
            let msg = if attempt == 1 {
                format!("Waiting for network to reconnect ({}s)...", remaining)
            } else {
                format!(
                    "Not connected yet \u{2014} retrying (attempt {} of {}, {}s)...",
                    attempt, RETRY_ATTEMPTS, remaining
                )
            };
            spinner.set_message(msg);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        spinner.set_message(format!(
            "Checking connectivity (attempt {} of {})...",
            attempt, RETRY_ATTEMPTS
        ));

        if check_connectivity().await {
            return true;
        }
    }
    false
}

/// Lightweight connectivity check via HTTP GET.
/// Returns true if we can reach the internet.
pub async fn check_connectivity() -> bool {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    match client
        .get("http://detectportal.firefox.com/success.txt")
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(body) = resp.text().await {
                body.trim() == "success"
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Verify DNS is stable after connectivity check passes.
/// Waits 2 seconds for DNS to settle, then resolves 3 domains.
pub async fn verify_dns_stability(spinner: &ProgressBar) -> bool {
    spinner.set_message("Verifying DNS resolution...");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    dns::verify_dns().await
}
