use crate::build::{Error, Receipt};
use crate::manifest::{DEFAULT_PLACE, Manifest};
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

#[derive(clap::Args)]
pub struct Args {
    /// Serve this place from crucible.toml.
    #[arg(long, value_name = "NAME", default_value = DEFAULT_PLACE)]
    pub place: String,
    /// Serve on this port instead of the project's servePort or Rojo's default, 34872.
    #[arg(long)]
    pub port: Option<u16>,
}

const COMPILED: &str = "Watching for file changes.";
const POLL: Duration = Duration::from_millis(100);
const DRAIN: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
enum Side {
    Rbxtsc,
    Rojo,
}

impl Side {
    fn tag(self) -> &'static str {
        match self {
            Self::Rbxtsc => "rbxtsc",
            Self::Rojo => "rojo",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Rbxtsc => "roblox-ts",
            Self::Rojo => "Rojo",
        }
    }
}

enum Event {
    Interrupted,
    Compiled,
}

struct Process {
    side: Side,
    child: Child,
    tree: Tree,
    drained: Receiver<()>,
}

impl Process {
    fn stop(self) {
        let Self { mut child, tree, drained, .. } = self;
        drop(tree);
        let _ = child.kill();
        let _ = child.wait();
        for _ in 0..2 {
            if drained.recv_timeout(DRAIN).is_err() {
                break;
            }
        }
    }
}

pub fn main(args: Args) -> ExitCode {
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("crucible dev: {error}");
            ExitCode::from(error.code())
        }
    }
}

fn run(args: Args) -> Result<(), Error> {
    let root = std::env::current_dir()
        .map_err(|error| Error::Failed(format!("cannot read the current directory: {error}")))?;
    let manifest = Manifest::read(&root).map_err(|error| Error::Usage(error.to_string()))?.unwrap_or_default();
    let place = manifest.places.get(&args.place).ok_or_else(|| {
        let known: Vec<_> = manifest.places.keys().map(String::as_str).collect();
        Error::Usage(format!("unknown place {}. Places: {}", args.place, known.join(", ")))
    })?;
    let mut rbxtsc = crate::build::rbxtsc(&root)?;
    rbxtsc.args(["--watch", "--type", "game"]);
    let rojo = rojo(&root.join(&place.project), args.port)?;
    let receipt = Receipt::prepare(&root)?;
    let mut running = Vec::new();
    let result = supervise(&mut running, rbxtsc, rojo, receipt);
    for process in running {
        process.stop();
    }
    result
}

fn supervise(
    running: &mut Vec<Process>,
    rbxtsc: Command,
    rojo: Command,
    receipt: Option<Receipt>,
) -> Result<(), Error> {
    let (events, inbox) = mpsc::channel();
    let interrupts = events.clone();
    ctrlc::set_handler(move || {
        let _ = interrupts.send(Event::Interrupted);
    })
    .map_err(|error| Error::Failed(format!("cannot handle Ctrl+C: {error}")))?;
    running.push(spawn(Side::Rbxtsc, rbxtsc, Some(events))?);
    let mut rojo = Some(rojo);
    loop {
        match inbox.recv_timeout(POLL) {
            Ok(Event::Interrupted) => return Ok(()),
            Ok(Event::Compiled) => {
                if let Some(rojo) = rojo.take() {
                    receipt.as_ref().map_or(Ok(()), Receipt::write)?;
                    running.push(spawn(Side::Rojo, rojo, None)?);
                }
            }
            Err(_) => {}
        }
        for process in running.iter_mut() {
            let status = process
                .child
                .try_wait()
                .map_err(|error| Error::Failed(format!("cannot check {}: {error}", process.side.name())))?;
            if let Some(status) = status {
                return Err(Error::Failed(format!("{} stopped ({status})", process.side.name())));
            }
        }
    }
}

fn rojo(project: &Path, port: Option<u16>) -> Result<Command, Error> {
    let exe = std::env::current_exe()
        .map_err(|error| Error::Failed(format!("cannot find the crucible executable: {error}")))?;
    let mut command = Command::new(exe);
    command.arg(ROJO_SERVE).arg(project);
    if let Some(port) = port {
        command.arg("--port").arg(port.to_string());
    }
    Ok(command)
}

fn spawn(side: Side, mut command: Command, compiled: Option<Sender<Event>>) -> Result<Process, Error> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    isolate(&mut command);
    let mut child = command.spawn().map_err(|error| {
        Error::Failed(match side {
            Side::Rbxtsc => format!("cannot run node: {error}. Install Node.js and put node on PATH"),
            Side::Rojo => format!("cannot start Rojo: {error}"),
        })
    })?;
    let tree = Tree::new(&child);
    let (drain, drained) = mpsc::channel();
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    relay(side, stdout, compiled, drain.clone());
    relay(side, stderr, None, drain);
    Ok(Process { side, child, tree, drained })
}

// Children never see the terminal's Ctrl+C, so only this process decides to stop, and a
// clean stop cannot race a child dying from the same signal. Tree ends each child with the
// processes it started, such as the real node behind a version manager's node shim.
fn isolate(command: &mut Command) {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    #[cfg(target_os = "linux")]
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(command, || {
            match libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) {
                0 => Ok(()),
                _ => Err(std::io::Error::last_os_error()),
            }
        });
    }
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        command,
        windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
    );
}

#[cfg(unix)]
struct Tree(libc::pid_t);

#[cfg(unix)]
impl Tree {
    fn new(child: &Child) -> Self {
        Self(child.id() as libc::pid_t)
    }
}

#[cfg(unix)]
impl Drop for Tree {
    fn drop(&mut self) {
        unsafe { libc::kill(-self.0, libc::SIGKILL) };
    }
}

#[cfg(windows)]
struct Tree(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Tree {
    fn new(child: &Child) -> Self {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
        };
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of_val(&limits) as u32,
            );
            AssignProcessToJobObject(job, child.as_raw_handle());
            Self(job)
        }
    }
}

#[cfg(windows)]
impl Drop for Tree {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

fn relay(side: Side, pipe: impl Read + Send + 'static, compiled: Option<Sender<Event>>, drain: Sender<()>) {
    std::thread::spawn(move || {
        let colour = std::io::stderr().is_terminal();
        let mut reader = BufReader::new(pipe);
        let mut bytes = Vec::new();
        while reader.read_until(b'\n', &mut bytes).is_ok_and(|read| read > 0) {
            let line = String::from_utf8_lossy(&bytes);
            let line = line.trim_end_matches(['\r', '\n']);
            let line = if colour { line.to_owned() } else { plain(line) };
            let _ = writeln!(std::io::stderr().lock(), "[{}] {line}", side.tag());
            if let Some(events) = &compiled
                && line.contains(COMPILED)
            {
                let _ = events.send(Event::Compiled);
            }
            bytes.clear();
        }
        let _ = drain.send(());
    });
}

fn plain(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(char) = chars.next() {
        if char != '\x1b' {
            plain.push(char);
        } else if chars.next() == Some('[') {
            chars.by_ref().find(|char| ('@'..='~').contains(char));
        }
    }
    plain
}

pub const ROJO_SERVE: &str = "__rojo-serve";

pub fn rojo_main(project: PathBuf, port: Option<u16>) -> ExitCode {
    crate::build::rojo_logger();
    let command = librojo::cli::ServeCommand { project, address: None, port, allowed_hosts: Vec::new() };
    match command.run(librojo::cli::GlobalOptions { verbosity: 0, color: librojo::cli::ColorChoice::Never }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:?}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::plain;

    #[test]
    fn plain_drops_colour_sequences_only() {
        assert_eq!(plain("\x1b[90m10:00:00 AM\x1b[0m - Found 0 errors."), "10:00:00 AM - Found 0 errors.");
        assert_eq!(plain("src/a.ts:1:2 - \x1b[91merror\x1b[0m TS1005"), "src/a.ts:1:2 - error TS1005");
        assert_eq!(plain("no escapes [here]"), "no escapes [here]");
    }
}
