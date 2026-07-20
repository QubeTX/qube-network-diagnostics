TT;DR: Make the useful repo-local agent configuration shareable without committing machine-specific paths or generated installer binaries.

## Why
The repository already tracks its Claude-oriented automation, but the equivalent `.agents` skills and `.codex` reviewers/hooks were left untracked. That makes the working tree look dirty and prevents other Codex-compatible environments from receiving the same release and installer safeguards. The Inno build directory is generated output and should not appear as source.

## Plan
Commit portable `.agents` and `.codex` equivalents, keep their instructions aligned with the current reusable Windows/macOS release workflow, document the cross-agent source-of-truth relationship, ignore and remove `inno/Output`, and validate the hook/config formats plus the installer build driver. Merge through the normal PR gate and return local `main` to a clean state.

## Impact
Future Claude and Codex sessions receive the same project-specific release, cfg, installer, and changelog guardrails. Machine paths and secrets remain excluded, generated installers stop dirtying Git, and product/runtime behavior is unchanged.

## Acceptance
All committed agent files are portable and secret-free, Claude/Codex release guidance reflects the 32-asset direct-PKG architecture, generated Inno output is ignored and absent, validation passes, the PR merges green, and local `main` is clean.

## Verification
- [x] `.agents` and `.codex` contain no user-specific absolute paths or secret values
- [x] Claude and Codex release/installer guidance matches current workflows
- [x] Hook JSON/TOML/Python and installer validation script pass local checks
- [x] `inno/Output` is ignored and generated binaries are removed
- [ ] Exact-head CI passes and the PR merges
- [ ] Local `main` matches `origin/main` with no staged, modified, or untracked files

## Status
Active. Portable cross-agent tooling and generated-output cleanup are locally validated; the exact-head PR gate and final clean-main audit remain.

## Activity
- 2026-07-19 — confirmed there are no staged or tracked modifications; classified `.agents`/`.codex` as useful local projections and `inno/Output` as generated build output (agent: codex)
- 2026-07-19 — made both agent trees portable, synchronized reviewer/hook bodies, corrected the truncated cfg-reviewer description, refreshed the 32-asset reusable-workflow guidance, removed four generated EXEs, and passed format, parity, behavior, path, secret, and ignore-policy checks (agent: codex)
