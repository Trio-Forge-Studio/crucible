use crate::child;
use crucible_compiler::{Compiled, InstanceMap, Module, Target, VirtualModule};
use crucible_project::{Manifest, ManifestError, Place};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fmt;
use std::fs::{OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read};
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
    /// Bind crucible:experience to this environment's IDs in crucible.lock. Without it, reading a handle's ID throws.
    #[arg(long, value_name = "NAME")]
    pub env: Option<String>,
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
        .and_then(|root| {
            Ok((build(&root, &manifest(&root)?, &args.places, args.output.as_deref(), args.env.as_deref())?, root))
        });
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
    env: Option<&str>,
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
    let mut packages = crate::reactive_packages::Checker::default();
    for (name, place) in &selected {
        packages
            .check(root, &root.join(&place.project))
            .map_err(|error| Error::Failed(format!("place {name}: {error}")))?;
    }
    refuse_declared_out_dir_overlaps(root, manifest)?;
    let experience = crate::experience::prepare(root, manifest, env)?;
    let compiles = compile_all(root, &selected, experience.as_deref())?;
    // A place that grants nothing is exempt from CRU039, so only the whole game shows a product nobody grants.
    if experience.is_some() && selected.len() == manifest.places.len() {
        crate::experience::refuse_ungranted(manifest, compiles.iter().flat_map(|shared| &shared.granted))?;
    }
    write(root, &compiles)?;
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
    granted: Vec<String>,
}

fn compile_all<'m>(
    root: &Path,
    selected: &[(&'m String, &'m Place)],
    experience: Option<&str>,
) -> Result<Vec<SharedCompile<'m>>, Error> {
    let mut compiles: Vec<SharedCompile<'m>> = Vec::new();
    for &(name, place) in selected {
        if compiles.iter().any(|shared| shared.place == place) {
            continue;
        }
        let (compiled, granted) =
            compile(root, place, experience).map_err(|error| Error::Failed(format!("place {name}: {error}")))?;
        compiles.push(SharedCompile { place, compiled, granted });
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
            (Some(dir), errors) if errors.is_empty() => dir,
            (None, errors) if errors.is_empty() => return Err(unknown("sets no outDir", String::new())),
            (_, errors) => {
                let details = errors.iter().map(|d| format!("\n  {d}")).collect();
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
            let changes = crucible_compiler::write(out, &shared.compiled.modules)
                .map_err(|error| failed(&error.path, error.error))?;
            eprintln!(
                "crucible build: wrote {} of {} modules to {}",
                changes.written,
                shared.compiled.modules.len(),
                shown(root, out)
            );
            written.push(case_folded_canonical(out));
        }
        let include = root.join(&shared.place.include);
        crucible_compiler::write_include(&include).map_err(|error| failed(&error.path, error.error))?;
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
    /// The key of each product the place grants through its handle.
    granted: Vec<String>,
}

// tsrs never frees a program, so each compile runs in its own process and a build's peak memory is its largest place.
fn compile(root: &Path, place: &Place, experience: Option<&str>) -> Result<(Compiled, Vec<String>), String> {
    let exe = std::env::current_exe().map_err(|error| format!("cannot find the crucible executable: {error}"))?;
    let mut command = Command::new(exe);
    command.arg(COMPILE).arg(&place.tsconfig).arg(&place.project).arg(&place.include);
    // The bindings can outgrow a Windows command line, so they reach the child in a file.
    let bindings = experience
        .map(|source| {
            let mut file = tempfile::NamedTempFile::new()?;
            std::io::Write::write_all(&mut file, source.as_bytes())?;
            Ok::<_, std::io::Error>(file)
        })
        .transpose()
        .map_err(|error| format!("cannot write the crucible:experience bindings: {error}"))?;
    if let Some(file) = &bindings {
        command.arg("--experience").arg(file.path());
    }
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
    Ok((Compiled { out_dir: child.out_dir, modules }, child.granted))
}

pub fn compile_main(tsconfig: PathBuf, project: PathBuf, include: PathBuf, experience: Option<PathBuf>) -> ExitCode {
    child::exit_with_parent();
    let fail = |message: String| {
        eprint!("{}", relative_to_cwd(&format!("error: {message}\n")));
        ExitCode::FAILURE
    };
    let mappings = match crucible_project::rojo::place(&project) {
        Ok(mappings) => mappings,
        Err(message) => return fail(message),
    };
    let mut bindings = None;
    if let Some(file) = experience {
        let source = match std::fs::read_to_string(&file) {
            Ok(source) => source,
            Err(error) => return fail(format!("cannot read the crucible:experience bindings: {error}")),
        };
        let path = match crucible_compiler::out_dir(&tsconfig) {
            (Some(out_dir), _) => crate::experience::module_path(&mappings.folders, &out_dir),
            // The compile reports why the tsconfig has no outDir.
            (None, _) => Ok("crucible-experience.luau".to_owned()),
        };
        match path {
            Ok(path) => bindings = Some((path, source)),
            Err(message) => return fail(message),
        }
    }
    let modules: Vec<_> = bindings
        .iter()
        .map(|(path, source)| VirtualModule { specifier: crucible_iac::bindings::SPECIFIER, path, source })
        .collect();
    let instances = InstanceMap::new(project.display().to_string(), mappings.folders, mappings.files);
    match crucible_transform::compile_granting(
        &tsconfig,
        Target::Game { instances: &instances, include: &include, modules: &modules },
    ) {
        Ok((compiled, granted)) => {
            let modules = compiled.modules.into_iter().map(|module| (module.path, module.source)).collect();
            let child = ChildModules { out_dir: compiled.out_dir, modules, granted };
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

// Rojo truncates the place file it is told to write, so it builds into the place's temporary file and
// `crucible_compiler::publish` puts that file in the place's: a failed or killed build leaves the last place whole
// unless the place must be written in place. The temporary file keeps the place's extension, from which Rojo picks
// the format. Rojo reads `.rbxm` and `.rbxmx` files in its project as models, so the temporary file is absent while
// Rojo reads the project: a killed build's is removed first, and Rojo makes it. The lock that keeps a second build of
// the place out is on a `.tmp` file, which Rojo reads in no folder.
fn rojo_build(project: &Path, output: &Path) -> Result<(), String> {
    let parent = output.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let cannot = |error: std::io::Error| format!("cannot write {}: {error}", output.display());
    let lock_path = crucible_compiler::temporary_path(output, "tmp");
    let lock = OpenOptions::new().write(true).create(true).truncate(false).open(&lock_path).map_err(cannot)?;
    // A file system without locks lets a concurrent build through, as builds made before temporary files did.
    if let Err(TryLockError::WouldBlock) = lock.try_lock() {
        return Err(cannot(std::io::Error::new(std::io::ErrorKind::ResourceBusy, "another build is writing it")));
    }
    let temporary = crucible_compiler::temporary_path(output, output.extension().and_then(OsStr::to_str).unwrap_or(""));
    let built = match std::fs::remove_file(&temporary) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(cannot(error)),
        _ => rojo_child(project, &temporary, output)
            .and_then(|()| crucible_compiler::publish(&temporary, output).map_err(cannot)),
    };
    if built.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    let _ = std::fs::remove_file(&lock_path);
    drop(lock);
    built
}

// Rojo prints progress with println!, so it runs in a child whose stdout comes back here and goes to stderr, which
// keeps our stdout for place paths. Rojo names the file it wrote, so its lines name the place instead of the temporary
// file.
fn rojo_child(project: &Path, temporary: &Path, output: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| format!("cannot find the crucible executable: {error}"))?;
    let mut tied =
        child::spawn_tied(Command::new(exe).arg(ROJO_BUILD).arg(project).arg(temporary).stdout(Stdio::piped()))
            .map_err(|error| format!("cannot start Rojo: {error}"))?;
    let name = |path: &Path| path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let (from, to) = (name(temporary), name(output));
    for line in BufReader::new(tied.child.stdout.take().expect("stdout is piped")).split(b'\n') {
        let Ok(line) = line else { break };
        eprintln!("{}", String::from_utf8_lossy(&line).replace(&from, &to));
    }
    let status = tied.child.wait().map_err(|error| format!("cannot start Rojo: {error}"))?;
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
