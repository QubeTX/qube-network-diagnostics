TT;DR: Qualify the gated macOS service-cycle implementation only on disposable or sacrificial hardware with console access and an independent watchdog. Production remains off meanwhile.

## Why
The removed implementation could delete a network service and incompletely recreate it, and the operator plausibly experienced lasting Wi-Fi trouble after testing. One successful active-Mac run cannot prove recovery from process death or command failure at every destructive transition.

## Plan
Use a disposable VM or sacrificial service with local console and offline/root watchdog. Run the checked-in harness after explicit interface/service acknowledgement. Inject failures and SIGINT/SIGTERM/SIGHUP through snapshot, restore registration, disable/re-enable, DNS/search restoration, route/reachability verification, and token resolution. Prove launchd is one-shot and the terminal marker prevents rerun.

## Impact
Passing would provide the only acceptable basis for a later reviewed production-gate change. Keeping it deferred sacrifices one last-resort repair action but prevents destructive reset risk.

## Acceptance
Every transition leaves state unchanged or exactly restores enabled state, DHCP/static mode, DNS/search domains, and route; restores drain LIFO; watchdog evidence survives controller loss; no test executes twice.

## Verification
- [ ] Disposable hardware and independent watchdog are available
- [ ] Every destructive transition has failure and signal coverage
- [ ] Exact pre-state restoration is verified after every case
- [ ] One-shot and no-rerun protections are proven

## Status
Deferred P1. `MACOS_SERVICE_CYCLE_VALIDATED` must remain false in v3.6.x.

## Activity
- 2026-07-17 10:18 — migrated ND36-009 to Backlog with production gate unchanged (agent: codex)
