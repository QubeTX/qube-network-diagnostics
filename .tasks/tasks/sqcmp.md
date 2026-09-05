TT;DR: Compare the live SpeedQX and Ookla websites after delivering the iOS candidate, then assess measurement accuracy without choosing a winner from unreferenced speed numbers.

## Why
The operator requested a side-by-side live comparison on 2026-09-05 and prioritizes accuracy over test duration. The initial iOS upload finished before this task began; the corrected 3.0.0 (18) upload also completed before follow-up observations.

## Scope
Use the operator's browser on the same machine and connection. Run three paired comparisons in alternating order: SpeedQX/Ookla, Ookla/SpeedQX, SpeedQX/Ookla. Keep tests sequential to avoid self-contention; preserve ordinary site defaults and record server/provider, duration and evidence. Do not discard inconvenient results. Inspect current official Ookla documentation and the deployed SpeedQX calculation/acquisition contract. Distinguish sustained multi-network application throughput, peak path capacity, repeatability and accuracy against known truth. Correct demonstrated flaws only with appropriate regression and deployment verification.

## Impact
Provides a reproducible comparison and an honest assessment of what the headline measurements mean. Existing lossy-path misses in #sqacc remain open; plan speed and a competitor's result are not ground truth.

## Verification
- [x] Three SpeedQX runs and one operator-supplied Ookla run retained; the browser-policy blocker and deviation from six alternating runs are documented.
- [x] SpeedQX implementation and available primary Ookla research evaluated with exact source references; current proprietary sampling details remain unverified.
- [x] Conclusions distinguish disagreement, repeatability and accuracy. No unsupported algorithm correction was made.
- [ ] Independent controlled paired evidence establishes comparative accuracy for a predeclared measurement target.

## Status
Open for comparative qualification; observations and source assessment are complete in docs/evidence/live-comparison-2026-09-05/REPORT.md and observations.json. SpeedQX Quick returned 267/160, 159/162 and 174/177 Mbps; the one manual Ookla result was 297.05/142.42 Mbps. Later SpeedQX observations are unpaired. Automated Ookla access remains blocked by browser security policy, and no independent live payload reference is available. Neither a superiority guarantee nor repeatability qualification is complete. #sqacc remains open, and repeated NDT7 upload unavailability is recorded for a separately instrumented diagnosis. Resume with a permitted paired setup and independent receiver-side truth; do not select favorable repetitions or reinterpret these disagreements as error.

## Activity
- 2026-09-05 - codex: created after confirming iOS submission 82f4b8e7-2e67-433d-9b8e-b07b456096c1 FINISHED. Preserved #sqacc and its missed targets; predeclared three alternating pairs before observing live results.

- 2026-09-05 - codex: retained first SpeedQX sample and manual Ookla screenshot data; two direct browser attempts were blocked, so no alternate route was attempted. Paused testing for the new exact candidate-version request.

- 2026-09-05 - codex: after corrected candidate delivery, retained two unpaired follow-ups, inspected measurement/acquisition/aggregation source and primary protocol/research sources, and recorded the assessment with all limitations. Kept the comparative guarantee and prior loss-path qualification open.
