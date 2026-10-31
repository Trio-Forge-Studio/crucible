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

/// Removes ANSI escape sequences from a byte stream, including sequences split
/// across reads.
#[derive(Default)]
pub struct Plain(Escape);

#[derive(Clone, Copy, Default)]
enum Escape {
    #[default]
    None,
    Start,
    Intermediate,
    Csi,
    CsiIntermediate,
    String,
    StringEsc,
}

impl Plain {
    pub fn strip(&mut self, bytes: &[u8], plain: &mut Vec<u8>) {
        for &byte in bytes {
            self.0 = match (self.0, byte) {
                (_, b'\n' | b'\r') => {
                    plain.push(byte);
                    Escape::None
                }
                (Escape::String | Escape::StringEsc, 0x07) | (Escape::StringEsc, b'\\') => Escape::None,
                (Escape::String | Escape::StringEsc, 0x1b) => Escape::StringEsc,
                (Escape::String, _) => Escape::String,
                (_, 0x1b) => Escape::Start,
                (Escape::None, _) => {
                    plain.push(byte);
                    Escape::None
                }
                (Escape::Start | Escape::StringEsc, b'[') => Escape::Csi,
                (Escape::Start | Escape::StringEsc, b']' | b'P' | b'X' | b'^' | b'_') => Escape::String,
                (Escape::Start | Escape::StringEsc | Escape::Intermediate, 0x20..=0x2f) => Escape::Intermediate,
                (Escape::Csi, 0x30..=0x3f) => Escape::Csi,
                (Escape::Csi | Escape::CsiIntermediate, 0x20..=0x2f) => Escape::CsiIntermediate,
                (
                    Escape::Csi | Escape::CsiIntermediate | Escape::Start | Escape::StringEsc | Escape::Intermediate,
                    _,
                ) => Escape::None,
            };
        }
    }
}

fn copy_plain(mut from: impl Read, mut to: impl Write) {
    let mut escapes = Plain::default();
    let mut buffer = [0; 8192];
    let mut plain = Vec::with_capacity(buffer.len());
    while let Ok(read @ 1..) = from.read(&mut buffer) {
        plain.clear();
        escapes.strip(&buffer[..read], &mut plain);
        let _ = to.write_all(&plain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Chunks<'a>(Vec<&'a [u8]>);

    impl Read for Chunks<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            while self.0.first().is_some_and(|chunk| chunk.is_empty()) {
                self.0.remove(0);
            }
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

    fn plain(text: &str) -> String {
        let mut plain = Vec::new();
        Plain::default().strip(text.as_bytes(), &mut plain);
        String::from_utf8(plain).unwrap()
    }

    #[test]
    fn strips_strings_and_charset_escapes() {
        assert_eq!(plain("see \x1b]8;;https://a.b/c\x1b\\docs\x1b]8;;\x1b\\ now"), "see docs now");
        assert_eq!(plain("\x1b]0;title\x07ready"), "ready");
        assert_eq!(plain("\x1b(Bplain \x1b[1;31mred\x1b[m [kept]"), "plain red [kept]");
        assert_eq!(plain("\x1b]8;;unterminated\nnext line\n"), "\nnext line\n");
        assert_eq!(plain("\x1b\x1b[31mx"), "x");
    }

    fn assert_plain_at_every_split(input: &[u8], expected: &[u8]) {
        for split in 0..=input.len() {
            let mut out = Vec::new();
            copy_plain(Chunks(vec![&input[..split], &input[split..]]), &mut out);
            assert_eq!(out, expected, "split at {split}");
        }
    }

    #[test]
    fn escape_aborts_control_strings() {
        for introducer in [b']', b'P'] {
            let mut input = vec![0x1b, introducer];
            input.extend_from_slice(b"payload\x1b[31mvisible\nnext\n");
            assert_plain_at_every_split(&input, b"visible\nnext\n");
        }
    }

    #[test]
    fn malformed_sequences_preserve_line_breaks() {
        for (input, expected) in [
            (b"before\x1b\nafter\n".as_slice(), b"before\nafter\n".as_slice()),
            (b"before\x1b(\nafter\n", b"before\nafter\n"),
            (b"before\x1b[12\nafter\n", b"before\nafter\n"),
            (b"before\x1b[12\x01visible\nafter\n", b"beforevisible\nafter\n"),
            (b"before\x1b[1 2visible\nafter\n", b"beforevisible\nafter\n"),
        ] {
            assert_plain_at_every_split(input, expected);
        }
    }
}
