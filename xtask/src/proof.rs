//! `cargo xtask proof [<base>]` prints the proof table a pull request body carries.
//!
//! A `Review: mechanical` commit passes only when every change in it is one of these, and the table lists everything
//! else it finds:
//!
//! - A rename that keeps at least 95% similarity.
//! - A change to a file that a CI check regenerates and compares: the golden baseline, the arch list and the Open
//!   Cloud registry.
//! - A line whose only changes are path edits. Each hunk must replace as many lines as it removes, and each new line
//!   must equal the old one byte for byte, whitespace included, outside the words a path edit swaps. Those words may
//!   sit only in string literals of the same quote on both lines, without `${`, and, in YAML and TOML files, whose
//!   values are often unquoted, anywhere on the line. A path edit swaps one word for another, both made of letters,
//!   digits and `_.-/@~+`, where one holds `/`, is `.` or `..`, or ends in a known file extension. A word with a
//!   backslash is never a path edit, so a changed escape sequence is flagged. Each line keeps its terminator, so a
//!   changed line ending or final newline is flagged too. The table lists every path edit, so a reader can skim them.
//!
//! String literals are found by their quote characters, without a full lexer, so a lone quote in a comment or a regular
//! expression can make code read as a string. An unterminated literal leaves the rest of the line as code.
//!
//! An added, deleted or copied file, a binary change, a mode change and a hunk that adds or removes lines are flagged.
//! The bias is to flag: a false flag costs the author a `Review: read` trailer, and a false pass skips a read.

use std::path::Path;

use std::collections::BTreeSet;

use crate::arch;
use crate::golden::{self, BASELINE};
use crate::run::{git, git_maybe, repo};

const USAGE: &str = "usage: cargo xtask proof [<base>]";
const SIMILARITY: u32 = 95;
/// Generated files whose CI check proves they match what their command writes.
const GENERATED: &[&str] = &[BASELINE, arch::LIST, "crates/oc-registry/spec/registry.json"];
/// The extensions that make a word read as a file name.
const EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "mjs", "cjs", "json", "jsonc", "toml", "yml", "yaml", "md", "txt", "lua", "luau", "rbxl",
    "rbxlx", "rbxm", "rbxmx", "lock", "sh", "ps1", "cmd", "html", "css", "svg", "png",
];

pub fn main(args: &[String]) -> Result<(), String> {
    let base = match args {
        [] => "main",
        [base] => base.as_str(),
        _ => return Err(USAGE.into()),
    };
    let root = repo();
    let fork = git(&root, &["merge-base", base, "HEAD"])?.trim().to_owned();
    let commits = git(&root, &["rev-list", "--reverse", &format!("{fork}..HEAD")])?;

    let golden = golden_row(&root, base, &fork)?;
    let arch = arch_row(&root, base, &fork)?;
    let mut out = String::new();
    out.push_str("| Check | Result |\n| --- | --- |\n");
    out.push_str(&format!("| Golden | {} |\n", golden.0));
    out.push_str(&format!("| Arch violations | {} |\n", arch.0));
    out.push_str("\n| Commit | Review | Evidence |\n| --- | --- | --- |\n");
    let mut path_edits = String::new();
    for commit in commits.lines() {
        let subject = git(&root, &["log", "-1", "--format=%s", commit])?.trim().to_owned();
        let review = git(&root, &["log", "-1", "--format=%(trailers:key=Review,valueonly,separator=%x2C)", commit])?
            .trim()
            .to_owned();
        let evidence = match review.as_str() {
            "mechanical" => {
                let (evidence, edits) = mechanical(&root, commit)?;
                if !edits.is_empty() {
                    path_edits.push_str(&format!("\nPath edits in `{}`:\n\n", &commit[..7]));
                    edits.iter().for_each(|edit| path_edits.push_str(&format!("- {edit}\n")));
                }
                evidence
            }
            "read" => "Read it.".to_owned(),
            "" => "No `Review:` trailer.".to_owned(),
            other => format!("Unknown kind `{other}`."),
        };
        let kind = if review.is_empty() { "none" } else { review.as_str() };
        out.push_str(&format!("| `{}` {} | {kind} | {evidence} |\n", &commit[..7], cell(&subject)));
    }
    out.push_str(&golden.1);
    out.push_str(&arch.1);
    out.push_str(&path_edits);
    print!("{out}");
    Ok(())
}

/// The golden cell and, when the output differs, a list of the differing files to print under the table.
fn golden_row(root: &Path, base: &str, fork: &str) -> Result<(String, String), String> {
    let current = golden::build(root)?;
    let (baseline, against) = match git_maybe(root, &["show", &format!("{fork}:{BASELINE}")]) {
        Some(text) => (golden::parse(&text)?, format!("the baseline at `{base}`")),
        None => {
            let path = root.join(BASELINE);
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            (golden::parse(&text)?, format!("`{BASELINE}` on this branch, because `{base}` has none"))
        }
    };
    golden::validate(&baseline).map_err(|e| format!("the golden baseline at `{base}`: {e}"))?;
    let diff = golden::compare(&baseline, &current);
    if diff.is_empty() {
        return Ok((format!("Identical. {} files match {against}.", current.len()), String::new()));
    }
    let cell = format!(
        "Differs from {against}: {} changed, {} added, {} removed.",
        diff.changed.len(),
        diff.added.len(),
        diff.removed.len()
    );
    let mut list = String::from("\nGolden differences:\n\n");
    for (label, paths) in [("changed", &diff.changed), ("added", &diff.added), ("removed", &diff.removed)] {
        for path in paths {
            list.push_str(&format!("- {label} `{path}`\n"));
        }
    }
    Ok((cell, list))
}

/// The arch cell and, when the ratchet fails, the new violations to print under the table.
fn arch_row(root: &Path, base: &str, fork: &str) -> Result<(String, String), String> {
    let head = arch::violations(root)?;
    let at_head = format!("{} at HEAD ({})", head.len(), arch::summary(&head));
    let Some(listed) = arch::base_list(root, fork)? else {
        return Ok((format!("No list at `{base}`, so the ratchet starts here. {at_head}."), String::new()));
    };
    let (new, _) = arch::difference(&listed, &head);
    if new.is_empty() {
        return Ok((format!("{} at `{base}`, {at_head}. None is new.", listed.len()), String::new()));
    }
    let cell = format!("{} at `{base}`, {at_head}. **{} new or more frequent.**", listed.len(), new.len());
    let mut list = String::from("\nNew arch violations:\n\n");
    new.iter().for_each(|line| list.push_str(&format!("- `{line}`\n")));
    Ok((cell, list))
}

/// The evidence cell for a mechanical `commit` and the path edits it found, each as `file`: `old` → `new`.
fn mechanical(root: &Path, commit: &str) -> Result<(String, Vec<String>), String> {
    let range = [format!("{commit}^"), commit.to_owned()];
    let status = git(root, &["diff", "--name-status", "-M50%", &range[0], &range[1]])?;
    let summary = git(root, &["diff", &format!("-M{SIMILARITY}%"), "--summary", &range[0], &range[1]])?;
    let mut flags = Vec::new();
    let mut renames = 0;
    for line in status.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let code = fields.first().copied().unwrap_or_default();
        let path = fields.last().copied().unwrap_or_default();
        match code.chars().next() {
            Some('R') => {
                renames += 1;
                let similarity: u32 = code[1..].parse().unwrap_or(0);
                if similarity < SIMILARITY {
                    flags.push(format!("renames `{}` at {similarity}% similarity", fields.get(1).unwrap_or(&"")));
                }
            }
            Some('M') => {}
            Some('A') => flags.push(format!("adds `{path}`")),
            Some('D') => flags.push(format!("deletes `{path}`")),
            Some('C') => flags.push(format!("copies `{path}`")),
            _ => flags.push(format!("changes the type of `{path}`")),
        }
    }
    let exact = summary.lines().filter(|line| line.contains("(100%)")).count();
    let patch = git(root, &["diff", "-U0", "-M50%", &range[0], &range[1]])?;
    let (mut edited, mut generated) = (0, 0);
    let mut edits = BTreeSet::new();
    for file in files(&patch) {
        if file.binary {
            flags.push(format!("changes binary `{}`", file.path));
            continue;
        }
        if file.mode {
            flags.push(format!("changes the mode of `{}`", file.path));
        }
        if file.hunks.is_empty() {
            continue;
        }
        if GENERATED.contains(&file.path.as_str()) {
            generated += 1;
            continue;
        }
        edited += 1;
        for hunk in &file.hunks {
            if hunk.added.len() != hunk.removed.len() {
                let (verb, count) = if hunk.added.len() > hunk.removed.len() {
                    ("gains", hunk.added.len() - hunk.removed.len())
                } else {
                    ("loses", hunk.removed.len() - hunk.added.len())
                };
                flags.push(format!("`{}` {verb} {count} lines", file.path));
                continue;
            }
            for (removed, added) in hunk.removed.iter().zip(&hunk.added) {
                match path_update(&file.path, removed, added) {
                    Some(pairs) => pairs.into_iter().for_each(|(old, new)| {
                        edits.insert(format!("`{}`: `{old}` → `{new}`", file.path));
                    }),
                    None => flags.push(format!("`{}` changes more than a path: `{}`", file.path, added.trim())),
                }
            }
        }
    }
    if flags.is_empty() {
        let listed = if edits.is_empty() { String::new() } else { format!(", {} listed below", edits.len()) };
        return Ok((
            format!(
                "Renames: {renames}, exact: {exact}. Regenerated: {generated}. Files with path-only edits: \
                 {edited}{listed}. Nothing flagged."
            ),
            edits.into_iter().collect(),
        ));
    }
    let shown: Vec<String> = flags.iter().take(10).map(|flag| cell(flag)).collect();
    let more = if flags.len() > 10 { format!(" And {} more.", flags.len() - 10) } else { String::new() };
    Ok((format!("Flagged: {}.{more}", shown.join("; ")), Vec::new()))
}

#[derive(Default)]
struct File {
    path: String,
    binary: bool,
    mode: bool,
    hunks: Vec<Hunk>,
}

/// Each line keeps its terminator, so a changed line ending or final newline is a changed line.
#[derive(Default)]
struct Hunk {
    removed: Vec<String>,
    added: Vec<String>,
    last_added: bool,
}

fn files(patch: &str) -> Vec<File> {
    let mut files: Vec<File> = Vec::new();
    for line in patch.split_terminator('\n') {
        if let Some(header) = line.strip_prefix("diff --git ") {
            let path = header.rsplit(" b/").next().unwrap_or(header).to_owned();
            files.push(File { path, ..File::default() });
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if file.hunks.is_empty() && (line.starts_with("old mode ") || line.starts_with("new mode ")) {
            file.mode = true;
        } else if line.starts_with("Binary files ") {
            file.binary = true;
        } else if line.starts_with("@@") {
            file.hunks.push(Hunk::default());
        } else if let Some(hunk) = file.hunks.last_mut() {
            if let Some(text) = line.strip_prefix('-') {
                hunk.removed.push(format!("{text}\n"));
                hunk.last_added = false;
            } else if let Some(text) = line.strip_prefix('+') {
                hunk.added.push(format!("{text}\n"));
                hunk.last_added = true;
            } else if line.starts_with('\\') {
                let side = if hunk.last_added { &mut hunk.added } else { &mut hunk.removed };
                if let Some(last) = side.last_mut() {
                    last.pop();
                }
            }
        }
    }
    files
}

#[derive(Debug, PartialEq)]
enum Span<'a> {
    Code(&'a str),
    /// A string literal's quote and the text between its quotes.
    Quoted(char, &'a str),
}

fn quotes(path: &str) -> &'static [char] {
    match extension(path) {
        "rs" => &['"'],
        "ts" | "tsx" | "js" | "mjs" | "cjs" | "md" => &['"', '\'', '`'],
        _ => &['"', '\''],
    }
}

fn extension(path: &str) -> &str {
    path.rsplit_once('.').map_or("", |(_, ext)| ext)
}

/// `line` cut into code and string literals. An unterminated literal leaves the rest of the line as code.
fn spans<'a>(line: &'a str, path: &str) -> Vec<Span<'a>> {
    let (quotes, rust) = (quotes(path), extension(path) == "rs");
    let mut found = Vec::new();
    let (mut code, mut at) = (0, 0);
    while let Some(c) = line[at..].chars().next() {
        if let Some(len) = char_literal(&line[at..]).filter(|_| rust) {
            at += len;
            continue;
        }
        if !quotes.contains(&c) {
            at += c.len_utf8();
            continue;
        }
        let Some(end) = closing(&line[at + 1..], c).map(|end| at + 1 + end) else { break };
        if code < at {
            found.push(Span::Code(&line[code..at]));
        }
        found.push(Span::Quoted(c, &line[at + 1..end]));
        at = end + 1;
        code = at;
    }
    if code < line.len() {
        found.push(Span::Code(&line[code..]));
    }
    found
}

/// The length of the Rust character literal that starts `text`, such as `'"'`, whose quote opens no string.
fn char_literal(text: &str) -> Option<usize> {
    let rest = text.strip_prefix('\'')?;
    let len = match rest.strip_prefix('\\') {
        Some(escaped) => escaped.get(1..)?.find('\'')? + 2,
        None => rest.chars().next()?.len_utf8(),
    };
    rest[len..].starts_with('\'').then_some(len + 2)
}

/// The offset of the `quote` that closes a literal whose text starts `text`, past any escaped character.
fn closing(text: &str, quote: char) -> Option<usize> {
    let mut chars = text.char_indices();
    while let Some((at, c)) = chars.next() {
        if c == '\\' {
            chars.next();
        } else if c == quote {
            return Some(at);
        }
    }
    None
}

/// The path edits that turn `removed` into `added`, or `None` when anything else changes too.
pub fn path_update(path: &str, removed: &str, added: &str) -> Option<Vec<(String, String)>> {
    let config = matches!(extension(path), "yml" | "yaml" | "toml");
    let (old, new) = (spans(removed, path), spans(added, path));
    if old.len() != new.len() {
        return None;
    }
    let mut pairs = Vec::new();
    for pair in old.iter().zip(&new) {
        let same = match pair {
            (a, b) if a == b => true,
            (Span::Code(a), Span::Code(b)) => config && only_paths_differ(a, b, &mut pairs),
            (Span::Quoted(q, a), Span::Quoted(r, b)) => {
                q == r && !a.contains("${") && !b.contains("${") && only_paths_differ(a, b, &mut pairs)
            }
            _ => false,
        };
        if !same {
            return None;
        }
    }
    Some(pairs)
}

/// Whether `old` and `new` are byte-identical once each word that differs is a path edit.
fn only_paths_differ(old: &str, new: &str, pairs: &mut Vec<(String, String)>) -> bool {
    let (old, new) = (words(old), words(new));
    old.len() == new.len()
        && old.iter().zip(&new).all(|((a, a_space), (b, b_space))| {
            if a_space != b_space {
                return false;
            }
            if a == b {
                return true;
            }
            let edit = path_word(a) && path_word(b) && (strong_path(a) || strong_path(b));
            if edit {
                pairs.push(((*a).to_owned(), (*b).to_owned()));
            }
            edit
        })
}

/// Each word of `text` with the whitespace character after it.
fn words(text: &str) -> Vec<(&str, &str)> {
    text.split_inclusive(char::is_whitespace)
        .map(|piece| piece.split_at(piece.trim_end_matches(char::is_whitespace).len()))
        .collect()
}

fn path_word(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(|c| c.is_alphanumeric() || "_.-/@~+".contains(c))
        && (word.chars().any(char::is_alphanumeric) || word == "." || word == "..")
}

fn strong_path(word: &str) -> bool {
    word.contains('/')
        || word == "."
        || word == ".."
        || word.rsplit_once('.').is_some_and(|(_, ext)| EXTENSIONS.contains(&ext))
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::{mechanical, path_update};
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(["-c", "core.autocrlf=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
    }

    #[test]
    fn mechanical_commits_are_flagged_when_they_add_logic() {
        let dir = std::env::temp_dir().join(format!("xtask-proof-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("xtask")).unwrap();
        let body: String = (0..40).map(|i| format!("let line_{i} = elapsed.as_secs_f64() * 1e3;\n")).collect();
        std::fs::write(dir.join("cli/lib.rs"), &body).unwrap();
        std::fs::write(dir.join("xtask/arch.txt"), "1 a \"x\"\n").unwrap();
        std::fs::write(dir.join("ci.yml"), "run: cargo test --manifest-path cli/Cargo.toml\n").unwrap();
        std::fs::write(dir.join("gone.rs"), "fn gone() {}\n").unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "base"]);

        std::fs::create_dir_all(dir.join("crates")).unwrap();
        std::fs::rename(dir.join("cli/lib.rs"), dir.join("crates/lib.rs")).unwrap();
        std::fs::write(dir.join("ci.yml"), "run: cargo test --manifest-path Cargo.toml\n").unwrap();
        std::fs::write(dir.join("xtask/arch.txt"), "").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "move"]);
        let clean = mechanical(&dir, "HEAD").unwrap();

        std::fs::write(dir.join("crates/lib.rs"), body.replacen("as_secs_f64() * 1e3", "as_secs_f64() / 1e3", 1))
            .unwrap();
        std::fs::write(dir.join("ci.yml"), "run: cargo test --release\n").unwrap();
        std::fs::remove_file(dir.join("gone.rs")).unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "logic"]);
        let flagged = mechanical(&dir, "HEAD").unwrap();

        std::fs::write(dir.join("crates/lib.rs"), body.replace("let line_3 = elapsed.as_secs_f64() * 1e3;\n", ""))
            .unwrap();
        git(&dir, &["commit", "-q", "-am", "removal"]);
        let removal = mechanical(&dir, "HEAD").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(
            clean,
            (
                "Renames: 1, exact: 1. Regenerated: 1. Files with path-only edits: 1, 1 listed below. Nothing flagged."
                    .to_owned(),
                vec!["`ci.yml`: `cli/Cargo.toml` → `Cargo.toml`".to_owned()]
            )
        );
        let flagged = flagged.0;
        assert!(flagged.starts_with("Flagged:"), "{flagged}");
        assert!(flagged.contains("`crates/lib.rs` changes more than a path"), "{flagged}");
        assert!(flagged.contains("`ci.yml` changes more than a path"), "{flagged}");
        assert!(flagged.contains("deletes `gone.rs`"), "{flagged}");
        assert!(removal.0.contains("`crates/lib.rs` loses 1 lines"), "{}", removal.0);
    }

    #[test]
    fn an_operator_change_in_a_renamed_file_is_flagged() {
        let dir = std::env::temp_dir().join(format!("xtask-proof-operator-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let body: String = (0..40).map(|i| format!("export const line{i} = \"./a/{i}.ts\";\n")).collect();
        std::fs::write(dir.join("src/a.ts"), format!("{body}let answer = a + ++b;\n")).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "base"]);

        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::remove_file(dir.join("src/a.ts")).unwrap();
        std::fs::write(dir.join("lib/a.ts"), format!("{body}let answer = a++ + b;\n")).unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "move"]);
        let operator = mechanical(&dir, "HEAD").unwrap().0;

        std::fs::write(dir.join("lib/a.ts"), format!("{}let answer = a++ + b;\n", body.replace("./a/", "./b/")))
            .unwrap();
        git(&dir, &["commit", "-q", "-am", "paths"]);
        let paths = mechanical(&dir, "HEAD").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(operator.starts_with("Flagged:"), "{operator}");
        assert!(operator.contains("`lib/a.ts` changes more than a path: `let answer = a++ + b;`"), "{operator}");
        assert!(paths.0.ends_with("Files with path-only edits: 1, 40 listed below. Nothing flagged."), "{}", paths.0);
    }

    #[test]
    fn an_escape_change_in_a_renamed_file_is_flagged() {
        let dir = std::env::temp_dir().join(format!("xtask-proof-escape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let body: String = (0..40).map(|i| format!("export const line{i} = {i};\n")).collect();
        std::fs::write(dir.join("src/a.ts"), format!("{body}let s = \"a\\nb\";\n")).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "base"]);

        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::remove_file(dir.join("src/a.ts")).unwrap();
        std::fs::write(dir.join("lib/a.ts"), format!("{body}let s = \"a\\tb\";\n")).unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "move"]);
        let escape = mechanical(&dir, "HEAD").unwrap().0;
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(escape.starts_with("Flagged:"), "{escape}");
        assert!(escape.contains("`lib/a.ts` changes more than a path: `let s = \"a\\tb\";`"), "{escape}");
    }

    #[test]
    fn a_line_ending_change_is_flagged() {
        let dir = std::env::temp_dir().join(format!("xtask-proof-eol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "let a = 1;\r\nlet b = \"cli/a.json\";\r\n").unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "base"]);

        std::fs::write(dir.join("a.rs"), "let a = 1;\nlet b = \"cli/a.json\";\r\n").unwrap();
        git(&dir, &["commit", "-q", "-am", "crlf"]);
        let crlf = mechanical(&dir, "HEAD").unwrap().0;

        std::fs::write(dir.join("a.rs"), "let a = 1;\nlet b = \"cli/a.json\";").unwrap();
        git(&dir, &["commit", "-q", "-am", "eof"]);
        let eof = mechanical(&dir, "HEAD").unwrap().0;

        std::fs::write(dir.join("a.rs"), "let a = 1;\nlet b = \"a.json\";").unwrap();
        git(&dir, &["commit", "-q", "-am", "path"]);
        let path = mechanical(&dir, "HEAD").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(crlf.starts_with("Flagged:"), "{crlf}");
        assert!(crlf.contains("`a.rs` changes more than a path: `let a = 1;`"), "{crlf}");
        assert!(eof.starts_with("Flagged:"), "{eof}");
        assert!(eof.contains("`a.rs` changes more than a path: `let b = \"cli/a.json\";`"), "{eof}");
        assert_eq!(
            path,
            (
                "Renames: 0, exact: 0. Regenerated: 0. Files with path-only edits: 1, 1 listed below. Nothing flagged."
                    .to_owned(),
                vec!["`a.rs`: `cli/a.json` → `a.json`".to_owned()]
            )
        );
    }

    #[test]
    fn path_updates_pass_and_new_logic_does_not() {
        let pass = |path: &str, removed: &str, added: &str| path_update(path, removed, added).is_some();
        assert!(pass(
            "ci.yml",
            "run: cargo test --manifest-path cli/Cargo.toml",
            "run: cargo test --manifest-path Cargo.toml"
        ));
        assert!(pass("ci.yml", "    working-directory: cli", "    working-directory: ."));
        assert!(pass(
            "a.rs",
            "include_str!(\"../../../spec/registry.json\")",
            "include_str!(\"../../spec/registry.json\")"
        ));
        assert!(pass(
            "package.json",
            "\"build\": \"cargo run --manifest-path ../cli/Cargo.toml\",",
            "\"build\": \"cargo run --manifest-path ../Cargo.toml\","
        ));
        assert!(pass("AGENTS.md", "Run it from `cli`.", "Run it from `.`."));
        assert!(pass("a.ts", "import { x } from \"../cli/a\";", "import { x } from \"../a\";"));
        assert!(pass("a.rs", "if c == '\"' { load(\"cli/a.json\") }", "if c == '\"' { load(\"a.json\") }"));
        assert!(pass("a.rs", "load(\"cli/a.json\");\r\n", "load(\"a.json\");\r\n"));
        assert!(pass("a.rs", "let s = \"a\\nb\"; load(\"cli/a.json\")", "let s = \"a\\nb\"; load(\"a.json\")"));

        assert!(!pass("a.rs", "let x = 1;", "let x = compute(1);"));
        assert!(!pass("ci.yml", "run: cargo test", "run: cargo test --release"));
        assert!(!pass(
            "facts.rs",
            "    println!(\"{:.0} ms\", elapsed.as_secs_f64() * 1e3);",
            "    println!(\"{:.0} ms\", elapsed.as_secs_f64() / 1e3);"
        ));
        assert!(!pass(
            "facts.rs",
            "    for u in facts.unanswered.iter().take(20) {",
            "    for u in facts.unanswered.iter().take(21) {"
        ));
        assert!(!pass("a.rs", "let p = a / b;", "let p = a . b;"));
        assert!(!pass("a.rs", "let p = \"a / b\";", "let p = \"a . b\";"));
        assert!(!pass("a.rs", "let name = \"cli\";", "let name = \"crates\";"));
        assert!(!pass("a.ts", "const s = `${a / b}`;", "const s = `${a.b}`;"));
        assert!(!pass("ci.yml", "if: github.ref == 'refs/heads/main'", "if: github.sha == 'refs/heads/main'"));
        assert!(!pass("ci.yml", "  run: cargo test", "    run: cargo test"));
        assert!(!pass(
            "a.rs",
            "fn f(x: &'static str) -> usize { x.len() * 2 }",
            "fn f(x: &'static str) -> usize { x.len() * a/2 }"
        ));
        assert!(!pass("a.ts", "let answer = a + ++b;", "let answer = a++ + b;"));
        assert!(!pass("a.ts", "let answer = a - -b;", "let answer = a--b;"));
        assert!(!pass("a.rs", "let p = load(\"../cli/a.json\") + 1;", "let p = load(\"../a.json\") - 1;"));
        assert!(!pass("a.rs", "let p = load(\"cli/a.json\");", "let p = load( \"a.json\");"));
        assert!(!pass("a.rs", "let p = load(\"cli/a.json\");", "let p = load('cli/a.json');"));
        assert!(!pass("package.json", "\"cargo run  cli/a.toml\"", "\"cargo run a.toml\""));
        assert!(!pass("a.rs", "if c == '\"' { n = a/b; } let s = \"x\";", "if c == '\"' { n = a/c; } let s = \"x\";"));
        assert!(!pass("a.ts", "let s = 'it; n = a/b;", "let s = 'it; n = a/c;"));
        assert!(!pass("ci.yml", "run: cargo test cli/a.toml", "run: cargo test\tcli/a.toml"));
        assert!(!pass("a.ts", "let s = \"a\\nb\";", "let s = \"a\\tb\";"));
        assert!(!pass("a.rs", "load(\"cli/a.json\", \"\\n\")", "load(\"a.json\", \"\\t\")"));
        assert!(!pass("a.rs", "load(\"cli/a.json \\n\")", "load(\"a.json \\t\")"));
        assert!(!pass("a.rs", "load(\"cli\\a.json\")", "load(\"a.json\")"));
        assert!(!pass("a.rs", "let x = 1;\r\n", "let x = 1;\n"));
        assert!(!pass("a.rs", "let x = 1;\n", "let x = 1;"));
        assert!(!pass("ci.yml", "run: cargo test cli/a.toml\r\n", "run: cargo test a.toml\n"));
    }
}
