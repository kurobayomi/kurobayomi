//! Remove-after-read slot arithmetic (`ReaderViewModel.deleteChapterIfNeeded`, `PlayerViewModel`) and the download
//! warnings. Pure functions; the callers hold the lists.

/// Milliseconds a download-queue notification stays up (`MangaDownloader.kt`, behaviour section 17).
pub const DOWNLOAD_NOTIFICATION_TIMEOUT_MS: u64 = 30_000;

/// Chapters in the download queue (unmetered sources left out) above which the bulk warning shows.
pub const DOWNLOADS_QUEUED_WARNING_THRESHOLD: usize = 30;

/// Chapters from one source in the download queue above which the bulk warning shows.
pub const CHAPTERS_PER_SOURCE_QUEUE_WARNING_THRESHOLD: usize = 15;

/// Chapters per source that a library update may queue before its warning shows (behaviour section 17). Unmetered
/// sources are left out of the count, as they are for the download queue.
pub const LIBRARY_UPDATE_PER_SOURCE_WARNING: usize = 60;

/// The choices of the remove-after-read setting, in the order the settings screen shows them: disabled, last read,
/// second to last, and so on up to fifth to last.
pub const REMOVE_AFTER_READ_OPTIONS: [i32; 6] = [-1, 0, 1, 2, 3, 4];

/// Total and largest per-source count of the queued items that are not from an unmetered source. Each item is
/// `(source id, is_unmetered)`.
fn metered_counts(items: impl IntoIterator<Item = (i64, bool)>) -> (usize, usize) {
    let mut per_source: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for (source, unmetered) in items {
        if !unmetered {
            *per_source.entry(source).or_default() += 1;
        }
    }
    (per_source.values().sum(), per_source.values().copied().max().unwrap_or(0))
}

/// Whether adding to the download queue shows the bulk warning: only when the queue was empty before the add and
/// auto-start is on, and then when more than 30 metered chapters are queued or more than 15 come from one source.
/// `queued` is the whole queue after the add.
pub fn download_queue_warning(was_empty: bool, auto_start: bool, queued: impl IntoIterator<Item = (i64, bool)>) -> bool {
    if !(was_empty && auto_start) {
        return false;
    }
    let (total, widest) = metered_counts(queued);
    total > DOWNLOADS_QUEUED_WARNING_THRESHOLD || widest > CHAPTERS_PER_SOURCE_QUEUE_WARNING_THRESHOLD
}

/// Whether a library update shows the queue-size warning (`showQueueSizeWarningNotificationIfNeeded`): more than 60
/// entries from one metered source. `to_update` holds `(source id, is_unmetered)` per entry.
pub fn library_update_warning(to_update: impl IntoIterator<Item = (i64, bool)>) -> bool {
    metered_counts(to_update).1 > LIBRARY_UPDATE_PER_SOURCE_WARNING
}

/// The index of the item to remove, counted back from `current` by `slots`. `-1` is disabled, and so is any other
/// negative value (E-DL-04). Returns `None` when the target is before the first item (E-DL-03).
pub fn remove_after_read_index(current: usize, slots: i32) -> Option<usize> {
    if slots < 0 {
        return None;
    }
    current.checked_sub(slots as usize)
}

/// The item of `sorted` to remove after `current` was read, or `None` (TC-DL-07, TC-DL-08).
pub fn remove_after_read_target<T>(sorted: &[T], current: usize, slots: i32) -> Option<&T> {
    remove_after_read_index(current, slots).and_then(|i| sorted.get(i))
}

/// `downloadOnlyOverWifi`: whether the download job may run on the current network. Kotlin asks WorkManager for an
/// unmetered network; here `network_not_metered` is the platform's answer (`kuro_platform::device`, read by areas 23
/// and 24), so a test can pass a fake.
pub fn download_network_allowed(only_over_wifi: bool, network_not_metered: bool) -> bool {
    !only_over_wifi || network_not_metered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_after_read_selects_current_or_earlier_items() {
        let chapters = ["c1", "c2", "c3", "c4"];
        assert_eq!(remove_after_read_target(&chapters, 3, 0), Some(&"c4"), "0 is the current chapter");
        assert_eq!(remove_after_read_target(&chapters, 3, 1), Some(&"c3"), "1 is the previous one");
        assert_eq!(remove_after_read_target(&chapters, 3, 3), Some(&"c1"));
    }

    #[test]
    fn remove_after_read_does_nothing_when_out_of_range_or_disabled() {
        let chapters = ["c1", "c2", "c3", "c4"];
        assert_eq!(remove_after_read_target(&chapters, 2, 4), None, "E-DL-03: only 2 before the current");
        assert_eq!(remove_after_read_target(&chapters, 3, -1), None, "-1 is disabled");
    }

    #[test]
    fn negative_slots_other_than_minus_one_are_disabled_not_later_items() {
        let chapters = ["c1", "c2", "c3", "c4"];
        assert_eq!(remove_after_read_target(&chapters, 1, -2), None, "E-DL-04: never index+2");
        assert_eq!(remove_after_read_index(1, -7), None);
    }

    #[test]
    fn the_settings_options_are_in_screen_order() {
        assert_eq!(REMOVE_AFTER_READ_OPTIONS, [-1, 0, 1, 2, 3, 4]);
    }

    #[test]
    fn wifi_only_blocks_metered_networks_and_nothing_else() {
        assert!(download_network_allowed(false, false), "any network when the option is off");
        assert!(download_network_allowed(true, true), "unmetered network");
        assert!(!download_network_allowed(true, false), "metered network with wifi-only on");
    }
}

#[cfg(test)]
mod queue_threshold_tests {
    use super::*;

    #[test]
    fn the_download_warning_needs_an_empty_queue_and_auto_start_and_skips_unmetered_sources() {
        let sixteen_from_one = || (0..16).map(|_| (1, false));
        assert!(download_queue_warning(true, true, sixteen_from_one()), "16 from one source");
        assert!(!download_queue_warning(true, true, (0..15).map(|_| (1, false))), "15 is not above the limit");
        assert!(!download_queue_warning(false, true, sixteen_from_one()), "queue was not empty");
        assert!(!download_queue_warning(true, false, sixteen_from_one()), "auto-start is off");
        assert!(!download_queue_warning(true, true, (0..40).map(|_| (1, true))), "unmetered items are not counted");
        let spread = (0..31).map(|i| (i % 3, false));
        assert!(download_queue_warning(true, true, spread), "31 in total across sources");
    }

    #[test]
    fn the_library_update_warning_counts_per_metered_source() {
        assert!(library_update_warning((0..61).map(|_| (7, false))));
        assert!(!library_update_warning((0..60).map(|_| (7, false))));
        assert!(!library_update_warning((0..100).map(|_| (7, true))), "unmetered excluded");
        assert!(!library_update_warning((0..120).map(|i| (i % 2, false))), "60 + 60 over two sources");
    }
}
