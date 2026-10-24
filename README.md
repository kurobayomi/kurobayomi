<p align="center">
  <img src="src/theme/icons/scalable/apps/kb-logo-shadow.svg" width="112" alt="Kurobayomi logo">
</p>

<h1 align="center">Kurobayomi</h1>

<p align="center">Watch anime and read manga on Linux and Windows.</p>

<p align="center">
  <a href="https://github.com/kurobayomi/kurobayomi/releases"><img src="https://img.shields.io/github/downloads/kurobayomi/kurobayomi/total?label=downloads" alt="Downloads"></a>
  <a href="https://github.com/kurobayomi/kurobayomi/releases/latest"><img src="https://img.shields.io/github/v/release/kurobayomi/kurobayomi?label=stable" alt="Stable release"></a>
  <a href="https://github.com/kurobayomi/kurobayomi/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/kurobayomi/kurobayomi/ci.yml?branch=main&label=CI" alt="CI"></a>
  <a href="https://github.com/kurobayomi/kurobayomi/releases/tag/nightly"><img src="https://img.shields.io/github/actions/workflow/status/kurobayomi/kurobayomi/appimage.yml?branch=main&label=nightly" alt="Nightly build"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/kurobayomi/kurobayomi" alt="Licence"></a>
</p>

Kurobayomi is a native desktop app, written in Rust with GTK4, that follows the model of
[Aniyomi](https://github.com/aniyomiorg/aniyomi): a library, a reader, a video player, sources you add yourself, and
progress tracking. It follows your GTK theme and draws its own icons, so it looks right on any desktop.

The name is *kurobako* (黒箱, "black box") plus *-yomi* ("reading").

![The welcome screen](docs/images/getting-started/01-welcome.png)

> **Status: first stable releases.** The whole flow from adding a repository to watching or reading works; expect some
> rough edges. The stable release is the one to use (an AppImage on Linux, an installer on Windows), and a nightly is
> built from every change. See [Install](#install).

## What it does

- **Your own manga and anime.** Put folders in `~/Kurobayomi/Local/Manga` or `~/Kurobayomi/Local/Anime`, or link a
  folder where it already is (Ctrl+Shift+O): they join the library and are read and played like anything else. Manga as
  image folders, CBZ, ZIP, CBR, RAR, CB7, CBT or EPUB; anime as MKV, MP4, WebM, AVI, MOV, FLV or WMV, with season
  folders. New files are picked up at every start and on Refresh.
- **Sources you add.** The app ships with no sources. Paste a repository link someone gave you and its sources appear
  at once; there is nothing to install. Sources are small files (text recipes or sandboxed WASM plug-ins) that are
  checked and updated every day.
- **One search for everything.** Search all your sources at once from Browse.
- **Anime player inside the app**, built on mpv: quality and server, subtitles and audio, speed, a Skip button for openings
  and endings, and a full-screen mode that shows only the video. Your own `mpv.conf` and `input.conf` are honoured.
- **Manga reader inside the app**: left to right by default, right to left or long strip per title, the next chapter
  follows the last page.
- **Library, Updates and History**: at every launch the library checks your titles for new chapters and episodes, and
  they show in Updates. Downloads for offline use.
- **Tracking with AniList and MyAnimeList** (optional): log in from Settings, then follow a title's status, progress,
  score and dates from its Tracking page. Progress moves on by itself as you read and watch.
- **Aniyomi and Tachiyomi backups** (`.tachibk`) restore into Kurobayomi. Their extension repositories do not work here:
  those extensions are Android apps.
- **Keeps itself up to date.** The AppImage and the Windows app download a newer version in the background and use it
  after a restart (not on a metered connection); About shows the progress.
- **Fits any window size**, from a full screen down to a phone-sized 360 px: the sidebar turns into a bar at the bottom.

<table>
  <tr>
    <td><img src="docs/images/getting-started/03-paste-link.png" alt="Adding a repository by link"></td>
    <td><img src="docs/images/getting-started/07-source-titles.png" alt="A source's titles"></td>
  </tr>
  <tr>
    <td><img src="docs/images/getting-started/08-search.png" alt="Search across all sources"></td>
    <td><img src="docs/images/getting-started/09-details.png" alt="A title's page with Start"></td>
  </tr>
</table>

<sub>Site names, the repository link and the covers in lists are blurred in these screenshots.</sub>

## Getting started

The step-by-step guide, with screenshots, is in **[docs/GETTING-STARTED.md](docs/GETTING-STARTED.md)**. In short:

1. Open the app and click **Add a source repository**.
2. Paste the repository link.
3. Click the search button 🔍 next to **Kurobayomi**, type a title and press **Enter**, open a title, click **Start**.

Downloads and backups go to `~/Kurobayomi` (on Windows, `Kurobayomi` in your user folder) unless you pick another folder in
Settings.

## Sources and repositories

Kurobayomi does not host, list or recommend any content, website or repository, and ships with none. A repository is a
link to an `index.min.json` file that lists sources; anyone can publish one.

**Using one**

- Add its link with **Add a source repository** on the welcome screen, or later in **Settings → Sources and tracking →
  Repositories → Add**. A `kurobayomi://add-repo?url=…` link (an "Add to Kurobayomi" button on a repository's site)
  opens the same dialog with the link filled in. You always confirm. Aniyomi and Tachiyomi repositories, and their
  "Add to Aniyomi" or "Add to Tachiyomi" buttons, do not work: their extensions are Android apps.
- Its sources start off: pick the ones you want in the list that opens, or later from the repository's **Sources**
  button. A source that is off is not downloaded at all.
- The app checks every repository at each start: fixes and new sources arrive by themselves, and a notice names new
  ones. Each source in Browse has a dot: green works, orange the site blocks the app for now, red broken.
- Removing a repository's link removes its sources; titles already in your library stay.

**Publishing one**

- The formats: [text recipes](docs/source-recipes.md) for simple HTML sites and [WASM plug-ins](docs/source-abi.md)
  for everything else. Each file in the index carries its SHA-256, and a plug-in can reach only the hosts it names.
- Give the index a `"url"` with its own address. If the repository ever moves, keep an index at the old address that
  names the new one: apps switch their saved link by themselves.
- For a button on your site, link to `kurobayomi://add-repo?url=` followed by your index's address, URL-encoded.

## Install

### Windows

Windows 10 and 11, 64-bit. Download the installer and run it: it installs for your user account, without an
administrator prompt, and adds Kurobayomi to the Start menu.

- **Stable** (recommended, from version 0.2.0): [Kurobayomi-Setup-x86_64.exe](https://github.com/kurobayomi/kurobayomi/releases/latest/download/Kurobayomi-Setup-x86_64.exe)
- **Nightly** (built from every change, may be unstable, installs beside the stable app with its own data):
  [Kurobayomi-Nightly-Setup-x86_64.exe](https://github.com/kurobayomi/kurobayomi/releases/download/nightly/Kurobayomi-Nightly-Setup-x86_64.exe)

The installer is not signed, so Windows SmartScreen may warn about it the first time: click **More info**, then **Run
anyway**. The installed app updates itself like the AppImage below, except that the new version installs when you quit
Kurobayomi (or at once with **Restart**). Uninstall it from **Settings → Apps**. Anime downloads need
[ffmpeg](https://ffmpeg.org/download.html) on your `PATH`, as on Linux.

### Linux (AppImage)

Download the AppImage, make it executable and run it. It carries its own libraries and needs glibc 2.39 or newer
(Ubuntu 24.04, Fedora 40, Debian 13, Arch and newer).

- **Stable** (recommended): [Kurobayomi-x86_64.AppImage](https://github.com/kurobayomi/kurobayomi/releases/latest/download/Kurobayomi-x86_64.AppImage)
- **Nightly** (built from every change, may be unstable, keeps its data apart from the stable app):
  [Kurobayomi-nightly-x86_64.AppImage](https://github.com/kurobayomi/kurobayomi/releases/download/nightly/Kurobayomi-nightly-x86_64.AppImage)

```bash
chmod +x Kurobayomi-x86_64.AppImage
./Kurobayomi-x86_64.AppImage
```

The AppImage updates itself: at start it looks for a newer version (stable: a higher version number; nightly: the latest
build), downloads it in the background, checks it, and runs it after a restart. It skips this on a metered connection.
**Settings → About** shows the progress, and **Check for updates at startup** there turns it off. AppImage managers
(Gear Lever, AppImageLauncher) work too.

A Flatpak is planned.

### From source

You need a recent stable Rust, GTK 4.14 or newer, and libmpv (on Arch: `pacman -S gtk4 mpv`; on Fedora:
`dnf install gtk4-devel mpv-libs-devel`; on Ubuntu 24.04: `apt install libgtk-4-dev libmpv-dev`). On Windows, build in
the [MSYS2](https://www.msys2.org/) UCRT64 shell after
`pacman -S git mingw-w64-ucrt-x86_64-{toolchain,rust,pkgconf,cmake,nasm,gtk4,mpv,dav1d,libarchive}`.

```bash
git clone https://github.com/kurobayomi/kurobayomi.git
cd kurobayomi
cargo build --release
./target/release/kurobayomi
```

## For developers

```bash
cargo run                      # run the app
cargo test --workspace         # all tests
```

Developer switches: `KUROBAYOMI_SIZE=WxH` (window size), `KUROBAYOMI_OPEN=<screen id>` (open a screen at start),
`KUROBAYOMI_SHOT=file.png` (render the window to a PNG and quit), `KUROBAYOMI_SHOT_DIR=dir` (create `dir/take` to save
every open window as a PNG; used for the guide's screenshots).

| Path | What it is |
|---|---|
| `src/` | the GTK4 app: screens, player and reader views |
| `crates/kuro-platform` | all OS-specific code (paths, secrets, opening files), with an engine for Linux and one for Windows |
| `crates/kuro-prefs`, `kuro-db`, `kuro-data`, `kuro-domain` | settings, databases, repositories, domain rules |
| `crates/kuro-extensions`, `kuro-source*`, `kuro-net` | repositories, the recipe engine, the WASM plug-in host, HTTP |
| `crates/kuro-player`, `kuro-downloads`, `kuro-images`, `kuro-local`, `kuro-archive` | mpv player, downloads, images, local files, CBZ |
| `crates/kuro-backup`, `kuro-track` | backups (Aniyomi compatible), AniList and MyAnimeList tracking |
| `docs/` | the user guide, design notes and audits |

## Licence

Apache License 2.0, see [LICENSE](LICENSE). Kurobayomi is derived from Aniyomi, Mihon and Tachiyomi, which use the same
licence; see [NOTICE](NOTICE).
