//! The `boot` macro: its load globs must be literals, and each becomes the instance path pattern of the modules it
//! loads.

use std::path::{Component, Path, PathBuf};

use crucible_compiler::extension::{Code, ExtractCx, Node, SyntaxKind as K};

use crate::Crucible;

fn skip_wrappers(mut n: Node) -> Node {
    while matches!(
        n.kind(),
        K::ParenthesizedExpression | K::AsExpression | K::TypeAssertionExpression | K::SatisfiesExpression
    ) {
        let Some(inner) = n.expression() else { break };
        n = inner;
    }
    n
}

fn property_name(p: Node) -> Option<&'static str> {
    let name = p.name()?;
    match name.kind() {
        K::Identifier | K::StringLiteral | K::NumericLiteral | K::NoSubstitutionTemplateLiteral => Some(name.text()),
        _ => None,
    }
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

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

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

impl Crucible {
    pub(crate) fn boot(&mut self, cx: &mut ExtractCx<'_>, call: Node) {
        let options = call.arguments().first().map(|&a| skip_wrappers(a));
        let Some(options) = options.filter(|o| o.kind() == K::ObjectLiteralExpression) else {
            cx.report(call, Code::Rule(6), "boot requires literal load globs.");
            return;
        };
        let properties = options.properties();
        let load = properties.iter().rposition(|&p| property_name(p) == Some("load"));
        if load.is_none() {
            cx.report(options, Code::Rule(6), "boot requires a literal load property.");
        }
        let mut loads = Vec::new();
        for (i, &property) in properties.iter().enumerate() {
            let computed = property.name().is_some_and(|n| n.kind() == K::ComputedPropertyName);
            if property.kind() == K::SpreadAssignment || computed {
                if load.is_none_or(|load| i > load) {
                    cx.report(
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
            let entries = (property.kind() == K::PropertyAssignment)
                .then(|| property.initializer())
                .flatten()
                .filter(|i| i.kind() == K::ArrayLiteralExpression);
            let Some(entries) = entries else {
                cx.report(property, Code::Rule(6), "load must be an array of string literals.");
                continue;
            };
            for entry in entries.elements() {
                if entry.kind() != K::StringLiteral {
                    cx.report(entry, Code::Rule(6), "load entry must be a string literal.");
                    continue;
                }
                match load_glob(cx, entry.text()) {
                    Ok(glob) => loads.push((entry, glob.into())),
                    Err(message) => cx.report(entry, Code::Rule(6), message),
                }
            }
        }
        self.loads.insert(call, loads);
    }
}

/// A load glob, relative to the package root, as the instance path pattern of the modules it matches.
fn load_glob(cx: &mut ExtractCx<'_>, glob: &str) -> Result<String, &'static str> {
    let Some(package) = cx.game_package_dir() else {
        return Err("Cannot load the running compiler's Rojo resolver or project mapping.");
    };
    let prefix = match glob.find(['?', '*', '{', '[']) {
        None => format!("{}/", dirname(glob)),
        Some(at) => glob[..glob[..at].rfind('/').map_or(0, |slash| slash + 1)].to_owned(),
    };
    let rest = glob.get(prefix.len()..).unwrap_or("");
    let suffix = rest.strip_suffix(".tsx").or_else(|| rest.strip_suffix(".ts")).unwrap_or(rest);
    let folder = package.join(&prefix);
    let by_folder = cx.out_folder(&folder).and_then(|out| cx.instance_path(&out));
    let mapped = match by_folder {
        Some(mapped) => mapped,
        None => match folder_by_files(cx, &folder) {
            Ok(Some(mapped)) => mapped,
            Ok(None) => return Err("load prefix is outside the Rojo mapping."),
            Err(()) => return Err("load prefix maps to more than one Rojo folder."),
        },
    };
    let pattern = format!("{}/{suffix}", mapped.join("/"));
    let segments: Vec<&str> = pattern.split('/').collect();
    if !loads_module(cx, &segments) {
        return Err("load glob matches no module.");
    }
    Ok(format!("crucible-path:{pattern}"))
}

/// Only TypeScript compiled through the transform can define a class for boot to construct, so Luau files and other
/// Rojo mappings are not candidates.
fn loads_module(cx: &mut ExtractCx<'_>, segments: &[&str]) -> bool {
    cx.source_files().into_iter().any(|file| {
        if file.is_declaration() {
            return false;
        }
        let Some(out) = cx.out_path(&normalize(Path::new(&file.path()))) else { return false };
        let script = out
            .file_stem()
            .and_then(|stem| Path::new(stem).extension())
            .is_some_and(|kind| kind == "server" || kind == "client");
        !script && cx.instance_path(&out).is_some_and(|rbx| path_matches(&rbx, segments))
    })
}

/// A Rojo project may map a folder's files one by one, as Rogen does before their outputs exist. Each such file names
/// the instance of the folder that holds it. `Err` when the files disagree.
fn folder_by_files(cx: &mut ExtractCx<'_>, folder: &Path) -> Result<Option<Vec<String>>, ()> {
    let folder = normalize(folder);
    let mut found: Vec<Vec<String>> = Vec::new();
    for file in cx.source_files() {
        if file.is_declaration() {
            continue;
        }
        let source = normalize(Path::new(&file.path()));
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
        let Some(rbx) = cx.out_path(&source).and_then(|out| cx.instance_path(&out)) else { continue };
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
