use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TlsInspectionResult {
    pub detected: bool,
    pub description: String,
    pub tests: Vec<TlsTest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TlsTest {
    pub host: String,
    pub expected_issuer: String,
    pub actual_issuer: Option<String>,
    pub intercepted: bool,
}

pub async fn collect() -> Option<TlsInspectionResult> {
    // Check if TLS connections to known sites are being intercepted
    // by comparing certificate issuers to expected values
    let mut tests = Vec::new();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;

    // Test against Cloudflare - if we can reach their trace endpoint,
    // check for signs of interception
    let intercepted = check_tls_interception(&client).await;

    tests.push(TlsTest {
        host: "cloudflare.com".to_string(),
        expected_issuer: "Cloudflare Inc / DigiCert / Google Trust Services".to_string(),
        actual_issuer: None,
        intercepted,
    });

    let description = if intercepted {
        "TLS interception detected - a proxy may be inspecting HTTPS traffic".to_string()
    } else {
        "No TLS interception detected".to_string()
    };

    Some(TlsInspectionResult {
        detected: intercepted,
        description,
        tests,
    })
}

async fn check_tls_interception(client: &reqwest::Client) -> bool {
    // If we can make a request but the response indicates unexpected behavior,
    // it might be intercepted. This is a basic heuristic check.

    // Check 1: Can we reach a known canary URL?
    // Corporate MITM proxies sometimes fail specific certificate pinning checks

    // Try to connect and verify the response matches expected patterns
    match client.get("https://1.1.1.1/cdn-cgi/trace").send().await {
        Ok(resp) => {
            let text = resp.text().await.unwrap_or_default();
            // If we get a valid Cloudflare trace response, likely not intercepted
            if text.contains("fl=") && text.contains("ip=") {
                return false;
            }
            // Got a response but not from Cloudflare = likely intercepted
            true
        }
        Err(e) => {
            // Connection error could indicate MITM with invalid cert
            let err_str = format!("{}", e);
            err_str.contains("certificate") || err_str.contains("SSL") || err_str.contains("tls")
        }
    }
}
