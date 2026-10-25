//! Auto-track (user, 2026-10-25): a title that joins the library is bound on each switched-on tracker the user is logged
//! in to (AniList and MyAnimeList today), when that tracker's search finds exactly one result with a name equal to the
//! title. A title with no such result, or with several, stays unbound there and the user picks on the tracking page as
//! before. Off by default (`TrackPrefs::auto_track`). Self-hosted trackers bind on their own (`bind_enhanced_*`).

use crate::ctx::{TrackCtx, now_ms};
use crate::bind::{bind_anime, bind_manga};
use crate::error::TrackError;
use crate::model::TrackerId;
use crate::tracker::Tracker;

/// A title as the match compares it: lower case, words of letters and digits only, a trailing Dub / Sub tag dropped,
/// and "2nd Season" read as "Season 2".
pub fn normalize(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    while words.last().is_some_and(|w| *w == "dub" || *w == "sub") {
        words.pop();
    }
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let ordinal = words[i].strip_suffix("st").or_else(|| words[i].strip_suffix("nd")).or_else(|| words[i].strip_suffix("rd")).or_else(|| words[i].strip_suffix("th"));
        match ordinal {
            Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && words.get(i + 1) == Some(&"season") => {
                out.push("season".into());
                out.push(n.into());
                i += 2;
            }
            _ => {
                out.push(words[i].into());
                i += 1;
            }
        }
    }
    out.join(" ")
}

/// The index of the one result whose display name or other names equal `title` after [`normalize`]. `None` when no
/// result or more than one does.
pub fn pick<'a>(title: &str, hits: impl IntoIterator<Item = (&'a str, &'a [String])>) -> Option<usize> {
    let wanted = normalize(title);
    if wanted.is_empty() {
        return None;
    }
    let mut found = None;
    for (i, (name, others)) in hits.into_iter().enumerate() {
        if std::iter::once(name).chain(others.iter().map(String::as_str)).any(|n| normalize(n) == wanted) {
            if found.is_some() {
                return None;
            }
            found = Some(i);
        }
    }
    found
}

/// The trackers auto-track binds on: switched on, logged in, and not self-hosted.
fn targets(ctx: &TrackCtx) -> Vec<std::sync::Arc<dyn Tracker>> {
    if !ctx.prefs.auto_track.get() {
        return Vec::new();
    }
    ctx.trackers
        .logged_in()
        .into_iter()
        .filter(|t| ctx.trackers.is_enabled(t.id()) && t.as_enhanced_anime().is_none() && t.as_enhanced_manga().is_none())
        .collect()
}

/// Binds a library anime on every [`targets`] tracker it is not bound on yet and [`pick`] finds its one match. Returns
/// the trackers it bound on, and the failures; one tracker failing does not stop the others.
pub async fn auto_bind_anime(ctx: &TrackCtx, entry_id: i64, title: &str) -> (Vec<TrackerId>, Vec<(TrackerId, TrackError)>) {
    let (mut bound, mut failed) = (Vec::new(), Vec::new());
    for tracker in targets(ctx) {
        let Some(api) = tracker.as_anime() else { continue };
        let id = tracker.id();
        let result = async {
            if ctx.anime_binding(entry_id, id).await?.is_some() {
                return Ok(false);
            }
            let hits = api.search(title).await?;
            let Some(i) = pick(title, hits.iter().map(|h| (h.title.as_str(), h.alt_titles.as_slice()))) else { return Ok(false) };
            let mut hit = hits[i].clone();
            hit.anime_id = entry_id;
            bind_anime(ctx, entry_id, api, hit.to_track(), now_ms()).await?;
            Ok::<bool, TrackError>(true)
        };
        match result.await {
            Ok(true) => bound.push(id),
            Ok(false) => {}
            Err(e) => failed.push((id, e)),
        }
    }
    (bound, failed)
}

/// Manga form of [`auto_bind_anime`].
pub async fn auto_bind_manga(ctx: &TrackCtx, entry_id: i64, title: &str) -> (Vec<TrackerId>, Vec<(TrackerId, TrackError)>) {
    let (mut bound, mut failed) = (Vec::new(), Vec::new());
    for tracker in targets(ctx) {
        let Some(api) = tracker.as_manga() else { continue };
        let id = tracker.id();
        let result = async {
            if ctx.manga_binding(entry_id, id).await?.is_some() {
                return Ok(false);
            }
            let hits = api.search(title).await?;
            let Some(i) = pick(title, hits.iter().map(|h| (h.title.as_str(), h.alt_titles.as_slice()))) else { return Ok(false) };
            let mut hit = hits[i].clone();
            hit.manga_id = entry_id;
            bind_manga(ctx, entry_id, api, hit.to_track(), now_ms()).await?;
            Ok::<bool, TrackError>(true)
        };
        match result.await {
            Ok(true) => bound.push(id),
            Ok(false) => {}
            Err(e) => failed.push((id, e)),
        }
    }
    (bound, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalize_ignores_case_punctuation_dub_and_season_wording() {
        assert_eq!(normalize("Shingeki no Kyojin: The Final Season"), "shingeki no kyojin the final season");
        assert_eq!(normalize("Mob Psycho 100 II (Dub)"), "mob psycho 100 ii");
        assert_eq!(normalize("Spy x Family 2nd Season"), normalize("SPY x FAMILY Season 2"));
        assert_eq!(normalize("Dr. Stone"), "dr stone");
        assert_eq!(normalize("…"), "");
    }

    #[test]
    fn pick_takes_the_one_exact_name_match_from_any_name() {
        let a = names(&["Shingeki no Kyojin"]);
        let b = names(&["Attack on Titan Movie"]);
        let hits = [("Attack on Titan", a.as_slice()), ("Shingeki no Kyojin Movie", b.as_slice())];
        assert_eq!(pick("shingeki no kyojin", hits.iter().map(|(n, o)| (*n, *o))), Some(0));
        assert_eq!(pick("Attack on Titan (Dub)", hits.iter().map(|(n, o)| (*n, *o))), Some(0));
    }

    #[test]
    fn pick_refuses_no_match_two_matches_and_an_empty_title() {
        let none: Vec<String> = Vec::new();
        let hits = [("Berserk", none.as_slice()), ("Berserk", none.as_slice())];
        assert_eq!(pick("Berserk", hits.iter().map(|(n, o)| (*n, *o))), None, "the series and the remake share the name");
        assert_eq!(pick("Berserk: Golden Age", hits[..1].iter().map(|(n, o)| (*n, *o))), None, "a near name is not a match");
        assert_eq!(pick("!!", hits[..1].iter().map(|(n, o)| (*n, *o))), None);
    }
}
