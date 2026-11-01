//! Checks a project and prints a summary of its fact table.
//!
//! `cargo run --release -p crucible-compiler --example facts -- <tsconfig>`

use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let Some(tsconfig) = std::env::args_os().nth(1) else {
        eprintln!("usage: facts <tsconfig>");
        return ExitCode::from(2);
    };
    let start = Instant::now();
    let facts = match crucible_compiler::analyze(tsconfig.as_ref()) {
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
    for u in facts.unanswered.iter().take(20) {
        println!("  {}:{}-{} {:?} {:?}", u.path, u.span.start, u.span.end, u.site, u.reason);
    }
    println!("time        {:.0} ms", elapsed.as_secs_f64() * 1e3);
    ExitCode::SUCCESS
}
