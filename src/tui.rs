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
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};

use crate::line_editor::{Handled, LineEditor};
use crate::memory::CommandMemory;
use crate::search;

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
pub fn run(store: &dyn PickerStore, memories: Vec<CommandMemory>) -> Result<Option<CommandMemory>> {
    let _guard = RawGuard::enter()?;
    let mut terminal = inline_terminal(18)?;
    let mut app = App::new(memories);

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
}

impl App {
    fn new(memories: Vec<CommandMemory>) -> Self {
        let haystacks = search::build_haystacks(&memories);
        let results: Vec<usize> = (0..memories.len()).collect();
        let mut state = ListState::default();
        state.select((!results.is_empty()).then_some(0));
        Self {
            memories,
            haystacks,
            query: LineEditor::default(),
            results,
            state,
            mode: Mode::Picker,
            dirty: false,
        }
    }

    fn selected_memory(&self) -> Option<&CommandMemory> {
        self.state
            .selected()
            .and_then(|i| self.results.get(i))
            .map(|&i| &self.memories[i])
    }

    fn render(&mut self, f: &mut Frame) {
        if let Mode::Editing { form, .. } = &self.mode {
            render_form(f, form);
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
        self.results = filter(self.query.text(), &self.memories, &self.haystacks);
    }

    /// Select the row at `pos`, clamped — the neighbor of a deleted row.
    fn select_near(&mut self, pos: usize) {
        let sel = (!self.results.is_empty()).then(|| pos.min(self.results.len() - 1));
        self.state.select(sel);
    }
}

fn filter(query: &str, memories: &[CommandMemory], haystacks: &[String]) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..memories.len()).collect();
    }
    search::ranked_indices(query, memories, haystacks, 200)
}

fn reselect(state: &mut ListState, len: usize) {
    let selected = if len == 0 {
        None
    } else {
        Some(state.selected().unwrap_or(0).min(len - 1))
    };
    state.select(selected);
}

fn move_selection(state: &mut ListState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let current = state.selected().unwrap_or(0) as isize;
    let next = (current + delta).rem_euclid(len as isize) as usize;
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

fn render_picker(
    f: &mut Frame,
    query: &LineEditor,
    results: &[usize],
    memories: &[CommandMemory],
    state: &mut ListState,
    confirming: bool,
) {
    let [top, middle, help] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let prompt = "search: ";
    f.render_widget(
        Paragraph::new(format!("{prompt}{}", query.text()))
            .block(Block::bordered().title("recall")),
        top,
    );
    set_line_cursor(f, top, prompt.chars().count() + query.cursor_col());

    let [list_area, preview_area] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(middle);

    let preview = preview_text(results, memories, state.selected());
    f.render_widget(
        Paragraph::new(preview)
            .block(Block::bordered().title("details"))
            .wrap(Wrap { trim: false }),
        preview_area,
    );

    let items: Vec<ListItem> = results
        .iter()
        .map(|&i| ListItem::new(row_line(&memories[i])))
        .collect();
    let title = format!(
        "{} match{}",
        results.len(),
        if results.len() == 1 { "" } else { "es" }
    );
    let list = List::new(items)
        .block(Block::bordered().title(title))
        .highlight_symbol("▌ ")
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(list, list_area, state);

    let help_text = if confirming {
        "delete this memory?  y / n"
    } else {
        "↑/↓ move · enter print · ^o edit · ^x delete · esc cancel"
    };
    f.render_widget(Paragraph::new(help_text), help);
}

fn preview_text(results: &[usize], memories: &[CommandMemory], selected: Option<usize>) -> String {
    let Some(m) = selected.and_then(|s| results.get(s)).map(|&i| &memories[i]) else {
        return String::new();
    };
    let mut out = format!("{}\n\n", m.command);
    match m.description.as_deref().filter(|d| !d.is_empty()) {
        Some(desc) => out.push_str(&format!("{desc}\n\n")),
        None => out.push_str("(no description yet — press ^o to add one)\n\n"),
    }
    if !m.tags.is_empty() {
        out.push_str(&format!("tags: {}\n", m.tags.join(", ")));
    }
    out.push_str(&format!(
        "used {} time{}",
        m.use_count,
        if m.use_count == 1 { "" } else { "s" }
    ));
    out
}

fn row_line(m: &CommandMemory) -> String {
    let mut line = m.command.clone();
    if let Some(desc) = m.description.as_deref().filter(|d| !d.is_empty()) {
        line.push_str("  — ");
        line.push_str(desc);
    }
    if !m.tags.is_empty() {
        line.push_str(&format!("  [{}]", m.tags.join(", ")));
    }
    line
}

/// Browse shell history and pick a command to promote. Returns the chosen index,
/// or `None` if cancelled.
pub fn history_picker(entries: &[String]) -> Result<Option<usize>> {
    let _guard = RawGuard::enter()?;
    let mut terminal = inline_terminal(18)?;

    let mut query = LineEditor::default();
    let mut results: Vec<usize> = (0..entries.len()).collect();
    let mut state = ListState::default();
    state.select((!results.is_empty()).then_some(0));

    let result = loop {
        terminal.draw(|f| render_history(f, &query, &results, entries, &mut state))?;

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
) {
    let [top, list, help] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let prompt = "history: ";
    f.render_widget(
        Paragraph::new(format!("{prompt}{}", query.text()))
            .block(Block::bordered().title("promote a command into a memory")),
        top,
    );
    set_line_cursor(f, top, prompt.chars().count() + query.cursor_col());

    let items: Vec<ListItem> = results
        .iter()
        .map(|&i| ListItem::new(entries[i].as_str()))
        .collect();
    let widget = List::new(items)
        .block(Block::bordered().title(format!("{} shown", results.len())))
        .highlight_symbol("▌ ")
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(widget, list, state);

    f.render_widget(
        Paragraph::new("↑/↓ move · enter annotate & save · esc cancel"),
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
    let mut form = FormState::new(command, description, tags);

    let result = loop {
        terminal.draw(|f| render_form(f, &form))?;

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

fn render_form(f: &mut Frame, form: &FormState) {
    let [command, description, tags, help] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .areas(f.area());

    f.render_widget(
        field(form.command.text(), "command", form.focus == 0),
        command,
    );
    f.render_widget(
        field(
            form.description.text(),
            "why (description)",
            form.focus == 1,
        ),
        description,
    );
    f.render_widget(
        field(
            form.tags.text(),
            "tags (space or comma separated)",
            form.focus == 2,
        ),
        tags,
    );

    let (focused_area, focused) = match form.focus {
        0 => (command, &form.command),
        1 => (description, &form.description),
        _ => (tags, &form.tags),
    };
    set_line_cursor(f, focused_area, focused.cursor_col());

    f.render_widget(Paragraph::new("tab move · enter save · esc cancel"), help);
}

fn field<'a>(value: &'a str, label: &'a str, focused: bool) -> Paragraph<'a> {
    let border = if focused {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
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
        let mut app = App::new(memories);
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
    fn edit_returns_to_the_picker_reselecting_by_id() {
        let memories = vec![mem(1, "alpha", None, &[]), mem(2, "beta", None, &[])];
        let store = MockStore {
            memories: std::cell::RefCell::new(memories.clone()),
        };
        let mut app = App::new(memories);
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
        let mut app = App::new(memories);
        app.state.select(Some(1));
        app.mode = Mode::ConfirmDelete;
        app.handle_event(key(KeyCode::Char('y')), &store).unwrap();
        assert!(matches!(app.mode, Mode::Picker));
        assert_eq!(app.results.len(), 2);
        // Was on index 1 ("b"); after deleting it the cursor holds index 1 ("c").
        assert_eq!(app.selected_memory().map(|m| m.id), Some(3));
    }

    #[test]
    fn empty_query_shows_everything() {
        let memories = vec![
            mem(1, "docker ps", None, &[]),
            mem(2, "git stash", None, &[]),
        ];
        let haystacks = search::build_haystacks(&memories);
        assert_eq!(filter("", &memories, &haystacks), vec![0, 1]);
    }

    #[test]
    fn query_ranks_the_best_match_first() {
        let memories = vec![
            mem(1, "docker ps", Some("list containers"), &["docker"]),
            mem(2, "git stash", Some("shelve changes"), &["git"]),
        ];
        let haystacks = search::build_haystacks(&memories);
        assert_eq!(filter("stash", &memories, &haystacks).first(), Some(&1));
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
        let text = preview_text(&[0], &memories, Some(0));
        assert!(text.contains("docker ps"));
        assert!(text.contains("list running containers"));
        assert!(text.contains("used 0 times"));
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
        let mut terminal = Terminal::new(TestBackend::new(90, 12)).unwrap();
        terminal
            .draw(|f| render_picker(f, &query, &results, &memories, &mut state, false))
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
        let mut terminal = Terminal::new(TestBackend::new(70, 12)).unwrap();
        terminal.draw(|f| render_form(f, &s)).unwrap();
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
