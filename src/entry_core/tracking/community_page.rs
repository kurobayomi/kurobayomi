//! The community score on the tracking page (`kuro_data::community_score`, filled by `crate::community`): one section
//! under the trackers with the entry's score and the title it was matched as, and Change. Change opens a search of the
//! score site for the entry's title: tapping a result makes it the entry's match, and "No score for this title" stops
//! the score for it. Either is the user's own choice, which the refresh job never replaces.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Align, Box as GBox, Button, ListBox, Orientation as O};
use kuro_data::community_score::{CommunityScore, MatchedBy};
use kuro_i18n::{Arg, MsgKey, tr, trf};
use kuro_platform::kind::Kind;
use kuro_scores::provider::Hit;

use super::page::{self, Entry};
use crate::screens::{Built, built, unavailable};
use crate::settings_ui::tracker_host;
use crate::ui;

/// The screen id of the search page (`spec/entries.json`).
const SEARCH_SCREEN: &str = "community_score_search";

/// What the section says about the stored row of an entry: the score and the matched title, or why there is none.
fn state_text(stored: Option<&CommunityScore>) -> String {
    let Some(row) = stored else { return tr(MsgKey::ui_score_not_looked_up) };
    if row.remote_id.is_none() {
        return tr(if row.matched_by == MatchedBy::User { MsgKey::ui_no_score_for_this_title } else { MsgKey::ui_score_not_found });
    }
    let score = score_or_none(row.score);
    if row.matched_title.is_empty() {
        return score;
    }
    format!("{score} \u{b7} {}", trf(MsgKey::ui_score_matched_as, &[("title", Arg::Str(&row.matched_title))]))
}

/// A score as text, or that the site has none for the title yet.
fn score_or_none(score: Option<f64>) -> String {
    score.map_or_else(|| tr(MsgKey::ui_score_none_yet), crate::community::score_text)
}

/// Appends the section to the tracking page of `entry`: what is stored for it, and Change.
pub fn section(root: &GBox, entry: &Entry) {
    let change = Button::with_label(&tr(MsgKey::ui_change));
    change.set_valign(Align::Center);
    change.connect_clicked(|_| ui::open(SEARCH_SCREEN));
    let state = state_text(crate::community::stored(entry.id).as_ref());
    let list = ui::boxed_list();
    list.append(&ui::row(&tr(MsgKey::ui_community_score), Some(&state), Some(change.upcast::<gtk::Widget>())));
    root.append(&list);
}

/// The search page: the score site searched for the entry's title, already run. Tapping a result makes it the entry's
/// match and returns to the tracking page; the row above the results stops the score for this title.
pub fn search() -> Option<Built> {
    let Some(entry) = page::entry() else { return unavailable(SEARCH_SCREEN) };
    let column = ui::column();
    let field = gtk::SearchEntry::new();
    field.set_placeholder_text(Some(&tr(MsgKey::ui_search_myanimelist)));
    field.set_text(&entry.title);
    let none = ui::boxed_list();
    let id = entry.id;
    none.append(&page::button_row(&tr(MsgKey::ui_no_score_for_this_title), &["flat"], move || choose(id, None)));
    let results = GBox::new(O::Vertical, 0);
    column.append(&field);
    column.append(&none);
    column.append(&results);

    let generation = Rc::new(Cell::new(0_u64));
    let (typed, shown_results, typed_generation) = (entry.clone(), results.clone(), Rc::clone(&generation));
    field.connect_activate(move |field| run_search(&typed, vec![field.text().to_string()], &shown_results, &typed_generation));
    run_search(&entry, kuro_track::auto_bind::title_queries(&entry.title), &results, &generation);
    built(ui::scrolled(&column), vec![])
}

/// Searches the score site and shows the results: `queries` in order, until one finds something. Only the latest
/// search may show anything.
fn run_search(entry: &Entry, queries: Vec<String>, results: &GBox, generation: &Rc<Cell<u64>>) {
    let queries: Vec<String> = queries.into_iter().filter(|query| !query.trim().is_empty()).collect();
    if queries.is_empty() {
        return;
    }
    let token = generation.get() + 1;
    generation.set(token);
    page::clear(results);
    results.append(&page::searching());
    let (kind, id, latest, shown) = (entry.kind, entry.id, Rc::clone(generation), results.clone());
    let again = (entry.clone(), queries.clone(), Rc::clone(generation));
    crate::background::run_async(
        async move { search_first(kind, queries).await },
        move |result| {
            if latest.get() != token {
                return;
            }
            page::clear(&shown);
            match result {
                Ok(hits) if hits.is_empty() => shown.append(&crate::widgets::status::empty("system-search-symbolic", &tr(MsgKey::no_results_found), None)),
                Ok(hits) => shown.append(&hits_list(id, hits)),
                Err(text) => {
                    let results = shown.clone();
                    shown.append(&crate::widgets::status::error(&text, None, Some(move || run_search(&again.0, again.1.clone(), &results, &again.2))));
                }
            }
        },
    );
}

/// The hits of the first of `queries` that finds any. A failed search stops there: its text is shown with Retry.
async fn search_first(kind: Kind, queries: Vec<String>) -> Result<Vec<Hit>, String> {
    for query in queries {
        let hits = crate::community::search(kind, query).await?;
        if !hits.is_empty() {
            return Ok(hits);
        }
    }
    Ok(Vec::new())
}

/// One row per result: its title, and its score underneath. Tapping a row makes it the entry's match.
fn hits_list(entry_id: i64, hits: Vec<Hit>) -> ListBox {
    let list = ui::boxed_list();
    for hit in &hits {
        // a plain row is not activatable on its own, and the list's activation below is what chooses
        let row = ui::row(&hit.title, Some(&score_or_none(hit.score)), None);
        row.set_activatable(true);
        list.append(&row);
    }
    list.connect_row_activated(move |_, row| {
        if let Some(hit) = usize::try_from(row.index()).ok().and_then(|index| hits.get(index)) {
            choose(entry_id, Some(hit.clone()));
        }
    });
    list
}

/// Stores the user's choice (a result, or `None` for no score) off the GTK thread, then returns to the tracking page,
/// which shows it. The library and the details page follow the score store on their own.
fn choose(entry_id: i64, hit: Option<Hit>) {
    crate::background::run_async(
        async move { tokio::task::spawn_blocking(move || crate::community::set_match(entry_id, hit)).await.unwrap_or_else(|error| Err(error.to_string())) },
        |result| match result {
            Ok(()) => {
                ui::pop();
                tracker_host::refresh();
                page::changed();
            }
            Err(text) => crate::toast::notify(text),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(remote_id: Option<i64>, by: MatchedBy, score: Option<f64>, title: &str) -> CommunityScore {
        CommunityScore { entry_id: 1, provider: "myanimelist".into(), remote_id, matched_title: title.into(), matched_by: by, score, fetched_at: 0 }
    }

    #[test]
    fn the_section_says_what_is_stored_for_the_entry() {
        assert_eq!(state_text(None), "Not looked up yet");
        assert_eq!(state_text(Some(&row(Some(5), MatchedBy::Title, Some(8.24), "Frieren"))), "8.2 \u{b7} Matched on MyAnimeList as Frieren");
        assert_eq!(state_text(Some(&row(Some(5), MatchedBy::Tracker, None, "Frieren"))), "No score yet \u{b7} Matched on MyAnimeList as Frieren");
        assert_eq!(state_text(Some(&row(Some(5), MatchedBy::User, Some(9.0), ""))), "9.0", "no title stored, the score alone");
        assert_eq!(state_text(Some(&row(None, MatchedBy::Title, None, ""))), "Not found on MyAnimeList");
        assert_eq!(state_text(Some(&row(None, MatchedBy::User, None, ""))), "No score for this title", "the user's own choice");
    }

    #[test]
    fn a_search_result_shows_its_score_or_that_it_has_none() {
        assert_eq!(score_or_none(Some(7.96)), "8.0");
        assert_eq!(score_or_none(None), "No score yet");
    }
}
