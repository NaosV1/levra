//! launchd jobs installed as property lists: agents of the user, agents of
//! every user, and system daemons.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::tools;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// `~/Library/LaunchAgents`: runs in the user's session.
    UserAgent,
    /// `/Library/LaunchAgents`: runs in the session of every user.
    Agent,
    /// `/Library/LaunchDaemons`: runs as root, outside any session.
    Daemon,
}

/// The folders launchd loads jobs from. `system` is `/` in production.
pub fn folders(home: &Path, system: &Path) -> [(PathBuf, Scope); 3] {
    [
        (home.join("Library/LaunchAgents"), Scope::UserAgent),
        (system.join("Library/LaunchAgents"), Scope::Agent),
        (system.join("Library/LaunchDaemons"), Scope::Daemon),
    ]
}

/// The keys of a job's property list that Levra reads. Scheduling keys
/// are read as any value, so that one of an unexpected type does not make
/// the whole file unreadable.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Job {
    pub label: Option<String>,
    pub program: Option<String>,
    #[serde(default)]
    pub program_arguments: Vec<String>,
    /// Jobs registered with SMAppService name their app this way.
    associated_bundle_identifiers: Option<plist::Value>,
    run_at_load: Option<plist::Value>,
    /// `true`, or the conditions under which launchd restarts the job.
    keep_alive: Option<plist::Value>,
    start_interval: Option<plist::Value>,
    start_calendar_interval: Option<plist::Value>,
}

impl Job {
    /// `None` when the file is not a job this process can read (daemons
    /// readable only by root, files that are not property lists).
    pub fn read(path: &Path) -> Option<Self> {
        plist::from_file(path).ok()
    }

    /// The executable launchd starts.
    pub fn program(&self) -> Option<&str> {
        self.program
            .as_deref()
            .or_else(|| self.program_arguments.first().map(String::as_str))
    }

    pub fn associated_ids(&self) -> Vec<&str> {
        match &self.associated_bundle_identifiers {
            Some(plist::Value::String(id)) => vec![id.as_str()],
            Some(plist::Value::Array(ids)) => ids.iter().filter_map(plist::Value::as_string).collect(),
            _ => Vec::new(),
        }
    }

    pub fn keeps_alive(&self) -> bool {
        match &self.keep_alive {
            Some(plist::Value::Boolean(on)) => *on,
            Some(plist::Value::Dictionary(_)) => true,
            _ => false,
        }
    }

    pub fn runs_on_calendar(&self) -> bool {
        self.start_calendar_interval.is_some()
    }

    pub fn runs_at_load(&self) -> bool {
        matches!(self.run_at_load, Some(plist::Value::Boolean(true)))
    }

    /// Seconds between runs.
    pub fn start_interval(&self) -> Option<u64> {
        self.start_interval.as_ref().and_then(plist::Value::as_unsigned_integer)
    }
}

/// The user whose session runs the agents: the owner of the home folder.
pub fn session_user(home: &Path) -> Option<u32> {
    fs::metadata(home).map(|m| m.uid()).ok()
}

/// Whether launchd has loaded the agent into the user's session. Only root
/// can ask the same about daemons.
pub fn agent_loaded(label: &str, uid: u32, home: &Path) -> bool {
    let target = format!("gui/{uid}/{label}");
    tools::output(
        Path::new("/bin/launchctl"),
        &["print", &target],
        home,
        Duration::from_secs(5),
    )
    .is_some()
}

/// Stops an agent of the user's session before its file goes away. Best
/// effort: an agent that is not loaded is simply skipped.
pub fn stop_agent(label: &str, uid: u32, home: &Path) {
    let target = format!("gui/{uid}/{label}");
    let _ = tools::output(
        Path::new("/bin/launchctl"),
        &["bootout", &target],
        home,
        Duration::from_secs(10),
    );
}

/// The privileged helper a daemon installed with SMJobBless runs: it is
/// named after the daemon's label.
pub fn helper_tool(system: &Path, label: &str) -> Option<PathBuf> {
    if label.is_empty() || label.contains('/') {
        return None;
    }
    let helper = system.join("Library/PrivilegedHelperTools").join(label);
    helper.exists().then_some(helper)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(xml_body: &str) -> Job {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>{xml_body}</dict></plist>"#
        );
        plist::from_bytes(xml.as_bytes()).unwrap()
    }

    #[test]
    fn the_program_is_the_first_argument_without_a_program_key() {
        let with_arguments = job("<key>Label</key><string>com.example.agent</string>
             <key>ProgramArguments</key><array><string>/opt/example/agent</string><string>--wake</string></array>");
        assert_eq!(with_arguments.program(), Some("/opt/example/agent"));

        let with_program = job("<key>Program</key><string>/opt/example/daemon</string>
             <key>ProgramArguments</key><array><string>daemon</string></array>");
        assert_eq!(with_program.program(), Some("/opt/example/daemon"));
    }

    #[test]
    fn reads_associated_bundle_ids_as_a_string_or_a_list() {
        let one = job("<key>AssociatedBundleIdentifiers</key><string>com.example.app</string>");
        assert_eq!(one.associated_ids(), vec!["com.example.app"]);
        let many = job(
            "<key>AssociatedBundleIdentifiers</key><array><string>com.example.a</string><string>com.example.b</string></array>",
        );
        assert_eq!(many.associated_ids(), vec!["com.example.a", "com.example.b"]);
    }

    #[test]
    fn a_scheduling_key_of_an_unexpected_type_does_not_hide_the_job() {
        let odd = job("<key>Label</key><string>com.example.odd</string>
             <key>RunAtLoad</key><string>yes</string>
             <key>StartInterval</key><string>3600</string>");
        assert_eq!(odd.label.as_deref(), Some("com.example.odd"));
        assert!(!odd.runs_at_load());
        assert_eq!(odd.start_interval(), None);
        assert_eq!(
            job("<key>StartInterval</key><integer>600</integer>").start_interval(),
            Some(600)
        );
    }

    #[test]
    fn keep_alive_conditions_count_as_always_running() {
        assert!(job("<key>KeepAlive</key><true/>").keeps_alive());
        assert!(job("<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>").keeps_alive());
        assert!(!job("<key>KeepAlive</key><false/>").keeps_alive());
        assert!(!job("").keeps_alive());
    }

    #[test]
    fn helper_tools_are_looked_up_by_label_only_inside_their_folder() {
        let dir = crate::testing::tempdir();
        let tools = dir.path().join("Library/PrivilegedHelperTools");
        fs::create_dir_all(&tools).unwrap();
        fs::write(tools.join("com.example.helper"), b"bin").unwrap();

        assert_eq!(
            helper_tool(dir.path(), "com.example.helper"),
            Some(tools.join("com.example.helper"))
        );
        assert_eq!(helper_tool(dir.path(), "com.example.missing"), None);
        assert_eq!(
            helper_tool(dir.path(), "../PrivilegedHelperTools/com.example.helper"),
            None
        );
    }
}
