use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Actions for text input UI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    Submit,
    Cancel,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    BackWord,
    Clear,
    ToggleDiff,
    Insert(char),
    None,
}

impl InputAction {
    pub fn from_key(key: KeyEvent) -> Self {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => InputAction::Submit,
            KeyCode::Esc => InputAction::Cancel,
            KeyCode::Backspace => InputAction::Backspace,
            KeyCode::Delete => InputAction::Delete,
            KeyCode::Left => InputAction::Left,
            KeyCode::Right => InputAction::Right,
            KeyCode::Home => InputAction::Home,
            KeyCode::End => InputAction::End,
            KeyCode::Char('a') if ctrl => InputAction::Home,
            KeyCode::Char('d') if ctrl => InputAction::ToggleDiff,
            KeyCode::Char('e') if ctrl => InputAction::End,
            KeyCode::Char('b') if ctrl => InputAction::BackWord,
            KeyCode::Char('u') | KeyCode::Char('c') if ctrl => InputAction::Clear,
            KeyCode::Char(c) => InputAction::Insert(c),
            _ => InputAction::None,
        }
    }
}

/// Result from input state machine
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputResult {
    Submit(String),
    Cancel,
}

/// Characters that act as word boundaries for cursor navigation
const WORD_BOUNDARIES: &[char] = &[':', '/', '?', '=', '&', '#', '@'];

/// Text input state machine
#[derive(Debug, Clone)]
pub struct InputState {
    input: String,
    cursor: usize,
}

impl InputState {
    pub fn new(default: Option<&str>) -> Self {
        let input = default.unwrap_or("").to_string();
        let cursor = input.len();
        Self { input, cursor }
    }

    pub fn text(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    /// Handle an input action, returns Some if the interaction is complete
    pub fn handle(&mut self, action: InputAction) -> Option<InputResult> {
        match action {
            InputAction::Submit => {
                if !self.input.is_empty() {
                    return Some(InputResult::Submit(self.input.clone()));
                }
            }
            InputAction::Cancel => return Some(InputResult::Cancel),
            InputAction::Backspace => {
                if self.cursor > 0 {
                    self.input.remove(self.cursor - 1);
                    self.cursor -= 1;
                }
            }
            InputAction::Delete => {
                if self.cursor < self.input.len() {
                    self.input.remove(self.cursor);
                }
            }
            InputAction::Left => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            InputAction::Right => {
                if self.cursor < self.input.len() {
                    self.cursor += 1;
                }
            }
            InputAction::Home => {
                self.cursor = 0;
            }
            InputAction::End => {
                self.cursor = self.input.len();
            }
            InputAction::BackWord => {
                self.cursor = self.find_prev_boundary();
            }
            InputAction::Clear => {
                self.input.clear();
                self.cursor = 0;
            }
            InputAction::Insert(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
            }
            InputAction::ToggleDiff | InputAction::None => {}
        }
        None
    }

    fn find_prev_boundary(&self) -> usize {
        if self.cursor == 0 {
            return 0;
        }
        let search_start = self.cursor.saturating_sub(1);
        self.input[..search_start]
            .rfind(WORD_BOUNDARIES)
            .map(|p| p + 1)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_state_new_empty() {
        let state = InputState::new(None);
        assert!(state.is_empty());
        assert_eq!(state.cursor(), 0);
    }

    #[test]
    fn test_input_state_new_with_default() {
        let state = InputState::new(Some("hello"));
        assert_eq!(state.text(), "hello");
        assert_eq!(state.cursor(), 5); // Cursor at end
    }

    #[test]
    fn test_input_insert() {
        let mut state = InputState::new(None);
        state.handle(InputAction::Insert('a'));
        state.handle(InputAction::Insert('b'));
        assert_eq!(state.text(), "ab");
        assert_eq!(state.cursor(), 2);
    }

    #[test]
    fn test_input_backspace() {
        let mut state = InputState::new(Some("abc"));
        state.handle(InputAction::Backspace);
        assert_eq!(state.text(), "ab");
    }

    #[test]
    fn test_input_submit() {
        let mut state = InputState::new(Some("test"));
        let result = state.handle(InputAction::Submit);
        assert_eq!(result, Some(InputResult::Submit("test".to_string())));
    }

    #[test]
    fn test_input_submit_empty() {
        let mut state = InputState::new(None);
        let result = state.handle(InputAction::Submit);
        assert_eq!(result, None); // Cannot submit empty
    }
}
