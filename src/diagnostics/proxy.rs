use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProxyConfig {
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    pub socks_proxy: Option<String>,
    pub no_proxy: Option<String>,
    pub pac_url: Option<String>,
    pub proxy_enabled: bool,
}

pub async fn collect() -> Option<ProxyConfig> {
    // Check environment variables first
    let http_proxy = std::env::var("HTTP_PROXY")
        .or_else(|_| std::env::var("http_proxy"))
        .ok();
    let https_proxy = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .ok();
    let socks_proxy = std::env::var("ALL_PROXY")
        .or_else(|_| std::env::var("all_proxy"))
        .ok();
    let no_proxy = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .ok();

    let mut config = ProxyConfig {
        http_proxy,
        https_proxy,
        socks_proxy,
        no_proxy,
        pac_url: None,
        proxy_enabled: false,
    };

    // Check OS-specific proxy settings
    #[cfg(windows)]
    {
        check_windows_proxy(&mut config).await;
    }

    #[cfg(target_os = "macos")]
    {
        check_macos_proxy(&mut config).await;
    }

    config.proxy_enabled = config.http_proxy.is_some()
        || config.https_proxy.is_some()
        || config.socks_proxy.is_some();

    Some(config)
}

#[cfg(windows)]
async fn check_windows_proxy(config: &mut ProxyConfig) {
    // Check Windows registry via reg query
    if let Ok(output) = tokio::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            "ProxyEnable",
        ])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains("0x1") {
            config.proxy_enabled = true;

            // Get proxy server
            if let Ok(output) = tokio::process::Command::new("reg")
                .args([
                    "query",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
                    "/v",
                    "ProxyServer",
                ])
                .output()
                .await
            {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    if line.contains("ProxyServer") {
                        if let Some(val) = line.split_whitespace().last() {
                            if config.http_proxy.is_none() {
                                config.http_proxy = Some(val.to_string());
                            }
                        }
                    }
                }
            }

            // Get PAC URL
            if let Ok(output) = tokio::process::Command::new("reg")
                .args([
                    "query",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
                    "/v",
                    "AutoConfigURL",
                ])
                .output()
                .await
            {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    if line.contains("AutoConfigURL") {
                        config.pac_url = line.split_whitespace().last().map(|s| s.to_string());
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
async fn check_macos_proxy(config: &mut ProxyConfig) {
    if let Ok(output) = tokio::process::Command::new("scutil")
        .args(["--proxy"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);

        for line in text.lines() {
            let line = line.trim();
            if line.contains("HTTPEnable") && line.contains("1") {
                config.proxy_enabled = true;
            }
            if line.contains("HTTPProxy") {
                config.http_proxy = line.splitn(2, ':').nth(1).map(|s| s.trim().to_string());
            }
            if line.contains("HTTPSProxy") {
                config.https_proxy = line.splitn(2, ':').nth(1).map(|s| s.trim().to_string());
            }
            if line.contains("SOCKSProxy") {
                config.socks_proxy = line.splitn(2, ':').nth(1).map(|s| s.trim().to_string());
            }
            if line.contains("ProxyAutoConfigURLString") {
                config.pac_url = line.splitn(2, ':').nth(1).map(|s| s.trim().to_string());
            }
        }
    }
}
