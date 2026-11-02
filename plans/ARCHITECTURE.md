# Kurobayomi target architecture

Status: synthesis document, written 2026-10-08 from the 54 area overviews, the 3,801 task rows in [TASKS.csv](TASKS.csv), the addendum in [addenda/requirements-cross-platform.md](addenda/requirements-cross-platform.md), and the existing mockup in `src/*.rs` and `spec/*.json`. Aniyomi source is read-only at `/home/blackbox/aniyomi-src`; every Kotlin path below is repo-relative to that directory. Rust paths are relative to `/home/blackbox/kurobayomi`.

Labels used throughout: **OBSERVED** = what the Aniyomi Kotlin does. **Proposal:** = what Kurobayomi should do. **Decided** = already accepted in an area overview. **User call** = needs the user's answer before the dependent task starts.

Where an area overview and this document disagree, this document wins, and section 3 lists every such case so the owning area can be corrected.

---

## 0. Reading order

1. Sections 1 to 3: decisions, conflicts resolved, and the target.
2. Sections 4 to 8: crate graph, dependency rules, async, UI state, navigation.
3. Sections 9 to 13: DI replacement, errors and logs, directories, scheduler, source boundary.
4. Sections 14 to 17: mockup evolution, build features, ownership map, phasing.
5. Section 18: open user calls.

Area links: each area id `area:NN-name` links to `plans/NN-name/00-overview.md`. Task IDs (`T-NNx-nnn`) are in [TASKS.csv](TASKS.csv) and in the per-area `p1-tasks.md` or `p2-tasks.md` files.

---

## 1. Decisions in force

| # | Decision | Why | Source |
|---|---|---|---|
| A1 | Rust 2024 edition, stable toolchain, Cargo workspace with the GTK application as a member package named `kurobayomi` at the repository root. | Matches the existing `Cargo.toml` (`edition = "2024"`). | [area:01](01-foundation-app-bootstrap/00-overview.md) section 2.2 |
| A2 | GUI is gtk4-rs 0.11 (feature `v4_14`) with libadwaita 1.5 (`adw`, feature `v1_5`); changed 2026-11-01 from "vanilla GTK 4, no libadwaita" after GTK-theme bugs on KDE (plans/LIBADWAITA-MIGRATION.md). | Requirement from the project brief; existing `Cargo.toml` line `gtk = { version = "0.11.5", package = "gtk4", features = ["v4_16"] }`. | brief, `Cargo.toml` |
| A3 | All non-GTK logic lives in `crates/kuro-*` crates that compile and test headless. Only the root package imports `gtk`, `glib`, `gio` widgets, `gdk`, or `webkit6`. | Headless unit tests for every area; the addendum needs platform code isolated. | [addenda/requirements-cross-platform.md](addenda/requirements-cross-platform.md) rules 1 to 3 |
| A4 | One process-wide tokio multi-thread runtime, owned by `kuro-app`, plus the GLib main context on the main thread. Tokio tasks never touch GTK. | Every area already assumes this split. | section 6 |
| A5 | No DI container. One `AppContext` struct built once at startup, with `OnceLock` fields for lazily created services. | Replaces Injekt. | [area:01](01-foundation-app-bootstrap/00-overview.md) D1, section 9 |
| A6 | Screens follow a Model-Update-View pattern. Pure `update` functions on the GTK thread, side effects as commands run on tokio, results return as messages. | Testable state; avoids callback spaghetti. | section 7 |
| A7 | Storage: SQLite through `rusqlite` 0.40 with the `bundled` feature. Two files, `manga.db` and `anime.db`. One writer connection, four readers, WAL. | [area:03](03-db-manga-schema-migrations/00-overview.md) D5, [area:04](04-db-anime-schema-migrations/00-overview.md) D3. | sections 4, 11 |
| A8 | Preferences: TOML files `config.toml` and `state.toml`, typed through `kuro-prefs`. | [area:02](02-preferences-system/00-overview.md) section 2.2. | section 11 |
| A9 | Video: libmpv through the render API into a `gtk::GLArea`, crate `libmpv2`. One mpv handle per player window. | [area:45](45-player-core/00-overview.md), D-3 and D-4. | section 4 |
| A10 | Sources are built-in Rust implementations first, declarative TOML scrapers second, WASM components third. Aniyomi APK extensions are not loaded. Source ids are frozen, so migrated libraries keep their entries. | [area:20](20-extension-strategy-redesign/00-overview.md) section 4.1. | section 13 |
| A11 | Every OS-specific call goes through traits in `kuro-platform`. No other crate uses `cfg(target_os)`, `zbus`, `libsecret`, `xdg`, or `XDG_*` environment variables. | Addendum rules 1 to 7. | section 5 |
| A12 | Every directory comes from the `Paths` trait. No path is built from a string literal outside `kuro-platform`. | Addendum rule 4. | section 11 |
| A13 | Message catalogs use Fluent (`fluent-bundle`), converted once from Android XML. | [area:51](51-i18n/00-overview.md) D1, D2. | section 4 |
| A14 | Backups use the Aniyomi wire format (protobuf inside gzip) so backups interoperate. Backups are the only data-transfer path from Aniyomi. | [area:21](21-backup-format-and-create/00-overview.md), [area:22](22-backup-restore/00-overview.md). | section 4 |
| A15 | Trackers use one `Tracker` trait, one HTTP client (`kuro-net`), credentials in `SecretStore`. | [area:27](27-tracking-core/00-overview.md). | section 13 |
| A16 | Reader and player are separate top-level windows, at most one of each open. Everything else is in the main window. | [area:53](53-platform-manifest-intents-workers/00-overview.md) D1, D2. | section 8 |

---

## 2. Summary of what is planned

Task counts from [TASKS.csv](TASKS.csv) (3,801 rows, export run on 2026-10-08):

| Priority | Count | Meaning in this project |
|---|---:|---|
| P0 | 1,196 | Needed to read manga and watch anime daily from one working source |
| P1 | 1,793 | Parity features most users rely on |
| P2 | 503 | Long tail and polish |
| P3 | 309 | Android-only or obsolete; each has a replacement or drop decision |

Every Aniyomi capability has a task, and each task is one of: port to Rust, replace with a named Linux-native equivalent, or drop with a reason. The P3 drop list is in each area's section 4 or 5.

---

## 3. Conflicts found across the plans, and the resolution

Each row is a defect in an area document. The owner must update the named document. Until then, this table governs.

| # | Topic | Conflicting statements | Resolution | Owner to correct |
|---|---|---|---|---|
| C1 | Crate prefix | `kb-*` names in [area:20](20-extension-strategy-redesign/00-overview.md) section 2.2 (`kb-source`, `kb-ext`, `kb-notify`, `kb-torrent`); `kuro-*` everywhere else. | All crates are `kuro-*`. `kb-ext` becomes `kuro-ext`, `kb-source-scrape` becomes `kuro-source-scrape`, `kb-sources-builtin` becomes `kuro-sources-builtin`, `kb-notify` is removed (notifications are `Notifier` in `kuro-platform`, section 4). | area:20 |
| C2 | Settings file name | `settings.toml` and `state.toml` in [area:01](01-foundation-app-bootstrap/00-overview.md) D6 and its data file; `config.toml` and `state.toml` in [area:02](02-preferences-system/00-overview.md) section 2.2 and the 02 data file. | `config.toml` (user preferences, owned by area:02) and `state.toml` (app state). The corrupt-file rule of area:02 applies (`config.toml.corrupt-<unix seconds>`). | area:01 |
| C3 | Database file names | `manga.db` and `anime.db` in areas 01, 03, 04; `tachiyomi.db` and `tachiyomi.animedb` in [area:05](05-data-repos-entries-items/00-overview.md) D2, with the claim that an Aniyomi database can be dropped in. | `manga.db` and `anime.db`. Area 05 D2 is withdrawn for the live file. Aniyomi files are never opened as live databases (see C4). | area:05 |
| C4 | Aniyomi file compatibility | [area:03](03-db-manga-schema-migrations/00-overview.md) D3 refuses any file with `application_id = 0` and routes it to the importer. Area 05 D2 says the file can be dropped in. | The importer is the only path for Aniyomi files (area:03 D3 and D9). Area 05 D2 is withdrawn. | area:05 |
| C5 | Crate that holds `AppContext` | [area:01](01-foundation-app-bootstrap/00-overview.md) puts `app_context.rs` in `kuro-core`. `AppContext` owns every service, so `kuro-core` would depend on every crate, which is a cycle. | `AppContext` and startup live in `kuro-app`, a crate with no GTK dependency that depends on every service crate. `kuro-core` keeps only leaf helpers. | area:01 |
| C6 | Migration runner location | [area:01](01-foundation-app-bootstrap/00-overview.md) puts the migrator in `kuro-core::migrate`. Migration steps touch `kuro-db` and `kuro-prefs`. | Migrator framework in `kuro-app::migrate`. Schema SQL steps in `kuro-db::migrate`. Preference steps in `kuro-prefs::legacy`. | area:01, area:11 |
| C7 | WASM priority | [area:20](20-extension-strategy-redesign/00-overview.md) T-20a-001 (WASM loader) is P0 while its own phase plan puts WASM in Phase 2. | Built-in sources and TOML scrapers are P0. WASM host is P1 (section 13). The priority change is a user call (section 18, Q3). | area:20 |
| C8 | WIT version rule | [area:20](20-extension-strategy-redesign/00-overview.md) T-20a-001 accepts only `1.x`. The topic document accepts major `1`, minor `0`, and rejects newer minors with `UnsupportedApi`. | Topic-document rule: major must equal `1`, minor must equal `0`, otherwise `UnsupportedApi`. | area:20 |
| C9 | Kotlin `lowercase()` in source ids | [area:20](20-extension-strategy-redesign/00-overview.md) Q6 (open). | Decision: `str::to_lowercase` (Unicode). Required for existing non-ASCII ids. Golden vectors for Turkish dotted I are in area:20 E12. | area:20 |
| C10 | Theme source of truth | Brief: "vanilla GTK so the user's GTK theme applies". Addendum rule 6: "the app ships its own Tokyonight CSS so the look does not depend on the host GTK theme". Memory note for this user: keep Tokyo Night everywhere. | Ship a bundled Tokyo Night CSS as the default palette (section 14). The GTK theme still provides widget metrics. A setting "Use system GTK theme colours" turns the bundled palette off. This is a user call (section 18, Q5). | area:49, addendum |
| C11 | Widgets versus "desktop equivalents" | [area:50](50-android-widgets/00-overview.md) is titled "Android home-screen widgets and their desktop equivalents" and has 24 P2 tasks. [area:01](01-foundation-app-bootstrap/00-overview.md) T-01a-024 and [area:53](53-platform-manifest-intents-workers/00-overview.md) D12 drop the widgets. | Widgets dropped. The desktop equivalents are the sidebar badge counts and the Updates tab (section 14). Area 50 keeps only the P2 tasks that implement those two, and its title is corrected to say so. | area:50 |
| C12 | Repository names in area:05 vs area:03 | [area:05](05-data-repos-entries-items/00-overview.md) puts repos in `kuro-data`; [area:03](03-db-manga-schema-migrations/00-overview.md) puts the connection and SQL in `kuro-db`. | Both crates exist. `kuro-db` owns connections, schema, SQL text. `kuro-data` owns typed repositories. Both may import `rusqlite`. No other crate may. | none (consistent) |
| C13 | Anime database schema module | [area:04](04-db-anime-schema-migrations/00-overview.md) does not name a crate. Area 03 names `kuro-db`. | Anime schema is `kuro-db::schema::anime`, next to `kuro-db::schema::manga`. Same connection runtime. | area:04 (naming only) |
| C14 | Notification categories | [area:01](01-foundation-app-bootstrap/00-overview.md) `kuro-core::notify` has the category table; [area:35](35-data-misc-updater-notification-export/00-overview.md) has `Notifier` in `kuro-platform`. | Category table in `kuro-core::notify` (data only). `Notifier` trait and the GIO implementation in `kuro-platform`. | area:35 (naming only) |
| C15 | Anime download path | [area:26](26-library-update-jobs/00-overview.md) D5: Aniyomi never calls `downloadEpisodes` for new episodes. | Wired with `autoStart = false`, as manga does. `download_new_episode` defaults to `false` (section 12). | area:26 (decided there) |
| C16 | Shared chapter and episode logic | [area:38](38-ui-entry-details-manga/00-overview.md) puts pure chapter logic in `kuro-chapters`. [area:39](39-ui-entry-details-anime/00-overview.md) has an analogous episode logic set with no crate. | `kuro-chapters` holds `chapter/` and `episode/` modules. Shared sort and filter helpers live in `kuro-chapters::common`. | area:39 (naming only) |
| C17 | Release panic strategy | [area:01](01-foundation-app-bootstrap/00-overview.md) D3 says release uses `panic = "abort"` after the hook spawns the reporter. The same area's build map (`p1-topic-build-and-dependency-map.md` section 1) sets `panic = "unwind"` for release. | `panic = "unwind"` in release. `contain()` (section 10.4) needs unwinding to catch panics in GTK callbacks and background tasks. The hook still reports before unwinding. | area:01 D3 |

---

## 4. Workspace and crate layout

### 4.1 Tree

```
kurobayomi/                            Cargo workspace root and the GTK package "kurobayomi"
  Cargo.toml                           [workspace] members = [".", "crates/*"]; resolver = "3"
  Cargo.lock                           committed (area:54)
  build.rs                             none in root; build identity lives in kuro-core
  src/                                 GTK application (layer 5)
    main.rs                            process entry (section 9.2)
    cli.rs                             argument parsing (area:53)
    shell/                             window, navigator, startup gate, lock gate (area:36)
    ui/                                screens, one module per screen (sections 14, 16)
    widgets/                           reusable GTK widgets; bind.rs for preference binders (area:02)
    theme/                             CSS provider, colour tokens (area:49)
    webview/                           WebKitGTK host implementing kuro_platform::WebViewHost (area:36)
    player_surface/                    GLArea + mpv render context (area:45, area:46)
    reader_surface/                    PagerView, webtoon view, tap zones (area:44)
    crash_window.rs                    reporter mode window (area:01)
  assets/                              CSS, symbolic SVG icons, bundled fonts (area:49b)
  spec/                                UI parity JSON (frozen fixtures, section 14)
  packaging/                           desktop file, metainfo, icons, Flatpak manifest (area:54)
  crates/
    kuro-platform/                     OS traits and Linux implementation (layer 0)
    kuro-core/                         leaf helpers, errors, time, ids, logging init, crash hook (layer 1)
    kuro-prefs/                        typed preferences (layer 1)
    kuro-testkit/                      dev-only fixtures (layer 1)
    kuro-db/                           SQLite runtime, schemas, SQL, migrations (layer 2)
    kuro-net/                          HTTP client, rate limit, cookies, Cloudflare (layer 2)
    kuro-source/                       source traits, models, filters, ids, manager types (layer 2)
    kuro-source-scrape/                TOML scraper evaluator (layer 2)
    kuro-source-local/                 local manga and anime sources (layer 2)
    kuro-player/                       libmpv2 wrapper, mpv options, key map, playlist logic (layer 2)
    kuro-track/                        Tracker trait, seven plus three tracker implementations (layer 2)
    kuro-images/                       image fetch, decode, cache, covers (layer 2)
    kuro-reader/                       viewer decisions: pager, webtoon, zoom, tap zones, preload (layer 2)
    kuro-data/                         typed repositories (layer 3)
    kuro-domain/                       models and interactors over repositories (layer 3)
    kuro-chapters/                     pure chapter and episode logic (layer 3)
    kuro-ext/                          extension registry, loader, install, trust, repo fetch (layer 3)
    kuro-ext-wasm/                     WASM component host, WIT world (layer 3, feature wasm-sources)
    kuro-sources-builtin/              built-in Rust sources (layer 3)
    kuro-downloads/                    download queue, downloader, provider, cache (layer 3)
    kuro-backup/                       backup create and restore, protobuf, gzip (layer 3)
    kuro-torrent/                      torrent metadata and TorrServer supervisor (layer 3, feature torrent)
    kuro-i18n/                         Fluent catalogs, locale list, date formatting (layer 1)
    kuro-app/                          composition root: AppContext, runtime, startup, migrator, jobs, interactors (layer 4)
```

### 4.2 Crate table

GTK column: "yes" means the crate may depend on `gtk4`, `glib`, `gio` widgets, or `webkit6`. Only the root package may be "yes". `gio` without widgets is allowed only in `kuro-platform`.

| Crate | Layer | GTK | Owner areas | Main dependencies (Proposal) | Replaces |
|---|---|---|---|---|---|
| `kuro-platform` | 0 | no (gio only, see column note) | 01, 02, 12, 13, 35, 36, 52, 53 | `directories`, `rustix`, `gio`, `zbus`, `keyring` (or `oo7`), `souvlaki`, `open`, `notify-rust` (Windows only), `sys-locale` | Android `Context` directories, `SecureActivityDelegate`, `NotificationManager`, `Intent`, `ShellUtil` |
| `kuro-core` | 1 | no | 01, 13, 14 (metadata), 35 (categories), 52 | `thiserror`, `time`, `tracing`, `serde`, `serde_json`, `futures`, `sha2`, `md-5`, `bencode` by hand, feature `archive` (libarchive FFI or `zip`) | `core/common` utilities, `DiskUtil`, `ImageUtil` metrics, `build.rs` for `AppInfo` |
| `kuro-prefs` | 1 | no | 02 | `serde`, `toml_edit`, `parking_lot`, `thiserror`, `tracing`, `atomic-write-file`, `fd-lock`, `chrono`, `kuro-platform` | `SharedPreferences`, `PreferenceStore` |
| `kuro-i18n` | 1 | no | 51 | `fluent-bundle`, `unic-langid`, `icu_datetime` (build-time table only) | `moko-resources`, `MR`/`AYMR` |
| `kuro-testkit` | 1 (dev) | no | 01, 02, 03 to 09 | `tempfile`, `proptest` | `test` sources |
| `kuro-db` | 2 | no | 03, 04 | `rusqlite` (`bundled`, `hooks`, `functions`, `backup`), `kuro-core` | SQLDelight, `AndroidSqliteDriver` |
| `kuro-net` | 2 | no | 12 | `reqwest` (`rustls-tls`, `cookies`, `gzip`, `brotli`, `json`), `hickory-resolver` (DoH), `governor` or a hand-written sliding window, `kuro-platform` (`NetworkStatus`) | OkHttp 5, `AndroidCookieJar`, `CloudflareInterceptor` |
| `kuro-source` | 2 | no | 15, 16, 17 (types) | `async-trait` or native `async fn` in traits, `serde`, `kuro-net` | `source-api` module |
| `kuro-source-scrape` | 2 | no | 20 | `scraper`, `serde`, `toml`, `kuro-source` | `ParsedHttpSource`, `Jsoup` |
| `kuro-source-local` | 2 | no | 14 | `zip` or `kuro-core::archive`, `image`, `quick-xml` (ComicInfo), `kuro-source` | local source, `ComicInfo.kt` |
| `kuro-player` | 2 | no | 45 (logic), 46 (logic) | `libmpv2`, `serde_json`, `kuro-core` | `MPVLib`, `AniyomiMPVView` logic |
| `kuro-track` | 2 | no | 27 to 33 | `reqwest` through `kuro-net`, `serde`, `oauth2` (PKCE), `kuro-platform` (`SecretStore`) | tracker classes, OAuth plumbing |
| `kuro-images` | 2 | no | 34 | `image` 0.25 (`jpeg`, `png`, `webp`, `gif`, `avif`), `jxl-oxide`, `kuro-net`, `kuro-core` | Coil, `ImageUtil`, `BitmapFactory` |
| `kuro-reader` | 2 | no | 43 (logic), 44 | `kuro-core`, `kuro-images` (decode outputs only) | `ReaderViewModel`, `PagerViewer` logic |
| `kuro-data` | 3 | no | 05, 06, 42 (queries) | `kuro-db`, `rusqlite` | `data/` repositories |
| `kuro-domain` | 3 | no | 07, 08, 09, 42 (models) | `kuro-data`, `kuro-core`, `kuro-prefs` | `domain/` module |
| `kuro-chapters` | 3 | no | 38, 39 | `kuro-core` | `Manga.kt` masks, chapter sort |
| `kuro-ext` | 3 | no | 18, 19, 20 | `kuro-source`, `kuro-net`, `kuro-data`, `kuro-platform`, `sha2`, `notify` (inotify) | `extension/` tree |
| `kuro-ext-wasm` | 3 | no | 20 | `wasmtime` with component model, `kuro-source` | none (new) |
| `kuro-sources-builtin` | 3 | no | 20 (list from Q2) | `kuro-source`, `kuro-source-scrape` | Aniyomi source packages (none ported yet) |
| `kuro-downloads` | 3 | no | 23, 24, 25 | `kuro-net`, `kuro-data`, `kuro-source`, `kuro-platform`, `kuro-torrent` (feature) | `download/` tree |
| `kuro-backup` | 3 | no | 21, 22 | `prost` (protobuf), `flate2` (gzip), `kuro-data`, `kuro-prefs` | `BackupCreator`, `BackupRestorer`, protobuf |
| `kuro-torrent` | 3 | no | 13 (torrent), 24 (torrent path) | `lava_torrent` or hand-written bencode, `kuro-net`, `kuro-platform` (process supervision) | `torrentutils`, `TorrServer` AAR |
| `kuro-app` | 4 | no | 01 (startup), 10, 11, 17, 26, 35 (notifications, updater, export), 43 (session) | every crate above except GTK, `tokio`, `tracing-appender`, `async-channel` | Injekt modules, `App.kt`, `MainActivity` service wiring, WorkManager |
| `kurobayomi` (root) | 5 | yes | 36, 37 to 42, 44 (surface), 46, 47, 47b, 48, 49, 49b, 53 (GTK side), 50 (dropped), 52 (GTK helpers) | `gtk4` 0.11 (`v4_16`), `glib`, `gio`, `gdk4`, `webkit6`, `kuro-app` | `ui/`, `presentation/`, Compose |

Crate names that no area overview fixes are assigned in this document and must be confirmed by the owning area: `kuro-backup` (areas 21, 22), `kuro-torrent` (areas 13, 24), `kuro-images` (area 34), `kuro-reader` (area 44), `kuro-player` (area 45), `kuro-chapters` (areas 38, 39), `kuro-sources-builtin` (area 20), `kuro-i18n` (area 51). `kuro-downloads` is named by area 23. No other new crates are introduced.

### 4.3 Concrete feature set

| Feature (workspace or crate) | Default | Effect | Source |
|---|---|---|---|
| `platform-linux` on `kuro-platform` | on | Linux implementations | addendum |
| `platform-windows`, `platform-macos` on `kuro-platform` | off | Later ports; compile only | addendum rule 2 |
| `updater` on `kuro-app` | off | self-update check, compiled out | [area:01](01-foundation-app-bootstrap/p1-topic-build-and-dependency-map.md) section 1.2, [area:35](35-data-misc-updater-notification-export/00-overview.md) |
| `archive` on `kuro-core` | on | libarchive readers | [area:13](13-core-util-storage-archive-torrent/00-overview.md) D-A |
| `archive-zip-only` on `kuro-core` | off | reduced format set without libarchive | [area:13](13-core-util-storage-archive-torrent/00-overview.md) D-A |
| `legacy-import` on `kuro-db` | on | replay of Aniyomi migrations 1 to 32 for the importer | [area:03](03-db-manga-schema-migrations/00-overview.md) section 2.2 |
| `heif` on `kuro-images` | off | libheif-based HEIF decode | [area:44](44-reader-viewers/00-overview.md) D4 |
| `wasm-sources` on `kuro-ext` | off until P1 | pulls `kuro-ext-wasm` | [area:20](20-extension-strategy-redesign/00-overview.md) C7 |
| `torrent` on `kuro-downloads` and `kuro-app` | off | torrent download path and TorrServer supervisor | [area:20](20-extension-strategy-redesign/00-overview.md) Q9 |
| `webview` on root | on | WebKitGTK host; without it the `WebViewHost` falls back to the system browser | addendum, [area:12](12-network-stack/00-overview.md) test gate `KURO_WEBKIT_TESTS` |
| `live-network-tests` on `kuro-net`, `kuro-track` | off | tests that contact real hosts | [area:12](12-network-stack/00-overview.md) |

---

## 5. Dependency rules

These rules hold for every crate. Each rule has a CI check (grep-based) that fails the build.

| # | Rule | CI check |
|---|---|---|
| R1 | Only the root package depends on `gtk4`, `glib` (widgets), `gdk4`, `webkit6`. `kuro-platform` may depend on `gio`. | `cargo tree -e normal -i gtk4` lists only `kurobayomi` |
| R2 | Only `kuro-platform` contains `cfg(target_os`, `zbus`, `libsecret`, `keyring`, `xdg`, `XDG_`, `xdg-open`, `Command::new("xdg-open")`. | `grep` over `crates/` and `src/` excluding `crates/kuro-platform` |
| R3 | Only `kuro-db` and `kuro-data` depend on `rusqlite`. | `cargo tree -i rusqlite` |
| R4 | Only `kuro-net` constructs a `reqwest::Client`. Every other crate receives a `kuro_net::Client` handle. | `grep -r "reqwest::Client::new\|ClientBuilder::new"` outside `kuro-net` returns nothing |
| R5 | Domain crates (`kuro-domain`, `kuro-chapters`) do not depend on `kuro-net`, `kuro-platform`, `gtk4`. | `cargo tree -p kuro-chapters` |
| R6 | Sources and extensions depend on `kuro-source` and `kuro-net` only. They never depend on `kuro-db`, `kuro-data`, `kuro-domain`. | `cargo tree -p kuro-sources-builtin` |
| R7 | `kuro-app` is the only crate that depends on all services. No service crate depends on `kuro-app`. | `cargo tree -i kuro-app` lists only `kurobayomi` |
| R8 | No `static mut`, no `lazy_static`, no `once_cell`. Use `std::sync::{OnceLock, LazyLock}` and `std::cell::{OnceCell, LazyCell}`. | `grep` |
| R9 | No `unwrap()` or `expect()` on data read from disk, the network, or the database outside tests. | clippy lint `unwrap_used` on library crates |
| R10 | Every user-visible string goes through `kuro-i18n`. Literal English in `src/` is allowed only for debug-only labels. | `grep` for string literals in `src/ui` |

Layer order (a crate may depend only on crates in a lower layer, or on same-layer crates where the table says so):

```
layer 5  kurobayomi (GTK)
layer 4  kuro-app
layer 3  kuro-data, kuro-domain, kuro-chapters, kuro-ext, kuro-ext-wasm, kuro-sources-builtin,
         kuro-downloads, kuro-backup, kuro-torrent
layer 2  kuro-db, kuro-net, kuro-source, kuro-source-scrape, kuro-source-local, kuro-player,
         kuro-track, kuro-images, kuro-reader
layer 1  kuro-core, kuro-prefs, kuro-i18n, kuro-testkit (dev)
layer 0  kuro-platform
```

Within layer 3, `kuro-data` must not depend on `kuro-domain`. `kuro-domain` depends on `kuro-data` because repositories are concrete types (area:05 D1).

---

## 6. Async model

### 6.1 Runtimes

| Runtime | Owner | Thread model | Used for |
|---|---|---|---|
| tokio multi-thread runtime | `kuro-app::runtime::Runtime`, created in `main` before `gtk::Application::run` | default worker count (tokio default); one runtime per process | network, database (through `tokio-rusqlite`-style dedicated threads), downloads, jobs, decoding via `spawn_blocking` |
| GLib main context | GTK, the main thread | one thread | every widget, every `Model` update, `gio::Notification`, mpv render callback handoff, WebKit |
| dedicated OS threads | `kuro-db` | one thread per SQLite connection (1 writer, 4 readers) | blocking SQLite calls |
| mpv core thread | libmpv | created by libmpv | playback; only `render_context` calls on the GL thread |

Area evidence: [area:05](05-data-repos-entries-items/00-overview.md) D5 (watch channels), [area:04](04-db-anime-schema-migrations/00-overview.md) D3 (one writer, four readers), [area:01](01-foundation-app-bootstrap/00-overview.md) D4 (migrations run before the first window).

### 6.2 Bridge code

Only these helpers may cross between the two executors. They live in `kuro-app::bridge` (no GTK) and in the root crate `src/shell/bridge.rs` (GTK side).

```rust
// kuro-app/src/runtime.rs (no GTK)
pub struct Runtime { rt: tokio::runtime::Runtime }
impl Runtime {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self { rt: tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("kuro-worker").build()? })
    }
    pub fn handle(&self) -> tokio::runtime::Handle { self.rt.handle().clone() }
}
```

```rust
// src/shell/bridge.rs (GTK side)
use std::future::Future;
pub fn spawn_ui<T, F, C>(handle: &tokio::runtime::Handle, work: F, on_done: C)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    C: FnOnce(T) + 'static,
{
    let join = handle.spawn(work);
    glib::spawn_future_local(async move {
        match join.await {
            Ok(value) => on_done(value),
            Err(e) if e.is_panic() => tracing::error!(target: "kuro::ui", "background task panicked: {e}"),
            Err(_) => {}
        }
    });
}
```

`tokio::task::JoinHandle` is a plain `Future` and can be awaited from the GLib executor. No `Runtime::enter` call is needed on the GTK thread.

Streams from the database: [area:05](05-data-repos-entries-items/00-overview.md) D5 and [area:03](03-db-manga-schema-migrations/00-overview.md) `runtime.rs` use `tokio::sync::watch` values. The GTK side runs one `glib::spawn_future_local` task per subscription that loops on `watch.changed().await`, reads `watch.borrow().clone()` after each change, and sends the value to the screen's message queue (section 7.1). `watch` coalesces bursts, which matches the SQLDelight conflated behaviour.

Cancellation: every long task takes a `tokio_util::sync::CancellationToken`. A screen that closes cancels its tokens in `Drop` of its model owner. Downloads, library passes, backups and imports are cancelled from the queue UI and persist their partial state (area:23, area:25).

### 6.3 Concurrency limits (single source)

| Limit | Value | Owner | Source |
|---|---|---|---|
| image fetch concurrency | 8 | kuro-images | [area:34](34-images-covers-cache/00-overview.md) |
| image decode concurrency | 3 | kuro-images | [area:34](34-images-covers-cache/00-overview.md) |
| extension install | 1 (one-permit semaphore) | kuro-ext | [area:20](20-extension-strategy-redesign/00-overview.md) decision 6 |
| library pass per kind | 5 (Kotlin value) | kuro-app::library_update | [area:26](26-library-update-jobs/00-overview.md) D7 |
| library pass per source | 1 (process-wide permit) | kuro-app::library_update | [area:26](26-library-update-jobs/00-overview.md) D7, Proposal accepted |
| read connections | 4 | kuro-db | [area:04](04-db-anime-schema-migrations/00-overview.md) D3 |
| `busy_timeout` | 5000 ms | kuro-db | [area:04](04-db-anime-schema-migrations/00-overview.md) D3 |
| `IN` list chunk size | 500 (anime), `json_each` rewrite (manga) | kuro-db | [area:04](04-db-anime-schema-migrations/00-overview.md) D5, [area:03](03-db-manga-schema-migrations/00-overview.md) D6 |
| decoded page cache | 512 MiB default, byte-budgeted LRU | kuro-reader | [area:44](44-reader-viewers/00-overview.md) D5 |
| HTTP connect timeout | 30 s | kuro-net | [area:12](12-network-stack/00-overview.md) section 1.1 |
| HTTP read timeout | 30 s | kuro-net | [area:12](12-network-stack/00-overview.md) section 1.1 |
| HTTP whole-call timeout | 120 s | kuro-net | [area:12](12-network-stack/00-overview.md) section 1.1 |
| HTTP response cache | 5 MiB | kuro-net | [area:12](12-network-stack/00-overview.md) section 1.1 |

---

## 7. State management (MVU)

### 7.1 Pattern

Every screen with state is a Model-Update-View triple.

```rust
pub trait Screen: 'static {
    type Model: 'static;
    type Msg: Send + 'static;
    type Widgets;

    fn init(ctx: &AppContext, arg: Self::Args) -> (Self::Model, Vec<Cmd<Self::Msg>>);
    fn update(model: &mut Self::Model, msg: Self::Msg) -> Vec<Cmd<Self::Msg>>;   // pure, no I/O
    fn view(model: &Self::Model, widgets: &Self::Widgets);                       // GTK only, no I/O
    fn subscriptions(ctx: &AppContext, model: &Self::Model) -> Vec<Sub<Self::Msg>>;
    type Args;
}

pub enum Cmd<M> {
    Task(futures::future::BoxFuture<'static, M>),   // run on tokio, result delivered as M
    Nothing,
}

pub struct Sub<M> { pub id: &'static str, pub stream: futures::stream::BoxStream<'static, M> }
```

Rules:

1. `update` never calls a service. It only changes the model and returns commands.
2. `view` never calls a service and never blocks. It diffs the model against the last rendered model and updates only what changed. Lists use `gio::ListStore::splice`.
3. Messages are queued in an `async_channel::Receiver<M>` consumed by one `glib::spawn_future_local` loop per screen. Messages are handled one at a time, so no re-entrancy.
4. A `Sub` is a stream of messages. The screen keeps the `JoinHandle` of its subscription tasks and aborts them when the screen is popped.

### 7.2 Why this shape

| Concern | Pattern in Kotlin | Pattern here |
|---|---|---|
| Screen state | `ScreenModel` with `MutableStateFlow` | `Model` struct, changed only by `update` |
| Side effects | a coroutine launched in the screen model scope | `Cmd::Task` |
| Repeated reads | `Flow` collected in ScreenModel | `Sub` backed by `watch` |
| Reader and player state | `ViewModel` | `Model` plus mpv event subscription |

The existing mockup builds widgets in functions (`src/entry.rs::entry`, `src/screens.rs::library`). These become `init` and `view` pairs (section 14).

### 7.3 Reader and player state machines

Reader and player are two `Screen` implementations each with a state enum in the model:

* Reader: `Loading`, `Ready { page: usize }`, `Error(ReaderError)`. Transitions are defined in [area:43](43-reader-core/00-overview.md).
* Player: `Idle`, `Loading`, `Playing`, `Paused`, `Ended`, `Error(PlayerError)`. Transitions are defined in [area:45](45-player-core/00-overview.md).

The state enums are pure Rust in `kuro-reader` and `kuro-player`. The GTK screen only maps them to widgets.

---

## 8. Navigation

### 8.1 Route model

`kuro-app` has no navigation. Navigation is in `src/shell/nav.rs`, consistent with [area:36](36-ui-shell-navigation-security-webview/00-overview.md) section 2.2.

```rust
pub enum Route {
    Home(Tab),                       // Library, Updates, History, Browse, More (area:36)
    EntryDetails { kind: Kind, entry_id: i64 },
    Browse { kind: Kind, source_id: Option<i64> },
    GlobalSearch { kind: Kind, query: String },
    Settings(SettingsPage),          // ids from spec/settings.json
    Reader { chapter_id: i64 },      // separate window (A16)
    Player { episode_id: i64 },      // separate window (A16)
    Webview { url: String, source_id: Option<i64> },
    Stats, Storage, Upcoming, Downloads, Categories,
}
```

`Navigator { stack: Vec<Route> }` with `stack[0] == Route::Home(_)` as an invariant (area:36). Back pops one route. Escape and Alt+Left go back (from the mockup `src/main.rs`).

### 8.2 Windows

| Window | Count | Opened by | Closed by |
|---|---|---|---|
| main `ApplicationWindow` | 1 | activation | last window closed when the tray or background mode is off |
| reader window | at most 1 | `Route::Reader` | close or Escape |
| player window | at most 1 | `Route::Player` | close or Escape |
| modal sheets and dialogs | any | screens | their own close |
| crash reporter window | only in `--crash-report` process | crash hook | close |
| unlock overlay | 1 | app lock state machine | unlock |

Single instance: a second launch with a deep link or file argument forwards the request to the running process through `gio::Application` (area:53 D1, D2). The `DeepLink` type lives in `kuro-core::deep_link` (area:01).

### 8.3 Responsive layout

From the mockup: sidebar visible when window width is at least 760 px or maximised; below that a bottom `StackSwitcher`. Reader and player hide both (mockup `src/main.rs` `refresh`). Keep the same thresholds (area:36 `layout.rs`).

---

## 9. Dependency injection replacement

### 9.1 AppContext

```rust
// kuro-app/src/context.rs
pub struct AppContext {
    pub info: &'static kuro_core::AppInfo,
    pub paths: Arc<dyn kuro_platform::Paths>,
    pub secrets: Arc<dyn kuro_platform::SecretStore>,
    pub notifier: Arc<dyn kuro_platform::Notifier>,
    pub opener: Arc<dyn kuro_platform::Opener>,
    pub prefs: Arc<kuro_prefs::Prefs>,
    rt: tokio::runtime::Handle,
    manga_db: OnceLock<Arc<kuro_db::Db>>,
    anime_db: OnceLock<Arc<kuro_db::Db>>,
    net: OnceLock<kuro_net::Client>,
    sources: OnceLock<Arc<kuro_source::SourceManager>>,
    extensions: OnceLock<Arc<kuro_ext::ExtensionRegistry>>,
    trackers: OnceLock<Arc<kuro_track::TrackerManager>>,
    images: OnceLock<Arc<kuro_images::ImagePipeline>>,
    downloads: OnceLock<Arc<kuro_downloads::DownloadManager>>,
    jobs: OnceLock<Arc<jobs::Scheduler>>,
}
```

Accessors are methods named after the Kotlin Injekt names, lower snake case: `ctx.manga_db()`, `ctx.anime_db()`, `ctx.network()`, `ctx.sources()`, `ctx.extensions()`, `ctx.trackers()`, `ctx.images()`, `ctx.downloads()`, `ctx.jobs()`. Each accessor is `self.x.get_or_init(|| build_x(self))`. Accessors that can fail return `Result` and are initialised in the startup sequence before the first window is shown.

Area evidence: [area:01](01-foundation-app-bootstrap/00-overview.md) section 2.3 and D1. Each accessor must be callable first on a fresh context (T-01a-003).

### 9.2 Process start order

`main()` in `src/main.rs`:

1. Parse CLI (`cli.rs`). `--crash-report <file>` jumps to the reporter path (section 10.4).
2. `kuro-platform::linux::LinuxPaths::new()` resolves XDG directories.
3. Create directories (`config`, `state`, `data`, `cache`, `logs`, `crashes`). Failure: print to stderr and exit with code 1.
4. Install logging (`tracing-subscriber` with reloadable `EnvFilter`, `tracing-appender` daily rotation, 7 files kept).
5. Install the panic hook (section 10.4).
6. Load `config.toml` and `state.toml` through `kuro-prefs` (corrupt file: rename and use defaults).
7. Build the tokio runtime.
8. Build `AppContext`.
9. Run the migrator (`kuro-app::migrate::run_blocking`) before the first window (area:01 D4). The migrator runs the Kurobayomi steps, plus the importer prompt when an Aniyomi file is found (area:03 D3).
10. Create `gtk::Application` with id `io.github.anas1412.Kurobayomi` and call `app.run()`.
11. `connect_activate`: build the shell, show the window, then run warm-up.

Warm-up (area:01 D5): after the first frame, one service per GLib idle callback, in this order: `manga_db`, `anime_db`, `network`, `sources`, `extensions`, `trackers`, `images`, `downloads`, `jobs`. Warm-up never blocks the main thread for more than one service.

### 9.3 Testing

Tests construct `AppContext` with `kuro-testkit::TempPaths`, an in-memory `MemoryBackend` for prefs, and fake secret and notifier implementations. No test needs GTK.

---

## 10. Error and logging strategy

### 10.1 Error types

| Layer | Type | Crate |
|---|---|---|
| platform | `PlatformError` (variant per trait) | `kuro-platform` |
| prefs | `PrefError` | `kuro-prefs` |
| db | `DbError` (`Sqlite`, `Constraint`, `TooNew`, `LegacyFile`, `Corrupt`, `Io`, `NotFound`) | `kuro-db` ([area:03](03-db-manga-schema-migrations/00-overview.md) `error.rs`) |
| data | `DataError` | `kuro-data` ([area:05](05-data-repos-entries-items/00-overview.md) D6) |
| network | `NetError` (`Http { status }`, `Timeout`, `Dns`, `Cloudflare`, `Tls`, `Io`), message format `HTTP error <code>` | `kuro-net` ([area:12](12-network-stack/00-overview.md)) |
| source | `SourceError` | `kuro-source` ([area:20](20-extension-strategy-redesign/00-overview.md)) |
| extension | `ExtError` with `InstallStep` | `kuro-ext` |
| track | `TrackError` | `kuro-track` ([area:32](32-tracker-mangaupdates-simkl/00-overview.md) `error.rs`) |
| backup | `BackupError` | `kuro-backup` |
| player | `PlayerError` | `kuro-player` |
| app | `anyhow::Error` only at `kuro-app` service boundary and in `main` | `kuro-app` |

Rules: library crates use `thiserror`. `anyhow` appears only in `kuro-app` boundary functions and `main`. Panics are bugs; the panic hook reports them.

### 10.2 User-visible messages

Errors reach the user as message keys in `kuro-i18n`. Keys named in the plans so far: `source_not_installed` ([area:20](20-extension-strategy-redesign/00-overview.md) T-20a-095), `ext_install_checksum_mismatch` ([area:20](20-extension-strategy-redesign/00-overview.md) B5), `extension_api_error` ([area:20](20-extension-strategy-redesign/00-overview.md) E3.2), and `requires_app_restart` ([area:51](51-i18n/00-overview.md) D5). Missing key: return the id and log one warning per key (area:51 D13).

Presentation: a toast (about 2 s and 3.5 s, one at a time, never takes focus; [area:13](13-core-util-storage-archive-torrent/00-overview.md) D-G) or an inline banner (area:36 `banners.rs`). A desktop notification is used only for background work (area:35).

### 10.3 Logging

* Facade: `tracing`. Targets: `kuro::<crate>` (for example `kuro::db`, `kuro::net`, `kuro::ext`), and `kuro::ui` for the GTK side.
* Levels: `ERROR` for failed user actions and panics; `WARN` for recoverable data problems (corrupt config, missing key); `INFO` for lifecycle events (startup, migration, extension load, download completion); `DEBUG` and `TRACE` for diagnosis.
* Default filter: `INFO`. `RUST_LOG` overrides. Aniyomi's default of `verbose_logging = BuildConfig.DEBUG` is not kept; release builds default to off (area:01 Q3, recommended, user call section 18 Q-logging).
* File: `$XDG_STATE_HOME/kurobayomi/logs/kurobayomi.log`, daily rotation, 7 files kept (section 11).
* Sensitive data: request URLs have query strings removed before logging; tokens and passwords are never logged (area:27, area:12).

### 10.4 Crash handling

* Panic hook installed in `kuro-core::crash` (area:01 section 2.3). It writes `crashes/crash-<unix_ms>.txt`, then spawns the same binary with `--crash-report <file>`.
* The reporter window (`src/crash_window.rs`) shows the report, copies it to the clipboard, and reveals the file. There is no share sheet (area:01 D7).
* Release profile uses `panic = "unwind"` (section 15). The hook does all reporting before returning. Panics inside GTK callbacks are caught by a `contain()` wrapper so one callback panic does not abort the process.
* Crash report contents: version, commit, build type, OS, backtrace, and the names and repository URLs of installed extensions. No library data (area:01 Q4, recommended, user call section 18).

---

## 11. Config, data, cache and XDG layout

All directories come from `kuro-platform::Paths`. Defaults follow the XDG base directory specification. `XDG_*` variables are honoured.

| Path | Default location | Contents | Owner |
|---|---|---|---|
| config | `$XDG_CONFIG_HOME/kurobayomi/` (`~/.config/kurobayomi/`) | `config.toml` (user preferences, `schema = 1`) | area:02 |
| state | `$XDG_STATE_HOME/kurobayomi/` (`~/.local/state/kurobayomi/`) | `state.toml` (app state, including the scheduler's `[jobs]` next-run times, section 12), `logs/kurobayomi.log[.YYYY-MM-DD]` (7 kept), `crashes/crash-<unix_ms>.txt` | area:01, area:02, section 12 |
| data | `$XDG_DATA_HOME/kurobayomi/` (`~/.local/share/kurobayomi/`) | `manga.db`, `anime.db` (+ `-wal`, `-shm`), `backups/` default, `extensions/` (installed packages, section 13), `torrents/` (feature `torrent`) | area:03, area:04, area:18, area:19 |
| cache | `$XDG_CACHE_HOME/kurobayomi/` (`~/.cache/kurobayomi/`) | `covers/`, `pages/`, `http/` (HTTP response cache, 5 MiB), `crash/kurobayomi_crash_logs.txt` (the dump used by "copy logs") | area:34, area:12, area:01 |
| downloads (default root) | `$XDG_DOWNLOAD_DIR/Kurobayomi/` | manga and anime download folders, user may choose another folder through the portal file chooser | area:13 D-H, area:53 D10 |
| temp | `$XDG_RUNTIME_DIR` when present, else `std::env::temp_dir()/kurobayomi/` | install temp files (`extensions/.tmp/`), torrent scratch | area:01, addendum rule 4 |

Rules:

1. Writes to `config.toml` and `state.toml` are atomic: write to a temp file in the same directory, `fsync`, rename (`atomic-write-file`). Flush is scheduled 400 ms after a change (area:02).
2. A corrupt `config.toml` is renamed to `config.toml.corrupt-<unix seconds>` and defaults are used (area:02).
3. Database files are created with mode `0600`. Config and state files with `0600`. Directories with `0700` (Proposal; secrets and private library data).
4. Download folder names are sanitised by `kuro-core::fs_name::sanitize` (addendum rule 5): forbidden characters `<>:"/\|?*`, reserved names `CON PRN AUX NUL COM1-9 LPT1-9` get a leading `_`, trailing dots and spaces removed, 240 UTF-16 unit cut moved back one unit when it splits a surrogate pair (area:13 D-D, D-E).
5. Secrets (tracker tokens, source credentials, optional app lock passphrase hash) live only in `SecretStore`. The config file stores only a reference name.

---

## 12. Background job scheduler

### 12.1 Model

`kuro-app::jobs::Scheduler` owns a list of `JobSpec { id, kind, interval, constraints, next_run }`. Next-run times persist in `state.toml` under `[jobs]`. The scheduler is a tokio task that sleeps until the nearest `next_run` and wakes on a `Notify` when a spec changes.

Job kinds:

| Kind | Interval source | Constraints | Owner area |
|---|---|---|---|
| library update (manga) | `library_update_interval` preference | outside release period restriction, network connected | area:26 |
| library update (anime) | same preference family, per kind | same | area:26 |
| metadata refresh (manga, anime) | per-kind | separate unique names `MetadataUpdate-manga`, `MetadataUpdate-anime` (area:26 D9) | area:26 |
| auto-backup | preference | none | area:21 |
| extension repo refresh | 1 day throttle (area:20 update rule) | network connected | area:20, area:18, area:19 |
| download queue resume | on start | none | area:23, area:24, area:25 |

Restrictions and rules (area:26 decisions):

* Restrictions also apply to manual passes (D8, parity). The settings help text says so.
* Missed runs while closed are not replayed. The next run starts at the next launch if due (D3).
* Battery constraint is not enforced on desktop (D4). Documented deviation.
* Manga and anime passes never run one source at once (D7, process-wide per-source permit 1).
* Anime favourite recheck happens before the sync (D6).
* New episodes are wired to download with `autoStart = false` (D5, C15). `download_new_episode` defaults to `false`.
* Tap on a new-item notification opens the highest-numbered new item (D10).

### 12.2 Running while the window is closed

Two mechanisms (area:26 D1):

* A (default): the process stays resident in the background with the `org.freedesktop.portal.Background.RequestBackground` portal. Autostart is requested through the portal.
* B (opt-in): a systemd user timer `kurobayomi-update.timer` runs `kurobayomi --library-update=auto`. A and B never run together.

Without a portal or systemd, the scheduler runs only while the app is open. This is shown in settings.

### 12.3 Notifications

Each job posts through `kuro-platform::Notifier` with a category from `kuro-core::notify` (C14). Actions are GIO button targets (`app.notice` with a JSON `AppAction`), so no request-code collisions (area:53 D3). Dismissal withdraws one id only (area:53 D4).

---

## 13. Plugin and source boundary

### 13.1 Three source kinds

| Kind | Crate | Phase | How it is loaded | Trust |
|---|---|---|---|---|
| built-in Rust | `kuro-sources-builtin` | P0 | compiled in; registered in `kuro-app::sources` | part of the binary |
| declarative TOML scraper | `kuro-source-scrape` | P0 or P1 (area:20 Q2 decides) | `<data>/extensions/<name>/manifest.toml` and selectors | repo key set and per-version trust rows (area:20 decision 5) |
| WASM component | `kuro-ext-wasm` | P1 (C7) | `<data>/extensions/<pkg>/source.wasm` | repo sha256 plus repo signing key; `allowed_hosts` in manifest (area:20 Q8) |

Rejected for the primary path (area:20 section 4.1): JVM bridge, Lua, JS as primary, sidecar processes, APK loading (area:20 decision 2).

### 13.2 Trait surface

`kuro-source` defines:

* `MangaSource` and `CatalogueSource` (area:15), `AnimeSource` and `AnimeCatalogueSource` (area:16), with the models `SManga`, `SChapter`, `Page`, `MangasPage`, `SAnime`, `SEpisode`, `Hoster`, `Video`, `Track`, `TimeStamp`, `AnimesPage`, `FetchType`, `UpdateStrategy`, and filters `Filter`, `FilterList`, `TriState`, `SortSelection`.
* `SourceError` variants used by reader and player (area:20 section 5.2).
* `source_id(name: &str, lang: &str, version_id: i64) -> i64` in `kuro-source::id`.
* `SourceManager` (area:17) with `get` and `get_or_stub`, and `SourceRef::{Live, Stub}`.

Overlaps resolved (area:20 section 5.3): `MangaSource` trait file is owned by area:15. The anime trait and the shared model are owned by area:20. `SourceManager` queries are owned by area:17, while stub creation and the id lookup are owned by area:20.

### 13.3 Source ids

Frozen (area:20 decision 3): `source_id = i64::MAX & first 8 bytes of md5("<name.to_lowercase()>/<lang>/<versionId>")` read big-endian. Default `versionId = 1` (`HttpSource.kt:45`). Kotlin reference: `source-api/src/commonMain/kotlin/eu/kanade/tachiyomi/source/online/HttpSource.kt:87-90` and `source-api/src/commonMain/kotlin/eu/kanade/tachiyomi/animesource/online/AnimeHttpSource.kt:101`.

Missing sources: stub rows `(id, lang, name)` keep entries, chapters and history, disable reader, search and refresh, and show `source_not_installed` (area:20 decision 4). Installing the source later re-activates the same id.

### 13.4 Extension registry and install

Registry: `kuro-ext::registry::ExtensionRegistry` with watch channels for installed, available and untrusted packages, and an `arc-swap`-style source table (area:20 section 2.2).

Install: one permit at a time. Download into `<data>/extensions/.tmp/`, verify sha256 when the repo gives one, rename into `<data>/extensions/<pkg>/`. Cancel deletes the temp file. Downgrades and key changes are refused (area:20 decision 5).

Store formats: JSON `index.min.json` first (area:20 Q11). Gzip is cheap to add. Protobuf is P2 (T-19a-014). `repo.json` without the new index is rejected with a clear message. Aniyomi APK entries show as "unsupported format" (area:20 Q12).

Repo list lives in SQLite `extension_repos` (area:20 Q5, area:03). `repos.toml` is dropped.

### 13.5 WASM host (P1)

WIT world `kurobayomi:source@1.0.0`, file `crates/kuro-ext-wasm/wit/source.wit`. Host imports are `http` (restricted to manifest `allowed_hosts`) and `settings`. Limits (fuel, epoch, memory caps) are defined in area:20's WASM design document. Version check: major `1`, minor `0`; otherwise `UnsupportedApi` (C8).

### 13.6 Source-facing HTTP

Every source request goes through `kuro-net` (R4), with per-host rate limits from the source definition (`rateLimit`, `rateLimitHost` in area:12). Cloudflare challenge solving uses `WebViewHost` (section 4.2 root crate, feature `webview`). Without WebKit, the challenge opens in the system browser through `Opener` (addendum, `WebViewHost` fallback).

---

## 14. Mockup evolution

The mockup is GTK code driven by JSON specs. It is a parity reference and is kept until each screen has a real implementation. The table gives the disposition of each file.

| File (lines) | Current role | Disposition |
|---|---|---|
| `src/main.rs` (237) | `main`, `build_ui`, the sidebar, tabs, immersive hide logic, key controller | `main.rs` keeps only process start (section 9.2). `build_ui` becomes `shell::build`. Sidebar and tab bar move to `src/shell/`. The `refresh` closure becomes a layout function in `shell/layout.rs` driven by window width. |
| `src/ui.rs` (525) | Global nav (`nav_init`, `window()`, `push`, `pop`, `pop_to_root`), `page`, `item_row`, `show_sheet`, `show_dialog`, `resolve(id, label)` string navigation, `toolbar`, `tag`, `owner_screen` | Globals removed. The `Shell` handle is passed to every screen. `resolve` by string becomes `Route` values. `item_row` moves to `src/widgets/spec_row.rs`. Dialogs and sheets become `Route` variants or a modal stack in `shell/modal.rs`. Tagging (`tag`, `owner_screen`) is replaced by typed widget references. |
| `src/screens.rs` (545) | Hand-built screens (`libraries`, `library`, `updates`, `history`, `more`, `downloads`, `categories`, `stats`, `storage`, `info_screen`) with `Built` values | Each becomes `src/ui/<screen>.rs` with `Model`, `Msg`, `init`, `update`, `view` (section 7). `libraries` and `library` go to `ui/library`. `updates` and `history` go to `ui/updates` and `ui/history`. `stats`, `storage` and `downloads` keep their names. Sample rows are replaced by data from `kuro-domain`. |
| `src/entry.rs` (585) | `entry` details (manga and anime with `anime: bool`), `browse`, `sources`, `extensions`, `migrate_sources`, `browse_source`, `global_search`, `migrate_entries`, `tracking`, `tracker_search`, `webview`, `cover_grid`, `chip_row` | Split by area: `ui/entry_details/manga` (area:38), `ui/entry_details/anime` (area:39), `ui/browse/manga` (area:40), `ui/browse/anime` (area:41), `ui/tracking` (area:27), `src/webview/` (area:36). The `anime: bool` branches become `Kind` values. |
| `src/player.rs` (337) | Player and reader demo overlays (`reader`, `player`, `osd`, `pill`, `fmt_time`) | Replaced by `src/reader_surface/` (area:44) and `src/player_surface/` (area:45, area:46). `fmt_time` is kept as `kuro-player::time::format_hms` after a unit test. |
| `src/settings.rs` (264) | Settings screen builder from spec (`body`, `main_menu`, `search`, `language`, `cards`, `editor`, `code_editor`, `clear_db`, `licenses`) | Becomes `src/ui/settings/` with one module per page. Preference values bind through `kuro-prefs` binders (area:02). `clear_db` moves to `kuro-app::maintenance` and runs in one transaction (area:04 D5). |
| `src/spec.rs` (191) | `serde` model of `spec/*.json` and `Spec::load` | Kept as a test fixture loader in `kuro-testkit::spec` after the UI no longer loads it at runtime. Used to check that every `settings` row in `spec/settings.json` has a preference key. |
| `src/clamp.rs` (62) | Its header: "AdwClamp without libadwaita: the child gets at most `max` px of width, centered, and can shrink down to its own minimum" | Moves to `src/widgets/clamp.rs` unchanged in behaviour. It replaces libadwaita's `AdwClamp` (A2). |
| `src/entry.rs` `cover_grid` | Cover placeholder grid | Replaced by `kuro-images` covers (area:34). |
| `spec/*.json` (settings, app, entries, reader_player) | UI parity fixture | Frozen. Read by `kuro-testkit` only. Changes go to the area that owns the screen. `spec/SCHEMA.md` stays. |

Preserve the mockup's interaction rules that the plans name: Escape and Alt+Left go back; no focus ring on the first header button at startup (`GtkWindowExt::set_focus(&win, None)`); immersive reader and player hide the sidebar.

Sample content rule (memory note): do not invent sample rows or bespoke CSS. The UI shows real data from the database or an explicit empty state. The mockup's sample data is removed when the screen gets real data.

Sidebar badges (area:01 Q5): `updates` and `browse` counts come from `ctx.jobs()` and `ctx.extensions()` through `Sub` messages. The mockup's hard-coded counts are removed.

Theme: CSS provider loaded from `assets/theme/tokyo-night.css` with colour tokens on `:root`-equivalent GTK named colours (`@define-color`). Dark only at first; light palette is P2 (area:49). Setting "Use system GTK theme colours" (C10) disables the provider.

---

## 15. Build features and profiles

### 15.1 Profiles

```toml
[profile.dev]
debug = "line-tables-only"

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
debug = "line-tables-only"
strip = "none"
panic = "unwind"

[profile.preview]
inherits = "release"
```

Source: [area:01](01-foundation-app-bootstrap/p1-topic-build-and-dependency-map.md) section 1. `strip = "none"` keeps symbols so crash reports show file and line (area:01 section 1.1). `panic = "unwind"` is kept so the panic hook runs before a `catch_unwind` in `contain()`.

Gradle build types map (area:01 section 1): `debug` becomes profile `dev`; `release` becomes `--release`; `preview` becomes `--profile preview` with `BUILD_TYPE = "preview"`; `benchmark` is dropped.

### 15.2 Build identity

`kuro-core/build.rs` sets `KURO_COMMIT_COUNT`, `KURO_COMMIT_SHA`, `KURO_BUILD_TIME`, `KURO_BUILD_TYPE` through `cargo:rustc-env` (area:01 section 1.3). `kuro-core::app_info` reads them. `--version` prints the same fields.

### 15.3 Packaging

One `x86_64-unknown-linux-gnu` binary. `aarch64-unknown-linux-gnu` optional and not tested (area:01 section 1.4). Flatpak manifest and desktop file in `packaging/` (area:54). `.desktop` `Actions=` entries replace static shortcuts (area:53). Windows: CI runs `cargo check --target x86_64-pc-windows-gnu` from the first milestone (addendum rule 7).

### 15.4 Gradle dependencies

Every entry in `app/build.gradle.kts` has a Rust answer in [area:01](01-foundation-app-bootstrap/p1-topic-build-and-dependency-map.md) section 2. Dropped entries list their reason there.

---

## 16. Module ownership map

Each row names the area, its Rust home, and the top-level module. "Dropped" means the area's own document lists the feature as dropped with a reason and a user-visible loss.

| Area | Title (from areas.py) | Rust home |
|---|---|---|
| [area:01](01-foundation-app-bootstrap/00-overview.md) | App bootstrap, DI, crash handling, app-level plumbing | `kuro-app::{context, runtime, migrate}`, `kuro-core::{crash, app_info, logging, deep_link}`, `src/main.rs`, `src/crash_window.rs` |
| [area:02](02-preferences-system/00-overview.md) | Preference store and every `*Preferences` class | `kuro-prefs::schema::*`, `kuro-prefs::backend::*`, `src/widgets/bind.rs` |
| [area:03](03-db-manga-schema-migrations/00-overview.md) | Manga database and migrations | `kuro-db::{schema::manga, migrate, legacy}` |
| [area:04](04-db-anime-schema-migrations/00-overview.md) | Anime database and migrations | `kuro-db::schema::anime`, `kuro-db::runtime` |
| [area:05](05-data-repos-entries-items/00-overview.md) | Data repositories: entries, items, categories | `kuro-data::{entries, items, category}` |
| [area:06](06-data-repos-history-track-source-misc/00-overview.md) | Data repositories: history, track, updates, sources, releases, custom buttons, handlers, extension repos | `kuro-data::{history, track, updates, source, release, custom_button, repos}` |
| [area:07](07-domain-entries-items/00-overview.md) | Domain: entries and items models, interactors | `kuro-domain::{entries, items}` |
| [area:08](08-domain-library-category-history-updates/00-overview.md) | Domain: library, category, history, updates | `kuro-domain::{library, category, history, updates}` |
| [area:09](09-domain-source-track-download-release-misc/00-overview.md) | Domain: source, track, download, release, storage, backup, custom buttons, repos | `kuro-domain::{source, track, download, release, storage, backup_model, custom_button}` |
| [area:10](10-app-domain-layer/00-overview.md) | App-level domain layer, interactors, mappers | `kuro-app::interactors`, `kuro-domain::mappers` |
| [area:11](11-app-migrations/00-overview.md) | In-app migration framework and each migration step | `kuro-app::migrate::steps` (framework from area:01) |
| [area:12](12-network-stack/00-overview.md) | HTTP stack, interceptors, Cloudflare, DoH, rate limit, cookies, user agent | `kuro-net::{client, cookies, rate, cloudflare, doh, ua}` |
| [area:13](13-core-util-storage-archive-torrent/00-overview.md) | core utilities, storage, archive readers, torrent helpers | `kuro-core::{archive, fs_name, natural_order, util}`, `kuro-platform::Paths`, `kuro-torrent` |
| [area:14](14-source-local-and-metadata/00-overview.md) | Local source and metadata parsing | `kuro-source-local`, `kuro-core::metadata` |
| [area:15](15-source-api-manga/00-overview.md) | Manga source API | `kuro-source::manga` |
| [area:16](16-source-api-anime/00-overview.md) | Anime source API | `kuro-source::anime` |
| [area:17](17-source-app-layer/00-overview.md) | App source layer: SourceManager, StubSource, pinning | `kuro-app::sources`, `kuro-source::manager` |
| [area:18](18-extensions-manga/00-overview.md) | Manga extension loader, installer, trust, update API, repos | `kuro-ext::{manga, install, trust, repo}` |
| [area:19](19-extensions-anime/00-overview.md) | Anime extension loader, installer, trust, update API, stores | `kuro-ext::{anime, install, trust, repo}` |
| [area:20](20-extension-strategy-redesign/00-overview.md) | Extension strategy: built-in, TOML, WASM; source catalog migration | `kuro-ext`, `kuro-source-scrape`, `kuro-ext-wasm`, `kuro-sources-builtin` |
| [area:21](21-backup-format-and-create/00-overview.md) | Backup format and create | `kuro-backup::{format, create}` |
| [area:22](22-backup-restore/00-overview.md) | Backup restore | `kuro-backup::{restore, validate}`, `kuro-app::jobs::restore` |
| [area:23](23-downloads-manga/00-overview.md) | Manga downloads | `kuro-downloads::manga` |
| [area:24](24-downloads-anime/00-overview.md) | Anime downloads (HTTP, HLS, torrent) | `kuro-downloads::anime`, `kuro-torrent` |
| [area:25](25-downloads-shared/00-overview.md) | Shared download infrastructure and storage layout | `kuro-downloads::{queue, storage, cache}` |
| [area:26](26-library-update-jobs/00-overview.md) | Library update jobs and scheduling | `kuro-app::{jobs, library_update}` |
| [area:27](27-tracking-core/00-overview.md) | Tracker framework | `kuro-track::{tracker, manager, oauth, creds}` |
| [area:28](28-tracker-anilist/00-overview.md) | AniList tracker | `kuro-track::trackers::anilist` |
| [area:29](29-tracker-myanimelist/00-overview.md) | MyAnimeList tracker | `kuro-track::trackers::myanimelist` |
| [area:30](30-tracker-kitsu/00-overview.md) | Kitsu tracker | `kuro-track::trackers::kitsu` |
| [area:31](31-tracker-shikimori-bangumi/00-overview.md) | Shikimori and Bangumi | `kuro-track::trackers::{shikimori, bangumi}` |
| [area:32](32-tracker-mangaupdates-simkl/00-overview.md) | MangaUpdates and Simkl | `kuro-track::trackers::{mangaupdates, simkl}` |
| [area:33](33-tracker-enhanced-kavita-komga-suwayomi-jellyfin/00-overview.md) | Enhanced trackers: Kavita, Komga, Suwayomi, Jellyfin | `kuro-track::trackers::{kavita, komga, suwayomi, jellyfin}` |
| [area:34](34-images-covers-cache/00-overview.md) | Image pipeline, covers, cache, saving | `kuro-images::{fetch, decode, cache, save}` |
| [area:35](35-data-misc-updater-notification-export/00-overview.md) | Notifications, updater, library export, misc | `kuro-platform::{notifier, opener, network}`, `kuro-app::{notifications, updater, export}`, `kuro-core::notify` |
| [area:36](36-ui-shell-navigation-security-webview/00-overview.md) | App shell, navigation, security, WebView, onboarding | `src/shell/*`, `src/webview/*`, `kuro-platform::{auth, webview}`, `src/ui/onboarding/` |
| [area:37](37-ui-library/00-overview.md) | Library screens | `src/ui/library/` |
| [area:38](38-ui-entry-details-manga/00-overview.md) | Manga details screen | `src/ui/entry_details/manga/`, `kuro-chapters::chapter` |
| [area:39](39-ui-entry-details-anime/00-overview.md) | Anime details screen | `src/ui/entry_details/anime/`, `kuro-chapters::episode` |
| [area:40](40-ui-browse-manga/00-overview.md) | Manga browse, global search, extensions, migration screens | `src/ui/browse/manga/` |
| [area:41](41-ui-browse-anime/00-overview.md) | Anime browse | `src/ui/browse/anime/` |
| [area:42](42-ui-updates-history-stats-storage-upcoming/00-overview.md) | Updates, History, Stats, Storage, Upcoming | `src/ui/{updates, history, stats, storage, upcoming}/`, `kuro-domain::{history, updates, upcoming, stats, storage, format}` |
| [area:43](43-reader-core/00-overview.md) | Manga reader core | `kuro-app::reader`, `kuro-reader::session` |
| [area:44](44-reader-viewers/00-overview.md) | Reader viewers | `kuro-reader::{pager, webtoon, zoom, tap}`, `src/reader_surface/` |
| [area:45](45-player-core/00-overview.md) | Anime player core | `kuro-player`, `kuro-app::player` |
| [area:46](46-player-controls-ui/00-overview.md) | Player controls UI | `src/player_surface/`, `kuro-player::{keys, controls}` |
| [area:47](47-settings-screens-general/00-overview.md) | Settings screens (general) | `src/ui/settings/{appearance, library, reader, browse, downloads, tracking, security, advanced, data, about, debug}` |
| [area:47b](47b-settings-widgets-search-more/00-overview.md) | Settings widgets, search, More tab, settings main menu | `src/widgets/spec_row.rs`, `src/ui/settings/search.rs`, `src/ui/more/` |
| [area:48](48-settings-player-and-scripts/00-overview.md) | Player settings, custom buttons editor, mpv script and config editor | `src/ui/settings/player/`, `kuro-player::config` |
| [area:49](49-theme-and-design-system/00-overview.md) | Themes, components, icons | `src/theme/`, `assets/theme/` |
| [area:49b](49b-android-res-values-drawables/00-overview.md) | Android resources mapped to CSS and icons | `assets/icons/`, `assets/theme/` |
| [area:50](50-android-widgets/00-overview.md) | Android home-screen widgets | Dropped. Sidebar badges replace the count (C11). |
| [area:51](51-i18n/00-overview.md) | Localisation | `kuro-i18n`, `xtask` converter |
| [area:52](52-util-app/00-overview.md) | App utilities | `kuro-core::util`, `kuro-platform::system`, `src/widgets/util.rs` |
| [area:53](53-platform-manifest-intents-workers/00-overview.md) | Manifest, intents, deep links, shortcuts, services and workers | `src/cli.rs`, `kuro-platform::{single_instance, file_assoc, autostart}`, `kuro-app::jobs`, `packaging/` |
| [area:54](54-build-release-distribution-docs/00-overview.md) | Build, release, distribution, docs | `Cargo.toml`, `packaging/`, `.github/workflows/`, `docs/` |
| [addenda](addenda/requirements-cross-platform.md) | Cross-platform readiness | `kuro-platform` traits, rules R1 to R2 (section 5), CI check for Windows |

Every area id above appears in [TASKS.csv](TASKS.csv) under its own folder prefix. Task IDs keep the area number (for example `T-43a-001`), so the table is also the owner lookup for a task.

---

## 17. Phasing

Priorities come from the task table. The order below is the critical path for a usable daily app (P0), then parity (P1).

| Step | Work | Areas | Exit test |
|---|---|---|---|
| 1 | Workspace, `kuro-platform` traits and Linux paths, logging, crash hook, build identity | 01, 13 (paths), 52, 54 | `cargo test` green; panic produces a report file |
| 2 | Preferences store with TOML backend | 02 | round-trip of every key in `config.toml` |
| 3 | Manga and anime schemas, `Db` runtime, repositories | 03, 04, 05, 06 | schema conformance tests TC-S00 to TC-S02 pass |
| 4 | Domain models and interactors for entries, items, library, categories, history | 07, 08, 09, 10 | domain tests pass headless |
| 5 | HTTP client with rate limits and Cloudflare via WebView fallback | 12 | fixture tests with `wiremock` |
| 6 | Source traits, source manager, stubs, frozen ids | 15, 16, 17, 20 (id and stub rules) | id golden vectors pass |
| 7 | First built-in source (Q2 of area:20 names it) | 20 | browse, details, chapters and pages from the live site |
| 8 | Reader core and viewers | 43, 44, 34 | open a chapter and page through it |
| 9 | Player core and controls on libmpv | 45, 46 | play one stream with progress saved |
| 10 | Shell, library, entry details, browse, updates, history | 36, 37, 38, 40, 42 | navigate every P0 route |
| 11 | Settings P0 rows | 47, 47b, 48 | every P0 settings row persists |
| 12 | Downloads (manga, then anime), library updates and notifications | 23, 24, 25, 26, 35 | a queued download completes |
| 13 | Trackers (AniList first) | 27, 28, then others | login, bind, update progress |
| 14 | Backup create and restore, importer | 21, 22, 11 | round-trip of a real Aniyomi backup file |
| 15 | P2 and P3 | all | per task |

Step 7 blocks step 8 in practice, because reading needs a real source. Step 2 has no dependency on GTK.

Areas with the most P0 tasks (from TASKS.csv): 43 reader core (86), 45 player core (73), 44 reader viewers (51), 39 anime details (47), 41 anime browse (44), 38 manga details (38), 34 images (36), 37 library (36), 15 manga source API (34), 17 source app layer (33).

---

## 18. Open user calls

Each item has a recommendation. Dependent tasks wait for the answer.

| # | Question | Options | Recommendation | Affects |
|---|---|---|---|---|
| Q1 | Import Aniyomi data directly from its on-device files, or only from backup files? | (a) backup file only, (b) direct import of `tachiyomi.db`-style and `*.xml` prefs | (a) | area:03, area:21, area:22 |
| Q2 | Keep an app lock? If yes, which unlock? | (a) drop, (b) passphrase hash in `SecretStore`, (c) polkit or PAM prompt | (b), state machine is platform-neutral (area:36) | area:36 |
| Q3 | WASM priority: P0 or P1? | P0 (area:20 table) or P1 (C7) | P1. Built-in and TOML first | area:20 |
| Q4 | Which built-in sources to port first? | export a backup, count entries per source id, port top sources | top sources by library count (area:20 Q2) | area:20, section 17 step 7 |
| Q5 | Theme: bundled Tokyo Night or the system GTK colours? | (a) bundled, system toggle off, (b) system colours only | (a) with the toggle (C10) | area:49, area:49b |
| Q6 | Verbose logging default | (a) on in debug only, (b) off in release, `RUST_LOG` overrides | (b) (area:01 Q3) | area:01 |
| Q7 | Crash report contents | (a) with library data, (b) extension names and URLs only | (b) (area:01 Q4) | area:01 |
| Q8 | Background updates while the window is closed | (A) portal resident process, (B) systemd user timer | (A) by default, (B) opt-in (area:26 D1) | area:26, section 12 |
| Q9 | First automatic library pass after enabling | run now when allowed, or wait for next interval | run now when allowed (area:26 D2) | area:26 |
| Q10 | Download new anime episodes | wire like manga, or keep the Aniyomi gap | wire with `autoStart = false` (area:26 D5, C15) | area:26, area:24 |
| Q11 | reSmush.it fallback | keep, HTTPS first with HTTP fallback and a note, or drop the proxy | HTTPS first with note (area:26 D11) | area:26, area:34 |
| Q12 | Torrent-only anime sources | defer, or support with librqbit or TorrServer | defer (area:20 Q9) | area:20, area:24, area:13 |
| Q13 | Store and repo formats | JSON first, protobuf P2, gzip with JSON | as area:20 Q11 | area:20, area:19 |
| Q14 | Repository set shipped by default | none, or a named repo | none (area:20 Q4) | area:20 |
| Q15 | Stop Aniyomi APK extensions working | accept (only library data carries over) | accept (area:20 Q1) | area:20 |

---

## 19. Risks

| Risk | Where | Mitigation |
|---|---|---|
| Cloudflare challenges block most real sites | area:12 | `WebViewHost` solver from day one; browser fallback |
| Source porting cost is per site | area:20 | start with the sites the user has in the library |
| libmpv render API differs by GL stack | area:45 | runtime check, fallback renderer `gpu` with a logged warning (area:45 D-4) |
| WebKitGTK on the GTK thread only | area:36, area:12 | every WebKit call goes through `glib::MainContext` |
| `strip = "none"` makes release binaries large | area:01 | accepted; reports need symbols |
| SQLite double-quoted literal behaviour differs by build | area:03 D5 | pin `rusqlite` with `bundled` |
| Upstream protobuf backup format changes | area:21 | golden files from a real backup |
| Trade-off: GTK theme vs bundled palette | C10 | user call Q5 |

---

## 20. Quality checks for this document

* Every area folder under `plans/` appears in section 16 or in section 3 (C1 to C16).
* Every crate named in an area overview appears in section 4.2 under one name. Renames are listed in C1 and C13.
* Every decision row in section 1 cites its area or addendum.
* Dependency rules R1 to R10 are checkable with the commands given in section 5.
