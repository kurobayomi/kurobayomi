//! OS-specific integration. This is the only crate allowed to contain
//! `cfg(target_os`, `xdg-open`, `libsecret`, `zbus`, `XDG_` or `libc::` strings, and the only one that names its per-OS
//! modules.
//! `scripts/check-platform-boundary.sh` fails the build when they appear anywhere else.
//! kuro-core and kuro-platform must not depend on gtk4; UI-side replacements live in the GTK crate.
//!
//! # Android to Linux API map
//!
//! Every row of `plans/52-util-app/p1-rust-design.md` section 6 appears once. The owner is the
//! module that holds the replacement, `dropped` (with the reason), or the crate or layer that
//! owns the non-platform row. "not implemented yet" marks a planned module with no code yet.
//!
//! | Android API (util file) | Replacement | Owner |
//! |---|---|---|
//! | `ClipboardManager.setPrimaryClip` (ContextExtensions) | `gdk::Display::clipboard().set_text()` | GTK layer |
//! | `Intent.ACTION_VIEW` (openInBrowser) | `gtk::UriLauncher` (portal OpenURI); `forceDefaultBrowser` dropped | GTK layer |
//! | `Intent.ACTION_SEND` chooser (toShareIntent, CrashLogUtil) | `gtk::FileDialog::save` / `gtk::FileLauncher`; no share sheet | GTK layer |
//! | `FileProvider.getUriCompat` (CrashLogUtil) | `gio::File::for_path`; no provider needed | GTK layer |
//! | `externalCacheDir` (createFileInCacheDir) | `Paths::cache_dir()` and `Paths::temp_dir()` | kuro-platform |
//! | `ConnectivityManager`, `NetworkCapabilities` (NetworkExtensions, NetworkStateTracker) | `gio::NetworkMonitor` plus NetworkManager over zbus (`device::read` covers the update conditions) | kuro-platform |
//! | `WifiManager.isWifiEnabled`, `connectionInfo.bssid` | NetworkManager `PrimaryConnectionType` (`device::read`) | kuro-platform |
//! | `ConnectivityManager.registerDefaultNetworkCallback` (networkStateFlow) | `power_network::PowerNetworkState` (a watch value bumped when the conditions change; polls NetworkManager and UPower, GIO network monitor fallback) | kuro-platform |
//! | `NotificationManagerCompat`, builders, channels (NotificationExtensions) | `Notifier` over `gio::Notification` (T-req-platform-004); channel groups dropped | kuro-platform |
//! | `POST_NOTIFICATIONS` runtime permission (NotificationExtensions) | none needed; the portal asks | dropped (no Linux permission) |
//! | `NotificationCompat.Builder.setColor` | dropped | dropped (no notification colour) |
//! | `WorkManager.getWorkInfosByTag` (isRunning) | `JobRegistry` (area 26, not implemented yet) | kuro-core |
//! | `CoroutineWorker.setForeground`, `delay(500)` (setForegroundSafely) | dropped | dropped (no foreground service) |
//! | `Settings.Global.ANIMATOR_DURATION_SCALE` (animatorDurationScale) | `gtk::Settings::is_gtk_enable_animations()` | GTK layer |
//! | `UiModeManager`, night mask (isNightMode, createReaderThemeContext) | portal `color-scheme` over zbus (`linux::appearance`), or `gtk::Settings` | kuro-platform |
//! | `Configuration.smallestScreenWidthDp`, `orientation` (DisplayExtensions) | window width and height in px | GTK layer |
//! | `Window.FLAG_SECURE` (setSecureScreen, SecureActivityDelegate) | dropped; noted in settings | dropped (no Linux equivalent) |
//! | `DisplayCutout`, `rootWindowInsets` (hasDisplayCutout) | dropped | dropped (no cutout on desktop) |
//! | `config_navBarNeedsScrim` (InternalResourceHelper) | dropped | dropped (no navigation bar) |
//! | `Resources.getSystem().displayMetrics` (isVisibleOnScreen) | window bounds from `compute_bounds` | GTK layer |
//! | `View.getGlobalVisibleRect`, `Rect.intersect` (isVisibleOnScreen) | `gtk::Widget::compute_bounds` and a rectangle intersection | GTK layer |
//! | `PopupMenu` (popupMenu) | `gtk::PopoverMenu` with `gio::Menu` | GTK layer |
//! | `ComposeView`, `setContent`, `TachiyomiTheme` (setComposeContent) | dropped (GTK widgets) | dropped (GTK widgets replace Compose) |
//! | `BiometricManager`, `BiometricPrompt` (AuthenticatorUtil) | PAM (`pam` crate) with optional fprintd (section 2.18, not implemented yet) | kuro-platform |
//! | `PackageManager`, `getApplicationInfo` (isPackageInstalled) | dropped | dropped (no package manager check) |
//! | `Build.MANUFACTURER`, `BRAND`, `DEVICE`, `MODEL` (CrashLogUtil) | read `/sys/class/dmi/id/*` (not implemented yet) | kuro-platform |
//! | `Build.VERSION.RELEASE`, `SDK_INT`, `DISPLAY` | `/etc/os-release` `PRETTY_NAME` and `/proc/sys/kernel/osrelease` (not implemented yet) | kuro-platform |
//! | `WebViewUtil.getVersion` (CrashLogUtil) | `gtk::major_version()` and GLib versions | GTK layer |
//! | `Runtime.exec("logcat ...")` (CrashLogUtil) | read the tracing log file tail via `Paths::log_dir()` | kuro-platform |
//! | `ActivityManager.isLowRamDevice` (DeviceUtil) | `/proc/meminfo` `MemTotal` below 3 GiB (not implemented yet) | kuro-platform |
//! | `ClassLoader`, `PathClassLoader`, `DexFile` (ChildFirstPathClassLoader) | dropped; extensions are designed in area 20 | dropped (extension design, area 20) |
//! | `TvUtils` (`UiModeManager.TYPE_TELEVISION`, `ACTION_OPEN_DOCUMENT`) | dropped | dropped (no TV mode) |
//! | `DynamicColors.isDynamicColorAvailable` (DeviceUtilExtensions) | dropped; the GTK theme applies | dropped (GTK theme) |
//! | `DeviceUtil.isSamsung`, `isMiui` (`ro.miui.ui.version.name`) | dropped | dropped (vendor checks) |
//! | `PowerManager` wake lock (powerManager) | `gtk::Application::inhibit` with the idle flag | GTK layer |
//! | `Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES` (launchRequestPackageInstallsPermission) | dropped | dropped (no install-source permission) |
//! | `UniFile`, SAF `fromUri`, `size` (UniFile, getUriSize) | `std::fs` and `symlink_metadata` | std |
//! | `Okio BufferedSource` (saveTo) | `tokio::io::AsyncRead` and `tokio::fs::File` (area 13) | kuro-core |
//! | `java.io.File.outputStream`, `mkdirs` | `std::fs::create_dir_all` | std |
//! | `java.time.*` (DateExtensions, AniChartApi) | `chrono`, with `chrono-tz` for explicit zones | kuro-core |
//! | `java.text.DateFormat`, `DateTimeFormatter.ofLocalized*` | `icu_datetime` (length short) | kuro-i18n |
//! | `java.util.Locale`, `LocaleListCompat` (LocaleHelper) | ICU4X `icu_displaynames` and `glib::language_names()` | kuro-i18n |
//! | `java.text.Collator` (SortUtil compareToWithCollator) | `icu_collator` at primary strength (`kuro_core::text::primary_cmp`) | kuro-core |
//! | plural resources (`pluralStringResource`) | `icu_plurals` with a key table | kuro-i18n |
//! | `SecureRandom`, `Base64.getUrlEncoder().withoutPadding()` (PkceUtil) | OS RNG with URL-safe base64 without padding (area 28) | kuro-core |
//! | `OkHttpClient`, `Request`, `POST`/`GET` helpers (AniChartApi) | `reqwest::Client` shared client (area 12, 10 s timeouts) | kuro-net |
//! | `kotlinx.serialization` `buildJsonObject` (AniChartApi) | `serde_json::json!` | caller crate |
//! | `OffsetDateTime.parse` (toUnixTimestamp) | `chrono::DateTime::parse_from_rfc3339` | kuro-core |
//! | `Calendar.getInstance().timeInMillis` | `chrono::Utc::now().timestamp_millis()` | kuro-core |
//! | `Hash.md5` (DiskUtil) | `md-5` crate (`kuro_core::hash`), kept for cache-name parity | kuro-core |
//! | RxJava `Observable.asFlow` (RxJavaExtensions) | dropped | dropped (no RxJava) |
//! | `callbackFlow`, `Flow`, `StateFlow` (NetworkStateTracker, PreferenceMutableState) | `tokio::sync::watch` | kuro-core |
//! | `Injekt.get<T>()` (many) | explicit constructor injection (area 01) | root crate and kuro-core |
//! | `Preference<T>.changes()` (PreferenceMutableState) | `Pref<T>::subscribe()` (area 02, not implemented yet) | kuro-platform |
//! | Compose `remember` and `collectAsState` (AnimeSourceUtil, MangaSourceUtil) | `watch::Receiver` and `glib::spawn_future_local` | GTK layer |
//! | `Context.toast(...)` (many) | `ui::toast::Toaster` (section 5) | GTK layer |
//! | `Drawable`, `BitmapDrawable`, Coil `ScaleDrawable` (DrawableExtensions) | `gdk::Texture` | GTK layer |
//!
//! Traits: [`paths::Paths`] (directories) and [`secrets::SecretStore`] (credentials).
//! Their system implementations: [`system_paths::SystemPaths`] and [`secrets::open`] (the system keyring, with the
//! encrypted file as fallback). OS-specific engines live in per-OS modules (`linux`, `windows`); every other module is shared and
//! picks the engine of the OS it is built for, with a fallback where an OS has none yet (see `hidden_pages`).

pub mod channel;
pub mod deep_link;
pub mod device;
pub mod dropped;
pub mod external_player;
pub mod gl;
pub mod idle;
pub mod incognito;
#[cfg(feature = "keyring")]
mod keyring_store;
pub mod kind;
pub mod launcher;
pub mod lifecycle;
pub mod links;
pub mod locale;
pub mod media;
pub mod notifications;
pub mod notifier;
pub mod opener;
pub mod paths;
pub mod power_network;
pub mod process;
pub mod self_update;
pub mod secrets;
mod secrets_file;
pub mod hidden_pages;
pub mod single_instance;
pub mod sort;
pub mod storage;
pub mod system;
pub mod system_paths;
pub mod toast;
pub mod webview;
pub mod window_level;

#[cfg(all(target_os = "linux", feature = "platform-linux"))]
pub mod linux;
#[cfg(all(target_os = "windows", feature = "platform-windows"))]
pub mod windows;
