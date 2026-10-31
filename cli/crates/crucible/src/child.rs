use std::io::{self, IsTerminal, Read, Write};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};

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

pub fn exit_with_parent() {
    std::thread::spawn(|| {
        let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
        std::process::exit(1);
    });
}

/// Runs the command with its stdout and stderr on our stderr. When our stderr is
/// not a terminal, ANSI escapes are removed, since tools such as TypeScript
/// colour diagnostics without checking.
pub fn status_on_stderr(mut command: Command) -> io::Result<ExitStatus> {
    if io::stderr().is_terminal() {
        return command.stdout(io::stderr()).status();
    }
    let (reader, writer) = io::pipe()?;
    command.stdout(writer.try_clone()?).stderr(writer);
    let mut child = command.spawn()?;
    drop(command);
    copy_plain(reader, io::stderr());
    child.wait()
}

#[derive(Clone, Copy)]
enum Escape {
    None,
    Start,
    Csi,
}

fn copy_plain(mut from: impl Read, mut to: impl Write) {
    let mut escape = Escape::None;
    let mut buffer = [0; 8192];
    let mut plain = Vec::with_capacity(buffer.len());
    while let Ok(read @ 1..) = from.read(&mut buffer) {
        plain.clear();
        for &byte in &buffer[..read] {
            escape = match (escape, byte) {
                (Escape::None, 0x1b) => Escape::Start,
                (Escape::None, _) => {
                    plain.push(byte);
                    Escape::None
                }
                (Escape::Start, b'[') => Escape::Csi,
                (Escape::Csi, 0x40..=0x7e) | (Escape::Start, _) => Escape::None,
                (Escape::Csi, _) => Escape::Csi,
            };
        }
        let _ = to.write_all(&plain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Chunks(Vec<&'static [u8]>);

    impl Read for Chunks {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            let chunk = self.0.remove(0);
            buffer[..chunk.len()].copy_from_slice(chunk);
            Ok(chunk.len())
        }
    }

    #[test]
    fn strips_escapes_split_across_reads() {
        let mut out = Vec::new();
        copy_plain(Chunks(vec![b"\x1b[96msrc/a.ts\x1b", b"[0m:\x1b[9", b"3m1\x1b[0m - error\x1bc\n"]), &mut out);
        assert_eq!(String::from_utf8(out).unwrap(), "src/a.ts:1 - error\n");
    }
}
