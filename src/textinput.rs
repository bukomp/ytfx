//! A one-line text field that edits like the input fields of other apps: a movable
//! cursor, word jumps, and the usual macOS and readline (emacs) editing keys.

use std::cell::Cell;
use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

use crate::youtube::is_safe_char;

#[derive(Debug, Default)]
pub struct TextInput {
    text: String,
    /// Byte offset of the cursor, always on a char boundary.
    cursor: usize,
    /// First display column on screen. The view scrolls only when the cursor
    /// would leave it, like a normal input field. Set while drawing.
    scroll: Cell<usize>,
}

/// Word jumps stop at the edges of runs of letters, digits and underscores.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn width(c: char) -> usize {
    c.width().unwrap_or(0)
}

impl TextInput {
    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Replace the text and put the cursor at its end.
    pub fn set(&mut self, s: &str) {
        self.text.clear();
        self.cursor = 0;
        self.scroll.set(0);
        self.insert(s);
    }

    /// Insert at the cursor, as typed or pasted. Line breaks and tabs become
    /// spaces; control and bidi-override characters are dropped.
    pub fn insert(&mut self, s: &str) {
        for c in s.trim_end_matches(['\r', '\n']).chars() {
            let c = if matches!(c, '\r' | '\n' | '\t') {
                ' '
            } else {
                c
            };
            if is_safe_char(c) {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
        }
    }

    /// Apply an editing or movement key. False if the key isn't one, so the
    /// caller can use it.
    pub fn handle(&mut self, k: KeyEvent) -> bool {
        let m = k.modifiers;
        let ctrl = m.contains(KeyModifiers::CONTROL);
        let alt = m.contains(KeyModifiers::ALT);
        // Cmd on macOS, with terminals that report it (kitty keyboard protocol).
        let cmd = m.intersects(KeyModifiers::SUPER | KeyModifiers::META);
        let word = ctrl || alt;
        let end = self.text.len();
        match k.code {
            KeyCode::Left if cmd => self.cursor = 0,
            KeyCode::Right if cmd => self.cursor = end,
            KeyCode::Left if word => self.cursor = self.word_left(),
            KeyCode::Right if word => self.cursor = self.word_right(),
            KeyCode::Left => self.cursor = self.prev(),
            KeyCode::Right => self.cursor = self.next(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = end,
            KeyCode::Backspace if cmd => self.delete_to(0),
            KeyCode::Backspace if word => self.delete_to(self.word_left()),
            KeyCode::Backspace => self.delete_to(self.prev()),
            KeyCode::Delete if word => self.delete_to(self.word_right()),
            KeyCode::Delete => self.delete_to(self.next()),
            KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
                'a' => self.cursor = 0,
                'e' => self.cursor = end,
                'b' => self.cursor = self.prev(),
                'f' => self.cursor = self.next(),
                'h' => self.delete_to(self.prev()),
                'd' => self.delete_to(self.next()),
                'w' => self.delete_to(self.word_left()),
                'u' => self.delete_to(0),
                'k' => self.delete_to(end),
                _ => return false,
            },
            // Option+arrow and option+backspace arrive like this in many macOS
            // terminals (esc b, esc f, esc d).
            KeyCode::Char(c) if alt => match c {
                'b' => self.cursor = self.word_left(),
                'f' => self.cursor = self.word_right(),
                'd' => self.delete_to(self.word_right()),
                _ => return false,
            },
            KeyCode::Char(_) if cmd => return false,
            KeyCode::Char(c) => self.insert(c.encode_utf8(&mut [0; 4])),
            _ => return false,
        }
        true
    }

    /// Put the cursor at column `col` of the field as last drawn (a mouse click).
    pub fn click(&mut self, col: usize) {
        let target = self.scroll.get() + col;
        let mut at = 0;
        self.cursor = self.text.len();
        for (i, c) in self.text.char_indices() {
            let w = width(c);
            // A click goes before the character under it, or after it on the
            // right half of a wide one.
            if at + w.div_ceil(2) > target {
                self.cursor = i;
                break;
            }
            at += w;
        }
    }

    /// What fits in `cols` columns with the cursor in view, split around it:
    /// text before the cursor, the character under it (None at the end), and
    /// the text after it.
    pub fn view(&self, cols: usize) -> (&str, Option<&str>, &str) {
        if cols == 0 {
            return ("", None, "");
        }
        let col_of = |byte: usize| self.text[..byte].chars().map(width).sum::<usize>();
        let next = self.next();
        let cur = col_of(self.cursor);
        // The cursor needs a cell even at the end of the text.
        let cur_end = cur + (col_of(next) - cur).max(1);
        let total = col_of(self.text.len()) + 1;
        let mut scroll = self.scroll.get().min(total.saturating_sub(cols));
        if cur < scroll {
            scroll = cur;
        } else if cur_end > scroll + cols {
            scroll = cur_end - cols;
        }
        self.scroll.set(scroll);

        let Range { start, end } = self.visible(scroll, scroll + cols);
        let at = (self.cursor < self.text.len()).then(|| &self.text[self.cursor..next]);
        let after = if at.is_some() {
            &self.text[next..end.max(next)]
        } else {
            ""
        };
        (&self.text[start..self.cursor], at, after)
    }

    /// Byte range of the characters that lie fully inside columns `from..to`.
    fn visible(&self, from: usize, to: usize) -> Range<usize> {
        let (mut start, mut end) = (self.text.len(), self.text.len());
        let mut col = 0;
        for (i, c) in self.text.char_indices() {
            if col >= from && start == self.text.len() {
                start = i;
            }
            col += width(c);
            if col > to {
                end = i;
                break;
            }
        }
        start.min(self.cursor)..end
    }

    fn prev(&self) -> usize {
        self.text[..self.cursor]
            .chars()
            .next_back()
            .map_or(0, |c| self.cursor - c.len_utf8())
    }

    fn next(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.text.len(), |c| self.cursor + c.len_utf8())
    }

    /// Start of the word before the cursor, past any separators in between.
    fn word_left(&self) -> usize {
        let mut pos = self.cursor;
        let mut in_word = false;
        for (i, c) in self.text[..self.cursor].char_indices().rev() {
            if is_word(c) {
                in_word = true;
            } else if in_word {
                break;
            }
            pos = i;
        }
        pos
    }

    /// End of the word after the cursor, past any separators in between.
    fn word_right(&self) -> usize {
        let mut in_word = false;
        for (i, c) in self.text[self.cursor..].char_indices() {
            if is_word(c) {
                in_word = true;
            } else if in_word {
                return self.cursor + i;
            }
        }
        self.text.len()
    }

    /// Delete between the cursor and `to`, either side of it.
    fn delete_to(&mut self, to: usize) {
        let range = self.cursor.min(to)..self.cursor.max(to);
        self.cursor = range.start;
        self.text.replace_range(range, "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    fn input(s: &str) -> TextInput {
        let mut t = TextInput::default();
        t.set(s);
        t
    }

    /// The text with `|` at the cursor.
    fn show(t: &TextInput) -> String {
        format!("{}|{}", &t.text[..t.cursor], &t.text[t.cursor..])
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const ALT: KeyModifiers = KeyModifiers::ALT;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;

    #[test]
    fn moves_and_inserts_mid_text() {
        let mut t = input("helo");
        t.handle(key(KeyCode::Left, NONE));
        t.handle(key(KeyCode::Char('l'), NONE));
        assert_eq!(show(&t), "hell|o");
        t.handle(key(KeyCode::Home, NONE));
        t.handle(key(KeyCode::Char('>'), KeyModifiers::SHIFT));
        assert_eq!(show(&t), ">|hello");
        t.handle(key(KeyCode::Char('e'), CTRL));
        assert_eq!(show(&t), ">hello|");
        t.handle(key(KeyCode::Right, NONE));
        assert_eq!(show(&t), ">hello|");
    }

    #[test]
    fn word_jumps() {
        let mut t = input("lo-fi  hip hop");
        t.handle(key(KeyCode::Left, ALT));
        assert_eq!(show(&t), "lo-fi  hip |hop");
        t.handle(key(KeyCode::Left, CTRL));
        assert_eq!(show(&t), "lo-fi  |hip hop");
        t.handle(key(KeyCode::Char('b'), ALT));
        assert_eq!(show(&t), "lo-|fi  hip hop");
        t.handle(key(KeyCode::Right, ALT));
        assert_eq!(show(&t), "lo-fi|  hip hop");
        t.handle(key(KeyCode::Char('f'), ALT));
        assert_eq!(show(&t), "lo-fi  hip| hop");
        t.handle(key(KeyCode::Left, KeyModifiers::SUPER));
        assert_eq!(show(&t), "|lo-fi  hip hop");
    }

    #[test]
    fn deletes() {
        let mut t = input("lo-fi hip hop");
        t.handle(key(KeyCode::Backspace, ALT));
        assert_eq!(show(&t), "lo-fi hip |");
        t.handle(key(KeyCode::Char('w'), CTRL));
        assert_eq!(show(&t), "lo-fi |");
        t.handle(key(KeyCode::Home, NONE));
        t.handle(key(KeyCode::Delete, NONE));
        assert_eq!(show(&t), "|o-fi ");
        t.handle(key(KeyCode::Char('d'), ALT));
        assert_eq!(show(&t), "|-fi ");
        t.handle(key(KeyCode::Right, NONE));
        t.handle(key(KeyCode::Char('k'), CTRL));
        assert_eq!(show(&t), "-|");
        t.handle(key(KeyCode::Char('u'), CTRL));
        assert_eq!(show(&t), "|");
        t.handle(key(KeyCode::Backspace, NONE));
        assert_eq!(show(&t), "|");
    }

    #[test]
    fn multibyte_text() {
        let mut t = input("héllo 世界");
        t.handle(key(KeyCode::Left, NONE));
        t.handle(key(KeyCode::Backspace, NONE));
        assert_eq!(show(&t), "héllo |界");
        t.handle(key(KeyCode::Left, ALT));
        t.handle(key(KeyCode::Right, NONE));
        t.handle(key(KeyCode::Delete, NONE));
        assert_eq!(show(&t), "h|llo 界");
    }

    #[test]
    fn leaves_other_keys_alone() {
        let mut t = input("x");
        for k in [
            key(KeyCode::Char('t'), CTRL),
            key(KeyCode::Char('x'), ALT),
            key(KeyCode::Enter, NONE),
            key(KeyCode::Esc, NONE),
            key(KeyCode::Tab, NONE),
        ] {
            assert!(!t.handle(k));
        }
        assert_eq!(show(&t), "x|");
    }

    #[test]
    fn paste_is_one_line_and_safe() {
        let mut t = input("ab");
        t.handle(key(KeyCode::Left, NONE));
        t.insert("1\t2\n3\u{1b}[31m\u{202e}\n");
        assert_eq!(show(&t), "a1 2 3[31m|b");
    }

    #[test]
    fn view_scrolls_to_keep_cursor_visible() {
        let mut t = input("abcdefghij");
        // The cursor at the end needs its own cell.
        assert_eq!(t.view(5), ("ghij", None, ""));
        t.handle(key(KeyCode::Home, NONE));
        assert_eq!(t.view(5), ("", Some("a"), "bcde"));
        // Moving inside the view doesn't scroll it.
        for _ in 0..4 {
            t.handle(key(KeyCode::Right, NONE));
        }
        assert_eq!(t.view(5), ("abcd", Some("e"), ""));
        t.handle(key(KeyCode::Right, NONE));
        assert_eq!(t.view(5), ("bcde", Some("f"), ""));
        t.handle(key(KeyCode::Left, NONE));
        assert_eq!(t.view(5), ("bcd", Some("e"), "f"));
        // Short text never stays scrolled.
        t.set("ab");
        assert_eq!(t.view(5), ("ab", None, ""));
    }

    #[test]
    fn view_handles_wide_chars() {
        let t = input("世界世界");
        assert_eq!(t.view(5), ("世界", None, ""));
    }

    #[test]
    fn click_places_cursor() {
        let mut t = input("hello world");
        let _ = t.view(20);
        t.click(3);
        assert_eq!(show(&t), "hel|lo world");
        t.click(50);
        assert_eq!(show(&t), "hello world|");
        // Clicks land relative to the scrolled view.
        t.set("abcdefghij");
        let _ = t.view(5);
        t.click(0);
        assert_eq!(show(&t), "abcdef|ghij");
    }
}
