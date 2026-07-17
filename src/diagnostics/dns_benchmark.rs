//! DNS resolver benchmark + integrity checks — deep diagnostic.
//!
//! Times A-record lookups against the system resolver and three public
//! resolvers (concurrent chains, sequential queries within each), then runs
//! two integrity probes:
//!
//! - **NXDOMAIN-wildcard hijack**: a random label under .com must NOT
//!   resolve; if it does, the resolver (typically the ISP) is rewriting
//!   NXDOMAIN responses to ad/search pages.
//! - **DNSSEC validation**: `dnssec-failed.org` carries a deliberately bogus
//!   signature. A validating resolver refuses to resolve it; a
//!   non-validating one returns an address. Subprocess-free and identical on
//!   all platforms.

use serde::Serialize;
use std::time::{Duration, Instant};

use super::util;

const TEST_DOMAINS: &[&str] = &["example.com", "wikipedia.org", "cloudflare.com"];

const PUBLIC_RESOLVERS: &[(&str, &str)] = &[
    ("Cloudflare", "1.1.1.1"),
    ("Google", "8.8.8.8"),
    ("Quad9", "9.9.9.9"),
];

/// Whole-module budget.
const MODULE_BUDGET: Duration = Duration::from_secs(20);

/// Deliberately-bogus DNSSEC zone maintained for validation testing.
const DNSSEC_BOGUS_DOMAIN: &str = "dnssec-failed.org";
/// A broadly available, DNSSEC-signed control. A bogus-zone failure means
/// nothing if this valid signed zone cannot resolve at the same time.
const DNSSEC_VALID_DOMAIN: &str = "cloudflare.com";

#[derive(Debug, Clone, Serialize)]
pub struct ResolverTiming {
    pub name: String,
    /// "system" for the OS resolver, else the server IP.
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_ms: Option<f64>,
    pub queries_ok: u8,
    pub queries_total: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct DnsBenchmark {
    pub resolvers: Vec<ResolverTiming>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fastest: Option<String>,
    /// How much slower the system resolver is than the fastest public one
    /// (positive = system slower).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_vs_fastest_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hijack_detected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dnssec_validating: Option<bool>,
    pub assessment: String,
    pub level: String,
}

pub async fn collect() -> Option<DnsBenchmark> {
    collect_with_budget(MODULE_BUDGET).await
}

async fn collect_with_budget(budget: Duration) -> Option<DnsBenchmark> {
    let deadline = tokio::time::Instant::now() + budget;

    // Benchmark all resolver chains concurrently.
    let mut chains = tokio::task::JoinSet::new();
    chains.spawn(benchmark_system());
    for (name, server) in PUBLIC_RESOLVERS {
        chains.spawn(benchmark_public(name, server));
    }
    let resolvers = join_before_deadline(chains, deadline).await?;

    // Control: did the system resolver resolve anything at all? If DNS is
    // dead the core DNS check already failed — skip this section.
    let system_ok = resolvers
        .iter()
        .any(|r| r.server == "system" && r.queries_ok > 0);
    if !system_ok {
        return None;
    }

    // Integrity probes (system resolver). The same valid signed control gates
    // both claims: a timeout or resolver outage is inconclusive, not evidence
    // that NXDOMAIN was preserved or that DNSSEC validation occurred.
    let integrity = async {
        let random_domain = random_hijack_domain();
        let (valid_control, bogus_zone, random_label) = tokio::join!(
            resolves(DNSSEC_VALID_DOMAIN),
            resolves(DNSSEC_BOGUS_DOMAIN),
            resolves(&random_domain),
        );
        (
            hijack_evidence(valid_control, random_label),
            dnssec_evidence(valid_control, bogus_zone),
        )
    };
    let (hijack_detected, dnssec_validating) =
        tokio::time::timeout_at(deadline, integrity).await.ok()?;

    Some(build_benchmark(
        resolvers,
        hijack_detected,
        dnssec_validating,
    ))
}

async fn join_before_deadline<T: 'static>(
    mut tasks: tokio::task::JoinSet<T>,
    deadline: tokio::time::Instant,
) -> Option<Vec<T>> {
    let mut values = Vec::new();
    while !tasks.is_empty() {
        match tokio::time::timeout_at(deadline, tasks.join_next()).await {
            Ok(Some(Ok(value))) => values.push(value),
            // A single failed worker is equivalent to that resolver being
            // unavailable. Preserve the other measurements.
            Ok(Some(Err(_))) => {}
            Ok(None) => break,
            Err(_) => {
                // JoinHandle/JoinSet drops detach work unless it is explicitly
                // aborted. Abort and reap every resolver chain before the
                // module returns so `dig`/`nslookup` cannot outlive the cap.
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return None;
            }
        }
    }
    Some(values)
}

async fn resolves(domain: &str) -> bool {
    util::lookup_host_timeout(format!("{domain}:443"), util::RESOLVE)
        .await
        .is_some_and(|addrs| !addrs.is_empty())
}

fn hijack_evidence(valid_control_resolved: bool, random_label_resolved: bool) -> Option<bool> {
    valid_control_resolved.then_some(random_label_resolved)
}

fn dnssec_evidence(valid_signed_resolved: bool, bogus_signed_resolved: bool) -> Option<bool> {
    if !valid_signed_resolved {
        None
    } else {
        Some(!bogus_signed_resolved)
    }
}

/// Pure assembly + verdict — unit-testable without a network.
fn build_benchmark(
    resolvers: Vec<ResolverTiming>,
    hijack_detected: Option<bool>,
    dnssec_validating: Option<bool>,
) -> DnsBenchmark {
    let fastest_public = resolvers
        .iter()
        .filter(|r| r.server != "system")
        .filter_map(|r| r.avg_ms.map(|ms| (r, ms)))
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let system_ms = resolvers
        .iter()
        .find(|r| r.server == "system")
        .and_then(|r| r.avg_ms);

    let fastest = fastest_public.map(|(r, _)| r.name.clone());
    let system_vs_fastest_ms = match (system_ms, fastest_public) {
        (Some(sys), Some((_, fast))) => Some(sys - fast),
        _ => None,
    };

    let system_much_slower = matches!(
        (system_ms, fastest_public),
        (Some(sys), Some((_, fast))) if sys > 2.0 * fast && (sys - fast) > 50.0
    );

    let integrity_inconclusive = hijack_detected.is_none() || dnssec_validating.is_none();
    let (assessment, level) = if hijack_detected == Some(true) {
        (
            "DNS hijack detected: non-existent domains resolve — the resolver is rewriting NXDOMAIN (typically ISP ad redirection)",
            "fail",
        )
    } else if dnssec_validating == Some(false) {
        (
            "Resolver does not validate DNSSEC — forged DNS answers would not be detected",
            "warn",
        )
    } else if system_much_slower {
        (
            "System resolver is much slower than public alternatives — consider changing DNS servers",
            "warn",
        )
    } else if integrity_inconclusive {
        (
            "Resolver performance was measured, but one or more DNS integrity checks were inconclusive",
            "warn",
        )
    } else {
        (
            "Resolver integrity checks passed: NXDOMAIN was preserved and DNSSEC validation was confirmed",
            "ok",
        )
    };

    DnsBenchmark {
        resolvers,
        fastest,
        system_vs_fastest_ms,
        hijack_detected,
        dnssec_validating,
        assessment: assessment.to_string(),
        level: level.to_string(),
    }
}

/// Time the system resolver across the test domains.
async fn benchmark_system() -> ResolverTiming {
    let mut times = Vec::new();
    let mut ok: u8 = 0;
    for domain in TEST_DOMAINS {
        let start = Instant::now();
        if util::lookup_host_timeout(format!("{}:443", domain), util::RESOLVE)
            .await
            .is_some_and(|a| !a.is_empty())
        {
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            ok += 1;
        }
    }
    ResolverTiming {
        name: "System".to_string(),
        server: "system".to_string(),
        avg_ms: avg(&times),
        queries_ok: ok,
        queries_total: TEST_DOMAINS.len() as u8,
    }
}

/// Time a public resolver across the test domains via the platform's DNS
/// query tool.
async fn benchmark_public(name: &str, server: &str) -> ResolverTiming {
    let mut times = Vec::new();
    let mut ok: u8 = 0;
    for domain in TEST_DOMAINS {
        if let Some(ms) = query_via_server(domain, server).await {
            times.push(ms);
            ok += 1;
        }
    }
    ResolverTiming {
        name: name.to_string(),
        server: server.to_string(),
        avg_ms: avg(&times),
        queries_ok: ok,
        queries_total: TEST_DOMAINS.len() as u8,
    }
}

fn avg(times: &[f64]) -> Option<f64> {
    if times.is_empty() {
        None
    } else {
        Some(times.iter().sum::<f64>() / times.len() as f64)
    }
}

/// Query `domain` against a specific `server`.
///
/// Unix: `dig` reports its own `Query time` (parse it — subprocess spawn
/// overhead excluded). Windows: `nslookup` has no timing output, so the
/// subprocess is wall-clocked; the constant spawn overhead affects every
/// resolver equally, so comparisons stay fair (noted for absolute values).
async fn query_via_server(domain: &str, server: &str) -> Option<f64> {
    #[cfg(windows)]
    {
        let start = Instant::now();
        let mut cmd = tokio::process::Command::new("nslookup");
        cmd.args(["-timeout=2", domain, server]);
        let output = util::run_with_timeout(cmd, util::SLOW).await?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        // The answer section repeats "Address"; a failed lookup prints
        // "can't find" instead.
        if text.contains("can't find") || !text.contains("Address") {
            return None;
        }
        Some(start.elapsed().as_secs_f64() * 1000.0)
    }

    #[cfg(unix)]
    {
        let mut cmd = tokio::process::Command::new("dig");
        cmd.args([
            &format!("@{}", server),
            domain,
            "+time=2",
            "+tries=1",
            "+noall",
            "+answer",
            "+stats",
        ]);
        let output = util::run_with_timeout(cmd, util::SLOW).await?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        parse_dig_query_time(&text)
    }
}

/// Parse `;; Query time: 23 msec` from dig output.
#[cfg(any(unix, test))]
fn parse_dig_query_time(text: &str) -> Option<f64> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(";; Query time:") {
            let num: String = rest
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            return num.parse().ok();
        }
    }
    None
}

/// Resolve a random label that must not exist. Resolving anyway = the
/// resolver wildcards NXDOMAIN. The label is derived from time + pid (no
/// rand dependency); collisions with a real domain are practically
/// impossible.
fn random_hijack_domain() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 + d.as_secs())
        .unwrap_or(0xdead_beef);
    let mixed = nanos
        .wrapping_mul(6364136223846793005)
        .wrapping_add(std::process::id() as u64);
    format!("nd300-{mixed:016x}.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(name: &str, server: &str, avg_ms: Option<f64>, ok: u8) -> ResolverTiming {
        ResolverTiming {
            name: name.to_string(),
            server: server.to_string(),
            avg_ms,
            queries_ok: ok,
            queries_total: 3,
        }
    }

    #[test]
    fn parse_dig_query_time_works() {
        let out =
            "example.com. 300 IN A 93.184.216.34\n;; Query time: 23 msec\n;; SERVER: 1.1.1.1#53";
        assert_eq!(parse_dig_query_time(out), Some(23.0));
        assert_eq!(parse_dig_query_time("no stats here"), None);
    }

    #[test]
    fn dnssec_requires_a_good_signed_control() {
        assert_eq!(dnssec_evidence(true, false), Some(true));
        assert_eq!(dnssec_evidence(true, true), Some(false));
        assert_eq!(dnssec_evidence(false, false), None);
    }

    #[test]
    fn hijack_claim_requires_a_good_control() {
        assert_eq!(hijack_evidence(true, false), Some(false));
        assert_eq!(hijack_evidence(true, true), Some(true));
        assert_eq!(hijack_evidence(false, false), None);
        assert_eq!(hijack_evidence(false, true), None);
    }

    #[test]
    fn inconclusive_integrity_never_claims_honesty() {
        for (hijack, dnssec) in [(None, Some(true)), (Some(false), None), (None, None)] {
            let b = build_benchmark(
                vec![timing("System", "system", Some(20.0), 3)],
                hijack,
                dnssec,
            );
            assert_eq!(b.level, "warn");
            assert!(b.assessment.contains("inconclusive"));
            assert!(!b.assessment.to_ascii_lowercase().contains("honest"));
        }
    }

    #[tokio::test]
    async fn module_deadline_aborts_and_reaps_spawned_work() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct Dropped(Arc<AtomicUsize>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicUsize::new(0));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..4 {
            let dropped = dropped.clone();
            tasks.spawn(async move {
                let _guard = Dropped(dropped);
                tokio::time::sleep(Duration::from_secs(60)).await;
                1usize
            });
        }

        let result = join_before_deadline(
            tasks,
            tokio::time::Instant::now() + Duration::from_millis(20),
        )
        .await;
        assert!(result.is_none());
        assert_eq!(dropped.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn hijack_is_fail_level() {
        let b = build_benchmark(
            vec![timing("System", "system", Some(20.0), 3)],
            Some(true),
            Some(true),
        );
        assert_eq!(b.level, "fail");
        assert!(b.assessment.contains("hijack"));
    }

    #[test]
    fn non_validating_dnssec_warns() {
        let b = build_benchmark(
            vec![timing("System", "system", Some(20.0), 3)],
            Some(false),
            Some(false),
        );
        assert_eq!(b.level, "warn");
        assert!(b.assessment.contains("DNSSEC"));
    }

    #[test]
    fn slow_system_resolver_warns() {
        let b = build_benchmark(
            vec![
                timing("System", "system", Some(180.0), 3),
                timing("Cloudflare", "1.1.1.1", Some(15.0), 3),
            ],
            Some(false),
            Some(true),
        );
        assert_eq!(b.level, "warn");
        assert_eq!(b.fastest.as_deref(), Some("Cloudflare"));
        assert!(b.system_vs_fastest_ms.is_some_and(|d| d > 100.0));
    }

    #[test]
    fn modestly_slower_system_is_ok() {
        // 2x but under the 50ms absolute floor — not worth a warning.
        let b = build_benchmark(
            vec![
                timing("System", "system", Some(30.0), 3),
                timing("Cloudflare", "1.1.1.1", Some(12.0), 3),
            ],
            Some(false),
            Some(true),
        );
        assert_eq!(b.level, "ok");
    }

    #[test]
    fn healthy_resolver_is_ok() {
        let b = build_benchmark(
            vec![
                timing("System", "system", Some(14.0), 3),
                timing("Cloudflare", "1.1.1.1", Some(15.0), 3),
                timing("Google", "8.8.8.8", Some(18.0), 3),
            ],
            Some(false),
            Some(true),
        );
        assert_eq!(b.level, "ok");
    }
}
