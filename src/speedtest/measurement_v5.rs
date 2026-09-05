//! Deterministic byte-accounted methodology v5. Mirrored by measurement-v5.ts.
use serde::{Deserialize, Serialize};

pub const WARMUP_MS: f64 = 2000.0;
pub const MIN_MEASUREMENT_MS: f64 = 4000.0;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub cap_ms: u64,
    pub direction_ms: u64,
    pub default_max_bytes: u64,
}

pub fn profile(deep: bool) -> &'static Profile {
    #[derive(Deserialize)]
    struct Contract {
        profiles: std::collections::BTreeMap<String, Profile>,
    }
    static CONTRACT: std::sync::OnceLock<Contract> = std::sync::OnceLock::new();
    &CONTRACT
        .get_or_init(|| {
            serde_json::from_str(include_str!("measurement-contract-v5.json"))
                .expect("Validated v5 contract")
        })
        .profiles[if deep { "full" } else { "fast" }]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterPoint {
    pub t: f64,
    pub bytes: u64,
    #[serde(default = "valid_default")]
    pub valid: bool,
}
fn valid_default() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementTrace {
    pub provider: String,
    pub endpoint: String,
    pub transport: String,
    pub streams: u32,
    pub direction: String,
    pub accounting: String,
    pub warmup_ms: f64,
    pub points: Vec<CounterPoint>,
    pub stop_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_tcp_min_rtt_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedRange {
    pub lower: f64,
    pub upper: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectionEstimate {
    pub sustained_mbps: Option<f64>,
    pub ceiling_mbps: Option<f64>,
    pub measured_bytes: u64,
    pub measured_ms: f64,
    pub samples: usize,
    pub repeatability: Option<ObservedRange>,
    pub qualification: String,
    pub warnings: Vec<String>,
}
impl Default for DirectionEstimate {
    fn default() -> Self {
        Self {
            sustained_mbps: None,
            ceiling_mbps: None,
            measured_bytes: 0,
            measured_ms: 0.0,
            samples: 0,
            repeatability: None,
            qualification: "unavailable".into(),
            warnings: vec![],
        }
    }
}
fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    if s.is_empty() {
        0.0
    } else if s.len() % 2 == 1 {
        s[s.len() / 2]
    } else {
        (s[s.len() / 2 - 1] + s[s.len() / 2]) / 2.0
    }
}
fn range(v: &[f64]) -> Option<ObservedRange> {
    if v.is_empty() {
        None
    } else {
        Some(ObservedRange {
            lower: v.iter().copied().fold(f64::INFINITY, f64::min),
            upper: v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        })
    }
}
fn unique(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
}

fn withhold_low_ceiling(out: &mut DirectionEstimate) {
    if let (Some(ceiling), Some(sustained)) = (out.ceiling_mbps, out.sustained_mbps) {
        if ceiling < sustained {
            out.ceiling_mbps = None;
            out.warnings
                .push("No repeatable ceiling at or above sustained throughput".into());
        }
    }
}

pub fn estimate_trace(trace: &MeasurementTrace) -> DirectionEstimate {
    if let Some(error) = &trace.integrity_error {
        return DirectionEstimate {
            warnings: vec![error.clone()],
            ..Default::default()
        };
    }
    if !trace.warmup_ms.is_finite() || trace.warmup_ms < 0.0 {
        let mut out = DirectionEstimate::default();
        out.warnings.push("Invalid warm-up duration".into());
        return out;
    }
    let mut out = DirectionEstimate::default();
    let p = &trace.points;
    if p.len() < 2 {
        return out;
    }
    if trace.stop_reason == "failed" && p.last().unwrap().bytes == p[0].bytes {
        out.warnings.push("No successful transfer".into());
        return out;
    }
    if p.iter().enumerate().any(|(i, x)| {
        !x.t.is_finite()
            || x.t < 0.0
            || x.bytes > 9_007_199_254_740_991
            || (i > 0 && (x.t <= p[i - 1].t || x.bytes < p[i - 1].bytes))
    }) {
        out.warnings
            .push("Invalid or reset measurement counters".into());
        return out;
    }
    let mut intervals = Vec::new();
    let cutoff = p[0].t + trace.warmup_ms;
    for pair in p.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.t < cutoff {
            continue;
        }
        if !a.valid || !b.valid {
            out.warnings.push("Interrupted intervals excluded".into());
            continue;
        }
        let dt = b.t - a.t;
        let bytes = b.bytes - a.bytes;
        intervals.push((a.t, b.t, bytes));
        out.measured_bytes += bytes;
        out.measured_ms += dt;
    }
    out.samples = intervals.len();
    if out.measured_ms == 0.0 {
        return out;
    }
    out.sustained_mbps = Some(out.measured_bytes as f64 * 0.008 / out.measured_ms);
    out.qualification =
        if out.measured_ms >= MIN_MEASUREMENT_MS && trace.accounting != "sender-drained" {
            "measured"
        } else {
            "provisional"
        }
        .into();
    if trace.accounting == "sender-drained" {
        out.warnings
            .push("Upload lacks receiver confirmation".into());
    }
    let mut windows = Vec::new();
    for i in 0..intervals.len() {
        let mut bytes = 0u64;
        for j in i..intervals.len() {
            if j > i && intervals[j].0 != intervals[j - 1].1 {
                break;
            }
            bytes += intervals[j].2;
            let dt = intervals[j].1 - intervals[i].0;
            if dt >= 3000.0 {
                windows.push((intervals[i].0, intervals[j].1, bytes as f64 * 0.008 / dt));
                break;
            }
        }
    }
    if windows.len() >= 2 {
        out.repeatability = range(&windows.iter().map(|w| w.2).collect::<Vec<_>>());
    }
    if out.qualification == "measured" {
        for i in 0..windows.len() {
            for j in i + 1..windows.len() {
                let (a, b) = (windows[i], windows[j]);
                let hi = a.2.max(b.2);
                let lo = a.2.min(b.2);
                if a.1 <= b.0 && hi > 0.0 && (hi - lo) / hi <= 0.1 {
                    out.ceiling_mbps = Some(out.ceiling_mbps.unwrap_or(0.0).max(lo));
                }
            }
        }
    }
    withhold_low_ceiling(&mut out);
    if let Some(r) = &out.repeatability {
        let speed = out.sustained_mbps.unwrap_or(0.0);
        if speed > 0.0 && (r.upper - r.lower) / speed > 0.2 {
            out.warnings
                .push("Throughput varied during measurement".into());
        }
    }
    if let (Some(first), Some(last)) = (windows.first(), windows.last()) {
        if first.1 <= last.0 && last.2 > 0.0 && last.2 > first.2 * 1.2 {
            out.warnings
                .push("Throughput was still increasing after warm-up".into());
        }
    }
    if trace.stop_reason != "complete" {
        out.warnings
            .push(format!("Measurement ended: {}", trace.stop_reason));
    }
    unique(&mut out.warnings);
    out
}

pub fn summarize_traces(
    traces: &[MeasurementTrace],
    direction: &str,
) -> (DirectionEstimate, Vec<String>) {
    let mut estimates = Vec::new();
    let mut providers = Vec::new();
    for provider in ["cloudflare", "msak"] {
        let qualified: Vec<_> = traces
            .iter()
            .filter(|t| t.provider == provider && t.direction == direction)
            .map(estimate_trace)
            .filter(|e| e.qualification == "measured")
            .collect();
        if qualified.is_empty() {
            continue;
        }
        let mut out = DirectionEstimate::default();
        out.measured_bytes = qualified.iter().map(|e| e.measured_bytes).sum();
        out.measured_ms = qualified.iter().map(|e| e.measured_ms).sum();
        out.samples = qualified.iter().map(|e| e.samples).sum();
        out.sustained_mbps = Some(out.measured_bytes as f64 * 0.008 / out.measured_ms);
        out.qualification = "measured".into();
        let ceilings: Vec<_> = qualified.iter().filter_map(|e| e.ceiling_mbps).collect();
        if !ceilings.is_empty() {
            out.ceiling_mbps = Some(median(&ceilings));
        }
        let bounds: Vec<_> = qualified
            .iter()
            .filter_map(|e| e.repeatability.as_ref())
            .flat_map(|r| [r.lower, r.upper])
            .collect();
        out.repeatability = range(&bounds);
        out.warnings = qualified.into_iter().flat_map(|e| e.warnings).collect();
        withhold_low_ceiling(&mut out);
        estimates.push(out);
        providers.push(provider.to_string());
    }
    let mut out = DirectionEstimate::default();
    if estimates.is_empty() {
        out.warnings
            .push("No qualifying primary measurement".into());
        return (out, providers);
    }
    let rates: Vec<_> = estimates.iter().filter_map(|e| e.sustained_mbps).collect();
    out.sustained_mbps = Some(median(&rates));
    out.qualification = "measured".into();
    out.measured_bytes = estimates.iter().map(|e| e.measured_bytes).sum();
    out.measured_ms = estimates.iter().map(|e| e.measured_ms).sum();
    out.samples = estimates.iter().map(|e| e.samples).sum();
    out.repeatability = if rates.len() > 1 {
        range(&rates)
    } else {
        estimates[0].repeatability.clone()
    };
    let ceilings: Vec<_> = estimates.iter().filter_map(|e| e.ceiling_mbps).collect();
    out.warnings = estimates.iter().flat_map(|e| e.warnings.clone()).collect();
    if let Some(r) = range(&ceilings) {
        if r.upper - r.lower <= 0.2 * r.upper {
            out.ceiling_mbps = Some(median(&ceilings));
        } else {
            out.warnings
                .push("Ceiling estimates disagree across providers".into());
        }
    }
    withhold_low_ceiling(&mut out);
    if rates.len() == 1 {
        out.warnings.push("Single primary source".into());
    } else if let Some(r) = range(&rates) {
        if r.upper - r.lower > 0.2 * r.upper {
            out.warnings.push("Primary providers disagree".into());
        }
    }
    unique(&mut out.warnings);
    (out, providers)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measurement_bounds_match_the_versioned_contract() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("measurement-contract-v5.json")).unwrap();
        assert_eq!(contract["warmupMs"].as_f64(), Some(WARMUP_MS));
        assert_eq!(
            contract["minimumMeasurementMs"].as_f64(),
            Some(MIN_MEASUREMENT_MS)
        );
    }
    #[test]
    fn recorded_counter_replay() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("measurement-v5-fixtures.json")).unwrap();
        for f in fixtures.as_array().unwrap() {
            let trace: MeasurementTrace = serde_json::from_value(f["trace"].clone()).unwrap();
            let actual = serde_json::to_value(estimate_trace(&trace)).unwrap();
            for field in [
                "sustainedMbps",
                "ceilingMbps",
                "qualification",
                "measuredBytes",
                "measuredMs",
            ] {
                match (actual[field].as_f64(), f["expected"][field].as_f64()) {
                    (Some(a), Some(b)) => {
                        assert!((a - b).abs() < 1e-8, "{}: {}: {a} != {b}", f["name"], field)
                    }
                    _ => assert_eq!(
                        actual[field], f["expected"][field],
                        "{}: {}",
                        f["name"], field
                    ),
                }
            }
        }
    }
}
