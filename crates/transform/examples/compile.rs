use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use crucible_compiler::{InstanceMap, Target, VirtualModule};

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let (args, module) = match &args[..] {
        [rest @ .., specifier, path, luau] if rest.len() == 4 => match std::fs::read_to_string(luau) {
            Ok(source) => (rest, Some((specifier.to_string_lossy(), path.to_string_lossy(), source))),
            Err(e) => {
                eprintln!("cannot read {}: {e}", luau.display());
                return ExitCode::FAILURE;
            }
        },
        _ => (&args[..], None),
    };
    let modules: Vec<_> =
        module.iter().map(|(specifier, path, source)| VirtualModule { specifier, path, source }).collect();
    let (tsconfig, out, place) = match args {
        [tsconfig, out] => (tsconfig, out, None),
        [tsconfig, out, project, include] => match crucible_project::rojo::place(project) {
            Ok(mappings) => {
                let instances = InstanceMap::new(project.display().to_string(), mappings.folders, mappings.files);
                (tsconfig, out, Some((instances, include)))
            }
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::FAILURE;
            }
        },
        _ => {
            eprintln!("usage: compile <tsconfig> <out> [<project> <include> [<specifier> <path> <luau>]]");
            return ExitCode::from(2);
        }
    };
    let target = match &place {
        Some((instances, include)) => Target::Game { instances, include, modules: &modules },
        None => Target::Package,
    };
    let start = Instant::now();
    let compiled = match crucible_transform::compile(tsconfig, target) {
        Ok(compiled) => compiled,
        Err(failure) => {
            eprint!("{failure}");
            return ExitCode::FAILURE;
        }
    };
    let elapsed = start.elapsed();
    let written = crucible_compiler::write(out, &compiled.modules).and_then(|changes| match target {
        Target::Game { include, .. } => crucible_compiler::write_include(include).map(|_| changes),
        Target::Package => Ok(changes),
    });
    let changes = match written {
        Ok(changes) => changes,
        Err(e) => {
            eprintln!("cannot write the output: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("modules     {}", compiled.modules.len());
    println!("written     {}", changes.written);
    println!("renamed     {}", changes.renamed);
    println!("removed     {}", changes.removed);
    println!("compile     {:.0} ms", elapsed.as_secs_f64() * 1e3);
    println!("total       {:.0} ms", start.elapsed().as_secs_f64() * 1e3);
    ExitCode::SUCCESS
}
