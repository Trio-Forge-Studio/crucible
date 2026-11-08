use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use crucible_compiler::Target;

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let (tsconfig, out, target) = match &args[..] {
        [tsconfig, out] => (tsconfig, out, Target::Package),
        [tsconfig, out, project, include] => (tsconfig, out, Target::Game { project, include }),
        _ => {
            eprintln!("usage: compile <tsconfig> <out> [<project> <include>]");
            return ExitCode::from(2);
        }
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
