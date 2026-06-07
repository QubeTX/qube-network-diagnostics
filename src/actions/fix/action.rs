//! Action type system and registry for the diagnostic-driven fix loop.
//!
//! Every fix primitive is wrapped as an [`Action`]. The triage layer picks
//! actions whose `targets` intersect the current failure set; the loop runner
//! calls `Action::apply` and records the [`ActionOutcome`].

use std::time::Duration;

use crate::actions::flush_dns_platform;
use crate::config::Config;

use super::adapters;
use super::arp;
use super::cmd::CmdOutcome;
use super::dhcp;
use super::dns::{self, DnsProvider};
use super::session::{RestoreOp, RestoreRegistry};
use super::stages;
use super::vpn;

/// Identifier for one of the user-mode core diagnostics. An [`Action`]'s
/// `targets` field declares which of these failure categories it can address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticKey {
    Adapters,
    Interfaces,
    Gateway,
    Dns,
    PublicIp,
    Latency,
    Ports,
    Speed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cost {
    Cheap,
    Medium,
    Expensive,
}

impl Cost {
    pub fn rank(self) -> u8 {
        match self {
            Cost::Cheap => 0,
            Cost::Medium => 1,
            Cost::Expensive => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Risk {
    Low,
    Medium,
    /// High-risk actions REQUIRE an attached [`RiskExplanation`] by
    /// construction. There is no way to reach the loop's apply path on a
    /// High-risk action without a complete plain-language explanation that the
    /// user can read before approving.
    High(RiskExplanation),
}

impl Risk {
    pub fn is_high(&self) -> bool {
        matches!(self, Risk::High(_))
    }
    pub fn rank(&self) -> u8 {
        match self {
            Risk::Low => 0,
            Risk::Medium => 1,
            Risk::High(_) => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Reversibility {
    /// Fully reversible without user action.
    FullyReversible,
    /// Requires a reboot to fully revert (e.g. Winsock reset).
    RebootToFullyRevert,
    /// Cannot be undone (e.g. delete Wi-Fi profile — passphrase is gone).
    NotReversible,
}

impl Reversibility {
    pub fn label(self) -> &'static str {
        match self {
            Reversibility::FullyReversible => "fully reversible",
            Reversibility::RebootToFullyRevert => "requires reboot to fully revert",
            Reversibility::NotReversible => "not reversible",
        }
    }
}

/// Plain-English explanation rendered before any High-risk action runs. Every
/// field is mandatory — there is no way to construct a High-risk
/// [`Action`] without a complete explanation.
#[derive(Debug, Clone)]
pub struct RiskExplanation {
    /// One-line headline of what the action does (e.g. "Reset Windows
    /// networking stack").
    pub what: &'static str,
    /// 1–3 sentence explanation of why this action exists and what kind of
    /// problem it fixes. Written for non-technical readers.
    pub why: &'static str,
    /// Concrete consequences the user should expect, one per bullet.
    pub side_effects: &'static [&'static str],
    /// How recoverable the action is.
    pub reversible: Reversibility,
    /// Human-readable estimate of how long the user will be disrupted.
    pub typical_duration: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionId {
    FlushDns,
    SetDnsCloudflare,
    SetDnsAutomatic,
    FlushArp,
    RestartNetworkServices,
    RenewDhcp,
    DisableConsumerVpns,
    BounceInterface,
    /// Platform-specific deep stack reset (Windows: Winsock+TCPIP+IPv6; macOS:
    /// remove/recreate network service; Linux: nmcli connection
    /// delete/recreate).
    DeepStackReset,
}

#[derive(Debug, Clone)]
pub struct Action {
    pub id: ActionId,
    /// Short label used in status lines (e.g. "Flush the DNS cache"). Should
    /// read naturally to a non-technical user.
    pub label: &'static str,
    /// One-sentence "why I'm trying this" rendered before the action runs.
    pub one_line_why: &'static str,
    /// Failure categories this action can address. Used by triage to match
    /// actions to current failures.
    pub targets: &'static [DiagnosticKey],
    pub cost: Cost,
    pub risk: Risk,
    /// Per-session attempt cap. Once reached, the action is removed from
    /// future plans even if its targets keep failing — prevents the loop from
    /// retrying a step that didn't help.
    pub max_attempts: u8,
    /// Wait after applying this action before the next iteration's
    /// diagnostics run. DHCP renew and interface bounce need ~10s to settle;
    /// a DNS flush only ~1s.
    pub stabilization: Duration,
}

#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub ok: bool,
    pub message: String,
    /// Captured subprocess invocations from inside this action. Currently
    /// populated only by actions that build their own commands; legacy
    /// helpers that return `Result<String, String>` leave this empty. The
    /// `message` field always carries a human-readable summary either way.
    pub cmd_outcomes: Vec<CmdOutcome>,
    /// Set by actions that change the network surface enough that the loop
    /// should re-probe immediately rather than apply additional actions in
    /// the same iteration (e.g. interface bounce, deep stack reset).
    pub fatal_environment_change: bool,
}

impl ActionOutcome {
    pub fn ok(msg: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: msg.into(),
            cmd_outcomes: Vec::new(),
            fatal_environment_change: false,
        }
    }
    pub fn fail(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: msg.into(),
            cmd_outcomes: Vec::new(),
            fatal_environment_change: false,
        }
    }
    pub fn from_result(r: Result<String, String>) -> Self {
        match r {
            Ok(msg) => Self::ok(msg),
            Err(msg) => Self::fail(msg),
        }
    }
    pub fn with_fatal_env_change(mut self) -> Self {
        self.fatal_environment_change = true;
        self
    }
}

impl Action {
    /// Run this action against the current system. Always returns an
    /// `ActionOutcome` — apply failures are encoded inside it, never
    /// propagated as Rust errors.
    ///
    /// `restore` is the run's [`RestoreRegistry`]: destructive actions
    /// (VPN disable, interface bounce, deep stack reset) register the inverse
    /// op here *before* mutating state, so any abort (Ctrl-C / timeout / panic)
    /// can roll the change back. Non-destructive actions (flush / DNS / DHCP /
    /// service restart) ignore it.
    pub async fn apply(&self, config: &Config, restore: &RestoreRegistry) -> ActionOutcome {
        match self.id {
            ActionId::FlushDns => apply_flush_dns().await,
            ActionId::SetDnsCloudflare => apply_set_dns(DnsProvider::Cloudflare).await,
            ActionId::SetDnsAutomatic => apply_set_dns(DnsProvider::Automatic).await,
            ActionId::FlushArp => apply_flush_arp().await,
            ActionId::RestartNetworkServices => apply_restart_services().await,
            ActionId::RenewDhcp => apply_renew_dhcp().await,
            ActionId::DisableConsumerVpns => apply_disable_consumer_vpns(config, restore).await,
            ActionId::BounceInterface => apply_bounce_interface(restore).await,
            ActionId::DeepStackReset => apply_deep_stack_reset(config, restore).await,
        }
    }
}

// ── Apply implementations ───────────────────────────────────────────────────

async fn apply_flush_dns() -> ActionOutcome {
    ActionOutcome::from_result(flush_dns_platform().await)
}

async fn apply_set_dns(provider: DnsProvider) -> ActionOutcome {
    let iface = match adapters::detect_default_interface().await {
        Some(i) => i,
        None => return ActionOutcome::fail("Could not detect a default network interface"),
    };
    let service_name = service_name_for(&iface).await;
    ActionOutcome::from_result(dns::set_dns_servers(&iface, &service_name, provider).await)
}

async fn apply_flush_arp() -> ActionOutcome {
    ActionOutcome::from_result(arp::flush_arp().await)
}

async fn apply_restart_services() -> ActionOutcome {
    ActionOutcome::from_result(stages::restart_services().await)
}

async fn apply_renew_dhcp() -> ActionOutcome {
    if let Some(iface) = adapters::detect_default_interface().await {
        ActionOutcome::from_result(adapters::renew_dhcp_on_interface(&iface).await)
    } else {
        ActionOutcome::from_result(dhcp::renew_dhcp().await)
    }
}

async fn apply_disable_consumer_vpns(config: &Config, restore: &RestoreRegistry) -> ActionOutcome {
    if !crate::actions::is_interactive(config) {
        return ActionOutcome::fail(
            "Skipped: disabling VPNs requires an interactive session so they can be re-enabled safely.",
        );
    }

    let disabled = vpn::detect_and_disable(config).await;
    if disabled.is_empty() {
        return ActionOutcome::ok("No consumer VPNs were active");
    }

    // Register a re-enable op for every VPN we disabled BEFORE we report
    // success. If the run is interrupted (Ctrl-C / timeout / panic) before the
    // normal-path offer below runs, the drain re-connects each still-registered
    // VPN so the user isn't left disconnected.
    let mut tokens = Vec::with_capacity(disabled.len());
    for v in &disabled {
        let token = restore
            .register(RestoreOp::ReEnableVpn(std::sync::Arc::new(v.clone())))
            .await;
        tokens.push(token);
    }

    let names: Vec<String> = disabled.iter().map(|v| v.name.clone()).collect();

    // Normal path: offer to re-enable now (default No keeps them off for the
    // re-probe). Whatever the user chooses, the offer has happened, so mark the
    // restore ops resolved — the drain must not blindly re-enable them on a
    // normal terminal path (that would undo a successful fix).
    vpn::offer_reenable(&disabled, config).await;
    for token in tokens {
        restore.mark_resolved(token).await;
    }

    ActionOutcome::ok(format!("Disabled consumer VPNs: {}", names.join(", ")))
        .with_fatal_env_change()
}

async fn apply_bounce_interface(restore: &RestoreRegistry) -> ActionOutcome {
    let iface = match adapters::detect_default_interface().await {
        Some(i) => i,
        None => return ActionOutcome::fail("Could not detect a default network interface"),
    };

    // Register the re-enable BEFORE disabling, so an interrupt between disable
    // and re-enable still brings the adapter back up via the drain.
    let token = restore
        .register(RestoreOp::ReEnableInterface {
            iface: iface.clone(),
        })
        .await;

    if let Err(e) = stages::disable_interface(&iface).await {
        // Disable never happened — nothing to restore.
        restore.mark_resolved(token).await;
        return ActionOutcome::fail(format!("Disable {} failed: {}", iface, e));
    }

    tokio::time::sleep(Duration::from_secs(3)).await;

    // Re-enable with one retry. Leaving an adapter disabled is far worse than a
    // slow retry, so mirror the legacy 2s-wait retry that the old Stage 2 had.
    if let Err(first_err) = stages::enable_interface(&iface).await {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Err(retry_err) = stages::enable_interface(&iface).await {
            // Still down. Leave the restore op REGISTERED so the drain retries
            // it on the terminal path, and surface a loud, actionable message.
            let cmd_hint = reenable_command_hint(&iface);
            return ActionOutcome::fail(format!(
                "Your network adapter \"{}\" is still DISABLED — re-enable failed twice ({}; retry: {}). \
                 nd300 will try again as it exits. If you still have no connection, run: {}",
                iface, first_err, retry_err, cmd_hint
            ))
            .with_fatal_env_change();
        }
    }

    // Adapter is back up — the action restored it itself.
    restore.mark_resolved(token).await;
    ActionOutcome::ok(format!("{} bounced (disable → 3s wait → re-enable)", iface))
        .with_fatal_env_change()
}

/// Platform-specific manual command to bring an interface back up, for the
/// loud failure message when an automated re-enable fails twice.
fn reenable_command_hint(iface: &str) -> String {
    #[cfg(windows)]
    {
        format!("netsh interface set interface \"{}\" enabled", iface)
    }
    #[cfg(target_os = "macos")]
    {
        format!(
            "networksetup -setairportpower {} on  (Wi-Fi)  or  ifconfig {} up  (wired)",
            iface, iface
        )
    }
    #[cfg(target_os = "linux")]
    {
        format!("sudo ip link set {} up", iface)
    }
}

async fn apply_deep_stack_reset(config: &Config, restore: &RestoreRegistry) -> ActionOutcome {
    let saved_ssid = super::wifi::capture_current_ssid().await;
    match stages::platform_stage3(config, &saved_ssid, restore).await {
        Ok(steps) => {
            if steps.is_empty() {
                ActionOutcome::fail("Stack reset attempted but no steps succeeded")
                    .with_fatal_env_change()
            } else {
                ActionOutcome::ok(format!("Stack reset: {}", steps.join("; ")))
                    .with_fatal_env_change()
            }
        }
        Err(e) => ActionOutcome::fail(e).with_fatal_env_change(),
    }
}

#[cfg(target_os = "macos")]
async fn service_name_for(iface: &str) -> String {
    // For macOS, fall back to the iface name itself if the lookup fails —
    // networksetup will reject unknown service names cleanly.
    if let Some(svc) = stages::detect_macos_service(iface).await {
        svc
    } else {
        iface.to_string()
    }
}

#[cfg(not(target_os = "macos"))]
async fn service_name_for(iface: &str) -> String {
    iface.to_string()
}

// ── Registry ────────────────────────────────────────────────────────────────

/// Returns every [`Action`] available on the current platform. Order is not
/// significant — [`super::triage::build_plan`] sorts by cost / risk /
/// effectiveness when assembling each iteration's plan.
pub fn all_actions() -> Vec<Action> {
    let mut actions = vec![
        Action {
            id: ActionId::FlushDns,
            label: "Flush the DNS cache",
            one_line_why: "Clears stale DNS records that often cause resolution failures.",
            targets: &[DiagnosticKey::Dns],
            cost: Cost::Cheap,
            risk: Risk::Low,
            max_attempts: 2,
            stabilization: Duration::from_secs(1),
        },
        Action {
            id: ActionId::SetDnsCloudflare,
            label: "Switch DNS to Cloudflare (1.1.1.1)",
            one_line_why: "Bypasses a broken or filtered DNS server provided by your network.",
            targets: &[DiagnosticKey::Dns],
            cost: Cost::Cheap,
            risk: Risk::Low,
            max_attempts: 1,
            stabilization: Duration::from_secs(2),
        },
        Action {
            id: ActionId::SetDnsAutomatic,
            label: "Reset DNS to your router's defaults (DHCP)",
            one_line_why: "Removes any custom DNS servers and lets your router choose.",
            targets: &[DiagnosticKey::Dns],
            cost: Cost::Cheap,
            risk: Risk::Low,
            max_attempts: 1,
            stabilization: Duration::from_secs(2),
        },
        Action {
            id: ActionId::FlushArp,
            label: "Flush the ARP cache",
            one_line_why: "Clears stale gateway entries that block traffic to your router.",
            targets: &[DiagnosticKey::Gateway, DiagnosticKey::Latency],
            cost: Cost::Cheap,
            risk: Risk::Low,
            max_attempts: 1,
            stabilization: Duration::from_secs(1),
        },
        Action {
            id: ActionId::RestartNetworkServices,
            label: "Restart networking services",
            one_line_why: "Brings the OS-level DNS / DHCP services back to a clean state.",
            targets: &[
                DiagnosticKey::Dns,
                DiagnosticKey::Gateway,
                DiagnosticKey::PublicIp,
            ],
            cost: Cost::Medium,
            risk: Risk::Low,
            max_attempts: 1,
            stabilization: Duration::from_secs(3),
        },
        Action {
            id: ActionId::RenewDhcp,
            label: "Renew the DHCP lease",
            one_line_why: "Asks your router for a fresh IP address and gateway.",
            targets: &[
                DiagnosticKey::Gateway,
                DiagnosticKey::PublicIp,
                DiagnosticKey::Adapters,
                DiagnosticKey::Interfaces,
            ],
            cost: Cost::Medium,
            risk: Risk::Low,
            max_attempts: 1,
            stabilization: Duration::from_secs(8),
        },
        Action {
            id: ActionId::DisableConsumerVpns,
            label: "Temporarily disable consumer VPNs",
            one_line_why: "Some consumer VPNs (NordVPN, ExpressVPN, Tailscale, etc.) interfere with diagnostics. Enterprise VPNs are never auto-disabled.",
            targets: &[
                DiagnosticKey::PublicIp,
                DiagnosticKey::Latency,
                DiagnosticKey::Dns,
            ],
            cost: Cost::Medium,
            risk: Risk::Medium,
            max_attempts: 1,
            stabilization: Duration::from_secs(2),
        },
        Action {
            id: ActionId::BounceInterface,
            label: "Restart your network adapter (disable → re-enable)",
            one_line_why: "Forces the adapter to reset its link, re-associate Wi-Fi, and re-DHCP.",
            targets: &[
                DiagnosticKey::Adapters,
                DiagnosticKey::Interfaces,
                DiagnosticKey::Gateway,
                DiagnosticKey::Dns,
                DiagnosticKey::PublicIp,
                DiagnosticKey::Latency,
            ],
            cost: Cost::Expensive,
            risk: Risk::Medium,
            max_attempts: 1,
            stabilization: Duration::from_secs(10),
        },
    ];

    // Deep stack reset — High risk, requires Y/N. Wording is per-platform.
    let deep_reset_explanation = make_deep_reset_explanation();
    actions.push(Action {
        id: ActionId::DeepStackReset,
        label: deep_reset_explanation.what,
        one_line_why: "Last-resort recovery when nothing else worked.",
        targets: &[
            DiagnosticKey::Dns,
            DiagnosticKey::Gateway,
            DiagnosticKey::PublicIp,
            DiagnosticKey::Adapters,
            DiagnosticKey::Interfaces,
        ],
        cost: Cost::Expensive,
        risk: Risk::High(deep_reset_explanation),
        max_attempts: 1,
        stabilization: Duration::from_secs(15),
    });

    actions
}

#[cfg(windows)]
fn make_deep_reset_explanation() -> RiskExplanation {
    RiskExplanation {
        what: "Reset Windows networking stack",
        why: "This rebuilds Windows' TCP/IP, Winsock, and IPv6 catalogs from scratch — the standard fix when simpler steps haven't recovered the connection.",
        side_effects: &[
            "You will lose internet for ~10–15 seconds.",
            "Open VPN sessions and SSH connections will drop.",
            "A reboot is recommended afterward; nd300 will remind you at the end.",
        ],
        reversible: Reversibility::RebootToFullyRevert,
        typical_duration: "10–15 seconds",
    }
}

#[cfg(target_os = "macos")]
fn make_deep_reset_explanation() -> RiskExplanation {
    RiskExplanation {
        what: "Recreate the macOS network service",
        why: "This removes and recreates your active network service in System Settings — the standard fix when a service entry is corrupted.",
        side_effects: &[
            "You will lose internet for ~10–20 seconds.",
            "Wi-Fi will need to reconnect; nd300 will try to restore it from Keychain.",
            "nd300 snapshots and attempts to restore DNS, proxy, service order, and IP mode settings.",
        ],
        reversible: Reversibility::NotReversible,
        typical_duration: "10–20 seconds",
    }
}

#[cfg(target_os = "linux")]
fn make_deep_reset_explanation() -> RiskExplanation {
    RiskExplanation {
        what: "Recreate the NetworkManager connection profile",
        why: "This deletes and recreates the active NetworkManager connection profile — the standard fix when a profile is corrupted.",
        side_effects: &[
            "You will lose internet briefly.",
            "Saved settings on the deleted profile are gone.",
            "For Wi-Fi, you'll need to provide the SSID and passphrase again.",
        ],
        reversible: Reversibility::NotReversible,
        typical_duration: "10–20 seconds",
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    use super::*;

    #[tokio::test]
    async fn json_mode_does_not_disable_consumer_vpns() {
        // The JSON path early-returns before touching the registry, so a fresh
        // empty registry is sufficient and the assertion is unchanged.
        let outcome =
            apply_disable_consumer_vpns(&Config::new().with_json(), &RestoreRegistry::new()).await;

        assert!(!outcome.ok);
        assert!(
            outcome.message.contains("requires an interactive session"),
            "unexpected outcome: {:?}",
            outcome
        );
    }
}
