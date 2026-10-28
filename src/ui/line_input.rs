//! Single-line text editor used by the agent panel input.
//!
//! The cursor is a `char` index, so multi-byte text (Chinese) is edited one
//! character at a time, and positions are converted to terminal cells with
//! the display width of each character (CJK characters take two cells).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Span;

/// Display width of one character in terminal cells.
pub fn char_width(c: char) -> usize {
    if c.is_ascii() {
        usize::from(!c.is_ascii_control())
    } else {
        let mut buf = [0u8; 4];
        Span::raw(&*c.encode_utf8(&mut buf)).width()
    }
}

/// Display width of a string in terminal cells.
pub fn text_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    chars: Vec<char>,
    /// Insertion point, `0..=chars.len()`.
    cursor: usize,
}

/// The visible part of the input for a given width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewport {
    pub text: String,
    /// Cursor column relative to the start of `text`.
    pub cursor_x: u16,
}

fn is_word(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, ',' | '，' | '.' | '。' | '/' | ';' | '；' | ':' | '：')
}

impl LineInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Replace the content and put the cursor at the end.
    pub fn set(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
    }

    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
    }

    /// Return the content and clear the input.
    pub fn take(&mut self) -> String {
        let text = self.text();
        self.clear();
        text
    }

    pub fn insert_char(&mut self, c: char) {
        let c = if c == '\t' { ' ' } else { c };
        if c.is_control() {
            return;
        }
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Insert pasted text at the cursor. Line breaks become spaces, so a
    /// multi-line paste never submits the input.
    pub fn insert_str(&mut self, text: &str) {
        let mut previous_space = false;
        for c in text.chars() {
            let c = if matches!(c, '\r' | '\n' | '\t') {
                ' '
            } else {
                c
            };
            if c == ' ' && previous_space {
                continue;
            }
            previous_space = c == ' ';
            self.insert_char(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.chars.len();
    }

    fn word_start_before(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && !is_word(self.chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.chars[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_end_after(&self) -> usize {
        let mut i = self.cursor;
        let n = self.chars.len();
        while i < n && !is_word(self.chars[i]) {
            i += 1;
        }
        while i < n && is_word(self.chars[i]) {
            i += 1;
        }
        i
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start_before();
    }

    pub fn word_right(&mut self) {
        self.cursor = self.word_end_after();
    }

    /// Ctrl+W: delete the word before the cursor.
    pub fn delete_word_before(&mut self) {
        let start = self.word_start_before();
        self.chars.drain(start..self.cursor);
        self.cursor = start;
    }

    /// Ctrl+U: delete from the start of the line to the cursor.
    pub fn kill_to_start(&mut self) {
        self.chars.drain(..self.cursor);
        self.cursor = 0;
    }

    /// Ctrl+K: delete from the cursor to the end of the line.
    pub fn kill_to_end(&mut self) {
        self.chars.truncate(self.cursor);
    }

    /// Apply an editing key. Returns `false` for keys the editor does not
    /// handle (Enter, Esc, Up, Down, Tab, ...), which the caller routes.
    /// Ctrl+B / Ctrl+F are left out on purpose: the panel uses Ctrl+F.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Left if ctrl || alt => self.word_left(),
            KeyCode::Right if ctrl || alt => self.word_right(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.home(),
            KeyCode::End => self.end(),
            KeyCode::Backspace if ctrl || alt => self.delete_word_before(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Char('a') if ctrl => self.home(),
            KeyCode::Char('e') if ctrl => self.end(),
            KeyCode::Char('h') if ctrl => self.backspace(),
            KeyCode::Char('d') if ctrl => self.delete(),
            KeyCode::Char('w') if ctrl => self.delete_word_before(),
            KeyCode::Char('u') if ctrl => self.kill_to_start(),
            KeyCode::Char('k') if ctrl => self.kill_to_end(),
            KeyCode::Char('b') if alt => self.word_left(),
            KeyCode::Char('f') if alt => self.word_right(),
            KeyCode::Char(_) if ctrl || alt => return false,
            KeyCode::Char(c) => self.insert_char(c),
            _ => return false,
        }
        true
    }

    /// The slice to draw in `width` cells, scrolled so the cursor stays
    /// visible. With `mask`, every character is drawn as `•`.
    pub fn viewport(&self, width: usize, mask: bool) -> Viewport {
        let shown: Vec<char> = if mask {
            vec!['•'; self.chars.len()]
        } else {
            self.chars.clone()
        };
        let widths: Vec<usize> = shown.iter().map(|c| char_width(*c)).collect();
        // Keep one cell free for the cursor after the last character.
        let room = width.saturating_sub(1).max(1);
        let mut start = 0;
        let mut before: usize = widths[..self.cursor].iter().sum();
        while before > room && start < self.cursor {
            before -= widths[start];
            start += 1;
        }
        let mut used = 0;
        let mut end = start;
        while end < shown.len() && used + widths[end] <= width {
            used += widths[end];
            end += 1;
        }
        Viewport {
            text: shown[start..end].iter().collect(),
            cursor_x: before as u16,
        }
    }
}

impl PartialEq<&str> for LineInput {
    fn eq(&self, other: &&str) -> bool {
        self.chars.iter().copied().eq(other.chars())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn input(text: &str) -> LineInput {
        let mut input = LineInput::new();
        input.set(text);
        input
    }

    #[test]
    fn edits_in_the_middle_of_chinese_text() {
        let mut i = input("多模大模型");
        i.left();
        i.left();
        i.left();
        i.insert_char('态');
        assert_eq!(i, "多模态大模型");
        i.backspace();
        i.backspace();
        assert_eq!(i, "多大模型");
        assert_eq!(i.cursor(), 1);
        i.home();
        i.delete();
        assert_eq!(i, "大模型");
        i.end();
        assert_eq!(i.cursor(), 3);
    }

    #[test]
    fn word_motion_and_deletion() {
        let mut i = input("security of LLM agents");
        i.word_left();
        assert_eq!(i.cursor(), 16);
        i.word_left();
        i.delete_word_before();
        assert_eq!(i, "security LLM agents");
        i.word_right();
        assert_eq!(i.cursor(), 12);
        i.kill_to_end();
        assert_eq!(i, "security LLM");
        i.kill_to_start();
        assert!(i.is_empty());
    }

    #[test]
    fn keys_map_to_edits_and_unknown_keys_are_returned() {
        let mut i = LineInput::new();
        for c in "hello world".chars() {
            assert!(i.handle_key(key(KeyCode::Char(c), KeyModifiers::NONE)));
        }
        assert!(i.handle_key(key(KeyCode::Char('W'), KeyModifiers::SHIFT)));
        assert_eq!(i, "hello worldW");
        assert!(i.handle_key(key(KeyCode::Char('w'), KeyModifiers::CONTROL)));
        assert_eq!(i, "hello ");
        assert!(i.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)));
        assert_eq!(i.cursor(), 0);
        assert!(i.handle_key(key(KeyCode::Right, KeyModifiers::CONTROL)));
        assert_eq!(i.cursor(), 5);
        assert!(i.handle_key(key(KeyCode::Backspace, KeyModifiers::ALT)));
        assert_eq!(i, " ");
        for code in [
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Tab,
        ] {
            assert!(!i.handle_key(key(code, KeyModifiers::NONE)));
        }
        // Unbound Ctrl chords are left to the caller (e.g. Ctrl+C).
        assert!(!i.handle_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert_eq!(i, " ");
    }

    #[test]
    fn pasted_line_breaks_become_single_spaces() {
        let mut i = input("ab");
        i.left();
        i.insert_str("1\r\n2\n\n3\t");
        assert_eq!(i, "a1 2 3 b");
        assert_eq!(i.cursor(), 7);
        i.insert_char('\u{7}');
        assert_eq!(i.len(), 8);
    }

    #[test]
    fn viewport_scrolls_to_keep_the_cursor_visible() {
        let i = input("abcdefghij");
        // Four characters and the cursor cell fill five columns.
        let v = i.viewport(5, false);
        assert_eq!(v.text, "ghij");
        assert_eq!(v.cursor_x, 4);

        let mut i = input("abcdefghij");
        i.home();
        let v = i.viewport(5, false);
        assert_eq!(v.text, "abcde");
        assert_eq!(v.cursor_x, 0);
    }

    #[test]
    fn viewport_counts_double_width_characters() {
        let i = input("会议ab");
        let v = i.viewport(20, false);
        assert_eq!(v.cursor_x, 6);
        let mut i = input("会议推荐理由");
        i.home();
        i.right();
        let v = i.viewport(6, false);
        assert_eq!(v.text, "会议推");
        assert_eq!(v.cursor_x, 2);
        let v = i.viewport(6, true);
        assert_eq!(v.text, "••••••");
        assert_eq!(v.cursor_x, 1);
    }

    #[test]
    fn take_returns_and_clears() {
        let mut i = input("x");
        assert_eq!(i.take(), "x");
        assert!(i.is_empty());
        assert_eq!(i.cursor(), 0);
    }
}
