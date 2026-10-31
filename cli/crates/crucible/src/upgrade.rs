use crate::build::{Error, node_package};
use crate::create::{FILES, package_manager, package_manager_command, render, tokens};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

#[derive(clap::Args)]
pub struct Args {
    /// The version to install. Defaults to the latest version in the registry.
    pub version: Option<String>,
    /// Install the version even when it is older than the installed one.
    #[arg(long)]
    pub allow_downgrade: bool,
}

pub const SNAPSHOT: &str = "__upgrade-snapshot";

const OWNED: &[(&str, Option<&str>)] = &[
    ("tsconfig.json", None),
    ("default.project.json", None),
    ("vite.config.ts", None),
    (".gitignore", None),
    ("LICENSE", None),
    ("AGENTS.md", Some("## Crucible")),
];

// Old and new releases exchange this as JSON, so fields are only ever added.
#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: String,
    owned: Vec<Part>,
    docs: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct Part {
    path: String,
    section: Option<String>,
    content: String,
}

impl Part {
    fn label(&self) -> String {
        match &self.section {
            Some(heading) => format!("{} ({heading})", self.path),
            None => self.path.clone(),
        }
    }
}

#[derive(Debug, PartialEq)]
enum Change {
    Current,
    Write,
    Edited,
}

pub fn snapshot_main(name: String) -> ExitCode {
    let tokens = tokens(&name);
    let owned = OWNED
        .iter()
        .map(|&(path, heading)| {
            let source = FILES.iter().find(|(file, _)| *file == path).expect("owned files are template files").1;
            let content = render(source, &tokens);
            let content = match heading {
                Some(heading) => content[section(&content, heading).expect("owned sections exist")].to_owned(),
                None => content,
            };
            Part { path: path.into(), section: heading.map(Into::into), content }
        })
        .collect();
    let docs = crate::docs::TOPICS
        .iter()
        .map(|topic| (topic.name.to_owned(), format!("{:x}", Sha256::digest(topic.markdown))))
        .collect();
    let snapshot = Snapshot { version: env!("CARGO_PKG_VERSION").into(), owned, docs };
    println!("{}", serde_json::to_string(&snapshot).expect("snapshots serialize"));
    ExitCode::SUCCESS
}

pub fn main(args: Args) -> ExitCode {
    let result = std::env::current_dir()
        .map_err(|error| Error::Failed(format!("cannot read the current directory: {error}")))
        .and_then(|root| upgrade(&root, args));
    match result {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("crucible upgrade: {error}");
            ExitCode::from(error.code())
        }
    }
}

fn upgrade(root: &Path, args: Args) -> Result<String, Error> {
    let manifest_path = root.join("package.json");
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|error| {
        Error::Failed(format!(
            "cannot read {}: {error}. Run crucible upgrade from the game's root",
            manifest_path.display()
        ))
    })?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text)
        .map_err(|error| Error::Failed(format!("{} is not valid JSON: {error}", manifest_path.display())))?;
    let name = manifest["name"]
        .as_str()
        .ok_or_else(|| Error::Failed(format!("{} has no name", manifest_path.display())))?
        .to_owned();
    let manager = detect_manager(root);
    let installed = installed_version(root)?.ok_or_else(|| {
        Error::Failed(format!("crucible is not installed in {}. Run {manager} install first", root.display()))
    })?;
    let target = match &args.version {
        Some(version) => version
            .parse()
            .map_err(|_| Error::Usage(format!("{version} is not a version, such as {}", env!("CARGO_PKG_VERSION"))))?,
        None => latest(root)?,
    };
    if target == installed {
        return Ok(format!("crucible is already at {installed}.\n"));
    }
    if target < installed && !args.allow_downgrade {
        return Err(Error::Usage(format!(
            "{target} is older than the installed {installed}. Pass --allow-downgrade to install it anyway"
        )));
    }
    let old = snapshot(root, &name);
    if old.is_none() {
        eprintln!(
            "crucible upgrade: crucible {installed} cannot describe its template, so no template file is refreshed"
        );
    }
    let bumped = bump(&manifest_text, &target.to_string())?;
    fs::write(&manifest_path, &bumped)
        .map_err(|error| Error::Failed(format!("cannot write {}: {error}", manifest_path.display())))?;
    eprintln!("crucible upgrade: installing crucible {target} with {manager}");
    if let Err(error) = install(root, manager) {
        return Err(match fs::write(&manifest_path, &manifest_text) {
            Ok(()) => Error::Failed(format!("{error}. package.json is restored")),
            Err(restore) => Error::Failed(format!("{error}. Restoring package.json also failed: {restore}")),
        });
    }
    match installed_version(root)? {
        Some(version) if version == target => {}
        other => {
            return Err(Error::Failed(format!(
                "{manager} install left crucible at {}, not {target}",
                other.map_or("nothing".into(), |version| version.to_string())
            )));
        }
    }
    let new = snapshot(root, &name)
        .ok_or_else(|| Error::Failed(format!("the installed crucible {target} cannot describe its template")))?;
    let mut written = Vec::new();
    let mut edited = Vec::new();
    for part in &new.owned {
        let file = root.join(&part.path);
        let current = fs::read_to_string(&file).ok();
        let old_part = old.as_ref().map(|old| {
            old.owned.iter().find(|candidate| candidate.path == part.path && candidate.section == part.section)
        });
        let range = |text: &str| match &part.section {
            Some(heading) => section(text, heading),
            None => Some(0..text.len()),
        };
        let current_part = current.as_deref().and_then(|text| Some(&text[range(text)?]));
        let whole_file = part.section.is_none();
        match reconcile(
            old_part.map(|old| old.map(|old| old.content.as_str())),
            &part.content,
            current_part,
            whole_file,
        ) {
            Change::Current => {}
            Change::Edited => edited.push(part.label()),
            Change::Write => {
                let content = match (&current, current.as_deref().and_then(range)) {
                    (Some(text), Some(range)) if !whole_file => {
                        format!("{}{}{}", &text[..range.start], part.content, &text[range.end..])
                    }
                    _ => part.content.clone(),
                };
                fs::write(&file, content)
                    .map_err(|error| Error::Failed(format!("cannot write {}: {error}", file.display())))?;
                written.push(part.label());
            }
        }
    }
    let topics: Vec<_> = new
        .docs
        .iter()
        .filter(|(topic, hash)| old.as_ref().is_none_or(|old| old.docs.get(*topic) != Some(hash)))
        .map(|(topic, _)| topic.as_str())
        .collect();
    let mut report = format!("Upgraded crucible and @trioforge/crucible from {installed} to {target}.\n");
    if !written.is_empty() {
        report += "\nRefreshed from the new template:\n";
        report.extend(written.iter().map(|label| format!("  {label}\n")));
    }
    if !edited.is_empty() {
        report += if old.is_some() {
            "\nThe new template changed these, but this project edited them, so they were left alone:\n"
        } else {
            "\nThese differ from the new template, and the old template is unknown, so they were left alone:\n"
        };
        report.extend(edited.iter().map(|label| format!("  {label}\n")));
        report += &format!(
            "Merge them by hand. `npx crucible create {name} <empty-dir> --no-install` writes the new template.\n"
        );
    }
    if !topics.is_empty() {
        report += "\nThese docs changed. Read them before you continue:\n";
        report.extend(topics.iter().map(|topic| format!("  npx crucible docs {topic}\n")));
    }
    report += &format!("\nRun `{manager} run check` next.\n");
    Ok(report)
}

// `old` is `None` when the old template is unknown and `Some(None)` when it lacked this part.
fn reconcile(old: Option<Option<&str>>, new: &str, current: Option<&str>, whole_file: bool) -> Change {
    let same = |a: &str, b: &str| a.replace("\r\n", "\n") == b.replace("\r\n", "\n");
    match (old, current) {
        (_, Some(current)) if same(current, new) => Change::Current,
        (Some(Some(old)), _) if same(old, new) => Change::Current,
        (Some(Some(old)), Some(current)) if same(current, old) => Change::Write,
        (Some(None), None) if whole_file => Change::Write,
        _ => Change::Edited,
    }
}

// Trailing whitespace is excluded so the section still matches when the game appends its own sections.
fn section(text: &str, heading: &str) -> Option<Range<usize>> {
    let level = heading.bytes().take_while(|&byte| byte == b'#').count();
    let mut start = None;
    let mut fenced = false;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end();
        if bare.starts_with("```") {
            fenced = !fenced;
        } else if !fenced && start.is_none() && bare == heading {
            start = Some(offset);
        } else if !fenced && start.is_some() {
            let hashes = bare.bytes().take_while(|&byte| byte == b'#').count();
            if (1..=level).contains(&hashes) && bare[hashes..].starts_with(' ') {
                break;
            }
        }
        offset += line.len();
    }
    let start = start?;
    Some(start..start + text[start..offset].trim_end().len())
}

// Edits the version strings in place, because reserializing would reformat the file and fail the game's format check.
fn bump(text: &str, version: &str) -> Result<String, Error> {
    let mut expected: serde_json::Value = serde_json::from_str(text).expect("package.json was parsed before");
    let mut result = text.to_owned();
    let mut found = false;
    for group in ["dependencies", "devDependencies"] {
        for package in ["crucible", "@trioforge/crucible"] {
            let Some(spec) = expected[group].get_mut(package) else { continue };
            let Some(old) = spec.as_str() else { continue };
            let key = serde_json::to_string(package).expect("strings serialize");
            let value = serde_json::to_string(old).expect("strings serialize");
            let new = serde_json::to_string(version).expect("strings serialize");
            let mut edited = String::with_capacity(result.len());
            let mut rest = result.as_str();
            while let Some(index) = rest.find(&key) {
                let after = &rest[index + key.len()..];
                let colon = after.trim_start();
                let matched = colon.strip_prefix(':').map(str::trim_start).filter(|tail| tail.starts_with(&value));
                match matched {
                    Some(tail) => {
                        let consumed = rest.len() - tail.len();
                        edited.push_str(&rest[..consumed]);
                        edited.push_str(&new);
                        rest = &tail[value.len()..];
                    }
                    None => {
                        edited.push_str(&rest[..index + key.len()]);
                        rest = after;
                    }
                }
            }
            edited.push_str(rest);
            result = edited;
            *spec = version.into();
            found |= package == "crucible";
        }
    }
    if !found {
        return Err(Error::Failed("package.json does not list crucible in dependencies or devDependencies".into()));
    }
    if serde_json::from_str::<serde_json::Value>(&result).ok().as_ref() != Some(&expected) {
        return Err(Error::Failed(format!(
            "cannot update package.json automatically. Set crucible and @trioforge/crucible to {version} and run install"
        )));
    }
    Ok(result)
}

fn detect_manager(root: &Path) -> &'static str {
    let locks = [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
        ("package-lock.json", "npm"),
    ];
    locks
        .iter()
        .find(|(lock, _)| root.join(lock).is_file())
        .map(|(_, manager)| *manager)
        .unwrap_or_else(|| package_manager(&std::env::var("npm_config_user_agent").unwrap_or_default()))
}

fn installed_version(root: &Path) -> Result<Option<semver::Version>, Error> {
    let Some(package) = node_package(root, "crucible") else { return Ok(None) };
    let manifest: serde_json::Value = fs::read(package.join("package.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    manifest["version"]
        .as_str()
        .and_then(|version| version.parse().ok())
        .map(Some)
        .ok_or_else(|| Error::Failed(format!("{} has no valid version", package.join("package.json").display())))
}

fn latest(root: &Path) -> Result<semver::Version, Error> {
    eprintln!("crucible upgrade: finding the latest crucible with npm view");
    let output = package_manager_command("npm")
        .args(["view", "crucible", "version"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| Error::Failed(format!("cannot run npm view: {error}")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        return Err(Error::Failed("npm view crucible version failed. Pass a version to skip the lookup".into()));
    }
    text.trim().parse().map_err(|_| Error::Failed(format!("npm view returned {:?}, not a version", text.trim())))
}

fn install(root: &Path, manager: &str) -> Result<(), String> {
    let status = package_manager_command(manager)
        .arg("install")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(std::io::stderr())
        .status()
        .map_err(|error| format!("cannot run {manager} install: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("{manager} install failed")) }
}

fn snapshot(root: &Path, name: &str) -> Option<Snapshot> {
    let package = node_package(root, "crucible")?;
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(package.join("package.json")).ok()?).ok()?;
    let bin = manifest["bin"]["crucible"].as_str().or(manifest["bin"].as_str())?;
    let output = Command::new("node")
        .arg(package.join(bin))
        .args([SNAPSHOT, name])
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then(|| serde_json::from_slice(&output.stdout).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_refreshes_only_untouched_parts() {
        use Change::*;
        let cases = [
            (Some(Some("a")), "b", Some("a"), true, Write),
            (Some(Some("a")), "b", Some("a\r\n"), true, Edited),
            (Some(Some("a\n")), "b\n", Some("a\r\n"), true, Write),
            (Some(Some("a")), "b", Some("edited"), true, Edited),
            (Some(Some("a")), "b", None, true, Edited),
            (Some(Some("a")), "a", Some("edited"), true, Current),
            (Some(Some("a")), "b", Some("b"), true, Current),
            (Some(None), "b", None, true, Write),
            (Some(None), "b", None, false, Edited),
            (Some(None), "b", Some("mine"), true, Edited),
            (None, "b", Some("a"), true, Edited),
            (None, "b", None, true, Edited),
            (None, "b", Some("b"), true, Current),
        ];
        for (old, new, current, whole_file, expected) in cases {
            assert_eq!(reconcile(old, new, current, whole_file), expected, "{old:?} {new:?} {current:?}");
        }
    }

    #[test]
    fn a_section_ends_at_the_next_heading_outside_code() {
        let text = "# Game\n\nintro\n\n## Crucible\n\nrules\n```sh\n# not a heading\n## nor this\n```\n\n### sub\n\nmore\n\n## Mine\n\nnotes\n";
        let range = section(text, "## Crucible").unwrap();
        assert!(text[range.clone()].starts_with("## Crucible\n"));
        assert!(text[range.clone()].ends_with("more"));
        assert!(text[range.end..].starts_with("\n\n## Mine"));
        let last = "## Crucible\nlast\n\n";
        assert_eq!(&last[section(last, "## Crucible").unwrap()], "## Crucible\nlast");
        assert_eq!(section(text, "## Missing"), None);
    }

    #[test]
    fn bump_edits_only_the_two_versions() {
        let text = "{\n\t\"name\": \"crucible-game\",\n\t\"scripts\": { \"crucible\": \"^0.1.0\" },\n\t\"dependencies\": {\n\t\t\"@trioforge/crucible\": \"0.1.0\"\n\t},\n\t\"devDependencies\": {\n\t\t\"crucible\" :  \"^0.1.0\"\n\t}\n}\n";
        let bumped = bump(text, "0.2.0");
        assert!(bumped.is_err(), "an ambiguous script entry is not rewritten silently");
        let text = text.replace("{ \"crucible\": \"^0.1.0\" }", "{}");
        let bumped = bump(&text, "0.2.0").unwrap();
        assert_eq!(bumped, text.replace("\"0.1.0\"", "\"0.2.0\"").replace("\"^0.1.0\"", "\"0.2.0\""));
        assert!(bump("{\"dependencies\":{}}", "0.2.0").is_err());
    }

    #[test]
    fn the_snapshot_owns_the_crucible_section_of_agents() {
        let agents = render(FILES.iter().find(|(path, _)| *path == "AGENTS.md").unwrap().1, &tokens("game"));
        let range = section(&agents, "## Crucible").unwrap();
        assert!(agents[range.clone()].contains("npx crucible upgrade"));
        assert_eq!(range.end, agents.trim_end().len());
    }

    #[test]
    fn lockfiles_pick_the_package_manager() {
        let dir = tempfile::tempdir().unwrap();
        for (lock, manager) in [("package-lock.json", "npm"), ("bun.lockb", "bun"), ("yarn.lock", "yarn")] {
            fs::write(dir.path().join(lock), "").unwrap();
            assert_eq!(detect_manager(dir.path()), manager);
        }
        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_manager(dir.path()), "pnpm");
    }
}
