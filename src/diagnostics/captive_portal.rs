//! Captive portal detection — deep diagnostic.
//!
//! Probes Firefox's portal-detection endpoint (the same URL the fix flow's
//! connectivity check uses) with redirects disabled and no proxy. A clean
//! network returns HTTP 200 with the literal body `success`; a captive
//! portal answers with a redirect to its sign-in page or rewrites the body.

use serde::Serialize;
use std::time::Duration;

use crate::actions::fix::connectivity::PORTAL_PROBE_URL;

#[derive(Debug, Clone, Serialize)]
pub struct CaptivePortalResult {
    pub portal_detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_location: Option<String>,
    pub body_matched: bool,
    pub assessment: String,
    pub level: String,
}

pub async fn collect() -> Option<CaptivePortalResult> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;

    let resp = match client.get(PORTAL_PROBE_URL).send().await {
        Ok(resp) => resp,
        // No connectivity at all — the core checks already failed; this
        // section is about portals, not outages. Skip.
        Err(_) => return None,
    };

    let status = resp.status().as_u16();
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body = resp.text().await.unwrap_or_default();

    Some(classify(status, &body, location.as_deref()))
}

/// Pure classification — unit-testable without a network.
fn classify(status: u16, body: &str, location: Option<&str>) -> CaptivePortalResult {
    let body_matched = body.trim() == "success";

    let (portal_detected, assessment, level) = if (300..400).contains(&status) {
        (
            true,
            format!(
                "Captive portal detected — open a browser and sign in{}",
                location
                    .map(|l| format!(" (redirects to {})", l))
                    .unwrap_or_default()
            ),
            "fail",
        )
    } else if status == 200 && body_matched {
        (false, "No captive portal".to_string(), "ok")
    } else if status == 200 {
        (
            true,
            "Probe response was rewritten — a captive portal or intercepting proxy is altering traffic".to_string(),
            "fail",
        )
    } else {
        (
            true,
            format!(
                "Probe returned HTTP {} — something is intercepting plain HTTP",
                status
            ),
            "warn",
        )
    };

    CaptivePortalResult {
        portal_detected,
        http_status: Some(status),
        redirect_location: location.map(|s| s.to_string()),
        body_matched,
        assessment,
        level: level.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_success_is_ok() {
        let r = classify(200, "success", None);
        assert!(!r.portal_detected);
        assert_eq!(r.level, "ok");
        assert!(r.body_matched);
    }

    #[test]
    fn redirect_is_portal() {
        let r = classify(302, "", Some("https://portal.hotel.example/login"));
        assert!(r.portal_detected);
        assert_eq!(r.level, "fail");
        assert!(r.assessment.contains("portal.hotel.example"));
    }

    #[test]
    fn rewritten_body_is_portal() {
        let r = classify(200, "<html>Welcome to FreeAirportWiFi</html>", None);
        assert!(r.portal_detected);
        assert_eq!(r.level, "fail");
        assert!(!r.body_matched);
    }

    #[test]
    fn other_status_warns() {
        let r = classify(503, "", None);
        assert!(r.portal_detected);
        assert_eq!(r.level, "warn");
    }
}
