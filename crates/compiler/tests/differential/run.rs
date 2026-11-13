//! Runs a batch of programs: one compile in a child process, one Node process and one Lune process.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The environment variable that turns the `compile_child` test into the compile step of a batch.
pub const CHILD_TSCONFIG: &str = "CRUCIBLE_DIFFERENTIAL_TSCONFIG";

/// What one program did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Both sides ran. The lines are Node's and Lune's traces.
    Ran { node: Vec<String>, lune: Vec<String> },
    /// The compiler refused the program: a type error, a Crucible rule or a lowering error.
    Rejected(String),
    /// One side printed nothing for the program within the time limit.
    Hung(&'static str),
}

/// How a program fails the test, which shrinking keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The traces differ.
    Differs,
    /// JavaScript threw an engine error, which a generated program never should: the generator is wrong.
    NodeError,
    Rejected,
    Hung,
}

impl Outcome {
    pub fn failure(&self) -> Option<Failure> {
        match self {
            Outcome::Ran { node, .. } if node.iter().any(|l| l.starts_with("!error ")) => Some(Failure::NodeError),
            Outcome::Ran { node, lune } if node != lune => Some(Failure::Differs),
            Outcome::Ran { .. } => None,
            Outcome::Rejected(_) => Some(Failure::Rejected),
            Outcome::Hung(_) => Some(Failure::Hung),
        }
    }
}

pub struct Runner {
    root: PathBuf,
    tsconfig_base: PathBuf,
    type_roots: [PathBuf; 2],
    differential: PathBuf,
    vendored: PathBuf,
    child: PathBuf,
    batches: usize,
    /// Keep each batch's folder, with its Luau, instead of removing it.
    pub keep: bool,
}

impl Runner {
    pub fn new(root: PathBuf) -> Runner {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repository = manifest.join("../..");
        let modules = repository.join("packages/crucible/node_modules");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the differential folder is made");
        // A copy of this test binary, because a build while the test runs replaces the binary itself.
        let exe = std::env::current_exe().expect("the test binary has a path");
        let child = root.join("compile-child").with_extension(exe.extension().unwrap_or_default());
        std::fs::copy(&exe, &child).expect("the test binary is copied");
        Runner {
            root,
            tsconfig_base: repository.join("tsconfig.json"),
            type_roots: [modules.join("@trioforge"), modules.join("@rbxts")],
            differential: manifest.join("tests/differential"),
            vendored: manifest.join("roblox-ts"),
            child,
            batches: 0,
            keep: false,
        }
    }

    /// Runs every program and returns their outcomes in order. A program the compiler refuses fails the whole
    /// compile, so the batch compiles again without it.
    pub fn run(&mut self, programs: &[String]) -> Vec<Outcome> {
        let mut outcomes: Vec<Option<Outcome>> = vec![None; programs.len()];
        loop {
            let pending: Vec<usize> = (0..programs.len()).filter(|&i| outcomes[i].is_none()).collect();
            if pending.is_empty() {
                break;
            }
            let dir = self.batch(&pending, programs);
            match compile(&self.child, &dir.join("tsconfig.json")) {
                Ok(()) => {
                    for (i, outcome) in self.execute(&dir, &pending) {
                        outcomes[i] = Some(outcome);
                    }
                }
                Err(text) => {
                    let mut blamed: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
                    for line in text.lines() {
                        if let Some(i) = blamed_program(line) {
                            blamed.entry(i).or_default().push(line);
                        }
                    }
                    assert!(!blamed.is_empty(), "the batch fails to compile and names no program:\n{text}");
                    for (i, lines) in blamed {
                        let lines: Vec<String> = lines.iter().map(|l| strip_folder(l)).collect();
                        outcomes[i] = Some(Outcome::Rejected(lines.join("\n")));
                    }
                }
            }
            if !self.keep {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
        outcomes.into_iter().map(|o| o.expect("every program has an outcome")).collect()
    }

    fn batch(&mut self, pending: &[usize], programs: &[String]) -> PathBuf {
        self.batches += 1;
        let dir = self.root.join(format!("batch-{}", self.batches));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).expect("the batch folder is made");
        for &i in pending {
            std::fs::write(src.join(format!("{}.ts", name(i))), &programs[i]).expect("the program is written");
        }
        let tsconfig = serde_json::json!({
            "extends": self.tsconfig_base,
            "compilerOptions": {
                "rootDir": "src",
                "outDir": "out",
                "typeRoots": self.type_roots,
                "types": ["compiler-types", "types"],
            },
            "include": ["src"],
        });
        std::fs::write(dir.join("tsconfig.json"), tsconfig.to_string()).expect("the tsconfig is written");
        // Node reads each program as an ES module, as TypeScript's `module: preserve` writes it.
        std::fs::write(src.join("package.json"), r#"{"type":"module"}"#).expect("package.json is written");
        dir
    }

    fn execute(&self, dir: &Path, pending: &[usize]) -> Vec<(usize, Outcome)> {
        let names: Vec<String> = pending.iter().map(|&i| name(i)).collect();
        let limit = Duration::from_secs(30 + names.len() as u64 / 4);
        let mut node = Command::new("node");
        node.arg("--disable-warning=ExperimentalWarning")
            .arg(self.differential.join("node.mjs"))
            .arg(dir.join("src"))
            .args(&names);
        let mut lune = Command::new("lune");
        lune.arg("run").arg(self.differential.join("lune.luau")).arg(dir.join("out")).arg(&self.vendored).args(&names);
        let node = traces(spawn(node, "node"), limit);
        let lune = traces(spawn(lune, "lune"), limit);
        let complete = |t: &Traces| t.finished && names.iter().all(|n| t.programs.contains_key(n));
        if (!complete(&node) || !complete(&lune)) && pending.len() > 1 {
            // One program hangs or kills its process. Run each alone to find it.
            return pending.iter().flat_map(|&i| self.execute(dir, &[i])).collect();
        }
        pending
            .iter()
            .zip(&names)
            .map(|(&i, name)| {
                let outcome = match (node.programs.get(name), lune.programs.get(name)) {
                    (Some(node), Some(lune)) => Outcome::Ran { node: node.clone(), lune: lune.clone() },
                    (None, _) => Outcome::Hung("node"),
                    (_, None) => Outcome::Hung("lune"),
                };
                (i, outcome)
            })
            .collect()
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

pub fn name(i: usize) -> String {
    format!("p{i:05}")
}

fn blamed_program(line: &str) -> Option<usize> {
    let at = line.find("/src/p")?;
    let digits: String = line[at + 6..].chars().take_while(char::is_ascii_digit).collect();
    line[at + 6 + digits.len()..].starts_with(".ts").then(|| digits.parse().ok()).flatten()
}

/// Drops the batch folder from a diagnostic, so it reads the same in every run.
fn strip_folder(line: &str) -> String {
    match line.find("/src/p") {
        Some(at) => {
            let start = line[..at].rfind(|c: char| c.is_whitespace() || c == '(').map_or(0, |s| s + 1);
            let end = at + line[at..].find(".ts").map_or(0, |e| e + 3);
            format!("{}program.ts{}", &line[..start], &line[end..])
        }
        None => line.to_owned(),
    }
}

/// Compiles the batch in a child process, because tsrs keeps every program it builds until the process exits.
fn compile(child: &Path, tsconfig: &Path) -> Result<(), String> {
    let output = Command::new(child)
        .args(["--exact", "compile_child", "--ignored", "--nocapture", "--test-threads=1", "-q"])
        .env(CHILD_TSCONFIG, tsconfig)
        .stdin(Stdio::null())
        .output()
        .expect("the compile child starts");
    let failure = tsconfig.with_file_name("failure.txt");
    if let Ok(text) = std::fs::read_to_string(&failure) {
        return Err(text);
    }
    assert!(
        output.status.success(),
        "the compile child fails:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

/// The compile step itself, run by the `compile_child` test in a child process.
pub fn compile_here(tsconfig: &Path) {
    match crucible_compiler::compile(tsconfig, crucible_compiler::Target::Package) {
        Ok(compiled) => {
            crucible_compiler::write(&compiled.out_dir, &compiled.modules).expect("the modules are written");
        }
        Err(failure) => {
            std::fs::write(tsconfig.with_file_name("failure.txt"), failure.to_string())
                .expect("the failure is written");
        }
    }
}

fn spawn(mut command: Command, what: &str) -> Child {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{what} starts (it must be on PATH): {e}"))
}

struct Traces {
    programs: BTreeMap<String, Vec<String>>,
    /// The process exited by itself in time, and successfully.
    finished: bool,
}

fn traces(mut child: Child, limit: Duration) -> Traces {
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let start = Instant::now();
    let finished = loop {
        if let Ok(Some(status)) = child.try_wait() {
            break status.success();
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let text = reader.join().unwrap_or_default();
    let _ = errors.join();
    let mut programs = BTreeMap::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix('#') {
            if let Some((name, lines)) = current.take() {
                programs.insert(name, lines);
            }
            current = Some((name.to_owned(), Vec::new()));
        } else if let Some((_, lines)) = &mut current {
            lines.push(line.to_owned());
        }
    }
    // A process that dies mid-program has printed nothing for it, because both runners print once at the end. Still,
    // only a clean exit vouches for the last program.
    if let Some((name, lines)) = current
        && finished
    {
        programs.insert(name, lines);
    }
    Traces { programs, finished }
}
