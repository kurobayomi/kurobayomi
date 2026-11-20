//! What a source of community scores answers: one title at a time, by an id it knows or by name, and the titles it can
//! list in bulk.

use std::time::Duration;

use async_trait::async_trait;
use kuro_data::community_score::MatchedBy;
use kuro_platform::kind::Kind;
use kuro_track::model::TrackerId;
use serde::{Deserialize, Serialize};

/// A tracker binding of the entry: its id on that tracker and the title the tracker gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackRef {
    pub tracker: TrackerId,
    pub remote_id: i64,
    pub title: String,
}

/// One entry to look up.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub entry_id: i64,
    pub kind: Kind,
    pub title: String,
    /// The provider's id from the stored match, and how that match was made. A match the user chose is asked by this id
    /// and nothing else.
    pub known: Option<(i64, MatchedBy)>,
    pub tracks: Vec<TrackRef>,
}

/// When a title on air is broadcast each week, in Japan's time (UTC+9, with no summer time): what MyAnimeList lists.
/// It is the weekly slot and nothing more: the site does not say which episode is next, nor when a title is on a break.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AirSlot {
    /// 0 is Monday, 6 is Sunday.
    pub weekday: u8,
    /// Minutes after midnight.
    pub minute: u16,
}

/// The minutes of a week, and Japan's distance from UTC in minutes.
const WEEK_MINUTES: i32 = 7 * 24 * 60;
const JAPAN_AHEAD_MINUTES: i32 = 9 * 60;

impl AirSlot {
    /// The slot in UTC, as minutes after Monday 00:00 UTC (0 to 10,079), for a clock to turn into its own time.
    pub fn utc_week_minute(self) -> u32 {
        let japan = i32::from(self.weekday) * 24 * 60 + i32::from(self.minute);
        (japan - JAPAN_AHEAD_MINUTES).rem_euclid(WEEK_MINUTES) as u32
    }
}

/// A title of the provider's catalogue. It is written to and read from the cache file of the bulk lists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub remote_id: i64,
    pub title: String,
    pub alt_titles: Vec<String>,
    /// 0 to 10; `None` while the site has no score for it.
    pub score: Option<f64>,
    /// Its weekly slot while it is on air, when the site lists one. A cache file written before this field reads as none.
    #[serde(default)]
    pub slot: Option<AirSlot>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_in_japan_is_nine_hours_earlier_in_utc_and_may_fall_on_the_day_before() {
        // Wednesday 22:00 in Japan is Wednesday 13:00 UTC
        assert_eq!(AirSlot { weekday: 2, minute: 22 * 60 }.utc_week_minute(), 2 * 24 * 60 + 13 * 60);
        // Monday 01:30 in Japan is Sunday 16:30 UTC: the week wraps
        assert_eq!(AirSlot { weekday: 0, minute: 90 }.utc_week_minute(), 6 * 24 * 60 + 16 * 60 + 30);
        // Sunday 23:15 in Japan is Sunday 14:15 UTC
        assert_eq!(AirSlot { weekday: 6, minute: 23 * 60 + 15 }.utc_week_minute(), 6 * 24 * 60 + 14 * 60 + 15);
        assert_eq!(AirSlot { weekday: 0, minute: 9 * 60 }.utc_week_minute(), 0, "Monday 09:00 in Japan is the start of the UTC week");
    }

    #[test]
    fn a_title_written_before_the_slot_existed_reads_without_one() {
        let old: Hit = serde_json::from_str(r#"{"remote_id":1,"title":"Naruto","alt_titles":[],"score":8.0}"#).expect("an old cache line");
        assert_eq!(old.slot, None);
        let new = Hit { remote_id: 1, title: "Naruto".into(), alt_titles: Vec::new(), score: Some(8.0), slot: Some(AirSlot { weekday: 3, minute: 600 }) };
        let read: Hit = serde_json::from_str(&serde_json::to_string(&new).expect("written")).expect("read");
        assert_eq!(read, new);
    }
}

/// The answer for one entry.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Found { hit: Hit, by: MatchedBy },
    NotFound,
}

/// Why a provider stopped answering: the caller keeps what it has and tries again later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The site asked to slow down, for this long when it said.
    RateLimited { retry_after: Option<Duration> },
    /// No answer, or one that cannot be read.
    Unreachable,
}

#[async_trait]
pub trait ScoreProvider: Send + Sync {
    /// The name stored with a score (`community_scores.provider`).
    fn id(&self) -> &'static str;
    fn supports(&self, kind: Kind) -> bool;
    /// The score of one entry. It paces its own requests.
    async fn lookup(&self, query: &Query) -> Result<Outcome, Stop>;
    /// The titles a text finds, for the user to pick the right one.
    async fn search(&self, kind: Kind, text: &str) -> Result<Vec<Hit>, Stop>;
    /// The titles the provider can list in bulk, with their scores: what a title is matched against by name, without
    /// a request of its own (`catalogue::Catalogue`). `today` is the year and the month, for a provider that lists by
    /// season. The default lists none.
    async fn catalogue(&self, _kind: Kind, _today: (i32, u32)) -> Result<Vec<Hit>, Stop> {
        Ok(Vec::new())
    }
}
