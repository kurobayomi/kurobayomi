//! SQL of `track_retry_queue`. The table is part of the unified database schema
//! (`kuro-db/sql/unified/schema.sql`); its key is `(kind, entry_id, tracker_id)`, so anime and manga with the same
//! numbers stay apart and a binding has one pending progress.

/// One statement. The highest pending progress wins, so a lower value never replaces a higher one.
pub const UPSERT: &str = "INSERT INTO track_retry_queue (kind, entry_id, tracker_id, progress, queued_at, attempts)
    VALUES (?1, ?2, ?3, ?4, ?5, 0)
    ON CONFLICT (kind, entry_id, tracker_id) DO UPDATE SET progress = MAX(progress, excluded.progress)";

pub const DELETE: &str =
    "DELETE FROM track_retry_queue WHERE kind = ?1 AND entry_id = ?2 AND tracker_id = ?3";

pub const LIST: &str = "SELECT entry_id, tracker_id, progress, queued_at, attempts
    FROM track_retry_queue WHERE kind = ?1 ORDER BY queued_at, entry_id, tracker_id";

pub const COUNT: &str = "SELECT COUNT(*) FROM track_retry_queue";

/// Removes the binding's item only when it was not raised after the run read `progress`. A newer add
/// during the run has a higher value, so it survives (race R2).
pub const REMOVE_PROCESSED: &str = "DELETE FROM track_retry_queue
    WHERE kind = ?1 AND entry_id = ?2 AND tracker_id = ?3 AND progress <= ?4";

/// One more failed run for the binding.
pub const BUMP_ATTEMPTS: &str = "UPDATE track_retry_queue SET attempts = attempts + 1
    WHERE kind = ?1 AND entry_id = ?2 AND tracker_id = ?3";

/// `kick` resets every item's attempts (`ExistingWorkPolicy.REPLACE`).
pub const RESET_ATTEMPTS: &str = "UPDATE track_retry_queue SET attempts = 0";
