//! Installed-source lookup and source preferences for the enhanced trackers, and the
//! source-id generator they use to find a source by its name (T-33a-002).

use std::sync::Arc;

use md5::{Digest, Md5};

/// Replaces `sourceManager.get(id) as ConfigurableSource`. `None` when the source is not installed (D-05, D-17).
pub trait SourceDirectory: Send + Sync {
    fn source_prefs(&self, source_id: i64) -> Option<Arc<dyn SourcePrefs>>;
}

/// Read-only view of one source's preference file. Returns the default when the key is absent.
pub trait SourcePrefs: Send + Sync {
    fn get_string(&self, key: &str, default: &str) -> String;
}

/// Kotlin `MD5("<name lowercase>/<lang>/<versionId>")`, first 8 bytes big-endian, `and Long.MAX_VALUE`.
pub fn generate_source_id(name: &str, lang: &str, version_id: i32) -> i64 {
    let key = format!("{}/{}/{}", name.to_lowercase(), lang, version_id);
    let digest = Md5::digest(key.as_bytes());
    let mut v: u64 = 0;
    for b in &digest[..8] {
        v = (v << 8) | u64::from(*b);
    }
    (v & (i64::MAX as u64)) as i64
}

/// In-memory directory for tests. `kuro-testkit` does not carry one yet, so each tracker test uses this.
#[cfg(test)]
pub(crate) mod memory {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default, Clone)]
    pub struct MemPrefs(pub HashMap<String, String>);

    impl SourcePrefs for MemPrefs {
        fn get_string(&self, key: &str, default: &str) -> String {
            self.0
                .get(key)
                .cloned()
                .unwrap_or_else(|| default.to_owned())
        }
    }

    #[derive(Default)]
    pub struct MemDirectory {
        pub sources: HashMap<i64, Arc<MemPrefs>>,
    }

    impl MemDirectory {
        /// Installs a source under `(name, lang, 1)` with the given preference pairs.
        pub fn install(&mut self, name: &str, lang: &str, prefs: &[(&str, &str)]) {
            let map = prefs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect();
            self.sources
                .insert(generate_source_id(name, lang, 1), Arc::new(MemPrefs(map)));
        }
    }

    impl SourceDirectory for MemDirectory {
        fn source_prefs(&self, source_id: i64) -> Option<Arc<dyn SourcePrefs>> {
            self.sources
                .get(&source_id)
                .map(|p| Arc::clone(p) as Arc<dyn SourcePrefs>)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::memory::MemDirectory;
    use super::*;

    #[test]
    fn t_33a_002_e103_kavita_1_matches_the_kotlin_id() {
        assert_eq!(
            generate_source_id("kavita_1", "all", 1),
            7266875667263635239
        );
    }

    #[test]
    fn t_33a_002_e104_e105_other_kavita_slots() {
        assert_eq!(
            generate_source_id("kavita_2", "all", 1),
            3764816291200082561
        );
        assert_eq!(generate_source_id("kavita_3", "all", 1), 778637366235559582);
    }

    #[test]
    fn t_33a_002_e106_to_e109_jellyfin_slots() {
        assert_eq!(
            generate_source_id("jellyfin", "all", 1),
            1100359934660540567
        );
        assert_eq!(
            generate_source_id("jellyfin (2)", "all", 1),
            5716273013801838310
        );
        assert_eq!(
            generate_source_id("jellyfin (3)", "all", 1),
            7066619062139039107
        );
        assert_eq!(
            generate_source_id("jellyfin (10)", "all", 1),
            3559930647551045450
        );
    }

    #[test]
    fn t_33a_002_e110_suwayomi_matches_the_kotlin_id() {
        assert_eq!(
            generate_source_id("tachidesk", "en", 1),
            3100117499901280806
        );
    }

    #[test]
    fn t_33a_002_e112_the_name_is_lowercased_before_hashing() {
        assert_eq!(
            generate_source_id("Kavita_1", "all", 1),
            generate_source_id("kavita_1", "all", 1)
        );
    }

    #[test]
    fn t_33a_002_e111_the_id_is_never_negative() {
        // A fixed walk of names, so the check is deterministic and needs no property-test crate.
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..10_000 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let name = format!("src{seed:x}");
            assert!(generate_source_id(&name, "all", (seed % 7) as i32) >= 0);
        }
    }

    #[test]
    fn t_33a_003_a_missing_source_is_none_not_a_panic() {
        let dir = MemDirectory::default();
        assert!(
            dir.source_prefs(generate_source_id("kavita_1", "all", 1))
                .is_none()
        );
    }

    #[test]
    fn t_33a_003_an_installed_source_returns_its_prefs_and_defaults() {
        let mut dir = MemDirectory::default();
        dir.install("jellyfin", "all", &[("api_key", "k")]);
        let prefs = dir
            .source_prefs(generate_source_id("jellyfin", "all", 1))
            .expect("installed");
        assert_eq!(prefs.get_string("api_key", ""), "k");
        assert_eq!(prefs.get_string("user_id", "none"), "none");
    }
}
