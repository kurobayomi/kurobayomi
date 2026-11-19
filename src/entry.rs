// Pages reached from the entry screens that have no module of their own: the settings fallthrough. The entry
// details, the tracking pages, Browse, global search and migration pages are live (`entry_core`, `browse`).
use crate::screens::Built;

pub fn body(id: &str) -> Option<Built> {
    match id {
        "tracking_sheet" => crate::entry_core::tracking::page::sheet(),
        "tracker_search" => crate::entry_core::tracking::page::search(),
        "community_score_search" => crate::entry_core::tracking::community_page::search(),
        "extension_repos" => crate::settings::body("settings_extension_repos"),
        _ => crate::settings::body(id),
    }
}
