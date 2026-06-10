/// Probe URL for the lightweight connectivity check. Firefox's captive-portal
/// detection endpoint: returns HTTP 200 with the literal body `success` on an
/// open network; a captive portal redirects or rewrites it.
pub const PORTAL_PROBE_URL: &str = "http://detectportal.firefox.com/success.txt";

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

    match client.get(PORTAL_PROBE_URL).send().await {
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
