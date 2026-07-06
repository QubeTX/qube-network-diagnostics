# Human Changelog

A plain-English companion to [CHANGELOG.md](./CHANGELOG.md). Every change in the technical changelog has a layman's-terms version here. No version numbers, no code references — just what changed and why.

For the technical version with versions, file paths, and PR links, see [CHANGELOG.md](./CHANGELOG.md).

---

## July 6, 2026 — one measurement method everywhere, two new speed sources, a fast mode, and honest answers when a service throttles us

The speed test now runs the exact same measurement method as the SpeedQX website
(and soon the iPhone app) — written down in one shared rulebook, with automated
checks proving all versions compute identical numbers down to the last digit. Run
it anywhere; the results mean the same thing.

**Added**
- Two more places to measure against — a global content-delivery network and a
  cloud provider with eight locations around the world (we automatically pick the
  closest one). That's eight independent sources in a full run.
- A fast mode: the three strongest sources with a smart "stop when we're sure"
  rule, so a quick check takes about a minute instead of several.
- Two honest headline numbers instead of one: what your line showed it can do,
  and the cautious average across every source — each with a range you can trust.
  A new agreement badge says how closely the sources matched, and a
  responsiveness score shows how usable your connection stays while it's busy.

**Improved**
- The ping measurement takes far more samples with a proper warm-up, so latency
  numbers are steadier and more trustworthy, especially on flaky connections.
- Download requests now size themselves to your connection instead of using one
  fixed size — gentler on slow links, better at filling fast ones.

**Fixed**
- One of the measurement services sometimes tells heavy users to slow down for a
  few minutes. The test used to sit there silently getting nothing (and could
  quietly understate your speed); now it notices immediately, says so plainly in
  the results, and the other sources carry the run.
- The quick mode no longer throws away a source's finished measurements when its
  time runs out mid-step on slow connections.

## June 10, 2026 — steadier results, more accurate speeds, two new speed-test providers, and a much deeper technician mode

This is a big one. The whole tool was given a stability and accuracy pass: the everyday checks no longer flip-flop between runs, the speed test numbers are more trustworthy (and now come with an honesty range), two well-known speed-test services joined the lineup, and technician mode grew from 18 deep checks to 25 — including several entirely new kinds of analysis.

**Improved — the everyday checks no longer flip-flop**
- Several of the basic checks used to rest on a single attempt: one ping to your router, one lookup of one website name, one connection attempt per port. One unlucky dropped packet could make a perfectly healthy network look broken. Now every one of those checks gathers several samples — and only reports a real problem when the failure is consistent. Your router now gets two rounds of pings before being called unreachable; the internet-name check tries three independent names and judges by the middle result, so one slow name can't spoil the verdict; and each port is tried against two different well-known services, so one company's outage no longer counts against your network.
- If the full check-up runs out of time on a badly degraded network, you now get everything that *did* finish, with the unfinished items clearly marked as timed out — instead of getting nothing at all. (If you have a script that watches for the old "timed out" message in the JSON output, it should now look at the new timed-out markers instead.)

**Improved — speed test accuracy**
- The way results from the different speed-test services are combined had a subtle flaw: a service that only managed to grab one or two measurements before failing was treated as *more* trustworthy than a service with lots of steady measurements — exactly backwards. That's fixed: sparse, unreliable contributions are now either set aside (and the results table tells you when that happened) or given the least say in the final number.
- Every service now produces far more individual measurements during its test — the M-Lab test used to contribute as little as one measurement per run, and one provider used to fetch such enormous chunks that slow connections barely got measured at all. Transfers now size themselves to your connection speed so fast and slow lines alike get a steady stream of samples.
- Your headline speed now comes with an honesty range — for example "941 Mbps ±12 Mbps" — so you can see how confident the measurement actually is. The fast.com latency reading, previously based on a single probe, is now a proper multi-probe measurement with jitter.
- The backup server list for one of the providers used to be entirely in Europe and the US East Coast; it now spans four continents, and the tool considers six times as many candidate servers when picking the closest one.

**Added — two new speed-test providers**
- The standalone speed test now runs **six** services instead of four. New are **M-Lab's next-generation multi-stream test** (which measures total capacity the way commercial speed tests do, where the older M-Lab test deliberately measures a single connection) and **Apple's network quality service** — the same infrastructure Apple's built-in macOS speed test uses. More independent, reputable measurements means the combined number is harder to skew. Each can be turned off with its own skip flag, and the full six-provider run takes about five and a half minutes at the default settings.
- We also looked hard at adding Speedtest.net (Ookla) — the most recognized name of all — and concluded their license terms simply don't allow an independent tool like this one to use their network. We've written the reasoning down in the code so the question doesn't have to be re-litigated.

**Added — technician mode is now exhaustive (18 → 25 deep checks)**
- **Route tracing**: follows your traffic hop by hop toward the internet, points out where your home network ends and your provider's begins, and flags whether a slowdown lives in your house, at your provider, or beyond.
- **Sustained packet-loss test**: thirty probes to each of three independent services — catches the intermittent drops that short tests miss.
- **Connection-sharing analysis**: detects "double NAT" (two routers stacked, a common cause of game/video-call trouble) and carrier-grade NAT (where your provider shares one public address among many customers — which quietly breaks port forwarding and hosting).
- **DNS service benchmark and honesty check**: times your name-lookup service against the big public alternatives, catches providers that hijack typo'd addresses to show ads, and checks whether your resolver verifies cryptographically-signed answers.
- **Wi-Fi link quality**: signal strength, channel, band, connection speed, and security — with a clear warning if you're on an open or obsolete-security network.
- **Captive portal detection**: spots hotel/café/airport sign-in pages that silently intercept your traffic.
- **Clock check**: a skewed system clock breaks secure connections in ways that look exactly like a broken network; the tool now measures your clock against internet time servers and says so plainly. (It found a real 1.2-second drift on our own test machine the first time it ran.)
- The existing deep checks got real teeth too: the bufferbloat test now genuinely measures latency *while* flooding the connection in both directions (it previously admitted its number was an estimate); the packet-size check now actually probes how large a packet can cross your whole path (catching VPN/tunnel squeeze); the IPv6 check now performs a real fetch over IPv6 rather than just noting an address exists; the proxy check verifies a configured auto-config script actually loads and warns if proxy auto-discovery is answering on your network; and the network-neighbors table is screened for signs of spoofing or a rogue second router.
- Technician mode takes about 2–4 minutes now — it's doing far more.

**Improved — the auto-repair command acts on solid evidence**
- Before changing anything, the fix command now **double-checks the problem with a second diagnostic pass** and only repairs what failed both times. A momentary blip now ends with "everything's fine" instead of an unnecessary repair. Problems that flickered between the two passes are noted in the report as transient.
- Its internal memory of "which repair helped" was giving every repair credit whenever anything got better — including things that recovered on their own. Credit now goes only to the repair actually aimed at the thing that recovered, so the tool's repair ordering learns from real wins.
- The fix command also no longer runs a full bandwidth test before every repair round (a repair can't fix raw speed anyway) — that alone frees most of its time budget for actual repairs. And after re-enabling your VPN, it now double-checks before concluding the VPN broke your connection and switching it back off.

**Changed (be aware if you script against the tool)**
- The default speed-test portion of a normal check-up runs a little longer (15 seconds per direction instead of 10) for steadier numbers — a standard run takes roughly half a minute longer.
- All the new data appears as *additions* to the JSON output; nothing existing was renamed or removed. The headline speed numbers can read differently than before on the same connection — that's the over-weighting fix doing its job, not a regression.

**Removed**
- A chunk of leftover machinery from the old fixed three-stage repair design (replaced two major versions ago) was deleted outright. Nothing you could reach still used it.

**Behind the scenes**
- About 110 new automated tests pin down the new verdict rules, the parsers for each operating system's command output, the statistics, and the repair loop's new double-checking behavior — including a test harness that can feed the repair loop scripted diagnostic results without touching a real network.

---

## June 8, 2026 — release pipeline made resilient to a GitHub outage

**Behind the scenes**
- While shipping the previous update, a GitHub content-delivery hiccup kept failing one small step of our automated build setup, which held up publishing for a few hours. We changed the way that build tool gets installed so the same kind of GitHub hiccup won't stall a release again. Nothing about the tool you run changes — this is purely about our release plumbing.

---

## June 8, 2026 — security tune-up of the underlying libraries

**Security**
- Updated several of the tool's underlying networking and encryption libraries to close known security advisories that our automated security scan had been flagging. These were in low-level building blocks, not in anything you interact with directly — updating them keeps the tool current and clears the warnings. Nothing about how you use the tool changes, and the results look exactly the same.

**Behind the scenes**
- Refreshed the progress-bar library to a current, maintained version (the old one relied on a small piece that is no longer maintained). The spinners and progress bars look and behave exactly as before.
- Two remaining low-level warnings can't be cleared yet: in both cases the fix lives in outside libraries we don't control, and in code paths the tool never actually runs. We've written down exactly why each one is safe to leave for now, so the security scan stays clean and honest, and we'll clear them when the upstream fixes arrive.

---

## June 8, 2026 — same release process across the tools, and a license file

**Changed (mostly behind the scenes)**
- ND-300 and its sibling tool TR-300 now build and ship releases the exact same way, so there's one consistent process across the products. The tool itself is unchanged.
- Added the project's license file to the repository, and the Windows setup programs now show the license during installation — matching the sibling tool.

**Behind the scenes**
- Releases are now cut by tagging a version (same as the sibling tool), and the cross-platform checks run on Mac, Linux, and Windows for every change so problems are caught earlier.

---

## June 8, 2026 — fix so all the Windows setup programs build and ship

**Fixed**
- Yesterday's "one copy at a time" cleanup feature shipped, but a small mistake in one of the Windows setup programs stopped two of the four Windows installers from being built and attached to the release. This release fixes that, so all four Windows installers are available again. The fix also simplifies how the "for everyone on this computer" setup program finds an old leftover copy to remove — it now uses the same safe, automatic approach as the other installers, and removing the other edition still skips gracefully if it would need administrator rights. Nothing about how you use the tool changes.

---

## June 7, 2026 — one copy of the tool, not several (automatic cleanup of leftover installs on Windows)

**Added**
- On Windows it used to be easy to end up with more than one copy of the tool on your computer at once — for example, an older copy installed through the Rust package manager sitting alongside a newer copy from a regular installer, with the old one quietly winning. You could also end up with both the "for everyone on this computer" edition and the "just for me" edition installed side by side. From now on the goal is simple: **one version, one edition, at a time.**
- The Windows installers now offer two tidy-up options, both turned on by default: remove an older leftover copy that was installed through the Rust package manager, and remove the other edition if it's also installed. You can untick either one during a normal install if you'd rather not. The cleanup is careful by design — it only ever removes the tool's own two programs, never your Rust toolchain (or anything else that lives in the same folder), never your Downloads folder, and never the copy you're currently running. If removing the other edition would need administrator rights you don't have, it simply skips that part and tells you, rather than failing.
- This same tidy-up now also happens automatically when the tool updates itself silently in the background — so a routine update quietly consolidates you down to a single, current install without you having to do anything. And because the cleanup can never cause an install or update to fail (it's strictly a helpful extra), there's no risk to your update if a leftover copy can't be removed.

**Behind the scenes**
- The cleanup reuses the tool's existing, already-tested "remove an install" machinery rather than introducing brand-new deletion code, which keeps the safety guarantees intact. macOS and Linux didn't need any of this — on those systems an update simply overwrites the single existing copy — so the change is Windows-only.

---

## June 7, 2026 — proper Windows installers, smarter updating, a new way to type the DNS command, and steadier speed tests

**Added**
- Windows now has four proper installers to choose from, and each one installs both the main tool and the standalone speed-test tool together. You can pick a normal "for everyone on this computer" install (which asks for administrator permission) or a "just for me" install that needs no admin rights at all — handy on locked-down work computers. Each comes in two flavors (a classic Windows Installer package and a regular setup program); pick whichever one flavor you prefer per install style. Every installer also ships with a fingerprint file so the tool can check a download wasn't tampered with before running it.
- There's a new, shorter way to type the DNS-change command. Instead of the old flag, you can just type the command word directly. It behaves exactly the same: if changing your DNS fails it stops, and if it succeeds it goes on to run the normal checks so you can immediately see the result. The old way still works.

**Improved**
- Updating on Windows is now much smarter. The tool remembers which kind of installer you used and, when you update, fetches the matching one to upgrade in place — rather than accidentally switching you to a different style and leaving two entries in your "installed programs" list. Before running any downloaded installer, it verifies the download against its published fingerprint and refuses to run if they don't match (protecting you from a corrupted or tampered download, including on networks that intercept traffic). After updating through the Rust package manager, it now double-checks that the new version actually took effect — because occasionally the package registry briefly still serves the old version right after a release — and if not, it quietly switches to the prebuilt installer instead of getting stuck repeatedly saying "an update is available." It also understands pre-release versions properly now, and tells you plainly when the update check hit GitHub's hourly request limit so you know to just wait.

**Fixed**
- The standalone speed test can no longer hang. If a server stalls partway through, individual requests now give up promptly instead of blocking for up to two minutes, and the whole test has an overall time limit as a backstop. The main tool already had this safety net; the standalone test now does too.
- The main tool's overall time limit now stretches to fit a longer speed test. If you deliberately ask for a long speed test, the tool no longer cuts it off early and wrongly reports your network as "severely degraded." Normal-length and skipped tests behave exactly as before.
- When a speed-test provider connects but can't actually transfer any data, the results table now shows a clear error for that provider instead of a blank/"not available" line — and that provider is correctly left out of the final averaged result.
- The standalone speed test now ends with a proper error signal when nothing could be measured at all, instead of pretending it succeeded. A genuinely slow-but-working connection is still treated as a success.
- The most thorough speed-test provider now keeps the measurements it already gathered if its connection drops partway through, rather than throwing the whole provider's results away. If it never managed to measure anything, it still reports a clear error.

**Heads up (only if you script against the tool's machine-readable output)**
- On Windows, the update command's machine-readable output now includes a field naming which kind of install you have, and the list of possible "how it updated" values gained entries for the four Windows installers. Nothing existing was renamed or removed; the new field doesn't appear on Mac or Linux.

---

## June 7, 2026 — honest results when the speed test can't run, plus quieter behind-the-scenes work

**Fixed**
- A speed test that completely fails now says so honestly, instead of pretending your connection is just slow. Before, if every speed-test server failed to respond, the tool would report "0.00 Mbps" and "download too slow," which made a total failure look like a working-but-slow connection — and it counted that as a minor warning. Now, when nothing could actually be measured, the tool clearly reports that the speed test failed and treats it as a real problem. A connection that genuinely is slow but working is still reported the same way as before.
- Fixed a safety time limit in the speed test that wasn't actually working. The speed test has a built-in cap meant to stop any single download or upload run from dragging on too long, but a coding slip meant the timer kept resetting and so never actually triggered. It now fires correctly, and the same safety cap was added to the upload side, which previously had none — so a stuck transfer can't make the speed test hang.

**Behind the scenes**
- Some quick system-information lookups (your network adapters and your router address) used to run in a way that could very briefly freeze the tool's other simultaneous checks. They now run out of the way so everything keeps flowing smoothly. No change to what you see — it's purely a smoothness improvement under the hood.

**Heads up**
- For anyone running Windows in a language other than English: a few of the advanced checks can come back blank because they currently look for English wording in Windows' output. We've noted this and plan to make those checks language-independent in a future update.

---

## June 7, 2026 — installing and updating are more robust, especially on locked-down machines

**Fixed**
- Installing the tool through the Rust package manager can no longer fail on locked-down or read-only machines. As part of installing, the tool generates its built-in manual pages and used to insist on saving them into a folder that, on some hardened or sandboxed computers, can't be written to — which would make the whole install crash. Now it only writes those pages where it's actually allowed to, and quietly skips the write if it can't, so the install always finishes. The manual pages still ship inside the package, so nothing is lost.
- On Windows, updating from an older copy that wasn't installed through the Rust package manager is now honest about what happened. Windows can't delete a program file while that program is still running, so the old copy is actually removed the moment the tool exits — not instantly. The tool used to claim the old copy was already gone, which could leave you quietly stuck on the old version. Now it tells you plainly that the old copy will be removed when the tool closes, that the freshly installed version will take over in a new window, and exactly what to delete by hand in the rare case it doesn't.
- Uninstalling on Windows now fully cleans up the leftover entry it adds to your system's list of program locations, even when that entry has an extra slash on the end. Before, a trailing slash could leave a dead entry behind; now it's matched and removed regardless.
- Hardened the internal cleanup step that schedules the old program file for deletion on Windows so it can't be tripped up by unusual characters in the install path. (These characters can't appear in real Windows paths anyway, so in practice this is just extra safety.)

**Changed (worth knowing if you read the machine-readable output)**
- The machine-readable output of the uninstall command gains one new yes/no field that says whether the program file is scheduled to be removed when the tool exits (this is the normal case on Windows). The existing "was it removed" field keeps its exact old meaning — "is it gone right now" — and the overall success flag now counts a scheduled-on-exit removal as a success. This is a purely additive change; nothing existing was renamed or removed, and on Mac and Linux the behavior is unchanged.

---

## June 7, 2026 — the auto-repair command now cleans up after itself

**Added**
- The auto-repair command now puts your network back the way it found it if it's interrupted or hits a problem. Before, if you cancelled a repair partway through, or it ran out of time, or it hit an unexpected error, you could be left worse off than when you started — your Wi-Fi adapter switched off, your VPN disconnected, or (on a Mac) your network settings removed. Now the tool keeps a running list of everything it changes, and on any exit — you press Ctrl-C, it times out, or it crashes — it goes back through that list and restores what it touched: your Wi-Fi adapter, your VPN, and your Mac network settings all get put back instead of left broken. Press Ctrl-C any time and it cleans up after itself before quitting.

**Fixed**
- Disconnected VPNs are reconnected. When the repair temporarily turns off a personal VPN to test your connection, it now reliably offers to turn it back on (and reconnects it automatically if the repair is interrupted first). Previously a turned-off VPN could be left off without warning. Work/company VPNs are still never touched automatically.
- A Mac can no longer be left with no working network connection. The deep-reset step that removes and recreates a Mac's network service now retries the recreate and restores your previous settings; if it still can't recreate the service, it tells you exactly what to fix by hand instead of quietly giving up halfway.
- Restarting your network adapter no longer risks leaving it switched off. If the tool can't switch the adapter back on the first time, it waits and tries again; if it still can't, it shows you the exact command to turn it back on yourself, in big clear letters, rather than leaving you disconnected.
- On Linux, the repair no longer claims it deleted a network profile when the deletion actually failed — it now checks and reports the real result.

**Changed (worth knowing if you read the machine-readable output or the report)**
- The repair's machine-readable output now says whether it was interrupted and lists anything it couldn't put back on its own. The saved repair report gets a clearly-marked "Manual recovery needed" section for the same information.

**Behind the scenes**
- One existing automated timing test from the previous release can occasionally report a false failure on very fast machines; it was left as-is to keep this update focused, and confirmed to be a pre-existing quirk rather than something this update introduced.

---

## June 7, 2026 — no more hanging on a broken network; faster results

**Fixed**
- The tool no longer hangs when your network or DNS is broken. Before, if your internet was down or your DNS server was unreachable, the tool could sit there for a long time waiting on the operating system before giving up. Now every check it runs has a built-in time limit, so a dead connection produces a clear "unreachable" result quickly instead of an apparent freeze. On a healthy network nothing changes — you get the same results as before. This also helps the auto-repair command, which runs the same checks while it works.
- Added an overall safety net: if the whole set of checks somehow takes too long, the tool now stops itself after about a minute and a half and tells you your network looks severely degraded, rather than appearing stuck. You can also press Ctrl-C at any time to cancel cleanly.

**Improved**
- The latency checks (the ping tests to a few well-known servers) now run all at once instead of one after another, so results come back faster — noticeably so on a flaky or dead connection, where the old one-at-a-time approach was slowest.

**Behind the scenes**
- Cleaned up some code-quality warnings flagged by newer build tooling so the project's automated checks pass cleanly again. No change to how the tool behaves.

---

## May 11, 2026 — docs republish

**Behind the scenes**
- Republished the package listing on the public Rust package registry so the project page there matches the up-to-date documentation in this repo. No change to how the tool actually behaves.
- Brought the internal agent and project-plan documents back in line with how releases and updates actually work today. Again, nothing user-facing changed.

---

## May 11, 2026 — safer updates over older installs

**Fixed**
- When you run the built-in updater and you previously installed the tool through one of the older methods (the shell installer, the Windows installer, or the MSI), the updater now cleans up the old install location after the new one is in place. Previously the old copy could quietly "win" the next time you typed the command, leaving you on the old version even though the new one was installed. Now the new one always takes over.
- If the updater hits a "there's already a copy of this tool here" error while installing via the Rust package path, it now removes the conflicting copy and tries again automatically instead of giving up.

**Behind the scenes**
- Clarified in the docs that installing the tool with the bare Rust package command (without going through our updater) won't run our cleanup steps for old install locations. If you have an older install hanging around, run our built-in update or uninstall command first so it can tidy up.

---

## May 11, 2026 — release-notes overhaul

**Behind the scenes**
- Expanded the release notes so anyone reading them sees the full picture of how a release gets built and published — the source checks, the cross-platform build matrix, the GitHub release, and the final publish to the public Rust package registry.
- Documented every supported way to install or update the tool in one place: the Rust package manager, the shell installer for Mac/Linux, the PowerShell installer for Windows, the Windows MSI, and the legacy installer names that still work for older copies.
- Updated the project's internal agent instructions so future automated work follows the new GitHub-driven release flow instead of the old hand-rolled local publishing steps.
- Bumped the version so the corrected docs are baked into the next published artifact. No behavior change versus the previous release.

---

## May 11, 2026 — first official Rust package registry publish

**Added**
- The tool is now available on the standard public Rust package registry under its proper name. You can install it with a single straightforward command from anywhere a Rust toolchain is set up.
- Every push to the main branch now automatically runs the full release machinery — formatting, tests, lint, the build for all supported platforms, the GitHub release, and the publish to the Rust package registry — so releases are no longer hand-driven.
- Legacy installer names are now published alongside the new ones, so older copies of the tool already out in the wild can still find their update file at the URL they remember.

**Improved**
- The README install instructions now cover every supported way to get the tool: shell installer, PowerShell installer, Windows MSI, the Rust package manager, and building from source.
- The Windows MSI's display name and install folder now match the tool's new official package name.

**Behind the scenes**
- The internal library name stays the same so existing code that depends on it keeps working — only the publish/package name was tidied up.
- Fixed a small compatibility issue with the latest version of the Rust linter that the build runner uses.

---

## May 10, 2026 — smarter "fix" command

**Fixed**
- The auto-repair command is now driven by actual evidence instead of running every step regardless of what's broken. If your network is already healthy, it does nothing. If your slow speeds are the only symptom, it tells you that's only a hint (since slow ping responses can be a normal quirk of healthy networks) instead of treating it as a problem. If DNS is broken, it starts with a quick cache flush and only considers swapping in a public DNS provider after the safer fixes haven't helped.
- The "auto-confirm" option (the `--yes` flag) now actually reaches the repair flow, so automated scripts can confirm the medium-risk steps without prompting. The genuinely risky steps still require a real person to type "y" — that's by design.
- When the auto-repair runs in script-friendly modes (JSON output or non-interactive), it no longer turns off consumer VPNs. It can't safely walk you through turning them back on in that mode, so it just leaves them alone.
- The deep network reset on Mac now remembers your DNS, proxy settings, network service order, and IPv4/IPv6 mode before resetting, and puts them back the way they were afterwards — instead of unilaterally forcing your Mac onto DHCP plus a hardcoded public DNS provider.
- On Linux, DNS changes now correctly find the active network profile for the network card you're using, instead of assuming the profile and the card share a name (which often isn't true). Wi-Fi profile resets also now collect the reconnection details before deleting the old profile, so you don't get stranded.
- On Mac, the latency check now uses the correct timeout value when pinging. Previously a typo in the timeout meant healthy servers could be reported as unreachable.
- Speed tests now combine results from every provider that succeeded — weighted by how confident each one is — and flag it when the providers disagree significantly. They also throw out failed transfers and won't ignore short test durations anymore.
- The "fast mode" and custom-speed-test-duration options now work whether you put them before or after the sub-command. So `nd300 fix --fast` works the same as `nd300 --fast fix`.

**Behind the scenes**
- Added a fresh round of regression tests covering all the above, so these don't quietly regress in the future.

---

## May 7, 2026 — updater no longer crashes when Rust isn't installed

**Fixed**
- The updater no longer crashes with a confusing "file not found" error when you installed the tool via the official shell installer but don't actually have Rust installed on your machine. Before this fix, the updater would see the binary sitting in a folder that *looked* like a Rust install (because the shell installer happens to put it in the same place when a Rust setup is detected) and would assume you could update via the Rust package manager — then crash when that tool wasn't there. This bug had been quietly shipping for a while.

**Improved**
- The updater now tries each install method in turn and falls back to the next one if a method isn't available. If Rust isn't installed at all, it's skipped entirely instead of attempted. On Mac and Linux it tries `curl`, then `wget`. On Windows it tries Windows PowerShell, then the cross-platform PowerShell. If nothing works, the error message now shows you exactly what was tried and what failed at each step.
- Tightened up the installer scripts so a 404 or DNS error fails loudly instead of being silently swallowed. There were a couple of latent "fails quietly and pretends to succeed" bugs in the same area — those are fixed too.
- When the updater is run in script mode, the output now includes more detail about which install strategy was picked and, on failure, the full list of what it tried.

---

## May 7, 2026 — Mac build fix

**Fixed**
- Fixed a Mac-only build issue from the previous release that was preventing new Mac releases from being published. Nothing was broken on Windows or Linux — this is a behind-the-scenes fix to get the Mac build green again so users on those machines could actually receive the new version.

---

## May 7, 2026 — the fix command, completely reinvented

This is a major release. The auto-repair command (`nd300 fix`) was rebuilt from the ground up. The old version always ran the same fixed sequence of three "stages" regardless of what was actually wrong. The new version is much smarter.

**Added**
- The fix command now runs your network diagnostics first, identifies *which specific things* are broken, applies *only* the repair steps that target those problems, runs the diagnostics again, and repeats until everything passes or there's nothing more it can safely try. It's bounded so it always finishes — at most six rounds and at most four minutes.
- A new sub-command form for the action commands. You can now type `nd300 fix`, `nd300 update`, `nd300 clear-dns`, and `nd300 uninstall` instead of (or in addition to) the older flag forms like `nd300 -f` and `nd300 --update`. Both styles still work and do exactly the same thing.
- When a cluster of problems all trace back to the same root cause (say, your network card being disabled, which makes the gateway, DNS, and public-IP checks all fail at once), the fix command now goes after the root cause instead of wasting time on the cascade of downstream symptoms.
- Genuinely risky repair steps (resetting Windows' networking subsystem, recreating the Mac network service, deleting a Linux network profile) now show a plain-language explanation before running. You see what the step does, why it's being attempted, what you'll experience, whether it's reversible, and how long it usually takes. You have to type "y" to proceed — Enter, blank, or anything else means no. **The auto-confirm option does NOT bypass these prompts** — they always require a real person, on purpose, so non-technical users can't accidentally OK something destructive.
- The fix command now recognizes situations it can't fix on its own and exits cleanly with guidance instead of thrashing. Examples: there's literally no cable or Wi-Fi link, your Internet provider is having an outage, or you're connected through a work VPN.
- Within a single fix run, if two different repair steps would both address the same problem, the one that's already shown it helps gets tried first.
- Each repair step now declares how long the system needs to settle before re-testing. DNS cache flush is fast, restarting a network card is slow — so the fix command waits the right amount of time after each instead of using a single one-size-fits-all delay.
- When a repair step rebuilds your network plumbing (renewing DHCP, restarting an adapter, recreating a profile), the fix command stops what it was planning to do next and immediately re-runs diagnostics, since the old plan was made against a network state that no longer exists.
- The fix command only considers repair steps that make sense for your operating system — Windows-only steps never show up on Mac and vice versa.
- After every fix run, you get a detailed Markdown report saved to your Downloads folder. It contains a plain-language summary, a snapshot of what was broken at the start, a per-round timeline of what was tried (including the actual command output and how long each step took), a final snapshot, your environment info, and a "what to try next" section if anything's still not fixed.
- A new `--yes` (or `-y`) option auto-confirms the medium-risk prompts when running from a script. It does *not* auto-confirm the high-risk prompts — those still require a human.

**Improved**
- A healthy network now finishes the fix command in under 8 seconds with zero changes applied. Previously it would still run a full first-stage sequence whether you needed it or not.
- The output formatting options (script mode, plain ASCII, no color, verbose) now work whether you put them before or after the sub-command.

**Changed (be aware if you script against the tool)**
- The script-mode output from the fix command has a new structure built around the new round-based behavior. Any external scripts that were reading the old "stages" array will need updating. The new format gives you a per-round breakdown — see the technical changelog for the field names.

**Still works the same**
- Every existing command-line flag still works exactly as before. The sub-command form is purely additive.

---

## April 12, 2026 — self-update + dramatically better speed-test accuracy

**Added**
- A built-in update command. Run `nd300 --update` (or `speedqx --update`) and it'll check for a newer version and reinstall using whichever method makes sense for your machine — the shell installer, the PowerShell installer, or the Rust package manager. Works in script mode too.
- A serious overhaul of how speed-test numbers are calculated. This is the same math the QubeTX web-based speed test uses. The short version: results are way more resistant to misleading outliers (a single weird sample no longer drags your number around), and the slow first few seconds of any test are now properly excluded so your number reflects how fast your connection actually goes, not how fast it ramps up.
- Speed tests now show a confidence interval — a small range around the headline number — so you can tell how repeatable each result is.
- Connection stability is now tracked too. You can see whether your speed was steady or wildly variable during the test.
- When the different speed-test providers disagree by more than 30%, you'll see a flag. That's usually a sign that something on the network (your ISP, a work VPN, a proxy) is treating different providers differently.

**Improved**
- Combining the results from multiple speed-test providers is now smarter — providers with more consistent results contribute more to the final number than ones that gave noisy data.
- The way the tool measures ping jitter (how much your latency varies sample to sample) now uses the standard method used by professional networking gear instead of a simpler approximation.

---

## March 19, 2026 — four-provider speed test

**Added**
- The speed test now runs against four providers simultaneously — Cloudflare, M-Lab, LibreSpeed, and Netflix's fast.com — and combines them for a much more accurate single number.
- A new way to specify how long to test: when you say "30 seconds" you now get a full 30 seconds for download *and* 30 seconds for upload from each provider, instead of 30 seconds total split between them.
- Better progress display during the test: which provider is running, a per-provider summary as each finishes, and an estimated total time up front.
- The upload portion of the test now ramps up the size of each data chunk during the test so it can keep up with fast connections instead of bottlenecking on small chunks.

**Improved**
- Ping is now reported as the *fastest* response across all providers, not the median — which more accurately reflects the actual minimum round-trip your connection can achieve.
- The shorter speed test inside the regular diagnostics still only uses Cloudflare and M-Lab for speed reasons. The four-provider behavior is for the standalone speed-test tool.

**Fixed**
- The M-Lab test now actually respects the duration you ask for. Previously a 30-second test would often take 40+ seconds because of how the test loop was structured.

---

## March 12, 2026 — multiple small releases on one day

These four small releases all went out the same day. Bundling them here.

**Fixed**
- Fixed the M-Lab speed-test provider failing to connect at all. The connection request was missing a small but mandatory piece of information; once that was added, connections work again. Also added a fallback path for environments with corporate certificate authorities that were rejecting the connection.
- Fixed the Windows installer being broken in the previous day's release. The new standalone speed-test program wasn't being added to the installer manifest, so MSI builds were failing.

**Added**
- The tool now ships with proper Mac/Linux manual pages. So if you type `man nd300` on a Mac or Linux machine, you get real reference docs.
- The standalone speed-test program now has usage examples in its help output, matching the main tool.

**Behind the scenes**
- Upgraded the release automation to a newer version that doesn't trip Node.js deprecation warnings on the build servers.
- Pulled the command-line definitions into a shared location so both programs (and the manual-page generator) can share them.

---

## March 12, 2026 — SpeedQX, the standalone speed-test program

**Added**
- A brand new standalone speed-test program called **SpeedQX**. It ships alongside the main diagnostic tool — you get both with one install. SpeedQX is just the speed-test part if you don't need the full diagnostics.
- Both programs now run two speed-test providers (Cloudflare and M-Lab) and average the results for more accuracy.
- Real ping, jitter, and packet loss measurements added to the speed test output — not just download and upload.
- The user-facing speed line now includes the ping value (e.g., `242 Mbps down / 30 Mbps up (12ms)`).
- The deeper "technician" view of the speed test now shows the per-provider breakdown — what each provider measured, which server it talked to, and how much data was transferred.
- A handful of new options on the standalone speed-test program: test duration (with `auto` as an option), how many ping probes to send, and the same output-format flags the main tool has.
- The uninstall command now removes both programs together.

**Changed (be aware if you script against the tool)**
- The script-mode output for speed details has changed shape. It now includes the new ping/jitter/loss numbers, a per-provider breakdown, and how long each test took.

---

## February 22, 2026 — fix reports + smarter default DNS

**Added**
- After running the auto-repair command, you now get a Markdown report saved to your Downloads folder. It walks through what was broken, what was fixed, what the likely root cause was (corrupted network stack? stale cache? bad DNS? bad interface?), and what to try next if you still have problems.
- A summary panel printed to the screen right after the fix completes — quick to skim, no need to open the report file unless you want details.

**Improved**
- The auto-repair command's "what's the root cause here" inference is much better. Instead of just listing what was done, it explains which problem each step was targeting.
- The script-mode output for the auto-repair now includes the path to the saved report file.

**Changed**
- The default DNS provider used by the various DNS-related fixes is now Cloudflare (`1.1.1.1`) instead of a Cloudflare+Google "hybrid" setup. The hybrid mode could cause sticky failover problems where the system would get stuck on a single provider, defeating the purpose of having two.
- Cloudflare is now option 1 in the DNS chooser menu (with "recommended" next to it). The old hybrid option is moved to the bottom of the list with a "not recommended" label.

---

## February 22, 2026 — DNS reset comes first

**Improved**
- The first step of the auto-repair command is now resetting your DNS settings back to "Automatic" — letting your router or ISP provide the DNS servers. Most DNS problems come from a previously-set custom DNS that's no longer working, so trying the simplest possible fix first usually does the trick. If that doesn't restore connectivity, the tool still offers to set a public DNS provider as a fallback.

---

## February 15, 2026 — `-v` for version

**Changed**
- The short-form flag for "show version" is now lowercase `-v` instead of uppercase `-V`. It's just easier to type and matches what most CLI tools use.

---

## February 15, 2026 — standalone DNS changer + NextDNS support

**Added**
- A new `-d` (or `--dns`) command that's just for changing your DNS. Pick a provider, the tool sets it, verifies your connection works with the new settings, and automatically reverts if anything goes wrong. After that, the full diagnostics run as usual.
- Added support for NextDNS as a DNS provider, with full encrypted-DNS configuration on all three operating systems (DNS-over-HTTPS on Windows, DNS-over-TLS on Linux, and the official NextDNS client on Mac).
- The DNS provider menu now has more options: Hybrid (Cloudflare + Google), Cloudflare, Google, NextDNS, or Automatic (let your router pick).
- The standalone DNS command supports script mode with structured success/failure/revert output.
- If you change DNS to a provider and then your connection breaks (because, say, you're on a corporate network that blocks public DNS), the tool automatically reverts to "Automatic" without you having to do anything.
- The `--help` output is now organized into clear sections (Modes, Output, Speed Test, Actions) with usage examples at the bottom.

**Behind the scenes**
- Quieted a Linux-only "unused code" warning that was making the build a bit noisier than it needed to be.

---

## February 15, 2026 — Linux build fix

**Fixed**
- Fixed a few small Linux-only build issues that snuck in with the previous release — a missing import inside a Linux-only code branch, and a couple of "this isn't used here, but it's used over there" warnings. No user-facing change.

---

## February 15, 2026 — DNS actually checked, plus Mac DNS gap fix

**Added**
- The auto-repair command now actually checks that DNS works (not just that HTTP works) at every stage. It does this by trying to look up three known domains, so it can tell the difference between "my Internet is up but DNS is broken" and "everything is fine."
- When DNS isn't working, you get a clear chooser: Hybrid (Cloudflare + Google, recommended), Cloudflare only, Google only, or Automatic.
- Before committing to a public DNS server, the tool now tests that the server is reachable from your network. If you're behind a corporate firewall that blocks public DNS, the tool catches that up front and adjusts instead of leaving you with DNS that can't talk to anyone.

**Fixed**
- Fixed the auto-repair on Mac reporting "you're connected!" when DNS was actually broken. There was a brief window after Mac's networking subsystem restart where HTTP would work (because of cached connections) but new DNS lookups would fail. The tool now explicitly checks DNS instead of trusting HTTP to be a proxy for "everything's fine."
- Fixed a Mac Wi-Fi issue where the DHCP renew step would fail because it ran before the Wi-Fi card had finished reconnecting. There's now a 5-second wait so things get a chance to settle.
- Fixed a related Mac issue where, after recreating the network service, there'd be a DNS gap until DHCP delivered the new servers. Now hybrid DNS is set immediately so you don't go through a dark period.

---

## February 13, 2026 — Mac sudo check + smaller fixes

**Fixed**
- On Mac, running the auto-repair without `sudo` used to silently fail every step. Now the tool checks for admin privileges up front and exits immediately with a clear "run this with `sudo`" message instead of letting you watch a wall of failures scroll by.
- The script-mode version of the same problem now returns a clean structured error saying privileges are required, instead of silently producing a failed-everything result.
- Fixed a confusing case on Mac where flushing the DNS cache would report failure if part of it needed root and part of it didn't. Now it reports partial success when one part worked and the other didn't, instead of pretending the whole thing failed.

---

## February 13, 2026 — way faster Windows diagnostics

**Improved**
- Windows network adapter info now comes from native Windows APIs instead of the older, slower WMI database. That's roughly 5 milliseconds versus 300–500 milliseconds for the same info — and the new path is more accurate too, properly identifying Wi-Fi, Ethernet, Bluetooth, and cellular adapters by their actual type instead of lumping everything as generic "Ethernet 802.3."
- Adapter status descriptions are more precise: you'll see "No Cable" vs "Disabled" vs "Down" vs "Standby" depending on what's actually true, instead of a vague catch-all.
- Detecting the right adapter for the auto-repair command now uses the same routing-table preference Windows itself uses. So if you have multiple connections, the tool picks the one Windows actually uses, not just the first one it finds.

**Added**
- The user-facing summary now shows each adapter's link speed (e.g., "Wi-Fi 866 Mbps").
- The deeper "technician" view shows a lot more per-adapter detail: MAC address, send and receive link speeds, gateway, DNS servers, MTU, and routing preference.
- Several new fields in the script-mode output for those who automate around it.

**Behind the scenes**
- Some Windows-specific deep info (the driver list) is now only collected in technician mode, so regular users don't pay the cost of looking it up.
- Drivers are now matched to adapters by hardware description instead of name substring, which is more reliable.

---

## February 13, 2026 — clearer adapter labels + fewer redundant calls

**Added**
- The diagnostic summary now tells you *what kind* of adapters are active, not just the count. So instead of "2 active" you'll see "2 active (Ethernet, Wi-Fi)."
- When the tool detects problems, you'll now see a hint suggesting you run the auto-repair command.

**Improved**
- The deeper "technician" view used to make the same system call multiple times for different diagnostic modules. Now each call happens once and the result is shared. Saves a couple hundred milliseconds in technician mode.
- "Bluetooth Device (Personal Area Network)" is now shown as "BT PAN" in the adapter summary so it's not confused with regular Bluetooth radio status.
- Better detection of virtual adapters (the kind virtual machines and certain network tools create) so they're labeled correctly.

---

## February 12, 2026 — auto-repair stops reporting false failures

**Fixed**
- After the auto-repair finishes, the connectivity check now retries up to three times with a 30-second visible countdown between attempts instead of just checking once. The old single-check behavior was reporting "fix failed" when actually your Wi-Fi just hadn't finished reconnecting yet (which can take 15–30+ seconds on Windows). The countdown also means the tool doesn't *look* frozen during those waits.

---

## February 11, 2026 — more Windows fixes

**Fixed**
- Fixed certain Windows service restarts failing with permission errors. The old approach didn't work for "protected" Windows services like the DNS client and DHCP — the tool now uses the proper Windows service-control method, with PowerShell as a backup, and gracefully degrades instead of hard-failing if neither works.
- Cleaned up some leftover terminal escape codes that were leaking through into the output on certain Windows terminals — you might have seen a stray weird character before the VPN check ran. Gone now.

---

## February 11, 2026 — timeouts everywhere + better progress feedback

**Fixed**
- Every command the auto-repair runs now has a timeout, so the tool can't get stuck waiting forever on a hung system command. There are three tiers: 15 seconds for quick things like a DNS cache flush, 30 seconds for medium things like a service restart, and 60 seconds for slow things like a DHCP renew.
- Added loading spinners to several auto-repair steps that previously ran silently. Things like VPN detection, the post-stage connectivity waits, and the interface disable/re-enable cycle now show progress so you can tell the tool is still working.
- Added a retry if a network interface fails to come back up after being toggled — prevents the worst case of leaving you with a disabled interface and no fix.
- Fixed a Linux-only compile issue that would have broken Linux builds otherwise.

---

## February 9, 2026 — the auto-repair gets graduated

**Improved**
- The auto-repair command is now smarter about how aggressive to get. It runs in three escalating stages — start gentle, escalate only if the gentler stuff didn't work — and checks if you're back online between each stage so it stops as soon as your network is healthy again.
  - Stage 1 is the gentle stuff: DNS flush, ARP cache flush, service restart, DHCP renew. Runs automatically.
  - Stage 2 disables and re-enables your default network adapter. Asks before running.
  - Stage 3 is the heavy stuff: resetting the Windows networking stack, deleting and rebuilding the Wi-Fi profile, etc. Asks with a clear warning.
- Linux adapter restart is now supported (it used to be skipped).
- In script mode, only stages 1 and 2 run — stage 3 requires user interaction, so it's skipped.

**Added**
- VPN detection and handling, built into the auto-repair. Before doing anything else, the tool checks if you have a VPN connected.
  - **Work / enterprise VPNs** (Cisco AnyConnect, Palo Alto GlobalProtect, Zscaler, etc.) are *never* touched automatically — those are explicitly off-limits.
  - **Consumer VPNs** (NordVPN, ExpressVPN, Mullvad, Tailscale, WireGuard) can be disabled via their official command-line tools so they're not interfering with the repair.
  - For unknown VPNs, the tool offers to disable them at the network-adapter level as a last resort.
  - After the repair, the tool offers to turn them back on — and if turning them on breaks things again, it'll auto-disable them so you're not stuck.
- Better Linux DNS flush: it now detects and flushes all three common Linux DNS caching layers instead of just one.
- The VPN-related part of the diagnostic now tells you which VPN vendor it detected, whether it's an enterprise-grade VPN, and what the operating system calls the interface.
- Stage 3 on Mac now captures your Wi-Fi password from the system keychain so it can reconnect you after rebuilding the network service.
- Stage 3 on Linux can now delete and recreate a NetworkManager Wi-Fi profile, with a fallback for systems that use a different Wi-Fi setup.

---

## February 8, 2026 — Mac/Linux build fix

**Fixed**
- Fixed a Mac-only and a couple of Mac/Linux warnings introduced in the previous release. Build-only — no user-facing behavior change.

---

## February 8, 2026 — the action commands arrive

**Added**
- A `-c` (or `--clear-dns`) command to flush your DNS cache. Works on all three operating systems.
- A `-f` (or `--fix`) command to attempt a full network reset (DNS flush + routing cache flush + DHCP renewal) automatically.
- An interactive prompt during the auto-repair to also restart your network adapters if needed.
- An `--uninstall` command that fully removes the tool from your machine — binary, install records, and PATH entry.
- Script-mode output for all of these so you can wire them into automation.

**Behind the scenes**
- Replaced some risky shortcuts in the code that could have crashed the tool with safe fallbacks.
- The uninstall command on Windows now preserves the exact format of your system PATH variable instead of accidentally changing how it's stored.

---

## February 8, 2026 — first patch releases

**Fixed**
- Fixed a Mac/Linux-only comparison bug in the IPv6 diagnostic that was breaking the build on those platforms.
- Added the missing identifier needed by the Windows installer (MSI) generator.

---

## February 8, 2026 — initial release

The first public release of ND300 — a cross-platform network diagnostic tool for Windows, Mac, and Linux.

**Added**
- Two viewing modes: User mode (clean summary, just what most people need) and Technician mode (deep diagnostics with lots of detail).
- 8 main diagnostic checks: network adapters, interfaces, gateway, DNS, public IP, latency, speed test, and port connectivity.
- 17 additional deep-dive checks available in technician mode: routing table, active connections, listening ports, DHCP details, protocol stats, adapter hardware, proxy settings, VPN status, firewall, DNS cache, IPv6, MTU, connection states, bufferbloat, reverse DNS, TLS inspection, and traffic counters.
- A clean Unicode table layout for results (with a plain-ASCII fallback if your terminal can't handle it).
- Color-coded status indicators (OK / Warn / Fail / Skip).
- A script-friendly JSON output mode.
- A built-in Cloudflare speed test with configurable duration.
- Bufferbloat detection with a graded score.
- Cross-platform installers — shell installer for Mac/Linux, PowerShell installer for Windows, plus a Windows MSI.

**Fixed**
- 19 separate small bugs caught in pre-release auditing.
