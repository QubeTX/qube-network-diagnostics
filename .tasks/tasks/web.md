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
- [x] Homepage fallback version equals the final public ND-300 version in four components
- [x] Homepage lint/build validation and PR checks pass
- [x] PR #7 is closed with a superseded-by-#8 note
- [x] PR #8 is merged only after public release validation
- [x] Vercel production deployment succeeds

## Status
Done. PR #8 head `dac12a6` set all four fallbacks to public v3.6.4, passed lint/build and Vercel preview, and merged as `fab1628`. Vercel production deployment `5498518176` succeeded; a no-cache production fetch returned HTTP 200 with the 3.6.4 bundle and live GitHub lookup. PR #7 is closed as superseded.

## Activity
- 2026-07-17 10:18 — imported ND36-008 and cleanup/deploy requirements (agent: codex)
- 2026-07-17 22:36 — refreshed PR #8 from the superseded 3.6.2 fallback to final public 3.6.4; lint, production build, and Vercel preview passed at `dac12a6` (agent: codex)
- 2026-07-17 22:37 — merged PR #8 as `fab1628`, verified successful Vercel production deployment `5498518176` and live HTTP/bundle/API results, then closed PR #7 as superseded (agent: codex)
