//! `cargo xtask proof [<base>]` prints the proof table a pull request body carries.
//!
//! A `Review: mechanical` commit gets two checks, and the table lists what each one flags:
//!
//! - Every rename keeps at least 95% similarity.
//! - Its edits only update paths. The heuristic compares each hunk's added lines with its removed lines, pair by
//!   pair, as runs of letters, digits and `/\.-_:@~`. A run the new line adds must hold `/`, `\` or `.`, so it
//!   reads as a path. A hunk that adds more lines than it removes, a new file and a binary change are flagged.
//!   Removing lines is allowed, because a removed line adds nothing to read.

use std::path::Path;

use crate::arch;
use crate::golden::{self, BASELINE};
use crate::run::{git, git_maybe, repo};

const USAGE: &str = "usage: cargo xtask proof [<base>]";
const SIMILARITY: u32 = 95;

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
    for commit in commits.lines() {
        let subject = git(&root, &["log", "-1", "--format=%s", commit])?.trim().to_owned();
        let review = git(&root, &["log", "-1", "--format=%(trailers:key=Review,valueonly,separator=%x2C)", commit])?
            .trim()
            .to_owned();
        let evidence = match review.as_str() {
            "mechanical" => mechanical(&root, commit)?,
            "read" => "Read it.".to_owned(),
            "" => "No `Review:` trailer.".to_owned(),
            other => format!("Unknown kind `{other}`."),
        };
        let kind = if review.is_empty() { "none" } else { review.as_str() };
        out.push_str(&format!("| `{}` {} | {kind} | {evidence} |\n", &commit[..7], cell(&subject)));
    }
    out.push_str(&golden.1);
    out.push_str(&arch.1);
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

/// What the mechanical checks flag in `commit`, or a summary of the evidence when they flag nothing.
fn mechanical(root: &Path, commit: &str) -> Result<String, String> {
    let range = [format!("{commit}^"), commit.to_owned()];
    let status = git(root, &["diff", "--name-status", "-M50%", &range[0], &range[1]])?;
    let summary = git(root, &["diff", &format!("-M{SIMILARITY}%"), "--summary", &range[0], &range[1]])?;
    let mut flags = Vec::new();
    let mut renames = 0;
    for line in status.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let code = fields.first().copied().unwrap_or_default();
        if let Some(similarity) = code.strip_prefix('R') {
            renames += 1;
            let similarity: u32 = similarity.parse().unwrap_or(0);
            if similarity < SIMILARITY {
                flags.push(format!("renames `{}` at {similarity}% similarity", fields.get(1).unwrap_or(&"")));
            }
        } else if code == "A" || code.starts_with('C') {
            flags.push(format!("adds `{}`", fields.last().unwrap_or(&"")));
        }
    }
    let exact = summary.lines().filter(|line| line.contains("(100%)")).count();
    let patch = git(root, &["diff", "-U0", "-M50%", &range[0], &range[1]])?;
    let mut edited = 0;
    for file in files(&patch) {
        if file.binary {
            flags.push(format!("changes binary `{}`", file.path));
            continue;
        }
        if file.hunks.is_empty() {
            continue;
        }
        edited += 1;
        for hunk in &file.hunks {
            if hunk.added.len() > hunk.removed.len() {
                flags.push(format!("`{}` gains {} lines", file.path, hunk.added.len() - hunk.removed.len()));
            }
            for (added, removed) in hunk.added.iter().zip(&hunk.removed) {
                if !path_update(removed, added) {
                    flags.push(format!("`{}` changes more than a path: `{}`", file.path, added.trim()));
                }
            }
        }
    }
    if flags.is_empty() {
        return Ok(format!(
            "Renames: {renames}, exact: {exact}. Files with path-only edits: {edited}. Nothing flagged."
        ));
    }
    let shown: Vec<String> = flags.iter().take(10).map(|flag| cell(flag)).collect();
    let more = if flags.len() > 10 { format!(" And {} more.", flags.len() - 10) } else { String::new() };
    Ok(format!("Flagged: {}.{more}", shown.join("; ")))
}

#[derive(Default)]
struct File {
    path: String,
    binary: bool,
    hunks: Vec<Hunk>,
}

#[derive(Default)]
struct Hunk {
    removed: Vec<String>,
    added: Vec<String>,
}

fn files(patch: &str) -> Vec<File> {
    let mut files: Vec<File> = Vec::new();
    for line in patch.lines() {
        if let Some(header) = line.strip_prefix("diff --git ") {
            let path = header.rsplit(" b/").next().unwrap_or(header).to_owned();
            files.push(File { path, ..File::default() });
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if line.starts_with("Binary files ") {
            file.binary = true;
        } else if line.starts_with("@@") {
            file.hunks.push(Hunk::default());
        } else if let Some(hunk) = file.hunks.last_mut() {
            if let Some(text) = line.strip_prefix('-') {
                hunk.removed.push(text.to_owned());
            } else if let Some(text) = line.strip_prefix('+') {
                hunk.added.push(text.to_owned());
            }
        }
    }
    files
}

fn tokens(line: &str) -> Vec<&str> {
    line.split(|c: char| !(c.is_alphanumeric() || "/\\.-_:@~".contains(c))).filter(|token| !token.is_empty()).collect()
}

/// Whether every token `added` has beyond the longest common run with `removed` looks like a path.
pub fn path_update(removed: &str, added: &str) -> bool {
    let (old, new) = (tokens(removed), tokens(added));
    let mut table = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i][j] = if old[i] == new[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    while j < new.len() {
        if i < old.len() && old[i] == new[j] {
            (i, j) = (i + 1, j + 1);
        } else if i < old.len() && table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            if !new[j].contains(['/', '\\', '.']) {
                return false;
            }
            j += 1;
        }
    }
    true
}

/// Text that is safe inside a Markdown table cell.
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
            .args(args)
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
    }

    #[test]
    fn mechanical_commits_are_flagged_when_they_add_logic() {
        let dir = std::env::temp_dir().join(format!("xtask-proof-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cli")).unwrap();
        let body: String = (0..40).map(|i| format!("line {i} stays the same\n")).collect();
        std::fs::write(dir.join("cli/lib.rs"), &body).unwrap();
        std::fs::write(dir.join("ci.yml"), "run: cargo test --manifest-path cli/Cargo.toml\n").unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "base"]);
        std::fs::create_dir_all(dir.join("crates")).unwrap();
        std::fs::rename(dir.join("cli/lib.rs"), dir.join("crates/lib.rs")).unwrap();
        std::fs::write(dir.join("ci.yml"), "run: cargo test --manifest-path Cargo.toml\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "move"]);
        let clean = mechanical(&dir, "HEAD").unwrap();
        std::fs::write(dir.join("crates/lib.rs"), body.replace("line 3 stays", "line 3 grows")).unwrap();
        std::fs::write(dir.join("ci.yml"), "run: cargo test --release\n").unwrap();
        git(&dir, &["commit", "-q", "-am", "logic"]);
        let flagged = mechanical(&dir, "HEAD").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(clean, "Renames: 1, exact: 1. Files with path-only edits: 1. Nothing flagged.");
        assert!(flagged.starts_with("Flagged:"), "{flagged}");
        assert!(flagged.contains("`crates/lib.rs` changes more than a path"), "{flagged}");
        assert!(flagged.contains("`ci.yml` changes more than a path"), "{flagged}");
    }

    #[test]
    fn path_updates_pass_and_new_logic_does_not() {
        assert!(path_update(
            "run: cargo test --manifest-path cli/Cargo.toml",
            "run: cargo test --manifest-path Cargo.toml"
        ));
        assert!(path_update("    working-directory: cli", "    working-directory: ."));
        assert!(path_update(
            "include_str!(\"../../../spec/registry.json\")",
            "include_str!(\"../../spec/registry.json\")"
        ));
        assert!(!path_update("let x = 1;", "let x = compute(1);"));
        assert!(!path_update("run: cargo test", "run: cargo test --release"));
    }
}
