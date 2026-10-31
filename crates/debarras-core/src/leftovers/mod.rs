//! Everything an application leaves behind, for a complete uninstall.
//!
//! Files are attributed to the app by, from most to least certain:
//! its exact bundle id (`com.docker.docker`, `com.docker.docker.plist`,
//! `com.docker.docker.helper`), its exact name in the folders where apps
//! use their name, its signing team for group containers, its vendor
//! prefix (`com.docker.*`) when no other installed app shares it, or a
//! name that starts like the app's (`Docker Desktop`). The first two are
//! preselected, team matches only when the team has no other app here;
//! anything another installed app may still use is left unselected.

mod brew;
mod launchd;
mod matcher;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rayon::prelude::*;
use serde::Serialize;

use crate::apps::{self, AppInfo};
use crate::execute::{self, Action, ExecutionReport, Failure};
use crate::tools;
use crate::walk::ScanState;

pub use brew::brew_cask_for;
use matcher::Matcher;
pub(crate) use matcher::vendor_prefix;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
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

/// Ordered from most to least certain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchReason {
    BundleId,
    Name,
    Team,
    Vendor,
    Similar,
}

#[derive(Debug, Clone, Serialize)]
pub struct Leftover {
    pub id: String,
    pub path: PathBuf,
    pub kind: LeftoverKind,
    pub reason: MatchReason,
    pub bytes: u64,
    /// macOS will not let this process move it: owned by root, or an app
    /// bundle under App Management protection. The Finder moves it instead,
    /// asking for the password.
    pub needs_authorization: bool,
    /// Another app's container that macOS keeps from Débarras until it has
    /// Full Disk Access: its size is incomplete and it cannot be removed.
    pub needs_full_disk_access: bool,
    pub selected: bool,
    pub note: Option<String>,
    /// launchd job to stop before its plist goes away.
    launchd_label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallPlan {
    pub app: AppInfo,
    /// False when the bundle is already gone (dragged to the Trash).
    pub app_present: bool,
    pub items: Vec<Leftover>,
    /// Homebrew cask that installed the app. `brew uninstall --zap` also
    /// removes the files its maintainers listed for this app.
    pub brew_cask: Option<String>,
}

/// Where to look. Production uses [`Roots::system`]; tests point every
/// root into a temporary folder.
#[derive(Debug, Clone)]
pub struct Roots {
    pub home: PathBuf,
    /// Root of the system-wide `Library` and receipts (`/` in production).
    pub system: PathBuf,
    /// Per-user cache and temp folders under `/private/var/folders`.
    pub user_temp: Vec<PathBuf>,
}

impl Roots {
    pub fn system(home: &Path) -> Self {
        let user_temp = ["DARWIN_USER_CACHE_DIR", "DARWIN_USER_TEMP_DIR"]
            .iter()
            .filter_map(|key| tools::output(Path::new("/usr/bin/getconf"), &[key], home, Duration::from_secs(5)))
            .map(|dir| PathBuf::from(dir.trim()))
            .filter(|dir| dir.is_dir())
            .collect();
        Self {
            home: home.to_path_buf(),
            system: PathBuf::from("/"),
            user_temp,
        }
    }
}

enum Base {
    Home,
    System,
}

struct Location {
    base: Base,
    rel: &'static str,
    kind: LeftoverKind,
    /// Whether apps name their folder there after themselves.
    by_name: bool,
}

const fn at(base: Base, rel: &'static str, kind: LeftoverKind, by_name: bool) -> Location {
    Location {
        base,
        rel,
        kind,
        by_name,
    }
}

#[rustfmt::skip]
const LOCATIONS: &[Location] = &[
    at(Base::Home,   "Library/Application Support",     LeftoverKind::Support,     true),
    at(Base::Home,   "Library/Caches",                  LeftoverKind::Cache,       true),
    at(Base::Home,   "Library/Preferences",             LeftoverKind::Preferences, false),
    at(Base::Home,   "Library/Preferences/ByHost",      LeftoverKind::Preferences, false),
    at(Base::Home,   "Library/Containers",              LeftoverKind::Container,   false),
    at(Base::Home,   "Library/Saved Application State", LeftoverKind::SavedState,  false),
    at(Base::Home,   "Library/HTTPStorages",            LeftoverKind::WebData,     false),
    at(Base::Home,   "Library/WebKit",                  LeftoverKind::WebData,     false),
    at(Base::Home,   "Library/Cookies",                 LeftoverKind::WebData,     false),
    at(Base::Home,   "Library/Logs",                    LeftoverKind::Logs,        true),
    at(Base::Home,   "Library/Application Scripts",     LeftoverKind::Scripts,     false),
    at(Base::System, "Library/Application Support",     LeftoverKind::Support,     true),
    at(Base::System, "Library/Caches",                  LeftoverKind::Cache,       true),
    at(Base::System, "Library/Preferences",             LeftoverKind::Preferences, false),
    at(Base::System, "Library/Logs",                    LeftoverKind::Logs,        true),
    at(Base::System, "Library/PrivilegedHelperTools",   LeftoverKind::HelperTool,  false),
    at(Base::System, "private/var/db/receipts",         LeftoverKind::Receipt,     false),
];

/// `brew_cask` comes from [`brew_cask_for`]; it is looked up separately
/// because asking Homebrew takes a moment and needs it installed.
pub fn plan(
    state: &ScanState,
    roots: &Roots,
    app: &AppInfo,
    installed: &[AppInfo],
    brew_cask: Option<String>,
) -> UninstallPlan {
    let matcher = Matcher::new(app, installed);
    let mut items: Vec<Leftover> = Vec::new();

    let app_present = fs::symlink_metadata(&app.path).is_ok();
    if app_present {
        items.push(leftover(
            app.path.clone(),
            LeftoverKind::Application,
            MatchReason::BundleId,
        ));
    }

    for location in LOCATIONS {
        let base = match location.base {
            Base::Home => &roots.home,
            Base::System => &roots.system,
        };
        for entry in entries(&base.join(location.rel)) {
            if let Some(reason) = matcher.classify(&file_name(&entry), location.by_name) {
                items.push(leftover(entry, location.kind, reason));
            }
        }
    }
    for dir in &roots.user_temp {
        for entry in entries(dir) {
            if let Some(reason) = matcher.classify(&file_name(&entry), false) {
                items.push(leftover(entry, LeftoverKind::Temporary, reason));
            }
        }
    }
    items.extend(group_containers(roots, app, installed, &matcher));
    items.extend(launchd::jobs(roots, app, &matcher));

    // An item found twice (a helper tool listed by name and as the binary
    // of a daemon) is kept once, with its most certain reason.
    items.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.reason.cmp(&b.reason))
            .then(a.kind.cmp(&b.kind))
    });
    items.dedup_by(|later, earlier| later.path == earlier.path);

    items.par_iter_mut().for_each(|item| {
        item.bytes = state.allocated_size(&item.path);
        item.needs_authorization = !execute::can_move(&item.path);
    });
    // Measuring the sizes above showed which containers macOS keeps closed.
    let denied = state.denied();
    for item in &mut items {
        if matches!(item.kind, LeftoverKind::Container | LeftoverKind::GroupContainer)
            && denied.iter().any(|path| path.starts_with(&item.path))
        {
            item.needs_full_disk_access = true;
            // The Finder cannot open it either: no point asking for the password.
            item.needs_authorization = false;
            item.selected = false;
            let protection = "Protégé par macOS : l'accès complet au disque est nécessaire pour le retirer";
            item.note = Some(match item.note.take() {
                Some(note) => format!("{note} · {protection}"),
                None => protection.to_owned(),
            });
        }
    }
    items.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| b.bytes.cmp(&a.bytes)));

    UninstallPlan {
        app: app.clone(),
        app_present,
        items,
        brew_cask,
    }
}

fn leftover(path: PathBuf, kind: LeftoverKind, reason: MatchReason) -> Leftover {
    Leftover {
        id: format!("leftover:{}", path.display()),
        selected: matches!(reason, MatchReason::BundleId | MatchReason::Name),
        path,
        kind,
        reason,
        bytes: 0,
        needs_authorization: false,
        needs_full_disk_access: false,
        note: None,
        launchd_label: None,
    }
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `group.com.example.app` belongs to the app; `TEAMID.anything` belongs
/// to its team, and stays unselected when the team has other apps here.
fn group_containers(roots: &Roots, app: &AppInfo, installed: &[AppInfo], matcher: &Matcher) -> Vec<Leftover> {
    let teammates: Vec<&str> = installed
        .iter()
        .filter(|other| other.bundle_id != app.bundle_id && other.team_id.is_some() && other.team_id == app.team_id)
        .map(|other| other.name.as_str())
        .collect();

    entries(&roots.home.join("Library/Group Containers"))
        .into_iter()
        .filter_map(|path| {
            let name = file_name(&path);
            if let Some(reason) = name
                .strip_prefix("group.")
                .and_then(|rest| matcher.classify(rest, false))
            {
                return Some(leftover(path, LeftoverKind::GroupContainer, reason));
            }
            let team = app.team_id.as_deref()?;
            if !name.starts_with(&format!("{team}.")) {
                return None;
            }
            let mut item = leftover(path, LeftoverKind::GroupContainer, MatchReason::Team);
            item.selected = teammates.is_empty();
            if !teammates.is_empty() {
                item.note = Some(format!("Partagé avec {}", teammates.join(", ")));
            }
            Some(item)
        })
        .collect()
}

impl UninstallPlan {
    /// Removes the selected items: with Homebrew first when asked, then
    /// stopping launch agents, then moving everything left to the Trash.
    pub fn execute(&self, ids: &[String], use_brew: bool, home: &Path) -> ExecutionReport {
        let mut report = ExecutionReport::default();
        if self.app_present && apps::is_running(&self.app, home) {
            report.failures.push(Failure {
                target: self.app.name.clone(),
                reason: format!("Quitte {} avant de la désinstaller.", self.app.name),
            });
            return report;
        }

        if use_brew && let (Some(token), Some(brew)) = (&self.brew_cask, tools::find("brew", home)) {
            let action = Action::Command {
                program: brew,
                args: ["uninstall", "--cask", "--zap", token].map(str::to_owned).to_vec(),
            };
            report.run(&format!("brew:{token}"), 0, &action, home);
        }

        let selected: Vec<&Leftover> = self.items.iter().filter(|item| ids.contains(&item.id)).collect();
        let user_id = crate::launchd::session_user(home);
        for item in &selected {
            if let (LeftoverKind::LaunchAgent, Some(label), Some(uid), false) =
                (item.kind, &item.launchd_label, user_id, item.needs_authorization)
            {
                crate::launchd::stop_agent(label, uid, home);
            }
        }
        for item in selected {
            let action = Action::Trash {
                paths: vec![item.path.clone()],
            };
            report.run(&item.id, item.bytes, &action, home);
        }
        report
    }

    /// Bytes reclaimed by the default selection.
    pub fn selected_bytes(&self) -> u64 {
        self.items.iter().filter(|i| i.selected).map(|i| i.bytes).sum()
    }
}

/// Groups items by kind, for display.
pub fn by_kind(items: &[Leftover]) -> BTreeMap<LeftoverKind, Vec<&Leftover>> {
    let mut groups: BTreeMap<LeftoverKind, Vec<&Leftover>> = BTreeMap::new();
    for item in items {
        groups.entry(item.kind).or_default().push(item);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::fake_app;

    struct Machine {
        _dir: tempfile::TempDir,
        roots: Roots,
    }

    impl Machine {
        fn new() -> Self {
            let dir = crate::testing::tempdir();
            let base = fs::canonicalize(dir.path()).unwrap();
            let roots = Roots {
                home: base.join("home"),
                system: base.join("system"),
                user_temp: vec![base.join("var/C")],
            };
            fs::create_dir_all(&roots.home).unwrap();
            fs::create_dir_all(&roots.system).unwrap();
            fs::create_dir_all(&roots.user_temp[0]).unwrap();
            Self { _dir: dir, roots }
        }

        fn home(&self, rel: &str) -> PathBuf {
            let path = self.roots.home.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"data").unwrap();
            path
        }

        fn home_dir(&self, rel: &str) -> PathBuf {
            let path = self.roots.home.join(rel);
            fs::create_dir_all(&path).unwrap();
            path
        }

        fn app(&self, name: &str, bundle_id: &str, team: Option<&str>) -> AppInfo {
            let path = fake_app(&self.roots.home.join("Applications"), name, bundle_id);
            let mut app = apps::read_bundle(&path).unwrap();
            app.team_id = team.map(str::to_owned);
            app
        }

        fn plan(&self, app: &AppInfo, installed: &[AppInfo]) -> UninstallPlan {
            plan(&ScanState::default(), &self.roots, app, installed, None)
        }
    }

    fn found(plan: &UninstallPlan) -> Vec<(String, LeftoverKind, MatchReason, bool)> {
        plan.items
            .iter()
            .map(|i| (file_name(&i.path), i.kind, i.reason, i.selected))
            .collect()
    }

    #[test]
    fn containers_macos_keeps_closed_are_flagged_and_left_unselected() {
        use std::os::unix::fs::PermissionsExt;
        let machine = Machine::new();
        let app = machine.app("PowerPoint", "com.microsoft.Powerpoint", None);
        machine.home("Library/Containers/com.microsoft.Powerpoint/.metadata.plist");
        let data = machine.home_dir("Library/Containers/com.microsoft.Powerpoint/Data");
        machine.home_dir("Library/Containers/com.microsoft.Powerpoint.widgetextension/Data");
        // Without Full Disk Access, macOS refuses to list the container's data.
        fs::set_permissions(&data, fs::Permissions::from_mode(0o000)).unwrap();

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();

        let item = |name: &str| plan.items.iter().find(|i| file_name(&i.path) == name).unwrap();
        let closed = item("com.microsoft.Powerpoint");
        assert!(closed.needs_full_disk_access && !closed.selected);
        assert!(
            closed
                .note
                .as_deref()
                .is_some_and(|n| n.contains("accès complet au disque"))
        );
        let open = item("com.microsoft.Powerpoint.widgetextension");
        assert!(!open.needs_full_disk_access && open.selected);
    }

    #[test]
    fn finds_files_named_after_the_bundle_id() {
        let machine = Machine::new();
        let app = machine.app("Docker", "com.docker.docker", None);
        machine.home("Library/Preferences/com.docker.docker.plist");
        machine.home("Library/Preferences/ByHost/com.docker.docker.1234-ABCD.plist");
        machine.home_dir("Library/Containers/com.docker.docker");
        machine.home_dir("Library/Saved Application State/com.docker.docker.savedState");
        machine.home("Library/HTTPStorages/com.docker.docker.binarycookies");
        machine.home("Library/Preferences/com.dockerish.other.plist");

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        let names: Vec<String> = plan.items.iter().map(|i| file_name(&i.path)).collect();
        assert!(plan.app_present);
        assert_eq!(plan.items[0].kind, LeftoverKind::Application);
        for expected in [
            "com.docker.docker.plist",
            "com.docker.docker.1234-ABCD.plist",
            "com.docker.docker",
            "com.docker.docker.savedState",
            "com.docker.docker.binarycookies",
        ] {
            assert!(
                names.contains(&expected.to_owned()),
                "{expected} missing from {names:?}"
            );
        }
        assert!(!names.contains(&"com.dockerish.other.plist".to_owned()));
        assert!(plan.items.iter().all(|i| i.selected));
    }

    #[test]
    fn folders_named_after_the_app_are_found_where_apps_use_names() {
        let machine = Machine::new();
        let app = machine.app("Notion", "notion.id", None);
        machine.home_dir("Library/Application Support/Notion");
        machine.home_dir("Library/Caches/notion");
        // Containers are always named by bundle id: a name match there is a coincidence.
        machine.home_dir("Library/Containers/Notion");

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        let found = found(&plan);
        assert!(
            found.contains(&("Notion".into(), LeftoverKind::Support, MatchReason::Name, true)),
            "{found:?}"
        );
        assert!(
            found.contains(&("notion".into(), LeftoverKind::Cache, MatchReason::Name, true)),
            "{found:?}"
        );
        assert!(!found.iter().any(|f| f.1 == LeftoverKind::Container), "{found:?}");
    }

    #[test]
    fn names_of_other_installed_apps_are_left_alone() {
        let machine = Machine::new();
        let app = machine.app("Code", "com.microsoft.VSCode", None);
        let other = AppInfo {
            name: "Code".into(),
            bundle_id: "com.example.code-clone".into(),
            ..app.clone()
        };
        machine.home_dir("Library/Application Support/Code");

        let plan = machine.plan(&app, &[app.clone(), other]);
        assert!(
            !found(&plan)
                .iter()
                .any(|f| f.0 == "Code" && f.1 == LeftoverKind::Support)
        );
    }

    #[test]
    fn vendor_matches_are_proposed_but_not_selected() {
        let machine = Machine::new();
        let app = machine.app("Docker", "com.docker.docker", None);
        machine.home("Library/Preferences/com.docker.helper.plist");

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        let helper = plan
            .items
            .iter()
            .find(|i| file_name(&i.path) == "com.docker.helper.plist")
            .unwrap();
        assert_eq!(helper.reason, MatchReason::Vendor);
        assert!(!helper.selected);
    }

    #[test]
    fn a_vendor_shared_with_an_installed_app_is_not_matched() {
        let machine = Machine::new();
        let word = machine.app("Microsoft Word", "com.microsoft.Word", None);
        let excel = machine.app("Microsoft Excel", "com.microsoft.Excel", None);
        machine.home("Library/Preferences/com.microsoft.office.plist");
        machine.home("Library/Preferences/com.microsoft.Excel.plist");

        let plan = machine.plan(&word, &[word.clone(), excel]);
        assert!(
            plan.items.iter().all(|i| i.kind == LeftoverKind::Application),
            "{:?}",
            found(&plan)
        );
    }

    #[test]
    fn apple_is_never_treated_as_a_vendor() {
        let machine = Machine::new();
        let app = machine.app("Keynote", "com.apple.iWork.Keynote", None);
        machine.home("Library/Preferences/com.apple.finder.plist");

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        assert!(!found(&plan).iter().any(|f| f.0 == "com.apple.finder.plist"));
    }

    #[test]
    fn a_longer_bundle_id_of_another_app_is_not_ours() {
        let machine = Machine::new();
        let app = machine.app("Foo", "com.example.foo", None);
        let pro = machine.app("Foo Pro", "com.example.foo.pro", None);
        machine.home("Library/Preferences/com.example.foo.pro.plist");
        machine.home("Library/Preferences/com.example.foo.plist");

        let plan = machine.plan(&app, &[app.clone(), pro]);
        let names: Vec<String> = plan.items.iter().map(|i| file_name(&i.path)).collect();
        assert!(names.contains(&"com.example.foo.plist".to_owned()));
        assert!(!names.contains(&"com.example.foo.pro.plist".to_owned()));
    }

    #[test]
    fn team_group_containers_shared_with_another_app_are_flagged() {
        let machine = Machine::new();
        let word = machine.app("Microsoft Word", "com.microsoft.Word", Some("UBF8T346G9"));
        let excel = machine.app("Microsoft Excel", "com.microsoft.Excel", Some("UBF8T346G9"));
        machine.home_dir("Library/Group Containers/UBF8T346G9.Office");

        let plan = machine.plan(&word, &[word.clone(), excel]);
        let group = plan
            .items
            .iter()
            .find(|i| i.kind == LeftoverKind::GroupContainer)
            .unwrap();
        assert!(!group.selected);
        assert_eq!(group.note.as_deref(), Some("Partagé avec Microsoft Excel"));
    }

    #[test]
    fn a_team_group_container_of_the_only_team_app_is_selected() {
        let machine = Machine::new();
        let app = machine.app("Docker", "com.docker.docker", Some("9BNSXJN65R"));
        machine.home_dir("Library/Group Containers/9BNSXJN65R.com.docker");
        machine.home_dir("Library/Group Containers/group.com.docker.docker");

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        let groups: Vec<_> = plan
            .items
            .iter()
            .filter(|i| i.kind == LeftoverKind::GroupContainer)
            .collect();
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|g| g.selected && g.note.is_none()));
    }

    #[test]
    fn launch_agents_starting_the_app_are_found_with_their_label() {
        let machine = Machine::new();
        let app = machine.app("Sync", "com.example.sync", None);
        let mut job = plist::Dictionary::new();
        job.insert("Label".into(), "io.unrelated.label".into());
        job.insert(
            "ProgramArguments".into(),
            plist::Value::Array(vec![format!("{}/Contents/MacOS/agent", app.path.display()).into()]),
        );
        let agent = machine.home("Library/LaunchAgents/io.unrelated.label.plist");
        plist::Value::Dictionary(job).to_file_xml(&agent).unwrap();

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        let item = plan.items.iter().find(|i| i.kind == LeftoverKind::LaunchAgent).unwrap();
        assert_eq!(item.launchd_label.as_deref(), Some("io.unrelated.label"));
        assert!(item.selected);
        assert!(!item.needs_authorization);
    }

    #[test]
    fn items_the_user_cannot_move_need_authorization() {
        use std::os::unix::fs::PermissionsExt;
        let machine = Machine::new();
        let app = machine.app("Docker", "com.docker.docker", None);
        let system_support = machine.roots.system.join("Library/Application Support");
        let locked = system_support.join("com.docker.docker");
        fs::create_dir_all(&locked).unwrap();
        let receipt = machine
            .roots
            .system
            .join("private/var/db/receipts/com.docker.docker.bom");
        fs::create_dir_all(receipt.parent().unwrap()).unwrap();
        fs::write(&receipt, b"bom").unwrap();
        fs::set_permissions(&system_support, fs::Permissions::from_mode(0o555)).unwrap();

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        fs::set_permissions(&system_support, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            plan.items
                .iter()
                .find(|i| i.path == locked)
                .unwrap()
                .needs_authorization
        );
        let receipt_item = plan.items.iter().find(|i| i.path == receipt).unwrap();
        assert_eq!(receipt_item.kind, LeftoverKind::Receipt);
        assert!(!receipt_item.needs_authorization, "a writable folder needs no password");
    }

    #[test]
    fn names_starting_like_the_app_are_proposed_unselected() {
        let machine = Machine::new();
        let app = machine.app("Docker", "com.docker.docker", None);
        machine.home_dir("Library/Application Support/Docker Desktop");
        machine.home_dir("Library/Caches/docker-secrets-engine");
        machine.home("Library/Preferences/com.electron.dockerdesktop.plist");
        machine.home_dir("Library/Group Containers/group.com.docker");
        machine.home_dir("Library/Application Support/Dockerfile Linter");

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        let found = found(&plan);
        for (name, reason) in [
            ("Docker Desktop", MatchReason::Similar),
            ("docker-secrets-engine", MatchReason::Similar),
            ("com.electron.dockerdesktop.plist", MatchReason::Similar),
            ("group.com.docker", MatchReason::Vendor),
        ] {
            assert!(
                found.iter().any(|f| f.0 == name && f.2 == reason && !f.3),
                "{name} in {found:?}"
            );
        }
        assert!(
            !found.iter().any(|f| f.0 == "Dockerfile Linter"),
            "no separator after the name"
        );
    }

    #[test]
    fn a_longer_name_of_another_app_is_not_similar() {
        let machine = Machine::new();
        let app = machine.app("Zenith", "com.example.zenith", None);
        let studio = machine.app("Zenith Studio", "com.other.zenithstudio", None);
        machine.home_dir("Library/Application Support/Zenith Studio");
        machine.home_dir("Library/Application Support/Zenith Studio-cache");
        machine.home_dir("Library/Application Support/Zenith Helper");

        let plan = machine.plan(&app, &[app.clone(), studio]);
        let names: Vec<String> = found(&plan).into_iter().map(|f| f.0).collect();
        assert!(names.contains(&"Zenith Helper".to_owned()), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("Zenith Studio")), "{names:?}");
    }

    #[test]
    fn the_most_certain_reason_wins_for_duplicates() {
        let machine = Machine::new();
        let app = machine.app("Docker", "com.docker.docker", None);
        let helper = machine
            .roots
            .system
            .join("Library/PrivilegedHelperTools/com.docker.socket");
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        fs::write(&helper, b"bin").unwrap();
        let mut job = plist::Dictionary::new();
        job.insert("Label".into(), "com.docker.socket".into());
        job.insert("AssociatedBundleIdentifiers".into(), "com.docker.docker".into());
        let daemon = machine
            .roots
            .system
            .join("Library/LaunchDaemons/com.docker.socket.plist");
        fs::create_dir_all(daemon.parent().unwrap()).unwrap();
        plist::Value::Dictionary(job).to_file_xml(&daemon).unwrap();

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        let helpers: Vec<_> = plan.items.iter().filter(|i| i.path == helper).collect();
        assert_eq!(helpers.len(), 1);
        assert_eq!(helpers[0].reason, MatchReason::BundleId);
        assert!(helpers[0].selected);
    }

    #[test]
    fn executing_moves_selected_items_to_the_trash() {
        let machine = Machine::new();
        let app = machine.app("Notion", "notion.id", None);
        let prefs = machine.home("Library/Preferences/notion.id.plist");
        let support = machine.home_dir("Library/Application Support/Notion");

        let plan = machine.plan(&app, std::slice::from_ref(&app));
        let ids: Vec<String> = plan
            .items
            .iter()
            .filter(|i| i.path != support)
            .map(|i| i.id.clone())
            .collect();
        let report = plan.execute(&ids, false, &machine.roots.home);

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(!app.path.exists() && !prefs.exists());
        assert!(support.exists(), "unselected items stay");
        assert!(machine.roots.home.join(".Trash/Notion.app").exists());
        assert!(machine.roots.home.join(".Trash/notion.id.plist").exists());
    }

    #[test]
    fn a_removed_app_still_gets_its_leftovers_found() {
        let machine = Machine::new();
        let app = machine.app("Notion", "notion.id", None);
        machine.home("Library/Preferences/notion.id.plist");
        fs::remove_dir_all(&app.path).unwrap();

        let plan = machine.plan(&app, &[]);
        assert!(!plan.app_present);
        assert_eq!(
            found(&plan),
            vec![(
                "notion.id.plist".into(),
                LeftoverKind::Preferences,
                MatchReason::BundleId,
                true
            )]
        );
    }
}
