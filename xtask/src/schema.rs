//! `cargo xtask schema` writes the JSON Schema of `crucible.toml` from `crucible-project`'s file model, and
//! `cargo xtask schema check` fails when the committed schema is stale. The CLI's npm package ships the file.

use std::fs;

use crate::run::repo;

pub const SCHEMA: &str = "crates/project/crucible.schema.json";
const USAGE: &str = "usage: cargo xtask schema [check]";

pub fn main(args: &[String]) -> Result<(), String> {
    let path = repo().join(SCHEMA);
    let schema = crucible_project::schema();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => {
            fs::write(&path, schema).map_err(|e| format!("cannot write {SCHEMA}: {e}"))?;
            println!("schema: wrote {SCHEMA}");
            Ok(())
        }
        ["check"] => {
            let written = fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
            if written == schema {
                println!("schema: {SCHEMA} is current with crates/project");
                Ok(())
            } else {
                Err(format!("schema: {SCHEMA} is stale. Run `cargo xtask schema`."))
            }
        }
        _ => Err(USAGE.into()),
    }
}
