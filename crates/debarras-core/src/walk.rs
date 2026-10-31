//! Parallel directory walking and on-disk size measurement.
//!
//! Sizes are *allocated* bytes (`st_blocks * 512`), like `du`: a sparse
//! file such as Docker's `Docker.raw` counts for what it really occupies,
//! hard-linked files count once per measurement, and the walk never leaves
//! the volume it started on.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rayon::prelude::*;

/// Cooperative cancellation shared between a caller and a running scan.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Counters updated while walking, read concurrently by progress reporters.
#[derive(Debug, Default)]
pub struct Progress {
    files: AtomicU64,
    bytes: AtomicU64,
}

impl Progress {
    pub fn files(&self) -> u64 {
        self.files.load(Ordering::Relaxed)
    }

    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub(crate) fn add(&self, files: u64, bytes: u64) {
        self.files.fetch_add(files, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// State shared by every measurement of one scan.
#[derive(Debug, Default)]
pub struct ScanState {
    pub cancel: CancelToken,
    pub progress: Progress,
    /// Directories the OS refused to list. On macOS these are mostly
    /// folders protected by TCC until the app gets Full Disk Access.
    denied: Mutex<Vec<PathBuf>>,
}

impl ScanState {
    pub fn new(cancel: CancelToken) -> Self {
        Self {
            cancel,
            ..Self::default()
        }
    }

    pub fn denied(&self) -> Vec<PathBuf> {
        let mut denied = self.denied.lock().expect("denied list poisoned").clone();
        denied.sort();
        denied.dedup();
        denied
    }

    pub(crate) fn record_denied(&self, path: &Path) {
        self.denied
            .lock()
            .expect("denied list poisoned")
            .push(path.to_path_buf());
    }

    /// Bytes allocated on disk under `path`, `0` if it does not exist.
    pub fn allocated_size(&self, path: &Path) -> u64 {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return 0;
        };
        if !meta.is_dir() {
            return allocated(&meta);
        }
        let links = HardLinks::default();
        allocated(&meta) + self.dir_contents_size(path, meta.dev(), &links)
    }

    fn dir_contents_size(&self, dir: &Path, dev: u64, links: &HardLinks) -> u64 {
        let Some(level) = self.read_level(dir, dev, links) else {
            return 0;
        };
        let files: u64 = level.files.iter().map(|f| f.bytes).sum();
        let dirs: u64 = level
            .subdirs
            .par_iter()
            .map(|sub| sub.own_bytes + self.dir_contents_size(&sub.path, dev, links))
            .sum();
        files + dirs
    }

    /// Lists one directory level. Returns `None` when the directory cannot
    /// be read or the scan was cancelled.
    pub(crate) fn read_level(&self, dir: &Path, dev: u64, links: &HardLinks) -> Option<Level> {
        if self.cancel.is_cancelled() {
            return None;
        }
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) => {
                if err.kind() == io::ErrorKind::PermissionDenied {
                    self.record_denied(dir);
                }
                return None;
            }
        };

        let mut level = Level::default();
        for entry in entries.flatten() {
            // `DirEntry::metadata` is an lstat: symlinks are never followed.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                // A different device means a mount point (disk image,
                // simulator runtime, network share): not this volume's space.
                if meta.dev() == dev {
                    level.subdirs.push(SubDir {
                        path: entry.path(),
                        name: entry.file_name(),
                        own_bytes: allocated(&meta),
                        modified: meta.modified().ok(),
                    });
                }
                continue;
            }
            let bytes = if meta.nlink() > 1 && !links.first_sighting(meta.dev(), meta.ino()) {
                0
            } else {
                allocated(&meta)
            };
            level.files.push(FileEntry {
                name: entry.file_name(),
                bytes,
                modified: meta.modified().ok(),
            });
        }

        let bytes: u64 = level.files.iter().map(|f| f.bytes).sum();
        self.progress.add(level.files.len() as u64, bytes);
        Some(level)
    }
}

pub(crate) fn allocated(meta: &Metadata) -> u64 {
    meta.blocks() * 512
}

#[derive(Default)]
pub(crate) struct Level {
    pub files: Vec<FileEntry>,
    pub subdirs: Vec<SubDir>,
}

pub(crate) struct FileEntry {
    pub name: OsString,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
}

pub(crate) struct SubDir {
    pub path: PathBuf,
    pub name: OsString,
    /// Blocks used by the directory entry itself, not its contents.
    pub own_bytes: u64,
    pub modified: Option<SystemTime>,
}

const LINK_SHARDS: usize = 16;

/// Remembers hard-linked inodes already counted. Sharded because pnpm
/// stores and cargo `target/` directories are full of hard links and
/// are walked by many threads at once.
#[derive(Default)]
pub(crate) struct HardLinks {
    shards: [Mutex<HashSet<(u64, u64)>>; LINK_SHARDS],
}

impl HardLinks {
    fn first_sighting(&self, dev: u64, ino: u64) -> bool {
        self.shards[(ino as usize) % LINK_SHARDS]
            .lock()
            .expect("hard link set poisoned")
            .insert((dev, ino))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn write_file(path: &Path, bytes: usize) {
        let mut file = File::create(path).unwrap();
        file.write_all(&vec![7u8; bytes]).unwrap();
        file.sync_all().unwrap();
    }

    #[test]
    fn missing_path_has_no_size() {
        let state = ScanState::default();
        assert_eq!(state.allocated_size(Path::new("/nonexistent/debarras")), 0);
    }

    #[test]
    fn counts_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        write_file(&dir.path().join("a/one"), 100_000);
        write_file(&dir.path().join("a/b/two"), 200_000);

        let state = ScanState::default();
        let size = state.allocated_size(dir.path());
        assert!(size >= 300_000, "size {size} should cover both files");
        assert_eq!(state.progress.files(), 2);
    }

    #[test]
    fn hard_links_are_counted_once() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("original"), 500_000);
        let alone = ScanState::default().allocated_size(dir.path());

        fs::hard_link(dir.path().join("original"), dir.path().join("link")).unwrap();
        let with_link = ScanState::default().allocated_size(dir.path());

        // Only the directory entry grows, never by a second copy of the data.
        assert!(with_link < alone + 100_000, "{with_link} vs {alone}");
    }

    #[test]
    fn symlinks_are_not_followed() {
        let outside = tempfile::tempdir().unwrap();
        write_file(&outside.path().join("big"), 1_000_000);
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();

        assert!(ScanState::default().allocated_size(dir.path()) < 100_000);
    }

    #[test]
    fn sparse_files_count_allocated_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sparse");
        let file = File::create(&path).unwrap();
        file.set_len(1 << 30).unwrap();

        assert!(ScanState::default().allocated_size(&path) < 1 << 20);
    }

    #[test]
    fn cancelled_scan_stops_early() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("file"), 100_000);
        let state = ScanState::default();
        state.cancel.cancel();

        assert!(state.allocated_size(dir.path()) < 100_000);
    }

    #[test]
    fn unreadable_directories_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();

        let state = ScanState::default();
        state.allocated_size(dir.path());
        fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        assert_eq!(state.denied(), vec![locked]);
    }
}
