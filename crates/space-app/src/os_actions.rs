//! Hand-offs to the operating system: Quick Look, default app, a terminal in
//! a folder and the share route. Every call spawns a detached process and
//! returns at once; the caller turns an error into a notification.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What an OS hand-off did, for the notification the caller shows.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// No tool for this on the platform (caller explains).
    Unsupported,
    Failed(String),
}

fn spawn(mut cmd: Command) -> Result<(), String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The folder an action should run in: the path itself or its parent.
pub fn folder_of(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.to_path_buf())
    }
}

/// Quick Look preview on macOS (`qlmanage -p`), detached.
pub fn quick_look(path: &Path) -> Outcome {
    if cfg!(target_os = "macos") {
        let mut cmd = Command::new("qlmanage");
        cmd.arg("-p").arg(path);
        match spawn(cmd) {
            Ok(()) => Outcome::Done,
            Err(e) => Outcome::Failed(e),
        }
    } else {
        Outcome::Unsupported
    }
}

/// Open a file with its default application.
pub fn open_default(path: &Path) -> Outcome {
    let cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(path);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    match spawn(cmd) {
        Ok(()) => Outcome::Done,
        Err(e) => Outcome::Failed(e),
    }
}

/// Terminal launch candidates for `dir`, tried in order.
pub fn terminal_commands(dir: &Path) -> Vec<Command> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.args(["-a", "Terminal"]).arg(dir);
        out.push(c);
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "cmd", "/K", "cd", "/d"]).arg(dir);
        out.push(c);
    } else {
        let mut c = Command::new("x-terminal-emulator");
        c.current_dir(dir);
        out.push(c);
        let mut c = Command::new("gnome-terminal");
        c.arg(format!("--working-directory={}", dir.display()));
        out.push(c);
        let mut c = Command::new("konsole");
        c.arg("--workdir").arg(dir);
        out.push(c);
        let mut c = Command::new("xterm");
        c.current_dir(dir);
        out.push(c);
    }
    out
}

/// Open a terminal window in `dir` (or the folder holding a file).
pub fn open_terminal(path: &Path) -> Outcome {
    let dir = folder_of(path);
    let mut last = String::new();
    for cmd in terminal_commands(&dir) {
        match spawn(cmd) {
            Ok(()) => return Outcome::Done,
            Err(e) => last = e,
        }
    }
    Outcome::Failed(last)
}

/// Share: a real share sheet needs AppKit bindings, so this reveals the
/// item in the file manager (where the system Share menu lives) and the
/// caller copies the path.
pub fn share(path: &Path) -> Outcome {
    crate::app::reveal_path(&path.to_string_lossy());
    Outcome::Done
}

/// Expand a leading `~` and trim what a user typed into the path field.
pub fn expand_typed_path(raw: &str, home: Option<&Path>) -> PathBuf {
    let s = raw.trim();
    let s = s.strip_suffix(std::path::MAIN_SEPARATOR).filter(|r| !r.is_empty()).unwrap_or(s);
    if let (Some(rest), Some(home)) = (s.strip_prefix('~'), home) {
        let rest = rest.trim_start_matches(['/', '\\']);
        return if rest.is_empty() { home.to_path_buf() } else { home.join(rest) };
    }
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_paths_expand_home_and_drop_a_trailing_slash() {
        let home = Path::new("/home/u");
        assert_eq!(expand_typed_path(" ~ ", Some(home)), PathBuf::from("/home/u"));
        assert_eq!(expand_typed_path("~/Movies/", Some(home)), PathBuf::from("/home/u/Movies"));
        assert_eq!(expand_typed_path("/var/log", Some(home)), PathBuf::from("/var/log"));
        assert_eq!(expand_typed_path("/", Some(home)), PathBuf::from("/"));
    }

    #[test]
    fn every_platform_has_a_terminal_candidate() {
        assert!(!terminal_commands(Path::new("/tmp")).is_empty());
    }
}
