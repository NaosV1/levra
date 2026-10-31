//! What can be reclaimed on this Mac, found by independent detectors.
//!
//! Each detector knows one family of reclaimable data (build artifacts,
//! package caches, Xcode, Android, Docker, personal files) and returns
//! [`Finding`]s carrying the exact [`Action`] that would reclaim them.
//! Detectors only read: nothing is removed until [`ScanReport::execute`].

mod android;
mod caches;
mod docker;
mod projects;
mod user_files;
mod xcode;

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::execute::{Action, ExecutionReport};
use crate::walk::ScanState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Safety {
    /// Regenerated automatically; the only cost is time or bandwidth.
    Safe,
    /// Reversible, but worth a look (reinstall needed, state lost).
    Review,
    /// May hold data that cannot be recreated.
    Careful,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    ProjectBuilds,
    GitWorktrees,
    PackageCaches,
    AppCaches,
    Xcode,
    Android,
    Docker,
    PersonalFiles,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Stable across scans of an unchanged disk, so a CLI user can select
    /// findings by id from a previous listing.
    pub id: String,
    pub category: Category,
    pub title: String,
    pub detail: String,
    pub bytes: u64,
    pub safety: Safety,
    pub action: Action,
    /// Last use of the owner (project, simulator, runtime…), Unix seconds.
    pub last_used: Option<u64>,
}

/// A project touched within this delay is probably being worked on.
const ACTIVE_PROJECT_SECS: u64 = 30 * 86_400;

impl Finding {
    /// Whether to select it by default: safe to remove, and not the build
    /// output of a project in use (rebuilding it would only slow the user).
    pub fn recommended(&self, now: u64) -> bool {
        let active_project = self.category == Category::ProjectBuilds
            && self
                .last_used
                .is_some_and(|used| now.saturating_sub(used) < ACTIVE_PROJECT_SECS);
        self.safety == Safety::Safe && !active_project
    }
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub home: PathBuf,
    /// Folders searched for development projects.
    pub project_roots: Vec<PathBuf>,
    /// Findings smaller than this are not worth showing.
    pub min_bytes: u64,
}

impl ScanOptions {
    pub fn for_home(home: &Path) -> Self {
        let project_roots = [
            "Developer",
            "Projects",
            "Code",
            "code",
            "dev",
            "src",
            "workspace",
            "IdeaProjects",
            "StudioProjects",
            "AndroidStudioProjects",
            "Documents/GitHub",
            "Documents/Projects",
        ]
        .iter()
        .map(|rel| home.join(rel))
        .filter(|path| path.is_dir())
        .collect();
        Self {
            home: home.to_path_buf(),
            project_roots,
            min_bytes: 50 * 1_000_000,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub findings: Vec<Finding>,
    /// Folders macOS refused to read: grant Full Disk Access to see them.
    pub denied: Vec<PathBuf>,
}

type Detector = fn(&ScanOptions, &ScanState) -> Vec<Finding>;

/// The parts of a scan, one per detector, which all run at once. As each
/// ends, its bit is set in [`crate::walk::Progress::steps_done`], in this
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Projects,
    Caches,
    Xcode,
    Android,
    Docker,
    PersonalFiles,
}

const DETECTORS: [(Step, Detector); 6] = [
    (Step::Projects, projects::detect),
    (Step::Caches, caches::detect),
    (Step::Xcode, xcode::detect),
    (Step::Android, android::detect),
    (Step::Docker, docker::detect),
    (Step::PersonalFiles, user_files::detect),
];

/// The steps of [`scan`], in the order of their bits.
pub fn steps() -> Vec<Step> {
    DETECTORS.iter().map(|(step, _)| *step).collect()
}

pub fn scan(options: &ScanOptions, state: &ScanState) -> ScanReport {
    run(&DETECTORS, options, state)
}

fn run(detectors: &[(Step, Detector)], options: &ScanOptions, state: &ScanState) -> ScanReport {
    let mut findings: Vec<Finding> = detectors
        .par_iter()
        .enumerate()
        .flat_map(|(index, (_, detect))| {
            let found = detect(options, state);
            state.progress.finish_step(index);
            found
        })
        .filter(|f| f.bytes >= options.min_bytes)
        .collect();
    findings.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.id.cmp(&b.id)));
    ScanReport {
        findings,
        denied: state.denied(),
    }
}

impl ScanReport {
    /// Executes the findings whose id is in `ids`, largest first.
    pub fn execute(&self, ids: &[String], home: &Path) -> ExecutionReport {
        let mut report = ExecutionReport::default();
        for finding in self.findings.iter().filter(|f| ids.contains(&f.id)) {
            report.run(&finding.id, finding.bytes, &finding.action, home);
        }
        report
    }
}

/// Builds a finding whose action removes `path` for good, sized on disk.
fn deletable(
    state: &ScanState,
    category: Category,
    path: PathBuf,
    title: String,
    detail: String,
    safety: Safety,
) -> Finding {
    Finding {
        id: format!("path:{}", path.display()),
        category,
        title,
        detail,
        bytes: state.allocated_size(&path),
        safety,
        action: Action::Delete { paths: vec![path] },
        last_used: None,
    }
}

/// Directories directly inside `dir`, sorted by name; empty if unreadable.
fn subdirectories(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    dirs
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(last_used: Option<u64>, safety: Safety) -> Finding {
        Finding {
            id: "x".into(),
            category: Category::ProjectBuilds,
            title: String::new(),
            detail: String::new(),
            bytes: 1,
            safety,
            action: Action::Manual,
            last_used,
        }
    }

    #[test]
    fn each_detector_marks_its_step_done() {
        fn nothing(_: &ScanOptions, _: &ScanState) -> Vec<Finding> {
            Vec::new()
        }
        let detectors = [(Step::Xcode, nothing as Detector), (Step::Docker, nothing as Detector)];
        let state = ScanState::default();
        assert_eq!(state.progress.steps_done(), 0);

        run(&detectors, &ScanOptions::for_home(Path::new("/nonexistent")), &state);
        assert_eq!(state.progress.steps_done(), 0b11);
        assert_eq!(steps().len(), DETECTORS.len());
    }

    #[test]
    fn builds_of_active_projects_are_not_recommended() {
        let now = 100 * 86_400;
        assert!(!build(Some(now - 3 * 86_400), Safety::Safe).recommended(now));
        assert!(build(Some(now - 60 * 86_400), Safety::Safe).recommended(now));
        assert!(build(None, Safety::Safe).recommended(now));
    }

    #[test]
    fn only_safe_findings_are_recommended() {
        assert!(!build(None, Safety::Review).recommended(0));
        let cache = Finding {
            category: Category::PackageCaches,
            last_used: Some(0),
            ..build(None, Safety::Safe)
        };
        assert!(cache.recommended(0), "recent use only matters for projects");
    }
}
