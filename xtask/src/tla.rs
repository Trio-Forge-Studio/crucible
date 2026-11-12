//! `cargo xtask tla` model-checks each TLA+ spec under `specs/` with TLC, then applies each mutant in the spec's
//! `mutants.toml` and fails unless TLC finds a counterexample to the property that mutant names. A spec is a folder
//! holding `<Name>.tla` and `<Name>.cfg`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::run::repo;

const TLC_VERSION: &str = "1.7.4";
const TLC_SHA256: &str = "936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88";
const USAGE: &str = "usage: cargo xtask tla";

#[derive(Deserialize)]
struct Mutants {
    mutant: Vec<Mutant>,
}

#[derive(Deserialize)]
struct Mutant {
    name: String,
    breaks: String,
    find: String,
    replace: String,
}

pub fn main(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(USAGE.into());
    }
    let root = repo();
    let jar = tlc_jar(&root)?;
    let java =
        std::env::var_os("JAVA_HOME").map_or_else(|| PathBuf::from("java"), |home| Path::new(&home).join("bin/java"));
    let work = root.join("target/tla");
    let mut specs: Vec<PathBuf> = fs::read_dir(root.join("specs"))
        .map_err(|e| format!("cannot read specs: {e}"))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    specs.sort();
    for dir in specs {
        let name = spec_name(&dir)?;
        let module = fs::read_to_string(dir.join(format!("{name}.tla"))).map_err(|e| format!("{name}.tla: {e}"))?;
        let config = fs::read_to_string(dir.join(format!("{name}.cfg"))).map_err(|e| format!("{name}.cfg: {e}"))?;
        let run = tlc(&java, &jar, &work.join(&name), &name, &module, &config)?;
        if !run.output.contains("Model checking completed. No error has been found.") {
            return Err(format!("tla: TLC found an error in {name}:\n{}", run.output));
        }
        println!("tla: {name} holds: {}", run.summary());
        let mutants = dir.join("mutants.toml");
        let Ok(text) = fs::read_to_string(&mutants) else { continue };
        let mutants: Mutants = toml::from_str(&text).map_err(|e| format!("{}: {e}", mutants.display()))?;
        for mutant in mutants.mutant {
            let mutated = mutate(&module, &mutant)?;
            let config = check_only(&config, &mutant.breaks)?;
            let run = tlc(&java, &jar, &work.join(format!("{name}-{}", mutant.name)), &name, &mutated, &config)?;
            if !caught(&run.output, &mutant.breaks) {
                return Err(format!(
                    "tla: mutant {} of {name} should break {}, but TLC found no counterexample:\n{}",
                    mutant.name, mutant.breaks, run.output
                ));
            }
            println!("tla: mutant {} breaks {} in {} states", mutant.name, mutant.breaks, run.trace_length());
        }
    }
    Ok(())
}

fn spec_name(dir: &Path) -> Result<String, String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.strip_suffix(".tla").map(str::to_owned))
        .filter(|name| dir.join(format!("{name}.cfg")).is_file())
        .collect();
    match names.len() {
        1 => Ok(names.remove(0)),
        _ => Err(format!("{} must hold exactly one <Name>.tla with a <Name>.cfg", dir.display())),
    }
}

fn mutate(module: &str, mutant: &Mutant) -> Result<String, String> {
    match module.matches(&mutant.find).count() {
        1 => Ok(module.replacen(&mutant.find, &mutant.replace, 1)),
        count => Err(format!("mutant {}: its find text occurs {count} times, not once", mutant.name)),
    }
}

/// The configuration with its invariants and properties replaced by `property` alone, under the section it came
/// from, so TLC reports the counterexample to that property and no other.
fn check_only(config: &str, property: &str) -> Result<String, String> {
    let mut kept = String::new();
    let mut section = None;
    let mut found = None;
    for line in config.lines() {
        let word = line.trim();
        if matches!(word, "INVARIANT" | "INVARIANTS" | "PROPERTY" | "PROPERTIES") {
            section = Some(if word.starts_with("INVARIANT") { "INVARIANT" } else { "PROPERTY" });
            continue;
        }
        match section {
            Some(kind) if word == property => found = Some(kind),
            Some(_) if word.is_empty() || line.starts_with(char::is_whitespace) => {}
            _ => {
                section = None;
                kept.push_str(line);
                kept.push('\n');
            }
        }
    }
    let kind = found.ok_or_else(|| format!("the configuration does not check {property}"))?;
    Ok(format!("{kept}{kind}\n    {property}\n"))
}

fn caught(output: &str, property: &str) -> bool {
    output.contains(&format!("Invariant {property} is violated."))
        || output.contains(&format!("Action property {property} is violated."))
        || output.contains("Temporal properties were violated.")
}

struct Run {
    output: String,
}

impl Run {
    fn summary(&self) -> String {
        let line = |text: &str| self.output.lines().rfind(|line| line.contains(text)).unwrap_or_default().trim();
        format!("{} {}", line("distinct states found"), line("The depth of the complete state graph search"))
    }

    fn trace_length(&self) -> usize {
        self.output.lines().filter(|line| line.starts_with("State ")).count()
    }
}

fn tlc(java: &Path, jar: &Path, work: &Path, name: &str, module: &str, config: &str) -> Result<Run, String> {
    let _ = fs::remove_dir_all(work);
    fs::create_dir_all(work).map_err(|e| format!("cannot create {}: {e}", work.display()))?;
    fs::write(work.join(format!("{name}.tla")), module).map_err(|e| format!("cannot write {name}.tla: {e}"))?;
    fs::write(work.join(format!("{name}.cfg")), config).map_err(|e| format!("cannot write {name}.cfg: {e}"))?;
    let output = Command::new(java)
        .current_dir(work)
        .arg(format!("-Djava.io.tmpdir={}", work.display()))
        .arg("-XX:+UseParallelGC")
        .arg("-cp")
        .arg(jar)
        .args(["tlc2.TLC", "-workers", "2", "-cleanup", "-metadir", "states", "-config"])
        .arg(format!("{name}.cfg"))
        .arg(format!("{name}.tla"))
        .output()
        .map_err(|e| format!("cannot start {}: {e}. TLC needs Java 11 or later.", java.display()))?;
    let output = String::from_utf8_lossy(&output.stdout).into_owned();
    fs::write(work.join("tlc.out"), &output).map_err(|e| format!("cannot write the TLC output: {e}"))?;
    Ok(Run { output })
}

fn tlc_jar(root: &Path) -> Result<PathBuf, String> {
    if let Some(jar) = std::env::var_os("TLA2TOOLS_JAR") {
        return Ok(jar.into());
    }
    let jar = root.join(format!("target/tla/tla2tools-{TLC_VERSION}.jar"));
    if !jar.is_file() {
        let url = format!("https://github.com/tlaplus/tlaplus/releases/download/v{TLC_VERSION}/tla2tools.jar");
        let bytes = ureq::get(&url)
            .call()
            .map_err(|e| format!("cannot download {url}: {e}"))?
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| format!("cannot read {url}: {e}"))?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if digest != TLC_SHA256 {
            return Err(format!("{url} has SHA-256 {digest}, not {TLC_SHA256}"));
        }
        fs::create_dir_all(jar.parent().expect("the jar has a folder")).map_err(|e| e.to_string())?;
        fs::write(&jar, bytes).map_err(|e| format!("cannot write {}: {e}", jar.display()))?;
    }
    Ok(jar)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "SPECIFICATION Spec\nCONSTANTS\n    Keys = {a, b}\nINVARIANTS\n    TypeOK\n    AtMostOne\n\
                          PROPERTIES\n    NoBlindCreate\n    Converges\n";

    #[test]
    fn a_mutant_checks_only_the_property_it_breaks() {
        let expected = "SPECIFICATION Spec\nCONSTANTS\n    Keys = {a, b}\n";
        assert_eq!(check_only(CONFIG, "AtMostOne").unwrap(), format!("{expected}INVARIANT\n    AtMostOne\n"));
        assert_eq!(check_only(CONFIG, "Converges").unwrap(), format!("{expected}PROPERTY\n    Converges\n"));
        assert!(check_only(CONFIG, "Missing").is_err());
    }

    #[test]
    fn a_mutant_must_match_once() {
        let mutant =
            |find: &str| Mutant { name: "m".into(), breaks: "P".into(), find: find.into(), replace: "y".into() };
        assert_eq!(mutate("a x b", &mutant("x")).unwrap(), "a y b");
        assert!(mutate("x x", &mutant("x")).is_err());
        assert!(mutate("a", &mutant("x")).is_err());
    }

    #[test]
    fn every_mutant_of_every_spec_applies_and_names_a_checked_property() {
        for dir in fs::read_dir(repo().join("specs")).unwrap().map(|entry| entry.unwrap().path()) {
            let name = spec_name(&dir).unwrap();
            let module = fs::read_to_string(dir.join(format!("{name}.tla"))).unwrap();
            let config = fs::read_to_string(dir.join(format!("{name}.cfg"))).unwrap();
            let mutants: Mutants = toml::from_str(&fs::read_to_string(dir.join("mutants.toml")).unwrap()).unwrap();
            for mutant in &mutants.mutant {
                mutate(&module, mutant).unwrap();
                check_only(&config, &mutant.breaks).unwrap();
            }
        }
    }
}
