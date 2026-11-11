pub mod search;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;

pub struct Topic {
    pub name: &'static str,
    pub markdown: &'static str,
}

macro_rules! topics {
    ($($file:literal),* $(,)?) => {
        &[$(Topic { name: stem($file), markdown: include_str!(concat!("../../../docs/", $file)) }),*]
    };
}

const fn stem(file: &'static str) -> &'static str {
    let Some((stem, b".md")) = file.as_bytes().split_last_chunk::<3>() else { panic!("a topic is a .md file") };
    match std::str::from_utf8(stem) {
        Ok(stem) => stem,
        Err(_) => panic!("a topic name is UTF-8"),
    }
}

/// A build script would need `std::env` to find `OUT_DIR`, which only the CLI may read, so the topics are listed here
/// and a test fails when the list and the `docs` folder differ.
pub static TOPICS: &[Topic] = topics![
    "boot.md",
    "cli.md",
    "configs.md",
    "data-stores.md",
    "diagnostics.md",
    "experience.md",
    "lifecycle.md",
    "logging.md",
    "networking.md",
    "oc.md",
    "profile-stores.md",
    "random.md",
    "reactive.md",
    "replicated.md",
    "sample-game.md",
    "spline.md",
    "testing.md",
    "ui.md",
    "vide.md",
];

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

/// The CLI's build script draws each Mermaid fence at these widths, and the reader picks the widest that fits the page.
pub const DIAGRAM_WIDTHS: [usize; 4] = [40, 60, 80, 100];

pub fn fences(markdown: &str) -> Vec<(String, String)> {
    let mut fences = Vec::new();
    let mut open: Option<(String, String)> = None;
    for event in Parser::new_ext(markdown, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => open = Some((info.to_string(), String::new())),
            Event::Text(text) => {
                if let Some((_, source)) = open.as_mut() {
                    source.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => fences.extend(open.take()),
            _ => {}
        }
    }
    fences
}

/// GitHub's anchor rules: lowercase, drop punctuation, spaces become hyphens, and repeats get `-1`, `-2`.
#[derive(Default)]
pub struct Slugger(HashMap<String, usize>);

impl Slugger {
    pub fn slug(&mut self, text: &str) -> String {
        let base: String = text
            .trim()
            .to_lowercase()
            .chars()
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                _ => None,
            })
            .collect();
        let mut slug = base.clone();
        while self.0.contains_key(&slug) {
            let count = self.0.get_mut(&base).expect("a taken slug's base is taken");
            *count += 1;
            slug = format!("{base}-{count}");
        }
        self.0.insert(slug.clone(), 0);
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_docs_file_is_a_topic() {
        let mut files: Vec<String> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
            .map(|path| path.file_stem().unwrap().to_str().unwrap().to_owned())
            .collect();
        files.sort();
        let topics: Vec<_> = TOPICS.iter().map(|topic| topic.name).collect();
        assert_eq!(topics, files, "TOPICS in crates/docs/src/lib.rs lists each docs/*.md file in byte order");
    }

    /// The CLI build embeds a fence that does not render as its source, so this test is what fails on it.
    #[test]
    fn every_mermaid_fence_renders_at_every_width() {
        let mut drawn = 0;
        for topic in TOPICS {
            for (_, source) in fences(topic.markdown).into_iter().filter(|(language, _)| language == "mermaid") {
                for width in DIAGRAM_WIDTHS {
                    if let Err(error) = mermaid_text::render_with_width(&source, Some(width)) {
                        panic!("docs/{}.md has a mermaid block that does not render at {width}: {error}", topic.name);
                    }
                }
                drawn += 1;
            }
        }
        assert!(drawn > 0, "the docs hold a mermaid diagram");
    }

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
