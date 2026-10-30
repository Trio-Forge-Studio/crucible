use crate::build::Error;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

#[derive(clap::Args)]
pub struct Args {
    /// The npm package name of the new game, using lowercase letters, digits, dots, hyphens, or underscores.
    pub name: String,
    /// The destination directory. Defaults to ./<name>.
    pub path: Option<PathBuf>,
    /// Write the project without installing dependencies.
    #[arg(long)]
    pub no_install: bool,
}

macro_rules! template {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_str!(concat!("../template/", $path)))),*]
    };
}

const FILES: &[(&str, &str)] = template![
    "package.json",
    "tsconfig.json",
    "default.project.json",
    "vite.config.ts",
    "crucible.toml",
    "AGENTS.md",
    "LICENSE",
    ".gitignore",
    "src/server/main.server.ts",
    "src/client/main.client.ts",
];

pub fn main(args: Args) -> ExitCode {
    match create(args) {
        Ok(path) => {
            println!("{}", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("crucible create: {error}");
            ExitCode::from(error.code())
        }
    }
}

fn create(args: Args) -> Result<PathBuf, Error> {
    let name = &args.name;
    if name.is_empty()
        || name.len() > 214
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-_.".contains(&c))
        || matches!(name.as_str(), "node_modules" | "favicon.ico")
    {
        return Err(Error::Usage(
            "name must be a lowercase npm package name starting with a letter or digit (at most 214 bytes)".into(),
        ));
    }
    let path = args.path.unwrap_or_else(|| PathBuf::from(name));
    if path.exists() {
        let mut entries =
            fs::read_dir(&path).map_err(|error| Error::Usage(format!("cannot use {}: {error}", path.display())))?;
        if entries.next().is_some() {
            return Err(Error::Usage(format!("{} is not empty", path.display())));
        }
    }
    fs::create_dir_all(&path).map_err(|error| Error::Failed(format!("cannot create {}: {error}", path.display())))?;
    for (relative, source) in FILES {
        let file = path.join(relative);
        let content = render(source, name);
        fs::create_dir_all(file.parent().expect("template files have parents"))
            .and_then(|()| fs::OpenOptions::new().write(true).create_new(true).open(&file))
            .and_then(|mut file| file.write_all(content.as_bytes()))
            .map_err(|error| Error::Failed(format!("cannot write {}: {error}", file.display())))?;
    }
    eprintln!("crucible create: wrote {}", path.display());
    if !args.no_install {
        let agent = std::env::var("npm_config_user_agent").unwrap_or_default();
        let manager = package_manager(&agent);
        eprintln!("crucible create: running {manager} install");
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/d", "/s", "/c", manager]);
            command
        } else {
            Command::new(manager)
        };
        let status =
            command.arg("install").current_dir(&path).stdin(Stdio::null()).stdout(std::io::stderr()).status().map_err(
                |error| {
                    Error::Failed(format!(
                        "cannot run {manager} install: {error}. The project remains in {}",
                        path.display()
                    ))
                },
            )?;
        if !status.success() {
            return Err(Error::Failed(format!("{manager} install failed. Retry it in {}", path.display())));
        }
    }
    Ok(path)
}

fn package_manager(agent: &str) -> &'static str {
    match agent.split('/').next().unwrap_or_default() {
        "pnpm" => "pnpm",
        "yarn" => "yarn",
        "bun" => "bun",
        _ => "npm",
    }
}

fn render(source: &str, name: &str) -> String {
    let workspace: serde_json::Value =
        serde_json::from_str(include_str!("../../../../package.json")).expect("workspace package.json is valid");
    let sample: serde_json::Value = serde_json::from_str(include_str!("../../../../examples/sample-game/package.json"))
        .expect("sample package.json is valid");
    let mut result = source.replace("__NAME__", name).replace("__VERSION__", env!("CARGO_PKG_VERSION"));
    for (token, package, manifest) in [
        ("__ROBLOX_TS__", "roblox-ts", &workspace),
        ("__TYPESCRIPT__", "typescript", &workspace),
        ("__VITE_PLUS__", "vite-plus", &workspace),
        ("__RBXTS_COMPILER_TYPES__", "@rbxts/compiler-types", &sample),
        ("__RBXTS_TYPES__", "@rbxts/types", &sample),
    ] {
        result =
            result.replace(token, manifest["devDependencies"][package].as_str().expect("toolchain version is pinned"));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_and_cli_release_together() {
        let runtime: serde_json::Value =
            serde_json::from_str(include_str!("../../../../packages/crucible/package.json")).unwrap();
        assert_eq!(runtime["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn detects_the_invoking_package_manager() {
        for (agent, expected) in [
            ("pnpm/12.15.0 npm/? node/v24", "pnpm"),
            ("npm/11 node/v24", "npm"),
            ("yarn/1.22", "yarn"),
            ("bun/1.3", "bun"),
            ("", "npm"),
            ("untrusted/1", "npm"),
        ] {
            assert_eq!(package_manager(agent), expected);
        }
    }
}
