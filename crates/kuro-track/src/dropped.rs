//! Decisions for the Android-only tracker plumbing (areas 27 to 33). Each entry is a decision recorded in code, and
//! the tests check that the code keeps it.
//!
//! * Activity lifecycle (T-27b-033, T-29a-045). `BaseOAuthLoginActivity`, the full-screen `LoadingScreen`, `finish()`
//!   and the clear-top relaunch are not ported. The main window is shown on callback (T-27b-019), and the login row
//!   shows a spinner during the exchange (T-29a-047).
//! * Login activity and link (T-29a-044). `TrackLoginActivity` and its intent filter are dropped. The MyAnimeList
//!   login uses its loopback redirect (T-29a-039). The shared link router in `oauth` still reads the registered
//!   `aniyomi://` redirects of AniList, Simkl and Bangumi (T-28a-040). It ignores `aniyomi://myanimelist-auth`.
//! * Credentials in backups (T-29a-046). The MyAnimeList token is in the platform secret store (`mal::store`), not in
//!   the preferences, so a backup does not carry it. A restored backup shows MyAnimeList logged out. The restore
//!   fixture test belongs to the backup area (22), which this crate does not own.
//! * Android-to-Rust map (T-30a-034, T-28a-041, T-32a-062). `android.graphics.Color` is a `u32` constant, `Uri` is
//!   `url::Url`, kotlinx.serialization is serde, OkHttp is reqwest, injekt is constructor injection through
//!   `TrackerDeps`, moko `StringResource` is a key, `withIOContext` is tokio, `SimpleDateFormat` is chrono, and
//!   `URLEncoder` is `url::form_urlencoded`. There is no
//!   `openInBrowser(forceDefaultBrowser)`: links open in the default browser only. `MURating.copyTo` was dead code
//!   and is not ported.
//! * Score display (T-30a-035). A score is shown with Rust's `f64` display, so `7.5` reads `7.5` in every locale.
//!   Stored values do not change.
//! * Interceptors (T-33a-023). The OkHttp interceptors are not ported. Each request builder takes its auth context.
//! * Service locator (T-33a-103). Injekt, `injectLazy` and lazy wiring are not ported. Dependencies come from
//!   constructor injection through `TrackerDeps`.
//! * Android resources (T-33a-104). `R.drawable.*`, `Color.rgb(...)` and `MR.strings.*` become asset keys, a `u32`
//!   colour and message keys.
//! * Dispatchers (T-33a-105). `withIOContext` is dropped because the tracker code is async end to end, and
//!   `withNonCancellableContext` becomes a detached `tokio::spawn` (area 27 section 4). The Kavita, Komga and
//!   Suwayomi calls `await` and do not use `spawn_blocking`.

#[cfg(test)]
mod tests {
    #[test]
    fn t30a035_a_score_reads_the_same_in_every_locale() {
        assert_eq!(crate::kitsu::score::display_score(7.5), "7.5");
        assert_eq!(crate::kitsu::score::display_score(7.0), "7");
    }
}
