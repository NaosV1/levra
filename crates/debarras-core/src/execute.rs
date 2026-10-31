//! The only module that removes anything.
//!
//! Each path goes through [`guard::check_removable`] right before it is
//! touched. Items macOS does not let this process move (owned by root, app
//! bundles under App Management protection) are reported in
//! [`ExecutionReport::needs_authorization`] so the app can hand them to the
//! Finder, which asks for the password, instead of failing silently. Other
//! apps' containers fail with [`RemovalError::ProtectedAppData`]: the Finder
//! cannot move them either, only Full Disk Access opens them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::guard::{self, GuardError};
use crate::tools;
use crate::walk::ScanState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Move to the user's Trash: recoverable until the Trash is emptied.
    Trash { paths: Vec<PathBuf> },
    /// Remove for good. Reserved for data tools regenerate on their own.
    Delete { paths: Vec<PathBuf> },
    /// Let the owning tool clean up (`docker builder prune`, `simctl`…).
    Command {
        program: PathBuf,
        args: Vec<String>,
        /// The folder the tool shrinks, when it frees only part of what the
        /// scan estimated (`pnpm store prune` keeps what projects use): it
        /// is measured before and after, and the report gives the real
        /// difference.
        measure: Option<PathBuf>,
    },
    /// Nothing automatic; the finding's detail says what to do.
    Manual,
}

impl Action {
    pub fn paths(&self) -> &[PathBuf] {
        match self {
            Self::Trash { paths } | Self::Delete { paths } => paths,
            Self::Command { .. } | Self::Manual => &[],
        }
    }

    pub fn command_line(&self) -> Option<String> {
        match self {
            Self::Command { program, args, .. } => {
                let name = program
                    .file_name()
                    .map_or_else(|| program.display().to_string(), |n| n.to_string_lossy().into_owned());
                Some(
                    std::iter::once(name)
                        .chain(args.iter().cloned())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub target: String,
    pub reason: String,
}

/// A path macOS kept this process from moving; the app hands it to the
/// system, which may ask the user for permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pending {
    /// The item the path belongs to.
    pub id: String,
    pub path: PathBuf,
    /// What the item frees once all its pending paths are moved, carried
    /// by its first pending path (0 on the others).
    pub bytes: u64,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ExecutionReport {
    /// Identifiers of the items fully handled.
    pub completed: Vec<String>,
    /// Estimated bytes released by the completed items.
    pub freed_bytes: u64,
    pub needs_authorization: Vec<Pending>,
    pub failures: Vec<Failure>,
}

impl ExecutionReport {
    /// Runs `action` for the item `id` and records the outcome.
    pub fn run(&mut self, id: &str, bytes: u64, action: &Action, home: &Path) {
        let before = (self.needs_authorization.len(), self.failures.len());
        let mut freed = bytes;
        match action {
            Action::Trash { paths } => {
                for path in paths {
                    self.record(id, path, move_to_trash(path, home));
                }
            }
            Action::Delete { paths } => {
                for path in paths {
                    self.record(id, path, delete(path, home));
                }
            }
            Action::Command { program, args, measure } => {
                let size = |dir: &Path| ScanState::default().allocated_size(dir);
                let size_before = measure.as_deref().map(size);
                match run_command(program, args, home) {
                    Ok(()) => {
                        if let (Some(dir), Some(size_before)) = (measure, size_before) {
                            freed = size_before.saturating_sub(size(dir));
                        }
                    }
                    Err(reason) => self.failures.push(Failure {
                        target: action.command_line().unwrap_or_default(),
                        reason,
                    }),
                }
            }
            Action::Manual => return,
        }
        let (pending_before, failures_before) = before;
        if self.failures.len() != failures_before {
            return;
        }
        match self.needs_authorization.get_mut(pending_before) {
            Some(first_pending) => first_pending.bytes = freed,
            None => {
                self.completed.push(id.to_owned());
                self.freed_bytes += freed;
            }
        }
    }

    fn record(&mut self, id: &str, path: &Path, outcome: Result<(), RemovalError>) {
        match outcome {
            Ok(()) | Err(RemovalError::Guard(GuardError::Missing)) => {}
            Err(RemovalError::NeedsAuthorization) => self.needs_authorization.push(Pending {
                id: id.to_owned(),
                path: path.to_path_buf(),
                bytes: 0,
            }),
            Err(err) => self.failures.push(Failure {
                target: path.display().to_string(),
                reason: err.to_string(),
            }),
        }
    }
}

#[derive(Debug)]
pub enum RemovalError {
    Guard(GuardError),
    NeedsAuthorization,
    /// Another app's container: macOS protects app data from every other
    /// app, the Finder included, unless Débarras has Full Disk Access.
    ProtectedAppData,
    OtherVolume,
    Io(io::Error),
}

impl std::fmt::Display for RemovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Guard(err) => err.fmt(f),
            Self::NeedsAuthorization => write!(f, "autorisation macOS requise"),
            Self::ProtectedAppData => write!(
                f,
                "données d'une autre app, protégées par macOS : donne l'accès complet au disque à Débarras, puis relance"
            ),
            Self::OtherVolume => write!(f, "sur un autre volume que la corbeille"),
            Self::Io(err) => err.fmt(f),
        }
    }
}

impl From<io::Error> for RemovalError {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::PermissionDenied => Self::NeedsAuthorization,
            io::ErrorKind::CrossesDevices => Self::OtherVolume,
            _ => Self::Io(err),
        }
    }
}

/// Whether this process may move the item to the Trash without asking
/// macOS: that takes write access to its folder and, for a folder, to the
/// folder itself (its `..` entry changes).
pub fn can_move(path: &Path) -> bool {
    let is_dir = fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
    path.parent().is_some_and(writable) && (!is_dir || writable(path))
}

fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` is a valid NUL-terminated string that outlives the call.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// Containers of sandboxed apps, which macOS keeps from other apps.
const APP_DATA: [&str; 2] = ["Library/Containers", "Library/Group Containers"];

/// macOS refuses app data with `EPERM`, where files of another user give
/// `EACCES`. Only the latter is worth handing to the Finder.
fn removal_error(err: io::Error, path: &Path, home: &Path) -> RemovalError {
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let app_data = APP_DATA.iter().any(|rel| path.starts_with(home.join(rel)));
    if app_data && err.raw_os_error() == Some(libc::EPERM) {
        RemovalError::ProtectedAppData
    } else {
        err.into()
    }
}

/// Moves `path` into `~/.Trash`, renaming it if the Trash already holds an
/// item with the same name.
pub fn move_to_trash(path: &Path, home: &Path) -> Result<(), RemovalError> {
    let resolved = guard::check_removable(path, home).map_err(RemovalError::Guard)?;
    let trash = home.join(".Trash");
    fs::create_dir_all(&trash)?;
    let destination = free_name_in(&trash, &resolved)?;
    fs::rename(&resolved, destination).map_err(|err| removal_error(err, &resolved, home))?;
    Ok(())
}

fn free_name_in(dir: &Path, item: &Path) -> io::Result<PathBuf> {
    let name = item.file_name().unwrap_or_default().to_string_lossy();
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_owned(), format!(".{ext}")),
        _ => (name.clone().into_owned(), String::new()),
    };
    for attempt in 0..1000 {
        let candidate = if attempt == 0 {
            dir.join(name.as_ref())
        } else {
            dir.join(format!("{stem} {attempt}{extension}"))
        };
        if fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "corbeille saturée de doublons",
    ))
}

pub fn delete(path: &Path, home: &Path) -> Result<(), RemovalError> {
    let resolved = guard::check_removable(path, home).map_err(RemovalError::Guard)?;
    // `remove_dir_all` does not follow symlinks found inside the tree.
    let removed = if fs::symlink_metadata(&resolved)?.is_dir() {
        fs::remove_dir_all(&resolved)
    } else {
        fs::remove_file(&resolved)
    };
    removed.map_err(|err| removal_error(err, &resolved, home))
}

fn run_command(program: &Path, args: &[String], home: &Path) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(home)
        .env("PATH", tools::search_path(home))
        .stdin(Stdio::null())
        .output()
        .map_err(|err| err.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    // Some tools (pnpm) print their errors on standard output.
    let last_line = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned)
    };
    Err(last_line(&output.stderr)
        .or_else(|| last_line(&output.stdout))
        .unwrap_or_else(|| "échec".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, PathBuf) {
        let dir = crate::testing::tempdir();
        let path = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(path.join("Library/Caches/com.example.app")).unwrap();
        fs::write(path.join("Library/Caches/com.example.app/data"), b"cache").unwrap();
        (dir, path)
    }

    #[test]
    fn trashing_moves_the_item_into_the_trash() {
        let (_dir, home) = home();
        let cache = home.join("Library/Caches/com.example.app");
        move_to_trash(&cache, &home).unwrap();

        assert!(!cache.exists());
        assert!(home.join(".Trash/com.example.app/data").exists());
    }

    #[test]
    fn trashing_never_overwrites_an_existing_trash_item() {
        let (_dir, home) = home();
        fs::create_dir_all(home.join(".Trash/com.example.app")).unwrap();
        move_to_trash(&home.join("Library/Caches/com.example.app"), &home).unwrap();

        assert!(home.join(".Trash/com.example.app").exists());
        assert!(home.join(".Trash/com.example 1.app/data").exists());
    }

    #[test]
    fn deleting_removes_the_tree() {
        let (_dir, home) = home();
        let cache = home.join("Library/Caches/com.example.app");
        delete(&cache, &home).unwrap();
        assert!(!cache.exists());
    }

    #[test]
    fn guarded_paths_are_refused() {
        let (_dir, home) = home();
        let err = delete(&home.join("Library/Caches"), &home).unwrap_err();
        assert!(matches!(err, RemovalError::Guard(GuardError::Protected(_))));
        assert!(home.join("Library/Caches/com.example.app/data").exists());
    }

    #[test]
    fn unwritable_items_are_reported_as_needing_authorization() {
        let (_dir, home) = home();
        let caches = home.join("Library/Caches");
        fs::set_permissions(&caches, std::os::unix::fs::PermissionsExt::from_mode(0o555)).unwrap();

        let mut report = ExecutionReport::default();
        let action = Action::Delete {
            paths: vec![caches.join("com.example.app")],
        };
        report.run("cache", 10, &action, &home);
        fs::set_permissions(&caches, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        assert!(report.completed.is_empty());
        assert_eq!(report.freed_bytes, 0);
        assert_eq!(
            report.needs_authorization,
            vec![Pending {
                id: "cache".to_owned(),
                path: caches.join("com.example.app"),
                bytes: 10,
            }]
        );
    }

    #[test]
    fn an_item_with_several_locked_paths_carries_its_size_once() {
        let (_dir, home) = home();
        let caches = home.join("Library/Caches");
        fs::create_dir_all(caches.join("com.example.other")).unwrap();
        fs::set_permissions(&caches, std::os::unix::fs::PermissionsExt::from_mode(0o555)).unwrap();

        let mut report = ExecutionReport::default();
        let action = Action::Trash {
            paths: vec![caches.join("com.example.app"), caches.join("com.example.other")],
        };
        report.run("app", 70, &action, &home);
        fs::set_permissions(&caches, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let sizes: Vec<(&str, u64)> = report
            .needs_authorization
            .iter()
            .map(|p| (p.id.as_str(), p.bytes))
            .collect();
        assert_eq!(sizes, [("app", 70), ("app", 0)]);
        assert!(report.completed.is_empty());
    }

    #[test]
    fn app_data_refusals_are_not_sent_to_the_finder() {
        let (_dir, home) = home();
        let eperm = || io::Error::from_raw_os_error(libc::EPERM);
        let eacces = || io::Error::from_raw_os_error(libc::EACCES);
        let container = home.join("Library/Containers/com.example.app");
        let group = home.join("Library/Group Containers/TEAM.com.example");
        let cache = home.join("Library/Caches/com.example.app");

        assert!(matches!(
            removal_error(eperm(), &container, &home),
            RemovalError::ProtectedAppData
        ));
        assert!(matches!(
            removal_error(eperm(), &group, &home),
            RemovalError::ProtectedAppData
        ));
        assert!(matches!(
            removal_error(eacces(), &container, &home),
            RemovalError::NeedsAuthorization
        ));
        assert!(matches!(
            removal_error(eperm(), &cache, &home),
            RemovalError::NeedsAuthorization
        ));
    }

    #[test]
    fn already_missing_items_count_as_done() {
        let (_dir, home) = home();
        let mut report = ExecutionReport::default();
        let action = Action::Trash {
            paths: vec![home.join("Library/Caches/gone")],
        };
        report.run("gone", 5, &action, &home);
        assert_eq!(report.completed, vec!["gone".to_owned()]);
    }

    #[test]
    fn a_partial_cleanup_reports_what_it_really_freed() {
        let (_dir, home) = home();
        let store = home.join("Library/pnpm/store");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("unused"), vec![1u8; 1_000_000]).unwrap();
        fs::write(store.join("used"), vec![1u8; 1_000_000]).unwrap();

        // Like `pnpm store prune`: only part of the folder goes.
        let mut report = ExecutionReport::default();
        let action = Action::Command {
            program: PathBuf::from("/bin/rm"),
            args: vec![store.join("unused").display().to_string()],
            measure: Some(store.clone()),
        };
        report.run("prune", 2_000_000, &action, &home);

        assert_eq!(report.completed, vec!["prune".to_owned()]);
        assert!(
            (1_000_000..1_100_000).contains(&report.freed_bytes),
            "{} instead of the 2 MB estimate",
            report.freed_bytes
        );
    }

    #[test]
    fn commands_run_from_the_home_folder() {
        // An app opened from the Finder runs in `/`, which is read-only:
        // pnpm failed there trying to write next to itself.
        let (_dir, home) = home();
        let mut report = ExecutionReport::default();
        let action = Action::Command {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), format!("test \"$(pwd -P)\" = '{}'", home.display())],
            measure: None,
        };
        report.run("pwd", 1, &action, &home);
        assert_eq!(report.completed, vec!["pwd".to_owned()], "{:?}", report.failures);
    }

    #[test]
    fn an_error_printed_on_standard_output_is_reported() {
        let (_dir, home) = home();
        let mut report = ExecutionReport::default();
        let action = Action::Command {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "echo ' EROFS  read-only file system'; exit 1".into()],
            measure: None,
        };
        report.run("pnpm", 1, &action, &home);
        assert_eq!(report.failures[0].reason, "EROFS  read-only file system");
    }

    #[test]
    fn failed_commands_are_reported_with_their_last_error_line() {
        let (_dir, home) = home();
        let mut report = ExecutionReport::default();
        let action = Action::Command {
            program: PathBuf::from("/bin/ls"),
            args: vec!["/nonexistent/debarras".into()],
            measure: None,
        };
        report.run("ls", 1, &action, &home);

        assert!(report.completed.is_empty());
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].target, "ls /nonexistent/debarras");
        assert!(
            report.failures[0].reason.contains("No such file"),
            "{:?}",
            report.failures
        );
    }

    #[test]
    fn manual_actions_are_never_marked_completed() {
        let (_dir, home) = home();
        let mut report = ExecutionReport::default();
        report.run("manual", 100, &Action::Manual, &home);
        assert!(report.completed.is_empty());
    }
}
