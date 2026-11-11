//! Golden-vector + ground-truth tests for the SpeedQX Methodology v4 core.
//!
//! This is the Rust half of the cross-language parity contract. Every value in
//! the committed `golden-vectors.json` (byte-identical to the SpeedQX web repo's
//! fixture) is recomputed here and compared:
//!
//!   * Arithmetic paths (quantile, PCG32 streams, plateau, cleaned arrays,
//!     trimean, Hodges–Lehmann, block-bootstrap θ̂/mean/variance, DL τ²/I²/Q,
//!     capacity/consensus, jitter, bufferbloat) are asserted **BIT-EXACT** —
//!     `f64::to_bits` equality.
//!   * Transcendental paths (BCa bounds via Acklam/West, HKSJ CIs via sqrt, and
//!     the empirical-Bernstein widths via `ln`) are asserted to **1e-9 relative**,
//!     matching the portability contract the TypeScript `vitest` suite honors.
//!
//! Mirrors `src/services/__tests__/statistics.test.ts` in the SpeedQX web repo.

use super::stat_primitives::{
    inv_normal, lemire_bounded, phi, quantile, sample_mean, sample_variance, sum, t975, Pcg32,
};
use super::statistics::{
    bufferbloat_delta, bufferbloat_grade, circular_block_bootstrap, empirical_bernstein_cs,
    filter_outliers_iqr, hodges_lehmann, ipdv_mean, jitter_metrics, median_absolute_deviation,
    merge_providers, modified_trimean, pdv, plateau_start, rpm, MergeProviderInput,
};
use serde_json::Value;

const GOLDEN_JSON: &str = include_str!("../../golden-vectors.json");

fn golden() -> Value {
    serde_json::from_str(GOLDEN_JSON).expect("golden-vectors.json must parse")
}

fn f64_vec(v: &Value) -> Vec<f64> {
    v.as_array()
        .expect("expected a JSON array")
        .iter()
        .map(|x| x.as_f64().expect("expected a JSON number"))
        .collect()
}

fn g_f64(v: &Value) -> f64 {
    v.as_f64().expect("expected a JSON number")
}

/// Bit-exact f64 comparison (mirrors vitest `toBe` for the arithmetic paths).
fn assert_exact(actual: f64, expected: f64, ctx: &str) {
    assert!(
        actual.to_bits() == expected.to_bits(),
        "{ctx}: not bit-exact\n  actual   = {actual:?} ({:#018x})\n  expected = {expected:?} ({:#018x})",
        actual.to_bits(),
        expected.to_bits()
    );
}

/// 1e-9 relative comparison (mirrors vitest `expectRelClose`, denom = max(1, |e|)).
fn assert_rel(actual: f64, expected: f64, ctx: &str) {
    const TOL: f64 = 1e-9;
    let denom = expected.abs().max(1.0);
    let rel = (actual - expected).abs() / denom;
    assert!(
        rel <= TOL,
        "{ctx}: rel {rel:e} > {TOL:e}\n  actual   = {actual}\n  expected = {expected}"
    );
}

// ── PCG32 + Lemire (bit-exact) ───────────────────────────────────────────

#[test]
fn pcg32_first64_u32_matches_golden() {
    let g = golden();
    let expected = g["pcg32"]["first64U32"].as_array().unwrap();
    let mut r = Pcg32::new();
    for (idx, e) in expected.iter().enumerate() {
        let got = u64::from(r.next_u32());
        assert_eq!(got, e.as_u64().unwrap(), "pcg32 first64U32 index {idx}");
        assert!(got <= u64::from(u32::MAX));
    }
}

#[test]
fn pcg32_first_four_ground_truth() {
    let mut r = Pcg32::new();
    assert_eq!(
        [r.next_u32(), r.next_u32(), r.next_u32(), r.next_u32()],
        [355248013, 41705475, 3406281715, 4186697710]
    );
}

#[test]
fn pcg32_bounded_streams_match_golden() {
    let g = golden();
    for &n in &[12usize, 27, 60, 200] {
        let key = n.to_string();
        let expected = g["pcg32"]["bounded"][key.as_str()].as_array().unwrap();
        let mut r = Pcg32::new();
        for (idx, e) in expected.iter().enumerate() {
            let got = r.bounded_index(n);
            assert_eq!(got as u64, e.as_u64().unwrap(), "bounded n={n} index {idx}");
            assert!(got < n, "bounded n={n} index {idx} out of range");
        }
    }
}

#[test]
fn lemire_bounded_ground_truth() {
    assert_eq!(lemire_bounded(0, 100), 0);
    assert_eq!(lemire_bounded(0xffff_ffff, 100), 99);
    assert_eq!(lemire_bounded(0x8000_0000, 10), 5);
}

// ── quantile (type-7, bit-exact) ─────────────────────────────────────────

#[test]
fn quantile_matches_golden_cases() {
    let g = golden();
    for (idx, c) in g["quantile"].as_array().unwrap().iter().enumerate() {
        let sorted = f64_vec(&c["sorted"]);
        let p = g_f64(&c["p"]);
        assert_exact(
            quantile(&sorted, p),
            g_f64(&c["expected"]),
            &format!("quantile case {idx}"),
        );
    }
}

#[test]
fn quantile_ground_truth() {
    assert_exact(quantile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.5, "q 0.5");
    assert_exact(quantile(&[1.0, 2.0, 3.0, 4.0], 0.25), 1.75, "q 0.25");
    assert_exact(
        quantile(
            &[10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0],
            0.1,
        ),
        19.0,
        "q 0.1",
    );
    assert!(quantile(&[], 0.5).is_nan());
}

// ── invNormal + phi (1e-9 relative) ──────────────────────────────────────

#[test]
fn inv_normal_and_phi_anchors() {
    assert_exact(inv_normal(0.5), 0.0, "invNormal(0.5)");
    assert_exact(phi(0.0), 0.5, "phi(0)");
    assert_eq!(inv_normal(0.0), f64::NEG_INFINITY);
    assert_eq!(inv_normal(1.0), f64::INFINITY);
    assert_rel(inv_normal(0.975), 1.959963984540054, "invNormal(0.975)");
    assert_rel(inv_normal(0.025), -1.959963984540054, "invNormal(0.025)");
    assert_rel(phi(1.959963984540054), 0.975, "phi(1.96)");
    assert_rel(phi(-1.959963984540054), 0.025, "phi(-1.96)");
    // phi ∘ invNormal round-trips.
    for &p in &[0.05, 0.2, 0.5, 0.8, 0.95] {
        assert_rel(phi(inv_normal(p)), p, "phi∘invNormal");
    }
}

// ── t975 + fixed-order sums (bit-exact) ──────────────────────────────────

#[test]
fn t975_and_sum_helpers_ground_truth() {
    assert_eq!(t975(1), 12.706);
    assert_eq!(t975(0), 12.706);
    assert_eq!(t975(7), 2.365);
    assert_eq!(t975(99), 2.365);

    let xs = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
    assert_exact(sum(&xs), 40.0, "sum");
    assert_exact(sample_mean(&xs), 5.0, "mean");
    assert_exact(sample_variance(&xs), 32.0 / 7.0, "variance");
}

// ── per-provider pipeline vs golden ──────────────────────────────────────

#[test]
fn plateau_start_ground_truth() {
    // All-equal series clamps to the 10% floor: clamp(0, ceil(0.8)=1, floor(3.2)=3) = 1.
    assert_eq!(plateau_start(&[5.0; 8]), 1);
    // n < 8 uses the 30% fallback: ceil(0.3*5) = 2.
    assert_eq!(plateau_start(&[1.0, 2.0, 3.0, 4.0, 5.0]), 2);
}

#[test]
fn modified_trimean_and_iqr_and_hl_ground_truth() {
    assert_exact(
        modified_trimean(&[10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0]),
        55.0,
        "modifiedTrimean ramp",
    );
    let cleaned = filter_outliers_iqr(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 100.0], 1.5);
    assert_eq!(cleaned, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    assert_exact(hodges_lehmann(&[1.0, 2.0, 3.0]), 2.0, "HL");
    assert_exact(hodges_lehmann(&[5.0]), 5.0, "HL single");
}

#[test]
fn per_provider_pipeline_matches_golden() {
    let g = golden();
    for &n in &[12usize, 27, 60, 200] {
        let key = n.to_string();
        let raw = f64_vec(&g["samples"][key.as_str()]);
        let pp = &g["perProvider"][key.as_str()];

        let ps = plateau_start(&raw);
        assert_eq!(
            ps as u64,
            pp["plateauStart"].as_u64().unwrap(),
            "plateauStart n={n}"
        );
        // Clamp invariant for n >= 8.
        assert!(ps >= (0.1 * n as f64).ceil() as usize);
        assert!(ps <= (0.4 * n as f64).floor() as usize);

        // discardedLength = post-warm-up series length (raw minus plateau discard).
        assert_eq!(
            (raw.len() - ps) as u64,
            pp["discardedLength"].as_u64().unwrap(),
            "discardedLength n={n}"
        );

        let cleaned = filter_outliers_iqr(&raw[ps..], 1.5);
        let expected_cleaned = f64_vec(&pp["cleaned"]);
        assert_eq!(cleaned.len(), expected_cleaned.len(), "cleaned len n={n}");
        for (idx, (a, e)) in cleaned.iter().zip(&expected_cleaned).enumerate() {
            assert_exact(*a, *e, &format!("cleaned n={n} idx {idx}"));
        }

        assert_exact(
            modified_trimean(&cleaned),
            g_f64(&pp["trimean"]),
            &format!("trimean n={n}"),
        );
        assert_exact(
            hodges_lehmann(&cleaned),
            g_f64(&pp["hodgesLehmann"]),
            &format!("hodgesLehmann n={n}"),
        );

        // Circular block bootstrap: fresh default-initialized PCG32 per case.
        const BOOTSTRAP_B: u64 = 2000;
        let boot = circular_block_bootstrap(&cleaned, &mut Pcg32::new(), BOOTSTRAP_B as usize);
        let bs = &pp["bootstrap"];
        // Resample count parity: the fixture and this suite both use B = 2000.
        assert_eq!(bs["B"].as_u64().unwrap(), BOOTSTRAP_B, "bootstrap B n={n}");
        assert_eq!(
            boot.block_length as u64,
            bs["blockLength"].as_u64().unwrap(),
            "blockLength n={n}"
        );
        assert_exact(
            boot.theta_hat,
            g_f64(&bs["thetaHat"]),
            &format!("thetaHat n={n}"),
        );
        assert_exact(
            boot.theta_star_mean,
            g_f64(&bs["thetaStarMean"]),
            &format!("thetaStarMean n={n}"),
        );
        assert_exact(
            boot.variance,
            g_f64(&bs["variance"]),
            &format!("variance n={n}"),
        );
        // BCa bounds: transcendental (invNormal/phi/pow) → 1e-9 relative.
        assert_rel(
            boot.ci_lower,
            g_f64(&bs["ciLower"]),
            &format!("ciLower n={n}"),
        );
        assert_rel(
            boot.ci_upper,
            g_f64(&bs["ciUpper"]),
            &format!("ciUpper n={n}"),
        );
    }
}

#[test]
fn bootstrap_is_deterministic() {
    let g = golden();
    let raw = f64_vec(&g["samples"]["27"]);
    let cleaned = filter_outliers_iqr(&raw[plateau_start(&raw)..], 1.5);
    let a = circular_block_bootstrap(&cleaned, &mut Pcg32::new(), 500);
    let b = circular_block_bootstrap(&cleaned, &mut Pcg32::new(), 500);
    assert_exact(a.theta_star_mean, b.theta_star_mean, "det thetaStarMean");
    assert_exact(a.variance, b.variance, "det variance");
    assert_exact(a.ci_lower, b.ci_lower, "det ciLower");
    assert_exact(a.ci_upper, b.ci_upper, "det ciUpper");
}

// ── cross-provider merge vs golden ───────────────────────────────────────

#[test]
fn merge_cases_match_golden() {
    let g = golden();
    for (i, case) in g["merge"].as_array().unwrap().iter().enumerate() {
        let label = case["label"].as_str().unwrap_or("<unlabeled>");
        let inputs: Vec<MergeProviderInput> =
            serde_json::from_value(case["providers"].clone()).expect("merge providers deserialize");
        let r = merge_providers(&inputs);
        let e = &case["expected"];

        assert_eq!(
            r.k as u64,
            e["k"].as_u64().unwrap(),
            "merge[{i}] {label}: k"
        );
        assert_exact(
            r.capacity,
            g_f64(&e["capacity"]),
            &format!("merge[{i}] {label} capacity"),
        );
        assert_exact(
            r.consensus,
            g_f64(&e["consensus"]),
            &format!("merge[{i}] {label} consensus"),
        );
        assert_exact(
            r.tau2,
            g_f64(&e["tau2"]),
            &format!("merge[{i}] {label} tau2"),
        );
        assert_exact(r.q, g_f64(&e["q"]), &format!("merge[{i}] {label} q"));

        // i2 may be null (k < 2).
        match r.i2 {
            Some(v) => {
                assert!(
                    !e["i2"].is_null(),
                    "merge[{i}] {label}: expected i2=null, got {v}"
                );
                assert_exact(v, g_f64(&e["i2"]), &format!("merge[{i}] {label} i2"));
            }
            None => assert!(
                e["i2"].is_null(),
                "merge[{i}] {label}: expected i2 value, got None"
            ),
        }

        assert_eq!(
            r.band.as_str(),
            e["band"].as_str().unwrap(),
            "merge[{i}] {label}: band"
        );

        let expected_tier: Vec<String> = e["tier"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        assert_eq!(r.tier, expected_tier, "merge[{i}] {label}: tier");

        let expected_excl = e["exclusions"].as_array().unwrap();
        assert_eq!(
            r.exclusions.len(),
            expected_excl.len(),
            "merge[{i}] {label}: exclusion count"
        );
        for (re, ee) in r.exclusions.iter().zip(expected_excl) {
            assert_eq!(
                re.name,
                ee["name"].as_str().unwrap(),
                "merge[{i}] {label}: exclusion name"
            );
            assert_eq!(
                re.samples as u64,
                ee["samples"].as_u64().unwrap(),
                "merge[{i}] {label}: exclusion samples"
            );
        }

        // Per-provider random-effects weights (qualifying order): y, effective
        // variance, and the wStar / wStarCapped / wCap diagnostics. All are pure
        // arithmetic off already-asserted τ² → bit-exact against the fixture.
        let expected_weights = e["weights"].as_array().unwrap();
        assert_eq!(
            r.weights.len(),
            expected_weights.len(),
            "merge[{i}] {label}: weight count"
        );
        for (rw, ew) in r.weights.iter().zip(expected_weights) {
            let name = &rw.name;
            assert_eq!(
                name.as_str(),
                ew["name"].as_str().unwrap(),
                "merge[{i}] {label}: weight name"
            );
            assert_exact(
                rw.y,
                g_f64(&ew["y"]),
                &format!("merge[{i}] {label} {name} y"),
            );
            assert_exact(
                rw.v,
                g_f64(&ew["v"]),
                &format!("merge[{i}] {label} {name} v"),
            );
            assert_exact(
                rw.w_star,
                g_f64(&ew["wStar"]),
                &format!("merge[{i}] {label} {name} wStar"),
            );
            assert_exact(
                rw.w_star_capped,
                g_f64(&ew["wStarCapped"]),
                &format!("merge[{i}] {label} {name} wStarCapped"),
            );
            assert_exact(
                rw.w_cap,
                g_f64(&ew["wCap"]),
                &format!("merge[{i}] {label} {name} wCap"),
            );
        }

        // CI bounds: sqrt/t-table only, but assert at 1e-9 per the parity contract.
        assert_rel(
            r.capacity_ci.lower,
            g_f64(&e["capacityCi"]["lower"]),
            &format!("merge[{i}] {label} capacityCi.lower"),
        );
        assert_rel(
            r.capacity_ci.upper,
            g_f64(&e["capacityCi"]["upper"]),
            &format!("merge[{i}] {label} capacityCi.upper"),
        );
        assert_rel(
            r.consensus_ci.lower,
            g_f64(&e["consensusCi"]["lower"]),
            &format!("merge[{i}] {label} consensusCi.lower"),
        );
        assert_rel(
            r.consensus_ci.upper,
            g_f64(&e["consensusCi"]["upper"]),
            &format!("merge[{i}] {label} consensusCi.upper"),
        );
    }
}

#[test]
fn merge_unknown_variance_adopts_max_known() {
    let g = golden();
    // Golden case index 2 = "k4-unknown-var-and-exclusion".
    let case = &g["merge"][2];
    let inputs: Vec<MergeProviderInput> =
        serde_json::from_value(case["providers"].clone()).unwrap();
    let r = merge_providers(&inputs);
    let libre = r
        .weights
        .iter()
        .find(|w| w.name == "librespeed")
        .expect("librespeed in weights");
    assert_exact(libre.v, 80.0, "librespeed effective variance = max known");
}

#[test]
fn merge_empty_qualifying_set() {
    let inputs = vec![MergeProviderInput {
        name: "cloudflare".to_string(),
        y: 500.0,
        v: Some(10.0),
        samples: 2,
        capability: None,
        bca: None,
    }];
    let r = merge_providers(&inputs);
    assert_eq!(r.k, 0);
    assert_exact(r.capacity, 0.0, "empty capacity");
    assert_eq!(r.band.as_str(), "insufficient");
    assert_eq!(r.exclusions.len(), 1);
    assert_eq!(r.exclusions[0].name, "cloudflare");
    assert_eq!(r.exclusions[0].samples, 2);
}

// ── jitter (PDV / IPDV / MAD) vs golden ──────────────────────────────────

#[test]
fn jitter_metrics_match_golden() {
    let g = golden();
    let samples = f64_vec(&g["jitter"]["samples"]);
    let m = jitter_metrics(&samples);
    assert_exact(m.pdv, g_f64(&g["jitter"]["pdv"]), "jitter.pdv");
    assert_exact(
        m.ipdv_mean,
        g_f64(&g["jitter"]["ipdvMean"]),
        "jitter.ipdvMean",
    );
    assert_exact(m.mad, g_f64(&g["jitter"]["mad"]), "jitter.mad");
    assert_exact(
        m.jitter_rfc3550,
        g_f64(&g["jitter"]["jitterRfc3550"]),
        "jitter.jitterRfc3550",
    );
}

#[test]
fn jitter_ground_truth() {
    assert_exact(pdv(&[10.0, 10.0, 10.0, 10.0, 10.0]), 0.0, "pdv flat");
    assert_exact(
        pdv(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0]),
        4.5,
        "pdv ramp",
    );
    assert_exact(ipdv_mean(&[5.0, 5.0, 5.0]), 0.0, "ipdv flat");
    assert_exact(ipdv_mean(&[1.0, 3.0, 2.0]), 1.5, "ipdv");
    assert_exact(
        median_absolute_deviation(&[1.0, 1.0, 1.0, 1.0]),
        0.0,
        "mad flat",
    );
}

// ── bufferbloat + RPM vs golden ──────────────────────────────────────────

#[test]
fn bufferbloat_and_rpm_match_golden() {
    let g = golden();
    let idle = f64_vec(&g["bufferbloat"]["idle"]);
    let loaded = f64_vec(&g["bufferbloat"]["loaded"]);
    let bb = bufferbloat_delta(&idle, &loaded);
    assert_exact(
        bb.delta_ms,
        g_f64(&g["bufferbloat"]["deltaMs"]),
        "bufferbloat.deltaMs",
    );
    assert_exact(
        bb.ratio,
        g_f64(&g["bufferbloat"]["ratio"]),
        "bufferbloat.ratio",
    );
    assert_eq!(
        bb.grade.as_str(),
        g["bufferbloat"]["grade"].as_str().unwrap()
    );
    assert_exact(rpm(&loaded), g_f64(&g["bufferbloat"]["rpm"]), "rpm");
}

#[test]
fn bufferbloat_grade_thresholds() {
    assert_eq!(bufferbloat_grade(4.999).as_str(), "A+");
    assert_eq!(bufferbloat_grade(5.0).as_str(), "A");
    assert_eq!(bufferbloat_grade(29.999).as_str(), "A");
    assert_eq!(bufferbloat_grade(30.0).as_str(), "B");
    assert_eq!(bufferbloat_grade(59.999).as_str(), "B");
    assert_eq!(bufferbloat_grade(60.0).as_str(), "C");
    assert_eq!(bufferbloat_grade(199.999).as_str(), "C");
    assert_eq!(bufferbloat_grade(200.0).as_str(), "D");
    assert_eq!(bufferbloat_grade(399.999).as_str(), "D");
    assert_eq!(bufferbloat_grade(400.0).as_str(), "F");
}

// ── empirical-Bernstein confidence sequence vs golden ────────────────────

#[test]
fn ebcs_matches_golden() {
    let g = golden();
    let src = f64_vec(&g["samples"]["60"]);
    for &t in &[12usize, 25, 50] {
        let cs = empirical_bernstein_cs(&src[..t], 0.05, 0.0);
        let key = t.to_string();
        let ge = &g["ebcs"][key.as_str()];
        assert_eq!(cs.t as u64, ge["t"].as_u64().unwrap(), "ebcs t={t}: t");
        assert_rel(cs.u, g_f64(&ge["U"]), &format!("ebcs t={t}: U"));
        assert_rel(
            cs.mu_hat_mbps,
            g_f64(&ge["muHatMbps"]),
            &format!("ebcs t={t}: muHat"),
        );
        assert_rel(
            cs.half_width_mbps,
            g_f64(&ge["halfWidthMbps"]),
            &format!("ebcs t={t}: halfWidth"),
        );
        assert_rel(cs.width, g_f64(&ge["width"]), &format!("ebcs t={t}: width"));
        assert_eq!(cs.stop, ge["stop"].as_bool().unwrap(), "ebcs t={t}: stop");
    }
}

#[test]
fn ebcs_stop_rule_behavior() {
    // Never stops before t = 12.
    assert!(!empirical_bernstein_cs(&[500.0; 11], 0.05, 0.0).stop);
    // Stops once a long, tight, converged series is inside the band.
    assert!(empirical_bernstein_cs(&[500.0; 600], 0.05, 0.0).stop);
    // RTT gate (> 50 ms) disables early stop even when converged.
    assert!(!empirical_bernstein_cs(&[500.0; 600], 0.05, 60.0).stop);
    // Empty and zero-max inputs are safe.
    assert!(!empirical_bernstein_cs(&[], 0.05, 0.0).stop);
    assert!(!empirical_bernstein_cs(&[0.0, 0.0, 0.0], 0.05, 0.0).stop);
}
