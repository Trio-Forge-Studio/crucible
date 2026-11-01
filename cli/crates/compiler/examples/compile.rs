//! Compiles a project to Luau, writing only the files whose bytes changed.
//!
//! `cargo run --release -p crucible-compiler --example compile -- <tsconfig> <out>`

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(tsconfig), Some(out)) = (args.next(), args.next()) else {
        eprintln!("usage: compile <tsconfig> <out>");
        return ExitCode::from(2);
    };
    let start = Instant::now();
    let modules = match crucible_compiler::compile(tsconfig.as_ref()) {
        Ok(modules) => modules,
        Err(failure) => {
            eprint!("{failure}");
            return ExitCode::FAILURE;
        }
    };
    let compiled = start.elapsed();
    let written = match crucible_compiler::write(&PathBuf::from(out), &modules) {
        Ok(written) => written,
        Err(e) => {
            eprintln!("cannot write the output: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("modules     {}", modules.len());
    println!("written     {written}");
    println!("compile     {:.0} ms", compiled.as_secs_f64() * 1e3);
    println!("total       {:.0} ms", start.elapsed().as_secs_f64() * 1e3);
    ExitCode::SUCCESS
}
