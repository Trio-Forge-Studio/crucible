mod languages;
mod reader;
mod render;

use crucible_docs::TOPICS;
use std::io::{IsTerminal, Write};
use std::process::ExitCode;

/// Agents read `crucible docs` through a pipe and need the Markdown bytes, so only a person at a terminal gets the reader.
pub fn run(topic: Option<&str>, plain: bool) -> ExitCode {
    let dumb = std::env::var_os("TERM").is_some_and(|term| term == "dumb");
    if plain || dumb || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return print(topic);
    }
    match reader::run(topic) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("crucible docs: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print(topic: Option<&str>) -> ExitCode {
    let text = match topic {
        Some(name) => {
            TOPICS.iter().find(|topic| topic.name == name).expect("clap accepts only topic names").markdown.to_owned()
        }
        None => {
            let width = TOPICS.iter().map(|topic| topic.name.len()).max().unwrap_or(0);
            TOPICS
                .iter()
                .map(|topic| {
                    format!("{:width$}  {}\n", topic.name, topic.summary().expect("every topic has a summary"))
                })
                .collect()
        }
    };
    if std::io::stdout().lock().write_all(text.as_bytes()).is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
