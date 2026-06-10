use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProxyConfig {
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    pub socks_proxy: Option<String>,
    pub no_proxy: Option<String>,
    pub pac_url: Option<String>,
    pub proxy_enabled: bool,
    /// The configured PAC script could actually be fetched (additive, v3.4.0+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pac_reachable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pac_size_bytes: Option<u64>,
    /// A bare `wpad` hostname resolves on this network — proxy auto-discovery
    /// is live, which is also a well-known LAN attack vector worth surfacing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wpad_dns_detected: Option<bool>,
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
        pac_reachable: None,
        pac_size_bytes: None,
        wpad_dns_detected: None,
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

    // Linux desktop fallback: GNOME proxy settings (env vars are empty for
    // GUI-configured proxies).
    #[cfg(target_os = "linux")]
    {
        if config.http_proxy.is_none() && config.https_proxy.is_none() {
            check_gnome_proxy(&mut config).await;
        }
    }

    config.proxy_enabled =
        config.http_proxy.is_some() || config.https_proxy.is_some() || config.socks_proxy.is_some();

    // PAC validation (v3.4.0+): a configured-but-dead PAC script silently
    // breaks browsing in ways env-var checks can't see.
    if let Some(ref pac_url) = config.pac_url {
        if let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .no_proxy()
            .build()
        {
            match client.get(pac_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    config.pac_reachable = Some(true);
                    config.pac_size_bytes = resp.bytes().await.ok().map(|b| b.len() as u64);
                }
                _ => config.pac_reachable = Some(false),
            }
        }
    }

    // WPAD auto-discovery probe: a resolvable bare `wpad` name means clients
    // here may auto-configure a proxy from the LAN.
    config.wpad_dns_detected = Some(
        super::util::lookup_host_timeout("wpad:80".to_string(), super::util::RESOLVE)
            .await
            .is_some_and(|a| !a.is_empty()),
    );

    Some(config)
}

/// GNOME proxy settings via gsettings (best-effort; absent on servers).
#[cfg(target_os = "linux")]
async fn check_gnome_proxy(config: &mut ProxyConfig) {
    let mut mode_cmd = tokio::process::Command::new("gsettings");
    mode_cmd.args(["get", "org.gnome.system.proxy", "mode"]);
    let Some(output) = super::util::run_with_timeout(mode_cmd, super::util::QUICK).await else {
        return;
    };
    let mode = String::from_utf8_lossy(&output.stdout)
        .trim()
        .replace('\'', "");
    match mode.as_str() {
        "manual" => {
            let mut host_cmd = tokio::process::Command::new("gsettings");
            host_cmd.args(["get", "org.gnome.system.proxy.http", "host"]);
            if let Some(host_out) =
                super::util::run_with_timeout(host_cmd, super::util::QUICK).await
            {
                let host = String::from_utf8_lossy(&host_out.stdout)
                    .trim()
                    .replace('\'', "");
                if !host.is_empty() {
                    config.http_proxy = Some(host);
                }
            }
        }
        "auto" => {
            let mut url_cmd = tokio::process::Command::new("gsettings");
            url_cmd.args(["get", "org.gnome.system.proxy", "autoconfig-url"]);
            if let Some(url_out) = super::util::run_with_timeout(url_cmd, super::util::QUICK).await
            {
                let url = String::from_utf8_lossy(&url_out.stdout)
                    .trim()
                    .replace('\'', "");
                if !url.is_empty() {
                    config.pac_url = Some(url);
                }
            }
        }
        _ => {}
    }
}

#[cfg(windows)]
async fn check_windows_proxy(config: &mut ProxyConfig) {
    // Check Windows registry via reg query
    let mut cmd = tokio::process::Command::new("reg");
    cmd.args([
        "query",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
        "/v",
        "ProxyEnable",
    ]);
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains("0x1") {
            config.proxy_enabled = true;

            // Get proxy server
            let mut cmd = tokio::process::Command::new("reg");
            cmd.args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
                "/v",
                "ProxyServer",
            ]);
            if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
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
            let mut cmd = tokio::process::Command::new("reg");
            cmd.args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
                "/v",
                "AutoConfigURL",
            ]);
            if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
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
    let mut cmd = tokio::process::Command::new("scutil");
    cmd.args(["--proxy"]);
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);

        for line in text.lines() {
            let line = line.trim();
            if line.contains("HTTPEnable") && line.contains("1") {
                config.proxy_enabled = true;
            }
            if line.contains("HTTPProxy") {
                config.http_proxy = line.split_once(':').map(|x| x.1.trim().to_string());
            }
            if line.contains("HTTPSProxy") {
                config.https_proxy = line.split_once(':').map(|x| x.1.trim().to_string());
            }
            if line.contains("SOCKSProxy") {
                config.socks_proxy = line.split_once(':').map(|x| x.1.trim().to_string());
            }
            if line.contains("ProxyAutoConfigURLString") {
                config.pac_url = line.split_once(':').map(|x| x.1.trim().to_string());
            }
        }
    }
}
