# Tasks

## Backlog
- [ ] **Share one immutable topology snapshot across diagnostics** - preserve serialized shapes while eliminating cross-module timing disagreement (ms #p36) #top
- [ ] **Expand hosted Unix updater fault injection** - add crash, filesystem, and rollback transition coverage (ms #p36) #ufi
- [ ] **Qualify the macOS service cycle on disposable hardware** - keep the production gate off until the destructive matrix passes (ms #p36) #msc
- [ ] **Repeat read-only real-Mac diagnostic smokes** - validate final parsers and both public architectures on safe hardware (ms #p36) #mrd
- [ ] **Design safe post-finalize cross-edition Windows unregistration** - remove retained installer registrations without nesting MSI transactions (ms #p36) #arp

## To-Do
- [ ] **Verify public Windows updates and upgrade the Alienware** - exercise all five origins against public artifacts (needs #rel) (ms #v36) #wup
- [ ] **Merge and verify the ND-300 homepage update** - deploy the final public fallback version through Vercel (needs #wup) (ms #v36) #web
- [ ] **Close release documentation with exact evidence** - record SHAs, runs, publishing, trust, and smoke results (needs #web) (ms #v36) #doc
- [ ] **Clean merged branches and worktrees in both repositories** - preserve unique work, prune stale refs, and enable automatic merged-branch deletion (needs #doc) (ms #v36) #cln

## Active
- [ ] **Tag and verify the final ND-300 release** - fix forward from immutable v3.6.0/v3.6.1 verifier failures and verify v3.6.2 trust, assets, and attestations (ms #v36) (owner codex) #rel

## Done
- [x] **Repair PR #18 release and cross-platform blockers** - exact head `4b2c623` passed every PR gate and both specialist reviews (ms #v36) #prf
- [x] **Harden Windows install ownership and uninstall lifecycle** - candidate run `29601781683` passed all five disposable origins (ms #v36) #win
- [x] **Run pre-merge validation and Alienware smoke** - local, installer, candidate VM, and non-mutating physical checks passed (ms #v36) #val
- [x] **Merge exact-SHA candidate and publish the crate** - merge `2dd0f92`, exact-main CI, and public crates.io 3.6.0 all passed (ms #v36) #mrg
