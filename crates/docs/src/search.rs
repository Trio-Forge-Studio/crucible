use super::Slugger;
use super::Topic;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::cmp::Reverse;

pub struct Index {
    sections: Vec<Section>,
}

struct Section {
    topic: &'static str,
    path: Vec<String>,
    slug: Option<String>,
    body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub topic: &'static str,
    pub title: String,
    pub slug: Option<String>,
    pub snippet: String,
}

/// Variant order is rank order: any title match beats any body match.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Body(usize),
    Fuzzy(u32),
    Words(Reverse<usize>),
    Heading,
}

const SNIPPET: usize = 72;

impl Index {
    pub fn new(topics: &[Topic]) -> Self {
        Index { sections: topics.iter().flat_map(sections).collect() }
    }

    pub fn search(&self, query: &str) -> Vec<Hit> {
        let query = query.trim();
        if query.is_empty() {
            return self.sections.iter().filter(|section| section.slug.is_none()).map(|s| s.hit(None)).collect();
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let words: Vec<String> = query.split_whitespace().map(str::to_ascii_lowercase).collect();
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buffer = Vec::new();
        let mut ranked: Vec<(Rank, usize, Hit)> = self
            .sections
            .iter()
            .enumerate()
            .filter_map(|(order, section)| {
                let title = match section.slug {
                    Some(_) => section.path.join(" "),
                    None => format!("{} {}", section.topic, section.path.join(" ")),
                };
                let lower = title.to_ascii_lowercase();
                let rank = if section.path.last().is_some_and(|heading| heading.eq_ignore_ascii_case(query)) {
                    Rank::Heading
                } else if words.iter().all(|word| lower.contains(word.as_str())) {
                    Rank::Words(Reverse(title.len()))
                } else if let Some(score) = pattern.score(Utf32Str::new(&title, &mut buffer), &mut matcher) {
                    Rank::Fuzzy(score)
                } else {
                    let body = section.body.to_ascii_lowercase();
                    let at = body.find(words[0].as_str())?;
                    if !words.iter().all(|word| body.contains(word.as_str())) {
                        return None;
                    }
                    let count = words.iter().map(|word| body.matches(word.as_str()).count()).sum();
                    return Some((Rank::Body(count), order, section.hit(Some(at))));
                };
                Some((rank, order, section.hit(None)))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranked.into_iter().map(|(_, _, hit)| hit).collect()
    }
}

impl Section {
    fn hit(&self, at: Option<usize>) -> Hit {
        Hit {
            topic: self.topic,
            title: self.path.join(" › "),
            slug: self.slug.clone(),
            snippet: snippet(&self.body, at.unwrap_or(0)),
        }
    }
}

fn snippet(body: &str, at: usize) -> String {
    let mut start = at.saturating_sub(SNIPPET / 3);
    while !body.is_char_boundary(start) {
        start -= 1;
    }
    if start > 0 {
        start = body[start..at].find(' ').map_or(start, |space| start + space + 1);
    }
    let text: String = body[start..].chars().take(SNIPPET).collect();
    let more = body[start..].chars().nth(SNIPPET).is_some();
    format!("{}{}{}", if start > 0 { "…" } else { "" }, text.trim_end(), if more { "…" } else { "" })
}

fn sections(topic: &Topic) -> Vec<Section> {
    let mut slugs = Slugger::default();
    let mut sections = vec![Section { topic: topic.name, path: Vec::new(), slug: None, body: String::new() }];
    let mut stack: Vec<(u8, String)> = Vec::new();
    let mut heading: Option<(u8, String)> = None;
    for event in Parser::new_ext(topic.markdown, pulldown_cmark::Options::ENABLE_TABLES) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => heading = Some((level as u8, String::new())),
            Event::End(TagEnd::Heading(_)) => {
                let (level, text) = heading.take().expect("a heading is open");
                let slug = slugs.slug(&text);
                if level == 1 {
                    sections[0].path = vec![text];
                    continue;
                }
                stack.retain(|(open, _)| *open < level);
                stack.push((level, text));
                let path = stack.iter().map(|(_, text)| text.clone()).collect();
                sections.push(Section { topic: topic.name, path, slug: Some(slug), body: String::new() });
            }
            Event::Text(text) | Event::Code(text) => match &mut heading {
                Some((_, heading)) => heading.push_str(&text),
                None => sections.last_mut().expect("sections start non-empty").body.push_str(&text),
            },
            Event::SoftBreak | Event::HardBreak | Event::End(_) => match &mut heading {
                Some((_, heading)) if matches!(event, Event::SoftBreak | Event::HardBreak) => heading.push(' '),
                Some(_) => {}
                None if is_inline(&event) => {}
                None => sections.last_mut().expect("sections start non-empty").body.push(' '),
            },
            _ => {}
        }
    }
    if sections[0].path.is_empty() {
        sections[0].path = vec![topic.name.to_owned()];
    }
    for section in &mut sections {
        section.body = section.body.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    sections
}

fn is_inline(event: &Event) -> bool {
    matches!(
        event,
        Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> Index {
        Index::new(crate::TOPICS)
    }

    #[test]
    fn an_empty_query_lists_every_topic() {
        let hits = index().search("");
        let topics: Vec<_> = hits.iter().map(|hit| hit.topic).collect();
        let expected: Vec<_> = crate::TOPICS.iter().map(|topic| topic.name).collect();
        assert_eq!(topics, expected);
        assert!(hits.iter().all(|hit| hit.slug.is_none()));
    }

    #[test]
    fn a_heading_query_ranks_that_heading_first() {
        let hits = index().search("exit codes");
        assert_eq!((hits[0].topic, hits[0].slug.as_deref()), ("oc", Some("exit-codes")), "{:#?}", &hits[..3]);
        assert_eq!(hits[0].title, "Exit codes");
    }

    #[test]
    fn a_body_word_finds_its_section_with_a_snippet() {
        let topics = [Topic {
            name: "t",
            markdown: "# T\n\nIntro.\n\n## Frames\n\nThe fixed rate is sixty steps.\n\n## Other\n\nNothing.\n",
        }];
        let hits = Index::new(&topics).search("sixty");
        assert_eq!(hits.len(), 1, "{hits:#?}");
        assert_eq!(hits[0].slug.as_deref(), Some("frames"));
        assert!(hits[0].snippet.contains("sixty steps"), "{}", hits[0].snippet);
    }

    #[test]
    fn titles_rank_above_body_text() {
        let topics =
            [Topic { name: "t", markdown: "# T\n\nIntro.\n\n## Notes\n\nSee frames.\n\n## Frames\n\nText.\n" }];
        let hits = Index::new(&topics).search("frames");
        let slugs: Vec<_> = hits.iter().map(|hit| hit.slug.as_deref()).collect();
        assert_eq!(slugs, [Some("frames"), Some("notes")]);
    }

    #[test]
    fn nested_headings_carry_their_path() {
        let hits = index().search("Construction and the first batch");
        assert_eq!(hits[0].topic, "lifecycle");
        assert_eq!(hits[0].title, "Order › Construction and the first batch");
        assert_eq!(hits[0].slug.as_deref(), Some("construction-and-the-first-batch"));
    }
}
