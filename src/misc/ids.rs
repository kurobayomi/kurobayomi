//! The notice id registry (T-35a-005): every desktop notice slot of the app has one id here, so two producers never
//! share a slot. The Android collision (`ID_APP_UPDATE_PROMPT` and `ID_DOWNLOAD_IMAGE` were both 2) is fixed by giving
//! `save-image` and `app-update.install` their own ids.
//!
//! Decision: the registry lives in `misc` because `src/notify` is not part of this lane; the ids are the strings of
//! p1-rust-design.md 3.2. A per-entry slot (one notice per manga or anime) is the base id plus `:` and the entry id.

use kuro_platform::kind::Kind;

pub const LIBRARY_PROGRESS: &str = "library.progress";
pub const LIBRARY_ERROR: &str = "library.error";
pub const LIBRARY_SIZE_WARNING: &str = "library.size-warning";
pub const LIBRARY_SUMMARY: &str = "library.summary";
pub const NEW_CHAPTERS: &str = "new.chapters";
pub const NEW_EPISODES: &str = "new.episodes";

/// The notices other crates own, listed here so the registry names every slot of the app.
pub use kuro_backup::notify::{ID_AUTO_WARNING, ID_BACKUP_PROGRESS, ID_BACKUP_RESULT, ID_RESTORE_PROGRESS, ID_RESTORE_RESULT};
pub use kuro_downloads::notify::{ANIME_ERROR_ID, ANIME_PROGRESS_ID, ANIME_WARNING_ID, MANGA_ERROR_ID, MANGA_PROGRESS_ID, MANGA_WARNING_ID};
pub use kuro_images::notify::PICTURE_NOTICE_ID;
pub const SHARED_PICTURE: &str = "reader-shared-picture";

/// The channel (`kuro_platform::notifications::CATEGORIES`) a notice belongs to, by its id. The user mutes a channel
/// (`muted_notification_channels`), and the one notifier (`misc::notify`) checks it here before anything is shown.
pub fn channel(id: &str) -> &'static str {
    let base = id.split(':').next().unwrap_or(id);
    match base {
        LIBRARY_PROGRESS | LIBRARY_SIZE_WARNING => "library_progress_channel",
        LIBRARY_ERROR => "library_errors_channel",
        LIBRARY_SUMMARY | NEW_CHAPTERS | NEW_EPISODES => "new_chapters_episodes_channel",
        MANGA_PROGRESS_ID | ANIME_PROGRESS_ID => "downloader_progress_channel",
        MANGA_WARNING_ID | ANIME_WARNING_ID | MANGA_ERROR_ID | ANIME_ERROR_ID => "downloader_error_channel",
        ID_BACKUP_PROGRESS | ID_RESTORE_PROGRESS => "backup_restore_progress_channel",
        ID_BACKUP_RESULT | ID_RESTORE_RESULT | ID_AUTO_WARNING => "backup_restore_complete_channel_v2",
        _ => "common_channel",
    }
}

fn kind_tag(kind: Kind) -> &'static str {
    match kind {
        Kind::Manga => "manga",
        Kind::Anime => "anime",
    }
}

/// The slot of the "new chapters / episodes" notice of one entry.
pub fn new_items(kind: Kind, entry_id: i64) -> String {
    let base = match kind {
        Kind::Manga => NEW_CHAPTERS,
        Kind::Anime => NEW_EPISODES,
    };
    format!("{base}:{entry_id}")
}

/// The slot of the failure notice of one pass. The two kinds keep apart slots so one pass never replaces the other's.
pub fn library_error(kind: Kind) -> String {
    format!("{LIBRARY_ERROR}:{}", kind_tag(kind))
}

/// The slot of the large-queue warning of one pass.
pub fn library_size_warning(kind: Kind) -> String {
    format!("{LIBRARY_SIZE_WARNING}:{}", kind_tag(kind))
}

/// The slot of the summary notice of one update pass: the new items of all its entries.
pub fn library_summary(kind: Kind) -> String {
    format!("{LIBRARY_SUMMARY}:{}", kind_tag(kind))
}

/// The slot of the progress notice of one update pass (T-26a-054 shares it with the update passes).
pub fn library_progress(kind: Kind) -> String {
    format!("{LIBRARY_PROGRESS}:{}", kind_tag(kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_producers_build_their_slots_from_registry_ids() {
        assert_eq!(new_items(Kind::Manga, 7), "new.chapters:7");
        assert_eq!(new_items(Kind::Anime, 7), "new.episodes:7");
        assert_ne!(new_items(Kind::Manga, 7), new_items(Kind::Anime, 7));
        assert_ne!(library_error(Kind::Manga), library_error(Kind::Anime));
        assert_ne!(library_size_warning(Kind::Manga), library_size_warning(Kind::Anime));
        assert!(library_error(Kind::Manga).starts_with(LIBRARY_ERROR));
        assert!(library_size_warning(Kind::Anime).starts_with(LIBRARY_SIZE_WARNING));
        assert_ne!(library_progress(Kind::Manga), library_progress(Kind::Anime));
        assert!(library_progress(Kind::Manga).starts_with(LIBRARY_PROGRESS));
    }

    const FIXED: [&str; 19] = [
        LIBRARY_PROGRESS, LIBRARY_ERROR, LIBRARY_SIZE_WARNING,
        LIBRARY_SUMMARY, NEW_CHAPTERS, NEW_EPISODES, ID_AUTO_WARNING, ID_BACKUP_PROGRESS, ID_BACKUP_RESULT,
        ID_RESTORE_PROGRESS, ID_RESTORE_RESULT, ANIME_ERROR_ID, ANIME_PROGRESS_ID, ANIME_WARNING_ID, MANGA_ERROR_ID,
        MANGA_PROGRESS_ID, MANGA_WARNING_ID, PICTURE_NOTICE_ID, SHARED_PICTURE,
    ];

    #[test]
    fn every_slot_is_its_own_and_belongs_to_a_real_channel() {
        let distinct: std::collections::BTreeSet<_> = FIXED.iter().collect();
        assert_eq!(distinct.len(), FIXED.len(), "two producers share a slot");
        let channels = kuro_platform::notifications::CATEGORIES.map(|c| c.channel_id);
        for id in FIXED.iter().copied() {
            assert!(channels.contains(&channel(id)), "{id}");
        }
        assert_eq!(channel(&new_items(Kind::Anime, 3)), "new_chapters_episodes_channel", "the switch in Settings mutes these");
        assert_eq!(channel(&library_summary(Kind::Manga)), "new_chapters_episodes_channel");
    }
}
