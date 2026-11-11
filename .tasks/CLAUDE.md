<!-- tasks-bootstrap: done -->
> Secrets: never stored here or in memory/. See .tasks/secure/ (gitignored), or env/keychain.

# Memory

## Me
Emmett owns the QubeTX diagnostic tools and uses Codex for evidence-driven implementation, testing, release work, and durable documentation.

## People
| Who | Role |
|-----|------|
| **Emmett** | Repository owner and release operator |

## Terms
| Term | Meaning |
|------|---------|
| **ND-300 / nd300** | Cross-platform Rust network diagnostics CLI; also ships `speedqx` |
| **P0** | Required before the current release is considered complete |
| **P1** | Recommended follow-up that does not block the current release |
| **Global** | Per-machine Windows edition under Program Files with system PATH |
| **Corporate** | Per-user Windows edition under LocalAppData with user PATH |
| **Exact-SHA** | Evidence and artifacts must bind to the precise reviewed or tagged commit |
| **LOCKSTEP** | Four installer paths, markers, identifiers, packaged binaries, and updater logic must agree |

## Projects
| Name | What |
|------|------|
| **ND-300 v3.6.0** | Mac safety, Unix updater, release-trust, Windows lifecycle, and cross-platform release |
| **Qube homepage** | Public install/download site; ND-300 fallback version ships in a separate PR and Vercel deploy |

## Preferences
- Investigate and reconcile authoritative repository sources before changing release behavior.
- Require concrete test, CI, artifact, and deployment evidence before claiming success.
- Never reuse release tags, force-push release history, or stage unrelated user-owned work.
- Prefer reliability and explicit prerequisite handling over brevity for user-facing install flows.
- Keep the macOS destructive service-cycle production gate off until disposable-hardware qualification.
