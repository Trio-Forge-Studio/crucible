use super::TOPICS;
use super::render::{Page, Target, render};
use super::search::{Hit, Index};
use ratatui::Frame;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;

const SIDEBAR: u16 = 22;
const SIDEBAR_FROM: u16 = 90;
const MAX_WIDTH: u16 = 100;
const WHEEL: isize = 3;
const DIM: Style = Style::new().fg(Color::DarkGray);
const HOME: &str = "index";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    pub topic: usize,
    pub scroll: usize,
}

pub enum Overlay {
    None,
    Palette { query: String, selected: usize, hits: Vec<Hit> },
    Help,
}

#[derive(Debug, PartialEq)]
pub enum Effect {
    None,
    Quit,
    Open(String),
}

struct Doc {
    name: &'static str,
    markdown: Cow<'static, str>,
}

pub struct Reader {
    docs: Vec<Doc>,
    index: Index,
    pages: HashMap<(usize, u16), Rc<Page>>,
    area: Rect,
    pub location: Location,
    back: Vec<Location>,
    forward: Vec<Location>,
    pub focused_link: Option<usize>,
    pub overlay: Overlay,
    pub notice: Option<String>,
}

struct Areas {
    sidebar: Option<Rect>,
    content: Rect,
    status: Rect,
}

fn areas(area: Rect) -> Areas {
    let body = Rect { height: area.height.saturating_sub(1), ..area };
    let status = Rect { y: area.bottom().saturating_sub(1), height: area.height.min(1), ..area };
    let sidebar = (area.width >= SIDEBAR_FROM).then_some(Rect { width: SIDEBAR, ..body });
    let left = sidebar.map_or(0, |sidebar| sidebar.width);
    let content = Rect {
        x: body.x + left + 2,
        y: body.y + 1,
        width: (body.width.saturating_sub(left + 4)).min(MAX_WIDTH),
        height: body.height.saturating_sub(1),
    };
    Areas { sidebar, content, status }
}

fn sidebar_rows(sidebar: Rect) -> Rect {
    Rect { y: sidebar.y + 3, height: sidebar.height.saturating_sub(3), width: sidebar.width - 1, ..sidebar }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(2));
    Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 4, width, height }
}

fn palette_area(area: Rect) -> Rect {
    centered(area, 76, 24)
}

fn help_area(area: Rect) -> Rect {
    centered(area, 56, HELP.len() as u16 + 4)
}

const HELP: &[(&str, &str)] = &[
    ("↑ ↓  j k", "Scroll a line"),
    ("PgUp PgDn Space", "Scroll a page"),
    ("g G  Home End", "Top or bottom"),
    ("Tab  Shift+Tab", "Focus the next or previous link"),
    ("Enter  click", "Follow the link"),
    ("Backspace  Alt+←", "Back"),
    ("Alt+→", "Forward"),
    ("Ctrl+K  /", "Search"),
    ("click a topic", "Open it"),
    ("?", "Toggle this help"),
    ("q  Esc  Ctrl+C", "Quit"),
];

impl Reader {
    pub fn new(topic: Option<&str>, area: Rect) -> Self {
        let mut home = String::from(
            "# Crucible docs\n\nThe Crucible guides. Press Ctrl+K to search them, or open a topic.\n\n| Topic | Summary |\n| --- | --- |\n",
        );
        for topic in TOPICS {
            let summary = topic.summary().unwrap_or_default().replace('|', "\\|");
            home += &format!("| [{0}]({0}.md) | {summary} |\n", topic.name);
        }
        let docs: Vec<Doc> = std::iter::once(Doc { name: HOME, markdown: Cow::Owned(home) })
            .chain(TOPICS.iter().map(|topic| Doc { name: topic.name, markdown: Cow::Borrowed(topic.markdown) }))
            .collect();
        let topic = topic.and_then(|name| docs.iter().position(|doc| doc.name == name)).unwrap_or(0);
        Reader {
            docs,
            index: Index::new(TOPICS),
            pages: HashMap::new(),
            area,
            location: Location { topic, scroll: 0 },
            back: Vec::new(),
            forward: Vec::new(),
            focused_link: None,
            overlay: Overlay::None,
            notice: None,
        }
    }

    pub fn topic(&self) -> &'static str {
        self.docs[self.location.topic].name
    }

    pub fn page(&mut self) -> Rc<Page> {
        self.page_of(self.location.topic)
    }

    fn page_of(&mut self, topic: usize) -> Rc<Page> {
        let width = areas(self.area).content.width;
        let markdown = &self.docs[topic].markdown;
        self.pages.entry((topic, width)).or_insert_with(|| Rc::new(render(markdown, width))).clone()
    }

    fn height(&self) -> usize {
        usize::from(areas(self.area).content.height).max(1)
    }

    fn max_scroll(&mut self) -> usize {
        self.page().lines.len().saturating_sub(self.height())
    }

    pub fn update(&mut self, event: Event) -> Effect {
        match event {
            Event::Resize(width, height) => {
                self.area = Rect::new(0, 0, width, height);
                self.focused_link = None;
                self.scroll_to(self.location.scroll);
                Effect::None
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Mouse(mouse) => self.mouse(mouse),
            _ => Effect::None,
        }
    }

    fn key(&mut self, key: KeyEvent) -> Effect {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ctrl && key.code == KeyCode::Char('c') {
            return Effect::Quit;
        }
        self.notice = None;
        match self.overlay {
            Overlay::Help => {
                self.overlay = Overlay::None;
                return Effect::None;
            }
            Overlay::Palette { .. } => return self.palette_key(key),
            Overlay::None => {}
        }
        let page = self.height().saturating_sub(2).max(1) as isize;
        match key.code {
            KeyCode::Char('k') if ctrl => self.open_palette(),
            KeyCode::Char('/') => self.open_palette(),
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Char('q') | KeyCode::Esc => return Effect::Quit,
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-page),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(page),
            KeyCode::Home | KeyCode::Char('g') => self.scroll_to(0),
            KeyCode::End | KeyCode::Char('G') => self.scroll_to(usize::MAX),
            KeyCode::Tab => self.focus(true),
            KeyCode::BackTab => self.focus(false),
            KeyCode::Enter => {
                let page = self.page();
                if let Some(link) = self.focused_link.and_then(|link| page.links.get(link)) {
                    return self.follow(&link.target);
                }
            }
            KeyCode::Backspace => self.back(),
            KeyCode::Left if alt => self.back(),
            KeyCode::Right if alt => self.forward(),
            _ => {}
        }
        Effect::None
    }

    fn palette_key(&mut self, key: KeyEvent) -> Effect {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Overlay::Palette { query, selected, hits } = &mut self.overlay else { return Effect::None };
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Enter => {
                if let Some(hit) = hits.get(*selected).cloned() {
                    self.overlay = Overlay::None;
                    self.jump(&hit);
                }
            }
            KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::Char('p') if ctrl => *selected = selected.saturating_sub(1),
            KeyCode::Down => *selected = (*selected + 1).min(hits.len().saturating_sub(1)),
            KeyCode::Char('n') if ctrl => *selected = (*selected + 1).min(hits.len().saturating_sub(1)),
            KeyCode::Backspace => {
                query.pop();
                *hits = self.index.search(query);
                *selected = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                query.push(c);
                *hits = self.index.search(query);
                *selected = 0;
            }
            _ => {}
        }
        Effect::None
    }

    fn mouse(&mut self, mouse: MouseEvent) -> Effect {
        let at = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta = if mouse.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
                match &mut self.overlay {
                    Overlay::Palette { selected, hits, .. } => {
                        *selected = selected.saturating_add_signed(delta).min(hits.len().saturating_sub(1));
                    }
                    Overlay::Help => {}
                    Overlay::None => self.scroll_by(delta * WHEEL),
                }
                Effect::None
            }
            MouseEventKind::Down(MouseButton::Left) => self.click(at),
            _ => Effect::None,
        }
    }

    fn click(&mut self, at: Position) -> Effect {
        match self.overlay {
            Overlay::Palette { .. } if palette_area(self.area).contains(at) => return Effect::None,
            Overlay::Help | Overlay::Palette { .. } => {
                self.overlay = Overlay::None;
                return Effect::None;
            }
            Overlay::None => {}
        }
        let areas = areas(self.area);
        if let Some(rows) = areas.sidebar.map(sidebar_rows).filter(|rows| rows.contains(at)) {
            let topic = self.sidebar_offset(rows) + usize::from(at.y - rows.y);
            if topic < self.docs.len() {
                self.navigate(Location { topic, scroll: 0 });
            }
            return Effect::None;
        }
        if !areas.content.contains(at) {
            return Effect::None;
        }
        let line = self.location.scroll + usize::from(at.y - areas.content.y);
        let column = at.x - areas.content.x;
        let page = self.page();
        let hit = page
            .links
            .iter()
            .position(|link| link.spans.iter().any(|span| span.line == line && span.columns.contains(&column)));
        match hit {
            Some(link) => {
                self.focused_link = Some(link);
                self.follow(&page.links[link].target)
            }
            None => Effect::None,
        }
    }

    fn sidebar_offset(&self, rows: Rect) -> usize {
        (self.location.topic + 1).saturating_sub(usize::from(rows.height))
    }

    fn open_palette(&mut self) {
        self.overlay = Overlay::Palette { query: String::new(), selected: 0, hits: self.index.search("") };
    }

    fn scroll_by(&mut self, delta: isize) {
        self.scroll_to(self.location.scroll.saturating_add_signed(delta));
        let (top, height) = (self.location.scroll, self.height());
        let page = self.page();
        let visible = self
            .focused_link
            .and_then(|link| page.links.get(link))
            .is_some_and(|link| link.spans.iter().any(|span| (top..top + height).contains(&span.line)));
        if !visible {
            self.focused_link = None;
        }
    }

    fn scroll_to(&mut self, scroll: usize) {
        self.location.scroll = scroll.min(self.max_scroll());
    }

    fn focus(&mut self, forward: bool) {
        let page = self.page();
        let count = page.links.len();
        if count == 0 {
            return;
        }
        let (top, height) = (self.location.scroll, self.height());
        let first_line = |link: usize| page.links[link].spans[0].line;
        let next = match self.focused_link {
            Some(link) if forward => (link + 1) % count,
            Some(link) => (link + count - 1) % count,
            None if forward => (0..count).find(|&link| first_line(link) >= top).unwrap_or(0),
            None => (0..count).rev().find(|&link| first_line(link) < top + height).unwrap_or(count - 1),
        };
        self.focused_link = Some(next);
        let first = first_line(next);
        let last = page.links[next].spans.last().map_or(first, |span| span.line);
        if first < top {
            self.scroll_to(first);
        } else if last >= top + height {
            self.scroll_to(last + 1 - height);
        }
    }

    fn follow(&mut self, target: &Target) -> Effect {
        let (topic, anchor) = match target {
            Target::External(url) => return Effect::Open(url.clone()),
            Target::Anchor(anchor) => (self.location.topic, Some(anchor.as_str())),
            Target::Topic { name, anchor } => match self.docs.iter().position(|doc| doc.name == name) {
                Some(topic) => (topic, anchor.as_deref()),
                None => {
                    self.notice = Some(format!("No topic named {name}"));
                    return Effect::None;
                }
            },
        };
        self.open(topic, anchor);
        Effect::None
    }

    fn jump(&mut self, hit: &Hit) {
        if let Some(topic) = self.docs.iter().position(|doc| doc.name == hit.topic) {
            self.open(topic, hit.slug.as_deref());
        }
    }

    fn open(&mut self, topic: usize, anchor: Option<&str>) {
        let page = self.page_of(topic);
        let scroll = anchor.and_then(|anchor| page.heading(anchor)).map_or(0, |heading| heading.line);
        self.navigate(Location { topic, scroll });
    }

    fn navigate(&mut self, to: Location) {
        if to != self.location {
            self.back.push(self.location);
            self.forward.clear();
        }
        self.location = to;
        self.focused_link = None;
        self.scroll_to(to.scroll);
    }

    fn back(&mut self) {
        if let Some(previous) = self.back.pop() {
            self.forward.push(self.location);
            self.location = previous;
            self.focused_link = None;
        }
    }

    fn forward(&mut self) {
        if let Some(next) = self.forward.pop() {
            self.back.push(self.location);
            self.location = next;
            self.focused_link = None;
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        if self.area != frame.area() {
            self.area = frame.area();
            self.scroll_to(self.location.scroll);
        }
        let areas = areas(self.area);
        let page = self.page();
        if let Some(sidebar) = areas.sidebar {
            self.draw_sidebar(frame, sidebar);
        }
        let top = self.location.scroll;
        let height = usize::from(areas.content.height);
        let lines: Vec<Line> = page.lines.iter().skip(top).take(height).cloned().collect();
        frame.render_widget(Paragraph::new(lines), areas.content);
        if let Some(link) = self.focused_link.and_then(|link| page.links.get(link)) {
            for span in link.spans.iter().filter(|span| (top..top + height).contains(&span.line)) {
                let x = areas.content.x + span.columns.start;
                let width = (span.columns.end - span.columns.start).min(areas.content.right().saturating_sub(x));
                let y = areas.content.y + (span.line - top) as u16;
                frame.buffer_mut().set_style(Rect::new(x, y, width, 1), Style::new().add_modifier(Modifier::REVERSED));
            }
        }
        self.draw_status(frame, areas.status, &page);
        match &self.overlay {
            Overlay::None => {}
            Overlay::Help => draw_help(frame, help_area(self.area)),
            Overlay::Palette { query, selected, hits } => {
                draw_palette(frame, palette_area(self.area), query, *selected, hits)
            }
        }
    }

    fn draw_sidebar(&self, frame: &mut Frame, sidebar: Rect) {
        frame.render_widget(Block::new().borders(Borders::RIGHT).border_style(DIM), sidebar);
        let title = Line::styled(" Crucible docs", Style::new().add_modifier(Modifier::BOLD));
        frame.render_widget(Paragraph::new(title), Rect { y: sidebar.y + 1, height: 1, ..sidebar });
        let rows = sidebar_rows(sidebar);
        let offset = self.sidebar_offset(rows);
        let lines: Vec<Line> = self
            .docs
            .iter()
            .enumerate()
            .skip(offset)
            .map(|(topic, doc)| {
                let label = format!(" {:width$}", doc.name, width = usize::from(rows.width) - 1);
                if topic == self.location.topic {
                    Line::styled(label, Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::REVERSED))
                } else {
                    Line::raw(label)
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), rows);
    }

    fn draw_status(&self, frame: &mut Frame, status: Rect, page: &Page) {
        let height = self.height();
        let top = self.location.scroll;
        let total = page.lines.len();
        let section = page.headings.iter().rev().find(|heading| heading.level > 1 && heading.line <= top);
        let position = if total <= height {
            "All".to_owned()
        } else if top == 0 {
            "Top".to_owned()
        } else if top + height >= total {
            "End".to_owned()
        } else {
            format!("{}%", (top + height) * 100 / total)
        };
        let left = match section {
            Some(section) => format!(" {} › {}  {position} ", self.topic(), section.text),
            None => format!(" {}  {position} ", self.topic()),
        };
        let room = usize::from(status.width).saturating_sub(left.chars().count());
        let right = match &self.notice {
            Some(notice) => format!(" {notice} "),
            None => {
                [" Ctrl+K search · Tab links · Backspace back · ? help · q quit ", " Ctrl+K search · ? help · q quit "]
                    .into_iter()
                    .find(|hint| hint.chars().count() <= room)
                    .unwrap_or_default()
                    .to_owned()
            }
        };
        let gap = usize::from(status.width).saturating_sub(left.chars().count() + right.chars().count());
        let line = Line::from(vec![
            Span::styled(left, Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(" ".repeat(gap)),
            Span::raw(right),
        ]);
        frame.render_widget(Paragraph::new(line).style(Style::new().add_modifier(Modifier::REVERSED)), status);
    }
}

fn overlay_block(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    let block =
        Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(Color::Cyan)).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn draw_palette(frame: &mut Frame, area: Rect, query: &str, selected: usize, hits: &[Hit]) {
    let inner = overlay_block(frame, area, " Search the docs ");
    if inner.height < 3 {
        return;
    }
    let prompt = Line::from(vec![Span::styled("› ", Style::new().fg(Color::Cyan)), Span::raw(query.to_owned())]);
    frame.render_widget(Paragraph::new(prompt), Rect { height: 1, ..inner });
    frame.set_cursor_position(Position::new(inner.x + 2 + query.chars().count() as u16, inner.y));
    let rule = Line::styled("─".repeat(usize::from(inner.width)), DIM);
    frame.render_widget(Paragraph::new(rule), Rect { y: inner.y + 1, height: 1, ..inner });
    let list = Rect { y: inner.y + 2, height: inner.height - 2, ..inner };
    if hits.is_empty() {
        frame.render_widget(Paragraph::new(Line::styled(" No matches", DIM)), list);
        return;
    }
    let shown = usize::from(list.height / 2).max(1);
    let offset = (selected + 1).saturating_sub(shown);
    let mut lines = Vec::new();
    for (index, hit) in hits.iter().enumerate().skip(offset).take(shown) {
        let highlight = if index == selected { Style::new().add_modifier(Modifier::REVERSED) } else { Style::new() };
        let title = Line::from(vec![
            Span::styled(format!(" {}", hit.title), Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", hit.topic), Style::new().fg(Color::Cyan)),
        ]);
        let pad = |line: Line<'static>| {
            let width = line.width();
            let mut spans = line.spans;
            spans.push(Span::raw(" ".repeat(usize::from(inner.width).saturating_sub(width))));
            Line::from(spans).patch_style(highlight)
        };
        lines.push(pad(title));
        let room = usize::from(inner.width).saturating_sub(4);
        let snippet: String = match hit.snippet.chars().nth(room) {
            Some(_) => hit.snippet.chars().take(room.saturating_sub(1)).chain(['…']).collect(),
            None => hit.snippet.clone(),
        };
        lines.push(pad(Line::styled(format!("   {snippet}"), DIM)));
    }
    frame.render_widget(Paragraph::new(lines), list);
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let inner = overlay_block(frame, area, " Keys ");
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(keys, action)| {
            Line::from(vec![Span::styled(format!(" {keys:18}"), Style::new().fg(Color::Cyan)), Span::raw(*action)])
        })
        .chain([Line::raw(""), Line::styled(" Press any key to close.", DIM)])
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Raw mode, the alternate screen and mouse capture outlive the process unless undone, so every exit path undoes them.
fn restore() {
    let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen, ratatui::crossterm::cursor::Show);
    let _ = disable_raw_mode();
}

struct Session;

impl Session {
    fn start() -> std::io::Result<Self> {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
        let session = Session;
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture, ratatui::crossterm::cursor::Hide)?;
        Ok(session)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        restore();
    }
}

pub fn run(topic: Option<&str>) -> std::io::Result<()> {
    let _session = Session::start()?;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;
    let size = terminal.size()?;
    let mut reader = Reader::new(topic, Rect::new(0, 0, size.width, size.height));
    loop {
        terminal.draw(|frame| reader.draw(frame))?;
        match reader.update(event::read()?) {
            Effect::None => {}
            Effect::Quit => return Ok(()),
            Effect::Open(url) => {
                reader.notice = Some(match open::that_detached(&url) {
                    Ok(()) => format!("Opened {url}"),
                    Err(error) => format!("Could not open {url}: {error}"),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyEventState, MouseEvent};

    fn reader(topic: &str) -> Reader {
        Reader::new(Some(topic), Rect::new(0, 0, 120, 30))
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn click(column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn type_text(reader: &mut Reader, text: &str) {
        for c in text.chars() {
            assert_eq!(reader.update(key(KeyCode::Char(c))), Effect::None);
        }
    }

    fn screen(reader: &mut Reader, width: u16, height: u16) -> String {
        let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| reader.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect::<String>().trim_end().to_owned() + "\n")
            .collect()
    }

    fn heading_line(reader: &mut Reader, slug: &str) -> usize {
        reader.page().heading(slug).unwrap().line
    }

    #[test]
    fn following_a_cross_topic_link_then_going_back_returns_to_the_same_place() {
        let mut reader = reader("cli");
        let page = reader.page();
        let link = page
            .links
            .iter()
            .position(|link| link.target == Target::Topic { name: "oc".into(), anchor: Some("exit-codes".into()) })
            .expect("cli.md links to oc.md#exit-codes");
        while reader.focused_link != Some(link) {
            reader.update(key(KeyCode::Tab));
        }
        let before = reader.location;
        assert_eq!(reader.update(key(KeyCode::Enter)), Effect::None);
        assert_eq!(reader.topic(), "oc");
        let expected = heading_line(&mut reader, "exit-codes").min(reader.max_scroll());
        assert_eq!(reader.location.scroll, expected);
        reader.update(key(KeyCode::Backspace));
        assert_eq!(reader.topic(), "cli");
        assert_eq!(reader.location, before);
        reader.update(Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT)));
        assert_eq!(reader.topic(), "oc");
    }

    #[test]
    fn palette_query_and_enter_land_on_the_heading() {
        let mut reader = reader("lifecycle");
        reader.update(ctrl('k'));
        assert!(matches!(reader.overlay, Overlay::Palette { .. }));
        type_text(&mut reader, "exit codes");
        reader.update(key(KeyCode::Enter));
        assert!(matches!(reader.overlay, Overlay::None));
        assert_eq!(reader.topic(), "oc");
        let expected = heading_line(&mut reader, "exit-codes");
        assert_eq!(reader.location.scroll, expected);
    }

    #[test]
    fn escape_closes_the_palette_and_then_quits() {
        let mut reader = reader("oc");
        reader.update(key(KeyCode::Char('/')));
        type_text(&mut reader, "q");
        assert_eq!(reader.update(key(KeyCode::Esc)), Effect::None);
        assert!(matches!(reader.overlay, Overlay::None));
        assert_eq!(reader.topic(), "oc");
        assert_eq!(reader.update(key(KeyCode::Esc)), Effect::Quit);
        assert_eq!(reader.update(ctrl('c')), Effect::Quit);
    }

    #[test]
    fn tab_cycles_through_the_links_and_shift_tab_goes_back() {
        let mut reader = reader(HOME);
        let count = reader.page().links.len();
        assert_eq!(count, TOPICS.len());
        let mut seen = Vec::new();
        for _ in 0..count + 1 {
            reader.update(key(KeyCode::Tab));
            seen.push(reader.focused_link.unwrap());
        }
        assert_eq!(seen, (0..count).chain([0]).collect::<Vec<_>>());
        reader.update(key(KeyCode::BackTab));
        assert_eq!(reader.focused_link, Some(count - 1));
        let line = reader.page().links[count - 1].spans[0].line;
        assert!((reader.location.scroll..reader.location.scroll + reader.height()).contains(&line));
    }

    #[test]
    fn a_click_on_a_sidebar_row_opens_that_topic() {
        let mut reader = reader(HOME);
        let rows = sidebar_rows(areas(reader.area).sidebar.unwrap());
        let oc = reader.docs.iter().position(|doc| doc.name == "oc").unwrap() as u16;
        reader.update(click(rows.x + 2, rows.y + oc));
        assert_eq!(reader.topic(), "oc");
        assert_eq!(reader.location.scroll, 0);
    }

    #[test]
    fn a_click_on_a_link_follows_it() {
        let mut reader = reader(HOME);
        let content = areas(reader.area).content;
        let page = reader.page();
        let (index, link) = page
            .links
            .iter()
            .enumerate()
            .find(|(_, link)| link.target == Target::Topic { name: "random".into(), anchor: None })
            .unwrap();
        let span = &link.spans[0];
        reader.update(click(content.x + span.columns.start, content.y + span.line as u16));
        assert_eq!(reader.topic(), "random", "link {index}");
    }

    #[test]
    fn an_external_link_asks_the_shell_to_open_it() {
        let mut reader = reader("random");
        let page = reader.page();
        let link = page.links.iter().position(|link| matches!(link.target, Target::External(_))).unwrap();
        while reader.focused_link != Some(link) {
            reader.update(key(KeyCode::Tab));
        }
        assert_eq!(reader.update(key(KeyCode::Enter)), Effect::Open("https://prng.di.unimi.it/".into()));
        assert_eq!(reader.topic(), "random");
    }

    #[test]
    fn scrolling_stays_inside_the_page() {
        let mut reader = reader("oc");
        reader.update(key(KeyCode::Up));
        assert_eq!(reader.location.scroll, 0);
        reader.update(key(KeyCode::Char('G')));
        let end = reader.location.scroll;
        assert_eq!(end, reader.page().lines.len() - reader.height());
        reader.update(key(KeyCode::Down));
        assert_eq!(reader.location.scroll, end);
        reader.update(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 50,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(reader.location.scroll, end - 3);
        reader.update(key(KeyCode::Char('g')));
        assert_eq!(reader.location.scroll, 0);
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut reader = reader("oc");
        let mut release = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        release.state = KeyEventState::NONE;
        assert_eq!(reader.update(Event::Key(release)), Effect::None);
    }

    #[test]
    fn the_frame_shows_the_sidebar_title_and_status_line() {
        let mut reader = reader("lifecycle");
        let screen = screen(&mut reader, 120, 30);
        let lines: Vec<_> = screen.lines().collect();
        assert!(lines[1].starts_with(" Crucible docs"), "{screen}");
        assert!(lines.iter().any(|line| line.starts_with(" lifecycle ")), "{screen}");
        assert!(lines.iter().any(|line| line.contains("Lifecycle") && line.contains("│")), "{screen}");
        assert!(lines[29].starts_with(" lifecycle  Top") && lines[29].ends_with("q quit"), "{screen}");
    }

    #[test]
    fn a_narrow_frame_hides_the_sidebar() {
        let mut reader = reader("lifecycle");
        let screen = screen(&mut reader, 80, 24);
        assert!(!screen.contains("Crucible docs"), "{screen}");
        assert!(screen.lines().nth(1).unwrap().contains("Lifecycle"), "{screen}");
    }

    #[test]
    fn the_palette_and_help_draw_over_the_page() {
        let mut reader = reader("lifecycle");
        reader.update(ctrl('k'));
        type_text(&mut reader, "exit codes");
        let screen = screen(&mut reader, 120, 30);
        assert!(screen.contains("Search the docs") && screen.contains("› exit codes"), "{screen}");
        assert!(screen.contains("Exit codes  oc"), "{screen}");
        reader.update(key(KeyCode::Esc));
        reader.update(key(KeyCode::Char('?')));
        assert!(screen_contains(&mut reader, "Toggle this help"));
        reader.update(key(KeyCode::Char('x')));
        assert!(!screen_contains(&mut reader, "Toggle this help"));
    }

    fn screen_contains(reader: &mut Reader, text: &str) -> bool {
        screen(reader, 120, 30).contains(text)
    }
}
