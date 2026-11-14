//! A differential test of the compiler. It generates TypeScript programs, runs each as JavaScript in Node and as the
//! compiler's Luau in Lune, and fails when their traces differ. `differential/README.md` states the subset the
//! generator covers, what it leaves out and why, and how both runtimes print a value. `differential/known.md` lists the
//! differences found so far.
//!
//! It needs `node` and `lune` on PATH and runs with the other ignored tests. `DIFFERENTIAL_CASES` sets the number of
//! programs (default 1,000) and `DIFFERENTIAL_SEED` the seed (default fixed), so CI always runs the same programs:
//!
//! `DIFFERENTIAL_CASES=50000 cargo test --release -p crucible-compiler --test differential -- --ignored`
//!
//! `DIFFERENTIAL_KEEP_GOING=1` runs every case instead of stopping at the first failing batch, writes each failing
//! program to `target/tmp/differential/failures`, and shrinks none of them.

#[path = "differential/ast.rs"]
mod ast;
#[path = "differential/generate.rs"]
mod generate;
#[path = "differential/print.rs"]
mod print;
#[path = "differential/run.rs"]
mod run;
#[path = "differential/shrink.rs"]
mod shrink;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

use run::{Outcome, Runner};

const DEFAULT_SEED: u64 = 0x0c0f_fee5_eed5_2026;

/// The generator's random source, so a seed always gives the same programs in the same order.
fn rng(seed: u64) -> TestRunner {
    let mut seed_bytes = [0u8; 32];
    seed_bytes[..8].copy_from_slice(&seed.to_le_bytes());
    TestRunner::new_with_rng(Config::default(), TestRng::from_seed(RngAlgorithm::ChaCha, &seed_bytes))
}

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[test]
#[ignore = "compiles in a child process for the differential test"]
fn compile_child() {
    if let Some(tsconfig) = std::env::var_os(run::CHILD_TSCONFIG) {
        run::compile_here(&PathBuf::from(tsconfig));
    }
}

#[test]
#[ignore = "runs generated programs in Node and Lune, which must be on PATH"]
fn generated_programs_trace_the_same_in_node_and_lune() {
    let cases: usize = env("DIFFERENTIAL_CASES", 1000);
    let seed: u64 = env("DIFFERENTIAL_SEED", DEFAULT_SEED);
    let batch: usize = env("DIFFERENTIAL_BATCH", 250);
    let keep_going = std::env::var_os("DIFFERENTIAL_KEEP_GOING").is_some();
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("differential");
    let mut rng = rng(seed);
    let strategy = generate::program();
    let mut runner = Runner::new(root.join(format!("work-{}", std::process::id())));
    runner.keep = std::env::var_os("DIFFERENTIAL_KEEP").is_some();
    let failures_dir = root.join("failures");
    if keep_going {
        let _ = std::fs::remove_dir_all(&failures_dir);
    }
    let started = Instant::now();
    let mut done = 0;
    let mut failed = 0;
    let mut traced = 0;
    while done < cases {
        let n = batch.min(cases - done);
        let trees: Vec<_> = (0..n).map(|_| strategy.new_tree(&mut rng).expect("a program is generated")).collect();
        let sources: Vec<String> = trees.iter().map(|t| print::print(&t.current())).collect();
        let outcomes = runner.run(&sources);
        let mut first = None;
        for (i, (tree, outcome)) in trees.into_iter().zip(outcomes).enumerate() {
            if let Outcome::Ran { node, .. } = &outcome {
                traced += node.len();
            }
            if outcome.failure().is_none() {
                continue;
            }
            failed += 1;
            if keep_going {
                std::fs::create_dir_all(&failures_dir).expect("the failures folder is made");
                let name = format!("{:06}", done + i);
                std::fs::write(failures_dir.join(format!("{name}.ts")), &sources[i]).expect("the program is written");
                std::fs::write(failures_dir.join(format!("{name}.txt")), describe(&outcome))
                    .expect("the outcome is written");
            } else if first.is_none() {
                first = Some((done + i, tree.current(), sources[i].clone(), outcome));
            }
        }
        done += n;
        eprintln!("{done} programs, {traced} trace lines, {failed} failing, {:.0?}", started.elapsed());
        if let Some((index, program, unshrunk, outcome)) = first {
            let limit = Duration::from_secs(env("DIFFERENTIAL_SHRINK_SECONDS", 300));
            let shrinking = Instant::now();
            let mut run = |_: &[ast::Program], sources: &[String]| runner.run(sources);
            let (source, shrunk, rounds) = shrink::shrink(program, outcome.clone(), batch, limit, &mut run);
            eprintln!("shrunk in {rounds} rounds and {:.0?}", shrinking.elapsed());
            std::fs::create_dir_all(&failures_dir).expect("the failures folder is made");
            let file = failures_dir.join(format!("seed-{seed}-program-{index}.ts"));
            let original = failures_dir.join(format!("seed-{seed}-program-{index}-unshrunk.ts"));
            std::fs::write(&file, &source).expect("the shrunk program is written");
            std::fs::write(&original, &unshrunk).expect("the unshrunk program is written");
            panic!(
                "program {index} of seed {seed} fails ({failed} of the {n} in its batch fail). Rerun the same programs \
                 with DIFFERENTIAL_SEED={seed} DIFFERENTIAL_CASES={}, and add DIFFERENTIAL_KEEP_GOING=1 to write every \
                 failing program unshrunk. Run one program alone with DIFFERENTIAL_FILE=<file> and the one_program \
                 test.\nunshrunk program: {}\n{}\nshrunk program: {}\n{}\n{source}",
                index + 1,
                original.display(),
                describe(&outcome),
                file.display(),
                describe(&shrunk),
            );
        }
    }
    assert!(failed == 0, "{failed} of {cases} programs fail; see {}", failures_dir.display());
}

/// The shrinker's own test, which stands an edit of the program in for a compiler bug, so it never changes the compiler:
/// the compiler gets each `a ?? b` over booleans as `a || b`, while Node runs the program as written. That is the bug
/// the first verification of this test made by having the compiler lower `??` to `or`, which only booleans tell apart,
/// and program 11 of the default seed is the one it found. The shrinker then swapped operands of any type, and shrank
/// that program to one with no `??`, a string in a `boolean[]`, that differs on the real compiler too. The shrunk
/// program must still hold a `??`, fail the same way under the edit, and trace the same in Node and Lune on the real
/// compiler, as the program it started from does.
#[test]
#[ignore = "runs programs in Node and Lune, which must be on PATH"]
fn shrinking_keeps_the_difference_it_started_from() {
    const PROGRAM: usize = 11;
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("differential-shrink-{}", std::process::id()));
    let mut edited = Runner::new(root.join("edited"));
    let mut run = |programs: &[ast::Program], sources: &[String]| {
        let compiled: Vec<String> =
            programs.iter().map(|p| print::print(&shrink::map_expressions(p, coalesce_as_or))).collect();
        edited.run_edited(sources, Some(&compiled))
    };
    let mut rng = rng(DEFAULT_SEED);
    let strategy = generate::program();
    let program = (0..=PROGRAM)
        .map(|_| strategy.new_tree(&mut rng).expect("a program is generated").current())
        .last()
        .expect("the program is generated");
    let source = print::print(&program);
    let outcome = run(std::slice::from_ref(&program), std::slice::from_ref(&source)).remove(0);
    assert!(
        matches!(outcome.failure(), Some(run::Failure::Differs(..))),
        "program {PROGRAM} no longer differs under the edit, so the generator changed: pick another that does\n{}",
        describe(&outcome)
    );
    let (shrunk, after, _) = shrink::shrink(program, outcome.clone(), 250, Duration::from_secs(300), &mut run);
    assert_eq!(after.failure(), outcome.failure(), "the shrunk program fails another way:\n{shrunk}");
    assert!(shrunk.contains("??"), "the shrunk program lost the ??:\n{shrunk}");
    let clean = Runner::new(root.join("clean")).run(&[source, shrunk.clone()]);
    assert!(clean[0].failure().is_none(), "program {PROGRAM} fails on the real compiler:\n{}", describe(&clean[0]));
    assert!(
        clean[1].failure().is_none(),
        "the shrunk program fails on the real compiler too, so it shows another difference:\n{}\n{shrunk}",
        describe(&clean[1])
    );
}

#[test]
fn a_difference_is_the_same_whatever_address_a_luau_table_has() {
    let failure = |lune: &str| Outcome::Ran { node: vec!["\"base\"".into()], lune: vec![lune.into()] }.failure();
    assert_eq!(failure("\"table: 0x00005645531e1870\""), failure("\"table: 0x000056091183af70\""));
    assert_ne!(failure("\"table: 0x00005645531e1870\""), failure("\"base2\""));
}

/// `a ?? b` over booleans as `a || b`, which is what lowering `??` to `or` does.
fn coalesce_as_or(e: &mut ast::Expr) {
    use ast::{Expr, Ty};
    if let Expr::Coalesce(a, b) = e
        && matches!(shrink::ty(a), Some(Ty::Bool | Ty::OptBool))
        && let Some(ty @ (Ty::Bool | Ty::OptBool)) = shrink::ty(b)
    {
        *e = Expr::Or(ty, a.clone(), b.clone());
    }
}

/// Runs every program in `differential/regressions`: each is the shrunk program of a difference the compiler no
/// longer has.
#[test]
#[ignore = "runs programs in Node and Lune, which must be on PATH"]
fn regressions_trace_the_same_in_node_and_lune() {
    let ran = run_folder("regressions");
    let failing: Vec<String> = ran
        .iter()
        .filter(|(_, o)| o.failure().is_some())
        .map(|(p, o)| format!("{}: {}", p.display(), describe(o)))
        .collect();
    assert!(failing.is_empty(), "{}", failing.join("\n"));
    assert!(ran.iter().all(|(_, o)| matches!(o, Outcome::Ran { node, .. } if !node.is_empty())), "{ran:?}");
}

/// Runs every program in `differential/known`, the smallest program of each open entry of `known.md`, and fails when
/// one of them no longer differs: the difference is fixed, and its program belongs in `regressions`.
#[test]
#[ignore = "runs programs in Node and Lune, which must be on PATH"]
fn known_differences_still_differ() {
    let ran = run_folder("known");
    let agreeing: Vec<String> = ran
        .iter()
        .filter(|(_, o)| !matches!(o, Outcome::Ran { node, lune } if node != lune))
        .map(|(p, o)| format!("{}: {o:?}", p.display()))
        .collect();
    assert!(agreeing.is_empty(), "these no longer differ:\n{}", agreeing.join("\n"));
}

fn run_folder(folder: &str) -> Vec<(PathBuf, Outcome)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/differential").join(folder);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the folder is read")
        .map(|e| e.expect("the entry is read").path())
        .filter(|p| p.extension().is_some_and(|e| e == "ts"))
        .collect();
    paths.sort();
    let sources: Vec<String> = paths.iter().map(|p| std::fs::read_to_string(p).expect("the program is read")).collect();
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("differential-{folder}-{}", std::process::id()));
    let outcomes = Runner::new(root).run(&sources);
    paths.into_iter().zip(outcomes).collect()
}

/// Runs the program in `DIFFERENTIAL_FILE` alone and prints both traces and its Luau, for a person reading a failure.
#[test]
#[ignore = "runs one program file in Node and Lune when DIFFERENTIAL_FILE names it"]
fn one_program() {
    let Some(path) = std::env::var_os("DIFFERENTIAL_FILE") else { return };
    let source = std::fs::read_to_string(&path).expect("the program file is read");
    let mut runner = Runner::new(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("differential-one"));
    runner.keep = true;
    let outcome = runner.run(&[source]).remove(0);
    match &outcome {
        Outcome::Ran { node, lune } => {
            for (i, line) in node.iter().enumerate() {
                let other = lune.get(i).map_or("(no line)", String::as_str);
                let mark = if other == line { " " } else { "!" };
                eprintln!("{mark} {:3} node {line}\n        lune {other}", i + 1);
            }
            for line in lune.iter().skip(node.len()) {
                eprintln!("!     node (no line)\n        lune {line}");
            }
        }
        other => eprintln!("{}", describe(other)),
    }
}

fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Ran { node, lune } => {
            let at = node.iter().zip(lune).position(|(a, b)| a != b).unwrap_or(node.len().min(lune.len()));
            let line = |lines: &[String]| lines.get(at).cloned().unwrap_or_else(|| "(no line)".into());
            format!("the traces differ at line {}:\n  node: {}\n  lune: {}", at + 1, line(node), line(lune))
        }
        Outcome::Rejected(text) => format!("the compiler rejects it:\n{text}"),
        Outcome::Hung(side) => format!("{side} printed nothing for it in time"),
    }
}
