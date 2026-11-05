# Releasing

Two channels, both built by GitHub Actions as an AppImage for Linux and an installer for Windows
(`dist/windows/kurobayomi.iss`, Inno Setup). See also `CLAUDE.md` ("Releases: nightly and stable").

| | Nightly | Stable |
|---|---|---|
| Trigger | every push to `main` that passes CI (`appimage.yml`) | a tag `vX.Y.Z` (`release.yml`) |
| Release | one rolling prerelease, tag `nightly`, replaced in place | one normal release per tag, shown as "Latest" |
| File | `Kurobayomi-nightly-x86_64.AppImage` (+ `.zsync`), `Kurobayomi-Nightly-Setup-x86_64.exe` | `Kurobayomi-x86_64.AppImage` (+ `.zsync`), `Kurobayomi-Setup-x86_64.exe`, `SHA256SUMS` |
| App | `io.github.kurobayomi.Kurobayomi.Nightly`, gold icon, own data folders | `io.github.kurobayomi.Kurobayomi` |
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

## Flatpak

For now `.github/workflows/flatpak.yml` runs by hand only (Actions, "Flatpak", Run workflow): it builds and offers the
bundle as a download of the run, and publishes nothing. Once the GPG key and Pages below are set up, its tag trigger comes
back, and every version tag then also runs it. It builds `flatpak/io.github.kurobayomi.Kurobayomi.yml`
(GNOME runtime 51, libmpv and FFmpeg built in, the app with its default features and no self-updater), attaches
`Kurobayomi-x86_64.flatpak` to the release, and publishes a signed Flatpak repository on GitHub Pages. A manual run of the
workflow only builds.

- **Install:** open `https://kurobayomi.github.io/kurobayomi/io.github.kurobayomi.Kurobayomi.flatpakref` (GNOME Software
  and KDE Discover take it), or `flatpak install --from <that link>`. After that the software centre updates it from the
  repository. The bundle file on the release installs too, but does not update.
- **What the workflow writes:** the AppStream metadata, from `src/shell/data/io.github.kurobayomi.Kurobayomi.metainfo.xml.in`
  and `CHANGELOG.md` (`scripts/metainfo.sh VERSION`); `kb-build.env` with the Shikimori client from the repository secrets;
  `flatpak/cargo-sources.json` from `Cargo.lock` (flatpak-builder builds offline). None of them is committed.
- **Sandbox:** network, sound, GPU (`--device=dri`), Wayland and X11, the `~/Kurobayomi` storage folder, the desktop
  keyring (tracker logins), NetworkManager and UPower (the Wi-Fi-only and charging-only conditions). Other folders come
  through the folder picker. The app checks `kuro_platform::sandbox::is_flatpak()`: no self-update or self-made launcher,
  the crash reporter and Restart start through the Flatpak portal (`flatpak-spawn`), and the external player and the
  torrent server, which are programs of the system, are not used.
- **Needed once:** a GPG key for the repository, its private key armoured in the `FLATPAK_GPG_PRIVATE_KEY` secret; GitHub
  Pages set to "GitHub Actions" as its source, with tags allowed to deploy to the `github-pages` environment.
- **Flathub, later:** Flathub's rules forbid AI-written manifests and AI-opened submission PRs, and ask for AI-made code to be
  disclosed. A Flathub submission therefore needs a manifest written by hand (this one shows what it must build), the
  keyring moved to the Secret portal (oo7), and no Shikimori secret (a public build cannot hold one).

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
