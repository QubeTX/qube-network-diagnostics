TT;DR: Leave the ND-300 and homepage repositories on clean, updated `main` branches with only active remote work and no stale worktrees.

## Why
The operator explicitly requested cleanup after deployment. Both repositories have old merged branches, and the homepage also has a superseded open branch. Cleanup must not discard unique or user-owned work.

## Plan
Inventory local branches, remote branches, PR associations, reachability, unique commits, working-tree state, and worktrees in both repositories. Close superseded PR #7; delete only merged/closed/superseded refs with no unpreserved unique work; remove only clean task-created or stale worktrees; prune remotes/worktree metadata; switch both repositories to updated main; enable `delete_branch_on_merge` for both repositories. Preserve the ND repo's `.agents/` and `.codex/` and the dirty external tasks-plugin clone.

## Impact
Navigation and future releases become substantially less error-prone. The main risk is destructive loss, controlled by the reachability/PR/worktree/dirty-state audit and by refusing deletion when evidence is ambiguous.

## Acceptance
Both repositories are on current main, clean except explicitly preserved user files, have no stale worktrees, remote/local inventories contain only main and active unmerged work, and automatic merged-branch deletion is enabled.

## Verification
- [x] Every deleted branch has merged/closed/superseded PR and unique-work evidence
- [x] No dirty worktree or unpushed unique commit is removed
- [x] ND-300 repository ends on updated main with preserved `.agents/` and `.codex/`
- [x] Homepage repository ends on updated main
- [x] Both repositories have automatic merged-branch deletion enabled
- [x] `git remote prune` and `git worktree prune` leave accurate inventories

## Status
Done. The safety audit tied every PR branch to a merged/closed PR, proved the two unmapped `emmett/wb-*` branches were ancestors of main with zero unique commits, and proved homepage PR #7 changed only the four fallbacks superseded by #8. ND deleted 21 remote and 16 local stale refs; the homepage deleted two remote and two local refs. Both repositories have one primary worktree, pruned metadata, automatic merged-branch deletion enabled, and only `main` outside this final auto-deleted record branch. ND's `.agents`, `.codex`, and generated installer output remain untouched.

## Activity
- 2026-07-17 10:18 — created from the operator's explicit post-release cleanup order (agent: codex)
- 2026-07-18 02:38 — audited PR state/reachability/unique work, deleted 21 ND remote plus 16 local refs and two homepage remote/local refs, pruned refs/worktree metadata, enabled automatic merged-branch deletion in both repositories, and verified one exact-main worktree per repository with preserved user files (agent: codex)
