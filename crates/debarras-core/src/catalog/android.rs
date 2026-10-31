//! Android emulators, system images and NDK versions.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Category, Finding, Safety, ScanOptions, deletable, file_name, subdirectories};
use crate::execute::Action;
use crate::walk::ScanState;

pub(super) fn detect(options: &ScanOptions, state: &ScanState) -> Vec<Finding> {
    findings_for(options, state, &sdk_roots(&options.home))
}

fn findings_for(options: &ScanOptions, state: &ScanState, sdks: &[PathBuf]) -> Vec<Finding> {
    let avds: Vec<PathBuf> = subdirectories(&options.home.join(".android/avd"))
        .into_iter()
        .filter(|dir| dir.extension().is_some_and(|ext| ext == "avd"))
        .collect();

    let mut findings = Vec::new();
    let mut used_images = HashSet::new();
    for avd in &avds {
        let name = avd_name(avd);
        if let Some(image) = system_image_of(avd) {
            used_images.insert(image);
        }
        let snapshots = avd.join("snapshots");
        if snapshots.is_dir() {
            findings.push(deletable(
                state,
                Category::Android,
                snapshots,
                format!("Snapshots Quick Boot de {name}"),
                "Le prochain démarrage sera à froid. Un nouveau snapshot est enregistré à la fermeture tant que Quick Boot reste activé."
                    .to_owned(),
                Safety::Safe,
            ));
        }
        let ini = avd.with_extension("ini");
        findings.push(Finding {
            id: format!("avd:{}", avd.display()),
            category: Category::Android,
            title: format!("Émulateur {name}"),
            detail: "Supprime l'émulateur avec les apps et données installées dedans.".to_owned(),
            bytes: state.allocated_size(avd),
            safety: Safety::Review,
            action: Action::Trash {
                paths: [avd.clone(), ini].into_iter().filter(|p| p.exists()).collect(),
            },
            last_used: None,
        });
    }

    for sdk in sdks {
        findings.extend(unused_system_images(state, sdk, &used_images));
        findings.extend(older_ndks(state, sdk));
    }
    findings
}

fn avd_name(avd: &Path) -> String {
    avd.file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().replace('_', " "))
}

fn sdk_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .chain([
            home.join("Library/Android/sdk"),
            PathBuf::from("/opt/homebrew/share/android-commandlinetools"),
            PathBuf::from("/usr/local/share/android-commandlinetools"),
        ])
        .filter(|root| root.is_dir())
        .filter_map(|root| fs::canonicalize(root).ok())
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

/// `image.sysdir.1=system-images/android-36/google_apis/arm64-v8a/` →
/// `system-images/android-36/google_apis/arm64-v8a`.
fn system_image_of(avd: &Path) -> Option<String> {
    let config = fs::read_to_string(avd.join("config.ini")).ok()?;
    config.lines().find_map(|line| {
        let value = line.strip_prefix("image.sysdir.1")?.trim_start().strip_prefix('=')?;
        Some(value.trim().trim_end_matches('/').to_owned())
    })
}

fn unused_system_images(state: &ScanState, sdk: &Path, used: &HashSet<String>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for api in subdirectories(&sdk.join("system-images")) {
        for tag in subdirectories(&api) {
            for abi in subdirectories(&tag) {
                let relative = format!(
                    "system-images/{}/{}/{}",
                    file_name(&api),
                    file_name(&tag),
                    file_name(&abi)
                );
                if used.contains(&relative) {
                    continue;
                }
                findings.push(deletable(
                    state,
                    Category::Android,
                    abi,
                    format!(
                        "Image système Android {} {} inutilisée",
                        file_name(&api),
                        file_name(&tag)
                    ),
                    "Aucun émulateur ne l'utilise. Retéléchargeable avec sdkmanager.".to_owned(),
                    Safety::Review,
                ));
            }
        }
    }
    findings
}

fn older_ndks(state: &ScanState, sdk: &Path) -> Vec<Finding> {
    let mut versions = subdirectories(&sdk.join("ndk"));
    if versions.len() < 2 {
        return Vec::new();
    }
    versions.sort_by_key(|dir| version_key(&file_name(dir)));
    let newest = versions.pop().map(|dir| file_name(&dir)).unwrap_or_default();
    versions
        .into_iter()
        .map(|dir| {
            let version = file_name(&dir);
            deletable(
                state,
                Category::Android,
                dir,
                format!("NDK Android {version}"),
                format!("Tu gardes le NDK {newest}. Vérifie qu'aucun projet n'exige ndkVersion = \"{version}\"."),
                Safety::Review,
            )
        })
        .collect()
}

fn version_key(version: &str) -> Vec<u64> {
    version.split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn reads_the_system_image_of_an_avd() {
        let dir = tempfile::tempdir().unwrap();
        let avd = dir.path().join("Pixel_9.avd");
        write(
            &avd.join("config.ini"),
            "hw.ramSize=2048\nimage.sysdir.1 = system-images/android-36/google_apis/arm64-v8a/\n",
        );
        assert_eq!(
            system_image_of(&avd).as_deref(),
            Some("system-images/android-36/google_apis/arm64-v8a")
        );
    }

    #[test]
    fn only_unused_system_images_are_proposed() {
        let sdk = tempfile::tempdir().unwrap();
        write(
            &sdk.path()
                .join("system-images/android-36/google_apis/arm64-v8a/system.img"),
            "img",
        );
        write(
            &sdk.path()
                .join("system-images/android-36/google_apis_playstore/arm64-v8a/system.img"),
            "img",
        );
        let used = HashSet::from(["system-images/android-36/google_apis/arm64-v8a".to_owned()]);

        let findings = unused_system_images(&ScanState::default(), sdk.path(), &used);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].title,
            "Image système Android android-36 google_apis_playstore inutilisée"
        );
    }

    #[test]
    fn keeps_the_newest_ndk() {
        let sdk = tempfile::tempdir().unwrap();
        for version in ["27.0.12077973", "27.1.12297006", "26.3.11579264"] {
            write(&sdk.path().join("ndk").join(version).join("source.properties"), "");
        }
        let mut titles: Vec<String> = older_ndks(&ScanState::default(), sdk.path())
            .into_iter()
            .map(|f| f.title)
            .collect();
        titles.sort();
        assert_eq!(titles, vec!["NDK Android 26.3.11579264", "NDK Android 27.0.12077973"]);
    }

    #[test]
    fn a_single_ndk_is_kept() {
        let sdk = tempfile::tempdir().unwrap();
        write(&sdk.path().join("ndk/27.1.12297006/source.properties"), "");
        assert!(older_ndks(&ScanState::default(), sdk.path()).is_empty());
    }

    #[test]
    fn avds_offer_snapshot_cleanup_and_removal() {
        let home = tempfile::tempdir().unwrap();
        let avd = home.path().join(".android/avd/Pixel_9_Play.avd");
        write(
            &avd.join("config.ini"),
            "image.sysdir.1=system-images/android-36/google_apis_playstore/arm64-v8a/\n",
        );
        write(&avd.join("snapshots/default_boot/ram.bin"), "ram");
        write(&home.path().join(".android/avd/Pixel_9_Play.ini"), "path=...");
        let options = ScanOptions {
            home: home.path().to_path_buf(),
            project_roots: Vec::new(),
            min_bytes: 0,
        };

        let findings = findings_for(&options, &ScanState::default(), &[]);
        let titles: Vec<&str> = findings.iter().map(|f| f.title.as_str()).collect();
        assert!(titles.contains(&"Snapshots Quick Boot de Pixel 9 Play"), "{titles:?}");
        let removal = findings.iter().find(|f| f.title == "Émulateur Pixel 9 Play").unwrap();
        assert_eq!(removal.action.paths().len(), 2, "the .avd folder and its .ini");
    }
}
