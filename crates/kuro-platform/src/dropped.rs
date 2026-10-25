//! Android platform features this crate does not port. Each row says what the user loses (often nothing).
//!
//! | Task | Android feature | Decision |
//! |---|---|---|
//! | T-01a-024 | home-screen widget managers | Not ported. The desktop has no home-screen widget surface. Lost: the home-screen cover grid and its locked state. The unread-updates count is `kuro_data::updates::UpdatesRepo::with_read(false, kuro_domain::updates::window::three_months_ago(..))`; the app (not this crate) calls it for the launcher badge, and the sidebar badges read the library update counters and the stored extension update count, never a literal. |
//! | T-01a-026 | `getPackageName` override and the `X-Requested-With` swap | Not ported. WebKitGTK sends no such header, so no code is written. Any header policy for challenge solving belongs to area 12 and area 36. |
//! | T-01a-027 | Conscrypt TLS provider insertion below Android 10 | Not ported. `reqwest` with rustls gives TLS 1.2 and 1.3 on every Linux system, so the default feature set has no OpenSSL. |
//! | T-01a-028 | WebView data-directory suffix | Not ported. WebKit data and cache live in `paths.data()/webkit` and `paths.cache()/webkit` (see [`crate::webview::data`]). |
//! | T-01a-033 | `FLAG_SECURE` secure-screen mode | Not ported. Wayland and X11 have no per-window "hide from screenshots and the task switcher" flag. Lost: screenshot blocking and thumbnail hiding. The preference `secure_screen_v2` stays for backup round trips (kuro-prefs). |
//! | T-01a-038 | RxJava bridge (`Observable.asFlow`) | Not ported. It only bridged RxJava 1 in legacy extension APIs. Rust sources are async functions or streams. |
//! | T-01a-041 | R8 shrinker rules | Not applicable to Rust. The extension host-API checklist is handed to area 20. |
//! | T-01a-044 | extension-installer strategies (legacy, package installer, Shizuku, private) | Not ported. The key `extension_installer` is accepted on backup import and kept as an unknown extra, never read. Extension installs are defined by area 20. |
//! | T-01a-046 | Picture-in-Picture capability flag | Not ported, and no flag exists. The always-on-top mini player replaces PiP (area 46). |
//!
//! | T-req-platform-006 | tracker login through `WebViewHost::oauth_login` | Not ported for AniList, Simkl, Shikimori and Bangumi. Their client ids are registered with the redirects `aniyomi://` and `kurobayomi://`, and a loopback address would be refused by those providers. MyAnimeList already logs in through its loopback redirect. `oauth_login` stays for a provider that accepts a loopback redirect. |
//! | T-req-platform-007 | second-launch forwarding through `SingleInstance` | The app uses GApplication forwarding (`is_unique` picks the flags), which also carries the command line and files over D-Bus and works in a sandbox. The Unix-socket `SocketInstance` was removed: every build has GApplication, and std's Unix sockets do not build on Windows. |
//! | T-req-platform-007 | external player through `ExternalPlayer` | `kuro_player::external` keeps its own argument builder: it adds title, start position, subtitle and IPC socket, which `CommandPlayer` does not carry, and it returns the child so mpv can be watched. |
//!
//! The Linux notifier (T-req-platform-004) is the app's `misc::notify`, behind [`crate::notifier::Notifier`]. Channel groups, the `POST_NOTIFICATIONS`
//! permission and the notification colour are dropped on Linux: the portal asks for permission and freedesktop
//! notifications have no colour.
