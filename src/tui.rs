use std::io;

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

/// What the user chose in the picker. Indices point into the `memories` slice.
pub enum Outcome {
    Select(usize),
    Edit(usize),
    Delete(usize),
    Cancel,
}

/// Run the picker. Draws in an inline viewport on stderr, so stdout stays clean for
/// the selected command and the panel collapses back to your prompt on exit.
pub fn run(memories: &[CommandMemory]) -> Result<Outcome> {
    let _guard = RawGuard::enter()?;
    let mut terminal = inline_terminal(18)?;

    let haystacks = search::build_haystacks(memories);
    let mut query = LineEditor::default();
    let mut results = filter(query.text(), memories, &haystacks);
    let mut state = ListState::default();
    state.select((!results.is_empty()).then_some(0));
    let mut confirming = false;

    let outcome = loop {
        terminal.draw(|f| render_picker(f, &query, &results, memories, &mut state, confirming))?;

        match event::read()? {
            Event::Paste(text) if !confirming => {
                query.insert_str(&text);
                refilter(query.text(), memories, &haystacks, &mut results, &mut state);
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let selected = state.selected().and_then(|i| results.get(i).copied());

                if confirming {
                    let confirmed = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
                    if let Some(i) = selected.filter(|_| confirmed) {
                        break Outcome::Delete(i);
                    }
                    confirming = false;
                    continue;
                }

                // Selection and action keys take priority; anything else is text
                // editing routed through the shared line editor.
                match key.code {
                    KeyCode::Esc => break Outcome::Cancel,
                    KeyCode::Char('c') if ctrl => break Outcome::Cancel,
                    KeyCode::Enter => {
                        if let Some(i) = selected {
                            break Outcome::Select(i);
                        }
                    }
                    KeyCode::Char('o') if ctrl => {
                        if let Some(i) = selected {
                            break Outcome::Edit(i);
                        }
                    }
                    KeyCode::Char('x') if ctrl => confirming = selected.is_some(),
                    KeyCode::Up => move_selection(&mut state, results.len(), -1),
                    KeyCode::Down => move_selection(&mut state, results.len(), 1),
                    _ => {
                        if query.handle_key(key) == Handled::Edited {
                            refilter(query.text(), memories, &haystacks, &mut results, &mut state);
                        }
                    }
                }
            }
            _ => {}
        }
    };

    terminal.clear()?;
    Ok(outcome)
}

fn filter(query: &str, memories: &[CommandMemory], haystacks: &[String]) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..memories.len()).collect();
    }
    search::ranked_indices(query, memories, haystacks, 200)
}

fn refilter(
    query: &str,
    memories: &[CommandMemory],
    haystacks: &[String],
    results: &mut Vec<usize>,
    state: &mut ListState,
) {
    *results = filter(query, memories, haystacks);
    reselect(state, results.len());
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
        "↑/↓ move · enter print · ^e edit · ^x delete · esc cancel"
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
        None => out.push_str("(no description yet — press ^e to add one)\n\n"),
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
