//! The Shikimori and Bangumi trackers as the app builds them: the user's OAuth clients from `config.toml`, the
//! Bangumi episode setting from the preferences, and the platform secret store (T-31a-039, T-31a-044). The caller adds
//! the two trackers to the others and hands the whole list to `TrackerManager::new`.

use std::sync::Arc;

use kuro_platform::secrets::SecretStore;
use kuro_prefs::pref::Pref;

use super::config::TrackClientConfig;
use crate::bangumi::BangumiTracker;
use crate::error::TrackError;
use crate::shikimori::ShikimoriTracker;
use crate::tracker::Tracker;

/// The redirect of the Shikimori login. The scheme is the one `oauth::parse_callback` accepts.
pub const SHIKIMORI_REDIRECT: &str = "kurobayomi://shikimori-auth";
/// The redirect of the Bangumi login.
pub const BANGUMI_REDIRECT: &str = "kurobayomi://bangumi-auth";

/// Builds Shikimori and Bangumi, in that order (registry order). Shikimori uses the build's built-in client unless `config`
/// names one. Bangumi without a client is still built: login is refused with `NotConfigured` and its Settings row says
/// why. `episode_progress` is `TrackPrefs::bangumi_episode_progress`.
pub fn shikimori_and_bangumi(
    config: &TrackClientConfig,
    episode_progress: Pref<bool>,
    secrets: Arc<dyn SecretStore>,
) -> Result<Vec<Arc<dyn Tracker>>, TrackError> {
    let shikimori = ShikimoriTracker::production(
        config.shikimori_or_built_in(),
        SHIKIMORI_REDIRECT,
        Arc::clone(&secrets),
    )?;
    let bangumi = BangumiTracker::production(config.bangumi.clone(), BANGUMI_REDIRECT, secrets)?
        .with_episode_progress_setting(episode_progress);
    Ok(vec![Arc::new(shikimori), Arc::new(bangumi)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::MemSecrets;
    use crate::model::TrackerId;
    use crate::prefs::TrackPrefs;
    use kuro_prefs::store::PrefStore;

    fn secrets() -> Arc<dyn SecretStore> {
        MemSecrets::new()
    }

    #[tokio::test]
    async fn t_31a_039_both_trackers_come_back_in_registry_order() {
        let prefs = TrackPrefs::new(PrefStore::in_memory());
        let got = shikimori_and_bangumi(
            &TrackClientConfig::default(),
            prefs.bangumi_episode_progress,
            secrets(),
        )
        .unwrap();
        let ids: Vec<TrackerId> = got.iter().map(|t| t.id()).collect();
        assert_eq!(ids, [TrackerId::SHIKIMORI, TrackerId::BANGUMI]);
    }

    #[tokio::test]
    async fn without_a_config_table_shikimori_logs_in_with_the_build_s_client_if_it_has_one() {
        let prefs = TrackPrefs::new(PrefStore::in_memory());
        let got = shikimori_and_bangumi(
            &TrackClientConfig::default(),
            prefs.bangumi_episode_progress,
            secrets(),
        )
        .unwrap();
        match crate::shikimori::config::built_in_shikimori() {
            Some(client) => {
                assert!(got[0].setup_message().is_none(), "a released build is ready for everyone");
                let url = got[0].browser_auth_url().expect("a login URL");
                assert!(url.contains(&format!("client_id={}", client.client_id)), "{url}");
            }
            None => assert!(got[0].setup_message().is_some(), "a local build says Shikimori is not configured"),
        }
        assert!(got[1].setup_message().is_some(), "Bangumi still needs its own client");
    }

    #[tokio::test]
    async fn t_31a_039_the_config_file_decides_which_login_is_enabled() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            kuro_prefs::toml_backend::config_file(dir.path()),
            "[track.clients.shikimori]\nclient_id = \"s-id\"\nclient_secret = \"s-sec\"\n",
        )
        .unwrap();
        let prefs = TrackPrefs::new(PrefStore::in_memory());
        let got = shikimori_and_bangumi(
            &TrackClientConfig::load(dir.path()),
            prefs.bangumi_episode_progress,
            secrets(),
        )
        .unwrap();
        let shikimori = &got[0];
        let bangumi = &got[1];
        assert!(
            shikimori.browser_auth_url().is_some(),
            "configured client gives a login URL"
        );
        assert!(shikimori.setup_message().is_none());
        assert!(
            bangumi.browser_auth_url().is_none(),
            "absent client disables login"
        );
        assert!(bangumi.setup_message().is_some());
        let err = bangumi.login("", "code").await.unwrap_err();
        assert!(matches!(
            err,
            TrackError::Key(crate::messages::MsgKey::TrackerNotConfigured(ref name)) if name == "Bangumi"
        ));
    }
}
