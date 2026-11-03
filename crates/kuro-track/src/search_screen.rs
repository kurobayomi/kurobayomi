//! Logic of the track search screen (`AnimeTrackerSearch.kt`, `MangaTrackerSearch.kt`). The
//! GUI draws it; this holds the query rules, the body state, the selection, the bottom bar and the result card.

use crate::error::TrackError;
use crate::messages::MsgKey;
use crate::model::TrackerId;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};

/// What the screen needs from a search result. Both kinds implement it.
pub trait SearchHit: Clone {
    /// The list key `(tracker, remote id)`. Two results of different trackers never collide (D10).
    fn list_key(&self) -> (TrackerId, i64);
    fn title(&self) -> &str;
    /// Empty when the tracker did not set one (D9). The open action is then disabled.
    fn tracking_url(&self) -> &str;
    /// The URL "open in browser" opens. The tracking URL unless the result also carries an absolute `source_url`
    /// (Simkl, whose tracking URL is a relative `/anime/<id>`).
    fn open_url(&self) -> &str {
        self.tracking_url()
    }
    fn card(&self) -> CardView;
    fn set_private(&mut self, private: bool);
}

/// One detail line of a card: a label key with its English text, and the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailLine {
    pub label_key: &'static str,
    pub label: &'static str,
    pub value: String,
}

/// Everything a result card shows. Empty optional parts are `None` or absent (behaviour 15.4).
#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    pub title: String,
    /// Authors and artists, distinct, joined with ", ". `None` when both lists are empty.
    pub byline: Option<String>,
    /// Cover image URL. `None` when the tracker has none.
    pub cover_url: Option<String>,
    /// Type, Started, Status, Count and Score, in that order. Only the lines with a value are present. A `my:` row
    /// has no Status line.
    pub details: Vec<DetailLine>,
    /// The summary trimmed, at most drawn as four lines by the GUI. `None` when blank.
    pub summary: Option<String>,
    pub can_open: bool,
    /// The card offers Refresh: it is a row of the `my:` search (T-29a-049).
    pub refreshable: bool,
}

/// Lower-cases the text and capitalises its first character ("ONGOING" becomes "Ongoing").
pub fn capitalise_lower(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A score as a whole number (E-56, a deliberate change from the Double text of the source). Truncates like `toInt()`.
pub fn score_text(score: f64) -> String {
    (score.trunc() as i64).to_string()
}

/// Fields of one result that the card reads. Anime and manga results both map to it.
struct CardFields<'a> {
    title: &'a str,
    authors: &'a [String],
    artists: &'a [String],
    publishing_type: &'a str,
    start_date: &'a str,
    publishing_status: &'a str,
    score: Option<f64>,
    summary: &'a str,
    open_url: &'a str,
    cover_url: &'a str,
    /// Chapters or episodes; 0 means unknown and the line is left out.
    count: i64,
    count_label: (&'static str, &'static str),
    from_my_list: bool,
}

fn card(f: CardFields<'_>) -> CardView {
    let mut names: Vec<&str> = Vec::new();
    for name in f.authors.iter().chain(f.artists.iter()) {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    let mut details = Vec::new();
    let kind = capitalise_lower(f.publishing_type);
    if !kind.is_empty() {
        details.push(DetailLine {
            label_key: "track_type",
            label: "Type",
            value: kind,
        });
    }
    if !f.start_date.is_empty() {
        details.push(DetailLine {
            label_key: "label_started",
            label: "Started",
            value: f.start_date.to_owned(),
        });
    }
    // E-37: the API's underscores read as spaces ("ON_HOLD" is "On hold").
    let status = capitalise_lower(&f.publishing_status.replace('_', " "));
    if !status.is_empty() && !f.from_my_list {
        details.push(DetailLine {
            label_key: "track_status",
            label: "Status",
            value: status,
        });
    }
    if f.count > 0 {
        details.push(DetailLine {
            label_key: f.count_label.0,
            label: f.count_label.1,
            value: f.count.to_string(),
        });
    }
    if let Some(score) = f.score {
        details.push(DetailLine {
            label_key: "score",
            label: "Score",
            value: score_text(score),
        });
    }
    let summary = Some(f.summary.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    CardView {
        title: f.title.to_owned(),
        byline: (!names.is_empty()).then(|| names.join(", ")),
        cover_url: Some(f.cover_url)
            .filter(|c| !c.is_empty())
            .map(str::to_owned),
        details,
        summary,
        can_open: !f.open_url.is_empty(),
        refreshable: f.from_my_list,
    }
}

impl SearchHit for AnimeTrackSearch {
    fn list_key(&self) -> (TrackerId, i64) {
        AnimeTrackSearch::list_key(self)
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn tracking_url(&self) -> &str {
        &self.tracking_url
    }
    fn open_url(&self) -> &str {
        AnimeTrackSearch::open_url(self)
    }
    fn card(&self) -> CardView {
        card(CardFields {
            title: &self.title,
            authors: &self.authors,
            artists: &self.artists,
            publishing_type: &self.publishing_type,
            start_date: &self.start_date,
            publishing_status: &self.publishing_status,
            score: self.score,
            summary: &self.summary,
            open_url: AnimeTrackSearch::open_url(self),
            cover_url: &self.cover_url,
            count: self.total_episodes,
            count_label: ("episodes", "Episodes"),
            from_my_list: self.from_my_list,
        })
    }
    fn set_private(&mut self, private: bool) {
        self.private = private;
    }
}

impl SearchHit for MangaTrackSearch {
    fn list_key(&self) -> (TrackerId, i64) {
        MangaTrackSearch::list_key(self)
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn tracking_url(&self) -> &str {
        &self.tracking_url
    }
    fn card(&self) -> CardView {
        card(CardFields {
            title: &self.title,
            authors: &self.authors,
            artists: &self.artists,
            publishing_type: &self.publishing_type,
            start_date: &self.start_date,
            publishing_status: &self.publishing_status,
            score: self.score,
            summary: &self.summary,
            open_url: &self.tracking_url,
            cover_url: &self.cover_url,
            count: self.total_chapters,
            count_label: ("chapters", "Chapters"),
            from_my_list: self.from_my_list,
        })
    }
    fn set_private(&mut self, private: bool) {
        self.private = private;
    }
}

/// Body state of the screen (behaviour 15.3). `Idle` is the screen before any query ran, which the source leaves
/// as a spinner forever when the title is blank.
#[derive(Debug, Clone, PartialEq)]
pub enum SearchState<R> {
    Idle,
    Loading,
    Empty,
    /// The message, or "Unknown error" when the failure carries none.
    Error(String),
    Results(Vec<R>),
}

/// One search screen for one entry and kind.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchScreen<R: SearchHit> {
    pub query: String,
    supports_private: bool,
    /// The `(tracker, remote id)` of the binding being edited; its result is preselected (E-57).
    bound: Option<(TrackerId, i64)>,
    state: SearchState<R>,
    selected: Option<(TrackerId, i64)>,
    /// Counts the queries started. Only the latest one may write results (D-6, E-32, E-33).
    generation: u64,
    /// True while a bind is being written. Both Track buttons are disabled then (T-28a-057).
    in_flight: bool,
}

impl<R: SearchHit> SearchScreen<R> {
    /// The screen opens with the title in the field. The returned query runs once when the title is not blank.
    /// `bound` is the `(tracker, remote id)` of the binding being edited, if any (076, E-57).
    pub fn open(
        initial_title: &str,
        bound: Option<(TrackerId, i64)>,
        supports_private: bool,
    ) -> (Self, Option<String>) {
        let mut screen = Self {
            query: initial_title.to_owned(),
            supports_private,
            bound,
            state: SearchState::Idle,
            selected: None,
            generation: 0,
            in_flight: false,
        };
        let first = screen.begin(initial_title);
        (screen, first)
    }

    /// Starts a query. A blank query is not sent: the function returns `None` and nothing changes (D11). Otherwise
    /// the state goes to loading and the selection is cleared (076). The caller runs the tracker search with the
    /// returned text.
    pub fn begin(&mut self, text: &str) -> Option<String> {
        if text.trim().is_empty() {
            return None;
        }
        self.generation += 1;
        self.start_loading();
        Some(text.to_owned())
    }

    /// Runs the query in the field again (the Retry button of the error state).
    pub fn retry(&mut self) -> Option<String> {
        let query = self.query.clone();
        self.begin(&query)
    }

    /// The generation of the latest query. The caller keeps it to pass to [`SearchScreen::finish_for`].
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Applies the result of the latest query. Same as [`SearchScreen::finish_for`] with the current generation.
    pub fn finish(&mut self, result: Result<Vec<R>, TrackError>) {
        let generation = self.generation;
        self.finish_for(generation, result);
    }

    /// Applies the result of the query `generation`. A result of an earlier query is dropped and `false` is
    /// returned (D-6). Otherwise the result whose key equals `bound` is selected (076, E-57).
    pub fn finish_for(&mut self, generation: u64, result: Result<Vec<R>, TrackError>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.selected = None;
        self.state = match result {
            Ok(hits) if hits.is_empty() => SearchState::Empty,
            Ok(hits) => {
                self.selected = self.auto_select(&hits);
                SearchState::Results(hits)
            }
            Err(e) => {
                let text = e.to_string();
                SearchState::Error(if text.trim().is_empty() {
                    crate::messages::english(&MsgKey::UnknownError)
                } else {
                    text
                })
            }
        };
        true
    }

    /// Marks a search as running: loading, nothing selected. [`SearchScreen::begin`] already calls it.
    pub fn start_loading(&mut self) {
        self.selected = None;
        self.state = SearchState::Loading;
    }

    fn auto_select(&self, hits: &[R]) -> Option<(TrackerId, i64)> {
        let bound = self.bound?;
        hits.iter().map(SearchHit::list_key).find(|k| *k == bound)
    }

    pub fn state(&self) -> &SearchState<R> {
        &self.state
    }

    /// A click on a card selects it. Clicking the selected card keeps it selected.
    pub fn select(&mut self, key: (TrackerId, i64)) {
        let known = match &self.state {
            SearchState::Results(hits) => hits.iter().any(|h| h.list_key() == key),
            _ => false,
        };
        if known {
            self.selected = Some(key);
        }
    }

    pub fn selected(&self) -> Option<&R> {
        match (&self.state, self.selected) {
            (SearchState::Results(hits), Some(key)) => hits.iter().find(|h| h.list_key() == key),
            _ => None,
        }
    }

    /// The bottom bar is shown only while a card is selected (TT-27-209).
    pub fn bottom_bar_shown(&self) -> bool {
        self.selected().is_some()
    }

    /// The private button appears only for trackers with private support (TT-27-210).
    pub fn private_button_shown(&self) -> bool {
        self.bottom_bar_shown() && self.supports_private
    }

    /// Marks a bind as being written (`true`) or finished (`false`). The GUI calls it around the bind.
    pub fn set_in_flight(&mut self, in_flight: bool) {
        self.in_flight = in_flight;
    }

    /// Both Track buttons are enabled: a card is selected and no bind is in flight (T-28a-057).
    pub fn track_buttons_enabled(&self) -> bool {
        self.bottom_bar_shown() && !self.in_flight
    }

    /// "Track" (`private` false) or the private button (`true`). Returns the result to register. Private is set
    /// only when the tracker supports it. `None` while a bind is in flight.
    pub fn confirm(&self, private: bool) -> Option<R> {
        if self.in_flight {
            return None;
        }
        let mut hit = self.selected()?.clone();
        hit.set_private(private && self.supports_private);
        Some(hit)
    }
}

/// The keys the tracker search answers (T-28a-052). The field and the result list have their own Enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKey {
    /// Enter while the search field has focus: runs the search.
    EnterInField,
    /// Enter while a result row has focus: tracks the selected row.
    EnterOnRow,
    Up,
    Down,
    Escape,
}

/// What a key asks the screen to do. The GUI runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKeyEffect {
    /// Run the search with the field text.
    Search,
    /// Track the selected row.
    Track,
    /// Close the search.
    Close,
    /// The selection moved.
    Moved,
    /// Nothing to do.
    Nothing,
}

impl<R: SearchHit> SearchScreen<R> {
    /// Keyboard navigation (T-28a-052). Up and Down move the selection through the results, starting at the first
    /// or the last row when none is selected. Enter on a row tracks it. Escape closes.
    pub fn key(&mut self, key: SearchKey) -> SearchKeyEffect {
        match key {
            SearchKey::EnterInField => SearchKeyEffect::Search,
            SearchKey::EnterOnRow if self.selected().is_some() => SearchKeyEffect::Track,
            SearchKey::EnterOnRow => SearchKeyEffect::Nothing,
            SearchKey::Escape => SearchKeyEffect::Close,
            SearchKey::Up => self.step(-1),
            SearchKey::Down => self.step(1),
        }
    }

    fn step(&mut self, delta: isize) -> SearchKeyEffect {
        let SearchState::Results(hits) = &self.state else {
            return SearchKeyEffect::Nothing;
        };
        let keys: Vec<(TrackerId, i64)> = hits.iter().map(|h| h.list_key()).collect();
        if keys.is_empty() {
            return SearchKeyEffect::Nothing;
        }
        let current = self.selected.and_then(|k| keys.iter().position(|x| *x == k));
        let next = match current {
            None if delta > 0 => 0,
            None => keys.len() - 1,
            Some(i) => i.saturating_add_signed(delta).min(keys.len() - 1),
        };
        self.selected = Some(keys[next]);
        SearchKeyEffect::Moved
    }
}

/// Items of the long-press menu of a card, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardMenuItem {
    CopyToClipboard,
    OpenInBrowser,
    /// Only for rows of the `my:` search (T-29a-049).
    Refresh,
}

/// What a card menu item does. The menu closes after every item (D15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardEffect {
    /// Copies the title and shows the copy toast.
    Copy { text: String, toast: MsgKey },
    /// Opens the tracker page in the browser.
    Open(String),
    /// Looks the row up again; carries its list key.
    Refresh((TrackerId, i64)),
}

/// Runs a menu item. `None` when the item is disabled: open with no URL (D9).
pub fn card_action(item: CardMenuItem, hit: &impl SearchHit) -> Option<CardEffect> {
    match item {
        CardMenuItem::CopyToClipboard => Some(CardEffect::Copy {
            text: hit.title().to_owned(),
            toast: crate::sheet::copy_toast(hit.title()),
        }),
        CardMenuItem::OpenInBrowser => {
            let url = hit.open_url();
            (!url.is_empty()).then(|| CardEffect::Open(url.to_owned()))
        }
        CardMenuItem::Refresh => hit
            .card()
            .refreshable
            .then(|| CardEffect::Refresh(hit.list_key())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::search::AnimeTrackSearch;

    fn hit(tracker: TrackerId, remote: i64, url: &str) -> AnimeTrackSearch {
        AnimeTrackSearch::new(tracker, remote, format!("Title {remote}"), url)
    }

    fn results() -> Vec<AnimeTrackSearch> {
        vec![
            hit(TrackerId::ANILIST, 1, "https://a.test/1"),
            hit(TrackerId::ANILIST, 2, "https://a.test/2"),
            hit(TrackerId::KITSU, 1, "https://k.test/1"),
        ]
    }

    #[test]
    fn t_28a_052_up_and_down_move_the_selection_and_enter_tracks_it() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, true);
        screen.finish(Ok(results()));
        assert_eq!(screen.key(SearchKey::Down), SearchKeyEffect::Moved);
        assert_eq!(screen.selected().map(|h| h.list_key()), Some((TrackerId::ANILIST, 1)));
        assert_eq!(screen.key(SearchKey::Down), SearchKeyEffect::Moved);
        assert_eq!(screen.selected().map(|h| h.list_key()), Some((TrackerId::ANILIST, 2)));
        screen.key(SearchKey::Down);
        screen.key(SearchKey::Down);
        assert_eq!(screen.selected().map(|h| h.list_key()), Some((TrackerId::KITSU, 1)), "the last row stays");
        screen.key(SearchKey::Up);
        assert_eq!(screen.selected().map(|h| h.list_key()), Some((TrackerId::ANILIST, 2)));
        assert_eq!(screen.key(SearchKey::EnterOnRow), SearchKeyEffect::Track);
        assert_eq!(screen.key(SearchKey::Escape), SearchKeyEffect::Close);
    }

    #[test]
    fn t_28a_052_enter_in_the_field_searches_and_up_selects_the_last_row() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, true);
        assert_eq!(screen.key(SearchKey::Up), SearchKeyEffect::Nothing, "no results yet");
        assert_eq!(screen.key(SearchKey::EnterInField), SearchKeyEffect::Search);
        screen.finish(Ok(results()));
        assert_eq!(screen.key(SearchKey::EnterOnRow), SearchKeyEffect::Nothing, "no row selected");
        assert_eq!(screen.key(SearchKey::Up), SearchKeyEffect::Moved);
        assert_eq!(screen.selected().map(|h| h.list_key()), Some((TrackerId::KITSU, 1)));
    }

    #[test]
    fn tr_27_d11_a_blank_query_is_not_sent_and_changes_nothing() {
        let (mut screen, first) = SearchScreen::<AnimeTrackSearch>::open("  ", None, true);
        assert_eq!(first, None);
        assert_eq!(screen.state(), &SearchState::Idle);
        screen.finish(Ok(results()));
        assert_eq!(screen.begin(""), None, "defect D11");
        assert!(
            matches!(screen.state(), SearchState::Results(_)),
            "results stay"
        );
    }

    #[test]
    fn tr_27_056_open_with_a_title_runs_one_search() {
        let (screen, first) = SearchScreen::<AnimeTrackSearch>::open("Frieren", None, false);
        assert_eq!(first.as_deref(), Some("Frieren"));
        assert_eq!(screen.query, "Frieren");
        assert_eq!(screen.state(), &SearchState::Loading);
        assert!(screen.selected().is_none());
    }

    #[test]
    fn tr_27b_012_a_slow_first_response_cannot_overwrite_the_second_query() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("a", None, true);
        let first = screen.generation();
        screen.begin("b").unwrap();
        let second = screen.generation();
        assert!(screen.finish_for(second, Ok(vec![hit(TrackerId::ANILIST, 2, "")])));
        assert!(
            !screen.finish_for(first, Ok(vec![hit(TrackerId::ANILIST, 1, "")])),
            "stale"
        );
        match screen.state() {
            SearchState::Results(hits) => {
                assert_eq!(hits[0].remote_id, 2, "only the latest query writes")
            }
            other => panic!("expected results, got {other:?}"),
        }
    }

    #[test]
    fn tr_27b_012_retry_reruns_the_field_and_a_blank_field_does_nothing() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, true);
        screen.finish(Err(TrackError::Message(String::new())));
        assert_eq!(screen.state(), &SearchState::Error("Unknown error".into()));
        assert_eq!(screen.retry().as_deref(), Some("x"));
        assert_eq!(screen.state(), &SearchState::Loading);
        screen.query = "  ".into();
        assert_eq!(screen.retry(), None);
    }

    #[test]
    fn tr_27_076_a_second_begin_clears_the_selection_and_shows_loading() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, true);
        screen.finish(Ok(results()));
        screen.select((TrackerId::ANILIST, 2));
        assert!(screen.selected().is_some());
        assert_eq!(screen.begin("y").as_deref(), Some("y"));
        assert_eq!(screen.state(), &SearchState::Loading);
        assert!(screen.selected().is_none());
        screen.finish(Ok(results()));
        assert!(
            screen.selected().is_none(),
            "the old selection did not survive"
        );
    }

    #[test]
    fn tr_27_208_states_loading_empty_error_and_results() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, false);
        screen.start_loading();
        assert_eq!(screen.state(), &SearchState::Loading);
        screen.finish(Ok(vec![]));
        assert_eq!(screen.state(), &SearchState::Empty);
        screen.start_loading();
        screen.finish(Err(TrackError::Message("timeout".into())));
        assert_eq!(screen.state(), &SearchState::Error("timeout".into()));
        screen.start_loading();
        screen.finish(Err(TrackError::Message(String::new())));
        assert_eq!(screen.state(), &SearchState::Error("Unknown error".into()));
        screen.start_loading();
        screen.finish(Ok(results()));
        assert!(matches!(screen.state(), SearchState::Results(r) if r.len() == 3));
    }

    #[test]
    fn tr_27_209_one_selection_then_a_second_click_changes_it() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, true);
        screen.finish(Ok(results()));
        assert!(!screen.bottom_bar_shown());
        screen.select((TrackerId::ANILIST, 1));
        assert!(screen.bottom_bar_shown());
        screen.select((TrackerId::ANILIST, 1));
        assert_eq!(screen.selected().map(|h| h.remote_id), Some(1), "no toggle");
        screen.select((TrackerId::KITSU, 1));
        assert_eq!(
            screen.selected().map(|h| h.tracker_id),
            Some(TrackerId::KITSU)
        );
        screen.select((TrackerId::KITSU, 99));
        assert_eq!(
            screen.selected().map(|h| h.tracker_id),
            Some(TrackerId::KITSU),
            "unknown key ignored"
        );
    }

    #[test]
    fn tr_27_076_bound_remote_id_auto_selects_and_a_new_query_clears_it() {
        let (mut screen, _) =
            SearchScreen::<AnimeTrackSearch>::open("x", Some((TrackerId::ANILIST, 2)), true);
        screen.finish(Ok(results()));
        assert_eq!(screen.selected().map(|h| h.remote_id), Some(2));
        screen.start_loading();
        assert!(screen.selected().is_none());
        assert_eq!(screen.state(), &SearchState::Loading);
    }

    #[test]
    fn tr_27_076_no_bound_id_means_no_selection() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, true);
        screen.finish(Ok(results()));
        assert!(screen.selected().is_none());
        let (mut other, _) =
            SearchScreen::<AnimeTrackSearch>::open("x", Some((TrackerId::ANILIST, 5)), true);
        other.finish(Ok(results()));
        assert!(
            other.selected().is_none(),
            "an id absent from the results never matches"
        );
        let (mut kitsu, _) =
            SearchScreen::<AnimeTrackSearch>::open("x", Some((TrackerId::KITSU, 1)), true);
        kitsu.finish(Ok(results()));
        assert_eq!(
            kitsu.selected().map(|h| h.list_key()),
            Some((TrackerId::KITSU, 1)),
            "the same remote id of another tracker is not chosen"
        );
        let (mut empty_url, _) =
            SearchScreen::<AnimeTrackSearch>::open("x", Some((TrackerId::ANILIST, 5)), true);
        empty_url.finish(Ok(vec![hit(TrackerId::ANILIST, 5, "")]));
        assert!(
            empty_url.selected().is_some(),
            "selection does not depend on the URL"
        );
    }

    #[test]
    fn tr_27_058_track_is_public_and_private_only_when_supported() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, true);
        screen.finish(Ok(results()));
        screen.select((TrackerId::ANILIST, 1));
        assert!(screen.private_button_shown());
        let public = screen.confirm(false).unwrap();
        assert!(!public.private);
        let private = screen.confirm(true).unwrap();
        assert!(private.private);

        let (mut plain, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, false);
        plain.finish(Ok(results()));
        plain.select((TrackerId::ANILIST, 1));
        assert!(plain.bottom_bar_shown());
        assert!(!plain.private_button_shown(), "TT-27-210");
        assert!(
            !plain.confirm(true).unwrap().private,
            "private needs support"
        );
    }

    #[test]
    fn tr_27_d10_results_with_the_same_remote_id_but_other_trackers_are_distinct() {
        let list = results();
        assert_ne!(list[0].list_key(), list[2].list_key());
        assert_eq!(list[0].list_key(), (TrackerId::ANILIST, 1));
    }

    #[test]
    fn tr_27_057_card_lines_follow_the_source_rules() {
        let mut h = hit(TrackerId::ANILIST, 7, "https://a.test/7");
        h.authors = vec!["Ann".into(), "Bo".into()];
        h.artists = vec!["Ann".into(), "Cy".into()];
        h.publishing_type = "OVA".into();
        h.publishing_status = "ONGOING".into();
        h.start_date = "2026-01-02".into();
        h.score = Some(8.0);
        h.summary = "  A story.  ".into();
        let card = h.card();
        assert_eq!(card.byline.as_deref(), Some("Ann, Bo, Cy"));
        let lines: Vec<(&str, &str)> = card
            .details
            .iter()
            .map(|d| (d.label, d.value.as_str()))
            .collect();
        assert_eq!(
            lines,
            [
                ("Type", "Ova"),
                ("Started", "2026-01-02"),
                ("Status", "Ongoing"),
                ("Score", "8"),
            ]
        );
        assert_eq!(card.summary.as_deref(), Some("A story."));
        assert!(card.can_open);
    }

    #[test]
    fn tr_27_057_empty_detail_lines_and_score_are_hidden() {
        let h = hit(TrackerId::ANILIST, 8, "https://a.test/8");
        let card = h.card();
        assert!(card.details.is_empty());
        assert_eq!(card.byline, None);
        assert_eq!(card.summary, None);
        assert_eq!(score_text(7.5), "7");
    }

    #[test]
    fn tr_27_d09_a_result_without_a_url_has_no_open_action() {
        let h = hit(TrackerId::ANILIST, 9, "");
        assert!(!h.card().can_open);
        assert_eq!(card_action(CardMenuItem::OpenInBrowser, &h), None);
        assert!(card_action(CardMenuItem::CopyToClipboard, &h).is_some());
    }

    #[test]
    fn tr_27_057_d15_menu_actions_copy_the_title_and_open_the_url() {
        let h = hit(TrackerId::ANILIST, 3, "https://a.test/3");
        match card_action(CardMenuItem::CopyToClipboard, &h) {
            Some(CardEffect::Copy { text, toast }) => {
                assert_eq!(text, "Title 3");
                assert!(crate::messages::english(&toast).starts_with("Copied to clipboard:"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            card_action(CardMenuItem::OpenInBrowser, &h),
            Some(CardEffect::Open("https://a.test/3".into()))
        );
    }

    #[test]
    fn t_32a_071_open_in_browser_uses_source_url_for_simkl_and_tracking_url_otherwise() {
        let mut simkl = hit(TrackerId::SIMKL, 5, "/anime/5");
        simkl.source_url = "https://simkl.com/anime/5/slug".into();
        assert!(simkl.card().can_open);
        assert_eq!(
            card_action(CardMenuItem::OpenInBrowser, &simkl),
            Some(CardEffect::Open("https://simkl.com/anime/5/slug".into()))
        );
        let mu = MangaTrackSearch::new(TrackerId::MANGAUPDATES, 5, "M", "https://mu.test/5");
        assert_eq!(
            card_action(CardMenuItem::OpenInBrowser, &mu),
            Some(CardEffect::Open("https://mu.test/5".into()))
        );
    }

    #[test]
    fn t_29a_049_the_card_carries_cover_and_count_before_the_score() {
        use crate::model::search::MangaTrackSearch;
        let mut m = MangaTrackSearch::new(TrackerId::MYANIMELIST, 1, "M", "https://m.test/1");
        m.cover_url = "https://img.test/c.jpg".into();
        m.total_chapters = 120;
        m.score = Some(8.4);
        let card = m.card();
        assert_eq!(card.cover_url.as_deref(), Some("https://img.test/c.jpg"));
        let lines: Vec<(&str, &str)> = card
            .details
            .iter()
            .map(|d| (d.label, d.value.as_str()))
            .collect();
        assert_eq!(lines, [("Chapters", "120"), ("Score", "8")]);
        assert_eq!(hit(TrackerId::MYANIMELIST, 2, "").card().cover_url, None);

        let mut a = hit(TrackerId::MYANIMELIST, 3, "");
        a.total_episodes = 12;
        assert_eq!(a.card().details[0].label, "Episodes");
        a.total_episodes = 0;
        assert!(a.card().details.is_empty(), "0 means unknown");
    }

    #[test]
    fn t_29a_049_my_list_rows_show_no_status_and_offer_refresh() {
        let mut h = hit(TrackerId::MYANIMELIST, 4, "https://m.test/4");
        h.publishing_status = "finished".into();
        assert!(h.card().details.iter().any(|d| d.label == "Status"));
        assert_eq!(card_action(CardMenuItem::Refresh, &h), None);

        h.from_my_list = true;
        let card = h.card();
        assert!(card.refreshable);
        assert!(!card.details.iter().any(|d| d.label == "Status"));
        assert_eq!(
            card_action(CardMenuItem::Refresh, &h),
            Some(CardEffect::Refresh((TrackerId::MYANIMELIST, 4)))
        );
    }

    #[test]
    fn t_28a_057_track_buttons_are_disabled_while_a_bind_is_in_flight() {
        let (mut screen, _) = SearchScreen::<AnimeTrackSearch>::open("x", None, true);
        screen.finish(Ok(results()));
        screen.select((TrackerId::ANILIST, 1));
        assert!(screen.track_buttons_enabled());
        assert!(screen.confirm(false).is_some());
        screen.set_in_flight(true);
        assert!(!screen.track_buttons_enabled());
        assert!(
            screen.confirm(true).is_none(),
            "no second bind while one is written"
        );
        screen.set_in_flight(false);
        assert!(screen.confirm(true).is_some());
    }

    #[test]
    fn t_28a_030_status_underscores_read_as_spaces_and_the_score_is_whole() {
        let mut hit = hit(TrackerId::ANILIST, 5, "https://a.test/5");
        hit.publishing_status = "ON_HOLD".into();
        hit.score = Some(85.9);
        let card = hit.card();
        let status = card
            .details
            .iter()
            .find(|d| d.label == "Status")
            .map(|d| d.value.clone());
        assert_eq!(status.as_deref(), Some("On hold"));
        let score = card
            .details
            .iter()
            .find(|d| d.label == "Score")
            .map(|d| d.value.clone());
        assert_eq!(score.as_deref(), Some("85"));
    }
}
