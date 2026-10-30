use crate::manifest::Manifest;
use sha2::{Digest, Sha256};
use std::fmt;
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
        .and_then(|root| {
            let manifest = Manifest::read(&root).map_err(|error| Error::Usage(error.to_string()))?.unwrap_or_default();
            Ok((build(&root, &manifest, &args.places, args.output.as_deref())?, root))
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
    compile(root)?;
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

pub fn compile(root: &Path) -> Result<(), Error> {
    let receipt = Receipt::prepare(root)?;
    let status = rbxtsc(root)?
        .args(["--type", "game"])
        .status()
        .map_err(|error| Error::Failed(format!("cannot run node: {error}. Install Node.js and put node on PATH")))?;
    if !status.success() {
        return Err(Error::Failed("roblox-ts failed".into()));
    }
    receipt.map_or(Ok(()), |receipt| receipt.write())
}

pub fn rbxtsc(root: &Path) -> Result<Command, Error> {
    let package = node_package(root, "roblox-ts").ok_or_else(|| {
        Error::Failed(format!("roblox-ts is not installed in {} or a parent node_modules", root.display()))
    })?;
    let manifest: serde_json::Value = std::fs::read(package.join("package.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let bin = manifest["bin"]["rbxtsc"]
        .as_str()
        .or(manifest["bin"].as_str())
        .ok_or_else(|| Error::Failed(format!("{} declares no rbxtsc bin", package.display())))?;
    let mut command = Command::new("node");
    command.arg(package.join(bin)).current_dir(root).stdin(Stdio::null()).stdout(std::io::stderr());
    Ok(command)
}

pub struct Receipt {
    path: PathBuf,
    fingerprint: String,
}

impl Receipt {
    pub fn prepare(root: &Path) -> Result<Option<Self>, Error> {
        let Some(package) = node_package(root, "@trioforge/crucible") else {
            return Ok(None);
        };
        let transformer = package.join("transformer");
        let mut files: Vec<_> = std::fs::read_dir(&transformer)
            .and_then(|entries| entries.map(|entry| entry.map(|entry| entry.path())).collect())
            .map_err(|error| Error::Failed(format!("cannot read {}: {error}", transformer.display())))?;
        files.sort();
        let mut hash = Sha256::new();
        for file in files.iter().filter(|file| file.is_file()) {
            let bytes = std::fs::read(file)
                .map_err(|error| Error::Failed(format!("cannot read {}: {error}", file.display())))?;
            hash.update(file.file_name().unwrap_or_default().as_encoded_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
        let fingerprint = format!("{:x}", hash.finalize());
        let out = root.join("out");
        let path = out.join("transformer.sha256");
        if out.exists() && std::fs::read_to_string(&path).ok().as_deref() != Some(&fingerprint) {
            eprintln!("crucible build: the installed Crucible transformer changed, compiling from clean");
            std::fs::remove_dir_all(&out)
                .map_err(|error| Error::Failed(format!("cannot remove {}: {error}", out.display())))?;
        }
        Ok(Some(Self { path, fingerprint }))
    }

    pub fn write(&self) -> Result<(), Error> {
        std::fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")))
            .and_then(|()| std::fs::write(&self.path, &self.fingerprint))
            .map_err(|error| Error::Failed(format!("cannot write {}: {error}", self.path.display())))
    }
}

fn node_package(root: &Path, name: &str) -> Option<PathBuf> {
    root.ancestors()
        .map(|dir| dir.join("node_modules").join(name))
        .find(|package| package.join("package.json").is_file())
}

// Rojo prints progress with println!, so it runs in a child whose stdout is our
// stderr, which keeps our stdout for place paths.
fn rojo_build(project: &Path, output: &Path) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let exe = std::env::current_exe().map_err(|error| format!("cannot find the crucible executable: {error}"))?;
    let status = Command::new(exe)
        .arg(ROJO_BUILD)
        .arg(project)
        .arg(output)
        .stdin(Stdio::null())
        .stdout(std::io::stderr())
        .status()
        .map_err(|error| format!("cannot start Rojo: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("Rojo could not build {}", project.display())) }
}

pub const ROJO_BUILD: &str = "__rojo-build";

pub fn rojo_main(project: PathBuf, output: PathBuf) -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_module_path(false)
        .format_timestamp(None)
        .init();
    let command = librojo::cli::BuildCommand { project, output: Some(output), plugin: None, watch: false };
    match command.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:?}");
            ExitCode::FAILURE
        }
    }
}
