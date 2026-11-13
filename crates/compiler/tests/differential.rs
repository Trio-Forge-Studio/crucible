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

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

use ast::Program;
use run::{Failure, Outcome, Runner};

const DEFAULT_SEED: u64 = 0x0c0f_fee5_eed5_2026;

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
    let mut seed_bytes = [0u8; 32];
    seed_bytes[..8].copy_from_slice(&seed.to_le_bytes());
    let mut rng = TestRunner::new_with_rng(Config::default(), TestRng::from_seed(RngAlgorithm::ChaCha, &seed_bytes));
    let strategy = generate::program();
    let mut runner = Runner::new(root.join(format!("work-{}", std::process::id())));
    let failures_dir = root.join("failures");
    let _ = std::fs::remove_dir_all(&failures_dir);
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
            let Some(kind) = outcome.failure() else { continue };
            failed += 1;
            if keep_going {
                std::fs::create_dir_all(&failures_dir).expect("the failures folder is made");
                let name = format!("{:06}", done + i);
                std::fs::write(failures_dir.join(format!("{name}.ts")), &sources[i]).expect("the program is written");
                std::fs::write(failures_dir.join(format!("{name}.txt")), describe(&outcome))
                    .expect("the outcome is written");
            } else if first.is_none() {
                first = Some((done + i, tree, kind, outcome));
            }
        }
        done += n;
        eprintln!("{done} programs, {traced} trace lines, {failed} failing, {:.0?}", started.elapsed());
        if let Some((index, tree, kind, outcome)) = first {
            let (source, outcome) = shrink(tree, kind, outcome, &sources[index - (done - n)], &mut runner);
            panic!(
                "program {index} of seed {seed} fails ({failed} of the {n} in its batch fail):\n{}\nshrunk program:\n{source}",
                describe(&outcome)
            );
        }
    }
    assert!(failed == 0, "{failed} of {cases} programs fail; see {}", failures_dir.display());
}

/// Runs every program in `differential/regressions`: each is the shrunk program of a difference the compiler no
/// longer has.
#[test]
#[ignore = "runs programs in Node and Lune, which must be on PATH"]
fn regressions_trace_the_same_in_node_and_lune() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/differential/regressions");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the regressions folder is read")
        .map(|e| e.expect("the entry is read").path())
        .filter(|p| p.extension().is_some_and(|e| e == "ts"))
        .collect();
    paths.sort();
    let sources: Vec<String> = paths.iter().map(|p| std::fs::read_to_string(p).expect("the program is read")).collect();
    let root =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("differential-regressions-{}", std::process::id()));
    let outcomes = Runner::new(root).run(&sources);
    let failing: Vec<String> = paths
        .iter()
        .zip(&outcomes)
        .filter(|(_, o)| o.failure().is_some())
        .map(|(p, o)| format!("{}: {}", p.display(), describe(o)))
        .collect();
    assert!(failing.is_empty(), "{}", failing.join("\n"));
    assert!(outcomes.iter().all(|o| matches!(o, Outcome::Ran { node, .. } if !node.is_empty())), "{outcomes:?}");
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

/// Shrinks with proptest's own value tree, keeping each simpler program that still fails the same way.
fn shrink(
    mut tree: Box<dyn ValueTree<Value = Program>>,
    kind: Failure,
    outcome: Outcome,
    source: &str,
    runner: &mut Runner,
) -> (String, Outcome) {
    let limit = Duration::from_secs(env("DIFFERENTIAL_SHRINK_SECONDS", 300));
    let started = Instant::now();
    let mut best = (source.to_owned(), outcome);
    let mut seen: HashMap<String, Outcome> = HashMap::new();
    let mut runs = 0;
    if !tree.simplify() {
        return best;
    }
    while started.elapsed() < limit {
        let source = print::print(&tree.current());
        let outcome = match seen.get(&source) {
            Some(outcome) => outcome.clone(),
            None => {
                runs += 1;
                let outcome = runner.run(std::slice::from_ref(&source)).remove(0);
                seen.insert(source.clone(), outcome.clone());
                outcome
            }
        };
        let more = if outcome.failure() == Some(kind) {
            best = (source, outcome);
            tree.simplify()
        } else {
            tree.complicate()
        };
        if !more {
            break;
        }
    }
    eprintln!("shrunk in {runs} runs and {:.0?}", started.elapsed());
    best
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
