# Tasks

## Backlog
- [ ] **Share one immutable topology snapshot across diagnostics** - preserve serialized shapes while eliminating cross-module timing disagreement (ms #p36) #top
- [ ] **Expand hosted Unix updater fault injection** - add crash, filesystem, and rollback transition coverage (ms #p36) #ufi
- [ ] **Qualify the macOS service cycle on disposable hardware** - keep the production gate off until the destructive matrix passes (ms #p36) #msc
- [ ] **Repeat read-only real-Mac diagnostic smokes** - validate final parsers and both public architectures on safe hardware (ms #p36) #mrd
- [ ] **Design safe post-finalize cross-edition Windows unregistration** - remove retained installer registrations without nesting MSI transactions (ms #p36) #arp

## To-Do
- [ ] **Run pre-merge validation and Alienware smoke** - validate Rust, scripts, four installers, disposable Windows upgrades, and physical diagnostics (needs #prf, #win) (ms #v36) #val
- [ ] **Merge exact-SHA candidate and publish the crate** - merge PR #18 only after every reviewed-head gate passes (needs #val) (ms #v36) #mrg
- [ ] **Tag and verify the final ND-300 release** - publish an immutable version and verify trust, assets, and attestations (needs #mrg) (ms #v36) #rel
- [ ] **Verify public Windows updates and upgrade the Alienware** - exercise all five origins against public artifacts (needs #rel) (ms #v36) #wup
- [ ] **Merge and verify the ND-300 homepage update** - deploy the final public fallback version through Vercel (needs #wup) (ms #v36) #web
- [ ] **Close release documentation with exact evidence** - record SHAs, runs, publishing, trust, and smoke results (needs #web) (ms #v36) #doc
- [ ] **Clean merged branches and worktrees in both repositories** - preserve unique work, prune stale refs, and enable automatic merged-branch deletion (needs #doc) (ms #v36) #cln

## Active
- [ ] **Repair PR #18 release and cross-platform blockers** - fix observed CI failures, provenance, and missing release-target gates (ms #v36) (owner codex) #prf
  - [x] Migrate the root Mac handoff into the tracked SHAUGHV board
    > Preserve every ND36 task's rationale, impact, acceptance, and safety constraints in task detail files.
  - [x] Fix Windows/Linux cfg and Clippy blockers
  - [x] Fix ShellCheck trap-handler diagnostics
  - [x] Bind Windows artifact attestations to the tag-push caller SHA
  - [x] Add Linux ARM and musl pre-tag compile gates
- [ ] **Harden Windows install ownership and uninstall lifecycle** - eliminate stale-marker routing while preserving advisory migration safety (ms #v36) (owner codex) #win
  - [x] Implement Cargo/registered-owner/marker precedence
  - [x] Delegate registered uninstall without shell-expanding registry commands
  - [x] Pass a validated hidden origin from all four installers
  - [x] Keep installer-internal migration advisory and file-only
  - [ ] Prove all five origins on disposable hosted Windows VMs

## Done
