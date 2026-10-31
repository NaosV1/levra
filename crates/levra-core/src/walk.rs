//! Parallel directory walking and on-disk size measurement.
//!
//! Sizes are *allocated* bytes (`st_blocks * 512`), like `du`: a sparse
//! file such as Docker's `Docker.raw` counts for what it really occupies,
//! hard-linked files count once per measurement, and the walk never leaves
//! the volume it started on nor enters a folder it already reaches by its
//! main path (see [`Bounds`]).

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use rayon::prelude::*;

use crate::listing::{EntryKind, list_dir};

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
    /// Bit `n` is set once step `n` of a scan made of steps is over.
    steps_done: AtomicU32,
}

impl Progress {
    pub fn files(&self) -> u64 {
        self.files.load(Ordering::Relaxed)
    }

    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub fn steps_done(&self) -> u32 {
        self.steps_done.load(Ordering::Relaxed)
    }

    pub(crate) fn finish_step(&self, index: usize) {
        self.steps_done.fetch_or(1 << index, Ordering::Relaxed);
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
        allocated(&meta) + self.dir_contents_size(path, &Bounds::new(path, &meta), &links)
    }

    fn dir_contents_size(&self, dir: &Path, bounds: &Bounds, links: &HardLinks) -> u64 {
        let Some(level) = self.read_level(dir, bounds, links) else {
            return 0;
        };
        let files: u64 = level.files.iter().map(|f| f.bytes).sum();
        let dirs: u64 = level
            .subdirs
            .par_iter()
            .map(|sub| sub.own_bytes + self.dir_contents_size(&sub.path, bounds, links))
            .sum();
        files + dirs
    }

    /// Lists one directory level. Returns `None` when the directory cannot
    /// be read or the scan was cancelled.
    pub(crate) fn read_level(&self, dir: &Path, bounds: &Bounds, links: &HardLinks) -> Option<Level> {
        if self.cancel.is_cancelled() {
            return None;
        }
        let entries = match list_dir(dir) {
            Ok(entries) => entries,
            Err(err) => {
                if err.kind() == io::ErrorKind::PermissionDenied {
                    self.record_denied(dir);
                }
                return None;
            }
        };

        let mut level = Level::default();
        for entry in entries {
            match entry.kind {
                EntryKind::Dir => {
                    let path = dir.join(&entry.name);
                    if bounds.includes(&path, entry.dev) && !entry.is_dataless() {
                        level.subdirs.push(SubDir {
                            path,
                            name: entry.name,
                            own_bytes: entry.allocated,
                            modified: entry.modified,
                        });
                    }
                }
                EntryKind::File | EntryKind::Other => {
                    let bytes = if entry.nlink > 1 && !links.first_sighting(entry.dev, entry.ino) {
                        0
                    } else {
                        entry.allocated
                    };
                    level.files.push(FileEntry {
                        name: entry.name,
                        bytes,
                        modified: entry.modified,
                    });
                }
            }
        }

        let bytes: u64 = level.files.iter().map(|f| f.bytes).sum();
        self.progress.add(level.files.len() as u64, bytes);
        Some(level)
    }
}

/// Bytes allocated under each path, measured side by side.
pub fn measure_all(paths: &[PathBuf]) -> Vec<u64> {
    // un état par dossier, comme ça chq taille est juste même si deux dossiers partagent des liens physiques
    paths
        .par_iter()
        .map(|path| ScanState::default().allocated_size(path))
        .collect()
}

pub(crate) fn allocated(meta: &Metadata) -> u64 {
    meta.blocks() * 512
}

/// Where the data volume is mounted. Its main folders are also reachable
/// through the "firmlinks" macOS lists in `/usr/share/firmlinks` (`/Users`,
/// `/Applications`, `/System/Library/AssetsV2`…): the same folder under two
/// paths, with the same device and inode.
const DATA_VOLUME: &str = "/System/Volumes/Data";

/// Where one walk may go.
pub(crate) struct Bounds {
    dev: u64,
    /// Data-volume paths whose main path is inside the walk too: entering
    /// them would count the same files twice.
    aliases: Vec<PathBuf>,
}

impl Bounds {
    pub(crate) fn new(root: &Path, root_meta: &Metadata) -> Self {
        Self::with_firmlinks(root, root_meta, system_firmlinks())
    }

    /// `firmlinks` pairs each main path with its data-volume alias.
    fn with_firmlinks(root: &Path, root_meta: &Metadata, firmlinks: &[(PathBuf, PathBuf)]) -> Self {
        Self {
            dev: root_meta.dev(),
            aliases: firmlinks
                .iter()
                .filter(|(main, _)| main.starts_with(root))
                .map(|(_, alias)| alias.clone())
                .collect(),
        }
    }

    /// A different device means a mount point (disk image, simulator
    /// runtime, network share): not this volume's space.
    fn includes(&self, dir: &Path, dev: u64) -> bool {
        dev == self.dev && !self.aliases.iter().any(|alias| alias == dir)
    }
}

fn system_firmlinks() -> &'static [(PathBuf, PathBuf)] {
    static FIRMLINKS: OnceLock<Vec<(PathBuf, PathBuf)>> = OnceLock::new();
    FIRMLINKS.get_or_init(|| parse_firmlinks(&fs::read_to_string("/usr/share/firmlinks").unwrap_or_default()))
}

/// Lines are `/Users<TAB>Users`: the main path, then the path inside the
/// data volume.
fn parse_firmlinks(text: &str) -> Vec<(PathBuf, PathBuf)> {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(main, inside)| (PathBuf::from(main.trim()), Path::new(DATA_VOLUME).join(inside.trim())))
        .collect()
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
    fn measures_several_folders_at_once() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("a")).unwrap();
        write_file(&dir.path().join("a/big"), 300_000);
        let sizes = measure_all(&[dir.path().join("a"), dir.path().join("missing")]);
        assert!(sizes[0] >= 300_000);
        assert_eq!(sizes[1], 0);
    }

    #[test]
    fn missing_path_has_no_size() {
        let state = ScanState::default();
        assert_eq!(state.allocated_size(Path::new("/nonexistent/levra")), 0);
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
    fn reads_the_firmlinks_list() {
        let parsed = parse_firmlinks("/Users\tUsers\n/System/Library/AssetsV2\tSystem/Library/AssetsV2\n");
        assert_eq!(
            parsed,
            vec![
                (PathBuf::from("/Users"), PathBuf::from("/System/Volumes/Data/Users")),
                (
                    PathBuf::from("/System/Library/AssetsV2"),
                    PathBuf::from("/System/Volumes/Data/System/Library/AssetsV2")
                ),
            ]
        );
    }

    #[test]
    fn a_folder_reachable_by_two_paths_counts_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("Users/me")).unwrap();
        fs::create_dir_all(root.join("Data/Users/me")).unwrap();
        write_file(&root.join("Users/me/big"), 1_000_000);
        write_file(&root.join("Data/Users/me/big"), 1_000_000);
        let firmlinks = [(root.join("Users"), root.join("Data/Users"))];
        let meta = fs::symlink_metadata(root).unwrap();

        // Measuring the whole disk: the alias is skipped.
        let state = ScanState::default();
        let bounds = Bounds::with_firmlinks(root, &meta, &firmlinks);
        let whole = state.dir_contents_size(root, &bounds, &HardLinks::default());
        assert!((1_000_000..2_000_000).contains(&whole), "{whole}");

        // Measuring the data volume alone: its folders are its own.
        let data = root.join("Data");
        let data_meta = fs::symlink_metadata(&data).unwrap();
        let bounds = Bounds::with_firmlinks(&data, &data_meta, &firmlinks);
        assert!(state.dir_contents_size(&data, &bounds, &HardLinks::default()) >= 1_000_000);
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
