use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// The repository root, two folders above this crate.
pub fn repo() -> PathBuf {
    normalize(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

/// Removes `.` and `..` components without touching the file system, so two spellings of a path compare equal.
/// `fs::canonicalize` would give Windows paths a `\\?\` prefix that other tools mishandle.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Runs `command` and fails unless it exits 0. Its output goes to stderr, which keeps stdout for results such as
/// the proof table.
pub fn status(command: &mut Command) -> Result<(), String> {
    let shown = describe(command);
    let status = command.stdout(std::io::stderr()).status().map_err(|e| format!("cannot start {shown}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{shown} failed with {status}")) }
}

/// Runs `command` and returns its standard output, failing unless it exits 0.
pub fn stdout(command: &mut Command) -> Result<String, String> {
    let shown = describe(command);
    let output = command.stderr(Stdio::inherit()).output().map_err(|e| format!("cannot start {shown}: {e}"))?;
    if !output.status.success() {
        return Err(format!("{shown} failed with {}", output.status));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("{shown} printed invalid UTF-8: {e}"))
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    stdout(Command::new("git").arg("-C").arg(dir).args(args))
}

/// Like `git`, but `None` when git exits non-zero, for questions such as "does this file exist at that commit".
pub fn git_maybe(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).stderr(Stdio::null()).output().ok()?;
    if output.status.success() { String::from_utf8(output.stdout).ok() } else { None }
}

/// A program found on PATH. Windows installs pnpm as a `.cmd` shim, which only `cmd` can start.
pub fn tool(name: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", name]);
        command
    } else {
        Command::new(name)
    }
}

fn describe(command: &Command) -> String {
    let mut shown = command.get_program().to_string_lossy().into_owned();
    for arg in command.get_args() {
        shown.push(' ');
        shown.push_str(&arg.to_string_lossy());
    }
    format!("`{shown}`")
}

/// The path with forward slashes, so manifests and violation lists read the same on Windows.
pub fn slashed(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}
