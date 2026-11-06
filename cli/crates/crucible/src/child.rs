use std::io;
use std::process::{Child, ChildStdin, Command, Stdio};

pub struct Tied {
    pub child: Child,
    // Held outside Child because Child::wait closes stdin before it waits.
    _lifeline: ChildStdin,
}

/// Spawns a child that runs `exit_with_parent`. The OS closes the child's stdin
/// when this process ends for any reason, kill included, and the child exits on
/// that EOF.
pub fn spawn_tied(command: &mut Command) -> io::Result<Tied> {
    let mut child = command.stdin(Stdio::piped()).spawn()?;
    let lifeline = child.stdin.take().expect("stdin is piped");
    Ok(Tied { child, _lifeline: lifeline })
}

/// A child that leads its own process group ends the whole group, so the
/// processes it started stop with it.
pub fn exit_with_parent() {
    std::thread::spawn(|| {
        let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
        #[cfg(unix)]
        unsafe {
            if libc::getpgrp() == libc::getpid() {
                libc::kill(0, libc::SIGKILL);
            }
        }
        std::process::exit(1);
    });
}
