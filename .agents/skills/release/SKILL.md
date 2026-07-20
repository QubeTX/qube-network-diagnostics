---
name: release
description: This skill should be used when the user asks to "release", "ship it", "tag the release", "push it out", "make this live", "cut a release", "do a release", or "go for it" after a change has been discussed. Codifies ND-300's canonical tag-push deploy (version bump → changelogs in lockstep → docs → homepage version bump → local verify → PR → merge → tag → watch CI → verify assets).
disable-model-invocation: true
allowed-tools: Read, Edit, Bash, Glob, Grep
---

# release (ND-300 tag-push deploy)

Drive an ND-300 release end to end. This is a checklist + the ND-300-specific
glue; **`AGENTS.md` is the source of truth** for the deploy mechanics, the
patch-bump loop, the CI-watch pattern, and the don't-do safety list. Read its
"Release Process" section before starting and defer to it on any conflict.

## Deploy model (read this first)

ND-300 deploys via **tag-push**, and there are TWO automation layers:

1. **Crate publish** happens on **merge to `main`** (`crates-publish.yml`) — the
   crates.io publish is wired to the main-branch merge, not the tag.
2. **GitHub Release + cargo-dist assets** are produced by `release.yml`. That
   same tag/SHA-bound workflow calls `windows-installers.yml` for the remaining
   Windows installers and sidecars, then `macos-installer.yml` for the direct
   universal PKG, compatibility DMG, and their sidecars.

The standard sequence is: merge the version bump to `main` (crate publishes),
then push the `vX.Y.Z` tag (release + installers build). The final GitHub Release
carries 22 cargo-dist base assets, 6 add-on Windows assets, and 4 macOS package
assets: **32 assets total**.
Confirm the exact trigger wiring against `.github/workflows/*.yml` at release
time — the workflows are authoritative; `AGENTS.md` describes intent.

## Steps

1. **Bump version** in `Cargo.toml` `[package] version`. Patch = fixes, minor =
   backward-compatible features, major = breaking. Must be a never-published
   version. Update `Cargo.lock` (a build or `cargo update -p nd300` refreshes it).

2. **Update `CHANGELOG.md`** — new top entry, Keep-a-Changelog format, dated with
   the host's **current** date (not the training-data date).

3. **Update `HUMAN_CHANGELOG.md` IN THE SAME COMMIT** (lockstep — non-negotiable;
   see the "Changelog rule" in `AGENTS.md` and the `human-changelog` skill). Strip
   version numbers, file paths, function names, metrics, and jargon; keep what
   changed and why it matters, under plain category labels. The repo's
   PostToolUse hook will remind you if you edit `CHANGELOG.md` without touching
   `HUMAN_CHANGELOG.md`.

4. **Update `README.md`** if the release adds/changes user-visible flags,
   commands, or behavior; update **`AGENTS.md`** if the architecture or workflow
   changed.

5. **Bump the homepage version fallbacks** in the
   `qube-machine-report-homepage` sibling repo. Update the fallback version
   in every `useGitHubVersion('QubeTX/qube-network-diagnostics', '<old>')` call:
   - `src/components/ND300Install.jsx`
   - `src/components/ND300Hero.jsx`
   - `src/components/ND300Footer.jsx`
   - `src/components/InstallGuideContent.jsx`

   Find them with:
   ```bash
   rg -n "useGitHubVersion\('QubeTX/qube-network-diagnostics'" \
     ../qube-machine-report-homepage/src
   ```
   These are the version shown before the live GitHub API responds; ship the
   homepage change through that repo's own workflow.

6. **Run local verification** (in the ND-300 repo, before committing):
   ```bash
   cargo fmt --all -- --check
   cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
   cargo test --locked --workspace --all-targets
   cargo build --release --locked
   cargo audit
   cargo package --locked --list
   cargo publish --dry-run --locked
   ```
   If installer or release behavior changed, also run `cargo build --release`
   and the **`validate-installers`** skill (builds all four Windows installers
   locally — catches CNDL0104 / iscc bugs before the post-tag installer build).
   For LOCKSTEP changes (installer paths / markers), run the
   `installer-lockstep-reviewer` subagent. For any platform-gated Rust change,
   run the `cross-platform-cfg-reviewer` subagent (CI is the source of truth for
   macOS/Linux; a green Windows build does not prove a green CI build).

7. **Stage specific files only** (`git add Cargo.toml Cargo.lock CHANGELOG.md
   HUMAN_CHANGELOG.md README.md src/... wix/... inno/...`). **Never `git add -A`
   / `git add .`** — risks pulling in `nul`, `.env`, build artifacts, temp files.

8. **Commit + PR + merge to `main`** following the `git-workflow` skill (worktree
   off the daily workbranch; full pre-PR gates; PR-gated merge — no direct push
   to `main`). Use a `feat:` / `fix:` / `docs:` message with a
   `Co-Authored-By:` trailer. Merging the version bump to `main` triggers the
   crate publish via `crates-publish.yml`.

9. **Push the `vX.Y.Z` tag** to trigger `release.yml` and its reusable Windows
   and macOS installer jobs. Follow `AGENTS.md`'s exact tag procedure — do NOT
   re-tag an existing version; always go forward to a new patch.

10. **Watch CI without blocking the control session.** Use the poll-loop pattern
    in `AGENTS.md` (one notification per job
    state-change + a terminal `RUN_DONE` line; `gh --jq`, never pipe to `jq`;
    verify monitor liveness within 60s).

11. **Verify publish outputs** after CI is green:
    ```bash
    git ls-remote --tags origin | grep vX.Y.Z
    gh release view vX.Y.Z
    cargo owner --list nd300
    cargo install nd300 --force
    ```
    Confirm the GitHub Release carries exactly 32 assets: 22 cargo-dist base
    assets, 6 Windows add-ons, and the direct PKG/compatibility DMG with both
    sidecars. Verify Apple trust, byte identity, checksums, and exact-tag-SHA
    attestations as documented in `AGENTS.md`.

## If CI fails (patch-bump loop)

A green local build does NOT prove a green CI build — CI builds macOS aarch64/
x86_64, Linux gnu/musl/aarch64, and Windows MSVC. A `#[cfg(target_os=...)]`
block can compile on Windows and break only on the gated target. Follow the
"Patch-bump loop" in `AGENTS.md`: pull the failing job log
(`gh run view <id> --log-failed`), fix the root cause, **bump the PATCH version**
(never re-tag), add CHANGELOG + HUMAN_CHANGELOG entries, re-verify, and re-ship.

## Safety (from `AGENTS.md`'s don't-do list)

- Never `git push --force` / `--force-with-lease` to `main` during a release.
- Never re-tag an already-pushed version — always go forward.
- Never `git add -A`.
- Never amend an already-pushed commit.
- Never bypass `pre-commit` hooks or delete a published tag/asset without
  explicit authorization.
