//! Launch agents and daemons belonging to an app.

use super::matcher::Matcher;
use super::{Leftover, LeftoverKind, MatchReason, Roots, entries, leftover};
use crate::apps::AppInfo;
use crate::launchd::{self, Job, Scope};

/// Launch agents and daemons that start the app, or whose label or
/// associated bundle id names it. A daemon's privileged helper goes too.
pub(super) fn jobs(roots: &Roots, app: &AppInfo, matcher: &Matcher) -> Vec<Leftover> {
    let app_prefix = format!("{}/", app.path.display());
    let mut items = Vec::new();
    for (folder, scope) in launchd::folders(&roots.home, &roots.system) {
        let kind = match scope {
            Scope::UserAgent | Scope::Agent => LeftoverKind::LaunchAgent,
            Scope::Daemon => LeftoverKind::LaunchDaemon,
        };
        for plist_path in entries(&folder) {
            let Some(job) = Job::read(&plist_path) else {
                continue;
            };
            let starts_app = job
                .program
                .iter()
                .chain(job.program_arguments.first())
                .any(|program| program.starts_with(&app_prefix));
            // Jobs registered with SMAppService name their app this way;
            // their `BundleProgram` is relative to that app's bundle.
            let associated = job.associated_ids().contains(&app.bundle_id.as_str());
            let by_label = job.label.as_deref().and_then(|label| matcher.classify(label, false));
            let reason = if starts_app || associated {
                MatchReason::BundleId
            } else if let Some(reason) = by_label {
                reason
            } else {
                continue;
            };

            let mut item = leftover(plist_path, kind, reason);
            item.launchd_label = job.label.clone();
            if kind == LeftoverKind::LaunchDaemon {
                item.note = Some("Service système : il s'arrête au prochain redémarrage.".to_owned());
                if let Some(helper) = job
                    .label
                    .as_deref()
                    .and_then(|label| launchd::helper_tool(&roots.system, label))
                {
                    items.push(leftover(helper, LeftoverKind::HelperTool, reason));
                }
            }
            items.push(item);
        }
    }
    items
}
