//! Pinned statistical primitives for SpeedQX Methodology v4.
//!
//! These primitives are the portability contract between the TypeScript
//! (website + app WebView) and Rust (CLI) implementations. They MUST behave
//! identically across languages so the committed `golden-vectors.json` fixture
//! asserts parity:
//!
//!   1. [`quantile`]            — type-7 linear interpolation (BIT-EXACT)
//!   2. [`Pcg32`] + [`lemire_bounded`] — deterministic PRNG index streams (BIT-EXACT)
//!   3. [`inv_normal`] / [`phi`] — Acklam / West rational approximations (1e-9 rel)
//!   4. [`t975`]                — hardcoded Student-t 0.975 quantiles (BIT-EXACT)
//!   5. [`sum`] / [`sample_mean`] / [`sample_variance`] — fixed-order summation
//!
//! FP discipline (governing rule): fixed left-to-right summation order, no FMA /
//! fast-math (never `mul_add`), `f64` everywhere, and `u64` wrapping arithmetic
//! for the PCG32 state. This mirrors `stat-primitives.ts` in the SpeedQX web
//! repo byte-for-byte in behavior. See METHODOLOGY.md §11.

// ── type-7 quantile ─────────────────────────────────────────────────────

/// Type-7 (linear-interpolation) quantile on a pre-sorted ascending slice.
///
/// `h = (n - 1) * p`, interpolate between the two bracketing order statistics.
/// BIT-EXACT primitive — identical formula in TS and Rust. `p` must be in
/// `[0, 1]`. Returns `NaN` for an empty slice (mirrors the TS reference).
pub fn quantile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return f64::NAN;
    }
    if n == 1 {
        return sorted[0];
    }
    let h = (n - 1) as f64 * p;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo])
}

// ── PCG32 + Lemire bounded index ─────────────────────────────────────────

/// PCG32 LCG multiplier (Melissa O'Neill's reference constant).
pub const PCG32_MULT: u64 = 6364136223846793005;
/// Default initial state (PCG reference `PCG32_INITIALIZER`). Pinned for golden vectors.
pub const PCG32_DEFAULT_STATE: u64 = 0x853c49e6748fea9b;
/// Default stream increment (odd). Pinned for golden vectors.
pub const PCG32_DEFAULT_INC: u64 = 0xda3e39cb94b95bdb;

/// Lemire multiply-shift bounded index: `floor(u32 * n / 2^32)`.
///
/// Uses the full 64-bit product then shifts right 32. Deterministic across TS
/// and Rust. Returns a value in `[0, n)`.
pub fn lemire_bounded(u32_val: u32, n: usize) -> usize {
    ((u32_val as u64 * n as u64) >> 32) as usize
}

/// PCG32 (permuted congruential generator, XSH-RR 64/32 variant).
///
/// The 64-bit state advances with wrapping `u64` arithmetic; the output
/// permutation (xorshift + rotate) is truncated to 32 bits, exactly mirroring
/// the C reference and the TS BigInt implementation. `inc` must be odd.
#[derive(Debug, Clone)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    /// Construct with the pinned default state + increment (golden-vector stream).
    pub fn new() -> Self {
        Self {
            state: PCG32_DEFAULT_STATE,
            inc: PCG32_DEFAULT_INC,
        }
    }

    /// Construct with an explicit state + increment. `inc` is used verbatim
    /// (the caller is responsible for it being odd, as in the reference).
    pub fn with_seed(state: u64, inc: u64) -> Self {
        Self { state, inc }
    }

    /// Advance the state and return the next uniformly-distributed `u32`.
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(PCG32_MULT).wrapping_add(self.inc);
        let xorshifted = ((((old >> 18) ^ old) >> 27) & 0xffff_ffff) as u32;
        let rot = (old >> 59) as u32; // 0..=31
        xorshifted.rotate_right(rot)
    }

    /// Lemire bounded index into `[0, n)` drawn from the next `u32`.
    pub fn bounded_index(&mut self, n: usize) -> usize {
        lemire_bounded(self.next_u32(), n)
    }

    /// Next double in `[0, 1)`: `next_u32() / 2^32`. Convenience for input synthesis.
    pub fn next_f64(&mut self) -> f64 {
        self.next_u32() as f64 / 4_294_967_296.0
    }
}

impl Default for Pcg32 {
    fn default() -> Self {
        Self::new()
    }
}

// ── invNormal (Acklam) + phi (West) ──────────────────────────────────────

/// Inverse standard-normal CDF via Peter Acklam's rational approximation.
///
/// Max relative error ≈ 1.15e-9 vs the true quantile (no Halley refinement, so
/// TS and Rust stay identical from the same constants). Returns ±∞ at the open
/// boundaries. 1e-9-relative primitive.
#[allow(clippy::excessive_precision)]
pub fn inv_normal(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }

    let a1 = -3.969683028665376e1;
    let a2 = 2.209460984245205e2;
    let a3 = -2.759285104469687e2;
    let a4 = 1.38357751867269e2;
    let a5 = -3.066479806614716e1;
    let a6 = 2.506628277459239e0;
    let b1 = -5.447609879822406e1;
    let b2 = 1.615858368580409e2;
    let b3 = -1.556989798598866e2;
    let b4 = 6.680131188771972e1;
    let b5 = -1.328068155288572e1;
    let c1 = -7.784894002430293e-3;
    let c2 = -3.223964580411365e-1;
    let c3 = -2.400758277161838e0;
    let c4 = -2.549732539343734e0;
    let c5 = 4.374664141464968e0;
    let c6 = 2.938163982698783e0;
    let d1 = 7.784695709041462e-3;
    let d2 = 3.224671290700398e-1;
    let d3 = 2.445134137142996e0;
    let d4 = 3.754408661907416e0;

    let p_low = 0.02425;
    let p_high = 1.0 - p_low;

    if p < p_low {
        let q = (-2.0 * p.ln()).sqrt();
        (((((c1 * q + c2) * q + c3) * q + c4) * q + c5) * q + c6)
            / ((((d1 * q + d2) * q + d3) * q + d4) * q + 1.0)
    } else if p <= p_high {
        let q = p - 0.5;
        let r = q * q;
        (((((a1 * r + a2) * r + a3) * r + a4) * r + a5) * r + a6) * q
            / (((((b1 * r + b2) * r + b3) * r + b4) * r + b5) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((c1 * q + c2) * q + c3) * q + c4) * q + c5) * q + c6)
            / ((((d1 * q + d2) * q + d3) * q + d4) * q + 1.0)
    }
}

/// Standard-normal CDF Φ(x) via Graeme West's `cumnorm` (Hart-style rational
/// approximation), accurate to ~1e-15 using only `exp` + arithmetic.
/// Φ(0) = 0.5 exactly. 1e-9-relative primitive; the companion of [`inv_normal`]
/// for BCa.
#[allow(clippy::excessive_precision)]
pub fn phi(x: f64) -> f64 {
    let xa = x.abs();
    if xa > 37.0 {
        return if x > 0.0 { 1.0 } else { 0.0 };
    }
    let e = (-xa * xa / 2.0).exp();
    let c: f64 = if xa < 7.07106781186547 {
        let mut b = 3.52624965998911e-2 * xa + 0.700383064443688;
        b = b * xa + 6.37396220353165;
        b = b * xa + 33.912866078383;
        b = b * xa + 112.079291497871;
        b = b * xa + 221.213596169931;
        b = b * xa + 220.206867912376;
        let num = e * b;
        b = 8.83883476483184e-2 * xa + 1.75566716318264;
        b = b * xa + 16.064177579207;
        b = b * xa + 86.7807322029461;
        b = b * xa + 296.564248779674;
        b = b * xa + 637.333633378831;
        b = b * xa + 793.826512519948;
        b = b * xa + 440.413735824752;
        num / b
    } else {
        let mut b = xa + 0.65;
        b = xa + 4.0 / b;
        b = xa + 3.0 / b;
        b = xa + 2.0 / b;
        b = xa + 1.0 / b;
        e / b / 2.506628274631
    };
    if x > 0.0 {
        1.0 - c
    } else {
        c
    }
}

// ── Student-t 0.975 table ────────────────────────────────────────────────

/// Student-t 0.975 quantile for `df` degrees of freedom.
///
/// The canonical table (METHODOLOGY.md §6) is pinned for `df ∈ [1, 7]`, which
/// covers every reachable provider count. `df ≤ 1` returns the `df = 1` value;
/// `df ≥ 7` clamps to `df = 7` (2.365) — conservative (wider CI) and
/// unreachable with the current provider registry. BIT-EXACT.
pub fn t975(df: usize) -> f64 {
    match df {
        0 | 1 => 12.706,
        2 => 4.303,
        3 => 3.182,
        4 => 2.776,
        5 => 2.571,
        6 => 2.447,
        _ => 2.365,
    }
}

// ── fixed-order sum helpers ──────────────────────────────────────────────

/// Deterministic left-to-right sum. No pairwise / Kahan reassociation — the
/// summation order is part of the golden-vector contract.
pub fn sum(xs: &[f64]) -> f64 {
    let mut s = 0.0;
    for &x in xs {
        s += x;
    }
    s
}

/// Arithmetic mean via fixed-order [`sum`]. Returns 0 for the empty slice.
pub fn sample_mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    sum(xs) / xs.len() as f64
}

/// Unbiased sample variance (Bessel's `n-1` correction), fixed summation order.
pub fn sample_variance(xs: &[f64]) -> f64 {
    let n = xs.len();
    if n < 2 {
        return 0.0;
    }
    let m = sample_mean(xs);
    let mut s = 0.0;
    for &x in xs {
        let d = x - m;
        s += d * d;
    }
    s / (n - 1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantile_hand_computed() {
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.5);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.25), 1.75);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.0), 1.0);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 1.0), 4.0);
        assert_eq!(quantile(&[7.0], 0.5), 7.0);
        assert!(quantile(&[], 0.5).is_nan());
    }

    #[test]
    fn pcg32_first_outputs() {
        let mut r = Pcg32::new();
        assert_eq!(
            [r.next_u32(), r.next_u32(), r.next_u32(), r.next_u32()],
            [355248013, 41705475, 3406281715, 4186697710]
        );
    }

    #[test]
    fn lemire_bounds() {
        assert_eq!(lemire_bounded(0, 100), 0);
        assert_eq!(lemire_bounded(0xffff_ffff, 100), 99);
        assert_eq!(lemire_bounded(0x8000_0000, 10), 5);
    }

    #[test]
    fn inv_normal_and_phi_anchors() {
        assert_eq!(inv_normal(0.5), 0.0);
        assert_eq!(phi(0.0), 0.5);
        assert_eq!(inv_normal(0.0), f64::NEG_INFINITY);
        assert_eq!(inv_normal(1.0), f64::INFINITY);
        assert!((inv_normal(0.975) - 1.959963984540054).abs() < 1e-8);
        assert!((phi(1.959963984540054) - 0.975).abs() < 1e-9);
    }

    #[test]
    fn t975_table_and_clamps() {
        assert_eq!(t975(1), 12.706);
        assert_eq!(t975(0), 12.706);
        assert_eq!(t975(2), 4.303);
        assert_eq!(t975(7), 2.365);
        assert_eq!(t975(99), 2.365);
    }

    #[test]
    fn fixed_order_sums() {
        let xs = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert_eq!(sum(&xs), 40.0);
        assert_eq!(sample_mean(&xs), 5.0);
        assert_eq!(sample_variance(&xs), 32.0 / 7.0);
        // The famous non-associative residue must be preserved.
        assert_eq!(sum(&[0.1, 0.1, 0.1]), 0.30000000000000004);
    }
}
