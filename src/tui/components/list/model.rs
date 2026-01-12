use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Actions that can be taken in a list selection UI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListAction {
    Up,
    Down,
    Select,
    ToggleDiff,
    Cancel,
    Toggle,
    ToggleAll,
    None,
}

impl ListAction {
    pub fn from_key(key: KeyEvent) -> Self {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => ListAction::Up,
            KeyCode::Down | KeyCode::Char('j') => ListAction::Down,
            KeyCode::Enter => ListAction::Select,
            KeyCode::Esc | KeyCode::Char('q') => ListAction::Cancel,
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                ListAction::ToggleDiff
            }
            KeyCode::Char(' ') => ListAction::Toggle,
            KeyCode::Char('u') | KeyCode::Char('U') => ListAction::ToggleAll,
            _ => ListAction::None,
        }
    }
}

/// Result from list state machine
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListResult {
    /// Selected indices and show_diff state
    Select(Vec<usize>, bool),
    Cancel,
}

/// Unified list state machine for single and multi-select
#[derive(Debug, Clone)]
pub struct ListState {
    cursor: usize,
    len: usize,
    selected: HashSet<usize>,
    show_diff: bool,
    multi_select: bool,
}

impl ListState {
    pub fn new(len: usize, multi_select: bool, initial_diff: bool) -> Self {
        Self {
            cursor: 0,
            len,
            selected: HashSet::new(),
            show_diff: initial_diff,
            multi_select,
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn show_diff(&self) -> bool {
        self.show_diff
    }

    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }

    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    /// Get sorted list of selected indices for deterministic iteration
    pub fn selected_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = self.selected.iter().copied().collect();
        indices.sort_unstable();
        indices
    }

    pub fn multi_select(&self) -> bool {
        self.multi_select
    }

    /// Handle a list action, returns Some if the interaction is complete
    pub fn handle(&mut self, action: ListAction) -> Option<ListResult> {
        match action {
            ListAction::Up => {
                self.cursor = if self.cursor == 0 {
                    self.len.saturating_sub(1)
                } else {
                    self.cursor - 1
                };
            }
            ListAction::Down => {
                self.cursor = if self.cursor >= self.len.saturating_sub(1) {
                    0
                } else {
                    self.cursor + 1
                };
            }
            ListAction::Toggle if self.multi_select => {
                if self.selected.contains(&self.cursor) {
                    self.selected.remove(&self.cursor);
                } else {
                    self.selected.insert(self.cursor);
                }
                // Move to next item after toggling
                self.cursor = if self.cursor >= self.len.saturating_sub(1) {
                    0
                } else {
                    self.cursor + 1
                };
            }
            ListAction::ToggleAll if self.multi_select => {
                if self.selected.len() == self.len {
                    self.selected.clear();
                } else {
                    self.selected = (0..self.len).collect();
                }
            }
            ListAction::ToggleDiff => {
                self.show_diff = !self.show_diff;
            }
            ListAction::Select => {
                if self.multi_select {
                    if self.selected.is_empty() {
                        return Some(ListResult::Cancel);
                    }
                    let mut indices: Vec<usize> = self.selected.iter().copied().collect();
                    indices.sort_unstable();
                    return Some(ListResult::Select(indices, self.show_diff));
                } else {
                    return Some(ListResult::Select(vec![self.cursor], self.show_diff));
                }
            }
            ListAction::Cancel => return Some(ListResult::Cancel),
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_select_navigation() {
        let mut state = ListState::new(3, false, false);
        assert_eq!(state.cursor(), 0);

        state.handle(ListAction::Down);
        assert_eq!(state.cursor(), 1);

        state.handle(ListAction::Down);
        assert_eq!(state.cursor(), 2);

        state.handle(ListAction::Down);
        assert_eq!(state.cursor(), 0); // Wrap around

        state.handle(ListAction::Up);
        assert_eq!(state.cursor(), 2); // Wrap around up
    }

    #[test]
    fn test_single_select_toggle_diff() {
        let mut state = ListState::new(3, false, false);
        assert!(!state.show_diff());

        state.handle(ListAction::ToggleDiff);
        assert!(state.show_diff());
    }

    #[test]
    fn test_single_select_select() {
        let mut state = ListState::new(3, false, true);
        state.handle(ListAction::Down);
        let result = state.handle(ListAction::Select);
        assert_eq!(result, Some(ListResult::Select(vec![1], true)));
    }

    #[test]
    fn test_single_select_ignores_toggle() {
        let mut state = ListState::new(3, false, false);
        state.handle(ListAction::Toggle);
        assert_eq!(state.selected_count(), 0); // Toggle ignored
        assert_eq!(state.cursor(), 0); // Cursor unchanged
    }

    #[test]
    fn test_multi_select_toggle() {
        let mut state = ListState::new(3, true, false);
        assert_eq!(state.selected_count(), 0);

        state.handle(ListAction::Toggle);
        assert_eq!(state.selected_count(), 1);
        assert!(state.is_selected(0));
        assert_eq!(state.cursor(), 1); // Cursor moves after toggle
    }

    #[test]
    fn test_multi_select_toggle_all() {
        let mut state = ListState::new(3, true, false);
        state.handle(ListAction::ToggleAll);
        assert_eq!(state.selected_count(), 3);

        state.handle(ListAction::ToggleAll);
        assert_eq!(state.selected_count(), 0);
    }

    #[test]
    fn test_multi_select_submit_empty() {
        let mut state = ListState::new(3, true, false);
        let result = state.handle(ListAction::Select);
        assert_eq!(result, Some(ListResult::Cancel)); // Empty selection cancels
    }
}
