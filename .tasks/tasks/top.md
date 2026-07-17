TT;DR: Introduce one immutable topology snapshot shared by interface, VPN, route, DHCP, and Wi-Fi diagnostics so a mid-run network change cannot make sections disagree.

## Why
Current parsers are individually coherent and fixture-tested but collect related state independently. A VPN or route transition between collections can briefly produce inconsistent sections even when every parser is correct.

## Plan
Create a timestamped `TopologySnapshot` owned by the diagnostic run and thread it through related consumers without changing serialized output or action IDs. Add Windows GUID/friendly-name, VPN carrier, renamed Mac service, Linux bridge, dormant tunnel, and mid-run transition fixtures.

## Impact
Improves cross-section consistency on all platforms. Possible costs are extra snapshot latency or making optional metadata block core diagnostics; keep core bounded and optional Wi-Fi enrichment non-blocking.

## Acceptance
All related sections derive from one snapshot, no consumer repeats topology commands, fixtures prove consistent VPN/carrier behavior, and core timing/output compatibility remain intact.

## Verification
- [ ] Snapshot fixtures cover Windows, macOS, and Linux topology variants
- [ ] Related consumers issue no second topology collection command
- [ ] Serialized JSON/table behavior remains compatible
- [ ] Core diagnostics remain within the wall-clock cap

## Status
Deferred P1 follow-up; no known release-blocking defect.

## Activity
- 2026-07-17 10:18 — migrated ND36-006 to Backlog (agent: codex)
