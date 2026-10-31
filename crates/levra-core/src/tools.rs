//! Locating and running the command-line tools Levra relies on
//! (`docker`, `brew`, `git`, `xcrun`…).
//!
//! Apps launched from the Finder get a minimal `PATH`, so tools installed by
//! Homebrew, Docker Desktop or rustup are searched in their usual places
//! too. They also start in `/`, on the read-only system volume, where tools
//! that write next to themselves (pnpm) fail: every program runs from the
//! home folder instead, and always directly, never through a shell.

use std::env;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn search_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = env::var_os("PATH")
        .map(|path| env::split_paths(&path).collect())
        .unwrap_or_default();
    dirs.extend(
        [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        .iter()
        .map(PathBuf::from),
    );
    dirs.extend(
        [".docker/bin", ".cargo/bin", ".local/bin", "Library/pnpm"]
            .iter()
            .map(|rel| home.join(rel)),
    );
    let mut seen = Vec::new();
    dirs.retain(|dir| {
        !seen.contains(dir) && {
            seen.push(dir.clone());
            true
        }
    });
    dirs
}

/// `PATH` value handed to the commands Levra runs, so they find their
/// own helpers the same way they would in a terminal.
pub fn search_path(home: &Path) -> OsString {
    env::join_paths(search_dirs(home)).unwrap_or_default()
}

pub fn find(name: &str, home: &Path) -> Option<PathBuf> {
    search_dirs(home)
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Runs a query command and returns its standard output if it succeeded
/// within `timeout`. Used for read-only commands only.
pub fn output(program: &Path, args: &[&str], home: &Path, timeout: Duration) -> Option<String> {
    let (succeeded, text) = capture(program, args, home, timeout, Stream::Stdout)?;
    succeeded.then_some(text)
}

/// Standard error of a command, whatever its exit status: tools such as
/// `codesign -dv` print their answer there.
pub fn error_output(program: &Path, args: &[&str], home: &Path, timeout: Duration) -> Option<String> {
    capture(program, args, home, timeout, Stream::Stderr).map(|(_, text)| text)
}

enum Stream {
    Stdout,
    Stderr,
}

fn capture(program: &Path, args: &[&str], home: &Path, timeout: Duration, stream: Stream) -> Option<(bool, String)> {
    let (stdout, stderr) = match stream {
        Stream::Stdout => (Stdio::piped(), Stdio::null()),
        Stream::Stderr => (Stdio::null(), Stdio::piped()),
    };
    let mut child = Command::new(program)
        .args(args)
        .current_dir(home)
        .env("PATH", search_path(home))
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .ok()?;

    // The pipe is drained on its own thread: a child writing more than the
    // pipe buffer would otherwise block forever and look like a timeout.
    let mut pipe: Box<dyn Read + Send> = match stream {
        Stream::Stdout => Box::new(child.stdout.take()?),
        Stream::Stderr => Box::new(child.stderr.take()?),
    };
    let reader = thread::spawn(move || {
        let mut text = String::new();
        pipe.read_to_string(&mut text).map(|_| text)
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let text = reader.join().ok()?.ok()?;
    Some((status.success(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_system_tools() {
        assert_eq!(find("ls", Path::new("/nonexistent")), Some(PathBuf::from("/bin/ls")));
        assert_eq!(find("levra-no-such-tool", Path::new("/nonexistent")), None);
    }

    #[test]
    fn captures_output_of_successful_commands() {
        let out = output(
            Path::new("/bin/echo"),
            &["bonjour"],
            Path::new("/"),
            Duration::from_secs(5),
        );
        assert_eq!(out.as_deref(), Some("bonjour\n"));
    }

    #[test]
    fn failed_commands_return_nothing() {
        assert_eq!(
            output(Path::new("/usr/bin/false"), &[], Path::new("/"), Duration::from_secs(5)),
            None
        );
    }

    #[test]
    fn slow_commands_are_killed_at_the_deadline() {
        let started = Instant::now();
        let out = output(
            Path::new("/bin/sleep"),
            &["5"],
            Path::new("/"),
            Duration::from_millis(200),
        );
        assert_eq!(out, None);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn large_outputs_do_not_deadlock() {
        let out = output(
            Path::new("/usr/bin/head"),
            &["-c", "200000", "/dev/zero"],
            Path::new("/"),
            Duration::from_secs(5),
        );
        assert_eq!(out.map(|text| text.len()), Some(200_000));
    }
}
