//! Pure planning logic for the diagnostic-driven fix loop.
//!
//! Everything in this module is a pure function of the current
//! [`DiagnosticResults`] plus per-session state. No IO, no side effects —
//! ideal for unit testing against synthetic fixtures.

use std::collections::{HashMap, HashSet};

use crate::diagnostics::{DiagnosticResults, DiagnosticStatus};

use super::action::{Action, ActionId, DiagnosticKey, Risk};

/// Per-session attempt counts indexed by [`ActionId`]. Used to enforce
/// `Action::max_attempts` across iterations.
pub type Attempts = HashMap<ActionId, u8>;

/// Within a single session, whether a previously-applied action helped (the
/// failure it targeted cleared on the next iteration). Used by
/// [`build_plan`] to break ties between candidate actions.
pub type Effectiveness = HashMap<(ActionId, DiagnosticKey), bool>;

/// Reasons the loop should stop immediately rather than apply any actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardBlock {
    CaptivePortal,
    NoPhysicalLink,
    IspOutage,
    EnterpriseVpnActive(String),
}

impl HardBlock {
    pub fn user_message(&self) -> String {
        match self {
            HardBlock::CaptivePortal => {
                "You appear to be behind a captive portal (a hotel / café / airport sign-in page). Open your browser, sign in, then re-run `nd300 fix`.".to_string()
            }
            HardBlock::NoPhysicalLink => {
                "No active network connection detected. Plug in an Ethernet cable or connect to Wi-Fi, then try again.".to_string()
            }
            HardBlock::IspOutage => {
                "Your local network is healthy, but the path to the wider internet is failing. This usually means your ISP is having an outage. Try again in a few minutes.".to_string()
            }
            HardBlock::EnterpriseVpnActive(name) => {
                format!("An enterprise VPN ({}) is active and is shaping the diagnostics. nd300 will not auto-disable enterprise VPNs. Disconnect the VPN, or contact IT, and try again.", name)
            }
        }
    }
}

/// Walk a [`DiagnosticResults`] and collect every category whose status is
/// actionable (we have a reasonable fix for it). `Skip` and `Ok` are ignored.
/// `Warn` is included only for categories where we know how to attempt a fix.
pub fn actionable_failures(results: &DiagnosticResults) -> HashSet<DiagnosticKey> {
    use DiagnosticKey::*;
    let mut out = HashSet::new();

    let pairs: &[(&_, DiagnosticKey)] = &[
        (&results.adapters, Adapters),
        (&results.interfaces, Interfaces),
        (&results.gateway, Gateway),
        (&results.dns, Dns),
        (&results.public_ip, PublicIp),
        (&results.latency, Latency),
        (&results.ports, Ports),
        (&results.speed, Speed),
    ];

    for (res, key) in pairs.iter() {
        match res.status {
            DiagnosticStatus::Fail => {
                if matches!(key, DiagnosticKey::Latency) {
                    // Public ICMP/UDP latency probes are commonly filtered
                    // even when HTTP/DNS/gateway connectivity is healthy.
                    continue;
                }
                out.insert(*key);
            }
            DiagnosticStatus::Warn => {
                // Warn-level findings are advisory in fix mode. Mutating a
                // healthy connection for moderate latency or a partial port
                // block is more likely to make a user's machine worse.
            }
            _ => {}
        }
    }

    out
}

/// Hardcoded dependency DAG between diagnostic categories. If a parent
/// category fails, a failing child rooted to it is suppressed for the current
/// iteration — fixing the parent typically cascades.
///
/// Order: parent first, then children that depend on it being healthy.
fn parents_of(key: DiagnosticKey) -> &'static [DiagnosticKey] {
    use DiagnosticKey::*;
    match key {
        Adapters => &[],
        Interfaces => &[Adapters],
        Gateway => &[Interfaces, Adapters],
        Dns => &[Gateway, Interfaces, Adapters],
        PublicIp => &[Gateway, Interfaces, Adapters],
        Latency => &[Gateway, Interfaces, Adapters],
        Ports => &[Gateway, Interfaces, Adapters],
        Speed => &[PublicIp, Gateway, Interfaces, Adapters],
    }
}

/// Group failures by their root cause. When a child failure has any failing
/// ancestor in the DAG, suppress the child this iteration — fixing the
/// ancestor typically cascades. The returned set contains only the root-most
/// failures the loop should target right now.
pub fn group_by_root_cause(failures: &HashSet<DiagnosticKey>) -> HashSet<DiagnosticKey> {
    failures
        .iter()
        .copied()
        .filter(|k| !parents_of(*k).iter().any(|p| failures.contains(p)))
        .collect()
}

/// Detect failure patterns where applying actions would only make things
/// worse. Returning `Some(_)` short-circuits the loop with a guidance message.
pub fn hard_block_detected(results: &DiagnosticResults) -> Option<HardBlock> {
    use DiagnosticStatus::*;

    // No active interfaces at all → no physical link.
    let any_active = matches!(results.adapters.status, Ok | Warn)
        || matches!(results.interfaces.status, Ok | Warn);
    if !any_active
        && matches!(results.adapters.status, Fail)
        && matches!(results.interfaces.status, Fail)
    {
        return Some(HardBlock::NoPhysicalLink);
    }

    // Enterprise VPN signal: the public-IP diagnostic carries a VPN-detection
    // hint in its summary text. Scan for known enterprise vendor names.
    let summary = results.public_ip.summary.to_lowercase();
    for vendor in &[
        "cisco anyconnect",
        "zscaler",
        "palo alto",
        "globalprotect",
        "f5 networks",
        "checkpoint",
        "juniper",
    ] {
        if summary.contains(vendor) {
            return Some(HardBlock::EnterpriseVpnActive((*vendor).to_string()));
        }
    }

    // ISP-side outage shape: gateway healthy, DNS healthy, but public IP
    // unreachable AND ports blocked. Network is fine; route to the wider
    // internet is broken.
    if matches!(results.gateway.status, Ok)
        && matches!(results.dns.status, Ok)
        && matches!(results.public_ip.status, Fail)
        && matches!(results.ports.status, Fail)
    {
        return Some(HardBlock::IspOutage);
    }

    None
}

fn action_stage(action: &Action, attempts: &Attempts) -> u8 {
    use ActionId::*;

    match action.id {
        FlushDns | FlushArp => {
            if attempts.get(&action.id).copied().unwrap_or(0) == 0 {
                0
            } else {
                4
            }
        }
        SetDnsAutomatic => {
            if attempts.get(&FlushDns).copied().unwrap_or(0) > 0 {
                1
            } else {
                2
            }
        }
        SetDnsCloudflare => {
            if attempts.get(&SetDnsAutomatic).copied().unwrap_or(0) > 0 {
                2
            } else {
                3
            }
        }
        RestartNetworkServices | RenewDhcp | DisableConsumerVpns => 3,
        BounceInterface => 4,
        DeepStackReset => 5,
    }
}

/// Build the next iteration's plan — an ordered `Vec<Action>` to attempt.
///
/// 1. Restrict attention to the root-cause group of currently-failing categories.
/// 2. For each candidate action: must target at least one failure in the group,
///    and must have remaining attempts under its `max_attempts`.
/// 3. Sort by `(cost rank, risk rank, novelty)`. Within the same cost+risk,
///    prefer actions that have already shown improvement this session.
pub fn build_plan(
    failures: &HashSet<DiagnosticKey>,
    attempts: &Attempts,
    effectiveness: &Effectiveness,
    registry: &[Action],
) -> Vec<Action> {
    let group = group_by_root_cause(failures);
    if group.is_empty() {
        return Vec::new();
    }

    let mut candidates: Vec<&Action> = registry
        .iter()
        .filter(|a| {
            let used = attempts.get(&a.id).copied().unwrap_or(0);
            used < a.max_attempts
        })
        .filter(|a| a.targets.iter().any(|t| group.contains(t)))
        .collect();

    let min_stage = candidates
        .iter()
        .map(|a| action_stage(a, attempts))
        .min()
        .unwrap_or(0);
    candidates.retain(|a| action_stage(a, attempts) == min_stage);

    candidates.sort_by(|a, b| {
        let ca = a.cost.rank();
        let cb = b.cost.rank();
        if ca != cb {
            return ca.cmp(&cb);
        }
        let ra = a.risk.rank();
        let rb = b.risk.rank();
        if ra != rb {
            return ra.cmp(&rb);
        }
        // Tie-break: prefer actions that have helped in this session.
        let ea = action_helpfulness(a, &group, effectiveness);
        let eb = action_helpfulness(b, &group, effectiveness);
        // Higher helpfulness should sort first.
        eb.cmp(&ea)
    });

    candidates.into_iter().cloned().collect()
}

fn action_helpfulness(
    action: &Action,
    group: &HashSet<DiagnosticKey>,
    effectiveness: &Effectiveness,
) -> u8 {
    let mut score = 0u8;
    for k in action.targets {
        if !group.contains(k) {
            continue;
        }
        if let Some(true) = effectiveness.get(&(action.id, *k)) {
            score = score.saturating_add(1);
        }
    }
    score
}

/// Whether the given action requires an interactive Y/N before applying.
/// High-risk actions always do; lower-risk actions never do.
pub fn requires_high_risk_consent(action: &Action) -> bool {
    matches!(action.risk, Risk::High(_))
}

/// Whether an action needs a Y/N gate before it mutates network state.
/// High-risk actions always require explicit consent. Medium/expensive and
/// DNS-changing actions can be auto-confirmed by `--yes`.
pub fn requires_confirmation(action: &Action, auto_confirm_medium_risk: bool) -> bool {
    if action.risk.is_high() {
        return true;
    }
    if auto_confirm_medium_risk {
        return false;
    }

    matches!(
        action.id,
        ActionId::SetDnsCloudflare
            | ActionId::SetDnsAutomatic
            | ActionId::RestartNetworkServices
            | ActionId::RenewDhcp
            | ActionId::DisableConsumerVpns
            | ActionId::BounceInterface
    )
}

/// Limit on the number of distinct loop iterations. Combined with per-action
/// `max_attempts` and the wall-clock cap in [`super::session::WALL_CLOCK_CAP`],
/// guarantees termination.
pub const MAX_ITERATIONS: u8 = 6;

// ── unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::action::Cost;
    use super::*;
    use crate::diagnostics::{DiagnosticResult, DiagnosticResults};

    fn empty_results() -> DiagnosticResults {
        DiagnosticResults {
            timestamp: "test".to_string(),
            adapters: DiagnosticResult::ok("Adapters", "1 active"),
            interfaces: DiagnosticResult::ok("Interfaces", "1 up"),
            gateway: DiagnosticResult::ok("Gateway", "192.168.1.1 reachable"),
            dns: DiagnosticResult::ok("DNS", "resolving"),
            public_ip: DiagnosticResult::ok("Public IP", "203.0.113.1"),
            latency: DiagnosticResult::ok("Latency", "20ms"),
            speed: DiagnosticResult::skip("Speed", "skipped"),
            ports: DiagnosticResult::ok("Ports", "443 open"),
            interface_details: None,
            adapter_details: None,
            gateway_details: None,
            dns_details: None,
            public_ip_details: None,
            latency_details: None,
            speed_details: None,
            port_details: None,
            technician: None,
        }
    }

    fn fail(category: &str, summary: &str) -> DiagnosticResult {
        DiagnosticResult::fail(category, summary)
    }

    #[test]
    fn no_failures_yields_empty_plan() {
        let r = empty_results();
        assert!(actionable_failures(&r).is_empty());

        let registry = super::super::action::all_actions();
        let plan = build_plan(
            &actionable_failures(&r),
            &Attempts::new(),
            &Effectiveness::new(),
            &registry,
        );
        assert!(plan.is_empty(), "expected empty plan; got {:?}", plan);
    }

    #[test]
    fn dns_only_failure_picks_dns_actions_in_cost_order() {
        let mut r = empty_results();
        r.dns = fail("DNS", "resolution failed");

        let failures = actionable_failures(&r);
        assert_eq!(failures.len(), 1);
        assert!(failures.contains(&DiagnosticKey::Dns));

        let registry = super::super::action::all_actions();
        let plan = build_plan(
            &failures,
            &Attempts::new(),
            &Effectiveness::new(),
            &registry,
        );

        assert!(!plan.is_empty());
        // First action should be Cheap-Low.
        let first = &plan[0];
        assert_eq!(first.cost.rank(), Cost::Cheap.rank());
        assert!(matches!(first.risk, Risk::Low));
        // Every action targets DNS.
        for a in &plan {
            assert!(
                a.targets.contains(&DiagnosticKey::Dns),
                "action {:?} doesn't target DNS",
                a.id
            );
        }
    }

    #[test]
    fn interface_down_cluster_is_grouped_to_root() {
        let mut r = empty_results();
        r.adapters = fail("Adapters", "no active adapter");
        r.interfaces = fail("Interfaces", "no interfaces up");
        r.gateway = fail("Gateway", "unreachable");
        r.dns = fail("DNS", "resolution failed");
        r.public_ip = fail("Public IP", "timeout");

        let failures = actionable_failures(&r);
        let grouped = group_by_root_cause(&failures);

        assert!(grouped.contains(&DiagnosticKey::Adapters));
        assert!(
            !grouped.contains(&DiagnosticKey::Dns),
            "DNS should be suppressed"
        );
        assert!(
            !grouped.contains(&DiagnosticKey::Gateway),
            "Gateway should be suppressed under Adapters"
        );
    }

    #[test]
    fn max_attempts_excludes_used_actions() {
        let mut r = empty_results();
        r.dns = fail("DNS", "failed");

        let registry = super::super::action::all_actions();
        let mut attempts = Attempts::new();
        attempts.insert(ActionId::FlushDns, 99);

        let plan = build_plan(
            &actionable_failures(&r),
            &attempts,
            &Effectiveness::new(),
            &registry,
        );

        assert!(plan.iter().all(|a| a.id != ActionId::FlushDns));
    }

    #[test]
    fn isp_outage_shape_returns_hard_block() {
        let mut r = empty_results();
        r.public_ip = fail("Public IP", "timeout");
        r.ports = fail("Ports", "all blocked");
        // gateway + dns stay Ok.

        let block = hard_block_detected(&r);
        assert_eq!(block, Some(HardBlock::IspOutage));
    }

    #[test]
    fn captive_portal_marker_in_public_ip_summary() {
        // hard_block_detected for now only catches enterprise VPN strings and
        // ISP outage shape; captive portal detection lives at the loop level
        // (where it has access to a fresh HTTP probe). This test pins the
        // current behavior: pure summary-string check should NOT misfire.
        let mut r = empty_results();
        r.public_ip = fail("Public IP", "timeout");
        let block = hard_block_detected(&r);
        // Not an ISP outage (gateway+dns Ok, ports Ok), no enterprise VPN.
        assert_eq!(block, None);
    }

    #[test]
    fn enterprise_vpn_marker_detected() {
        let mut r = empty_results();
        r.public_ip = DiagnosticResult::warn("Public IP", "Detected via Cisco AnyConnect adapter");
        let block = hard_block_detected(&r);
        assert!(matches!(block, Some(HardBlock::EnterpriseVpnActive(_))));
    }

    #[test]
    fn effectiveness_breaks_ties_within_same_cost_risk() {
        let mut r = empty_results();
        r.dns = fail("DNS", "failed");

        let registry = super::super::action::all_actions();
        let mut effectiveness = Effectiveness::new();
        // Mark SetDnsCloudflare as helpful in this session.
        effectiveness.insert((ActionId::SetDnsCloudflare, DiagnosticKey::Dns), true);

        let plan = build_plan(
            &actionable_failures(&r),
            &Attempts::new(),
            &effectiveness,
            &registry,
        );

        // Among the cost::Cheap / risk::Low DNS actions, the one with proven
        // effectiveness should rank ahead of an untracked peer.
        let cheap_low: Vec<&Action> = plan
            .iter()
            .filter(|a| a.cost == Cost::Cheap && matches!(a.risk, Risk::Low))
            .collect();
        let cf_pos = cheap_low
            .iter()
            .position(|a| a.id == ActionId::SetDnsCloudflare);
        let other_pos = cheap_low
            .iter()
            .position(|a| a.id == ActionId::SetDnsAutomatic);
        if let (Some(cf), Some(other)) = (cf_pos, other_pos) {
            assert!(
                cf < other,
                "expected SetDnsCloudflare ahead of SetDnsAutomatic when marked helpful"
            );
        }
    }

    #[test]
    fn latency_warn_is_advisory_not_actionable() {
        let mut r = empty_results();
        r.latency = DiagnosticResult::warn("Latency", "Moderate latency (~125ms avg)");

        let failures = actionable_failures(&r);
        assert!(
            failures.is_empty(),
            "latency warning should not cause mutating fix actions: {:?}",
            failures
        );
    }

    #[test]
    fn latency_fail_is_advisory_when_other_connectivity_passes() {
        let mut r = empty_results();
        r.latency = fail("Latency", "All endpoints unreachable");

        let failures = actionable_failures(&r);
        assert!(
            failures.is_empty(),
            "ICMP-only latency failure should not mutate an otherwise working network: {:?}",
            failures
        );
    }

    #[test]
    fn dns_failure_starts_with_cache_flush_only() {
        let mut r = empty_results();
        r.dns = fail("DNS", "resolution failed");

        let registry = super::super::action::all_actions();
        let plan = build_plan(
            &actionable_failures(&r),
            &Attempts::new(),
            &Effectiveness::new(),
            &registry,
        );

        let ids: Vec<ActionId> = plan.iter().map(|a| a.id).collect();
        assert_eq!(ids, vec![ActionId::FlushDns]);
    }

    #[test]
    fn dns_failure_progresses_to_automatic_before_public_dns() {
        let mut r = empty_results();
        r.dns = fail("DNS", "resolution failed");

        let registry = super::super::action::all_actions();
        let mut attempts = Attempts::new();
        attempts.insert(ActionId::FlushDns, 1);

        let plan = build_plan(
            &actionable_failures(&r),
            &attempts,
            &Effectiveness::new(),
            &registry,
        );

        let ids: Vec<ActionId> = plan.iter().map(|a| a.id).collect();
        assert_eq!(ids, vec![ActionId::SetDnsAutomatic]);
    }

    #[test]
    fn dns_failure_uses_public_dns_only_after_automatic_dns_fails() {
        let mut r = empty_results();
        r.dns = fail("DNS", "resolution failed");

        let registry = super::super::action::all_actions();
        let mut attempts = Attempts::new();
        attempts.insert(ActionId::FlushDns, 1);
        attempts.insert(ActionId::SetDnsAutomatic, 1);

        let plan = build_plan(
            &actionable_failures(&r),
            &attempts,
            &Effectiveness::new(),
            &registry,
        );

        let ids: Vec<ActionId> = plan.iter().map(|a| a.id).collect();
        assert_eq!(ids, vec![ActionId::SetDnsCloudflare]);
    }

    #[test]
    fn medium_risk_actions_need_confirmation_without_auto_confirm() {
        let registry = super::super::action::all_actions();
        let renew = registry
            .iter()
            .find(|a| a.id == ActionId::RenewDhcp)
            .expect("renew action exists");
        let restart = registry
            .iter()
            .find(|a| a.id == ActionId::RestartNetworkServices)
            .expect("restart action exists");
        let vpn = registry
            .iter()
            .find(|a| a.id == ActionId::DisableConsumerVpns)
            .expect("vpn action exists");

        assert!(requires_confirmation(renew, false));
        assert!(requires_confirmation(restart, false));
        assert!(requires_confirmation(vpn, false));
        assert!(!requires_confirmation(renew, true));
        assert!(!requires_confirmation(restart, true));
        assert!(!requires_confirmation(vpn, true));
    }
}
