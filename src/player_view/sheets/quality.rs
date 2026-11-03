//! The Quality and Server lists of an online episode, in the menu of the bar's Quality button. When the episode starts,
//! its servers and their videos load on the source's runtime ([`preload`]) and are kept for the episode; a menu opened
//! before then says "Quality: loading…". One lists the heights ("1080p, 720p, 360p"), the other the servers. Choosing a height plays it on the playing server (else on the first server that has
//! it); choosing a server plays the playing height there (else its first video). The stream resolves and reloads at the
//! playing position, with that video's headers, subtitles and audio. The last choice is kept for the session: the next
//! episode plays the same server and title when its list has them, else the default quality of the settings, else the
//! source's best video. A local episode has neither list. The menu gets them through [`menu_items`], over the loaded
//! servers ([`Servers`]).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use kuro_data::entry::EntryRepo;
use kuro_data::item::ItemRepo;
use kuro_domain::source::remote_update::to_s_episode;
use kuro_i18n::{MsgKey, tr};
use kuro_source::SourceError;
use kuro_source::anime::{
    AnimeSource, Hoster, HosterState, NO_HOSTER_LIST, SEpisode, Video, best_video, http_hosters, load_hoster, resolve_once,
};

use gtk::prelude::*;

use crate::widgets::context_menu::Item;

use super::super::{Media, Shared, source};
use super::Choice;

/// A server and a video title: the choice the session keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    hoster: String,
    title: String,
}

/// The last choice, which is also the one playing. A static lock, because [`choose`] runs on the source's runtime,
/// not on the GTK thread.
static LAST: Mutex<Option<Pick>> = Mutex::new(None);

fn last() -> Option<Pick> {
    LAST.lock().ok().and_then(|guard| (*guard).clone())
}

fn set_last(pick: Option<Pick>) {
    if let Ok(mut last) = LAST.lock() {
        *last = pick;
    }
}

thread_local! {
    /// The choice whose stream resolves now, picked in the menu: the button and the menu show it at once, until it plays
    /// (it becomes [`last`]) or fails.
    static SWITCHING: RefCell<Option<Pick>> = const { RefCell::new(None) };
}

fn switching() -> Option<Pick> {
    SWITCHING.with(|s| s.borrow().clone())
}

fn set_switching(pick: Option<Pick>) {
    SWITCHING.with(|s| *s.borrow_mut() = pick);
}

/// The choice the button and the menu show: the one resolving, else the one playing.
fn shown() -> Option<Pick> {
    switching().or_else(last)
}

/// The servers of an episode that the open menu lists, kept for the episode they belong to.
struct Loaded {
    episode_id: i64,
    states: Vec<HosterState>,
}

/// The servers of the playing episode, loaded once per episode for the Quality button. Cloning shares them.
#[derive(Clone, Default)]
pub struct Servers(Rc<ServersInner>);

#[derive(Default)]
struct ServersInner {
    loaded: RefCell<Option<Loaded>>,
    /// The episode whose servers are loading: asking again meanwhile does not ask the source twice.
    pending: Cell<Option<i64>>,
}

impl Servers {
    /// The loaded servers of `episode_id`, or `None` while they load. The first ask starts the load from `known` (the
    /// servers the episode played from; empty: asked from the source) and `done` runs with its answer; an ask while it
    /// runs starts nothing and its `done` is dropped. An answer for an episode that is no longer the one loading (the
    /// next episode started meanwhile) is dropped too, so it neither ends the newer load nor replaces its servers.
    fn get_or_load(&self, episode_id: i64, source: Arc<dyn AnimeSource>, episode: SEpisode, known: Vec<Hoster>, done: impl FnOnce(Result<Vec<HosterState>, String>) + 'static) -> Option<Vec<HosterState>> {
        let cached = self.0.loaded.borrow().as_ref().filter(|loaded| loaded.episode_id == episode_id).map(|loaded| loaded.states.clone());
        if cached.is_some() {
            return cached;
        }
        if self.0.pending.get() == Some(episode_id) {
            return None;
        }
        self.0.pending.set(Some(episode_id));
        let this = self.clone();
        load(source, episode, known, move |answer| {
            if this.0.pending.get() != Some(episode_id) {
                return;
            }
            this.0.pending.set(None);
            if let Ok(states) = &answer {
                *this.0.loaded.borrow_mut() = Some(Loaded { episode_id, states: states.clone() });
            }
            done(answer);
        });
        None
    }
}

/// "1080p".
pub fn height_label(height: i32) -> String {
    format!("{height}p")
}

/// The first height in a video title ("1080p", "SubsPlease · 720p (1280x720)").
fn title_height(title: &str) -> Option<i32> {
    static HEIGHT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\b(\d{3,4})p\b").expect("a valid pattern"));
    HEIGHT.captures(title).and_then(|c| c[1].parse().ok())
}

/// The height of a video: the source's `resolution`, else the one in its title.
fn height(video: &Video) -> Option<i32> {
    video.resolution.filter(|r| *r > 0).or_else(|| title_height(&video.video_title))
}



/// The entries of the bar's Quality menu: the heights, then a Server submenu (a source that lists no servers has none).
/// The servers load when the episode starts ([`preload`]); a menu opened before they are in shows a greyed-out
/// "Quality: loading…" and is not changed while it shows: the next opening has the list. A failed load is asked again
/// by that opening, and its failure is a toast.
pub fn menu_items(shared: &Shared, servers: &Servers) -> Vec<Item> {
    let episode_id = shared.borrow().episode_id;
    let Some((source, episode)) = online_episode(episode_id) else { return Vec::new() };
    let known = shared.borrow().media.hosters.clone();
    let states = servers.get_or_load(episode_id, Arc::clone(&source), episode, known, |answer| {
        if let Err(text) = answer {
            crate::toast::notify(text);
        }
    });
    let Some(states) = states else {
        return vec![Item::new(tr(MsgKey::ui_quality_loading), 0, || {}).disabled()];
    };
    let mut items = Vec::new();
    // the heights first, as the menu's own entries; the servers in their submenu
    for (index, (heading, choices)) in sections(shared, &source, &states).into_iter().enumerate() {
        if index == 0 {
            items.extend(super::items(choices));
        } else if !choices.is_empty() {
            items.push(Item::submenu(heading.unwrap_or_default(), 0, super::items(choices)));
        }
    }
    items
}

/// Loads the servers of the playing episode from the ones it played from, so the Quality menu has them when it opens.
/// A file (a download) has none and asks nothing here: the menu asks the source when opened. A failure is only logged
/// here: opening the menu asks again and says so.
pub fn preload(shared: &Shared, servers: &Servers) {
    let (episode_id, known) = {
        let player = shared.borrow();
        (player.episode_id, player.media.hosters.clone())
    };
    if known.is_empty() {
        return;
    }
    let Some((source, episode)) = online_episode(episode_id) else { return };
    servers.get_or_load(episode_id, source, episode, known, |answer| {
        if let Err(text) = answer {
            tracing::debug!(error = %text, "player: the servers did not load ahead");
        }
    });
}

/// Whether `episode_id` is an episode of an installed source (the Quality button shows only then).
pub fn is_online(episode_id: i64) -> bool {
    online_episode(episode_id).is_some()
}

/// Shows the playing height on the Quality button, or its icon while the height is not known. A height just picked shows
/// at once, greyed out until its stream plays: a second pick could not start meanwhile. Called on every tick, so the
/// button changes only when it differs.
pub fn refresh(button: &gtk::MenuButton) {
    let busy = switching().is_some();
    if button.is_sensitive() == busy {
        button.set_sensitive(!busy);
    }
    match shown().and_then(|pick| title_height(&pick.title)) {
        Some(height) => {
            let text = height_label(height);
            if button.label().as_deref() != Some(text.as_str()) {
                button.set_label(&text);
            }
        }
        None if button.icon_name().is_none() => button.set_icon_name("video-display-symbolic"),
        None => {}
    }
}

/// The video an episode plays (the source's stream choice; see `best_stream` in `source.rs`). The last choice is
/// used when this episode's servers have the same server and title; the choice made here is then kept for the session.
pub async fn choose(source: &dyn AnimeSource, hosters: &mut [Hoster], quality: i32) -> Result<Option<Video>, SourceError> {
    if let Some(pick) = last() {
        if let Some(video) = remembered(source, hosters, &pick).await {
            if let Some(resolved) = resolve_once(source, video).await? {
                set_last(Some(pick));
                return Ok(Some(resolved));
            }
        }
    }
    if let Some((pick, video)) = with_height(source, hosters, quality).await {
        if let Some(resolved) = resolve_once(source, video).await? {
            set_last(Some(pick));
            return Ok(Some(resolved));
        }
    }
    let best = best_video(source, hosters).await?;
    set_last(best.as_ref().and_then(|video| identify(hosters, &video.video_title)));
    Ok(best)
}

/// The video a download takes: the first video of the `quality` height, else the source's best video. Unlike [`choose`]
/// it neither reads nor keeps the session's last choice, which belongs to what is playing.
pub(crate) async fn default_or_best(source: &dyn AnimeSource, hosters: &[Hoster], quality: i32) -> Result<Option<Video>, SourceError> {
    let mut hosters = hosters.to_vec();
    if let Some((_, video)) = with_height(source, &mut hosters, quality).await {
        if let Some(resolved) = resolve_once(source, video).await? {
            return Ok(Some(resolved));
        }
    }
    best_video(source, &hosters).await
}

/// The videos of `hoster`: its list, else fetched now and kept on it, so the Quality menu does not ask again.
async fn videos_of(source: &dyn AnimeSource, hoster: &mut Hoster) -> Result<Vec<Video>, SourceError> {
    if let Some(videos) = &hoster.video_list {
        return Ok(videos.clone());
    }
    let videos = source.videos_for_hoster(hoster).await?;
    hoster.video_list = Some(videos.clone());
    Ok(videos)
}

/// The first video of `height`, servers in the source's order (its preferred first). A lazy server's list is fetched here.
async fn with_height(source: &dyn AnimeSource, hosters: &mut [Hoster], wanted: i32) -> Option<(Pick, Video)> {
    for hoster in hosters {
        let videos = videos_of(source, hoster).await.unwrap_or_default();
        if let Some(video) = videos.into_iter().find(|v| height(v) == Some(wanted)) {
            return Some((Pick { hoster: hoster.hoster_name.clone(), title: video.video_title.clone() }, video));
        }
    }
    None
}

/// The video of `pick` among its server's videos. A lazy server's list is fetched here.
async fn remembered(source: &dyn AnimeSource, hosters: &mut [Hoster], pick: &Pick) -> Option<Video> {
    for hoster in hosters.iter_mut().filter(|h| h.hoster_name == pick.hoster) {
        let Ok(videos) = videos_of(source, hoster).await else { continue };
        if let Some(video) = videos.into_iter().find(|v| v.video_title == pick.title) {
            return Some(video);
        }
    }
    None
}

/// The server that lists a video called `title`. Only servers whose list is already known can say.
fn identify(hosters: &[Hoster], title: &str) -> Option<Pick> {
    hosters
        .iter()
        .find(|h| h.video_list.as_ref().is_some_and(|videos| videos.iter().any(|v| v.video_title == title)))
        .map(|h| Pick { hoster: h.hoster_name.clone(), title: title.to_owned() })
}

/// The source and the episode row of `episode_id`. `None` for a local episode, an unknown row or a source that is not
/// installed.
fn online_episode(episode_id: i64) -> Option<(Arc<dyn AnimeSource>, SEpisode)> {
    if !crate::data::is_installed() {
        return None;
    }
    let db = &crate::data::app().db;
    let episode = ItemRepo::new(Arc::clone(db)).by_id(episode_id).ok()??;
    let anime = EntryRepo::new(Arc::clone(db)).by_id(episode.entry_id).ok()?;
    let source = crate::screens::anime_sources().into_iter().find(|s| s.id() == anime.source)?;
    Some((source, to_s_episode(&episode)))
}

/// The servers that loaded and have videos, in the source's order: name and videos.
fn ready(states: &[HosterState]) -> Vec<(&str, Vec<&Video>)> {
    states
        .iter()
        .filter_map(|state| match state {
            HosterState::Ready { hoster_name, videos } if !videos.is_empty() => Some((hoster_name.as_str(), videos.iter().map(|e| &e.video).collect())),
            _ => None,
        })
        .collect()
}

/// What a pick of the menu plays: `height` on `server` (else the first server that has it), or for a video without a
/// height, its own title.
fn find<'a>(servers: &[(&'a str, Vec<&'a Video>)], server: Option<&str>, wanted: &Key) -> Option<(&'a str, &'a Video)> {
    let on = |name: &'a str, videos: &[&'a Video]| videos.iter().find(|v| key(v) == *wanted).map(|v| (name, *v));
    server.and_then(|s| servers.iter().find(|(name, _)| *name == s)).and_then(|(name, videos)| on(name, videos)).or_else(|| servers.iter().find_map(|(name, videos)| on(name, videos)))
}

/// A row of the Quality list: a height, or the title of a video that names none.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Height(i32),
    Title(String),
}

fn key(video: &Video) -> Key {
    height(video).map_or_else(|| Key::Title(video.video_title.clone()), Key::Height)
}

fn key_label(key: &Key) -> String {
    match key {
        Key::Height(h) => height_label(*h),
        Key::Title(t) => t.clone(),
    }
}

/// The Quality list (every height any server has, highest first) and the Server list (a source that lists no servers
/// has none).
fn sections(shared: &Shared, source: &Arc<dyn AnimeSource>, states: &[HosterState]) -> Vec<(Option<String>, Vec<Choice>)> {
    let servers = ready(states);
    let playing = shown();
    let playing_key = playing.as_ref().and_then(|pick| servers.iter().find(|(n, _)| *n == pick.hoster).and_then(|(_, vs)| vs.iter().find(|v| v.video_title == pick.title)).map(|v| key(v)));
    let playing_server = playing.as_ref().map(|pick| pick.hoster.clone());
    let mut keys: Vec<Key> = Vec::new();
    for video in servers.iter().flat_map(|(_, videos)| videos) {
        let k = key(video);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    keys.sort_by(|a, b| match (a, b) {
        (Key::Height(x), Key::Height(y)) => y.cmp(x),
        (Key::Height(_), Key::Title(_)) => std::cmp::Ordering::Less,
        (Key::Title(_), Key::Height(_)) => std::cmp::Ordering::Greater,
        (Key::Title(_), Key::Title(_)) => std::cmp::Ordering::Equal,
    });
    let qualities = keys
        .iter()
        .filter_map(|k| {
            let (server, video) = find(&servers, playing_server.as_deref(), k)?;
            Some(choice(shared, source, server, video, key_label(k), playing_key.as_ref() == Some(k)))
        })
        .collect();
    let mut out = Vec::new();
    out.push((Some(tr(MsgKey::player_quality)), qualities));
    let failed: Vec<&str> = states.iter().filter_map(|s| match s {
        HosterState::Error { hoster_name } => Some(hoster_name.as_str()),
        _ => None,
    }).collect();
    let named = servers.iter().map(|(name, _)| *name).chain(failed.iter().copied()).any(|name| heading(name).is_some());
    if named {
        let mut rows: Vec<Choice> = servers
            .iter()
            .filter_map(|(name, videos)| {
                // the playing height on this server, else its first video
                let video = playing_key.as_ref().and_then(|k| videos.iter().find(|v| key(v) == *k)).or_else(|| videos.first())?;
                let label = heading(name).unwrap_or_default().to_owned();
                Some(choice(shared, source, name, video, label, playing_server.as_deref() == Some(*name)))
            })
            .collect();
        rows.extend(failed.iter().map(|name| Choice { title: (*name).to_owned(), subtitle: Some(tr(MsgKey::ui_could_not_load)), current: false, enabled: false, pick: Box::new(|| {}) }));
        out.push((Some(tr(MsgKey::player_server)), rows));
    }
    out
}

fn heading(hoster_name: &str) -> Option<&str> {
    (hoster_name != NO_HOSTER_LIST).then_some(hoster_name)
}

fn choice(shared: &Shared, source: &Arc<dyn AnimeSource>, hoster: &str, video: &Video, title: String, current: bool) -> Choice {
    let pick = Pick { hoster: hoster.to_owned(), title: video.video_title.clone() };
    let (shared, source, video, shown) = (Rc::clone(shared), Arc::clone(source), video.clone(), title.clone());
    Choice {
        title,
        subtitle: None,
        current,
        enabled: true,
        pick: Box::new(move || {
            // the quality picked is kept for the anime, as its audio and subtitles are
            if let Some(height) = height(&video) {
                super::super::remember_quality(shared.borrow().episode_id, height);
            }
            switch(&shared, Arc::clone(&source), pick.clone(), video.clone(), &shown);
        }),
    }
}

/// The servers of an episode with their videos, every one loaded (a lazy server too). Runs on the source's runtime and
/// calls `done` on the GTK thread.
fn load(source: Arc<dyn AnimeSource>, episode: SEpisode, known: Vec<Hoster>, done: impl FnOnce(Result<Vec<HosterState>, String>) + 'static) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        done(Err(tr(MsgKey::player_hoster_failed)));
        return;
    };
    crate::background::run_async_on(
        &handle,
        async move {
            // the spinner of the menu never stays: after 30 seconds the toast says the source is not answering
            let answer = crate::browse::errors::settle(crate::browse::errors::LOAD_TIMEOUT, source.name(), all_states(source.as_ref(), &episode, known)).await;
            answer
        },
        move |answer| {
            match answer {
                Some(answer) => done(answer),
                // a closed channel means the source task panicked
                None => done(Err(tr(MsgKey::no_hosters))),
            }
        },
    );
}

/// Every server of an episode with its videos. `known` is the list the episode played from (empty: asked from the source
/// now). A server whose videos it already holds costs no request; the others load at the same time.
async fn all_states(source: &dyn AnimeSource, episode: &SEpisode, known: Vec<Hoster>) -> Result<Vec<HosterState>, String> {
    let hosters = if known.is_empty() { crate::browse::errors::plain(source.name(), http_hosters(source, episode)).await? } else { known };
    let loads = hosters.iter().map(|hoster| async move {
        load_hoster(source, hoster, true).await.unwrap_or_else(|_| HosterState::Error { hoster_name: hoster.hoster_name.clone() })
    });
    Ok(futures::future::join_all(loads).await)
}

/// Plays `video` of `pick` in the open page: its stream resolves in the background (a spinner shows), then the page
/// reloads at the playing position. The Quality button shows the choice meanwhile, greyed out ([`refresh`]); a failure
/// puts back the one playing. Nothing starts while another stream resolves. `shown` is said on the video at once.
fn switch(shared: &Shared, source: Arc<dyn AnimeSource>, pick: Pick, video: Video, shown: &str) {
    let title = {
        let mut player = shared.borrow_mut();
        if player.resolving {
            return;
        }
        player.resolving = true;
        let title = player.media.title.clone();
        title
    };
    set_switching(Some(pick.clone()));
    shared.borrow().playback.show_text(shown);
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        set_switching(None);
        shared.borrow_mut().resolving = false;
        crate::toast::notify(tr(MsgKey::player_hoster_failed));
        return;
    };
    let shared = Rc::clone(shared);
    crate::background::run_async_on(
        &handle,
        async move {
            let answer = match crate::browse::errors::plain(source.name(), resolve_once(source.as_ref(), video)).await {
                // the chosen stream's links are checked like the first one's (`source::allowed`)
                Ok(Some(resolved)) => super::super::source::allowed(source.id(), resolved).await.map(Some),
                other => other,
            };
            answer
        },
        move |answer| {
            // a closed channel means the source task panicked
            let answer = answer.unwrap_or_else(|| Err(tr(MsgKey::video_list_empty_error)));
            set_switching(None);
            shared.borrow_mut().resolving = false;
            match answer {
                Ok(Some(resolved)) => {
                    set_last(Some(pick));
                    let (preferred, preferred_audio) = super::super::preferred_languages(shared.borrow().episode_id);
                    reload(&shared, source::media_of(resolved, title, &preferred, &preferred_audio));
                }
                Ok(None) => crate::toast::notify(tr(MsgKey::video_list_empty_error)),
                Err(text) => crate::toast::notify(text),
            }
        },
    );
}

/// Loads `media` at the playing position and keeps the pause state. Its side tracks are added when mpv has loaded it,
/// with the audio and subtitles picked now selected again. The last frame stays over the video until the new stream
/// plays (`Still`), so the switch shows no black and no new player.
fn reload(shared: &Shared, media: Media) {
    if shared.borrow().closed {
        return;
    }
    let still = shared.borrow().still.clone();
    still.hold();
    let mut player = shared.borrow_mut();
    player.held = super::super::Held::Loading;
    let start = player.playback.position_ms().unwrap_or(0);
    let paused = player.playback.is_paused();
    // the same episode: the servers it played from stay
    let hosters = std::mem::take(&mut player.media.hosters);
    let mut media = Media { hosters, ..media };
    super::super::menu::picked(player.playback.mpv()).apply(&mut media.tracks);
    player.media = media;
    if let Err(e) = player.playback.load(&player.media.target, player.media.headers.as_deref(), start) {
        player.held = super::super::Held::No;
        drop(player);
        still.release();
        tracing::error!(error = %e, "player: the chosen quality could not load");
        crate::toast::notify(tr(MsgKey::player_hoster_failed));
        return;
    }
    if let Err(e) = player.playback.mpv().set_property_flag("pause", paused) {
        tracing::warn!(error = %e, "player: the pause state was not restored");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hoster(name: &str, titles: &[&str]) -> Hoster {
        let videos = titles.iter().map(|t| Video::new(format!("https://cdn.test/{t}"), *t)).collect();
        Hoster { hoster_name: name.into(), video_list: Some(videos), ..Hoster::default() }
    }

    #[test]
    fn a_title_is_identified_by_the_server_that_lists_it() {
        let hosters = vec![hoster("Alpha", &["480p"]), hoster("Beta", &["1080p", "720p"])];
        assert_eq!(identify(&hosters, "720p"), Some(Pick { hoster: "Beta".into(), title: "720p".into() }));
        assert_eq!(identify(&hosters, "2160p"), None, "a title no server lists is not identified");
    }

    #[test]
    fn a_server_with_a_lazy_list_cannot_be_identified_yet() {
        let lazy = Hoster { hoster_name: "Lazy".into(), lazy: true, ..Hoster::default() };
        assert_eq!(identify(&[lazy], "1080p"), None);
    }

    #[test]
    fn the_height_comes_from_the_resolution_or_the_title() {
        assert_eq!(title_height("1080p"), Some(1080));
        assert_eq!(title_height("SubsPlease · 720p (1280x720)"), Some(720));
        assert_eq!(title_height("Default"), None);
        let titled = Video::new("u", "Server - 480p");
        assert_eq!(height(&titled), Some(480));
        assert_eq!(height(&Video { resolution: Some(360), ..titled }), Some(360), "the source's resolution first");
        assert_eq!(height_label(1080), "1080p");
    }

    #[test]
    fn a_height_plays_on_the_playing_server_else_the_first_that_has_it() {
        let (a1080, a720, b480) = (Video::new("a1", "1080p"), Video::new("a2", "720p"), Video::new("b1", "480p"));
        let servers = vec![("A", vec![&a1080, &a720]), ("B", vec![&b480])];
        assert_eq!(find(&servers, Some("B"), &Key::Height(480)).map(|(s, v)| (s, v.video_url.as_str())), Some(("B", "b1")));
        assert_eq!(find(&servers, Some("B"), &Key::Height(720)).map(|(s, _)| s), Some("A"), "B has no 720p");
        assert_eq!(find(&servers, None, &Key::Height(2160)), None);
    }

    #[test]
    fn the_legacy_list_has_no_heading() {
        assert_eq!(heading(NO_HOSTER_LIST), None);
        assert_eq!(heading("Alpha"), Some("Alpha"));
    }
}
