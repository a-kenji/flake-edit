use crossterm::event::{KeyCode, KeyEvent};

/// Actions for confirmation dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Apply,
    Back,
    Exit,
    None,
}

impl ConfirmAction {
    pub fn from_key(key: KeyEvent) -> Self {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => ConfirmAction::Apply,
            KeyCode::Char('b') | KeyCode::Char('B') => ConfirmAction::Back,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                ConfirmAction::Exit
            }
            _ => ConfirmAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn test_confirm_action_apply() {
        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(ConfirmAction::from_key(key), ConfirmAction::Apply);

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(ConfirmAction::from_key(key), ConfirmAction::Apply);
    }

    #[test]
    fn test_confirm_action_back() {
        let key = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE);
        assert_eq!(ConfirmAction::from_key(key), ConfirmAction::Back);
    }

    #[test]
    fn test_confirm_action_exit() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(ConfirmAction::from_key(key), ConfirmAction::Exit);

        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(ConfirmAction::from_key(key), ConfirmAction::Exit);
    }
}
