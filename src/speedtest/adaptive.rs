//! Adaptive transfer sizing for per-request providers.
//!
//! Fixed chunk sizes invert the sampling rate: fast links finish huge chunks
//! quickly but slow links produce almost no samples (LibreSpeed's old fixed
//! 100 MB chunks yielded 0-2 samples in a 30s window on sub-50 Mbps lines —
//! far below what the trimean pipeline needs). Sizing each request to a
//! target duration keeps the per-request sample rate roughly constant across
//! link speeds.

/// Bytes for the next request so it takes ~`target_secs` at the most recently
/// measured throughput, clamped to `[min, max]`. Non-finite or non-positive
/// input falls back to `min` (the safe warmup size).
pub(crate) fn adaptive_chunk_bytes(last_mbps: f64, target_secs: f64, min: u64, max: u64) -> u64 {
    if !last_mbps.is_finite() || last_mbps <= 0.0 || !target_secs.is_finite() || target_secs <= 0.0
    {
        return min;
    }
    let bytes = last_mbps * 1_000_000.0 / 8.0 * target_secs;
    if !bytes.is_finite() {
        return max;
    }
    (bytes as u64).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn chunk_targets_two_seconds() {
        // 100 Mbps × 2s = 25 MB (decimal).
        let bytes = adaptive_chunk_bytes(100.0, 2.0, MB, 100 * MB);
        assert_eq!(bytes, 25_000_000);
    }

    #[test]
    fn chunk_clamps_min_max() {
        assert_eq!(adaptive_chunk_bytes(0.1, 2.0, MB, 100 * MB), MB);
        assert_eq!(adaptive_chunk_bytes(100_000.0, 2.0, MB, 100 * MB), 100 * MB);
    }

    #[test]
    fn chunk_handles_zero_and_nan() {
        assert_eq!(adaptive_chunk_bytes(0.0, 2.0, MB, 100 * MB), MB);
        assert_eq!(adaptive_chunk_bytes(-5.0, 2.0, MB, 100 * MB), MB);
        assert_eq!(adaptive_chunk_bytes(f64::NAN, 2.0, MB, 100 * MB), MB);
        assert_eq!(adaptive_chunk_bytes(100.0, f64::NAN, MB, 100 * MB), MB);
    }
}
