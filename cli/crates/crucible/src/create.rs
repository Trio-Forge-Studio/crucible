use crate::build::Error;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
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

pub const FILES: &[(&str, &str)] = template![
    "package.json",
    "tsconfig.json",
    "default.project.json",
    "vite.config.ts",
    "crucible.toml",
    "AGENTS.md",
    "LICENSE",
    ".gitignore",
    ".gitattributes",
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
        || !portable_component(name)
    {
        return Err(Error::Usage(
            "name must be a lowercase npm package name starting with a letter or digit (at most 214 bytes), without a trailing dot or space or a Windows device name".into(),
        ));
    }
    let path = args.path.unwrap_or_else(|| PathBuf::from(name));
    validate_target(&path)?;
    if path.exists() {
        let mut entries =
            fs::read_dir(&path).map_err(|error| Error::Usage(format!("cannot use {}: {error}", path.display())))?;
        if entries.next().is_some() {
            return Err(Error::Usage(format!("{} is not empty", path.display())));
        }
    }
    fs::create_dir_all(&path).map_err(|error| Error::Failed(format!("cannot create {}: {error}", path.display())))?;
    let tokens = tokens(name);
    let agent = std::env::var("npm_config_user_agent").unwrap_or_default();
    let manager = package_manager(&agent);
    let yarn_config = [(".yarnrc.yml", "nodeLinker: node-modules\n")];
    let extra_files = if manager == "yarn" { &yarn_config[..] } else { &[] };
    for (relative, source) in FILES.iter().chain(extra_files) {
        let file = path.join(relative);
        let content = render(source, &tokens);
        fs::create_dir_all(file.parent().expect("template files have parents"))
            .and_then(|()| fs::OpenOptions::new().write(true).create_new(true).open(&file))
            .and_then(|mut file| file.write_all(content.as_bytes()))
            .map_err(|error| Error::Failed(format!("cannot write {}: {error}", file.display())))?;
    }
    eprintln!("crucible create: wrote {}", path.display());
    if !args.no_install {
        eprintln!("crucible create: running {manager} install");
        let status = package_manager_command(manager)
            .arg("install")
            .current_dir(&path)
            .stdin(Stdio::null())
            .stdout(std::io::stderr())
            .status()
            .map_err(|error| {
                Error::Failed(format!(
                    "cannot run {manager} install: {error}. The project remains in {}",
                    path.display()
                ))
            })?;
        if !status.success() {
            return Err(Error::Failed(format!("{manager} install failed. Retry it in {}", path.display())));
        }
    }
    Ok(path)
}

fn portable_component(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or_default().trim_end().to_ascii_lowercase();
    !name.ends_with(['.', ' '])
        && !name.chars().any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        && !matches!(base.as_str(), "con" | "prn" | "aux" | "nul")
        && !(base.len() == 4
            && (base.starts_with("com") || base.starts_with("lpt"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}

fn validate_target(path: &Path) -> Result<(), Error> {
    if path.as_os_str().is_empty() {
        return Err(Error::Usage("target path cannot be empty".into()));
    }
    for component in path.components() {
        if let Component::Normal(name) = component
            && !name.to_str().is_some_and(portable_component)
        {
            return Err(Error::Usage(format!(
                "invalid target component {}: trailing dots, trailing spaces, Windows device names and reserved characters are not allowed",
                name.to_string_lossy()
            )));
        }
    }
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>().into_iter().rev() {
        match fs::metadata(ancestor) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err(Error::Usage(format!("{} is not a directory", ancestor.display())));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(Error::Usage(format!("cannot use {}: {error}", ancestor.display())));
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn package_manager_command(manager: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/d", "/s", "/c", manager]);
        command
    } else {
        Command::new(manager)
    }
}

pub fn package_manager(agent: &str) -> &'static str {
    match agent.split('/').next().unwrap_or_default() {
        "pnpm" => "pnpm",
        "yarn" => "yarn",
        "bun" => "bun",
        _ => "npm",
    }
}

pub fn tokens(name: &str) -> Vec<(&'static str, String)> {
    let workspace: serde_json::Value =
        serde_json::from_str(include_str!("../../../../package.json")).expect("workspace package.json is valid");
    let sample: serde_json::Value = serde_json::from_str(include_str!("../../../../examples/sample-game/package.json"))
        .expect("sample package.json is valid");
    let mut tokens = vec![("__NAME__", name.into()), ("__VERSION__", env!("CARGO_PKG_VERSION").into())];
    for (token, package, manifest) in [
        ("__ROBLOX_TS__", "roblox-ts", &workspace),
        ("__TYPESCRIPT__", "typescript", &workspace),
        ("__VITE_PLUS__", "vite-plus", &workspace),
        ("__RBXTS_COMPILER_TYPES__", "@rbxts/compiler-types", &sample),
        ("__RBXTS_TYPES__", "@rbxts/types", &sample),
    ] {
        tokens
            .push((token, manifest["devDependencies"][package].as_str().expect("toolchain version is pinned").into()));
    }
    tokens
}

pub fn render(source: &str, tokens: &[(&str, String)]) -> String {
    tokens.iter().fold(source.to_owned(), |result, (token, value)| result.replace(token, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_token_is_rendered() {
        let tokens = tokens("my-game");
        for (path, source) in FILES {
            let rendered = render(source, &tokens);
            for (start, _) in rendered.match_indices("__") {
                let rest = &rendered[start + 2..];
                if let Some(end) = rest.find("__") {
                    let token = &rest[..end];
                    assert!(
                        token.is_empty() || !token.bytes().all(|c| c.is_ascii_uppercase() || c == b'_'),
                        "unresolved token in {path}: {token}"
                    );
                }
            }
        }
    }

    #[test]
    fn device_names_are_rejected_on_every_platform() {
        let mut reserved = vec!["con".to_owned(), "prn".into(), "aux".into(), "nul".into()];
        for prefix in ["com", "lpt"] {
            for number in 1..=9 {
                reserved.push(format!("{prefix}{number}"));
            }
        }
        for name in reserved {
            for spelling in
                [name.clone(), name.to_uppercase(), format!("{name}.txt"), format!("{}.JSON", name.to_uppercase())]
            {
                assert!(!portable_component(&spelling), "{spelling}");
            }
        }
        for name in ["com0", "com10", "lpt0", "lpt10", "console", "auxiliary", "with spaces"] {
            assert!(portable_component(name), "{name}");
        }
    }

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
