# Security PR relevance review — November 10, 2026

The operator requested independent investigation of PRs #42–46, conflict repair,
and deployment only where changes remained relevant and needed. All five were
compared with current main `d9f0d35be5767152d8d3c670a991537d0bc1c7cf`, rather than
treating their original descriptions or test counts as current evidence.

## Dispositions

| PR | Current finding | Disposition |
|---|---|---|
| [42](https://github.com/QubeTX/qube-network-diagnostics/pull/42) | The Linux interactive dhcpcd repair still interpolates SSID and passphrase into privileged configuration. | Relevant. Replace the original escaping patch with native hexadecimal SSIDs and derived WPA-PSKs, validation, and checked writes/reloads. |
| [43](https://github.com/QubeTX/qube-network-diagnostics/pull/43) | Repair and embedded diagnostic subprocesses still resolve bare utilities from inherited lookup locations. | Relevant under an attacker-influenced elevated invocation. Replace its fail-open resolver with OS-derived/fixed paths and constrain interpreter startup/module lookup. |
| [44](https://github.com/QubeTX/qube-network-diagnostics/pull/44) | Read-only workflow defaults, scoped write permissions and protected Apple signing are already implemented. Some cargo-dist downloads/install scripts remain unverified. | Partly superseded. Preserve current permission/signing gates; verify pinned prebuilt tool bytes before use. Do not adopt its action wrapper as proof of a verified Rust bootstrap. |
| [45](https://github.com/QubeTX/qube-network-diagnostics/pull/45) | Optional Context7 invocation and downloaded local installer tools remain mutable or unauthenticated. | Relevant supply-chain hardening. Pin both agent configurations and authenticate WiX/Inno downloads and cache reconstruction. |
| [46](https://github.com/QubeTX/qube-network-diagnostics/pull/46) | The claimed unsigned-script execution bypass is not substantiated by the current verified, signed PKG route. | Reject the proposed certificate/script pins. The original certificate is the wrong certificate type, and script byte equality would reject legitimate later releases. |

These classifications do not claim that an attack occurred or that every issue
has a CVE. A known weakness pattern is not proof that every precondition is
present in a particular deployment.

## Wi-Fi configuration evidence

The relevant sink is `stage3_linux` in `src/actions/fix/stages.rs`. It is reached
only for an interactive, elevated, explicitly confirmed high-risk repair on
Linux without NetworkManager, with a detected Wi-Fi interface and writable
existing configuration. Saved SSIDs come from `iwgetid --raw`, and the fallback
also accepts prompted network text. The old formatter directly inserted that
text between quotes. A short SSID such as `x"\n  disabled=1\n#` can introduce a
separate network directive when interpreted as actual newline bytes.

The original patch treated ordinary quoted values as C-escaped strings. Upstream
`wpa_config_parse_string` distinguishes literal quoted values from the separate
`P"..."` escaped representation; therefore inserting backslashes into ordinary
quoted SSIDs changes the network's bytes. PSK parsing has its own passphrase
rules. The replacement uses the documented hex SSID and 64-hex PSK forms, with
WPA's required PBKDF2-HMAC-SHA1, 4096 iterations and 32-byte output. It preserves
SSID bytes and printable password punctuation without placing untrusted text in
configuration syntax. The derived PSK remains a credential and is never logged.

Tests cover the IEEE `password`/`IEEE` known answer (also independently computed
with Python `hashlib`), hostile/control/UTF-8 SSIDs, password punctuation and
invalid limits. A separate Linux CI test invokes the real `wpa_supplicant`
parser with an intentionally unsupported driver. It proves the former
`disabled=1` injection as a positive control and requires the new block to parse
the complete SSID without a new directive. The unsupported driver prevents any
network-device initialization. This is parser evidence, not a physical Wi-Fi
reconnection claim.

Primary references:

- [Upstream configuration examples and credential formats](https://android.googlesource.com/platform/external/wpa_supplicant_8/+/refs/heads/main/wpa_supplicant/wpa_supplicant.conf)
- [Upstream literal/escaped/hex string parser](https://android.googlesource.com/platform/external/wpa_supplicant_8/+/fb79edc9df1f20461e90e478363d207348213d35/src/utils/common.c)
- [Upstream initialization reads configuration before selecting a driver](https://android.googlesource.com/platform/external/wpa_supplicant_8/+/refs/heads/main/wpa_supplicant/wpa_supplicant.c)

## Executable and interpreter lookup evidence

The original #43 helper falls back to the same bare executable when a Unix tool
is absent, and derives the Windows directory from mutable `SystemRoot`. Both
leave a search-path influence. The replacement uses `GetSystemDirectoryW` on
Windows, fixed root-controlled Unix system directories, and absolute failed
candidates for missing tools. It covers diagnostic helpers because diagnostics
also run inside elevated repair sessions. Existing invoking-user Cargo dispatch
and independently proven installer paths retain their own ownership rules.

An isolated subprocess test plants a harmless program in PATH/current directory.
The old bare constructor is a positive control; the trusted constructors must
refuse that program across ordinary, capture, mutation and diagnostic runners.
The child also poisons `SystemRoot` and `WINDIR` while a genuine system shell must
still run. This does not require running a repair or disabling a network link.

Independent review reproduced two related inherited channels using harmless
markers: PowerShell `Get-Command Restart-Service` imported a fake management
module through `PSModulePath` even with `-NoProfile`, and Bash executed `BASH_ENV`
before its intended command. The final constructors must constrain these
startup/module paths as well as direct executable lookup. This is not a general
subprocess sandbox or a defense against administrator-owned system files being
replaced. Unix sudo commonly sanitizes PATH, so the local privilege-escalation
claim depends on an elevated invocation inheriting attacker-influenced state.

Recognized third-party VPN CLIs outside trusted locations must not silently
cause a new generic adapter-disable attempt. They require manual disconnection
unless their owning path is separately proven. Enterprise VPN exclusions,
confirmation and restoration rules remain mandatory.

Final compatibility review found that the optional PowerShell 7 fallback was
incompatible with fixed system lookup. The updater now deliberately uses only
the OS-resolved native Windows PowerShell launcher for standalone Windows
updates. Native launcher absence reports failure and same-channel manual
recovery; it does not reopen PATH discovery or change registered/Cargo channels.

- [MITRE CWE-426: Untrusted Search Path](https://cwe.mitre.org/data/definitions/426.html)
- [Microsoft PowerShell module lookup](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_psmodulepath?view=powershell-5.1)
- [GNU Bash startup files](https://www.gnu.org/software/bash/manual/html_node/Bash-Startup-Files.html)

## Build automation evidence

Current main already limits default tokens to read and confines Apple signing
to protected refs and the `apple-signing` environment. #44's old permissions
patch must not overwrite that newer architecture. Generated cargo-dist
`install_dist.run` commands still execute external installers; committed
publisher digests and direct archive installation remove that mutable script
step. The current six-target plan has no container. A future container must
provide the pinned Rust tooling rather than silently downloading an unverified
bootstrap. Same-run immutable Actions artifacts are not evidence of an arbitrary
cross-job cache exploit.

#45's WiX digest was independently matched to the official archive. Inno is
pinned to the current maintained 6.7.4 publisher digest rather than the old PR's
6.7.1. Cached extraction is reconstructed from a verified archive, rather than
trusting a marker beside mutable executable files. Tests must prove that wrong
download bytes never reach extraction/installation and altered extracted tools
are not reused. Context7's explicit package release narrows mutable top-level
selection; it does not claim a fully locked transitive npm dependency graph.

- [GitHub's least-privilege workflow guidance](https://docs.github.com/en/actions/reference/security/secure-use)
- [Official cargo-dist 0.31.0 release](https://github.com/axodotdev/cargo-dist/releases/tag/v0.31.0)

## Why #46 is not a valid fix

The current updater verifies the immutable-tag download and sidecar before
checking the Developer ID Installer signature and trusted timestamp, stapled
notarization, uncached Gatekeeper install policy, exact payload and package
metadata. `pkgbuild --scripts` places preinstall/postinstall inside the signed
flat package. Apple documents that later modification invalidates that package
signature; the scripts are not unsigned material outside this boundary.

The public v4.0.1 PKG was downloaded and its published sidecar verified without
executing it. Its SHA-256 is
`1dbb66b914244a48e56014be7166c3e3c51769152b4f56dd90a4621d22efa843`.
Its XAR certificate chain has Installer leaf SHA-1
`2120DB3CFC4C774CAF99A7BF8817575BF12B1FED`, subject
`Developer ID Installer: ES Development LLC (M9D5379H93)` and a G2 issuer.
The original PR instead compares the Installer leaf to Application certificate
`739B04530883FF9B665C66BD464F98C622971B32`; this rejects the real current package.
Its parser also assumes older SHA-1-formatted `pkgutil` output. Exact script-byte
pinning would reject legitimate future script changes, and lifetime leaf
pinning requires a deliberate certificate-renewal design.

No reachable script-signature bypass or attacker-obtainable identity-confusion
certificate was demonstrated. The speculative extra parser was therefore not
included. Locale-stable output and trusted utility invocation are covered by
the command-hardening work without changing the package trust contract.

- [Apple Code Signing Guide: signing flat installer packages](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html)
- [Apple Developer ID certificate replacement](https://developer.apple.com/help/account/certificates/replace-developer-id-certificates/)

## Published dependency advisories

The current audit independently found 11 advisories in the old lockfile.
Compatible patch updates to rustls 0.23.46, rustls-webpki 0.103.16,
webpki-roots 1.0.10 and optional quinn-proto 0.11.18 clear them without new
exceptions. QUIC is retained as an optional lockfile dependency; its presence
is not a claim that the active ND-300 HTTP configuration enables each affected
feature. Existing documented unmaintained/unsound advisory policy is unchanged.

- [Rustls advisory and patched version](https://rustsec.org/advisories/RUSTSEC-2026-0346.html)
- [WebPKI distrust enforcement advisory](https://rustsec.org/advisories/RUSTSEC-2026-0382.html)
- [Quinn conditional unordered-stream advisory](https://rustsec.org/advisories/RUSTSEC-2026-0338.html)

## Delivery evidence

Implementation/review evidence and open gates are tracked in
`.tasks/tasks/sec42.md`. Exact final commit, CI, release, artifact and deployment
receipts belong there and in the release evidence record. Descriptions above
are not substitutes for those final checks.
