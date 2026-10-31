//! Projets oubliés : repérés par leur dernière activité, compressés en
//! `.tar.gz` sans ce qui se recrée tout seul, puis l'original part à la corbeille.

// l'idée c'est de garder le code et l'historique git mais pas les 3 Go de node_modules
// d'un projet pas ouvert depuis 2 ans, et de pouvoir le ranger sur un disque externe

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use rayon::prelude::*;
use serde::Serialize;

use crate::listing::{EntryKind, list_dir};
use crate::tools;
use crate::walk::ScanState;

/// Ce qui prouve qu'un dossier est un projet.
const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "package.json",
    "Cargo.toml",
    "Package.swift",
    "pubspec.yaml",
    "build.gradle",
    "build.gradle.kts",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "Gemfile",
    "composer.json",
    "CMakeLists.txt",
    "Podfile",
    "project.godot",
];
/// Profondeur max sous un dossier de projets.
const MAX_DEPTH: usize = 4;

#[derive(Debug, Clone, Serialize)]
pub struct OldProject {
    pub path: PathBuf,
    pub name: String,
    /// Dernière modification d'un fichier du projet, Unix seconds.
    pub last_activity: u64,
    pub bytes: u64,
    /// Ce qui se recrée tout seul (dépendances, builds) et ne sera pas archivé.
    pub regenerable_bytes: u64,
    pub is_git: bool,
    /// Des changements pas commités : à regarder avant d'archiver.
    pub has_uncommitted: bool,
}

/// Les projets sans activité depuis `idle_for`, les plus gros d'abord.
pub fn find_old_projects(roots: &[PathBuf], idle_for: Duration, home: &Path, state: &ScanState) -> Vec<OldProject> {
    let found = Mutex::new(Vec::new());
    roots.par_iter().for_each(|root| find(root, 0, state, &found));
    let limit = SystemTime::now() - idle_for;
    let git = tools::find("git", home);

    let mut projects: Vec<OldProject> = found
        .into_inner()
        .expect("projects poisoned")
        .into_par_iter()
        .filter_map(|path| {
            if state.cancel.is_cancelled() {
                return None;
            }
            let survey = survey(&path, state);
            let last = survey.last_activity?;
            if last >= limit {
                return None;
            }
            let is_git = path.join(".git").exists();
            let has_uncommitted = is_git
                && git.as_deref().is_some_and(|git| {
                    let dir = path.to_string_lossy();
                    tools::output(
                        git,
                        &["-C", &dir, "status", "--porcelain"],
                        home,
                        Duration::from_secs(10),
                    )
                    .is_some_and(|out| !out.trim().is_empty())
                });
            Some(OldProject {
                name: path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                last_activity: crate::time::unix_seconds(last),
                bytes: survey.bytes,
                regenerable_bytes: survey.regenerable.iter().map(|(_, b)| b).sum(),
                is_git,
                has_uncommitted,
                path,
            })
        })
        .collect();
    projects.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    projects
}

fn find(dir: &Path, depth: usize, state: &ScanState, found: &Mutex<Vec<PathBuf>>) {
    if depth > MAX_DEPTH || state.cancel.is_cancelled() {
        return;
    }
    let Ok(entries) = list_dir(dir) else { return };
    // le dossier racine des projets n'est jamais un projet lui même
    if depth > 0
        && entries
            .iter()
            .any(|e| PROJECT_MARKERS.contains(&e.name.to_string_lossy().as_ref()))
    {
        found.lock().expect("projects poisoned").push(dir.to_path_buf());
        return;
    }
    let subdirs: Vec<PathBuf> = entries
        .iter()
        .filter(|e| e.kind == EntryKind::Dir && !e.is_dataless() && !e.name.to_string_lossy().starts_with('.'))
        .map(|e| dir.join(&e.name))
        .collect();
    subdirs.par_iter().for_each(|sub| find(sub, depth + 1, state, found));
}

struct Survey {
    bytes: u64,
    last_activity: Option<SystemTime>,
    /// Dossiers recréables, chemin et taille.
    regenerable: Vec<(PathBuf, u64)>,
}

/// Taille, dernière activité (hors dossiers recréables et `.git`, que
/// `git fetch` touche sans qu'on travaille) et dossiers recréables.
fn survey(project: &Path, state: &ScanState) -> Survey {
    let mut survey = Survey {
        bytes: 0,
        last_activity: None,
        regenerable: Vec::new(),
    };
    let mut stack = vec![project.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if state.cancel.is_cancelled() {
            break;
        }
        let Ok(entries) = list_dir(&dir) else { continue };
        let names: Vec<String> = entries.iter().map(|e| e.name.to_string_lossy().into_owned()).collect();
        for entry in &entries {
            let path = dir.join(&entry.name);
            match entry.kind {
                EntryKind::Dir if entry.is_dataless() => {}
                EntryKind::Dir => {
                    let name = entry.name.to_string_lossy();
                    if is_regenerable(&path, &name, &names) {
                        let bytes = state.allocated_size(&path);
                        survey.bytes += bytes;
                        survey.regenerable.push((path, bytes));
                    } else if name == ".git" {
                        survey.bytes += state.allocated_size(&path);
                        // l'index bouge à chq commit/checkout, c le vrai signe d'activité ds git
                        if let Ok(modified) = fs::metadata(path.join("index")).and_then(|m| m.modified()) {
                            survey.last_activity = survey.last_activity.max(Some(modified));
                        }
                    } else {
                        stack.push(path);
                    }
                }
                EntryKind::File | EntryKind::Other => {
                    survey.bytes += entry.allocated;
                    survey.last_activity = survey.last_activity.max(entry.modified);
                }
            }
        }
    }
    survey
}

/// Dossiers que leur outil recrée, reconnus à un marqueur fiable.
fn is_regenerable(path: &Path, name: &str, siblings: &[String]) -> bool {
    let has = |file: &str| siblings.iter().any(|s| s == file);
    match name {
        "node_modules" | ".next" | ".nuxt" | ".turbo" | ".svelte-kit" | ".parcel-cache" => has("package.json"),
        "target" => path.join("CACHEDIR.TAG").is_file(),
        ".venv" | "venv" => path.join("pyvenv.cfg").is_file(),
        "Pods" => has("Podfile"),
        ".build" => has("Package.swift"),
        "build" => has("build.gradle") || has("build.gradle.kts"),
        ".gradle" => has("settings.gradle") || has("settings.gradle.kts") || has("gradlew"),
        ".dart_tool" => has("pubspec.yaml"),
        ".zig-cache" => has("build.zig"),
        "DerivedData" => true,
        _ => false,
    }
}

/// Compresse `project` dans `destination` sans ses dossiers recréables,
/// puis relit l'archive pour vérifier qu'elle est complète. Renvoie son
/// chemin ; le projet lui-même n'est pas touché.
pub fn archive_project(project: &Path, destination: &Path, state: &ScanState) -> io::Result<PathBuf> {
    let (Some(parent), Some(name)) = (project.parent(), project.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "projet sans dossier parent",
        ));
    };
    fs::create_dir_all(destination)?;
    let archive = unique_archive_path(destination, &name.to_string_lossy());

    let survey = survey(project, state);
    let mut command = Command::new("/usr/bin/tar");
    command.arg("-czf").arg(&archive);
    for (path, _) in &survey.regenerable {
        // les exclusions de bsdtar sont ancrées au début, donc chemin relatif exact
        if let Ok(relative) = path.strip_prefix(parent) {
            command.arg("--exclude").arg(relative);
        }
    }
    command.arg("-C").arg(parent).arg(name);
    let status = command.status()?;
    if !status.success() {
        let _ = fs::remove_file(&archive);
        return Err(io::Error::other("la compression a échoué"));
    }

    // relecture complète : si l'archive est abîmée on garde le projet tel quel
    let listed = Command::new("/usr/bin/tar").arg("-tzf").arg(&archive).output()?;
    let entries = String::from_utf8_lossy(&listed.stdout).lines().count();
    if !listed.status.success() || entries == 0 {
        let _ = fs::remove_file(&archive);
        return Err(io::Error::other("l'archive créée est illisible"));
    }
    Ok(archive)
}

/// `nom (2026-10-31).tar.gz`, avec un numéro si ça existe déjà.
fn unique_archive_path(destination: &Path, name: &str) -> PathBuf {
    let date = crate::time::today();
    let mut candidate = destination.join(format!("{name} ({date}).tar.gz"));
    let mut index = 2;
    while candidate.exists() {
        candidate = destination.join(format!("{name} ({date}) {index}.tar.gz"));
        index += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn age(path: &Path, days: u64) {
        let file = File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(days * 86_400))
            .unwrap();
    }

    fn project(root: &Path, name: &str, days: u64) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("node_modules/lib")).unwrap();
        fs::write(dir.join("package.json"), "{}").unwrap();
        fs::write(dir.join("src/index.js"), "console.log(1)").unwrap();
        fs::write(dir.join("node_modules/lib/big.js"), vec![1u8; 200_000]).unwrap();
        age(&dir.join("package.json"), days);
        age(&dir.join("src/index.js"), days);
        dir
    }

    #[test]
    fn only_idle_projects_are_found() {
        let root = tempfile::tempdir().unwrap();
        project(root.path(), "vieux", 400);
        project(root.path(), "actif", 2);
        // un fichier récent ds node_modules compte pas comme de l'activité
        let found = find_old_projects(
            &[root.path().to_path_buf()],
            Duration::from_secs(180 * 86_400),
            root.path(),
            &ScanState::default(),
        );
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["vieux"]);
        assert!(found[0].regenerable_bytes >= 200_000);
        assert!(found[0].bytes > found[0].regenerable_bytes);
    }

    #[test]
    fn archives_leave_regenerable_folders_out() {
        let root = tempfile::tempdir().unwrap();
        let dir = project(root.path(), "app", 400);
        let destination = root.path().join("Archives");
        let archive = archive_project(&dir, &destination, &ScanState::default()).unwrap();

        let listed = Command::new("/usr/bin/tar").arg("-tzf").arg(&archive).output().unwrap();
        let listed = String::from_utf8_lossy(&listed.stdout);
        assert!(listed.contains("app/src/index.js"), "{listed}");
        assert!(!listed.contains("node_modules"), "{listed}");
        assert!(dir.exists(), "le projet reste en place");

        let second = archive_project(&dir, &destination, &ScanState::default()).unwrap();
        assert_ne!(archive, second);
    }
}
