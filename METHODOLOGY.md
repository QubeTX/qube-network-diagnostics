# SpeedQX Methodology

**Methodology version: `4.0`** · Applies to: SpeedQX website (speedqx.com), SpeedQX iOS app, `speedqx` CLI (bundled with nd300)

> This document is the single source of truth for how SpeedQX measures internet performance.
> It is committed **byte-identical** to all three product repositories. Every result payload
> produced by any SpeedQX product carries the `methodologyVersion` string above, so any result
> can be traced to the exact algorithm that produced it. Changes to this document bump the
> methodology version — independently of any product's release version.

---

## 1. Versioning & self-identification

- `methodologyVersion` (`"4.0"`) is embedded in every result payload: CLI `--json` output,
  website/app share & clipboard text, and the UI drill-down.
- `platform` identifies the producer: `"web"`, `"app"`, or `"cli"`.
- `providerSet` identifies the test mode: `"fast"` or `"full"` (§3).

## 2. Test lifecycle

1. **Preflight** (platform-dependent): provider DNS/HTTP reachability, network-path
   metadata. Observability only — never merged into results.
2. **Idle latency** (§4): dense HTTP probe engine against the Cloudflare edge.
3. **Per-provider throughput** (§5): providers run **sequentially**, each measuring
   download then upload, with loaded-latency probes running during Cloudflare saturation.
   Inter-provider transition gap: **1000 ms**.
4. **Aggregation** (§6): cross-provider merge, confidence intervals, agreement, grades.

## 3. Provider registry & platform availability

| Provider | Transport | Directions | Browser/WebView | CLI | Capability prior |
|---|---|---|---|---|---|
| Cloudflare (`speed.cloudflare.com`) | HTTPS GET/POST | DL + UL + loaded latency + packet loss | ✅ | ✅ | 1.00 |
| M-Lab NDT7 | WebSocket (`net.measurementlab.ndt.v7`) | DL + UL, kernel `TCPInfo.MinRTT` | ✅ | ✅ | 0.70 |
| M-Lab MSAK | WebSocket ×2 streams (`net.measurementlab.throughput.v1`) | DL + UL, kernel MinRTT | ✅ | ✅ | 0.85 |
| LibreSpeed | HTTPS (pinned CORS-enabled backend) | DL + UL | ✅ | ✅ | 0.95 |
| fast.com (Netflix) | HTTPS to OCA nodes (token via relay on web/app) | DL (+UL where offered) | ✅* | ✅ | 1.00 |
| CacheFly | HTTPS range requests (`cachefly.cachefly.net/{1,10,100}mb.test`) | DL only | ✅ | ✅ | 0.95 |
| Vultr | HTTPS range requests, multi-POP min-RTT selection (`*-ping.vultr.com/vultr.com.100MB.bin`) | DL only | ✅ | ✅ | 0.95 |
| Apple networkQuality | HTTPS ×4 parallel (`mensura.cdn-apple.com`) | DL + UL | ❌ (CORS) | ✅ | 1.00 |

\* fast.com on web/app is labeled an *estimate* (token relay; OCA selection is relay-IP-based)
and hides itself on failure rather than reporting a degraded number.

**The spec defines the full set; each platform contributes what it can run.** Providers a
platform cannot run appear in the payload with `availability: "unavailable-platform"` —
visible, never silently missing. Failed providers appear with `availability: "failed"`.

**Test modes:**
- **FAST** — Cloudflare + NDT7 + MSAK, with confidence-sequence early termination (§8).
  Target ≈ 15–25 s. Default CLI flag: `speedqx --fast`.
- **FULL** — every provider available on the platform, fixed durations, no early stop.
  Default for `speedqx` (CLI).

## 4. Latency instrument

- Endpoint: `https://speed.cloudflare.com/__down?bytes=0`, cache-busted per probe.
- Sample count: `clamp(50, round(durationSeconds × 3.3), 200)` — auto (30 s) → 99.
- **3 warmup probes discarded** (DNS + TCP + TLS amortization); 50 ms inter-probe interval.
- Timing: `PerformanceResourceTiming.responseStart − requestStart` where trustworthy
  (`0 < rtt < 3 × wall-clock fallback`), else wall-clock. CLI: monotonic clock.
- Paused while the page/tab is hidden (browser/WebView) to avoid throttling artifacts.
- Kernel cross-check: NDT7/MSAK report `TCPInfo.MinRTT` (µs → ms) from the server TCP stack.
- **Headline ping = min-RTT** across the probe engine and kernel MinRTT values — the
  physical floor of the path. P50/P75/P95/P99/mean/max are first-tap drill-down.
- Cross-provider latency blend (for the latency stats block): Cloudflare 0.4 / NDT7 0.6.

## 5. Per-provider throughput pipeline

Applied independently per provider, per direction:

1. **Raw samples**: throughput readings on 250–500 ms ticks. HTTP providers use adaptive
   request sizing (~2 s of transfer at the last measured rate). WebSocket upload frames grow
   by the reference 16×-bytes-sent rule.
2. **Warm-up removal — plateau detector** (replaces the fixed 30% discard of v2/v3):
   steady state begins at the first index `t` where **3 consecutive** samples each sit
   within **±10%** of `median(samples[t..end])`. The cut is clamped to **[10%, 40%]** of the
   series. Samples before `t` are discarded.
3. **Outlier filter**: IQR fences with **k = 1.5** (applied when n ≥ 4).
4. **Upload only**: keep the **fastest 50%** of post-warm-up samples (industry-standard
   compensation for sender-side buffering artifacts), before step 3.
5. **Location estimate — modified trimean**: `(P10 + 8·P50 + P90) / 10`.
   This is the same statistic used by Ookla's Speed Score — a deliberate, citable choice.
6. **Cross-check**: Hodges–Lehmann estimator computed on the cleaned samples; if
   `|HL − trimean| / trimean > 0.15`, an internal instability flag is raised on the provider.
7. **Uncertainty**: **circular block bootstrap** — block length `ℓ = max(2, round(n^⅓))`
   (throughput samples are autocorrelated; IID resampling understates variance),
   **B = 2000** resamples, **BCa** (bias-corrected accelerated) intervals at 95%.
   The bootstrap variance of the trimean is the provider's variance `v_j` in §6.

## 6. Cross-provider merge (the SpeedQX hybrid)

Providers with **≥ 4** cleaned samples in a direction qualify for the merge
(`MIN_MERGE_SAMPLES = 4`); the rest are recorded in `mergeExclusions` — never silently dropped.

Each qualifying provider contributes `(y_j = trimean, v_j = bootstrap variance)`.

**Between-provider heterogeneity (DerSimonian–Laird):**

```
w_j  = 1/v_j                     mu_F = Σ w_j·y_j / Σ w_j
Q    = Σ w_j·(y_j − mu_F)²       C    = Σ w_j − (Σ w_j²)/(Σ w_j)
τ²   = max(0, (Q − (k−1)) / C)
I²   = max(0, (Q − (k−1)) / Q)
```

**Headline = CAPACITY** — the speed the tests that can actually saturate the line agree on:

```
tier = { j : y_j ≥ 0.85 · max(y) }        # if k ≥ 3 and |tier| < 2 → top-2 providers
w'_j = capability_j / (v_j + τ²)           # capability priors from §3
capacity = Σ_tier w'_j·y_j / Σ_tier w'_j
```

**Secondary = CONSENSUS** — the conservative, all-providers number:

```
w*_j = 1/(v_j + τ²)               consensus = Σ w*_j·y_j / Σ w*_j
```

**Confidence interval** — Hartung–Knapp–Sidik–Jonkman on the random-effects model:

```
q  = Σ w*_j·(y_j − consensus)² / (k−1)      q' = max(1, q)
SE = sqrt(q' / Σ w*_j)
CI = estimate ± t(k−1, 0.975) · SE          # t-table (df: 1→12.706, 2→4.303, 3→3.182, 4→2.776, 5→2.571)
```

- **k = 2**: τ² is untrustworthy → report the honest union band
  `[min(y_j − 1.96·se_j), max(y_j + 1.96·se_j)]` and agreement = "insufficient".
- **k = 1**: no merge; the provider's own BCa interval is the CI.
- Per-provider weight is additionally capped at **0.70** of the total (defense in depth).
- Unknown-variance providers are assigned the **maximum known variance** (least trusted).

**Provider agreement (replaces the v3 ">30% spread" flag):**

| I² | Band | Presentation |
|---|---|---|
| < 0.25 | High | normal |
| 0.25–0.50 | Moderate | normal |
| 0.50–0.75 | Low | caution chip |
| > 0.75 | Very low | show the **range**, not a single headline |

Agreement is **diagnostic, never failure** — AQM fair-queuing and cross-traffic legitimately
produce single- vs multi-stream spread. When single-flow (NDT7) and multi-flow figures
diverge materially, both are disclosed (flow-count transparency).

## 7. Jitter, bufferbloat, responsiveness, packet loss

- **Jitter (canonical) — PDV** (RFC 5481 flavor): `P95(RTT) − P50(RTT)`.
  Secondary: `IPDV mean` (mean |consecutive ΔRTT|) and `MAD × 1.4826`.
  RFC 3550 EWMA (`J += (|D|−J)/16`) is retained as a compatibility field only.
- **Bufferbloat delta (canonical)**: `P95(loaded RTT) − P50(idle RTT)` in ms, graded
  **A+ < 5 · A < 30 · B < 60 · C < 200 · D < 400 · F ≥ 400**.
  Loaded RTT comes from probes fired during Cloudflare saturation (200 ms throttle,
  max 50 points per direction). The loaded/idle **ratio** is a disclosed secondary.
- **Responsiveness (RPM, approx)**: `60000 / P50(loaded RTT ms)` — the working-conditions
  round-trips-per-minute figure aligned with the IETF responsiveness draft, labeled
  *approx* because browser probes are same-origin multiplexed rather than the full
  foreign+self probe design.
- **Packet loss**: UDP via Cloudflare TURN relay (1000 packets), reported as a percentage.
  On TURN failure, packet loss is `unavailable` — never fabricated. NDT7's TCP-derived
  loss signals are disclosed in the provider drill-down where available.

## 8. FAST mode early termination

Naive "check the CI after every sample and stop when it's tight" is statistically invalid
(optional-stopping bias → real coverage well below nominal). FAST mode instead uses an
**anytime-valid confidence sequence** (empirical-Bernstein type) per provider:

- Samples are rescaled to `[0, 1]` by a generous cap `U` (2× the fastest observed sample).
- The CS is valid at every sample size simultaneously; stopping when
  `CS half-width ≤ max(5% of estimate, 2 Mbps)` incurs **no** peeking penalty.
- Hard cap: **25 s** per provider; a provider that hasn't converged reports what it has.
- Aggressiveness is gated on measured RTT (never on early throughput guesses):
  paths with min-RTT > 50 ms use the full duration — early termination on
  high-RTT/low-throughput paths is unsafe (TurboTest, 2026).

FULL mode uses fixed durations and never early-terminates.

## 9. Result payload schema

All platforms emit the same logical schema (camelCase in TS, snake_case in Rust):

```
methodologyVersion, platform, providerSet,
ping (min-RTT), jitter (PDV),
capacityMbps { download, upload } ± CI,
consensusMbps { download, upload } ± CI,
agreement { i2, band },
packetLoss, rpm,
bufferbloat { grade, deltaMs, ratio, unloadedLatencyMs, loadedLatencyMs { download, upload } },
latencyStats { p50, p75, p95, p99, min, max, mean, stddev, pdv, jitterMad, jitterRfc3550 },
stability { downloadCV, uploadCV, downloadStable, uploadStable },   # stable = CV < 0.15
providers [ { name, server, availability: ran|unavailable-platform|failed,
              pingMs, downloadMbps, uploadMbps, samples, bytes, error? } ],
mergeExclusions [ { provider, direction, samples } ],
confidenceIntervals { download, upload, confidenceLevel: 0.95 }
```

## 10. Progressive disclosure

| Layer | Content | Surface |
|---|---|---|
| **L0 — Headline** | capacity ± CI, ping, jitter, bufferbloat grade, RPM | hero numbers / CLI results table |
| **L1 — Quality** | consensus, CI bounds, I² agreement band, stability CV, packet loss, methodology chip | one tap / summary rows |
| **L2 — Providers** | per-provider breakdown, exclusions, greyed `unavailable-platform` entries | expander / per-provider sections |
| **L3 — Raw** | sample arrays, full percentiles, preflight, network metadata, JSON export | deep drill-down / `--json` |

Everything computed is observable at some layer. Nothing on the headline lacks a
confidence annotation.

## 11. Portability contract (golden vectors)

TypeScript (website + app WebView) and Rust (CLI) implementations must agree on shared
golden-vector fixtures (`golden-vectors.json`, committed to all three repos):

| Primitive | Pinned implementation | Comparison |
|---|---|---|
| `quantile(sorted, p)` | Type-7 linear interpolation: `h=(n−1)p`, interpolate | **bit-exact** |
| `invNormal / Phi` | Acklam (or Wichura AS241) rational approximation | ≤ 1e-9 relative |
| PRNG | **PCG32**, fixed seed + stream, **Lemire** bounded index (`floor(u32/2³²·n)`) | **bit-exact** index streams |
| t-quantiles | hardcoded table (§6) | **bit-exact** |
| FP discipline | fixed summation order; no FMA/fast-math; no JS 32-bit bitwise on 64-bit values | — |

Arithmetic-only estimators (trimean, IQR, plateau, merge, PDV, bufferbloat) are compared
**bit-exact**; transcendental paths (BCa, confidence sequences) at 1e-9 relative tolerance.
The governing principle of v4: **prefer closed-form arithmetic over iterative or
transcendental-heavy methods** — exact two-language reproducibility is a product feature.

## 12. Change log of this spec

- **4.0** (2026-07): First unified spec. Capacity/consensus hybrid merge with DL τ² + HKSJ
  CIs and I² agreement; plateau warm-up detection; block bootstrap + BCa; PDV jitter;
  delta-ms bufferbloat + RPM; min-RTT headline ping; FAST/FULL modes with anytime-valid
  early stopping; 8-provider registry; golden-vector portability contract.
  Supersedes the v2/v3 era (fixed 60/40 or [0.3,0.7]-clamped 2-provider merges,
  fixed 30% slow-start discard, RFC 3550 headline jitter, ratio-graded bufferbloat).
