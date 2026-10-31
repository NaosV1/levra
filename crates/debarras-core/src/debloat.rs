//! What got installed without being asked for, and what can go.
//!
//! - launchd jobs from other publishers (updaters, helpers), which apps
//!   install for themselves and often leave behind. Each is attributed to
//!   the installed apps of its publisher. One that starts a program that no
//!   longer exists does nothing and is proposed for removal; one that
//!   belongs to an installed app stays, since the app would break or put it
//!   back.
//! - Apple's apps. Those that ship with macOS (Stocks, News, Tips…) live on
//!   the sealed, read-only system volume and come back with every update:
//!   removing them would take turning System Integrity Protection off,
//!   which Débarras never does, so they are only counted. The Apple apps
//!   new Macs get in `/Applications` uninstall like any other app.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::apps::{self, AppInfo};
use crate::execute::{self, Action, ExecutionReport, Failure};
use crate::launchd::{self, Job, Scope};
use crate::leftovers::vendor_prefix;
use crate::walk::ScanState;

/// Apps Apple installs on new Macs outside the system volume.
const REMOVABLE_APPLE_APPS: &[&str] = &[
    "com.apple.garageband10",
    "com.apple.iMovieApp",
    "com.apple.iWork.Keynote",
    "com.apple.iWork.Numbers",
    "com.apple.iWork.Pages",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "seconds", rename_all = "snake_case")]
pub enum Schedule {
    /// Each time launchd loads it: at login for agents, at startup for daemons.
    AtLoad,
    /// All the time: launchd restarts it when it stops.
    KeepAlive,
    /// Every this many seconds.
    Interval(u64),
    /// At set times.
    Calendar,
    /// Only when another program asks for it.
    OnDemand,
    /// The file is readable only by root.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "apps", rename_all = "snake_case")]
pub enum Owner {
    /// The installed apps that name it or contain its program, or failing
    /// that the installed apps of its publisher; sorted by name.
    Apps(Vec<AppInfo>),
    /// A `brew services` job.
    Homebrew,
    /// No installed app from its publisher, but it may still start something.
    NoApp,
    /// The program it starts is gone, or the file starts nothing.
    Broken,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundItem {
    pub id: String,
    /// The job's property list.
    pub path: PathBuf,
    /// Its label, or the file name when the file says none.
    pub label: String,
    pub scope: Scope,
    pub program: Option<PathBuf>,
    pub schedule: Schedule,
    pub owner: Owner,
    /// Loaded in the user's session. `None` for daemons: only root can tell.
    pub loaded: Option<bool>,
    /// A daemon's privileged helper, removed along with it.
    pub helper: Option<PathBuf>,
    pub bytes: u64,
    pub needs_authorization: bool,
    /// Proposed for removal: it starts nothing.
    pub selected: bool,
}

impl BackgroundItem {
    /// Jobs of installed apps and Homebrew services are managed by their
    /// owner; removing one here would break it or be undone.
    pub fn removable(&self) -> bool {
        matches!(self.owner, Owner::NoApp | Owner::Broken)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DebloatReport {
    pub background: Vec<BackgroundItem>,
    /// Apps that ship with macOS, on the sealed system volume.
    pub system_apps: usize,
    pub system_apps_bytes: u64,
    /// Apple apps installed in the application folders, which can go.
    pub removable_apple_apps: Vec<AppInfo>,
}

/// `system` is `/` in production; `installed` is the list of apps the
/// user sees, used to attribute each job.
pub fn scan(state: &ScanState, home: &Path, system: &Path, installed: &[AppInfo]) -> DebloatReport {
    let system_apps = system.join("System/Applications");
    DebloatReport {
        background: background_items(state, home, system, installed),
        system_apps: apps::bundles_in(&system_apps, 1).len(),
        system_apps_bytes: state.allocated_size(&system_apps),
        removable_apple_apps: installed
            .iter()
            .filter(|app| REMOVABLE_APPLE_APPS.contains(&app.bundle_id.as_str()))
            .cloned()
            .collect(),
    }
}

fn background_items(state: &ScanState, home: &Path, system: &Path, installed: &[AppInfo]) -> Vec<BackgroundItem> {
    let uid = launchd::session_user(home);
    let files: Vec<(PathBuf, Scope)> = launchd::folders(home, system)
        .into_iter()
        .flat_map(|(folder, scope)| job_files(&folder).into_iter().map(move |path| (path, scope)))
        .collect();
    let mut items: Vec<BackgroundItem> = files
        .into_par_iter()
        .filter_map(|(path, scope)| {
            let job = Job::read(&path);
            let label = job
                .as_ref()
                .and_then(|job| job.label.clone())
                .or_else(|| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))?;
            if label.starts_with("com.apple.") {
                return None;
            }
            let owner = owner(&label, job.as_ref(), installed);
            // A file without a label is never loaded, whatever its name says.
            let has_label = job.as_ref().is_none_or(|job| job.label.is_some());
            let loaded = (scope != Scope::Daemon)
                .then(|| has_label && uid.is_some_and(|uid| launchd::agent_loaded(&label, uid, home)));
            let helper = (scope == Scope::Daemon)
                .then(|| launchd::helper_tool(system, &label))
                .flatten();
            let needs_authorization =
                !execute::can_move(&path) || helper.as_deref().is_some_and(|h| !execute::can_move(h));
            Some(BackgroundItem {
                id: format!("background:{}", path.display()),
                bytes: state.allocated_size(&path) + helper.as_deref().map_or(0, |h| state.allocated_size(h)),
                program: job.as_ref().and_then(Job::program).map(PathBuf::from),
                schedule: job.as_ref().map_or(Schedule::Unknown, schedule),
                selected: owner == Owner::Broken,
                path,
                label,
                scope,
                owner,
                loaded,
                helper,
                needs_authorization,
            })
        })
        .collect();
    items.sort_by_key(|item| {
        (
            rank(&item.owner),
            first_app_name(&item.owner),
            item.label.to_lowercase(),
        )
    });
    items
}

fn job_files(folder: &Path) -> Vec<PathBuf> {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "plist"))
        .filter(|path| fs::symlink_metadata(path).is_ok_and(|m| m.is_file()))
        .collect()
}

/// The apps that name the job or contain its program, else the apps of its
/// publisher.
fn owner(label: &str, job: Option<&Job>, installed: &[AppInfo]) -> Owner {
    let program = job.and_then(Job::program).map(Path::new);
    let associated = job.map(Job::associated_ids).unwrap_or_default();
    let mut apps: Vec<AppInfo> = installed
        .iter()
        .filter(|app| associated.contains(&app.bundle_id.as_str()) || program.is_some_and(|p| p.starts_with(&app.path)))
        .cloned()
        .collect();
    if apps.is_empty()
        && let Some(vendor) = vendor_prefix(label)
    {
        apps = installed
            .iter()
            .filter(|app| {
                let id = app.bundle_id.to_lowercase();
                id.starts_with(&vendor) || Some(id.as_str()) == vendor.strip_suffix('.')
            })
            .cloned()
            .collect();
    }
    if !apps.is_empty() {
        apps.sort_by_key(|app| app.name.to_lowercase());
        return Owner::Apps(apps);
    }
    if label.starts_with("homebrew.mxcl.") {
        return Owner::Homebrew;
    }
    let Some(job) = job else {
        return Owner::NoApp;
    };
    match job.program().map(Path::new) {
        // Relative programs resolve inside an app bundle Débarras cannot see.
        Some(program) if !program.is_absolute() || !is_missing(program) => Owner::NoApp,
        _ => Owner::Broken,
    }
}

/// Only a definite "not found": a program behind a folder this process may
/// not read is not called missing.
fn is_missing(path: &Path) -> bool {
    fs::metadata(path).is_err_and(|err| err.kind() == io::ErrorKind::NotFound)
}

fn schedule(job: &Job) -> Schedule {
    if job.keeps_alive() {
        Schedule::KeepAlive
    } else if let Some(seconds) = job.start_interval() {
        Schedule::Interval(seconds)
    } else if job.runs_on_calendar() {
        Schedule::Calendar
    } else if job.runs_at_load() {
        Schedule::AtLoad
    } else {
        Schedule::OnDemand
    }
}

/// Display order: what can go first.
fn rank(owner: &Owner) -> u8 {
    match owner {
        Owner::Broken => 0,
        Owner::NoApp => 1,
        Owner::Homebrew => 2,
        Owner::Apps(_) => 3,
    }
}

fn first_app_name(owner: &Owner) -> String {
    match owner {
        Owner::Apps(apps) => apps.first().map(|app| app.name.to_lowercase()).unwrap_or_default(),
        _ => String::new(),
    }
}

impl DebloatReport {
    /// Moves the chosen jobs (and daemons' helpers) to the Trash, stopping
    /// agents of the user's session first. Jobs that are not removable are
    /// refused.
    pub fn execute(&self, ids: &[String], home: &Path) -> ExecutionReport {
        let mut report = ExecutionReport::default();
        let uid = launchd::session_user(home);
        for item in self.background.iter().filter(|item| ids.contains(&item.id)) {
            if !item.removable() {
                report.failures.push(Failure {
                    target: item.label.clone(),
                    reason: "Appartient à une app installée ou à Homebrew : désinstalle-la plutôt.".to_owned(),
                });
                continue;
            }
            if let (Some(true), Some(uid), false) = (item.loaded, uid, item.needs_authorization) {
                launchd::stop_agent(&item.label, uid, home);
            }
            let paths = std::iter::once(item.path.clone()).chain(item.helper.clone()).collect();
            report.run(&item.id, item.bytes, &Action::Trash { paths }, home);
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::fake_app;

    struct Machine {
        _dir: tempfile::TempDir,
        home: PathBuf,
        /// Inside the home folder, so that the guard lets tests remove from it.
        system: PathBuf,
    }

    impl Machine {
        fn new() -> Self {
            let dir = crate::testing::tempdir();
            let home = fs::canonicalize(dir.path()).unwrap();
            let system = home.join("system");
            for (folder, _) in launchd::folders(&home, &system) {
                fs::create_dir_all(folder).unwrap();
            }
            Self {
                _dir: dir,
                home,
                system,
            }
        }

        fn job(&self, scope: Scope, file: &str, body: &str) -> PathBuf {
            let folder = launchd::folders(&self.home, &self.system)
                .into_iter()
                .find(|(_, s)| *s == scope)
                .unwrap()
                .0;
            let path = folder.join(file);
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>{body}</dict></plist>"#
            );
            fs::write(&path, xml).unwrap();
            path
        }

        fn program(&self, rel: &str) -> PathBuf {
            let path = self.home.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"bin").unwrap();
            path
        }

        fn app(&self, name: &str, bundle_id: &str) -> AppInfo {
            let path = fake_app(&self.home.join("Applications"), name, bundle_id);
            apps::read_bundle(&path).unwrap()
        }

        fn scan(&self, installed: &[AppInfo]) -> DebloatReport {
            scan(&ScanState::default(), &self.home, &self.system, installed)
        }
    }

    fn agent(label: &str, program: &Path, extra: &str) -> String {
        format!(
            "<key>Label</key><string>{label}</string>
             <key>ProgramArguments</key><array><string>{}</string></array>{extra}",
            program.display()
        )
    }

    fn item<'a>(report: &'a DebloatReport, label: &str) -> &'a BackgroundItem {
        report.background.iter().find(|i| i.label == label).unwrap()
    }

    fn owner_names(item: &BackgroundItem) -> Vec<String> {
        match &item.owner {
            Owner::Apps(apps) => apps.iter().map(|a| a.name.clone()).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn jobs_that_start_nothing_are_proposed_for_removal() {
        let machine = Machine::new();
        let gone = machine.home.join("Library/Application Support/Gone/updater");
        machine.job(
            Scope::UserAgent,
            "com.gone.updater.plist",
            &agent("com.gone.updater", &gone, ""),
        );
        // Google leaves empty files where its old updater's agents were.
        machine.job(Scope::UserAgent, "com.google.keystone.agent.plist", "");

        let report = machine.scan(&[]);
        for label in ["com.gone.updater", "com.google.keystone.agent"] {
            let job = item(&report, label);
            assert_eq!(job.owner, Owner::Broken, "{label}");
            assert!(job.selected && job.removable(), "{label}");
            assert_eq!(job.loaded, Some(false), "{label}");
        }
    }

    #[test]
    fn jobs_of_installed_apps_are_attributed_and_kept() {
        let machine = Machine::new();
        let docker = machine.app("Docker", "com.docker.docker");
        let word = machine.app("Microsoft Word", "com.microsoft.Word");
        let excel = machine.app("Microsoft Excel", "com.microsoft.Excel");
        let installed = [docker.clone(), word, excel];

        let inside = docker.path.join("Contents/MacOS/Docker");
        machine.job(
            Scope::UserAgent,
            "com.example.starter.plist",
            &agent("com.example.starter", &inside, ""),
        );
        let socket = machine.program("bin/docker-socket");
        machine.job(
            Scope::Daemon,
            "com.docker.socket.plist",
            &agent(
                "com.docker.socket",
                &socket,
                "<key>AssociatedBundleIdentifiers</key><array><string>com.docker.docker</string></array>",
            ),
        );
        let updater = machine.program("bin/msupdate");
        machine.job(
            Scope::Agent,
            "com.microsoft.update.agent.plist",
            &agent("com.microsoft.update.agent", &updater, ""),
        );

        let report = machine.scan(&installed);
        assert_eq!(owner_names(item(&report, "com.example.starter")), ["Docker"]);
        assert_eq!(owner_names(item(&report, "com.docker.socket")), ["Docker"]);
        assert_eq!(
            owner_names(item(&report, "com.microsoft.update.agent")),
            ["Microsoft Excel", "Microsoft Word"]
        );
        assert!(report.background.iter().all(|i| !i.selected && !i.removable()));
        assert_eq!(
            item(&report, "com.docker.socket").loaded,
            None,
            "daemons cannot be queried"
        );
    }

    #[test]
    fn a_job_without_an_app_is_shown_but_never_proposed() {
        let machine = Machine::new();
        let updater = machine.program("Library/Application Support/Google/GoogleUpdater/updater");
        machine.job(
            Scope::UserAgent,
            "com.google.GoogleUpdater.wake.plist",
            &agent(
                "com.google.GoogleUpdater.wake",
                &updater,
                "<key>StartInterval</key><integer>3600</integer>",
            ),
        );
        // Readable only by root: what it starts is unknown.
        fs::write(
            machine.system.join("Library/LaunchDaemons/com.vpn.helper.plist"),
            b"\0\0",
        )
        .unwrap();

        let report = machine.scan(&[]);
        let google = item(&report, "com.google.GoogleUpdater.wake");
        assert_eq!(google.owner, Owner::NoApp);
        assert_eq!(google.schedule, Schedule::Interval(3600));
        assert!(!google.selected && google.removable());

        let vpn = item(&report, "com.vpn.helper");
        assert_eq!((vpn.owner.clone(), vpn.schedule), (Owner::NoApp, Schedule::Unknown));
        assert!(!vpn.selected);
    }

    #[test]
    fn homebrew_services_and_apple_jobs_are_left_alone() {
        let machine = Machine::new();
        let missing = machine.home.join("missing");
        machine.job(
            Scope::UserAgent,
            "homebrew.mxcl.postgresql@16.plist",
            &agent("homebrew.mxcl.postgresql@16", &missing, ""),
        );
        machine.job(
            Scope::Agent,
            "com.apple.example.plist",
            &agent("com.apple.example", &missing, ""),
        );
        fs::write(machine.home.join("Library/LaunchAgents/.DS_Store"), b"").unwrap();

        let report = machine.scan(&[]);
        assert_eq!(report.background.len(), 1);
        assert_eq!(report.background[0].owner, Owner::Homebrew);
        assert!(!report.background[0].removable());
    }

    #[test]
    fn reads_when_a_job_runs() {
        let machine = Machine::new();
        let program = machine.program("bin/tool");
        let cases = [
            ("a.keepalive", "<key>KeepAlive</key><true/>", Schedule::KeepAlive),
            (
                "a.interval",
                "<key>StartInterval</key><integer>600</integer>",
                Schedule::Interval(600),
            ),
            (
                "a.calendar",
                "<key>StartCalendarInterval</key><dict><key>Hour</key><integer>3</integer></dict>",
                Schedule::Calendar,
            ),
            ("a.atload", "<key>RunAtLoad</key><true/>", Schedule::AtLoad),
            ("a.ondemand", "<key>MachServices</key><dict/>", Schedule::OnDemand),
        ];
        for (label, extra, _) in &cases {
            machine.job(
                Scope::UserAgent,
                &format!("{label}.plist"),
                &agent(label, &program, extra),
            );
        }

        let report = machine.scan(&[]);
        for (label, _, expected) in cases {
            assert_eq!(item(&report, label).schedule, expected, "{label}");
        }
    }

    #[test]
    fn a_daemon_goes_with_its_privileged_helper() {
        let machine = Machine::new();
        let helper = machine.system.join("Library/PrivilegedHelperTools/com.gone.helper");
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        fs::write(&helper, b"bin").unwrap();
        machine.job(
            Scope::Daemon,
            "com.gone.helper.plist",
            &agent("com.gone.helper", &machine.home.join("missing"), ""),
        );

        let report = machine.scan(&[]);
        let daemon = item(&report, "com.gone.helper");
        assert_eq!(daemon.helper.as_deref(), Some(helper.as_path()));

        let result = report.execute(std::slice::from_ref(&daemon.id), &machine.home);
        assert_eq!(result.completed, std::slice::from_ref(&daemon.id));
        assert!(!daemon.path.exists() && !helper.exists());
        assert!(machine.home.join(".Trash/com.gone.helper.plist").exists());
        assert!(machine.home.join(".Trash/com.gone.helper").exists());
    }

    #[test]
    fn refuses_to_remove_a_job_of_an_installed_app() {
        let machine = Machine::new();
        let app = machine.app("Steam", "com.valvesoftware.steam");
        let program = machine.program("Library/Application Support/Steam/steamclean");
        let path = machine.job(
            Scope::UserAgent,
            "com.valvesoftware.steamclean.plist",
            &agent("com.valvesoftware.steamclean", &program, ""),
        );

        let report = machine.scan(std::slice::from_ref(&app));
        let id = item(&report, "com.valvesoftware.steamclean").id.clone();
        let result = report.execute(&[id], &machine.home);
        assert!(result.completed.is_empty());
        assert_eq!(result.failures.len(), 1);
        assert!(path.exists());
    }

    #[test]
    fn counts_system_apps_and_lists_the_apple_apps_that_can_go() {
        let machine = Machine::new();
        let system_apps = machine.system.join("System/Applications");
        fake_app(&system_apps, "Stocks", "com.apple.stocks");
        fake_app(&system_apps.join("Utilities"), "Terminal", "com.apple.Terminal");
        let garageband = machine.app("GarageBand", "com.apple.garageband10");
        let xcode = machine.app("Xcode", "com.apple.dt.Xcode");

        let report = machine.scan(&[garageband, xcode]);
        assert_eq!(report.system_apps, 2);
        assert!(report.system_apps_bytes > 0);
        let names: Vec<&str> = report.removable_apple_apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["GarageBand"]);
    }
}
