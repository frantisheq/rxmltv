//! A one-line text field.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

#[derive(Clone, Default)]
pub struct Input {
    chars: Vec<char>,
    cursor: usize,
}

impl Input {
    pub fn new(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let cursor = chars.len();
        Input { chars, cursor }
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    /// Applies an editing key. False when the key means nothing to a text field.
    pub fn handle(&mut self, key: &KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('u') if ctrl => {
                self.chars.clear();
                self.cursor = 0;
            }
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.chars.len(),
            KeyCode::Char(c) if !ctrl && !alt => {
                self.chars.insert(self.cursor, c);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.chars.remove(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.chars.len(),
            _ => return false,
        }
        true
    }

    /// The text with the cursor drawn as an inverted character, scrolled so the cursor stays inside `width`.
    pub fn spans(&self, width: usize) -> Vec<Span<'static>> {
        let width = width.max(2);
        let len = self.chars.len();
        let cursor = self.cursor.min(len);
        let start = (cursor + 1).saturating_sub(width);
        let end = (start + width).min(len);

        let before: String = self.chars[start..cursor].iter().collect();
        let at = self.chars.get(cursor).map(|c| c.to_string()).unwrap_or_else(|| " ".to_string());
        let after_start = (cursor + 1).min(len);
        let after: String = self.chars[after_start..end.max(after_start)].iter().collect();

        vec![
            Span::raw(before),
            Span::styled(at, Style::new().add_modifier(Modifier::REVERSED)),
            Span::raw(after),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_and_editing() {
        let mut input = Input::new("ab");
        assert!(input.handle(&key(KeyCode::Char('c'))));
        assert_eq!(input.value(), "abc");

        input.handle(&key(KeyCode::Left));
        input.handle(&key(KeyCode::Backspace));
        assert_eq!(input.value(), "ac");

        input.handle(&key(KeyCode::Home));
        input.handle(&key(KeyCode::Delete));
        assert_eq!(input.value(), "c");

        input.handle(&key(KeyCode::End));
        input.handle(&key(KeyCode::Char('č')));
        assert_eq!(input.value(), "cč");
    }

    #[test]
    fn unrelated_keys_are_not_consumed() {
        let mut input = Input::new("");
        assert!(!input.handle(&key(KeyCode::Enter)));
        assert!(!input.handle(&key(KeyCode::Esc)));
        assert!(!input.handle(&key(KeyCode::Up)));
    }

    #[test]
    fn ctrl_u_clears() {
        let mut input = Input::new("hello");
        input.handle(&KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(input.value(), "");
    }

    #[test]
    fn long_text_scrolls_with_the_cursor() {
        let input = Input::new("0123456789");
        let shown: String = input.spans(5).iter().map(|s| s.content.to_string()).collect();
        assert!(shown.chars().count() <= 5);
        assert!(shown.starts_with("6789"));
    }
}
