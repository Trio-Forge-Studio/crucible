//! Checks a project and prints a summary of its fact table.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use crucible_compiler::Target;

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let (tsconfig, target) = match &args[..] {
        [tsconfig] => (tsconfig, Target::Package),
        [tsconfig, project, include] => (tsconfig, Target::Game { project, include }),
        _ => {
            eprintln!("usage: facts <tsconfig> [<project> <include>]");
            return ExitCode::from(2);
        }
    };
    let start = Instant::now();
    let facts = match crucible_compiler::analyze(tsconfig, target, &mut []) {
        Ok(facts) => facts,
        Err(diagnostics) => {
            for d in &diagnostics {
                eprintln!("{d}");
            }
            eprintln!("{} diagnostics", diagnostics.len());
            return ExitCode::FAILURE;
        }
    };
    let elapsed = start.elapsed();
    println!("files       {}", facts.files.len());
    println!("sites       {}", facts.sites);
    println!("facts       {}", facts.len());
    println!("unanswered  {}", facts.unanswered.len());
    println!("diagnostics {}", facts.diagnostics.len());
    for u in facts.unanswered.iter().take(20) {
        println!("  {}:{}-{} {:?} {:?}", u.path, u.span.start, u.span.end, u.site, u.reason);
    }
    println!("time        {:.0} ms", elapsed.as_secs_f64() * 1e3);
    ExitCode::SUCCESS
}
