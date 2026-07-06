//! Statistical utilities for accurate speed test measurement.
//!
//! Port of the QubeTX web speed test's `statistics.ts` module.
//!
//! Split into two eras:
//!   * **v3 estimators** (percentile, classic/modified trimean, IQR filter,
//!     slow-start discard, RFC 3550 jitter, CV, percentile bootstrap,
//!     inverse-variance merge) — retained for the current aggregation pipeline
//!     and imported by diagnostics elsewhere in the crate.
//!   * **v4 core** (SpeedQX Methodology v4): plateau warm-up detection,
//!     Hodges–Lehmann cross-check, circular block bootstrap + BCa, DerSimonian–
//!     Laird τ²/I² with HKSJ confidence intervals, capacity/consensus hybrid
//!     merge, PDV/IPDV/MAD jitter, delta-ms bufferbloat + grade + RPM, and the
//!     empirical-Bernstein confidence sequence for FAST-mode early stopping.
//!
//! The v4 functions build only on the pinned primitives in [`stat_primitives`]
//! so the TypeScript and Rust implementations produce identical golden vectors.

use super::stat_primitives::{
    inv_normal, phi, quantile, sample_mean, sample_variance, sum, t975, Pcg32,
};
use serde::Serialize;

// ── Percentile ──────────────────────────────────────────────────────────

/// Linear-interpolation percentile on a pre-sorted slice. `p` is in [0.0, 1.0].
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let idx = p * (sorted.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    sorted[lo] + (sorted[hi] - sorted[lo]) * (idx - lo as f64)
}

// ── Sanitization ────────────────────────────────────────────────────────

/// Drop non-finite samples (NaN / ±inf). The single choke point through
/// which every merge input flows: provider arithmetic is individually
/// guarded, but a corrupted sample must never reach the trimean pipeline or
/// the inverse-variance merge.
pub fn sanitize(values: &[f64]) -> Vec<f64> {
    values.iter().copied().filter(|v| v.is_finite()).collect()
}

// ── Central tendency ────────────────────────────────────────────────────

pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile(&sorted, 0.5)
}

/// Sample standard deviation (Bessel's correction).
pub fn stddev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    variance(values).sqrt()
}

/// Sample variance (Bessel's correction).
pub fn variance(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let m = mean(values);
    values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (values.len() - 1) as f64
}

// ── Trimean ─────────────────────────────────────────────────────────────

/// Classic Tukey trimean: `(Q1 + 2*median + Q3) / 4`.
pub fn trimean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q1 = percentile(&sorted, 0.25);
    let q2 = percentile(&sorted, 0.50);
    let q3 = percentile(&sorted, 0.75);
    (q1 + 2.0 * q2 + q3) / 4.0
}

/// Ookla-style modified trimean: `(P10 + 8*P50 + P90) / 10`.
pub fn modified_trimean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p10 = percentile(&sorted, 0.10);
    let p50 = percentile(&sorted, 0.50);
    let p90 = percentile(&sorted, 0.90);
    (p10 + 8.0 * p50 + p90) / 10.0
}

// ── Outlier filtering ───────────────────────────────────────────────────

/// Remove values outside `[Q1 - k*IQR, Q3 + k*IQR]`. Default `k = 1.5`.
pub fn filter_outliers_iqr(values: &[f64], k: f64) -> Vec<f64> {
    if values.len() < 4 {
        return values.to_vec();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q1 = percentile(&sorted, 0.25);
    let q3 = percentile(&sorted, 0.75);
    let iqr = q3 - q1;
    let lo = q1 - k * iqr;
    let hi = q3 + k * iqr;
    values
        .iter()
        .copied()
        .filter(|v| *v >= lo && *v <= hi)
        .collect()
}

// ── Slow-start discard ──────────────────────────────────────────────────

/// Discard the first `fraction` of samples to eliminate TCP slow-start
/// ramp-up contamination. Default: discard first 30%.
pub fn discard_slow_start(values: &[f64], fraction: f64) -> Vec<f64> {
    if values.len() < 4 {
        return values.to_vec();
    }
    let cut = (values.len() as f64 * fraction).ceil() as usize;
    values[cut..].to_vec()
}

// ── Winsorization ───────────────────────────────────────────────────────

/// Cap extreme values at the given percentiles instead of removing them.
pub fn winsorize(values: &[f64], lower: f64, upper: f64) -> Vec<f64> {
    if values.len() < 4 {
        return values.to_vec();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let lo = percentile(&sorted, lower);
    let hi = percentile(&sorted, upper);
    values.iter().map(|v| v.max(lo).min(hi)).collect()
}

// ── Bandwidth pipelines ─────────────────────────────────────────────────

/// Full accuracy pipeline for download bandwidth samples:
/// 1. Discard slow-start ramp-up (first 30%)
/// 2. Remove IQR outliers
/// 3. Compute modified trimean
/// 4. Cross-validate with Winsorized trimean (average if >15% divergence)
pub fn accurate_bandwidth(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let after_slow_start = discard_slow_start(samples, 0.3);

    // Primary: IQR-filtered trimean
    let cleaned = filter_outliers_iqr(&after_slow_start, 1.5);
    let iqr_result = if cleaned.is_empty() {
        modified_trimean(&after_slow_start)
    } else {
        modified_trimean(&cleaned)
    };

    // Cross-check: Winsorized trimean
    if after_slow_start.len() >= 4 {
        let winsorized = winsorize(&after_slow_start, 0.05, 0.95);
        let win_result = modified_trimean(&winsorized);

        if iqr_result > 0.0 && win_result > 0.0 {
            let divergence = (iqr_result - win_result).abs() / iqr_result.max(win_result);
            if divergence > 0.15 {
                return (iqr_result + win_result) / 2.0;
            }
        }
    }

    iqr_result
}

/// Upload-specific accuracy pipeline.
/// Upload ramp-up is slower and more variable than download. Following
/// Speedtest.net's methodology, we keep only the fastest 50% of post-warmup
/// samples before computing the trimean.
///
/// Pipeline:
/// 1. Discard slow-start ramp-up (first 30%)
/// 2. Keep only the fastest 50% of remaining samples
/// 3. Remove IQR outliers
/// 4. Compute modified trimean + Winsorized cross-validation
pub fn accurate_upload_bandwidth(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let after_slow_start = discard_slow_start(samples, 0.3);
    if after_slow_start.len() < 2 {
        return accurate_bandwidth(samples);
    }

    // Keep fastest 50% (sort descending, take top half)
    let mut sorted_desc = after_slow_start.clone();
    sorted_desc.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let top_half_count = (sorted_desc.len() as f64 / 2.0).ceil() as usize;
    let top_half: Vec<f64> = sorted_desc[..top_half_count].to_vec();

    // Primary: IQR filter → modified trimean
    let cleaned = filter_outliers_iqr(&top_half, 1.5);
    let iqr_result = if cleaned.is_empty() {
        modified_trimean(&top_half)
    } else {
        modified_trimean(&cleaned)
    };

    // Cross-check: Winsorized trimean on same top-half set
    if top_half.len() >= 4 {
        let winsorized = winsorize(&top_half, 0.05, 0.95);
        let win_result = modified_trimean(&winsorized);

        if iqr_result > 0.0 && win_result > 0.0 {
            let divergence = (iqr_result - win_result).abs() / iqr_result.max(win_result);
            if divergence > 0.15 {
                return (iqr_result + win_result) / 2.0;
            }
        }
    }

    iqr_result
}

// ── Jitter ──────────────────────────────────────────────────────────────

/// RFC 3550 jitter: exponentially weighted moving average of inter-arrival
/// variance. `J[i] = J[i-1] + (|D(i-1,i)| - J[i-1]) / 16`
pub fn jitter_rfc3550(samples: &[f64]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mut j = 0.0_f64;
    for i in 1..samples.len() {
        let d = (samples[i] - samples[i - 1]).abs();
        j += (d - j) / 16.0;
    }
    j
}

/// Mean absolute deviation of consecutive samples (original method).
pub fn jitter_mad(samples: &[f64]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let sum: f64 = samples.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
    sum / (samples.len() - 1) as f64
}

// ── Stability ───────────────────────────────────────────────────────────

/// Coefficient of variation (stddev / mean). Lower = more stable.
pub fn coefficient_of_variation(values: &[f64]) -> f64 {
    let m = mean(values);
    if m == 0.0 {
        return 0.0;
    }
    stddev(values) / m
}

// ── Confidence-weighted merge ───────────────────────────────────────────

/// Weighted average of two values. If one is zero/missing, return the other.
/// `weight_a` is the weight for `a`; `b` gets `1 - weight_a`.
pub fn weighted_merge(a: f64, b: f64, weight_a: f64) -> f64 {
    let has_a = a > 0.0;
    let has_b = b > 0.0;
    if has_a && has_b {
        a * weight_a + b * (1.0 - weight_a)
    } else if has_a {
        a
    } else {
        b
    }
}

// ── Inverse-variance merge ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct InverseVarianceResult {
    pub value: f64,
    pub weight_a: f64,
    pub weight_b: f64,
}

/// Inverse-variance weighted merge of two estimates.
/// Minimum-variance unbiased estimator for combining independent measurements.
/// Weights clamped to [0.3, 0.7] to prevent one source from dominating.
pub fn inverse_variance_merge(a: f64, var_a: f64, b: f64, var_b: f64) -> InverseVarianceResult {
    if a <= 0.0 && b <= 0.0 {
        return InverseVarianceResult {
            value: 0.0,
            weight_a: 0.5,
            weight_b: 0.5,
        };
    }
    if a <= 0.0 {
        return InverseVarianceResult {
            value: b,
            weight_a: 0.0,
            weight_b: 1.0,
        };
    }
    if b <= 0.0 {
        return InverseVarianceResult {
            value: a,
            weight_a: 1.0,
            weight_b: 0.0,
        };
    }
    if var_a <= 0.0 && var_b <= 0.0 {
        return InverseVarianceResult {
            value: (a + b) / 2.0,
            weight_a: 0.5,
            weight_b: 0.5,
        };
    }
    if var_a <= 0.0 {
        return InverseVarianceResult {
            value: a,
            weight_a: 1.0,
            weight_b: 0.0,
        };
    }
    if var_b <= 0.0 {
        return InverseVarianceResult {
            value: b,
            weight_a: 0.0,
            weight_b: 1.0,
        };
    }

    let w_a = 1.0 / var_a;
    let w_b = 1.0 / var_b;
    let total = w_a + w_b;
    let mut weight_a = w_a / total;
    let mut weight_b = w_b / total;

    // Clamp to [0.3, 0.7] to prevent degenerate weighting
    if weight_a < 0.3 {
        weight_a = 0.3;
        weight_b = 0.7;
    } else if weight_a > 0.7 {
        weight_a = 0.7;
        weight_b = 0.3;
    }

    InverseVarianceResult {
        value: a * weight_a + b * weight_b,
        weight_a,
        weight_b,
    }
}

// ── Bootstrap confidence interval ──────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapCI {
    pub estimate: f64,
    pub lower: f64,
    pub upper: f64,
    pub margin: f64,
}

/// Simple xorshift64 PRNG for bootstrap resampling. Deterministic given seed.
struct Xorshift64(u64);

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        // Ensure non-zero seed
        Self(if seed == 0 { 0x517cc1b727220a95 } else { seed })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_usize(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// Bootstrap confidence interval via percentile method.
/// Resamples the data `b` times, computes the statistic on each,
/// then takes the alpha/2 and 1-alpha/2 percentiles as CI bounds.
pub fn bootstrap_ci(
    samples: &[f64],
    stat_fn: fn(&[f64]) -> f64,
    b: usize,
    alpha: f64,
) -> BootstrapCI {
    if samples.len() < 4 {
        let est = stat_fn(samples);
        return BootstrapCI {
            estimate: est,
            lower: est,
            upper: est,
            margin: 0.0,
        };
    }

    let estimate = stat_fn(samples);

    // Seed from sample data for deterministic results
    let seed = samples.iter().fold(0u64, |acc, v| {
        acc.wrapping_add(v.to_bits())
            .wrapping_mul(6364136223846793005)
    });
    let mut rng = Xorshift64::new(seed);

    let n = samples.len();
    let mut bootstrap_stats: Vec<f64> = Vec::with_capacity(b);
    let mut resample = vec![0.0_f64; n];

    for _ in 0..b {
        for val in resample.iter_mut() {
            *val = samples[rng.next_usize(n)];
        }
        bootstrap_stats.push(stat_fn(&resample));
    }

    bootstrap_stats.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let lower = percentile(&bootstrap_stats, alpha / 2.0);
    let upper = percentile(&bootstrap_stats, 1.0 - alpha / 2.0);

    BootstrapCI {
        estimate,
        lower,
        upper,
        margin: (upper - lower) / 2.0,
    }
}

// ════════════════════════════════════════════════════════════════════════
// SpeedQX Methodology v4 core
// ════════════════════════════════════════════════════════════════════════

/// Minimum cleaned samples for a provider to qualify for the cross-provider merge.
pub const MIN_MERGE_SAMPLES: usize = 4;

/// Capability prior for a provider (METHODOLOGY.md §3), keyed by lowercase
/// registry name. Returns `None` for an unknown provider (caller defaults to 1.0).
pub fn capability_prior(name: &str) -> Option<f64> {
    match name {
        "cloudflare" | "applenq" | "fastcom" => Some(1.0),
        "librespeed" | "cachefly" | "vultr" => Some(0.95),
        "msak" => Some(0.85),
        "ndt7" => Some(0.70),
        _ => None,
    }
}

/// Median on a fresh sorted copy via the type-7 [`quantile`] primitive.
/// Mirrors the TS `medianOf` helper (used by plateau/HL/MAD), which is
/// bit-identical to [`median`] but kept separate to track the reference.
fn median_of(values: &[f64]) -> f64 {
    let mut s = values.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    quantile(&s, 0.5)
}

// ── Plateau warm-up detector ─────────────────────────────────────────────

/// Warm-up cut index (replaces the fixed 30% slow-start discard).
///
/// Steady state begins at the first index `t` where 3 consecutive samples sit
/// within ±10% of the forward median `median(samples[t..end])`; the cut is
/// clamped to `[ceil(0.10·n), floor(0.40·n)]`. For `n < 8` (too few to detect)
/// returns `ceil(0.30·n)`. Discard `samples[0 .. plateau_start)`.
pub fn plateau_start(samples: &[f64]) -> usize {
    let n = samples.len();
    if n < 8 {
        return (0.30_f64 * n as f64).ceil() as usize;
    }

    let eps = 0.10;
    let w_len = 3;
    let mut t_star: i64 = -1;

    for t in 0..=(n - w_len) {
        let ref_med = median_of(&samples[t..]);
        if ref_med <= 0.0 {
            continue;
        }
        let mut ok = true;
        for &s in &samples[t..t + w_len] {
            if (s - ref_med).abs() / ref_med >= eps {
                ok = false;
                break;
            }
        }
        if ok {
            t_star = t as i64;
            break;
        }
    }
    if t_star < 0 {
        t_star = (0.30_f64 * n as f64).ceil() as i64;
    }

    let lo = (0.10_f64 * n as f64).ceil() as i64;
    let hi = (0.40_f64 * n as f64).floor() as i64;
    t_star.max(lo).min(hi) as usize
}

// ── Hodges–Lehmann estimator ─────────────────────────────────────────────

/// Hodges–Lehmann location: median of all Walsh averages `(x_i + x_j)/2` for
/// `i ≤ j`. Internal robustness cross-check against the trimean.
pub fn hodges_lehmann(values: &[f64]) -> f64 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return values[0];
    }
    let mut walsh = Vec::with_capacity(n * (n + 1) / 2);
    for i in 0..n {
        for &vj in &values[i..] {
            walsh.push((values[i] + vj) / 2.0);
        }
    }
    walsh.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    quantile(&walsh, 0.5)
}

// ── Circular block bootstrap + BCa ───────────────────────────────────────

/// Result of the circular block bootstrap of the modified trimean.
#[derive(Debug, Clone, Serialize)]
pub struct BlockBootstrapResult {
    /// Point estimate: `modified_trimean(cleaned)`.
    pub theta_hat: f64,
    /// Mean of the `B` resample trimeans.
    pub theta_star_mean: f64,
    /// Bootstrap variance of the trimean (`v_j` for the merge; `n-1` denominator).
    pub variance: f64,
    /// BCa lower bound (2.5%).
    pub ci_lower: f64,
    /// BCa upper bound (97.5%).
    pub ci_upper: f64,
    /// Block length `ℓ = max(2, round(n^(1/3)))`.
    pub block_length: usize,
    /// Resample count `B`.
    pub b: usize,
}

fn bca_bound(sorted_theta_star: &[f64], z0: f64, a: f64, alpha: f64) -> f64 {
    let z = inv_normal(alpha);
    let denom = 1.0 - a * (z0 + z);
    let adj = if denom != 0.0 {
        z0 + (z0 + z) / denom
    } else {
        z0
    };
    let mut aa = phi(adj);
    if !aa.is_finite() {
        aa = alpha;
    }
    // `aa` is finite here, so clamp is bit-identical to the TS `min(max(aa,0),1)`.
    aa = aa.clamp(0.0, 1.0);
    quantile(sorted_theta_star, aa)
}

/// Circular block bootstrap of the modified trimean with a BCa 95% interval.
///
/// Block length `ℓ = max(2, round(n^(1/3)))`; `num_blocks = ceil(n/ℓ)`; each
/// resample concatenates whole circular blocks (start via PCG32 + Lemire,
/// wrapping mod `n`) and is trimmed to `n`. The `rng` stream is caller-owned so
/// the orchestrator can thread ONE deterministic stream across DL then UL, per
/// provider in registry order. Returns the bootstrap variance for the merge.
pub fn circular_block_bootstrap(
    cleaned: &[f64],
    rng: &mut Pcg32,
    b_count: usize,
) -> BlockBootstrapResult {
    let n = cleaned.len();
    let theta_hat = modified_trimean(cleaned);
    if n < 2 {
        return BlockBootstrapResult {
            theta_hat,
            theta_star_mean: theta_hat,
            variance: 0.0,
            ci_lower: theta_hat,
            ci_upper: theta_hat,
            block_length: n,
            b: b_count,
        };
    }

    let l = 2.max((n as f64).cbrt().round() as usize);
    let num_blocks = n.div_ceil(l);
    let mut theta_star = vec![0.0_f64; b_count];
    let mut resample = vec![0.0_f64; n];

    for slot in theta_star.iter_mut() {
        let mut filled = 0usize;
        let mut blk = 0usize;
        while blk < num_blocks && filled < n {
            let start = rng.bounded_index(n);
            let mut t = 0usize;
            while t < l && filled < n {
                resample[filled] = cleaned[(start + t) % n];
                filled += 1;
                t += 1;
            }
            blk += 1;
        }
        *slot = modified_trimean(&resample);
    }

    let theta_star_mean = sample_mean(&theta_star);
    let boot_var = sample_variance(&theta_star);
    let mut sorted_ts = theta_star.clone();
    sorted_ts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    if boot_var == 0.0 {
        return BlockBootstrapResult {
            theta_hat,
            theta_star_mean,
            variance: 0.0,
            ci_lower: theta_hat,
            ci_upper: theta_hat,
            block_length: l,
            b: b_count,
        };
    }

    // Bias-correction z0.
    let mut count_less = 0usize;
    for &ts in &theta_star {
        if ts < theta_hat {
            count_less += 1;
        }
    }
    const EPS: f64 = 1e-12;
    // The ratio is finite in [0, 1], so clamp matches the TS `min(max(p,EPS),1-EPS)`.
    let prop = (count_less as f64 / b_count as f64).clamp(EPS, 1.0 - EPS);
    let z0 = inv_normal(prop);

    // Acceleration a via jackknife of the trimean.
    let mut jack = vec![0.0_f64; n];
    let mut loo = vec![0.0_f64; n - 1];
    for (i, jack_i) in jack.iter_mut().enumerate() {
        let mut idx = 0usize;
        for (j, &c) in cleaned.iter().enumerate() {
            if j != i {
                loo[idx] = c;
                idx += 1;
            }
        }
        *jack_i = modified_trimean(&loo);
    }
    let jack_mean = sample_mean(&jack);
    let mut s2 = 0.0_f64;
    let mut s3 = 0.0_f64;
    for &jk in &jack {
        let d = jack_mean - jk;
        s2 += d * d;
        s3 += d * d * d;
    }
    let a_den = 6.0 * s2.powf(1.5);
    let a = if a_den != 0.0 { s3 / a_den } else { 0.0 };

    let ci_lower = bca_bound(&sorted_ts, z0, a, 0.025);
    let ci_upper = bca_bound(&sorted_ts, z0, a, 0.975);
    BlockBootstrapResult {
        theta_hat,
        theta_star_mean,
        variance: boot_var,
        ci_lower,
        ci_upper,
        block_length: l,
        b: b_count,
    }
}

// ── Cross-provider hybrid merge (DL τ²/I² + HKSJ + capacity/consensus) ────

/// Provider agreement band derived from `I²` (METHODOLOGY.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgreementBand {
    High,
    Moderate,
    Low,
    VeryLow,
    Insufficient,
}

impl AgreementBand {
    /// Canonical wire string (`high`/`moderate`/`low`/`very-low`/`insufficient`).
    pub fn as_str(self) -> &'static str {
        match self {
            AgreementBand::High => "high",
            AgreementBand::Moderate => "moderate",
            AgreementBand::Low => "low",
            AgreementBand::VeryLow => "very-low",
            AgreementBand::Insufficient => "insufficient",
        }
    }
}

/// A provider's own BCa interval (used verbatim when `k == 1`).
#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize)]
pub struct BcaInterval {
    pub lower: f64,
    pub upper: f64,
}

/// Confidence-interval bounds for a merged estimate.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CiBounds {
    pub lower: f64,
    pub upper: f64,
}

/// One provider-direction input to [`merge_providers`].
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MergeProviderInput {
    /// Lowercase registry name (looks up [`capability_prior`]).
    pub name: String,
    /// Point estimate (modified trimean) for this direction.
    pub y: f64,
    /// Bootstrap variance `v_j`; `None`/non-finite/≤0 is treated as "unknown".
    #[serde(default)]
    pub v: Option<f64>,
    /// Cleaned sample count in this direction (qualification gate).
    pub samples: usize,
    /// Optional capability override; else `capability_prior(name)` ?? 1.0.
    #[serde(default)]
    pub capability: Option<f64>,
    /// Provider's own BCa interval, used verbatim when `k == 1`.
    #[serde(default)]
    pub bca: Option<BcaInterval>,
}

/// A provider direction excluded from the merge for insufficient samples.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergeExclusion {
    pub name: String,
    pub samples: usize,
}

/// Per-provider weight diagnostics from the merge.
#[derive(Debug, Clone, Serialize)]
pub struct MergeWeight {
    pub name: String,
    pub y: f64,
    /// Effective variance (unknown → max known).
    pub v: f64,
    pub w_star: f64,
    pub w_star_capped: f64,
    pub w_cap: f64,
}

/// Full result of the SpeedQX hybrid cross-provider merge for one direction.
#[derive(Debug, Clone, Serialize)]
pub struct MergeResult {
    /// Qualifying provider count (`samples ≥ MIN_MERGE_SAMPLES`).
    pub k: usize,
    /// Headline: capability-weighted top-tier robust mean.
    pub capacity: f64,
    /// Secondary: DL random-effects mean over all qualifying providers.
    pub consensus: f64,
    pub capacity_ci: CiBounds,
    pub consensus_ci: CiBounds,
    /// DerSimonian–Laird between-provider variance τ².
    pub tau2: f64,
    /// I² heterogeneity (`None` when `k < 2`).
    pub i2: Option<f64>,
    /// Cochran's Q (diagnostic).
    pub q: f64,
    pub band: AgreementBand,
    /// Provider names in the capacity tier.
    pub tier: Vec<String>,
    pub weights: Vec<MergeWeight>,
    pub exclusions: Vec<MergeExclusion>,
}

/// Whether a variance is a usable known value (finite and strictly positive).
fn known_variance(v: Option<f64>) -> Option<f64> {
    match v {
        Some(x) if x.is_finite() && x > 0.0 => Some(x),
        _ => None,
    }
}

fn empty_merge(exclusions: Vec<MergeExclusion>) -> MergeResult {
    MergeResult {
        k: 0,
        capacity: 0.0,
        consensus: 0.0,
        capacity_ci: CiBounds {
            lower: 0.0,
            upper: 0.0,
        },
        consensus_ci: CiBounds {
            lower: 0.0,
            upper: 0.0,
        },
        tau2: 0.0,
        i2: None,
        q: 0.0,
        band: AgreementBand::Insufficient,
        tier: Vec::new(),
        weights: Vec::new(),
        exclusions,
    }
}

/// SpeedQX hybrid cross-provider merge for one direction (METHODOLOGY.md §6).
///
/// Qualification: `samples ≥ MIN_MERGE_SAMPLES`; the rest are recorded in
/// `exclusions`. Unknown-variance qualifiers adopt the maximum known variance
/// (least trusted); if no variance is known, all weights are equal.
///
/// * `k = 0` → empty result. `k = 1` → passthrough with the provider's own BCa CI.
/// * `k = 2` → capacity/consensus points computed, but CI is the honest union
///   band `[min(y − 1.96·se), max(y + 1.96·se)]` and agreement = "insufficient".
/// * `k ≥ 3` → DL τ²/I², capped (0.70) random-effects consensus with HKSJ CI,
///   and the capability-weighted capacity tier (`y ≥ 0.85·max`, ≥ 2 members)
///   with its own HKSJ CI over the tier.
pub fn merge_providers(inputs: &[MergeProviderInput]) -> MergeResult {
    let mut exclusions: Vec<MergeExclusion> = Vec::new();
    let mut qualifying: Vec<&MergeProviderInput> = Vec::new();
    for p in inputs {
        if p.samples >= MIN_MERGE_SAMPLES {
            qualifying.push(p);
        } else {
            exclusions.push(MergeExclusion {
                name: p.name.clone(),
                samples: p.samples,
            });
        }
    }
    let k = qualifying.len();
    if k == 0 {
        return empty_merge(exclusions);
    }

    // Effective variances: unknown → max known; none known → 1 (equal weights).
    let known_vs: Vec<f64> = qualifying
        .iter()
        .filter_map(|p| known_variance(p.v))
        .collect();
    let max_known_v = known_vs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let v_eff: Vec<f64> = qualifying
        .iter()
        .map(|p| {
            known_variance(p.v).unwrap_or(if known_vs.is_empty() {
                1.0
            } else {
                max_known_v
            })
        })
        .collect();
    let capability: Vec<f64> = qualifying
        .iter()
        .map(|p| {
            p.capability
                .unwrap_or_else(|| capability_prior(&p.name).unwrap_or(1.0))
        })
        .collect();

    if k == 1 {
        let p = qualifying[0];
        let (lo, hi) = match p.bca {
            Some(b) => (b.lower, b.upper),
            None => (p.y, p.y),
        };
        let w_star = 1.0 / v_eff[0];
        return MergeResult {
            k: 1,
            capacity: p.y,
            consensus: p.y,
            capacity_ci: CiBounds {
                lower: lo,
                upper: hi,
            },
            consensus_ci: CiBounds {
                lower: lo,
                upper: hi,
            },
            tau2: 0.0,
            i2: None,
            q: 0.0,
            band: AgreementBand::Insufficient,
            tier: vec![p.name.clone()],
            weights: vec![MergeWeight {
                name: p.name.clone(),
                y: p.y,
                v: v_eff[0],
                w_star,
                w_star_capped: w_star,
                w_cap: capability[0] / v_eff[0],
            }],
            exclusions,
        };
    }

    // DerSimonian–Laird heterogeneity (k ≥ 2).
    let w: Vec<f64> = v_eff.iter().map(|v| 1.0 / v).collect();
    let sum_w = sum(&w);
    let mu_f = {
        let terms: Vec<f64> = qualifying
            .iter()
            .enumerate()
            .map(|(i, p)| w[i] * p.y)
            .collect();
        sum(&terms) / sum_w
    };
    let q = {
        let terms: Vec<f64> = qualifying
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let d = p.y - mu_f;
                w[i] * (d * d)
            })
            .collect();
        sum(&terms)
    };
    let sum_w2 = {
        let terms: Vec<f64> = w.iter().map(|x| x * x).collect();
        sum(&terms)
    };
    let c = sum_w - sum_w2 / sum_w;
    let tau2 = if c > 0.0 {
        (0.0_f64).max((q - (k - 1) as f64) / c)
    } else {
        0.0
    };
    let i2 = if q > 0.0 {
        (0.0_f64).max((q - (k - 1) as f64) / q)
    } else {
        0.0
    };

    // Random-effects weights with a single 0.70·Σ cap (defense in depth).
    let w_star: Vec<f64> = v_eff.iter().map(|v| 1.0 / (v + tau2)).collect();
    let sum_w_star = sum(&w_star);
    let cap = 0.70 * sum_w_star;
    let w_star_capped: Vec<f64> = w_star.iter().map(|x| x.min(cap)).collect();
    let sum_w_star_capped = sum(&w_star_capped);
    let consensus = {
        let terms: Vec<f64> = qualifying
            .iter()
            .enumerate()
            .map(|(i, p)| w_star_capped[i] * p.y)
            .collect();
        sum(&terms) / sum_w_star_capped
    };

    // Capacity tier: y ≥ 0.85·max; if k ≥ 3 and fewer than 2, take the top-2 by y.
    let ymax = qualifying
        .iter()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut tier_idx: Vec<usize> = Vec::new();
    for (i, p) in qualifying.iter().enumerate() {
        if p.y >= 0.85 * ymax {
            tier_idx.push(i);
        }
    }
    if k >= 3 && tier_idx.len() < 2 {
        let mut order: Vec<usize> = (0..k).collect();
        order.sort_by(|&i, &j| {
            qualifying[j]
                .y
                .partial_cmp(&qualifying[i].y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(i.cmp(&j))
        });
        let mut top2: Vec<usize> = order.into_iter().take(2).collect();
        top2.sort_unstable();
        tier_idx = top2;
    }
    let w_cap: Vec<f64> = v_eff
        .iter()
        .enumerate()
        .map(|(i, v)| capability[i] / (v + tau2))
        .collect();
    let cap_den = {
        let terms: Vec<f64> = tier_idx.iter().map(|&i| w_cap[i]).collect();
        sum(&terms)
    };
    let capacity = if cap_den > 0.0 {
        let terms: Vec<f64> = tier_idx
            .iter()
            .map(|&i| w_cap[i] * qualifying[i].y)
            .collect();
        sum(&terms) / cap_den
    } else {
        ymax
    };

    let consensus_ci;
    let capacity_ci;
    let band;

    if k == 2 {
        let lower = qualifying
            .iter()
            .enumerate()
            .map(|(i, p)| p.y - 1.96 * v_eff[i].sqrt())
            .fold(f64::INFINITY, f64::min);
        let upper = qualifying
            .iter()
            .enumerate()
            .map(|(i, p)| p.y + 1.96 * v_eff[i].sqrt())
            .fold(f64::NEG_INFINITY, f64::max);
        consensus_ci = CiBounds { lower, upper };
        capacity_ci = CiBounds { lower, upper };
        band = AgreementBand::Insufficient;
    } else {
        // HKSJ over all qualifying → consensus CI.
        let qc_num = {
            let terms: Vec<f64> = qualifying
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let d = p.y - consensus;
                    w_star_capped[i] * (d * d)
                })
                .collect();
            sum(&terms)
        };
        let se_c = ((1.0_f64).max(qc_num / (k - 1) as f64) / sum_w_star_capped).sqrt();
        consensus_ci = CiBounds {
            lower: consensus - t975(k - 1) * se_c,
            upper: consensus + t975(k - 1) * se_c,
        };

        // HKSJ over the tier (same RE weights) → capacity CI.
        let tier_n = tier_idx.len();
        if tier_n >= 2 {
            let sum_w_star_tier = {
                let terms: Vec<f64> = tier_idx.iter().map(|&i| w_star_capped[i]).collect();
                sum(&terms)
            };
            let q_cap_num = {
                let terms: Vec<f64> = tier_idx
                    .iter()
                    .map(|&i| {
                        let d = qualifying[i].y - capacity;
                        w_star_capped[i] * (d * d)
                    })
                    .collect();
                sum(&terms)
            };
            let se_cap = ((1.0_f64).max(q_cap_num / (tier_n - 1) as f64) / sum_w_star_tier).sqrt();
            capacity_ci = CiBounds {
                lower: capacity - t975(tier_n - 1) * se_cap,
                upper: capacity + t975(tier_n - 1) * se_cap,
            };
        } else {
            let i = tier_idx[0];
            let se = v_eff[i].sqrt();
            capacity_ci = CiBounds {
                lower: qualifying[i].y - 1.96 * se,
                upper: qualifying[i].y + 1.96 * se,
            };
        }

        band = if i2 < 0.25 {
            AgreementBand::High
        } else if i2 < 0.50 {
            AgreementBand::Moderate
        } else if i2 < 0.75 {
            AgreementBand::Low
        } else {
            AgreementBand::VeryLow
        };
    }

    let weights: Vec<MergeWeight> = qualifying
        .iter()
        .enumerate()
        .map(|(i, p)| MergeWeight {
            name: p.name.clone(),
            y: p.y,
            v: v_eff[i],
            w_star: w_star[i],
            w_star_capped: w_star_capped[i],
            w_cap: w_cap[i],
        })
        .collect();

    MergeResult {
        k,
        capacity,
        consensus,
        capacity_ci,
        consensus_ci,
        tau2,
        i2: Some(i2),
        q,
        band,
        tier: tier_idx
            .iter()
            .map(|&i| qualifying[i].name.clone())
            .collect(),
        weights,
        exclusions,
    }
}

// ── Jitter (PDV / IPDV / MAD) ────────────────────────────────────────────

/// Canonical jitter — packet delay variation: `P95(RTT) − P50(RTT)` (RFC 5481 flavor).
pub fn pdv(rtts: &[f64]) -> f64 {
    if rtts.is_empty() {
        return 0.0;
    }
    let mut s = rtts.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    quantile(&s, 0.95) - quantile(&s, 0.5)
}

/// IPDV mean: mean of `|consecutive ΔRTT|`.
pub fn ipdv_mean(rtts: &[f64]) -> f64 {
    if rtts.len() < 2 {
        return 0.0;
    }
    let total: f64 = rtts.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
    total / (rtts.len() - 1) as f64
}

/// Robust scale: `1.4826 · median(|x − median(x)|)`.
pub fn median_absolute_deviation(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let med = median_of(values);
    let mut dev: Vec<f64> = values.iter().map(|v| (v - med).abs()).collect();
    dev.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    1.4826 * quantile(&dev, 0.5)
}

/// The v4 jitter bundle (PDV canonical, IPDV/MAD secondary, RFC 3550 compat).
#[derive(Debug, Clone, Serialize)]
pub struct JitterMetrics {
    /// Canonical: `P95 − P50`.
    pub pdv: f64,
    /// Secondary: mean `|ΔRTT|`.
    pub ipdv_mean: f64,
    /// Secondary: `1.4826 · MAD`.
    pub mad: f64,
    /// Compatibility field only.
    pub jitter_rfc3550: f64,
}

/// Compute all v4 jitter metrics from an RTT series.
pub fn jitter_metrics(rtts: &[f64]) -> JitterMetrics {
    JitterMetrics {
        pdv: pdv(rtts),
        ipdv_mean: ipdv_mean(rtts),
        mad: median_absolute_deviation(rtts),
        jitter_rfc3550: jitter_rfc3550(rtts),
    }
}

// ── Bufferbloat (delta-ms + grade) + RPM ─────────────────────────────────

/// Bufferbloat letter grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BufferbloatGrade {
    #[serde(rename = "A+")]
    APlus,
    A,
    B,
    C,
    D,
    F,
}

impl BufferbloatGrade {
    /// Canonical wire string (`A+`/`A`/`B`/`C`/`D`/`F`).
    pub fn as_str(self) -> &'static str {
        match self {
            BufferbloatGrade::APlus => "A+",
            BufferbloatGrade::A => "A",
            BufferbloatGrade::B => "B",
            BufferbloatGrade::C => "C",
            BufferbloatGrade::D => "D",
            BufferbloatGrade::F => "F",
        }
    }
}

/// Grade the bufferbloat delta (ms): A+ <5 · A <30 · B <60 · C <200 · D <400 · F ≥400.
pub fn bufferbloat_grade(delta_ms: f64) -> BufferbloatGrade {
    if delta_ms < 5.0 {
        BufferbloatGrade::APlus
    } else if delta_ms < 30.0 {
        BufferbloatGrade::A
    } else if delta_ms < 60.0 {
        BufferbloatGrade::B
    } else if delta_ms < 200.0 {
        BufferbloatGrade::C
    } else if delta_ms < 400.0 {
        BufferbloatGrade::D
    } else {
        BufferbloatGrade::F
    }
}

/// Delta-ms bufferbloat result (canonical delta, secondary ratio, grade).
#[derive(Debug, Clone, Serialize)]
pub struct BufferbloatDeltaResult {
    /// Canonical: `P95(loaded RTT) − P50(idle RTT)`.
    pub delta_ms: f64,
    /// Secondary: `P95(loaded) / P50(idle)`.
    pub ratio: f64,
    pub grade: BufferbloatGrade,
}

/// Delta-ms bufferbloat: `P95(loaded) − P50(idle)`, graded; ratio disclosed as secondary.
pub fn bufferbloat_delta(idle_rtts: &[f64], loaded_rtts: &[f64]) -> BufferbloatDeltaResult {
    let p50_idle = if idle_rtts.is_empty() {
        0.0
    } else {
        let mut s = idle_rtts.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        quantile(&s, 0.5)
    };
    let p95_loaded = if loaded_rtts.is_empty() {
        0.0
    } else {
        let mut s = loaded_rtts.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        quantile(&s, 0.95)
    };
    let delta_ms = p95_loaded - p50_idle;
    let ratio = if p50_idle > 0.0 {
        p95_loaded / p50_idle
    } else {
        0.0
    };
    BufferbloatDeltaResult {
        delta_ms,
        ratio,
        grade: bufferbloat_grade(delta_ms),
    }
}

/// Responsiveness (approx): `60000 / P50(loaded RTT ms)` round-trips per minute.
pub fn rpm(loaded_rtts: &[f64]) -> f64 {
    if loaded_rtts.is_empty() {
        return 0.0;
    }
    let mut s = loaded_rtts.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50 = quantile(&s, 0.5);
    if p50 > 0.0 {
        60000.0 / p50
    } else {
        0.0
    }
}

// ── FAST-mode empirical-Bernstein confidence sequence ────────────────────

/// Anytime-valid empirical-Bernstein confidence sequence state at one sample count.
#[derive(Debug, Clone, Serialize)]
pub struct ConfidenceSequence {
    /// Samples consumed.
    pub t: usize,
    /// Rescaling cap `U = 2·max(samples)`.
    pub u: f64,
    /// CS midpoint in Mbps.
    pub mu_hat_mbps: f64,
    /// CS half-width in Mbps (the quantity the stop rule tests).
    pub half_width_mbps: f64,
    /// Rescaled `[0, 1]` half-width.
    pub width: f64,
    /// Stop early: `t ≥ 12`, RTT gate open, and `halfWidth ≤ max(5%·est, 2 Mbps)`.
    pub stop: bool,
}

/// Empirical-Bernstein confidence sequence for FAST-mode early termination
/// (METHODOLOGY.md §8).
///
/// Samples (raw Mbps, time order) are rescaled by `U = 2·max` into `[0,1]`; the
/// running mean inside the variance accumulator is **strictly predictable** —
/// `muHat_i = (0.5 + Σ_{j<i} X_j)/i` uses prior samples plus the 0.5 prior,
/// NEVER the current sample (the anytime-valid guarantee requires a predictable
/// comparator; PINNED 2026-07-06). Stop when
/// `half_width_mbps ≤ max(0.05·estimate, 2 Mbps)` AND `t ≥ 12`, unless the
/// measured min-RTT gate (`> 50 ms`) forbids early stop. Caller applies the 25 s
/// hard cap.
pub fn empirical_bernstein_cs(
    samples_so_far: &[f64],
    alpha: f64,
    min_rtt_ms: f64,
) -> ConfidenceSequence {
    let t = samples_so_far.len();
    if t == 0 {
        return ConfidenceSequence {
            t: 0,
            u: 0.0,
            mu_hat_mbps: 0.0,
            half_width_mbps: f64::INFINITY,
            width: f64::INFINITY,
            stop: false,
        };
    }

    let mut max_v = 0.0_f64;
    for &s in samples_so_far {
        if s > max_v {
            max_v = s;
        }
    }
    let u = 2.0 * max_v;
    if u <= 0.0 {
        return ConfidenceSequence {
            t,
            u: 0.0,
            mu_hat_mbps: 0.0,
            half_width_mbps: f64::INFINITY,
            width: f64::INFINITY,
            stop: false,
        };
    }

    let mut x_sum = 0.0_f64;
    let mut sig2_sum = 0.0_f64;
    for (i, &s) in samples_so_far.iter().enumerate() {
        let x = s / u;
        // Predictable (prior-only) running mean: muHat_i uses X_1..X_{i-1} plus
        // the 0.5 prior, NEVER the current sample. Compute from x_sum BEFORE
        // adding x. Spec decision 2026-07-06 (METHODOLOGY.md §8).
        let mu_hat_prior = (0.5 + x_sum) / (i as f64 + 1.0);
        let d = x - mu_hat_prior;
        sig2_sum += d * d;
        x_sum += x;
    }
    let mu_hat_t = (0.5 + x_sum) / (t as f64 + 1.0);
    let sig2_t = (0.25 + sig2_sum) / (t as f64 + 1.0);
    let ln_term = (2.0 / alpha).ln();
    let width = (2.0 * sig2_t * ln_term / t as f64).sqrt() + 3.0 * ln_term / t as f64;
    let half_width_mbps = width * u;
    let mu_hat_mbps = mu_hat_t * u;
    let gated = min_rtt_ms > 50.0;
    let stop = !gated && t >= 12 && half_width_mbps <= (0.05 * mu_hat_mbps).max(2.0);
    ConfidenceSequence {
        t,
        u,
        mu_hat_mbps,
        half_width_mbps,
        width,
        stop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_drops_nan_and_infinite() {
        let cleaned = sanitize(&[f64::NAN, 1.0, f64::INFINITY, 2.0, f64::NEG_INFINITY]);
        assert_eq!(cleaned, vec![1.0, 2.0]);
    }

    #[test]
    fn sanitize_keeps_clean_input_intact() {
        let cleaned = sanitize(&[3.0, 1.0, 2.0]);
        assert_eq!(cleaned, vec![3.0, 1.0, 2.0]);
    }

    fn well_behaved_samples() -> Vec<f64> {
        (0..20).map(|i| 95.0 + (i % 5) as f64 * 2.5).collect()
    }

    /// The PRNG is seeded from the data, so identical input must produce
    /// identical bounds — JSON output stays reproducible.
    #[test]
    fn bootstrap_ci_is_deterministic() {
        let samples = well_behaved_samples();
        let a = bootstrap_ci(&samples, accurate_bandwidth, 1000, 0.05);
        let b = bootstrap_ci(&samples, accurate_bandwidth, 1000, 0.05);
        assert_eq!(a.lower, b.lower);
        assert_eq!(a.upper, b.upper);
        assert_eq!(a.estimate, b.estimate);
    }

    #[test]
    fn bootstrap_ci_brackets_estimate() {
        let samples = well_behaved_samples();
        let ci = bootstrap_ci(&samples, accurate_bandwidth, 1000, 0.05);
        assert!(
            ci.lower <= ci.estimate && ci.estimate <= ci.upper,
            "lower {} <= estimate {} <= upper {}",
            ci.lower,
            ci.estimate,
            ci.upper
        );
        assert!(ci.margin >= 0.0);
    }

    /// Below 4 samples the percentile method is degenerate; the function
    /// returns a zero-margin CI — callers must suppress display in that case.
    #[test]
    fn bootstrap_ci_degenerate_below_four() {
        let ci = bootstrap_ci(&[100.0, 110.0, 105.0], accurate_bandwidth, 1000, 0.05);
        assert_eq!(ci.margin, 0.0);
        assert_eq!(ci.lower, ci.upper);
    }
}
