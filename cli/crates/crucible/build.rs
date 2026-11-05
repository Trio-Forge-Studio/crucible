#[path = "src/docs/languages.rs"]
mod languages;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::collections::BTreeSet;
use std::path::Path;
use syntect::parsing::syntax_definition::{Context, ContextId, ContextReference, MatchOperation, Pattern};
use syntect::parsing::{SyntaxDefinition, SyntaxSet, SyntaxSetBuilder};

/// The reader picks the widest of these that fits the page, so the diagrams cost no Mermaid renderer in the binary.
const DIAGRAM_WIDTHS: [usize; 4] = [40, 60, 80, 100];

fn main() {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs");
    println!("cargo::rerun-if-changed={}", docs.display());
    let mut files: Vec<_> = std::fs::read_dir(&docs)
        .expect("the repository has a docs folder")
        .map(|entry| entry.expect("the docs folder is readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect();
    files.sort();
    let topics: String = files
        .iter()
        .map(|path| {
            let name = path.file_stem().and_then(|stem| stem.to_str()).expect("topic names are UTF-8");
            let path = path.to_str().expect("docs paths are UTF-8");
            format!("Topic {{ name: {name:?}, markdown: include_str!({path:?}) }},\n")
        })
        .collect();
    let mut diagrams = String::new();
    let mut languages = BTreeSet::new();
    for path in &files {
        let markdown = std::fs::read_to_string(path).expect("docs files are UTF-8");
        for (language, source) in fences(&markdown) {
            if language == "mermaid" {
                let renders: Vec<String> = DIAGRAM_WIDTHS
                    .iter()
                    .map(|&width| match mermaid_text::render_with_width(&source, Some(width)) {
                        Ok(diagram) => format!("{diagram:?}"),
                        Err(error) => panic!("{} has a mermaid block that does not render: {error}", path.display()),
                    })
                    .collect();
                diagrams += &format!("({source:?}, &[{}]),\n", renders.join(", "));
            }
            languages.insert(language);
        }
    }
    let out = Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).to_owned();
    std::fs::write(out.join("topics.rs"), format!("[\n{topics}]\n")).expect("OUT_DIR is writable");
    std::fs::write(out.join("diagrams.rs"), format!("[\n{diagrams}]\n")).expect("OUT_DIR is writable");
    let syntaxes = syntax_subset(&languages);
    std::fs::write(out.join("syntaxes.packdump"), syntect::dumps::dump_binary(&syntaxes)).expect("OUT_DIR is writable");
    let themes = two_face::theme::extra();
    let theme = themes.get(two_face::theme::EmbeddedThemeName::Ansi);
    std::fs::write(out.join("theme.themedump"), syntect::dumps::dump_binary(theme)).expect("OUT_DIR is writable");
}

/// two-face's full set is about a megabyte, so the binary keeps only the syntaxes the docs highlight and those they
/// include. Linked contexts point at syntaxes by index, so every syntax keeps its place and the rest become empty
/// stubs that no token finds.
fn syntax_subset(languages: &BTreeSet<String>) -> SyntaxSet {
    let full = two_face::syntax::extra_newlines();
    let index = |name: &str| {
        full.syntaxes().iter().position(|syntax| syntax.name == name).expect("a found syntax is in its set")
    };
    let mut needed: Vec<usize> = std::iter::once(full.find_syntax_plain_text())
        .chain(
            languages
                .iter()
                .filter_map(|language| languages::syntax_token(language))
                .filter_map(|token| full.find_syntax_by_token(token)),
        )
        .map(|syntax| index(&syntax.name))
        .collect();
    let definitions = full.into_builder().syntaxes().to_vec();
    let mut kept = vec![false; definitions.len()];
    while let Some(index) = needed.pop() {
        if !std::mem::replace(&mut kept[index], true) {
            needed.extend(definitions[index].contexts.values().flat_map(referenced_syntaxes));
        }
    }
    let mut builder = SyntaxSetBuilder::new();
    for (definition, kept) in definitions.into_iter().zip(kept) {
        builder.add(if kept { definition } else { stub(definition) });
    }
    builder.build()
}

fn stub(definition: SyntaxDefinition) -> SyntaxDefinition {
    SyntaxDefinition {
        name: String::new(),
        file_extensions: Vec::new(),
        first_line_match: None,
        hidden: true,
        variables: Default::default(),
        contexts: Default::default(),
        ..definition
    }
}

fn referenced_syntaxes(context: &Context) -> Vec<usize> {
    let mut references: Vec<&ContextReference> = Vec::new();
    for pattern in &context.patterns {
        match pattern {
            Pattern::Include(reference) => references.push(reference),
            Pattern::Match(pattern) => {
                if let MatchOperation::Push(pushed) | MatchOperation::Set(pushed) = &pattern.operation {
                    references.extend(pushed);
                }
                references.extend(&pattern.with_prototype);
            }
        }
    }
    let direct = references.into_iter().filter_map(|reference| match reference {
        ContextReference::Direct { 0: id, .. } => Some(*id),
        _ => None,
    });
    direct.chain(context.prototype).map(syntax_index).collect()
}

/// `ContextId` keeps its fields private, and its serialized form is the only public way to read them.
fn syntax_index(id: ContextId) -> usize {
    let id = serde_json::to_value(id).expect("a context id serializes");
    id["syntax_index"].as_u64().expect("a context id names its syntax") as usize
}

fn fences(markdown: &str) -> Vec<(String, String)> {
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
