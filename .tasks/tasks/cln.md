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
- [ ] Every deleted branch has merged/closed/superseded PR and unique-work evidence
- [ ] No dirty worktree or unpushed unique commit is removed
- [ ] ND-300 repository ends on updated main with preserved `.agents/` and `.codex/`
- [ ] Homepage repository ends on updated main
- [ ] Both repositories have automatic merged-branch deletion enabled
- [ ] `git remote prune` and `git worktree prune` leave accurate inventories

## Status
Blocked on #doc. Initial inventory found one worktree per repository and only merged/superseded stale branches outside the active release PRs.

## Activity
- 2026-07-17 10:18 — created from the operator's explicit post-release cleanup order (agent: codex)
