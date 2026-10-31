//! Identical files, and the space removing the extra copies really frees.
//!
//! Files are compared in three passes so that most are never read: same
//! size, then a BLAKE3 hash of their first and last 64 KiB, then a BLAKE3
//! hash of their whole content. Two macOS specifics are handled:
//! - A copy made by the Finder is an APFS clone sharing its blocks with
//!   the original, so deleting it frees almost nothing. What each copy
//!   really holds comes from `getattrlist(ATTR_CMNEXT_PRIVATESIZE)`, and
//!   groups that would free nothing are left out.
//! - iCloud files kept online only ("dataless") are skipped: reading them
//!   would download them.
//!
//! Only the user's own files are compared: `~/Library`, hidden folders,
//! dependency and cache folders, and the inside of bundles (`.app`,
//! `.photoslibrary`…) are not walked.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::fs::{self, File};
use std::hash::Hash;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::macos::fs::MetadataExt as _;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use rayon::prelude::*;
use serde::Serialize;

use crate::execute::{Action, ExecutionReport, Failure};
use crate::walk::{ScanState, allocated};

/// Size of the head and of the tail hashed by the second pass.
const SAMPLE_BYTES: u64 = 64 * 1024;
/// `SF_DATALESS` from `<sys/stat.h>`: the content is in iCloud only.
const SF_DATALESS: u32 = 0x4000_0000;
/// Folders of installed dependencies, rebuilt by their tools.
const DEPENDENCY_DIRS: &[&str] = &["node_modules", "Pods", "DerivedData"];

#[derive(Debug, Clone)]
pub struct DuplicateOptions {
    pub roots: Vec<PathBuf>,
    pub home: PathBuf,
    /// Smaller files are not worth the comparison.
    pub min_bytes: u64,
}

impl DuplicateOptions {
    pub fn for_home(home: &Path) -> Self {
        Self {
            roots: vec![home.to_path_buf()],
            home: home.to_path_buf(),
            min_bytes: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateFile {
    pub path: PathBuf,
    /// Unix seconds.
    pub modified: Option<u64>,
    /// Bytes removing this copy frees; below its size when it shares
    /// blocks with an APFS clone.
    pub reclaimable: u64,
    /// The copy proposed to keep: the oldest one.
    pub keep: bool,
    #[serde(skip)]
    scanned: Fingerprint,
}

/// What a file looked like when it was hashed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    modified: Option<SystemTime>,
}

impl DuplicateFile {
    fn unchanged(&self) -> bool {
        fs::symlink_metadata(&self.path).is_ok_and(|meta| {
            meta.is_file()
                && Fingerprint {
                    len: meta.len(),
                    modified: meta.modified().ok(),
                } == self.scanned
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateGroup {
    /// Start of the content hash, stable while the content is unchanged.
    pub id: String,
    /// Size of each copy.
    pub size: u64,
    pub files: Vec<DuplicateFile>,
}

impl DuplicateGroup {
    /// Space freed by removing every copy except the one to keep.
    pub fn reclaimable(&self) -> u64 {
        self.files.iter().filter(|f| !f.keep).map(|f| f.reclaimable).sum()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateReport {
    /// Largest reclaimable space first.
    pub groups: Vec<DuplicateGroup>,
    /// Groups of APFS clones left out: removing copies would free nothing.
    pub clone_groups: usize,
    pub denied: Vec<PathBuf>,
}

struct Candidate {
    path: PathBuf,
    dev: u64,
    ino: u64,
    scanned: Fingerprint,
    allocated: u64,
}

pub fn find(options: &DuplicateOptions, state: &ScanState) -> DuplicateReport {
    let found = Mutex::new(Vec::new());
    options.roots.par_iter().for_each(|root| {
        if let Ok(meta) = fs::symlink_metadata(root)
            && meta.is_dir()
        {
            collect(root, meta.dev(), options, state, &found);
        }
    });
    let mut candidates = found.into_inner().expect("candidates poisoned");

    // Hard links are one file under several names, not copies of it.
    candidates.sort_by(|a, b| (a.dev, a.ino, &a.path).cmp(&(b.dev, b.ino, &b.path)));
    candidates.dedup_by_key(|c| (c.dev, c.ino));

    let same_size = split(candidates, |c| Some(c.scanned.len));
    let same_sample: Vec<Vec<Candidate>> = same_size
        .into_par_iter()
        .flat_map(|(_, group)| split(group, |c| sample_hash(c, state)))
        .map(|(_, group)| group)
        .collect();
    let identical: Vec<(blake3::Hash, Vec<Candidate>)> = same_sample
        .into_par_iter()
        .flat_map(|group| split(group, |c| full_hash(c, state)))
        .collect();

    let groups: Vec<DuplicateGroup> = identical
        .into_par_iter()
        .map(|(hash, group)| duplicate_group(&hash, group))
        .collect();
    let (mut groups, clones): (Vec<_>, Vec<_>) = groups
        .into_iter()
        .partition(|group| group.reclaimable() >= options.min_bytes);
    groups.sort_by(|a, b| b.reclaimable().cmp(&a.reclaimable()).then_with(|| a.id.cmp(&b.id)));

    DuplicateReport {
        groups,
        clone_groups: clones.len(),
        denied: state.denied(),
    }
}

fn collect(dir: &Path, dev: u64, options: &DuplicateOptions, state: &ScanState, found: &Mutex<Vec<Candidate>>) {
    if state.cancel.is_cancelled() {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            if err.kind() == io::ErrorKind::PermissionDenied {
                state.record_denied(dir);
            }
            return;
        }
    };

    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        // An lstat: symlinks are neither followed nor compared.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        if meta.is_dir() {
            if meta.dev() == dev && !is_skipped_dir(&path, &options.home) {
                subdirs.push(path);
            }
        } else if meta.is_file() && meta.len() >= options.min_bytes && !is_dataless(&meta) {
            files.push(Candidate {
                path,
                dev: meta.dev(),
                ino: meta.ino(),
                scanned: Fingerprint {
                    len: meta.len(),
                    modified: meta.modified().ok(),
                },
                allocated: allocated(&meta),
            });
        }
    }
    state.progress.add(files.len() as u64, 0);
    found.lock().expect("candidates poisoned").extend(files);
    subdirs
        .par_iter()
        .for_each(|sub| collect(sub, dev, options, state, found));
}

fn is_skipped_dir(path: &Path, home: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.starts_with('.')
        || name == "Library" && path.parent() == Some(home)
        || DEPENDENCY_DIRS.contains(&name)
        || is_bundle(name)
        || path.join("CACHEDIR.TAG").is_file()
        || path.join("pyvenv.cfg").is_file()
}

/// Bundles (`Docker.app`, `Photos Library.photoslibrary`, `Notes.rtfd`)
/// carry a purely alphabetic extension; `Release 1.2` does not.
fn is_bundle(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.len() >= 2 && ext.chars().all(|c| c.is_ascii_alphabetic()))
}

fn is_dataless(meta: &fs::Metadata) -> bool {
    meta.st_flags() & SF_DATALESS != 0 || meta.blocks() == 0
}

/// Groups items by key, computed in parallel; keeps groups of two or more.
/// Items whose key cannot be computed (unreadable file) are dropped.
fn split<K, F>(items: Vec<Candidate>, key: F) -> Vec<(K, Vec<Candidate>)>
where
    K: Eq + Hash + Send,
    F: Fn(&Candidate) -> Option<K> + Sync,
{
    if items.len() < 2 {
        return Vec::new();
    }
    let keyed: Vec<(K, Candidate)> = items
        .into_par_iter()
        .filter_map(|item| key(&item).map(|k| (k, item)))
        .collect();
    let mut groups: HashMap<K, Vec<Candidate>> = HashMap::new();
    for (k, item) in keyed {
        groups.entry(k).or_default().push(item);
    }
    groups.into_iter().filter(|(_, group)| group.len() >= 2).collect()
}

fn sample_hash(candidate: &Candidate, state: &ScanState) -> Option<blake3::Hash> {
    if state.cancel.is_cancelled() {
        return None;
    }
    let mut file = File::open(&candidate.path).ok()?;
    let len = candidate.scanned.len;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; SAMPLE_BYTES as usize];

    let head = read_up_to(&mut file, &mut buffer).ok()?;
    hasher.update(&buffer[..head]);
    let mut read = head as u64;
    if len > 2 * SAMPLE_BYTES {
        file.seek(SeekFrom::Start(len - SAMPLE_BYTES)).ok()?;
        let tail = read_up_to(&mut file, &mut buffer).ok()?;
        hasher.update(&buffer[..tail]);
        read += tail as u64;
    }
    state.progress.add(0, read);
    Some(hasher.finalize())
}

fn read_up_to(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn full_hash(candidate: &Candidate, state: &ScanState) -> Option<blake3::Hash> {
    if state.cancel.is_cancelled() {
        return None;
    }
    let file = File::open(&candidate.path).ok()?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file).ok()?;
    state.progress.add(0, candidate.scanned.len);
    Some(hasher.finalize())
}

fn duplicate_group(hash: &blake3::Hash, group: Vec<Candidate>) -> DuplicateGroup {
    let size = group[0].scanned.len;
    let mut files: Vec<DuplicateFile> = group
        .into_iter()
        .map(|candidate| DuplicateFile {
            reclaimable: private_size(&candidate.path).unwrap_or(candidate.allocated),
            modified: candidate.scanned.modified.map(crate::time::unix_seconds),
            path: candidate.path,
            keep: false,
            scanned: candidate.scanned,
        })
        .collect();
    // The oldest copy is most likely the original; then the shortest path.
    files.sort_by(|a, b| {
        (a.scanned.modified, a.path.as_os_str().len(), &a.path).cmp(&(
            b.scanned.modified,
            b.path.as_os_str().len(),
            &b.path,
        ))
    });
    files[0].keep = true;
    DuplicateGroup {
        id: hash.to_hex()[..16].to_owned(),
        size,
        files,
    }
}

/// Reply layout of `getattrlist` for `ATTR_CMNEXT_PRIVATESIZE`: total
/// length, the attributes actually returned, then the value.
#[repr(C, packed(4))]
struct PrivateSizeReply {
    length: u32,
    returned: libc::attribute_set_t,
    private_size: libc::off_t,
}

/// Bytes deleting `path` would free immediately: blocks shared with an
/// APFS clone or held by a snapshot are not counted. `None` where the
/// file system cannot tell (it then has no clones either).
fn private_size(path: &Path) -> Option<u64> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: libc::ATTR_CMNEXT_PRIVATESIZE,
    };
    // SAFETY: an all-zero reply is a valid value of this plain-data struct.
    let mut reply: PrivateSizeReply = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is NUL-terminated, `request` and `reply` live for the
    // whole call, and the kernel writes at most `size_of` bytes of reply.
    let status = unsafe {
        libc::getattrlist(
            path.as_ptr(),
            (&raw mut request).cast(),
            (&raw mut reply).cast(),
            size_of::<PrivateSizeReply>(),
            libc::FSOPT_NOFOLLOW | libc::FSOPT_ATTR_CMN_EXTENDED,
        )
    };
    let returned = reply.returned;
    if status != 0 || returned.forkattr & libc::ATTR_CMNEXT_PRIVATESIZE == 0 {
        return None;
    }
    u64::try_from(reply.private_size).ok()
}

impl DuplicateReport {
    /// Moves the chosen copies to the Trash. Nothing in a group is touched
    /// unless one of its other copies stays, unchanged since the scan, and
    /// a copy modified since the scan is never removed.
    pub fn execute(&self, paths: &[PathBuf], home: &Path) -> ExecutionReport {
        let chosen_paths: HashSet<&Path> = paths.iter().map(PathBuf::as_path).collect();
        let mut report = ExecutionReport::default();
        for group in &self.groups {
            let (chosen, kept): (Vec<&DuplicateFile>, Vec<&DuplicateFile>) = group
                .files
                .iter()
                .partition(|f| chosen_paths.contains(f.path.as_path()));
            if chosen.is_empty() {
                continue;
            }
            if !kept.iter().any(|f| f.unchanged()) {
                report.failures.extend(chosen.iter().map(|f| Failure {
                    target: f.path.display().to_string(),
                    reason:
                        "Aucun autre exemplaire intact ne resterait : rien n'est supprimé dans ce groupe.".to_owned(),
                }));
                continue;
            }
            for file in chosen {
                let target = file.path.display().to_string();
                if !file.unchanged() {
                    report.failures.push(Failure {
                        target,
                        reason: "Modifié depuis l'analyse : laissé en place.".to_owned(),
                    });
                    continue;
                }
                let action = Action::Trash {
                    paths: vec![file.path.clone()],
                };
                report.run(&target, file.reclaimable, &action, home);
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Content that differs per `seed` and is not compressible or sparse.
    fn content(seed: u8, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn write(path: &Path, data: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    struct Home {
        _dir: tempfile::TempDir,
        path: PathBuf,
    }

    impl Home {
        fn new() -> Self {
            let dir = crate::testing::tempdir();
            let path = fs::canonicalize(dir.path()).unwrap();
            Self { _dir: dir, path }
        }

        fn file(&self, rel: &str, data: &[u8]) -> PathBuf {
            let path = self.path.join(rel);
            write(&path, data);
            path
        }

        fn scan(&self) -> DuplicateReport {
            let options = DuplicateOptions {
                min_bytes: 10_000,
                ..DuplicateOptions::for_home(&self.path)
            };
            find(&options, &ScanState::default())
        }
    }

    fn paths(group: &DuplicateGroup) -> Vec<PathBuf> {
        group.files.iter().map(|f| f.path.clone()).collect()
    }

    #[test]
    fn finds_identical_files_and_only_them() {
        let home = Home::new();
        let video = content(1, 300_000);
        let a = home.file("Movies/clip.mov", &video);
        let b = home.file("Downloads/clip (1).mov", &video);
        home.file("Downloads/same-size-other.mov", &content(2, 300_000));
        home.file("Documents/other-size.mov", &video[..200_000]);

        let report = home.scan();
        assert_eq!(report.groups.len(), 1);
        let mut found = paths(&report.groups[0]);
        found.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(found, expected);
        assert_eq!(report.groups[0].size, 300_000);
        assert!(report.groups[0].reclaimable() >= 300_000);
    }

    #[test]
    fn files_differing_only_in_the_middle_are_not_duplicates() {
        let home = Home::new();
        let original = content(3, 400_000);
        let mut edited = original.clone();
        edited[200_000] ^= 0xFF;
        home.file("Documents/a.bin", &original);
        home.file("Documents/b.bin", &edited);

        assert!(home.scan().groups.is_empty());
    }

    #[test]
    fn hard_links_are_not_duplicates() {
        let home = Home::new();
        let a = home.file("Documents/a.bin", &content(4, 50_000));
        fs::hard_link(&a, home.path.join("Documents/link.bin")).unwrap();

        assert!(home.scan().groups.is_empty());
    }

    #[test]
    fn apfs_clones_free_nothing_and_are_left_out() {
        let home = Home::new();
        let a = home.file("Documents/a.bin", &content(5, 2_000_000));
        assert!(
            private_size(&a).is_some_and(|size| size >= 2_000_000),
            "a fresh file owns its blocks"
        );

        let clone = home.path.join("Documents/a copy.bin");
        let status = std::process::Command::new("/bin/cp")
            .arg("-c")
            .arg(&a)
            .arg(&clone)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(
            private_size(&clone).is_some_and(|size| size < 100_000),
            "a clone shares its blocks"
        );

        let report = home.scan();
        assert!(report.groups.is_empty());
        assert_eq!(report.clone_groups, 1);
    }

    #[test]
    fn library_hidden_dependency_cache_and_bundle_folders_are_not_compared() {
        let home = Home::new();
        let data = content(6, 50_000);
        home.file("Documents/keep.bin", &data);
        for rel in [
            "Library/Caches/x.bin",
            ".config/x.bin",
            "Developer/app/node_modules/x.bin",
            "Developer/tool/target/x.bin",
            "Applications/Tool.app/Contents/x.bin",
            "Pictures/Photos Library.photoslibrary/originals/x.bin",
        ] {
            home.file(rel, &data);
        }
        home.file(
            "Developer/tool/target/CACHEDIR.TAG",
            b"Signature: 8a477f597d28d172789f06886806bc55",
        );

        assert!(home.scan().groups.is_empty());
    }

    #[test]
    fn folders_with_version_numbers_are_still_compared() {
        let home = Home::new();
        let data = content(7, 50_000);
        home.file("Documents/Release 1.2/setup.bin", &data);
        home.file("Documents/setup.bin", &data);

        assert_eq!(home.scan().groups.len(), 1);
    }

    #[test]
    fn the_oldest_copy_is_kept() {
        let home = Home::new();
        let data = content(8, 50_000);
        let newer = home.file("Downloads/photo.jpg", &data);
        let older = home.file("Pictures/photo.jpg", &data);
        let a_year_ago = SystemTime::now() - Duration::from_secs(365 * 86_400);
        File::options()
            .write(true)
            .open(&older)
            .unwrap()
            .set_modified(a_year_ago)
            .unwrap();

        let group = &home.scan().groups[0];
        let kept: Vec<&Path> = group
            .files
            .iter()
            .filter(|f| f.keep)
            .map(|f| f.path.as_path())
            .collect();
        assert_eq!(kept, vec![older.as_path()]);
        assert!(group.files.iter().any(|f| f.path == newer && !f.keep));
    }

    #[test]
    fn removing_extra_copies_moves_them_to_the_trash() {
        let home = Home::new();
        let data = content(9, 50_000);
        let a = home.file("Documents/a.bin", &data);
        let b = home.file("Downloads/b.bin", &data);
        let report = home.scan();
        let extra = report.groups[0].files.iter().find(|f| !f.keep).unwrap().path.clone();

        let result = report.execute(std::slice::from_ref(&extra), &home.path);
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert_eq!(result.completed.len(), 1);
        assert!(!extra.exists());
        assert!(a.exists() || b.exists());
        assert_eq!(fs::read_dir(home.path.join(".Trash")).unwrap().count(), 1);
    }

    #[test]
    fn every_copy_of_a_group_is_never_removed() {
        let home = Home::new();
        let data = content(10, 50_000);
        let a = home.file("Documents/a.bin", &data);
        let b = home.file("Downloads/b.bin", &data);
        let report = home.scan();

        let result = report.execute(&[a.clone(), b.clone()], &home.path);
        assert!(result.completed.is_empty());
        assert_eq!(result.failures.len(), 2);
        assert!(a.exists() && b.exists());
    }

    #[test]
    fn a_copy_changed_since_the_scan_is_left_alone() {
        let home = Home::new();
        let data = content(11, 50_000);
        home.file("Documents/a.bin", &data);
        let b = home.file("Downloads/b.bin", &data);
        let report = home.scan();
        fs::write(&b, content(12, 60_000)).unwrap();

        let result = report.execute(std::slice::from_ref(&b), &home.path);
        assert!(result.completed.is_empty(), "{:?}", result.completed);
        assert_eq!(result.failures[0].reason, "Modifié depuis l'analyse : laissé en place.");
        assert!(b.exists());
    }
}
