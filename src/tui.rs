use std::io;

use anyhow::Result;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use crate::memory::CommandMemory;
use crate::search;

/// Run the picker. Returns the index of the chosen Memory, or `None` if cancelled.
/// The UI draws on stderr so stdout stays clean for the selected command.
pub fn run(memories: &[CommandMemory]) -> Result<Option<usize>> {
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stderr()))?;

    let mut query = String::new();
    let mut results = filter(&query, memories);
    let mut state = ListState::default();
    state.select((!results.is_empty()).then_some(0));

    loop {
        terminal.draw(|f| render(f, &query, &results, memories, &mut state))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Char('c') if ctrl => return Ok(None),
            KeyCode::Enter => return Ok(state.selected().and_then(|i| results.get(i).copied())),
            KeyCode::Up => move_selection(&mut state, results.len(), -1),
            KeyCode::Down => move_selection(&mut state, results.len(), 1),
            KeyCode::Backspace => {
                query.pop();
                refilter(&query, memories, &mut results, &mut state);
            }
            KeyCode::Char(c) => {
                query.push(c);
                refilter(&query, memories, &mut results, &mut state);
            }
            _ => {}
        }
    }
}

fn filter(query: &str, memories: &[CommandMemory]) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..memories.len()).collect();
    }
    search::search(query, memories, 200)
        .into_iter()
        .filter_map(|hit| memories.iter().position(|m| m.id == hit.id))
        .collect()
}

fn refilter(query: &str, memories: &[CommandMemory], results: &mut Vec<usize>, state: &mut ListState) {
    *results = filter(query, memories);
    let selected = if results.is_empty() {
        None
    } else {
        Some(state.selected().unwrap_or(0).min(results.len() - 1))
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

fn render(
    f: &mut Frame,
    query: &str,
    results: &[usize],
    memories: &[CommandMemory],
    state: &mut ListState,
) {
    let [top, bottom] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(f.area());

    let header = Paragraph::new(format!("search: {query}"))
        .block(Block::bordered().title("recall — ↑/↓ move · enter print · esc cancel"));
    f.render_widget(header, top);

    let items: Vec<ListItem> = results.iter().map(|&i| ListItem::new(row_line(&memories[i]))).collect();
    let title = format!("{} match{}", results.len(), if results.len() == 1 { "" } else { "es" });
    let list = List::new(items)
        .block(Block::bordered().title(title))
        .highlight_symbol("▌ ")
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(list, bottom, state);
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

/// Restores cooked mode and the main screen on drop — including during a panic.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stderr(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stderr(), LeaveAlternateScreen);
    }
}

/// What the add form collected. Tags are raw text, split and normalized by the caller.
pub struct AddForm {
    pub command: String,
    pub description: String,
    pub tags: String,
}

/// Interactive capture: fill in command / description / tags. Returns `None` if
/// cancelled. Draws on stderr, like the picker.
pub fn add_form(initial_command: &str) -> Result<Option<AddForm>> {
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stderr()))?;
    let mut form = FormState::new(initial_command);

    loop {
        terminal.draw(|f| render_form(f, &form))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Char('c') if ctrl => return Ok(None),
            KeyCode::Enter if !form.command.trim().is_empty() => {
                return Ok(Some(form.into_add_form()));
            }
            KeyCode::Tab | KeyCode::Down => form.next(),
            KeyCode::BackTab | KeyCode::Up => form.prev(),
            KeyCode::Backspace => form.backspace(),
            KeyCode::Char(c) => form.insert(c),
            _ => {}
        }
    }
}

struct FormState {
    command: String,
    description: String,
    tags: String,
    focus: usize,
}

impl FormState {
    fn new(command: &str) -> Self {
        Self {
            command: command.to_string(),
            description: String::new(),
            tags: String::new(),
            focus: 0,
        }
    }

    fn field(&mut self) -> &mut String {
        match self.focus {
            0 => &mut self.command,
            1 => &mut self.description,
            _ => &mut self.tags,
        }
    }

    fn insert(&mut self, c: char) {
        self.field().push(c);
    }

    fn backspace(&mut self) {
        self.field().pop();
    }

    fn next(&mut self) {
        self.focus = (self.focus + 1) % 3;
    }

    fn prev(&mut self) {
        self.focus = (self.focus + 2) % 3;
    }

    fn into_add_form(self) -> AddForm {
        AddForm {
            command: self.command.trim().to_string(),
            description: self.description,
            tags: self.tags,
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

    f.render_widget(field(&form.command, "command", form.focus == 0), command);
    f.render_widget(field(&form.description, "why (description)", form.focus == 1), description);
    f.render_widget(field(&form.tags, "tags (space or comma separated)", form.focus == 2), tags);
    f.render_widget(Paragraph::new("tab move · enter save · esc cancel"), help);
}

fn field<'a>(value: &'a str, label: &'a str, focused: bool) -> Paragraph<'a> {
    let cursor = if focused { "▊" } else { "" };
    let border = if focused {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    Paragraph::new(format!("{value}{cursor}")).block(Block::bordered().title(label).border_style(border))
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
        let memories = vec![mem(1, "docker ps", None, &[]), mem(2, "git stash", None, &[])];
        assert_eq!(filter("", &memories), vec![0, 1]);
    }

    #[test]
    fn query_ranks_the_best_match_first() {
        let memories = vec![
            mem(1, "docker ps", Some("list containers"), &["docker"]),
            mem(2, "git stash", Some("shelve changes"), &["git"]),
        ];
        // "git stash" (index 1) is the strong match and must come first; fuzzy
        // matching may still include weaker rows below it.
        assert_eq!(filter("stash", &memories).first(), Some(&1));
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
    fn renders_query_and_a_result_row() {
        let memories = vec![mem(1, "docker ps", Some("list running containers"), &["docker"])];
        let results = vec![0usize];
        let mut state = ListState::default();
        state.select(Some(0));

        let mut terminal = Terminal::new(TestBackend::new(70, 8)).unwrap();
        terminal
            .draw(|f| render(f, "docker", &results, &memories, &mut state))
            .unwrap();

        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("search: docker"), "header missing:\n{text}");
        assert!(text.contains("docker ps"), "result row missing:\n{text}");
    }

    #[test]
    fn form_edits_the_focused_field_and_cycles() {
        let mut s = FormState::new("docker ps");
        s.next();
        s.insert('h');
        s.insert('i');
        assert_eq!(s.description, "hi");
        s.prev();
        s.backspace();
        assert_eq!(s.command, "docker p");
        s.prev();
        s.insert('x');
        assert_eq!(s.tags, "x");
    }

    #[test]
    fn form_renders_its_fields_and_initial_command() {
        let s = FormState::new("docker ps");
        let mut terminal = Terminal::new(TestBackend::new(70, 12)).unwrap();
        terminal.draw(|f| render_form(f, &s)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("docker ps"), "command missing:\n{text}");
        assert!(text.contains("command"));
        assert!(text.contains("tags"));
    }
}
