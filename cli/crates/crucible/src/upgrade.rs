use crate::build::{Error, node_package};
use crate::create::{FILES, package_manager, package_manager_command, render, tokens};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(clap::Args)]
pub struct Args {
    /// The version to install. Defaults to the latest version in the registry.
    pub version: Option<String>,
    /// Install the version even when it is older than the installed one.
    #[arg(long)]
    pub allow_downgrade: bool,
}

pub const APPLY: &str = "__upgrade-apply";

// The hand-off contract in cli/README.md says when to raise this.
const FORMAT: u32 = 1;
const CHILD_TIMEOUT: Duration = Duration::from_secs(120);
const RUNTIME: &str = "@trioforge/crucible";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Upkeep {
    Refresh,
    // Rewriting a game's license text has legal effect, so a change is only reported.
    Report,
}

const OWNED: &[(&str, Option<&str>, Upkeep)] = &[
    ("tsconfig.json", None, Upkeep::Refresh),
    ("default.project.json", None, Upkeep::Refresh),
    ("vite.config.ts", None, Upkeep::Refresh),
    (".gitignore", None, Upkeep::Refresh),
    (".gitattributes", None, Upkeep::Refresh),
    ("LICENSE", None, Upkeep::Report),
    ("AGENTS.md", Some("## Crucible"), Upkeep::Refresh),
];

// Snapshot format 1. Never change these structs. A new shape gets a new format, new structs, and a decode arm.
#[derive(Serialize, Deserialize)]
struct SnapshotV1 {
    format: u32,
    version: String,
    owned: Vec<PartV1>,
    docs: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct PartV1 {
    path: String,
    section: Option<String>,
    // The whole rendered file, so the reading binary cuts the section with its own rules.
    content: String,
}

impl PartV1 {
    fn label(&self) -> String {
        match &self.section {
            Some(heading) => format!("{} ({heading})", self.path),
            None => self.path.clone(),
        }
    }

    fn find(&self, text: &str) -> Option<Range<usize>> {
        match &self.section {
            Some(heading) => section(text, heading),
            None => Some(0..text.len()),
        }
    }

    fn template(&self) -> Option<&str> {
        self.find(&self.content).map(|range| &self.content[range])
    }

    fn same_place(&self, other: &PartV1) -> bool {
        self.path == other.path && self.section == other.section
    }
}

#[derive(Debug, PartialEq)]
enum Change {
    Current,
    Write,
    Edited,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Manager {
    Npm,
    Pnpm,
    Yarn,
    YarnBerry,
    Bun,
}

impl Manager {
    fn program(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn | Self::YarnBerry => "yarn",
            Self::Bun => "bun",
        }
    }

    // Under CI, pnpm freezes the lockfile and Yarn Berry makes installs immutable, so both refuse the bumped package.json.
    fn install_args(self) -> &'static [&'static str] {
        match self {
            Self::Pnpm => &["install", "--no-frozen-lockfile"],
            Self::YarnBerry => &["install", "--no-immutable"],
            _ => &["install"],
        }
    }

    fn install_line(self) -> String {
        [self.program()].iter().chain(self.install_args()).copied().collect::<Vec<_>>().join(" ")
    }
}

struct Pins {
    crucible: String,
    runtime: String,
}

struct Installed {
    crucible: Version,
    runtime: Option<Version>,
}

#[derive(Debug, PartialEq)]
enum Plan {
    Current,
    NewerThanLatest(Version),
    Install,
}

pub fn main(args: Args) -> ExitCode {
    let result = std::env::current_dir()
        .map_err(|error| Error::Failed(format!("cannot read the current directory: {error}")))
        .and_then(|root| upgrade(&root, args));
    exit(result)
}

pub fn apply_main(from: Version, crucible: String, runtime: String) -> ExitCode {
    let pins = Pins { crucible, runtime };
    let mut input = String::new();
    let result = std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| Error::Failed(format!("cannot read the old template from stdin: {error}")))
        .and_then(|_| {
            std::env::current_dir()
                .map_err(|error| Error::Failed(format!("cannot read the current directory: {error}")))
        })
        .and_then(|root| apply(&root, &from, &pins, &input))
        .map(|report| print!("{report}"));
    exit(result)
}

fn exit(result: Result<(), Error>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("crucible upgrade: {error}");
            ExitCode::from(error.code())
        }
    }
}

fn upgrade(root: &Path, args: Args) -> Result<(), Error> {
    let manifest_path = root.join("package.json");
    let (manifest_text, manifest) = read_manifest(root)?;
    let name = package_name(&manifest)?;
    let pins = read_pins(&manifest)?;
    let manager = detect_manager(root, &std::env::var("npm_config_user_agent").unwrap_or_default());
    let install = manager.install_line();
    let installed = Installed {
        crucible: installed_version(root, "crucible")?.ok_or_else(|| {
            Error::Failed(format!("crucible is not installed in {}. Run `{install}` first", root.display()))
        })?,
        runtime: installed_version(root, RUNTIME)?,
    };
    let target = match &args.version {
        Some(version) => version
            .parse()
            .map_err(|_| Error::Usage(format!("{version} is not a version, such as {}", env!("CARGO_PKG_VERSION"))))?,
        None => latest(root)?,
    };
    match plan(&pins, &installed, &target, args.version.is_some(), args.allow_downgrade)? {
        Plan::Current => {
            println!("crucible is already at {target}.");
            return Ok(());
        }
        Plan::NewerThanLatest(newest) => {
            println!("This game is already on {newest}, newer than latest, {target}.");
            return Ok(());
        }
        Plan::Install => {}
    }
    let old = serde_json::to_string(&snapshot(&name)).expect("snapshots serialize");
    let bumped = bump(&manifest_text, &target.to_string())?;
    write_atomic(&manifest_path, &bumped)
        .map_err(|error| Error::Failed(format!("cannot write {}: {error}", manifest_path.display())))?;
    eprintln!(
        "crucible upgrade: running `{install}` for crucible {target}. If it is interrupted, restore package.json and run `{install}`"
    );
    if let Err(error) = run_install(root, manager) {
        let pinned = format!("crucible {} and {RUNTIME} {}", pins.crucible, pins.runtime);
        return Err(Error::Failed(match restore(&manifest_path, &manifest_text) {
            Ok(()) => format!(
                "{error}. package.json is restored to {pinned}. Run `{install}` to return node_modules to those versions"
            ),
            Err(restore) => {
                format!("{error}. Restoring package.json also failed: {restore}. Set {pinned} and run `{install}`")
            }
        }));
    }
    refresh(root, &name, &target, &installed.crucible, &pins, old, &install)
}

fn plan(
    pins: &Pins,
    installed: &Installed,
    target: &Version,
    requested: bool,
    allow_downgrade: bool,
) -> Result<Plan, Error> {
    let pinned = |spec: &str| spec.parse::<Version>().is_ok_and(|version| version == *target);
    if pinned(&pins.crucible)
        && pinned(&pins.runtime)
        && installed.crucible == *target
        && installed.runtime.as_ref() == Some(target)
    {
        return Ok(Plan::Current);
    }
    let newest = installed.runtime.as_ref().map_or(&installed.crucible, |runtime| runtime.max(&installed.crucible));
    if target < newest {
        if !requested {
            return Ok(Plan::NewerThanLatest(newest.clone()));
        }
        if !allow_downgrade {
            return Err(Error::Usage(format!(
                "{target} is older than the installed {newest}. Pass --allow-downgrade to install it anyway"
            )));
        }
    }
    Ok(Plan::Install)
}

// package.json and node_modules are at the target by now, so a rerun reports it as installed and cannot finish the refresh.
fn refresh(
    root: &Path,
    name: &str,
    target: &Version,
    from: &Version,
    pins: &Pins,
    old: String,
    install: &str,
) -> Result<(), Error> {
    hand_off(root, target, from, pins, old, install).map_err(|error| {
        Error::Failed(format!(
            "{error}. package.json and node_modules are at {target}, but the template was not refreshed, and running crucible upgrade again reports crucible as already at {target}. To compare by hand, run `npx crucible create {name} <empty-dir> --no-install` and diff {} with this game's",
            owned_labels()
        ))
    })
}

// The installed binary reconciles the template, so a fix to these rules reaches games still on an older version.
fn hand_off(
    root: &Path,
    target: &Version,
    from: &Version,
    pins: &Pins,
    old: String,
    install: &str,
) -> Result<(), Error> {
    let now = [installed_version(root, "crucible")?, installed_version(root, RUNTIME)?];
    if now.iter().any(|version| version.as_ref() != Some(target)) {
        let [crucible, runtime] = now.map(|version| version.map_or("nothing".into(), |version| version.to_string()));
        return Err(Error::Failed(format!(
            "after `{install}`, crucible is at {crucible} and {RUNTIME} at {runtime}, not {target}. Delete node_modules and run `{install}`"
        )));
    }
    let shim = shim(root).ok_or_else(|| Error::Failed("the installed crucible package has no crucible bin".into()))?;
    let mut version = Command::new("node");
    version.arg(&shim).arg("--version").current_dir(root).stdout(Stdio::piped());
    let output = finish(version, None, "crucible --version")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let reported = reported_version(&stdout).unwrap_or("nothing");
    if !output.status.success() || reported != target.to_string() {
        return Err(Error::Failed(format!(
            "the installed crucible binary reports {reported}, not {target}. A stale @trioforge/crucible-cli package may be hoisted. Delete node_modules and run `{install}`"
        )));
    }
    let mut apply = Command::new("node");
    apply.arg(&shim).args([APPLY, &from.to_string(), &pins.crucible, &pins.runtime]).current_dir(root);
    if finish(apply, Some(old), &format!("crucible {APPLY}"))?.status.success() {
        Ok(())
    } else {
        Err(Error::Failed(format!("crucible {APPLY} failed")))
    }
}

fn owned_labels() -> String {
    let labels: Vec<_> = OWNED
        .iter()
        .map(|&(path, heading, _)| match heading {
            Some(heading) => format!("the {heading} section of {path}"),
            None => path.into(),
        })
        .collect();
    let (last, rest) = labels.split_last().expect("the template owns files");
    format!("{}, and {last}", rest.join(", "))
}

fn reported_version(stdout: &str) -> Option<&str> {
    stdout.strip_prefix("crucible ")?.split_whitespace().next()
}

fn apply(root: &Path, from: &Version, pins: &Pins, input: &str) -> Result<String, Error> {
    let (_, manifest) = read_manifest(root)?;
    let name = package_name(&manifest)?;
    let old = old_template(input, from, pins)
        .inspect_err(|reason| eprintln!("crucible upgrade: {reason}, so the old template is unknown"))
        .ok();
    let new = snapshot(&name);
    let to = installed_version(root, "crucible")?
        .ok_or_else(|| Error::Failed(format!("crucible is not installed in {}", root.display())))?;
    let mut written = Vec::new();
    let mut edited = Vec::new();
    let mut deleted = Vec::new();
    let mut review = Vec::new();
    for (part, &(_, _, upkeep)) in new.owned.iter().zip(OWNED) {
        let new_part = part.template().expect("owned sections exist");
        let file = root.join(&part.path);
        let current = fs::read_to_string(&file).ok();
        let range = current.as_deref().and_then(|text| part.find(text));
        let current_part = current.as_deref().zip(range.clone()).map(|(text, range)| &text[range]);
        let old_part =
            old.as_ref().and_then(|old| match old.owned.iter().find(|candidate| candidate.same_place(part)) {
                Some(old) => old.template().map(Some),
                None => Some(None),
            });
        match (reconcile(old_part, new_part, current_part, part.section.is_none()), upkeep) {
            (Change::Current, _) => {}
            (Change::Edited, _) => edited.push(part.label()),
            (Change::Deleted, _) => deleted.push(part.label()),
            (Change::Write, Upkeep::Report) => review.push(part.label()),
            (Change::Write, Upkeep::Refresh) => {
                let content = match (&current, range) {
                    (Some(text), Some(range)) => {
                        format!("{}{}{}", &text[..range.start], line_endings_like(new_part, text), &text[range.end..])
                    }
                    _ => new_part.to_owned(),
                };
                write_atomic(&file, &content)
                    .map_err(|error| Error::Failed(format!("cannot write {}: {error}", file.display())))?;
                written.push(part.label());
            }
        }
    }
    let dropped: Vec<_> = old
        .iter()
        .flat_map(|old| &old.owned)
        .filter(|part| !new.owned.iter().any(|candidate| candidate.same_place(part)))
        .filter(|part| fs::read_to_string(root.join(&part.path)).is_ok_and(|text| part.find(&text).is_some()))
        .map(PartV1::label)
        .collect();
    let topics: Vec<_> = new
        .docs
        .iter()
        .filter(|(topic, hash)| old.as_ref().is_none_or(|old| old.docs.get(*topic) != Some(hash)))
        .map(|(topic, _)| topic.as_str())
        .collect();

    let mut report = if *from == to {
        format!("Reinstalled crucible and {RUNTIME} at {to}.\n")
    } else {
        format!("Upgraded crucible and {RUNTIME} from {from} to {to}.\n")
    };
    let mut list = |heading: &str, labels: &[String]| {
        if !labels.is_empty() {
            report += &format!("\n{heading}\n");
            report.extend(labels.iter().map(|label| format!("  {label}\n")));
        }
    };
    list("Refreshed from the new template:", &written);
    list(
        if old.is_some() {
            "The new template changed these, but this project edited them, so they were left alone:"
        } else {
            "These differ from the new template, and the old template is unknown, so they were left alone:"
        },
        &edited,
    );
    list("These are deleted in this project, so they were not recreated:", &deleted);
    list("The new template changed these, and crucible upgrade never rewrites them. Review the change:", &review);
    list("The new template no longer includes these. Delete them if this game does not use them:", &dropped);
    if !edited.is_empty() || !deleted.is_empty() || !review.is_empty() {
        report +=
            &format!("Merge by hand. `npx crucible create {name} <empty-dir> --no-install` writes the new template.\n");
    }
    if !topics.is_empty() {
        report += "\nThese docs changed. Read them before you continue:\n";
        report.extend(topics.iter().map(|topic| format!("  npx crucible docs {topic}\n")));
    }
    let manager = detect_manager(root, &std::env::var("npm_config_user_agent").unwrap_or_default());
    report += &format!("\nRun `{} run check` next.\n", manager.program());
    Ok(report)
}

fn old_template(input: &str, from: &Version, pins: &Pins) -> Result<SnapshotV1, String> {
    let old = decode(input)?;
    if old.version.parse().ok().as_ref() != Some(from) {
        return Err(format!("the old template is from crucible {}, but {from} was installed", old.version));
    }
    if let Some(pin) = [&pins.crucible, &pins.runtime].into_iter().find(|pin| satisfies(pin, from) == Some(false)) {
        return Err(format!("package.json pinned {pin}, which does not match the installed {from}"));
    }
    Ok(old)
}

fn decode(input: &str) -> Result<SnapshotV1, String> {
    #[derive(Deserialize)]
    struct Header {
        format: u32,
    }
    let unreadable = |error: serde_json::Error| format!("the old template is not readable: {error}");
    match serde_json::from_str::<Header>(input).map_err(unreadable)?.format {
        1 => serde_json::from_str(input).map_err(unreadable),
        format if format > FORMAT => {
            Err(format!("the old template uses format {format}, newer than this version's {FORMAT}"))
        }
        format => Err(format!("the old template uses unknown format {format}")),
    }
}

fn snapshot(name: &str) -> SnapshotV1 {
    let tokens = tokens(name);
    let owned = OWNED
        .iter()
        .map(|&(path, heading, _)| {
            let source = FILES.iter().find(|(file, _)| *file == path).expect("owned files are template files").1;
            PartV1 { path: path.into(), section: heading.map(Into::into), content: render(source, &tokens) }
        })
        .collect();
    let docs = crate::docs::TOPICS
        .iter()
        .map(|topic| (topic.name.to_owned(), format!("{:x}", Sha256::digest(topic.markdown))))
        .collect();
    SnapshotV1 { format: FORMAT, version: env!("CARGO_PKG_VERSION").into(), owned, docs }
}

// `old` is `None` when the old template is unknown and `Some(None)` when it lacked this part.
fn reconcile(old: Option<Option<&str>>, new: &str, current: Option<&str>, whole_file: bool) -> Change {
    let same = |a: &str, b: &str| a.replace("\r\n", "\n") == b.replace("\r\n", "\n");
    match (old, current) {
        (_, Some(current)) if same(current, new) => Change::Current,
        (Some(Some(old)), _) if same(old, new) => Change::Current,
        (Some(Some(old)), Some(current)) if same(current, old) => Change::Write,
        (Some(None), None) if whole_file => Change::Write,
        (Some(None), _) => Change::Edited,
        (_, None) => Change::Deleted,
        _ => Change::Edited,
    }
}

fn line_endings_like(content: &str, existing: &str) -> String {
    let content = content.replace("\r\n", "\n");
    if existing.contains("\r\n") { content.replace('\n', "\r\n") } else { content }
}

// Trailing whitespace is excluded so the section still matches when the game appends its own sections.
fn section(text: &str, heading: &str) -> Option<Range<usize>> {
    let level = heading.bytes().take_while(|&byte| byte == b'#').count();
    let mut start = None;
    let mut fence: Option<(u8, usize)> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end();
        let indent = bare.len() - bare.trim_start_matches(' ').len();
        if indent <= 3 {
            let block = &bare[indent..];
            match (fence, fence_marker(block)) {
                (None, Some(marker)) => fence = Some(marker),
                (Some((open, length)), Some((marker, run)))
                    if marker == open && run >= length && run == block.len() =>
                {
                    fence = None;
                }
                (None, None) if start.is_none() => {
                    if block == heading {
                        start = Some(offset);
                    }
                }
                (None, None) => {
                    let hashes = block.bytes().take_while(|&byte| byte == b'#').count();
                    if (1..=level).contains(&hashes) && (block.len() == hashes || block[hashes..].starts_with(' ')) {
                        break;
                    }
                }
                _ => {}
            }
        }
        offset += line.len();
    }
    let start = start?;
    Some(start..start + text[start..offset].trim_end().len())
}

fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let marker = *line.as_bytes().first()?;
    let run = line.bytes().take_while(|&byte| byte == marker).count();
    (matches!(marker, b'`' | b'~') && run >= 3).then_some((marker, run))
}

fn read_manifest(root: &Path) -> Result<(String, serde_json::Value), Error> {
    let path = root.join("package.json");
    let text = fs::read_to_string(&path).map_err(|error| {
        Error::Failed(format!("cannot read {}: {error}. Run crucible upgrade from the game's root", path.display()))
    })?;
    let manifest = serde_json::from_str(&text)
        .map_err(|error| Error::Failed(format!("{} is not valid JSON: {error}", path.display())))?;
    Ok((text, manifest))
}

fn package_name(manifest: &serde_json::Value) -> Result<String, Error> {
    manifest["name"].as_str().map(Into::into).ok_or_else(|| Error::Failed("package.json has no name".into()))
}

fn read_pins(manifest: &serde_json::Value) -> Result<Pins, Error> {
    let mut found = [None, None];
    for group in ["dependencies", "devDependencies"] {
        for (slot, package) in found.iter_mut().zip(["crucible", RUNTIME]) {
            let Some(spec) = manifest[group].get(package) else { continue };
            let spec = spec.as_str().filter(|spec| registry_spec(spec)).ok_or_else(|| {
                Error::Usage(format!(
                    "package.json sets {package} to {spec}, which is not a registry version or range. crucible upgrade only rewrites those, so change it by hand"
                ))
            })?;
            slot.get_or_insert_with(|| spec.to_owned());
        }
    }
    let [crucible, runtime] = found;
    let missing = |package: &str| {
        Error::Failed(format!("package.json does not list {package} in dependencies or devDependencies"))
    };
    Ok(Pins {
        crucible: crucible.ok_or_else(|| missing("crucible"))?,
        runtime: runtime.ok_or_else(|| missing(RUNTIME))?,
    })
}

// npm reads a spec as a version range unless a protocol, URL, path, Git shorthand, or tag name says otherwise.
fn registry_spec(spec: &str) -> bool {
    let spec = spec.trim();
    let mut chars = spec.chars();
    let (first, second) = (chars.next(), chars.next());
    !spec.contains([':', '/', '\\', '#'])
        && match first {
            None => true,
            Some(first) if first.is_ascii_digit() || "^~<>=*".contains(first) => true,
            Some('v' | 'V') => second.is_some_and(|second| second.is_ascii_digit()),
            Some('x' | 'X') => second.is_none_or(|second| " .|".contains(second)),
            _ => false,
        }
}

// None when the semver crate would read the range differently from npm, such as `1.2`, `a || b`, or `a - b`.
fn satisfies(spec: &str, version: &Version) -> Option<bool> {
    let spec = spec.trim();
    if let Ok(exact) = spec.parse::<Version>() {
        return Some(exact == *version);
    }
    let comparable = spec.starts_with(['^', '~', '<', '>', '=', '*']) && !spec.contains(['|', ',', ' ']);
    VersionReq::parse(spec).ok().filter(|_| comparable).map(|range| range.matches(version))
}

// Edits the version strings in place, because reserializing would reformat the file and fail the game's format check.
fn bump(text: &str, version: &str) -> Result<String, Error> {
    let mut expected: serde_json::Value = serde_json::from_str(text).expect("package.json was parsed before");
    let mut result = text.to_owned();
    for group in ["dependencies", "devDependencies"] {
        for package in ["crucible", RUNTIME] {
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
        }
    }
    if serde_json::from_str::<serde_json::Value>(&result).ok().as_ref() != Some(&expected) {
        return Err(Error::Failed(format!(
            "cannot update package.json automatically. Set crucible and {RUNTIME} to {version} and run install"
        )));
    }
    Ok(result)
}

fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    let (temporary, target) = stage(path, content)?;
    replace(&temporary, &target).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

// A failed restore keeps its temporary file, because that file then holds the only copy of the original.
fn restore(path: &Path, original: &str) -> Result<(), String> {
    let (temporary, target) = stage(path, original).map_err(|error| error.to_string())?;
    replace(&temporary, &target).map_err(|error| {
        format!("{error}. {} holds the original; move it over {}", temporary.display(), target.display())
    })
}

fn stage(path: &Path, content: &str) -> io::Result<(PathBuf, PathBuf)> {
    let linked = fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink());
    let target = if linked { fs::canonicalize(path)? } else { path.to_owned() };
    let mut temporary = target.as_os_str().to_owned();
    temporary.push(".crucible-upgrade");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, content).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })?;
    Ok((temporary, target))
}

// On Windows an editor, indexer, or antivirus scanner holding the file open fails the rename for a moment.
fn replace(temporary: &Path, target: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match fs::rename(temporary, target) {
            Err(error) if cfg!(windows) && busy(&error) && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            result => return result,
        }
    }
}

fn busy(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ResourceBusy)
        || matches!(error.raw_os_error(), Some(32 | 33))
}

// A pnpm, Yarn, or npm workspace keeps one lockfile at its root, above the game.
fn detect_manager(root: &Path, agent: &str) -> Manager {
    let locks = [
        ("pnpm-lock.yaml", Manager::Pnpm),
        ("yarn.lock", Manager::Yarn),
        ("bun.lock", Manager::Bun),
        ("bun.lockb", Manager::Bun),
        ("package-lock.json", Manager::Npm),
    ];
    for dir in root.ancestors() {
        if let Some(&(lock, manager)) = locks.iter().find(|(lock, _)| dir.join(lock).is_file()) {
            let berry = || fs::read_to_string(dir.join(lock)).is_ok_and(|text| text.contains("__metadata:"));
            return if manager == Manager::Yarn && berry() { Manager::YarnBerry } else { manager };
        }
    }
    match package_manager(agent) {
        "pnpm" => Manager::Pnpm,
        "yarn" => {
            let major = agent.strip_prefix("yarn/").and_then(|rest| rest.split('.').next()?.parse::<u32>().ok());
            if major.is_some_and(|major| major >= 2) { Manager::YarnBerry } else { Manager::Yarn }
        }
        "bun" => Manager::Bun,
        _ => Manager::Npm,
    }
}

fn installed_version(root: &Path, package: &str) -> Result<Option<Version>, Error> {
    let Some(directory) = node_package(root, package) else { return Ok(None) };
    let manifest: serde_json::Value = fs::read(directory.join("package.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    manifest["version"]
        .as_str()
        .and_then(|version| version.parse().ok())
        .map(Some)
        .ok_or_else(|| Error::Failed(format!("{} has no valid version", directory.join("package.json").display())))
}

fn shim(root: &Path) -> Option<PathBuf> {
    let package = node_package(root, "crucible")?;
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(package.join("package.json")).ok()?).ok()?;
    let bin = manifest["bin"]["crucible"].as_str().or(manifest["bin"].as_str())?;
    Some(package.join(bin))
}

fn latest(root: &Path) -> Result<Version, Error> {
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

fn run_install(root: &Path, manager: Manager) -> Result<(), String> {
    let line = manager.install_line();
    let status = package_manager_command(manager.program())
        .args(manager.install_args())
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(std::io::stderr())
        .status()
        .map_err(|error| format!("cannot run `{line}`: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("`{line}` failed")) }
}

struct Finished {
    status: ExitStatus,
    stdout: Vec<u8>,
}

fn finish(command: Command, input: Option<String>, label: &str) -> Result<Finished, Error> {
    finish_within(command, input, label, CHILD_TIMEOUT)
}

fn finish_within(
    mut command: Command,
    input: Option<String>,
    label: &str,
    timeout: Duration,
) -> Result<Finished, Error> {
    command.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() });
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|error| Error::Failed(format!("cannot run {label}: {error}")))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        thread::spawn(move || stdin.write_all(input.as_bytes()));
    }
    let reader = child.stdout.take().map(|mut stdout| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        })
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                kill_tree(&mut child);
                return Err(Error::Failed(format!("{label} did not finish within {} seconds", timeout.as_secs())));
            }
            Err(error) => return Err(Error::Failed(format!("cannot wait for {label}: {error}"))),
        }
    };
    let stdout = reader.map(|reader| reader.join().unwrap_or_default()).unwrap_or_default();
    Ok(Finished { status, stdout })
}

// The shim runs the native binary as a grandchild, so killing only the shim would leave it running.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins(crucible: &str, runtime: &str) -> Pins {
        Pins { crucible: crucible.into(), runtime: runtime.into() }
    }

    fn version(text: &str) -> Version {
        text.parse().unwrap()
    }

    #[test]
    fn reconcile_refreshes_only_untouched_parts() {
        use Change::*;
        let cases = [
            (Some(Some("a")), "b", Some("a"), true, Write),
            (Some(Some("a")), "b", Some("a\r\n"), true, Edited),
            (Some(Some("a\n")), "b\n", Some("a\r\n"), true, Write),
            (Some(Some("a")), "b", Some("edited"), true, Edited),
            (Some(Some("a")), "b", None, true, Deleted),
            (Some(Some("a")), "a", None, true, Current),
            (Some(Some("a")), "a", Some("edited"), true, Current),
            (Some(Some("a")), "b", Some("b"), true, Current),
            (Some(None), "b", None, true, Write),
            (Some(None), "b", None, false, Edited),
            (Some(None), "b", Some("mine"), true, Edited),
            (None, "b", Some("a"), true, Edited),
            (None, "b", None, true, Deleted),
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
    fn tilde_and_indented_fences_hide_headings() {
        let text = "## Crucible\n\n~~~sh\n## not a heading\n```\n# still code\n~~~~\n\n   ````\n# nor this\n   ```\n````\n\nend\n\n  ## Mine\n";
        let range = section(text, "## Crucible").unwrap();
        assert!(text[range.clone()].ends_with("end"), "{:?}", &text[range]);
        let empty = "## Crucible\nbody\n##\nafter\n";
        assert_eq!(&empty[section(empty, "## Crucible").unwrap()], "## Crucible\nbody");
    }

    #[test]
    fn bump_edits_only_the_two_versions() {
        let text = "{\n\t\"name\": \"crucible-game\",\n\t\"scripts\": { \"crucible\": \"^0.1.0\" },\n\t\"dependencies\": {\n\t\t\"@trioforge/crucible\": \"0.1.0\"\n\t},\n\t\"devDependencies\": {\n\t\t\"crucible\" :  \"^0.1.0\"\n\t}\n}\n";
        let bumped = bump(text, "0.2.0");
        assert!(bumped.is_err(), "an ambiguous script entry is not rewritten silently");
        let text = text.replace("{ \"crucible\": \"^0.1.0\" }", "{}");
        let bumped = bump(&text, "0.2.0").unwrap();
        assert_eq!(bumped, text.replace("\"0.1.0\"", "\"0.2.0\"").replace("\"^0.1.0\"", "\"0.2.0\""));
        let crlf = text.replace('\n', "\r\n");
        assert_eq!(bump(&crlf, "0.2.0").unwrap(), bumped.replace('\n', "\r\n"));
    }

    #[test]
    fn pins_refuse_non_registry_specs() {
        for spec in [
            "workspace:*",
            "file:../crucible",
            "link:../crucible",
            "npm:other@0.1.0",
            "github:a/b",
            "a/b",
            "git+https://example.com/a.git",
            "https://example.com/a.tgz",
            "../crucible",
            "latest",
            "next",
        ] {
            let manifest =
                serde_json::json!({ "dependencies": { RUNTIME: "0.1.0" }, "devDependencies": { "crucible": spec } });
            assert!(
                matches!(read_pins(&manifest), Err(Error::Usage(message)) if message.contains(spec)),
                "{spec} is refused with a usage error naming it"
            );
        }
        let manifest =
            serde_json::json!({ "dependencies": { RUNTIME: "~0.1.0" }, "devDependencies": { "crucible": "^0.1.0" } });
        let pins = read_pins(&manifest).unwrap();
        assert_eq!((pins.crucible.as_str(), pins.runtime.as_str()), ("^0.1.0", "~0.1.0"));
        assert!(matches!(
            read_pins(&serde_json::json!({ "devDependencies": { "crucible": "0.1.0" } })),
            Err(Error::Failed(_))
        ));
    }

    #[test]
    fn plan_skips_only_a_fully_installed_target() {
        let at = |crucible: &str, runtime: Option<&str>| Installed {
            crucible: version(crucible),
            runtime: runtime.map(version),
        };
        let b = version("0.2.0");
        let plan_for =
            |pins: &Pins, installed: &Installed, requested, allow| plan(pins, installed, &b, requested, allow).unwrap();
        assert_eq!(plan_for(&pins("0.2.0", "0.2.0"), &at("0.2.0", Some("0.2.0")), true, false), Plan::Current);
        assert_eq!(
            plan_for(&pins("0.3.0", "0.3.0"), &at("0.2.0", Some("0.2.0")), true, false),
            Plan::Install,
            "pins left at another version"
        );
        assert_eq!(
            plan_for(&pins("^0.2.0", "0.2.0"), &at("0.2.0", Some("0.2.0")), true, false),
            Plan::Install,
            "a range is pinned exactly"
        );
        assert_eq!(
            plan_for(&pins("0.2.0", "0.2.0"), &at("0.2.0", Some("0.1.0")), true, false),
            Plan::Install,
            "runtime not installed at the target"
        );
        assert_eq!(plan_for(&pins("0.2.0", "0.2.0"), &at("0.2.0", None), true, false), Plan::Install);
        assert_eq!(plan_for(&pins("0.1.0", "0.1.0"), &at("0.1.0", Some("0.1.0")), false, false), Plan::Install);
        assert_eq!(
            plan_for(&pins("0.3.0-rc.1", "0.3.0-rc.1"), &at("0.3.0-rc.1", Some("0.3.0-rc.1")), false, false),
            Plan::NewerThanLatest(version("0.3.0-rc.1"))
        );
        assert!(matches!(
            plan(&pins("0.3.0", "0.3.0"), &at("0.3.0", Some("0.3.0")), &b, true, false),
            Err(Error::Usage(_))
        ));
        assert_eq!(plan_for(&pins("0.3.0", "0.3.0"), &at("0.3.0", Some("0.3.0")), true, true), Plan::Install);
    }

    #[test]
    fn pins_match_the_installed_version_like_npm() {
        assert_eq!(satisfies("0.1.0", &version("0.1.0")), Some(true));
        assert_eq!(satisfies("0.1.0", &version("0.1.1")), Some(false), "a bare version is exact in npm");
        assert_eq!(satisfies("^0.1.0", &version("0.1.4")), Some(true));
        assert_eq!(satisfies("~0.1.0", &version("0.2.0")), Some(false));
        for range in ["1.2", "^0.1.0 || ^0.2.0", ">=0.1.0 <0.2.0", "0.1.0 - 0.2.0", "x", "v0.1.0"] {
            assert_eq!(satisfies(range, &version("0.1.0")), None, "{range} is left to npm");
        }
    }

    #[test]
    fn the_old_template_is_known_only_for_a_known_format_from_the_installed_version() {
        let current = serde_json::to_string(&snapshot("game")).unwrap();
        let from = version(env!("CARGO_PKG_VERSION"));
        let exact = pins(env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_VERSION"));
        assert!(old_template(&current, &from, &exact).is_ok());
        assert!(
            old_template(&current, &version("0.0.1"), &pins("0.0.1", "0.0.1"))
                .is_err_and(|reason| reason.contains("but 0.0.1 was installed"))
        );
        assert!(
            old_template(&current, &from, &pins("0.0.1", env!("CARGO_PKG_VERSION")))
                .is_err_and(|reason| reason.contains("package.json pinned 0.0.1"))
        );
        assert!(
            old_template(&current, &from, &pins("^0.0.1 || >=0.0.1", "*")).is_ok(),
            "npm-only ranges trust node_modules"
        );
        let mut future: serde_json::Value = serde_json::from_str(&current).unwrap();
        future["format"] = (FORMAT + 1).into();
        future["owned"] = "a later shape".into();
        assert!(
            old_template(&future.to_string(), &from, &exact)
                .is_err_and(|reason| reason.contains("newer than this version"))
        );
        assert!(old_template("", &from, &exact).is_err());
    }

    #[test]
    fn the_hand_off_reads_the_binary_version() {
        assert_eq!(reported_version("crucible 0.2.0 (rojo 7.7.1)\n"), Some("0.2.0"));
        assert_eq!(reported_version("something else"), None);
        use clap::CommandFactory;
        let real = crate::cli::Cli::command().render_version();
        assert_eq!(reported_version(&real), Some(env!("CARGO_PKG_VERSION")), "{real:?} keeps the hand-off format");
    }

    #[test]
    fn crlf_files_keep_crlf() {
        assert_eq!(line_endings_like("a\nb\n", "x\r\ny\r\n"), "a\r\nb\r\n");
        assert_eq!(line_endings_like("a\r\nb\n", "x\ny\n"), "a\nb\n");
    }

    #[test]
    fn apply_preserves_crlf_and_reports_deleted_and_dropped_parts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut old = snapshot("game");
        let new = snapshot("game");
        let agents = new.owned.iter().find(|part| part.path == "AGENTS.md").unwrap().template().unwrap();
        let old_agents = old.owned.iter_mut().find(|part| part.path == "AGENTS.md").unwrap();
        old_agents.content = format!("{}\n\nAn old line.\n", old_agents.content.trim_end());
        let old_section = old_agents.template().unwrap().to_owned();
        old.owned.iter_mut().find(|part| part.path == "LICENSE").unwrap().content = "old license".into();
        old.owned.push(PartV1 { path: "old.json".into(), section: None, content: "{}".into() });
        fs::write(root.join("package.json"), "{\"name\":\"game\"}").unwrap();
        fs::create_dir_all(root.join("node_modules/crucible")).unwrap();
        fs::write(root.join("node_modules/crucible/package.json"), "{\"version\":\"9.0.0\"}").unwrap();
        fs::write(
            root.join("AGENTS.md"),
            format!("# Game\r\n\r\n{}\r\n\r\n## Mine\r\n", old_section.replace('\n', "\r\n")),
        )
        .unwrap();
        fs::write(root.join("old.json"), "{}").unwrap();
        for part in new.owned.iter().filter(|part| part.section.is_none() && part.path != "LICENSE") {
            fs::write(root.join(&part.path), &part.content).unwrap();
        }
        let from = version(env!("CARGO_PKG_VERSION"));
        let exact = pins(env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_VERSION"));
        let report = apply(root, &from, &exact, &serde_json::to_string(&old).unwrap()).unwrap();
        assert!(
            report.starts_with(&format!(
                "Upgraded crucible and {RUNTIME} from {} to 9.0.0.\n",
                env!("CARGO_PKG_VERSION")
            ))
        );
        assert!(report.contains("Refreshed from the new template:\n  AGENTS.md (## Crucible)\n"), "{report}");
        assert!(report.contains("deleted in this project, so they were not recreated:\n  LICENSE\n"), "{report}");
        assert!(
            report.contains("no longer includes these. Delete them if this game does not use them:\n  old.json\n"),
            "{report}"
        );
        assert!(!root.join("LICENSE").exists());
        assert_eq!(
            fs::read_to_string(root.join("AGENTS.md")).unwrap(),
            format!("# Game\r\n\r\n{}\r\n\r\n## Mine\r\n", agents.replace('\n', "\r\n"))
        );
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
        let game = dir.path().join("games").join("one");
        fs::create_dir_all(&game).unwrap();
        assert_eq!(detect_manager(&game, "pnpm/12.15.0 npm/? node/v24"), Manager::Pnpm);
        assert_eq!(detect_manager(&game, "yarn/1.22.22 npm/? node/v24"), Manager::Yarn);
        assert_eq!(detect_manager(&game, "yarn/4.5.0 npm/? node/v24"), Manager::YarnBerry);
        for (lock, manager) in
            [("package-lock.json", Manager::Npm), ("bun.lockb", Manager::Bun), ("yarn.lock", Manager::Yarn)]
        {
            fs::write(game.join(lock), "# yarn lockfile v1\n").unwrap();
            assert_eq!(detect_manager(&game, ""), manager);
        }
        fs::write(game.join("yarn.lock"), "__metadata:\n  version: 8\n").unwrap();
        assert_eq!(detect_manager(&game, ""), Manager::YarnBerry);
        fs::write(game.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_manager(&game, ""), Manager::Pnpm);
        let member = dir.path().join("games").join("two");
        fs::create_dir_all(&member).unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_manager(&member, "npm/11.0.0"), Manager::Pnpm, "a workspace lockfile above the game wins");
    }

    #[test]
    fn ci_safe_install_commands() {
        assert_eq!(Manager::Pnpm.install_line(), "pnpm install --no-frozen-lockfile");
        assert_eq!(Manager::YarnBerry.install_line(), "yarn install --no-immutable");
        assert_eq!(Manager::Yarn.install_line(), "yarn install");
        assert_eq!(Manager::Npm.install_line(), "npm install");
    }

    #[test]
    fn npm_ranges_are_registry_specs() {
        for spec in ["^0.1.0 || ^0.2.0", ">=0.1.0 <0.2.0", "0.1.0 - 0.2.0", "1.2", "*", "", "x", "v0.1.0"] {
            let manifest =
                serde_json::json!({ "dependencies": { RUNTIME: spec }, "devDependencies": { "crucible": spec } });
            assert!(read_pins(&manifest).is_ok(), "{spec:?} is a registry range");
        }
        assert_ne!(satisfies("1.2", &version("1.3.0")), Some(true), "npm reads 1.2 as 1.2.x");
    }

    #[test]
    fn a_newer_runtime_is_newer_than_latest() {
        let installed = Installed { crucible: version("0.2.0"), runtime: Some(version("0.3.0-rc.1")) };
        assert_eq!(
            plan(&pins("0.3.0-rc.1", "0.3.0-rc.1"), &installed, &version("0.2.0"), false, false).unwrap(),
            Plan::NewerThanLatest(version("0.3.0-rc.1"))
        );
        assert!(matches!(
            plan(&pins("0.3.0-rc.1", "0.3.0-rc.1"), &installed, &version("0.2.0"), true, false),
            Err(Error::Usage(_))
        ));
    }

    #[test]
    fn the_new_binary_cuts_the_old_section_and_never_rewrites_the_license() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let new = snapshot("game");
        let agents = render(FILES.iter().find(|(path, _)| *path == "AGENTS.md").unwrap().1, &tokens("game"));
        let old_agents = format!("{}\n\nAn old line.\n", agents.trim_end());
        let license = &new.owned.iter().find(|part| part.path == "LICENSE").unwrap().content;
        let old_license = format!("{license}An older license.\n");
        let mut old = serde_json::to_value(&new).unwrap();
        for part in old["owned"].as_array_mut().unwrap() {
            match part["path"].as_str().unwrap() {
                "AGENTS.md" => part["content"] = old_agents.clone().into(),
                "LICENSE" => part["content"] = old_license.clone().into(),
                _ => {}
            }
        }
        fs::write(root.join("package.json"), "{\"name\":\"game\"}").unwrap();
        fs::create_dir_all(root.join("node_modules/crucible")).unwrap();
        fs::write(root.join("node_modules/crucible/package.json"), "{\"version\":\"9.0.0\"}").unwrap();
        fs::write(root.join("AGENTS.md"), &old_agents).unwrap();
        fs::write(root.join("LICENSE"), &old_license).unwrap();
        let exact = pins(env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_VERSION"));
        let report = apply(root, &version(env!("CARGO_PKG_VERSION")), &exact, &old.to_string()).unwrap();
        assert!(report.contains("Refreshed from the new template:\n  AGENTS.md (## Crucible)\n"), "{report}");
        assert_eq!(fs::read_to_string(root.join("AGENTS.md")).unwrap(), agents);
        assert!(report.contains("never rewrites them. Review the change:\n  LICENSE\n"), "{report}");
        assert_eq!(fs::read_to_string(root.join("LICENSE")).unwrap(), old_license, "the license is never rewritten");
    }

    #[test]
    fn write_atomic_cleans_up_and_writes_through_links() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("blocked");
        fs::create_dir(&blocked).unwrap();
        fs::write(blocked.join("inside"), "x").unwrap();
        assert!(write_atomic(&blocked, "text").is_err());
        assert!(!dir.path().join("blocked.crucible-upgrade").exists(), "a failed write leaves no temporary file");

        let target = dir.path().join("target.json");
        let link = dir.path().join("link.json");
        fs::write(&target, "old").unwrap();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&target, &link);
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&target, &link);
        if linked.is_ok() {
            write_atomic(&link, "new").unwrap();
            assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link survives");
            assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        }
    }

    #[cfg(windows)]
    #[test]
    fn write_atomic_waits_out_a_sharing_violation() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("package.json");
        fs::write(&path, "old").unwrap();
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        write_atomic(&path, "new").unwrap();
        release.join().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn a_hung_child_times_out_with_its_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = if cfg!(windows) { Command::new("cmd") } else { Command::new("sh") };
        if cfg!(windows) {
            command.args(["/d", "/c", "cmd", "/d", "/c", "ping -n 3 127.0.0.1 >nul & echo late>late.txt"]);
        } else {
            command.args(["-c", "sh -c 'sleep 2; echo late > late.txt'; true"]);
        }
        command.current_dir(dir.path());
        let started = Instant::now();
        let result = finish_within(command, None, "sleeper", Duration::from_millis(300));
        assert!(matches!(result, Err(Error::Failed(message)) if message.contains("did not finish")));
        assert!(started.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_secs(4));
        assert!(!dir.path().join("late.txt").exists(), "the grandchild was killed with the child");
    }

    fn installed(root: &Path, crucible: &str, runtime: &str, bin: &str) {
        fs::create_dir_all(root.join("node_modules/crucible")).unwrap();
        fs::create_dir_all(root.join("node_modules/@trioforge/crucible")).unwrap();
        let manifest = serde_json::json!({ "version": crucible, "bin": { "crucible": "bin.js" } });
        fs::write(root.join("node_modules/crucible/package.json"), manifest.to_string()).unwrap();
        fs::write(root.join("node_modules/crucible/bin.js"), bin).unwrap();
        let runtime = serde_json::json!({ "version": runtime });
        fs::write(root.join("node_modules/@trioforge/crucible/package.json"), runtime.to_string()).unwrap();
    }

    #[test]
    fn a_failed_hand_off_says_the_template_was_not_refreshed() {
        let cases = [
            ("0.1.0", "", "@trioforge/crucible at 0.1.0, not 0.2.0. Delete node_modules"),
            ("0.2.0", "console.log('crucible 0.1.0 (rojo 7.7.1)')", "binary reports 0.1.0, not 0.2.0"),
            (
                "0.2.0",
                "if (process.argv[2] === '--version') console.log('crucible 0.2.0'); else process.exit(3);",
                "crucible __upgrade-apply failed",
            ),
        ];
        for (runtime, bin, cause) in cases {
            let dir = tempfile::tempdir().unwrap();
            installed(dir.path(), "0.2.0", runtime, bin);
            let (from, to, old_pins) = (version("0.1.0"), version("0.2.0"), pins("0.1.0", "0.1.0"));
            let result = refresh(dir.path(), "game", &to, &from, &old_pins, "{}".into(), "npm install");
            let Err(Error::Failed(message)) = result else { panic!("{cause}: the hand-off fails") };
            assert!(message.contains(cause), "{message}");
            assert!(message.contains("but the template was not refreshed"), "{message}");
            assert!(
                message.contains("`npx crucible create game <empty-dir> --no-install` and diff tsconfig.json,"),
                "{message}"
            );
            assert!(message.contains("and the ## Crucible section of AGENTS.md with this game's"), "{message}");
        }
    }

    #[test]
    fn the_hand_off_passes_the_old_version_pins_and_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let bin = "const fs = require('fs');
if (process.argv[2] === '--version') console.log('crucible 0.2.0 (rojo 7.7.1)');
else fs.writeFileSync('seen.json', JSON.stringify({ argv: process.argv.slice(2), stdin: fs.readFileSync(0, 'utf8') }));";
        installed(dir.path(), "0.2.0", "0.2.0", bin);
        let old_pins = pins("^0.1.0 || ^0.2.0", ">=0.1.0 <0.2.0");
        let snapshot = "{\"format\":1}";
        refresh(dir.path(), "game", &version("0.2.0"), &version("0.1.0"), &old_pins, snapshot.into(), "npm install")
            .unwrap();
        let seen: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("seen.json")).unwrap()).unwrap();
        assert_eq!(seen["argv"], serde_json::json!([APPLY, "0.1.0", "^0.1.0 || ^0.2.0", ">=0.1.0 <0.2.0"]));
        assert_eq!(seen["stdin"], snapshot);
    }
}
