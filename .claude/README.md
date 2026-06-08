# Repo-local Claude Code automations

This directory holds ND-300's repo-local Claude Code configuration: hooks,
user-invocable skills, and read-only review subagents. The team-shared MCP server
lives in `.mcp.json` at the repo root. Everything here is **additive** — it
encodes the deploy process, the installer LOCKSTEP contract, and the
installer-build pitfalls already documented in the repo's `CLAUDE.md` /
`AGENTS.md`, so the same mistakes that broke past releases get caught earlier.

## Hooks (`.claude/settings.json` + `.claude/hooks/`)

Hooks are command hooks that run a small Python script (Python 3 is on PATH on
the dev host). Paths are anchored on `$CLAUDE_PROJECT_DIR` so nothing hardcodes a
user directory.

### WiX comment guard — `wix_comment_guard.py`

- **Event:** `PreToolUse` on `Edit` / `Write` / `MultiEdit`.
- **Scope:** only files under `wix/**.wxs` and `wix-corporate/**.wxs`.
- **What it does:** blocks an edit that would put a double hyphen (`--`) **inside
  an XML comment body** (`<!-- ... -->`). WiX 3's `candle.exe` rejects that with
  error **CNDL0104**, and cargo-dist hides candle's stderr, so the failure only
  appears at release time on the Windows MSI build — exactly what broke the
  v3.2.0 MSI release. The `<!--` / `-->` delimiters themselves are fine, and `--`
  inside attribute *values* (e.g. `ExeCommand='x --flag'`) is allowed — only
  comment bodies are checked. Multi-line and multiple comments are handled.
- **On violation:** denies the tool call (exit 2) with a message naming CNDL0104
  and the offending comment. Fails open (allows) on any unexpected/malformed
  input so it can never wedge legitimate editing.

### Changelog lockstep reminder — `changelog_lockstep_reminder.py`

- **Event:** `PostToolUse` on `Edit` / `Write`.
- **Scope:** only `CHANGELOG.md`.
- **What it does:** if `CHANGELOG.md` was edited but `HUMAN_CHANGELOG.md` is
  unchanged in the working tree, emits a **non-blocking** reminder (the repo's
  Changelog rule requires both files move in the same commit). Stays silent if
  `HUMAN_CHANGELOG.md` is already dirty. Never blocks.

## Skills (`.claude/skills/`)

Both skills are **user-invocable only** (`disable-model-invocation: true`) — they
have side effects / drive a release, so the model won't auto-trigger them.

### `validate-installers`

Builds all **four** Windows installers locally (Global MSI, Corporate MSI, Global
EXE, Corporate EXE) and reports exit codes, reproducing
`.github/workflows/windows-installers.yml`. Run it before a tag-push release to
catch installer-build bugs (CNDL0104, dropped arg quotes, invalid Inno
constants) locally. Bundles `scripts/validate-installers.ps1`, which ensures WiX
3.11 and Inno Setup 6 are present (downloading/installing them if needed),
`cargo build --release`s, and builds all four installers with the exact CI
candle/light/iscc invocations.

```powershell
pwsh -File .claude/skills/validate-installers/scripts/validate-installers.ps1
```

### `release`

Codifies the canonical tag-push deploy: version bump → `CHANGELOG.md` +
`HUMAN_CHANGELOG.md` (lockstep) → docs → bump the homepage version fallbacks in
`qube-machine-report-homepage` → local verify (fmt/clippy/test/publish-dry-run +
`validate-installers`) → PR + merge to `main` (crate publishes) → push the
`vX.Y.Z` tag (release + installers) → watch CI via Monitor → verify the crate and
release assets. It summarizes the steps and points to `CLAUDE.md` / `AGENTS.md`
as the source of truth rather than duplicating them.

## Subagents (`.claude/agents/`)

Both are **read-only** reviewers (Glob/Grep/Read/Bash) — they report risks and do
not edit code.

### `cross-platform-cfg-reviewer`

Reviews `#[cfg(...)]`-gated Rust for the "only fails on one OS" class: visibility
behind public items (the `private_interfaces` error that hit
`MacosNetworkSnapshot`), E0603 private-fn-across-cfg, dead-code/unused warnings
that only appear on a platform under `-D warnings`, and non-exhaustive matches on
cfg-gated enums. It reasons per-target because macOS/Linux can't be compiled on
the Windows host, and flags risks before they reach release CI.

### `installer-lockstep-reviewer`

Verifies the LOCKSTEP contract: install paths + `HKCU\Software\ND300\InstallSource`
markers in the four installers staying consistent with
`src/actions/update.rs` (`read_install_source_marker` / `classify_install_path` /
`detect_install_origin`), both binaries packaged in every installer, and GUIDs
distinct/permanent — **including `src/actions/migrate.rs`** (the `nd300 migrate-cleanup`
cross-method cleanup), which has its own edition paths + the `OUR_BINARIES`
allowlist and reuses `update.rs` / `uninstall.rs`.

## MCP (`.mcp.json` at repo root)

### context7 (team-shared)

Live, version-accurate documentation for ND-300's key dependencies (clap, tokio,
reqwest, tokio-tungstenite, and the rest). Configured as a **stdio** server via
`npx -y @upstash/context7-mcp` (Node/npx are on the dev host). No API key is
required for basic use; the free tier works out of the box. To raise rate limits,
set a `CONTEXT7_API_KEY` environment variable — `.mcp.json` expands
`${CONTEXT7_API_KEY}` into the server's env, so the key stays out of the repo.
Get a key at https://context7.com. Run `/mcp` in Claude Code to confirm the
server connected.

### microsoft-docs (NOT in `.mcp.json` — recommended at the environment level)

The Microsoft Learn MCP server (`microsoft_docs_search` / `microsoft_docs_fetch`)
is very useful for this repo's Windows-specific work — WiX/Inno installer
semantics, `msiexec` switches, registry/PATH behavior, Win32 APIs (`GetIfEntry2`,
TCP/UDP tables), and WMI queries. It is an **environment-/global-level MCP**, not
a repo-addable one, so it deliberately is **not** placed in this repo's
`.mcp.json`. Add it at the user/global Claude Code level (it's already available
in this environment) rather than committing it here.
