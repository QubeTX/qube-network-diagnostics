pub(crate) mod adaptive;
pub mod applenq;
pub mod cachefly;
pub mod cloudflare;
pub mod display;
pub mod fastcom;
pub mod librespeed;
pub mod msak;
pub mod ndt7;
pub mod stat_primitives;
pub mod statistics;
pub mod vultr;

#[cfg(test)]
mod golden_tests;

use serde::Serialize;
use stat_primitives::Pcg32;
use statistics::{merge_providers, BcaInterval, MergeProviderInput};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

/// SpeedQX methodology version stamped into every result payload
/// (METHODOLOGY.md §1). Byte-identical to the website/app implementations.
pub const METHODOLOGY_VERSION: &str = "4.0";

/// Per-provider hard cap in FAST mode (METHODOLOGY.md §8): a provider that
/// hasn't converged reports what it has; a stuck one is abandoned. This is the
/// outer safety net (`run_provider_future`); providers whose runtime is
/// RTT-scaled (Cloudflare's dense-latency engine) additionally self-bound a
/// margin below this so they return real partial data instead of being killed
/// and discarded here.
pub(crate) const FAST_HARD_CAP: Duration = Duration::from_secs(25);

/// Short per-direction budget for FAST-mode providers. Cloudflare's RTT-scaled
/// dense-latency engine can still push a single provider toward [`FAST_HARD_CAP`]
/// on high-RTT links, so `cloudflare::run` additionally caps its whole FAST run
/// against an internal soft deadline (a margin below `FAST_HARD_CAP`) and
/// returns the data it has gathered, rather than being killed by the outer
/// timeout and discarded (§8 "reports what it has"). The empirical-Bernstein
/// confidence sequence (§8) trims the transfer phases further per provider.
const FAST_PER_DIR_SECS: u64 = 8;

/// Dense idle-latency probe count (METHODOLOGY.md §4):
/// `clamp(50, round(durationSeconds × 3.3), 200)` — auto (≈15 s) → 50, 30 s → 99.
pub fn dense_probe_count(duration_secs: f64) -> u32 {
    (duration_secs * 3.3).round().clamp(50.0, 200.0) as u32
}

/// Inter-probe interval for the dense latency engine (METHODOLOGY.md §4).
pub const DENSE_PROBE_INTERVAL: Duration = Duration::from_millis(50);

/// Warm-up probes discarded by the dense latency engine (DNS + TCP + TLS
/// amortization), METHODOLOGY.md §4.
pub const DENSE_PROBE_WARMUP: usize = 3;

/// Test duration configuration
#[derive(Debug, Clone)]
pub enum TestDuration {
    /// Fixed duration per direction in seconds (e.g., 30 = 30s download + 30s upload)
    Seconds(u64),
    /// Let providers use their natural duration
    Auto,
}

/// Which providers to run (METHODOLOGY.md §3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProviderSet {
    /// FULL: every provider — Cloudflare, NDT7, MSAK, LibreSpeed, fast.com,
    /// CacheFly, Vultr, Apple networkQuality (speedqx default).
    All,
    /// FAST: Cloudflare + NDT7 + MSAK, with per-provider empirical-Bernstein
    /// confidence-sequence early stop (RTT-gated) and a 25 s per-provider cap.
    Fast,
    /// Diagnostic subset: Cloudflare + NDT7 only (nd300 default).
    Diagnostic,
}

/// Configuration for the speed test orchestrator
#[derive(Debug, Clone)]
pub struct SpeedTestConfig {
    /// Duration per direction for CF, NDT7, LibreSpeed, MSAK, Apple (default: 30s)
    pub duration: TestDuration,
    /// Duration per direction for fast.com (default: Auto)
    pub fastcom_duration: TestDuration,
    /// Number of latency probes
    pub latency_probes: u32,
    /// Which providers to run
    pub provider_set: ProviderSet,
    /// Run the M-Lab MSAK multi-stream provider (All mode only)
    pub msak_enabled: bool,
    /// Run the Apple networkQuality provider (All mode only)
    pub apple_enabled: bool,
    /// Enable colored output
    pub use_colors: bool,
}

impl Default for SpeedTestConfig {
    fn default() -> Self {
        Self {
            duration: TestDuration::Seconds(30),
            fastcom_duration: TestDuration::Auto,
            latency_probes: 20,
            provider_set: ProviderSet::All,
            msak_enabled: true,
            apple_enabled: true,
            use_colors: true,
        }
    }
}

/// Phase indicator for progress callbacks
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    CfLatency,
    CfDownload,
    CfUpload,
    Ndt7Discovery,
    Ndt7Download,
    Ndt7Upload,
    LsDiscovery,
    LsDownload,
    LsUpload,
    FcDiscovery,
    FcDownload,
    FcUpload,
    MsakDiscovery,
    MsakDownload,
    MsakUpload,
    CfyLatency,
    CfyDownload,
    VultrDiscovery,
    VultrLatency,
    VultrDownload,
    AnqDiscovery,
    AnqDownload,
    AnqUpload,
    Computing,
}

/// Raw per-request Mbps samples for statistical post-processing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BandwidthSamples {
    pub download: Vec<f64>,
    pub upload: Vec<f64>,
}

/// Provider availability in the v4 payload (METHODOLOGY.md §3). On the CLI a
/// provider either `ran` or `failed`; `unavailable-platform` exists for schema
/// parity with the browser/app producers (nothing is platform-blocked here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderAvailability {
    Ran,
    Failed,
    UnavailablePlatform,
}

/// Dense idle-latency percentile block (METHODOLOGY.md §4/§9). The raw sample
/// array is kept internally (for bufferbloat's idle P50) but not serialized —
/// it is L3 drill-down data, absent from the headline schema.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LatencyStats {
    #[serde(skip)]
    pub samples: Vec<f64>,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
    pub p99: f64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub stddev: f64,
    /// Canonical jitter: `P95 − P50`.
    pub pdv: f64,
    pub jitter_mad: f64,
    /// Compatibility field only (RFC 3550 EWMA).
    pub jitter_rfc3550: f64,
}

impl LatencyStats {
    /// Build the percentile block from an RTT series (ms). `None` when empty.
    pub fn from_rtts(rtts: &[f64]) -> Option<Self> {
        if rtts.is_empty() {
            return None;
        }
        let mut sorted = rtts.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(Self {
            samples: rtts.to_vec(),
            p50: statistics::percentile(&sorted, 0.50),
            p75: statistics::percentile(&sorted, 0.75),
            p95: statistics::percentile(&sorted, 0.95),
            p99: statistics::percentile(&sorted, 0.99),
            min: sorted[0],
            max: sorted[sorted.len() - 1],
            mean: statistics::mean(rtts),
            stddev: statistics::stddev(rtts),
            pdv: statistics::pdv(rtts),
            jitter_mad: statistics::jitter_mad(rtts),
            jitter_rfc3550: statistics::jitter_rfc3550(rtts),
        })
    }
}

/// Loaded-latency RTT samples captured during Cloudflare saturation (§7).
/// Feeds the delta-ms bufferbloat grade and the RPM estimate.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LoadedLatency {
    pub download: Vec<f64>,
    pub upload: Vec<f64>,
}

/// A merged directional estimate with its confidence interval (METHODOLOGY.md §6/§9).
#[derive(Debug, Clone, Serialize)]
pub struct MergedDirection {
    pub download: f64,
    pub upload: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_ci: Option<statistics::CiBounds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_ci: Option<statistics::CiBounds>,
}

/// Provider agreement summary (`I²` + band) — replaces the v3 ">30% spread" flag.
#[derive(Debug, Clone, Serialize)]
pub struct AgreementInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i2: Option<f64>,
    pub band: statistics::AgreementBand,
}

/// Delta-ms bufferbloat summary (METHODOLOGY.md §7).
#[derive(Debug, Clone, Serialize)]
pub struct BufferbloatSummary {
    pub grade: statistics::BufferbloatGrade,
    pub delta_ms: f64,
    pub ratio: f64,
}

/// Connection stability metrics (coefficient of variation).
#[derive(Debug, Clone, Serialize)]
pub struct StabilityMetrics {
    pub download_cv: f64,
    pub upload_cv: f64,
    pub download_stable: bool,
    pub upload_stable: bool,
}

/// Provider divergence detection.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderDivergence {
    pub download: f64,
    pub upload: f64,
    pub significant: bool,
}

/// A provider direction excluded from the headline merge for insufficient
/// samples (additive JSON field, v3.4.0+).
#[derive(Debug, Clone, Serialize)]
pub struct MergeExclusion {
    pub provider: String,
    /// "download" or "upload".
    pub direction: &'static str,
    pub samples: usize,
}

/// Bootstrap confidence intervals on the pooled per-direction sample sets
/// (additive JSON field, v3.4.0+). The pooled-pipeline CI is an honest proxy
/// for the headline number's uncertainty: it naturally widens when providers
/// diverge.
#[derive(Debug, Clone, Serialize)]
pub struct ConfidenceIntervals {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<statistics::BootstrapCI>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload: Option<statistics::BootstrapCI>,
    pub confidence_level: f64,
}

/// Per-provider speed test result
#[derive(Debug, Clone, Serialize)]
pub struct ProviderResult {
    pub provider: String,
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_mbps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_mbps: Option<f64>,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub download_duration_s: f64,
    pub upload_duration_s: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_loss_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bandwidth_samples: Option<BandwidthSamples>,
    /// Availability for the v4 payload (`ran`/`failed`/`unavailable-platform`).
    pub availability: ProviderAvailability,
    /// Dense idle-latency percentile block (HTTP providers only; §4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_stats: Option<LatencyStats>,
    /// Loaded-latency probes captured during saturation (Cloudflare only; §7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loaded_latency: Option<LoadedLatency>,
}

/// Aggregated speed test result (used by both speedqx and nd300).
///
/// Carries the SpeedQX Methodology v4 payload (capacity/consensus + CIs, I²
/// agreement, RPM, delta-ms bufferbloat, PDV jitter, per-provider availability)
/// alongside the legacy v3 headline fields kept for one release.
#[derive(Debug, Clone, Serialize)]
pub struct SpeedTestResult {
    /// Methodology version ("4.0").
    pub methodology_version: &'static str,
    /// Producer platform identifier ("cli").
    pub platform: &'static str,
    /// Test mode: "full" / "fast" / "diagnostic".
    pub provider_set: &'static str,

    /// Headline min-RTT ping across engine + kernel MinRTT (METHODOLOGY.md §4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_ms: Option<f64>,
    /// Canonical jitter — PDV (`P95 − P50`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    /// Compatibility jitter (RFC 3550 EWMA).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_rfc3550: Option<f64>,
    /// Headline download = capacity (legacy field name kept for one release).
    pub download_mbps: f64,
    /// Headline upload = capacity (legacy field name kept for one release).
    pub upload_mbps: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_loss_pct: Option<f64>,

    /// CAPACITY: capability-weighted top-tier robust mean ± CI (headline).
    pub capacity: MergedDirection,
    /// CONSENSUS: conservative all-providers random-effects mean ± CI.
    pub consensus: MergedDirection,
    /// Download-direction agreement (I² + band).
    pub agreement: AgreementInfo,
    /// Upload-direction agreement (I² + band).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_agreement: Option<AgreementInfo>,
    /// Responsiveness (round-trips per minute) from CF loaded latency, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm: Option<f64>,
    /// Delta-ms bufferbloat grade from CF loaded latency, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bufferbloat: Option<BufferbloatSummary>,
    /// Headline latency percentile block (the §4 Cloudflare instrument).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_stats: Option<LatencyStats>,

    pub providers: Vec<ProviderResult>,
    pub duration_s: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability: Option<StabilityMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_divergence: Option<ProviderDivergence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_intervals: Option<ConfidenceIntervals>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub merge_exclusions: Vec<MergeExclusion>,
}

/// Canonical registry rank (METHODOLOGY.md §3). The shared PCG32 block-bootstrap
/// stream is drawn in this order (download then upload per provider) so the
/// index streams are reproducible regardless of the order providers ran in.
fn registry_rank(display_name: &str) -> usize {
    match display_name {
        "Cloudflare" => 0,
        "M-Lab NDT7" => 1,
        "M-Lab MSAK" => 2,
        "LibreSpeed" => 3,
        "fast.com" => 4,
        "CacheFly" => 5,
        "Vultr" => 6,
        "Apple networkQuality" => 7,
        _ => usize::MAX,
    }
}

/// Lowercase registry key for a provider display name — drives the capability
/// prior in [`statistics::merge_providers`] and labels merge exclusions.
fn registry_key(display_name: &str) -> &'static str {
    match display_name {
        "Cloudflare" => "cloudflare",
        "M-Lab NDT7" => "ndt7",
        "M-Lab MSAK" => "msak",
        "LibreSpeed" => "librespeed",
        "fast.com" => "fastcom",
        "CacheFly" => "cachefly",
        "Vultr" => "vultr",
        "Apple networkQuality" => "applenq",
        _ => "unknown",
    }
}

/// Display name for a registry key (inverse of [`registry_key`]); labels merge
/// exclusions with the human-facing provider name.
fn display_name_for_key(key: &str) -> String {
    match key {
        "cloudflare" => "Cloudflare",
        "ndt7" => "M-Lab NDT7",
        "msak" => "M-Lab MSAK",
        "librespeed" => "LibreSpeed",
        "fastcom" => "fast.com",
        "cachefly" => "CacheFly",
        "vultr" => "Vultr",
        "applenq" => "Apple networkQuality",
        other => other,
    }
    .to_string()
}

/// Per-provider cleaning (METHODOLOGY.md §5 steps 2–4, minus bootstrap):
/// sanitize → plateau warm-up discard → (upload only: keep fastest 50%) →
/// IQR fences at k = 1.5. Mirrors the TS `cleanDirection`.
fn clean_direction(raw: &[f64], is_upload: bool) -> Vec<f64> {
    let sane = statistics::sanitize(raw);
    if sane.is_empty() {
        return Vec::new();
    }
    let cut = statistics::plateau_start(&sane).min(sane.len());
    let after_plateau = &sane[cut..];
    if is_upload {
        // Keep the fastest ceil(n/2) post-warm-up samples before the IQR filter.
        let mut desc = after_plateau.to_vec();
        desc.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let keep = ((desc.len() as f64) / 2.0).ceil() as usize;
        let top = &desc[..keep.min(desc.len())];
        statistics::filter_outliers_iqr(top, 1.5)
    } else {
        statistics::filter_outliers_iqr(after_plateau, 1.5)
    }
}

/// Aggregation result carrying the full v4 payload for one run.
struct AggregateResult {
    ping: Option<f64>,
    jitter: Option<f64>,
    jitter_rfc3550: Option<f64>,
    download: f64,
    upload: f64,
    packet_loss: Option<f64>,
    capacity: MergedDirection,
    consensus: MergedDirection,
    agreement: AgreementInfo,
    upload_agreement: AgreementInfo,
    rpm: Option<f64>,
    bufferbloat: Option<BufferbloatSummary>,
    latency_stats: Option<LatencyStats>,
    stability: Option<StabilityMetrics>,
    divergence: Option<ProviderDivergence>,
    confidence: Option<ConfidenceIntervals>,
    exclusions: Vec<MergeExclusion>,
}

fn empty_direction() -> MergedDirection {
    MergedDirection {
        download: 0.0,
        upload: 0.0,
        download_ci: None,
        upload_ci: None,
    }
}

/// SpeedQX v4 cross-provider aggregation (METHODOLOGY.md §5–§7).
///
/// Per successful provider (drawn in registry order so the shared PCG32
/// block-bootstrap stream is reproducible), each direction is cleaned
/// (plateau → [upload: fastest 50%] → IQR) and block-bootstrapped for its
/// trimean point estimate + variance; the results feed
/// [`statistics::merge_providers`] to produce capacity (headline) + consensus
/// with HKSJ CIs and I² agreement. Headline ping is min-RTT across every
/// provider's ping (NDT7/MSAK carry kernel MinRTT — the cross-check); headline
/// jitter is PDV from the dense latency block; bufferbloat + RPM come from the
/// Cloudflare loaded-latency probes when present.
fn aggregate(providers: &[ProviderResult]) -> AggregateResult {
    // Successful providers in canonical registry order.
    let mut ordered: Vec<&ProviderResult> =
        providers.iter().filter(|p| p.error.is_none()).collect();
    ordered.sort_by_key(|p| registry_rank(&p.provider));

    if ordered.is_empty() {
        return AggregateResult {
            ping: None,
            jitter: None,
            jitter_rfc3550: None,
            download: 0.0,
            upload: 0.0,
            packet_loss: None,
            capacity: empty_direction(),
            consensus: empty_direction(),
            agreement: AgreementInfo {
                i2: None,
                band: statistics::AgreementBand::Insufficient,
            },
            upload_agreement: AgreementInfo {
                i2: None,
                band: statistics::AgreementBand::Insufficient,
            },
            rpm: None,
            bufferbloat: None,
            latency_stats: None,
            stability: None,
            divergence: None,
            confidence: None,
            exclusions: Vec::new(),
        };
    }

    // ── Per-provider cleaning + block bootstrap (ONE PCG32 stream) ──────
    let mut rng = Pcg32::new();
    let mut dl_inputs: Vec<MergeProviderInput> = Vec::new();
    let mut ul_inputs: Vec<MergeProviderInput> = Vec::new();
    let mut pooled_dl: Vec<f64> = Vec::new();
    let mut pooled_ul: Vec<f64> = Vec::new();
    // Per-provider cleaned point estimates, keyed by display name.
    let mut points: HashMap<String, (Option<f64>, Option<f64>)> = HashMap::new();

    for p in &ordered {
        let (dl_raw, ul_raw): (&[f64], &[f64]) = match &p.bandwidth_samples {
            Some(bs) => (&bs.download, &bs.upload),
            None => (&[], &[]),
        };
        let dl_clean = clean_direction(dl_raw, false);
        let ul_clean = clean_direction(ul_raw, true);
        let dl_boot = statistics::circular_block_bootstrap(&dl_clean, &mut rng, 2000);
        let ul_boot = statistics::circular_block_bootstrap(&ul_clean, &mut rng, 2000);
        let key = registry_key(&p.provider);

        dl_inputs.push(MergeProviderInput {
            name: key.to_string(),
            y: dl_boot.theta_hat,
            v: Some(dl_boot.variance),
            samples: dl_clean.len(),
            capability: None,
            bca: Some(BcaInterval {
                lower: dl_boot.ci_lower,
                upper: dl_boot.ci_upper,
            }),
        });
        ul_inputs.push(MergeProviderInput {
            name: key.to_string(),
            y: ul_boot.theta_hat,
            v: Some(ul_boot.variance),
            samples: ul_clean.len(),
            capability: None,
            bca: Some(BcaInterval {
                lower: ul_boot.ci_lower,
                upper: ul_boot.ci_upper,
            }),
        });

        points.insert(
            p.provider.clone(),
            (
                (!dl_clean.is_empty()).then_some(dl_boot.theta_hat),
                (!ul_clean.is_empty()).then_some(ul_boot.theta_hat),
            ),
        );
        pooled_dl.extend_from_slice(&dl_clean);
        pooled_ul.extend_from_slice(&ul_clean);
    }

    let dl_merge = merge_providers(&dl_inputs);
    let ul_merge = merge_providers(&ul_inputs);

    // Headline = capacity; degrade to the max per-provider point when no
    // provider qualified for the merge (never 0.0 while data exists).
    let fallback_max = |download: bool| -> f64 {
        points
            .values()
            .filter_map(|(dl, ul)| if download { *dl } else { *ul })
            .fold(0.0_f64, f64::max)
    };
    let download = if dl_merge.capacity > 0.0 {
        dl_merge.capacity
    } else {
        fallback_max(true)
    };
    let upload = if ul_merge.capacity > 0.0 {
        ul_merge.capacity
    } else {
        fallback_max(false)
    };

    // ── Headline ping: min-RTT across engine + kernel MinRTT (§4) ───────
    let mut ping = f64::INFINITY;
    for p in &ordered {
        if let Some(pg) = p.ping_ms {
            if pg > 0.0 && pg < ping {
                ping = pg;
            }
        }
        if let Some(ls) = &p.latency_stats {
            if ls.min > 0.0 && ls.min < ping {
                ping = ls.min;
            }
        }
    }
    let ping = ping.is_finite().then_some(ping);

    // Headline latency block = the §4 instrument (Cloudflare), else the first
    // provider in registry order with a dense engine block.
    let latency_stats = ordered.iter().find_map(|p| p.latency_stats.clone());

    // Headline jitter = PDV (canonical); RFC 3550 EWMA kept as a compat field.
    let jitter = latency_stats.as_ref().map(|ls| ls.pdv).or_else(|| {
        let js: Vec<f64> = ordered
            .iter()
            .filter_map(|p| p.jitter_ms)
            .filter(|j| *j > 0.0)
            .collect();
        (!js.is_empty()).then(|| statistics::mean(&js))
    });
    let jitter_rfc3550 = latency_stats
        .as_ref()
        .map(|ls| ls.jitter_rfc3550)
        .or_else(|| ordered.iter().find_map(|p| p.jitter_ms));

    // ── Bufferbloat + RPM from Cloudflare loaded-latency probes (§7) ────
    let mut bufferbloat = None;
    let mut rpm = None;
    if let Some(cf) = ordered.iter().find(|p| p.provider == "Cloudflare") {
        if let Some(loaded) = &cf.loaded_latency {
            let mut loaded_pooled = loaded.download.clone();
            loaded_pooled.extend_from_slice(&loaded.upload);
            if !loaded_pooled.is_empty() {
                let idle = cf
                    .latency_stats
                    .as_ref()
                    .map(|ls| ls.samples.clone())
                    .unwrap_or_default();
                let bb = statistics::bufferbloat_delta(&idle, &loaded_pooled);
                rpm = Some(statistics::rpm(&loaded_pooled));
                bufferbloat = Some(BufferbloatSummary {
                    grade: bb.grade,
                    delta_ms: bb.delta_ms,
                    ratio: bb.ratio,
                });
            }
        }
    }

    // Packet loss from Cloudflare (only provider that measures it).
    let packet_loss = ordered
        .iter()
        .find(|p| p.provider == "Cloudflare")
        .and_then(|p| p.packet_loss_pct);

    // ── Stability CV on pooled cleaned samples ──────────────────────────
    let stability = if pooled_dl.len() > 2 || pooled_ul.len() > 2 {
        let dl_cv = statistics::coefficient_of_variation(&pooled_dl);
        let ul_cv = statistics::coefficient_of_variation(&pooled_ul);
        Some(StabilityMetrics {
            download_cv: dl_cv,
            upload_cv: ul_cv,
            download_stable: dl_cv < 0.15,
            upload_stable: ul_cv < 0.15,
        })
    } else {
        None
    };

    // ── Agreement bands from I² (replaces the v3 >30% spread flag) ──────
    let agreement = AgreementInfo {
        i2: dl_merge.i2,
        band: dl_merge.band,
    };
    let upload_agreement = AgreementInfo {
        i2: ul_merge.i2,
        band: ul_merge.band,
    };
    let band_low = |b: statistics::AgreementBand| {
        matches!(
            b,
            statistics::AgreementBand::Low | statistics::AgreementBand::VeryLow
        )
    };
    let divergence = Some(ProviderDivergence {
        download: dl_merge.i2.unwrap_or(0.0),
        upload: ul_merge.i2.unwrap_or(0.0),
        significant: band_low(dl_merge.band) || band_low(ul_merge.band),
    });

    // ── Capacity / consensus with CIs ───────────────────────────────────
    let capacity = MergedDirection {
        download,
        upload,
        download_ci: (dl_merge.k > 0).then_some(dl_merge.capacity_ci),
        upload_ci: (ul_merge.k > 0).then_some(ul_merge.capacity_ci),
    };
    let consensus = MergedDirection {
        download: dl_merge.consensus,
        upload: ul_merge.consensus,
        download_ci: (dl_merge.k > 0).then_some(dl_merge.consensus_ci),
        upload_ci: (ul_merge.k > 0).then_some(ul_merge.consensus_ci),
    };

    // Legacy confidence_intervals surface (capacity CI) for one release.
    let ci_from = |value: f64, ci: statistics::CiBounds| statistics::BootstrapCI {
        estimate: value,
        lower: ci.lower,
        upper: ci.upper,
        margin: ((ci.upper - ci.lower) / 2.0).max(0.0),
    };
    let confidence = if (dl_merge.k > 0 && download > 0.0) || (ul_merge.k > 0 && upload > 0.0) {
        Some(ConfidenceIntervals {
            download: (dl_merge.k > 0 && download > 0.0)
                .then(|| ci_from(download, dl_merge.capacity_ci)),
            upload: (ul_merge.k > 0 && upload > 0.0).then(|| ci_from(upload, ul_merge.capacity_ci)),
            confidence_level: 0.95,
        })
    } else {
        None
    };

    // ── Merge exclusions from both directions (display-name labeled) ────
    let mut exclusions: Vec<MergeExclusion> = Vec::new();
    for e in &dl_merge.exclusions {
        exclusions.push(MergeExclusion {
            provider: display_name_for_key(&e.name),
            direction: "download",
            samples: e.samples,
        });
    }
    for e in &ul_merge.exclusions {
        exclusions.push(MergeExclusion {
            provider: display_name_for_key(&e.name),
            direction: "upload",
            samples: e.samples,
        });
    }

    AggregateResult {
        ping,
        jitter,
        jitter_rfc3550,
        download,
        upload,
        packet_loss,
        capacity,
        consensus,
        agreement,
        upload_agreement,
        rpm,
        bufferbloat,
        latency_stats,
        stability,
        divergence,
        confidence,
        exclusions,
    }
}

/// Callback type for provider completion notifications.
pub type ProviderCompleteCallback = Arc<dyn Fn(&ProviderResult) + Send + Sync>;

/// Synthesize a failed [`ProviderResult`] (used by the FAST hard-cap path).
fn failed_provider(provider: &str, msg: &str) -> ProviderResult {
    ProviderResult {
        provider: provider.to_string(),
        server: "unknown".to_string(),
        location: None,
        ping_ms: None,
        jitter_ms: None,
        download_mbps: None,
        upload_mbps: None,
        download_bytes: 0,
        upload_bytes: 0,
        download_duration_s: 0.0,
        upload_duration_s: 0.0,
        packet_loss_pct: None,
        error: Some(msg.to_string()),
        bandwidth_samples: None,
        availability: ProviderAvailability::Failed,
        latency_stats: None,
        loaded_latency: None,
    }
}

/// Await a provider future, applying the FAST per-provider 25 s hard cap
/// (METHODOLOGY.md §8). Outside FAST the future runs to completion.
/// Liveness watchdog: a provider that reports NO progress for this long is
/// wedged on something other than the network under test (a rate limiter, a
/// silent WebSocket, a dead service, a hung discovery call) and is failed so
/// the rest of the run continues untouched. Every honest code path — including
/// a badly degraded link — emits progress at seconds scale (per latency probe,
/// per completed/deadline-bounded transfer request, per WS measurement
/// message), so 60 s of true silence is never a slow network.
const PROVIDER_STALL_SECS: u64 = 60;

async fn run_provider_future(
    fast: bool,
    provider_name: &str,
    last_activity: Arc<StdMutex<Instant>>,
    fut: impl std::future::Future<Output = ProviderResult>,
) -> ProviderResult {
    let stall = Duration::from_secs(PROVIDER_STALL_SECS);
    let watchdog = async {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let idle = last_activity
                .lock()
                .map(|t| t.elapsed())
                .unwrap_or_default();
            if idle >= stall {
                break;
            }
        }
    };
    let bounded = async {
        tokio::select! {
            r = fut => Some(r),
            _ = watchdog => None,
        }
    };
    let outcome = if fast {
        match tokio::time::timeout(FAST_HARD_CAP, bounded).await {
            Ok(o) => o,
            Err(_) => return failed_provider(provider_name, "FAST 25s hard cap reached"),
        }
    } else {
        bounded.await
    };
    outcome.unwrap_or_else(|| {
        failed_provider(
            provider_name,
            &format!("stalled — no progress for {PROVIDER_STALL_SECS}s (non-network wedge)"),
        )
    })
}

/// Wrap a progress callback so every event stamps the provider's liveness
/// clock for the stall watchdog in [`run_provider_future`].
fn stamped_progress<F>(
    progress: Arc<F>,
    last_activity: Arc<StdMutex<Instant>>,
) -> impl Fn(Phase, f64) + Send + Sync
where
    F: Fn(Phase, f64) + Send + Sync + 'static,
{
    move |phase, p| {
        if let Ok(mut t) = last_activity.lock() {
            *t = Instant::now();
        }
        progress(phase, p)
    }
}

/// Run the speed test with the given configuration and progress callback.
/// The `on_provider_complete` callback is called after each provider finishes,
/// allowing the UI to show per-provider summaries.
pub async fn run<F>(
    config: SpeedTestConfig,
    progress: F,
    on_provider_complete: Option<ProviderCompleteCallback>,
) -> SpeedTestResult
where
    F: Fn(Phase, f64) + Send + Sync + 'static,
{
    let start = Instant::now();
    let mut providers = Vec::new();
    let progress = Arc::new(progress);
    let fast = config.provider_set == ProviderSet::Fast;

    // FAST providers use a short per-direction budget. Cloudflare additionally
    // bounds its whole run against an internal soft deadline (below the 25 s
    // hard cap) so its RTT-scaled dense-latency engine can't push it past the
    // cap and get its completed work discarded; the empirical-Bernstein early
    // stop (inside the providers, gated on measured min-RTT) trims the transfer
    // phases further.
    let provider_config = if fast {
        SpeedTestConfig {
            duration: TestDuration::Seconds(FAST_PER_DIR_SECS),
            ..config.clone()
        }
    } else {
        config.clone()
    };

    let record = |providers: &mut Vec<ProviderResult>, result: ProviderResult| {
        if let Some(ref cb) = on_provider_complete {
            cb(&result);
        }
        providers.push(result);
    };

    // ── Providers run sequentially in canonical registry order (§2/§3) ──

    // Cloudflare (always).
    {
        let last = Arc::new(StdMutex::new(Instant::now()));
        let cb = stamped_progress(progress.clone(), last.clone());
        let r = run_provider_future(
            fast,
            "Cloudflare",
            last,
            cloudflare::run(&provider_config, cb),
        )
        .await;
        record(&mut providers, r);
    }

    // M-Lab NDT7 (always).
    let ndt7_locate_rate_limited = {
        let last = Arc::new(StdMutex::new(Instant::now()));
        let cb = stamped_progress(progress.clone(), last.clone());
        let r =
            run_provider_future(fast, "M-Lab NDT7", last, ndt7::run(&provider_config, cb)).await;
        let rate_limited = r
            .error
            .as_deref()
            .is_some_and(|e| e.contains("discovery rate-limited"));
        record(&mut providers, r);
        rate_limited
    };

    // M-Lab MSAK — FAST always; FULL when enabled; never in Diagnostic.
    let run_msak = match config.provider_set {
        ProviderSet::Fast => true,
        ProviderSet::All => config.msak_enabled,
        ProviderSet::Diagnostic => false,
    };
    if run_msak {
        // Both M-Lab providers are assigned servers by the same Locate API.
        // If NDT7's discovery was just refused with a rate limit, MSAK's
        // would be too — skip the request instead of feeding another call
        // into an already-tripped per-IP limiter.
        if ndt7_locate_rate_limited {
            record(
                &mut providers,
                msak::error_result(
                    "skipped: M-Lab Locate is rate-limiting this network (NDT7 discovery was refused) — try again later"
                        .to_string(),
                ),
            );
        } else {
            let last = Arc::new(StdMutex::new(Instant::now()));
            let cb = stamped_progress(progress.clone(), last.clone());
            let r = run_provider_future(fast, "M-Lab MSAK", last, msak::run(&provider_config, cb))
                .await;
            record(&mut providers, r);
        }
    }

    // FULL-only providers, in registry order: LibreSpeed, fast.com, CacheFly,
    // Vultr, Apple networkQuality.
    if config.provider_set == ProviderSet::All {
        {
            let last = Arc::new(StdMutex::new(Instant::now()));
            let cb = stamped_progress(progress.clone(), last.clone());
            let r = run_provider_future(
                fast,
                "LibreSpeed",
                last,
                librespeed::run(&provider_config, cb),
            )
            .await;
            record(&mut providers, r);
        }
        {
            let last = Arc::new(StdMutex::new(Instant::now()));
            let cb = stamped_progress(progress.clone(), last.clone());
            let r = run_provider_future(fast, "fast.com", last, fastcom::run(&provider_config, cb))
                .await;
            record(&mut providers, r);
        }
        {
            let last = Arc::new(StdMutex::new(Instant::now()));
            let cb = stamped_progress(progress.clone(), last.clone());
            let r =
                run_provider_future(fast, "CacheFly", last, cachefly::run(&provider_config, cb))
                    .await;
            record(&mut providers, r);
        }
        {
            let last = Arc::new(StdMutex::new(Instant::now()));
            let cb = stamped_progress(progress.clone(), last.clone());
            let r =
                run_provider_future(fast, "Vultr", last, vultr::run(&provider_config, cb)).await;
            record(&mut providers, r);
        }
        if config.apple_enabled {
            let last = Arc::new(StdMutex::new(Instant::now()));
            let cb = stamped_progress(progress.clone(), last.clone());
            let r = run_provider_future(
                fast,
                "Apple networkQuality",
                last,
                applenq::run(&provider_config, cb),
            )
            .await;
            record(&mut providers, r);
        }
    }

    progress(Phase::Computing, 1.0);

    let agg = aggregate(&providers);
    let duration = start.elapsed().as_secs_f64();

    let provider_set = match config.provider_set {
        ProviderSet::All => "full",
        ProviderSet::Fast => "fast",
        ProviderSet::Diagnostic => "diagnostic",
    };

    SpeedTestResult {
        methodology_version: METHODOLOGY_VERSION,
        platform: "cli",
        provider_set,
        ping_ms: agg.ping,
        jitter_ms: agg.jitter,
        jitter_rfc3550: agg.jitter_rfc3550,
        download_mbps: agg.download,
        upload_mbps: agg.upload,
        packet_loss_pct: agg.packet_loss,
        capacity: agg.capacity,
        consensus: agg.consensus,
        agreement: agg.agreement,
        upload_agreement: Some(agg.upload_agreement),
        rpm: agg.rpm,
        bufferbloat: agg.bufferbloat,
        latency_stats: agg.latency_stats,
        providers,
        duration_s: duration,
        stability: agg.stability,
        provider_divergence: agg.divergence,
        confidence_intervals: agg.confidence,
        merge_exclusions: agg.exclusions,
    }
}

/// Format Mbps value for display
pub fn format_mbps(mbps: f64) -> String {
    if mbps >= 1000.0 {
        format!("{:.1} Gbps", mbps / 1000.0)
    } else if mbps >= 100.0 {
        format!("{:.0} Mbps", mbps)
    } else if mbps >= 10.0 {
        format!("{:.1} Mbps", mbps)
    } else {
        format!("{:.2} Mbps", mbps)
    }
}

/// Format bytes for display
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_with_samples(name: &str, download: Vec<f64>, upload: Vec<f64>) -> ProviderResult {
        ProviderResult {
            provider: name.to_string(),
            server: "test".to_string(),
            location: None,
            ping_ms: None,
            jitter_ms: None,
            download_mbps: download.last().copied(),
            upload_mbps: upload.last().copied(),
            download_bytes: 1,
            upload_bytes: 1,
            download_duration_s: 1.0,
            upload_duration_s: 1.0,
            packet_loss_pct: None,
            error: None,
            bandwidth_samples: Some(BandwidthSamples { download, upload }),
            availability: ProviderAvailability::Ran,
            latency_stats: None,
            loaded_latency: None,
        }
    }

    /// A dense, tight sample series (16 points around ~100) so both directions
    /// survive plateau + IQR cleaning with ≥ 4 cleaned samples.
    fn dense(center: f64) -> Vec<f64> {
        (0..16).map(|i| center + (i % 4) as f64 - 1.5).collect()
    }

    #[test]
    fn dense_probe_count_clamps() {
        assert_eq!(dense_probe_count(15.0), 50); // round(49.5)=50
        assert_eq!(dense_probe_count(30.0), 99); // round(99)
        assert_eq!(dense_probe_count(60.0), 198);
        assert_eq!(dense_probe_count(1000.0), 200); // clamp high
        assert_eq!(dense_probe_count(1.0), 50); // clamp low
    }

    /// Headline download = capacity; well-sampled providers around ~100 Mbps
    /// merge to a capacity near 100 with a CI attached.
    #[test]
    fn capacity_headline_tracks_well_sampled_providers() {
        let providers = vec![
            provider_with_samples("Cloudflare", dense(100.0), dense(20.0)),
            provider_with_samples("M-Lab NDT7", dense(100.0), dense(20.0)),
        ];
        let agg = aggregate(&providers);
        assert!(
            (agg.download - 100.0).abs() < 10.0,
            "capacity should track ~100, got {}",
            agg.download
        );
        assert_eq!(agg.capacity.download, agg.download);
        assert!(agg.capacity.download_ci.is_some());
    }

    /// A download-only provider (empty upload) is surfaced as an upload
    /// exclusion rather than silently dropped.
    #[test]
    fn download_only_provider_recorded_as_upload_exclusion() {
        let providers = vec![
            provider_with_samples("Cloudflare", dense(100.0), dense(20.0)),
            provider_with_samples("CacheFly", dense(400.0), vec![]),
        ];
        let agg = aggregate(&providers);
        assert!(
            agg.exclusions
                .iter()
                .any(|e| e.provider == "CacheFly" && e.direction == "upload"),
            "CacheFly should be an upload exclusion: {:?}",
            agg.exclusions
                .iter()
                .map(|e| (&e.provider, e.direction, e.samples))
                .collect::<Vec<_>>()
        );
    }

    /// A 1-sample provider collapses to zero cleaned samples and is excluded;
    /// the headline still tracks the well-sampled provider.
    #[test]
    fn low_sample_provider_excluded_from_merge() {
        let providers = vec![
            provider_with_samples("Cloudflare", dense(100.0), dense(20.0)),
            provider_with_samples("LibreSpeed", vec![1000.0], vec![1000.0]),
        ];
        let agg = aggregate(&providers);
        assert!(
            (agg.download - 100.0).abs() < 12.0,
            "merge should track the well-sampled provider, got {}",
            agg.download
        );
        assert!(
            agg.exclusions
                .iter()
                .any(|e| e.provider == "LibreSpeed" && e.direction == "download"),
            "LibreSpeed download exclusion should be recorded: {:?}",
            agg.exclusions
                .iter()
                .map(|e| (&e.provider, e.direction, e.samples))
                .collect::<Vec<_>>()
        );
    }

    /// When no provider reaches the 4-sample floor, the headline degrades to
    /// the max per-provider point estimate — never 0.0 while data exists.
    #[test]
    fn degraded_all_sparse_still_nonzero() {
        let providers = vec![
            provider_with_samples("Cloudflare", vec![100.0, 102.0], vec![20.0, 21.0]),
            provider_with_samples("LibreSpeed", vec![110.0, 108.0], vec![22.0, 23.0]),
        ];
        let agg = aggregate(&providers);
        assert!(agg.download > 0.0, "degraded merge must not return 0.0");
    }

    /// Headline ping is the min-RTT across providers (NDT7 kernel MinRTT wins
    /// here — the cross-check), not a weighted blend.
    #[test]
    fn headline_ping_is_min_rtt() {
        let mut cf = provider_with_samples("Cloudflare", dense(100.0), dense(20.0));
        cf.ping_ms = Some(30.0);
        let mut ndt = provider_with_samples("M-Lab NDT7", dense(100.0), dense(20.0));
        ndt.ping_ms = Some(12.0);
        let agg = aggregate(&[cf, ndt]);
        assert_eq!(agg.ping, Some(12.0));
    }

    /// PDV is the canonical headline jitter, sourced from the dense latency block.
    #[test]
    fn headline_jitter_is_pdv_from_latency_block() {
        let mut cf = provider_with_samples("Cloudflare", dense(100.0), dense(20.0));
        cf.latency_stats =
            LatencyStats::from_rtts(&[10.0, 11.0, 12.0, 20.0, 10.5, 30.0, 10.2, 11.5]);
        let agg = aggregate(&[cf]);
        let ls = agg.latency_stats.as_ref().expect("latency block present");
        assert_eq!(agg.jitter, Some(ls.pdv));
        assert!(agg.jitter.unwrap() >= 0.0);
    }

    /// Bufferbloat + RPM are computed from Cloudflare loaded-latency probes.
    #[test]
    fn bufferbloat_and_rpm_from_loaded_latency() {
        let mut cf = provider_with_samples("Cloudflare", dense(100.0), dense(20.0));
        cf.latency_stats = LatencyStats::from_rtts(&[10.0; 8]);
        cf.loaded_latency = Some(LoadedLatency {
            download: vec![50.0; 5],
            upload: vec![55.0; 5],
        });
        let agg = aggregate(&[cf]);
        let bb = agg.bufferbloat.expect("bufferbloat present");
        assert!(
            bb.delta_ms > 30.0,
            "loaded 50-55 vs idle 10, got {}",
            bb.delta_ms
        );
        assert!(agg.rpm.expect("rpm present") > 0.0);
    }

    /// The empty-run path yields a zeroed headline and an insufficient band.
    #[test]
    fn empty_run_is_zeroed() {
        let mut failed = provider_with_samples("Cloudflare", vec![], vec![]);
        failed.error = Some("boom".to_string());
        let agg = aggregate(&[failed]);
        assert_eq!(agg.download, 0.0);
        assert_eq!(agg.agreement.band, statistics::AgreementBand::Insufficient);
        assert!(agg.ping.is_none());
    }
}
