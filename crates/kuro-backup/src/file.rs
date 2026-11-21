//! Output file (area 21, T-21a-022), post-write self-check (T-21a-023), startup cleanup (T-21a-025), and the
//! suggested file name (T-21a-036).
//!
//! Write order: `<name>.part` (gzip, fsync), self-check decode, then rename to the final name. A failure at any step
//! removes the `.part` file (E-30).

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use flate2::{Compression, write::GzEncoder};

use crate::creator::{CreateInputs, build_backup, build_backup_report};
use crate::error::BackupError;
use crate::format;
use crate::options::BackupOptions;
use crate::schema::Backup;
use crate::wire::Message;

const EXTENSION: &str = ".tachibk";
const PART_SUFFIX: &str = ".part";

/// Where the file goes.
pub enum Target {
    /// The path the user chose in the save dialog. An existing file is replaced.
    Manual(PathBuf),
}

/// Builds the backup from the library and writes it.
pub fn create_file(inputs: &CreateInputs<'_>, opts: &BackupOptions, target: &Target) -> Result<PathBuf, BackupError> {
    let backup = build_backup(inputs, opts)?;
    write_backup(&backup, target)
}

/// [`create_file`] that also returns the extension packages whose artifact was missing and skipped (E-16), for the
/// completion notice (T-21a-019).
pub fn create_file_report(inputs: &CreateInputs<'_>, opts: &BackupOptions, target: &Target) -> Result<(PathBuf, Vec<String>), BackupError> {
    let (backup, skipped) = build_backup_report(inputs, opts)?;
    Ok((write_backup(&backup, target)?, skipped))
}

/// Encodes, writes, self-checks and publishes `backup`. Returns the final path.
pub fn write_backup(backup: &Backup, target: &Target) -> Result<PathBuf, BackupError> {
    let payload = backup.to_bytes().map_err(BackupError::Encode)?;
    let Target::Manual(path) = target;
    let final_path = path.clone();
    let part = part_path(&final_path);
    write_part(&payload, &part)?;
    publish(&part, &final_path)?;
    Ok(final_path)
}

/// `<final>.part`, in the same directory so the rename is atomic.
pub fn part_path(final_path: &Path) -> PathBuf {
    let mut s = final_path.as_os_str().to_owned();
    s.push(PART_SUFFIX);
    PathBuf::from(s)
}

/// Writes the gzip stream to `part` and fsyncs it. Removes `part` on any failure.
pub fn write_part(payload: &[u8], part: &Path) -> Result<(), BackupError> {
    let result = (|| -> io::Result<()> {
        let file = File::create(part)?;
        let mut enc = GzEncoder::new(BufWriter::new(file), Compression::default());
        enc.write_all(payload)?;
        let buf = enc.finish()?;
        let file = buf.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()
    })();
    result.map_err(|e| {
        let _ = fs::remove_file(part);
        BackupError::Io(e)
    })
}

/// Self-check (T-21a-023) then rename `part` to `final_path`. A decode error or a failed rename removes `part`.
pub fn publish(part: &Path, final_path: &Path) -> Result<(), BackupError> {
    if let Err(e) = format::decode_file(part) {
        let _ = fs::remove_file(part);
        return Err(e);
    }
    fs::rename(part, final_path).map_err(|e| {
        let _ = fs::remove_file(part);
        BackupError::Io(e)
    })
}

/// The name the manual save dialog suggests: `kurobayomi_yyyy-MM-dd_HH-mm.tachibk`, no seconds, like Aniyomi's
/// `BackupCreator.getFilename` (T-21a-036).
pub fn manual_suggested_name(now: DateTime<Local>) -> String {
    format!("kurobayomi_{}{}", now.format("%Y-%m-%d_%H-%M"), EXTENSION)
}

/// Removes leftover `*.tachibk.part` files from a crashed run (E-30). A missing folder is not an error.
pub fn remove_stale_parts(dir: &Path) -> Result<usize, BackupError> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        let is_part = entry.file_name().to_str().is_some_and(|n| n.ends_with(".tachibk.part"));
        if is_part && entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::BackupAnime;
    use chrono::TimeZone;

    fn sample() -> Backup {
        Backup {
            anime: vec![BackupAnime { source: 1, url: "a".into(), ..Default::default() }],
            ..Default::default()
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn manual_suggested_name_has_no_seconds() {
        let now = chrono::Local.with_ymd_and_hms(2026, 10, 9, 14, 3, 7).unwrap();
        assert_eq!(manual_suggested_name(now), "kurobayomi_2026-10-09_14-03.tachibk");
    }

    #[test]
    fn write_uses_part_then_rename() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("out.tachibk");
        let part = part_path(&final_path);
        write_part(&sample().to_bytes().unwrap(), &part).unwrap();
        assert!(part.exists());
        assert!(!final_path.exists(), "no final file before the rename");
        publish(&part, &final_path).unwrap();
        assert!(final_path.exists());
        assert!(!part.exists());
        assert_eq!(format::decode_file(&final_path).unwrap().anime.len(), 1);
    }

    #[test]
    fn failure_removes_part() {
        // A final path that is a directory makes the rename fail; the part file must not survive.
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("taken.tachibk");
        fs::create_dir(&final_path).unwrap();
        let err = write_backup(&sample(), &Target::Manual(final_path.clone())).unwrap_err();
        assert!(matches!(err, BackupError::Io(_)));
        assert!(!part_path(&final_path).exists());
    }

    #[test]
    fn write_failure_into_missing_folder_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("missing").join("out.tachibk");
        assert!(write_backup(&sample(), &Target::Manual(final_path)).is_err());
    }

    #[test]
    fn manual_overwrite_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manual.tachibk");
        fs::write(&path, vec![7u8; 100_000]).unwrap();
        write_backup(&sample(), &Target::Manual(path.clone())).unwrap();
        let len = fs::metadata(&path).unwrap().len();
        assert!(len < 100_000, "old longer content must not remain");
        assert_eq!(format::decode_file(&path).unwrap().anime.len(), 1);
    }

    #[test]
    fn self_check_failure_deletes_part() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("bad.tachibk");
        let part = part_path(&final_path);
        fs::write(&part, b"\x0A\xFF\xFF definitely not a backup").unwrap();
        let err = publish(&part, &final_path).unwrap_err();
        assert!(matches!(err, BackupError::Corrupt(_)));
        assert!(!part.exists());
        assert!(!final_path.exists());
    }

    #[test]
    fn stale_part_removed_on_start_real_backups_kept() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("kurobayomi_2026-10-08_19-31-00.tachibk.part"), b"x").unwrap();
        fs::write(dir.path().join("kurobayomi_2026-10-08_19-31-00.tachibk"), b"x").unwrap();
        assert_eq!(remove_stale_parts(dir.path()).unwrap(), 1);
        assert_eq!(names(dir.path()), vec!["kurobayomi_2026-10-08_19-31-00.tachibk".to_string()]);
        assert_eq!(remove_stale_parts(&dir.path().join("absent")).unwrap(), 0);
    }
}
