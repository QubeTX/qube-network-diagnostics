TT;DR: Merge the prepared homepage fallback update only after the final ND-300 version is public, then verify the production Vercel deployment.

## Why
Four homepage components carry fallback versions used when GitHub/API lookup is unavailable. Merging too early advertises unpublished artifacts; leaving it stale after release advertises the old version.

## Plan
Keep homepage PR #8 open until the final release version is known. If a patch bump occurs, update its four fallbacks, package version, and both changelogs before merge. Close PR #7 as superseded. Merge PR #8 through the homepage workflow and verify Vercel production plus all four fallback locations.

## Impact
Public install guidance stays aligned with the deployed binary. SD-300 and Shaughv OS pages remain delisted/WIP and are not changed.

## Acceptance
The final public version appears in all four fallbacks, homepage CI succeeds, PR #8 is merged, Vercel production succeeds, and PR #7 is closed as superseded.

## Verification
- [ ] Homepage fallback version equals the final public ND-300 version in four components
- [ ] Homepage lint/build validation and PR checks pass
- [ ] PR #7 is closed with a superseded-by-#8 note
- [ ] PR #8 is merged only after public release validation
- [ ] Vercel production deployment succeeds

## Status
Blocked on #wup. PR #8 is prepared for 3.6.0 and currently green.

## Activity
- 2026-07-17 10:18 — imported ND36-008 and cleanup/deploy requirements (agent: codex)
