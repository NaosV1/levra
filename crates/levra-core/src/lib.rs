//! Levra core: measures what takes space on a Mac, finds what can be
//! reclaimed and duplicated, and plans complete application uninstalls.
//!
//! Scanning only reads. Every removal goes through [`execute`], which
//! re-checks each path with [`guard::check_removable`] right before acting.

pub mod apps;
pub mod catalog;
pub mod debloat;
pub mod duplicates;
pub mod execute;
pub mod explore;
pub mod guard;
pub mod launchd;
pub mod leftovers;
mod listing;
pub mod memory;
pub mod time;
pub mod tools;
pub mod walk;

pub use apps::AppInfo;
pub use catalog::{Category, Finding, Safety, ScanOptions, ScanReport};
pub use duplicates::{DuplicateFile, DuplicateGroup, DuplicateOptions, DuplicateReport};
pub use execute::{Action, ExecutionReport, Failure};
pub use explore::{Node, NodeId, NodeKind, Tree};
pub use leftovers::{Leftover, LeftoverKind, MatchReason, Roots, UninstallPlan};
pub use walk::{CancelToken, Progress, ScanState};

#[cfg(test)]
pub(crate) mod testing {
    /// A temporary folder outside every root the guard allows. The system
    /// temp folder cannot be used for that: it lives in `/private/var/folders`,
    /// which is one of those roots.
    pub fn tempdir() -> tempfile::TempDir {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-homes");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::tempdir_in(base).unwrap()
    }
}
