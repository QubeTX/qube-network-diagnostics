# SpeedQX measurement methodology v5.0

Status: release authorized September 5, 2026 with disclosed validation limits. Physical-device acceptance and lossy-path repeatability remain follow-ups, not passed checks; evidence is tracked in evidence/v5.

Sustained application throughput on this device, across the paths tested. Your radio, browser, operating system and the test servers are all part of that result.

## Download and upload

Each transfer records elapsed time and cumulative payload bytes. After a fixed two-second warm-up, sustained speed is measured bytes divided by measured time. A pause stays in the measurement. Upload uses completed HTTP requests or the receiving server’s byte count; bytes merely queued for sending cannot establish a measured result.

At least four seconds of valid measurement are required for a qualifying source. Shorter traces remain provisional in details. An unavailable value means there was not enough evidence; a measured zero means the transfer stalled.

## One common comparison

Cloudflare and M-Lab MSAK are the primary sources, each using two logical streams. Their qualifying sustained estimates receive one vote each; the headline is their median. Deep combines repeated runs within each provider before comparing the two providers. A single successful source is explicitly labeled.

M-Lab NDT7 is a separate single-stream measurement. NDT7 and MSAK share a network and never count as two independent networks. Deep also attempts LibreSpeed, CacheFly, Vultr and fast.com through existing public services. Supplementary measurements stay in details and cannot change the headline. Apple networkQuality is outside this common comparison.

## Estimated ceiling

A ceiling needs two non-overlapping windows of at least three seconds whose speeds are within 10%. The lower speed of the pair is the candidate. An isolated spike cannot establish it. A candidate below the sustained result is withheld, including after repeated-provider and cross-provider aggregation; it is never raised artificially. The estimate describes repeatable throughput observed during this run; it is not the physical capacity of your ISP line.

If provider ceilings differ by more than 20%, the common ceiling is withheld and the disagreement is shown. No ceiling is a valid outcome, especially for short, interrupted or unstable transfers.

## Ping, jitter and load

Ping is the median of idle HTTP round trips to Cloudflare’s common reference endpoint. One initial probe warms the connection and is discarded; up to twelve idle probes follow. The minimum HTTP RTT remains a separate detail. Jitter is the difference between the 95th percentile and median of the idle samples.

Download-loaded and upload-loaded HTTP RTT use that same reference while the respective transfer is active. HTTP includes browser and server processing and can include connection setup. A server’s TCP minimum RTT, when provided, is labeled separately. HTTP probe failures are not packet loss. This common run does not measure UDP loss; unavailable UDP loss is never displayed as zero.

## Quick, Deep and data use

Quick is the default and has a 90-second overall cap. Each primary direction gets a bounded ten-second transfer including warm-up. Deep has a five-minute overall cap and repeats the primary comparison with twenty-second directions. NDT7 remains limited to its ten-second protocol schedule. Server protocol limits and rate limits take priority.

The default payload budget is 5 GB for Quick and 20 GB for Deep, using decimal units. Settings let you choose a smaller ceiling. These are upper estimates, not promised consumption. Actual payload bytes are shown separately. Upload budget accounting includes bytes offered to the transport, so it is conservative; network headers, encryption and unrelated traffic are outside these totals. Already-arriving socket data may exceed the ceiling by a bounded in-flight amount.

Stop preserves qualifying partial results and records why the run ended. Time and byte limits do the same. Leaving the app, background throttling or a detected network transition ends the comparable run and excludes the ambiguous final interval. A transition that the operating system does not expose may still affect a run.

## Variation and repeatability

Details show each provider, its chronological trace and observed spread. The estimated repeatability range describes the observed windows or provider values. It assumes conditions remain similar. Windows can overlap and samples can be correlated; this is not a calibrated 95% confidence interval or a guarantee of future accuracy.

Continued ramp-up, stalls and provider disagreement are disclosed. Compare sequential runs using the same profile, endpoints and device. Different Wi-Fi conditions, radios and devices are measured variables, so matching formulas alone cannot guarantee matching results.

## Privacy and consent

SpeedQX transfers synthetic payloads, not your files. The remote test services necessarily see your public IP address and connection metadata. No new SpeedQX measurement servers are used.

M-Lab publishes measurement results and IP addresses. Both M-Lab tests require your consent before discovery or transfer. Without consent, the primary result uses Cloudflare alone and is labeled single-source. The CLI requires --accept-mlab. Review each provider’s policy before testing on a connection with sensitive or metered usage.

## Reading older results

Saved results retain their methodology version. Version 4 used a different sample-selection and aggregation pipeline, so its headline and uncertainty fields keep their original interpretation. Version 5 maps the existing headline download and upload fields to sustained throughput and adds explicit validity, ceiling and acquisition fields. The result details and exported record identify the version.

## Normative acquisition contract

The machine-readable contract is `src/services/measurement-contract-v5.json` in QubeTX/speedtest. TypeScript is canonical for the browser and generated Expo WebView engine. Rust remains an independent implementation. The generated mobile manifest pins the canonical commit and SHA-256 of every copied source, fixture, geometry and documentation file; CI rejects dirty-source pins or content drift.

All trace timestamps are monotonic milliseconds relative to transfer start. Payload counters are nonnegative safe integers (maximum 2^53 - 1 for interoperable JSON). Timestamps must strictly increase, and counters must never decrease within a trace. An invalid timestamp, counter reset or explicitly corrupt provider measurement invalidates the trace. A new connection session requires a new trace. Endpoint provenance removes signed query tokens. Direction, transport, logical stream count, accounting method, warm-up duration and stop reason accompany the counters.

A scheduled 500 ms counter sample records real zero-byte intervals. A final sample records a partial final interval. The first eligible interval begins at a recorded point at or after the two-second boundary: unknown byte arrivals across the boundary are not interpolated. Warm-up samples remain in the exported trace. Invalid intervals and their adjoining boundary samples cannot contribute to sustained or ceiling windows. No retrospective plateau selection, fastest-half upload filter, outlier deletion, or confidence-triggered early stop is applied.

Sustained Mbps = measured payload bytes × 0.008 / measured milliseconds. Qualification requires at least 4,000 valid measured milliseconds and receiver confirmation. Insufficient-duration or sender-only evidence is provisional. No usable evidence is unavailable. Only qualifying primary sources contribute to the headline, including legitimate measured zeros. Repeated primary directions are combined by bytes/time within their provider, then each primary network receives one median vote. Supplementary and single-stream NDT7 results remain separate.

Ceiling windows start at actual measurement boundaries and end at the first boundary at least 3,000 ms later. Windows cannot bridge excluded intervals. For two non-overlapping windows, (higher - lower) / higher must be at most 0.10. The largest lower value of any qualifying pair is the provider candidate. Repeated provider candidates use their median. Common provider ceilings use their median only when their observed difference is at most 20% of the higher value. A trace with increasing first-to-last non-overlapping windows above 20% is disclosed as continued ramp-up.

HTTP uses two logical lanes (an HTTP/2 connection can multiplex them). Downloads adapt from 1 MB toward two-second requests, within 64 KB to 8 MB. Uploads adapt from 1 KB toward 250 ms acknowledged requests, within 1 KB to 8 MB. Request growth is limited to 2× per completion to avoid a fast initial response creating an oversized upload across warm-up. HTTP upload confirmation granularity and request overhead remain transport limitations, particularly at high RTT. Downloads consume streaming binary bodies; the bounded non-streaming WebView fallback requires a valid Content-Length no larger than its reserved allocation. Binary buffers are synthetic and bounded; text decoding is never used for payloads.

MSAK uses two WebSockets with the provider protocol and millisecond duration parameter, capped at 25 seconds. NDT7 uses one WebSocket and a ten-second client direction schedule. Upload frames start at 8 KiB and grow up to 1 MiB, with bounded send queues; upload results use server application-byte counters. Download results count locally received binary messages. Protocol messages over 16 MiB are refused. Server TCP MinRTT is converted from microseconds and exposed separately. No UDP loss or TCP retransmission percentage is inferred from HTTP failures or unacknowledged application bytes.

Quick: idle probes, Cloudflare download/upload, consenting MSAK download/upload, consenting NDT7 download/upload. Deep: idle probes, two rounds of Cloudflare and consenting MSAK download/upload, consenting NDT7, then LibreSpeed download/upload and CacheFly, Vultr and fast.com download-only where transport qualification succeeds. Primary directions are 10 seconds Quick or 20 seconds Deep including warm-up; NDT7 and supplementary directions are 10 seconds. A 500 ms gap separates provider sessions. The overall 90/300 second cap also bounds discovery and idle probes. Provider refusal, rate limits, cancellation or a byte ceiling can shorten the schedule.

One owner tracks all transfer requests, sockets, timers and reservations. Payload received plus upload payload offered consumes the byte budget, including warm-up and cancelled requests. Confirmed payload is reported separately as bytesTransferred. Reservations prevent concurrent HTTP lanes from spending the same remaining budget. HTTP bodies, protocol queues and already-arriving WebSocket data are bounded separately; the payload budget is not an on-wire byte counter. HTTP probe requests and provider discovery exchange small protocol metadata and are outside payload totals. Auxiliary DNS and network-information checks are separate diagnostics, start after completed measurement, and never delay publication of the bounded measurement result. They are skipped after an interrupted run.

The browser listens for visibility and observable network-type/offline transitions. The native adapter forwards AppState and network-type/connectivity changes with a run identifier on every asynchronous callback. Rust watches the selected local source address without transmitting UDP probes. These are best-effort transition signals: an unreported same-address handoff can remain undetected. A detected transition preserves earlier valid measurement and excludes the ambiguous final counter interval.

## Validation and limitations

Deterministic fixture replay compares complete estimates and primary aggregation across TypeScript and Rust. The paced loopback acquisition harness independently records server-delivered payload and aligns it to the client's measured interval. It reports configured rate limits separately from delivered application throughput. Its v4 estimator replay compares selection bias on the same traces; it is not a full comparison of all legacy transports.

An application-level pacer does not establish physical-device accuracy, TCP packet-loss behavior, radio performance or confidence-band coverage. Physical iOS/Android, hosted platform builds, matched real-path runs and paired animation impact remain separate acceptance evidence. No nominal 95% accuracy or coverage guarantee is made. The intended clean controlled-path target is within 5%; failures and regressions must remain visible in the validation report.

## Historical interpretation

`docs/METHODOLOGY-v4.md` preserves the previous contract. Historical records are not recomputed or relabeled as v5. Numeric legacy headline aliases remain for existing consumers; consult the explicit v5 qualification and nullable sustained fields to distinguish unavailable from measured zero.

## Primary references

- [M-Lab measurement and data policy](https://www.measurementlab.net/tests/ndt/)
- [NDT7 protocol](https://github.com/m-lab/ndt-server/blob/main/spec/ndt7-protocol.md)
- [M-Lab MSAK](https://github.com/m-lab/msak)
- [Time-uniform confidence sequence research](https://arxiv.org/abs/1810.08240)
