use crate::build::{Error, Receipt};
use crate::child::{self, Plain, Tied};
use crate::manifest::DEFAULT_PLACE;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
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
    tied: Tied,
    drained: Receiver<()>,
}

impl Process {
    fn stop(self) {
        let Self { mut tied, drained, .. } = self;
        // Each child leads its own process group (see isolate), so this also ends
        // what roblox-ts started.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(tied.child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = tied.child.kill();
        let _ = tied.child.wait();
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
    let manifest = crate::build::manifest(&root)?;
    let place = manifest.places.get(&args.place).ok_or_else(|| {
        let known: Vec<_> = manifest.places.keys().map(String::as_str).collect();
        Error::Usage(format!("unknown place {}. Places: {}", args.place, known.join(", ")))
    })?;
    let mut rbxtsc = crate::build::rbxtsc(&root)?;
    rbxtsc.args(["--watch", "--type", "game"]);
    let mut guarded = crucible()?;
    guarded.arg(WATCH_GUARD).arg(rbxtsc.get_program()).args(rbxtsc.get_args()).current_dir(&root);
    let mut rojo = crucible()?;
    rojo.arg(ROJO_SERVE).arg(root.join(&place.project));
    if let Some(port) = args.port {
        rojo.arg("--port").arg(port.to_string());
    }
    let receipt = Receipt::prepare(&root)?;
    let mut running = Vec::new();
    let result = supervise(&mut running, guarded, rojo, receipt);
    for process in running {
        process.stop();
    }
    result
}

fn crucible() -> Result<Command, Error> {
    std::env::current_exe()
        .map(Command::new)
        .map_err(|error| Error::Failed(format!("cannot find the crucible executable: {error}")))
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
                .tied
                .child
                .try_wait()
                .map_err(|error| Error::Failed(format!("cannot check {}: {error}", process.side.name())))?;
            if let Some(status) = status {
                return Err(Error::Failed(format!("{} stopped ({status})", process.side.name())));
            }
        }
    }
}

fn spawn(side: Side, mut command: Command, compiled: Option<Sender<Event>>) -> Result<Process, Error> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    isolate(&mut command);
    let mut tied = child::spawn_tied(&mut command)
        .map_err(|error| Error::Failed(format!("cannot start {}: {error}", side.name())))?;
    let (drain, drained) = mpsc::channel();
    let stdout = tied.child.stdout.take().expect("stdout is piped");
    let stderr = tied.child.stderr.take().expect("stderr is piped");
    relay(side, stdout, compiled, drain.clone());
    relay(side, stderr, None, drain);
    Ok(Process { side, tied, drained })
}

// Children never see the terminal's Ctrl+C, so only this process decides to stop, and a
// clean stop cannot race a child dying from the same signal.
fn isolate(command: &mut Command) {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    #[cfg(windows)]
    hide(command);
}

#[cfg(windows)]
fn hide(command: &mut Command) {
    std::os::windows::process::CommandExt::creation_flags(
        command,
        windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
    );
}

fn relay(side: Side, pipe: impl Read + Send + 'static, compiled: Option<Sender<Event>>, drain: Sender<()>) {
    std::thread::spawn(move || {
        let colour = std::io::stderr().is_terminal();
        let mut reader = BufReader::new(pipe);
        let mut escapes = Plain::default();
        let (mut bytes, mut plain) = (Vec::new(), Vec::new());
        while reader.read_until(b'\n', &mut bytes).is_ok_and(|read| read > 0) {
            plain.clear();
            escapes.strip(&bytes, &mut plain);
            let line = String::from_utf8_lossy(if colour { &bytes } else { &plain });
            let _ = writeln!(std::io::stderr().lock(), "[{}] {}", side.tag(), line.trim_end_matches(['\r', '\n']));
            if let Some(events) = &compiled
                && String::from_utf8_lossy(&plain).contains(COMPILED)
            {
                let _ = events.send(Event::Compiled);
            }
            bytes.clear();
        }
        let _ = drain.send(());
    });
}

pub const WATCH_GUARD: &str = "__watch-guard";

pub fn guard_main(command: Vec<OsString>) -> ExitCode {
    child::exit_with_parent();
    match guard(&command) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("crucible: {error}");
            ExitCode::FAILURE
        }
    }
}

fn guard(command: &[OsString]) -> Result<ExitCode, String> {
    #[cfg(windows)]
    contain_descendants().map_err(|error| format!("cannot put roblox-ts in a job object: {error}"))?;
    let (program, args) = command.split_first().ok_or("no command to run")?;
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    hide(&mut command);
    let status =
        command.status().map_err(|error| format!("cannot run node: {error}. Install Node.js and put node on PATH"))?;
    Ok(ExitCode::from(status.code().and_then(|code| u8::try_from(code).ok()).unwrap_or(1)))
}

// The job handle stays open until this process ends for any reason. Closing it then
// kills every process roblox-ts started, which inherit the job.
#[cfg(windows)]
fn contain_descendants() -> std::io::Result<()> {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = size_of_val(&limits) as u32;
        if SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&raw const limits).cast(), size) == 0
            || AssignProcessToJobObject(job, GetCurrentProcess()) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

pub const ROJO_SERVE: &str = "__rojo-serve";

pub fn rojo_main(project: PathBuf, port: Option<u16>) -> ExitCode {
    child::exit_with_parent();
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
