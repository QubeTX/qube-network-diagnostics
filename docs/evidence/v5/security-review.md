# SpeedQX v5 independent pre-release security review

Date: 2026-09-05. Reviewer: independent Codex GPT-6 Astra agent. Review mode: read-only source/diff review, local mocked-input probes, and current dependency advisory checks. No public measurement transfers, production mutations, or physical-device acceptance were performed by this reviewer.

## Exact reviewed revisions

| Repository | Candidate head | Comparison base (merge-base with origin/main) |
|---|---|---|
| QubeTX/qube-network-diagnostics | `807de96f45816912f7e3db2e876428970e784bcc` | `0d77c016c2a2080fb0860d8101ed35e5292adc12` |
| QubeTX/speedtest | `6758168dd2fd73f7de8749fc0846a9278f39cc6d` | `285b803450a63a43e463603933d56c6499e1a456` |
| RealEmmettS/speedtest-app-expo | `0fdbc1b8665aa539a428ebd44233b6b0bc59f954` | `8f510eb77bfec611ea05bcf5343c6efa55ea7ef3` |
| QubeTX/qube-machine-report-homepage | `671f50cf2d02ae1004037ed2ed1bddaebf9995d7` | `d415459a06750e546bac60e21f8361e6b731a7b6` |

The shared TypeScript implementation is pinned to `08d1c337b977d07e2296fe3a2e57f2983afa298f`. Both app `sync-v5-engine.mjs --check --source-check` and Rust `sync-v5-contract.mjs --check --source-check` passed. Identical generated app code was reviewed through its canonical source. Later release-documentation-only commits do not change the reviewed runtime; any subsequent runtime or workflow changes require delta review.

## Findings and follow-ups

### Medium — provider control messages do not have an application byte ceiling

Category: uncontrolled resource consumption. Locations: website `src/services/acquisition-v5.ts:140-142`, `181-189`, `222-225`; identical app `services/dom/v5/services/acquisition-v5.ts`; CLI `src/speedtest/acquisition_v5.rs:150-154`, `207-210`.

Discovery responses use `response.json()` without a maximum response size, and browser/WebView WebSocket text messages reach `JSON.parse()` without a length check. A faulty or compromised trusted provider can send a large metadata response or control message, consuming memory and synchronous parsing time outside the synthetic-payload budget. The wall-clock timeout does not bound already buffered JSON or synchronous parsing. A local mocked WebSocket confirmed that a 2,097,191-character text message was parsed while `budget.used` remained zero. No real provider was contacted and an OOM was not attempted. Rust WebSocket messages already have a 16 MiB protocol cap; its discovery JSON remains unbounded.

Recommended fix: consume discovery bodies with a small streaming byte limit before JSON parsing, reject oversized metadata/control frames before parsing, cap aggregate control-message bytes/count, and validate the small expected schema. Retain failed/partial result semantics. Browser WebSocket APIs allocate a completed message before invoking JavaScript, so a JavaScript length check reduces additional parsing pressure but cannot guarantee a strict transport-level memory ceiling.

### Low — M-Lab destination validation delegates host trust entirely to discovery

Category: endpoint validation / defense in depth. Locations: website `src/services/acquisition-v5.ts:147-154`; identical generated app copy; CLI `src/speedtest/acquisition_v5.rs:160-181`.

The discovery adapters require `wss:` but do not restrict destination hosts, ports, credentials, or protocol paths to the intended M-Lab service. A mocked successful Locate response containing `wss://127.0.0.1:9443/ndt/v7/download` was accepted without a network request. Exploitation requires control of the trusted HTTPS discovery response or another trusted interception point, and successful WSS/TLS handling at the destination; this is not an unauthenticated public SSRF endpoint. With those prerequisites, a consenting run could attempt connections outside the intended provider network. Browsers may impose additional private-network restrictions; native and CLI behavior must not depend on those protections.

Recommended fix: validate approved M-Lab host suffixes at label boundaries, expected paths and ports, reject URL credentials and IP-literal destinations, and explicitly constrain HTTP redirects to secure approved destinations. Keep discovery tokens out of presentation labels and logs. Qualify legitimate provider naming/redirect requirements before enforcing an allowlist.

### Medium advisory / no demonstrated product exploit — inherited app URI decoder

Location: app `package-lock.json:5063` (`decode-uri-component@0.2.2`, via `query-string@7.1.3` and React Navigation). This same version exists at the comparison base. [GHSA-vcc3-ghjq-m6fr](https://github.com/advisories/GHSA-vcc3-ghjq-m6fr) reports excessive CPU use for malformed encoded input and names 0.5.0 as patched. The installed Expo Router fork parses query parameters with `URLSearchParams`, while the general React Navigation parser still references `query-string`. No end-to-end exploit through the app's current route handling was established, so this review does not label the candidate's deep-link path demonstrably vulnerable.

Recommended fix: adopt a compatible upstream decoder/navigation fix or narrowly tested input-size protection; verify malformed-link behavior in native builds. Do not apply npm's suggested unrelated Expo/router major downgrades merely to clear an audit counter.

### Informational — inherited UUID advisory has no affected call site identified

Location: app `package-lock.json:12105` (`uuid@7.0.3`, unchanged from the base). [GHSA-w5hq-g745-h8pq](https://github.com/advisories/GHSA-w5hq-g745-h8pq) concerns caller-provided output buffers in v3/v5/v6 APIs. The inspected Xcode build-tool caller uses `uuid.v4()` without an output buffer (`node_modules/xcode/lib/pbxProject.js:90`); no affected product call site was identified. Track a compatible upstream build-tool update.

## Verified protections and scope coverage

- No Critical/High finding was identified in the changed production source or workflows. New acquisition uses fixed HTTPS discovery, secure WSS, synthetic random payloads, conservative upload budgeting, finite transfer windows, bounded normal payload buffers/concurrency, owned cancellation, and no shell dispatch from server data.
- M-Lab discovery and transfers remain behind explicit consent in the common orchestration; NDT7 and MSAK do not bypass the gate. The Expo bridge loads bundled code, passes declarative settings and result/progress callbacks, and rejects stale run messages. No server text is evaluated as code or injected as HTML. Cassette `dangerouslySetInnerHTML` receives a source-controlled static SVG constant.
- No added SQL/database surface, user-controlled filesystem mutation, command injection, unsafe object deserialization, or credential-bearing production CORS endpoint was identified. The permissive CORS reference server is a test fixture; production builds do not import it. Namespace mutation is restricted to named disposable GitHub-runner fixtures.
- Added GitHub workflows use `pull_request`/restricted triggers and `contents: read`; cross-repository source pins are exact SHAs, and secondary checkouts disable persisted credentials. No new `pull_request_target` execution or release-secret exposure was introduced. EAS UI test workflows use internal/e2e profiles; the production profile does not enable `EXPO_PUBLIC_UI_LAB`. The UI Lab route also gates rendering at runtime. Existing broad iOS ATS configuration was not changed by this candidate.
- A targeted secret-pattern scan covered all added text in the four diffs plus all eight added compressed JSON evidence files: no private-key, GitHub-token, AWS-access-key, service-key, or JWT-shaped matches. This was a bounded regex check, not exhaustive history scanning or an entropy-based secret audit.
- Fresh website `npm audit` reported zero advisories. Fresh app `npm audit` reported 19 moderate dependency entries rooted in the two advisory packages above, zero High/Critical entries. `cargo audit --json` reported zero unignored vulnerabilities using advisory database commit `5a0ebedfe8bdd2e295b171f4162f8c977bcad9a5`; its two existing documented ignores are unchanged. Homepage changes are static content/version fallbacks with no dependency-version additions.

## Limits and verdict

This is an independent focused diff review, not a comprehensive penetration test, a security guarantee, a third-party provider audit, or physical performance acceptance. Provider compromise, hostile-server fuzzing, a strict mobile memory bound, and deployed infrastructure controls were not proven safe. The Medium/Low findings above remain open follow-ups; this verdict does not waive or mark them fixed. The separately failing hosted Claude job produced no review and was not counted as evidence. Accuracy/repeatability and physical-device qualifications retain their separately documented status.

**PASS WITH NOTES.** No Critical or High finding was identified. Attach this report to the PRs/release evidence and retain the listed hardening work.
