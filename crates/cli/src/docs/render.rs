use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub struct Page {
    pub lines: Vec<Line<'static>>,
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
}

pub struct Heading {
    pub level: u8,
    pub text: String,
    pub slug: String,
    pub line: usize,
}

pub struct Link {
    pub target: Target,
    pub spans: Vec<LinkSpan>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkSpan {
    pub line: usize,
    pub columns: Range<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Topic { name: String, anchor: Option<String> },
    Anchor(String),
    External(String),
}

impl Target {
    pub fn parse(href: &str) -> Self {
        if let Some(anchor) = href.strip_prefix('#') {
            return Target::Anchor(anchor.to_owned());
        }
        let (path, anchor) = match href.split_once('#') {
            Some((path, anchor)) => (path, Some(anchor.to_owned())),
            None => (href, None),
        };
        match path.strip_suffix(".md").filter(|_| !href.contains("://")) {
            Some(name) => Target::Topic { name: name.rsplit('/').next().unwrap_or(name).to_owned(), anchor },
            None => Target::External(href.to_owned()),
        }
    }
}

impl Page {
    pub fn heading(&self, slug: &str) -> Option<&Heading> {
        self.headings.iter().find(|heading| heading.slug == slug)
    }
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

static SYNTAXES: LazyLock<SyntaxSet> =
    LazyLock::new(|| syntect::dumps::from_binary(include_bytes!(concat!(env!("OUT_DIR"), "/syntaxes.packdump"))));
static THEME: LazyLock<Theme> =
    LazyLock::new(|| syntect::dumps::from_binary(include_bytes!(concat!(env!("OUT_DIR"), "/theme.themedump"))));

fn theme() -> &'static Theme {
    &THEME
}

const CODE: Style = Style::new().fg(Color::Yellow);
const LINK: Style = Style::new().fg(Color::LightBlue).add_modifier(Modifier::UNDERLINED);
const FRAME: Style = Style::new().fg(Color::DarkGray);
const LABEL: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::ITALIC);

pub fn render(markdown: &str, width: u16) -> Page {
    let mut builder = Builder { width: usize::from(width.max(20)), ..Builder::default() };
    for event in Parser::new_ext(markdown, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
        builder.event(event);
    }
    Page { lines: builder.lines, headings: builder.headings, links: builder.links }
}

#[derive(Clone)]
struct Piece {
    text: String,
    style: Style,
    link: Option<usize>,
    space: bool,
}

#[derive(Default)]
struct Row {
    spans: Vec<Span<'static>>,
    width: usize,
    links: Vec<(usize, Range<usize>)>,
}

impl Row {
    fn push(&mut self, text: impl Into<String>, style: Style, link: Option<usize>) {
        let text = text.into();
        let width = text.width();
        if let Some(link) = link {
            match self.links.last_mut() {
                Some((last, range)) if *last == link && range.end == self.width => range.end += width,
                _ => self.links.push((link, self.width..self.width + width)),
            }
        }
        self.width += width;
        self.spans.push(Span::styled(text, style));
    }

    fn append(&mut self, other: Row) {
        let offset = self.width;
        self.links
            .extend(other.links.into_iter().map(|(link, range)| (link, range.start + offset..range.end + offset)));
        self.width += other.width;
        self.spans.extend(other.spans);
    }

    fn pad(&mut self, width: usize, style: Style) {
        if width > self.width {
            self.push(" ".repeat(width - self.width), style, None);
        }
    }
}

enum Container {
    Quote,
    List { next: Option<u64> },
    Item { marker: String, used: bool },
}

struct Table {
    rows: Vec<Vec<Vec<Piece>>>,
    head: usize,
}

#[derive(Default)]
struct Builder {
    width: usize,
    lines: Vec<Line<'static>>,
    headings: Vec<Heading>,
    links: Vec<Link>,
    slugs: Slugger,
    inline: Vec<Piece>,
    space: bool,
    styles: Vec<Style>,
    link: Option<usize>,
    containers: Vec<Container>,
    blank: bool,
    code: Option<(String, String)>,
    table: Option<Table>,
}

impl Builder {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match &mut self.code {
                Some((_, code)) => code.push_str(&text),
                None => self.text(&text, self.style()),
            },
            Event::Code(code) => self.piece(code.into_string(), CODE),
            Event::InlineMath(text) | Event::DisplayMath(text) => self.piece(text.into_string(), CODE),
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html, FRAME),
            Event::SoftBreak => self.space = true,
            Event::HardBreak => self.piece("\n".into(), Style::new()),
            Event::Rule => {
                self.flush();
                self.open_block();
                let width = self.width - self.prefix_width();
                self.push_row(row(&"─".repeat(width), FRAME));
                self.blank = true;
            }
            Event::FootnoteReference(name) => self.piece(format!("[{name}]"), FRAME),
            Event::TaskListMarker(done) => self.piece(if done { "[x]" } else { "[ ]" }.into(), FRAME),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => self.flush(),
            Tag::Heading { .. } => self.flush(),
            Tag::BlockQuote(_) => {
                self.flush();
                self.containers.push(Container::Quote);
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(next) => {
                self.flush();
                self.containers.push(Container::List { next });
            }
            Tag::Item => {
                self.flush();
                let marker = match self.containers.last_mut() {
                    Some(Container::List { next: Some(number) }) => {
                        *number += 1;
                        format!("{}. ", *number - 1)
                    }
                    _ => "• ".to_owned(),
                };
                self.containers.push(Container::Item { marker, used: false });
            }
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Table { rows: Vec::new(), head: 0 });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.rows.push(Vec::new());
                }
            }
            Tag::TableCell => self.inline.clear(),
            Tag::Emphasis => self.styles.push(self.style().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.styles.push(self.style().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.styles.push(self.style().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.links.push(Link { target: Target::parse(&dest_url), spans: Vec::new() });
                self.link = Some(self.links.len() - 1);
                self.styles.push(self.style().patch(LINK));
            }
            Tag::Image { dest_url, .. } => self.piece(format!("[image {dest_url}]"), FRAME),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                self.blank = true;
            }
            TagEnd::Heading(level) => self.heading(level),
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.blank = true;
            }
            TagEnd::CodeBlock => {
                let (lang, code) = self.code.take().expect("a code block is open");
                self.code_block(&lang, &code);
                self.blank = true;
            }
            TagEnd::List(_) => {
                self.flush();
                self.containers.pop();
                if !self.containers.iter().any(|container| matches!(container, Container::Item { .. })) {
                    self.blank = true;
                }
            }
            TagEnd::Item => {
                self.flush();
                self.containers.pop();
            }
            TagEnd::Table => {
                let table = self.table.take().expect("a table is open");
                self.draw_table(table);
                self.blank = true;
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.head = table.rows.len();
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                self.space = false;
                if let Some(row) = self.table.as_mut().and_then(|table| table.rows.last_mut()) {
                    row.push(cell);
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                self.link = None;
            }
            _ => {}
        }
    }

    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn text(&mut self, text: &str, style: Style) {
        for (index, word) in text.split(char::is_whitespace).enumerate() {
            if index > 0 {
                self.space = true;
            }
            if !word.is_empty() {
                self.piece(word.to_owned(), style);
            }
        }
    }

    fn piece(&mut self, text: String, style: Style) {
        let space = std::mem::take(&mut self.space);
        self.inline.push(Piece { text, style: self.style().patch(style), link: self.link, space });
    }

    fn prefix(&mut self) -> Row {
        let mut row = Row::default();
        for container in &mut self.containers {
            match container {
                Container::Quote => row.push("▎ ", FRAME, None),
                Container::List { .. } => {}
                Container::Item { marker, used } => {
                    if *used {
                        row.push(" ".repeat(marker.width()), Style::new(), None);
                    } else {
                        row.push(marker.clone(), FRAME, None);
                        *used = true;
                    }
                }
            }
        }
        row
    }

    fn prefix_width(&self) -> usize {
        self.containers
            .iter()
            .map(|container| match container {
                Container::Quote => 2,
                Container::List { .. } => 0,
                Container::Item { marker, .. } => marker.width(),
            })
            .sum()
    }

    fn open_block(&mut self) {
        if std::mem::take(&mut self.blank) && !self.lines.is_empty() {
            let quotes = self.containers.iter().filter(|container| matches!(container, Container::Quote)).count();
            self.lines.push(Line::styled("▎ ".repeat(quotes), FRAME));
        }
    }

    fn push_row(&mut self, content: Row) {
        let mut row = self.prefix();
        row.append(content);
        let line = self.lines.len();
        for (link, columns) in row.links {
            self.links[link].spans.push(LinkSpan { line, columns: columns.start as u16..columns.end as u16 });
        }
        self.lines.push(Line::from(row.spans));
    }

    fn flush(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        self.open_block();
        let pieces = std::mem::take(&mut self.inline);
        self.space = false;
        let width = self.width.saturating_sub(self.prefix_width()).max(10);
        for row in wrap(&pieces, width) {
            self.push_row(row);
        }
    }

    fn heading(&mut self, level: HeadingLevel) {
        let pieces = std::mem::take(&mut self.inline);
        self.space = false;
        let text = pieces.iter().fold(String::new(), |mut text, piece| {
            if piece.space && !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&piece.text);
            text
        });
        let style = match level {
            HeadingLevel::H1 => Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            HeadingLevel::H2 => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            HeadingLevel::H3 => Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            _ => Style::new().add_modifier(Modifier::BOLD),
        };
        let pieces: Vec<_> =
            pieces.into_iter().map(|piece| Piece { style: piece.style.patch(style), ..piece }).collect();
        self.open_block();
        let line = self.lines.len();
        let width = self.width.saturating_sub(self.prefix_width()).max(10);
        let rows = wrap(&pieces, width);
        let widest = rows.iter().map(|row| row.width).max().unwrap_or(0);
        for row in rows {
            self.push_row(row);
        }
        if level == HeadingLevel::H1 {
            self.push_row(row(&"━".repeat(widest), style.remove_modifier(Modifier::BOLD)));
        }
        let slug = self.slugs.slug(&text);
        self.headings.push(Heading { level: level as u8, text, slug, line });
        self.blank = true;
    }

    fn code_block(&mut self, lang: &str, code: &str) {
        self.open_block();
        let avail = self.width.saturating_sub(self.prefix_width()).max(12);
        if lang == "mermaid"
            && let Some(diagram) = diagram(code, avail)
        {
            for line in diagram {
                self.push_row(row(line.trim_end(), Style::new().fg(Color::Cyan)));
            }
            return;
        }
        let rows: Vec<Row> = highlight(lang, code).into_iter().flat_map(|line| split(line, avail - 4)).collect();
        let label = if lang.is_empty() { String::new() } else { format!(" {lang} ") };
        let inner = rows.iter().map(|row| row.width).max().unwrap_or(0).max(label.width() + 2).min(avail - 4);
        let mut top = Row::default();
        top.push("╭─", FRAME, None);
        top.push(label.clone(), LABEL, None);
        top.push(format!("{}╮", "─".repeat((inner + 1).saturating_sub(label.width()))), FRAME, None);
        self.push_row(top);
        for content in rows {
            let mut line = Row::default();
            line.push("│ ", FRAME, None);
            line.append(content);
            line.pad(inner + 2, Style::new());
            line.push(" │", FRAME, None);
            self.push_row(line);
        }
        self.push_row(row(&format!("╰{}╯", "─".repeat(inner + 2)), FRAME));
    }

    fn draw_table(&mut self, table: Table) {
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        self.open_block();
        let avail = self.width.saturating_sub(self.prefix_width()).saturating_sub(3 * columns + 1);
        let mut widths = vec![1; columns];
        for row in &table.rows {
            for (column, cell) in row.iter().enumerate() {
                widths[column] = widths[column].max(wrap(cell, usize::MAX).first().map_or(0, |row| row.width));
            }
        }
        while widths.iter().sum::<usize>() > avail {
            let widest = (0..columns).max_by_key(|&column| widths[column]).expect("a table has columns");
            if widths[widest] <= 3 {
                break;
            }
            widths[widest] -= 1;
        }
        let border = |left: &str, middle: &str, right: &str| {
            let cells: Vec<_> = widths.iter().map(|width| "─".repeat(width + 2)).collect();
            row(&format!("{left}{}{right}", cells.join(middle)), FRAME)
        };
        self.push_row(border("┌", "┬", "┐"));
        for (index, cells) in table.rows.iter().enumerate() {
            if index == table.head && index > 0 {
                self.push_row(border("├", "┼", "┤"));
            }
            let bold = if index < table.head { Modifier::BOLD } else { Modifier::empty() };
            let wrapped: Vec<Vec<Row>> = (0..columns)
                .map(|column| {
                    let cell = cells.get(column).map(Vec::as_slice).unwrap_or_default();
                    let cell: Vec<_> =
                        cell.iter().map(|p| Piece { style: p.style.add_modifier(bold), ..p.clone() }).collect();
                    wrap(&cell, widths[column])
                })
                .collect();
            let height = wrapped.iter().map(Vec::len).max().unwrap_or(0).max(1);
            for line in 0..height {
                let mut out = Row::default();
                for (column, cell) in wrapped.iter().enumerate() {
                    out.push(if column == 0 { "│ " } else { " │ " }, FRAME, None);
                    let start = out.width;
                    if let Some(content) = cell.get(line) {
                        out.append(Row {
                            spans: content.spans.clone(),
                            width: content.width,
                            links: content.links.clone(),
                        });
                    }
                    out.pad(start + widths[column], Style::new());
                }
                out.push(" │", FRAME, None);
                self.push_row(out);
            }
        }
        self.push_row(border("└", "┴", "┘"));
    }
}

fn row(text: &str, style: Style) -> Row {
    let mut row = Row::default();
    row.push(text, style, None);
    row
}

fn wrap(pieces: &[Piece], width: usize) -> Vec<Row> {
    let mut rows = vec![Row::default()];
    let mut words: Vec<&[Piece]> = Vec::new();
    let mut start = 0;
    for index in 1..=pieces.len() {
        if index == pieces.len() || pieces[index].space || pieces[index].text == "\n" || pieces[index - 1].text == "\n"
        {
            words.push(&pieces[start..index]);
            start = index;
        }
    }
    for word in words {
        if word[0].text == "\n" {
            rows.push(Row::default());
            continue;
        }
        let word_width: usize = word.iter().map(|piece| piece.text.width()).sum();
        let current = rows.last_mut().expect("rows start non-empty");
        let gap = usize::from(current.width > 0);
        if current.width > 0 && current.width + gap + word_width > width {
            rows.push(Row::default());
        } else if gap == 1 {
            let link = word[0].link.filter(|link| current.links.last().is_some_and(|(last, _)| last == link));
            current.push(" ", if link.is_some() { word[0].style } else { Style::new() }, link);
        }
        for piece in word {
            push_split(&mut rows, &piece.text, piece.style, piece.link, width);
        }
    }
    rows.retain(|row| row.width > 0 || !row.spans.is_empty());
    rows
}

fn highlight(lang: &str, code: &str) -> Vec<Row> {
    let syntax = super::languages::syntax_token(lang).and_then(|token| SYNTAXES.find_syntax_by_token(token));
    let code = code.replace('\t', "    ");
    let Some(syntax) = syntax else {
        return code.lines().map(|line| row(line, Style::new())).collect();
    };
    let mut highlighter = HighlightLines::new(syntax, theme());
    LinesWithEndings::from(&code)
        .map(|line| {
            let mut out = Row::default();
            let regions =
                highlighter.highlight_line(line, &SYNTAXES).unwrap_or_else(|_| vec![(Default::default(), line)]);
            for (style, text) in regions {
                let text = text.trim_end_matches(['\n', '\r']);
                if !text.is_empty() {
                    out.push(text, convert(style), None);
                }
            }
            out
        })
        .collect()
}

/// The ANSI theme stores a palette index in `r` with alpha 0, and "terminal default" as alpha 1,
/// so code follows the terminal's own colours on light and dark backgrounds.
fn convert(style: syntect::highlighting::Style) -> Style {
    let color = style.foreground;
    let mut out = match color.a {
        0 => Style::new().fg(match color.r {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            n => Color::Indexed(n),
        }),
        1 => Style::new(),
        _ => Style::new().fg(Color::Rgb(color.r, color.g, color.b)),
    };
    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

fn split(line: Row, width: usize) -> Vec<Row> {
    if line.width <= width {
        return vec![line];
    }
    let mut rows = vec![Row::default()];
    for span in line.spans {
        push_split(&mut rows, &span.content, span.style, None, width);
    }
    rows
}

fn push_split(rows: &mut Vec<Row>, text: &str, style: Style, link: Option<usize>, width: usize) {
    let mut chunk = String::new();
    let mut chunk_width = 0;
    for c in text.chars() {
        let c_width = c.width().unwrap_or(0);
        let current = rows.last_mut().expect("rows start non-empty");
        if current.width + chunk_width + c_width > width && current.width + chunk_width > 0 {
            current.push(std::mem::take(&mut chunk), style, link);
            chunk_width = 0;
            rows.push(Row::default());
        }
        chunk.push(c);
        chunk_width += c_width;
    }
    if !chunk.is_empty() {
        rows.last_mut().expect("rows start non-empty").push(chunk, style, link);
    }
}

/// Each Mermaid fence in `docs`, drawn by `build.rs` at a few widths, narrowest first.
static DIAGRAMS: &[(&str, &[&str])] = &include!(concat!(env!("OUT_DIR"), "/diagrams.rs"));

fn diagram(source: &str, width: usize) -> Option<Vec<&'static str>> {
    let (_, drawings) = DIAGRAMS.iter().find(|(known, _)| *known == source)?;
    let drawings: Vec<Vec<&str>> = drawings.iter().map(|drawing| compact(drawing)).collect();
    let fits = |drawing: &&Vec<&str>| drawing.iter().all(|line| line.trim_end().width() <= width);
    drawings.iter().rfind(fits).or(drawings.first()).cloned()
}

/// mermaid-text pads every edge with many identical rows of bare `│` and centres the drawing in spaces.
/// One row of each run and no shared indent keep the drawing intact in fewer cells.
fn compact(diagram: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = Vec::new();
    for line in diagram.lines() {
        let passthrough = line.chars().all(|c| c == ' ' || c == '│');
        if passthrough && lines.last() == Some(&line) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start_matches(' ').len())
        .min()
        .unwrap_or(0);
    lines.iter().map(|line| line.get(indent..).unwrap_or("")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn page_text(page: &Page) -> String {
        page.lines.iter().map(|line| text(line) + "\n").collect()
    }

    #[test]
    fn heading_slugs_follow_github_anchors() {
        let page = render("# Open Cloud\n\n## Exit codes\n\n## `crucible docs`\n\n## Exit codes\n", 80);
        let slugs: Vec<_> = page.headings.iter().map(|heading| heading.slug.as_str()).collect();
        assert_eq!(slugs, ["open-cloud", "exit-codes", "crucible-docs", "exit-codes-1"]);
        assert_eq!(text(&page.lines[page.heading("exit-codes").unwrap().line]), "Exit codes");
    }

    #[test]
    fn a_heading_never_takes_an_anchor_a_repeat_already_took() {
        let page = render("# Foo\n\n## Foo\n\n## Foo-1\n\n## Foo\n", 80);
        let slugs: Vec<_> = page.headings.iter().map(|heading| heading.slug.as_str()).collect();
        assert_eq!(slugs, ["foo", "foo-1", "foo-1-1", "foo-2"]);
    }

    #[test]
    fn a_typescript_fence_is_highlighted_in_a_labelled_frame() {
        let page = render("```ts\nconst answer: number = 42;\n```\n", 80);
        assert!(text(&page.lines[0]).starts_with("╭─ ts "), "{}", page_text(&page));
        assert!(text(&page.lines[1]).contains("const answer: number = 42;"));
        assert!(text(page.lines.last().unwrap()).starts_with("╰"));
        let styles: std::collections::HashSet<_> =
            page.lines[1].spans.iter().filter(|span| !span.content.trim().is_empty()).map(|span| span.style).collect();
        assert!(styles.len() > 1, "{styles:?}");
    }

    #[test]
    fn a_text_fence_stays_plain() {
        let page = render("```text\nplain words here\n```\n", 80);
        let styles: std::collections::HashSet<_> =
            page.lines[1].spans.iter().filter(|span| span.content.contains("plain")).map(|span| span.style).collect();
        assert_eq!(styles.into_iter().collect::<Vec<_>>(), [Style::new()]);
    }

    #[test]
    fn a_table_is_drawn_with_box_characters() {
        let page = render("| Hook | Runs on |\n| --- | --- |\n| `update` | PostSimulation |\n", 80);
        let lines: Vec<_> = page.lines.iter().map(text).collect();
        assert_eq!(
            lines,
            [
                "┌────────┬────────────────┐",
                "│ Hook   │ Runs on        │",
                "├────────┼────────────────┤",
                "│ update │ PostSimulation │",
                "└────────┴────────────────┘",
            ]
        );
    }

    #[test]
    fn a_narrow_table_wraps_its_cells_inside_the_width() {
        let page = render("| A | B |\n| - | - |\n| one two three four five six | seven eight nine ten |\n", 30);
        assert!(page.lines.iter().all(|line| line.width() <= 30), "{}", page_text(&page));
        assert!(page_text(&page).contains("five"));
    }

    #[test]
    fn a_link_records_its_columns_and_target() {
        let page = render("See [exit codes](oc.md#exit-codes) and [the site](https://example.com).\n", 80);
        let line = text(&page.lines[0]);
        let link = &page.links[0];
        assert_eq!(link.target, Target::Topic { name: "oc".into(), anchor: Some("exit-codes".into()) });
        assert_eq!(link.spans.len(), 1);
        let span = &link.spans[0];
        assert_eq!(&line[span.columns.start as usize..span.columns.end as usize], "exit codes");
        assert_eq!(page.links[1].target, Target::External("https://example.com".into()));
    }

    #[test]
    fn a_wrapped_link_is_one_link_on_two_lines() {
        let page = render("Words before [a link that wraps](#here) after.\n", 24);
        assert_eq!(page.links.len(), 1);
        let lines: Vec<_> = page.links[0].spans.iter().map(|span| span.line).collect();
        assert_eq!(lines, [0, 1], "{}", page_text(&page));
        assert_eq!(page.links[0].target, Target::Anchor("here".into()));
    }

    #[test]
    fn link_targets_parse_once() {
        assert_eq!(Target::parse("oc.md"), Target::Topic { name: "oc".into(), anchor: None });
        assert_eq!(
            Target::parse("./cli.md#crucible-docs"),
            Target::Topic { name: "cli".into(), anchor: Some("crucible-docs".into()) }
        );
        assert_eq!(Target::parse("#order"), Target::Anchor("order".into()));
        assert_eq!(Target::parse("https://x.dev/a.md"), Target::External("https://x.dev/a.md".into()));
    }

    fn mermaid_fences() -> Vec<(&'static str, String)> {
        let mut fences = Vec::new();
        for topic in crate::docs::TOPICS {
            let mut fence = None;
            for event in Parser::new(topic.markdown) {
                match event {
                    Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if info.as_ref() == "mermaid" => {
                        fence = Some(String::new());
                    }
                    Event::Text(text) => fence.iter_mut().for_each(|fence| fence.push_str(&text)),
                    Event::End(TagEnd::CodeBlock) => fences.extend(fence.take().map(|fence| (topic.name, fence))),
                    _ => {}
                }
            }
        }
        fences
    }

    #[test]
    fn a_mermaid_fence_becomes_a_diagram_that_fits() {
        let (_, source) = mermaid_fences().into_iter().find(|(topic, _)| *topic == "lifecycle").unwrap();
        for width in [40, 80, 120] {
            let page = render(&format!("```mermaid\n{source}```\n"), width);
            let drawing = page_text(&page);
            assert!(drawing.contains('┌') && drawing.contains('▾'), "{drawing}");
            assert!(drawing.contains("Every start") && !drawing.contains("-->"), "{drawing}");
            assert!(page.lines.iter().all(|line| line.width() <= usize::from(width)), "{width}:\n{drawing}");
            let gaps = drawing.lines().collect::<Vec<_>>().windows(2).filter(|pair| pair[0] == pair[1]).count();
            assert_eq!(gaps, 0, "{drawing}");
        }
    }

    #[test]
    fn a_mermaid_fence_outside_the_docs_shows_its_source() {
        let page = render("```mermaid\nflowchart TD\n  A --> B\n```\n", 80);
        assert!(page_text(&page).contains("A --> B"));
        assert!(text(&page.lines[0]).starts_with("╭─ mermaid "));
    }

    #[test]
    fn every_mermaid_fence_in_the_docs_has_a_drawing() {
        let fences = mermaid_fences();
        assert!(!fences.is_empty(), "the docs hold a mermaid diagram");
        for (topic, source) in fences {
            assert!(diagram(&source, 80).is_some(), "build.rs did not draw the mermaid block in docs/{topic}.md");
        }
    }

    #[test]
    fn the_bundled_syntaxes_parse_every_docs_fence_like_the_full_set() {
        use syntect::parsing::ParseState;
        let full = two_face::syntax::extra_newlines();
        let mut checked = 0;
        for topic in crate::docs::TOPICS {
            let mut fence: Option<(String, String)> = None;
            for event in Parser::new(topic.markdown) {
                match event {
                    Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                        fence = Some((info.to_string(), String::new()));
                    }
                    Event::Text(text) => fence.iter_mut().for_each(|(_, code)| code.push_str(&text)),
                    Event::End(TagEnd::CodeBlock) => {
                        let Some((language, code)) = fence.take() else { continue };
                        let Some(token) = super::super::languages::syntax_token(&language) else { continue };
                        let expected = full.find_syntax_by_token(token);
                        let bundled = SYNTAXES.find_syntax_by_token(token);
                        assert_eq!(
                            bundled.map(|syntax| &syntax.name),
                            expected.map(|syntax| &syntax.name),
                            "docs/{}.md {language}",
                            topic.name
                        );
                        let (Some(expected), Some(bundled)) = (expected, bundled) else { continue };
                        let (mut want, mut got) = (ParseState::new(expected), ParseState::new(bundled));
                        for line in LinesWithEndings::from(&code) {
                            assert_eq!(
                                got.parse_line(line, &SYNTAXES).unwrap(),
                                want.parse_line(line, &full).unwrap(),
                                "docs/{}.md {language}: {line}",
                                topic.name
                            );
                        }
                        checked += 1;
                    }
                    _ => {}
                }
            }
        }
        assert!(checked > 100, "only {checked} fences were highlighted");
    }

    #[test]
    fn lists_nest_with_markers_and_hanging_indent() {
        let page = render("1. First step that is long enough to wrap\n   - nested\n2. Second\n", 24);
        let lines: Vec<_> = page.lines.iter().map(text).collect();
        assert_eq!(lines, ["1. First step that is", "   long enough to wrap", "   • nested", "2. Second"]);
    }

    #[test]
    fn every_topic_renders_at_every_width() {
        for topic in crate::docs::TOPICS {
            for width in [40, 80, 160] {
                let page = render(topic.markdown, width);
                assert!(!page.lines.is_empty(), "{}", topic.name);
                assert_eq!(page.headings[0].level, 1, "{}", topic.name);
                let widest = page.lines.iter().max_by_key(|line| line.width()).unwrap();
                assert!(widest.width() <= usize::from(width), "docs/{}.md at {width}: {}", topic.name, text(widest));
            }
        }
    }
}
