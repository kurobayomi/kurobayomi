//! The notification channel table (`Notifications.kt`) and its freedesktop mapping. Channels do not exist on
//! Linux, so the table is only the id and category vocabulary that the `Notifier` keys are built from.
//! Registration of channels is therefore a no-op and cannot fail.

/// Channel importance, as registered by Aniyomi (`IMPORTANCE_LOW`, `DEFAULT`, `HIGH`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Importance {
    Low,
    Default,
    High,
}

/// One notification channel. `group` is the channel group key; `name_key` is the translation key of its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationCategory {
    pub channel_id: &'static str,
    pub group: Option<&'static str>,
    pub importance: Importance,
    pub name_key: &'static str,
    pub show_badge: bool,
    pub silent: bool,
}

/// The 12 channels that the app registers at startup, in the order of the Kotlin file.
pub const CATEGORIES: [NotificationCategory; 12] = [
    NotificationCategory { channel_id: "common_channel", group: None, importance: Importance::Low, name_key: "channel_common", show_badge: true, silent: false },
    NotificationCategory { channel_id: "library_progress_channel", group: Some("group_library"), importance: Importance::Low, name_key: "channel_progress", show_badge: false, silent: false },
    NotificationCategory { channel_id: "library_errors_channel", group: Some("group_library"), importance: Importance::Low, name_key: "channel_errors", show_badge: false, silent: false },
    NotificationCategory { channel_id: "new_chapters_episodes_channel", group: None, importance: Importance::Default, name_key: "channel_new_chapters_episodes", show_badge: true, silent: false },
    NotificationCategory { channel_id: "downloader_progress_channel", group: Some("group_downloader"), importance: Importance::Low, name_key: "channel_progress", show_badge: false, silent: false },
    NotificationCategory { channel_id: "downloader_error_channel", group: Some("group_downloader"), importance: Importance::Low, name_key: "channel_errors", show_badge: false, silent: false },
    NotificationCategory { channel_id: "backup_restore_progress_channel", group: Some("group_backup_restore"), importance: Importance::Low, name_key: "channel_progress", show_badge: false, silent: false },
    NotificationCategory { channel_id: "backup_restore_complete_channel_v2", group: Some("group_backup_restore"), importance: Importance::High, name_key: "channel_complete", show_badge: false, silent: true },
    NotificationCategory { channel_id: "incognito_mode_channel", group: None, importance: Importance::Low, name_key: "pref_incognito_mode", show_badge: true, silent: false },
    NotificationCategory { channel_id: "http_server_channel", group: None, importance: Importance::Low, name_key: "pref_http_server_name", show_badge: false, silent: false },
    NotificationCategory { channel_id: "app_apk_update_channel", group: Some("group_apk_updates"), importance: Importance::Default, name_key: "channel_app_updates", show_badge: true, silent: false },
    NotificationCategory { channel_id: "ext_apk_update_channel", group: Some("group_apk_updates"), importance: Importance::Default, name_key: "channel_ext_updates", show_badge: true, silent: false },
];

/// Finds a channel by id.
pub fn category(channel_id: &str) -> Option<&'static NotificationCategory> {
    CATEGORIES.iter().find(|c| c.channel_id == channel_id)
}

/// The string key of a notification: `"{channel}:{id}"`. Aniyomi reuses id `2` on two channels, so the
/// channel is part of the key.
pub fn key(channel_id: &str, id: i32) -> String {
    format!("{channel_id}:{id}")
}

/// Urgency hint of a freedesktop notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
}

/// The freedesktop hints that a channel maps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hints {
    pub urgency: Urgency,
    /// The spec `category` hint, when the channel has one.
    pub category: Option<&'static str>,
    /// `suppress-sound` for silent channels.
    pub suppress_sound: bool,
    /// `resident = false` for high-importance channels (asks for a heads-up), otherwise not set.
    pub resident: Option<bool>,
}

/// The `Notifier` mapping of a channel (`p1-rust-design.md` 3.7): Low gives urgency low, Default and High give
/// urgency normal, `silent` suppresses sound, and the category hint depends on the channel.
pub fn hints(category: &NotificationCategory) -> Hints {
    let urgency = match category.importance {
        Importance::Low => Urgency::Low,
        Importance::Default | Importance::High => Urgency::Normal,
    };
    let spec_category = match category.channel_id {
        "library_progress_channel" | "downloader_progress_channel" | "backup_restore_progress_channel"
        | "backup_restore_complete_channel_v2" => Some("transfer"),
        "library_errors_channel" | "downloader_error_channel" => Some("transfer.error"),
        "http_server_channel" => Some("device"),
        _ => None,
    };
    Hints {
        urgency,
        category: spec_category,
        suppress_sound: category.silent,
        resident: (category.importance == Importance::High).then_some(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colliding_ids_map_to_distinct_keys() {
        assert_ne!(key("common_channel", 2), key("app_apk_update_channel", 2));
        assert_eq!(key("common_channel", 2), "common_channel:2");
        assert_eq!(key("incognito_mode_channel", -701), "incognito_mode_channel:-701");
    }

    #[test]
    fn low_maps_to_low_and_default_or_high_to_normal() {
        let low = category("library_progress_channel").unwrap();
        let default = category("app_apk_update_channel").unwrap();
        let high = category("backup_restore_complete_channel_v2").unwrap();
        assert_eq!(hints(low).urgency, Urgency::Low);
        assert_eq!(hints(default).urgency, Urgency::Normal);
        assert_eq!(hints(high).urgency, Urgency::Normal);
        assert_eq!(hints(high).resident, Some(false));
        assert_eq!(hints(low).resident, None);
    }

    #[test]
    fn silent_channel_suppresses_sound() {
        assert!(hints(category("backup_restore_complete_channel_v2").unwrap()).suppress_sound);
        assert!(!hints(category("common_channel").unwrap()).suppress_sound);
    }

    #[test]
    fn category_hints_follow_the_channel_families() {
        let cat = |id| hints(category(id).unwrap()).category;
        assert_eq!(cat("library_progress_channel"), Some("transfer"));
        assert_eq!(cat("backup_restore_complete_channel_v2"), Some("transfer"));
        assert_eq!(cat("library_errors_channel"), Some("transfer.error"));
        assert_eq!(cat("downloader_error_channel"), Some("transfer.error"));
        assert_eq!(cat("http_server_channel"), Some("device"));
        assert_eq!(cat("common_channel"), None);
    }
}
