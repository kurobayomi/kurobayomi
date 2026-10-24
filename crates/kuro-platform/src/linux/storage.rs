//! Disk space through `statvfs`, for the filesystem that holds a path, and the storage roots from the mount table.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rustix::fs::statvfs;

/// Bytes available to unprivileged users on the filesystem holding `path`.
/// `None` when the path is missing or `statvfs` fails.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let s = statvfs(path).ok()?;
    Some(s.f_bavail * s.f_frsize)
}

/// Size of the filesystem holding `path`. `None` when the path is missing or `statvfs` fails.
pub fn total_bytes(path: &Path) -> Option<u64> {
    let s = statvfs(path).ok()?;
    Some(s.f_blocks * s.f_frsize)
}

/// The file-system types a storage root can have (T-47b-037).
pub const ROOT_FS_TYPES: [&str; 11] = ["ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3", "fuseblk"];

/// The mount table of this process.
const MOUNT_INFO: &str = "/proc/self/mountinfo";

/// One line of the mount table, with the fields the roots need.
#[derive(Debug, PartialEq)]
struct Mount {
    device: String,
    point: PathBuf,
    fs_type: String,
    source: String,
}

/// The mount lines of a `mountinfo` table. The fields after the optional ones end at `-`, then come the file-system
/// type and the source; a line without that separator is skipped.
fn parse_mounts(table: &str) -> Vec<Mount> {
    table
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let dash = fields.iter().position(|f| *f == "-")?;
            Some(Mount {
                device: fields.get(2)?.to_string(),
                point: PathBuf::from(unescape(fields.get(4)?)),
                fs_type: fields.get(dash + 1)?.to_string(),
                source: fields.get(dash + 2).unwrap_or(&"").to_string(),
            })
        })
        .collect()
}

/// The mount table writes a space as `\040` and a tab as `\011`: three octal digits after a backslash.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = (bytes[i] == b'\\' && i + 3 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 4]).ok().and_then(|d| u8::from_str_radix(d, 8).ok()))
            .flatten();
        match octal {
            Some(byte) => {
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The storage roots of a mount table: the mounts of the [`ROOT_FS_TYPES`] whose source is a `/dev/` device (any
/// `fuseblk` mount), and the mount that holds `data_dir`. One root per device, the shortest mount point; the roots are
/// sorted.
pub fn roots_in(table: &str, data_dir: &Path) -> Vec<PathBuf> {
    let mounts = parse_mounts(table);
    let mut candidates: Vec<&Mount> = mounts
        .iter()
        .filter(|m| ROOT_FS_TYPES.contains(&m.fs_type.as_str()) && (m.source.starts_with("/dev/") || m.fs_type == "fuseblk"))
        .collect();
    if let Some(holder) = mounts.iter().filter(|m| data_dir.starts_with(&m.point)).max_by_key(|m| m.point.as_os_str().len()) {
        candidates.push(holder);
    }
    let mut by_device: BTreeMap<&str, &Path> = BTreeMap::new();
    for mount in candidates {
        match by_device.get(mount.device.as_str()) {
            Some(kept) if kept.as_os_str().len() <= mount.point.as_os_str().len() => {}
            _ => {
                by_device.insert(&mount.device, &mount.point);
            }
        }
    }
    let mut roots: Vec<PathBuf> = by_device.into_values().map(Path::to_path_buf).collect();
    roots.sort();
    roots
}

/// The storage roots of this process's mount table ([`roots_in`]).
pub fn roots(data_dir: &Path) -> Vec<PathBuf> {
    roots_in(&std::fs::read_to_string(MOUNT_INFO).unwrap_or_default(), data_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
20 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
21 20 8:3 / /home rw,relatime shared:2 - btrfs /dev/sdb1 rw,subvol=/@home
22 20 0:45 / /run/user/1000/gvfs rw,nosuid - fuse.gvfsd-fuse gvfsd-fuse rw
23 20 8:4 / /run/media/me/My\\040Disk rw - exfat /dev/sdc1 rw
24 20 0:50 / /proc rw - proc proc rw
25 20 7:0 / /snap/core rw - squashfs /dev/loop0 ro
26 20 8:3 /@snap /home/me/.snapshots rw - btrfs /dev/sdb1 rw,subvol=/@snap
27 20 0:60 / /mnt/phone rw - fuseblk mtp:host=phone rw
28 20 0:70 / /srv rw - tmpfs tmpfs rw
29 20 8:9 / /boot rw - vfat /dev/sdd1 rw
garbage without separator
";

    #[test]
    fn roots_are_the_device_mounts_once_per_device_sorted_and_the_fuse_mount_counts() {
        let roots = roots_in(TABLE, Path::new("/home/me/.local/share/kurobayomi"));
        let shown: Vec<String> = roots.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(shown, ["/", "/boot", "/home", "/mnt/phone", "/run/media/me/My Disk"]);
    }

    #[test]
    fn the_data_directory_mount_is_a_root_even_when_its_type_is_not_a_disk_type() {
        let roots = roots_in(TABLE, Path::new("/srv/kuro"));
        assert!(roots.iter().any(|p| p == Path::new("/srv")), "{roots:?}");
        assert!(!roots.iter().any(|p| p == Path::new("/proc") || p == Path::new("/snap/core")));
    }

    #[test]
    fn an_empty_table_has_no_roots_beyond_the_data_mount() {
        assert!(roots_in("", Path::new("/x")).is_empty());
    }

    #[test]
    fn octal_escapes_are_decoded() {
        assert_eq!(unescape("a\\040b\\011c"), "a b\tc");
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape("tail\\04"), "tail\\04", "a short escape stays as written");
    }


    #[test]
    fn missing_path_has_no_space_figures() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("absent");
        assert_eq!(free_bytes(&missing), None);
        assert_eq!(total_bytes(&missing), None);
    }

    #[test]
    fn existing_directory_reports_total_and_free_space() {
        let tmp = tempfile::tempdir().unwrap();
        let total = total_bytes(tmp.path()).unwrap();
        assert!(total > 0);
        assert!(free_bytes(tmp.path()).unwrap() <= total);
    }
}
