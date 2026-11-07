//! Rule 5: the runtime root never reaches the server, client, emit or testing entries.
//!
//! This reads the import and export statements of `packages/crucible/src` line by line. The formatter puts each
//! statement at the start of a line and ends it with a semicolon, which is all the parsing relies on. A statement that
//! imports only types is skipped, because the compiler erases it. Any other import counts, even one whose names the
//! compiler would erase as unused, so the check can over-report but never misses a value import.

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use crate::run::{normalize, slashed};

const SOURCE: &str = "packages/crucible/src";
const ROOT: &str = "packages/crucible/src/index.ts";
const PACKAGE: &str = "@trioforge/crucible";
/// The side entries the root must never load.
const FORBIDDEN: &[&str] = &["server", "client", "emit", "testing"];

/// Each import edge from a file the root reaches into a forbidden entry, as repository-relative paths.
pub fn root_reaches(repo: &Path) -> Result<Vec<(String, String)>, String> {
    let repo = &normalize(repo);
    let source = repo.join(SOURCE);
    let forbidden = |path: &Path| {
        path.strip_prefix(&source)
            .ok()
            .and_then(|relative| relative.components().next())
            .is_some_and(|first| FORBIDDEN.iter().any(|entry| first.as_os_str() == *entry))
    };
    let shown = |path: &Path| slashed(path.strip_prefix(repo).unwrap_or(path));
    let mut found = Vec::new();
    let mut seen = BTreeSet::from([repo.join(ROOT)]);
    let mut queue = VecDeque::from([repo.join(ROOT)]);
    while let Some(file) = queue.pop_front() {
        for statement in statements(&file)? {
            if statement.types_only {
                continue;
            }
            let Some(target) = resolve(&source, &file, &statement.specifier)? else { continue };
            if forbidden(&target) {
                found.push((shown(&file), shown(&target)));
            } else if seen.insert(target.clone()) {
                queue.push_back(target);
            }
        }
    }
    Ok(found)
}

/// The value names a runtime source file exports, following `export *`.
pub fn value_exports(repo: &Path, file: &Path) -> Result<BTreeSet<String>, String> {
    let text = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut names = BTreeSet::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("export ") else { continue };
        let rest = rest.strip_prefix("declare ").unwrap_or(rest);
        for keyword in ["function ", "const ", "let ", "class ", "enum "] {
            if let Some(name) = rest.strip_prefix(keyword) {
                names.insert(name.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("").to_owned());
            }
        }
    }
    for statement in statements(file)? {
        if !statement.export || statement.types_only {
            continue;
        }
        if statement.star
            && let Some(target) = resolve(&repo.join(SOURCE), file, &statement.specifier)?
        {
            names.extend(value_exports(repo, &target)?);
        }
        names.extend(statement.values);
    }
    names.remove("");
    Ok(names)
}

#[derive(Debug, PartialEq)]
struct Statement {
    export: bool,
    /// `export * from`.
    star: bool,
    specifier: String,
    types_only: bool,
    /// The value names a named `export { .. } from` re-exports, after any `as`.
    values: Vec<String>,
}

fn statements(file: &Path) -> Result<Vec<Statement>, String> {
    let text = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(parse(&text))
}

fn parse(text: &str) -> Vec<Statement> {
    let mut found = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let export = line.starts_with("export {") || line.starts_with("export *") || line.starts_with("export type {");
        if !(export || line.starts_with("import ") || line.starts_with("import{")) {
            continue;
        }
        let mut statement = line.to_owned();
        while !statement.trim_end().ends_with(';') {
            match lines.next() {
                Some(next) => {
                    statement.push(' ');
                    statement.push_str(next.trim());
                }
                None => break,
            }
        }
        if let Some(parsed) = statement_of(&statement, export) {
            found.push(parsed);
        }
    }
    found
}

fn statement_of(text: &str, export: bool) -> Option<Statement> {
    let specifier = last_quoted(text)?;
    let body = text.strip_prefix(if export { "export" } else { "import" })?.trim_start();
    let star = export && body.starts_with('*');
    let explicit_type = body.starts_with("type ") || body.starts_with("type{");
    let braces = body.find('{').zip(body.find('}')).filter(|(open, close)| open < close);
    let named: Vec<&str> = braces
        .map(|(open, close)| body[open + 1..close].split(',').map(str::trim).filter(|name| !name.is_empty()).collect())
        .unwrap_or_default();
    // A default or namespace import binds a value even next to `{ type .. }`.
    let outside = braces.map_or(body, |(open, _)| &body[..open]);
    let binds_outside = !body.starts_with('"') && !body.starts_with('\'') && {
        let outside = outside.trim().trim_end_matches(',').trim();
        !outside.is_empty() && outside != "type" && !outside.starts_with("from")
    };
    let all_named_types = !named.is_empty() && named.iter().all(|name| name.starts_with("type "));
    let side_effect = !export && (body.starts_with('"') || body.starts_with('\''));
    let types_only = !side_effect && !star && (explicit_type || (all_named_types && !binds_outside));
    let values = if export && !explicit_type {
        named
            .iter()
            .filter(|name| !name.starts_with("type "))
            .map(|name| name.rsplit(" as ").next().unwrap_or(name).trim().to_owned())
            .collect()
    } else {
        Vec::new()
    };
    Some(Statement { export, star, specifier, types_only, values })
}

fn last_quoted(text: &str) -> Option<String> {
    let end = text.rfind(['"', '\''])?;
    let quote = text[end..].chars().next()?;
    let start = text[..end].rfind(quote)?;
    Some(text[start + 1..end].to_owned())
}

/// The source file a specifier names, or `None` for another package.
fn resolve(source: &Path, importer: &Path, specifier: &str) -> Result<Option<PathBuf>, String> {
    let base = if specifier.starts_with('.') {
        importer.parent().unwrap_or(importer).join(specifier)
    } else if specifier == PACKAGE {
        source.to_path_buf()
    } else if let Some(sub) = specifier.strip_prefix(PACKAGE).and_then(|rest| rest.strip_prefix('/')) {
        source.join(sub)
    } else {
        return Ok(None);
    };
    let with = |suffix: &str| {
        let mut path = base.clone().into_os_string();
        path.push(suffix);
        PathBuf::from(path)
    };
    let file = [with(".ts"), with(".tsx"), base.join("index.ts"), base.join("index.tsx")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("{}: cannot resolve the import of {specifier}", importer.display()))?;
    Ok(Some(normalize(&file)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Statement {
        let mut found = parse(text);
        assert_eq!(found.len(), 1, "{text}");
        found.remove(0)
    }

    #[test]
    fn type_only_statements_are_told_apart_from_value_imports() {
        assert!(one("import type { A } from \"./a\";").types_only);
        assert!(one("import { type A, type B } from \"./a\";").types_only);
        assert!(one("export type { A } from \"./a\";").types_only);
        assert!(!one("import { type A, b } from \"./a\";").types_only);
        assert!(!one("import D, { type A } from \"./a\";").types_only);
        assert!(!one("import * as ns from \"./a\";").types_only);
        assert!(!one("import \"./a\";").types_only);
        assert!(!one("export * from \"./a\";").types_only);
        let multi = one("import {\n\ta,\n\ttype B,\n} from \"../server\";");
        assert_eq!((multi.specifier.as_str(), multi.types_only), ("../server", false));
    }

    #[test]
    fn named_re_exports_give_their_exported_names() {
        let statement = one("export { a, b as c, type D } from \"./x\";");
        assert_eq!(statement.values, ["a", "c"]);
    }

    #[test]
    fn a_root_that_reaches_a_side_entry_is_reported() {
        let repo = std::env::temp_dir().join(format!("xtask-arch-ts-{}", std::process::id()));
        let source = repo.join(SOURCE);
        let write = |path: &str, text: &str| {
            let path = source.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write("index.ts", "export { a } from \"./a\";\nimport type { S } from \"./server\";\n");
        write("a.ts", "import { s } from \"./server\";\nexport const a = s;\n");
        write("server/index.ts", "export const s = 1;\n");
        let found = root_reaches(&repo.join("packages/.."));
        fs::remove_dir_all(&repo).unwrap();
        assert_eq!(
            found.unwrap(),
            [("packages/crucible/src/a.ts".to_owned(), "packages/crucible/src/server/index.ts".to_owned())]
        );
    }
}
