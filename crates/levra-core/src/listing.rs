//! Listing d'un dossier avec ses attributs en un seul appel système par lot.

// readdir puis un lstat par entrée ça fait 1 syscall par fichier, ici
// getattrlistbulk renvoie nom taille inode etc pr des centaines d'entrées d'un coup
// et on bloque la matérialisation icloud/file provider sinon le scan télécharge tt le cloud mdr

use std::cell::RefCell;
use std::ffi::{CString, OsString};
use std::fs;
use std::io;
use std::mem;
use std::os::macos::fs::MetadataExt as _;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Once;
use std::time::{Duration, SystemTime};

/// `SF_DATALESS` de `<sys/stat.h>` : contenu seulement ds le cloud.
pub(crate) const SF_DATALESS: u32 = 0x4000_0000;

// pas ds la crate libc donc on les déclare à la main
const ATTR_CMN_ERROR: u32 = 0x2000_0000;
const SF_FIRMLINK: u32 = 0x0080_0000;
const VREG: u32 = 1;
const VDIR: u32 = 2;
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
const IOPOL_SCOPE_PROCESS: libc::c_int = 0;
const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: libc::c_int = 1;

unsafe extern "C" {
    fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
}

/// 256 Kio : qq centaines d'entrées par appel.
const BUFFER_WORDS: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    Dir,
    File,
    /// Lien symbolique, socket… : jamais suivi, compté pr ses propres blocs.
    Other,
}

#[derive(Debug)]
pub(crate) struct Entry {
    pub name: OsString,
    pub kind: EntryKind,
    pub dev: u64,
    pub ino: u64,
    pub nlink: u32,
    /// Octets alloués sur le disque, comme `st_blocks * 512`.
    pub allocated: u64,
    /// Taille logique, comme `st_size`.
    pub len: u64,
    pub flags: u32,
    pub modified: Option<SystemTime>,
}

impl Entry {
    pub(crate) fn is_dataless(&self) -> bool {
        self.flags & SF_DATALESS != 0
    }
}

/// Toutes les entrées de `dir` sans suivre les liens symboliques.
pub(crate) fn list_dir(dir: &Path) -> io::Result<Vec<Entry>> {
    never_materialize();
    match list_bulk(dir) {
        Err(err) if matches!(err.raw_os_error(), Some(libc::EINVAL | libc::ENOTSUP)) => list_slow(dir),
        result => result,
    }
}

/// Un dossier cloud pas encore téléchargé renvoie une erreur au lieu de
/// partir sur le réseau, ça vaut pr tt le process et c'est voulu.
fn never_materialize() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_PROCESS,
            IOPOL_MATERIALIZE_DATALESS_FILES_OFF,
        );
    });
}

struct Fd(libc::c_int);

impl Drop for Fd {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

thread_local! {
    // un buffer par thread rayon, pas de malloc de 256k à chaque dossier
    static BUFFER: RefCell<Vec<u64>> = RefCell::new(vec![0; BUFFER_WORDS]);
}

fn list_bulk(dir: &Path) -> io::Result<Vec<Entry>> {
    let path = CString::new(dir.as_os_str().as_bytes()).map_err(|_| io::ErrorKind::InvalidInput)?;
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = Fd(fd);

    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS
            | libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_DEVID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_MODTIME
            | libc::ATTR_CMN_FLAGS
            | libc::ATTR_CMN_FILEID
            | ATTR_CMN_ERROR,
        volattr: 0,
        dirattr: libc::ATTR_DIR_MOUNTSTATUS,
        fileattr: libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE | libc::ATTR_FILE_DATALENGTH,
        forkattr: 0,
    };

    BUFFER.with_borrow_mut(|buffer| {
        let mut entries = Vec::new();
        loop {
            let count = unsafe {
                libc::getattrlistbulk(
                    fd.0,
                    (&raw mut request).cast(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len() * mem::size_of::<u64>(),
                    0,
                )
            };
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            if count == 0 {
                return Ok(entries);
            }
            let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8) };
            let mut offset = 0;
            for _ in 0..count {
                // un enregistrement louche et on arrête là plutôt que de lire n'importe où
                let Some(length) = read::<u32>(bytes, offset).filter(|&l| l > 0) else {
                    break;
                };
                let Some(record) = bytes.get(offset..offset + length as usize) else {
                    break;
                };
                if let Some(entry) = parse(record, dir) {
                    entries.push(entry);
                }
                offset += length as usize;
            }
        }
    })
}

fn read<T: Copy>(bytes: &[u8], at: usize) -> Option<T> {
    let end = at.checked_add(mem::size_of::<T>())?;
    let slice = bytes.get(at..end)?;
    Some(unsafe { slice.as_ptr().cast::<T>().read_unaligned() })
}

/// Un enregistrement : longueur, attributs renvoyés, puis chq attribut
/// demandé ds l'ordre de son bit, seulement s'il est renvoyé.
fn parse(record: &[u8], dir: &Path) -> Option<Entry> {
    let returned: libc::attribute_set_t = read(record, 4)?;
    let mut at = 4 + mem::size_of::<libc::attribute_set_t>();
    let common = returned.commonattr;

    let mut name = OsString::new();
    if common & libc::ATTR_CMN_NAME != 0 {
        let reference: libc::attrreference_t = read(record, at)?;
        let start = at.checked_add_signed(reference.attr_dataoffset as isize)?;
        let raw = record.get(start..start + reference.attr_length as usize)?;
        // la longueur compte le zéro final
        let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
        name = OsString::from_vec(raw.to_vec());
        at += mem::size_of::<libc::attrreference_t>();
    }
    let mut dev = 0;
    if common & libc::ATTR_CMN_DEVID != 0 {
        dev = read::<i32>(record, at)? as u32 as u64;
        at += 4;
    }
    let mut objtype = 0;
    if common & libc::ATTR_CMN_OBJTYPE != 0 {
        objtype = read::<u32>(record, at)?;
        at += 4;
    }
    let mut modified = None;
    if common & libc::ATTR_CMN_MODTIME != 0 {
        let time: libc::timespec = read(record, at)?;
        modified = u64::try_from(time.tv_sec)
            .ok()
            .map(|secs| SystemTime::UNIX_EPOCH + Duration::new(secs, time.tv_nsec as u32));
        at += mem::size_of::<libc::timespec>();
    }
    let mut flags = 0;
    if common & libc::ATTR_CMN_FLAGS != 0 {
        flags = read::<u32>(record, at)?;
        at += 4;
    }
    let mut ino = 0;
    if common & libc::ATTR_CMN_FILEID != 0 {
        ino = read::<u64>(record, at)?;
        at += 8;
    }
    if common & ATTR_CMN_ERROR != 0 {
        // pareil que qd le lstat échoue : on zappe l'entrée
        if read::<u32>(record, at)? != 0 {
            return None;
        }
        at += 4;
    }
    let mut mount_point = false;
    if returned.dirattr & libc::ATTR_DIR_MOUNTSTATUS != 0 {
        mount_point = read::<u32>(record, at)? & libc::DIR_MNTSTATUS_MNTPOINT != 0;
        at += 4;
    }
    let (mut nlink, mut allocated, mut len) = (1, 0, 0);
    let file = returned.fileattr;
    if file & libc::ATTR_FILE_LINKCOUNT != 0 {
        nlink = read::<u32>(record, at)?;
        at += 4;
    }
    if file & libc::ATTR_FILE_ALLOCSIZE != 0 {
        allocated = read::<i64>(record, at)?.max(0) as u64;
        at += 8;
    }
    if file & libc::ATTR_FILE_DATALENGTH != 0 {
        len = read::<i64>(record, at)?.max(0) as u64;
    }

    let kind = match objtype {
        VDIR => EntryKind::Dir,
        VREG => EntryKind::File,
        _ => EntryKind::Other,
    };
    if mount_point || flags & SF_FIRMLINK != 0 {
        // pr un point de montage on a le devid du dossier recouvert et pr un firmlink
        // de / des attributs de façade, le lstat donne les vrais, c rare donc tkt le syscall en plus
        let meta = fs::symlink_metadata(dir.join(&name)).ok()?;
        dev = meta.dev();
        ino = meta.ino();
        flags = meta.st_flags();
        modified = meta.modified().ok();
    }
    Some(Entry {
        name,
        kind,
        dev,
        ino,
        nlink,
        allocated,
        len,
        flags,
        modified,
    })
}

/// Pr les systèmes de fichiers sans getattrlistbulk (partages réseau anciens…).
fn list_slow(dir: &Path) -> io::Result<Vec<Entry>> {
    Ok(fs::read_dir(dir)?
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            let kind = if meta.is_dir() {
                EntryKind::Dir
            } else if meta.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            Some(Entry {
                name: entry.file_name(),
                kind,
                dev: meta.dev(),
                ino: meta.ino(),
                nlink: meta.nlink() as u32,
                allocated: meta.blocks() * 512,
                len: meta.len(),
                flags: meta.st_flags(),
                modified: meta.modified().ok(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn compare(dir: &Path) {
        let mut fast = list_bulk(dir).unwrap();
        let mut slow = list_slow(dir).unwrap();
        fast.sort_by(|a, b| a.name.cmp(&b.name));
        slow.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(fast.len(), slow.len(), "{}", dir.display());
        for (f, s) in fast.iter().zip(&slow) {
            assert_eq!(f.name, s.name);
            assert_eq!(f.kind, s.kind, "{:?}", f.name);
            assert_eq!(f.dev, s.dev, "{:?}", f.name);
            assert_eq!(f.ino, s.ino, "{:?}", f.name);
            assert_eq!(f.flags, s.flags, "{:?}", f.name);
            assert_eq!(f.modified, s.modified, "{:?}", f.name);
            if f.kind == EntryKind::File {
                assert_eq!(f.nlink, s.nlink, "{:?}", f.name);
                assert_eq!(f.allocated, s.allocated, "{:?}", f.name);
                assert_eq!(f.len, s.len, "{:?}", f.name);
            }
        }
    }

    #[test]
    fn bulk_listing_matches_lstat() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        File::create(dir.path().join("héhé.txt"))
            .unwrap()
            .write_all(&[1; 70_000])
            .unwrap();
        File::create(dir.path().join("sparse"))
            .unwrap()
            .set_len(1 << 30)
            .unwrap();
        fs::hard_link(dir.path().join("héhé.txt"), dir.path().join("lien")).unwrap();
        std::os::unix::fs::symlink("/nowhere", dir.path().join("symlink")).unwrap();
        for i in 0..2_000 {
            File::create(dir.path().join(format!("f{i}"))).unwrap();
        }
        compare(dir.path());
    }

    #[test]
    #[ignore = "lit le vrai disque"]
    fn bulk_listing_matches_lstat_on_this_mac() {
        let home = std::env::var("HOME").unwrap();
        for dir in [
            "/",
            "/System/Volumes/Data",
            "/Applications",
            &home,
            &format!("{home}/Library"),
        ] {
            compare(Path::new(dir));
        }
    }

    #[test]
    fn missing_folder_is_an_error() {
        assert!(list_dir(Path::new("/nonexistent/levra")).is_err());
    }
}
