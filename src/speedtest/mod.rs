pub(crate) mod adaptive;
pub mod cloudflare;
pub mod display;
pub mod fastcom;
pub mod librespeed;
pub mod ndt7;
pub mod statistics;

use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

/// Test duration configuration
#[derive(Debug, Clone)]
pub enum TestDuration {
    /// Fixed duration per direction in seconds (e.g., 30 = 30s download + 30s upload)
    Seconds(u64),
    /// Let providers use their natural duration
    Auto,
}

/// Which providers to run
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProviderSet {
    /// All 4 providers: Cloudflare, NDT7, LibreSpeed, fast.com (speedqx default)
    All,
    /// Diagnostic subset: Cloudflare + NDT7 only (nd300 default)
    Diagnostic,
}

/// Configuration for the speed test orchestrator
#[derive(Debug, Clone)]
pub struct SpeedTestConfig {
    /// Duration per direction for CF, NDT7, LibreSpeed (default: 30s)
    pub duration: TestDuration,
    /// Duration per direction for fast.com (default: Auto)
    pub fastcom_duration: TestDuration,
    /// Number of latency probes
    pub latency_probes: u32,
    /// Which providers to run
    pub provider_set: ProviderSet,
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
    Computing,
}

/// Raw per-request Mbps samples for statistical post-processing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BandwidthSamples {
    pub download: Vec<f64>,
    pub upload: Vec<f64>,
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
}

/// Aggregated speed test result (used by both speedqx and nd300)
#[derive(Debug, Clone, Serialize)]
pub struct SpeedTestResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    pub download_mbps: f64,
    pub upload_mbps: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_loss_pct: Option<f64>,
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

/// Latency weight for Cloudflare (NDT7 gets 1 - this).
/// NDT7's MinRTT from TCP kernel is structurally superior, not just lower-variance.
const CF_LATENCY_WEIGHT: f64 = 0.4;

/// Divergence threshold: flag when providers differ by more than this fraction.
const DIVERGENCE_THRESHOLD: f64 = 0.3;

fn divergence_ratio(a: f64, b: f64) -> f64 {
    if a <= 0.0 || b <= 0.0 {
        return 0.0;
    }
    (a - b).abs() / a.max(b)
}

fn divergence_spread(values: &[(f64, f64)]) -> f64 {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;

    for (value, _) in values {
        if *value <= 0.0 {
            continue;
        }
        min = min.min(*value);
        max = max.max(*value);
    }

    if !min.is_finite() || !max.is_finite() || max <= 0.0 || min == max {
        0.0
    } else {
        divergence_ratio(min, max)
    }
}

fn inverse_variance_merge_many(values: &[(f64, f64)]) -> f64 {
    let positive: Vec<(f64, f64)> = values
        .iter()
        .copied()
        .filter(|(value, _)| value.is_finite() && *value > 0.0)
        .collect();

    if positive.is_empty() {
        return 0.0;
    }
    if positive.len() == 1 {
        return positive[0].0;
    }

    // A variance that is zero, negative, or non-finite means UNKNOWN
    // precision (a 1-sample provider reports variance 0.0; a fallback value
    // has no samples at all). Unknown precision must be treated as the LEAST
    // trusted entry — assign it the maximum known variance. The old rule
    // clamped unknowns to the variance floor, which handed a degenerate
    // single-sample provider the highest possible weight.
    let known = |v: f64| v.is_finite() && v > 0.0;

    let max_known_variance = positive
        .iter()
        .filter_map(|(_, variance)| known(*variance).then_some(*variance))
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let Some(max_variance) = max_known_variance else {
        // No provider has a known variance — plain average.
        return positive.iter().map(|(value, _)| value).sum::<f64>() / positive.len() as f64;
    };

    let variance_floor = positive
        .iter()
        .filter_map(|(_, variance)| known(*variance).then_some(*variance))
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(max_variance)
        .max(0.000_001);

    let raw_weights: Vec<f64> = positive
        .iter()
        .map(|(_, variance)| {
            let effective = if known(*variance) {
                *variance
            } else {
                max_variance
            };
            1.0 / effective.max(variance_floor)
        })
        .collect();
    let raw_total = raw_weights.iter().sum::<f64>();
    if raw_total <= 0.0 {
        return positive.iter().map(|(value, _)| value).sum::<f64>() / positive.len() as f64;
    }

    let weights = capped_inverse_variance_weights(&raw_weights, raw_total, 0.70);

    positive
        .iter()
        .zip(weights.iter())
        .map(|((value, _), weight)| value * weight)
        .sum()
}

fn capped_inverse_variance_weights(raw_weights: &[f64], raw_total: f64, cap: f64) -> Vec<f64> {
    if raw_weights.is_empty() {
        return Vec::new();
    }
    if raw_weights.len() == 1 {
        return vec![1.0];
    }
    if raw_total <= 0.0 {
        let equal = 1.0 / raw_weights.len() as f64;
        return vec![equal; raw_weights.len()];
    }

    let cap = cap.max(1.0 / raw_weights.len() as f64);
    let mut weights = vec![0.0; raw_weights.len()];
    let mut remaining: Vec<usize> = (0..raw_weights.len()).collect();
    let mut remaining_mass = 1.0;

    loop {
        if remaining.is_empty() {
            break;
        }

        let remaining_raw_total = remaining.iter().map(|idx| raw_weights[*idx]).sum::<f64>();
        if remaining_raw_total <= 0.0 {
            let equal = remaining_mass / remaining.len() as f64;
            for idx in remaining {
                weights[idx] = equal;
            }
            break;
        }

        let mut capped = Vec::new();
        for idx in &remaining {
            let candidate = remaining_mass * raw_weights[*idx] / remaining_raw_total;
            if candidate > cap {
                weights[*idx] = cap;
                remaining_mass = (remaining_mass - cap).max(0.0);
                capped.push(*idx);
            }
        }

        if capped.is_empty() {
            for idx in remaining {
                weights[idx] = remaining_mass * raw_weights[idx] / remaining_raw_total;
            }
            break;
        }

        remaining.retain(|idx| !capped.contains(idx));
    }

    weights
}

/// Aggregation result including new metrics.
struct AggregateResult {
    ping: Option<f64>,
    jitter: Option<f64>,
    download: f64,
    upload: f64,
    packet_loss: Option<f64>,
    stability: Option<StabilityMetrics>,
    divergence: Option<ProviderDivergence>,
    confidence: Option<ConfidenceIntervals>,
    exclusions: Vec<MergeExclusion>,
}

/// Minimum sanitized samples a provider needs in a direction before it joins
/// the headline inverse-variance merge. Matches the statistics pipeline's own
/// internal thresholds (slow-start discard, IQR filter, and winsorize all
/// bail below 4) — a trimean over fewer samples is noise, and its degenerate
/// variance used to let it dominate the merge.
const MIN_MERGE_SAMPLES: usize = 4;

/// Pooled samples needed before a bootstrap CI is computed. Below 8 the
/// percentile method's bounds are too coarse to be honest (and below 4 they
/// collapse to a misleading "±0").
const MIN_CI_SAMPLES: usize = 8;

/// One provider direction proposed for the merge.
struct MergeCandidate {
    provider: String,
    value: f64,
    variance: f64,
    samples: usize,
}

/// Apply the minimum-sample floor: providers with enough samples make the
/// merge; the rest are recorded as exclusions. Degraded fallback: when NO
/// provider reaches the floor, every positive candidate is kept (the merge
/// must never return 0.0 while data exists).
fn select_for_merge(
    candidates: Vec<MergeCandidate>,
    direction: &'static str,
    exclusions: &mut Vec<MergeExclusion>,
) -> Vec<(f64, f64)> {
    let any_qualified = candidates.iter().any(|c| c.samples >= MIN_MERGE_SAMPLES);
    if !any_qualified {
        return candidates.iter().map(|c| (c.value, c.variance)).collect();
    }

    let mut kept = Vec::new();
    for c in candidates {
        if c.samples >= MIN_MERGE_SAMPLES {
            kept.push((c.value, c.variance));
        } else {
            exclusions.push(MergeExclusion {
                provider: c.provider,
                direction,
                samples: c.samples,
            });
        }
    }
    kept
}

/// Inverse-variance weighted aggregation across providers.
/// Uses accurate bandwidth pipeline on raw samples, fixed latency weights,
/// stability metrics, and divergence detection.
fn aggregate(providers: &[ProviderResult]) -> AggregateResult {
    let successful: Vec<&ProviderResult> = providers.iter().filter(|p| p.error.is_none()).collect();

    if successful.is_empty() {
        return AggregateResult {
            ping: None,
            jitter: None,
            download: 0.0,
            upload: 0.0,
            packet_loss: None,
            stability: None,
            divergence: None,
            confidence: None,
            exclusions: Vec::new(),
        };
    }

    // ── Compute accurate bandwidth per provider from sanitized samples ──
    let mut dl_candidates: Vec<MergeCandidate> = Vec::new();
    let mut ul_candidates: Vec<MergeCandidate> = Vec::new();
    let mut all_dl_samples: Vec<f64> = Vec::new();
    let mut all_ul_samples: Vec<f64> = Vec::new();

    for p in &successful {
        if let Some(ref samples) = p.bandwidth_samples {
            let download = statistics::sanitize(&samples.download);
            if !download.is_empty() {
                let acc = statistics::accurate_bandwidth(&download);
                let var = statistics::variance(&download);
                if acc > 0.0 {
                    dl_candidates.push(MergeCandidate {
                        provider: p.provider.clone(),
                        value: acc,
                        variance: var,
                        samples: download.len(),
                    });
                }
                all_dl_samples.extend_from_slice(&download);
            }
            let upload = statistics::sanitize(&samples.upload);
            if !upload.is_empty() {
                let acc = statistics::accurate_upload_bandwidth(&upload);
                let var = statistics::variance(&upload);
                if acc > 0.0 {
                    ul_candidates.push(MergeCandidate {
                        provider: p.provider.clone(),
                        value: acc,
                        variance: var,
                        samples: upload.len(),
                    });
                }
                all_ul_samples.extend_from_slice(&upload);
            }
        }
        // Fallback: use the provider-reported value if no raw samples. It
        // carries zero samples and an unknown (0.0) variance, so it is
        // excluded whenever a sampled provider qualifies, and least-trusted
        // when it does participate.
        if p.bandwidth_samples
            .as_ref()
            .is_none_or(|s| s.download.is_empty())
        {
            if let Some(dl) = p.download_mbps {
                if dl.is_finite() && dl > 0.0 {
                    dl_candidates.push(MergeCandidate {
                        provider: p.provider.clone(),
                        value: dl,
                        variance: 0.0,
                        samples: 0,
                    });
                }
            }
        }
        if p.bandwidth_samples
            .as_ref()
            .is_none_or(|s| s.upload.is_empty())
        {
            if let Some(ul) = p.upload_mbps {
                if ul.is_finite() && ul > 0.0 {
                    ul_candidates.push(MergeCandidate {
                        provider: p.provider.clone(),
                        value: ul,
                        variance: 0.0,
                        samples: 0,
                    });
                }
            }
        }
    }

    // ── Minimum-sample floor, then inverse-variance merge ──────────
    let mut exclusions: Vec<MergeExclusion> = Vec::new();
    let provider_dl = select_for_merge(dl_candidates, "download", &mut exclusions);
    let provider_ul = select_for_merge(ul_candidates, "upload", &mut exclusions);

    let download = inverse_variance_merge_many(&provider_dl);

    let upload = inverse_variance_merge_many(&provider_ul);

    // ── Bootstrap confidence intervals on the pooled sample sets ───
    let download_ci = (all_dl_samples.len() >= MIN_CI_SAMPLES).then(|| {
        statistics::bootstrap_ci(&all_dl_samples, statistics::accurate_bandwidth, 1000, 0.05)
    });
    let upload_ci = (all_ul_samples.len() >= MIN_CI_SAMPLES).then(|| {
        statistics::bootstrap_ci(
            &all_ul_samples,
            statistics::accurate_upload_bandwidth,
            1000,
            0.05,
        )
    });
    let confidence = if download_ci.is_some() || upload_ci.is_some() {
        Some(ConfidenceIntervals {
            download: download_ci,
            upload: upload_ci,
            confidence_level: 0.95,
        })
    } else {
        None
    };

    // ── Latency: confidence-weighted merge (CF 0.4 / NDT7 0.6) ─────
    let cf_ping = successful
        .iter()
        .find(|p| p.provider == "Cloudflare")
        .and_then(|p| p.ping_ms);
    let ndt_ping = successful
        .iter()
        .find(|p| p.provider == "M-Lab NDT7")
        .and_then(|p| p.ping_ms);
    let cf_jitter = successful
        .iter()
        .find(|p| p.provider == "Cloudflare")
        .and_then(|p| p.jitter_ms);
    let ndt_jitter = successful
        .iter()
        .find(|p| p.provider == "M-Lab NDT7")
        .and_then(|p| p.jitter_ms);

    let ping = match (cf_ping, ndt_ping) {
        (Some(cf), Some(ndt)) => Some(statistics::weighted_merge(cf, ndt, CF_LATENCY_WEIGHT)),
        (Some(cf), None) => Some(cf),
        (None, Some(ndt)) => Some(ndt),
        (None, None) => {
            // Fallback: minimum across providers, but prefer those that also
            // report jitter — jitter presence means the ping came from a
            // multi-probe measurement, not a single noisy sample that could
            // otherwise become the global minimum.
            let min_of = |require_jitter: bool| {
                successful
                    .iter()
                    .filter(|p| !require_jitter || p.jitter_ms.is_some())
                    .filter_map(|p| p.ping_ms)
                    .filter(|p| *p > 0.0)
                    .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            };
            min_of(true).or_else(|| min_of(false))
        }
    };

    let jitter = match (cf_jitter, ndt_jitter) {
        (Some(cf), Some(ndt)) => Some(statistics::weighted_merge(cf, ndt, CF_LATENCY_WEIGHT)),
        (Some(cf), None) => Some(cf),
        (None, Some(ndt)) => Some(ndt),
        (None, None) => {
            let jitters: Vec<f64> = successful
                .iter()
                .filter_map(|p| p.jitter_ms)
                .filter(|j| *j > 0.0)
                .collect();
            if jitters.is_empty() {
                None
            } else {
                Some(statistics::mean(&jitters))
            }
        }
    };

    // Packet loss from Cloudflare (only provider that measures it)
    let packet_loss = successful
        .iter()
        .find(|p| p.provider == "Cloudflare")
        .and_then(|p| p.packet_loss_pct);

    // ── Stability metrics ──────────────────────────────────────────
    let stability = if all_dl_samples.len() > 2 || all_ul_samples.len() > 2 {
        let dl_cv = statistics::coefficient_of_variation(&all_dl_samples);
        let ul_cv = statistics::coefficient_of_variation(&all_ul_samples);
        Some(StabilityMetrics {
            download_cv: dl_cv,
            upload_cv: ul_cv,
            download_stable: dl_cv < 0.15,
            upload_stable: ul_cv < 0.15,
        })
    } else {
        None
    };

    // ── Provider divergence ────────────────────────────────────────
    let divergence = if provider_dl.len() >= 2 || provider_ul.len() >= 2 {
        let dl_div = divergence_spread(&provider_dl);
        let ul_div = divergence_spread(&provider_ul);
        Some(ProviderDivergence {
            download: dl_div,
            upload: ul_div,
            significant: dl_div > DIVERGENCE_THRESHOLD || ul_div > DIVERGENCE_THRESHOLD,
        })
    } else {
        None
    };

    AggregateResult {
        ping,
        jitter,
        download,
        upload,
        packet_loss,
        stability,
        divergence,
        confidence,
        exclusions,
    }
}

/// Callback type for provider completion notifications.
pub type ProviderCompleteCallback = Arc<dyn Fn(&ProviderResult) + Send + Sync>;

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

    // Cloudflare (always runs)
    {
        let pg = progress.clone();
        let cf_result = cloudflare::run(&config, move |phase, p| pg(phase, p)).await;
        if let Some(ref cb) = on_provider_complete {
            cb(&cf_result);
        }
        providers.push(cf_result);
    }

    // M-Lab NDT7 (always runs)
    {
        let pg = progress.clone();
        let ndt_result = ndt7::run(&config, move |phase, p| pg(phase, p)).await;
        if let Some(ref cb) = on_provider_complete {
            cb(&ndt_result);
        }
        providers.push(ndt_result);
    }

    // LibreSpeed + fast.com (only in All mode)
    if config.provider_set == ProviderSet::All {
        {
            let pg = progress.clone();
            let ls_result = librespeed::run(&config, move |phase, p| pg(phase, p)).await;
            if let Some(ref cb) = on_provider_complete {
                cb(&ls_result);
            }
            providers.push(ls_result);
        }

        {
            let pg = progress.clone();
            let fc_result = fastcom::run(&config, move |phase, p| pg(phase, p)).await;
            if let Some(ref cb) = on_provider_complete {
                cb(&fc_result);
            }
            providers.push(fc_result);
        }
    }

    progress(Phase::Computing, 1.0);

    let agg = aggregate(&providers);
    let duration = start.elapsed().as_secs_f64();

    SpeedTestResult {
        ping_ms: agg.ping,
        jitter_ms: agg.jitter,
        download_mbps: agg.download,
        upload_mbps: agg.upload,
        packet_loss_pct: agg.packet_loss,
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

    fn provider(name: &str, download: f64, upload: f64, variance: f64) -> ProviderResult {
        let delta = variance.sqrt();
        ProviderResult {
            provider: name.to_string(),
            server: "test".to_string(),
            location: None,
            ping_ms: None,
            jitter_ms: None,
            download_mbps: Some(download),
            upload_mbps: Some(upload),
            download_bytes: 1,
            upload_bytes: 1,
            download_duration_s: 1.0,
            upload_duration_s: 1.0,
            packet_loss_pct: None,
            error: None,
            bandwidth_samples: Some(BandwidthSamples {
                download: vec![download - delta, download, download + delta, download],
                upload: vec![upload - delta, upload, upload + delta, upload],
            }),
        }
    }

    #[test]
    fn aggregate_uses_more_than_first_two_providers() {
        let first_two = vec![
            provider("Cloudflare", 100.0, 20.0, 4.0),
            provider("M-Lab NDT7", 100.0, 20.0, 4.0),
        ];
        let with_four = vec![
            provider("Cloudflare", 100.0, 20.0, 4.0),
            provider("M-Lab NDT7", 100.0, 20.0, 4.0),
            provider("LibreSpeed", 900.0, 180.0, 4.0),
            provider("fast.com", 900.0, 180.0, 4.0),
        ];

        let two = aggregate(&first_two);
        let four = aggregate(&with_four);

        assert!(
            four.download > two.download + 100.0,
            "third/fourth providers should materially influence aggregate: two={}, four={}",
            two.download,
            four.download
        );
        assert!(
            four.upload > two.upload + 20.0,
            "third/fourth providers should materially influence upload aggregate: two={}, four={}",
            two.upload,
            four.upload
        );
    }

    #[test]
    fn divergence_uses_full_provider_spread() {
        let providers = vec![
            provider("Cloudflare", 100.0, 20.0, 4.0),
            provider("M-Lab NDT7", 105.0, 22.0, 4.0),
            provider("LibreSpeed", 450.0, 90.0, 4.0),
        ];

        let agg = aggregate(&providers);
        let div = agg.divergence.expect("divergence should be reported");

        assert!(div.significant);
        assert!(
            div.download > 0.70,
            "expected divergence to use 100 vs 450 spread, got {}",
            div.download
        );
        assert!(
            div.upload > 0.70,
            "expected divergence to use 20 vs 90 spread, got {}",
            div.upload
        );
    }

    #[test]
    fn inverse_variance_merge_caps_single_provider_dominance() {
        let merged = inverse_variance_merge_many(&[(1000.0, 0.000_001), (1.0, 1000.0)]);

        assert!(
            merged < 701.0,
            "dominant provider should be capped near 70%, got {}",
            merged
        );
    }

    /// A provider whose degenerate sample set yields an unknown (0.0)
    /// variance must adopt the WORST known variance in the merge (least
    /// trusted), not the best. With known variances {4, 25}, the unknown
    /// 1000 Mbps entry gets 25; the old floor rule handed it 4 — tied for
    /// the highest weight — which pulled the merge above 500.
    #[test]
    fn nan_variance_provider_does_not_dominate_merge() {
        let merged = inverse_variance_merge_many(&[(100.0, 4.0), (102.0, 25.0), (1000.0, 0.0)]);
        assert!(
            merged < 300.0,
            "unknown-variance provider must be least-trusted; got {}",
            merged
        );

        // Non-finite variance is treated the same as unknown.
        let merged_nan =
            inverse_variance_merge_many(&[(100.0, 4.0), (102.0, 25.0), (1000.0, f64::NAN)]);
        assert!(
            merged_nan < 300.0,
            "NaN-variance provider must be least-trusted; got {}",
            merged_nan
        );
    }

    #[test]
    fn non_finite_values_filtered_from_merge() {
        let merged = inverse_variance_merge_many(&[(f64::NAN, 4.0), (100.0, 4.0)]);
        assert_eq!(merged, 100.0);
        let merged_inf = inverse_variance_merge_many(&[(f64::INFINITY, 4.0), (100.0, 4.0)]);
        assert_eq!(merged_inf, 100.0);
    }

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
        }
    }

    /// A 1-sample provider at a wild value is excluded from the merge when a
    /// well-sampled provider exists, and the exclusion is reported.
    #[test]
    fn single_sample_provider_excluded_from_merge() {
        let clean: Vec<f64> = (0..10).map(|i| 98.0 + (i % 3) as f64).collect();
        let providers = vec![
            provider_with_samples("Cloudflare", clean.clone(), clean.clone()),
            provider_with_samples("LibreSpeed", vec![1000.0], vec![1000.0]),
        ];

        let agg = aggregate(&providers);

        assert!(
            (agg.download - 99.0).abs() < 5.0,
            "merge should track the well-sampled provider, got {}",
            agg.download
        );
        assert!(
            agg.exclusions
                .iter()
                .any(|e| e.provider == "LibreSpeed" && e.direction == "download" && e.samples == 1),
            "LibreSpeed download exclusion should be recorded: {:?}",
            agg.exclusions
                .iter()
                .map(|e| (&e.provider, e.direction, e.samples))
                .collect::<Vec<_>>()
        );
    }

    /// When NO provider reaches the floor, all positive candidates still
    /// merge — never 0.0 while data exists.
    #[test]
    fn all_sparse_providers_still_merge() {
        let providers = vec![
            provider_with_samples("Cloudflare", vec![100.0, 102.0], vec![20.0, 21.0]),
            provider_with_samples("LibreSpeed", vec![110.0, 108.0], vec![22.0, 23.0]),
        ];

        let agg = aggregate(&providers);

        assert!(agg.download > 0.0, "degraded merge must not return 0.0");
        assert!(agg.exclusions.is_empty(), "no exclusions in degraded mode");
    }

    /// A no-sample fallback value participates only with least-trust
    /// weighting (and is excluded entirely when a sampled provider exists).
    #[test]
    fn fallback_value_provider_excluded_when_sampled_provider_exists() {
        let clean: Vec<f64> = (0..10).map(|i| 98.0 + (i % 3) as f64).collect();
        let mut fallback_only = provider_with_samples("fast.com", vec![], vec![]);
        fallback_only.download_mbps = Some(1000.0);
        fallback_only.upload_mbps = Some(1000.0);

        let providers = vec![
            provider_with_samples("Cloudflare", clean.clone(), clean.clone()),
            fallback_only,
        ];

        let agg = aggregate(&providers);

        assert!(
            (agg.download - 99.0).abs() < 5.0,
            "fallback-only provider must not skew the merge, got {}",
            agg.download
        );
        assert!(agg
            .exclusions
            .iter()
            .any(|e| e.provider == "fast.com" && e.samples == 0));
    }

    /// In the no-CF/no-NDT7 latency fallback, a single-probe (jitterless)
    /// provider must not become the global minimum when a multi-probe
    /// provider is available.
    #[test]
    fn latency_fallback_ignores_jitterless_single_probe() {
        let mut multi = provider_with_samples("LibreSpeed", vec![100.0; 5], vec![20.0; 5]);
        multi.ping_ms = Some(15.0);
        multi.jitter_ms = Some(1.2);
        let mut single = provider_with_samples("fast.com", vec![100.0; 5], vec![20.0; 5]);
        single.ping_ms = Some(2.0); // suspiciously low single sample
        single.jitter_ms = None;

        let agg = aggregate(&[multi, single]);

        assert_eq!(
            agg.ping,
            Some(15.0),
            "jitter-bearing provider should win the fallback"
        );
    }

    #[test]
    fn confidence_intervals_suppressed_below_threshold() {
        let providers = vec![provider_with_samples(
            "Cloudflare",
            vec![100.0, 101.0, 99.0],
            vec![20.0, 21.0],
        )];
        let agg = aggregate(&providers);
        assert!(agg.confidence.is_none());
    }

    #[test]
    fn confidence_intervals_present_with_sufficient_samples() {
        let samples: Vec<f64> = (0..12).map(|i| 95.0 + (i % 5) as f64 * 2.0).collect();
        let providers = vec![provider_with_samples(
            "Cloudflare",
            samples.clone(),
            samples.clone(),
        )];
        let agg = aggregate(&providers);
        let ci = agg.confidence.expect("CI should be computed");
        let dl = ci.download.expect("download CI present");
        assert!(dl.lower <= dl.estimate && dl.estimate <= dl.upper);
        assert_eq!(ci.confidence_level, 0.95);
    }
}
