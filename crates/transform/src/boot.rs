use std::path::{Path, PathBuf};

use crucible_abi::{PACKAGE, SUBPATHS};
use crucible_compiler::extension::{Kind, Node};

use crate::Code;
use crate::api::normalize;
use crate::facts::Fact;
use crate::registration::Walk;

/// The matcher of `load` in `src/internal/boot.ts`, over an instance path instead of the instance tree.
fn path_matches(rbx: &[String], segments: &[&str]) -> bool {
    match segments.split_first() {
        None => rbx.is_empty(),
        Some((&"**", rest)) => path_matches(rbx, rest) || (!rbx.is_empty() && path_matches(&rbx[1..], segments)),
        Some((segment, rest)) => {
            rbx.split_first().is_some_and(|(name, tail)| name_matches(name, segment) && path_matches(tail, rest))
        }
    }
}

fn name_matches(name: &str, pattern: &str) -> bool {
    if let Some(open) = pattern.find('{') {
        let Some(close) = pattern[open..].find('}').map(|at| open + at) else { return false };
        return pattern[open + 1..close]
            .split(',')
            .any(|part| name_matches(name, &format!("{}{part}{}", &pattern[..open], &pattern[close + 1..])));
    }
    let Some(star) = pattern.find('*') else { return name == pattern };
    let Some(rest) = name.strip_prefix(&pattern[..star]) else { return false };
    let tail = &pattern[star + 1..];
    rest.char_indices().map(|(at, _)| at).chain([rest.len()]).any(|at| name_matches(&rest[at..], tail))
}

/// The folder of a path as Node's `path.dirname` gives it, with `.` for a bare name.
fn dirname(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/",
        Some(i) => trimmed[..i].trim_end_matches('/'),
        None if path.starts_with('/') => "/",
        None => ".",
    }
}

fn skip_wrappers(mut n: Node) -> Node {
    while matches!(
        n.kind(),
        Kind::ParenthesizedExpression | Kind::AsExpression | Kind::TypeAssertionExpression | Kind::SatisfiesExpression
    ) {
        n = n.expression();
    }
    n
}

fn property_name(p: Node) -> Option<&'static str> {
    let name = p.name()?;
    match name.kind() {
        Kind::Identifier | Kind::StringLiteral | Kind::NumericLiteral | Kind::NoSubstitutionTemplateLiteral => {
            Some(name.text())
        }
        _ => None,
    }
}

fn allowed() -> String {
    let (last, rest) = SUBPATHS.split_last().expect("the package publishes subpaths");
    format!("the root, {} or {last}", rest.join(", "))
}

impl Walk<'_, '_> {
    pub(crate) fn entry_import(&mut self, n: Node, specifier: Option<Node>) {
        if let Some(specifier) = specifier
            && specifier.kind() == Kind::StringLiteral
            && let Some(entry) = specifier.text().strip_prefix(PACKAGE).and_then(|rest| rest.strip_prefix('/'))
            && !SUBPATHS.contains(&entry)
        {
            let message = format!("Import only {} Crucible entries.", allowed());
            self.report(n, Code::Rule(4), &message);
        }
    }

    pub(crate) fn boot(&mut self, call: Node) {
        let options = call.arguments().get(0).map(skip_wrappers);
        let Some(options) = options.filter(|o| o.kind() == Kind::ObjectLiteralExpression) else {
            self.report(call, Code::Rule(6), "boot requires literal load globs.");
            return;
        };
        let properties: Vec<Node> = options.properties().iter().collect();
        let load = properties.iter().rposition(|&p| property_name(p) == Some("load"));
        if load.is_none() {
            self.report(options, Code::Rule(6), "boot requires a literal load property.");
        }
        for (i, &property) in properties.iter().enumerate() {
            let computed = property.name().is_some_and(|n| n.kind() == Kind::ComputedPropertyName);
            if property.kind() == Kind::SpreadAssignment || computed {
                if load.is_none_or(|load| i > load) {
                    self.report(
                        property,
                        Code::Rule(6),
                        "A spread or computed key after load can replace its literal globs.",
                    );
                }
                continue;
            }
            if property_name(property) != Some("load") {
                continue;
            }
            let entries = (property.kind() == Kind::PropertyAssignment)
                .then(|| property.initializer())
                .flatten()
                .filter(|i| i.kind() == Kind::ArrayLiteralExpression);
            let Some(entries) = entries else {
                self.report(property, Code::Rule(6), "load must be an array of string literals.");
                continue;
            };
            for entry in entries.elements().iter() {
                if entry.kind() != Kind::StringLiteral {
                    self.report(entry, Code::Rule(6), "load entry must be a string literal.");
                    continue;
                }
                match self.load_glob(entry.text()) {
                    Ok(glob) => self.store(entry, Fact::Load(glob.into())),
                    Err(message) => self.report(entry, Code::Rule(6), message),
                }
            }
        }
        self.store(call, Fact::Boot);
    }

    /// A load glob, relative to the package root, as the instance path pattern of the modules it matches.
    fn load_glob(&mut self, glob: &str) -> Result<String, &'static str> {
        if !self.cx.is_game() {
            return Err("Cannot load the running compiler's Rojo resolver or project mapping.");
        }
        let config = self.cx.config_dir();
        let package = self.cx.package_dir(&config.join("tsconfig.json")).unwrap_or(config);
        let prefix = match glob.find(['?', '*', '{', '[']) {
            None => format!("{}/", dirname(glob)),
            Some(at) => glob[..glob[..at].rfind('/').map_or(0, |slash| slash + 1)].to_owned(),
        };
        let rest = glob.get(prefix.len()..).unwrap_or("");
        let suffix = rest.strip_suffix(".tsx").or_else(|| rest.strip_suffix(".ts")).unwrap_or(rest);
        let folder = package.join(&prefix);
        let by_folder = self.cx.out_folder(&folder).and_then(|out| self.cx.instance_path(&out));
        let mapped = match by_folder {
            Some(mapped) => mapped,
            None => match self.folder_by_files(&folder) {
                Ok(Some(mapped)) => mapped,
                Ok(None) => return Err("load prefix is outside the Rojo mapping."),
                Err(()) => return Err("load prefix maps to more than one Rojo folder."),
            },
        };
        let pattern = format!("{}/{suffix}", mapped.join("/"));
        let segments: Vec<&str> = pattern.split('/').collect();
        if !self.loads_module(&segments) {
            return Err("load glob matches no module.");
        }
        Ok(format!("crucible-path:{pattern}"))
    }

    /// Only TypeScript compiled through the transform can define a class for boot to construct, so Luau files and
    /// other Rojo mappings are not candidates.
    fn loads_module(&self, segments: &[&str]) -> bool {
        self.cx.files().into_iter().any(|file| {
            if file.is_declaration() {
                return false;
            }
            let Some(out) = self.cx.out_path(&normalize(Path::new(&file.path()))) else { return false };
            let script = out
                .file_stem()
                .and_then(|stem| Path::new(stem).extension())
                .is_some_and(|kind| kind == "server" || kind == "client");
            !script && self.cx.instance_path(&out).is_some_and(|rbx| path_matches(&rbx, segments))
        })
    }

    /// A Rojo project may map a folder's files one by one, as Rogen does before their outputs exist. Each such file
    /// names the instance of the folder that holds it. `Err` when the files disagree.
    fn folder_by_files(&self, folder: &Path) -> Result<Option<Vec<String>>, ()> {
        let folder = normalize(folder);
        let mut found: Vec<Vec<String>> = Vec::new();
        for file in self.cx.files() {
            if file.is_declaration() {
                continue;
            }
            let source: PathBuf = normalize(Path::new(&file.path()));
            let Ok(relative) = source.strip_prefix(&folder) else { continue };
            let mut parts: Vec<String> =
                relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let Some(last) = parts.pop() else { continue };
            let stem = last.strip_suffix(".tsx").or_else(|| last.strip_suffix(".ts")).unwrap_or(&last);
            let name = stem.strip_suffix(".server").or_else(|| stem.strip_suffix(".client")).unwrap_or(stem);
            if name == "index" {
                continue;
            }
            parts.push(name.to_owned());
            let Some(rbx) = self.cx.out_path(&source).and_then(|out| self.cx.instance_path(&out)) else { continue };
            if rbx.len() > parts.len() && rbx.ends_with(&parts) {
                let candidate = rbx[..rbx.len() - parts.len()].to_vec();
                if !found.contains(&candidate) {
                    found.push(candidate);
                }
            }
        }
        match found.len() {
            0 => Ok(None),
            1 => Ok(found.pop()),
            _ => Err(()),
        }
    }
}
