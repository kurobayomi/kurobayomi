# Releasing

Two channels, both built by GitHub Actions as an AppImage for Linux and an installer for Windows
(`dist/windows/kurobayomi.iss`, Inno Setup). See also `CLAUDE.md` ("Releases: nightly and stable").

| | Nightly | Stable |
|---|---|---|
| Trigger | every push to `main` that passes CI (`appimage.yml`) | a tag `vX.Y.Z` (`release.yml`) |
| Release | one rolling prerelease, tag `nightly`, replaced in place | one normal release per tag, shown as "Latest" |
| File | `Kurobayomi-nightly-x86_64.AppImage` (+ `.zsync`), `Kurobayomi-Nightly-Setup-x86_64.exe` | `Kurobayomi-x86_64.AppImage` (+ `.zsync`), `Kurobayomi-Setup-x86_64.exe`, `SHA256SUMS` |
| App | `io.github.anas1412.Kurobayomi.Nightly`, gold icon, own data folders | `io.github.anas1412.Kurobayomi` |
| Updates | updates itself from the `nightly` release | the AppImage and the installed Windows copy update themselves from the latest release (`SHA256SUMS` when GitHub gives no digest); another build only shows the notice |
| Windows | installs beside stable, takes no `kurobayomi://` links or `.tachibk` files | registers `kurobayomi://` and `.tachibk` |

A Windows copy updates by running the new setup silently when the app quits (or at once with Restart): a running program
cannot replace its own files there. Both channels install per user, in `%LOCALAPPDATA%\Programs`.

## Cutting a stable release

1. Make sure `main` is what you want to ship and its CI run is green.
2. Pick the version (semantic versioning) and set it in `Cargo.toml` (`[package]` and `[workspace.package]` `version`).
3. In `CHANGELOG.md` rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD` (and start a new empty `## [Unreleased]`
   above it). The release notes are that section: the tag run fails when it is missing or empty. Check it with
   `scripts/release-notes.sh X.Y.Z`.
4. Commit and push to `main`, wait for CI to pass on that commit.
5. Tag it and push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.

The `Release` workflow then checks that the tag equals the `Cargo.toml` version, sits on `main` and has a passing CI run,
builds with `--features updater`, packs the AppImage (`scripts/build-appimage.sh`) and, on a Windows runner with MSYS2, the
installer (`scripts/bundle-windows.sh`, `scripts/build-windows-installer.sh`). Once both are built, it creates the release
with the changelog section, the commit and the SHA-256 sums of both files. A re-run for the same tag replaces the files and
notes.

## Testing the pipeline without a tag

Run the `Release` workflow by hand (Actions, "Release", Run workflow). It builds and packs the same AppImage and Windows
installer from the branch you pick and attaches them to the run as downloads (kept 7 days); nothing is published.

## Download addresses

- Stable, always the newest: `https://github.com/kurobayomi/kurobayomi/releases/latest/download/Kurobayomi-x86_64.AppImage`
- Nightly: `https://github.com/kurobayomi/kurobayomi/releases/download/nightly/Kurobayomi-nightly-x86_64.AppImage`
- Windows, stable: `https://github.com/kurobayomi/kurobayomi/releases/latest/download/Kurobayomi-Setup-x86_64.exe`
- Windows, nightly: `https://github.com/kurobayomi/kurobayomi/releases/download/nightly/Kurobayomi-Nightly-Setup-x86_64.exe`

## Licences inside the AppImage

`usr/share/licenses/kurobayomi/` holds our `LICENSE` and `NOTICE`, `SOURCE-OFFER.txt`, and in `libraries/` the copyright file
and the package name and version of every bundled system library (GTK, libmpv, ffmpeg, ...). `scripts/build-appimage.sh`
collects them from the packages of the build machine and fails the build when it finds none.

Open question, for the owner to decide before a wide release. A test build of the packaging (Ubuntu 24.04) bundled 202
libraries; the copyright files show that libmpv2 (GPL-2+ and GPL-3+ parts), libavcodec60 (LGPL with GPL parts) and libx264 and
libx265 (GPL) are among them. A GPL library shipped inside an app can bring the combined work under GPL terms, and Apache-2.0
(our licence) is not compatible with GPLv2-only code. The copyright files in `libraries/` show which licence each library
has. If this is a problem for you, the options are: an mpv/ffmpeg build without the GPL parts (LGPL only), not bundling mpv
and ffmpeg (the user then needs them installed), or releasing the whole app under a GPL-compatible licence. This is a legal
decision, not a technical one.
