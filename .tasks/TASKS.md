# Tasks

## Backlog
- [ ] **Share one immutable topology snapshot across diagnostics** - preserve serialized shapes while eliminating cross-module timing disagreement (ms #p36) #top
- [ ] **Expand hosted Unix updater fault injection** - add crash, filesystem, and rollback transition coverage (ms #p36) #ufi
- [ ] **Qualify the macOS service cycle on disposable hardware** - keep the production gate off until the destructive matrix passes (ms #p36) #msc
- [ ] **Repeat read-only real-Mac diagnostic smokes** - validate final parsers and both public architectures on safe hardware (ms #p36) #mrd
- [ ] **Design safe post-finalize cross-edition Windows unregistration** - remove retained installer registrations without nesting MSI transactions (ms #p36) #arp

## To-Do
- [ ] **Begin the native macOS PKG channel** - after the updater fix-forward is public, add the signed/notarized package path while the testing Mac is available (needs #wup) (ms #p36) #pkg
- [ ] **Clean merged branches and worktrees in both repositories** - preserve unique work, prune stale refs, and enable automatic merged-branch deletion (needs #doc) (ms #v36) #cln

## Active
- [ ] **Close release documentation with exact evidence** - record SHAs, runs, publishing, trust, and smoke results (needs #web) (ms #v36) (owner codex) #doc

## Done
- [x] **Repair public updates and upgrade the Alienware** - v3.6.4 passed public matrices and upgraded this Global MSI host from 3.5.2 with exact public bytes (ms #v36) #wup
- [x] **Merge and verify the ND-300 homepage update** - PR #8 deployed the 3.6.4 fallback through Vercel and obsolete PR #7 closed (ms #v36) #web
- [x] **Publish and verify ND-300 v3.6.2** - exact tag `f3d83fc`, run `29609073182`, 28 assets, Apple trust, checksums, and attestations all passed (ms #v36) #rel
- [x] **Repair PR #18 release and cross-platform blockers** - exact head `4b2c623` passed every PR gate and both specialist reviews (ms #v36) #prf
- [x] **Harden Windows install ownership and uninstall lifecycle** - candidate run `29601781683` passed all five disposable origins (ms #v36) #win
- [x] **Run pre-merge validation and Alienware smoke** - local, installer, candidate VM, and non-mutating physical checks passed (ms #v36) #val
- [x] **Merge exact-SHA candidate and publish the crate** - merge `2dd0f92`, exact-main CI, and public crates.io 3.6.0 all passed (ms #v36) #mrg
