TT;DR: Bound SpeedQX provider metadata and constrain discovered endpoints. Open follow-up after the September 5 authorized release.

## Why
Independent review found Medium unbounded discovery/control JSON and Low destination validation gaps requiring a faulty or compromised trusted provider.

## Scope
Add bounded discovery readers and control-message limits across canonical TypeScript and Rust; constrain approved M-Lab hosts, paths, ports and redirects after provider compatibility verification. Track inherited decoder and UUID advisories without unrelated framework downgrades.

## Impact
Keeps the released product's limitations explicit and improves qualification without rewriting historical evidence.

## Verification
- [ ] Oversized metadata and malformed endpoint regressions, byte-identical generated app source, cross-platform checks, and native malformed-link verification for any decoder fix.

## Status
Backlog. Not completed or counted as a successful release test.

## Activity
- 2026-09-05 — codex: split from #sq5 release handoff; accuracy/device publication limits were disclosed before owner release authorization. Security findings were recorded by the independent review as nonblocking follow-ups.
