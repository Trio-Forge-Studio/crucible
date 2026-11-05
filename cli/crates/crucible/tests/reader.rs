use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ROWS: u16 = 30;
const COLUMNS: u16 = 120;

struct Terminal {
    screen: Arc<Mutex<vt100::Parser>>,
    input: Box<dyn Write + Send>,
}

impl Terminal {
    fn wait_for(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let contents = self.screen.lock().unwrap().screen().contents();
            if contents.contains(text) {
                return;
            }
            assert!(Instant::now() < deadline, "the screen never showed {text:?}:\n{contents}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn press(&mut self, keys: &str) {
        self.input.write_all(keys.as_bytes()).unwrap();
        self.input.flush().unwrap();
    }
}

#[test]
fn the_reader_searches_and_quits_in_a_real_terminal() {
    let pty =
        native_pty_system().openpty(PtySize { rows: ROWS, cols: COLUMNS, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_crucible"));
    command.args(["docs", "lifecycle"]);
    command.env("TERM", "xterm-256color");
    let mut child = KillOnDrop(pty.slave.spawn_command(command).unwrap());
    drop(pty.slave);
    let screen = Arc::new(Mutex::new(vt100::Parser::new(ROWS.try_into().unwrap(), COLUMNS.try_into().unwrap(), 0)));
    let mut output = pty.master.try_clone_reader().unwrap();
    let input = Arc::new(Mutex::new(pty.master.take_writer().unwrap()));
    let (parser, replies) = (screen.clone(), input.clone());
    std::thread::spawn(move || {
        let mut buffer = [0; 8192];
        let mut unanswered = Vec::new();
        while let Ok(read @ 1..) = output.read(&mut buffer) {
            let bytes = &buffer[..read];
            parser.lock().unwrap().process(bytes);
            unanswered.extend_from_slice(bytes);
            // ConPTY asks for the cursor position before it draws anything and waits for the answer.
            if unanswered.windows(4).any(|window| window == b"\x1b[6n") {
                let _ = replies.lock().unwrap().write_all(b"\x1b[1;1R");
                unanswered.clear();
            }
            unanswered.drain(..unanswered.len().saturating_sub(3));
        }
    });
    let mut terminal = Terminal { screen, input: Box::new(SharedWriter(input)) };

    terminal.wait_for("Crucible docs");
    terminal.wait_for(" lifecycle  Top");
    assert!(terminal.screen.lock().unwrap().screen().alternate_screen(), "the reader draws on the alternate screen");
    terminal.press("\x0b");
    terminal.wait_for("Search the docs");
    terminal.press("exit codes");
    terminal.wait_for("Exit codes  oc");
    terminal.press("\r");
    terminal.wait_for(" oc › Exit codes ");
    terminal.press("q");

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "crucible docs did not exit after q");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let parser = terminal.screen.lock().unwrap();
        let screen = parser.screen();
        let restored = !screen.alternate_screen()
            && !screen.hide_cursor()
            && screen.mouse_protocol_mode() == vt100::MouseProtocolMode::None;
        if restored {
            break;
        }
        assert!(Instant::now() < deadline, "the terminal was left in reader mode:\n{}", screen.contents());
        drop(parser);
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct SharedWriter(Arc<Mutex<Box<dyn Write + Send>>>);

impl Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

struct KillOnDrop(Box<dyn portable_pty::Child + Send + Sync>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}
