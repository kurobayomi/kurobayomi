//! The `instantiate` API of a build without the WASM host (the `wasm` feature off, for fast UI builds): the same
//! functions, and every plug-in is refused, so the app starts with no repository sources.

use kuro_platform::kind::Kind;

/// No compile cache without the host.
pub fn set_compile_cache_dir(_dir: std::path::PathBuf) {}

/// No plug-in reads it without the host.
pub fn set_show_adult(_read: impl Fn() -> bool + Send + Sync + 'static) {}

/// The sources of one plug-in of a repository link, by the kind its index entry names.
pub enum RepoPlugin {
    Manga(Vec<std::sync::Arc<dyn kuro_source::CatalogueSource>>),
    Anime(Vec<std::sync::Arc<dyn kuro_source::anime::AnimeSource>>),
}

/// Refuses the plug-in: this build has no WASM host.
pub async fn instantiate_repo_plugin(
    _bytes: Vec<u8>,
    _hosts: Vec<String>,
    _kind: Kind,
    _client: kuro_net::NetworkClient,
    _prefs: kuro_prefs::store::PrefStore,
) -> Result<RepoPlugin, String> {
    Err("this build has no WASM host (built without the `wasm` feature)".into())
}
