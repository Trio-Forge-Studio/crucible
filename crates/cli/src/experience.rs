//! `crucible build`'s half of `crucible:experience`: it writes the declaration game code type-checks against, and
//! gives each place's compile the bindings of the environment it builds for.

use crate::build::Error;
use crucible_iac::bindings::{self, Build};
use crucible_iac::lock::{self, Lock};
use crucible_iac::model::{Experience, Kind, Lifecycle, Problem};
use crucible_project::Manifest;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

/// Writes `src/crucible-experience.d.ts` when `crucible.toml` declares a product, a pass or a badge, or the file
/// exists, and returns the bindings' Luau for `env`. Returns `None` for a game with neither, which gets no module.
pub fn prepare(root: &Path, manifest: &Manifest, env: Option<&str>) -> Result<Option<String>, Error> {
    let env = env.map(|name| environment(manifest, name)).transpose()?;
    let declared = &manifest.experience;
    let path = root.join(bindings::DECLARATION);
    if declared.products.is_empty() && declared.passes.is_empty() && declared.badges.is_empty() && !path.exists() {
        return Ok(None);
    }
    let experience = Experience::new(manifest)
        .map_err(|problems| broken("crucible.toml breaks the experience rules:", &problems))?;
    match env {
        None => bind(&path, &experience, Build::Unbound),
        Some((name, env)) => {
            let lock = match std::fs::read_to_string(root.join(lock::FILE)) {
                Ok(text) => Some(Lock::parse(&text).map_err(|error| Error::Usage(error.to_string()))?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(Error::Failed(format!("cannot read {}: {error}", lock::FILE))),
            };
            bind(&path, &experience, Build::Env { name, env, lock: lock.as_ref().and_then(|lock| lock.env(name)) })
        }
    }
}

fn bind(path: &Path, experience: &Experience, build: Build<'_>) -> Result<Option<String>, Error> {
    let source = bindings::bindings(experience, build)
        .map_err(|problems| broken("this build cannot bind crucible:experience:", &problems))?;
    write_if_changed(path, bindings::declaration(experience).as_bytes())
        .map_err(|error| Error::Failed(format!("cannot write {}: {error}", bindings::DECLARATION)))?;
    Ok(Some(source))
}

/// Fails when no place of the game grants a product that is neither retired nor untracked, because Roblox would charge
/// for it and no server would grant it. `granted` holds the keys every place's compile granted through their handles.
pub fn refuse_ungranted<'g>(manifest: &Manifest, granted: impl IntoIterator<Item = &'g String>) -> Result<(), Error> {
    let granted: BTreeSet<&str> = granted.into_iter().map(String::as_str).collect();
    let experience = Experience::new(manifest)
        .map_err(|problems| broken("crucible.toml breaks the experience rules:", &problems))?;
    let problems: Vec<Problem> = experience
        .resources()
        .filter(|resource| resource.kind == Kind::Product && resource.lifecycle == Lifecycle::Managed)
        .filter(|resource| !granted.contains(resource.key.as_str()))
        .map(|resource| {
            let handle = resource.handle();
            Problem::at(
                resource.line,
                format!("{}.{}", Kind::Product.table(), resource.key),
                format!(
                    "no place grants {handle}, so a player could buy it and never receive it. Grant it once with a \
                     profile store's grant({handle}, ...) in a place that sells it"
                ),
            )
        })
        .collect();
    match problems.is_empty() {
        true => Ok(()),
        false => Err(Error::Failed(
            problems.iter().fold("no place of this game grants these products:".to_owned(), |text, problem| {
                text + &format!("\n  {problem}")
            }),
        )),
    }
}

fn environment<'m>(manifest: &'m Manifest, name: &'m str) -> Result<(&'m str, &'m crucible_project::Env), Error> {
    manifest.envs.get(name).map(|env| (name, env)).ok_or_else(|| match manifest.envs.is_empty() {
        true => Error::Usage(format!("unknown environment {name}. crucible.toml configures none")),
        false => Error::Usage(format!(
            "unknown environment {name}. Environments: {}",
            manifest.envs.keys().map(String::as_str).collect::<Vec<_>>().join(", ")
        )),
    })
}

fn broken(message: &str, problems: &[Problem]) -> Error {
    Error::Usage(problems.iter().fold(message.to_owned(), |text, problem| text + &format!("\n  {problem}")))
}

/// An editor, a formatter or a watcher may read the file at any time, so it changes only by a rename, and only when
/// its bytes differ, which keeps its time stamp and the watchers quiet.
fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    match std::fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    crucible_iac::provision::write_atomic(path, bytes)
}

/// The output path of the bindings: in the shallowest folder of the output that the place maps under
/// ReplicatedStorage, because the server and the client both read the handles.
pub fn module_path(folders: &[(PathBuf, Vec<String>)], out_dir: &Path) -> Result<String, String> {
    let out_dir = normalize(&std::path::absolute(out_dir).map_err(|error| error.to_string())?);
    let (relative, _) = folders
        .iter()
        .filter(|(_, instance)| instance.first().map(String::as_str) == Some("ReplicatedStorage"))
        .filter_map(|(folder, instance)| Some((normalize(folder).strip_prefix(&out_dir).ok()?.to_path_buf(), instance)))
        .min_by(|(a, x), (b, y)| x.len().cmp(&y.len()).then_with(|| a.cmp(b)))
        .ok_or_else(|| {
            format!(
                "the Rojo project maps no folder of {} under ReplicatedStorage, where {} goes. Map one, such as \
                 {}/shared",
                out_dir.display(),
                bindings::SPECIFIER,
                out_dir.file_name().unwrap_or_default().to_string_lossy()
            )
        })?;
    let mut parts: Vec<String> =
        relative.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect();
    parts.push("crucible-experience.luau".into());
    Ok(parts.join("/"))
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            part => out.push(part),
        }
    }
    out
}
