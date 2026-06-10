use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

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
    /// Enter search mode (`/` in normal mode)
    SearchStart,
    /// Append a character to the search query
    SearchInput(char),
    /// Remove the last character from the search query
    SearchBackspace,
    /// Leave search mode and restore the unfiltered list
    SearchCancel,
    None,
}

impl ListAction {
    /// Map a key event to an action.
    ///
    /// The mapping depends on the mode. While searching, printable
    /// characters (including the normal-mode bindings j/k/q/u) extend
    /// the query, Escape leaves search mode instead of cancelling, and
    /// navigation moves to the arrow keys and Ctrl+J/Ctrl+K.
    pub fn from_key(key: KeyEvent, search_active: bool) -> Self {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if search_active {
            return match key.code {
                KeyCode::Up => ListAction::Up,
                KeyCode::Down => ListAction::Down,
                KeyCode::Enter => ListAction::Select,
                KeyCode::Esc => ListAction::SearchCancel,
                KeyCode::Backspace => ListAction::SearchBackspace,
                // Plain j/k are query input here, so navigation needs
                // Ctrl chords. Ctrl+J/Ctrl+K follow the fzf convention.
                KeyCode::Char('j') if ctrl => ListAction::Down,
                KeyCode::Char('k') if ctrl => ListAction::Up,
                KeyCode::Char('d') if ctrl => ListAction::ToggleDiff,
                // Space still toggles so multi-select works while
                // filtered. Item ids never contain spaces, so the
                // query loses nothing.
                KeyCode::Char(' ') => ListAction::Toggle,
                KeyCode::Char(c) if !ctrl => ListAction::SearchInput(c),
                _ => ListAction::None,
            };
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => ListAction::Up,
            KeyCode::Down | KeyCode::Char('j') => ListAction::Down,
            KeyCode::Enter => ListAction::Select,
            KeyCode::Esc | KeyCode::Char('q') => ListAction::Cancel,
            KeyCode::Char('d') if ctrl => ListAction::ToggleDiff,
            KeyCode::Char(' ') => ListAction::Toggle,
            KeyCode::Char('u') | KeyCode::Char('U') => ListAction::ToggleAll,
            KeyCode::Char('/') => ListAction::SearchStart,
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
///
/// Owns the item texts so filtering stays internal to the state
/// machine. `visible` maps display positions to absolute indices into
/// `items`, and `cursor` addresses a position within `visible`. The
/// `selected` set holds absolute indices, which is what lets
/// selections survive filtering.
#[derive(Debug, Clone)]
pub struct ListState {
    items: Vec<String>,
    cursor: usize,
    selected: HashSet<usize>,
    show_diff: bool,
    multi_select: bool,
    /// Search query, `Some` while search mode is active
    query: Option<String>,
    /// Absolute indices of the items currently shown, best match first
    visible: Vec<usize>,
}

impl ListState {
    pub fn new(items: Vec<String>, multi_select: bool, initial_diff: bool) -> Self {
        let visible = (0..items.len()).collect();
        Self {
            items,
            cursor: 0,
            selected: HashSet::new(),
            show_diff: initial_diff,
            multi_select,
            query: None,
            visible,
        }
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Cursor position within the visible (possibly filtered) list
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn search_active(&self) -> bool {
        self.query.is_some()
    }

    /// The current search query, `Some` (possibly empty) while search
    /// mode is active
    pub fn search_query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// The currently visible items as (absolute index, text) pairs
    pub fn visible_items(&self) -> impl Iterator<Item = (usize, &str)> {
        self.visible.iter().map(|&i| (i, self.items[i].as_str()))
    }

    /// Absolute index of the item under the cursor, `None` when the
    /// filtered list is empty
    pub fn highlighted(&self) -> Option<usize> {
        self.visible.get(self.cursor).copied()
    }

    pub fn show_diff(&self) -> bool {
        self.show_diff
    }

    pub fn set_show_diff(&mut self, show_diff: bool) {
        self.show_diff = show_diff;
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
                    self.visible.len().saturating_sub(1)
                } else {
                    self.cursor - 1
                };
            }
            ListAction::Down => {
                self.cursor = if self.cursor >= self.visible.len().saturating_sub(1) {
                    0
                } else {
                    self.cursor + 1
                };
            }
            ListAction::Toggle if self.multi_select => {
                if let Some(&index) = self.visible.get(self.cursor) {
                    if !self.selected.remove(&index) {
                        self.selected.insert(index);
                    }
                    // Move to next item after toggling
                    self.cursor = if self.cursor >= self.visible.len().saturating_sub(1) {
                        0
                    } else {
                        self.cursor + 1
                    };
                }
            }
            ListAction::ToggleAll if self.multi_select => {
                let all_visible_selected = !self.visible.is_empty()
                    && self.visible.iter().all(|i| self.selected.contains(i));
                if all_visible_selected {
                    for index in &self.visible {
                        self.selected.remove(index);
                    }
                } else {
                    self.selected.extend(self.visible.iter().copied());
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
                    return Some(ListResult::Select(self.selected_indices(), self.show_diff));
                }
                if let Some(&index) = self.visible.get(self.cursor) {
                    return Some(ListResult::Select(vec![index], self.show_diff));
                }
            }
            ListAction::Cancel => return Some(ListResult::Cancel),
            ListAction::SearchStart => {
                self.query = Some(String::new());
                self.refilter();
            }
            ListAction::SearchInput(c) => {
                if let Some(query) = &mut self.query {
                    query.push(c);
                    self.refilter();
                }
            }
            ListAction::SearchBackspace => {
                if let Some(query) = &mut self.query {
                    query.pop();
                    self.refilter();
                }
            }
            ListAction::SearchCancel => {
                // Keep the match highlighted in the restored full list.
                // The unfiltered visible order is the identity, so the
                // absolute index is also the display position.
                let highlighted = self.highlighted();
                self.query = None;
                self.refilter();
                self.cursor = highlighted.unwrap_or(0);
            }
            _ => {}
        }
        None
    }

    /// Recompute `visible` from the current query and reset the cursor.
    ///
    /// Matching is fuzzy with smart case, ordered best score first.
    /// The sort is stable so equal scores keep the original list order.
    fn refilter(&mut self) {
        let query = self.query.as_deref().unwrap_or("");
        if query.is_empty() {
            self.visible = (0..self.items.len()).collect();
        } else {
            let mut matcher = Matcher::new(Config::DEFAULT);
            let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
            let mut char_buf: Vec<char> = Vec::new();
            let mut scored: Vec<(u32, usize)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    char_buf.clear();
                    let haystack = Utf32Str::new(item, &mut char_buf);
                    pattern
                        .score(haystack, &mut matcher)
                        .map(|score| (score, i))
                })
                .collect();
            scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
            self.visible = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("item{i}")).collect()
    }

    #[test]
    fn test_single_select_navigation() {
        let mut state = ListState::new(items(3), false, false);
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
        let mut state = ListState::new(items(3), false, false);
        assert!(!state.show_diff());

        state.handle(ListAction::ToggleDiff);
        assert!(state.show_diff());
    }

    #[test]
    fn test_single_select_select() {
        let mut state = ListState::new(items(3), false, true);
        state.handle(ListAction::Down);
        let result = state.handle(ListAction::Select);
        assert_eq!(result, Some(ListResult::Select(vec![1], true)));
    }

    #[test]
    fn test_single_select_ignores_toggle() {
        let mut state = ListState::new(items(3), false, false);
        state.handle(ListAction::Toggle);
        assert_eq!(state.selected_count(), 0); // Toggle ignored
        assert_eq!(state.cursor(), 0); // Cursor unchanged
    }

    #[test]
    fn test_multi_select_toggle() {
        let mut state = ListState::new(items(3), true, false);
        assert_eq!(state.selected_count(), 0);

        state.handle(ListAction::Toggle);
        assert_eq!(state.selected_count(), 1);
        assert!(state.is_selected(0));
        assert_eq!(state.cursor(), 1); // Cursor moves after toggle
    }

    #[test]
    fn test_multi_select_toggle_all() {
        let mut state = ListState::new(items(3), true, false);
        state.handle(ListAction::ToggleAll);
        assert_eq!(state.selected_count(), 3);

        state.handle(ListAction::ToggleAll);
        assert_eq!(state.selected_count(), 0);
    }

    fn input_ids() -> Vec<String> {
        [
            "bookah",
            "centerpiece",
            "clan",
            "crane",
            "depot",
            "direnv-instant",
            "disko",
            "distro",
            "flake-fmt",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn visible_texts(state: &ListState) -> Vec<&str> {
        state.visible_items().map(|(_, text)| text).collect()
    }

    #[test]
    fn test_slash_starts_search_in_normal_mode() {
        assert_eq!(
            ListAction::from_key(key(KeyCode::Char('/')), false),
            ListAction::SearchStart
        );
        // Navigation keys keep their meaning outside of search mode
        assert_eq!(
            ListAction::from_key(key(KeyCode::Char('j')), false),
            ListAction::Down
        );
        assert_eq!(
            ListAction::from_key(key(KeyCode::Char('q')), false),
            ListAction::Cancel
        );
    }

    #[test]
    fn test_search_mode_key_mapping() {
        // Printable characters become query input, including the
        // normal-mode bindings j/k/q/u and a literal slash.
        for c in ['j', 'k', 'q', 'u', 'U', '/', 'f'] {
            assert_eq!(
                ListAction::from_key(key(KeyCode::Char(c)), true),
                ListAction::SearchInput(c)
            );
        }
        assert_eq!(
            ListAction::from_key(key(KeyCode::Esc), true),
            ListAction::SearchCancel
        );
        assert_eq!(
            ListAction::from_key(key(KeyCode::Enter), true),
            ListAction::Select
        );
        assert_eq!(
            ListAction::from_key(key(KeyCode::Backspace), true),
            ListAction::SearchBackspace
        );
        assert_eq!(ListAction::from_key(key(KeyCode::Up), true), ListAction::Up);
        assert_eq!(
            ListAction::from_key(key(KeyCode::Down), true),
            ListAction::Down
        );
        // Space still toggles in multi-select. Item ids never contain spaces.
        assert_eq!(
            ListAction::from_key(key(KeyCode::Char(' ')), true),
            ListAction::Toggle
        );
        // Control chords are not query input
        assert_eq!(
            ListAction::from_key(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
                true
            ),
            ListAction::ToggleDiff
        );
        assert_eq!(
            ListAction::from_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                true
            ),
            ListAction::None
        );
        // Ctrl+J / Ctrl+K navigate while searching (fzf convention)
        assert_eq!(
            ListAction::from_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
                true
            ),
            ListAction::Down
        );
        assert_eq!(
            ListAction::from_key(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
                true
            ),
            ListAction::Up
        );
    }

    #[test]
    fn test_search_filters_incrementally() {
        let mut state = ListState::new(input_ids(), false, false);
        assert!(!state.search_active());

        state.handle(ListAction::SearchStart);
        assert!(state.search_active());
        assert_eq!(state.search_query(), Some(""));
        // Empty query shows the full list
        assert_eq!(state.visible_items().count(), 9);

        state.handle(ListAction::SearchInput('f'));
        state.handle(ListAction::SearchInput('l'));
        assert_eq!(state.search_query(), Some("fl"));
        assert_eq!(visible_texts(&state), vec!["flake-fmt"]);
        // The highlighted item refers to the original index
        assert_eq!(state.highlighted(), Some(8));
    }

    #[test]
    fn test_search_enter_returns_original_index() {
        let mut state = ListState::new(input_ids(), false, true);
        state.handle(ListAction::SearchStart);
        state.handle(ListAction::SearchInput('f'));
        state.handle(ListAction::SearchInput('l'));

        let result = state.handle(ListAction::Select);
        assert_eq!(result, Some(ListResult::Select(vec![8], true)));
    }

    #[test]
    fn test_search_escape_restores_full_list() {
        let mut state = ListState::new(input_ids(), false, false);
        state.handle(ListAction::SearchStart);
        state.handle(ListAction::SearchInput('f'));
        state.handle(ListAction::SearchInput('l'));

        state.handle(ListAction::SearchCancel);
        assert!(!state.search_active());
        assert_eq!(state.search_query(), None);
        assert_eq!(state.visible_items().count(), 9);
        // The previously highlighted match stays highlighted
        assert_eq!(state.highlighted(), Some(8));
    }

    #[test]
    fn test_search_no_match_then_backspace() {
        let mut state = ListState::new(input_ids(), false, false);
        state.handle(ListAction::SearchStart);
        state.handle(ListAction::SearchInput('f'));
        state.handle(ListAction::SearchInput('l'));
        state.handle(ListAction::SearchInput('z'));
        assert_eq!(state.visible_items().count(), 0);
        assert_eq!(state.highlighted(), None);

        // Enter on an empty filtered list must not select anything
        assert_eq!(state.handle(ListAction::Select), None);
        // Navigation on an empty filtered list stays at the empty cursor
        state.handle(ListAction::Down);
        state.handle(ListAction::Up);
        assert_eq!(state.cursor(), 0);
        assert_eq!(state.highlighted(), None);

        // Backspace recovers the previous matches
        state.handle(ListAction::SearchBackspace);
        assert_eq!(visible_texts(&state), vec!["flake-fmt"]);
    }

    #[test]
    fn test_search_smart_case_matches_capitals() {
        let items = vec!["Flake-Fmt".to_string(), "bookah".to_string()];
        let mut state = ListState::new(items, false, false);
        state.handle(ListAction::SearchStart);
        state.handle(ListAction::SearchInput('f'));
        state.handle(ListAction::SearchInput('l'));
        assert_eq!(visible_texts(&state), vec!["Flake-Fmt"]);
    }

    #[test]
    fn test_search_best_match_first() {
        let items = vec!["fenix-lib".to_string(), "flake-fmt".to_string()];
        let mut state = ListState::new(items, false, false);
        state.handle(ListAction::SearchStart);
        state.handle(ListAction::SearchInput('f'));
        state.handle(ListAction::SearchInput('l'));
        // Both ids match "fl", but the contiguous prefix in flake-fmt
        // outscores the scattered f..l in fenix-lib, so the better
        // match lands under the cursor despite coming later in the list
        assert_eq!(visible_texts(&state), vec!["flake-fmt", "fenix-lib"]);
        assert_eq!(state.highlighted(), Some(1));
    }

    #[test]
    fn test_search_navigation_within_filtered_list() {
        let mut state = ListState::new(input_ids(), false, false);
        state.handle(ListAction::SearchStart);
        state.handle(ListAction::SearchInput('d'));
        state.handle(ListAction::SearchInput('i'));
        // "di" matches direnv-instant (5), disko (6), and distro (7)
        let mut visible: Vec<usize> = state.visible_items().map(|(i, _)| i).collect();
        visible.sort_unstable();
        assert_eq!(visible, vec![5, 6, 7]);

        // Navigate to "distro" within the filtered view and select it
        let target = state
            .visible_items()
            .position(|(i, _)| i == 7)
            .expect("distro is visible");
        for _ in 0..target {
            state.handle(ListAction::Down);
        }
        let result = state.handle(ListAction::Select);
        assert_eq!(result, Some(ListResult::Select(vec![7], false)));
    }

    #[test]
    fn test_multi_select_selections_survive_filtering() {
        let mut state = ListState::new(input_ids(), true, false);
        // Select "bookah" before filtering
        state.handle(ListAction::Toggle);
        assert!(state.is_selected(0));

        // Filter down to flake-fmt and select it too
        state.handle(ListAction::SearchStart);
        state.handle(ListAction::SearchInput('f'));
        state.handle(ListAction::SearchInput('l'));
        state.handle(ListAction::Toggle);
        assert!(state.is_selected(8));

        // Leaving search keeps both selections intact
        state.handle(ListAction::SearchCancel);
        assert_eq!(state.selected_indices(), vec![0, 8]);

        let result = state.handle(ListAction::Select);
        assert_eq!(result, Some(ListResult::Select(vec![0, 8], false)));
    }

    #[test]
    fn test_multi_select_submit_empty() {
        let mut state = ListState::new(items(3), true, false);
        let result = state.handle(ListAction::Select);
        assert_eq!(result, Some(ListResult::Cancel)); // Empty selection cancels
    }
}
