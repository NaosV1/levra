//! Installed applications and their identity (bundle id, signing team).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::tools;
use crate::walk::ScanState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub path: PathBuf,
    pub name: String,
    pub bundle_id: String,
    pub version: Option<String>,
    /// `CFBundleExecutable`: crash reports and some folders use this name.
    pub executable: Option<String>,
    /// Apple Developer team that signed the app; apps of one team often
    /// share a group container.
    pub team_id: Option<String>,
    pub bytes: u64,
    /// Last time the app was opened according to Spotlight, Unix seconds.
    pub last_used: Option<u64>,
}

pub fn application_dirs(home: &Path) -> Vec<PathBuf> {
    vec![PathBuf::from("/Applications"), home.join("Applications")]
}

/// Apps in the application folders and one level of subfolders
/// (`/Applications/Utilities`, vendor folders). Symlinks are skipped:
/// Safari in `/Applications` points into the sealed system volume.
pub fn list_apps(state: &ScanState, home: &Path) -> Vec<AppInfo> {
    let bundles: Vec<PathBuf> = application_dirs(home)
        .iter()
        .flat_map(|dir| bundles_in(dir, 1))
        .collect();
    let mut apps: Vec<AppInfo> = bundles
        .par_iter()
        .filter_map(|path| {
            let mut app = read_bundle(path).ok()?;
            app.team_id = signing_team(path, home);
            app.bytes = state.allocated_size(path);
            Some(app)
        })
        .collect();

    let dates = last_used_dates(&apps.iter().map(|a| a.path.as_path()).collect::<Vec<_>>(), home);
    for (app, last_used) in apps.iter_mut().zip(dates) {
        app.last_used = last_used;
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

pub(crate) fn bundles_in(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut bundles = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "app") {
            bundles.push(path);
        } else if depth > 0 {
            bundles.extend(bundles_in(&path, depth - 1));
        }
    }
    bundles
}

/// Full identity of one app, including its size and signing team.
pub fn inspect_app(state: &ScanState, path: &Path, home: &Path) -> io::Result<AppInfo> {
    let mut app = read_bundle(path)?;
    app.team_id = signing_team(path, home);
    app.bytes = state.allocated_size(path);
    app.last_used = last_used_dates(&[path], home).pop().flatten();
    Ok(app)
}

/// Reads `Contents/Info.plist` (XML or binary).
pub fn read_bundle(path: &Path) -> io::Result<AppInfo> {
    let invalid = |why: &str| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {why}", path.display()));
    let info =
        plist::Value::from_file(path.join("Contents/Info.plist")).map_err(|_| invalid("Info.plist illisible"))?;
    let info = info.as_dictionary().ok_or_else(|| invalid("Info.plist invalide"))?;
    let text = |key: &str| info.get(key).and_then(plist::Value::as_string).map(str::to_owned);

    let bundle_id = text("CFBundleIdentifier").ok_or_else(|| invalid("pas d'identifiant de bundle"))?;
    let file_stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(AppInfo {
        path: path.to_path_buf(),
        name: text("CFBundleDisplayName")
            .or_else(|| text("CFBundleName"))
            .unwrap_or(file_stem),
        bundle_id,
        version: text("CFBundleShortVersionString"),
        executable: text("CFBundleExecutable"),
        team_id: None,
        bytes: 0,
        last_used: None,
    })
}

fn signing_team(path: &Path, home: &Path) -> Option<String> {
    let path_text = path.to_string_lossy();
    let details = tools::error_output(
        Path::new("/usr/bin/codesign"),
        &["-dv", "--verbose=2", &path_text],
        home,
        Duration::from_secs(10),
    )?;
    parse_team(&details)
}

fn parse_team(codesign_details: &str) -> Option<String> {
    let team = codesign_details
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))?
        .trim();
    (!team.is_empty() && team != "not set").then(|| team.to_owned())
}

/// One `mdls` call for every app: it prints the values separated by NUL.
fn last_used_dates(paths: &[&Path], home: &Path) -> Vec<Option<u64>> {
    if paths.is_empty() {
        return Vec::new();
    }
    let mut args = vec!["-name", "kMDItemLastUsedDate", "-raw", "-nullMarker", "(null)"];
    let texts: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    args.extend(texts.iter().map(String::as_str));

    let output = tools::output(Path::new("/usr/bin/mdls"), &args, home, Duration::from_secs(30));
    let values: Vec<Option<u64>> = output
        .map(|text| text.split('\0').map(crate::time::parse_utc).collect())
        .unwrap_or_default();
    if values.len() == paths.len() {
        values
    } else {
        vec![None; paths.len()]
    }
}

/// Whether the app itself is open: its main executable runs. Other
/// processes from its bundle do not count: macOS starts extensions such as
/// widgets on its own, and quitting the app does not stop them.
pub fn is_running(app: &AppInfo, home: &Path) -> bool {
    let Some(executable) = &app.executable else {
        return false;
    };
    let main = app.path.join("Contents/MacOS").join(executable);
    tools::output(Path::new("/bin/ps"), &["-axo", "comm="], home, Duration::from_secs(5))
        .is_some_and(|list| runs(&list, &main))
}

/// Whether `ps -o comm` output lists this executable.
fn runs(process_list: &str, executable: &Path) -> bool {
    process_list
        .lines()
        .any(|line| Path::new(line.trim_end()) == executable)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Creates a minimal `.app` bundle with an XML Info.plist.
    pub fn fake_app(dir: &Path, name: &str, bundle_id: &str) -> PathBuf {
        let app = dir.join(format!("{name}.app"));
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIdentifier".into(), bundle_id.into());
        info.insert("CFBundleName".into(), name.into());
        info.insert("CFBundleExecutable".into(), name.into());
        info.insert("CFBundleShortVersionString".into(), "1.2.3".into());
        plist::Value::Dictionary(info)
            .to_file_xml(app.join("Contents/Info.plist"))
            .unwrap();
        fs::write(app.join("Contents/MacOS").join(name), vec![0u8; 50_000]).unwrap();
        app
    }

    #[test]
    fn reads_bundle_identity() {
        let dir = tempfile::tempdir().unwrap();
        let app = fake_app(dir.path(), "Notion", "notion.id");
        let info = read_bundle(&app).unwrap();
        assert_eq!(info.name, "Notion");
        assert_eq!(info.bundle_id, "notion.id");
        assert_eq!(info.version.as_deref(), Some("1.2.3"));
        assert_eq!(info.executable.as_deref(), Some("Notion"));
    }

    #[test]
    fn folders_without_info_plist_are_not_apps() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("Broken.app/Contents")).unwrap();
        assert!(read_bundle(&dir.path().join("Broken.app")).is_err());
    }

    #[test]
    fn finds_apps_one_level_deep_but_not_inside_bundles() {
        let dir = tempfile::tempdir().unwrap();
        fake_app(dir.path(), "Top", "com.example.top");
        fake_app(&dir.path().join("Utilities"), "Nested", "com.example.nested");
        fake_app(
            &dir.path().join("Top.app/Contents/Helpers"),
            "Helper",
            "com.example.helper",
        );

        let mut names: Vec<String> = bundles_in(dir.path(), 1)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["Nested.app", "Top.app"]);
    }

    #[test]
    fn parses_the_signing_team() {
        assert_eq!(
            parse_team("Identifier=com.docker.docker\nTeamIdentifier=9BNSXJN65R\n").as_deref(),
            Some("9BNSXJN65R")
        );
        assert_eq!(parse_team("TeamIdentifier=not set\n"), None);
        assert_eq!(parse_team("code object is not signed at all"), None);
    }

    #[test]
    fn only_the_main_executable_means_the_app_is_open() {
        let main = Path::new("/Applications/Microsoft PowerPoint.app/Contents/MacOS/Microsoft PowerPoint");
        // macOS keeps the widget running while the app is closed.
        let widget_only = "/usr/libexec/chronod\n\
            /Applications/Microsoft PowerPoint.app/Contents/PlugIns/PowerPointWidget_mac.appex/Contents/MacOS/PowerPointWidget_mac\n";
        assert!(!runs(widget_only, main));

        let open = format!("{widget_only}{}\n", main.display());
        assert!(runs(&open, main));

        let helper = "/Applications/Microsoft PowerPoint.app/Contents/MacOS/Microsoft PowerPoint Helper\n";
        assert!(!runs(helper, main));
    }

    #[test]
    fn an_app_without_an_executable_never_runs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = read_bundle(&fake_app(dir.path(), "Notion", "notion.id")).unwrap();
        app.executable = None;
        assert!(!is_running(&app, dir.path()));
    }

    #[test]
    fn inspects_a_real_system_app() {
        let app = inspect_app(
            &ScanState::default(),
            Path::new("/System/Applications/Calculator.app"),
            Path::new("/"),
        )
        .unwrap();
        assert_eq!(app.bundle_id, "com.apple.calculator");
        assert!(app.bytes > 0);
    }
}
