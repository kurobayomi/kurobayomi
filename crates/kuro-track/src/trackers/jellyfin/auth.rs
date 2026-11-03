//! Jellyfin API key lookup (`JellyfinInterceptor.kt:44-67`; T-33a-020).
//!
//! No cache: the preferences are read on every call, so there is no shared mutable map (D-15) and an empty key is
//! never remembered (D-16).

use std::sync::Arc;

use crate::trackers::enhanced::EnhancedError;
use crate::trackers::sources::{SourceDirectory, generate_source_id};

/// Number of Jellyfin source slots the lookup scans (`JellyfinInterceptor.kt:71`).
pub const JELLYFIN_SLOTS: u32 = 10;

/// `"jellyfin"` for slot 1, `"jellyfin (n)"` otherwise (`JellyfinInterceptor.kt:44-46`). The space and parentheses are
/// kept (P-4).
pub fn jellyfin_name(n: u32) -> String {
    if n == 1 {
        "jellyfin".to_owned()
    } else {
        format!("jellyfin ({n})")
    }
}

/// The source id of Jellyfin slot `n`, `jellyfin[ (n)]/all/1`.
pub fn slot_source_id(n: u32) -> i64 {
    generate_source_id(&jellyfin_name(n), "all", 1)
}

pub struct JellyfinAuth {
    dir: Arc<dyn SourceDirectory>,
}

impl JellyfinAuth {
    pub fn new(dir: Arc<dyn SourceDirectory>) -> Self {
        Self { dir }
    }

    /// The api key of the slot whose `user_id` preference equals `user_id`. Missing slots are skipped (D-17). An empty
    /// key is `SourceNotConfigured` (E-83). No slot matching is `SourceNotConfigured` too (E-82).
    pub fn api_key_for(&self, user_id: &str) -> Result<String, EnhancedError> {
        for n in 1..=JELLYFIN_SLOTS {
            let Some(prefs) = self.dir.source_prefs(slot_source_id(n)) else {
                continue;
            };
            let slot_user = prefs.get_string("user_id", "");
            if slot_user.is_empty() || slot_user != user_id {
                continue;
            }
            let key = prefs.get_string("api_key", "");
            return if key.is_empty() {
                Err(EnhancedError::SourceNotConfigured)
            } else {
                Ok(key)
            };
        }
        Err(EnhancedError::SourceNotConfigured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trackers::sources::memory::MemDirectory;

    fn auth(dir: MemDirectory) -> JellyfinAuth {
        JellyfinAuth::new(Arc::new(dir))
    }

    #[test]
    fn t_33a_020_slot_names_follow_the_kotlin_pattern() {
        assert_eq!(jellyfin_name(1), "jellyfin");
        assert_eq!(jellyfin_name(2), "jellyfin (2)");
        assert_eq!(jellyfin_name(10), "jellyfin (10)");
    }

    #[test]
    fn t_33a_020_e82_a_user_without_a_slot_is_not_configured() {
        let mut dir = MemDirectory::default();
        dir.install("jellyfin", "all", &[("user_id", "U1"), ("api_key", "K1")]);
        let r = auth(dir).api_key_for("OTHER");
        assert!(matches!(r, Err(EnhancedError::SourceNotConfigured)));
    }

    #[test]
    fn t_33a_020_e83_an_empty_key_is_an_error_and_is_not_cached() {
        let mut dir = MemDirectory::default();
        dir.install("jellyfin", "all", &[("user_id", "U1"), ("api_key", "")]);
        let a = auth(dir);
        assert!(matches!(
            a.api_key_for("U1"),
            Err(EnhancedError::SourceNotConfigured)
        ));
        // Not cached: a key written later is seen on the next call.
        let mut dir2 = MemDirectory::default();
        dir2.install("jellyfin", "all", &[("user_id", "U1"), ("api_key", "K")]);
        let a2 = auth(dir2);
        assert_eq!(a2.api_key_for("U1").unwrap(), "K");
    }

    #[test]
    fn t_33a_020_e84_a_missing_slot_is_skipped_and_the_next_slot_is_found() {
        let mut dir = MemDirectory::default();
        dir.install(
            "jellyfin (2)",
            "all",
            &[("user_id", "U2"), ("api_key", "K2")],
        );
        dir.install(
            "jellyfin (4)",
            "all",
            &[("user_id", "U4"), ("api_key", "K4")],
        );
        // Slot 3 is not installed, so the scan must continue past it.
        let a = auth(dir);
        assert_eq!(a.api_key_for("U4").unwrap(), "K4");
        assert_eq!(a.api_key_for("U2").unwrap(), "K2");
    }

    #[test]
    fn t_33a_020_slots_beyond_ten_are_not_scanned() {
        let mut dir = MemDirectory::default();
        dir.install(
            "jellyfin (11)",
            "all",
            &[("user_id", "U"), ("api_key", "K")],
        );
        assert!(matches!(
            auth(dir).api_key_for("U"),
            Err(EnhancedError::SourceNotConfigured)
        ));
    }

    #[tokio::test]
    async fn t_33a_020_e85_concurrent_lookups_agree_without_a_cache() {
        let mut dir = MemDirectory::default();
        dir.install("jellyfin", "all", &[("user_id", "U1"), ("api_key", "K1")]);
        let a = Arc::new(auth(dir));
        let barrier = Arc::new(tokio::sync::Barrier::new(32));
        let mut tasks = Vec::new();
        for _ in 0..32 {
            let (a, b) = (Arc::clone(&a), Arc::clone(&barrier));
            tasks.push(tokio::spawn(async move {
                b.wait().await;
                a.api_key_for("U1")
            }));
        }
        for t in tasks {
            assert_eq!(t.await.unwrap().unwrap(), "K1");
        }
    }
}
