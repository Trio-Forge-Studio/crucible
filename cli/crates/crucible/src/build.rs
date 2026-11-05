use crate::child;
use crate::manifest::{Manifest, ManifestError, Place};
use crucible_compiler::{Compiled, Module, Target};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

#[derive(clap::Args)]
pub struct Args {
    /// Build only this place from crucible.toml. Repeat to build several.
    #[arg(long = "place", value_name = "NAME")]
    pub places: Vec<String>,
    /// Write the place file here instead of build/<name>.rbxl. Needs exactly one place.
    #[arg(long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

#[derive(Debug)]
pub enum Error {
    Usage(String),
    Failed(String),
}

impl Error {
    pub fn code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

pub fn main(args: Args) -> ExitCode {
    let result = std::env::current_dir()
        .map_err(|error| Error::Failed(format!("cannot read the current directory: {error}")))
        .and_then(|root| Ok((build(&root, &manifest(&root)?, &args.places, args.output.as_deref())?, root)));
    match result {
        Ok((built, root)) => {
            for (_, path) in built {
                println!("{}", path.strip_prefix(&root).unwrap_or(&path).display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("crucible build: {error}");
            ExitCode::from(error.code())
        }
    }
}

pub fn manifest(root: &Path) -> Result<Manifest, Error> {
    Manifest::read(root)
        .map_err(|error| match error {
            ManifestError::Read(_) => Error::Failed(error.to_string()),
            _ => Error::Usage(error.to_string()),
        })
        .map(Option::unwrap_or_default)
}

pub fn build(
    root: &Path,
    manifest: &Manifest,
    places: &[String],
    output: Option<&Path>,
) -> Result<Vec<(String, PathBuf)>, Error> {
    if let Some(unknown) = places.iter().find(|place| !manifest.places.contains_key(*place)) {
        let known: Vec<_> = manifest.places.keys().map(String::as_str).collect();
        return Err(Error::Usage(format!("unknown place {unknown}. Places: {}", known.join(", "))));
    }
    let selected: Vec<_> =
        manifest.places.iter().filter(|(name, _)| places.is_empty() || places.contains(name)).collect();
    if output.is_some() && selected.len() != 1 {
        return Err(Error::Usage(format!(
            "--output needs exactly one place, and this build has {}. Pick one with --place",
            selected.len()
        )));
    }
    refuse_declared_out_dir_overlaps(root, manifest)?;
    write(root, &compile_all(root, &selected)?)?;
    let mut built = Vec::new();
    for (name, place) in selected {
        let path = match output {
            Some(output) => root.join(output),
            None => root.join("build").join(format!("{name}.rbxl")),
        };
        rojo_build(&root.join(&place.project), &path)
            .map_err(|error| Error::Failed(format!("place {name}: {error}")))?;
        built.push((name.clone(), path));
    }
    Ok(built)
}

struct SharedCompile<'m> {
    place: &'m Place,
    compiled: Compiled,
}

fn compile_all<'m>(root: &Path, selected: &[(&'m String, &'m Place)]) -> Result<Vec<SharedCompile<'m>>, Error> {
    let mut compiles: Vec<SharedCompile<'m>> = Vec::new();
    for &(name, place) in selected {
        if compiles.iter().any(|shared| shared.place == place) {
            continue;
        }
        let compiled = compile(root, place).map_err(|error| Error::Failed(format!("place {name}: {error}")))?;
        compiles.push(SharedCompile { place, compiled });
    }
    Ok(compiles)
}

struct Output<'m> {
    place: &'m str,
    dir: PathBuf,
    key: PathBuf,
    inputs: [PathBuf; 3],
}

// The tsconfig, project and include together decide the code, so only places that share all three may share an outDir.
fn refuse_declared_out_dir_overlaps(root: &Path, manifest: &Manifest) -> Result<(), Error> {
    let mut outputs = Vec::new();
    for (name, place) in &manifest.places {
        let tsconfig = root.join(&place.tsconfig);
        let unknown = |why: &str, details: String| {
            Error::Failed(format!(
                "place {name}: {} {why}, so the build cannot tell where it writes. Nothing was compiled{details}",
                shown(root, &tsconfig)
            ))
        };
        let dir = match crucible_compiler::out_dir(&tsconfig) {
            Ok(Some(dir)) => dir,
            Ok(None) => return Err(unknown("sets no outDir", String::new())),
            Err(diagnostics) => {
                let details = diagnostics.iter().map(|d| format!("\n  {d}")).collect();
                return Err(unknown("cannot be read", details));
            }
        };
        let inputs =
            [&tsconfig, &root.join(&place.project), &root.join(&place.include)].map(|path| case_folded_canonical(path));
        outputs.push(Output { place: name, key: case_folded_canonical(&dir), dir, inputs });
    }
    for (a, x) in outputs.iter().enumerate() {
        for y in &outputs[a + 1..] {
            if x.key == y.key && x.inputs != y.inputs {
                return Err(Error::Failed(format!(
                    "places {} and {} compile different code into {}. Give each a tsconfig with its own outDir",
                    x.place,
                    y.place,
                    shown(root, &x.dir)
                )));
            }
            if x.key != y.key && (x.key.starts_with(&y.key) || y.key.starts_with(&x.key)) {
                return Err(Error::Failed(format!(
                    "places {} and {} compile into {} and {}, one inside the other. Give each an outDir outside the other's",
                    x.place,
                    y.place,
                    shown(root, &x.dir),
                    shown(root, &y.dir)
                )));
            }
        }
    }
    Ok(())
}

fn case_folded_canonical(path: &Path) -> PathBuf {
    let (mut existing, mut missing) = (path, Vec::new());
    let resolved = loop {
        if let Ok(resolved) = existing.canonicalize() {
            break resolved;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                existing = parent;
            }
            _ => break existing.to_path_buf(),
        }
    };
    let full = missing.iter().rev().fold(resolved, |full, name| full.join(name));
    full.to_string_lossy().to_lowercase().into()
}

fn write(root: &Path, compiles: &[SharedCompile<'_>]) -> Result<(), Error> {
    let failed =
        |path: &Path, error: std::io::Error| Error::Failed(format!("cannot write {}: {error}", shown(root, path)));
    let mut written: Vec<PathBuf> = Vec::new();
    for shared in compiles {
        let out = shared.compiled.out_dir.as_path();
        if !written.contains(&case_folded_canonical(out)) {
            let changes =
                crucible_compiler::write(out, &shared.compiled.modules).map_err(|error| failed(out, error))?;
            eprintln!(
                "crucible build: wrote {} of {} modules to {}",
                changes.written,
                shared.compiled.modules.len(),
                shown(root, out)
            );
            written.push(case_folded_canonical(out));
        }
        let include = root.join(&shared.place.include);
        crucible_compiler::write_include(&include).map_err(|error| failed(&include, error))?;
    }
    Ok(())
}

fn shown(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

pub const COMPILE: &str = "__compile";

#[derive(Serialize, Deserialize)]
struct ChildModules {
    out_dir: PathBuf,
    modules: Vec<(String, String)>,
}

// tsrs never frees a program, so each compile runs in its own process and a build's peak memory is its largest place.
fn compile(root: &Path, place: &Place) -> Result<Compiled, String> {
    let exe = std::env::current_exe().map_err(|error| format!("cannot find the crucible executable: {error}"))?;
    let mut command = Command::new(exe);
    command.arg(COMPILE).arg(&place.tsconfig).arg(&place.project).arg(&place.include);
    command.current_dir(root).stdout(Stdio::piped());
    let mut tied = child::spawn_tied(&mut command).map_err(|error| format!("cannot start the compiler: {error}"))?;
    let mut json = Vec::new();
    let read = tied.child.stdout.take().expect("stdout is piped").read_to_end(&mut json);
    let status = tied.child.wait().map_err(|error| format!("the compiler did not finish: {error}"))?;
    match status.code() {
        Some(0) => {}
        Some(1) => return Err("the compile failed, so nothing was written".into()),
        _ => return Err(format!("the compiler stopped unexpectedly ({status}), so nothing was written")),
    }
    read.map_err(|error| format!("cannot read the compiler's output: {error}"))?;
    let child: ChildModules =
        serde_json::from_slice(&json).map_err(|error| format!("cannot read the compiler's output: {error}"))?;
    let modules = child.modules.into_iter().map(|(path, source)| Module { path, source }).collect();
    Ok(Compiled { out_dir: child.out_dir, modules })
}

// The checker recurses as deep as the source nests, so the compile gets the stack tsrs's own command line gives it.
pub fn compile_main(tsconfig: PathBuf, project: PathBuf, include: PathBuf) -> ExitCode {
    child::exit_with_parent();
    match std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(move || compile_and_send(&tsconfig, &project, &include))
    {
        Ok(compile) => compile.join().unwrap_or(ExitCode::from(101)),
        Err(error) => {
            eprintln!("crucible build: cannot start the compile thread: {error}");
            ExitCode::from(2)
        }
    }
}

fn compile_and_send(tsconfig: &Path, project: &Path, include: &Path) -> ExitCode {
    match crucible_compiler::compile(tsconfig, Target::Game { project, include }) {
        Ok(compiled) => {
            let modules = compiled.modules.into_iter().map(|module| (module.path, module.source)).collect();
            let child = ChildModules { out_dir: compiled.out_dir, modules };
            match serde_json::to_writer(std::io::stdout().lock(), &child) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("crucible build: cannot send the compiled modules: {error}");
                    ExitCode::from(2)
                }
            }
        }
        Err(failure) => {
            eprint!("{}", relative_to_cwd(&failure.to_string()));
            ExitCode::FAILURE
        }
    }
}

fn relative_to_cwd(text: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_default().to_string_lossy().into_owned();
    text.replace(&(cwd.replace('\\', "/") + "/"), "").replace(&(cwd + "\\"), "")
}

// Rojo prints progress with println!, so it runs in a child whose stdout is our
// stderr, which keeps our stdout for place paths.
fn rojo_build(project: &Path, output: &Path) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let exe = std::env::current_exe().map_err(|error| format!("cannot find the crucible executable: {error}"))?;
    let status =
        child::spawn_tied(Command::new(exe).arg(ROJO_BUILD).arg(project).arg(output).stdout(std::io::stderr()))
            .and_then(|mut tied| tied.child.wait())
            .map_err(|error| format!("cannot start Rojo: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("Rojo could not build {}", project.display())) }
}

pub const ROJO_BUILD: &str = "__rojo-build";

pub fn rojo_logger() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_module_path(false)
        .format_timestamp(None)
        .init();
}

pub fn rojo_main(project: PathBuf, output: PathBuf) -> ExitCode {
    child::exit_with_parent();
    rojo_logger();
    #[cfg(debug_assertions)]
    if std::env::var_os("CRUCIBLE_TEST_HOLD_ROJO").is_some() {
        eprintln!("rojo child {} holding", std::process::id());
        loop {
            std::thread::park();
        }
    }
    let command = librojo::cli::BuildCommand { project, output: Some(output), plugin: None, watch: false };
    match command.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:?}");
            ExitCode::FAILURE
        }
    }
}
