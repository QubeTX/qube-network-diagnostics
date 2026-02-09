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

    match client.get("http://detectportal.firefox.com/success.txt").send().await {
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
