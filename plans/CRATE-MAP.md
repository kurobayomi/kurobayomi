# Kotlin library to Rust crate map (Kurobayomi)

Synthesis of every area's Rust design. One decision per library, one answer per conflict, one version per crate. Read this file before adding any `[dependencies]` line.

Verification date: 2026-10-08. Versions and MSRV come from the crates.io API (`max_stable_version`, `rust_version` of that release) on that date. Licences are from crates.io metadata where the row says so; every other licence is checked by `cargo deny` (section 10) when the crate is added.

Sources read: the 67 `*rust-design.md` files under `plans/NN-*/`, `plans/addenda/requirements-cross-platform.md`, `plans/20-extension-strategy-redesign/00-overview.md` section 2.2, `plans/01-foundation-app-bootstrap/p1-topic-build-and-dependency-map.md` sections 1 to 2, the Gradle catalogs `gradle/libs.versions.toml`, `gradle/androidx.versions.toml`, `gradle/compose.versions.toml`, `gradle/kotlinx.versions.toml`, `gradle/aniyomi.versions.toml` in `/home/blackbox/aniyomi-src`, and the module build scripts `app/build.gradle.kts`, `core/archive/build.gradle.kts`, `core/common/build.gradle.kts`, `core-metadata/build.gradle.kts`, `data/build.gradle.kts`, `domain/build.gradle.kts`, `source-api/build.gradle.kts`, `source-local/build.gradle.kts`, `presentation-core/build.gradle.kts`, `presentation-widget/build.gradle.kts`, `i18n/build.gradle.kts`, `i18n-aniyomi/build.gradle.kts`, `macrobenchmark/build.gradle.kts`.

Labels used below:

- **Decided**: this file is the decision. Area docs must follow it.
- **Proposal:** a recommendation that still needs the owner's call. Each one names the owner.
- **Verify at add:** a fact that could not be settled from the docs or crates.io metadata alone. The `cargo add` step checks it (section 11).

Decision codes for Gradle entries: **R** = Rust crate (row in section 4). **G** = GTK 4, GLib or GIO facility, no new crate. **S** = `std` or a few lines of hand-written code. **X** = external program or system library. **D** = dropped, with the reason.

---

## 0. Summary of decisions

1. Stack: gtk4 0.11.5 (feature `v4_16`, no libadwaita), tokio 1.53.2, rusqlite 0.40.2 (`bundled`), reqwest 0.13.5 with rustls, serde 1.0.229 and serde_json 1.0.151, libmpv2 6.0.0 for video, Fluent (fluent-bundle 0.16.0) for strings.
2. Minimum supported Rust is **1.98**, set by `avif-decode` 3.0.0 (section 2.2). gtk4 0.11.5 alone needs 1.92.
3. Crate names use the `kuro-` prefix. `kb-` names in area:20 are renamed (section 2.1).
4. Dates use **chrono** only. `time` is not used.
5. SQLite access: one writer and one reader `tokio_rusqlite::Connection`. No r2d2, no deadpool.
6. Desktop notifications use **GIO** (`gio::Notification`). `notify-rust` is not used.
7. Strings use **Fluent** (`locales/<tag>/kurobayomi.ftl`). gettext is not used.
8. Logging is **tracing**. The `log` facade is not used directly.
9. Video: libmpv2 render API in a `GtkGLArea`. Downloads: `ffmpeg` and `ffprobe` as child processes (no ffmpeg-next). Torrents: TorrServer as an external process, deferred to P2.
10. Extension strategy follows area:20: built-in Rust sources, then TOML scrapers, then WASM components (`wasmtime` 49.0.2) in P2. JavaScript (`rquickjs`) is optional P2.

---

## 1. Conflicts resolved

Each row names the conflicting docs, the decision, and the docs that must be amended. "Amend" means the owner edits that file to match this table. Nothing else in the area docs overrides a row here.

| ID | Topic | Conflicting proposals (repo-relative) | Decision | Amend |
|---|---|---|---|---|
| C-01 | Minimum Rust version | `plans/54-build-release-distribution-docs/p1-rust-design.md` section 3 says `rust-version = "1.85"`. gtk4 0.11.5 needs 1.92 (crates.io). avif-decode 3.0.0 needs 1.98. | `rust-version = "1.98"` on the workspace. Local toolchain here is rustc 1.99.0 (Arch), so it builds. | 54 section 3 |
| C-02 | Date and time crate | `time` in `plans/01-foundation-app-bootstrap/p1-rust-design.md` section 2, `plans/05-data-repos-entries-items/p1-rust-design.md` section 2, `plans/08-domain-library-category-history-updates/p1-rust-design.md` section 2, `plans/10-app-domain-layer/p1-rust-design.md` section 4, `plans/14-source-local-and-metadata/p1-rust-design.md` section 1.1 and section 19, `plans/27-tracking-core/p1-rust-design.md` section 2 (says time), `plans/54-build-release-distribution-docs/p1-rust-design.md` section 4 (`build.rs` uses time). `chrono` in `plans/04-db-anime-schema-migrations/p1-rust-design.md` section 2, `plans/07-domain-entries-items/p1-rust-design.md` section 2, `plans/26-library-update-jobs/p1-rust-design.md` section 7, `plans/28-tracker-anilist/p1-rust-design.md` section 11, `plans/36-ui-shell-navigation-security-webview/p1-rust-design.md` section 4, `plans/38-ui-entry-details-manga/p1-rust-design.md` section 5, `plans/39-ui-entry-details-anime/p2-rust-design.md` section 4, `plans/47-settings-screens-general/p1-rust-design.md` section 7, `plans/27-tracking-core/p2-rust-design.md` section 2. | **chrono 0.4.45** (`clock`, `std`). `time`'s local-offset lookup fails on multi-threaded Linux (14 section 19 documents this), and chrono reads the OS zone directly. No `time` dependency anywhere. `build.rs` (area:01 topic section 1.3 forbids chrono there) formats the UTC timestamp by hand with std only. | 01, 05, 08, 10, 14 (delete section 19), 27 p1, 54 section 4 (`build.rs`) |
| C-03 | SQLite connection model | r2d2 in `plans/04-db-anime-schema-migrations/p1-rust-design.md` section 2 and `plans/06-data-repos-history-track-source-misc/p1-rust-design.md` section 2. tokio-rusqlite in `plans/05-data-repos-entries-items/p1-rust-design.md` section 2 and tokio-rusqlite 0.6 in `plans/47-settings-screens-general/p1-rust-design.md` section 7. One `std::thread` worker in `plans/37-ui-library/p1-rust-design.md` R-10. Writer `Mutex` with per-call WAL readers in `plans/26-library-update-jobs/p1-rust-design.md` section 3. | **tokio-rusqlite 0.8.0** (depends on rusqlite ^0.40.1). Two connections in `kuro-db`: one writer, one read-only reader, both WAL. No pool crate. Each `call(closure)` is one transaction. | 04 section 2, 06 section 2, 37 R-10, 26 section 3, 47 section 7 |
| C-04 | Desktop notifications | `notify-rust` in `plans/20-extension-strategy-redesign/00-overview.md` section 2.2 (`kb-notify`), `plans/21-backup-format-and-create/p1-rust-design.md`, `plans/24-downloads-anime/p1-rust-design.md` section 2, `plans/26-library-update-jobs/p1-rust-design.md` section 2.6, `plans/53-platform-manifest-intents-workers/p1-rust-design.md`. `gio::Notification` in `plans/35-data-misc-updater-notification-export/p1-rust-design.md` section 4 and section 13 (rejects notify-rust). | **GIO**: `gio::Notification` sent with `gtk::Application::send_notification`, withdrawn with `withdraw_notification`. Actions route to the `app.notice` GAction (35 section 4). No crate. | 20 section 2.2 (`kb-notify` becomes a module of `kuro-platform`), 21, 24 section 2, 26 section 2.6, 53 |
| C-05 | Logging facade | `log` in `plans/22-backup-restore/p1-rust-design.md` section 2, `plans/35-data-misc-updater-notification-export/p1-rust-design.md` section 12 and section 13, `plans/01-foundation-app-bootstrap/p1-topic-build-and-dependency-map.md` mentions none. `tracing` everywhere else. `eprintln!` proposed in `plans/51-i18n/p1-rust-design.md` section 6. | **tracing 0.1.44**, `tracing-subscriber` 0.3.23 (`env-filter`, `fmt`, `reload`), `tracing-appender` 0.2.5 (daily rotation). Area:35's "`log` facade" line becomes "tracing". | 22 section 2, 35 sections 12 and 13, 51 section 6 |
| C-06 | thiserror major | `thiserror` 1 in `plans/40-ui-browse-manga/p1-rust-design.md` section 8 and `plans/40-ui-browse-manga/p2-rust-design.md` section 8. Everything else says 2. | **thiserror 2.0.21** | 40 section 8 (both parts) |
| C-07 | Regex with look-around | `fancy-regex` in `plans/06-data-repos-history-track-source-misc/p1-rust-design.md` section 2 (mention rewrite) and `plans/10-app-domain-layer/p1-rust-design.md` section 4 (`SeasonRecognition`). Hand-written scanner in `plans/35-data-misc-updater-notification-export/p1-rust-design.md` section 8.5. Consuming rewrite in `plans/07-domain-entries-items/p1-rust-design.md` section 2. | **regex 1.13.1** for everything. **fancy-regex 0.19.2** only for `SeasonRecognition` (the one pattern with look-behind). The `@mention` rewrite uses the hand scanner from area:35 (no crate). | 06 section 2 (drop fancy-regex, use the 35 scanner) |
| C-08 | Seeded shuffle RNG | `rand = 0.8` in `plans/37-ui-library/p1-rust-design.md` R-15 and `plans/29-tracker-myanimelist/p1-rust-design.md` section 2. `rand` plus `rand_chacha` (`ChaCha8Rng`) in `plans/08-domain-library-category-history-updates/p1-rust-design.md` section 2 and section 8. | **rand 0.10.3** with features `std`, `std_rng`, `chacha`. One crate. `rand_chacha` is not a direct dependency. | 37 R-15, 29 section 2, 08 section 2 |
| C-09 | ICU collator | `icu_collator` 1.x in `plans/39-ui-entry-details-anime/p2-rust-design.md` section 4. 2.x in `plans/37-ui-library/p1-rust-design.md` R-15, `plans/13-core-util-storage-archive-torrent/p1-rust-design.md` section 5 (also `icu_locale_core`), `plans/40-ui-browse-manga/p2-rust-design.md` section 8. | **icu_collator 2.3.1** and **icu_locale 2.3.1** (both MSRV 1.88). `icu_locale_core` is not added directly. | 39 p2 section 4, 13 section 5 (drop icu_locale_core) |
| C-10 | Translation format | Fluent: `plans/51-i18n/p1-rust-design.md` (the whole file). gettext: `plans/54-build-release-distribution-docs/p1-rust-design.md` section 9 (`po/*.po`, `xgettext`, `gettext-rs`). `gettext-rs` in `plans/49-theme-and-design-system/p2-rust-design.md` section 7. "gettext-style catalogs" in `plans/01-foundation-app-bootstrap/p1-topic-build-and-dependency-map.md` section 2.1. | **Fluent**: `fluent-bundle` 0.16.0 (`concurrent`), `unic-langid` 0.9.6, `fluent-syntax` 0.12.0 (xtask validation only). Catalogues are `locales/<tag>/kurobayomi.ftl`. Area:51 section 8 (the moko converter) is kept. gettext-rs 0.8.0 is rejected: it needs libintl and has no named arguments (51 section 1 says the same). | 54 section 9 (rewrite around `.ftl`), 49 p2 section 7, 01 topic section 2.1 |
| C-11 | Language display names | Runtime `icu_displaynames` in `plans/40-ui-browse-manga/p1-rust-design.md` section 8, `plans/41-ui-browse-anime/p2-rust-design.md` section 5, `plans/45-player-core/p2-rust-design.md` section 2, `plans/47-settings-screens-general/p1-rust-design.md` section 7, `plans/17-source-app-layer/p1-rust-design.md` section 13 (ICU4X displaynames). Dev-only generator in `plans/51-i18n/p1-rust-design.md` section 1. `isolang` in `plans/11-app-migrations/p1-rust-design.md` and `plans/48-settings-player-and-scripts/p1-rust-design.md`. | A committed table `src/i18n/names.rs` generated once from `icu_displaynames` by `xtask`. `icu_displaynames` is a **dev-dependency of xtask only**; no runtime crate needs display names. The runtime uses `isolang` 2.4.0 for ISO 639 codes. | 40 p1 section 8, 41 p2 section 5, 45 p2 section 2, 47 section 7, 17 section 13 |
| C-12 | System language list | `sys-locale` in `plans/02-preferences-system/p1-rust-design.md`, `plans/07-domain-entries-items/p1-rust-design.md` section 2, `plans/10-app-domain-layer/p1-rust-design.md` section 4, `plans/11-app-migrations/p1-rust-design.md`. `glib::language_names()` in `plans/51-i18n/p1-rust-design.md` section 1 and section 4. | **`glib::language_names()`** (already in the GTK stack). `sys-locale` is dropped. | 02, 07, 10, 11 |
| C-13 | Insertion-ordered maps | `indexmap` in `plans/10-app-domain-layer/p1-rust-design.md` section 4 and `plans/23-downloads-manga/p1-rust-design.md` (crates list). `Vec` with a `BTreeMap` index in `plans/37-ui-library/p1-rust-design.md`. | **No indexmap.** A `Vec` plus `HashMap` index, or `BTreeMap`, as each area's code needs. indexmap 2.14.2 is listed here only as rejected. | 10 section 4, 23 crates list |
| C-14 | Cookie store | `cookie_store` and `reqwest_cookie_store` in `plans/12-network-stack/p1-rust-design.md` section 6. Area:12 also says reqwest `cookies` feature. | **cookie_store 0.22.1** (its MSRV is 1.88) plus the reqwest `cookies` feature. `reqwest_cookie_store` is dropped. **Verify at add:** `cargo tree -i cookie_store` shows one version, the one reqwest 0.13 uses. | 12 section 6 |
| C-15 | async-trait | Used in `plans/26-library-update-jobs/p1-rust-design.md` section 2.4 (`Source`, `ImageCompressor` only; section 7 says so), `plans/27-tracking-core/p1-rust-design.md` (required for `dyn Tracker`), `plans/28-tracker-anilist/p1-rust-design.md` ("only if needed"), `plans/39-ui-entry-details-anime/p2-rust-design.md` ("avoid"), `plans/40-ui-browse-manga/p1-rust-design.md` (`dyn` traits), `plans/41-ui-browse-anime/p2-rust-design.md` section 5 (`dyn AnimeSource`). | **async-trait 0.1.92**, only on traits used as `dyn` (sources, trackers, interceptors, `WebViewHost`, `ImageCompressor`, `RemoteUpdater`, `PageSource`). Generic-only traits use native `async fn`. | 26, 28, 39 p2 |
| C-16 | Hex encoding | `hex` 0.4 in `plans/19-extensions-anime/p1-rust-design.md` section 13 and `plans/35-data-misc-updater-notification-export/p1-rust-design.md` section 13. Rejected in `plans/13-core-util-storage-archive-torrent/p1-rust-design.md` section 5 ("`format!("{:02x}")` is enough"). | **No hex crate.** `format!("{:02x}")`. | 19 section 13, 35 section 13 |
| C-17 | Crate prefix and workspace member names | `kuro-*` in areas 01 to 17, 23, 25, 36, 47, 52, 53. `kb-*` in `plans/20-extension-strategy-redesign/00-overview.md` section 2.2 (`kb-source`, `kb-ext`, `kb-ext-wasm`, `kb-source-scrape`, `kb-sources-builtin`, `kb-notify`, `kb-torrent`). `kuro-sources` in 33, `kuro-source` in 09 and 15, `kuro-downloads` in 23 and `kuro-download` in 25. | **`kuro-` prefix** (section 2.1 has the final list). Renames: `kb-source` to `kuro-source`; `kb-ext` to `kuro-extensions`; `kb-ext-wasm` to `kuro-source-wasm`; `kb-source-scrape` to `kuro-source-scrape`; `kb-sources-builtin` to `kuro-sources-builtin`; `kb-notify` removed (it becomes `kuro-platform::notifier`); `kb-torrent` removed (C-18). `kuro-sources` (33) becomes `kuro-track`'s identity module. `kuro-downloads` (23) is the crate; 25 is its module `downloads::shared`. | 20 section 2.2, 33, 09, 15, 23, 25 |
| C-18 | Torrent backend | `librqbit` in `plans/20-extension-strategy-redesign/00-overview.md` (Q9, deferred to P2), `plans/16-source-api-anime/00-overview.md` and `plans/45-player-core/00-overview.md` (`p1-topic-local-servers.md`). TorrServer as an external process in `plans/13-core-util-storage-archive-torrent/p1-rust-design.md` section 5. | **TorrServer external process** (HTTP, `tokio::process` to start it, reqwest to talk to it). `librqbit` is a P2 candidate only (area:20 Q9). No crate in P0 or P1. | 16, 20 (Q9), 45, 13 |
| C-19 | JavaScript for sources | `rquickjs` (P2) in `plans/12-network-stack/p1-rust-design.md` section 6 and `plans/01-foundation-app-bootstrap/p1-rust-design.md` section 2 (the "other areas" list). Rejected as primary in `plans/20-extension-strategy-redesign/p1-topic-option-matrix.md` ("kept as optional guest behind the same WIT"). | **rquickjs 0.14.0**, Cargo feature `js-sources`, off by default, P2 only. Not in P0 or P1. | 12 section 6 |
| C-20 | Extension runtime and package signing | `wasmtime` and `ed25519-dalek` required in `plans/18-extensions-manga/p1-rust-design.md` section 15 and `plans/19-extensions-anime/p1-rust-design.md` section 13. Area:20 puts WASM in phase 2 (Q3) and defers ed25519 "until a second repo exists" (Q7). | **wasmtime 49.0.2** in P2 only (feature-gated crate `kuro-source-wasm`). **ed25519-dalek 3.0.0** for the repo-level signature in P1 (repo key trust root). Per-package ed25519 signatures are deferred (area:20 Q7). | 18 section 15, 19 section 13, 20 Q3 and Q7 |
| C-21 | Watching the extension directory | `notify` 8.2.0 (inotify, 300 ms debounce) in `plans/20-extension-strategy-redesign/00-overview.md` (`watch.rs`). "No live directory watch; the store reloads on explicit commands" in `plans/18-extensions-manga/p1-rust-design.md` section 15. | **No `notify` crate in P0 or P1.** Rescan on explicit command (area:18). `notify` 8.2.0 is an optional P2 feature if area:20 decides a live watch. | 20 `watch.rs` row, 18 section 15 |
| C-22 | Local stream HTTP server | `nanohttpd` (Android) in `plans/15-source-api-manga/p1-rust-design.md` section 2 says "not used". `axum` in `plans/16-source-api-anime/p1-edge-cases-and-tests.md` and `plans/16-source-api-anime/p1-behavior.md` (LocalStreamServer, recommended in Q4 of the overview). `tiny_http` as the lazier alternative (same file). | **axum 0.8.9** (features `tokio`, `http1`) for `LocalStreamServer` in `kuro-source`. `tiny_http` rejected: range requests and async handlers are needed (area:16 Q4). | 16 Q4 |
| C-23 | AVIF decoding | `avif-decode` (feature `avif`) in `plans/34-images-covers-cache/p1-rust-design.md` section 1.1. `image` with `dav1d` in `plans/23-downloads-manga/p1-rust-design.md`, `plans/44-reader-viewers/p1-rust-design.md` section 15 (`avif` feature), `plans/01-foundation-app-bootstrap/p1-topic-build-and-dependency-map.md` section 2.5. | **avif-decode 3.0.0** (pure Rust AV1), feature `avif` on the `kuro-images` and `kuro-reader` crates. The `image` crate's `avif` feature is **not** enabled, so `dav1d` is not needed. | 34, 44 section 15, 23 |
| C-24 | HEIF binding | `libheif-rs` 2.x in `plans/34-images-covers-cache/p1-rust-design.md` section 1.1. `libheif-rs` in `plans/13-core-util-storage-archive-torrent/p1-rust-design.md` section 5 and `plans/44-reader-viewers/p1-rust-design.md` section 15 (optional `heif`). | **libheif-rs 3.0.0**, feature `heif`, off by default, P2. Needs the Arch `libheif` package when enabled. | 34 section 1.1 |
| C-25 | `image` feature set | `jpeg, png, webp` (38 and 41), `jpeg, png, gif, webp` (13), `jpeg, png, webp, gif, bmp, tiff` (43), `jpeg, png, gif, webp, bmp` (34), `avif` (44). | **image 0.25.10**, `default-features = false`, features `jpeg`, `png`, `gif`, `webp`, `bmp`, `tiff`. AVIF comes from avif-decode (C-23). JPEG XL comes from jxl-oxide (`image` never decodes it). | 13, 34, 38, 41, 43, 44 |
| C-26 | Hash crates | `md-5` 0.10 (`plans/34-images-covers-cache/p1-rust-design.md` section 1.1, `plans/39-ui-entry-details-anime/p1-rust-design.md` section 2), `sha2` 0.10 (`plans/13-core-util-storage-archive-torrent/p1-rust-design.md` section 5, `plans/19-extensions-anime/p1-rust-design.md` section 13, `plans/35-data-misc-updater-notification-export/p1-rust-design.md` section 13), `sha1` 0.10 (13). | **md-5 0.11.0**, **sha2 0.11.0**, **sha1 0.11.0**. All three MSRV 1.85. The lib name of md-5 is `md5`. | 13, 15, 34, 39 |
| C-27 | Free disk space | `fs4` (34 section 1.1), `rustix` `statvfs` (22, 24, 13, 01), `nix` (22) | **rustix 1.1.5** (`fs`, `system`). `fs4` and `nix` are not added. | 34 section 1.1, 22 section 2 |
| C-28 | Secret storage | `keyring` 3 (27 section 2, 27 p2 section 2, 28 section 11, 29 section 2, 30 section 11, 47 section 7). `oo7` (addenda, `RISKS-AND-DECISIONS` file). | **keyring 4.2.0** with the Linux Secret Service backend. Verified: keyring 4 is a facade over `keyring-core` 1.0.0. The Linux store is the optional dependency `zbus-secret-service-keyring-store` (pure Rust over zbus). **Verify at add:** the feature name that enables it. **oo7 is rejected.** | 27, 28, 29, 30, 47 |
| C-29 | Opening URLs and files, paths | `open` crate (addenda table `Opener`, 34 section 1.1). `gio::AppInfo` (27, 35, 47). `glib::user_cache_dir` (22). `directories` (addenda rule 4, 14, 34, 35). | **Opener**: `gio::AppInfo::launch_default_for_uri` and `gio::File::for_path` (Flatpak-safe, no crate). `open` is not added. **Paths**: `directories` 6.0.0 inside `kuro-platform` only (addenda rule 4). `glib::user_cache_dir` is not used. | 22, 34 section 1.1 |
| C-30 | Video playback binding | `libmpv2` (01 topic 2.6, 45 p2, 46, 48, 20). `libmpv-sys` (46 section, 20). `ffmpeg-next` (01 topic 2.6). `FFmpegKit` through `ffmpeg` executable (24 section 2, 01 topic 2.6). | **libmpv2 6.0.0**, feature `render`. `libmpv-sys` and `ffmpeg-next` are not added. Downloads use the `ffmpeg` executable (C-31). | 45 p2 section 2, 46, 48, 01 topic 2.6 |
| C-31 | Media muxing and probing | `ffmpeg` and `ffprobe` as child processes, in `plans/24-downloads-anime/p1-rust-design.md` section 2, `plans/14-source-local-and-metadata/p1-rust-design.md`, `plans/16-source-api-anime/` (`ffprobe`). `ffmpeg-next` in `plans/01-foundation-app-bootstrap/p1-topic-build-and-dependency-map.md`. | **Child process** (`tokio::process::Command`). The binary is the contract. No crate. | 24 section 2, 14 |
| C-32 | MPRIS media controls | `souvlaki` (addenda `MediaControls`, 01 topic 2.3). `mpris-server` in the Gradle-to-Rust map row for `aniyomi.versions.toml` in `plans/catalogs/android-api-mapping-05.md` or `-06.md`. | **souvlaki 0.8.3** with `default-features = false` and feature `use_zbus`. (Default feature `use_dbus` links libdbus, which is rejected.) | addenda table, 01 topic 2.3 |
| C-33 | RAR, 7z and tar reading | `unrar` (01 topic 2.1, 43). Rejected in 13 section 5 (non-free licence). `libarchive3-sys` FFI (13 section 5). `libarchive` crate (13 section 5, "verify in spike"). | **RAR, 7z, tar, and the rest: libarchive** through a spike (T-13a-031). Candidate crate `libarchive3-sys` 0.1.2 (raw FFI, system `libarchive`). If its API does not cover `read_open_memory`, a small FFI module in `kuro-archive` replaces it. `unrar` is not used. **CBZ and EPUB: zip** (C-34). | 13 section 5, 01 topic 2.1 |
| C-34 | zip major | `zip` 2.x in 13 section 5, 18 section 15, 19 section 13, 41 section 9 | **zip 8.6.0** (MSRV 1.88), `default-features = false`, feature `deflate`. Writing uses `CompressionMethod::Stored` only (13 section 5). **Verify at add:** the 8.x read API (`ZipArchive::new`, `by_index`). | 13, 14, 18, 19, 41 |
| C-35 | Binary cache serialization | `bincode` 2.x (24 section 2, 23 crates list `bincode`). | **serde_json**, no bincode. bincode 3.0.0 is the current major (crates.io) and is not chosen. The download index file is JSON (`dl_anime_index_v3.json` in area:24's paths, with the same key layout). | 24 section 2, 23 |
| C-36 | protobuf | `prost` 0.13 (19 section 13, 47 p2 section 1). `prost` with no version (06, 21, 22). | **prost 0.14.4**, `default-features = false`, feature `derive`, no `prost-build` (06 section 2, no `protoc` at build time). | 19, 47, 06, 21, 22 |
| C-37 | HTTP client | `reqwest` 0.12 with `rustls-tls` in areas 12, 15, 18, 19, 26, 28, 29, 30, 31, 32, 33, 34, 35, 38, 39, 40, 41, 45, 47 and 54. | **reqwest 0.13.5**: the feature `rustls-tls` is now `rustls`, and the default TLS is native-tls, so defaults are turned off (section 3.4). Every area's `reqwest` line becomes the workspace line. | all of the above |
| C-38 | DNS resolution for DoH | `hickory-resolver` 0.24 (47 section 7), `hickory-proto` (12 section 6), `hickory-dns` reqwest feature (47 section 7, 01 topic 2.4). | **hickory-resolver 0.26.3**, feature `https-rustls`, used as the DoH resolver behind reqwest's custom DNS hook (area:12 section 4). `hickory-proto` is not added. **Verify at add:** the feature name and the reqwest 0.13 resolver hook. | 12 section 6, 47 section 7 |
| C-39 | `ashpd` (portals) | 0.9+ (38), 0.11 (47 section 7), "latest" (49), 0.x (36 section 4). | **ashpd 0.13.13**, features `tokio`, `gtk4`. Verified MSRV 1.87. | 36, 38, 47, 49 |
| C-40 | Image and scraper versions | `scraper` 0.20+ (`plans/13-core-util-storage-archive-torrent/p1-rust-design.md` section 5, `plans/14-source-local-and-metadata/p1-rust-design.md` section 1.1). `quick-xml` 0.37 (14 section 1.1), `quick-xml` with `serialize` (22 section 2). | **scraper 0.27.0**, **quick-xml 0.42.0** (feature `serialize` only where serde derive is used: area:22). | 13, 14, 15, 22 |
| C-41 | Webview binding | `webkit6` 0.6.x (12 section 6), `webkit6` 0.4+ (38 section 5), `webkit6` (36, 01 topic 2.5). | **webkit6 0.6.1**, default features. Needs the Arch `webkit2gtk-6.0` package. | 12, 38, 36 |
| C-42 | Crypto for PKCE and passphrases | `base64` 0.22 (27 p2, 29, 19), 0.23 (reqwest 0.13 uses 0.23). `getrandom` 0.2 (27 p2). `argon2` 0.5 (47), 0.6 (36). `secrecy`/`zeroize` (36). | **base64 0.23.1**. **getrandom** not added (rand 0.10 provides the bytes). **argon2 0.6.0**. **secrecy 0.10.3** (zeroize comes in through secrecy; no direct dependency). | 27 p2, 29, 36, 47 |
| C-43 | Structured errors at the boundary | `anyhow` in 01 (app edges) and 27 (GUI edges); 50 says it is not used. | **anyhow 1.0.104** only in the binary crate (`src/main.rs` and the GTK boundary). Library crates use `thiserror`. | 01, 27 |
| C-44 | Lazy statics | `once_cell` in `plans/27-tracking-core/p1-rust-design.md` section 2, `plans/33-tracker-enhanced-kavita-komga-suwayomi-jellyfin/p1-rust-design.md` section 2 (says not used), `plans/49-theme-and-design-system/p2-rust-design.md` section 7. | **No once_cell, no lazy_static.** `std::sync::{LazyLock, OnceLock}`. | 27, 49 p2 |
| C-45 | Native calls for process time zone | `libc::localtime_r` in `plans/10-app-domain-layer/p1-rust-design.md` section 4 (inside `kuro-platform`). | **Removed.** chrono reads the zone. `libc` is not a direct dependency of any crate. | 10 section 4 |
| C-46 | Notification icon rendering | `gdk-pixbuf` as a direct dependency (26 section 2.6, 40 section 2 and section 4). | **No direct gdk-pixbuf pin.** Use the version gtk4 0.11.5 pulls in (gdk-pixbuf 0.22.x). **Verify at add:** `cargo tree -i gdk-pixbuf` shows one version. | 26, 40 |
| C-47 | Crate for ICU datetime | `icu_datetime` in 51 section 1 and 52 (`icu_datetime`, `icu_plurals`). `chrono` `strftime` only (06, 22). | **icu_datetime 2.3.0** only in `kuro-i18n` for localized short dates and month names. `icu_plurals` is not added (Fluent does plural selection). chrono does the numeric formats. | 51, 52 |
| C-48 | LRU caches | `lru` 0.12 (`plans/34-images-covers-cache/p1-rust-design.md` section 1.1) and `lru` (`plans/44-reader-viewers/p1-rust-design.md` section 15). | **lru 0.18.5** (MSRV 1.85). One version. | 34, 44 |

---

## 2. Toolchain, workspace and crate layout

### 2.1 Workspace members (Decided)

Root package `kurobayomi` (the GTK binary) stays at the repository root and becomes a workspace member with `members = [".", "crates/*", "xtask"]`. The list of library crates follows. Owner is the area that designs the crate's contents.

| Crate | Owner area | Responsibility | Depends on (kuro crates) |
|---|---|---|---|
| `kuro-platform` | area:01, addenda | Platform traits and Linux implementation: `Paths`, `Notifier`, `Opener`, `NetworkStatus`, `SecretStore`, `MediaControls`, `WebViewHost`, `IdleInhibitor`, `SingleInstance`. The only crate that may use `zbus`, `keyring`, `souvlaki`, `webkit6`, `ashpd`, `directories` or `rustix` for system calls. | none |
| `kuro-core` | area:01 | `AppContext`, tracing setup, natural-order compare (Gradle row `natural-comparator`, section 4.1), magic-byte table (area:14 section 4.4), `LocalDate` helpers on chrono, safe-name function (addenda rule 5). | kuro-platform |
| `kuro-i18n` | area:51 | Fluent catalogue, `tr`, `trf`, `tr_plural`, locale resolution, generated `MsgKey` and `LOCALES`, names table. | none |
| `kuro-prefs` | area:02 | Typed preferences over `config.toml` (`toml_edit`) and `state.toml`. Keys from area:02. | kuro-platform |
| `kuro-db` | area:03, area:04 | Two `tokio_rusqlite::Connection`s (writer, reader), migrations by `PRAGMA user_version` (no migration crate), schema SQL under `crates/kuro-db/sql/`, change hook bus. | kuro-platform |
| `kuro-data` | area:05, area:06 | Repositories (entries, items, history, tracks, sources, misc) over `kuro-db`. | kuro-db |
| `kuro-domain` | area:07, area:08, area:09, area:10 | Entries, items, category, library, history, updates, source and track domain logic, download-state model, release model. | kuro-data, kuro-prefs, kuro-net |
| `kuro-net` | area:12 | reqwest client, interceptors, cookies, DoH resolver, rate limiter, Cloudflare handling via `WebViewHost`, HTTP cache (P2). | kuro-platform |
| `kuro-archive` | area:13 | CBZ and EPUB (zip), libarchive for other formats, memmap2 reading, image sniffing. | kuro-core |
| `kuro-local` | area:14 | Local source (folders, archives, ComicInfo, EPUB metadata, OPF, video thumbnails through ffmpeg). | kuro-archive, kuro-source |
| `kuro-source` | area:15, area:16, area:17 | Source traits (`MangaSource`, `AnimeSource`), models, filter list, id derivation (MD5), `SourceManager`, preferences schema, `LocalStreamServer` (axum). | kuro-net, kuro-prefs |
| `kuro-sources-builtin` | area:20 | Compiled-in Rust sources (none in P0). | kuro-source |
| `kuro-source-scrape` | area:20 | TOML scraper definitions (scraper) and evaluator. P1. | kuro-source |
| `kuro-source-wasm` | area:18, area:20 | wasmtime host, WIT world, limits. P2. Feature-gated. | kuro-source |
| `kuro-extensions` | area:18, area:19, area:20 | Registry, loader, manifest parse, installer (download, sha256, atomic rename), repo index parse, trust (repo key via ed25519-dalek, P1). | kuro-source, kuro-net, kuro-platform |
| `kuro-downloads` | area:23, area:24, area:25 | Manga and anime download queue, scheduler, page download, CBZ packing, shared delete/index. | kuro-source, kuro-data, kuro-archive, kuro-platform |
| `kuro-track` | area:27 to area:33 | Tracker trait, OAuth/PKCE, AniList, MyAnimeList, Kitsu, Shikimori, Bangumi, MangaUpdates, Simkl, Kavita, Komga, Suwayomi, Jellyfin, source identity table. | kuro-net, kuro-data, kuro-platform |
| `kuro-images` | area:34 | Covers, image decode (image, avif-decode, jxl-oxide, libheif-rs behind a feature), disk and memory caches, cover file layout. | kuro-net, kuro-platform |
| `kuro-backup` | area:21, area:22 | Backup `.tachibk` create and restore (prost, flate2), restore planner, dialogs' model. | kuro-data, kuro-prefs, kuro-downloads |
| `kuro-player` | area:45, area:46 | libmpv2 wrapper, playlist model, AniSkip client, player state machine, MPRIS through kuro-platform. | kuro-domain, kuro-net, kuro-platform |
| `kuro-testkit` | area:01 | Test fixtures, in-memory `Db`, wiremock helpers, fake sources. dev-dependency only. | all of the above (dev) |
| `xtask` | area:54, area:51 | Build and release commands, `.ftl` import (moko XML through quick-xml), `.ftl` validation (fluent-syntax), names-table generator (icu_displaynames), cargo-about invocation. | none |

GTK code (widgets, windows, `src/`) stays in the root package `kurobayomi` (area:01 section 1; area:36 section 1). Library crates have no GTK types. Only `kurobayomi` and `kuro-platform` (for the Linux implementation) link `gtk4`.

### 2.2 Workspace Cargo.toml

Decided. The root `Cargo.toml` is:

```toml
[workspace]
resolver = "3"
members = [".", "crates/*", "xtask"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.98"            # C-01: set by avif-decode 3.0.0
license = "Apache-2.0"

[workspace.lints.rust]
unsafe_code = "forbid"
unused_must_use = "deny"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
unwrap_used = "warn"
```

`unsafe_code = "forbid"` is the default for every crate. The one exception is a crate that wraps libarchive FFI in `kuro-archive` if the spike (C-33) needs its own bindings; that crate sets `unsafe_code = "deny"` and documents each block.

Edition 2024 requires `unsafe` for `std::env::set_var`. The design never calls it (area:51 section 7).

### 2.3 Dependency sharing rule

Each crate's `Cargo.toml` writes `name = { workspace = true }` for every dependency listed in section 8. A crate that needs a feature beyond the workspace line adds `features = [...]` to its own line; it does not change the version.

---

## 3. Cross-area decision table (crates)

Columns: version is the crates.io max stable on 2026-10-08. MSRV is that release's `rust_version`. "none" means crates.io does not publish one. Licence is the crates.io value where it was read; `(verify)` marks a licence that `cargo deny` must confirm.

### 3.1 Async runtime, concurrency and bridging

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| tokio | 1.53.2 | 1.71 (verify) | MIT | `rt-multi-thread`, `macros`, `sync`, `time`, `fs`, `process`, `io-util` | all async areas (01, 05, 12, 23, 24, 26, 27, 28 to 34, 45) | The async runtime. One multi-thread runtime created in `main`. `spawn_blocking` for CPU-heavy work (image decode, sort of big libraries). `process` for ffmpeg and TorrServer. Test runs add the dev feature `test-util`. | async-std, smol |
| tokio-util | 0.7.19 | 1.71 (verify) | MIT | `sync` (CancellationToken) | 26, 35, 23, 24, 34, 41, 43, 45 | `CancellationToken` per job, per download and per loader. | hand-written flags |
| tokio-stream | 0.1.19 | 1.71 (verify) | MIT | default | 10 (`WatchStream`), 05 | Turns `watch::Receiver` into a `Stream` where a Kotlin `Flow` is consumed as a stream. | `futures::stream` on its own (cannot read `watch`) |
| futures | 0.3.34 | 1.71 (verify) | MIT OR Apache-2.0 | default | 06, 09, 19, 23, 26, 33, 40, 41, 50 | `join_all`, `select`, `BoxFuture`, `StreamExt::buffer_unordered`. One crate; `futures-util` is not added. | futures-util separately |
| async-channel | 2.5.0 | 1.63 (verify) | Apache-2.0 OR MIT | default | 01, 18, 19, 20, 22, 23, 24, 25, 26, 27, 28, 30, 34, 35, 38, 39, 40, 41, 45, 46, 47 | The bridge from tokio tasks to the GLib main context. Works without a runtime on the receiving side; `glib::MainContext::channel` is deprecated (area:39 section 4). | `glib::MainContext::channel`, `std::sync::mpsc` (blocks the UI) |
| async-trait | 0.1.92 | 1.71 (verify) | MIT OR Apache-2.0 | default | 06, 09, 10, 12, 15, 18, 27, 28, 31, 32, 40, 41 | Object-safe `async fn` for `dyn` traits only (C-15). | hand-written `Pin<Box<dyn Future>>` everywhere (more code, same cost) |
| glib | 0.22.10 | 1.92 (matches gtk4) | MIT | via gtk4 | UI | The GLib main loop helpers: `glib::spawn_future_local`, `glib::timeout_add_local`, `glib::MainContext::default().invoke`. Use the version gtk4 0.11.5 pulls in (`glib` 0.22.x); pin only through `gtk4`. | none |
| glib-build-tools | 0.22.8 | 1.92 | MIT | build-dep | 49 section 7, 49b | Compiles `assets/icons.gresource.xml` in `build.rs` (GResource icons). | `include_bytes!` per icon (more code) |

### 3.2 GTK and GNOME stack

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| gtk4 (package name) | 0.11.5 | 1.92 | MIT | `v4_16` | all UI areas | Required toolkit. Dependency name in the root crate is `gtk` (as in the mockup): `gtk = { package = "gtk4", version = "0.11.5", features = ["v4_16"] }`. Gives `gtk::FileDialog`, `gtk::AlertDialog`, `gtk::UriLauncher`, `gtk::GridView`, `gtk::ListView`, `gtk::Revealer`, `gtk::Overlay`, `gtk::GestureZoom` (GTK 4.16 APIs). | libadwaita (`adw`) was forbidden by the brief until 2026-11-01, now used (plans/LIBADWAITA-MIGRATION.md); relm4 (second component model, area:37/38/40 rejected) |
| gio, gdk4, gdk-pixbuf, pango, cairo, graphene, gsk4 | 0.22.x / 0.11.x | 1.92 | MIT | via gtk4 | UI | Re-exported through gtk4 or pulled by it. Never pinned directly (C-46). | direct pins |

### 3.3 Storage

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| rusqlite | 0.40.2 | none | MIT | `bundled`, `functions` (Unicode fold function `kuro_fold`), `hooks` (update, commit, rollback hooks), `backup` (backup in area:04 T-04a-031), `limits` | 03, 04, 05, 06, 22, 37, 47, 39 | SQLite binding. `bundled` pins SQLite independent of the distro (areas 03, 05). The bundled SQLite must be at least 3.45 (the Aniyomi `sqlite-android` version, `gradle/libs.versions.toml:39`). **Verify at add:** `libsqlite3-sys` version in `cargo tree`. | system `libsqlite3` (distro-dependent); diesel, sqlx, sea-query (no ORM; SQL is the contract, area:04 section 2) |
| tokio-rusqlite | 0.8.0 | none | MIT | default | 05, 06, 04, 47 | One thread per connection, async `call(closure)`. Two instances in `kuro-db` (C-03). | r2d2 + r2d2_sqlite 0.35 (synchronous pool, needs `spawn_blocking` at every call site); deadpool-sqlite 0.14 (MSRV 1.95, needs `interact`) |
| serde | 1.0.229 | 1.56 | MIT OR Apache-2.0 | `derive` | all | DTOs, prefs enums, JSON models. | none |
| serde_json | 1.0.151 | 1.71 | MIT OR Apache-2.0 | `preserve_order` | 04, 05, 06, 07, 19, 21, 22, 23, 24 | JSON. `preserve_order` for the `memo` object (area:05 section 2). Replaces kotlinx `JsonObject`. Equality of memo objects uses `serde_json::Value` (area:10 section 4). | none |
| toml_edit | 0.25.16 | 1.85 | MIT OR Apache-2.0 | default | 02 | Round-trip editing of `config.toml` so unknown keys, comments and ordering survive (area:02 section 3). | toml (no round trip; area:02 needs it), preferences as JSON only |
| toml | 1.1.7 | 1.85 | MIT OR Apache-2.0 | default | 01, 02 (only for read-only tables such as `spec`) | Read-only TOML where round trip is not needed (extension manifests, area:20). | none |
| tempfile | 3.27.0 | 1.63 | MIT OR Apache-2.0 | default | 13, 14, 15, 25, 06 dev | Temp files for atomic writes (`NamedTempFile::persist`) and for archive extraction. `persist` is the atomic-rename step in area:02 and area:23. | hand-written temp names |
| memmap2 | 0.9.11 | 1.65 | MIT OR Apache-2.0 | default | 13, 14 | Read-only memory maps for archive scanning. | std::fs reads (larger memory use) |
| walkdir | 2.5.0 | none | MIT OR Unlicense | default | 13, 14, 25 | Directory walk with `follow_links(false)` (area:13 section 5). | std::fs::read_dir recursion (more code) |
| lru | 0.18.5 | 1.85 | MIT | default | 34, 43, 44 | Recency order for decoded images and the memory cache (C-48). | moka (concurrent cache not needed), hand-written list |
| rust_decimal | 1.43.0 | 1.67.1 | MIT | default | 34 section 11, 38 section 5 | HALF_EVEN rounding for chapter number display (area:34 section 11). | f64 formatting (wrong rounding) |

### 3.4 Networking

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| reqwest | 0.13.5 | 1.85 | MIT OR Apache-2.0 | `default-features = false`; features `rustls`, `json`, `stream`, `gzip`, `brotli`, `zstd`, `cookies`, `form`, `query`, `hickory-dns` | 12, 15, 18, 19, 26, 28, 29, 30, 31, 32, 33, 34, 35, 38, 39, 40, 41, 45, 47, 54 | The only HTTP client (C-37). `rustls` gives no OpenSSL link. Form bodies use `form`. | reqwest 0.12 (every area's line, superseded); ureq 3.4.2 (sync, no streaming cookies story here); native-tls (OpenSSL) |
| http | 1.x | none | MIT OR Apache-2.0 | default | 12 | Header and status types. Comes through reqwest. Import through `reqwest::header` where possible; add `http` only for `HeaderMap` in `kuro-net`. | none |
| bytes | 1.x | none | MIT | default | 12, 15, 34, 13 | Body buffers (`Bytes`) replacing okio. | `Vec<u8>` copies |
| url | 2.5.8 | 1.63 | MIT OR Apache-2.0 | default | 03, 06, 11, 12, 14, 15, 26, 27, 28, 29, 31, 33, 35, 36, 40, 41, 53 | URL parsing and building. `url::form_urlencoded::byte_serialize` replaces `URLEncoder.encode` (C-37 notes; area:12 section 2.9). `path_segments_mut` replaces `removePathSegment` (area:33). | none |
| hickory-resolver | 0.26.3 | 1.88 | MIT OR Apache-2.0 | `https-rustls` | 12 | DoH resolver behind reqwest's DNS hook (C-38). | hickory-proto (wire only, more code), manual DoH |
| psl | 2.1.241 | none | MIT OR Apache-2.0 (verify) | default | 12 | Public suffix check for the DoH private-host rule (area:12 section 6). | hand-maintained list |
| cookie_store | 0.22.1 | 1.88.0 | MIT OR Apache-2.0 | default | 12 | Cookie persistence as JSON (C-14). **Verify at add:** same version as reqwest's `cookies` feature. | reqwest_cookie_store (dropped, C-14) |
| http-cache-semantics | 3.0.0 | 1.85 | BSD-2-Clause (verify) | default | 12 (P2) | RFC 9111 freshness for the HTTP cache. P2 only. | hand-written freshness rules |
| cacache | 13.1.0 | none | Apache-2.0 (verify) | default | 12 (P2) | Content-addressed disk cache for HTTP responses. P2 only. | the image cache in area:34 (mtime LRU) |
| axum | 0.8.9 | 1.80 | MIT | `tokio`, `http1` | 16 | `LocalStreamServer` on `127.0.0.1:0`, with Range requests (C-22). | tiny_http (blocking, no async handler); hyper directly (more code) |
| encoding_rs | 0.8.42 | 1.88 | (Apache-2.0 OR MIT) AND BSD-3-Clause (verify) | default | 15 | Charset decoding of HTTP text bodies (`HttpResponse::text()`). | none |

### 3.5 Serialization, parsing and text

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| prost | 0.14.4 | 1.85 | Apache-2.0 | `derive` (no `prost-build`) | 06, 19, 21, 22, 47 p2 | Protobuf for the backup format and the extension store (C-36). Tag numbers are on the Rust structs. | protobuf codegen (needs protoc) |
| quick-xml | 0.42.0 | 1.86 | MIT | `serialize` only where serde derive is used | 13, 14, 22, 51 (xtask) | ComicInfo.xml write and read (area:14 section 3, area:22). | xmlutil (Kotlin), roxmltree (read only) |
| scraper | 0.27.0 | none | ISC (verify) | default | 13, 14, 15, 16, 20 | Tolerant HTML parse for EPUB pages, source HTML, and TOML scrapers (replaces jsoup, C-40). | html5ever directly (more code); select (unmaintained for this) |
| ammonia | 4.2.1 | 1.85 | MIT OR Apache-2.0 | default | 28, 32 | Strips all tags from tracker HTML (area:28 section 11 and area:32 section 2). | regex strip (wrong for nested markup) |
| html-escape | 0.2.15 | 1.58 | MIT OR Apache-2.0 | default | 28, 32 | Decodes HTML entities (`htmlDecode`, area:32 B-2.14). | hand decode table |
| pulldown-cmark | 0.13.4 | 1.71.1 | MIT | default-features off | 47b section 11, 49, 01 topic 2.5 | Markdown to `gtk::TextBuffer` tags for changelogs and descriptions (replaces richtext). | comrak (larger), ammonia on Markdown (wrong layer) |
| regex | 1.13.1 | 1.65 | MIT OR Apache-2.0 | default | 07, 10, 16, 40, 46, 48 | Linear-time patterns (C-07). | fancy-regex as default (slower, look-around not needed here) |
| fancy-regex | 0.19.2 | 1.66 | MIT | default | 10 (SeasonRecognition only) | The one pattern with look-behind (C-07). | none |
| isolang | 2.4.0 | none | MIT OR Apache-2.0 (verify) | default | 11, 48 | ISO 639-1 and 639-3 code tables (`from_639_1`, `from_639_3`). | icu_displaynames at runtime (C-11) |

### 3.6 Dates, numbers and locales

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| chrono | 0.4.45 | 1.62.0 | MIT OR Apache-2.0 | `clock`, `std` | 02, 04, 07, 26, 27, 28, 29, 30, 34, 36, 38, 39, 42, 46, 47, 48, 50, 52 | The only date crate (C-02). Local start-of-day for release windows (area:26 section 2.3), `NaiveDate` arithmetic for fetch intervals, `strftime` for date formats. | time 0.3.55 (local-offset fails on multi-threaded Linux, C-02), std SystemTime alone (no calendar math) |
| icu_collator | 2.3.1 | 1.88 | Unicode-3.0 (verify) | default (compiled data) | 07, 10, 13, 37, 39, 40 | PRIMARY-strength collation for library and source sorts (C-09). `CollatorOptions { strength: Some(Strength::Primary) }`. | natord (ASCII-only rules; area:13 section 5), sort by lowercase (wrong for accents) |
| icu_locale | 2.3.1 | 1.88 | Unicode-3.0 (verify) | default | 37, 40, 52 | Locale parsing for the collator (C-09). | unic-langid alone for collation |
| icu_datetime | 2.3.0 | 1.88 | Unicode-3.0 (verify) | default | 51, 52 | Localized short dates and month names only (C-47). | chrono strftime (no localized month names) |
| icu_displaynames | 0.11.2 | 1.67 | Unicode-3.0 (verify) | dev only (xtask) | 51 section 1 | Generates `src/i18n/names.rs` once. Dev-only (C-11). **Verify at add:** version compatibility with icu_locale 2.3.1; if they conflict, the committed table is hand-written from the ISO 639 list. | runtime display names (not needed) |
| unic-langid | 0.9.6 | none | MIT OR Apache-2.0 (verify) | default | 51 | Language identifiers for Fluent (`LanguageIdentifier`). | icu_locale for Fluent (Fluent is built on unic-langid) |
| fluent-bundle | 0.16.0 | 1.67.0 | Apache-2.0 (verify) | `concurrent` | 51 | Message formatting (named args, CLDR plurals) (C-10). Bundles are `Send + Sync` through `concurrent`, so `OnceLock<Catalog>` works. | gettext-rs 0.8.0 (libintl, no named args), rust-i18n (weaker plurals) |
| fluent-syntax | 0.12.0 | 1.64.0 | Apache-2.0 (verify) | default | 51 (xtask only) | Validates generated `.ftl` at import (51 section 8 step 3). | regex check (not a parser) |
| sys-locale | (none) | | | | | Dropped (C-12). | sys-locale 0.3.2 |
| rand | 0.10.3 | 1.85 | MIT OR Apache-2.0 | `std`, `std_rng`, `chacha` | 08, 11, 27, 29, 37, 52 | Seeded shuffle (`ChaCha8Rng` family, area:08 section 9 and area:37 R-5) and secure random for PKCE and state strings (area:27 section 2, area:29 section 2). **Verify at add:** the 0.10 names `SeedableRng::seed_from_u64` and the OS RNG source (`sys_rng` feature is in the manifest; the `std_rng` feature gives `StdRng`). | rand 0.8 (superseded, C-08); getrandom alone (no seeded RNG) |

### 3.7 Images, archives and media

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| image | 0.25.10 | 1.88.0 | MIT OR Apache-2.0 | `default-features = false`; `jpeg`, `png`, `gif`, `webp`, `bmp`, `tiff` | 13, 34, 38, 41, 43, 44, 23 | Decode, resize and encode of raster images; GIF and WebP frames for animation (C-25). `ImageReader::with_guessed_format` gives the format (area:23 section 9). | gdk-pixbuf loaders only (cannot decode everything Aniyomi supports), zune-jpeg (P2 speed-up only) |
| avif-decode | 3.0.0 | 1.98 | verify | `avif` (feature of kuro-images and kuro-reader) | 34, 44, 23 | Pure Rust AVIF decode (C-23). Sets the workspace MSRV. | image's `avif` feature with dav1d (system library), libavif C binding |
| jxl-oxide | 0.12.6 | none | MIT OR Apache-2.0 (verify) | default | 34, 44, 13 | Pure Rust JPEG XL decode (area:13 section 5, area:44 section 15). | libjxl C binding |
| libheif-rs | 3.0.0 | 1.82.0 | MIT OR Apache-2.0 (verify) | feature `heif`, off by default | 34, 44, 13 | HEIF containers (C-24). Needs the `libheif` system library when on. P2. | none |
| lcms2 | 6.2.0 | none | verify | feature `icc`, off by default | 34 section 9 | ICC display profile (area:34 deviation D-3). P2. | none |
| zip | 8.6.0 | 1.88 | MIT (verify) | `default-features = false`, `deflate` | 13, 18, 19, 41 | CBZ and EPUB reading, `.kuroext` package reading (area:18 section 15). Writing uses Stored only (area:13). | zip 2.x (superseded, C-34) |
| flate2 | 1.1.10 | 1.67.0 | MIT OR Apache-2.0 | default | 06, 09, 13, 19, 21, 22, 24, 47 p2 | gzip for index bodies and `.tachibk` (area:21, area:22). Gzip sniffing by magic bytes (area:06). | none |
| libarchive3-sys | 0.1.2 | none | verify | system `libarchive` via pkg-config | 13 | RAR, 7z, tar and other formats through libarchive (C-33). Spike task T-13a-031 decides whether the crate's API covers `read_open_memory`; if not, `kuro-archive` carries its own FFI. | unrar (non-free, C-33), the `libarchive` crate (13 section 5 option, unverified) |
| ttf-parser | 0.25.1 | 1.63.0 | MIT OR Apache-2.0 | default | 48 | Font family name for subtitle styling (replaces truetypeparser-light). | fontdb (larger) |
| infer | (none) | | | | | Dropped: a 20-line magic-byte table in `kuro-core` (area:14 section 4.4). | infer 0.22.0 |
| md-5 | 0.11.0 | 1.85 | MIT OR Apache-2.0 | default | 15, 16, 33, 34, 39 p1, 13 | MD5 for source ids and cache file names. The lib name is `md5` (C-26). | md-5 0.10 (superseded) |
| sha2 | 0.11.0 | 1.85 | MIT OR Apache-2.0 | default | 13, 19, 27, 35 | SHA-256 for downloads, extension checksums and PKCE S256 (C-26). | sha256 crates |
| sha1 | 0.11.0 | 1.85 | MIT OR Apache-2.0 | default | 13 | SHA-1 where the archive format needs it (area:13 section 5). | none |
| semver | 1.0.28 | 1.68 | MIT OR Apache-2.0 | default | 35, 09 | App update version rule (area:35 section 8.2) and extension version rule (area:09). | hand-written compare (wrong for pre-release tags) |
| base64 | 0.23.1 | 1.71.0 | MIT OR Apache-2.0 | default | 27, 29, 19 | URL-safe no-pad PKCE and extension signatures (C-42). | 0.22 (superseded) |
| hex | (none) | | | | | Dropped (C-16). | hex 0.4.3 |

### 3.8 Platform and OS

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| directories | 6.0.0 | none | MIT OR Apache-2.0 | default | 01, 34, 35, 47 (all through the `Paths` trait) | XDG directories inside `kuro-platform::linux::paths` only (C-29). `ProjectDirs::from("", "", "kurobayomi")`. | glib::user_cache_dir (C-29), hard-coded `$HOME` paths (addenda rule 4) |
| rustix | 1.1.5 | 1.65 | MIT OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception (verify) | `fs`, `system` | 01, 13, 22, 24, 47 p2, 34 | `statvfs` for free space (area:34 section 6.2), `uname` (area:01). No `libc` calls in our code. | nix (C-27), libc direct (unsafe), fs4 (C-27) |
| zbus | 5.19.0 | 1.87 | MIT | default | 26 (NetworkManager, UPower), 35 (FileManager1), 36 (fprintd), 47 (UPower, NM, fprintd), 45 (MPRIS through souvlaki) | Pure Rust D-Bus client. Lives in `kuro-platform` only (addenda rule 1); the GTK side never calls it. | libdbus (C library, `dbus` crate), GIO's GDBusConnection (works but is GLib-callback-shaped) |
| ashpd | 0.13.13 | 1.87 | MIT | `tokio`, `gtk4` | 36, 38, 47, 49, 52, 53 | Portals: FileChooser where gtk::FileDialog is not enough, OpenURI, Settings (`color-scheme`, `accent-color`), Background (library updates), Notification fallback. Inside `kuro-platform` (addenda). | GTK-only dialogs for portals (no sandbox story) |
| keyring | 4.2.0 | 1.88.0 | MIT OR Apache-2.0 | Linux Secret Service backend (C-28) | 02, 27, 28, 29, 30, 47 | Secret Service on KDE (KWallet bridge) and GNOME. `SecretStore` in `kuro-platform`. **Verify at add:** the feature enabling `zbus-secret-service-keyring-store`. | keyring 3 (superseded), oo7 (C-28), plain text in preferences (area:27 security rule) |
| souvlaki | 0.8.3 | 1.67 | MIT | `default-features = false`, `use_zbus` | 45, 46 | MPRIS media keys and now-playing (addenda `MediaControls`). | mpris-server (not in the plans' decision), `use_dbus` feature (libdbus) |
| notify-rust | 4.18.2 | 1.89.0 | MIT OR Apache-2.0 | not added | (none) | Not used (C-04). Its zbus backend is the default but its action callbacks need a thread per notification. | GIO is used |
| open | not added | | | not added | (none) | Not used (C-29). `gio::AppInfo::launch_default_for_uri` covers URLs and files, and routes through the portal in Flatpak. | |
| webkit6 | 0.6.1 | none | MIT | default | 12 (WebView), 36 (WebViewHost), 38 (login and Cloudflare) | WebKitGTK 6.0 for Cloudflare and tracker login (C-41). Behind `WebViewHost`; the browser fallback is the default path (addenda table). Needs Arch `webkit2gtk-6.0`. | webview2 (Windows, later port) |
| libmpv2 | 6.0.0 | none | LGPL-2.1 | `render` | 45, 46, 48 | libmpv client API and the render API into a `GtkGLArea` (addenda, C-30). `build_libmpv` feature is not enabled: the system `libmpv` is used. | libmpv-sys (lower level), mpv CLI (no embedding) |
| (ffmpeg) | external executable | | | | 24, 14, 16 | Child process `ffmpeg`, `ffprobe` through `tokio::process` (C-31). | ffmpeg-next (C-30), FFmpegKit |
| (TorrServer) | external executable | | | | 13, 45 | External process over HTTP (C-18). P2. | librqbit (P2 option only) |
| glib::language_names | (via glib) | | | | 51, 02 | Language preference list (C-12). | sys-locale (dropped) |

### 3.9 Trackers, authentication and secrets

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| secrecy | 0.10.3 | 1.60 | MIT OR Apache-2.0 | default | 36 | Wraps the app-lock passphrase so `Debug` cannot print it (area:36 section 4). Tracker tokens use it too (decision: C-42). | plain `String` (leak risk) |
| argon2 | 0.6.0 | 1.85 | MIT OR Apache-2.0 | default | 36, 47 | App-lock passphrase hash in PHC form (area:36 section 4, area:47 decision 9). | sha256 hash of passphrase (no work factor) |

### 3.10 Extensions, safety and signatures

| Crate | Version | MSRV | Licence | Features | Areas | Decision and reason | Rejected |
|---|---|---|---|---|---|---|---|
| wasmtime | 49.0.2 | 1.96.0 | Apache-2.0 WITH LLVM-exception | `default-features = false`; `runtime`, `component-model`, `cranelift`, `std` | 18, 19, 20 (P2) | WASM component host (area:20 section 4). Feature-gated behind `kuro-source-wasm`. | Lua, JS as primary (area:20 option matrix), dlopen native plugins (area:18 section 15) |
| ed25519-dalek | 3.0.0 | 1.85 | BSD-3-Clause (verify) | default (`verify_strict` is used) | 18, 19, 20 | Repo-level signature check in P1 (C-20). | per-package signatures in P1 (area:20 Q7 defers them) |
| rquickjs | 0.14.0 | 1.87 | MIT | feature `js-sources` (off) | 12, 20 (P2) | Optional JS guest for source compatibility (C-19). | |
| cargo-about | 0.9.2 | 1.88.0 | MIT (verify) | build tool, not a dependency | 47b section 11 | Licence list JSON for the About page, embedded with `include_str!` (area:47 T-47a-096). | aboutLibraries (Android) |
| notify | 8.2.0 | 1.77 | CC0-1.0 (verify) | not added in P0 or P1 | 20 (P2 option) | Live extension directory watch, P2 only (C-21). | rescan on command (chosen) |

### 3.11 Testing (dev-dependencies only)

| Crate | Version | MSRV | Licence | Areas | Decision |
|---|---|---|---|---|---|
| proptest | 1.11.0 | 1.85 | MIT OR Apache-2.0 | 13 (natural order, bencode), 44 (geometry) | Property tests for the parsers and the natural sort only (area:13 section 5, area:44 section 15). |
| wiremock | 0.6.5 | none | MIT OR Apache-2.0 | 15, 30, 31 | HTTP fixtures. Area:34 uses hyper 1 as its fixture server (`plans/34-images-covers-cache/p1-rust-design.md` section 1.1). |
| tempfile | 3.27.0 | 1.63 | MIT OR Apache-2.0 | 06, 13, 14 | Temp databases and files. |
| tokio test-util | (feature of tokio) | | | 06, 26, 27 | `start_paused = true` for interval tests (area:01 topic 2.6, area:06 section 2). |
| insta | 1.49.0 | 1.66.0 | Apache-2.0 (verify) | 44 (optional) | Not added. Plain `assert_eq!` (area:44 section 15 says optional, and the ladder says no). |
| pretty_assertions | (none) | | | 28 | Not added; `assert_eq!` diff output is enough. |

### 3.12 Build tools and CI helpers

| Tool | Version | Purpose | Areas |
|---|---|---|---|
| cargo-deny | (CLI, install with `cargo install cargo-deny --locked`) | Licence, advisory, source and ban checks (section 10) | 54 section 13 |
| cargo-about | 0.9.2 (see 3.10) | Licence list | 47b |
| hyperfine | (system package `hyperfine`) | Startup benchmark (replaces macrobenchmark, area:54 section 10) | 54 |
| flatpak-cargo-generator.py | (Flatpak tools) | Vendored sources for Flatpak builds (area:54 section 7) | 54 |
| flatpak-builder-lint | (Flatpak tools) | Manifest and AppStream lint (area:54 section 6) | 54 |
| appstreamcli | (system package `appstream`) | Metainfo validation (area:54 section 6) | 54 |

---

## 4. Gradle dependency decision table

Every entry in the five catalogs is listed with its file and line number. Module-level declarations (`app/build.gradle.kts`, `core/archive/build.gradle.kts`, `core/common/build.gradle.kts`, `core-metadata/build.gradle.kts`, `data/build.gradle.kts`, `domain/build.gradle.kts`, `source-api/build.gradle.kts`, `source-local/build.gradle.kts`, `presentation-core/build.gradle.kts`, `presentation-widget/build.gradle.kts`, `i18n/build.gradle.kts`, `i18n-aniyomi/build.gradle.kts`, `macrobenchmark/build.gradle.kts`) only reference these entries. Their references are listed in section 5.

### 4.1 `gradle/libs.versions.toml`

| Entry (file:line) | Coordinate and version | Code | Rust answer | Area |
|---|---|---|---|---|
| `desugar` (libs.versions.toml:15) | `com.android.tools:desugar_jdk_libs:2.1.5` | D | Java 8 API desugaring for Android has no equivalent; Rust std is used. | 01 |
| `android-shortcut-gradle` (:16) | `com.github.zellius:android-shortcut-gradle-plugin:0.1.2` | D | Launcher shortcuts become `.desktop` `Actions=` entries (area:53 and area:54 section 8). | 53, 54 |
| `rxjava` (:18) | `io.reactivex:rxjava:1.3.8` | D | Dropped (T-01a-038). Streams become `tokio::sync::watch` and `broadcast`. | 01 |
| `okhttp-core` (:20) | `com.squareup.okhttp3:okhttp:5.4.0` | R | reqwest 0.13.5 (3.4). | 12 |
| `okhttp-logging` (:21) | `logging-interceptor` | D | Replaced by a `tracing` debug interceptor in `kuro-net` (area:12 section 2). | 12 |
| `okhttp-zstd` (:22) | `okhttp-zstd` | R | reqwest feature `zstd`. | 12 |
| `okhttp-brotli` (:23) | `okhttp-brotli` | R | reqwest feature `brotli`. | 12 |
| `okhttp-dnsoverhttps` (:24) | `okhttp-dnsoverhttps` | R | hickory-resolver 0.26.3 (3.4), C-38. | 12 |
| `okio` (:25) | `com.squareup.okio:okio:3.10.2` | S | `bytes` 1.x (3.4) and `std::io`. | 12, 13 |
| `conscrypt-android` (:27) | `org.conscrypt:conscrypt-android:2.5.3` | D | Dropped (T-01a-027). reqwest `rustls` uses the platform trust store (area:12 section 6 with `rustls-tls-native-roots`). **Verify at add:** the native roots path in reqwest 0.13. | 12 |
| `quickjs-android` (:29) | `app.cash.quickjs:quickjs-android:0.9.2` | R | rquickjs 0.14.0, feature `js-sources`, P2 only (C-19). | 12, 20 |
| `jsoup` (:31) | `org.jsoup:jsoup:1.22.2` | R | scraper 0.27.0 and ammonia 4.2.1 (3.5). | 13, 14, 15, 28, 32 |
| `disklrucache` (:33) | `com.jakewharton:disklrucache:2.0.2` | S | Directory of md5-named files with mtime eviction (area:34 section 6.2). No crate. | 34 |
| `unifile` (:34) | `com.github.tachiyomiorg:unifile:e0def6b3dc` | G | `std::fs` and `gio::File` (SAF is dropped; plain paths). | 13, 53 |
| `libarchive` (:35) | `me.zhanghai.android.libarchive:library:1.1.4` | X | System libarchive (Arch `libarchive`) through the C-33 spike. | 13 |
| `sqlite-framework` (:37) | `androidx.sqlite:sqlite-framework:2.4.0` | R | rusqlite 0.40.2 `bundled`. | 03, 04 |
| `sqlite-ktx` (:38) | `androidx.sqlite:sqlite-ktx:2.4.0` | D | Coroutine extensions for the SQLite driver; `tokio_rusqlite::Connection::call` replaces them. | 05 |
| `sqlite-android` (:39) | `com.github.requery:sqlite-android:3.45.0` | R | rusqlite 0.40.2 `bundled` (the bundled SQLite replaces the requery build). | 03, 05 |
| `preferencektx` (:41) | `androidx.preference:preference-ktx:1.2.1` | R | toml_edit 0.25.16 with serde (area:02). | 02 |
| `injekt` (:43) | `com.github.mihonapp:injekt:91edab2317` | S | `AppContext` struct, constructor injection (area:01 section 1). | 01 |
| `coil-bom` (:45) | `io.coil-kt.coil3:coil-bom:3.1.0` | D | No BOM in Rust; versions are in the workspace table. | 34 |
| `coil-core` (:46) | `io.coil-kt.coil3:coil` | R | `kuro-images` over reqwest, image 0.25.10 and `gdk::Texture` (area:34). | 34 |
| `coil-gif` (:47) | `io.coil-kt.coil3:coil-gif` | R | image 0.25.10 `gif` frames (area:34, area:44). | 34, 44 |
| `coil-compose` (:48) | `io.coil-kt.coil3:coil-compose` | D | Compose image binding; `gtk::Picture` replaces it. | 34 |
| `coil-network-okhttp` (:49) | `io.coil-kt.coil3:coil-network-okhttp` | D | reqwest is used directly. | 34 |
| `subsamplingscaleimageview` (:51) | `com.github.tachiyomiorg:subsampling-scale-image-view:66e0db195d` | G | Tiled `GtkSnapshot` drawing in a custom widget (area:43 section 10, area:44). | 43, 44 |
| `image-decoder` (:52) | `com.github.tachiyomiorg:image-decoder:41c059e540` | R | image 0.25.10 (3.7), avif-decode 3.0.0, jxl-oxide 0.12.6, libheif-rs 3.0.0 (feature `heif`). | 34, 44 |
| `natural-comparator` (:54) | `com.github.gpanther:java-nat-sort:natural-comparator-1.1` | S | Port of the comparator, about 60 lines in `kuro-core` (area:13 section 5). `natord` 1.0.9 is rejected. | 13, 14, 43 |
| `richtext-commonmark` (:56) | `com.halilibo.compose-richtext:richtext-commonmark:0.20.0` | R | pulldown-cmark 0.13.4 into `gtk::TextBuffer` tags (area:47b). | 47b |
| `richtext-m3` (:57) | `com.halilibo.compose-richtext:richtext-ui-material3:0.20.0` | D | Compose rendering; `gtk::TextView` replaces it. | 47b |
| `material` (:59) | `com.google.android.material:material:1.12.0` | D | Android Views Material; GTK widgets and app CSS (area:49). | 49 |
| `flexible-adapter-core` (:60) | `com.github.arkon.FlexibleAdapter:flexible-adapter:c8013533` | G | `gtk::ListView` with factories (area:37, area:40). | 37, 40 |
| `photoview` (:61) | `com.github.chrisbanes:PhotoView:2.3.0` | G | `gtk::GestureZoom`, `GestureDrag` (area:44). | 44 |
| `directionalviewpager` (:62) | `com.github.tachiyomiorg:DirectionalViewPager:1.0.0` | G | Custom pager widget (area:44). | 44 |
| `insetter` (:63) | `dev.chrisbanes.insetter:insetter:0.6.1` | D | No system bars on the desktop. | 49 |
| `compose-materialmotion` (:64) | `io.github.fornewid:material-motion-compose-core:2.0.1` | G | `gtk::StackTransitionType` (area:36, area:37). | 36 |
| `compose-webview` (:65) | `io.github.kevinnzou:compose-webview:0.33.6` | R | webkit6 0.6.1 behind `WebViewHost` (C-41). | 36, 12 |
| `compose-grid` (:66) | `io.woong.compose.grid:grid:1.2.2` | G | `gtk::GridView`. | 37, 40 |
| `compose-stablemarker` (:67) | `com.github.skydoves:compose-stable-marker:1.0.5` | D | Compose stability annotation; Rust types need no marker. | 07 |
| `reorderable` (:68) | `sh.calvin.reorderable:reorderable:2.4.3` | G | `gtk::DragSource` and `gtk::DropTarget` (area:08, area:37). | 08, 37 |
| `swipe` (:70) | `me.saket.swipe:swipe:1.3.0` | G | `gtk::GestureSwipe` and `GestureDrag` (area:46). | 46 |
| `moko-core` (:72) | `dev.icerock.moko:resources:0.24.5` | R | Fluent through `kuro-i18n` (C-10, area:51). | 51 |
| `logcat` (:74) | `com.squareup.logcat:logcat:0.1` | R | tracing 0.1.44, tracing-subscriber 0.3.23, tracing-appender 0.2.5 (C-05). | 01 |
| `aboutLibraries-compose` (:76) | `com.mikepenz:aboutlibraries-compose-m3:11.6.3` | X | cargo-about 0.9.2 generates the licence JSON at build time (area:47b). | 47b |
| `shizuku-api` (:78) | `dev.rikka.shizuku:api:13.1.0` | D | Android privileged-service API; no equivalent. | 01 |
| `shizuku-provider` (:79) | `dev.rikka.shizuku:provider:13.1.0` | D | Same. | 01 |
| `leakcanary-android` (:81) | `com.squareup.leakcanary:leakcanary-android:2.14` | D | Rust has no GC leaks; `heaptrack` when needed (area:01 topic 2.6). | 01 |
| `leakcanary-plumber` (:82) | `com.squareup.leakcanary:plumber-android:2.14` | D | Same. | 01 |
| `sqldelight-android-driver` (:84) | `app.cash.sqldelight:android-driver:2.0.2` | D | Hand-written SQL with rusqlite (area:05 section 8). | 05 |
| `sqldelight-coroutines` (:85) | `app.cash.sqldelight:coroutines-extensions-jvm:2.0.2` | D | `tokio::sync::watch` fed by the change hook (area:05). | 05 |
| `sqldelight-android-paging` (:86) | `app.cash.sqldelight:androidx-paging3-extensions:2.0.2` | D | Dropped (area:05 section 8): `gio::ListStore` holds full lists. | 05 |
| `sqldelight-dialects-sql` (:87) | `app.cash.sqldelight:sqlite-3-38-dialect:2.0.2` | D | SQL checked by rusqlite at test time; no codegen. | 05 |
| `junit` (:89) | `org.junit.jupiter:junit-jupiter:5.11.4` | S | `#[test]` (area:01 topic 2.6). | all |
| `kotest-assertions` (:90) | `io.kotest:kotest-assertions-core:5.9.1` | S | `assert!`, `assert_eq!`. | all |
| `mockk` (:91) | `io.mockk:mockk:1.13.17` | S | Hand-written fakes in `kuro-testkit` (area:01 topic 2.6). | all |
| `voyager-navigator` (:93) | `cafe.adriel.voyager:voyager-navigator:2.2.21-1.10.3` | S | The `ui::push`/`pop`/`pop_to_root` stack in `src/ui.rs` (area:01 topic 2.5, area:36). | 36 |
| `voyager-screenmodel` (:94) | `cafe.adriel.voyager:voyager-screenmodel` | S | Per-screen model structs (area:36). | 36 |
| `voyager-tab-navigator` (:95) | `cafe.adriel.voyager:voyager-tab-navigator` | S | `gtk::Stack` plus the spec-driven nav (area:36 section 6.2). | 36 |
| `voyager-transitions` (:96) | `cafe.adriel.voyager:voyager-transitions` | G | `gtk::StackTransitionType` (area:36). | 36 |
| `spotless-gradle` (:98) | `com.diffplug.spotless:spotless-plugin-gradle:7.0.2` | X | `cargo fmt --check` and `cargo clippy` (area:54 section 6). | 54 |
| `ktlint-core` (:99) | `com.pinterest.ktlint:ktlint-cli:1.7.0` | D | Kotlin lint; rustfmt replaces it. | 54 |
| `aboutLibraries` plugin (:102) | `com.mikepenz.aboutlibraries.plugin` | X | cargo-about (above). | 47b |
| `sqldelight` plugin (:103) | `app.cash.sqldelight` | D | No codegen. | 05 |
| `moko` plugin (:104) | `dev.icerock.mobile.multiplatform-resources` | D | Replaced by `xtask i18n-import` (area:51 section 8). | 51 |

Bundles in the same file (`okhttp`, `js-engine`, `sqlite`, `coil`, `shizuku`, `sqldelight`, `voyager`, `richtext`, `test`; lines 107 to 115) are unions of the entries above. Each member is decided above.

### 4.2 `gradle/androidx.versions.toml`

| Entry (file:line) | Coordinate and version | Code | Rust answer | Area |
|---|---|---|---|---|
| `gradle` (androidx.versions.toml:8) | `com.android.tools.build:gradle` 8.9.1 (AGP) | D | Android build plugin; Cargo replaces it. | 54 |
| `annotation` (:10) | `androidx.annotation:annotation:1.9.1` | D | Java annotations; not used in Rust. | 01 |
| `appcompat` (:11) | `androidx.appcompat:appcompat:1.7.0` | G | `gtk::Settings` for dark-mode choice (area:01 topic 2.3). | 01, 49 |
| `biometricktx` (:12) | `androidx.biometric:biometric-ktx:1.2.0-alpha05` | X | App lock: passphrase (argon2 0.6.0, secrecy 0.10.3) with PAM or polkit as an option (area:36 section 4). | 36 |
| `constraintlayout` (:13) | `androidx.constraintlayout:constraintlayout:2.2.1` | D | GTK containers. | 49 |
| `corektx` (:14) | `androidx.core:core-ktx:1.15.0` | S | `std`. | all |
| `localbroadcastmanager` (:15) | `androidx.localbroadcastmanager:localbroadcastmanager:1.1.0` | S | `tokio::sync::broadcast` (area:01 topic 2.3). | 01 |
| `splashscreen` (:16) | `androidx.core:core-splashscreen:1.0.1` | D | The window appears after migrations (area:01 topic 2.3). | 01 |
| `recyclerview` (:17) | `androidx.recyclerview:recyclerview:1.4.0` | G | `gtk::ListView` (area:37). | 37 |
| `viewpager` (:18) | `androidx.viewpager:viewpager:1.1.0` | G | `gtk::Notebook` or `gtk::Stack`. | 36, 44 |
| `profileinstaller` (:19) | `androidx.profileinstaller:profileinstaller:1.4.1` | D | ART profiles; not applicable. | 54 |
| `lifecycle-common` (:21) | `androidx.lifecycle:lifecycle-common:2.8.7` | S | App lifecycle in `kuro-core` (area:01 topic section 7). | 01 |
| `lifecycle-process` (:22) | `androidx.lifecycle:lifecycle-process:2.8.7` | S | Same. | 01 |
| `lifecycle-runtimektx` (:23) | `androidx.lifecycle:lifecycle-runtime-ktx:2.8.7` | S | Same. | 01 |
| `workmanager` (:25) | `androidx.work:work-runtime:2.10.0` | R | tokio timers in `kuro-downloads` and `kuro-track`, with `SchedulerBackend` (area:26 section 2.7); optional systemd user timer through `kuro-platform` (X). | 26, 53 |
| `paging-runtime` (:27) | `androidx.paging:paging-runtime:3.3.6` | G | `gio::ListStore` with incremental loading (area:40, area:41). | 40, 41 |
| `paging-compose` (:28) | `androidx.paging:paging-compose:3.3.6` | D | Compose binding. | 40 |
| `interpolator` (:30) | `androidx.interpolator:interpolator:1.0.0` | S | Tick-callback easing in `kuro-core::anim` (area:49, area:01 topic 2.3). | 49 |
| `benchmark-macro` (:32) | `androidx.benchmark:benchmark-macro-junit4:1.3.3` | X | `hyperfine` (area:54 section 10). | 54 |
| `test-ext` (:33) | `androidx.test.ext:junit-ktx:1.2.1` | D | Android instrumented tests; `cargo test`. | 54 |
| `test-espresso-core` (:34) | `androidx.test.espresso:espresso-core:3.6.1` | D | Android UI tests; the `KUROBAYOMI_SHOT` screenshot hook replaces them (area:01 topic 2.2). | 54 |
| `test-uiautomator` (:35) | `androidx.test.uiautomator:uiautomator:2.3.0` | D | Same. | 54 |

### 4.3 `gradle/kotlinx.versions.toml`

| Entry (file:line) | Coordinate and version | Code | Rust answer | Area |
|---|---|---|---|---|
| `reflect` (kotlinx.versions.toml:8) | `org.jetbrains.kotlin:kotlin-reflect` 2.4.0 | D | No reflection; serde derive. | 01 |
| `gradle` (:9) | `org.jetbrains.kotlin:kotlin-gradle-plugin` 2.4.0 | D | Cargo. | 54 |
| `compose-compiler-gradle` (:10) | `org.jetbrains.kotlin:compose-compiler-gradle-plugin` 2.4.0 | D | No Compose. | 01 |
| `immutables` (:12) | `org.jetbrains.kotlinx:kotlinx-collections-immutable:0.3.8` | D | Ownership and `Arc` replace immutable collections. | 01 |
| `coroutines-bom` (:14) | `kotlinx-coroutines-bom` 1.10.2 | D | Version alignment; not needed. | 01 |
| `coroutines-core` (:15) | `kotlinx-coroutines-core` 1.10.2 | R | tokio 1.53.2, futures 0.3.34, tokio-util 0.7.19, tokio-stream 0.1.19 (section 3.1). | 01 |
| `coroutines-android` (:16) | `kotlinx-coroutines-android` 1.10.2 | D | `Dispatchers.Main` is the GLib main context (`glib::spawn_future_local`). | 01 |
| `coroutines-guava` (:17) | `kotlinx-coroutines-guava` 1.10.2 | D | No Guava. | 01 |
| `coroutines-test` (:18) | `kotlinx-coroutines-test` 1.10.2 | R | tokio `test-util` feature, `#[tokio::test(start_paused = true)]`. | 01 |
| `serialization-json` (:20) | `kotlinx-serialization-json` 1.9.0 | R | serde 1.0.229 and serde_json 1.0.151. | 01, all |
| `serialization-json-okio` (:21) | `kotlinx-serialization-json-okio` 1.9.0 | D | `serde_json::from_reader` on `std::io`. | 01 |
| `serialization-protobuf` (:22) | `kotlinx-serialization-protobuf` 1.9.0 | R | prost 0.14.4 (C-36). | 21, 22 |
| `serialization-xml-core` (:23) | `io.github.pdvrieze.xmlutil:core-android` 0.90.3 | R | quick-xml 0.42.0. | 14, 22 |
| `serialization-xml` (:24) | `io.github.pdvrieze.xmlutil:serialization-android` 0.90.3 | R | quick-xml 0.42.0 with `serialize`. | 22 |
| `coroutines` bundle (:27), `serialization` bundle (:28) | bundles | | Members decided above. | |
| `android` (:31), `compose-compiler` (:32), `serialization` (:33) plugins | Kotlin Gradle plugins | D | Cargo. | 54 |

### 4.4 `gradle/compose.versions.toml`

| Entry (file:line) | Coordinate and version | Code | Rust answer | Area |
|---|---|---|---|---|
| `compose-bom` (compose.versions.toml:2) | `androidx.compose:compose-bom:2025.10.01` | D | No Compose. | 49 |
| `activity` (:5) | `androidx.activity:activity-compose:1.10.1` | G | `gtk::Application` lifecycle (area:01). | 01 |
| `bom` (:6) | `androidx.compose:compose-bom` alias for the BOM version | D | Compose BOM alias; no Rust counterpart. | 49 |
| `foundation` (:7) | `androidx.compose.foundation:foundation` | G | GTK widgets. | 49 |
| `animation` (:8) | `androidx.compose.animation:animation` | G | `gtk::Revealer`, `gtk::StackTransitionType`, CSS transitions (area:49). | 49 |
| `animation-graphics` (:9) | `androidx.compose.animation:animation-graphics` | D | Animated vector drawables; SVG through GTK's pixbuf loaders (area:49b). | 49b |
| `runtime` (:10) | `androidx.compose.runtime:runtime` | D | Compose runtime; `Rc`/`RefCell` in GTK callbacks. | 49 |
| `ui-tooling` (:11) | `androidx.compose.ui:ui-tooling` | D | Replaced by the `KUROBAYOMI_SHOT=file.png` hook (area:01 topic 2.2). | 01 |
| `ui-tooling-preview` (:12) | `androidx.compose.ui:ui-tooling-preview` | D | Same. | 01 |
| `ui-util` (:13) | `androidx.compose.ui:ui-util` | S | `std` iterators. | 01 |
| `material3-core` (:15) | `androidx.compose.material3:material3` | G | GTK widgets plus app CSS (area:49 section 7). | 49 |
| `material-icons` (:16) | `androidx.compose.material:material-icons-extended` | G | Symbolic icons bundled as SVG, looked up through `kuro-platform::icons` (addenda rule 6). | 49, 49b |
| `glance` (:18) | `androidx.glance:glance-appwidget:1.1.1` | D | Android home-screen widgets; dropped (T-01a-024, area:50). | 50 |

### 4.5 `gradle/aniyomi.versions.toml` (native-library catalog, not in `libs.versions.toml`)

| Entry (file:line) | Coordinate and version | Code | Rust answer | Area |
|---|---|---|---|---|
| `aniyomi-mpv` (aniyomi.versions.toml:13) | `com.github.aniyomiorg:aniyomi-mpv-lib:1.18.n` | R, X | libmpv2 6.0.0 feature `render`, with the system `libmpv` (C-30). | 45, 46 |
| `arthenica-smartexceptions` (:14) | `com.arthenica:smart-exception-java:0.2.1` | D | FFmpegKit error decoration; Rust errors carry their own text. | 24 |
| `compose-constraintlayout` (:15) | `androidx.constraintlayout:constraintlayout-compose:1.1.0` | D | GTK containers. | 49 |
| `ffmpeg-kit` (:16) | `com.github.jmir1:ffmpeg-kit:1.18` | X | System `ffmpeg` and `ffprobe` processes (C-31). | 24, 14, 16 |
| `mediasession` (:17) | `androidx.media:media:1.7.0` | R | souvlaki 0.8.3 `use_zbus` (C-32). | 45, 46 |
| `nanohttpd` (:18) | `org.nanohttpd:nanohttpd:2.3.1` | R | axum 0.8.9 (C-22). | 16 |
| `seeker` (:19) | `io.github.2307vivek:seeker:1.2.2` | G | `gtk::Scale` with chapter marks drawn in the widget (area:46). | 46 |
| `torrserver` (:20) | `io.github.secozzi:torrserver:0.1.1` | X | External TorrServer process (C-18). P2. | 13, 45 |
| `truetypeparser` (:21) | `io.github.yubyf:truetypeparser-light:2.1.4` | R | ttf-parser 0.25.1 (3.7). | 48 |

Gradle-wrapper and JVM settings (`gradle/wrapper/gradle-wrapper.properties` pins Gradle 8.13; `gradle/gradle-daemon-jvm.properties` lists toolchains) are dropped with Gradle (D). The Rust toolchain is pinned by `rust-toolchain.toml` at 1.98 (area:54).

### 4.6 Module-level declarations

Each module's `build.gradle.kts` references only the catalog entries above. The references are:

| Module (file) | Gradle references (catalog entries, section 4 above) | Rust crate that replaces the module |
|---|---|---|
| `app/build.gradle.kts` | Everything in the `app` dependency list, including `libs.bundles.okhttp`, `libs.bundles.coil`, `libs.bundles.sqlite`, `libs.bundles.voyager`, `libs.bundles.richtext`, `libs.bundles.shizuku`, `aniyomilibs.*`, `kotlinx.bundles.*`, `compose.*`, `androidx.*` | the root `kurobayomi` package plus the library crates in section 2.1 |
| `core/archive/build.gradle.kts` | `libs.jsoup`, `libs.libarchive`, `libs.unifile` | `kuro-archive` |
| `core/common/build.gradle.kts` | `kotlinx.coroutines.core`, `kotlinx.serialization.json`, `kotlinx.serialization.json.okio`, `libs.logcat`, `libs.okhttp.*`, `libs.okio`, `libs.preferencektx`, `libs.rxjava`, `aniyomilibs.ffmpeg.kit`, `aniyomilibs.torrserver`, `libs.bundles.js.engine`, `libs.image.decoder`, `libs.jsoup`, `libs.libarchive`, `libs.natural.comparator`, `libs.unifile`, `projects.i18n`, `libs.bundles.test` | `kuro-core`, `kuro-net`, `kuro-platform`, `kuro-i18n` |
| `core-metadata/build.gradle.kts` | `kotlinx.bundles.serialization`, `projects.sourceApi` | `kuro-local` (metadata part) |
| `data/build.gradle.kts` | `libs.bundles.sqldelight`, `kotlinx.serialization.json`, `kotlinx.serialization.json.okio`, `kotlinx.serialization.protobuf`, `projects.core.common`, `projects.domain`, `projects.sourceApi` | `kuro-data`, `kuro-db` |
| `domain/build.gradle.kts` | `libs.sqldelight.android.paging`, `libs.compose.stablemarker`, `kotlinx.bundles.coroutines`, `kotlinx.bundles.serialization`, `libs.unifile`, `kotlinx.coroutines.bom`, `projects.core.common`, `projects.sourceApi`, tests | `kuro-domain` |
| `source-api/build.gradle.kts` | `aniyomilibs.nanohttpd`, `kotlinx.serialization.json`, `libs.injekt`, `libs.jsoup`, `libs.preferencektx`, `libs.rxjava`, `compose.runtime`, `kotlinx.coroutines.android`, `compose.bom`, `kotlinx.coroutines.bom`, `projects.core.common` | `kuro-source` |
| `source-local/build.gradle.kts` | `aniyomilibs.ffmpeg.kit`, `kotlinx.bundles.serialization`, `libs.unifile`, `projects.core.archive`, `projects.core.common`, `projects.coreMetadata`, `projects.domain`, `projects.sourceApi`, `projects.i18n`, `projects.i18nAniyomi` | `kuro-local` |
| `presentation-core/build.gradle.kts` | `projects.core.common`, `projects.i18n`, Compose entries | GTK widget module in `kurobayomi` (area:49) |
| `presentation-widget/build.gradle.kts` | `libs.injekt`, `projects.i18n`, `projects.i18nAniyomi`, `compose.glance`, `libs.coil.core`, `libs.material`, `projects.core.common`, `projects.domain`, `projects.presentationCore` | Dropped (area:50, T-01a-024) |
| `i18n/build.gradle.kts`, `i18n-aniyomi/build.gradle.kts` | `libs.moko.core` | `kuro-i18n` (C-10) |
| `macrobenchmark/build.gradle.kts` | `androidx.benchmark.macro`, `androidx.test.espresso.core`, `androidx.test.ext`, `androidx.test.uiautomator` | `hyperfine` (area:54 section 10) |

---

## 5. Version corrections to plan documents

These are the versions quoted in the area docs that crates.io contradicts, with the value to use. Each area owner updates the cited line when the crate is added.

| Crate | Quoted in (repo-relative) | Quoted | Use |
|---|---|---|---|
| rust-version | `plans/54-build-release-distribution-docs/p1-rust-design.md` section 3 | 1.85 | 1.98 (C-01) |
| reqwest | 12, 15, 18, 19, 26, 28, 29, 30, 31, 32, 33, 34, 35, 38, 39, 40, 41, 45, 47, 54 | 0.12, feature `rustls-tls` | 0.13.5, feature `rustls` (C-37) |
| rusqlite | 37 R-15 (0.32), 39 p1 (0.32), 40 (0.31), 38 (0.3x), 47 section 7 (0.32), 54 section 3 (0.33), 18 section 15 (0.32), 42 section 2 | 0.31 to 0.33 | 0.40.2 |
| tokio-rusqlite | 47 section 7 | 0.6 | 0.8.0 |
| thiserror | 40 section 8 | 1 | 2.0.21 |
| chrono | 04, 07, 26, 28, 36, 38, 39 p2, 47 (quoted as "0.4") | 0.4 | 0.4.45 (feature set in section 3.6) |
| time | 01, 05, 08, 10, 14, 27 p1 | 0.3 | removed (C-02) |
| rand | 29 section 2, 37 R-15 | 0.8 | 0.10.3 (C-08) |
| icu_collator | 39 p2 section 4 | 1.x | 2.3.1 (C-09) |
| icu_displaynames | 40 p1 section 8 (1.x), 41 p2 section 5 and 51 section 1 (no version) | 1.x or none | 0.11.2, dev only (C-11) |
| prost | 19 section 13, 47 p2 section 1 | 0.13 | 0.14.4 (C-36) |
| quick-xml | 14, 22 | 0.37, serialize | 0.42.0 (C-40) |
| scraper | 13 section 5, 14 section 1.1 | 0.20+ | 0.27.0 (C-40) |
| zip | 13 section 5, 14, 18 section 15, 19 section 13, 41 section 9 | 2 | 8.6.0 (C-34) |
| keyring | 27 section 2, 27 p2 section 2, 28 section 11, 29 section 2, 30 section 11, 47 section 7 | 3 | 4.2.0 (C-28) |
| ashpd | 36 section 4, 38 section 5, 47 section 7, 49 section 11 | 0.9+, 0.11, latest | 0.13.13 (C-39) |
| webkit6 | 12 section 6 (0.6.x), 38 section 5 (0.4+) | | 0.6.1 (C-41) |
| sha2, sha1 | 13 section 5, 19 section 13, 35 section 13 | 0.10 | 0.11.0 (C-26) |
| md-5 | 34 section 1.1, 39 p1 section 2 | 0.10 | 0.11.0 (C-26) |
| hickory-resolver | 47 section 7 | 0.24 | 0.26.3 (C-38) |
| directories | 47 section 7 | 5 | 6.0.0 (C-29) |
| bincode | 23, 24 | 2.x | not used (C-35) |
| libheif-rs | 34 section 1.1 | 2.x | 3.0.0 (C-24) |
| serde | 18 section 15 (1.0.229) | | 1.0.229 (unchanged) |
| serde_json | 18 section 15 (1.0.151) | | 1.0.151 (unchanged) |
| base64 | 27 p2, 29, 19 | 0.22 | 0.23.1 (C-42) |
| argon2 | 47 section 7 | 0.5 | 0.6.0 (C-42) |
| rustix | 47 p2 section 1 | 0.38 | 1.1.5 (C-27) |
| zbus | 47 section 7, 26 section 7 | 5 | 5.19.0 (no change in major) |
| notify-rust | 24 section 2, 26 section 2.6 | 4.x | not used (C-04) |
| lru | 34 section 1.1 | 0.12 | 0.18.5 (C-48) |
| image | 13, 34, 43, 44 | 0.25 | 0.25.10 (no major change; MSRV 1.88) |
| pulldown-cmark | 47b section 11 | (no version) | 0.13.4 |
| fluent-bundle | 51 section 1 | "*" | 0.16.0 |
| cookie_store | 12 section 6 | (no version) | 0.22.1 (C-14) |
| wasmtime | 18 section 15 | "latest stable major" | 49.0.2 (P2) |
| libmpv2 | 45 p2 section 2 | "verify" | 6.0.0 feature `render` |
| souvlaki | addenda table, 01 topic 2.3 | (no version) | 0.8.3, `use_zbus` |
| fancy-regex | 06 section 2, 10 section 4 | (no version) | 0.19.2 only for SeasonRecognition (C-07) |
| glib | 47 p2 section 1 | 0.22 | 0.22.10 via gtk4 (no direct pin) |

---

## 6. Arch Linux system packages and runtime programs

Package names are the Arch repository names. `pacman -S --needed <name>` installs them.

| Package (pacman) | Needed for | When | Crate or program |
|---|---|---|---|
| `gtk4` | GTK 4 runtime and headers (pkg-config `gtk4`, version 4.16 or newer) | build and run | gtk4 0.11.5 |
| `pkgconf` | build scripts that locate system libraries | build | gtk4-sys, libmpv2-sys, webkit6 |
| `webkit2gtk-6.0` | WebKitGTK 6.0 (pkg-config `webkitgtk-6.0`) | build and run | webkit6 0.6.1 (P0 login fallback is the browser, so run is optional for P0) |
| `mpv` | `libmpv.so` and `mpv/render.h` (pkg-config `mpv`) | build and run | libmpv2 6.0.0. Arch's `mpv` package uses mpv's default meson options, which include GPL parts (verify with `pacman -Qi mpv` on the target); Flatpak builds set `-Dgpl=false` (area:54 section 7), and that build is LGPL |
| `ffmpeg` | `ffmpeg` and `ffprobe` executables | run | area:24 downloads, area:14 video thumbnails |
| `libarchive` | `libarchive.so` and `archive.h` (pkg-config `libarchive`) | build and run | libarchive3-sys (C-33 spike) |
| `libheif` | `libheif.so` (pkg-config `libheif`) | build and run only with feature `heif` | libheif-rs 3.0.0 (P2) |
| `xdg-desktop-portal-kde` | Portal backend for FileChooser, OpenURI, Settings, Background, Notification on KDE Plasma | run | ashpd 0.13.13, gtk::FileDialog |
| `xdg-desktop-portal` | Portal front end | run | ashpd |
| `kwallet` | Secret Service provider on KDE (KWallet exposes `org.freedesktop.secrets`) | run | keyring 4.2.0 |
| `gnome-keyring` | Alternative Secret Service provider | run, optional | keyring 4.2.0 |
| `clang` | `libclang` for bindgen if libmpv2-sys runs bindgen during build | build, only if the build fails without it | libmpv2 6.0.0 (**verify at add**) |
| `hyperfine` | Startup benchmark | dev only | area:54 |

Not needed (no system library): SQLite (bundled by rusqlite), zlib (flate2 uses miniz_oxide), OpenSSL (rustls), libsecret (zbus Secret Service client), gettext (Fluent), dav1d (avif-decode is pure Rust), libjxl (jxl-oxide is pure Rust), libdbus (zbus and `use_zbus`).

Runtime programs that may be absent: when `ffmpeg` is absent, downloads that need remux fail with `DlError::Other("ffmpeg not found")` and thumbnails are skipped. When TorrServer is absent (P2), torrent sources show an error entry. When no Secret Service provider runs, `SecretStore` returns `Err` and tracker login asks again each session (area:47 T-47a-059 fallback).

---

## 7. Licences

Workspace licence: Apache-2.0 (`[workspace.package]`, section 2.2).

Licences that need a decision or a check:

| Item | Licence | Effect |
|---|---|---|
| gtk4, glib, gio and the gtk-rs crates | MIT (gtk-rs) over LGPL-2.1+ GTK and GLib libraries | Dynamic linking through the system libraries. Allowed. |
| libmpv2 and system libmpv | LGPL-2.1 (crate); mpv is LGPL when built with `-Dgpl=false` | Allowed with `-Dgpl=false` (Flatpak). Arch's GPL build of mpv makes a local binary GPL. |
| webkit6 and WebKitGTK | MIT (crate); WebKitGTK LGPL-2.0+ | Dynamic linking. Allowed. |
| wasmtime | Apache-2.0 WITH LLVM-exception | Allowed (added to the deny list, section 10). |
| unicode data (icu_*, unicode-ident) | Unicode-3.0 | Allowed (added to the deny list). |
| ed25519-dalek | BSD-3-Clause | Allowed. |
| encoding_rs | (Apache-2.0 OR MIT) AND BSD-3-Clause | Allowed. |

Per-crate licence text is included in the About page from `cargo about` output (area:47b section 11, task T-47a-096).

---

## 8. Workspace dependency table (Decided)

This is the complete `[workspace.dependencies]` table. A crate that needs one of these writes `name = { workspace = true }`.

```toml
[workspace.dependencies]
# runtime and GTK
gtk = { package = "gtk4", version = "0.11.5", features = ["v4_16"] }
tokio = { version = "1.53.2", features = ["rt-multi-thread", "macros", "sync", "time", "fs", "process", "io-util"] }
tokio-util = { version = "0.7.19", features = ["sync"] }
tokio-stream = "0.1.19"
futures = "0.3.34"
async-channel = "2.5.0"
async-trait = "0.1.92"
thiserror = "2.0.21"
anyhow = "1.0.104"

# logging
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter", "fmt", "reload"] }
tracing-appender = "0.2.5"

# storage
rusqlite = { version = "0.40.2", features = ["bundled", "functions", "hooks", "backup", "limits"] }
tokio-rusqlite = "0.8.0"
serde = { version = "1.0.229", features = ["derive"] }
serde_json = { version = "1.0.151", features = ["preserve_order"] }
toml = "1.1.7"
toml_edit = "0.25.16"
memmap2 = "0.9.11"
walkdir = "2.5.0"
lru = "0.18.5"
rust_decimal = "1.43.0"

# network
reqwest = { version = "0.13.5", default-features = false, features = ["rustls", "json", "stream", "gzip", "brotli", "zstd", "cookies", "form", "query", "hickory-dns"] }
url = "2.5.8"
bytes = "1"
hickory-resolver = { version = "0.26.3", features = ["https-rustls"] }
psl = "2.1.241"
cookie_store = "0.22.1"
axum = { version = "0.8.9", default-features = false, features = ["tokio", "http1"] }
encoding_rs = "0.8.42"

# parsing and text
prost = { version = "0.14.4", default-features = false, features = ["derive"] }
quick-xml = { version = "0.42.0", features = ["serialize"] }
scraper = "0.27.0"
ammonia = "4.2.1"
html-escape = "0.2.15"
pulldown-cmark = { version = "0.13.4", default-features = false }
regex = "1.13.1"
fancy-regex = "0.19.2"
isolang = "2.4.0"

# i18n and dates
chrono = { version = "0.4.45", default-features = false, features = ["clock", "std"] }
icu_collator = "2.3.1"
icu_locale = "2.3.1"
icu_datetime = "2.3.0"
fluent-bundle = { version = "0.16.0", features = ["concurrent"] }
unic-langid = "0.9.6"
rand = { version = "0.10.3", features = ["std", "std_rng", "chacha"] }

# media and archives
image = { version = "0.25.10", default-features = false, features = ["jpeg", "png", "gif", "webp", "bmp", "tiff"] }
avif-decode = "3.0.0"
jxl-oxide = "0.12.6"
libheif-rs = "3.0.0"
zip = { version = "8.6.0", default-features = false, features = ["deflate"] }
flate2 = "1.1.10"
libarchive3-sys = "0.1.2"
ttf-parser = "0.25.1"
md-5 = "0.11.0"
sha2 = "0.11.0"
sha1 = "0.11.0"
semver = "1.0.28"
base64 = "0.23.1"
libmpv2 = { version = "6.0.0", features = ["render"] }
souvlaki = { version = "0.8.3", default-features = false, features = ["use_zbus"] }

# platform
directories = "6.0.0"
rustix = { version = "1.1.5", features = ["fs", "system"] }
zbus = "5.19.0"
ashpd = { version = "0.13.13", features = ["tokio", "gtk4"] }
keyring = "4.2.0"
webkit6 = "0.6.1"

# security
secrecy = "0.10.3"
argon2 = "0.6.0"
ed25519-dalek = "3.0.0"

# extensions (P2 features)
wasmtime = { version = "49.0.2", default-features = false, features = ["runtime", "component-model", "cranelift", "std"] }
rquickjs = "0.14.0"

# dev and build
proptest = "1.11.0"
wiremock = "0.6.5"
tempfile = "3.27.0"
glib-build-tools = "0.22.8"
icu_displaynames = "0.11.2"
fluent-syntax = "0.12.0"
```

Notes:

- `libheif-rs`, `wasmtime` and `rquickjs` are not optional in the workspace table (Cargo does not allow `optional` there). Each owning crate lists them with `optional = true` and `workspace = true`, and they are enabled only by the features `heif`, `wasm` and `js-sources` (C-18 to C-24).
- `icu_displaynames` and `fluent-syntax` appear only in the `xtask` crate. They are not dependencies of any runtime crate.
- The `gtk` dependency name follows the mockup's `Cargo.toml`.
- The table is a starting point: every line is re-checked by `cargo add` and `cargo update` at implementation, because section 11 lists the features still to confirm.

Features of the root package (`kurobayomi`), following area:54 section 4 with corrected names:

```toml
[features]
default = []
updater = []        # area:35 app updater (never in Flatpak)
dev-tools = []      # debug-only UI
heif = ["kuro-images/heif"]
wasm = ["kuro-extensions/wasm"]
js-sources = ["kuro-extensions/js-sources"]
```

---

## 9. Crates by area (lookup table)

Read the row for your area, then section 3 for the version and rationale.

| Area | Crates it adds (from this file) |
|---|---|
| 01 foundation | tokio, tracing, tracing-subscriber, tracing-appender, thiserror, anyhow, serde, serde_json, toml, chrono, directories, rustix, async-channel, futures, gtk4 |
| 02 preferences | toml_edit, toml, serde, keyring (for secrets), chrono, kuro-platform |
| 03 DB schema | rusqlite (bundled), url (for URLs in rows), tokio-rusqlite |
| 04 DB anime | rusqlite, tokio-rusqlite, serde_json (`preserve_order`), chrono |
| 05 data repos | rusqlite, tokio-rusqlite, serde_json, tokio, async-channel |
| 06 data history, track, source | rusqlite, tokio, futures, serde, serde_json, prost, flate2, url, regex (mention rewrite by hand, C-07), thiserror, tracing, async-trait, tempfile (dev) |
| 07 domain entries | regex, chrono, icu_collator, serde_json, tracing, thiserror, tokio |
| 08 domain library | kuro-db, kuro-prefs, tokio, icu_collator, icu_locale, rand, chrono, thiserror, tracing |
| 09 domain source, track, download, release | rusqlite, reqwest, flate2, serde, serde_json, semver, regex, url, thiserror, tokio, async-trait |
| 10 app domain layer | tokio, tokio-stream, futures, async-trait, fancy-regex (SeasonRecognition only), regex, chrono, icu_collator, serde_json, tempfile, thiserror, tracing |
| 11 app migrations | isolang, serde_json, rand, thiserror, tracing, kuro-platform (zbus indirectly through it) |
| 12 network | reqwest, cookie_store, tokio, tokio-util, async-trait, futures, hickory-resolver, psl, webkit6, serde, serde_json, base64, url, bytes, thiserror, tracing; P2: http-cache-semantics, cacache, rquickjs |
| 13 core util, archive, torrent | zip, libarchive3-sys (spike), memmap2, scraper, image, jxl-oxide, libheif-rs (P2), icu_collator, md-5, sha1, sha2, tempfile, walkdir, rustix, url, form_urlencoded (via url), serde, serde_json, quick-xml, thiserror, tracing, tokio, reqwest, proptest (dev) |
| 14 local and metadata | kuro-archive, quick-xml, chrono, url, scraper, tempfile, tokio, thiserror, tracing, serde, serde_json, regex |
| 15 source API manga | async-trait, thiserror, serde, serde_json, md-5, url, reqwest, bytes, scraper, encoding_rs, tokio, futures, tokio-util, tracing, wiremock (dev) |
| 16 source API anime | axum, ffmpeg (process), reqwest, scraper, md-5, regex, tokio, tracing, serde, serde_json |
| 17 source app layer | tokio, rusqlite, thiserror, tracing, serde (its ICU4X display-name and collation use moves to the committed names table, C-11) |
| 18 extensions manga | tokio, tokio-util, reqwest, async-channel, futures, rusqlite, serde, serde_json, zip, ed25519-dalek, sha2, base64, wasmtime (P2), thiserror, tracing, url |
| 19 extensions anime | tokio, reqwest, serde, serde_json, prost, flate2, zip, sha2, ed25519-dalek, base64, url, rusqlite, async-channel, tokio-util, thiserror, tracing, wasmtime (P2) |
| 20 extension strategy | scraper (P1), wasmtime (P2), notify (P2 option only), rquickjs (P2 option only), async-channel, tokio, kuro-platform (notifier) |
| 21 backup format | prost, flate2, quick-xml, serde, serde_json, chrono, rusqlite, tokio, async-channel, thiserror, tracing |
| 22 backup restore | rusqlite, prost, flate2, quick-xml, serde, serde_json, chrono, async-channel, thiserror, tracing, rustix |
| 23 downloads manga | tokio, tokio-util, futures, async-channel, async-trait, indexmap (rejected, C-13: use Vec), serde_json (index, C-35), zip, quick-xml, image, rusqlite, rustix, tracing |
| 24 downloads anime | tokio (process), tokio-util, futures, async-channel, rusqlite, serde_json, rustix, zbus (through kuro-platform), tracing, thiserror, ffmpeg (process) |
| 25 downloads shared | tokio, async-channel, thiserror, tracing, rusqlite, serde, serde_json, tempfile (dev), walkdir (dev) |
| 26 library update jobs | tokio, tokio-util, async-channel, async-trait, chrono, reqwest, serde_json, rusqlite, thiserror, tracing, zbus (through kuro-platform), ashpd (through kuro-platform) |
| 27 tracking core | tokio, serde, serde_json, rusqlite, thiserror, tracing, futures, async-trait, keyring, gtk (gio via gtk), url, base64, sha2, rand, chrono, secrecy, async-channel |
| 28 AniList | reqwest, serde, serde_json, tokio, async-trait, chrono, url, ammonia, html-escape, keyring, thiserror, tracing, bytes, async-channel, wiremock (dev) |
| 29 MyAnimeList | reqwest, serde, serde_json, url, base64, rand, chrono, tokio, futures, thiserror, tracing, keyring |
| 30 Kitsu | reqwest, serde, serde_json, chrono, tokio, async-channel, url, thiserror, keyring, tracing, wiremock (dev) |
| 31 Shikimori and Bangumi | reqwest, serde, serde_json, tokio, async-trait, thiserror, url, tracing, wiremock (dev) |
| 32 MangaUpdates and Simkl | reqwest, serde, serde_json (preserve_order), async-trait, thiserror, url, html-escape, ammonia, keyring, tracing |
| 33 Kavita, Komga, Suwayomi, Jellyfin | reqwest, tokio, futures, serde, serde_json, md-5, url, thiserror, tracing, chrono |
| 34 images and covers | image, avif-decode, jxl-oxide, libheif-rs (P2), lcms2 (P2), md-5, serde_json, reqwest, tokio, tokio-util, bytes, thiserror, tracing, rustix, chrono, rust_decimal, lru, async-channel |
| 35 updater, notifications, export | semver, sha2, reqwest, tokio, tokio-util, async-channel, thiserror, tracing, url, serde, serde_json, gio (through gtk), directories (through kuro-platform) |
| 36 UI shell, security, webview | webkit6, ashpd (portal), argon2, secrecy, zbus (fprintd through kuro-platform), chrono, url, tokio, thiserror, tracing |
| 37 UI library | rusqlite, tokio, async-channel, icu_collator, rand, serde, serde_json, tracing |
| 38 entry details manga | tokio, async-channel, rusqlite, reqwest, serde, serde_json, thiserror, tracing, regex, chrono, rust_decimal, icu_collator, image, ashpd (optional), webkit6 (optional) |
| 39 entry details anime | gtk, tokio, tokio-util, async-channel, rusqlite, reqwest, serde, serde_json, md-5, tempfile, thiserror, tracing, chrono, icu_collator 2.3.1 (39 p2 must move from 1.x, C-09) |
| 40 browse manga | gtk, tokio, tokio-util, async-channel, rusqlite, reqwest, serde, serde_json, toml (repo config), thiserror, async-trait, tempfile, regex, icu_locale, icu_collator, tracing; display names come from the committed table (C-11) |
| 41 browse anime | gtk, tokio, async-channel, futures, reqwest, rusqlite, serde, serde_json, zip, image, thiserror, async-trait, tracing; display names come from the committed table (C-11) |
| 42 updates, history, stats, storage, upcoming | gtk, rusqlite, chrono, tokio, async-channel, thiserror, tracing |
| 43 reader core | gtk, image, tokio, rusqlite, thiserror, tracing, lru |
| 44 reader viewers | gtk, tokio, tokio-util, async-channel, image, avif-decode (feature), jxl-oxide, libheif-rs (optional), lru, serde, serde_json, tracing, proptest (dev) |
| 45 player core | libmpv2, reqwest, serde, serde_json, tokio, tokio-util, async-channel, rusqlite (through kuro-domain), isolang (display names from the committed table, C-11), regex, chrono, tracing, souvlaki (through kuro-platform), axum (for LocalStreamServer, through kuro-source) |
| 46 player controls UI | gtk, souvlaki (through kuro-platform), libmpv2, ashpd (through kuro-platform), async-channel, tracing |
| 47 settings screens general | gtk, glib, tokio, tokio-rusqlite, rusqlite, serde, serde_json, async-channel, thiserror, tracing, tracing-subscriber, zbus, ashpd, keyring, argon2, chrono, reqwest, hickory-resolver, directories, cargo-about (build tool), prost, flate2, rustix, isolang (display names from the committed table, C-11) |
| 47b settings widgets, search, more | gtk, glib, tokio, pulldown-cmark, serde, serde_json, tracing, cargo-about (build) |
| 48 settings player and scripts | libmpv2, regex (fancy-regex is not used here, C-07), isolang, ttf-parser, serde, serde_json, rusqlite, prost, tracing |
| 49 theme and design system | gtk, glib, ashpd, tokio, serde, serde_json, glib-build-tools (build), gettext-rs (rejected, C-10) |
| 49b Android res values and drawables | gtk, glib-build-tools (build); no runtime crate |
| 50 Android widgets | Dropped (T-01a-024). No crate. |
| 51 i18n | fluent-bundle, unic-langid, icu_datetime, isolang, glib (language list); xtask only: quick-xml, fluent-syntax, icu_displaynames |
| 52 util app | gtk, glib, ashpd, icu_collator, icu_datetime, chrono, tokio, tokio-util, thiserror, tracing, md-5, rand, reqwest, url, serde, serde_json, anyhow |
| 53 platform manifest, intents, workers | kuro-platform (gio, zbus, ashpd, directories, rustix), tracing, tracing-subscriber, thiserror, url, anyhow; arguments by `std::env::args` (no argument crate) |
| 54 build, release, docs | tokio, rusqlite, reqwest, tracing, tracing-subscriber, thiserror, anyhow, glib-build-tools (through the root package), cargo-about, cargo-deny, hyperfine, flatpak tools; `build.rs` uses std only (C-02) |

Note on area:52: `chrono-tz` is rejected (C-02 and section 3.6). Local midnight comes from `chrono::Local`.

---

## 10. Deny list (`deny.toml`) update

Replace the allow list in `plans/54-build-release-distribution-docs/p1-rust-design.md` section 13 with:

```toml
[advisories]
yanked = "deny"

[licenses]
allow = [
  "Apache-2.0",
  "Apache-2.0 WITH LLVM-exception",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "BSL-1.0",
  "CC0-1.0",
  "ISC",
  "LGPL-2.1",
  "LGPL-2.1-or-later",
  "MIT",
  "MPL-2.0",
  "Unicode-3.0",
  "Unlicense",
  "Zlib",
]
confidence-threshold = 0.9

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

Additions over the 54 list: `Apache-2.0 WITH LLVM-exception` (wasmtime, C-20), `BSL-1.0` (ryu via serde_json, which is `Apache-2.0 OR BSL-1.0`), `CC0-1.0` (notify, P2 option), `Unicode-3.0` (icu_* and unicode-ident), `Unlicense` (walkdir). `multiple-versions = "warn"` is kept: the reqwest 0.13 tree has its own `http`, `hyper` and `cookie_store` versions, and each duplicate is reviewed once at the first `cargo deny check` (area:54 section 13).

---

## 11. Verify at add (open technical checks)

Each row is a fact that the docs do not settle and crates.io metadata does not show. The step is `cargo add` plus the listed command. A failure changes only the row's crate line, not the decision.

| # | Check | Command or method | If it fails |
|---|---|---|---|
| V-01 | keyring 4.2.0 feature that enables the Linux Secret Service store (`zbus-secret-service-keyring-store`) | Read `keyring`'s `Cargo.toml` features after `cargo add keyring`; run the `kuro-platform` unit test against `gnome-keyring-daemon --unlock` | Fall back to the `secret-service` crate (zbus-based) behind `SecretStore` |
| V-02 | reqwest 0.13 `cookies` feature and `cookie_store` version | `cargo tree -i cookie_store` shows one version | Pin `cookie_store` to the reqwest version; drop the explicit line |
| V-03 | reqwest 0.13 rustls uses the Linux system trust store | Request a site with a private CA on the test machine | Enable the platform verifier feature of rustls in reqwest 0.13 (name per its Cargo.toml) |
| V-04 | reqwest DNS hook for hickory-resolver 0.26 (DoH) | Compile `kuro-net`'s DoH resolver against the reqwest 0.13 `dns_resolver` API | Use `hickory-dns` for system DNS and skip DoH until fixed (P2) |
| V-05 | rand 0.10 API: `SeedableRng::seed_from_u64`, `StdRng`, and OS RNG source for PKCE | Unit test in `kuro-core` | Use `rand_chacha` 0.10.0 directly (MSRV 1.85) |
| V-06 | avif-decode 3.0.0 decodes the AVIF fixtures of area:44 and area:34 | Test with the fixture files in `tests/fixtures/images/` | Keep AVIF behind a feature off by default; MSRV then stays 1.92 for P0 |
| V-07 | libarchive3-sys 0.1.2 covers `read_open_memory` and format detection | Spike T-13a-031 | Own FFI module in `kuro-archive` (declared `unsafe_code = "deny"` in that crate) |
| V-08 | icu_displaynames 0.11.2 works with icu_locale 2.3.1 | `cargo tree -i icu_locale_core` shows one version | Hand-written names table from the ISO 639 list in `isolang` |
| V-09 | fluent-syntax 0.12.0 and fluent-bundle 0.16.0 are one release family | Parse a file produced by the converter (area:51 section 8) | Use `fluent-syntax` at the version fluent-bundle 0.16 depends on |
| V-10 | libmpv2 6.0.0 builds against the Arch `mpv` headers without bindgen libclang | `cargo build` on the target machine | Install `clang` (section 6) |
| V-11 | souvlaki 0.8.3 `use_zbus` registers MPRIS on KDE Plasma | Run the app, check `playerctl -l` | Keep media keys off on KDE in P0 and note it in the release notes |
| V-12 | gtk4 0.11.5 with `v4_16` provides `gtk::FileDialog`, `gtk::AlertDialog`, `gtk::UriLauncher`, `gtk::GridView` (the APIs area:37 to area:47 use) | `cargo doc` for gtk4 0.11.5 | Upgrade to the next minor when the API is absent (keep the feature gate) |
| V-13 | wasmtime 49.0.2 compile time and binary size with the four features | Build `kuro-source-wasm` with `--release` once | Keep it behind the `wasm` feature (already the rule) |
| V-14 | glib version of gtk4 0.11.5 equals the glib that `glib-build-tools` 0.22.8 targets | `cargo tree -i glib` | Pin `glib-build-tools` to the matching line |
| V-15 | webkit6 0.6.1 compiles against Arch `webkit2gtk-6.0` | `cargo build -p kurobayomi --features webview` (if the feature is added; area:36 default) | Keep the browser fallback path (area:36) |
| V-16 | tokio-rusqlite 0.8.0 and rusqlite 0.40.2 share one `libsqlite3-sys` | `cargo tree -i libsqlite3-sys` | Pin both to the same rusqlite line |

---

## 12. Pitfalls specific to this map

1. Adding `time` through a transitive dependency (for example from `tracing-subscriber` features) and then calling `time::OffsetDateTime::now_local()` in a multithreaded process reintroduces the C-02 failure. The `chrono` clock is the only local-time source.
2. `reqwest` 0.12 feature names (`rustls-tls`, `rustls-tls-native-roots`) do not exist in 0.13. A copy-paste from an area doc fails to compile with "unknown feature".
3. `gtk4` 0.11.5 needs Rust 1.92, `avif-decode` 3.0.0 needs 1.98. The workspace `rust-version` is 1.98 (C-01), so `cargo +1.85 build` is not a supported path.
4. `icu_collator` 1.x and 2.x cannot be mixed in one binary (C-09). `cargo tree -d` lists duplicate ICU crates; fix before merging.
5. Adding `notify-rust` next to `gio::Notification` would post every notice twice on KDE (both reach `org.freedesktop.Notifications`). Only `gio::Notification` is used (C-04), so there is one path. A `cargo tree -i notify-rust` that finds it is a build error to fix.
6. `keyring` 3 and 4 have different APIs (`Entry::new` versus the facade). Code written from area:27 or area:47 snippets must be adapted to 4.2.0 (V-01).
7. Render-context calls for libmpv2 run inside the `GtkGLArea` render handler on the GTK main thread, as area:45 requires. The crate does not change that rule.
8. `chrono` with `default-features = false` and feature `clock` is required for `Local`. Dropping `clock` silently breaks the release window in area:26 (the code compiles and returns UTC).
9. Two copies of `serde` (1.0.229 and an older one pulled by a crate) make `#[derive(Serialize)]` types incompatible across crates. Run `cargo tree -d` after each `cargo add`.
10. The Fluent isolation marks (area:51 section 10, pitfall 1) apply to every `FluentBundle`; `set_use_isolating(false)` is mandatory for every bundle built in `kuro-i18n`.

---

## 13. Change control

Adding a crate that is not in section 3 requires a new row here first (ID `C-NN` if it conflicts, otherwise a new row in section 3), then the `cargo add`. Area docs cite rows by the crate name in section 3 or by the conflict ID in section 1.
