//! Last line of defence before anything is removed.
//!
//! Every path handed to [`crate::execute`] is checked here, whatever
//! produced it. A path is removable only if, once its parent is resolved
//! (so a symlinked ancestor cannot redirect the removal elsewhere), it sits
//! strictly below one of a few allowed roots and deep enough not to be a
//! container folder the system or the user relies on.

use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardError {
    NotAbsolute,
    UnresolvedComponents,
    Missing,
    Protected(PathBuf),
    OutsideAllowedRoots(PathBuf),
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAbsolute => write!(f, "chemin relatif refusé"),
            Self::UnresolvedComponents => write!(f, "chemin contenant « . » ou « .. » refusé"),
            Self::Missing => write!(f, "élément introuvable"),
            Self::Protected(path) => write!(f, "{} est protégé", path.display()),
            Self::OutsideAllowedRoots(path) => {
                write!(f, "{} est hors des emplacements autorisés", path.display())
            }
        }
    }
}

impl std::error::Error for GuardError {}

/// Allowed roots and how many components a path needs below each one.
/// `~/x` (depth 1) is never removable: those are the user's own folders.
fn allowed_roots(home: &Path) -> [(PathBuf, usize); 6] {
    [
        (home.to_path_buf(), 2),
        (PathBuf::from("/Applications"), 1),
        (PathBuf::from("/Library"), 2),
        (PathBuf::from("/opt/homebrew"), 2),
        (PathBuf::from("/private/var/folders"), 4),
        (PathBuf::from("/private/var/db/receipts"), 1),
    ]
}

/// Folders that must survive even though they are deep enough.
fn protected(home: &Path) -> Vec<PathBuf> {
    let library = home.join("Library");
    [
        "Application Support",
        "Caches",
        "Containers",
        "Group Containers",
        "Preferences",
        "LaunchAgents",
        "Logs",
        "Mobile Documents",
        "Keychains",
        "Mail",
        "Messages",
        "Photos",
    ]
    .iter()
    .map(|name| library.join(name))
    .chain(
        [
            "Library/Application Support",
            "Library/Caches",
            "Library/LaunchAgents",
            "Library/LaunchDaemons",
            "Library/Preferences",
            "Library/PrivilegedHelperTools",
        ]
        .iter()
        .map(|rel| Path::new("/").join(rel)),
    )
    .collect()
}

/// Returns the resolved path to act on, or why it must not be touched.
pub fn check_removable(path: &Path, home: &Path) -> Result<PathBuf, GuardError> {
    if !path.is_absolute() {
        return Err(GuardError::NotAbsolute);
    }
    if path
        .components()
        .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        return Err(GuardError::UnresolvedComponents);
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(GuardError::Protected(path.to_path_buf()));
    };
    if fs::symlink_metadata(path).is_err() {
        return Err(GuardError::Missing);
    }

    // The entry itself is not resolved: removing a symlink removes the link.
    let resolved = fs::canonicalize(parent).map_err(|_| GuardError::Missing)?.join(name);
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());

    if protected(&home).contains(&resolved) {
        return Err(GuardError::Protected(resolved));
    }
    let allowed = allowed_roots(&home).into_iter().any(|(root, depth)| {
        resolved
            .strip_prefix(&root)
            .is_ok_and(|rest| rest.components().count() >= depth)
    });
    if allowed {
        Ok(resolved)
    } else {
        Err(GuardError::OutsideAllowedRoots(resolved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Home {
        dir: tempfile::TempDir,
    }

    impl Home {
        fn new() -> Self {
            let dir = crate::testing::tempdir();
            for rel in [
                "Library/Caches/com.example.app",
                "Library/Application Support",
                "Developer/app/target",
            ] {
                fs::create_dir_all(dir.path().join(rel)).unwrap();
            }
            Self { dir }
        }

        fn path(&self) -> PathBuf {
            fs::canonicalize(self.dir.path()).unwrap()
        }
    }

    #[test]
    fn accepts_items_deep_inside_home() {
        let home = Home::new();
        let target = home.path().join("Developer/app/target");
        assert_eq!(check_removable(&target, &home.path()), Ok(target));
        let cache = home.path().join("Library/Caches/com.example.app");
        assert!(check_removable(&cache, &home.path()).is_ok());
    }

    #[test]
    fn rejects_home_and_its_direct_children() {
        let home = Home::new();
        assert!(check_removable(&home.path(), &home.path()).is_err());
        assert!(matches!(
            check_removable(&home.path().join("Developer"), &home.path()),
            Err(GuardError::OutsideAllowedRoots(_))
        ));
    }

    #[test]
    fn rejects_library_container_folders() {
        let home = Home::new();
        for rel in ["Library/Caches", "Library/Application Support"] {
            assert!(
                matches!(
                    check_removable(&home.path().join(rel), &home.path()),
                    Err(GuardError::Protected(_))
                ),
                "{rel} must be protected"
            );
        }
    }

    #[test]
    fn rejects_relative_and_dotted_paths() {
        let home = Home::new();
        assert_eq!(
            check_removable(Path::new("Library/Caches/x"), &home.path()),
            Err(GuardError::NotAbsolute)
        );
        let dotted = home.path().join("Developer/app/../app/target");
        assert_eq!(
            check_removable(&dotted, &home.path()),
            Err(GuardError::UnresolvedComponents)
        );
    }

    #[test]
    fn rejects_system_locations() {
        let home = Home::new();
        for path in ["/", "/System/Library", "/usr/bin", "/Applications", "/Library/Caches"] {
            assert!(check_removable(Path::new(path), &home.path()).is_err(), "{path}");
        }
    }

    #[test]
    fn rejects_missing_items() {
        let home = Home::new();
        let missing = home.path().join("Developer/app/gone");
        assert_eq!(check_removable(&missing, &home.path()), Err(GuardError::Missing));
    }

    #[test]
    fn a_symlinked_ancestor_cannot_escape_home() {
        let home = Home::new();
        let elsewhere = crate::testing::tempdir();
        fs::create_dir_all(elsewhere.path().join("precious/data")).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), home.path().join("Developer/escape")).unwrap();

        let through_link = home.path().join("Developer/escape/precious/data");
        assert!(matches!(
            check_removable(&through_link, &home.path()),
            Err(GuardError::OutsideAllowedRoots(_))
        ));
    }

    #[test]
    fn a_symlink_itself_can_be_removed() {
        let home = Home::new();
        let link = home.path().join("Developer/app/link");
        std::os::unix::fs::symlink("/System", &link).unwrap();
        assert_eq!(check_removable(&link, &home.path()), Ok(link));
    }
}
