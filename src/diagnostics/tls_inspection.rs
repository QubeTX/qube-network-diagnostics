use reqwest::tls::TlsInfo;
use serde::Serialize;
use std::time::Duration;

const TLS_PROBE_URL: &str = "https://cloudflare.com/cdn-cgi/trace";
const TLS_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

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

enum TlsEvidence {
    Clear(String),
    Detected(Option<String>, String),
    Inconclusive(Option<String>, String),
}

#[derive(Debug, Clone)]
struct ParsedCertificateIssuer {
    display: String,
    organizations: Vec<String>,
    common_names: Vec<String>,
}

struct TlsProbe {
    issuer: Option<ParsedCertificateIssuer>,
    valid_trace: bool,
}

enum ProbeFailure {
    CertificateValidation(String),
    Other(String),
}

enum ProbeOutcome {
    Success(TlsProbe),
    Failure(ProbeFailure),
}

enum RootPolicy {
    Native,
    Public,
}

pub async fn collect() -> Option<TlsInspectionResult> {
    // The native-root probe represents what applications on this host trust,
    // including an enterprise inspection CA. A WebPKI-only control then tells
    // us whether that same connection is accepted by the public root set.
    // Both remain rustls handshakes and both retain the peer leaf certificate.
    let native_client = build_client(RootPolicy::Native).ok()?;
    let public_client = build_client(RootPolicy::Public).ok();
    let (native, public) = tokio::join!(probe(&native_client), async {
        match public_client {
            Some(client) => probe(&client).await,
            None => ProbeOutcome::Failure(ProbeFailure::Other(
                "the public-root control client could not be built".to_string(),
            )),
        }
    });

    let evidence = classify_evidence(native, public);
    let (detected, actual_issuer, description) = match evidence {
        TlsEvidence::Clear(issuer) => (
            false,
            Some(issuer),
            "TLS inspection check is clear: Cloudflare content, its public issuer, and public-root validation were verified"
                .to_string(),
        ),
        TlsEvidence::Detected(issuer, reason) => (
            true,
            issuer,
            format!("TLS interception detected: {reason}"),
        ),
        TlsEvidence::Inconclusive(issuer, reason) => (
            false,
            issuer,
            format!("TLS inspection check inconclusive: {reason}"),
        ),
    };

    Some(TlsInspectionResult {
        detected,
        description,
        tests: vec![TlsTest {
            host: "cloudflare.com".to_string(),
            expected_issuer: "Cloudflare Inc / DigiCert / Google Trust Services / Let's Encrypt"
                .to_string(),
            actual_issuer,
            intercepted: detected,
        }],
    })
}

fn build_client(policy: RootPolicy) -> Result<reqwest::Client, reqwest::Error> {
    let builder = reqwest::Client::builder()
        .timeout(TLS_PROBE_TIMEOUT)
        .tls_info(true)
        // Do not silently union the root stores for either side of the
        // comparison. Each client has exactly one trust policy.
        .tls_built_in_root_certs(false);
    let builder = match policy {
        RootPolicy::Native => builder.tls_built_in_native_certs(true),
        RootPolicy::Public => builder.tls_built_in_webpki_certs(true),
    };
    builder.build()
}

async fn probe(client: &reqwest::Client) -> ProbeOutcome {
    let response = match client.get(TLS_PROBE_URL).send().await {
        Ok(response) => response,
        Err(error) => {
            let message = error_chain(&error);
            return ProbeOutcome::Failure(classify_probe_error(&message));
        }
    };

    let issuer = response
        .extensions()
        .get::<TlsInfo>()
        .and_then(TlsInfo::peer_certificate)
        .and_then(parse_certificate_issuer);
    let body = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            return ProbeOutcome::Failure(ProbeFailure::Other(format!(
                "the HTTPS body could not be read ({error})"
            )));
        }
    };
    let valid_trace = std::str::from_utf8(&body)
        .map(|text| {
            text.lines().any(|line| line.starts_with("fl="))
                && text.lines().any(|line| line.starts_with("ip="))
        })
        .unwrap_or(false);

    ProbeOutcome::Success(TlsProbe {
        issuer,
        valid_trace,
    })
}

fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source {
        let message = error.to_string();
        if !messages.iter().any(|existing| existing == &message) {
            messages.push(message);
        }
        source = error.source();
    }
    messages.join(": ")
}

fn classify_evidence(native: ProbeOutcome, public: ProbeOutcome) -> TlsEvidence {
    let native = match native {
        ProbeOutcome::Success(probe) => probe,
        ProbeOutcome::Failure(failure) => {
            return TlsEvidence::Inconclusive(
                None,
                format!(
                    "the native-root HTTPS probe failed before peer-certificate evidence could be collected ({})",
                    failure_message(&failure)
                ),
            );
        }
    };

    let Some(issuer) = native.issuer else {
        return TlsEvidence::Inconclusive(
            None,
            "the TLS backend did not expose a parseable peer certificate".to_string(),
        );
    };
    let issuer_display = Some(issuer.display.clone());

    // An issuer that explicitly identifies an inspection product is direct
    // evidence even if a public-root control happened to fail for another
    // reason.
    if issuer_looks_like_inspection_ca(&issuer) {
        return TlsEvidence::Detected(
            issuer_display,
            "the peer certificate was issued by a recognizable inspection/proxy authority"
                .to_string(),
        );
    }

    // Native trust succeeding while the public-root-only control rejects the
    // certificate is positive evidence that a locally trusted certificate
    // replaced the public Cloudflare chain. This catches enterprise CAs with
    // generic names rather than relying only on brand keywords.
    if let ProbeOutcome::Failure(ProbeFailure::CertificateValidation(reason)) = &public {
        return TlsEvidence::Detected(
            issuer_display,
            format!(
                "the host trust store accepted the peer certificate, but the public root set rejected it ({reason})"
            ),
        );
    }

    let public_trace_valid = matches!(
        &public,
        ProbeOutcome::Success(TlsProbe {
            valid_trace: true,
            ..
        })
    );
    if native.valid_trace && public_trace_valid && issuer_is_expected_public(&issuer) {
        return TlsEvidence::Clear(issuer.display);
    }

    let reason = match public {
        ProbeOutcome::Success(_) if !native.valid_trace => {
            "the peer certificate was available, but the native-root response was not Cloudflare's trace payload"
                .to_string()
        }
        ProbeOutcome::Success(_) if !issuer_is_expected_public(&issuer) => {
            "the public-root control succeeded, but the issuer was outside the recognized Cloudflare public set"
                .to_string()
        }
        ProbeOutcome::Success(_) => {
            "the public-root control did not return the expected Cloudflare trace payload".to_string()
        }
        ProbeOutcome::Failure(failure) => format!(
            "the public-root control was unavailable for a non-certificate reason ({})",
            failure_message(&failure)
        ),
    };
    TlsEvidence::Inconclusive(issuer_display, reason)
}

fn classify_probe_error(message: &str) -> ProbeFailure {
    let lower = message.to_ascii_lowercase();
    if [
        "invalid peer certificate",
        "unknownissuer",
        "unknown issuer",
        "certificate verify failed",
        "certificate validation",
        "certificate error",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        ProbeFailure::CertificateValidation(message.to_string())
    } else {
        ProbeFailure::Other(message.to_string())
    }
}

fn failure_message(failure: &ProbeFailure) -> &str {
    match failure {
        ProbeFailure::CertificateValidation(message) | ProbeFailure::Other(message) => message,
    }
}

fn issuer_looks_like_inspection_ca(issuer: &ParsedCertificateIssuer) -> bool {
    [
        "inspection",
        "corporate",
        "proxy",
        "zscaler",
        "netskope",
        "fortinet",
        "palo alto",
        "blue coat",
        "checkpoint",
        "sophos",
    ]
    .iter()
    .any(|marker| {
        std::iter::once(issuer.display.as_str())
            .chain(issuer.organizations.iter().map(String::as_str))
            .chain(issuer.common_names.iter().map(String::as_str))
            .any(|value| value.to_ascii_lowercase().contains(marker))
    })
}

fn parse_certificate_issuer(der: &[u8]) -> Option<ParsedCertificateIssuer> {
    use x509_parser::prelude::FromDer;
    let (_, certificate) = x509_parser::certificate::X509Certificate::from_der(der).ok()?;
    let issuer = certificate.issuer();
    let organizations = issuer
        .iter_organization()
        .filter_map(|attribute| attribute.as_str().ok().map(ToString::to_string))
        .collect();
    let common_names = issuer
        .iter_common_name()
        .filter_map(|attribute| attribute.as_str().ok().map(ToString::to_string))
        .collect();
    Some(ParsedCertificateIssuer {
        display: issuer.to_string(),
        organizations,
        common_names,
    })
}

fn issuer_is_expected_public(issuer: &ParsedCertificateIssuer) -> bool {
    const PUBLIC_ORGANIZATIONS: &[&str] = &[
        "Cloudflare, Inc.",
        // Current Cloudflare WE1 certificates use this exact organization;
        // older chains used the LLC suffix.
        "Google Trust Services",
        "Google Trust Services LLC",
        "DigiCert Inc",
        "Let's Encrypt",
        "Internet Security Research Group",
        "Sectigo Limited",
    ];

    issuer.organizations.iter().any(|organization| {
        PUBLIC_ORGANIZATIONS
            .iter()
            .any(|expected| organization.eq_ignore_ascii_case(expected))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issuer(organization: &str, common_name: &str) -> ParsedCertificateIssuer {
        ParsedCertificateIssuer {
            display: format!("CN={common_name}, O={organization}"),
            organizations: vec![organization.to_string()],
            common_names: vec![common_name.to_string()],
        }
    }

    fn success(issuer: ParsedCertificateIssuer, valid_trace: bool) -> ProbeOutcome {
        ProbeOutcome::Success(TlsProbe {
            issuer: Some(issuer),
            valid_trace,
        })
    }

    #[test]
    fn current_google_trust_services_organization_is_exactly_allowed() {
        assert!(issuer_is_expected_public(&issuer(
            "Google Trust Services",
            "WE1"
        )));
        assert!(issuer_is_expected_public(&issuer(
            "google trust services llc",
            "WE1"
        )));
        assert!(!issuer_is_expected_public(&issuer(
            "Google Trust Services Corporate Proxy",
            "WE1"
        )));
    }

    #[test]
    fn clear_requires_native_and_public_root_payload_evidence() {
        let evidence = classify_evidence(
            success(issuer("Google Trust Services", "WE1"), true),
            success(issuer("Google Trust Services", "WE1"), true),
        );
        assert!(matches!(evidence, TlsEvidence::Clear(_)));

        let evidence = classify_evidence(
            success(issuer("Google Trust Services", "WE1"), true),
            ProbeOutcome::Failure(ProbeFailure::Other("timeout".to_string())),
        );
        assert!(matches!(evidence, TlsEvidence::Inconclusive(_, _)));
    }

    #[test]
    fn native_only_enterprise_ca_is_detected_without_brand_keyword() {
        let evidence = classify_evidence(
            success(issuer("Acme IT", "Acme Root CA"), true),
            ProbeOutcome::Failure(ProbeFailure::CertificateValidation(
                "invalid peer certificate: UnknownIssuer".to_string(),
            )),
        );
        match evidence {
            TlsEvidence::Detected(Some(actual), reason) => {
                assert!(actual.contains("Acme IT"));
                assert!(reason.contains("public root set rejected"));
            }
            _ => panic!("native-only enterprise trust must be detected"),
        }
    }

    #[test]
    fn inspection_marker_wins_over_public_brand_substrings() {
        let inspection = issuer("DigiCert Corporate Inspection", "Inspection CA");
        assert!(!issuer_is_expected_public(&inspection));
        assert!(issuer_looks_like_inspection_ca(&inspection));
        assert!(matches!(
            classify_evidence(
                success(inspection, true),
                success(issuer("DigiCert Inc", "CA"), true)
            ),
            TlsEvidence::Detected(_, _)
        ));
    }

    #[test]
    fn non_certificate_failures_remain_inconclusive() {
        assert!(matches!(
            classify_probe_error("request timed out"),
            ProbeFailure::Other(_)
        ));
        let evidence = classify_evidence(
            success(issuer("Acme IT", "Acme Root CA"), true),
            ProbeOutcome::Failure(ProbeFailure::Other("request timed out".to_string())),
        );
        assert!(matches!(evidence, TlsEvidence::Inconclusive(_, _)));
    }
}
