//! Build outputs and dependency folders inside development projects, and
//! clean git worktrees.
//!
//! A folder is reported only when a reliable marker proves what it is:
//! `target/` must hold Cargo's `CACHEDIR.TAG`, a virtualenv its
//! `pyvenv.cfg`, `node_modules/` must sit next to a `package.json`, and so
//! on. Any other folder tagged with a valid `CACHEDIR.TAG` is a cache by
//! definition (<https://bford.info/cachedir/>).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rayon::prelude::*;

use super::{Category, Finding, Safety, ScanOptions, deletable, file_name};
use crate::execute::Action;
use crate::tools;
use crate::walk::ScanState;

const MAX_DEPTH: usize = 8;
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

enum Marker {
    /// One of these files sits next to the folder.
    Sibling(&'static [&'static str]),
    /// This file exists inside the folder.
    Inside(&'static str),
}

struct Rule {
    dir: &'static str,
    marker: Marker,
    what: &'static str,
    safety: Safety,
    regenerate: &'static str,
}

const PACKAGE_JSON: &[&str] = &["package.json"];
const GRADLE_BUILD: &[&str] = &["build.gradle", "build.gradle.kts"];
const GRADLE_ROOT: &[&str] = &["settings.gradle", "settings.gradle.kts", "gradlew"];
const NODE_LOCKFILES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
    "bun.lock",
];

const RULES: &[Rule] = &[
    Rule {
        dir: "target",
        marker: Marker::Inside("CACHEDIR.TAG"),
        what: "build Rust (target/)",
        safety: Safety::Safe,
        regenerate: "Recompilé au prochain cargo build.",
    },
    Rule {
        dir: "node_modules",
        marker: Marker::Sibling(PACKAGE_JSON),
        what: "dépendances npm (node_modules/)",
        safety: Safety::Safe,
        regenerate: "Réinstallées avec npm/pnpm/yarn install.",
    },
    Rule {
        dir: ".venv",
        marker: Marker::Inside("pyvenv.cfg"),
        what: "environnement Python (.venv/)",
        safety: Safety::Review,
        regenerate: "À recréer avec python3 -m venv puis pip install.",
    },
    Rule {
        dir: "venv",
        marker: Marker::Inside("pyvenv.cfg"),
        what: "environnement Python (venv/)",
        safety: Safety::Review,
        regenerate: "À recréer avec python3 -m venv puis pip install.",
    },
    Rule {
        dir: "Pods",
        marker: Marker::Sibling(&["Podfile"]),
        what: "pods CocoaPods (Pods/)",
        safety: Safety::Safe,
        regenerate: "Réinstallés avec pod install.",
    },
    Rule {
        dir: ".next",
        marker: Marker::Sibling(PACKAGE_JSON),
        what: "build Next.js (.next/)",
        safety: Safety::Safe,
        regenerate: "Recréé par next dev ou next build.",
    },
    Rule {
        dir: ".turbo",
        marker: Marker::Sibling(PACKAGE_JSON),
        what: "cache Turborepo (.turbo/)",
        safety: Safety::Safe,
        regenerate: "Recréé au prochain build.",
    },
    Rule {
        dir: ".nuxt",
        marker: Marker::Sibling(PACKAGE_JSON),
        what: "build Nuxt (.nuxt/)",
        safety: Safety::Safe,
        regenerate: "Recréé par nuxt dev ou nuxt build.",
    },
    Rule {
        dir: ".svelte-kit",
        marker: Marker::Sibling(PACKAGE_JSON),
        what: "build SvelteKit (.svelte-kit/)",
        safety: Safety::Safe,
        regenerate: "Recréé au prochain build.",
    },
    Rule {
        dir: ".parcel-cache",
        marker: Marker::Sibling(PACKAGE_JSON),
        what: "cache Parcel",
        safety: Safety::Safe,
        regenerate: "Recréé au prochain build.",
    },
    Rule {
        dir: "build",
        marker: Marker::Sibling(GRADLE_BUILD),
        what: "build Gradle (build/)",
        safety: Safety::Safe,
        regenerate: "Recréé par Gradle au prochain build.",
    },
    Rule {
        dir: ".gradle",
        marker: Marker::Sibling(GRADLE_ROOT),
        what: "cache Gradle du projet (.gradle/)",
        safety: Safety::Safe,
        regenerate: "Recréé par Gradle au prochain build.",
    },
    Rule {
        dir: ".build",
        marker: Marker::Sibling(&["Package.swift"]),
        what: "build SwiftPM (.build/)",
        safety: Safety::Safe,
        regenerate: "Recréé par swift build.",
    },
    Rule {
        dir: ".dart_tool",
        marker: Marker::Sibling(&["pubspec.yaml"]),
        what: "outils Dart (.dart_tool/)",
        safety: Safety::Safe,
        regenerate: "Recréé par flutter pub get.",
    },
    Rule {
        dir: ".zig-cache",
        marker: Marker::Sibling(&["build.zig"]),
        what: "cache Zig",
        safety: Safety::Safe,
        regenerate: "Recréé par zig build.",
    },
];

/// Folders never worth descending into while looking for projects.
const SKIP: &[&str] = &[".git", "node_modules", "Library", ".Trash"];

/// A project folder and how to name it: its path below the project root
/// (`us-applications/apps/mobile/ios` says more than `ios`).
struct Project {
    path: PathBuf,
    label: String,
}

impl Project {
    fn new(path: &Path, root: &Path) -> Self {
        let label = path
            .strip_prefix(root)
            .ok()
            .filter(|rel| !rel.as_os_str().is_empty())
            .map_or_else(|| file_name(path), |rel| rel.display().to_string());
        Self {
            path: path.to_path_buf(),
            label,
        }
    }
}

enum Hit {
    Artifact {
        path: PathBuf,
        project: Project,
        rule: Option<&'static Rule>,
    },
    Repository(Project),
}

pub(super) fn detect(options: &ScanOptions, state: &ScanState) -> Vec<Finding> {
    let hits = Mutex::new(Vec::new());
    options
        .project_roots
        .par_iter()
        .for_each(|root| walk(root, root, 0, state, &hits));
    let hits = hits.into_inner().expect("hits poisoned");

    let (mut artifacts, mut repositories) = (Vec::new(), Vec::new());
    for hit in hits {
        match hit {
            Hit::Artifact { path, project, rule } => artifacts.push((path, project, rule)),
            Hit::Repository(project) => repositories.push(project),
        }
    }

    let git = tools::find("git", &options.home);
    let mut findings: Vec<Finding> = artifacts
        .into_par_iter()
        .map(|(path, project, rule)| artifact_finding(state, git.as_deref(), &options.home, path, &project, rule))
        .collect();
    if let Some(git) = &git {
        findings.extend(
            repositories
                .par_iter()
                .flat_map(|repo| worktree_findings(state, git, &options.home, repo))
                .collect::<Vec<_>>(),
        );
    }
    findings
}

fn walk(root: &Path, dir: &Path, depth: usize, state: &ScanState, hits: &Mutex<Vec<Hit>>) {
    if depth > MAX_DEPTH || state.cancel.is_cancelled() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names = HashSet::new();
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            subdirs.push(name.clone());
        }
        names.insert(name);
    }

    let mut descend = Vec::new();
    for name in subdirs {
        let path = dir.join(&name);
        if name == ".git" {
            if path.join("worktrees").is_dir() {
                hits.lock()
                    .expect("hits poisoned")
                    .push(Hit::Repository(Project::new(dir, root)));
            }
            continue;
        }
        let rule = matching_rule(&name, &path, &names);
        if rule.is_some() || has_cachedir_tag(&path) {
            hits.lock().expect("hits poisoned").push(Hit::Artifact {
                path,
                project: Project::new(dir, root),
                rule,
            });
        } else if !SKIP.contains(&name.as_str()) && !name.starts_with('.') && !is_linked_worktree(&path) {
            descend.push(path);
        }
    }
    descend
        .par_iter()
        .for_each(|sub| walk(root, sub, depth + 1, state, hits));
}

fn matching_rule(name: &str, path: &Path, siblings: &HashSet<String>) -> Option<&'static Rule> {
    RULES.iter().find(|rule| {
        rule.dir == name
            && match rule.marker {
                Marker::Sibling(files) => files.iter().any(|f| siblings.contains(*f)),
                Marker::Inside(file) => path.join(file).is_file(),
            }
    })
}

fn has_cachedir_tag(dir: &Path) -> bool {
    fs::read(dir.join("CACHEDIR.TAG")).is_ok_and(|content| content.starts_with(CACHEDIR_SIGNATURE))
}

/// Linked worktrees are reported as a whole by [`worktree_findings`], so
/// their contents are not listed a second time.
fn is_linked_worktree(dir: &Path) -> bool {
    fs::read_to_string(dir.join(".git"))
        .is_ok_and(|content| content.starts_with("gitdir:") && content.contains("/worktrees/"))
}

fn artifact_finding(
    state: &ScanState,
    git: Option<&Path>,
    home: &Path,
    path: PathBuf,
    project: &Project,
    rule: Option<&'static Rule>,
) -> Finding {
    let (what, mut safety, regenerate) = match rule {
        Some(rule) => (rule.what.to_owned(), rule.safety, rule.regenerate),
        None => (
            format!("cache ({}/)", file_name(&path)),
            Safety::Safe,
            "Dossier marqué comme cache (CACHEDIR.TAG) : recréé par son outil.",
        ),
    };
    let mut detail = regenerate.to_owned();
    if rule.is_some_and(|r| r.dir == "node_modules") && !NODE_LOCKFILES.iter().any(|f| project.path.join(f).is_file()) {
        safety = Safety::Review;
        detail.push_str(" Aucun fichier de verrouillage : les versions réinstallées peuvent différer.");
    }

    let title = format!("{} — {what}", project.label);
    let mut finding = deletable(state, Category::ProjectBuilds, path, title, detail, safety);
    finding.last_used = last_activity(git, home, &project.path);
    finding
}

/// Date of the last commit when the project is in a git repository,
/// otherwise the most recent change among its top-level entries.
fn last_activity(git: Option<&Path>, home: &Path, project: &Path) -> Option<u64> {
    let project_text = project.to_string_lossy();
    let from_git = git.and_then(|git| {
        tools::output(
            git,
            &["-C", &project_text, "log", "-1", "--format=%ct"],
            home,
            Duration::from_secs(5),
        )
    });
    if let Some(seconds) = from_git.and_then(|out| out.trim().parse().ok()) {
        return Some(seconds);
    }
    fs::read_dir(project)
        .ok()?
        .flatten()
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
        .map(crate::time::unix_seconds)
}

fn worktree_findings(state: &ScanState, git: &Path, home: &Path, repo: &Project) -> Vec<Finding> {
    let repo_text = repo.path.to_string_lossy();
    let Some(listing) = tools::output(
        git,
        &["-C", &repo_text, "worktree", "list", "--porcelain"],
        home,
        Duration::from_secs(10),
    ) else {
        return Vec::new();
    };

    parse_worktrees(&listing)
        .into_iter()
        // The first entry is the main working tree.
        .skip(1)
        .filter(|wt| wt.path.is_dir() && is_clean(git, home, &wt.path))
        .map(|wt| {
            let branch = wt.branch.as_deref().unwrap_or("HEAD détachée");
            let path_text = wt.path.to_string_lossy().into_owned();
            Finding {
                id: format!("worktree:{path_text}"),
                category: Category::GitWorktrees,
                title: format!("{} — worktree « {branch} »", repo.label),
                detail: "Aucune modification non commitée. Le dossier est retiré, la branche et ses commits restent dans le dépôt."
                    .to_owned(),
                bytes: state.allocated_size(&wt.path),
                safety: Safety::Review,
                action: Action::Command {
                    program: git.to_path_buf(),
                    args: vec!["-C".into(), repo_text.clone().into_owned(), "worktree".into(), "remove".into(), path_text],
                },
                last_used: last_activity(Some(git), home, &wt.path),
            }
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct Worktree {
    path: PathBuf,
    branch: Option<String>,
}

fn parse_worktrees(porcelain: &str) -> Vec<Worktree> {
    porcelain
        .split("\n\n")
        .filter_map(|block| {
            let mut lines = block.lines();
            let path = lines.next()?.strip_prefix("worktree ")?;
            let branch = lines
                .find_map(|l| l.strip_prefix("branch "))
                .map(|b| b.trim_start_matches("refs/heads/").to_owned());
            Some(Worktree {
                path: PathBuf::from(path),
                branch,
            })
        })
        .collect()
}

/// Untracked files count as changes: `git worktree remove` would refuse
/// to drop them anyway. Ignored files (build output) do not.
fn is_clean(git: &Path, home: &Path, worktree: &Path) -> bool {
    let text = worktree.to_string_lossy();
    tools::output(
        git,
        &["-C", &text, "status", "--porcelain"],
        home,
        Duration::from_secs(10),
    )
    .is_some_and(|status| status.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    fn big_file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; 200_000]).unwrap();
    }

    fn options(root: &Path) -> ScanOptions {
        ScanOptions {
            home: root.to_path_buf(),
            project_roots: vec![root.to_path_buf()],
            min_bytes: 0,
        }
    }

    fn found(root: &Path) -> Vec<Finding> {
        let mut findings = detect(&options(root), &ScanState::default());
        findings.sort_by(|a, b| a.id.cmp(&b.id));
        findings
    }

    fn deleted_path(finding: &Finding) -> &Path {
        &finding.action.paths()[0]
    }

    #[test]
    fn finds_rust_targets_only_with_their_cachedir_tag() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("tool/Cargo.toml"));
        fs::create_dir_all(root.join("tool/target")).unwrap();
        fs::write(root.join("tool/target/CACHEDIR.TAG"), CACHEDIR_SIGNATURE).unwrap();
        big_file(&root.join("tool/target/debug/binary"));
        // A folder merely named target is not a build directory.
        big_file(&root.join("site/target/important.txt"));

        let findings = found(root);
        assert_eq!(findings.len(), 1);
        assert_eq!(deleted_path(&findings[0]), root.join("tool/target"));
        assert_eq!(findings[0].title, "tool — build Rust (target/)");
        assert_eq!(findings[0].safety, Safety::Safe);
        assert!(findings[0].bytes >= 200_000);
    }

    #[test]
    fn node_modules_without_lockfile_needs_review() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("locked/package.json"));
        touch(&root.join("locked/pnpm-lock.yaml"));
        touch(&root.join("locked/node_modules/a/index.js"));
        touch(&root.join("loose/package.json"));
        touch(&root.join("loose/node_modules/a/index.js"));

        let findings = found(root);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].safety, Safety::Safe, "{}", findings[0].id);
        assert_eq!(findings[1].safety, Safety::Review, "{}", findings[1].id);
    }

    #[test]
    fn nested_artifacts_are_not_reported_twice() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("app/package.json"));
        touch(&root.join("app/node_modules/dep/package.json"));
        touch(&root.join("app/node_modules/dep/node_modules/inner/index.js"));

        assert_eq!(found(root).len(), 1);
    }

    #[test]
    fn virtualenvs_are_recognised_by_their_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("bot/venv/pyvenv.cfg"));
        touch(&root.join("bot/.venv/pyvenv.cfg"));
        touch(&root.join("notes/venv/readme.md"));

        let findings = found(root);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.safety == Safety::Review));
    }

    #[test]
    fn gradle_build_requires_a_gradle_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("mobile/android/app/build.gradle"));
        touch(&root.join("mobile/android/app/build/outputs/app.apk"));
        touch(&root.join("website/build/index.html"));

        let findings = found(root);
        assert_eq!(findings.len(), 1);
        assert_eq!(deleted_path(&findings[0]), root.join("mobile/android/app/build"));
    }

    #[test]
    fn any_folder_with_a_valid_cachedir_tag_is_a_cache() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("proj/.cache-thing")).unwrap();
        fs::create_dir_all(root.join("proj/tool-cache")).unwrap();
        fs::write(root.join("proj/tool-cache/CACHEDIR.TAG"), CACHEDIR_SIGNATURE).unwrap();
        fs::create_dir_all(root.join("proj/fake")).unwrap();
        fs::write(root.join("proj/fake/CACHEDIR.TAG"), b"not a signature").unwrap();

        let findings = found(root);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].title, "proj — cache (tool-cache/)");
    }

    #[test]
    fn linked_worktrees_are_not_walked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("wt")).unwrap();
        fs::write(root.join("wt/.git"), "gitdir: /repo/.git/worktrees/wt\n").unwrap();
        touch(&root.join("wt/package.json"));
        touch(&root.join("wt/node_modules/a/index.js"));

        assert!(found(root).is_empty());
    }

    #[test]
    fn parses_git_worktree_listings() {
        let porcelain = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /repo/.claude/worktrees/x\nHEAD def\nbranch refs/heads/feature/x\n\nworktree /tmp/detached\nHEAD 123\ndetached\n";
        assert_eq!(
            parse_worktrees(porcelain),
            vec![
                Worktree {
                    path: "/repo".into(),
                    branch: Some("main".into())
                },
                Worktree {
                    path: "/repo/.claude/worktrees/x".into(),
                    branch: Some("feature/x".into())
                },
                Worktree {
                    path: "/tmp/detached".into(),
                    branch: None
                },
            ]
        );
    }
}
