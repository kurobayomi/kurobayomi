//! Kurobayomi migrations: version N to N+1, one transaction per step together with the
//! `user_version` bump, after a `VACUUM INTO` backup. The list itself lives in `schema_unified`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Transaction};

use crate::open::{DbError, DbResult};

/// Pre-migration backups kept next to the database file.
pub const KEEP_BACKUPS: usize = 3;

/// Highest version a bundle supports: its last migration, or the baseline when there is none.
pub fn latest_version(baseline: i64, migrations: &[(i64, &str)]) -> i64 {
    migrations.last().map_or(baseline, |m| m.0)
}

/// True when the targets are `baseline + 1, baseline + 2, ...` with no gap and no duplicate.
pub fn is_contiguous(baseline: i64, migrations: &[(i64, &str)]) -> bool {
    migrations
        .iter()
        .enumerate()
        .all(|(i, (to, _))| *to == baseline + 1 + i as i64)
}

/// Applies every migration above `from`. A backup is taken first when any is pending. Each step
/// runs in its own transaction and is rolled back unless `PRAGMA foreign_key_check` is clean.
pub fn run(c: &mut Connection, path: &Path, from: i64, migrations: &[(i64, &str)]) -> DbResult<()> {
    let pending: Vec<&(i64, &str)> = migrations.iter().filter(|(to, _)| *to > from).collect();
    if pending.is_empty() {
        return Ok(());
    }
    backup(c, path, from)?;
    for &&(to, sql) in &pending {
        let tx = c.transaction()?;
        tx.execute_batch(sql)?;
        check_foreign_keys(&tx, to)?;
        tx.pragma_update(None, "user_version", to)?;
        tx.commit()?;
        tracing::info!(from = to - 1, to, "applied database migration");
    }
    Ok(())
}

/// Version-code bookkeeping for one open. `last_version_code` is the code the app last ran with. The
/// caller reads it from `state.toml` (kuro-prefs, `LAST_VERSION_CODE`, default 0) and stores the
/// returned code back there; `None` means store nothing. A downgrade (stored code above the current
/// one) logs a warning, runs nothing and returns `None`. Otherwise the pending steps run, which is a
/// no-op when `user_version` is already at the latest step.
pub fn on_open(
    c: &mut Connection,
    path: &Path,
    last_version_code: i64,
    current_version_code: i64,
    migrations: &[(i64, &str)],
) -> DbResult<Option<i64>> {
    if last_version_code > current_version_code {
        tracing::warn!("Downgrade detected ({last_version_code} > {current_version_code})");
        return Ok(None);
    }
    tracing::info!("Migration from {last_version_code} to {current_version_code}");
    let from: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
    run(c, path, from, migrations)?;
    tracing::info!("Updating last version to {current_version_code}");
    Ok(Some(current_version_code))
}

fn check_foreign_keys(tx: &Transaction<'_>, to: i64) -> DbResult<()> {
    let mut st = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = st.query([])?;
    if let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        return Err(DbError::Constraint(format!(
            "migration to version {to} leaves a foreign key violation in {table}"
        )));
    }
    Ok(())
}

/// Writes `<file>.pre-v<from>-<UTC>.bak` next to the database and keeps the newest `KEEP_BACKUPS`.
fn backup(c: &Connection, path: &Path, from: i64) -> DbResult<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("database file name is not UTF-8"))?;
    let dest = dir.join(format!("{file}.pre-v{from}-{}.bak", utc_stamp(now_secs())));
    let dest = dest
        .to_str()
        .ok_or_else(|| std::io::Error::other("backup path is not UTF-8"))?;
    c.execute("VACUUM INTO ?1", [dest])?;
    prune(dir, file)
}

/// The pre-migration backups of the database file `file` in `dir`, oldest first, with their stamps.
pub(crate) fn backups_of(dir: &Path, file: &str) -> DbResult<Vec<(String, PathBuf)>> {
    let prefix = format!("{file}.pre-v");
    let mut backups: Vec<(String, PathBuf)> = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_owned();
            if !(name.starts_with(&prefix) && name.ends_with(".bak")) {
                return None;
            }
            // The stamp follows the last '-' and has no '-' itself, so sorting it is chronological.
            let stamp = name.rsplit_once('-')?.1.to_owned();
            Some((stamp, p))
        })
        .collect();
    backups.sort();
    Ok(backups)
}

fn prune(dir: &Path, file: &str) -> DbResult<()> {
    let mut backups = backups_of(dir, file)?;
    let excess = backups.len().saturating_sub(KEEP_BACKUPS);
    for (_, p) in backups.drain(..excess) {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// `YYYYMMDDTHHMMSSZ` for a Unix time in seconds (proleptic Gregorian, UTC).
pub fn utc_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400) + 719_468;
    let rem = secs.rem_euclid(86_400);
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_unified::UNIFIED_SCHEMA;

    #[test]
    fn latest_is_last_migration_or_baseline() {
        assert_eq!(latest_version(33, &[]), 33);
        assert_eq!(latest_version(33, &[(34, ""), (35, "")]), 35);
    }

    #[test]
    fn contiguity_rejects_gaps_and_duplicates() {
        assert!(is_contiguous(33, &[]));
        assert!(is_contiguous(33, &[(34, ""), (35, "")]));
        assert!(!is_contiguous(33, &[(35, "")]));
        assert!(!is_contiguous(33, &[(34, ""), (34, "")]));
        assert!(!is_contiguous(33, &[(34, ""), (36, "")]));
    }

    #[test]
    fn shipped_migration_list_is_contiguous() {
        assert!(is_contiguous(UNIFIED_SCHEMA.baseline_version, UNIFIED_SCHEMA.migrations));
    }

    /// Shared buffer that the test subscriber writes into.
    #[derive(Clone, Default)]
    struct Logs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Logs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
        type Writer = Logs;

        fn make_writer(&'a self) -> Logs {
            self.clone()
        }
    }

    /// Runs `f` under an info-level subscriber and returns its value with the text it logged.
    fn logged<T>(f: impl FnOnce() -> T) -> (T, String) {
        let logs = Logs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .finish();
        let value = tracing::subscriber::with_default(subscriber, || {
            // parallel tests can cache a log statement as unwatched before this thread's subscriber arrives: rebuild the cache
            tracing::callsite::rebuild_interest_cache();
            f()
        });
        let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        (value, text)
    }

    /// A file database at schema version 33 with one table and no steps applied.
    fn file_db_at_33(dir: &Path) -> (PathBuf, Connection) {
        let path = dir.join("manga.db");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE base(x INTEGER);").unwrap();
        c.pragma_update(None, "user_version", 33).unwrap();
        (path, c)
    }

    const STEPS: &[(i64, &str)] = &[
        (34, "CREATE TABLE step34(x INTEGER);"),
        (35, "CREATE TABLE step35(x INTEGER);"),
    ];

    fn user_version(c: &Connection) -> i64 {
        c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap()
    }

    #[test]
    fn opening_an_older_schema_logs_both_messages_and_returns_the_code_to_store() {
        let dir = tempfile::tempdir().unwrap();
        let (path, mut c) = file_db_at_33(dir.path());
        let (result, logs) = logged(|| on_open(&mut c, &path, 1, 2, STEPS));
        assert_eq!(result.unwrap(), Some(2));
        assert_eq!(user_version(&c), 35, "both pending steps ran");
        assert!(logs.contains("Migration from 1 to 2"), "{logs}");
        assert!(logs.contains("Updating last version to 2"), "{logs}");
    }

    #[test]
    fn first_install_with_an_empty_plan_still_returns_the_code_to_store() {
        let dir = tempfile::tempdir().unwrap();
        let (path, mut c) = file_db_at_33(dir.path());
        let (result, logs) = logged(|| on_open(&mut c, &path, 0, 2, &[]));
        assert_eq!(result.unwrap(), Some(2));
        assert_eq!(user_version(&c), 33);
        assert!(logs.contains("Migration from 0 to 2"), "{logs}");
        assert!(logs.contains("Updating last version to 2"), "{logs}");
    }

    #[test]
    fn a_downgrade_warns_runs_nothing_and_stores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (path, mut c) = file_db_at_33(dir.path());
        let (result, logs) = logged(|| on_open(&mut c, &path, 5, 2, STEPS));
        assert_eq!(result.unwrap(), None);
        assert_eq!(user_version(&c), 33);
        assert!(logs.contains("Downgrade detected (5 > 2)"), "{logs}");
        assert!(!logs.contains("Updating last version"), "{logs}");
    }

    #[test]
    fn utc_stamp_matches_known_instants() {
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(1_700_000_000), "20231114T221320Z");
        assert_eq!(utc_stamp(1_709_164_800), "20240229T000000Z");
        assert_eq!(utc_stamp(1_709_251_199), "20240229T235959Z");
    }

    #[test]
    fn prune_keeps_newest_backups_of_this_file_only() {
        let dir = tempfile::tempdir().unwrap();
        let names = [
            "manga.db.pre-v33-20260101T000000Z.bak",
            "manga.db.pre-v33-20260102T000000Z.bak",
            "manga.db.pre-v34-20260103T000000Z.bak",
            "manga.db.pre-v33-20260104T000000Z.bak",
            "manga.db.pre-v33-20260105T000000Z.bak",
            "other.db.pre-v33-20260101T000000Z.bak",
            "manga.db",
        ];
        for n in names {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        prune(dir.path(), "manga.db").unwrap();
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "manga.db",
                "manga.db.pre-v33-20260104T000000Z.bak",
                "manga.db.pre-v33-20260105T000000Z.bak",
                "manga.db.pre-v34-20260103T000000Z.bak",
                "other.db.pre-v33-20260101T000000Z.bak",
            ]
        );
    }
}
