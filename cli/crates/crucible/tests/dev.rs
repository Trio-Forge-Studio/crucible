use std::fs;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FAKE_RBXTSC: &str = r#"
const fs = require("node:fs");
const shimmed = require("node:child_process").spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], {
	stdio: "ignore",
	windowsHide: true,
});
fs.writeFileSync("rbxtsc.pids", `${process.pid} ${shimmed.pid}`);
console.log(JSON.stringify(process.argv.slice(2)));
if (process.env.FAKE_RBXTSC_EXIT) process.exit(Number(process.env.FAKE_RBXTSC_EXIT));
fs.mkdirSync("out", { recursive: true });
fs.writeFileSync("out/main.server.luau", "print('hi')");
console.log("\x1b[90m10:00:00 AM\x1b[0m - Found 0 errors. Watching for file changes.");
setInterval(() => {}, 1000);
"#;

const PROJECT: &str = r#"{"name":"fixture","tree":{"$className":"DataModel","ReplicatedStorage":{"$className":"ReplicatedStorage","Out":{"$path":"out"}}}}"#;

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let rbxts = root.join("node_modules/roblox-ts");
    fs::create_dir_all(rbxts.join("out/CLI")).unwrap();
    fs::write(rbxts.join("package.json"), r#"{"name":"roblox-ts","bin":{"rbxtsc":"out/CLI/cli.js"}}"#).unwrap();
    fs::write(rbxts.join("out/CLI/cli.js"), FAKE_RBXTSC).unwrap();
    let crucible = root.join("node_modules/@trioforge/crucible");
    fs::create_dir_all(crucible.join("transformer")).unwrap();
    fs::write(crucible.join("package.json"), r#"{"name":"@trioforge/crucible"}"#).unwrap();
    fs::write(crucible.join("transformer/index.js"), "module.exports = 1;").unwrap();
    fs::write(root.join("default.project.json"), PROJECT).unwrap();
    dir
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn wait_until<T>(what: &str, timeout: Duration, mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out after {timeout:?} waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct Dev {
    child: Child,
    stdout: Output,
    stderr: Output,
}

#[derive(Clone)]
struct Output {
    text: Arc<Mutex<String>>,
    closed: Arc<Mutex<bool>>,
}

impl Output {
    fn collect(pipe: impl std::io::Read + Send + 'static) -> Self {
        let output = Self { text: Arc::default(), closed: Arc::default() };
        let sink = output.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                let mut text = sink.text.lock().unwrap();
                text.push_str(&line);
                text.push('\n');
            }
            *sink.closed.lock().unwrap() = true;
        });
        output
    }

    fn text(&self) -> String {
        self.text.lock().unwrap().clone()
    }

    fn closed(&self) -> bool {
        *self.closed.lock().unwrap()
    }
}

impl Dev {
    fn start(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crucible"));
        command
            .arg("dev")
            .args(args)
            .current_dir(dir)
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(
            &mut command,
            windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP,
        );
        let mut child = command.spawn().unwrap();
        let stdout = Output::collect(child.stdout.take().unwrap());
        let stderr = Output::collect(child.stderr.take().unwrap());
        Self { child, stdout, stderr }
    }

    fn stderr(&self) -> String {
        self.stderr.text()
    }

    fn wait_for_stderr(&self, text: &str, timeout: Duration) {
        wait_until(text, timeout, || self.stderr().contains(text).then_some(()));
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn interrupt(&self) {
        assert!(self.try_interrupt(), "Ctrl+C reaches crucible dev");
    }

    #[cfg(unix)]
    fn try_interrupt(&self) -> bool {
        Command::new("kill").args(["-INT", &self.pid().to_string()]).status().is_ok_and(|status| status.success())
    }

    #[cfg(windows)]
    fn try_interrupt(&self) -> bool {
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
        unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.pid()) != 0 }
    }

    fn wait(&mut self, timeout: Duration) -> ExitStatus {
        let status = wait_until("crucible dev to exit", timeout, || self.child.try_wait().unwrap());
        wait_until("every process to close crucible dev's output", STOP, || {
            (self.stdout.closed() && self.stderr.closed()).then_some(())
        });
        assert_eq!(self.stdout.text(), "", "crucible dev prints nothing on stdout");
        status
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none()
            && (!self.try_interrupt() || wait_until_exit(&mut self.child, Duration::from_secs(10)).is_none())
        {
            let _ = self.child.kill();
        }
    }
}

fn wait_until_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().ok().flatten() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Process {
    pid: u32,
    parent: u32,
    created: u64,
    running: bool,
}

fn processes() -> Vec<Process> {
    let output = if cfg!(windows) {
        let list = "Get-CimInstance Win32_Process | Where-Object { $_.CreationDate } | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId) $($_.CreationDate.ToUniversalTime().Ticks) R\" }";
        Command::new("powershell").args(["-NoProfile", "-Command", list]).output()
    } else {
        Command::new("ps").args(["-e", "-o", "pid=,ppid=,stat="]).output()
    }.unwrap();
    assert!(output.status.success(), "process snapshot failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).lines().map(parse_process).collect()
}

fn parse_process(line: &str) -> Process {
    let mut fields = line.split_whitespace();
    let pid = fields.next().unwrap().parse().unwrap();
    let parent = fields.next().unwrap().parse().unwrap();
    let field = fields.next().unwrap();
    let (created, state) = match fields.next() {
        Some(state) => (field.parse().unwrap(), state),
        None => (0, field),
    };
    Process { pid, parent, created, running: !state.starts_with('Z') }
}

fn descendants(pid: u32) -> Vec<Process> {
    descendants_in(pid, &processes())
}

fn descendants_in(pid: u32, snapshot: &[Process]) -> Vec<Process> {
    let root = *snapshot.iter().find(|process| process.pid == pid).expect("parent is running");
    let mut found = vec![root];
    let mut next = 0;
    while next < found.len() {
        let parent = found[next];
        for child in snapshot {
            // Windows keeps a dead parent's PID even after that PID is reused.
            if child.parent == parent.pid && child.created >= parent.created && !found.contains(child) {
                found.push(*child);
            }
        }
        next += 1;
    }
    found.remove(0);
    found
}

fn alive(process: &Process, snapshot: &[Process]) -> bool {
    snapshot.iter().any(|current| current.pid == process.pid && current.created == process.created && current.running)
}

#[test]
fn reused_parent_pid_is_not_a_descendant() {
    let root = Process { pid: 10, parent: 1, created: 100, running: true };
    let child = Process { pid: 20, parent: 10, created: 110, running: true };
    let grandchild = Process { pid: 30, parent: 20, created: 120, running: true };
    let stale = Process { pid: 40, parent: 20, created: 105, running: true };
    let unrelated = Process { pid: 50, parent: 40, created: 130, running: true };
    assert_eq!(descendants_in(10, &[root, child, grandchild, stale, unrelated]), vec![child, grandchild]);
    assert!(!alive(&child, &[Process { created: 200, ..child }]));
}

#[test]
fn zombies_are_not_alive() {
    let process = parse_process("20 10 S");
    assert!(alive(&process, &[process]));
    assert!(!alive(&process, &[parse_process("20 10 Z")]));
    assert!(!alive(&process, &[parse_process("20 10 Z+")]));
}

#[cfg(target_os = "linux")]
#[test]
fn unreaped_child_is_not_alive() {
    let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
    let pid = child.id();
    wait_until("child to become an unreaped zombie", STOP, || {
        let snapshot = processes();
        let process = snapshot.iter().find(|process| process.pid == pid)?;
        (!alive(process, &snapshot)).then_some(())
    });
    assert!(child.wait().unwrap().success());
}

fn port_is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn rbxtsc_pids(dir: &Path) -> Vec<u32> {
    fs::read_to_string(dir.join("rbxtsc.pids")).unwrap().split(' ').map(|pid| pid.parse().unwrap()).collect()
}

fn assert_all_stop(pids: &[Process], port: u16) {
    let deadline = Instant::now() + STOP;
    while Instant::now() < deadline {
        let snapshot = processes();
        if pids.iter().all(|process| !alive(process, &snapshot)) && port_is_free(port) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let snapshot = processes();
    let survivors: Vec<_> =
        pids.iter().filter(|process| alive(process, &snapshot)).map(|process| process.pid).collect();
    // Survivors hold this test's output handles, so the run would hang on them.
    for pid in &survivors {
        let _ = if cfg!(windows) {
            Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output()
        } else {
            Command::new("kill").args(["-9", &pid.to_string()]).output()
        };
    }
    panic!("{survivors:?} outlived crucible dev by {STOP:?}. Port {port} free: {}", port_is_free(port));
}

const START: Duration = Duration::from_secs(30);
const STOP: Duration = Duration::from_secs(15);

#[test]
fn ctrl_c_stops_roblox_ts_and_rojo() {
    let game = fixture();
    let port = free_port();
    let mut dev = Dev::start(game.path(), &["--port", &port.to_string()], &[]);
    dev.wait_for_stderr("Rojo server listening", START);
    let stderr = dev.stderr();
    assert!(stderr.contains(r#"[rbxtsc] ["--watch","--type","game"]"#), "{stderr}");
    assert!(stderr.contains("[rbxtsc] 10:00:00 AM - Found 0 errors. Watching for file changes."), "{stderr}");
    assert!(stderr.contains(&format!("[rojo]   Port:    {port}")), "{stderr}");
    assert!(game.path().join("out/transformer.sha256").is_file(), "the first compile writes the receipt");
    let spawned = descendants(dev.pid());
    let rbxtsc = rbxtsc_pids(game.path());
    assert!(rbxtsc.iter().all(|pid| spawned.iter().any(|process| process.pid == *pid)), "{rbxtsc:?} in {spawned:?}");
    assert!(spawned.len() >= 4, "a guard, roblox-ts, its child and Rojo: {spawned:?}");
    dev.interrupt();
    let status = dev.wait(STOP);
    assert_eq!(status.code(), Some(0), "{}", dev.stderr());
    assert_all_stop(&spawned, port);
}

#[test]
fn killing_only_crucible_dev_stops_its_children() {
    let game = fixture();
    let port = free_port();
    let mut dev = Dev::start(game.path(), &["--port", &port.to_string()], &[]);
    dev.wait_for_stderr("Rojo server listening", START);
    let spawned = descendants(dev.pid());
    assert!(spawned.len() >= 4, "a guard, roblox-ts, its child and Rojo: {spawned:?}");
    assert!(
        rbxtsc_pids(game.path()).iter().all(|pid| spawned.iter().any(|process| process.pid == *pid)),
        "{spawned:?}"
    );
    dev.child.kill().unwrap();
    dev.child.wait().unwrap();
    assert_all_stop(&spawned, port);
}

#[test]
fn rojo_failure_stops_roblox_ts() {
    let game = fixture();
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let mut dev = Dev::start(game.path(), &["--port", &port.to_string()], &[]);
    let status = dev.wait(START);
    let stderr = dev.stderr();
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("[rojo] ") && stderr.contains("crucible dev: Rojo stopped"), "{stderr}");
    let pids = rbxtsc_pids(game.path());
    wait_until("roblox-ts to stop when Rojo does", STOP, || {
        let snapshot = processes();
        pids.iter().all(|pid| !snapshot.iter().any(|process| process.pid == *pid && process.running)).then_some(())
    });
}

#[test]
fn roblox_ts_exit_stops_dev_before_rojo_starts() {
    let game = fixture();
    let mut dev = Dev::start(game.path(), &["--port", &free_port().to_string()], &[("FAKE_RBXTSC_EXIT", "3")]);
    let status = dev.wait(START);
    let stderr = dev.stderr();
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("crucible dev: roblox-ts stopped"), "{stderr}");
    assert!(!stderr.contains("[rojo]"), "{stderr}");
}

#[test]
fn unknown_place_exits_2_before_starting() {
    let game = fixture();
    let mut dev = Dev::start(game.path(), &["--place", "arena"], &[]);
    let status = dev.wait(START);
    assert_eq!(status.code(), Some(2));
    assert!(dev.stderr().contains("crucible dev: unknown place arena. Places: start"), "{}", dev.stderr());
    assert!(!game.path().join("rbxtsc.pids").exists(), "nothing started");
}

struct Restore {
    path: PathBuf,
    original: Vec<u8>,
}

impl Drop for Restore {
    fn drop(&mut self) {
        if let Err(error) = fs::write(&self.path, &self.original) {
            eprintln!("cannot restore {}: {error}", self.path.display());
        }
    }
}

fn get(url: &str) -> Option<Vec<u8>> {
    ureq::get(url).call().ok()?.body_mut().read_to_vec().ok()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerInfo {
    root_instance_id: String,
}

#[test]
fn sample_game_edit_reaches_rojo() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/sample-game").canonicalize().unwrap();
    assert!(
        sample.join("node_modules/@trioforge/crucible/transformer/index.js").is_file()
            && sample.join("node_modules/@trioforge/crucible/out/init.luau").is_file(),
        "run pnpm install and pnpm --filter @trioforge/crucible build:transformer build from the repo root first"
    );
    let port = free_port();
    let origin = format!("http://127.0.0.1:{port}");
    let mut dev = Dev::start(&sample, &["--port", &port.to_string()], &[]);
    let info = wait_until("Rojo to serve the sample game", Duration::from_secs(180), || {
        let bytes = get(&format!("{origin}/api/rojo"))?;
        Some(rmp_serde::from_slice::<ServerInfo>(&bytes).unwrap())
    });
    let read = format!("{origin}/api/read/{}", info.root_instance_id);
    let source = sample.join("src/server/main.server.ts");
    let original = fs::read(&source).unwrap();
    let probe = format!("crucible dev probe {}", std::process::id());
    let served = || get(&read).map(|bytes| bytes.windows(probe.len()).any(|window| window == probe.as_bytes()));
    let restore = Restore { path: source.clone(), original: original.clone() };
    assert_eq!(served(), Some(false));
    let mut edited = original.clone();
    edited.extend_from_slice(format!("print(\"{probe}\");\n").as_bytes());
    fs::write(&source, edited).unwrap();
    wait_until("the edit to reach Rojo", Duration::from_secs(60), || served()?.then_some(()));
    drop(restore);
    wait_until("the restored source to reach Rojo", Duration::from_secs(60), || (!served()?).then_some(()));
    let spawned = descendants(dev.pid());
    assert!(spawned.len() >= 3, "a guard, roblox-ts and Rojo: {spawned:?}");
    dev.interrupt();
    let status = dev.wait(STOP);
    assert_eq!(status.code(), Some(0), "{}", dev.stderr());
    assert_all_stop(&spawned, port);
}
