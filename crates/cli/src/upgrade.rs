use crate::build::Error;
use crate::create::{FILES, package_manager, package_manager_command, render, tokens};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
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

// The hand-off contract in crates/cli/README.md says when to raise this.
const FORMAT: u32 = 1;
const CHILD_TIMEOUT: Duration = Duration::from_secs(120);
const CLI: &str = "@trioforge/crucible-cli";
const RUNTIME: &str = "@trioforge/crucible";
const COMPILER_TYPES: &str = "@trioforge/compiler-types";
const ANVIL: &str = "@trioforge/anvil";
const RELEASED_WITH_CLI: [&str; 3] = [CLI, RUNTIME, COMPILER_TYPES];
const GROUPS: [&str; 2] = ["dependencies", "devDependencies"];
const ROBLOX_TS_TOOLCHAIN: [&str; 3] = ["roblox-ts", "typescript", "@rbxts/compiler-types"];
const RBXTSC_HEADER: &[u8] = b"-- Compiled with roblox-ts";

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
pub(crate) enum Manager {
    Npm,
    Pnpm,
    Yarn,
    YarnBerry,
    Bun,
}

impl Manager {
    pub(crate) fn program(self) -> &'static str {
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

    // A failed pnpm or Yarn Berry install can leave the manager's own record of node_modules describing packages it
    // never placed, and then a plain install reports nothing to do. Each command here restored such a node_modules.
    fn recovery(self) -> &'static str {
        match self {
            Self::Npm => "run `npm install`",
            Self::Pnpm => "run `pnpm install --force --no-frozen-lockfile`",
            Self::Yarn => "run `yarn install`",
            Self::YarnBerry => "delete node_modules/.yarn-state.yml, then run `yarn install --no-immutable`",
            Self::Bun => "run `bun install`",
        }
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
        .and_then(Report::print);
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
        crucible: installed_version(root, CLI)?.ok_or_else(|| {
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
        Plan::Current if on_roblox_ts(&manifest) => {
            let mut report = format!("crucible is already at {target}.\n");
            reconcile_dependencies(root, manager)?.report(&mut report);
            let migration = move_off_roblox_ts(root, &target, manager);
            migration.report(&mut report);
            return Report::new(report, migration, manager).print();
        }
        Plan::Current => {
            let mut report = format!("crucible is already at {target}.\n");
            let dependencies = reconcile_dependencies(root, manager)?;
            dependencies.report(&mut report);
            if !dependencies.updated.is_empty() {
                report += &format!("\nRun `{} run check` next.\n", manager.program());
            }
            print!("{report}");
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
    let bumped = peer_manifest_with(root, &bumped, target_peers(root, &target)?)?;
    write_atomic(&manifest_path, &bumped)
        .map_err(|error| Error::Failed(format!("cannot write {}: {error}", manifest_path.display())))?;
    let recovery = manager.recovery();
    eprintln!(
        "crucible upgrade: running `{install}` for crucible {target}. If it is interrupted, restore package.json and {recovery}"
    );
    if let Err(error) = run_install(root, manager) {
        let pinned = format!("{CLI} {} and {RUNTIME} {}", pins.crucible, pins.runtime);
        return Err(Error::Failed(match restore(&manifest_path, &manifest_text) {
            Ok(()) => format!(
                "{error}. package.json is restored to {pinned}. To return node_modules to those versions, {recovery}"
            ),
            Err(restore) => {
                format!("{error}. Restoring package.json also failed: {restore}. Set {pinned}, then {recovery}")
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
            "{error}. package.json and node_modules are at {target}, but the template was not refreshed, and running crucible upgrade again reports {CLI} as already at {target}. To compare by hand, run `npx crucible create {name} <empty-dir> --no-install` and diff {} with this game's",
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
    let now = [installed_version(root, CLI)?, installed_version(root, RUNTIME)?];
    if now.iter().any(|version| version.as_ref() != Some(target)) {
        let [crucible, runtime] = now.map(|version| version.map_or("nothing".into(), |version| version.to_string()));
        return Err(Error::Failed(format!(
            "after `{install}`, {CLI} is at {crucible} and {RUNTIME} at {runtime}, not {target}. Delete node_modules and run `{install}`"
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
    let line = stdout.lines().next()?.strip_prefix("crucible ")?;
    let version = line.split_once(' ').map_or(line, |(version, _)| version);
    Version::parse(version).ok()?;
    Some(version)
}

fn apply(root: &Path, from: &Version, pins: &Pins, input: &str) -> Result<Report, Error> {
    let (_, manifest) = read_manifest(root)?;
    let name = package_name(&manifest)?;
    let old = old_template(input, from, pins)
        .inspect_err(|reason| eprintln!("crucible upgrade: {reason}, so the old template is unknown"))
        .ok();
    let from = &old.as_ref().and_then(|old| old.version.parse().ok()).unwrap_or_else(|| from.clone());
    let new = snapshot(&name);
    let to = installed_version(root, CLI)?
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
    let manager = detect_manager(root, &std::env::var("npm_config_user_agent").unwrap_or_default());
    let dependencies =
        if manifest["dependencies"].get(RUNTIME).is_some() || manifest["devDependencies"].get(RUNTIME).is_some() {
            reconcile_dependencies(root, manager)
        } else {
            Ok(Dependencies::default())
        };
    let (dependencies, migration) = match dependencies {
        Ok(dependencies) => (dependencies, move_off_roblox_ts(root, &to, manager)),
        Err(error) => (Dependencies::default(), Migration { failed: Some(error.to_string()), ..Migration::default() }),
    };

    let mut report = if *from == to {
        format!("Reinstalled crucible and {RUNTIME} at {to}.\n")
    } else {
        format!("Upgraded crucible and {RUNTIME} from {from} to {to}.\n")
    };
    dependencies.report(&mut report);
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
    migration.report(&mut report);
    if !topics.is_empty() {
        report += "\nThese docs changed. Read them before you continue:\n";
        report.extend(topics.iter().map(|topic| format!("  npx crucible docs {topic}\n")));
    }
    Ok(Report::new(report, migration, manager))
}

struct Report {
    text: String,
    failed: Option<String>,
}

impl Report {
    fn new(mut text: String, migration: Migration, manager: Manager) -> Self {
        if migration.failed.is_none() {
            text += &format!("\nRun `{} run check` next.\n", manager.program());
        }
        Self { text, failed: migration.failed }
    }

    fn print(self) -> Result<(), Error> {
        print!("{}", self.text);
        self.failed.map_or(Ok(()), |failed| Err(Error::Failed(failed)))
    }
}

// An install that failed on Windows can leave the new shim package beside the old binary, so `from` names the new
// version while the snapshot comes from the old binary. Pins that hold the snapshot's version still vouch for it.
fn old_template(input: &str, from: &Version, pins: &Pins) -> Result<SnapshotV1, String> {
    let old = decode(input)?;
    let pins = [&pins.crucible, &pins.runtime];
    let pinned = |version: &Version| pins.iter().all(|pin| satisfies(pin, version) == Some(true));
    let Some(version) = old.version.parse::<Version>().ok().filter(|version| version == from || pinned(version)) else {
        return Err(format!("the old template is from crucible {}, but {from} was installed", old.version));
    };
    if let Some(pin) = pins.into_iter().find(|pin| satisfies(pin, &version) == Some(false)) {
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
    let docs = crucible_docs::TOPICS
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
    let mut found = [None, None, None];
    for group in ["dependencies", "devDependencies"] {
        for (slot, package) in found.iter_mut().zip(RELEASED_WITH_CLI) {
            let Some(spec) = manifest[group].get(package) else { continue };
            let spec = spec.as_str().filter(|spec| registry_spec(spec)).ok_or_else(|| {
                Error::Usage(format!(
                    "package.json sets {package} to {spec}, which is not a registry version or range. crucible upgrade only rewrites those, so change it by hand"
                ))
            })?;
            slot.get_or_insert_with(|| spec.to_owned());
        }
    }
    let [crucible, runtime, _] = found;
    let missing = |package: &str| {
        Error::Failed(format!("package.json does not list {package} in dependencies or devDependencies"))
    };
    Ok(Pins { crucible: crucible.ok_or_else(|| missing(CLI))?, runtime: runtime.ok_or_else(|| missing(RUNTIME))? })
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
    for group in GROUPS {
        for package in RELEASED_WITH_CLI {
            let Some(spec) = expected.get_mut(group).and_then(|packages| packages.get_mut(package)) else { continue };
            let Some(old) = spec.as_str() else { continue };
            for entry in entries(&result, package, old).into_iter().rev() {
                result.replace_range(entry.value, &quoted(version));
            }
            *spec = version.into();
        }
    }
    if serde_json::from_str::<serde_json::Value>(&result).ok().as_ref() != Some(&expected) {
        return Err(Error::Failed(format!(
            "cannot update package.json automatically. Set {CLI}, {RUNTIME}, and {COMPILER_TYPES} if the game lists it, to {version} and run install"
        )));
    }
    Ok(result)
}

fn peer_satisfies(old: &str, required: &str) -> bool {
    if old == required {
        return true;
    }
    let parse = |spec: &str| {
        let normalized = if spec.parse::<Version>().is_ok() { format!("={spec}") } else { spec.to_owned() };
        VersionReq::parse(&normalized)
    };
    let Ok(wanted) = parse(required) else { return false };
    old.split("||").all(|arm| {
        let normalized = arm.split_whitespace().collect::<Vec<_>>().join(", ");
        let Ok(existing) = parse(&normalized) else { return false };
        let Some((lower, upper)) = peer_bounds(&existing) else { return false };
        let Some((wanted_lower, wanted_upper)) = peer_bounds(&wanted) else { return false };
        let above = lower.0 > wanted_lower.0 || (lower.0 == wanted_lower.0 && (!lower.1 || wanted_lower.1));
        let below = match (upper, wanted_upper) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(end), Some(wanted)) => end.0 < wanted.0 || (end.0 == wanted.0 && (!end.1 || wanted.1)),
        };
        above && below
    })
}

type PeerBound = (Version, bool);

fn peer_bounds(requirement: &VersionReq) -> Option<(PeerBound, Option<PeerBound>)> {
    use semver::Op;
    let mut lower = (Version::new(0, 0, 0), true);
    let mut upper: Option<PeerBound> = None;
    for comparator in &requirement.comparators {
        if !comparator.pre.is_empty() {
            return None;
        }
        let minor = comparator.minor.unwrap_or(0);
        let patch = comparator.patch.unwrap_or(0);
        let version = Version::new(comparator.major, minor, patch);
        let next_major = || comparator.major.checked_add(1).map(|major| Version::new(major, 0, 0));
        let next_minor = || minor.checked_add(1).map(|minor| Version::new(comparator.major, minor, 0));
        let next_patch = || patch.checked_add(1).map(|patch| Version::new(comparator.major, minor, patch));
        let (start, end) = match comparator.op {
            Op::GreaterEq => (Some((version, true)), None),
            Op::Greater if comparator.patch.is_some() => (Some((version, false)), None),
            Op::Less => (None, Some((version, false))),
            Op::LessEq if comparator.patch.is_some() => (None, Some((version, true))),
            Op::Exact | Op::Wildcard if comparator.minor.is_none() => {
                (Some((version, true)), Some((next_major()?, false)))
            }
            Op::Exact | Op::Wildcard if comparator.patch.is_none() => {
                (Some((version, true)), Some((next_minor()?, false)))
            }
            Op::Exact => (Some((version.clone(), true)), Some((version, true))),
            Op::Tilde => (
                Some((version, true)),
                Some((if comparator.minor.is_some() { next_minor()? } else { next_major()? }, false)),
            ),
            Op::Caret => {
                let end = if comparator.major > 0 || comparator.minor.is_none() {
                    next_major()?
                } else if minor > 0 || comparator.patch.is_none() {
                    next_minor()?
                } else {
                    next_patch()?
                };
                (Some((version, true)), Some((end, false)))
            }
            _ => return None,
        };
        if let Some(start) = start
            && (start.0 > lower.0 || (start.0 == lower.0 && !start.1))
        {
            lower = start;
        }
        if let Some(end) = end
            && upper.as_ref().is_none_or(|current| end.0 < current.0 || (end.0 == current.0 && !end.1))
        {
            upper = Some(end);
        }
    }
    Some((lower, upper))
}

fn peer_manifest(root: &Path, text: &str) -> Result<String, Error> {
    peer_manifest_with(root, text, crate::create::peer_versions())
}

fn peer_manifest_with(root: &Path, text: &str, versions: Vec<(&str, String)>) -> Result<String, Error> {
    let mut manifest: serde_json::Value = serde_json::from_str(text).expect("manifest was parsed");
    let listed = |package: &str| GROUPS.iter().any(|group| manifest[group].get(package).is_some());
    let ui = listed("@rbxts/vide") || listed("@rbxts/vide-charm") || uses_ui(root);
    let mut result = text.to_owned();
    for (package, required) in versions {
        if !ui && matches!(package, "@rbxts/vide" | "@rbxts/vide-charm") {
            continue;
        }
        let mut found = false;
        for group in GROUPS {
            let Some(old) = manifest[group][package].as_str() else { continue };
            found = true;
            let compatible = peer_satisfies(old, &required);
            if compatible {
                continue;
            }
            for entry in entries(&result, package, old).into_iter().rev() {
                result.replace_range(entry.value, &quoted(&required));
            }
            manifest[group][package] = required.clone().into();
        }
        if !found {
            let group = GROUPS
                .into_iter()
                .find(|group| manifest[group].get(RUNTIME).is_some())
                .ok_or_else(|| Error::Failed("package.json must list the runtime".into()))?;
            result = insert_sorted(&result, group, package, &required)
                .ok_or_else(|| Error::Failed("cannot edit runtime peers in package.json".into()))?;
            manifest[group][package] = required.into();
        }
    }
    if serde_json::from_str::<serde_json::Value>(&result).ok().as_ref() != Some(&manifest) {
        return Err(Error::Failed("cannot edit runtime peers in package.json automatically".into()));
    }
    Ok(result)
}

// Package managers write each dependency group sorted by name, so a new entry goes where the next install would put
// it: before the first entry that sorts after it, with that entry's indentation and line ending.
fn insert_sorted(text: &str, group: &str, package: &str, version: &str) -> Option<String> {
    let parsed = jsonc_parser::parse_to_ast(text, &Default::default(), &Default::default()).ok()?;
    let root = parsed.value?;
    let entries = &root.as_object()?.get(group)?.value.as_object()?.properties;
    let separator = |start: usize| &text[text[..start].trim_end().len()..start];
    let entry = format!("{}: {}", quoted(package), quoted(version));
    let mut result = text.to_owned();
    match entries.iter().find(|existing| existing.name.as_str() > package) {
        Some(next) => result.insert_str(next.range.start, &format!("{entry},{}", separator(next.range.start))),
        None => {
            let last = entries.last()?;
            result.insert_str(last.range.end, &format!(",{}{entry}", separator(last.range.start)));
        }
    }
    Some(result)
}

fn imports_ui(source: &str) -> bool {
    let entry = format!("{}/{}", crucible_abi::PACKAGE, crucible_abi::UI);
    imports(source, |specifier| specifier == entry)
}

fn imports_anvil(source: &str) -> bool {
    imports(source, |specifier| {
        specifier.strip_prefix(ANVIL).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

fn imports(source: &str, matches: impl Fn(&str) -> bool) -> bool {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut importing = false;
    let mut exporting = false;
    while at < bytes.len() {
        if bytes[at..].starts_with(b"//") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
        } else if bytes[at..].starts_with(b"/*") {
            at += 2;
            while at < bytes.len() && !bytes[at..].starts_with(b"*/") {
                at += 1;
            }
            at = (at + 2).min(bytes.len());
        } else if matches!(bytes[at], b'\'' | b'"' | b'`') {
            let quote = bytes[at];
            at += 1;
            let start = at;
            while at < bytes.len() && bytes[at] != quote {
                if bytes[at] == b'\\' {
                    at += 1;
                }
                at += 1;
            }
            if importing && source.get(start..at).is_some_and(&matches) {
                return true;
            }
            at = (at + 1).min(bytes.len());
            importing = false;
        } else if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            let start = at;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || matches!(bytes[at], b'_' | b'$')) {
                at += 1;
            }
            match &source[start..at] {
                "import" | "require" => importing = true,
                "export" => exporting = true,
                "from" if exporting => importing = true,
                "const" | "let" | "var" | "function" | "class" | "interface" => exporting = false,
                _ => {}
            }
        } else {
            if bytes[at] == b';' {
                importing = false;
                exporting = false;
            }
            at += 1;
        }
    }
    false
}

fn uses_ui(root: &Path) -> bool {
    any_source(root, &["ts", "tsx"], imports_ui)
}

fn any_source(root: &Path, extensions: &[&str], imports: fn(&str) -> bool) -> bool {
    let Ok(entries) = fs::read_dir(root) else { return false };
    entries.flatten().any(|entry| {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || matches!(name.as_ref(), "node_modules" | "out" | "build" | "target") {
            return false;
        }
        if path.is_dir() && !entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
            return any_source(&path, extensions, imports);
        }
        path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| extensions.contains(&ext))
            && fs::read_to_string(path).is_ok_and(|source| imports(&source))
    })
}

#[derive(Default)]
struct Dependencies {
    updated: Vec<String>,
    left: Option<String>,
}

impl Dependencies {
    fn report(&self, report: &mut String) {
        if !self.updated.is_empty() {
            *report += "\nUpdated dependencies:\n";
            report.extend(self.updated.iter().map(|line| format!("  {line}\n")));
        }
        if let Some(left) = &self.left {
            *report += &format!("\n{left}\n");
        }
    }
}

fn reconcile_dependencies(root: &Path, manager: Manager) -> Result<Dependencies, Error> {
    let (text, _) = read_manifest(root)?;
    let peers = peer_manifest(root, &text)?;
    let (edited, anvil) = without_anvil(root, &peers);
    let mut dependencies = Dependencies::default();
    if peers != text {
        dependencies.updated.push("package.json now lists the runtime's required peers".into());
    }
    match anvil {
        Anvil::Absent => {}
        Anvil::Removed => {
            dependencies
                .updated
                .push(format!("package.json no longer lists {ANVIL}, whose exports moved to {RUNTIME}/ui"));
        }
        Anvil::Imported => {
            dependencies.left = Some(format!(
                "This game imports {ANVIL}, whose exports moved to {RUNTIME}/ui. Move those imports, then remove {ANVIL} from package.json. `npx crucible docs ui` explains the move."
            ));
        }
        Anvil::Unedited => {
            dependencies.left = Some(format!(
                "Remove {ANVIL} from package.json, because its exports moved to {RUNTIME}/ui. crucible upgrade cannot edit it in place. Then run `{}`.",
                manager.install_line()
            ));
        }
    }
    if edited == text {
        return Ok(dependencies);
    }
    let path = root.join("package.json");
    write_atomic(&path, &edited).map_err(|error| Error::Failed(format!("cannot write {}: {error}", path.display())))?;
    eprintln!("crucible upgrade: running `{}` for the dependencies it updated", manager.install_line());
    if let Err(error) = run_install(root, manager) {
        let restored = match restore(&path, &text) {
            Ok(()) => "package.json is restored".to_owned(),
            Err(error) => format!("restoring package.json also failed: {error}"),
        };
        return Err(Error::Failed(format!(
            "{error}. {restored}. To return node_modules to it, {}, then run `npx crucible upgrade` again",
            manager.recovery()
        )));
    }
    dependencies.updated.push(format!("ran `{}`", manager.install_line()));
    Ok(dependencies)
}

enum Anvil {
    Absent,
    Removed,
    Imported,
    Unedited,
}

// A game that kept the retired package would load a second copy of the primitives on another reactive core.
fn without_anvil(root: &Path, text: &str) -> (String, Anvil) {
    let manifest: serde_json::Value = serde_json::from_str(text).expect("manifest was parsed");
    if !GROUPS.iter().any(|group| manifest[group].get(ANVIL).is_some()) {
        return (text.to_owned(), Anvil::Absent);
    }
    if any_source(root, &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"], imports_anvil) {
        return (text.to_owned(), Anvil::Imported);
    }
    let mut expected = manifest;
    match remove_packages(text, &mut expected, &[ANVIL])
        .filter(|edited| serde_json::from_str::<serde_json::Value>(edited).ok().as_ref() == Some(&expected))
    {
        Some(edited) => (edited, Anvil::Removed),
        None => (text.to_owned(), Anvil::Unedited),
    }
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("strings serialize")
}

struct Entry {
    start: usize,
    value: Range<usize>,
}

fn entries(text: &str, key: &str, value: &str) -> Vec<Entry> {
    let (key, value) = (quoted(key), quoted(value));
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(start) = text[from..].find(&key).map(|index| from + index) {
        let after = &text[start + key.len()..];
        let tail = after.trim_start().strip_prefix(':').map(str::trim_start).filter(|tail| tail.starts_with(&value));
        from = match tail {
            Some(tail) => {
                let end = text.len() - tail.len() + value.len();
                found.push(Entry { start, value: end - value.len()..end });
                end
            }
            None => start + key.len(),
        };
    }
    found
}

#[derive(Default)]
struct Migration {
    done: Vec<String>,
    left: Vec<String>,
    failed: Option<String>,
}

impl Migration {
    fn report(&self, report: &mut String) {
        for (heading, lines) in [
            ("Moved this game from roblox-ts to Crucible's compiler:", &self.done),
            (
                "Finish the move by hand. `npx crucible docs cli` explains each step under Upgrading from roblox-ts:",
                &self.left,
            ),
        ] {
            if !lines.is_empty() {
                *report += &format!("\n{heading}\n");
                report.extend(lines.iter().map(|line| format!("  {line}\n")));
            }
        }
    }
}

fn on_roblox_ts(manifest: &serde_json::Value) -> bool {
    ["roblox-ts", "@rbxts/compiler-types"]
        .iter()
        .any(|package| GROUPS.iter().any(|group| manifest[group].get(package).is_some()))
}

fn move_off_roblox_ts(root: &Path, to: &Version, manager: Manager) -> Migration {
    let mut migration = Migration::default();
    let roblox_ts = move_toolchain(root, to, manager, &mut migration);
    if migration.failed.is_some() {
        return migration;
    }
    let places = crucible_project::Manifest::read(root).ok().flatten().unwrap_or_default().places;
    let mut tsconfigs: Vec<_> = places.into_values().map(|place| place.tsconfig).collect();
    tsconfigs.sort();
    tsconfigs.dedup();
    for tsconfig in &tsconfigs {
        clear_rbxtsc_output(root, tsconfig, &mut migration);
    }
    for tsconfig in tsconfigs.iter().filter(|tsconfig| root.join(tsconfig).is_file()) {
        for line in typescript_7_changes(root, tsconfig, roblox_ts) {
            if !migration.left.contains(&line) {
                migration.left.push(line);
            }
        }
    }
    migration
}

fn move_toolchain(root: &Path, to: &Version, manager: Manager, migration: &mut Migration) -> bool {
    let Ok((text, manifest)) = read_manifest(root) else { return false };
    if !on_roblox_ts(&manifest) {
        return false;
    }
    let listed = |package: &str| GROUPS.iter().any(|group| manifest[group].get(package).is_some());
    let dropped: Vec<_> = ROBLOX_TS_TOOLCHAIN.into_iter().filter(|package| listed(package)).collect();
    let added = (!listed(COMPILER_TYPES)).then(|| to.to_string());
    let install = manager.install_line();
    let by_hand = |reason: String| {
        let add = added.as_ref().map(|version| format!("add \"{COMPILER_TYPES}\": \"{version}\" beside {CLI}, "));
        format!(
            "package.json: {reason}, so change it by hand: {}remove {}, then run `{install}`",
            add.unwrap_or_default(),
            dropped.join(", ")
        )
    };
    let Some(edited) = drop_toolchain(&text, &manifest, &dropped, added.as_deref()) else {
        migration.left.push(by_hand("crucible upgrade cannot edit it in place".into()));
        return true;
    };
    let manifest_path = root.join("package.json");
    if let Err(error) = write_atomic(&manifest_path, &edited) {
        migration.left.push(by_hand(format!("crucible upgrade cannot write it: {error}")));
        return true;
    }
    if let Err(error) = run_install(root, manager) {
        let then = "then run `npx crucible upgrade` again to finish the move off roblox-ts";
        let recovery = manager.recovery();
        migration.failed = Some(match restore(&manifest_path, &text) {
            Ok(()) => format!(
                "{error}. package.json is restored, so it still lists {}. To return node_modules to it, {recovery}, {then}",
                dropped.join(", ")
            ),
            Err(restore) => format!(
                "{error}. Restoring package.json also failed: {restore}. Once package.json is restored, {recovery}, {then}"
            ),
        });
        return true;
    }
    if let Some(version) = &added {
        migration.done.push(format!("package.json lists {COMPILER_TYPES} {version}"));
    }
    migration.done.push(format!("package.json no longer lists {}", dropped.join(", ")));
    migration.done.push(format!("ran `{install}`"));
    true
}

fn drop_toolchain(text: &str, manifest: &serde_json::Value, dropped: &[&str], added: Option<&str>) -> Option<String> {
    let mut expected = manifest.clone();
    let mut result = remove_packages(text, &mut expected, dropped)?;
    if let Some(version) = added {
        let group = GROUPS.into_iter().find(|group| expected[group].get(CLI).is_some())?;
        result = insert_sorted(&result, group, COMPILER_TYPES, version)?;
        expected[group][COMPILER_TYPES] = version.into();
    }
    (serde_json::from_str::<serde_json::Value>(&result).ok()? == expected).then_some(result)
}

// Removes each package from `text` and from `expected`, which the caller compares with the edited text.
fn remove_packages(text: &str, expected: &mut serde_json::Value, packages: &[&str]) -> Option<String> {
    let mut result = text.to_owned();
    for group in GROUPS {
        for package in packages {
            let Some(spec) = expected.get_mut(group).and_then(|packages| packages.as_object_mut()?.remove(*package))
            else {
                continue;
            };
            for entry in entries(&result, package, spec.as_str()?).into_iter().rev() {
                remove_entry(&mut result, entry);
            }
        }
    }
    Some(result)
}

fn remove_entry(text: &mut String, entry: Entry) {
    let (before, end) = (text[..entry.start].trim_end().len(), entry.value.end);
    let rest = text[end..].trim_start().strip_prefix(',').map(str::len);
    match rest {
        Some(rest) => text.replace_range(before..text.len() - rest, ""),
        None if text[..before].ends_with(',') => text.replace_range(before - 1..end, ""),
        None => text.replace_range(before..end, ""),
    }
}

fn clear_rbxtsc_output(root: &Path, tsconfig: &Path, migration: &mut Migration) {
    let (Some(out), _) = crucible_compiler::out_dir(&root.join(tsconfig)) else { return };
    let shown = out.strip_prefix(root).unwrap_or(&out).to_string_lossy().replace('\\', "/");
    match remove_rbxtsc_modules(&out) {
        Ok(0) => {}
        Ok(removed) => migration.done.push(format!("deleted the {removed} modules rbxtsc compiled into {shown}")),
        Err(error) => migration.left.push(format!("{shown}: delete the modules rbxtsc compiled there ({error})")),
    }
}

fn remove_rbxtsc_modules(dir: &Path) -> io::Result<usize> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let (path, kind) = (entry.path(), entry.file_type()?);
        if kind.is_dir() {
            let inside = remove_rbxtsc_modules(&path)?;
            if inside > 0 && fs::read_dir(&path)?.next().is_none() {
                fs::remove_dir(&path)?;
            }
            removed += inside;
        } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "luau" || extension == "lua")
        {
            let mut head = Vec::with_capacity(RBXTSC_HEADER.len());
            fs::File::open(&path)?.take(RBXTSC_HEADER.len() as u64).read_to_end(&mut head)?;
            if head == RBXTSC_HEADER {
                fs::remove_file(&path)?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

type Applies = fn(Option<&serde_json::Value>, missing_counts: bool) -> bool;

const TYPESCRIPT_7: [(&str, Applies, &str); 8] = [
    (
        "moduleResolution",
        |value, _| {
            let resolution = value.and_then(serde_json::Value::as_str).map(str::to_ascii_lowercase);
            matches!(resolution.as_deref(), Some("node" | "node10" | "classic"))
        },
        r#"Set "module" to "preserve" and "moduleResolution" to "bundler"."#,
    ),
    ("baseUrl", |value, _| value.is_some(), r#"Remove "baseUrl", and add a "paths" entry for each source root."#),
    ("downlevelIteration", |value, _| value.is_some(), r#"Remove "downlevelIteration"."#),
    ("typeRoots", |value, _| value.is_some(), r#"Remove "typeRoots"."#),
    (
        "types",
        |value, missing_counts| match value.and_then(serde_json::Value::as_array) {
            Some(types) => !types.iter().any(|name| name == "@trioforge/compiler-types"),
            None => missing_counts,
        },
        r#"Set "types" to ["@trioforge/compiler-types", "@rbxts/types"], and name any other type package in full."#,
    ),
    (
        "rootDir",
        |value, missing_counts| missing_counts && value.is_none(),
        r#"Set "rootDir" to the source folder, such as "src"."#,
    ),
    (
        "skipLibCheck",
        |value, missing_counts| value.map_or(missing_counts, |value| *value != true),
        r#"Set "skipLibCheck" to true."#,
    ),
    ("plugins", |value, _| value.is_some(), r#"Remove "plugins". The compiler applies Crucible's transform itself."#),
];

#[derive(Default)]
struct Chain {
    settings: BTreeMap<String, (serde_json::Value, PathBuf)>,
    unread: Vec<(PathBuf, String)>,
}

// TypeScript merges compilerOptions key by key, so each setting comes from the nearest tsconfig in the chain that sets it.
fn read_chain(file: &Path, chain: &mut Chain, stack: &mut Vec<PathBuf>) {
    let parsed = fs::read_to_string(file)
        .ok()
        .and_then(|text| jsonc_parser::parse_to_serde_value(&text, &Default::default()).ok()?);
    let Some(config) = parsed else {
        chain.unread.push((file.to_owned(), "it is not valid JSON, so check it for each TypeScript 7 change".into()));
        return;
    };
    let bases = match &config["extends"] {
        serde_json::Value::String(base) => vec![base.as_str()],
        serde_json::Value::Array(bases) => bases.iter().filter_map(serde_json::Value::as_str).collect(),
        _ => Vec::new(),
    };
    stack.push(file.to_owned());
    for base in bases {
        match extended(file.parent().unwrap_or(Path::new("")), base) {
            Some(path) if stack.contains(&path) => {}
            Some(path) => read_chain(&path, chain, stack),
            None => chain.unread.push((
                file.to_owned(),
                format!(
                    "crucible upgrade cannot find {base}, which it extends, so check that tsconfig for each TypeScript 7 change"
                ),
            )),
        }
    }
    stack.pop();
    for (key, value) in config["compilerOptions"].as_object().into_iter().flatten() {
        chain.settings.insert(key.clone(), (value.clone(), file.to_owned()));
    }
}

// TypeScript's rules: a relative or rooted path, with .json added when it names no file, or else a package in node_modules.
fn extended(dir: &Path, base: &str) -> Option<PathBuf> {
    let base = base.replace('\\', "/");
    let file = |path: &Path| {
        let path = lexical(path);
        let mut json = path.clone().into_os_string();
        json.push(".json");
        [path, PathBuf::from(json)].into_iter().find(|path| path.is_file())
    };
    if base.starts_with("./") || base.starts_with("../") || Path::new(&base).is_absolute() {
        return file(&dir.join(&base));
    }
    dir.ancestors().map(|dir| dir.join("node_modules").join(&base)).find_map(|package| {
        file(&package).or_else(|| {
            let manifest: serde_json::Value =
                fs::read(package.join("package.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())?;
            let tsconfig = lexical(&package.join(manifest["tsconfig"].as_str().unwrap_or("tsconfig.json")));
            tsconfig.is_file().then_some(tsconfig)
        })
    })
}

fn lexical(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            component => normal.push(component),
        }
    }
    normal
}

// A game that never used roblox-ts chose its own settings, and a base that cannot be read may supply a missing one, so
// a missing setting counts only in a roblox-ts game whose whole chain was read.
fn typescript_7_changes(root: &Path, tsconfig: &Path, moved_off_roblox_ts: bool) -> Vec<String> {
    let top = lexical(&root.join(tsconfig));
    let mut chain = Chain::default();
    read_chain(&top, &mut chain, &mut Vec::new());
    let shown = |file: &Path| file.strip_prefix(root).unwrap_or(file).to_string_lossy().replace('\\', "/");
    let changes = |missing_counts| -> Vec<String> {
        TYPESCRIPT_7
            .iter()
            .filter_map(|&(key, applies, change)| {
                let setting = chain.settings.get(key);
                applies(setting.map(|(value, _)| value), missing_counts)
                    .then(|| format!("{}: {change}", shown(setting.map_or(&top, |(_, file)| file))))
            })
            .collect()
    };
    let held = changes(false);
    let missing_counts = (moved_off_roblox_ts || !held.is_empty()) && chain.unread.is_empty();
    let mut lines = if missing_counts { changes(true) } else { held };
    lines.extend(chain.unread.iter().map(|(file, problem)| format!("{}: {problem}", shown(file))));
    lines
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
pub(crate) fn detect_manager(root: &Path, agent: &str) -> Manager {
    detect_manager_in(root.ancestors(), agent)
}

fn detect_manager_in<'a>(directories: impl IntoIterator<Item = &'a Path>, agent: &str) -> Manager {
    let locks = [
        ("pnpm-lock.yaml", Manager::Pnpm),
        ("yarn.lock", Manager::Yarn),
        ("bun.lock", Manager::Bun),
        ("bun.lockb", Manager::Bun),
        ("package-lock.json", Manager::Npm),
    ];
    for dir in directories {
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

fn node_package(root: &Path, name: &str) -> Option<PathBuf> {
    root.ancestors()
        .map(|dir| dir.join("node_modules").join(name))
        .find(|package| package.join("package.json").is_file())
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
    let package = node_package(root, CLI)?;
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(package.join("package.json")).ok()?).ok()?;
    let bin = manifest["bin"]["crucible"].as_str().or(manifest["bin"].as_str())?;
    Some(package.join(bin))
}

fn latest(root: &Path) -> Result<Version, Error> {
    eprintln!("crucible upgrade: finding the latest {CLI} with npm view");
    let output = package_manager_command("npm")
        .args(["view", CLI, "version"])
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

fn target_peers(root: &Path, target: &Version) -> Result<Vec<(&'static str, String)>, Error> {
    let package = format!("{RUNTIME}@{target}");
    eprintln!("crucible upgrade: finding {package}'s peers with npm view");
    let output = package_manager_command("npm")
        .args(["view", &package, "peerDependencies", "--json"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| Error::Failed(format!("cannot run npm view: {error}")))?;
    if !output.status.success() {
        return Err(Error::Failed(format!("npm view {package} peerDependencies failed; package.json is unchanged")));
    }
    let peers = if output.stdout.iter().all(u8::is_ascii_whitespace) {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&output.stdout)
            .map_err(|error| Error::Failed(format!("npm view {package} returned invalid peerDependencies: {error}")))?
    };
    if !peers.is_object() {
        return Err(Error::Failed(format!("npm view {package} returned invalid peerDependencies")));
    }
    Ok(crate::create::peer_versions_from(&peers))
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
    #[cfg(windows)]
    let (mut child, job) = crate::child::job::Job::spawn(&mut command)
        .map_err(|error| Error::Failed(format!("cannot run {label}: {error}")))?;
    #[cfg(not(windows))]
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
                #[cfg(windows)]
                let _ = job.terminate();
                kill_tree(&mut child);
                return Err(Error::Failed(format!("{label} did not finish within {} seconds", timeout.as_secs())));
            }
            Err(error) => return Err(Error::Failed(format!("cannot wait for {label}: {error}"))),
        }
    };
    #[cfg(windows)]
    drop(job);
    let stdout = reader.map(|reader| reader.join().unwrap_or_default()).unwrap_or_default();
    Ok(Finished { status, stdout })
}

// The shim runs the native binary as a grandchild, so killing only the shim would leave it running.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
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
    fn first_install_uses_the_target_runtimes_peers() {
        let dir = tempfile::tempdir().unwrap();
        let peers = serde_json::json!({"@rbxts/charm": "0.12.0", "@rbxts/charm-sync": "0.5.0"});
        let text = r#"{"dependencies":{"@trioforge/crucible":"0.6.0","@rbxts/charm":"0.10.0"}}"#;
        let updated = peer_manifest_with(dir.path(), text, crate::create::peer_versions_from(&peers)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(value["dependencies"]["@rbxts/charm"], "0.12.0");
        assert_eq!(value["dependencies"]["@rbxts/charm-sync"], "0.5.0");
        assert!(value["dependencies"].get("@rbxts/vide").is_none());
        assert_eq!(
            peer_manifest_with(dir.path(), text, crate::create::peer_versions_from(&serde_json::json!({}))).unwrap(),
            text
        );
    }

    #[test]
    fn ui_detection_ignores_comments_and_unrelated_strings() {
        for source in [
            "// import '@trioforge/crucible/ui';",
            "/* import '@trioforge/crucible/ui'; */",
            "const note = '@trioforge/crucible/ui';",
        ] {
            assert!(!imports_ui(source), "{source}");
        }
        for source in [
            "import '@trioforge/crucible/ui';",
            "import { Modal } from '@trioforge/crucible/ui';",
            "export { Modal } from '@trioforge/crucible/ui';",
            "const ui = require('@trioforge/crucible/ui');",
        ] {
            assert!(imports_ui(source), "{source}");
        }
    }

    #[test]
    fn runtime_peer_ranges_keep_only_specs_inside_the_required_range() {
        for old in ["0.6.1", "0.6.2", "^0.6.1", "^0.6.2", "~0.6.2", ">=0.6.1 <0.7.0", "^0.6.1 || ~0.6.2"] {
            assert!(peer_satisfies(old, "^0.6.1"), "{old}");
        }
        for old in ["^0.5.0", "^0.7.0", "*", ">=0.6.1", "0.6.1-beta.1"] {
            assert!(!peer_satisfies(old, "^0.6.1"), "{old}");
        }
        assert!(!peer_satisfies("^0.11.1", "0.11.1"));
        assert!(peer_satisfies("0.11.1", "0.11.1"));
    }

    #[test]
    fn upgrade_reconciles_runtime_peers_for_each_manager() {
        for lock in ["package-lock.json", "pnpm-lock.yaml", "yarn.lock"] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join(lock), "").unwrap();
            for old in [None, Some("0.3.0"), Some("0.4.1")] {
                let mut manifest = serde_json::json!({"dependencies": {RUNTIME: "0.5.0", "@rbxts/charm": "^0.11.1"}});
                if let Some(old) = old {
                    manifest["dependencies"]["@rbxts/charm-sync"] = old.into();
                }
                let text = manifest.to_string();
                let updated = peer_manifest(dir.path(), &text).unwrap();
                let value: serde_json::Value = serde_json::from_str(&updated).unwrap();
                assert_eq!(value["dependencies"]["@rbxts/charm-sync"], "0.4.1", "{lock}");
                assert_eq!(value["dependencies"]["@rbxts/charm"], "0.11.1");
                assert!(value["dependencies"].get("@rbxts/vide").is_none());
                assert!(value["dependencies"].get("@rbxts/vide-charm").is_none());
                assert_eq!(peer_manifest(dir.path(), &updated).unwrap(), updated);
            }
        }
    }

    #[test]
    fn upgrade_preserves_compatible_ui_versions_and_adds_missing_peers() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/view.tsx"), "import {} from '@trioforge/crucible/ui';").unwrap();
        let missing = r#"{"dependencies":{"@trioforge/crucible":"0.5.0"}}"#;
        let added: serde_json::Value = serde_json::from_str(&peer_manifest(dir.path(), missing).unwrap()).unwrap();
        assert_eq!(added["dependencies"]["@rbxts/vide"], "^0.6.1");
        assert_eq!(added["dependencies"]["@rbxts/vide-charm"], "0.4.1");
        let text = r#"{"dependencies":{"@trioforge/crucible":"0.5.0","@rbxts/vide":"0.6.2"}}"#;
        let updated = peer_manifest(dir.path(), text).unwrap();
        let value: serde_json::Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(value["dependencies"]["@rbxts/vide"], "0.6.2");
        assert_eq!(value["dependencies"]["@rbxts/vide-charm"], "0.4.1");
        assert_eq!(peer_manifest(dir.path(), &updated).unwrap(), updated);
    }

    #[test]
    fn bump_edits_only_the_crucible_versions() {
        let text = "{\n\t\"name\": \"crucible-game\",\n\t\"scripts\": { \"@trioforge/crucible-cli\": \"^0.1.0\" },\n\t\"dependencies\": {\n\t\t\"@trioforge/crucible\": \"0.1.0\"\n\t},\n\t\"devDependencies\": {\n\t\t\"@trioforge/crucible-cli\" :  \"^0.1.0\",\n\t\t\"@trioforge/compiler-types\": \"0.1.0\",\n\t\t\"@rbxts/types\": \"1.0.959\"\n\t}\n}\n";
        let bumped = bump(text, "0.2.0");
        assert!(
            matches!(bumped, Err(Error::Failed(message)) if message.contains(COMPILER_TYPES)),
            "an ambiguous script entry is not rewritten silently, and the message names every package to set"
        );
        let text = text.replace("{ \"@trioforge/crucible-cli\": \"^0.1.0\" }", "{}");
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
            for package in [CLI, COMPILER_TYPES] {
                let mut manifest =
                    serde_json::json!({ "dependencies": { RUNTIME: "0.1.0" }, "devDependencies": { CLI: "0.1.0" } });
                manifest["devDependencies"][package] = spec.into();
                assert!(
                    matches!(read_pins(&manifest), Err(Error::Usage(message)) if message.contains(spec) && message.contains(package)),
                    "{package} {spec} is refused with a usage error naming it"
                );
            }
        }
        let manifest =
            serde_json::json!({ "dependencies": { RUNTIME: "~0.1.0" }, "devDependencies": { CLI: "^0.1.0" } });
        let pins = read_pins(&manifest).unwrap();
        assert_eq!((pins.crucible.as_str(), pins.runtime.as_str()), ("^0.1.0", "~0.1.0"));
        assert!(matches!(
            read_pins(&serde_json::json!({ "devDependencies": { CLI: "0.1.0" } })),
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
        for invalid in ["crucible \n0.1.1", "crucible", "crucible 0.1", "\ncrucible 0.1.1"] {
            assert_eq!(reported_version(invalid), None, "{invalid:?}");
        }
        assert_eq!(reported_version("crucible 0.1.0 (rojo 7.7.1)"), Some("0.1.0"));
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
        fs::create_dir_all(root.join("node_modules/@trioforge/crucible-cli")).unwrap();
        fs::write(root.join("node_modules/@trioforge/crucible-cli/package.json"), "{\"version\":\"9.0.0\"}").unwrap();
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
        let report = apply(root, &from, &exact, &serde_json::to_string(&old).unwrap()).unwrap().text;
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
        let parent = tempfile::tempdir().unwrap();
        fs::write(parent.path().join("pnpm-lock.yaml"), "").unwrap();
        let dir = tempfile::tempdir_in(parent.path()).unwrap();
        let detect = |root: &Path, agent: &str| {
            detect_manager_in(root.ancestors().take_while(|path| path.starts_with(dir.path())), agent)
        };
        let game = dir.path().join("games").join("one");
        fs::create_dir_all(&game).unwrap();
        assert_eq!(detect(&game, "pnpm/12.15.0 npm/? node/v24"), Manager::Pnpm);
        assert_eq!(detect(&game, "yarn/1.22.22 npm/? node/v24"), Manager::Yarn);
        assert_eq!(detect(&game, "yarn/4.5.0 npm/? node/v24"), Manager::YarnBerry);
        for (lock, manager) in
            [("package-lock.json", Manager::Npm), ("bun.lockb", Manager::Bun), ("yarn.lock", Manager::Yarn)]
        {
            fs::write(game.join(lock), "# yarn lockfile v1\n").unwrap();
            assert_eq!(detect(&game, ""), manager);
        }
        fs::write(game.join("yarn.lock"), "__metadata:\n  version: 8\n").unwrap();
        assert_eq!(detect(&game, ""), Manager::YarnBerry);
        fs::write(game.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect(&game, ""), Manager::Pnpm);
        let member = dir.path().join("games").join("two");
        fs::create_dir_all(&member).unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect(&member, "npm/11.0.0"), Manager::Pnpm, "a workspace lockfile above the game wins");
    }

    #[test]
    fn ci_safe_install_commands() {
        assert_eq!(Manager::Pnpm.install_line(), "pnpm install --no-frozen-lockfile");
        assert_eq!(Manager::YarnBerry.install_line(), "yarn install --no-immutable");
        assert_eq!(Manager::Yarn.install_line(), "yarn install");
        assert_eq!(Manager::Npm.install_line(), "npm install");
    }

    #[test]
    fn each_manager_names_a_recovery_that_reinstalls_what_its_record_already_lists() {
        assert_eq!(Manager::Npm.recovery(), "run `npm install`");
        assert_eq!(Manager::Pnpm.recovery(), "run `pnpm install --force --no-frozen-lockfile`");
        assert_eq!(Manager::Yarn.recovery(), "run `yarn install`");
        assert_eq!(
            Manager::YarnBerry.recovery(),
            "delete node_modules/.yarn-state.yml, then run `yarn install --no-immutable`"
        );
        assert_eq!(Manager::Bun.recovery(), "run `bun install`");
    }

    #[test]
    fn new_entries_go_where_a_package_manager_sorts_them() {
        let text = "{\n\t\"dependencies\": {\n\t\t\"@rbxts/vide\": \"^0.6.1\",\n\t\t\"@trioforge/crucible\": \"0.5.0\"\n\t},\n\t\"devDependencies\": {\n\t\t\"@trioforge/crucible-cli\": \"0.5.0\"\n\t}\n}\n";
        let insert = |text: &str, group, package| insert_sorted(text, group, package, "1.0.0").unwrap();
        assert_eq!(
            insert(text, "dependencies", "@rbxts/charm"),
            text.replace("{\n\t\t\"@rbxts/vide\"", "{\n\t\t\"@rbxts/charm\": \"1.0.0\",\n\t\t\"@rbxts/vide\"")
        );
        assert_eq!(
            insert(text, "dependencies", "@rbxts/vide-charm"),
            text.replace("\"^0.6.1\",", "\"^0.6.1\",\n\t\t\"@rbxts/vide-charm\": \"1.0.0\",")
        );
        assert_eq!(
            insert(text, "dependencies", "zod"),
            text.replace("\"0.5.0\"\n\t},", "\"0.5.0\",\n\t\t\"zod\": \"1.0.0\"\n\t},")
        );
        assert_eq!(
            insert(text, "devDependencies", "@trioforge/compiler-types"),
            text.replace(
                "{\n\t\t\"@trioforge/crucible-cli\"",
                "{\n\t\t\"@trioforge/compiler-types\": \"1.0.0\",\n\t\t\"@trioforge/crucible-cli\""
            )
        );
        let spaces = text.replace('\t', "  ").replace('\n', "\r\n");
        assert_eq!(
            insert(&spaces, "dependencies", "@rbxts/charm"),
            insert(text, "dependencies", "@rbxts/charm").replace('\t', "  ").replace('\n', "\r\n")
        );
        assert_eq!(insert_sorted("{\"dependencies\":{}}", "dependencies", "a", "1.0.0"), None);
    }

    #[test]
    fn added_peers_keep_the_group_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let text = "{\n  \"dependencies\": {\n    \"@rbxts/services\": \"1.5.5\",\n    \"@trioforge/crucible\": \"0.5.0\"\n  }\n}\n";
        let peers = serde_json::json!({"@rbxts/charm": "0.11.1", "@rbxts/charm-sync": "0.4.1"});
        let updated = peer_manifest_with(dir.path(), text, crate::create::peer_versions_from(&peers)).unwrap();
        assert_eq!(
            updated,
            "{\n  \"dependencies\": {\n    \"@rbxts/charm\": \"0.11.1\",\n    \"@rbxts/charm-sync\": \"0.4.1\",\n    \"@rbxts/services\": \"1.5.5\",\n    \"@trioforge/crucible\": \"0.5.0\"\n  }\n}\n"
        );
    }

    #[test]
    fn anvil_imports_count_only_in_code() {
        for source in [
            "// import { Dialog } from '@trioforge/anvil';",
            "const name = '@trioforge/anvil';",
            "import { Dialog } from '@trioforge/anvilish';",
            "import { Dialog } from '@trioforge/crucible/ui';",
        ] {
            assert!(!imports_anvil(source), "{source}");
        }
        for source in [
            "import { Dialog } from '@trioforge/anvil';",
            "import type { AnvilAdapter } from \"@trioforge/anvil\";",
            "export * from '@trioforge/anvil/out/dialog';",
            "const anvil = require('@trioforge/anvil');",
        ] {
            assert!(imports_anvil(source), "{source}");
        }
    }

    #[test]
    fn anvil_is_removed_only_when_no_source_imports_it() {
        let dir = tempfile::tempdir().unwrap();
        let text = "{\r\n\t\"dependencies\": {\r\n\t\t\"@trioforge/anvil\": \"0.3.0\",\r\n\t\t\"@trioforge/crucible\": \"0.5.0\"\r\n\t}\r\n}\r\n";
        let (edited, anvil) = without_anvil(dir.path(), text);
        assert!(matches!(anvil, Anvil::Removed));
        assert_eq!(edited, "{\r\n\t\"dependencies\": {\r\n\t\t\"@trioforge/crucible\": \"0.5.0\"\r\n\t}\r\n}\r\n");
        let (unchanged, anvil) = without_anvil(dir.path(), &edited);
        assert!(matches!(anvil, Anvil::Absent));
        assert_eq!(unchanged, edited);

        fs::create_dir_all(dir.path().join("src/client")).unwrap();
        fs::write(dir.path().join("src/client/menu.tsx"), "import { Dialog } from \"@trioforge/anvil\";\n").unwrap();
        let (kept, anvil) = without_anvil(dir.path(), text);
        assert!(matches!(anvil, Anvil::Imported));
        assert_eq!(kept, text);
        fs::create_dir_all(dir.path().join("node_modules/@trioforge/anvil")).unwrap();
        fs::rename(dir.path().join("src/client/menu.tsx"), dir.path().join("node_modules/@trioforge/anvil/index.ts"))
            .unwrap();
        assert!(matches!(without_anvil(dir.path(), text).1, Anvil::Removed), "node_modules is not the game's source");
    }

    #[test]
    fn the_old_binary_beside_a_newer_shim_package_still_vouches_for_its_template() {
        let current = serde_json::to_string(&snapshot("game")).unwrap();
        let here = env!("CARGO_PKG_VERSION");
        let newer = version("99.0.0");
        assert!(old_template(&current, &newer, &pins(here, here)).is_ok());
        assert!(old_template(&current, &newer, &pins(&format!("^{here}"), here)).is_ok());
        for (crucible, runtime) in [(here, "99.0.0"), ("99.0.0", here), (">=0.0.1 || 1.x", here)] {
            assert!(
                old_template(&current, &newer, &pins(crucible, runtime))
                    .is_err_and(|reason| reason.contains("but 99.0.0 was installed")),
                "{crucible} {runtime}"
            );
        }
    }

    #[test]
    fn npm_ranges_are_registry_specs() {
        for spec in ["^0.1.0 || ^0.2.0", ">=0.1.0 <0.2.0", "0.1.0 - 0.2.0", "1.2", "*", "", "x", "v0.1.0"] {
            let manifest = serde_json::json!({ "dependencies": { RUNTIME: spec }, "devDependencies": { CLI: spec } });
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
        fs::create_dir_all(root.join("node_modules/@trioforge/crucible-cli")).unwrap();
        fs::write(root.join("node_modules/@trioforge/crucible-cli/package.json"), "{\"version\":\"9.0.0\"}").unwrap();
        fs::write(root.join("AGENTS.md"), &old_agents).unwrap();
        fs::write(root.join("LICENSE"), &old_license).unwrap();
        let exact = pins(env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_VERSION"));
        let report = apply(root, &version(env!("CARGO_PKG_VERSION")), &exact, &old.to_string()).unwrap().text;
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

    #[cfg(windows)]
    #[test]
    fn an_exited_middle_process_does_not_release_its_descendants() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("grandchild.cmd"), "@ping -n 6 127.0.0.1 >nul\r\n@echo late>late.txt\r\n").unwrap();
        fs::write(dir.path().join("middle.cmd"), "@start \"\" /b cmd /d /c .\\grandchild.cmd\r\n").unwrap();
        fs::write(
            dir.path().join("parent.cmd"),
            "@cmd /d /c .\\middle.cmd\r\n@echo exited>exited.txt\r\n@ping -n 30 127.0.0.1 >nul\r\n",
        )
        .unwrap();
        let mut command = Command::new("cmd");
        command.args(["/d", "/c", ".\\parent.cmd"]).current_dir(dir.path());
        let result = finish_within(command, None, "orphan", Duration::from_secs(2));
        assert!(matches!(result, Err(Error::Failed(message)) if message.contains("did not finish")));
        assert!(dir.path().join("exited.txt").exists(), "the intermediate shell exited before timeout");
        thread::sleep(Duration::from_secs(6));
        assert!(!dir.path().join("late.txt").exists(), "the orphaned grandchild was killed");
    }

    fn installed(root: &Path, crucible: &str, runtime: &str, bin: &str) {
        fs::create_dir_all(root.join("node_modules/@trioforge/crucible-cli")).unwrap();
        fs::create_dir_all(root.join("node_modules/@trioforge/crucible")).unwrap();
        let manifest = serde_json::json!({ "version": crucible, "bin": { "crucible": "bin.js" } });
        fs::write(root.join("node_modules/@trioforge/crucible-cli/package.json"), manifest.to_string()).unwrap();
        fs::write(root.join("node_modules/@trioforge/crucible-cli/bin.js"), bin).unwrap();
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

    const ROBLOX_TS_TSCONFIG: &str = r#"{
	"compilerOptions": {
		"allowSyntheticDefaultImports": true,
		"downlevelIteration": true,
		"module": "commonjs",
		"moduleResolution": "Node",
		"noLib": true,
		"resolveJsonModule": true,
		"forceConsistentCasingInFileNames": true,
		"moduleDetection": "force",
		"strict": true,
		"target": "ESNext",
		"declaration": false,
		"incremental": true,
		"typeRoots": ["node_modules/@rbxts", "node_modules/@trioforge"],
		"types": ["compiler-types", "types"],
		"rootDir": "src",
		"outDir": "out",
		"tsBuildInfoFile": "out/tsconfig.tsbuildinfo",
		"plugins": [
			{
				"transform": "@trioforge/crucible/transformer"
			}
		]
	},
	"include": ["src"]
}
"#;

    fn template(path: &str) -> String {
        render(FILES.iter().find(|(file, _)| *file == path).unwrap().1, &tokens("game"))
    }

    #[test]
    fn the_roblox_ts_template_package_json_becomes_the_new_one() {
        let new = template("package.json");
        let version = env!("CARGO_PKG_VERSION");
        let types = format!("\n\t\t\"{COMPILER_TYPES}\": \"{version}\",");
        let old = new.replace(
            &types,
            "\n\t\t\"roblox-ts\": \"3.0.0\",\n\t\t\"typescript\": \"5.5.3\",\n\t\t\"@rbxts/compiler-types\": \"3.0.0-types.0\",",
        );
        assert_ne!(old, new);
        let toolchain = ROBLOX_TS_TOOLCHAIN.to_vec();
        let migrate = |text: &str| {
            drop_toolchain(text, &serde_json::from_str(text).unwrap(), &toolchain, Some(version))
                .expect("edits in place")
        };
        assert_eq!(migrate(&old), new);
        assert_eq!(migrate(&old.replace('\n', "\r\n")), new.replace('\n', "\r\n"));

        let compact = r#"{"devDependencies":{"@trioforge/crucible-cli":"1.0.0","roblox-ts":"3.0.0"},"dependencies":{"@rbxts/compiler-types":"3.0.0-types.0","@trioforge/crucible":"1.0.0"}}"#;
        let dropped = ["roblox-ts", "@rbxts/compiler-types"];
        let migrated =
            drop_toolchain(compact, &serde_json::from_str(compact).unwrap(), &dropped, Some("1.0.0")).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&migrated).unwrap(),
            serde_json::json!({ "devDependencies": { CLI: "1.0.0", COMPILER_TYPES: "1.0.0" }, "dependencies": { RUNTIME: "1.0.0" } }),
            "{migrated}"
        );

        let overridden =
            old.replace("\"private\": true,", "\"private\": true,\n\t\"overrides\": { \"typescript\": \"5.5.3\" },");
        let manifest = serde_json::from_str(&overridden).unwrap();
        assert_eq!(drop_toolchain(&overridden, &manifest, &toolchain, Some(version)), None, "an override stays");
    }

    #[test]
    fn each_typescript_7_change_names_the_tsconfig_in_the_extends_chain_that_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let write = |path: &str, text: &str| {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), text).unwrap();
        };
        let changes = |tsconfig: &str, moved| typescript_7_changes(root, Path::new(tsconfig), moved);
        let roblox_ts = |file: &str| -> Vec<String> {
            [
                r#"Set "module" to "preserve" and "moduleResolution" to "bundler"."#,
                r#"Remove "downlevelIteration"."#,
                r#"Remove "typeRoots"."#,
                r#"Set "types" to ["@trioforge/compiler-types", "@rbxts/types"], and name any other type package in full."#,
                r#"Set "skipLibCheck" to true."#,
                r#"Remove "plugins". The compiler applies Crucible's transform itself."#,
            ]
            .iter()
            .map(|change| format!("{}: {change}", if change.contains("skipLibCheck") { "tsconfig.json" } else { file }))
            .collect()
        };

        write(
            "tsconfig.json",
            &ROBLOX_TS_TSCONFIG.replacen(
                "{
",
                "{
	// made by crucible create 0.3.0
",
                1,
            ),
        );
        assert_eq!(changes("tsconfig.json", false), roblox_ts("tsconfig.json"));
        write("tsconfig.json", &template("tsconfig.json"));
        assert_eq!(changes("tsconfig.json", true), Vec::<String>::new());

        write("config/base.json", ROBLOX_TS_TSCONFIG);
        write("tsconfig.json", r#"{ "extends": "./config/base", "compilerOptions": { "noImplicitReturns": true, }, }"#);
        assert_eq!(
            changes("tsconfig.json", false),
            roblox_ts("config/base.json"),
            "a game that moved its settings into a base"
        );

        write("node_modules/@game/tsconfig/package.json", r#"{ "tsconfig": "roblox.json" }"#);
        write(
            "node_modules/@game/tsconfig/roblox.json",
            r#"{ "compilerOptions": { "downlevelIteration": true, "skipLibCheck": false, "rootDir": "src" } }"#,
        );
        write("local.json", r#"{ "compilerOptions": { "baseUrl": "src" } }"#);
        write(
            "tsconfig.arena.json",
            r#"{ "extends": ["@game/tsconfig", "./local.json"], "compilerOptions": { "skipLibCheck": true } }"#,
        );
        assert_eq!(
            changes("tsconfig.arena.json", true),
            [
                r#"local.json: Remove "baseUrl", and add a "paths" entry for each source root."#,
                r#"node_modules/@game/tsconfig/roblox.json: Remove "downlevelIteration"."#,
                r#"tsconfig.arena.json: Set "types" to ["@trioforge/compiler-types", "@rbxts/types"], and name any other type package in full."#,
            ],
            "a package base supplies rootDir, and the tsconfig overrides its skipLibCheck"
        );

        write(
            "tsconfig.lost.json",
            r#"{ "extends": "@game/missing", "compilerOptions": { "downlevelIteration": true } }"#,
        );
        assert_eq!(
            changes("tsconfig.lost.json", true),
            [
                r#"tsconfig.lost.json: Remove "downlevelIteration"."#,
                "tsconfig.lost.json: crucible upgrade cannot find @game/missing, which it extends, so check that tsconfig for each TypeScript 7 change",
            ],
            "a base that cannot be read may supply the missing settings"
        );
        write("broken.json", "{ not json");
        write("tsconfig.broken.json", r#"{ "extends": "./broken.json" }"#);
        assert_eq!(
            changes("tsconfig.broken.json", true),
            ["broken.json: it is not valid JSON, so check it for each TypeScript 7 change"]
        );
        write("loop-a.json", r#"{ "extends": "./loop-b" }"#);
        write("loop-b.json", r#"{ "extends": "./sub/../loop-a.json" }"#);
        assert_eq!(changes("loop-a.json", false), Vec::<String>::new(), "a cycle ends");
    }

    #[test]
    fn apply_clears_rbxtsc_output_and_lists_what_an_edited_tsconfig_needs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("package.json"), "{\"name\":\"game\"}").unwrap();
        fs::create_dir_all(root.join("node_modules/@trioforge/crucible-cli")).unwrap();
        fs::write(root.join("node_modules/@trioforge/crucible-cli/package.json"), "{\"version\":\"9.0.0\"}").unwrap();
        fs::write(root.join("tsconfig.json"), ROBLOX_TS_TSCONFIG.replace("\"strict\": true,", "\"strict\": false,"))
            .unwrap();
        let rbxtsc = "-- Compiled with roblox-ts v3.0.0\nreturn nil\n";
        for (path, text) in [
            ("out/server/main.server.luau", rbxtsc),
            ("out/shared/old/gone.lua", rbxtsc),
            ("out/client/main.client.luau", "-- Compiled with crucible-compiler\nreturn nil\n"),
            ("out/server/notes.txt", rbxtsc),
        ] {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), text).unwrap();
        }
        let exact = pins(env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_VERSION"));
        let report = apply(root, &version(env!("CARGO_PKG_VERSION")), &exact, "").unwrap().text;
        assert!(
            report.contains("Moved this game from roblox-ts to Crucible's compiler:\n  deleted the 2 modules rbxtsc compiled into out\n"),
            "{report}"
        );
        assert!(report.contains("by hand"), "{report}");
        assert!(report.contains("\n  tsconfig.json: Remove \"downlevelIteration\".\n"), "{report}");
        assert!(report.contains("\n  tsconfig.json: Remove \"plugins\"."), "{report}");
        assert!(!root.join("out/server/main.server.luau").exists() && !root.join("out/shared").exists());
        assert!(root.join("out/client/main.client.luau").is_file() && root.join("out/server/notes.txt").is_file());
        assert!(
            fs::read_to_string(root.join("tsconfig.json")).unwrap().contains("\"strict\": false"),
            "never rewritten"
        );

        let again = apply(root, &version(env!("CARGO_PKG_VERSION")), &exact, "").unwrap().text;
        assert!(!again.contains("Moved this game"), "a second run finds nothing to move: {again}");
    }
}
