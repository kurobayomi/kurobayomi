//! Network settings read from `Settings.network` (T-12a-007). The keys and defaults live in `kuro-prefs`; this
//! type is the plain snapshot the client is built from.

use http::HeaderValue;
use kuro_prefs::network::NetworkPrefs;

use crate::doh::DohProvider;

/// The helper line under the DoH dropdown while DoH is on (T-12a-044; a Linux addition).
pub const LOCAL_NAME_HINT: &str = "Local names such as localhost are not resolved while DoH is on.";

/// Whether the DoH helper line is shown: only while a provider is selected (T-12a-044).
pub fn local_name_hint_visible(doh_provider: i32) -> bool {
    DohProvider::from_pref(doh_provider).is_some()
}

/// The user agent dialog's OK button: the entry is not blank and differs from the stored value (T-12a-031).
pub fn user_agent_ok_enabled(entry: &str, stored: &str) -> bool {
    !entry.trim().is_empty() && entry != stored
}

/// Whether a user agent can be sent as a header. Invalid header characters keep the dialog open with
/// `Invalid user agent string` (T-12a-031).
pub fn user_agent_is_valid(entry: &str) -> bool {
    HeaderValue::from_str(entry).is_ok()
}

/// A snapshot of the `[network]` preferences, taken when the client is built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkSettings {
    pub verbose_logging: bool,
    /// `-1` is off; see `kuro_prefs::network::doh_endpoint`.
    pub doh_provider: i32,
    /// The default `User-Agent`, trimmed.
    pub default_user_agent: String,
}

impl NetworkSettings {
    /// The network settings. The settings diet cut every `[network]` key (verbose logging, DNS over HTTPS and the user
    /// agent), so the hard-coded defaults are used and `_prefs` is not read. The parameter stays so the client
    /// constructors keep their signature until they are changed (docs/drop-decisions.md).
    pub fn from_prefs(_prefs: &NetworkPrefs) -> Self {
        Self {
            verbose_logging: cfg!(debug_assertions),
            doh_provider: -1,
            default_user_agent: DEFAULT_USER_AGENT.to_owned(),
        }
    }
}

/// The default `User-Agent` (the settings diet cut the setting; this is its hard-coded default).
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0";

#[cfg(test)]
mod tests {
    use kuro_prefs::store::PrefStore;

    use super::*;

    #[test]
    fn a_missing_file_yields_the_defaults() {
        let prefs = NetworkPrefs::new(PrefStore::in_memory());
        let settings = NetworkSettings::from_prefs(&prefs);
        assert_eq!(settings.verbose_logging, cfg!(debug_assertions));
        assert_eq!(settings.doh_provider, -1);
        assert_eq!(
            settings.default_user_agent,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0"
        );
    }

    #[test]
    fn the_user_agent_is_the_hard_coded_default() {
        assert_eq!(NetworkSettings::from_prefs(&NetworkPrefs::new(PrefStore::in_memory())).default_user_agent, DEFAULT_USER_AGENT);
    }
}
