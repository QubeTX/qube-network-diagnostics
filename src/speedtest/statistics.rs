//! Statistical utilities for accurate speed test measurement.
//!
//! Port of the QubeTX web speed test's `statistics.ts` module.
//! Implements Ookla-style trimean, IQR outlier filtering, slow-start discard,
//! RFC 3550 jitter, inverse-variance weighting, and bootstrap confidence intervals.

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
    values.iter().copied().filter(|v| *v >= lo && *v <= hi).collect()
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
    let sum: f64 = samples
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .sum();
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
pub fn inverse_variance_merge(
    a: f64,
    var_a: f64,
    b: f64,
    var_b: f64,
) -> InverseVarianceResult {
    if a <= 0.0 && b <= 0.0 {
        return InverseVarianceResult { value: 0.0, weight_a: 0.5, weight_b: 0.5 };
    }
    if a <= 0.0 {
        return InverseVarianceResult { value: b, weight_a: 0.0, weight_b: 1.0 };
    }
    if b <= 0.0 {
        return InverseVarianceResult { value: a, weight_a: 1.0, weight_b: 0.0 };
    }
    if var_a <= 0.0 && var_b <= 0.0 {
        return InverseVarianceResult { value: (a + b) / 2.0, weight_a: 0.5, weight_b: 0.5 };
    }
    if var_a <= 0.0 {
        return InverseVarianceResult { value: a, weight_a: 1.0, weight_b: 0.0 };
    }
    if var_b <= 0.0 {
        return InverseVarianceResult { value: b, weight_a: 0.0, weight_b: 1.0 };
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
        return BootstrapCI { estimate: est, lower: est, upper: est, margin: 0.0 };
    }

    let estimate = stat_fn(samples);

    // Seed from sample data for deterministic results
    let seed = samples.iter().fold(0u64, |acc, v| {
        acc.wrapping_add(v.to_bits()).wrapping_mul(6364136223846793005)
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
