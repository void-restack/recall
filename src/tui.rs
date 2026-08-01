use std::io;
use std::time::Duration;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};

use crate::line_editor::{Handled, LineEditor};
use crate::memory::CommandMemory;
use crate::search;
use crate::theme::Theme;

type Term = Terminal<CrosstermBackend<io::Stderr>>;

/// Persistence the live picker drives while it stays open, so edits and deletes
/// apply without tearing the viewport down and back up. Implemented over the Store
/// in the command layer, keeping the TUI decoupled from the repository.
pub trait PickerStore {
    fn reload(&self) -> Result<Vec<CommandMemory>>;
    fn save_edit(&self, id: i64, form: AddForm) -> Result<()>;
    fn delete(&self, id: i64) -> Result<()>;
}

/// Which surface the single viewport is showing. Editing and confirm-delete are
/// modes of the same picker, not separate screens.
enum Mode {
    Picker,
    Editing { form: FormState, id: i64 },
    ConfirmDelete,
}

/// Whether the event loop should keep running or exit with a selection.
enum Flow {
    Continue,
    Done(Option<CommandMemory>),
}

/// Run the picker over one persistent viewport. Draws on stderr so stdout stays
/// clean for the selected command; returns the chosen memory, or `None` on cancel.
/// `now` stamps relative times in the preview without the TUI reading the clock.
pub fn run(
    store: &dyn PickerStore,
    memories: Vec<CommandMemory>,
    now: i64,
) -> Result<Option<CommandMemory>> {
    let _guard = RawGuard::enter()?;
    let mut terminal = inline_terminal(18)?;
    let mut app = App::new(memories, now);

    let outcome = 'outer: loop {
        terminal.draw(|f| app.render(f))?;

        // Block for one event, then apply any already-queued events before the next
        // draw — a burst of keystrokes (or a slow SSH link) refilters once, not N times.
        let mut event = event::read()?;
        loop {
            if let Flow::Done(selection) = app.handle_event(event, store)? {
                break 'outer selection;
            }
            if !event::poll(Duration::ZERO)? {
                break;
            }
            event = event::read()?;
        }
        app.settle();
    };

    terminal.clear()?;
    Ok(outcome)
}

struct App {
    memories: Vec<CommandMemory>,
    haystacks: Vec<String>,
    query: LineEditor,
    results: Vec<usize>,
    state: ListState,
    mode: Mode,
    dirty: bool,
    drafts_only: bool,
    theme: Theme,
    now: i64,
}

impl App {
    fn new(memories: Vec<CommandMemory>, now: i64) -> Self {
        let haystacks = search::build_haystacks(&memories);
        let mut app = Self {
            memories,
            haystacks,
            query: LineEditor::default(),
            results: Vec::new(),
            state: ListState::default(),
            mode: Mode::Picker,
            dirty: false,
            drafts_only: false,
            theme: Theme::detect(),
            now,
        };
        app.recompute_results();
        app.state.select((!app.results.is_empty()).then_some(0));
        app
    }

    fn selected_memory(&self) -> Option<&CommandMemory> {
        self.state
            .selected()
            .and_then(|i| self.results.get(i))
            .map(|&i| &self.memories[i])
    }

    fn render(&mut self, f: &mut Frame) {
        if let Mode::Editing { form, .. } = &self.mode {
            render_form(f, form, &self.theme);
            return;
        }
        let confirming = matches!(self.mode, Mode::ConfirmDelete);
        render_picker(
            f,
            &self.query,
            &self.results,
            &self.memories,
            &mut self.state,
            confirming,
            self.drafts_only,
            &self.theme,
            self.now,
        );
    }

    fn handle_event(&mut self, event: Event, store: &dyn PickerStore) -> Result<Flow> {
        match self.mode {
            Mode::Picker => self.handle_picker(event),
            Mode::Editing { .. } => self.handle_editing(event, store),
            Mode::ConfirmDelete => self.handle_confirm(event, store),
        }
    }

    fn handle_picker(&mut self, event: Event) -> Result<Flow> {
        match event {
            Event::Paste(text) => {
                self.query.insert_str(&text);
                self.dirty = true;
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Esc => return Ok(Flow::Done(None)),
                    KeyCode::Char('c') if ctrl => return Ok(Flow::Done(None)),
                    KeyCode::Enter => {
                        if let Some(m) = self.selected_memory() {
                            return Ok(Flow::Done(Some(m.clone())));
                        }
                    }
                    KeyCode::Char('o') if ctrl => self.begin_edit(),
                    KeyCode::Char('x') if ctrl => {
                        if self.selected_memory().is_some() {
                            self.mode = Mode::ConfirmDelete;
                        }
                    }
                    KeyCode::Up => move_selection(&mut self.state, self.results.len(), -1),
                    KeyCode::Down => move_selection(&mut self.state, self.results.len(), 1),
                    KeyCode::Char('p') if ctrl => {
                        move_selection(&mut self.state, self.results.len(), -1)
                    }
                    KeyCode::Char('n') if ctrl => {
                        move_selection(&mut self.state, self.results.len(), 1)
                    }
                    KeyCode::PageUp => page_selection(&mut self.state, self.results.len(), -PAGE),
                    KeyCode::PageDown => page_selection(&mut self.state, self.results.len(), PAGE),
                    KeyCode::Char('d') if ctrl => {
                        self.drafts_only = !self.drafts_only;
                        self.dirty = true;
                    }
                    _ => {
                        if self.query.handle_key(key) == Handled::Edited {
                            self.dirty = true;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn handle_editing(&mut self, event: Event, store: &dyn PickerStore) -> Result<Flow> {
        match event {
            Event::Paste(text) => {
                if let Mode::Editing { form, .. } = &mut self.mode {
                    form.field().insert_str(&text);
                }
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Picker,
                    KeyCode::Char('c') if ctrl => self.mode = Mode::Picker,
                    KeyCode::Enter => self.commit_edit(store)?,
                    KeyCode::Tab | KeyCode::Down => {
                        if let Mode::Editing { form, .. } = &mut self.mode {
                            form.next();
                        }
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        if let Mode::Editing { form, .. } = &mut self.mode {
                            form.prev();
                        }
                    }
                    _ => {
                        if let Mode::Editing { form, .. } = &mut self.mode {
                            form.field().handle_key(key);
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn handle_confirm(&mut self, event: Event, store: &dyn PickerStore) -> Result<Flow> {
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
        {
            let confirmed = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
            if confirmed && let Some(id) = self.selected_memory().map(|m| m.id) {
                let pos = self.state.selected().unwrap_or(0);
                store.delete(id)?;
                self.reload(store)?;
                self.select_near(pos);
            }
            self.mode = Mode::Picker;
        }
        Ok(Flow::Continue)
    }

    fn begin_edit(&mut self) {
        if let Some(m) = self.selected_memory() {
            let form = FormState::new(
                &m.command,
                m.description.as_deref().unwrap_or(""),
                &m.tags.join(" "),
            );
            self.mode = Mode::Editing { form, id: m.id };
        }
    }

    fn commit_edit(&mut self, store: &dyn PickerStore) -> Result<()> {
        let taken = std::mem::replace(&mut self.mode, Mode::Picker);
        let Mode::Editing { form, id } = taken else {
            return Ok(());
        };
        if form.command_is_blank() {
            self.mode = Mode::Editing { form, id };
            return Ok(());
        }
        store.save_edit(id, form.into_add_form())?;
        self.reload(store)?;
        // Keep the just-edited row under the cursor even if it moved in the results.
        let idx = self.results.iter().position(|&i| self.memories[i].id == id);
        self.state
            .select(idx.or_else(|| (!self.results.is_empty()).then_some(0)));
        Ok(())
    }

    /// After the drained event batch, refilter once if the query changed.
    fn settle(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.recompute_results();
        reselect(&mut self.state, self.results.len());
    }

    fn reload(&mut self, store: &dyn PickerStore) -> Result<()> {
        self.memories = store.reload()?;
        self.haystacks = search::build_haystacks(&self.memories);
        self.recompute_results();
        Ok(())
    }

    fn recompute_results(&mut self) {
        let mut results = if self.query.text().trim().is_empty() {
            // No query: rank by frecency instead of insertion order.
            search::frecency_order(&self.memories, self.now)
        } else {
            search::ranked_indices(self.query.text(), &self.memories, &self.haystacks, 200)
        };
        if self.drafts_only {
            results.retain(|&i| self.memories[i].is_draft());
        }
        self.results = results;
    }

    /// Select the row at `pos`, clamped — the neighbor of a deleted row.
    fn select_near(&mut self, pos: usize) {
        let sel = (!self.results.is_empty()).then(|| pos.min(self.results.len() - 1));
        self.state.select(sel);
    }
}

fn reselect(state: &mut ListState, len: usize) {
    let selected = if len == 0 {
        None
    } else {
        Some(state.selected().unwrap_or(0).min(len - 1))
    };
    state.select(selected);
}

/// Rows moved by PgUp/PgDn.
const PAGE: isize = 10;

fn move_selection(state: &mut ListState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let current = state.selected().unwrap_or(0) as isize;
    let next = (current + delta).rem_euclid(len as isize) as usize;
    state.select(Some(next));
}

/// Like `move_selection` but clamped, not wrapped — paging past an end stops there.
fn page_selection(state: &mut ListState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let current = state.selected().unwrap_or(0) as isize;
    let next = (current + delta).clamp(0, len as isize - 1) as usize;
    state.select(Some(next));
}

/// Place the terminal caret at `col` inside a bordered single-line box, so the
/// active line editor shows a real, native cursor. Clamped to the box interior.
fn set_line_cursor(f: &mut Frame, area: Rect, col: usize) {
    let inner = Block::bordered().inner(area);
    if inner.width == 0 {
        return;
    }
    let x = (inner.x + col as u16).min(inner.right().saturating_sub(1));
    f.set_cursor_position(Position { x, y: inner.y });
}

#[allow(clippy::too_many_arguments)]
fn render_picker(
    f: &mut Frame,
    query: &LineEditor,
    results: &[usize],
    memories: &[CommandMemory],
    state: &mut ListState,
    confirming: bool,
    drafts_only: bool,
    theme: &Theme,
    now: i64,
) {
    let area = f.area();
    let [top, middle, help] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let prompt = "search: ";
    f.render_widget(
        Paragraph::new(format!("{prompt}{}", query.text()))
            .block(Block::bordered().title("recall").border_style(theme.accent)),
        top,
    );
    set_line_cursor(f, top, prompt.chars().count() + query.cursor_col());

    // Wide: list + preview pane. Medium: list + a 2-line detail strip. Narrow: list only.
    if area.width >= 100 {
        let [list_area, preview_area] =
            Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                .areas(middle);
        render_list(
            f,
            list_area,
            results,
            memories,
            state,
            query.text(),
            drafts_only,
            theme,
        );
        let preview = preview_text(
            results,
            memories,
            state.selected(),
            query.text(),
            theme,
            now,
        );
        f.render_widget(
            Paragraph::new(preview)
                .block(Block::bordered().title("details"))
                .wrap(Wrap { trim: false }),
            preview_area,
        );
    } else if area.width >= 60 {
        let [list_area, strip] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(middle);
        render_list(
            f,
            list_area,
            results,
            memories,
            state,
            query.text(),
            drafts_only,
            theme,
        );
        render_detail_strip(f, strip, results, memories, state.selected(), theme, now);
    } else {
        render_list(
            f,
            middle,
            results,
            memories,
            state,
            query.text(),
            drafts_only,
            theme,
        );
    }

    let help_line = if confirming {
        Line::styled("delete this memory?  y / n", theme.danger)
    } else {
        Line::styled(footer_hint(area.width), theme.dim)
    };
    f.render_widget(Paragraph::new(help_line), help);
}

/// The footer adapts to width: the full key legend when there's room, a terse one
/// when there isn't.
fn footer_hint(width: u16) -> &'static str {
    if width >= 84 {
        "↑/↓ move · pgup/pgdn page · ⏎ print · ^o edit · ^x delete · ^d drafts · esc quit"
    } else {
        "↑↓ move · ⏎ print · ^o edit · ^x del · ^d drafts · esc quit"
    }
}

#[allow(clippy::too_many_arguments)]
fn render_list(
    f: &mut Frame,
    area: Rect,
    results: &[usize],
    memories: &[CommandMemory],
    state: &mut ListState,
    query: &str,
    drafts_only: bool,
    theme: &Theme,
) {
    // Reserve the interior minus the 2-cell highlight gutter for row text.
    let text_width = Block::bordered().inner(area).width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = results
        .iter()
        .map(|&i| ListItem::new(row_line(&memories[i], query, theme, text_width)))
        .collect();
    let pos = state.selected().map_or(0, |i| i + 1);
    let mut title = if results.is_empty() {
        "no matches".to_string()
    } else {
        format!("{pos}/{}", results.len())
    };
    if drafts_only {
        title.push_str(" · drafts only");
    } else {
        let drafts = memories.iter().filter(|m| m.is_draft()).count();
        if drafts > 0 {
            title.push_str(&format!(
                " · {drafts} draft{}",
                if drafts == 1 { "" } else { "s" }
            ));
        }
    }
    let list = List::new(items)
        .block(Block::bordered().title(title))
        .highlight_symbol("▌ ")
        .highlight_style(theme.selection);
    f.render_stateful_widget(list, area, state);
}

/// The medium-width stand-in for the preview pane: the command, then its why (or
/// usage), on two lines truncated to fit.
fn render_detail_strip(
    f: &mut Frame,
    area: Rect,
    results: &[usize],
    memories: &[CommandMemory],
    selected: Option<usize>,
    theme: &Theme,
    now: i64,
) {
    let Some(m) = selected.and_then(|s| results.get(s)).map(|&i| &memories[i]) else {
        return;
    };
    let width = area.width as usize;
    let first = Line::from(fit_spans(program_spans(&m.command, theme), width));
    let second = match m.description.as_deref().filter(|d| !d.is_empty()) {
        Some(desc) => Line::styled(truncate_str(desc, width), theme.dim),
        None => Line::styled(truncate_str(&usage_line(m, now), width), theme.dim),
    };
    f.render_widget(Paragraph::new(Text::from(vec![first, second])), area);
}

fn preview_text<'a>(
    results: &[usize],
    memories: &'a [CommandMemory],
    selected: Option<usize>,
    query: &str,
    theme: &Theme,
    now: i64,
) -> Text<'a> {
    let Some(m) = selected.and_then(|s| results.get(s)).map(|&i| &memories[i]) else {
        return Text::default();
    };
    let mut lines: Vec<Line> = Vec::new();

    // Command with its first token (the program) bold.
    lines.push(program_first(&m.command, theme));
    lines.push(Line::default());

    match m.description.as_deref().filter(|d| !d.is_empty()) {
        Some(desc) => lines.push(Line::raw(desc.to_string())),
        None => lines.push(Line::styled(
            "(no description yet — press ^o to add one)",
            theme.dim,
        )),
    }
    lines.push(Line::default());

    if !m.tags.is_empty() {
        lines.push(Line::styled(
            format!("tags: {}", m.tags.join(", ")),
            theme.tag,
        ));
    }

    lines.push(Line::styled(usage_line(m, now), theme.dim));

    // Why this row matched: the terms that hit, and the filler dropped.
    if !query.trim().is_empty() {
        let haystack = search::build_haystacks(std::slice::from_ref(m));
        let (matched, dropped) = search::explain_match(query, &haystack[0]);
        if !matched.is_empty() {
            let mut s = format!("matched: {}", matched.join(", "));
            if !dropped.is_empty() {
                s.push_str(&format!(" (dropped: {})", dropped.join(", ")));
            }
            lines.push(Line::styled(s, theme.dim));
        }
    }

    Text::from(lines)
}

/// The command as a line, with the program (first whitespace token) in bold.
fn program_first(command: &str, theme: &Theme) -> Line<'static> {
    Line::from(program_spans(command, theme))
}

/// Spans for a command with the program (first whitespace token) bold.
fn program_spans(command: &str, theme: &Theme) -> Vec<Span<'static>> {
    match command.split_once(char::is_whitespace) {
        Some((head, rest)) => vec![
            Span::styled(head.to_string(), theme.strong),
            Span::raw(format!(" {rest}")),
        ],
        None => vec![Span::styled(command.to_string(), theme.strong)],
    }
}

/// A row: a curated/draft gutter badge, the matched-char-highlighted command, dim
/// why, and tag chips — truncated with an ellipsis to `width`, trimming the tail
/// (tags, then why) before the command.
fn row_line(m: &CommandMemory, query: &str, theme: &Theme, width: usize) -> Line<'static> {
    let badge = if m.is_draft() {
        Span::styled("○ ", theme.draft)
    } else {
        Span::styled("● ", theme.dim)
    };
    let mut spans = vec![badge];
    spans.extend(command_spans(&m.command, query, theme));
    if let Some(desc) = m.description.as_deref().filter(|d| !d.is_empty()) {
        spans.push(Span::styled(format!("  — {desc}"), theme.dim));
    }
    if !m.tags.is_empty() {
        spans.push(Span::styled(
            format!("  [{}]", m.tags.join(", ")),
            theme.tag,
        ));
    }
    Line::from(fit_spans(spans, width))
}

/// Truncate a span run to `width` columns, appending `…` when it overflows. Trims
/// from the tail, so leading spans (the command) survive longest.
fn fit_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if total <= width {
        return spans;
    }
    if width == 0 {
        return Vec::new();
    }
    let budget = width - 1; // leave a cell for the ellipsis
    let mut out = Vec::new();
    let mut used = 0;
    for span in spans {
        let w = span.content.chars().count();
        if used + w <= budget {
            used += w;
            out.push(span);
        } else {
            let take = budget - used;
            if take > 0 {
                let head: String = span.content.chars().take(take).collect();
                out.push(Span::styled(head, span.style));
            }
            break;
        }
    }
    out.push(Span::raw("…"));
    out
}

/// Char-truncate a string to `width`, appending `…` when it overflows.
fn truncate_str(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// One-line usage summary: `used 14× · last used 2d ago`, or `not used yet`.
fn usage_line(m: &CommandMemory, now: i64) -> String {
    if m.use_count == 0 {
        return "not used yet".to_string();
    }
    let mut s = format!("used {}×", m.use_count);
    if let Some(last) = m.last_used_at {
        s.push_str(&format!(" · last used {}", humanize_ago(now, last)));
    }
    s
}

/// Split a command into spans, styling the query-matched characters. Highlighting
/// runs only for the ≤200 ranked rows (empty queries skip it entirely).
fn command_spans(command: &str, query: &str, theme: &Theme) -> Vec<Span<'static>> {
    if query.trim().is_empty() {
        return vec![Span::raw(command.to_string())];
    }
    let matched: std::collections::HashSet<usize> = search::match_positions(query, command)
        .into_iter()
        .collect();
    if matched.is_empty() {
        return vec![Span::raw(command.to_string())];
    }
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_hl = false;
    for (i, ch) in command.chars().enumerate() {
        let hl = matched.contains(&i);
        if hl != run_hl && !run.is_empty() {
            spans.push(styled_run(std::mem::take(&mut run), run_hl, theme));
        }
        run_hl = hl;
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(styled_run(run, run_hl, theme));
    }
    spans
}

fn styled_run(text: String, highlighted: bool, theme: &Theme) -> Span<'static> {
    if highlighted {
        Span::styled(text, theme.matched)
    } else {
        Span::raw(text)
    }
}

/// Compact relative time like `2d ago`, from millisecond timestamps.
fn humanize_ago(now: i64, then: i64) -> String {
    let secs = (now - then).max(0) / 1000;
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Browse shell history and pick a command to promote. Returns the chosen index,
/// or `None` if cancelled.
pub fn history_picker(entries: &[String]) -> Result<Option<usize>> {
    let _guard = RawGuard::enter()?;
    let mut terminal = inline_terminal(18)?;

    let theme = Theme::detect();
    let mut query = LineEditor::default();
    let mut results: Vec<usize> = (0..entries.len()).collect();
    let mut state = ListState::default();
    state.select((!results.is_empty()).then_some(0));

    let result = loop {
        terminal.draw(|f| render_history(f, &query, &results, entries, &mut state, &theme))?;

        match event::read()? {
            Event::Paste(text) => {
                query.insert_str(&text);
                results = filter_history(query.text(), entries);
                reselect(&mut state, results.len());
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Esc => break None,
                    KeyCode::Char('c') if ctrl => break None,
                    KeyCode::Enter => break state.selected().and_then(|i| results.get(i).copied()),
                    KeyCode::Up => move_selection(&mut state, results.len(), -1),
                    KeyCode::Down => move_selection(&mut state, results.len(), 1),
                    _ => {
                        if query.handle_key(key) == Handled::Edited {
                            results = filter_history(query.text(), entries);
                            reselect(&mut state, results.len());
                        }
                    }
                }
            }
            _ => {}
        }
    };

    terminal.clear()?;
    Ok(result)
}

fn filter_history(query: &str, entries: &[String]) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..entries.len()).collect();
    }
    search::rank_lines(query, entries, 500)
}

fn render_history(
    f: &mut Frame,
    query: &LineEditor,
    results: &[usize],
    entries: &[String],
    state: &mut ListState,
    theme: &Theme,
) {
    let [top, list, help] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let prompt = "history: ";
    f.render_widget(
        Paragraph::new(format!("{prompt}{}", query.text())).block(
            Block::bordered()
                .title("promote a command into a memory")
                .border_style(theme.accent),
        ),
        top,
    );
    set_line_cursor(f, top, prompt.chars().count() + query.cursor_col());

    let items: Vec<ListItem> = results
        .iter()
        .map(|&i| {
            ListItem::new(Line::from(command_spans(
                entries[i].as_str(),
                query.text(),
                theme,
            )))
        })
        .collect();
    let widget = List::new(items)
        .block(Block::bordered().title(format!("{} shown", results.len())))
        .highlight_symbol("▌ ")
        .highlight_style(theme.selection);
    f.render_stateful_widget(widget, list, state);

    f.render_widget(
        Paragraph::new(Line::styled(
            "↑/↓ move · enter annotate & save · esc cancel",
            theme.dim,
        )),
        help,
    );
}

/// What the add/edit form collected. Tags are raw text, split and normalized by the caller.
pub struct AddForm {
    pub command: String,
    pub description: String,
    pub tags: String,
}

/// Interactive capture/edit form, pre-filled with the given values. Returns `None`
/// if cancelled. Draws in an inline viewport on stderr, like the picker.
pub fn add_form(command: &str, description: &str, tags: &str) -> Result<Option<AddForm>> {
    let _guard = RawGuard::enter()?;
    let mut terminal = inline_terminal(14)?;
    let theme = Theme::detect();
    let mut form = FormState::new(command, description, tags);

    let result = loop {
        terminal.draw(|f| render_form(f, &form, &theme))?;

        match event::read()? {
            Event::Paste(text) => form.field().insert_str(&text),
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Esc => break None,
                    KeyCode::Char('c') if ctrl => break None,
                    KeyCode::Enter if !form.command_is_blank() => break Some(form.into_add_form()),
                    KeyCode::Tab | KeyCode::Down => form.next(),
                    KeyCode::BackTab | KeyCode::Up => form.prev(),
                    _ => {
                        form.field().handle_key(key);
                    }
                }
            }
            _ => {}
        }
    };

    terminal.clear()?;
    Ok(result)
}

struct FormState {
    command: LineEditor,
    description: LineEditor,
    tags: LineEditor,
    focus: usize,
}

impl FormState {
    fn new(command: &str, description: &str, tags: &str) -> Self {
        Self {
            command: LineEditor::new(command),
            description: LineEditor::new(description),
            tags: LineEditor::new(tags),
            focus: 0,
        }
    }

    fn field(&mut self) -> &mut LineEditor {
        match self.focus {
            0 => &mut self.command,
            1 => &mut self.description,
            _ => &mut self.tags,
        }
    }

    fn next(&mut self) {
        self.focus = (self.focus + 1) % 3;
    }

    fn prev(&mut self) {
        self.focus = (self.focus + 2) % 3;
    }

    fn command_is_blank(&self) -> bool {
        self.command.text().trim().is_empty()
    }

    fn into_add_form(self) -> AddForm {
        AddForm {
            command: self.command.into_text().trim().to_string(),
            description: self.description.into_text(),
            tags: self.tags.into_text(),
        }
    }
}

fn render_form(f: &mut Frame, form: &FormState, theme: &Theme) {
    let [command, description, tags, help] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .areas(f.area());

    f.render_widget(
        field(form.command.text(), "command", form.focus == 0, theme),
        command,
    );
    f.render_widget(
        field(
            form.description.text(),
            "why (description)",
            form.focus == 1,
            theme,
        ),
        description,
    );
    f.render_widget(
        field(
            form.tags.text(),
            "tags (space or comma separated)",
            form.focus == 2,
            theme,
        ),
        tags,
    );

    let (focused_area, focused) = match form.focus {
        0 => (command, &form.command),
        1 => (description, &form.description),
        _ => (tags, &form.tags),
    };
    set_line_cursor(f, focused_area, focused.cursor_col());

    f.render_widget(
        Paragraph::new(Line::styled(
            "tab move · enter save · esc cancel",
            theme.dim,
        )),
        help,
    );
}

fn field<'a>(value: &'a str, label: &'a str, focused: bool, theme: &Theme) -> Paragraph<'a> {
    let border = if focused { theme.accent } else { Style::new() };
    Paragraph::new(value).block(Block::bordered().title(label).border_style(border))
}

/// An inline viewport anchored under the prompt (fzf-style), sized to fit but never
/// taller than the terminal. On stderr so stdout stays free for the selection.
fn inline_terminal(desired_rows: u16) -> Result<Term> {
    let rows = size().map(|(_, r)| r).unwrap_or(24);
    let height = desired_rows.min(rows.saturating_sub(1)).max(3);
    let terminal = Terminal::with_options(
        CrosstermBackend::new(io::stderr()),
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )?;
    Ok(terminal)
}

/// Enables raw mode and bracketed paste, restoring both on drop — including during
/// a panic. No alternate screen: the inline viewport keeps your scrollback visible
/// above it.
struct RawGuard;

impl RawGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        // Bracketed paste lets a multi-word paste arrive as one Event::Paste instead
        // of a burst of keystrokes. Independent of the keyboard-enhancement flags.
        execute!(io::stderr(), EnableBracketedPaste)?;
        Ok(Self)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stderr(), DisableBracketedPaste);
        let _ = disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn mem(id: i64, command: &str, description: Option<&str>, tags: &[&str]) -> CommandMemory {
        CommandMemory {
            id,
            command: command.into(),
            description: description.map(Into::into),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            created_at: 0,
            updated_at: 0,
            use_count: 0,
            last_used_at: None,
        }
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(ratatui::crossterm::event::KeyEvent::new(
            code,
            KeyModifiers::NONE,
        ))
    }

    /// Flatten styled Text back to a plain string for content assertions.
    fn flatten(text: &Text) -> String {
        text.lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// In-memory PickerStore for exercising the App state machine.
    struct MockStore {
        memories: std::cell::RefCell<Vec<CommandMemory>>,
    }

    impl PickerStore for MockStore {
        fn reload(&self) -> Result<Vec<CommandMemory>> {
            Ok(self.memories.borrow().clone())
        }
        fn save_edit(&self, id: i64, form: AddForm) -> Result<()> {
            if let Some(m) = self.memories.borrow_mut().iter_mut().find(|m| m.id == id) {
                m.command = form.command;
                m.description = Some(form.description).filter(|d| !d.trim().is_empty());
            }
            Ok(())
        }
        fn delete(&self, id: i64) -> Result<()> {
            self.memories.borrow_mut().retain(|m| m.id != id);
            Ok(())
        }
    }

    #[test]
    fn typing_coalesces_and_narrows_on_settle() {
        let memories = vec![
            mem(1, "docker ps", Some("containers"), &["docker"]),
            mem(2, "git status", Some("changes"), &["git"]),
        ];
        let mut app = App::new(memories, 0);
        for c in "git".chars() {
            app.handle_picker(key(KeyCode::Char(c))).unwrap();
        }
        // Coalesced: results don't change until the drained batch settles.
        assert_eq!(app.results.len(), 2);
        app.settle();
        assert_eq!(app.results.len(), 1);
        assert_eq!(app.memories[app.results[0]].id, 2);
    }

    #[test]
    fn ctrl_d_filters_to_drafts_only() {
        let memories = vec![
            mem(1, "docker ps", Some("curated"), &[]),
            mem(2, "rm -rf tmp", None, &[]),
        ];
        let mut app = App::new(memories, 0);
        let ctrl_d = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        ));
        app.handle_picker(ctrl_d).unwrap();
        app.settle();
        assert!(app.drafts_only);
        assert_eq!(app.results, vec![1]);
    }

    #[test]
    fn edit_returns_to_the_picker_reselecting_by_id() {
        let memories = vec![mem(1, "alpha", None, &[]), mem(2, "beta", None, &[])];
        let store = MockStore {
            memories: std::cell::RefCell::new(memories.clone()),
        };
        let mut app = App::new(memories, 0);
        app.state.select(Some(1));
        app.begin_edit();
        app.handle_event(key(KeyCode::Enter), &store).unwrap();
        assert!(matches!(app.mode, Mode::Picker));
        assert_eq!(app.selected_memory().map(|m| m.id), Some(2));
    }

    #[test]
    fn delete_keeps_the_cursor_on_a_neighbor() {
        let memories = vec![
            mem(1, "a", None, &[]),
            mem(2, "b", None, &[]),
            mem(3, "c", None, &[]),
        ];
        let store = MockStore {
            memories: std::cell::RefCell::new(memories.clone()),
        };
        let mut app = App::new(memories, 0);
        app.state.select(Some(1));
        app.mode = Mode::ConfirmDelete;
        app.handle_event(key(KeyCode::Char('y')), &store).unwrap();
        assert!(matches!(app.mode, Mode::Picker));
        assert_eq!(app.results.len(), 2);
        // Was on index 1 ("b"); after deleting it the cursor holds index 1 ("c").
        assert_eq!(app.selected_memory().map(|m| m.id), Some(3));
    }

    #[test]
    fn empty_query_uses_frecency_order() {
        let memories = vec![
            mem(1, "docker ps", None, &[]),
            mem(2, "git stash", None, &[]),
        ];
        let app = App::new(memories, 0);
        assert_eq!(app.results, vec![0, 1]);
    }

    #[test]
    fn typing_ranks_the_best_match_first() {
        let memories = vec![
            mem(1, "docker ps", Some("list containers"), &["docker"]),
            mem(2, "git stash", Some("shelve changes"), &["git"]),
        ];
        let mut app = App::new(memories, 0);
        for c in "stash".chars() {
            app.handle_picker(key(KeyCode::Char(c))).unwrap();
        }
        app.settle();
        assert_eq!(app.results.first(), Some(&1));
    }

    #[test]
    fn selection_wraps_both_ways() {
        let mut state = ListState::default();
        state.select(Some(0));
        move_selection(&mut state, 2, -1);
        assert_eq!(state.selected(), Some(1));
        move_selection(&mut state, 2, 1);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn preview_shows_the_description_and_usage() {
        let memories = vec![mem(
            1,
            "docker ps",
            Some("list running containers"),
            &["docker"],
        )];
        let preview = preview_text(&[0], &memories, Some(0), "", &Theme::detect(), 0);
        let text = flatten(&preview);
        assert!(text.contains("docker ps"));
        assert!(text.contains("list running containers"));
        assert!(text.contains("not used yet"));
    }

    #[test]
    fn renders_query_list_and_preview() {
        let memories = vec![mem(
            1,
            "docker ps",
            Some("list running containers"),
            &["docker"],
        )];
        let results = vec![0usize];
        let mut state = ListState::default();
        state.select(Some(0));

        let query = LineEditor::new("docker");
        let theme = Theme::detect();
        // ≥100 cols draws the side-by-side preview pane.
        let mut terminal = Terminal::new(TestBackend::new(110, 12)).unwrap();
        terminal
            .draw(|f| {
                render_picker(
                    f, &query, &results, &memories, &mut state, false, false, &theme, 0,
                )
            })
            .unwrap();

        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("search: docker"), "header missing:\n{text}");
        assert!(text.contains("docker ps"), "row/preview missing:\n{text}");
        assert!(text.contains("details"), "preview pane missing:\n{text}");
    }

    #[test]
    fn narrow_layout_drops_the_preview_pane() {
        let memories = vec![mem(1, "docker ps", Some("list containers"), &["docker"])];
        let results = vec![0usize];
        let mut state = ListState::default();
        state.select(Some(0));
        let query = LineEditor::new("docker");
        let theme = Theme::detect();
        let mut terminal = Terminal::new(TestBackend::new(50, 12)).unwrap();
        terminal
            .draw(|f| {
                render_picker(
                    f, &query, &results, &memories, &mut state, false, false, &theme, 0,
                )
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            !text.contains("details"),
            "narrow view should have no pane:\n{text}"
        );
        assert!(text.contains("docker ps"), "row missing:\n{text}");
    }

    #[test]
    fn fit_spans_truncates_the_tail_with_an_ellipsis() {
        let theme = Theme::detect();
        let m = mem(
            1,
            "docker ps",
            Some("a very long description that overflows"),
            &["docker"],
        );
        let line = row_line(&m, "", &theme, 16);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.chars().count() <= 16, "overflowed: {text:?}");
        assert!(text.ends_with('…'), "no ellipsis: {text:?}");
        // The curated badge and command survive; the tail (why/tags) is trimmed first.
        assert!(
            text.starts_with("● docker"),
            "command clipped first: {text:?}"
        );
    }

    #[test]
    fn paging_clamps_at_both_ends() {
        let mut state = ListState::default();
        state.select(Some(0));
        page_selection(&mut state, 5, -PAGE);
        assert_eq!(state.selected(), Some(0));
        page_selection(&mut state, 5, PAGE);
        assert_eq!(state.selected(), Some(4));
    }

    #[test]
    fn form_edits_the_focused_field_and_cycles() {
        let mut s = FormState::new("docker ps", "", "");
        s.next();
        s.field().insert('h');
        s.field().insert('i');
        assert_eq!(s.description.text(), "hi");
        s.prev();
        assert_eq!(s.command.text(), "docker ps");
        s.prev();
        s.field().insert('x');
        assert_eq!(s.tags.text(), "x");
    }

    #[test]
    fn form_prefills_all_fields() {
        let s = FormState::new("docker ps", "list containers", "docker cleanup");
        let theme = Theme::detect();
        let mut terminal = Terminal::new(TestBackend::new(70, 12)).unwrap();
        terminal.draw(|f| render_form(f, &s, &theme)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("docker ps"));
        assert!(text.contains("list containers"));
        assert!(text.contains("docker cleanup"));
    }
}
