# Live SpeedQX / Ookla comparison - September 5, 2026

These observations do not prove that SpeedQX is more accurate than Ookla. SpeedQX provides a traceable sustained-throughput calculation, but accuracy requires an independent reference for a defined quantity. That reference was not available on this live Wi-Fi connection. No algorithm parameters were adjusted to favor a result.

## Observed results

| Test | Local start/result time (Chicago) | Download Mbps | Upload Mbps | Displayed idle latency | Duration |
|---|---|---:|---:|---:|---:|
| SpeedQX Quick 1 | 18:15:55 start | 267 | 160 | 63.9 ms median HTTP | 66.2 s |
| Ookla Multi 1, operator screenshot | 18:19 result | 297.05 | 142.42 | 7 ms ping | Not recorded |
| SpeedQX Quick 2, unpaired follow-up | 18:44:41 start | 159 | 162 | 49.4 ms median HTTP | 63.7 s |
| SpeedQX Quick 3, unpaired follow-up | 18:48:19 start | 174 | 177 | 49.1 ms median HTTP | 63.8 s |

The first SpeedQX result is 10.12% lower on download and 12.34% higher on upload than the supplied Ookla result. These percentages describe disagreement, not error. Ookla used SmartBurst LLC in Aubrey, TX. The connection was Pavlov Media via an Intel Wi-Fi 7 BE202 adapter reporting a 574 Mbps PHY link; that link rate is not an independent internet-capacity measurement. The operator's subscribed rate was not established.

SpeedQX's three displayed downloads span 159-267 Mbps (median 174), and uploads span 160-177 Mbps (median 162). The later readings followed the release correction and screenshot work, approximately half an hour after the first pair. They cannot be paired against the earlier Ookla result. Both Cloudflare and MSAK showed the later download reduction. Changing conditions are a possible explanation, but these data do not isolate a cause or validate repeatability under fixed conditions. No sample was discarded.

## Protocol and evidence limits

The predeclared schedule was three sequential alternating pairs: SpeedQX/Ookla, Ookla/SpeedQX, SpeedQX/Ookla. Browser security policy rejected automated access to Ookla. The operator's supplied screenshots provide one manual result; the later SpeedQX runs are unpaired observations. The planned six-run comparison was not completed.

Tests were started sequentially. No concurrent test or bulk artifact transfer was intentionally started during the SpeedQX measurements. Household background traffic and Wi-Fi conditions were not independently controlled. Quick and the existing M-Lab consent setting were preserved. The three SpeedQX runs reported 3347.7 MB confirmed payload and 4031.7 MB charged against their budgets; protocol overhead is additional. The final SpeedQX result is left open in the browser.

[observations.json](observations.json) retains displayed values, providers, times, protocol deviations and source-image hashes. Raw screenshots containing the public IP are not copied into this repository.

## SpeedQX calculation inspected

The deployed site is version 4.0.3, source `82c9a45593ed55e2d19d053caf1521eabf23a9cd`, using methodology 5.0. The inspected engine matches that production source.

- [Measurement](https://github.com/QubeTX/speedtest/blob/82c9a45593ed55e2d19d053caf1521eabf23a9cd/src/services/measurement-v5.ts#L79): after the two-second warm-up, sustained Mbps is received payload bytes multiplied by 0.008 and divided by measured milliseconds. Recorded stalls remain in the denominator. Counter resets, invalid timestamps and corrupt traces are rejected. Qualification requires four seconds of valid evidence and receiver-confirmed accounting.
- [Aggregation](https://github.com/QubeTX/speedtest/blob/82c9a45593ed55e2d19d053caf1521eabf23a9cd/src/services/measurement-v5.ts#L132): Cloudflare and consenting MSAK each receive one vote. Their median is the arithmetic mean when both qualify. Deep combines repeated runs by bytes/time within each provider. NDT7 does not add a second M-Lab vote.
- [Acquisition](https://github.com/QubeTX/speedtest/blob/82c9a45593ed55e2d19d053caf1521eabf23a9cd/src/services/acquisition-v5.ts#L69): two logical HTTP streams or MSAK sockets; completed HTTP requests or server application counters provide upload evidence. Logical HTTP streams do not guarantee separate TCP connections because browsers may multiplex them.
- [Schedule and latency](https://github.com/QubeTX/speedtest/blob/82c9a45593ed55e2d19d053caf1521eabf23a9cd/src/services/engine-v5.ts#L9): Quick uses ten-second primary directions within 90 seconds; Deep repeats twenty-second primary directions within five minutes. Ping is median HTTP RTT to Cloudflare after a discarded connection warm-up probe; jitter is P95 minus median. A different endpoint and statistic make the supplied Ookla 7 ms value unsuitable as an error reference.

The estimated ceiling requires two non-overlapping three-second windows agreeing within 10% and is withheld if below sustained throughput. This guards against an isolated spike; it does not calibrate physical line capacity. No fastest-half filter or confidence-triggered early stop is used.

## What can be established about Ookla

The screenshot confirms Multi mode and the selected server. A primary controlled study published in 2023 found that Ookla adapted connection count and test duration, and that server choice and path conditions affected disagreement with NDT7. The authors distinguished average transferred throughput from the reported value and stated that the then-current sampling method was not public. These are historical findings, not verification of every detail in the September 2026 browser implementation. [MacMillan et al., A Comparative Analysis of Ookla Speedtest and NDT7](https://arxiv.org/html/2205.12376).

The current official Ookla methodology page could not be fetched in this session. Older official HTTP-client documentation does not establish the exact sample-selection rule of the current client. No proprietary or leaked implementation document was used.

## Accuracy assessment and remaining work

For sustained application throughput on a defined path, received payload divided by the corresponding interval is an appropriate transparent quantity. SpeedQX preserves stalls and separates unconfirmed upload evidence. With only two primary providers, however, its cross-provider median is not robust against one slow path and is not a representative sample of the whole internet. A longer test can improve sampling without automatically improving accuracy.

For peak access-link capacity, parallelism, server placement, warm-up and cross-traffic behavior matter. A lower result does not establish a more accurate capacity estimate, and a higher one does not prove inflation. No general superiority claim is justified.

Three obligations remain:

1. **Controlled qualification.** Task #sqacc retains earlier loss-path paired-median misses of 35.11% download and 11.95% upload against a 5% repeatability target. These are earlier qualification discrepancies, not errors measured in these live runs and not an Ookla comparison.
2. **NDT7 upload diagnosis.** Upload was unavailable in all three runs. The first trace recorded no server application bytes although server TCP minimum RTT was available. The collector reads `AppInfo.NumBytes`; the [NDT7 protocol](https://github.com/m-lab/ndt-server/blob/main/spec/ndt7-protocol.md) makes AppInfo optional. This establishes missing usable counter evidence, not its cause. NDT7 did not affect the headline. A separately labeled diagnostic should inspect sanitized message fields and close reasons before selecting a fix; TCP or sender counters must not silently replace application goodput.
3. **A fair comparative reference.** Use an isolated controlled network with independent receiver-side payload counters/timestamps aligned to the measurement interval. Predeclare rate, RTT, loss/jitter, cross-traffic, congestion-control, device/browser and test-order cases. Retain every repetition and provider-limited result. Report signed bias, absolute error, tail error and repeatability separately, with distinct sustained-goodput and capacity targets. Complete paired trials when the browser access and independent reference setup permit them.

The observations and source assessment are complete. The requested guarantee that SpeedQX is more accurate remains unproven and is not marked complete.
