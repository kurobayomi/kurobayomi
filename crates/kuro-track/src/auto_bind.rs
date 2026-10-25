//! Auto-track (user, 2026-10-25): a title that joins the library is bound on AniList when the search finds exactly one
//! result with a name equal to the title. A title with no such result, or with several, stays unbound and the user picks
//! on the tracking page as before. Off by default (`TrackPrefs::auto_track`).

use crate::ctx::{TrackCtx, now_ms};
use crate::bind::{bind_anime, bind_manga};
use crate::error::TrackError;
use crate::model::TrackerId;

/// The tracker auto-track binds on.
pub const TRACKER: TrackerId = TrackerId::ANILIST;

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

/// Binds a library anime on AniList when the switch is on, AniList is logged in and switched on, the anime is not bound
/// there yet, and [`pick`] finds its one match. `Ok(true)` when it bound.
pub async fn auto_bind_anime(ctx: &TrackCtx, entry_id: i64, title: &str) -> Result<bool, TrackError> {
    if !ctx.prefs.auto_track.get() || !ctx.trackers.is_enabled(TRACKER) {
        return Ok(false);
    }
    let Some(tracker) = ctx.trackers.get(TRACKER).filter(|t| t.is_logged_in()) else { return Ok(false) };
    let Some(api) = tracker.as_anime() else { return Ok(false) };
    if ctx.anime_binding(entry_id, TRACKER).await?.is_some() {
        return Ok(false);
    }
    let hits = api.search(title).await?;
    let Some(i) = pick(title, hits.iter().map(|h| (h.title.as_str(), h.alt_titles.as_slice()))) else { return Ok(false) };
    let mut hit = hits[i].clone();
    hit.anime_id = entry_id;
    bind_anime(ctx, entry_id, api, hit.to_track(), now_ms()).await?;
    Ok(true)
}

/// Manga form of [`auto_bind_anime`].
pub async fn auto_bind_manga(ctx: &TrackCtx, entry_id: i64, title: &str) -> Result<bool, TrackError> {
    if !ctx.prefs.auto_track.get() || !ctx.trackers.is_enabled(TRACKER) {
        return Ok(false);
    }
    let Some(tracker) = ctx.trackers.get(TRACKER).filter(|t| t.is_logged_in()) else { return Ok(false) };
    let Some(api) = tracker.as_manga() else { return Ok(false) };
    if ctx.manga_binding(entry_id, TRACKER).await?.is_some() {
        return Ok(false);
    }
    let hits = api.search(title).await?;
    let Some(i) = pick(title, hits.iter().map(|h| (h.title.as_str(), h.alt_titles.as_slice()))) else { return Ok(false) };
    let mut hit = hits[i].clone();
    hit.manga_id = entry_id;
    bind_manga(ctx, entry_id, api, hit.to_track(), now_ms()).await?;
    Ok(true)
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
