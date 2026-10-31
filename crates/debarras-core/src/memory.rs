//! What uses memory, counted the way Activity Monitor does.
//!
//! System figures come from the kernel's VM statistics. "Used" memory is
//! app memory + wired + compressed; the file cache is reported apart
//! because macOS hands it back as soon as an app needs it, which is also
//! why free memory is not a goal: memory pressure and swap are what tell
//! a Mac is short of RAM.
//!
//! Each process is measured by its physical footprint (Activity Monitor's
//! "Memory" column) and grouped under the app it belongs to (by bundle id),
//! so a browser's dozens of helper processes add up to one line. Processes
//! of other users (WindowServer, root daemons) cannot be measured without
//! privileges — `top` can because it is setuid root — so they are counted,
//! not measured; their memory is part of the system figures all the same.

use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::fmt;
use std::io;
use std::mem::{MaybeUninit, size_of};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::OnceLock;

use serde::Serialize;

use crate::apps;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pressure {
    Normal,
    /// macOS compresses and swaps more; apps may slow down.
    Warning,
    /// macOS is short of memory and asks apps to release some.
    Critical,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryStats {
    pub total: u64,
    /// App memory + wired + compressed, Activity Monitor's "Memory Used".
    pub used: u64,
    pub app: u64,
    /// Kept in RAM by the kernel and drivers; cannot be compressed.
    pub wired: u64,
    /// Space the compressor occupies in RAM.
    pub compressed: u64,
    /// File contents cached in RAM, given back on demand.
    pub cached_files: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub pressure: Pressure,
}

pub fn stats() -> io::Result<MemoryStats> {
    // SAFETY: both values are plain integers.
    let total: u64 = unsafe { sysctl("hw.memsize")? };
    // SAFETY: an int, filled by the kernel.
    let level: i32 = unsafe { sysctl("kern.memorystatus_vm_pressure_level")? };
    // SAFETY: `xsw_usage` holds integers only.
    let swap: libc::xsw_usage = unsafe { sysctl("vm.swapusage")? };
    let vm = vm_statistics()?;

    // SAFETY: sysconf has no preconditions.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).map_err(|_| io::Error::last_os_error())?;
    let pages = |count: u32| u64::from(count) * page;
    let app = pages(vm.internal_page_count).saturating_sub(pages(vm.purgeable_count));
    let wired = pages(vm.wire_count);
    let compressed = pages(vm.compressor_page_count);

    Ok(MemoryStats {
        total,
        used: app + wired + compressed,
        app,
        wired,
        compressed,
        cached_files: pages(vm.external_page_count) + pages(vm.purgeable_count),
        swap_used: swap.xsu_used,
        swap_total: swap.xsu_total,
        pressure: match level {
            2 => Pressure::Warning,
            4 => Pressure::Critical,
            _ => Pressure::Normal,
        },
    })
}

/// Reads a fixed-size sysctl value.
///
/// # Safety
/// Every bit pattern must be a valid `T` (integers and structs of them).
unsafe fn sysctl<T>(name: &str) -> io::Result<T> {
    let name = CString::new(name)?;
    let mut value = MaybeUninit::<T>::zeroed();
    let mut len = size_of::<T>();
    // SAFETY: `value` has room for `len` bytes and `name` is NUL-terminated.
    let status = unsafe { libc::sysctlbyname(name.as_ptr(), value.as_mut_ptr().cast(), &mut len, ptr::null_mut(), 0) };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    if len != size_of::<T>() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected sysctl size"));
    }
    // SAFETY: the kernel filled all of it, and the caller guarantees any bytes are a valid `T`.
    Ok(unsafe { value.assume_init() })
}

/// libc steers its Mach bindings to the `mach2` crate; these two calls are
/// long-stable kernel API and not worth a dependency of their own.
#[allow(deprecated)]
fn vm_statistics() -> io::Result<libc::vm_statistics64> {
    // One send right for the process lifetime instead of one per call.
    static HOST: OnceLock<libc::mach_port_t> = OnceLock::new();
    // SAFETY: returns the host port; no preconditions.
    let host = *HOST.get_or_init(|| unsafe { libc::mach_host_self() });
    let mut info = MaybeUninit::<libc::vm_statistics64>::zeroed();
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: `info` is large enough for HOST_VM_INFO64_COUNT integers.
    let status = unsafe { libc::host_statistics64(host, libc::HOST_VM_INFO64, info.as_mut_ptr().cast(), &mut count) };
    if status != libc::KERN_SUCCESS {
        return Err(io::Error::other(format!("host_statistics64 a échoué ({status})")));
    }
    // SAFETY: filled by the kernel; the struct is made of integers.
    Ok(unsafe { info.assume_init() })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessKind {
    /// Runs from an app bundle; quit it like the Dock does.
    App,
    /// A command-line program the user installed (node, java, rustc…).
    Tool,
    /// A virtual machine started by an app (Docker Desktop, the Claude app…).
    VirtualMachine,
    /// Part of macOS.
    System,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub pid: i32,
    pub name: String,
    pub path: PathBuf,
    pub footprint: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessGroup {
    /// The app bundle the processes run from, or the executable.
    pub path: PathBuf,
    /// For apps: what AppKit needs to quit them.
    pub bundle_id: Option<String>,
    pub name: String,
    pub kind: ProcessKind,
    pub footprint: u64,
    pub processes: Vec<ProcessInfo>,
    /// Whether Débarras offers to quit it (never macOS itself, a virtual
    /// machine, or Débarras).
    pub quittable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemorySnapshot {
    pub stats: MemoryStats,
    /// Largest first.
    pub groups: Vec<ProcessGroup>,
    /// Processes of other users this user may not measure.
    pub unmeasured: usize,
}

const VIRTUAL_MACHINE: &str = "com.apple.Virtualization.VirtualMachine";
const SYSTEM_PREFIXES: &[&str] = &[
    "/System/",
    "/usr/",
    "/bin/",
    "/sbin/",
    "/Library/Apple/",
    "/private/var/",
];
/// Folder names that say nothing about the program inside.
const GENERIC_FOLDERS: &[&str] = &["versions", "bin", "libexec", "current", "Contents", "MacOS"];

pub fn snapshot() -> io::Result<MemorySnapshot> {
    let stats = stats()?;
    let own_pid = std::process::id();
    let own_bundle_id = std::env::current_exe()
        .ok()
        .and_then(|exe| outermost_app(&exe))
        .and_then(|bundle| apps::read_bundle(&bundle).ok())
        .map(|app| app.bundle_id);
    let (processes, unmeasured) = processes();

    // Bundle path → (bundle id, name), read once per snapshot.
    let mut identities: HashMap<PathBuf, Option<(String, String)>> = HashMap::new();
    let mut groups: Vec<ProcessGroup> = Vec::new();
    for process in processes {
        let (kind, path) = classify(&process.path);
        let identity = if kind == ProcessKind::App {
            identities
                .entry(path.clone())
                .or_insert_with(|| apps::read_bundle(&path).ok().map(|app| (app.bundle_id, app.name)))
                .clone()
        } else {
            None
        };
        let bundle_id = identity.as_ref().map(|(id, _)| id.clone());
        let same_group = |group: &ProcessGroup| match (&bundle_id, &group.bundle_id) {
            (Some(id), Some(other)) => id == other,
            _ => group.path == path,
        };
        if let Some(group) = groups.iter_mut().find(|g| same_group(g)) {
            group.footprint += process.footprint;
            group.quittable &= process.pid.unsigned_abs() != own_pid;
            // A browser's main process may run from a temporary copy;
            // show where the app is installed.
            if is_temporary(&group.path) && !is_temporary(&path) {
                group.path = path;
            }
            group.processes.push(process);
            continue;
        }
        let quittable = match kind {
            ProcessKind::App => !is_system_app(&path) && (own_bundle_id.is_none() || bundle_id != own_bundle_id),
            ProcessKind::Tool => process.pid.unsigned_abs() != own_pid,
            ProcessKind::VirtualMachine | ProcessKind::System => false,
        };
        groups.push(ProcessGroup {
            name: identity.map_or_else(|| group_name(kind, &path), |(_, name)| name),
            bundle_id,
            path,
            kind,
            footprint: process.footprint,
            processes: vec![process],
            quittable,
        });
    }
    for group in &mut groups {
        group
            .processes
            .sort_by_key(|process| std::cmp::Reverse(process.footprint));
    }
    groups.sort_by_key(|group| std::cmp::Reverse(group.footprint));

    Ok(MemorySnapshot {
        stats,
        groups,
        unmeasured,
    })
}

/// Processes this user may inspect, and how many others there are.
fn processes() -> (Vec<ProcessInfo>, usize) {
    let mut unmeasured = 0;
    let mut measured = Vec::new();
    for pid in all_pids().into_iter().filter(|&pid| pid > 0) {
        let Some(footprint) = footprint(pid) else {
            unmeasured += 1;
            continue;
        };
        // Gone between listing and measuring.
        let Some(path) = executable_path(pid) else {
            continue;
        };
        measured.push(ProcessInfo {
            pid,
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path,
            footprint,
        });
    }
    (measured, unmeasured)
}

fn is_temporary(path: &Path) -> bool {
    path.starts_with("/private/var/")
}

fn all_pids() -> Vec<i32> {
    // SAFETY: a null buffer asks for the number of processes.
    let count = unsafe { libc::proc_listallpids(ptr::null_mut(), 0) };
    let Ok(count) = usize::try_from(count) else {
        return Vec::new();
    };
    // Room for processes started between the two calls.
    let mut pids = vec![0i32; count + 64];
    let bytes = i32::try_from(pids.len() * size_of::<i32>()).unwrap_or(i32::MAX);
    // SAFETY: `pids` holds `bytes` bytes.
    let filled = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    pids.truncate(usize::try_from(filled).unwrap_or(0).min(pids.len()));
    pids
}

/// `None` for processes of other users, which the kernel will not measure.
fn footprint(pid: i32) -> Option<u64> {
    let mut info = MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: `info` has room for a rusage_info_v4, the flavor requested.
    let status = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, info.as_mut_ptr().cast()) };
    // SAFETY: filled by the kernel on success; integers only.
    (status == 0).then(|| unsafe { info.assume_init() }.ri_phys_footprint)
}

fn executable_path(pid: i32) -> Option<PathBuf> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buffer` holds PROC_PIDPATHINFO_MAXSIZE bytes.
    let len = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let len = usize::try_from(len).ok().filter(|&len| len > 0)?;
    Some(PathBuf::from(OsStr::from_bytes(&buffer[..len])))
}

fn owner(pid: i32) -> Option<libc::uid_t> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: `info` has room for `size` bytes, the PROC_PIDTBSDINFO reply.
    let filled = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    // SAFETY: fully filled by the kernel when it returns `size`.
    (filled == size).then(|| unsafe { info.assume_init() }.pbi_uid)
}

/// What a process is, and the path it is grouped under.
fn classify(executable: &Path) -> (ProcessKind, PathBuf) {
    if let Some(bundle) = outermost_app(executable) {
        return (ProcessKind::App, bundle);
    }
    let kind = if executable.file_name() == Some(OsStr::new(VIRTUAL_MACHINE)) {
        ProcessKind::VirtualMachine
    } else if is_system_path(executable) {
        ProcessKind::System
    } else {
        ProcessKind::Tool
    };
    (kind, executable.to_path_buf())
}

/// `/Applications/Google Chrome.app/…/Google Chrome Helper.app/…` →
/// `/Applications/Google Chrome.app`. Chromium browsers may also run from
/// a signed temporary copy named `<App>.app.bundle`.
fn outermost_app(path: &Path) -> Option<PathBuf> {
    let mut bundle = PathBuf::new();
    for component in path.components() {
        bundle.push(component);
        let name = component.as_os_str().to_string_lossy();
        if name.ends_with(".app") || name.ends_with(".app.bundle") {
            return Some(bundle);
        }
    }
    None
}

fn is_system_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    SYSTEM_PREFIXES.iter().any(|prefix| text.starts_with(prefix)) && !text.starts_with("/usr/local/")
}

/// Finder, the Dock, loginwindow… live in `/System/Library`: quitting them
/// disrupts the session (loginwindow logs the user out). Apple's regular
/// apps (`/System/Applications`, Safari's cryptex) can be quit.
fn is_system_app(bundle: &Path) -> bool {
    let text = bundle.to_string_lossy();
    text.starts_with("/System/Library/") || text.starts_with("/Library/Apple/")
}

fn group_name(kind: ProcessKind, path: &Path) -> String {
    match kind {
        ProcessKind::VirtualMachine => Some("Machine virtuelle".to_owned()),
        ProcessKind::App => path
            .file_stem()
            .map(|stem| stem.to_string_lossy().trim_end_matches(".app").to_owned()),
        ProcessKind::Tool | ProcessKind::System => program_name(path),
    }
    .unwrap_or_else(|| path.display().to_string())
}

/// The executable's name, or the folder naming it when the executable is
/// named after its version (`~/.local/share/claude/versions/2.1.311` →
/// `claude`).
fn program_name(path: &Path) -> Option<String> {
    path.ancestors()
        .filter_map(|ancestor| ancestor.file_name()?.to_str())
        .find(|name| name.chars().any(|c| c.is_ascii_alphabetic()) && !GENERIC_FOLDERS.contains(name))
        .map(str::to_owned)
}

#[derive(Debug, PartialEq, Eq)]
pub enum TerminateError {
    NotRunning,
    /// The process id now belongs to another program.
    Changed,
    /// macOS itself, a virtual machine, an app (quit those through AppKit),
    /// another user's process, or Débarras.
    Protected,
    Failed(String),
}

impl fmt::Display for TerminateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning => write!(f, "le processus est déjà terminé"),
            Self::Changed => write!(f, "ce numéro de processus correspond maintenant à un autre programme"),
            Self::Protected => write!(f, "ce processus ne peut pas être arrêté par Débarras"),
            Self::Failed(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for TerminateError {}

/// Asks a command-line program to stop (`SIGTERM`), or forces it
/// (`SIGKILL`). Everything is checked again right before the signal, since
/// the process may have exited and its id been reused.
pub fn terminate(pid: i32, expected: &Path, force: bool) -> Result<(), TerminateError> {
    if pid <= 0 || pid.unsigned_abs() == std::process::id() {
        return Err(TerminateError::Protected);
    }
    let path = executable_path(pid).ok_or(TerminateError::NotRunning)?;
    if path != expected {
        return Err(TerminateError::Changed);
    }
    if classify(&path).0 != ProcessKind::Tool {
        return Err(TerminateError::Protected);
    }
    // SAFETY: getuid has no preconditions.
    if owner(pid) != Some(unsafe { libc::getuid() }) {
        return Err(TerminateError::Protected);
    }
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: plain system call on a process this user owns.
    if unsafe { libc::kill(pid, signal) } != 0 {
        return Err(TerminateError::Failed(io::Error::last_os_error().to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[test]
    fn system_figures_add_up() {
        let stats = stats().unwrap();
        let mut expected_total: u64 = 0;
        let mut len = size_of::<u64>();
        let name = CString::new("hw.memsize").unwrap();
        // SAFETY: reads a u64 into a u64.
        unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&raw mut expected_total).cast(),
                &mut len,
                ptr::null_mut(),
                0,
            )
        };
        assert_eq!(stats.total, expected_total);
        assert_eq!(stats.used, stats.app + stats.wired + stats.compressed);
        assert!(stats.used > 0 && stats.used <= stats.total, "{stats:?}");
        assert!(stats.swap_used <= stats.swap_total || stats.swap_total == 0);
    }

    #[test]
    fn the_running_test_is_measured() {
        let snapshot = snapshot().unwrap();
        let own = std::process::id();
        let group = snapshot
            .groups
            .iter()
            .find(|g| g.processes.iter().any(|p| p.pid.unsigned_abs() == own))
            .expect("the test process is listed");
        assert!(group.footprint > 0);
        assert!(!group.quittable, "Débarras never offers to quit itself");
        let sorted = snapshot.groups.windows(2).all(|w| w[0].footprint >= w[1].footprint);
        assert!(sorted);
    }

    #[test]
    fn helpers_are_grouped_under_their_app() {
        let helper = Path::new(
            "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)",
        );
        assert_eq!(
            classify(helper),
            (ProcessKind::App, PathBuf::from("/Applications/Google Chrome.app"))
        );
    }

    #[test]
    fn a_browser_running_from_a_temporary_copy_is_still_an_app() {
        let main = Path::new(
            "/private/var/folders/8_/xyz/X/ai.perplexity.comet.code_sign_clone/code_sign_clone.OeUgcJ/Comet.app.bundle/Contents/MacOS/Comet",
        );
        let (kind, bundle) = classify(main);
        assert_eq!(kind, ProcessKind::App);
        assert!(bundle.ends_with("Comet.app.bundle"));
        assert_eq!(group_name(kind, &bundle), "Comet");
    }

    #[test]
    fn programs_named_after_their_version_take_their_folder_name() {
        assert_eq!(
            program_name(Path::new("/Users/me/.local/share/claude/versions/2.1.311")).as_deref(),
            Some("claude")
        );
        assert_eq!(
            program_name(Path::new("/opt/homebrew/bin/node")).as_deref(),
            Some("node")
        );
    }

    #[test]
    fn other_users_processes_are_counted_not_measured() {
        // SAFETY: getuid has no preconditions.
        if unsafe { libc::getuid() } != 0 {
            assert!(
                snapshot().unwrap().unmeasured > 0,
                "WindowServer and launchd belong to other users"
            );
        }
    }

    #[test]
    fn classifies_tools_system_and_virtual_machines() {
        assert_eq!(classify(Path::new("/opt/homebrew/bin/node")).0, ProcessKind::Tool);
        assert_eq!(classify(Path::new("/usr/local/bin/docker")).0, ProcessKind::Tool);
        assert_eq!(classify(Path::new("/usr/libexec/trustd")).0, ProcessKind::System);
        assert_eq!(
            classify(Path::new(
                "/System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine"
            ))
            .0,
            ProcessKind::VirtualMachine
        );
    }

    #[test]
    fn session_apps_of_macos_are_not_quittable() {
        assert!(is_system_app(Path::new("/System/Library/CoreServices/Finder.app")));
        assert!(is_system_app(Path::new("/System/Library/CoreServices/loginwindow.app")));
        assert!(!is_system_app(Path::new("/System/Applications/Notes.app")));
        assert!(!is_system_app(Path::new(
            "/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app"
        )));
    }

    fn wait_for_exit(child: &mut std::process::Child) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn stops_a_user_program() {
        let dir = crate::testing::tempdir();
        let program = fs_canonical(dir.path()).join("long-task");
        std::fs::copy("/bin/sleep", &program).unwrap();
        let mut child = Command::new(&program).arg("30").spawn().unwrap();
        let pid = i32::try_from(child.id()).unwrap();

        assert_eq!(
            terminate(pid, Path::new("/somewhere/else"), false),
            Err(TerminateError::Changed)
        );
        terminate(pid, &program, false).unwrap();
        assert!(wait_for_exit(&mut child), "SIGTERM stops it");
    }

    #[test]
    fn refuses_system_programs_itself_and_missing_processes() {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        assert_eq!(
            terminate(pid, Path::new("/bin/sleep"), false),
            Err(TerminateError::Protected)
        );
        child.kill().unwrap();
        child.wait().unwrap();

        let own = i32::try_from(std::process::id()).unwrap();
        assert_eq!(
            terminate(own, &std::env::current_exe().unwrap(), true),
            Err(TerminateError::Protected)
        );
        assert_eq!(
            terminate(pid, Path::new("/bin/sleep"), false),
            Err(TerminateError::NotRunning)
        );
    }

    fn fs_canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap()
    }
}
