//! The few texts of the Browse tab that the string catalogue does not hold yet. They are Proposals of the plan
//! (T-41a-027, T-41a-036) that replace an upstream text bug or add a hint, and the catalogue crate is not part of this
//! lane, so they live here as constants until `kuro-i18n` takes them: each constant names the key it will become.

use kuro_i18n::{Arg, MsgKey, tr, trf};

/// `browse_global_search_empty_hint`: the hint of an empty global search (T-41a-027, ui-parity S4).
pub fn global_search_empty_hint() -> String {
    tr(MsgKey::browse_global_search_empty_hint)
}

/// `loading_sources`: the line under the spinner of a Sources tab while the repository sources load.
pub fn loading_sources() -> String {
    tr(MsgKey::browse_loading_sources)
}

#[cfg(test)]
/// `migrate_sources_empty`: the empty picker of the anime migration, which upstream words as "Your library is empty"
/// (a text bug; T-41a-036, ui-parity S9).
pub const MIGRATE_SOURCES_EMPTY: &str = "No source has favorites to migrate";

/// `migrate_tracks_always`: the dim line under the migrate options. Tracking data is copied whatever is ticked (T-40a-045).
pub fn migrate_tracks_always() -> String {
    tr(MsgKey::browse_migrate_tracks_always)
}

/// `migration_failed`: the inline error of a failed migration (T-41a-035, a Proposal over Aniyomi's silent path).
pub fn migration_failed(reason: &str) -> String {
    trf(MsgKey::migration_failed_reason, &[("reason", Arg::Str(&reason.to_string()))])
}

/// `action_open`: the first item of a cover's or a source's right-click menu.
pub const OPEN: &str = "Open";

/// `action_open_website`: a source row's right-click item that opens the source's site in the browser.
pub fn open_website() -> String {
    tr(MsgKey::browse_open_website)
}

#[cfg(test)]
/// The migrate tab's help action (T-41b-040, help URLs of `p2-data-and-config.md` section 5).
pub const SOURCE_MIGRATION_HELP_URL: &str = kuro_platform::links::URL_SOURCE_MIGRATION;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_texts_are_the_ones_the_plan_words() {
        assert_eq!(global_search_empty_hint(), "Search a source by title");
        assert_eq!(loading_sources(), "Loading sources…");
        assert_eq!(MIGRATE_SOURCES_EMPTY, "No source has favorites to migrate");
        assert_eq!(migrate_tracks_always(), "Tracking data is always transferred.");
        assert_eq!(SOURCE_MIGRATION_HELP_URL, "https://kurobayomi.github.io/docs/guides/source-migration");
        assert_eq!(migration_failed("disk full"), "Migration failed: disk full");
    }
}
