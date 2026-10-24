# Kurobayomi: how to build and run waves fast and safely

This file is for Claude working on this repo. The speed-ups below cut a GUI wave from about 2.5 to 3 hours down to about 1 hour (waves 38 and 39 vs wave 42) and kept the 15 GB machine from running out of memory.

## Speed-ups (all in use since 2026-10-10)

1. **Give lanes their own build folder.** One shared `target/` has one lock, so every lane's `cargo test` queues behind the others. Seed extra folders instantly (btrfs reflink): `cp -a --reflink=auto ~/kurobayomi/target ~/kurobayomi-targets/B` (also `C`). Put `CARGO_TARGET_DIR=$HOME/kurobayomi-targets/B` in the lane prompt. Spread lanes across the shared folder, B and C.
2. **Incremental builds on.** `.cargo/config.toml` (git-excluded) sets `incremental = true`. The 126k-line GUI crate otherwise recompiles from scratch on every test run.
3. **Hard cap on concurrent compiles.** `.cargo/config.toml` also sets `rustc-wrapper = "/home/blackbox/kurobayomi-notes/rustc-limit.sh"`, which lets at most `N` (now 3) rustc processes run machine-wide, whatever `-j` the lane passes. Lower `N` in the script when memory stalls (PSI full) stay above about 15%. The `.cargo/` folder is excluded through `.git/info/exclude`, so recreate it on a new machine.
4. **No final "verify" agent in a wave.** It cannot give a clean result while other lanes edit. Verify yourself instead: commit a snapshot, `git worktree add --detach ~/kurobayomi-verify HEAD`, run `CARGO_TARGET_DIR=$HOME/kurobayomi-verify/target cargo test --workspace --no-fail-fast -j 2` there, push that exact commit, then remove the worktree.
5. **Disjoint ownership.** Crate lanes own crates. GUI lanes own separate `src/<module>/` folders. Shared files (`src/main.rs`, `src/ui.rs`, `src/screens.rs`, `Cargo.toml`) get small additive edits only. A lane that sees compile errors in files it does not own waits 90 seconds and retries up to 5 times.
6. **GUI tests run on one GTK thread.** Use `crate::testing::with_display(...)`; tests that need a display skip themselves when none exists. Without this the test binary crashed with SIGSEGV about 1 run in 4.
7. **Wave shape.** About 25 tasks per lane. Implement and review on Haiku 5.5 high, fix on Sonnet 5.5 high, pinned explicitly in every `agent()` call. P0 tasks first. Keep at most about 11 lanes running.

## Housekeeping that keeps the machine alive

- `/tmp` is RAM (tmpfs). Agents leave big scratch cargo folders there. `~/kurobayomi-notes/prune-tmp.py` (run by the watcher every 15 minutes) deletes ones over 300 MB that are 45 minutes stale. Tell lanes to build scratch projects on disk, not in `/tmp`.
- `~/kurobayomi-notes/prune-target.py` (hourly) deletes only OUR crates' superseded artifacts. Never delete dependency artifacts: cargo then fails with "extern location ... does not exist" (fix: `cargo clean -p <dep>`).
- `~/kurobayomi-notes/watcher.py` is armed with the Monitor tool (30 minute timeout, re-arm each time). It reports idle capacity, memory stalls and prune results.
- systemd-oomd kills the biggest swap user when swap passes 90%. It killed Claude Desktop once. Mark the live Claude cgroups with `setfattr -n user.oomd_avoid -v 1` (resets when the app restarts).
- Never `pkill -f` a pattern that also appears in your own command line.

## RAM watching and limits (the user's standing rule: never let the machine run out of memory)

The machine has 15 GB RAM plus a 15 GB zram swap that sits at 8 to 15 GB used, so judge memory by stalls and available RAM, not by swap alone.

- **What to watch:** `free -m` (available), `/proc/pressure/memory` (`full avg10/avg60/avg300`, percent of time all tasks stalled), `df -h /tmp` (tmpfs is RAM). The watcher prints `MEMORY WARNING` when available RAM drops below 2500 MB, or when swap is above 15.3 GB together with stalls above 8%.
- **Rough sizes:** one rustc is 0.4 to 1.4 GB (the GUI crate and linking are the big ones). Three of them plus the other apps (Brave about 1 GB, Claude Desktop 0.5 GB) are what pushed the machine into swap.
- **Thresholds and responses:**
  - Stalls under about 10% and available above 4 GB: normal, do nothing.
  - Stalls 10 to 15% for several minutes: lower `N` in `rustc-limit.sh` by one, or launch no new lanes until a wave finishes.
  - Stalls above 15% sustained, or available under 2 GB: stop launching, lower `N` to 1, check `df /tmp`, run `prune-tmp.py`, and ask the user to close big apps (Lens, Brave). Do not kill running workflows or agents.
- **Lane budget:** at most about 11 lanes at once, at most 2 compiles at once (cap `N`), cargo `jobs = 2`.
- **Usage limit:** agents can fail with "You've hit your weekly limit ... resets <date>". It has been transient (agents worked again minutes later). Resume the same wave with `resumeFromRunId`, then check that agent transcripts are being written. Do not stop other work, and do not assume everything is blocked until the reset date.
- **Never stop a running workflow without being asked.** A user saying "pause" does not mean TaskStop.
- **Disk:** every build folder (`~/kurobayomi/target`, `~/kurobayomi-targets/*`) grows to 30 to 100 GB, mostly from incremental builds, and agents create their own folders there. On 2026-10-10 `/home` fell from 126 GB to 39 GB free in a few hours. `~/kurobayomi-notes/prune-disk.py` (run by the watcher every 10 minutes) deletes idle folders (no cargo or rustc using them, untouched for 45 minutes; for 2 hours when space is fine) and, under 60 GB free, deletes idle folders' incremental data and then the least recently used idle folders. Never delete a folder a running build uses. Tell lanes to reuse `B` or `C` instead of copying a new target folder.

## CI checks that agents keep breaking (read before editing paths or platform code)

- **Literal path joins (`scripts/check-path-literals.sh`).** Outside `crates/kuro-platform`, never write `paths.cache_dir().join("x")`, `paths.data_dir().join(..)` or any other Paths accessor followed by `.join(` with a literal. Ask the `Paths` trait for the role instead (`extensions_dir()`, `image_cache_dir()`, `chapter_cache_dir()`, `auto_backups_dir()`, `cookie_jar_file()`, ...). If no accessor fits, add a default method to the `Paths` trait in `crates/kuro-platform/src/paths.rs` with a test, then call it. The check reads the accessor list from kuro-platform, so only real Paths accessors are flagged (test helpers named `fixture_dir()` or glib's `home_dir()` are fine).
- **OS-specific code boundary (`scripts/check-platform-boundary.sh`).** The words the script greps for (OS cfg attributes, the desktop opener command, the keyring library, the D-Bus crate and the base-directory environment variable prefix; the script lists them) must not appear anywhere outside `crates/kuro-platform`, not even in comments or docs. Describe the idea in other words, for example "the platform data directory".
- **GitHub owner and app ID (`identity.toml`).** Never edit `kurobayomi/kurobayomi` or `io.github.anas1412.Kurobayomi` by hand: `scripts/set-identity.py --owner X` (or `--app-id`) rewrites every file and renames the icon files; the `Identity` workflow runs `--check` on every push. Plan: `docs/ORG-MIGRATION.md`.
- **Source traffic stays on the internet (security).** Sources are written by anyone. Requests for a source go through the app's client (`net_client::build`, built with `Transport::public_only`), and links a source hands to something else (the player, the HLS relay) are checked with `kuro_net::public_only::link_allowed`. Never send a source's URL through a plain `reqwest` client. Tests that need a local server build an unguarded `NetworkClient::new`.
- **Run both scripts before reporting a lane done:** `bash scripts/check-path-literals.sh && bash scripts/check-platform-boundary.sh .` They take about a second. A failure here turns the GitHub CI red even when every test passes.
- **GUI tests** that create GTK objects must run through `crate::testing::with_display(...)` (one GTK thread), otherwise the test binary crashes at random.

## Never hand out the same task twice (user, 2026-10-10: "pls no repeated work")

- Every new wave script must pick its tasks with `~/kurobayomi-notes/pick.py` (`from pick import pick`), which skips everything in `~/kurobayomi-notes/ledger.py`: every task of every earlier wave script, the M02 chunks, and all of milestones M00 and M01 (done before the waves). Do not write ad-hoc picking code.
- **Before every Workflow launch run `python3 ~/kurobayomi-notes/check-wave.py <script.js>`.** If it prints `CHECK-WAVE FAILED`, fix the script. The only exception is an intentional re-run of partial work: mark that group `"rerun": true` in `GROUPS`.
- Never generate a wave script over an existing wave file (it removes its tasks from the "already handed out" set).
- What went wrong before: the old picker skipped only tasks it could parse, so about 217 finished M00 and M01 tasks and about 18 others (ids like `T-req-platform-006` and two-letter suffixes like `T-49ba-003`) were handed out again, about 6% of the plan.

- **Waiting for a long test:** agents must wait with ONE command (`timeout 590 bash -c 'until grep -q EXIT= LOG; do sleep 10; done'`) and read the log. Calling the Monitor tool repeatedly in a loop creates a new background task every few seconds (86 identical ones on 2026-10-10, from one wave 53 agent) and burns tokens.

## Wave bookkeeping

- Task source: `plans/TASKS.csv`. Pick by plan area, skip ids already present in any wave script's `GROUPS`. Never regenerate over a wave script that has already run: its ids would drop out of the "taken" set and be handed out twice.
- Resume a stopped wave with `Workflow({scriptPath, resumeFromRunId})`. A "weekly limit" error from agents can be transient: resume and check whether agents write.
- Commit messages carry no Claude or Anthropic attribution (global rule).

## Later: small improvements to review (the user asked to look at these after the main waves)

Do not start these before the plan waves, the P0 audit and the first push of a green `main` are done. Pick them up in a cleanup wave.

- **Crash window Restart button** (`src/reporter.rs`): after the binary is replaced Linux reports the exe path with " (deleted)", and the relaunch silently fails. Add `kuro_platform::process::current_exe()` that strips the suffix, log the failure, add a test. Details in `~/kurobayomi-notes/FINDINGS.md`.
- **Header buttons that look pressed after a click**: `ui::no_sticky_focus` is in place; confirm it by eye on the real window with the user's Tokyonight theme.
- **Local covers** use a `file://` thumbnail drawn by GTK, not the `kuro-images` cache; move them onto the cache.
- **Local folders** with the same title are resolved by id now; check the manga and anime library menus for a clear Rescan control.
- **Open gaps from the early waves:** one-time "secrets file fallback" notice has no app caller; `InMemorySettings` in production paths; startup migration wiring; `apply_theme` caller.
- **183 open review findings** in `~/kurobayomi-notes/FINDINGS.md`: re-review the crates that were never re-reviewed, then close or fix.
- **Skipped plan tasks** that describe an SQLite key-value preference store (areas 43b, 47a, 48a): settings live in `config.toml` via `kuro-prefs`, so decide whether any of them is still needed.
- **Fixer leftovers:** performance and stability test loops written by a fixer (for example `perf_2000_*` in `src/updates`); keep only what is cheap and `#[ignore]` the rest.
- **Warnings:** about 19 unused-`Result` warnings in `kuro-domain` tests and a few dead-code warnings in the GUI.
- **CI:** the GitHub Actions run on the real runner has never been checked; add a headless-display job (xvfb) so the GTK tests run in CI instead of skipping.
- **oomd exemption** is only set on the running Claude app cgroups; make it permanent with a user systemd drop-in if the user agrees.
- **AnimePahe (user, 2026-10-12: separate job for later):** test the automatic Cloudflare window (`src/cloudflare_window.rs`) on AnimePahe and see whether the site accepts the app's requests with the cookie it collects; if the TLS fingerprint is refused, no cookie import helps. The manual cookie import and the Torrent settings page were deleted the same day; torrent streaming comes back only with a torrent source (bundled TorrServer or a Rust engine, no settings).
- **Docs:** refresh the README progress table with an audited number after the P0 audit.

## Standing instruction update (user, 2026-10-11): the P0 audit is started

The user lifted the hold on the P0 audit (read-only, no cargo, reports in docs/audit/). The cleanup wave and the "small improvements" list stay on hold until the user says so.


## Top bar rule (user, 2026-10-11)
Never hide or remove the top bar of any screen. The manga reader's top bar always stays visible; only its bottom bar may follow the menu toggle (guard test in src/reader/topbar_guard.rs).

**Every screen has a top bar with the window buttons, and the window can always be dragged by it (user, 2026-10-12: "did I mention the top bar should ALWAYS be there except the fullscreen button?").** The window's own titlebar slot is hidden, so a screen that builds no header is undraggable and has no minimize/maximize/close. Every full-page screen (wizard-like pages, dialogs pages, webview, settings, empty states, error pages) must be built with ui::page / page_header::header or an equivalent HeaderBar that carries the decoration layout. A guard test must fail if a pushed page has no HeaderBar.

The one exception (user, 2026-10-12): the fullscreen button of the anime player fullscreens the video ENTIRELY: no top bar, no window chrome, only the video (the control bar appears on mouse move and hides after a moment). Esc or the fullscreen button leaves it and the top bar is back. The manga reader has no such exception.

## Design principles (user, 2026-10-11)

1. Fewer, better defaults: a setting is either kept as a visible setting or cut completely (code, row, tests, stored key); no hiding of bloat.
2. One way to do each thing, and as much modularity as possible (shared helpers and constants; OS-specific code only in crates/kuro-platform so a Windows port stays easy).
3. Keep data compatibility: Aniyomi backups and source ids must keep loading even when a setting is cut.
4. Power-user features are decided one by one (keep or delete) with the user; never parked in a hidden corner. The only exception already agreed: the player's own mpv config file, with no UI.


## Target behavior
The app flow the user decided on 2026-10-11 is in docs/design/app-flow.md. Build and keep only what serves it.

## Lanes never build or test (user, 2026-10-11)

Agents in a workflow lane only read and write code (and write tests); they do NOT run cargo, rustc, tests, formatters or the app. The lead does ONE build, the compile fixes and the full test run at the very end. Agents therefore read the real definition of every item they use, match signatures and imports exactly, keep edits small, and end their report with a list titled UNSURE of the places they are not certain compile. Models: Haiku 5.5 high for implement and review, Sonnet 5.5 medium for fixes, never Opus. This supersedes the older "wait for a long build with one command" rules while it is in force.

## Releases: nightly and stable (user, 2026-10-14)

Follow this pattern for every release and for any workflow that publishes one.

**Stable** (tagged; the pipeline is ready in `.github/workflows/release.yml`, no tag exists yet; steps in `docs/RELEASING.md`): tag `vMAJOR.MINOR.PATCH` (semantic versioning); title "Kurobayomi X.Y.Z"; notes are that version's changelog grouped as Added, Changed, Fixed, Removed; a normal release (not a prerelease), so GitHub shows it as "Latest"; a fixed-name AppImage plus a checksum file. A stable build has no `KB_CHANNEL`. The stable AppImage updates itself from `releases/latest` (the same `src/appimage_update.rs`, track `Stable`): a higher tag version, the asset `Kurobayomi-x86_64.AppImage`, checked by GitHub's digest or `SHA256SUMS`; so keep that file name and the `SHA256SUMS` line format (`sha256sum` output).

**Nightly** (rolling, built by `.github/workflows/appimage.yml`): tag `nightly`, moved to each new build, and the one release is updated in place (files first, notes last). Title "Nightly (<8-char hash>)"; always a prerelease. The first line of the notes is exactly `Built from commit <full 40-character hash>.`: the app's self-updater (`src/appimage_update.rs`) reads the commit from it, so never change that line. Human text may follow it (a warning that nightly may be unstable and keeps separate data, then the commits since the last stable tag).

How the nightly works:
- It builds after the CI tests pass on `main` (`workflow_run`), never before. File name `Kurobayomi-nightly-x86_64.AppImage` never changes, plus a `.zsync` file next to it.
- The build gets `KB_CHANNEL=nightly` and `KB_COMMIT=<sha>`. `channel::NIGHTLY` (GTK crate) and `kuro_platform::channel::NIGHTLY` read them. A nightly build is its own app: id `io.github.anas1412.Kurobayomi.Nightly`, name "Kurobayomi Nightly", gold-frame logo, data in `kurobayomi-nightly` and `~/Kurobayomi-Nightly`. Its icons are `src/shell/data/icons-nightly`, and `build.rs` recolours the in-app logos.
- It updates itself: at start it reads the `nightly` release and, if the commit differs, replaces its own AppImage file (size, sha256 and ELF checks first). Only when `APPIMAGE` is set and the build has a commit.
- Shelly cannot read embedded update info, so it needs a one-time `shelly sync appimage Kurobayomi <download url> StaticUrl`; the URL is `https://github.com/kurobayomi/kurobayomi/releases/download/nightly/Kurobayomi-nightly-x86_64.AppImage`.
- GTK stays at 4.14 (`v4_14`). Lowering it to 4.6 gave 86 compile errors (file dialogs, launchers, accessibility API, picture content fit); older distros are a job for a Flatpak.
- `scripts/build-appimage.sh` cannot run on Arch (linuxdeploy's GTK plugin expects the Ubuntu folder layout); test it in an Ubuntu 24.04 container.

- **Windows ships one way: the Inno Setup installer** (`dist/windows/kurobayomi.iss`, built by `scripts/bundle-windows.sh` and `scripts/build-windows-installer.sh` on an MSYS2 UCRT64 runner). Files: `Kurobayomi-Setup-x86_64.exe` (stable, in `SHA256SUMS`) and `Kurobayomi-Nightly-Setup-x86_64.exe` (nightly); never rename them, the Windows self-updater looks for them. Only the stable installer registers `kurobayomi://` and `.tachibk`. Both workflows publish from a final `publish` job that waits for the AppImage and the installer, files first and notes last, so neither self-updater sees a commit whose file is not up yet.
- **Licences ship inside every AppImage** (nightly and stable): `usr/share/licenses/kurobayomi/` has LICENSE, NOTICE, SOURCE-OFFER.txt and the copyright files of the bundled system libraries; `scripts/build-appimage.sh` fails the build when it cannot trace any library to a package. The libmpv/ffmpeg GPL question is open (see `docs/RELEASING.md`).
- The stable `Release` workflow can be run by hand without a tag; it then only offers the AppImage as a run download.

## Hidden on purpose (user, 2026-10-14: "useless, hide them and note that we hid them")

These things exist in the code but the UI does not offer them. Bring one back only when the user asks.

- **Library "Sort by" menu** (`src/library/view/options.rs`, both kinds): Total episodes/chapters, Episode/Chapter fetch date, Airing time and Random (`HIDDEN_SORTS`), and the "Reverse order" check (`OFFER_REVERSE`). The sort types, their labels and any stored sort stay valid, so a sort saved earlier still applies; the menu just shows no radio for a hidden one. The episode list of an entry page keeps its own Reverse order (it is that list's main control).
