use std::io::Write;
use std::process::ExitCode;

pub struct Topic {
    pub name: &'static str,
    pub markdown: &'static str,
}

pub static TOPICS: &[Topic] = &include!(concat!(env!("OUT_DIR"), "/topics.rs"));

impl Topic {
    pub fn summary(&self) -> Option<&'static str> {
        let mut lines = self.markdown.lines();
        lines.next()?.strip_prefix("# ")?;
        if !lines.next()?.is_empty() {
            return None;
        }
        let summary = lines.next()?;
        (summary.ends_with('.') && lines.next().is_none_or(str::is_empty)).then_some(summary)
    }
}

pub fn run(topic: Option<&str>) -> ExitCode {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_has_a_summary_line() {
        assert!(!TOPICS.is_empty(), "docs are embedded");
        for topic in TOPICS {
            assert!(
                topic.summary().is_some(),
                "docs/{}.md needs `# Title`, a blank line, then a one-line summary ending in a period",
                topic.name
            );
        }
    }

    #[test]
    fn a_summary_is_one_sentence_line_under_the_title() {
        let summary = |markdown| Topic { name: "t", markdown }.summary();
        assert_eq!(summary("# T\n\nPrints things.\n\n## More\n"), Some("Prints things."));
        assert_eq!(summary("# T\n\nPrints things."), Some("Prints things."));
        assert_eq!(summary("T\n\nPrints things.\n"), None, "title line");
        assert_eq!(summary("# T\nPrints things.\n"), None, "blank line after the title");
        assert_eq!(summary("# T\n\n```console\n"), None, "a sentence");
        assert_eq!(summary("# T\n\nPrints\nthings.\n"), None, "one line");
        assert_eq!(summary("# T\n\nPrints things.\nMore.\n"), None, "one line paragraph");
    }
}
