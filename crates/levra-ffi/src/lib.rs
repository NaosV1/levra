//! Swift bindings for the Levra core, generated with UniFFI.
//!
//! Only display data and identifiers cross the boundary. The app picks
//! findings or leftovers by id; the paths and commands behind them never
//! leave Rust, so the core alone decides what gets removed.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

use levra_core as core;

uniffi::setup_scaffolding!();

#[derive(Debug, uniffi::Error)]
pub enum LevraError {
    MissingHome,
    Io {
        message: String,
    },
    /// The bundle at this path is no longer the app that was listed.
    AppChanged {
        message: String,
    },
    /// The core refused to stop a process.
    TerminateRefused {
        message: String,
    },
}

impl fmt::Display for LevraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHome => write!(f, "dossier personnel introuvable"),
            Self::Io { message } | Self::AppChanged { message } | Self::TerminateRefused { message } => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for LevraError {}

fn home() -> Result<PathBuf, LevraError> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(LevraError::MissingHome)
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn now() -> u64 {
    core::time::unix_seconds(SystemTime::now())
}

// MARK: Progress and cancellation

#[uniffi::export(with_foreign)]
pub trait ProgressListener: Send + Sync {
    /// `steps_done` has bit `n` set once step `n` is over, for scans made
    /// of steps (see [`scan_steps`]); 0 for the others.
    fn on_progress(&self, files: u64, bytes: u64, steps_done: u32);
}

#[derive(uniffi::Object, Default)]
pub struct CancelHandle {
    token: core::CancelToken,
}

#[uniffi::export]
impl CancelHandle {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn cancel(&self) {
        self.token.cancel();
    }
}

/// Runs `work`, calling the listener about every 150 ms meanwhile.
fn with_progress<T>(state: &core::ScanState, listener: &Arc<dyn ProgressListener>, work: impl FnOnce() -> T) -> T {
    let done = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                let progress = &state.progress;
                listener.on_progress(progress.files(), progress.bytes(), progress.steps_done());
                thread::park_timeout(Duration::from_millis(150));
            }
        });
        let result = work();
        done.store(true, Ordering::Relaxed);
        result
    })
}

// MARK: Results of removals

#[derive(uniffi::Record)]
pub struct FailureInfo {
    pub target: String,
    pub reason: String,
}

/// A path macOS kept the core from moving; `bytes` is what its item frees
/// once all of the item's pending paths are moved (0 after the first).
#[derive(uniffi::Record)]
pub struct PendingRemoval {
    pub id: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(uniffi::Record)]
pub struct ExecutionResult {
    pub completed: Vec<String>,
    pub freed_bytes: u64,
    /// What macOS kept this process from moving; the app hands it to the
    /// Finder, which asks the user for the password.
    pub needs_authorization: Vec<PendingRemoval>,
    pub failures: Vec<FailureInfo>,
}

impl From<core::ExecutionReport> for ExecutionResult {
    fn from(report: core::ExecutionReport) -> Self {
        Self {
            completed: report.completed,
            freed_bytes: report.freed_bytes,
            needs_authorization: report
                .needs_authorization
                .into_iter()
                .map(|pending| PendingRemoval {
                    id: pending.id,
                    path: text(&pending.path),
                    bytes: pending.bytes,
                })
                .collect(),
            failures: report
                .failures
                .into_iter()
                .map(|f| FailureInfo {
                    target: f.target,
                    reason: f.reason,
                })
                .collect(),
        }
    }
}

// MARK: Cleanup

/// What the cleanup scan looks at; all parts run at once.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum ScanStep {
    Projects,
    Caches,
    Xcode,
    Android,
    Docker,
    PersonalFiles,
}

/// The steps of [`scan_cleanup`], in the order of the `steps_done` bits.
#[uniffi::export]
pub fn scan_steps() -> Vec<ScanStep> {
    core::catalog::steps()
        .into_iter()
        .map(|step| match step {
            core::catalog::Step::Projects => ScanStep::Projects,
            core::catalog::Step::Caches => ScanStep::Caches,
            core::catalog::Step::Xcode => ScanStep::Xcode,
            core::catalog::Step::Android => ScanStep::Android,
            core::catalog::Step::Docker => ScanStep::Docker,
            core::catalog::Step::PersonalFiles => ScanStep::PersonalFiles,
        })
        .collect()
}

#[derive(uniffi::Enum, Clone, Copy)]
pub enum Safety {
    Safe,
    Review,
    Careful,
}

#[derive(uniffi::Enum, Clone, Copy)]
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

#[derive(uniffi::Enum, Clone, Copy)]
pub enum ActionKind {
    Trash,
    Delete,
    Command,
    Manual,
}

#[derive(uniffi::Record)]
pub struct FindingInfo {
    pub id: String,
    pub category: Category,
    pub title: String,
    pub detail: String,
    pub bytes: u64,
    pub safety: Safety,
    pub action: ActionKind,
    /// The exact command run for command actions, shown to the user.
    pub command: Option<String>,
    pub paths: Vec<String>,
    pub last_used: Option<u64>,
    pub recommended: bool,
}

impl FindingInfo {
    fn new(finding: &core::Finding, now: u64) -> Self {
        Self {
            id: finding.id.clone(),
            category: match finding.category {
                core::Category::ProjectBuilds => Category::ProjectBuilds,
                core::Category::GitWorktrees => Category::GitWorktrees,
                core::Category::PackageCaches => Category::PackageCaches,
                core::Category::AppCaches => Category::AppCaches,
                core::Category::Xcode => Category::Xcode,
                core::Category::Android => Category::Android,
                core::Category::Docker => Category::Docker,
                core::Category::PersonalFiles => Category::PersonalFiles,
            },
            title: finding.title.clone(),
            detail: finding.detail.clone(),
            bytes: finding.bytes,
            safety: match finding.safety {
                core::Safety::Safe => Safety::Safe,
                core::Safety::Review => Safety::Review,
                core::Safety::Careful => Safety::Careful,
            },
            action: match finding.action {
                core::Action::Trash { .. } => ActionKind::Trash,
                core::Action::Delete { .. } => ActionKind::Delete,
                core::Action::Command { .. } => ActionKind::Command,
                core::Action::Manual => ActionKind::Manual,
            },
            command: finding.action.command_line(),
            paths: finding.action.paths().iter().map(|p| text(p)).collect(),
            last_used: finding.last_used,
            recommended: finding.recommended(now),
        }
    }
}

#[derive(uniffi::Object)]
pub struct CleanupReport {
    home: PathBuf,
    report: core::ScanReport,
}

#[uniffi::export]
impl CleanupReport {
    pub fn findings(&self) -> Vec<FindingInfo> {
        let now = now();
        self.report.findings.iter().map(|f| FindingInfo::new(f, now)).collect()
    }

    /// Folders macOS refused to read: they need Full Disk Access.
    pub fn denied_paths(&self) -> Vec<String> {
        self.report.denied.iter().map(|p| text(p)).collect()
    }

    pub fn execute(&self, ids: Vec<String>) -> ExecutionResult {
        self.report.execute(&ids, &self.home).into()
    }
}

#[uniffi::export]
pub fn scan_cleanup(
    cancel: Arc<CancelHandle>,
    listener: Arc<dyn ProgressListener>,
) -> Result<Arc<CleanupReport>, LevraError> {
    let home = home()?;
    let state = core::ScanState::new(cancel.token.clone());
    let report = with_progress(&state, &listener, || {
        core::catalog::scan(&core::ScanOptions::for_home(&home), &state)
    });
    Ok(Arc::new(CleanupReport { home, report }))
}

// MARK: Explorer

#[derive(uniffi::Enum, Clone, Copy)]
pub enum NodeKind {
    Directory,
    File,
    SmallFiles,
}

#[derive(uniffi::Record)]
pub struct NodeInfo {
    pub id: u64,
    pub name: String,
    pub kind: NodeKind,
    pub bytes: u64,
    pub items: u64,
    pub modified: Option<u64>,
    pub has_children: bool,
    /// Chemin complet, calculé une fois ici plutôt qu'à chq rendu côté Swift.
    pub path: String,
    /// Whether the core's guard lets it go to the Trash: macOS's own files
    /// and the user's top-level folders do not.
    pub removable: bool,
}

#[derive(uniffi::Object)]
pub struct ExploreTree {
    home: PathBuf,
    tree: core::Tree,
    denied: Vec<PathBuf>,
}

impl ExploreTree {
    fn info(&self, id: core::NodeId) -> Option<NodeInfo> {
        let node = self.tree.node(id)?;
        let path = self.tree.path(id)?;
        Some(NodeInfo {
            id: id as u64,
            name: node.name.clone(),
            kind: match node.kind {
                core::NodeKind::Directory => NodeKind::Directory,
                core::NodeKind::File => NodeKind::File,
                core::NodeKind::SmallFiles => NodeKind::SmallFiles,
            },
            bytes: node.bytes,
            items: node.items,
            modified: node.modified.map(core::time::unix_seconds),
            has_children: !self.tree.children(id).is_empty(),
            removable: node.kind != core::NodeKind::SmallFiles
                && core::guard::check_removable(&path, &self.home).is_ok(),
            path: text(&path),
        })
    }

    fn node_id(id: u64) -> core::NodeId {
        usize::try_from(id).unwrap_or(usize::MAX)
    }
}

#[uniffi::export]
impl ExploreTree {
    pub fn root(&self) -> Option<NodeInfo> {
        self.info(core::Tree::ROOT)
    }

    pub fn children(&self, id: u64) -> Vec<NodeInfo> {
        self.tree
            .children(Self::node_id(id))
            .iter()
            .filter_map(|&child| self.info(child))
            .collect()
    }

    pub fn parent(&self, id: u64) -> Option<NodeInfo> {
        self.tree.parent(Self::node_id(id)).and_then(|parent| self.info(parent))
    }

    pub fn path(&self, id: u64) -> Option<String> {
        self.tree.path(Self::node_id(id)).map(|p| text(&p))
    }

    /// Largest files, optionally only those unmodified for `untouched_days`.
    pub fn largest_files(&self, limit: u32, min_bytes: u64, untouched_days: u32) -> Vec<NodeInfo> {
        let before =
            (untouched_days > 0).then(|| SystemTime::now() - Duration::from_secs(u64::from(untouched_days) * 86_400));
        self.tree
            .largest_files(limit as usize, min_bytes, before)
            .into_iter()
            .filter_map(|id| self.info(id))
            .collect()
    }

    pub fn denied_paths(&self) -> Vec<String> {
        self.denied.iter().map(|p| text(p)).collect()
    }
}

#[uniffi::export]
pub fn explore(
    path: String,
    cancel: Arc<CancelHandle>,
    listener: Arc<dyn ProgressListener>,
) -> Result<Arc<ExploreTree>, LevraError> {
    let home = home()?;
    let state = core::ScanState::new(cancel.token.clone());
    let tree = with_progress(&state, &listener, || {
        core::explore::explore(&state, Path::new(&path), core::explore::DEFAULT_MIN_FILE_BYTES)
    })
    .map_err(|err| LevraError::Io {
        message: err.to_string(),
    })?;
    Ok(Arc::new(ExploreTree {
        home,
        tree,
        denied: state.denied(),
    }))
}

/// Size of the Trash, `None` when macOS keeps it closed (no Full Disk
/// Access): Levra can still have the Finder empty it.
#[uniffi::export]
pub fn trash_size() -> Result<Option<u64>, LevraError> {
    let trash = home()?.join(".Trash");
    if std::fs::read_dir(&trash).is_err() {
        return Ok(None);
    }
    Ok(Some(core::ScanState::default().allocated_size(&trash)))
}

/// Moves items the user picked (in the explorer) to the Trash.
#[uniffi::export]
pub fn move_to_trash(paths: Vec<String>) -> Result<ExecutionResult, LevraError> {
    let home = home()?;
    let mut report = core::ExecutionReport::default();
    for path in paths {
        let bytes = core::ScanState::default().allocated_size(Path::new(&path));
        let action = core::Action::Trash {
            paths: vec![PathBuf::from(&path)],
        };
        report.run(&path, bytes, &action, &home);
    }
    Ok(report.into())
}

// MARK: Duplicates

#[derive(uniffi::Record)]
pub struct DuplicateFileInfo {
    pub path: String,
    pub modified: Option<u64>,
    /// Bytes removing this copy frees (less than its size for APFS clones).
    pub reclaimable: u64,
    /// The copy proposed to keep.
    pub keep: bool,
}

#[derive(uniffi::Record)]
pub struct DuplicateGroupInfo {
    pub id: String,
    pub size: u64,
    pub reclaimable: u64,
    pub files: Vec<DuplicateFileInfo>,
}

#[derive(uniffi::Object)]
pub struct DuplicateScan {
    home: PathBuf,
    report: core::DuplicateReport,
}

#[uniffi::export]
impl DuplicateScan {
    pub fn groups(&self) -> Vec<DuplicateGroupInfo> {
        self.report
            .groups
            .iter()
            .map(|group| DuplicateGroupInfo {
                id: group.id.clone(),
                size: group.size,
                reclaimable: group.reclaimable(),
                files: group
                    .files
                    .iter()
                    .map(|file| DuplicateFileInfo {
                        path: text(&file.path),
                        modified: file.modified,
                        reclaimable: file.reclaimable,
                        keep: file.keep,
                    })
                    .collect(),
            })
            .collect()
    }

    /// Groups of APFS clones left out because removing them frees nothing.
    pub fn clone_groups(&self) -> u64 {
        self.report.clone_groups as u64
    }

    pub fn denied_paths(&self) -> Vec<String> {
        self.report.denied.iter().map(|p| text(p)).collect()
    }

    /// Moves the chosen copies to the Trash. The core refuses to empty a
    /// group or to remove a copy changed since the scan.
    pub fn execute(&self, paths: Vec<String>) -> ExecutionResult {
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        self.report.execute(&paths, &self.home).into()
    }
}

#[uniffi::export]
pub fn find_duplicates(
    cancel: Arc<CancelHandle>,
    listener: Arc<dyn ProgressListener>,
) -> Result<Arc<DuplicateScan>, LevraError> {
    let home = home()?;
    let state = core::ScanState::new(cancel.token.clone());
    let report = with_progress(&state, &listener, || {
        core::duplicates::find(&core::DuplicateOptions::for_home(&home), &state)
    });
    Ok(Arc::new(DuplicateScan { home, report }))
}

// MARK: Memory

#[derive(uniffi::Enum, Clone, Copy)]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}

#[derive(uniffi::Record)]
pub struct MemoryStatsInfo {
    pub total: u64,
    pub used: u64,
    pub app: u64,
    pub wired: u64,
    pub compressed: u64,
    pub cached_files: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub pressure: MemoryPressure,
}

impl From<core::memory::MemoryStats> for MemoryStatsInfo {
    fn from(stats: core::memory::MemoryStats) -> Self {
        Self {
            total: stats.total,
            used: stats.used,
            app: stats.app,
            wired: stats.wired,
            compressed: stats.compressed,
            cached_files: stats.cached_files,
            swap_used: stats.swap_used,
            swap_total: stats.swap_total,
            pressure: match stats.pressure {
                core::memory::Pressure::Normal => MemoryPressure::Normal,
                core::memory::Pressure::Warning => MemoryPressure::Warning,
                core::memory::Pressure::Critical => MemoryPressure::Critical,
            },
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy)]
pub enum ProcessKind {
    App,
    Tool,
    VirtualMachine,
    System,
}

/// Not `ProcessInfo`, which Swift code knows as Foundation's class.
#[derive(uniffi::Record)]
pub struct RunningProcess {
    pub pid: i32,
    pub name: String,
    pub path: String,
    pub footprint: u64,
}

#[derive(uniffi::Record)]
pub struct ProcessGroupInfo {
    pub path: String,
    pub bundle_id: Option<String>,
    pub name: String,
    pub kind: ProcessKind,
    pub footprint: u64,
    pub processes: Vec<RunningProcess>,
    pub quittable: bool,
}

#[derive(uniffi::Record)]
pub struct MemorySnapshotInfo {
    pub stats: MemoryStatsInfo,
    pub groups: Vec<ProcessGroupInfo>,
    pub unmeasured: u32,
}

fn io_error(err: std::io::Error) -> LevraError {
    LevraError::Io {
        message: err.to_string(),
    }
}

/// System figures only: cheap enough for the menu bar and alerts.
#[uniffi::export]
pub fn memory_stats() -> Result<MemoryStatsInfo, LevraError> {
    core::memory::stats().map(MemoryStatsInfo::from).map_err(io_error)
}

#[uniffi::export]
pub fn memory_snapshot() -> Result<MemorySnapshotInfo, LevraError> {
    let snapshot = core::memory::snapshot().map_err(io_error)?;
    Ok(MemorySnapshotInfo {
        stats: snapshot.stats.into(),
        groups: snapshot
            .groups
            .into_iter()
            .map(|group| ProcessGroupInfo {
                path: text(&group.path),
                bundle_id: group.bundle_id,
                name: group.name,
                kind: match group.kind {
                    core::memory::ProcessKind::App => ProcessKind::App,
                    core::memory::ProcessKind::Tool => ProcessKind::Tool,
                    core::memory::ProcessKind::VirtualMachine => ProcessKind::VirtualMachine,
                    core::memory::ProcessKind::System => ProcessKind::System,
                },
                footprint: group.footprint,
                processes: group
                    .processes
                    .into_iter()
                    .map(|process| RunningProcess {
                        pid: process.pid,
                        name: process.name,
                        path: text(&process.path),
                        footprint: process.footprint,
                    })
                    .collect(),
                quittable: group.quittable,
            })
            .collect(),
        unmeasured: u32::try_from(snapshot.unmeasured).unwrap_or(u32::MAX),
    })
}

/// Stops a command-line program. Apps are quit through AppKit instead, so
/// they can save their work.
#[uniffi::export]
pub fn terminate_process(pid: i32, path: String, force: bool) -> Result<(), LevraError> {
    core::memory::terminate(pid, Path::new(&path), force).map_err(|err| LevraError::TerminateRefused {
        message: err.to_string(),
    })
}

// MARK: Applications

#[derive(uniffi::Record, Clone)]
pub struct AppRecord {
    pub path: String,
    pub name: String,
    pub bundle_id: String,
    pub version: Option<String>,
    pub executable: Option<String>,
    pub team_id: Option<String>,
    pub bytes: u64,
    pub last_used: Option<u64>,
}

impl From<core::AppInfo> for AppRecord {
    fn from(app: core::AppInfo) -> Self {
        Self {
            path: text(&app.path),
            name: app.name,
            bundle_id: app.bundle_id,
            version: app.version,
            executable: app.executable,
            team_id: app.team_id,
            bytes: app.bytes,
            last_used: app.last_used,
        }
    }
}

impl From<AppRecord> for core::AppInfo {
    fn from(app: AppRecord) -> Self {
        Self {
            path: PathBuf::from(app.path),
            name: app.name,
            bundle_id: app.bundle_id,
            version: app.version,
            executable: app.executable,
            team_id: app.team_id,
            bytes: app.bytes,
            last_used: app.last_used,
        }
    }
}

#[uniffi::export]
pub fn list_apps() -> Result<Vec<AppRecord>, LevraError> {
    let home = home()?;
    Ok(core::apps::list_apps(&core::ScanState::default(), &home)
        .into_iter()
        .map(AppRecord::from)
        .collect())
}

#[uniffi::export]
pub fn inspect_app(path: String) -> Result<AppRecord, LevraError> {
    let home = home()?;
    core::apps::inspect_app(&core::ScanState::default(), Path::new(&path), &home)
        .map(AppRecord::from)
        .map_err(|err| LevraError::Io {
            message: err.to_string(),
        })
}

// MARK: Uninstaller

#[derive(uniffi::Enum, Clone, Copy)]
pub enum LeftoverKind {
    Application,
    Support,
    Cache,
    Preferences,
    Container,
    GroupContainer,
    SavedState,
    WebData,
    Logs,
    Scripts,
    LaunchAgent,
    LaunchDaemon,
    HelperTool,
    Temporary,
    Receipt,
}

#[derive(uniffi::Enum, Clone, Copy)]
pub enum MatchReason {
    BundleId,
    Name,
    Team,
    Vendor,
    Similar,
}

#[derive(uniffi::Record)]
pub struct LeftoverInfo {
    pub id: String,
    pub path: String,
    pub kind: LeftoverKind,
    pub reason: MatchReason,
    pub bytes: u64,
    pub needs_authorization: bool,
    /// Another app's container macOS keeps closed until Levra has Full
    /// Disk Access.
    pub needs_full_disk_access: bool,
    pub selected: bool,
    pub note: Option<String>,
}

impl From<&core::Leftover> for LeftoverInfo {
    fn from(item: &core::Leftover) -> Self {
        Self {
            id: item.id.clone(),
            path: text(&item.path),
            kind: match item.kind {
                core::LeftoverKind::Application => LeftoverKind::Application,
                core::LeftoverKind::Support => LeftoverKind::Support,
                core::LeftoverKind::Cache => LeftoverKind::Cache,
                core::LeftoverKind::Preferences => LeftoverKind::Preferences,
                core::LeftoverKind::Container => LeftoverKind::Container,
                core::LeftoverKind::GroupContainer => LeftoverKind::GroupContainer,
                core::LeftoverKind::SavedState => LeftoverKind::SavedState,
                core::LeftoverKind::WebData => LeftoverKind::WebData,
                core::LeftoverKind::Logs => LeftoverKind::Logs,
                core::LeftoverKind::Scripts => LeftoverKind::Scripts,
                core::LeftoverKind::LaunchAgent => LeftoverKind::LaunchAgent,
                core::LeftoverKind::LaunchDaemon => LeftoverKind::LaunchDaemon,
                core::LeftoverKind::HelperTool => LeftoverKind::HelperTool,
                core::LeftoverKind::Temporary => LeftoverKind::Temporary,
                core::LeftoverKind::Receipt => LeftoverKind::Receipt,
            },
            reason: match item.reason {
                core::MatchReason::BundleId => MatchReason::BundleId,
                core::MatchReason::Name => MatchReason::Name,
                core::MatchReason::Team => MatchReason::Team,
                core::MatchReason::Vendor => MatchReason::Vendor,
                core::MatchReason::Similar => MatchReason::Similar,
            },
            bytes: item.bytes,
            needs_authorization: item.needs_authorization,
            needs_full_disk_access: item.needs_full_disk_access,
            selected: item.selected,
            note: item.note.clone(),
        }
    }
}

#[derive(uniffi::Object)]
pub struct UninstallSession {
    home: PathBuf,
    plan: core::UninstallPlan,
}

#[uniffi::export]
impl UninstallSession {
    pub fn app(&self) -> AppRecord {
        self.plan.app.clone().into()
    }

    /// False when the app was already moved away (to the Trash).
    pub fn app_present(&self) -> bool {
        self.plan.app_present
    }

    pub fn items(&self) -> Vec<LeftoverInfo> {
        self.plan.items.iter().map(LeftoverInfo::from).collect()
    }

    pub fn brew_cask(&self) -> Option<String> {
        self.plan.brew_cask.clone()
    }

    pub fn is_app_running(&self) -> bool {
        self.plan.app_present && core::apps::is_running(&self.plan.app, &self.home)
    }

    pub fn execute(&self, ids: Vec<String>, use_brew: bool) -> ExecutionResult {
        self.plan.execute(&ids, use_brew, &self.home).into()
    }
}

/// `installed` is the app list the user sees, so that files shared with
/// another installed app are recognised without listing apps again.
#[uniffi::export]
pub fn plan_uninstall(app: AppRecord, installed: Vec<AppRecord>) -> Result<Arc<UninstallSession>, LevraError> {
    let home = home()?;
    let app: core::AppInfo = app.into();
    // The record comes from an earlier listing: if a bundle is still at
    // that path, it must be the same app.
    if let Ok(current) = core::apps::read_bundle(&app.path)
        && current.bundle_id != app.bundle_id
    {
        return Err(LevraError::AppChanged {
            message: format!("{} n'est plus {}", app.path.display(), app.name),
        });
    }
    let installed: Vec<core::AppInfo> = installed.into_iter().map(core::AppInfo::from).collect();
    let brew_cask = core::leftovers::brew_cask_for(&app, &home);
    let plan = core::leftovers::plan(
        &core::ScanState::default(),
        &core::Roots::system(&home),
        &app,
        &installed,
        brew_cask,
    );
    Ok(Arc::new(UninstallSession { home, plan }))
}

// MARK: Debloat

#[derive(uniffi::Enum, Clone, Copy)]
pub enum JobScope {
    UserAgent,
    Agent,
    Daemon,
}

#[derive(uniffi::Enum, Clone, Copy)]
pub enum JobSchedule {
    AtLoad,
    KeepAlive,
    Interval { seconds: u64 },
    Calendar,
    OnDemand,
    Unknown,
}

#[derive(uniffi::Enum, Clone, Copy)]
pub enum JobOwner {
    Apps,
    Homebrew,
    NoApp,
    Broken,
}

#[derive(uniffi::Record)]
pub struct BackgroundItemInfo {
    pub id: String,
    pub label: String,
    pub path: String,
    pub program: Option<String>,
    pub scope: JobScope,
    pub schedule: JobSchedule,
    pub owner: JobOwner,
    /// The installed apps it belongs to, when `owner` is `Apps`.
    pub apps: Vec<AppRecord>,
    /// `None` for daemons: only root can tell.
    pub loaded: Option<bool>,
    pub has_helper: bool,
    pub needs_authorization: bool,
    pub removable: bool,
    pub selected: bool,
}

impl From<&core::debloat::BackgroundItem> for BackgroundItemInfo {
    fn from(item: &core::debloat::BackgroundItem) -> Self {
        use core::debloat::{Owner, Schedule};
        let (owner, apps) = match &item.owner {
            Owner::Apps(apps) => (JobOwner::Apps, apps.iter().cloned().map(AppRecord::from).collect()),
            Owner::Homebrew => (JobOwner::Homebrew, Vec::new()),
            Owner::NoApp => (JobOwner::NoApp, Vec::new()),
            Owner::Broken => (JobOwner::Broken, Vec::new()),
        };
        Self {
            id: item.id.clone(),
            label: item.label.clone(),
            path: text(&item.path),
            program: item.program.as_deref().map(text),
            scope: match item.scope {
                core::launchd::Scope::UserAgent => JobScope::UserAgent,
                core::launchd::Scope::Agent => JobScope::Agent,
                core::launchd::Scope::Daemon => JobScope::Daemon,
            },
            schedule: match item.schedule {
                Schedule::AtLoad => JobSchedule::AtLoad,
                Schedule::KeepAlive => JobSchedule::KeepAlive,
                Schedule::Interval(seconds) => JobSchedule::Interval { seconds },
                Schedule::Calendar => JobSchedule::Calendar,
                Schedule::OnDemand => JobSchedule::OnDemand,
                Schedule::Unknown => JobSchedule::Unknown,
            },
            owner,
            apps,
            loaded: item.loaded,
            has_helper: item.helper.is_some(),
            needs_authorization: item.needs_authorization,
            removable: item.removable(),
            selected: item.selected,
        }
    }
}

#[derive(uniffi::Object)]
pub struct DebloatSession {
    home: PathBuf,
    report: core::debloat::DebloatReport,
}

#[uniffi::export]
impl DebloatSession {
    pub fn background_items(&self) -> Vec<BackgroundItemInfo> {
        self.report.background.iter().map(BackgroundItemInfo::from).collect()
    }

    /// Apps that ship with macOS, on the sealed system volume.
    pub fn system_app_count(&self) -> u64 {
        self.report.system_apps as u64
    }

    pub fn system_app_bytes(&self) -> u64 {
        self.report.system_apps_bytes
    }

    /// Apple apps in the application folders, which uninstall like any app.
    pub fn removable_apple_apps(&self) -> Vec<AppRecord> {
        self.report
            .removable_apple_apps
            .iter()
            .cloned()
            .map(AppRecord::from)
            .collect()
    }

    pub fn execute(&self, ids: Vec<String>) -> ExecutionResult {
        self.report.execute(&ids, &self.home).into()
    }
}

/// `installed` is the app list the user sees: each job is attributed to
/// those apps.
#[uniffi::export]
pub fn scan_debloat(installed: Vec<AppRecord>) -> Result<Arc<DebloatSession>, LevraError> {
    let home = home()?;
    let installed: Vec<core::AppInfo> = installed.into_iter().map(core::AppInfo::from).collect();
    let report = core::debloat::scan(&core::ScanState::default(), &home, Path::new("/"), &installed);
    Ok(Arc::new(DebloatSession { home, report }))
}
