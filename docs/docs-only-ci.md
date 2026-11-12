# Documentation-only CI

Ordinary documentation PRs and main-branch pushes run lightweight checks. They
do not compile Rust, run measurement replay, build or sign Mac packages, or
exercise installer lifecycles. `CI required` still runs and must pass.

The shared change classifier reads the complete Git change set, rather than a
truncated API file list or just the last commit. For PRs it checks both the full
branch diff from its merge base and the effective merge result. For main pushes
it checks the event's complete before/after range. It considers both old and new
paths for renames.

## Which changes use the fast path?

| Change | Validation |
|---|---|
| Content edits to the existing regular `README.md` | Documentation only |
| Regular Markdown files under `docs/` or `.tasks/` | Documentation only |
| `AGENTS.md`, `CLAUDE.md`, `CODEX_PROJECT.md`, `ND300-Project-Plan.md` | Documentation only |
| Root technical/human changelogs, `METHODOLOGY.md`, `LICENSE`, or `man/` | Full |
| Rust, dependencies, build scripts, workflows, configuration, installer sources, fixtures, or unknown paths | Full |
| Mixed documentation and code changes | Full |
| Executable-mode, symlink, submodule, or file-type changes | Full |
| Missing/shallow history, malformed event metadata, ambiguous or empty change sets | Full |
| Release tags, manual repair/preflight dispatches, or other explicit workflows | Full |

Markdown additions, edits and deletions under the allowlist are supported.
Removing or renaming the packaged root README requires full validation. The
allowlist does not include executable task-board code or JSON evidence files.
The root methodology document is an input to the measurement contract check;
its `.md` extension does not make it ordinary prose for CI purposes.

## What still runs?

Every CI run retains change classification, policy and boundary fixtures, and
ShellCheck plus the existing lightweight release-script fixtures. The docs path
also verifies changed Markdown is valid UTF-8 text without binary NUL bytes.
Intentional Markdown trailing spaces are allowed. The required-check aggregator
accepts only the expected complete result set: a failed, cancelled, missing or
unexpectedly skipped check cannot turn green.

The Windows origin matrix retains its existing installer-related PR path
coverage and explicit dispatch behavior. Its existing filter excludes ordinary
documentation; changes to the new classification and evidence logic trigger
the full matrix too. Applicable code/release coverage is not reduced.

## Publishing and release qualification

A separate `Full CI` job succeeds only after all full-validation checks pass.
For documentation-only runs, `Documentation checks` succeeds and `Full CI` is
skipped. Both paths still require successful classification and `CI required`.

The crate publisher verifies these job results from the exact triggering CI
run, repository, workflow, source SHA and latest attempt before setting up Rust
or accessing the registry credential. A positively verified docs-only result
produces a successful no-op. It does not try to publish an unpublished version
left by an earlier unrelated change.

Tag releases require positive full-CI evidence for their exact source, plus the
existing nine individually successful Apple qualification jobs for that same
source. A docs-only green run, an older source commit, or missing markers do not
qualify a release. Normal version changes touch Cargo metadata and therefore run
the full path. Explicit release and repair workflows retain their existing
source, signing, checksum, provenance, compatibility and inventory checks.

CI retries may reuse successful jobs from earlier attempts of the same immutable
run. A newer failed/cancelled result, a different run/source, or a retry appearing
while evidence is being read is refused; an older green run is not substituted.

## Verification

Local fixtures use real temporary Git repositories for multi-commit pushes,
stale-base PRs, merge-result changes, renames, file modes, shallow history and
more than 300 changed files. Other fixtures cover strict required-check results,
paginated exact-run evidence, failed/cancelled/missing markers, retries and the
existing Apple source/qualification policy.

### Hosted implementation

[PR #51](https://github.com/QubeTX/qube-network-diagnostics/pull/51) merged as
`dbc784b3b51702928603d8041f045a78bbd6a0fc`. Its final head,
`9566695971db61e09723761d88a53e341e430b72`, passed the full path:

| Gate | Successful run |
|---|---|
| Full cross-platform CI, including `CI required` and `Full CI` | [43151456264](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/43151456264) |
| Windows installer build and all 12 lifecycle/transition/refusal rows | [43151456322](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/43151456322) |
| Native Intel and Apple Silicon candidates | [43151456336](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/43151456336) |
| Complete recorded-trace parity | [43151456602](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/43151456602) |
| Release planning | [43151456423](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/43151456423) |

The `classify_jobs` marker interpreter read the real final PR job metadata and
returned `full`. Full release verification separately requires a successful
latest main-push run; PR metadata alone cannot authorize publishing.
Independent review and all 65 local policy/boundary fixtures passed. Hosted
validation also caught GitHub's transitive skipped-job behavior before merge;
the full marker now uses an explicit cancellation-aware condition with positive
classification and required-gate results, and the fresh run above proved it.

Exact-merge main CI [43153071950](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/43153071950),
signed Mac qualification [43153071864](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/43153071864),
and parity [43153071927](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/43153071927)
are being verified separately. The documentation-only PR/main receipts remain
open until observed; a green implementation PR alone does not prove the skip
path. This workflow change does not create a product version or release tag.
