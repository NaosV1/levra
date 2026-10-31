//! Whether a file or folder name belongs to an app.

use super::MatchReason;
use crate::apps::AppInfo;

/// Executable names too generic to identify an app by (Electron apps).
const GENERIC_NAMES: &[&str] = &["electron", "app", "main", "launcher", "helper", "updater", "run"];
const FILE_SUFFIXES: &[&str] = &[".plist", ".savedstate", ".binarycookies", ".bom"];
/// Bundle id prefixes used by many unrelated publishers or by the system.
const SHARED_VENDORS: &[&str] = &[
    "com.apple.",
    "com.github.",
    "io.github.",
    "com.electron.",
    "org.chromium.",
];

/// The publisher part of a bundle id or launchd label, lowercased
/// (`com.docker.` for `com.docker.docker`), unless publishers share it.
pub(crate) fn vendor_prefix(id: &str) -> Option<String> {
    let id = id.to_lowercase();
    let parts: Vec<&str> = id.split('.').collect();
    (parts.len() >= 3)
        .then(|| format!("{}.{}.", parts[0], parts[1]))
        .filter(|vendor| !SHARED_VENDORS.contains(&vendor.as_str()))
}

/// Decides whether a file or folder name belongs to the app.
pub(super) struct Matcher {
    bundle_id: String,
    names: Vec<String>,
    vendor: Option<String>,
    other_ids: Vec<String>,
    other_names: Vec<String>,
}

impl Matcher {
    pub(super) fn new(app: &AppInfo, installed: &[AppInfo]) -> Self {
        let bundle_id = app.bundle_id.to_lowercase();
        let others: Vec<&AppInfo> = installed.iter().filter(|o| o.bundle_id != app.bundle_id).collect();
        let other_ids: Vec<String> = others.iter().map(|o| o.bundle_id.to_lowercase()).collect();

        let mut names = vec![app.name.to_lowercase()];
        if let Some(stem) = app.path.file_stem() {
            names.push(stem.to_string_lossy().to_lowercase());
        }
        if let Some(exe) = &app.executable {
            names.push(exe.to_lowercase());
        }
        names.retain(|n| n.len() >= 3 && !GENERIC_NAMES.contains(&n.as_str()));
        names.sort();
        names.dedup();

        let vendor =
            vendor_prefix(&bundle_id).filter(|vendor| !other_ids.iter().any(|id| id.starts_with(vendor.as_str())));

        Self {
            bundle_id,
            names,
            vendor,
            other_names: others.iter().map(|o| o.name.to_lowercase()).collect(),
            other_ids,
        }
    }

    pub(super) fn classify(&self, entry: &str, by_name: bool) -> Option<MatchReason> {
        let lower = entry.to_lowercase();
        let stem = FILE_SUFFIXES
            .iter()
            .find_map(|suffix| lower.strip_suffix(suffix))
            .unwrap_or(&lower);

        let owned_by_other = self.other_ids.iter().any(|other| {
            other.len() > self.bundle_id.len() && (stem == other || stem.starts_with(&format!("{other}.")))
        });
        if owned_by_other {
            return None;
        }
        if stem == self.bundle_id || stem.starts_with(&format!("{}.", self.bundle_id)) {
            return Some(MatchReason::BundleId);
        }
        if by_name && self.names.iter().any(|n| n == stem) && !self.other_names.iter().any(|n| n == stem) {
            return Some(MatchReason::Name);
        }
        if let Some(vendor) = &self.vendor
            && (stem.starts_with(vendor.as_str()) || Some(stem) == vendor.strip_suffix('.'))
        {
            return Some(MatchReason::Vendor);
        }
        if self.is_similar(stem, by_name) {
            return Some(MatchReason::Similar);
        }
        None
    }

    /// `Docker Desktop` or `docker-secrets-engine` for Docker, and the
    /// default Electron id `com.electron.dockerdesktop`. A longer name of
    /// another installed app wins (`Foo Pro` is not a leftover of `Foo`).
    fn is_similar(&self, stem: &str, by_name: bool) -> bool {
        let starts_like = |name: &str| {
            name.len() >= 4
                && stem
                    .strip_prefix(name)
                    .is_some_and(|rest| rest.starts_with([' ', '-', '_', '.']))
        };
        let shortest_name = self.names.iter().map(String::len).min().unwrap_or(0);
        let other_owns = self
            .other_names
            .iter()
            .any(|other| other.len() > shortest_name && (stem == other || starts_like(other)));
        let by_prefix = by_name && self.names.iter().any(|n| starts_like(n));
        let electron = stem.strip_prefix("com.electron.").is_some_and(|rest| {
            self.names
                .iter()
                .any(|n| n.len() >= 4 && rest.starts_with(&n.replace(' ', "")))
        });
        (by_prefix || electron) && !other_owns
    }
}
