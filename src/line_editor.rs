use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A single-line text buffer with a cursor and readline-style editing. Shared by
/// the picker's query box and every form field, so editing feels the same
/// everywhere. `cursor` is a byte offset into `text`, always on a char boundary.
#[derive(Debug, Default, Clone)]
pub struct LineEditor {
    text: String,
    cursor: usize,
}

/// What a keypress did to the buffer, so callers can refilter only when the text
/// actually changed (a bare cursor move shouldn't trigger a re-search).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    Edited,
    Moved,
    Ignored,
}

impl LineEditor {
    pub fn new(initial: &str) -> Self {
        Self {
            text: initial.to_string(),
            cursor: initial.len(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }

    /// The cursor's column, counted in characters — where the on-screen caret goes.
    pub fn cursor_col(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }

    /// The cursor's (row, column) for a field that may hold newlines.
    pub fn cursor_row_col(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        let row = before.matches('\n').count();
        let col = before
            .rsplit('\n')
            .next()
            .map_or(0, |line| line.chars().count());
        (row, col)
    }

    /// How many display lines the buffer spans (at least 1).
    pub fn line_count(&self) -> usize {
        self.text.matches('\n').count() + 1
    }

    /// Apply a standard editing/motion key. Returns `Ignored` for keys this editor
    /// doesn't own (Enter, Tab, arrows used for list nav, etc.) so the caller can.
    pub fn handle_key(&mut self, key: KeyEvent) -> Handled {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char(c) if !ctrl && !alt => {
                self.insert(c);
                Handled::Edited
            }
            KeyCode::Backspace if alt => self.edited(Self::delete_word_back),
            KeyCode::Backspace => self.edited(Self::backspace),
            KeyCode::Delete => self.edited(Self::delete),
            KeyCode::Char('w') if ctrl => self.edited(Self::delete_word_back),
            KeyCode::Char('u') if ctrl => self.edited(Self::kill_to_start),
            KeyCode::Char('k') if ctrl => self.edited(Self::kill_to_end),
            KeyCode::Char('d') if alt => self.edited(Self::delete_word_forward),
            KeyCode::Left if alt => self.moved(Self::word_left),
            KeyCode::Right if alt => self.moved(Self::word_right),
            KeyCode::Char('b') if alt => self.moved(Self::word_left),
            KeyCode::Char('f') if alt => self.moved(Self::word_right),
            KeyCode::Left => self.moved(Self::left),
            KeyCode::Right => self.moved(Self::right),
            KeyCode::Char('b') if ctrl => self.moved(Self::left),
            KeyCode::Char('f') if ctrl => self.moved(Self::right),
            KeyCode::Home => self.moved(Self::home),
            KeyCode::End => self.moved(Self::end),
            KeyCode::Char('a') if ctrl => self.moved(Self::home),
            KeyCode::Char('e') if ctrl => self.moved(Self::end),
            _ => Handled::Ignored,
        }
    }

    fn edited(&mut self, op: fn(&mut Self)) -> Handled {
        op(self);
        Handled::Edited
    }

    fn moved(&mut self, op: fn(&mut Self)) -> Handled {
        op(self);
        Handled::Moved
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Insert a run of text at the cursor — used for bracketed paste.
    pub fn insert_str(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.prev_boundary();
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self.next_boundary();
        self.text.replace_range(self.cursor..next, "");
    }

    fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.prev_boundary();
        }
    }

    fn right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor = self.next_boundary();
        }
    }

    fn home(&mut self) {
        self.cursor = 0;
    }

    fn end(&mut self) {
        self.cursor = self.text.len();
    }

    fn kill_to_end(&mut self) {
        self.text.truncate(self.cursor);
    }

    fn kill_to_start(&mut self) {
        self.text.replace_range(0..self.cursor, "");
        self.cursor = 0;
    }

    /// Ctrl-W / Alt-Backspace: delete back to the previous whitespace, so one chord
    /// removes a whole path or flag token.
    fn delete_word_back(&mut self) {
        let start = self.ws_word_start();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    fn delete_word_forward(&mut self) {
        let end = self.word_end();
        self.text.replace_range(self.cursor..end, "");
    }

    fn word_left(&mut self) {
        self.cursor = self.word_start();
    }

    fn word_right(&mut self) {
        self.cursor = self.word_end();
    }

    fn prev_boundary(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }

    /// Start of the whitespace-delimited token the cursor sits in (Ctrl-W target).
    fn ws_word_start(&self) -> usize {
        let mut i = self.cursor;
        i = self.scan_back(i, |c| c.is_whitespace());
        self.scan_back(i, |c| !c.is_whitespace())
    }

    /// Start of the alphanumeric word (Alt-B target): skip separators, then letters.
    fn word_start(&self) -> usize {
        let mut i = self.cursor;
        i = self.scan_back(i, |c| !is_word(c));
        self.scan_back(i, is_word)
    }

    /// End of the alphanumeric word (Alt-F / Alt-D target).
    fn word_end(&self) -> usize {
        let mut i = self.cursor;
        i = self.scan_forward(i, |c| !is_word(c));
        self.scan_forward(i, is_word)
    }

    /// Walk backward from `i` while the preceding char satisfies `pred`.
    fn scan_back(&self, mut i: usize, pred: impl Fn(char) -> bool) -> usize {
        while i > 0 {
            let c = self.text[..i].chars().next_back().unwrap();
            if !pred(c) {
                break;
            }
            i -= c.len_utf8();
        }
        i
    }

    /// Walk forward from `i` while the char at `i` satisfies `pred`.
    fn scan_forward(&self, mut i: usize, pred: impl Fn(char) -> bool) -> usize {
        while i < self.text.len() {
            let c = self.text[i..].chars().next().unwrap();
            if !pred(c) {
                break;
            }
            i += c.len_utf8();
        }
        i
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn ctrl(c: char) -> KeyEvent {
        key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn alt(code: KeyCode) -> KeyEvent {
        key(code, KeyModifiers::ALT)
    }

    #[test]
    fn types_and_backspaces() {
        let mut e = LineEditor::default();
        for c in "docker".chars() {
            e.insert(c);
        }
        assert_eq!(e.text(), "docker");
        e.handle_key(key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(e.text(), "docke");
        assert_eq!(e.cursor_col(), 5);
    }

    #[test]
    fn inserts_at_the_cursor() {
        let mut e = LineEditor::new("dockerps");
        e.handle_key(ctrl('a'));
        for _ in 0..6 {
            e.handle_key(key(KeyCode::Right, KeyModifiers::NONE));
        }
        e.insert(' ');
        assert_eq!(e.text(), "docker ps");
    }

    #[test]
    fn home_end_and_forward_delete() {
        let mut e = LineEditor::new("abc");
        e.handle_key(ctrl('a'));
        assert_eq!(e.cursor_col(), 0);
        e.handle_key(key(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(e.text(), "bc");
        e.handle_key(ctrl('e'));
        assert_eq!(e.cursor_col(), 2);
    }

    #[test]
    fn kill_to_start_and_end() {
        let mut e = LineEditor::new("docker system prune");
        // Move to just after "docker".
        e.handle_key(ctrl('a'));
        for _ in 0..6 {
            e.handle_key(key(KeyCode::Right, KeyModifiers::NONE));
        }
        e.handle_key(ctrl('k'));
        assert_eq!(e.text(), "docker");
        e.handle_key(ctrl('u'));
        assert_eq!(e.text(), "");
    }

    #[test]
    fn ctrl_w_deletes_a_whole_path_token() {
        let mut e = LineEditor::new("cp /a/b/c ");
        e.handle_key(ctrl('w'));
        assert_eq!(e.text(), "cp ");
    }

    #[test]
    fn alt_word_motions_and_delete() {
        let mut e = LineEditor::new("git commit --amend");
        e.handle_key(alt(KeyCode::Left)); // to start of "amend"
        e.handle_key(alt(KeyCode::Char('d'))); // delete "amend"
        assert_eq!(e.text(), "git commit --");
        e.handle_key(alt(KeyCode::Left)); // start of "commit"
        e.handle_key(alt(KeyCode::Left)); // start of "git"
        assert_eq!(e.cursor_col(), 0);
    }

    #[test]
    fn respects_utf8_boundaries() {
        let mut e = LineEditor::new("café");
        e.handle_key(key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(e.text(), "caf");
        e.insert('é');
        e.insert('☕');
        assert_eq!(e.text(), "café☕");
        e.handle_key(key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(e.text(), "café");
    }

    #[test]
    fn tracks_rows_and_columns_across_newlines() {
        let mut e = LineEditor::new("docker run \\");
        e.insert('\n');
        e.insert(' ');
        e.insert('x');
        assert_eq!(e.line_count(), 2);
        assert_eq!(e.cursor_row_col(), (1, 2));
    }

    #[test]
    fn paste_inserts_a_run() {
        let mut e = LineEditor::new("a");
        e.insert_str("bcd");
        assert_eq!(e.text(), "abcd");
        assert_eq!(e.cursor_col(), 4);
    }

    #[test]
    fn reports_edited_versus_moved() {
        let mut e = LineEditor::new("hi");
        assert_eq!(
            e.handle_key(key(KeyCode::Left, KeyModifiers::NONE)),
            Handled::Moved
        );
        assert_eq!(
            e.handle_key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
            Handled::Edited
        );
        assert_eq!(
            e.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
            Handled::Ignored
        );
    }
}
