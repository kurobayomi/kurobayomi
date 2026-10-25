//! Extension repositories, the link flow of `docs/design/app-flow.md`. A repository is one link to an `index.min.json`
//! that lists its sources. Adding the link makes the sources available at once; each source's file is fetched on first
//! use and checked against the sha256 in the index. Each start of the app re-reads the index.
//!
//! A source's file is a recipe (`.toml`, `docs/source-recipes.md`) or a plug-in component (`.wasm`, `wit/kuro-source.wit`).
//! The index is read here; the GTK side stores the links and the index text, and decides where a fetched file goes.

use kuro_platform::kind::Kind;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;


/// The largest index or source file accepted. Both are small text files.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// The largest source icon accepted: a square PNG of about 192 px.
pub const MAX_ICON_BYTES: usize = 256 * 1024;

/// One source a repository lists. `id` names the file the source is saved under, so it is limited to lower-case letters,
/// digits, `.`, `_` and `-`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub lang: String,
    pub kind: Kind,
    pub version: u32,
    /// The https link of the source file.
    pub file: String,
    /// The SHA-256 of the file, in hex.
    pub sha256: String,
    /// The hosts a plug-in may reach through the host's `http` import (`*.x` matches subdomains). Empty for a recipe.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// The https link of the source's square PNG icon, about 192 px. Optional: without one the row shows its letter tile.
    #[serde(default)]
    pub icon: Option<String>,
    /// The SHA-256 of the icon PNG, in hex. An icon is saved only when its bytes match this.
    #[serde(rename = "iconSha256", default)]
    pub icon_sha256: Option<String>,
    /// Whether the source works, as the repository last checked it. Missing or unknown means active.
    #[serde(default, skip_serializing_if = "Status::is_active")]
    pub status: Status,
    /// One short line on what the source offers ("Manga and webtoons, chapters with dates"), shown under its name.
    /// Optional: an older index has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A source's state as its repository reports it: the Browse row shows it as a coloured dot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The site blocks the app for now (Cloudflare, a region block, a domain move).
    Blocked,
    /// The source no longer works and needs fixing.
    Broken,
    /// Works. Last, so an unknown value reads as this one.
    #[default]
    #[serde(other)]
    Active,
}

impl Status {
    pub fn is_active(&self) -> bool {
        *self == Status::Active
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    /// The repository's name, shown on its card instead of the link. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The repository's web page (its source code), opened by the card's globe button. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    /// The address this index lives at now. A repository that moves keeps an index at its old address naming the new
    /// one, and a saved link follows it ([`moved_to`]). Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The sources of a kind this app knows. An entry of another kind (a later app's "movie" or "novel") is left out,
    /// so a repository can list new kinds without breaking the apps that cannot show them.
    #[serde(deserialize_with = "known_kinds")]
    pub sources: Vec<Entry>,
}

/// Reads the `sources` list, leaving out an entry whose `kind` is a word this app does not know. Any other entry is
/// read as before, so a missing or malformed field still fails the index.
fn known_kinds<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<Entry>, D::Error> {
    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    raw.into_iter()
        .filter(|entry| match &entry["kind"] {
            serde_json::Value::String(kind) => serde_json::from_value::<Kind>(serde_json::Value::String(kind.clone())).is_ok(),
            _ => true,
        })
        .map(|entry| serde_json::from_value(entry).map_err(serde::de::Error::custom))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepoError {
    #[error("the index could not be read: {0}")]
    Parse(String),
    #[error("the source id {0:?} is listed twice or is not a plain name")]
    BadId(String),
    #[error("the source {0:?} has a sha256 that is not 64 hex digits")]
    BadChecksum(String),
    #[error("the source {0:?} has a link that is not an https .toml, .wasm or .png file")]
    BadLink(String),
    #[error("the repository link must be https")]
    BadRepoLink,
    #[error("the file is larger than 1 MiB")]
    TooLarge,
    #[error("the file could not be reached: {0}")]
    Network(String),
    #[error("the server answered HTTP {0}")]
    Status(u16),
    #[error("the file does not match the checksum in the index")]
    Mismatch,
    #[error("the file could not be saved: {0}")]
    Write(String),
}

/// Reads an index and checks it: ids are unique and plain, the checksums are 64 hex digits, and the file links are https
/// `.toml` or `.wasm` files. A repository may list any number of sources.
pub fn parse_index(text: &str) -> Result<Index, RepoError> {
    let index: Index = serde_json::from_str(text).map_err(|e| RepoError::Parse(e.to_string()))?;
    check(&index)?;
    Ok(index)
}

fn check(index: &Index) -> Result<(), RepoError> {
    let mut ids = HashSet::new();
    for entry in &index.sources {
        if !is_plain_id(&entry.id) || !ids.insert(entry.id.as_str()) {
            return Err(RepoError::BadId(entry.id.clone()));
        }
        if !is_sha256_hex(&entry.sha256) {
            return Err(RepoError::BadChecksum(entry.id.clone()));
        }
        if !(entry.file.starts_with("https://") && (entry.file.ends_with(".toml") || entry.file.ends_with(".wasm"))) {
            return Err(RepoError::BadLink(entry.id.clone()));
        }
        if entry.icon.as_ref().is_some_and(|icon| !(icon.starts_with("https://") && icon.ends_with(".png"))) {
            return Err(RepoError::BadLink(entry.id.clone()));
        }
        if entry.icon_sha256.as_ref().is_some_and(|sha| !is_sha256_hex(sha)) {
            return Err(RepoError::BadChecksum(entry.id.clone()));
        }
    }
    Ok(())
}

fn is_plain_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The sources of one kind, grouped by language. The map is ordered by language, so a language filter is one lookup.
pub fn by_language(index: &Index, kind: Kind) -> BTreeMap<&str, Vec<&Entry>> {
    let mut groups: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for entry in index.sources.iter().filter(|entry| entry.kind == kind) {
        groups.entry(entry.lang.as_str()).or_default().push(entry);
    }
    groups
}

/// The SHA-256 of `bytes` as lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Adds a repository link to `links`. The link is trimmed, must be https, and is kept once: adding it twice changes nothing.
pub fn add_link(links: &mut Vec<String>, link: &str) -> Result<(), RepoError> {
    let link = link.trim();
    if !link.starts_with("https://") {
        return Err(RepoError::BadRepoLink);
    }
    if !links.iter().any(|known| known == link) {
        links.push(link.to_owned());
    }
    Ok(())
}

/// Removes a repository link. True when it was there.
pub fn remove_link(links: &mut Vec<String>, link: &str) -> bool {
    let before = links.len();
    links.retain(|known| known != link);
    links.len() != before
}

/// Fetches a repository's index and checks it. A link that is not https is refused before any request.
pub async fn fetch_index(client: &reqwest::Client, link: &str) -> Result<Index, RepoError> {
    if !link.starts_with("https://") {
        return Err(RepoError::BadRepoLink);
    }
    let bytes = get(client, link).await?;
    let text = String::from_utf8(bytes).map_err(|e| RepoError::Parse(e.to_string()))?;
    parse_index(&text)
}

/// Fetches one source file and checks it against the sha256 of its index entry. A file that does not match is an error,
/// so the caller keeps the copy it has.
pub async fn fetch_source(client: &reqwest::Client, entry: &Entry) -> Result<Vec<u8>, RepoError> {
    let bytes = get(client, &entry.file).await?;
    if sha256_hex(&bytes) != entry.sha256.to_ascii_lowercase() {
        return Err(RepoError::Mismatch);
    }
    Ok(bytes)
}

/// Where the copy of a source's file lives under `root`: `<id>.toml` or `<id>.wasm`, the extension the link has. The id
/// is checked by [`parse_index`], so it cannot leave the folder.
pub fn cached_path(root: &Path, entry: &Entry) -> PathBuf {
    let extension = if entry.file.ends_with(".wasm") { "wasm" } else { "toml" };
    root.join(format!("{}.{extension}", entry.id))
}

/// The path of the source's file under `root`, fetched first when there is no copy or the copy's bytes do not match the
/// index. A matching copy is used as it is, so a source is downloaded on its first use and then read from disk.
pub async fn ensure_cached(client: &reqwest::Client, root: &Path, entry: &Entry) -> Result<PathBuf, RepoError> {
    let path = cached_path(root, entry);
    let cached = std::fs::read(&path).ok();
    if cached.is_some_and(|bytes| sha256_hex(&bytes) == entry.sha256.to_ascii_lowercase()) {
        return Ok(path);
    }
    let bytes = fetch_source(client, entry).await?;
    write_atomic(&path, &bytes).map_err(|e| RepoError::Write(e.to_string()))?;
    Ok(path)
}

/// Where a source's icon lives under `root`: `<id>.png` beside its file.
pub fn cached_icon_path(root: &Path, entry: &Entry) -> PathBuf {
    root.join(format!("{}.png", entry.id))
}

/// The saved icon of a source, when the source lists one and the saved bytes still match its `iconSha256`.
pub fn read_icon(dir: &Path, entry: &Entry) -> Option<PathBuf> {
    entry.icon.as_ref()?;
    let want = entry.icon_sha256.as_deref()?;
    let path = cached_icon_path(dir, entry);
    let bytes = std::fs::read(&path).ok()?;
    (sha256_hex(&bytes) == want.to_ascii_lowercase()).then_some(path)
}

/// Saves a source's icon under `root` once. A source without an icon or a checksum saves nothing. An icon whose bytes do
/// not match is an error and leaves no file behind.
async fn ensure_icon(client: &reqwest::Client, root: &Path, entry: &Entry) -> Result<(), RepoError> {
    let (Some(url), Some(want)) = (&entry.icon, &entry.icon_sha256) else { return Ok(()) };
    if read_icon(root, entry).is_some() {
        return Ok(());
    }
    let path = cached_icon_path(root, entry);
    let bytes = get_within(client, url, MAX_ICON_BYTES).await?;
    if sha256_hex(&bytes) != want.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&path);
        return Err(RepoError::Mismatch);
    }
    write_atomic(&path, &bytes).map_err(|e| RepoError::Write(e.to_string()))
}

/// The address a repository moved to: its index's `url`, when that is another https link to a `.json` file. The index
/// already decides which files the app runs, so following its own address adds no trust; the caller still checks that
/// the new address answers before it switches.
pub fn moved_to<'a>(link: &str, index: &'a Index) -> Option<&'a str> {
    index.url.as_deref().map(str::trim).filter(|url| url.starts_with("https://") && url.ends_with(".json") && *url != link.trim())
}

/// Moves the saved copies of link `old` to the folder of link `new`, so a repository that moved fetches nothing again.
/// When `new` has copies already (both links were added), the old ones are dropped. No copies of `old` is no error.
pub fn move_link_dir(root: &Path, old: &str, new: &str) -> std::io::Result<()> {
    let (from, to) = (link_dir(root, old), link_dir(root, new));
    if !from.exists() {
        return Ok(());
    }
    if to.exists() {
        return std::fs::remove_dir_all(&from);
    }
    std::fs::rename(&from, &to)
}

/// The folder a link's copies live in: the SHA-256 of the link, so two links never share one.
pub fn link_dir(root: &Path, link: &str) -> PathBuf {
    root.join(sha256_hex(link.as_bytes()))
}

/// The name of a repository's saved index inside its folder.
const INDEX_FILE: &str = "index.min.json";

/// The saved index of a link's folder, when it is there and still reads as a valid index.
pub fn read_index(dir: &Path) -> Option<Index> {
    parse_index(&std::fs::read_to_string(dir.join(INDEX_FILE)).ok()?).ok()
}

/// The saved copy of one source, when its bytes still match the checksum of the entry.
pub fn read_source(dir: &Path, entry: &Entry) -> Option<Vec<u8>> {
    let bytes = std::fs::read(cached_path(dir, entry)).ok()?;
    (sha256_hex(&bytes) == entry.sha256.to_ascii_lowercase()).then_some(bytes)
}

/// Fetches a repository's index and the file of each source `wanted` accepts (the sources the user turned on), and saves
/// them under `dir`. Every listed source's icon is saved, so a list of the sources that are off still shows them. A source
/// is fetched again only when its copy is missing or does not match the index; the copy of a source that is not wanted is
/// deleted. Any failure is returned and the saved index stays as it was.
pub async fn sync_link(client: &reqwest::Client, dir: &Path, link: &str, wanted: &(dyn Fn(&Entry) -> bool + Sync)) -> Result<Index, RepoError> {
    let index = fetch_index(client, link).await?;
    for entry in &index.sources {
        if wanted(entry) {
            ensure_cached(client, dir, entry).await?;
        }
        // an icon is only a picture: one that fails is logged and the repository is still added
        if let Err(error) = ensure_icon(client, dir, entry).await {
            tracing::warn!(id = entry.id.as_str(), %error, "a source icon could not be saved; the source shows its letter tile");
        }
    }
    let text = serde_json::to_string(&index).map_err(|e| RepoError::Parse(e.to_string()))?;
    write_atomic(&dir.join(INDEX_FILE), text.as_bytes()).map_err(|e| RepoError::Write(e.to_string()))?;
    prune(dir, &index, wanted);
    Ok(index)
}

/// Deletes the copies in `dir` that `index` no longer lists or that are not `wanted`: a source dropped from the repo or
/// turned off, or a file it replaced (a recipe that became a plug-in). Icons stay for every listed source. Files still
/// being written (`.part`) are left alone.
pub fn prune(dir: &Path, index: &Index, wanted: &dyn Fn(&Entry) -> bool) {
    let Ok(files) = std::fs::read_dir(dir) else { return };
    let files_kept = index.sources.iter().filter(|e| wanted(e)).map(|e| cached_path(dir, e));
    let icons = index.sources.iter().map(|e| cached_icon_path(dir, e));
    let keep: std::collections::HashSet<PathBuf> = files_kept.chain(icons).chain([dir.join(INDEX_FILE)]).collect();
    for path in files.flatten().map(|f| f.path()) {
        if !keep.contains(&path) && path.extension().is_none_or(|ext| ext != "part") {
            if let Err(error) = std::fs::remove_file(&path) {
                tracing::warn!(%error, path = %path.display(), "a dropped source's copy could not be deleted");
            }
        }
    }
}

/// Writes `bytes` to `path` through a `.part` file, so a crash never leaves a half-written copy under the final name.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // a name of its own per write, so two fetches of one entry never rename each other's file
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let part = path.with_extension(format!("{}.{n}.part", std::process::id()));
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })
}

async fn get(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, RepoError> {
    get_within(client, url, MAX_FILE_BYTES).await
}

/// Downloads `url`; an answer larger than `max` bytes is refused.
async fn get_within(client: &reqwest::Client, url: &str, max: usize) -> Result<Vec<u8>, RepoError> {
    let mut response = client.get(url).send().await.map_err(|e| RepoError::Network(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RepoError::Status(status.as_u16()));
    }
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err(RepoError::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| RepoError::Network(e.to_string()))? {
        if bytes.len() + chunk.len() > max {
            return Err(RepoError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn entry(id: &str, kind: &str, lang: &str) -> String {
        format!(
            r#"{{"id":"{id}","name":"Name {id}","lang":"{lang}","kind":"{kind}","version":1,"file":"https://raw.example/{id}.toml","sha256":"{SHA_ABC}"}}"#
        )
    }

    fn index_of(entries: &[String]) -> String {
        format!(r#"{{"version":1,"sources":[{}]}}"#, entries.join(","))
    }

    #[test]
    fn a_source_of_an_unknown_kind_is_left_out_and_the_rest_are_read() {
        let index = parse_index(&index_of(&[entry("m", "manga", "en"), entry("film", "movie", "en"), entry("a", "anime", "ja"), entry("book", "novel", "en")])).expect("a new kind does not break the index");
        let ids: Vec<&str> = index.sources.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["m", "a"]);
        // a missing or malformed kind is still an error, as before
        let missing = entry("x", "manga", "en").replace(r#""kind":"manga","#, "");
        assert!(matches!(parse_index(&index_of(&[missing])), Err(RepoError::Parse(_))));
        let number = entry("x", "manga", "en").replace(r#""kind":"manga""#, r#""kind":3"#);
        assert!(matches!(parse_index(&index_of(&[number])), Err(RepoError::Parse(_))));
    }

    #[test]
    fn a_description_is_read_when_given_and_optional_otherwise() {
        let described = entry("a", "manga", "en").replace(r#""version":1"#, r#""version":1,"description":"Webtoons with dates""#);
        let index = parse_index(&index_of(&[described, entry("b", "manga", "en")])).unwrap();
        assert_eq!(index.sources[0].description.as_deref(), Some("Webtoons with dates"));
        assert_eq!(index.sources[1].description, None, "an older index has none");
        assert!(!serde_json::to_string(&index.sources[1]).unwrap().contains("description"), "none is not written out");
    }

    #[test]
    fn a_valid_index_is_read_and_grouped_by_language() {
        let text = index_of(&[entry("a.one", "manga", "en"), entry("b.two", "manga", "ja"), entry("c.three", "anime", "en")]);
        let index = parse_index(&text).expect("valid");
        let manga = by_language(&index, Kind::Manga);
        assert_eq!(manga.keys().copied().collect::<Vec<_>>(), vec!["en", "ja"]);
        assert_eq!(manga["en"][0].id, "a.one");
        assert_eq!(by_language(&index, Kind::Anime).len(), 1);
    }

    #[test]
    fn a_repository_may_list_any_number_of_sources() {
        let many: Vec<String> = (0..40).map(|i| entry(&format!("m{i}"), if i % 2 == 0 { "manga" } else { "anime" }, "en")).collect();
        assert_eq!(parse_index(&index_of(&many)).unwrap().sources.len(), 40);
    }

    #[test]
    fn a_duplicate_or_unsafe_id_is_refused() {
        let twice = index_of(&[entry("same", "manga", "en"), entry("same", "anime", "en")]);
        assert_eq!(parse_index(&twice), Err(RepoError::BadId("same".into())));
        let slash = index_of(&[entry("../escape", "manga", "en")]);
        assert_eq!(parse_index(&slash), Err(RepoError::BadId("../escape".into())));
    }

    #[test]
    fn a_bad_checksum_or_link_is_refused() {
        let short = index_of(&[entry("x", "manga", "en").replace(SHA_ABC, "abc")]);
        assert_eq!(parse_index(&short), Err(RepoError::BadChecksum("x".into())));
        let plain = index_of(&[entry("x", "manga", "en").replace("https://", "http://")]);
        assert_eq!(parse_index(&plain), Err(RepoError::BadLink("x".into())));
        let not_source = index_of(&[entry("x", "manga", "en").replace(".toml", ".exe")]);
        assert_eq!(parse_index(&not_source), Err(RepoError::BadLink("x".into())));
    }

    #[test]
    fn a_plug_in_link_is_accepted_and_its_copy_is_named_by_the_id_and_the_extension() {
        let text = index_of(&[entry("x", "anime", "en").replace(".toml", ".wasm")]);
        let index = parse_index(&text).expect("a .wasm link is a source too");
        assert_eq!(cached_path(Path::new("/c"), &index.sources[0]), PathBuf::from("/c/x.wasm"));
        let recipe = parse_index(&index_of(&[entry("y", "manga", "en")])).expect("valid");
        assert_eq!(cached_path(Path::new("/c"), &recipe.sources[0]), PathBuf::from("/c/y.toml"));
    }

    #[test]
    fn an_entry_with_a_type_field_is_read_and_the_kind_comes_from_the_link() {
        // the published index names each file "recipe" or "wasm"; the reader ignores that field
        let text = index_of(&[entry("x", "manga", "en").replace("\"kind\"", "\"type\":\"recipe\",\"kind\"")]);
        let index = parse_index(&text).expect("a type field is tolerated");
        assert_eq!(index.sources[0].kind, Kind::Manga);
        assert_eq!(cached_path(Path::new("/c"), &index.sources[0]), PathBuf::from("/c/x.toml"));
    }

    #[test]
    fn a_saved_copy_is_read_back_only_while_its_bytes_match_the_index() {
        let dir = tempfile::tempdir().expect("temp dir");
        let index = parse_index(&index_of(&[entry("ok", "manga", "en")])).expect("valid");
        std::fs::write(dir.path().join(INDEX_FILE), serde_json::to_string(&index).expect("json")).expect("write index");
        assert_eq!(read_index(dir.path()), Some(index.clone()));
        let source = &index.sources[0];
        std::fs::write(cached_path(dir.path(), source), b"abc").expect("write source");
        assert_eq!(read_source(dir.path(), source), Some(b"abc".to_vec()));
        std::fs::write(cached_path(dir.path(), source), b"changed").expect("overwrite source");
        assert_eq!(read_source(dir.path(), source), None, "a changed copy is not used");
    }

    #[test]
    fn a_status_is_read_and_a_missing_or_unknown_one_is_active() {
        let with = |status: &str| entry("x", "manga", "en").replace("\"version\"", &format!("\"status\":\"{status}\",\"version\""));
        let index = parse_index(&index_of(&[entry("a", "manga", "en"), with("blocked").replace("\"x\"", "\"b\""), with("broken").replace("\"x\"", "\"c\""), with("later").replace("\"x\"", "\"d\"")])).expect("valid");
        let statuses: Vec<Status> = index.sources.iter().map(|e| e.status).collect();
        assert_eq!(statuses, [Status::Active, Status::Blocked, Status::Broken, Status::Active]);
    }

    #[test]
    fn copies_the_index_no_longer_lists_are_deleted() {
        let dir = tempfile::tempdir().expect("temp dir");
        let index = parse_index(&index_of(&[entry("kept", "manga", "en")])).expect("valid");
        for name in [INDEX_FILE, "kept.toml", "kept.png", "gone.toml", "gone.png", "kept.wasm", "x.1.part"] {
            std::fs::write(dir.path().join(name), b"x").expect("write");
        }
        prune(dir.path(), &index, &|_| true);
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).expect("read").flatten().map(|f| f.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, [INDEX_FILE, "kept.png", "kept.toml", "x.1.part"]);
    }

    /// A source the user turned off is not on this computer: its file goes, its icon stays for the source list.
    #[test]
    fn the_file_of_a_source_that_is_not_wanted_is_deleted_and_its_icon_kept() {
        let dir = tempfile::tempdir().expect("temp dir");
        let index = parse_index(&index_of(&[entry("on", "manga", "en"), entry("off", "manga", "en")])).expect("valid");
        for name in [INDEX_FILE, "on.toml", "off.toml", "off.png"] {
            std::fs::write(dir.path().join(name), b"x").expect("write");
        }
        prune(dir.path(), &index, &|e| e.id == "on");
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).expect("read").flatten().map(|f| f.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, [INDEX_FILE, "off.png", "on.toml"]);
    }

    #[test]
    fn sha256_matches_the_standard_vector() {
        assert_eq!(sha256_hex(b"abc"), SHA_ABC);
    }

    #[test]
    fn links_must_be_https_and_are_kept_once() {
        let mut links = Vec::new();
        add_link(&mut links, "  https://host/index.min.json ").expect("added");
        add_link(&mut links, "https://host/index.min.json").expect("same link again");
        assert_eq!(links, vec!["https://host/index.min.json".to_owned()]);
        assert_eq!(add_link(&mut links, "http://host/index.min.json"), Err(RepoError::BadRepoLink));
        assert!(remove_link(&mut links, "https://host/index.min.json"));
        assert!(!remove_link(&mut links, "https://host/index.min.json"));
    }

    #[tokio::test]
    async fn a_source_file_is_returned_only_when_its_checksum_matches() {
        use wiremock::matchers::path;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(path("/ok.toml")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc".to_vec())).mount(&server).await;
        Mock::given(path("/bad.toml")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"xyz".to_vec())).mount(&server).await;
        let client = reqwest::Client::new();
        let mut good = parse_index(&index_of(&[entry("ok", "manga", "en")])).expect("valid").sources.remove(0);
        good.file = format!("{}/ok.toml", server.uri());
        assert_eq!(fetch_source(&client, &good).await.expect("matches"), b"abc".to_vec());
        let mut bad = good.clone();
        bad.file = format!("{}/bad.toml", server.uri());
        assert_eq!(fetch_source(&client, &bad).await, Err(RepoError::Mismatch));
    }

    #[tokio::test]
    async fn an_index_over_plain_http_is_refused_before_any_request() {
        let client = reqwest::Client::new();
        assert_eq!(fetch_index(&client, "http://host/index.min.json").await, Err(RepoError::BadRepoLink));
    }

    #[tokio::test]
    async fn a_source_is_fetched_when_it_has_no_copy_and_read_from_disk_after_that() {
        use wiremock::matchers::path;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        // two fetches: the first use, and the one after the copy is changed on disk
        Mock::given(path("/ok.toml"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc".to_vec()))
            .expect(2)
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let mut source = parse_index(&index_of(&[entry("ok", "manga", "en")])).expect("valid").sources.remove(0);
        source.file = format!("{}/ok.toml", server.uri());
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("sources");

        let first = ensure_cached(&client, &root, &source).await.expect("fetched");
        let second = ensure_cached(&client, &root, &source).await.expect("read from the copy");
        assert_eq!(first, second);
        assert_eq!(std::fs::read(&first).expect("the copy"), b"abc");

        std::fs::write(&first, b"changed").expect("overwrite the copy");
        ensure_cached(&client, &root, &source).await.expect("fetched again");
        assert_eq!(std::fs::read(&first).expect("the copy"), b"abc");
    }

    #[test]
    fn an_index_naming_another_https_address_has_moved_there() {
        let at = |url: &str| parse_index(&format!(r#"{{"version":1,"url":"{url}","sources":[]}}"#)).expect("valid");
        let old = "https://old.example/index.min.json";
        let new = "https://new.example/index.min.json";
        assert_eq!(moved_to(old, &at(new)), Some(new));
        assert_eq!(moved_to(old, &at(old)), None, "an index at its own address has not moved");
        assert_eq!(moved_to(old, &at(&format!(" {old} "))), None, "spaces do not make another address");
        assert_eq!(moved_to(old, &at("http://new.example/index.min.json")), None, "never to plain http");
        assert_eq!(moved_to(old, &at("https://new.example/page.html")), None, "only to an index file");
        assert_eq!(moved_to(old, &parse_index(&index_of(&[])).expect("valid")), None, "no url, no move");
        let saved = serde_json::to_string(&at(new)).expect("json");
        assert_eq!(parse_index(&saved).expect("saved copy").url.as_deref(), Some(new), "the saved copy keeps the address");
    }

    #[test]
    fn a_moved_link_takes_its_saved_copies_with_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        let (old, new) = ("https://old.example/index.min.json", "https://new.example/index.min.json");
        std::fs::create_dir_all(link_dir(root, old)).expect("old folder");
        std::fs::write(link_dir(root, old).join("x.wasm"), b"abc").expect("a copy");
        move_link_dir(root, old, new).expect("moved");
        assert!(!link_dir(root, old).exists());
        assert_eq!(std::fs::read(link_dir(root, new).join("x.wasm")).expect("the copy moved"), b"abc");
        move_link_dir(root, old, new).expect("no copies of the old link is no error");

        std::fs::create_dir_all(link_dir(root, old)).expect("old folder again");
        move_link_dir(root, old, new).expect("both links had copies");
        assert!(!link_dir(root, old).exists(), "the old copies are dropped");
        assert!(link_dir(root, new).join("x.wasm").exists(), "the new link's copies stay");
    }

    /// An entry JSON with an icon link and its checksum added, as a published index writes them.
    fn with_icon(entry: String, icon: &str, sha: &str) -> String {
        format!(r#"{},"icon":"{icon}","iconSha256":"{sha}"}}"#, entry.trim_end_matches('}'))
    }

    #[test]
    fn a_source_icon_is_read_with_or_without_its_link_and_checksum() {
        let with = parse_index(&index_of(&[with_icon(entry("x", "manga", "en"), "https://raw.example/x.png", SHA_ABC)])).expect("valid icon");
        assert_eq!(with.sources[0].icon.as_deref(), Some("https://raw.example/x.png"));
        assert_eq!(with.sources[0].icon_sha256.as_deref(), Some(SHA_ABC));
        let without = parse_index(&index_of(&[entry("y", "manga", "en")])).expect("an icon is optional");
        assert_eq!((without.sources[0].icon.as_deref(), without.sources[0].icon_sha256.as_deref()), (None, None));
        let plain = index_of(&[with_icon(entry("x", "manga", "en"), "http://raw.example/x.png", SHA_ABC)]);
        assert_eq!(parse_index(&plain), Err(RepoError::BadLink("x".into())));
        let jpg = index_of(&[with_icon(entry("x", "manga", "en"), "https://raw.example/x.jpg", SHA_ABC)]);
        assert_eq!(parse_index(&jpg), Err(RepoError::BadLink("x".into())));
        let short = index_of(&[with_icon(entry("x", "manga", "en"), "https://raw.example/x.png", "abc")]);
        assert_eq!(parse_index(&short), Err(RepoError::BadChecksum("x".into())));
    }

    #[tokio::test]
    async fn an_icon_is_saved_once_and_an_icon_with_a_bad_checksum_leaves_no_file() {
        use wiremock::matchers::path;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        // one fetch: the second call finds the saved icon and does not ask again
        Mock::given(path("/good.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/bad.png")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"xyz".to_vec())).mount(&server).await;
        let client = reqwest::Client::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let mut good = parse_index(&index_of(&[with_icon(entry("ok", "manga", "en"), "https://raw.example/ok.png", SHA_ABC)])).expect("valid").sources.remove(0);
        good.icon = Some(format!("{}/good.png", server.uri()));
        ensure_icon(&client, dir.path(), &good).await.expect("saved");
        ensure_icon(&client, dir.path(), &good).await.expect("kept");
        assert_eq!(read_icon(dir.path(), &good), Some(cached_icon_path(dir.path(), &good)));

        let mut bad = good.clone();
        bad.id = "bad".into();
        bad.icon = Some(format!("{}/bad.png", server.uri()));
        assert_eq!(ensure_icon(&client, dir.path(), &bad).await, Err(RepoError::Mismatch));
        assert!(!cached_icon_path(dir.path(), &bad).exists(), "a bad checksum leaves no file");
        assert_eq!(read_icon(dir.path(), &bad), None);
    }
}
