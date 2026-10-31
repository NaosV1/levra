//! Package-manager caches and per-application caches.

use std::path::Path;

use rayon::prelude::*;

use super::{Category, Finding, Safety, ScanOptions, deletable, file_name, subdirectories};
use crate::execute::Action;
use crate::tools;
use crate::walk::ScanState;

enum Cleanup {
    Delete,
    /// Prefer the owning tool when installed; it knows what is still in use.
    Tool {
        name: &'static str,
        args: &'static [&'static str],
        /// Whether removing the folder is an acceptable stand-in.
        delete_without_tool: bool,
    },
}

struct KnownCache {
    /// Relative to the home folder.
    path: &'static str,
    title: &'static str,
    detail: &'static str,
    safety: Safety,
    cleanup: Cleanup,
}

const KNOWN: &[KnownCache] = &[
    KnownCache {
        path: ".npm/_cacache",
        title: "Cache npm",
        detail: "Retéléchargé à la demande par npm.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".npm/_npx",
        title: "Paquets temporaires npx",
        detail: "Retéléchargés au prochain npx.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "Library/pnpm/store",
        title: "Store pnpm",
        detail: "pnpm store prune ne retire que les paquets qu'aucun projet n'utilise.",
        safety: Safety::Safe,
        cleanup: Cleanup::Tool {
            name: "pnpm",
            args: &["store", "prune"],
            delete_without_tool: false,
        },
    },
    KnownCache {
        path: "Library/Caches/pnpm",
        title: "Métadonnées pnpm",
        detail: "Retéléchargées à la demande par pnpm.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "Library/Caches/Yarn",
        title: "Cache Yarn",
        detail: "Retéléchargé à la demande par Yarn.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".bun/install/cache",
        title: "Cache Bun",
        detail: "Retéléchargé à la demande par Bun.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".gradle/caches",
        title: "Cache Gradle",
        detail: "Dépendances et builds retéléchargés ou recompilés par Gradle.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".cargo/registry/cache",
        title: "Archives de crates Rust",
        detail: "Retéléchargées par cargo au besoin.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".cargo/registry/src",
        title: "Sources de crates Rust décompressées",
        detail: "Redécompressées par cargo au besoin.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".cargo/git/checkouts",
        title: "Dépendances git de cargo",
        detail: "Reclonées par cargo au besoin.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "Library/Caches/Homebrew",
        title: "Cache Homebrew",
        detail: "Anciennes versions et téléchargements de Homebrew.",
        safety: Safety::Safe,
        cleanup: Cleanup::Tool {
            name: "brew",
            args: &["cleanup", "--prune=all", "-s"],
            delete_without_tool: true,
        },
    },
    KnownCache {
        path: "Library/Caches/pip",
        title: "Cache pip",
        detail: "Retéléchargé à la demande par pip.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".cache/pip",
        title: "Cache pip",
        detail: "Retéléchargé à la demande par pip.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".cache/uv",
        title: "Cache uv",
        detail: "Retéléchargé à la demande par uv.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "Library/Caches/go-build",
        title: "Cache de compilation Go",
        detail: "Recompilé au prochain go build.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "go/pkg/mod",
        title: "Modules Go",
        detail: "Retéléchargés par go au besoin.",
        safety: Safety::Review,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "Library/Caches/CocoaPods",
        title: "Cache CocoaPods",
        detail: "Retéléchargé au prochain pod install.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "Library/Caches/ms-playwright",
        title: "Navigateurs Playwright",
        detail: "À réinstaller avec npx playwright install avant les prochains tests.",
        safety: Safety::Review,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".cache/huggingface",
        title: "Modèles HuggingFace",
        detail: "Retéléchargés à la prochaine utilisation, souvent plusieurs Go.",
        safety: Safety::Review,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".m2/repository",
        title: "Dépôt Maven local",
        detail: "Retéléchargé par Maven au besoin.",
        safety: Safety::Review,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "Library/Caches/JetBrains",
        title: "Caches JetBrains",
        detail: "Index reconstruits à la prochaine ouverture de l'IDE.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: ".expo/ios-simulator-app-cache",
        title: "Expo Go pour simulateur",
        detail: "Retéléchargé au prochain lancement sur simulateur.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "Library/Application Support/Code/CachedExtensionVSIXs",
        title: "Paquets d'extensions VS Code",
        detail: "Copies d'installation des extensions, déjà installées.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
    KnownCache {
        path: "Library/Application Support/Code/CachedData",
        title: "Cache VS Code",
        detail: "Recréé par VS Code.",
        safety: Safety::Safe,
        cleanup: Cleanup::Delete,
    },
];

pub(super) fn detect(options: &ScanOptions, state: &ScanState) -> Vec<Finding> {
    let mut findings: Vec<Finding> = KNOWN
        .par_iter()
        .filter_map(|known| known_finding(options, state, known))
        .collect();
    findings.extend(application_caches(options, state));
    findings
}

fn known_finding(options: &ScanOptions, state: &ScanState, known: &KnownCache) -> Option<Finding> {
    let path = options.home.join(known.path);
    if !path.is_dir() {
        return None;
    }
    let finding = deletable(
        state,
        Category::PackageCaches,
        path,
        known.title.to_owned(),
        known.detail.to_owned(),
        known.safety,
    );
    match known.cleanup {
        Cleanup::Delete => Some(finding),
        Cleanup::Tool {
            name,
            args,
            delete_without_tool,
        } => match tools::find(name, &options.home) {
            Some(program) => Some(Finding {
                id: format!("tool:{name}:{}", known.path),
                action: Action::Command {
                    program,
                    args: args.iter().map(|a| (*a).to_owned()).collect(),
                },
                ..finding
            }),
            None => delete_without_tool.then_some(finding),
        },
    }
}

/// Apple caches that do not carry the `com.apple.` prefix.
const APPLE_CACHES: &[&str] = &[
    "CloudKit",
    "FamilyCircle",
    "familycircled",
    "GeoServices",
    "Metadata",
    "PassKit",
];

/// One finding per application cache folder in `~/Library/Caches`.
/// Apple's own caches are left alone: system daemons keep them open, and
/// macOS denies reading several of them anyway.
fn application_caches(options: &ScanOptions, state: &ScanState) -> Vec<Finding> {
    let caches = options.home.join("Library/Caches");
    let known: Vec<&Path> = KNOWN.iter().map(|k| Path::new(k.path)).collect();

    subdirectories(&caches)
        .into_par_iter()
        .filter(|dir| {
            let name = file_name(dir);
            !name.starts_with("com.apple.")
                && !APPLE_CACHES.contains(&name.as_str())
                && !known.iter().any(|k| options.home.join(k).starts_with(dir))
                && std::fs::read_dir(dir).is_ok()
        })
        .map(|dir| {
            let name = file_name(&dir);
            deletable(
                state,
                Category::AppCaches,
                dir,
                format!("Cache de {name}"),
                "Recréé par l'application. Ferme-la avant pour éviter qu'elle le réécrive aussitôt.".to_owned(),
                Safety::Safe,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn home_with(paths: &[&str]) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        for rel in paths {
            fs::create_dir_all(home.path().join(rel)).unwrap();
            fs::write(home.path().join(rel).join("blob"), vec![0u8; 10_000]).unwrap();
        }
        home
    }

    fn titles(home: &Path) -> Vec<String> {
        let options = ScanOptions {
            home: home.to_path_buf(),
            project_roots: Vec::new(),
            min_bytes: 0,
        };
        let mut titles: Vec<String> = detect(&options, &ScanState::default())
            .into_iter()
            .map(|f| f.title)
            .collect();
        titles.sort();
        titles
    }

    #[test]
    fn reports_known_caches_that_exist() {
        let home = home_with(&[".npm/_cacache", ".gradle/caches"]);
        assert_eq!(titles(home.path()), vec!["Cache Gradle", "Cache npm"]);
    }

    #[test]
    fn application_caches_skip_apple_and_known_ones() {
        let home = home_with(&[
            "Library/Caches/com.spotify.client",
            "Library/Caches/com.apple.Safari",
            "Library/Caches/CocoaPods",
        ]);
        assert_eq!(
            titles(home.path()),
            vec!["Cache CocoaPods", "Cache de com.spotify.client"]
        );
    }

    #[test]
    fn caches_live_under_their_own_finding_ids() {
        let home = home_with(&["Library/Caches/com.spotify.client"]);
        let options = ScanOptions {
            home: home.path().to_path_buf(),
            project_roots: Vec::new(),
            min_bytes: 0,
        };
        let findings = detect(&options, &ScanState::default());
        let expected = home.path().join("Library/Caches/com.spotify.client");
        assert_eq!(findings[0].id, format!("path:{}", expected.display()));
        assert_eq!(findings[0].action, Action::Delete { paths: vec![expected] });
    }
}
