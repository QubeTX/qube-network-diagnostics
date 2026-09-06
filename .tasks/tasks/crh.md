TT;DR: Remove the automatic Claude Code review workflow that failed on PR #36, as requested by the operator.

## Why
The automatic `claude-review` job failed before producing a review on PR #36. The operator explicitly requested removal of that integration on 2026-09-05.

## Scope
Delete `.github/workflows/claude-code-review.yml`. Preserve the separate manually invoked `@claude` workflow, local agent hooks, technical CI, and release automation. GitHub branch protection and repository rulesets are the source of truth for any required-check dependency; neither currently requires this check.

## Impact
Opening or updating a PR no longer launches the removed automatic review job. Historical failed runs remain in GitHub history. No runtime or release artifact changes are needed.

## Acceptance
The automatic workflow is absent, no remaining configuration depends on `claude-review`, and all remaining workflows pass actionlint. Delivery follows the existing PR-to-main workflow.

## Verification
- [x] The diff deletes only the automatic review workflow, apart from this task record.
- [x] Remaining configuration has no `claude-review` or `claude-code-review` references and no required-check dependency.
- [x] All 9 remaining workflow files pass actionlint; `git diff --check` passes.

## Status
Implementation and local validation are complete. The cleanup commit removes only the automatic workflow and records this task; no runtime code, manually invoked integration, local hook, or remaining CI workflow changes.

## Activity
- 2026-09-05 - codex: created from the operator request, refreshed origin/main, identified the failing automatic workflow, and confirmed main has no branch protection or repository rulesets requiring this check.
- 2026-09-05 - codex: removed the workflow, confirmed no dependent configuration references remain, and passed actionlint on all 9 remaining workflows plus git diff --check. Completed the source cleanup for delivery through a PR to main.
