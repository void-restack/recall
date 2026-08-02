use std::io;
use std::time::Duration;

use anyhow::Result;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};

use crate::line_editor::{Handled, LineEditor};
use crate::memory::CommandMemory;
use crate::search;
use crate::theme::Theme;

type Term = Terminal<TtyProbeBackend>;

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

/// Run the picker over one persistent viewport. Draws on stderr so stdout stays clean
/// for the chosen command (which a shell widget captures with `$(recall)`). Returns the
/// chosen memory, or `None` on cancel. `now` stamps relative times in the preview
/// without the TUI reading the clock.
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
            // A 1ms (not zero) timeout: with crossterm's use-dev-tty source, poll(0) can
            // spuriously report "no events" and defeat coalescing.
            if !event::poll(Duration::from_millis(1))? {
                break;
            }
            event = event::read()?;
        }
        app.settle();
    };

    terminal.clear()?;
    // Apply the session's deletions now that the viewport is closing. Anything the
    // user undid with Ctrl-Z was popped off `deleted` and is never touched.
    for id in &app.deleted {
        store.delete(*id)?;
    }
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
    /// Ids marked for deletion — hidden from the view now, deleted from the store on
    /// exit, so Ctrl-Z can restore them within the session.
    deleted: Vec<i64>,
    /// A transient footer message (e.g. the undo hint), cleared on the next key.
    status: Option<String>,
    /// Distinct tags across the collection, offered as form autocompletions.
    all_tags: Vec<String>,
    theme: Theme,
    now: i64,
}

impl App {
    fn new(memories: Vec<CommandMemory>, now: i64) -> Self {
        let haystacks = search::build_haystacks(&memories);
        let all_tags = crate::memory::collect_tags(&memories);
        let mut app = Self {
            memories,
            haystacks,
            query: LineEditor::default(),
            results: Vec::new(),
            state: ListState::default(),
            mode: Mode::Picker,
            dirty: false,
            drafts_only: false,
            deleted: Vec::new(),
            status: None,
            all_tags,
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
            self.status.as_deref(),
            &self.theme,
            self.now,
        );
    }

    fn handle_event(&mut self, event: Event, store: &dyn PickerStore) -> Result<Flow> {
        match self.mode {
            Mode::Picker => self.handle_picker(event),
            Mode::Editing { .. } => self.handle_editing(event, store),
            Mode::ConfirmDelete => self.handle_confirm(event),
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
                self.status = None; // any keystroke dismisses the transient toast
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
                    KeyCode::Char('z') if ctrl => self.undo_delete(),
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
                let alt = key.modifiers.contains(KeyModifiers::ALT);
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Picker,
                    KeyCode::Char('c') if ctrl => self.mode = Mode::Picker,
                    KeyCode::Enter if alt => {
                        if let Mode::Editing { form, .. } = &mut self.mode {
                            form.newline();
                        }
                    }
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
                            form.handle_field_key(key);
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn handle_confirm(&mut self, event: Event) -> Result<Flow> {
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
        {
            let confirmed = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
            if confirmed && let Some(m) = self.selected_memory() {
                let (id, label) = (m.id, doomed_label(&m.command));
                let pos = self.state.selected().unwrap_or(0);
                // Hide it now; the store delete happens on exit, so Ctrl-Z can undo.
                self.deleted.push(id);
                self.recompute_results();
                self.select_near(pos);
                self.status = Some(format!("deleted {label} · ^z undo"));
            }
            self.mode = Mode::Picker;
        }
        Ok(Flow::Continue)
    }

    /// Restore the most recently deleted memory (Ctrl-Z), reselecting it by id.
    fn undo_delete(&mut self) {
        if let Some(id) = self.deleted.pop() {
            self.recompute_results();
            let idx = self.results.iter().position(|&i| self.memories[i].id == id);
            self.state
                .select(idx.or_else(|| (!self.results.is_empty()).then_some(0)));
            self.status = Some("restored".to_string());
        }
    }

    fn begin_edit(&mut self) {
        if let Some(m) = self.selected_memory() {
            let form = FormState::new(
                &m.command,
                m.description.as_deref().unwrap_or(""),
                &m.tags.join(" "),
                self.all_tags.clone(),
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
        self.all_tags = crate::memory::collect_tags(&self.memories);
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
        if !self.deleted.is_empty() {
            results.retain(|&i| !self.deleted.contains(&self.memories[i].id));
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

/// Place the terminal caret at `col` on the first line of a bordered box.
fn set_line_cursor(f: &mut Frame, area: Rect, col: usize) {
    set_area_cursor(f, area, 0, col);
}

/// Place the terminal caret at `(row, col)` inside a bordered box, so the active
/// line editor shows a real, native cursor. Clamped to the box interior.
fn set_area_cursor(f: &mut Frame, area: Rect, row: usize, col: usize) {
    let inner = Block::bordered().inner(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let x = (inner.x + col as u16).min(inner.right().saturating_sub(1));
    let y = (inner.y + row as u16).min(inner.bottom().saturating_sub(1));
    f.set_cursor_position(Position { x, y });
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
    status: Option<&str>,
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
    let search_line = if query.text().is_empty() {
        // Empty-query placeholder: teach what the box does and how the list is ordered.
        Line::from(vec![
            Span::raw(prompt),
            Span::styled("type to search · browsing by recent use", theme.dim),
        ])
    } else {
        Line::raw(format!("{prompt}{}", query.text()))
    };
    f.render_widget(
        Paragraph::new(search_line)
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
            confirming,
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
            confirming,
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
            confirming,
            theme,
        );
    }

    let help_line = if confirming {
        let name = state
            .selected()
            .and_then(|s| results.get(s))
            .map(|&i| doomed_label(&memories[i].command))
            .unwrap_or_default();
        Line::styled(format!("delete {name}?  y / n"), theme.danger)
    } else if let Some(status) = status {
        Line::styled(status.to_string(), theme.accent)
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
    confirming: bool,
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
    // While confirming a delete, the selected row is the doomed one — paint it red.
    let highlight = if confirming {
        theme.danger.add_modifier(Modifier::REVERSED)
    } else {
        theme.selection
    };
    let list = List::new(items)
        .block(Block::bordered().title(title))
        .highlight_symbol("▌ ")
        .highlight_style(highlight);
    f.render_stateful_widget(list, area, state);

    // Teach when there's nothing to show.
    if results.is_empty() {
        let hint = if !query.trim().is_empty() {
            format!(
                "no matches for \"{}\" — try fewer or different words",
                truncate_str(query.trim(), 30)
            )
        } else if drafts_only {
            "no drafts — every memory has a why".to_string()
        } else {
            "nothing here — capture a command with `recall add`".to_string()
        };
        f.render_widget(
            Paragraph::new(Line::styled(hint, theme.dim)).wrap(Wrap { trim: true }),
            Block::bordered().inner(area),
        );
    }
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
    let (cmd, extra) = split_command(&m.command);
    let mut cmd_spans = program_spans(cmd, theme);
    if extra > 0 {
        cmd_spans.push(Span::styled(format!(" ⏎{extra}"), theme.accent));
    }
    let first = Line::from(fit_spans(cmd_spans, width));
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

    // Command verbatim: first line with the program bold, further lines as-is.
    for (i, line) in m.command.lines().enumerate() {
        if i == 0 {
            lines.push(program_first(line, theme));
        } else {
            lines.push(Line::raw(line.to_string()));
        }
    }
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
    // A multiline command collapses to its first line plus a ⏎N line-count marker.
    let (first, extra) = split_command(&m.command);
    spans.extend(command_spans(first, query, theme));
    if extra > 0 {
        spans.push(Span::styled(format!(" ⏎{extra}"), theme.accent));
    }
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

/// First line of a command and how many further lines it has, for compact rows.
fn split_command(command: &str) -> (&str, usize) {
    match command.split_once('\n') {
        Some((first, rest)) => (first, rest.lines().count()),
        None => (command, 0),
    }
}

/// A short, quoted first-line label of a command, for the confirm and undo messages.
fn doomed_label(command: &str) -> String {
    let (first, _) = split_command(command);
    format!("\"{}\"", truncate_str(first, 40))
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

/// Interactive capture/edit form, pre-filled with the given values. `tag_pool` seeds
/// tag autocompletion. Returns `None` if cancelled. Draws in an inline viewport.
pub fn add_form(
    command: &str,
    description: &str,
    tags: &str,
    tag_pool: &[String],
) -> Result<Option<AddForm>> {
    let _guard = RawGuard::enter()?;
    let mut terminal = inline_terminal(14)?;
    let theme = Theme::detect();
    let mut form = FormState::new(command, description, tags, tag_pool.to_vec());

    let result = loop {
        terminal.draw(|f| render_form(f, &form, &theme))?;

        match event::read()? {
            Event::Paste(text) => form.field().insert_str(&text),
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let alt = key.modifiers.contains(KeyModifiers::ALT);
                match key.code {
                    KeyCode::Esc => break None,
                    KeyCode::Char('c') if ctrl => break None,
                    KeyCode::Enter if alt => form.newline(),
                    KeyCode::Enter if !form.command_is_blank() => break Some(form.into_add_form()),
                    KeyCode::Tab | KeyCode::Down => form.next(),
                    KeyCode::BackTab | KeyCode::Up => form.prev(),
                    _ => form.handle_field_key(key),
                }
            }
            _ => {}
        }
    };

    terminal.clear()?;
    Ok(result)
}

const TAGS_FIELD: usize = 2;

struct FormState {
    command: LineEditor,
    description: LineEditor,
    tags: LineEditor,
    focus: usize,
    /// Existing tags, offered as ghost completions in the tags field.
    tag_pool: Vec<String>,
    /// Which matching tag the ghost is currently showing (cycled with Ctrl-N/P).
    tag_pick: usize,
}

impl FormState {
    fn new(command: &str, description: &str, tags: &str, tag_pool: Vec<String>) -> Self {
        Self {
            command: LineEditor::new(command),
            description: LineEditor::new(description),
            tags: LineEditor::new(tags),
            focus: 0,
            tag_pool,
            tag_pick: 0,
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

    /// The tag token currently being typed — the text after the last separator.
    fn tag_token(&self) -> &str {
        self.tags.text().rsplit([',', ' ']).next().unwrap_or("")
    }

    /// Pool tags that extend the current token and aren't already present.
    fn tag_candidates(&self) -> Vec<&String> {
        let token = self.tag_token().to_lowercase();
        if token.is_empty() {
            return Vec::new();
        }
        let present: std::collections::HashSet<&str> = self
            .tags
            .text()
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .collect();
        self.tag_pool
            .iter()
            .filter(|t| {
                t.starts_with(&token) && t.as_str() != token && !present.contains(t.as_str())
            })
            .collect()
    }

    /// The ghost suffix shown after the typed token, if any candidate matches.
    fn tag_ghost(&self) -> Option<String> {
        let candidates = self.tag_candidates();
        if candidates.is_empty() {
            return None;
        }
        let token = self.tag_token().to_lowercase();
        candidates[self.tag_pick % candidates.len()]
            .strip_prefix(token.as_str())
            .map(str::to_string)
    }

    /// Accept the current ghost, completing the tag in place.
    fn accept_tag_ghost(&mut self) -> bool {
        if let Some(ghost) = self.tag_ghost() {
            self.tags.insert_str(&ghost);
            self.tag_pick = 0;
            true
        } else {
            false
        }
    }

    fn cycle_tag(&mut self, delta: isize) {
        let n = self.tag_candidates().len();
        if n == 0 {
            return;
        }
        self.tag_pick = (self.tag_pick as isize + delta).rem_euclid(n as isize) as usize;
    }

    /// Route an editing key to the focused field. In the tags field, → accepts the
    /// ghost completion and Ctrl-N/P cycle candidates.
    fn handle_field_key(&mut self, key: KeyEvent) {
        if self.focus == TAGS_FIELD {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Right if self.tags.at_end() && self.tag_ghost().is_some() => {
                    self.accept_tag_ghost();
                    return;
                }
                KeyCode::Char('n') if ctrl => return self.cycle_tag(1),
                KeyCode::Char('p') if ctrl => return self.cycle_tag(-1),
                _ => {}
            }
            if self.tags.handle_key(key) == Handled::Edited {
                self.tag_pick = 0; // a fresh edit resets the ghost to the best match
            }
            return;
        }
        self.field().handle_key(key);
    }

    /// Alt-Enter adds a line to the command field only; the why and tags stay single-line.
    fn newline(&mut self) {
        if self.focus == 0 {
            self.command.insert('\n');
        }
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
    // The command box grows with its line count (capped) so multi-line commands show.
    let command_rows = form.command.line_count().clamp(1, 5) as u16;
    let [command, description, tags, help] = Layout::vertical([
        Constraint::Length(command_rows + 2),
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
    // The tags field trails a dim ghost of the best-matching existing tag (→ accepts).
    let tags_focused = form.focus == TAGS_FIELD;
    let mut tag_spans = vec![Span::raw(form.tags.text().to_string())];
    if tags_focused && let Some(ghost) = form.tag_ghost() {
        tag_spans.push(Span::styled(ghost, theme.dim));
    }
    let tag_border = if tags_focused {
        theme.accent
    } else {
        Style::new()
    };
    f.render_widget(
        Paragraph::new(Line::from(tag_spans)).block(
            Block::bordered()
                .title("tags (space or comma separated)")
                .border_style(tag_border),
        ),
        tags,
    );

    let (focused_area, focused) = match form.focus {
        0 => (command, &form.command),
        1 => (description, &form.description),
        _ => (tags, &form.tags),
    };
    let (row, col) = focused.cursor_row_col();
    set_area_cursor(f, focused_area, row, col);

    let hint = if tags_focused {
        "→ accept tag · ^n/^p cycle · enter save · esc cancel"
    } else {
        "tab move · alt+⏎ newline · enter save · esc cancel"
    };
    f.render_widget(Paragraph::new(Line::styled(hint, theme.dim)), help);
}

fn field<'a>(value: &'a str, label: &'a str, focused: bool, theme: &Theme) -> Paragraph<'a> {
    let border = if focused { theme.accent } else { Style::new() };
    Paragraph::new(value).block(Block::bordered().title(label).border_style(border))
}

/// An inline viewport anchored under the prompt (fzf-style), sized to fit but never
/// taller than the terminal. Draws to stderr so stdout stays free for the selection.
fn inline_terminal(desired_rows: u16) -> Result<Term> {
    let rows = size().map(|(_, r)| r).unwrap_or(24);
    let height = desired_rows.min(rows.saturating_sub(1)).max(3);
    // Open /dev/tty for the cursor probe (see TtyProbeBackend). Its own read+write handle
    // keeps the probe off stdin/stdout, which the widget redirects.
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")?;
    let backend = TtyProbeBackend {
        inner: CrosstermBackend::new(io::stderr()),
        tty,
        rows,
    };
    let terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )?;
    Ok(terminal)
}

/// Wraps the crossterm backend to replace the one operation that hangs from a shell
/// keybinding: the inline viewport's cursor-position probe. crossterm writes `ESC[6n`
/// to stdout but reads the reply from stdin with no read timeout, so inside a ZLE widget
/// (where the child often isn't the terminal's foreground group) that read takes SIGTTIN
/// and stops the process — the "frozen picker". We instead probe on our own `/dev/tty`
/// handle, bounded and signal-safe, and everything else delegates unchanged.
struct TtyProbeBackend {
    inner: CrosstermBackend<io::Stderr>,
    tty: std::fs::File,
    rows: u16,
}

impl Backend for TtyProbeBackend {
    type Error = io::Error;

    fn get_cursor_position(&mut self) -> io::Result<ratatui::layout::Position> {
        Ok(ratatui::layout::Position {
            x: 0,
            y: cursor_row_0based(&mut self.tty, self.rows),
        })
    }

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
    {
        self.inner.draw(content)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }
    fn set_cursor_position<P: Into<ratatui::layout::Position>>(
        &mut self,
        position: P,
    ) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }
    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }
    fn clear_region(&mut self, clear_type: ratatui::backend::ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }
    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }
    fn size(&self) -> io::Result<ratatui::layout::Size> {
        self.inner.size()
    }
    fn window_size(&mut self) -> io::Result<ratatui::backend::WindowSize> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Query the cursor's 0-based row on a single `/dev/tty` handle: write `ESC[6n`, then
/// read the `ESC[row;colR` reply on the *same* fd, bounded to 200 ms. This replaces
/// crossterm's inline probe, which splits the write/read across fd 1 and fd 0 with the
/// timeout on `poll()` not `read()`. SIGTTIN/SIGTTOU are ignored for the duration so a
/// background-group read can't stop us. Falls back to the bottom row on no reply.
fn cursor_row_0based(tty: &mut std::fs::File, rows: u16) -> u16 {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;

    let (old_ttin, old_ttou) = unsafe {
        (
            libc::signal(libc::SIGTTIN, libc::SIG_IGN),
            libc::signal(libc::SIGTTOU, libc::SIG_IGN),
        )
    };
    let parsed = (|| -> Option<u16> {
        tty.write_all(b"\x1b[6n").ok()?;
        tty.flush().ok()?;
        let mut pfd = libc::pollfd {
            fd: tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut pfd, 1, 200) } <= 0 {
            return None;
        }
        let mut buf = [0u8; 32];
        let n = tty.read(&mut buf).ok().filter(|&n| n > 0)?;
        // Reply is ESC [ <row> ; <col> R (1-based).
        let open = buf[..n].iter().position(|&b| b == b'[')?;
        let rest = &buf[open + 1..n];
        let semi = rest.iter().position(|&b| b == b';')?;
        std::str::from_utf8(&rest[..semi]).ok()?.trim().parse().ok()
    })();
    unsafe {
        libc::signal(libc::SIGTTIN, old_ttin);
        libc::signal(libc::SIGTTOU, old_ttou);
    }
    parsed
        .map(|r: u16| r.saturating_sub(1))
        .unwrap_or_else(|| rows.saturating_sub(1))
}

/// Enables raw mode and bracketed paste, restoring both on drop — including during a
/// panic. No alternate screen: the inline viewport keeps your scrollback visible above it.
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
    fn delete_then_undo_restores_and_reselects() {
        let memories = vec![
            mem(1, "a", None, &[]),
            mem(2, "b", None, &[]),
            mem(3, "c", None, &[]),
        ];
        let mut app = App::new(memories, 0);
        app.state.select(Some(1));
        app.mode = Mode::ConfirmDelete;
        app.handle_confirm(key(KeyCode::Char('y'))).unwrap();
        assert_eq!(app.results.len(), 2);
        assert!(app.deleted.contains(&2));
        assert!(app.status.is_some());

        let ctrl_z = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        ));
        app.handle_picker(ctrl_z).unwrap();
        assert!(app.deleted.is_empty());
        assert_eq!(app.results.len(), 3);
        assert_eq!(app.selected_memory().map(|m| m.id), Some(2));
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
                    f, &query, &results, &memories, &mut state, false, false, None, &theme, 0,
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
                    f, &query, &results, &memories, &mut state, false, false, None, &theme, 0,
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
    fn multiline_command_collapses_to_a_line_marker_in_a_row() {
        let theme = Theme::detect();
        let m = mem(1, "docker run \\\n  -it ubuntu", Some("shell"), &[]);
        let line = row_line(&m, "", &theme, 80);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("docker run"));
        assert!(text.contains("⏎1"), "missing line marker: {text:?}");
        assert!(!text.contains('\n'), "row spilled a newline: {text:?}");
    }

    #[test]
    fn tag_ghost_completes_from_the_pool() {
        let pool = vec![
            "docker".to_string(),
            "deploy".to_string(),
            "git".to_string(),
        ];
        let mut form = FormState::new("cmd", "why", "doc", pool);
        form.focus = TAGS_FIELD;
        assert_eq!(form.tag_ghost().as_deref(), Some("ker"));
        // Cycle to the other "d…" candidate.
        form.cycle_tag(1);
        assert_eq!(form.tag_token(), "doc");
        // "deploy" doesn't extend "doc", so the only candidate is "docker".
        assert_eq!(form.tag_candidates().len(), 1);
        form.accept_tag_ghost();
        assert_eq!(form.tags.text(), "docker");
    }

    #[test]
    fn tag_ghost_skips_already_present_tags() {
        let pool = vec!["docker".to_string()];
        let mut form = FormState::new("cmd", "why", "docker doc", pool);
        form.focus = TAGS_FIELD;
        // "docker" is already present, so typing "doc" again offers no completion.
        assert!(form.tag_ghost().is_none());
    }

    #[test]
    fn no_matches_teaches_in_the_empty_list() {
        let memories = vec![mem(1, "docker ps", Some("containers"), &["docker"])];
        let mut app = App::new(memories, 0);
        for c in "zznomatch".chars() {
            app.handle_picker(key(KeyCode::Char(c))).unwrap();
        }
        app.settle();
        assert!(app.results.is_empty());
        let mut terminal = Terminal::new(TestBackend::new(90, 12)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("no matches"), "missing teach text:\n{text}");
    }

    #[test]
    fn alt_enter_adds_a_line_only_to_the_command() {
        let mut form = FormState::new("docker run", "why", "", Vec::new());
        form.newline();
        assert_eq!(form.command.line_count(), 2);
        form.next(); // focus the why field
        form.newline();
        assert_eq!(form.description.line_count(), 1);
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
        let mut s = FormState::new("docker ps", "", "", Vec::new());
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
        let s = FormState::new("docker ps", "list containers", "docker cleanup", Vec::new());
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
