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

/// A title of the provider's catalogue. It is written to and read from the cache file of the bulk lists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub remote_id: i64,
    pub title: String,
    pub alt_titles: Vec<String>,
    /// 0 to 10; `None` while the site has no score for it.
    pub score: Option<f64>,
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
