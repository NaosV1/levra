//! Xcode build data, device symbols, archives, simulators and runtimes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use super::{Category, Finding, Safety, ScanOptions, deletable, file_name, subdirectories};
use crate::execute::Action;
use crate::tools;
use crate::walk::ScanState;

const XCRUN: &str = "/usr/bin/xcrun";
/// An erased simulator still keeps a few megabytes of fresh data.
const ERASED_SIMULATOR_BYTES: u64 = 20_000_000;
const HEAVY_SIMULATOR_BYTES: u64 = 1_000_000_000;

pub(super) fn detect(options: &ScanOptions, state: &ScanState) -> Vec<Finding> {
    let developer = options.home.join("Library/Developer");
    if !developer.is_dir() {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for (rel, title, detail) in [
        (
            "Xcode/DerivedData",
            "DerivedData Xcode",
            "Index et builds intermédiaires, recréés au prochain build (plus long).",
        ),
        (
            "CoreSimulator/Caches",
            "Caches des simulateurs",
            "Recréés au prochain démarrage d'un simulateur.",
        ),
        (
            "Xcode/UserData/Previews",
            "Simulateurs des aperçus SwiftUI",
            "Recréés à la prochaine ouverture d'un aperçu.",
        ),
    ] {
        let path = developer.join(rel);
        if path.is_dir() {
            findings.push(deletable(
                state,
                Category::Xcode,
                path,
                title.to_owned(),
                detail.to_owned(),
                Safety::Safe,
            ));
        }
    }

    for platform_dir in subdirectories(&developer.join("Xcode"))
        .into_iter()
        .filter(|dir| file_name(dir).ends_with("DeviceSupport"))
    {
        for version in subdirectories(&platform_dir) {
            findings.push(deletable(
                state,
                Category::Xcode,
                version.clone(),
                format!("Symboles de l'appareil {}", file_name(&version)),
                "Recopiés depuis l'appareil la prochaine fois qu'il est branché (quelques minutes).".to_owned(),
                Safety::Review,
            ));
        }
    }

    let archives = developer.join("Xcode/Archives");
    let archive_days = subdirectories(&archives);
    if !archive_days.is_empty() {
        findings.push(Finding {
            id: format!("path:{}", archives.display()),
            category: Category::Xcode,
            title: "Archives Xcode".to_owned(),
            detail: "Builds envoyés sur l'App Store ou exportés. Garde-les si tu dois symboliser des crashs de ces versions."
                .to_owned(),
            bytes: state.allocated_size(&archives),
            safety: Safety::Careful,
            action: Action::Trash { paths: archive_days },
            last_used: None,
        });
    }

    let xcrun = Path::new(XCRUN);
    if xcrun.is_file() {
        if let Some(json) = tools::output(
            xcrun,
            &["simctl", "list", "-j", "devices"],
            &options.home,
            Duration::from_secs(20),
        ) {
            findings.extend(simulator_findings(&json));
        }
        if let Some(json) = tools::output(
            xcrun,
            &["simctl", "runtime", "list", "-j"],
            &options.home,
            Duration::from_secs(20),
        ) {
            findings.extend(runtime_findings(&json));
        }
    }
    findings
}

#[derive(Deserialize)]
struct DeviceList {
    devices: BTreeMap<String, Vec<Device>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Device {
    udid: String,
    name: String,
    state: String,
    is_available: bool,
    #[serde(default)]
    data_path_size: u64,
    last_used_at: Option<String>,
}

fn simulator_findings(json: &str) -> Vec<Finding> {
    let Ok(list) = serde_json::from_str::<DeviceList>(json) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    let mut unavailable_bytes = 0;
    for (runtime, devices) in &list.devices {
        for device in devices {
            if !device.is_available {
                unavailable_bytes += device.data_path_size;
            } else if device.state == "Shutdown"
                && device.data_path_size >= HEAVY_SIMULATOR_BYTES
                && is_uuid(&device.udid)
            {
                findings.push(Finding {
                    id: format!("simulator:{}", device.udid),
                    category: Category::Xcode,
                    title: format!("Simulateur {} ({})", device.name, runtime_label(runtime)),
                    detail: "L'effacer le remet à neuf : les apps installées et leurs données disparaissent."
                        .to_owned(),
                    bytes: device.data_path_size.saturating_sub(ERASED_SIMULATOR_BYTES),
                    safety: Safety::Review,
                    action: Action::Command {
                        program: PathBuf::from(XCRUN),
                        args: vec!["simctl".into(), "erase".into(), device.udid.clone()],
                    },
                    last_used: device.last_used_at.as_deref().and_then(crate::time::parse_utc),
                });
            }
        }
    }
    if unavailable_bytes > 0 {
        findings.push(Finding {
            id: "simulators:unavailable".to_owned(),
            category: Category::Xcode,
            title: "Simulateurs indisponibles".to_owned(),
            detail: "Simulateurs dont le runtime n'est plus installé : ils ne peuvent plus démarrer.".to_owned(),
            bytes: unavailable_bytes,
            safety: Safety::Safe,
            action: Action::Command {
                program: PathBuf::from(XCRUN),
                args: vec!["simctl".into(), "delete".into(), "unavailable".into()],
            },
            last_used: None,
        });
    }
    findings
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Runtime {
    identifier: String,
    version: String,
    platform_identifier: String,
    #[serde(default)]
    size_bytes: u64,
    #[serde(default)]
    deletable: bool,
    last_used_at: Option<String>,
}

/// Every deletable runtime except the newest of each platform.
fn runtime_findings(json: &str) -> Vec<Finding> {
    let Ok(runtimes) = serde_json::from_str::<BTreeMap<String, Runtime>>(json) else {
        return Vec::new();
    };
    let mut by_platform: BTreeMap<&str, Vec<&Runtime>> = BTreeMap::new();
    for runtime in runtimes.values() {
        by_platform
            .entry(&runtime.platform_identifier)
            .or_default()
            .push(runtime);
    }

    let mut findings = Vec::new();
    for platform in by_platform.values_mut() {
        platform.sort_by_key(|runtime| std::cmp::Reverse(version_key(&runtime.version)));
        let newest = platform[0];
        for runtime in &platform[1..] {
            if !runtime.deletable || !is_uuid(&runtime.identifier) {
                continue;
            }
            let name = platform_name(&runtime.platform_identifier);
            findings.push(Finding {
                id: format!("runtime:{}", runtime.identifier),
                category: Category::Xcode,
                title: format!("Runtime {name} {}", runtime.version),
                detail: format!(
                    "Tu gardes {name} {}. Les simulateurs {name} {} deviendront indisponibles.",
                    newest.version, runtime.version
                ),
                bytes: runtime.size_bytes,
                safety: Safety::Review,
                action: Action::Command {
                    program: PathBuf::from(XCRUN),
                    args: vec![
                        "simctl".into(),
                        "runtime".into(),
                        "delete".into(),
                        runtime.identifier.clone(),
                    ],
                },
                last_used: runtime.last_used_at.as_deref().and_then(crate::time::parse_utc),
            });
        }
    }
    findings
}

fn version_key(version: &str) -> Vec<u32> {
    version.split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

/// `com.apple.CoreSimulator.SimRuntime.iOS-27-0` → `iOS 27.0`.
fn runtime_label(runtime: &str) -> String {
    let short = runtime.rsplit('.').next().unwrap_or(runtime);
    match short.split_once('-') {
        Some((platform, version)) => format!("{platform} {}", version.replace('-', ".")),
        None => short.to_owned(),
    }
}

fn platform_name(identifier: &str) -> &str {
    match identifier {
        "com.apple.platform.iphonesimulator" => "iOS",
        "com.apple.platform.watchsimulator" => "watchOS",
        "com.apple.platform.appletvsimulator" => "tvOS",
        "com.apple.platform.xrsimulator" => "visionOS",
        other => other,
    }
}

/// Identifiers end up as command arguments: only accept the expected shape.
fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.chars().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICES: &str = r#"{"devices": {
        "com.apple.CoreSimulator.SimRuntime.iOS-27-0": [
            {"udid": "B01D04BE-C265-4B11-B857-922FA31CC3AD", "name": "iPhone 18 Pro Max", "state": "Shutdown",
             "isAvailable": true, "dataPathSize": 3054723072, "lastUsedAt": "2026-09-24T21:48:39Z"},
            {"udid": "C4E527F9-681F-472E-9945-450B6F7DDFD6", "name": "iPhone 17", "state": "Shutdown",
             "isAvailable": true, "dataPathSize": 17000000},
            {"udid": "F7E50F9D-0E12-4E7B-B428-448E71CA7544", "name": "iPad", "state": "Booted",
             "isAvailable": true, "dataPathSize": 5000000000}
        ],
        "com.apple.CoreSimulator.SimRuntime.iOS-17-0": [
            {"udid": "1D195081-4D76-466F-AE24-7D5CB2470B58", "name": "iPhone 15", "state": "Shutdown",
             "isAvailable": false, "dataPathSize": 400000000}
        ]
    }}"#;

    const RUNTIMES: &str = r#"{
        "BF2FA13B-5D54-445F-B78B-691E89BB7630": {"identifier": "BF2FA13B-5D54-445F-B78B-691E89BB7630",
            "version": "27.0", "platformIdentifier": "com.apple.platform.iphonesimulator",
            "sizeBytes": 8000000000, "deletable": true},
        "910098F9-D852-4ECE-9DE8-E54A1378A776": {"identifier": "910098F9-D852-4ECE-9DE8-E54A1378A776",
            "version": "26.5", "platformIdentifier": "com.apple.platform.iphonesimulator",
            "sizeBytes": 8494282293, "deletable": true, "lastUsedAt": "2026-10-21T20:09:22Z"},
        "AAAAAAAA-5D54-445F-B78B-691E89BB7630": {"identifier": "AAAAAAAA-5D54-445F-B78B-691E89BB7630",
            "version": "11.0", "platformIdentifier": "com.apple.platform.watchsimulator",
            "sizeBytes": 3000000000, "deletable": true}
    }"#;

    #[test]
    fn heavy_shutdown_simulators_can_be_erased() {
        let findings = simulator_findings(DEVICES);
        let erase: Vec<_> = findings.iter().filter(|f| f.id.starts_with("simulator:")).collect();
        assert_eq!(erase.len(), 1, "booted and light simulators are left alone");
        assert_eq!(erase[0].title, "Simulateur iPhone 18 Pro Max (iOS 27.0)");
        assert_eq!(erase[0].last_used, Some(1_790_286_519));
        assert_eq!(
            erase[0].action.command_line().unwrap(),
            "xcrun simctl erase B01D04BE-C265-4B11-B857-922FA31CC3AD"
        );
    }

    #[test]
    fn unavailable_simulators_are_grouped() {
        let findings = simulator_findings(DEVICES);
        let unavailable = findings.iter().find(|f| f.id == "simulators:unavailable").unwrap();
        assert_eq!(unavailable.bytes, 400_000_000);
        assert_eq!(unavailable.safety, Safety::Safe);
    }

    #[test]
    fn only_older_runtimes_of_each_platform_are_proposed() {
        let findings = runtime_findings(RUNTIMES);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].title, "Runtime iOS 26.5");
        assert_eq!(findings[0].bytes, 8_494_282_293);
        assert!(findings[0].detail.starts_with("Tu gardes iOS 27.0."));
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(version_key("26.10") > version_key("26.9"));
    }

    #[test]
    fn identifiers_must_be_uuids() {
        assert!(is_uuid("910098F9-D852-4ECE-9DE8-E54A1378A776"));
        assert!(!is_uuid("910098F9-D852-4ECE-9DE8-E54A1378A77; rm"));
        assert!(!is_uuid("unavailable"));
    }

    #[test]
    fn malformed_json_yields_nothing() {
        assert!(simulator_findings("not json").is_empty());
        assert!(runtime_findings("{}").is_empty());
    }
}
