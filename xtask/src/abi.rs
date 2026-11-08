//! `cargo xtask abi` writes the runtime's `abi.d.ts` from `crates/abi`. `cargo xtask abi check` fails when that file is
//! stale, when the runtime's `package.json` records another ABI version, or when the runtime exports no entry for a
//! subpath the table lets games import. It never reads the runtime's code. `emit/index.ts` imports the declaration's
//! types, so tsrs fails `pnpm check` when the runtime drifts from the table.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::run::repo;

pub const DECLARATION: &str = "packages/crucible/src/emit/abi.d.ts";
const MANIFEST: &str = "packages/crucible/package.json";
const USAGE: &str = "usage: cargo xtask abi [check]";

pub fn main(args: &[String]) -> Result<(), String> {
    let root = repo();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => {
            let path = root.join(DECLARATION);
            fs::write(&path, crucible_abi::declaration()).map_err(|e| format!("cannot write {DECLARATION}: {e}"))?;
            println!("abi: wrote {DECLARATION} for ABI {}", crucible_abi::VERSION);
            check(&root).map(|summary| println!("abi: {summary}"))
        }
        ["check"] => check(&root).map(|summary| println!("abi: {summary}")),
        _ => Err(USAGE.into()),
    }
}

/// One line saying what matched, or every mismatch.
pub fn check(root: &Path) -> Result<String, String> {
    let mut problems = Vec::new();
    let written = fs::read_to_string(root.join(DECLARATION)).unwrap_or_default().replace("\r\n", "\n");
    if written != crucible_abi::declaration() {
        problems.push(format!("{DECLARATION} is stale. Run `cargo xtask abi`."));
    }
    let manifest = fs::read_to_string(root.join(MANIFEST)).map_err(|e| format!("cannot read {MANIFEST}: {e}"))?;
    let manifest: Value = serde_json::from_str(&manifest).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let field = crucible_abi::MANIFEST_FIELD;
    if manifest[field] != crucible_abi::VERSION {
        problems.push(format!("{MANIFEST} sets {field} to {}, not {}.", manifest[field], crucible_abi::VERSION));
    }
    for subpath in crucible_abi::SUBPATHS {
        if manifest["exports"].get(format!("./{subpath}")).is_none() {
            problems.push(format!("{MANIFEST} exports no ./{subpath}, which games may import."));
        }
    }
    if problems.is_empty() {
        Ok(format!(
            "{DECLARATION} is current with crates/abi, and {MANIFEST} records ABI {} and exports every subpath games \
             may import. This does not check the runtime's code: tsrs checks emit/index.ts against {DECLARATION} in \
             `pnpm check`.",
            crucible_abi::VERSION
        ))
    } else {
        Err(format!("abi: the runtime does not match crates/abi:\n{}", problems.join("\n")))
    }
}
