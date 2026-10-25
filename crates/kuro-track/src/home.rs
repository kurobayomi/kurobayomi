//! The Home model of the tracking sheet (`HomeModel`, behaviour 3.2 and 3.3, `p2-rust-design.md` section 5.1). It
//! re-assembles the rows on every store change and every login change, emits a list only when the rows differ, and
//! keeps the last good list when the store fails. Refresh on open reports one toast per failed tracker.

use kuro_platform::kind::Kind;
use std::sync::Arc;

use tokio::sync::watch;

use crate::ctx::TrackCtx;
use crate::error::{TrackError, redact};
use crate::item::{TrackItem, assemble_items};
use crate::manager::TrackerManager;
use crate::messages::MsgKey;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::picker::picker_day;
use crate::record::TrackRecord;
use crate::repo::RowsWatch;
use crate::sync;
use crate::tracker::Tracker;

/// The state of the Home list for one entry.
#[derive(Clone)]
pub struct HomeModel {
    pub entry_id: i64,
    pub kind: Kind,
    /// Qualified source id, `None` for a stub source.
    pub source: Option<String>,
    pub items: Vec<TrackItem>,
    /// `app_date_format`, read once when Home opens (behaviour 3.1). A later change shows on the next open.
    date_format: String,
}

impl HomeModel {
    /// `date_format` is the `app_date_format` preference. It is read here, once.
    pub fn new(
        entry_id: i64,
        kind: Kind,
        source: Option<String>,
        date_format: &kuro_prefs::pref::Pref<String>,
    ) -> Self {
        Self {
            entry_id,
            kind,
            source,
            items: Vec::new(),
            date_format: date_format.get(),
        }
    }

    /// The text of a start or finish date row, in the format read at open. `None` for an unset date.
    pub fn date_text(&self, stored_ms: i64, zone: &impl chrono::TimeZone) -> Option<String> {
        row_date_text(stored_ms, &self.date_format, zone)
    }

    /// Re-assembles the list. Returns `true` and replaces the list only when `(tracker id, record)` pairs differ.
    pub fn apply(&mut self, logged_in: &[Arc<dyn Tracker>], rows: &[TrackRecord]) -> bool {
        let next = assemble_items(self.kind, logged_in, rows, self.source.as_deref());
        let same = next.len() == self.items.len()
            && next.iter().zip(&self.items).all(|(a, b)| a.same_as(b));
        if same {
            return false;
        }
        self.items = next;
        true
    }
}

/// The rows of one entry, of either kind, as a live source.
pub enum RowSource {
    Anime(RowsWatch<AnimeTrack>),
    Manga(RowsWatch<MangaTrack>),
}

impl RowSource {
    /// Waits for the next published value. `false` once the source is gone.
    async fn changed(&mut self) -> bool {
        match self {
            RowSource::Anime(rx) => rx.changed().await.is_ok(),
            RowSource::Manga(rx) => rx.changed().await.is_ok(),
        }
    }

    /// The current rows as track records, or the store error text.
    fn snapshot(&mut self) -> Result<Vec<TrackRecord>, String> {
        match self {
            RowSource::Anime(rx) => {
                let value: Result<Vec<AnimeTrack>, String> = rx.borrow_and_update().clone();
                value.map(|rows| rows.into_iter().map(TrackRecord::from).collect())
            }
            RowSource::Manga(rx) => {
                let value: Result<Vec<MangaTrack>, String> = rx.borrow_and_update().clone();
                value.map(|rows| rows.into_iter().map(TrackRecord::from).collect())
            }
        }
    }
}

/// Live Home list: rows and logins in, distinct lists out.
pub struct HomeFeed {
    model: HomeModel,
    trackers: Arc<TrackerManager>,
    rows: RowSource,
    logins: watch::Receiver<Vec<TrackerId>>,
    /// The last rows read without a store error. A store error keeps these.
    records: Vec<TrackRecord>,
    rows_open: bool,
    logins_open: bool,
}

enum Wake {
    Rows(bool),
    Logins(bool),
}

impl HomeFeed {
    /// Starts from the current rows and logins. The initial list is in [`HomeFeed::items`]; [`HomeFeed::next`]
    /// reports only later changes.
    pub fn new(
        mut model: HomeModel,
        trackers: Arc<TrackerManager>,
        mut rows: RowSource,
        logins: watch::Receiver<Vec<TrackerId>>,
    ) -> Self {
        let records = match rows.snapshot() {
            Ok(records) => records,
            Err(e) => {
                tracing::error!(error = %redact(&e.to_string()), "track rows could not be read");
                Vec::new()
            }
        };
        model.apply(&trackers.logged_in(), &records);
        Self {
            model,
            trackers,
            rows,
            logins,
            records,
            rows_open: true,
            logins_open: true,
        }
    }

    /// The list as it stands.
    pub fn items(&self) -> &[TrackItem] {
        &self.model.items
    }

    /// Waits for the next change of the list and returns it. A store error or a change that leaves the list equal
    /// is not returned. `None` once both sources are gone.
    pub async fn next(&mut self) -> Option<Vec<TrackItem>> {
        loop {
            if !self.rows_open && !self.logins_open {
                return None;
            }
            let wake = {
                let HomeFeed {
                    rows,
                    logins,
                    rows_open,
                    logins_open,
                    ..
                } = self;
                tokio::select! {
                    ok = rows.changed(), if *rows_open => Wake::Rows(ok),
                    ok = logins.changed(), if *logins_open => Wake::Logins(ok.is_ok()),
                }
            };
            match wake {
                Wake::Rows(false) => self.rows_open = false,
                Wake::Logins(false) => self.logins_open = false,
                Wake::Rows(true) => match self.rows.snapshot() {
                    Ok(records) => {
                        self.records = records;
                        if let Some(list) = self.reassemble() {
                            return Some(list);
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %redact(&e.to_string()), "track store read failed, last list kept")
                    }
                },
                Wake::Logins(true) => {
                    if let Some(list) = self.reassemble() {
                        return Some(list);
                    }
                }
            }
        }
    }

    fn reassemble(&mut self) -> Option<Vec<TrackItem>> {
        let logged_in = self.trackers.logged_in();
        self.model
            .apply(&logged_in, &self.records)
            .then(|| self.model.items.clone())
    }
}

/// Behaviour 3.3: refreshes every logged-in binding of the entry concurrently. Returns one toast per failed tracker,
/// `"<name> error: <message>"`. A failure never stops the other trackers.
pub async fn refresh_toasts(
    ctx: &TrackCtx,
    kind: Kind,
    entry_id: i64,
) -> Result<Vec<MsgKey>, TrackError> {
    let failures = match kind {
        Kind::Anime => sync::refresh_anime(ctx, entry_id).await?.failures,
        Kind::Manga => sync::refresh_manga(ctx, entry_id).await?.failures,
    };
    Ok(failures
        .into_iter()
        .map(|(id, e)| {
            let name = ctx
                .trackers
                .get(id)
                .map(|t| t.name().to_owned())
                .unwrap_or_else(|| id.0.to_string());
            e.toast(&name)
        })
        .collect())
}

/// The text of a Home date row (T-27b-031). `app_date_format` holds a Java date pattern. Empty gives the short date of
/// the UI locale, as `FormatStyle.SHORT` does. A pattern the settings list offers is applied with the UI locale's
/// month names. Anything else gives the short date, like the preferences crate. Unset (0) gives `None`. The day is the
/// local day in `zone`, as in the picker.
pub fn row_date_text(
    stored_ms: i64,
    app_date_format: &str,
    zone: &impl chrono::TimeZone,
) -> Option<String> {
    let day = picker_day(stored_ms, zone)?;
    let format = kuro_i18n::date::DateFormat::from_pref(app_date_format);
    Some(format.format(day, kuro_i18n::current_locale().tag))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::fakes::{Harness, anime_binding};
    use crate::model::TrackerId;

    fn model(entry: i64) -> HomeModel {
        let store = kuro_prefs::store::PrefStore::in_memory();
        let date = kuro_prefs::pref::Pref::new(store, "app_date_format", String::new());
        HomeModel::new(entry, Kind::Anime, Some("src.one".into()), &date)
    }

    #[tokio::test]
    async fn tr_27b_004_apply_replaces_the_list_only_when_the_pairs_differ() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        let logged = h.ctx.trackers.logged_in();
        let mut m = model(1);
        assert!(m.apply(&logged, &[]), "first list is new");
        assert!(!m.apply(&logged, &[]), "same list is not replaced");
        let row = TrackRecord::from(anime_binding(1, TrackerId::ANILIST, 2.0));
        assert!(
            m.apply(&logged, std::slice::from_ref(&row)),
            "a changed record is replaced"
        );
        assert!(!m.apply(&logged, std::slice::from_ref(&row)));
        assert!(m.items.iter().any(|i| i.record.as_ref() == Some(&row)));
    }

    #[tokio::test]
    async fn tr_27b_004_feed_emits_row_and_login_changes_and_skips_equal_lists() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        let (rows_tx, rows_rx) = watch::channel(Ok(Vec::<AnimeTrack>::new()));
        let (logins_tx, logins_rx) = watch::channel(Vec::<TrackerId>::new());
        let mut feed = HomeFeed::new(
            model(1),
            h.ctx.trackers.clone(),
            RowSource::Anime(rows_rx),
            logins_rx,
        );
        assert!(feed.items().iter().all(|i| i.record.is_none()));

        rows_tx
            .send(Ok(vec![anime_binding(1, TrackerId::ANILIST, 2.0)]))
            .unwrap();
        let list = feed.next().await.expect("a row arrived");
        assert_eq!(list.iter().filter(|i| i.record.is_some()).count(), 1);

        rows_tx
            .send(Ok(vec![anime_binding(1, TrackerId::ANILIST, 2.0)]))
            .unwrap();
        let again = tokio::time::timeout(Duration::from_millis(100), feed.next()).await;
        assert!(again.is_err(), "an equal list is not emitted");

        rows_tx.send(Err("disk busy".into())).unwrap();
        let failed = tokio::time::timeout(Duration::from_millis(100), feed.next()).await;
        assert!(failed.is_err(), "a store error emits nothing");
        assert_eq!(
            feed.items().iter().filter(|i| i.record.is_some()).count(),
            1,
            "last good list stays"
        );

        h.anime_tracker(TrackerId::KITSU)
            .save_credentials("u", "p")
            .unwrap();
        logins_tx.send(vec![TrackerId::KITSU]).unwrap();
        let after_login = feed.next().await.expect("a login changes the list");
        assert!(
            after_login
                .iter()
                .any(|i| i.tracker.id() == TrackerId::KITSU)
        );
    }

    #[test]
    fn tr_27b_005_a_null_message_keeps_the_colon() {
        let toast = TrackError::Message(String::new()).toast("Kitsu");
        assert_eq!(crate::messages::english(&toast), "Kitsu error: ");
    }

    #[tokio::test]
    async fn tr_27b_005_each_failed_tracker_gives_one_toast_and_the_others_still_run() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::ANILIST, 1.0))
            .await
            .unwrap();
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::KITSU, 1.0))
            .await
            .unwrap();
        *h.anime_tracker(TrackerId::KITSU).fail.lock().unwrap() = Some("kitsu is down".into());
        let toasts = refresh_toasts(&h.ctx, Kind::Anime, 1).await.unwrap();
        assert_eq!(
            toasts,
            vec![MsgKey::TrackerError {
                tracker: "Kitsu".into(),
                detail: "kitsu is down".into()
            }]
        );
    }

    #[test]
    fn tr_27b_031_empty_format_is_the_locale_short_date_and_a_java_pattern_is_applied() {
        use chrono::{FixedOffset, NaiveDate, TimeZone};
        let zone = FixedOffset::east_opt(0).unwrap();
        let ms = zone
            .with_ymd_and_hms(2026, 10, 8, 12, 0, 0)
            .unwrap()
            .timestamp_millis();
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        let short =
            kuro_i18n::date::DateFormat::LocaleShort.format(day, kuro_i18n::current_locale().tag);
        assert_eq!(
            row_date_text(ms, "", &zone).as_deref(),
            Some(short.as_str())
        );
        assert_eq!(
            row_date_text(ms, "  ", &zone).as_deref(),
            Some(short.as_str())
        );
        assert_eq!(
            row_date_text(ms, "dd MMM yyyy", &zone).as_deref(),
            Some("08 Oct 2026"),
            "a Java pattern is not printed literally"
        );
        assert_eq!(
            row_date_text(ms, "yyyy-MM-dd", &zone).as_deref(),
            Some("2026-10-08")
        );
        assert_eq!(
            row_date_text(ms, "%Q bad", &zone).as_deref(),
            Some(short.as_str()),
            "an unknown pattern falls back"
        );
        assert_eq!(row_date_text(0, "yyyy", &zone), None, "unset shows nothing");
    }

    #[test]
    fn tr_27b_031_the_model_reads_the_preference_once_at_open() {
        let store = kuro_prefs::store::PrefStore::in_memory();
        let pref = kuro_prefs::pref::Pref::new(store, "app_date_format", String::new());
        pref.set("yyyy-MM-dd".into()).unwrap();
        let model = HomeModel::new(1, Kind::Anime, None, &pref);
        pref.set("dd/MM/yy".into()).unwrap();
        let zone = chrono::FixedOffset::east_opt(0).unwrap();
        let ms = chrono::TimeZone::with_ymd_and_hms(&zone, 2026, 10, 8, 12, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(model.date_text(ms, &zone).as_deref(), Some("2026-10-08"));
    }
}
