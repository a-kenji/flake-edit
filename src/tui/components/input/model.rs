use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

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
    ToggleFollows,
    Insert(char),
    /// Move completion selection up
    CompletionUp,
    /// Move completion selection down
    CompletionDown,
    /// Accept current completion (Tab)
    Accept,
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
            KeyCode::Up => InputAction::CompletionUp,
            KeyCode::Down => InputAction::CompletionDown,
            KeyCode::Tab => InputAction::Accept,
            KeyCode::Home => InputAction::Home,
            KeyCode::End => InputAction::End,
            KeyCode::Char('a') if ctrl => InputAction::Home,
            KeyCode::Char('d') if ctrl => InputAction::ToggleDiff,
            KeyCode::Char('f') if ctrl => InputAction::ToggleFollows,
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

/// Maximum number of completions to display (used by both model and view)
pub const MAX_VISIBLE_COMPLETIONS: usize = 2;

/// Query parameters available for flake URIs: (param, description)
const QUERY_PARAMS: &[(&str, &str)] = &[
    ("?ref=", "Git/Mercurial branch or tag"),
    ("?rev=", "Git/Mercurial commit hash"),
    ("?dir=", "Subdirectory containing flake.nix"),
    ("?branch=", "Git branch name"),
    ("?host=", "Custom host for GitHub/GitLab/SourceHut"),
    ("?shallow=", "Shallow clone (1 = enabled)"),
    ("?submodules=", "Fetch Git submodules (1 = enabled)"),
    ("?narHash=", "NAR hash in SRI format"),
];

/// Parsed query parameter context from a flake URI
#[derive(Debug, Clone)]
struct QueryContext {
    /// Position where completions should be anchored for rendering
    anchor: usize,
    /// End position for the base string (after ? or &)
    base_end: usize,
    /// The partial param name being typed (e.g., "ref" when typing "?ref")
    param_prefix: String,
}

impl QueryContext {
    /// Parse query context from input, returns None if not in query param mode
    fn parse(input: &str) -> Option<Self> {
        // Must have a URI-like pattern first (contains : followed by content)
        let has_uri = input.contains(':') && !input.ends_with(':');
        if !has_uri {
            return None;
        }

        let q_pos = input.rfind('?')?;
        let after_q = &input[q_pos + 1..];

        if let Some(amp_pos) = after_q.rfind('&') {
            let param_part = &after_q[amp_pos + 1..];
            // Only if we haven't completed the param (no = yet)
            if !param_part.contains('=') {
                let pos = q_pos + 1 + amp_pos + 1;
                return Some(Self {
                    anchor: pos,
                    base_end: pos,
                    param_prefix: param_part.to_string(),
                });
            }
        } else if !after_q.contains('=') {
            // No & and no = means we're typing the first param name
            return Some(Self {
                anchor: q_pos,
                base_end: q_pos + 1,
                param_prefix: after_q.to_string(),
            });
        }

        None
    }

    /// Get the base input for appending a query param (everything up to and including ? or &)
    fn base<'a>(&self, input: &'a str) -> &'a str {
        &input[..self.base_end]
    }
}

/// A completion item with text and optional description
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub text: String,
    pub description: Option<String>,
    /// Indices of matched characters (for highlighting)
    pub match_indices: Vec<u32>,
}

/// State for completion dropdown
#[derive(Debug, Clone)]
pub struct CompletionState {
    /// All available completion items (URI types)
    items: Vec<String>,
    /// Filtered items based on current input prefix
    filtered: Vec<CompletionItem>,
    /// Currently selected index in filtered list
    selected: Option<usize>,
    /// Scroll offset for the visible window
    scroll_offset: usize,
    /// Whether the completion dropdown is visible
    visible: bool,
    /// Query context if in query param mode, None for URI mode
    query_context: Option<QueryContext>,
}

impl CompletionState {
    fn new(items: Vec<String>) -> Self {
        let filtered = items
            .iter()
            .map(|s| CompletionItem {
                text: s.clone(),
                description: None,
                match_indices: Vec::new(),
            })
            .collect();
        Self {
            filtered,
            items,
            selected: None,
            scroll_offset: 0,
            visible: false,
            query_context: None,
        }
    }

    fn is_query_param(&self) -> bool {
        self.query_context.is_some()
    }

    fn filter(&mut self, input: &str) {
        let was_query_param = self.is_query_param();
        let new_query_context = QueryContext::parse(input);

        match &new_query_context {
            Some(ctx) => self.filter_query_params(ctx),
            None => self.filter_uris(input),
        }

        let mode_changed = was_query_param != new_query_context.is_some();
        self.query_context = new_query_context;
        self.update_selection_state(input, mode_changed);
    }

    /// Filter completions for query parameters (prefix matching)
    fn filter_query_params(&mut self, ctx: &QueryContext) {
        let prefix_lower = ctx.param_prefix.to_lowercase();
        let query_with_prefix = format!("?{}", prefix_lower);
        self.filtered = QUERY_PARAMS
            .iter()
            .filter(|(p, _)| p.to_lowercase().starts_with(&query_with_prefix))
            .map(|(text, desc)| {
                let match_indices: Vec<u32> = (0..query_with_prefix.len() as u32).collect();
                CompletionItem {
                    text: text.to_string(),
                    description: Some(desc.to_string()),
                    match_indices,
                }
            })
            .collect();
    }

    /// Filter completions for URIs (fuzzy matching with nucleo)
    fn filter_uris(&mut self, input: &str) {
        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::parse(input, CaseMatching::Smart, Normalization::Smart);

        let mut results: Vec<(String, u32, Vec<u32>)> = Vec::new();
        let mut char_buf: Vec<char> = Vec::new();
        let mut indices_buf: Vec<u32> = Vec::new();

        for item in &self.items {
            char_buf.clear();
            indices_buf.clear();
            let haystack = Utf32Str::new(item, &mut char_buf);
            if let Some(score) = pattern.indices(haystack, &mut matcher, &mut indices_buf) {
                results.push((item.clone(), score, indices_buf.clone()));
            }
        }

        results.sort_by(|a, b| b.1.cmp(&a.1));

        self.filtered = results
            .into_iter()
            .map(|(text, _, match_indices)| CompletionItem {
                text,
                description: None,
                match_indices,
            })
            .collect();
    }

    /// Update selection and scroll state after filtering
    fn update_selection_state(&mut self, input: &str, mode_changed: bool) {
        if self.filtered.is_empty() {
            self.selected = None;
            self.scroll_offset = 0;
            self.visible = false;
        } else {
            self.visible = !input.is_empty();
            if mode_changed {
                self.selected = Some(0);
                self.scroll_offset = 0;
            } else {
                match self.selected {
                    None => {
                        self.selected = Some(0);
                        self.scroll_offset = 0;
                    }
                    Some(sel) if sel >= self.filtered.len() => {
                        self.selected = Some(self.filtered.len() - 1);
                        self.scroll_offset =
                            self.filtered.len().saturating_sub(MAX_VISIBLE_COMPLETIONS);
                    }
                    _ => {}
                }
            }
        }
    }

    fn select_next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let new_selected = match self.selected {
            None => 0,
            Some(n) if n >= self.filtered.len() - 1 => 0,
            Some(n) => n + 1,
        };
        self.selected = Some(new_selected);

        // Adjust scroll to keep selection visible
        if new_selected == 0 {
            // Wrapped to top
            self.scroll_offset = 0;
        } else if new_selected >= self.scroll_offset + MAX_VISIBLE_COMPLETIONS {
            // Selection below visible window
            self.scroll_offset = new_selected + 1 - MAX_VISIBLE_COMPLETIONS;
        }
    }

    fn select_prev(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let new_selected = match self.selected {
            None => self.filtered.len() - 1,
            Some(0) => self.filtered.len() - 1,
            Some(n) => n - 1,
        };
        self.selected = Some(new_selected);

        // Adjust scroll to keep selection visible
        if new_selected == self.filtered.len() - 1 {
            // Wrapped to bottom
            self.scroll_offset = self.filtered.len().saturating_sub(MAX_VISIBLE_COMPLETIONS);
        } else if new_selected < self.scroll_offset {
            // Selection above visible window
            self.scroll_offset = new_selected;
        }
    }

    fn selected_item(&self) -> Option<&str> {
        self.selected
            .and_then(|idx| self.filtered.get(idx))
            .map(|item| item.text.as_str())
    }

    fn hide(&mut self) {
        self.visible = false;
        self.selected = None;
        self.scroll_offset = 0;
    }
}

/// Text input state machine
#[derive(Debug, Clone)]
pub struct InputState {
    input: String,
    cursor: usize,
    /// Optional completion state (None means completions disabled)
    completion: Option<CompletionState>,
}

impl InputState {
    pub fn new(default: Option<&str>) -> Self {
        let input = default.unwrap_or("").to_string();
        let cursor = input.len();
        Self {
            input,
            cursor,
            completion: None,
        }
    }

    /// Create input state with completions enabled
    pub fn with_completions(default: Option<&str>, items: Vec<String>) -> Self {
        let input = default.unwrap_or("").to_string();
        let cursor = input.len();
        let mut completion = CompletionState::new(items);
        // Filter based on initial input
        if !input.is_empty() {
            completion.filter(&input);
        }
        Self {
            input,
            cursor,
            completion: Some(completion),
        }
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

    // Completion accessors for the view

    /// Whether completions are enabled and visible with items to show
    pub fn has_visible_completions(&self) -> bool {
        self.completion
            .as_ref()
            .is_some_and(|c| c.visible && !c.filtered.is_empty())
    }

    /// Get the filtered completion items (visible window with scroll offset)
    pub fn filtered_completions(&self) -> &[CompletionItem] {
        static EMPTY: &[CompletionItem] = &[];
        self.completion
            .as_ref()
            .map(|c| {
                let start = c.scroll_offset;
                let end = (c.scroll_offset + MAX_VISIBLE_COMPLETIONS).min(c.filtered.len());
                &c.filtered[start..end]
            })
            .unwrap_or(EMPTY)
    }

    /// Get the currently selected completion index (absolute)
    pub fn selected_index(&self) -> Option<usize> {
        self.completion.as_ref().and_then(|c| c.selected)
    }

    /// Get the selected index relative to the visible window (for rendering)
    pub fn visible_selection_index(&self) -> Option<usize> {
        self.completion
            .as_ref()
            .and_then(|c| c.selected.map(|sel| sel.saturating_sub(c.scroll_offset)))
    }

    /// Get the character position where completions should be anchored
    /// For query params, this is at the ?; for normal completions, it's 0
    pub fn completion_anchor(&self) -> usize {
        self.completion
            .as_ref()
            .and_then(|c| c.query_context.as_ref())
            .map(|ctx| ctx.anchor)
            .unwrap_or(0)
    }

    /// Update completions filter based on current input
    fn update_completions(&mut self) {
        if let Some(ref mut comp) = self.completion {
            comp.filter(&self.input);
        }
    }

    /// Accept the currently selected completion
    fn accept_completion(&mut self) -> bool {
        if let Some(ref mut comp) = self.completion
            && let Some(text) = comp.selected_item()
        {
            if let Some(ref ctx) = comp.query_context {
                // Append query param: text is like "?ref=", we want "ref=" to append
                let base = ctx.base(&self.input);
                let param = text.trim_start_matches('?');
                self.input = format!("{}{}", base, param);
            } else {
                self.input = text.to_string();
            }
            self.cursor = self.input.len();
            comp.hide();
            return true;
        }
        false
    }

    /// Handle an input action, returns Some if the interaction is complete
    pub fn handle(&mut self, action: InputAction) -> Option<InputResult> {
        match action {
            InputAction::Submit => {
                // Enter always submits the user's typed input.
                // Use Tab to accept a completion instead.
                if !self.input.is_empty() {
                    return Some(InputResult::Submit(self.input.clone()));
                }
            }
            InputAction::Cancel => {
                // If completions are visible, hide them first
                if self.has_visible_completions() {
                    if let Some(ref mut comp) = self.completion {
                        comp.hide();
                    }
                    return None;
                }
                return Some(InputResult::Cancel);
            }
            InputAction::Backspace => {
                if self.cursor > 0 {
                    self.input.remove(self.cursor - 1);
                    self.cursor -= 1;
                    self.update_completions();
                }
            }
            InputAction::Delete => {
                if self.cursor < self.input.len() {
                    self.input.remove(self.cursor);
                    self.update_completions();
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
                self.update_completions();
            }
            InputAction::Insert(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
                self.update_completions();
            }
            InputAction::CompletionUp => {
                if let Some(ref mut comp) = self.completion {
                    comp.select_prev();
                }
            }
            InputAction::CompletionDown => {
                if let Some(ref mut comp) = self.completion {
                    comp.select_next();
                }
            }
            InputAction::Accept => {
                // Tab: accept if selected, otherwise select first
                if self
                    .completion
                    .as_ref()
                    .is_some_and(|c| c.selected.is_some())
                {
                    self.accept_completion();
                } else if let Some(ref mut comp) = self.completion {
                    comp.select_next(); // Select first item
                }
            }
            InputAction::ToggleDiff | InputAction::ToggleFollows | InputAction::None => {}
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

    #[test]
    fn test_completions_filter() {
        let items = vec![
            "github:".to_string(),
            "gitlab:".to_string(),
            "git+https://".to_string(),
        ];
        let mut state = InputState::with_completions(None, items);

        // No completions visible when input is empty
        assert!(!state.has_visible_completions());

        // Type "git" - should show matches (limited to visible window of 2)
        state.handle(InputAction::Insert('g'));
        state.handle(InputAction::Insert('i'));
        state.handle(InputAction::Insert('t'));
        assert!(state.has_visible_completions());
        // Only 2 visible at once, but all 3 match
        assert_eq!(state.filtered_completions().len(), 2);

        // Type "hub" -> "github" - should show only "github:" (fuzzy)
        state.handle(InputAction::Insert('h'));
        state.handle(InputAction::Insert('u'));
        state.handle(InputAction::Insert('b'));
        assert!(state.has_visible_completions());
        assert_eq!(state.filtered_completions().len(), 1);
        assert_eq!(state.filtered_completions()[0].text, "github:");

        // Type "z" -> "githubz" - no matches
        state.handle(InputAction::Insert('z'));
        assert!(!state.has_visible_completions());
    }

    #[test]
    fn test_completions_navigation() {
        // Use items that all fuzzy-match a common pattern
        let items = vec!["alpha".to_string(), "able".to_string(), "about".to_string()];
        let mut state = InputState::with_completions(None, items);

        // Type "a" - all items match fuzzy
        state.handle(InputAction::Insert('a'));
        assert!(state.has_visible_completions());
        // First item auto-selected
        assert_eq!(state.selected_index(), Some(0));

        // Down moves to second
        state.handle(InputAction::CompletionDown);
        assert_eq!(state.selected_index(), Some(1));

        // Up moves back to first
        state.handle(InputAction::CompletionUp);
        assert_eq!(state.selected_index(), Some(0));

        // Up from first wraps to last
        state.handle(InputAction::CompletionUp);
        assert_eq!(state.selected_index(), Some(2));

        // Down from last wraps to first
        state.handle(InputAction::CompletionDown);
        assert_eq!(state.selected_index(), Some(0));
    }

    #[test]
    fn test_completions_accept() {
        let items = vec!["github:".to_string(), "gitlab:".to_string()];
        let mut state = InputState::with_completions(None, items);

        // Type "g" to show completions - first item is auto-selected
        state.handle(InputAction::Insert('g'));
        assert!(state.has_visible_completions());
        assert_eq!(state.selected_index(), Some(0));

        // Accept with Tab (no need to press Down, first item already selected)
        state.handle(InputAction::Accept);
        assert_eq!(state.text(), "github:");
        assert!(!state.has_visible_completions());
    }

    #[test]
    fn test_completions_cancel_hides() {
        let items = vec!["github:".to_string()];
        let mut state = InputState::with_completions(None, items);

        // Type to show completions
        state.handle(InputAction::Insert('g'));
        assert!(state.has_visible_completions());

        // Cancel should hide completions, not return Cancel result
        let result = state.handle(InputAction::Cancel);
        assert_eq!(result, None);
        assert!(!state.has_visible_completions());

        // Cancel again should return Cancel result
        let result = state.handle(InputAction::Cancel);
        assert_eq!(result, Some(InputResult::Cancel));
    }

    #[test]
    fn test_query_param_completions() {
        let items = vec!["github:".to_string()];
        let mut state = InputState::with_completions(None, items);

        // Type a full URI then ?
        for c in "github:nixos/nixpkgs?".chars() {
            state.handle(InputAction::Insert(c));
        }

        // Should show query param completions
        assert!(state.has_visible_completions());
        let completions = state.filtered_completions();
        assert!(!completions.is_empty());
        assert!(completions.iter().any(|c| c.text.contains("ref")));

        // Type "r" to filter to ref/rev
        state.handle(InputAction::Insert('r'));
        assert!(state.has_visible_completions());
        let completions = state.filtered_completions();
        assert!(completions.iter().all(|c| c.text.contains("r")));

        // Select and accept
        state.handle(InputAction::CompletionDown);
        state.handle(InputAction::Accept);

        // Should have appended the param
        assert!(state.text().starts_with("github:nixos/nixpkgs?"));
        assert!(state.text().contains("="));
        assert!(!state.has_visible_completions());
    }

    #[test]
    fn test_query_param_no_completions_without_uri() {
        let items = vec!["github:".to_string()];
        let mut state = InputState::with_completions(None, items);

        // Type just "?" without a URI - should not show query params
        state.handle(InputAction::Insert('?'));
        // Should show URI completions (none match "?") so not visible
        assert!(!state.has_visible_completions());
    }

    #[test]
    fn test_fuzzy_matching() {
        let items = vec![
            "github:".to_string(),
            "github:mic92/vmsh".to_string(),
            "github:nixos/nixpkgs".to_string(),
        ];
        let mut state = InputState::with_completions(None, items);

        // Type "vm" - should fuzzy match "github:mic92/vmsh"
        state.handle(InputAction::Insert('v'));
        state.handle(InputAction::Insert('m'));
        assert!(state.has_visible_completions());
        let completions = state.filtered_completions();
        assert!(completions.iter().any(|c| c.text.contains("vmsh")));

        // Type "nix" - should fuzzy match nixpkgs
        let mut state2 = InputState::with_completions(
            None,
            vec![
                "github:".to_string(),
                "github:mic92/vmsh".to_string(),
                "github:nixos/nixpkgs".to_string(),
            ],
        );
        for c in "nix".chars() {
            state2.handle(InputAction::Insert(c));
        }
        assert!(state2.has_visible_completions());
        let completions = state2.filtered_completions();
        assert!(completions.iter().any(|c| c.text.contains("nixpkgs")));
    }

    #[test]
    fn test_completion_scrolling() {
        // Create more items than MAX_VISIBLE_COMPLETIONS (2)
        let items = vec![
            "item0".to_string(),
            "item1".to_string(),
            "item2".to_string(),
            "item3".to_string(),
        ];
        let mut state = InputState::with_completions(None, items);

        // Type to show all items
        state.handle(InputAction::Insert('i'));
        assert!(state.has_visible_completions());

        // Should show first 2 items (scroll_offset = 0)
        assert_eq!(state.filtered_completions().len(), 2);
        assert_eq!(state.filtered_completions()[0].text, "item0");
        assert_eq!(state.filtered_completions()[1].text, "item1");
        assert_eq!(state.visible_selection_index(), Some(0));

        // Navigate down once - still in first window
        state.handle(InputAction::CompletionDown);
        assert_eq!(state.selected_index(), Some(1));
        assert_eq!(state.visible_selection_index(), Some(1));
        assert_eq!(state.filtered_completions()[0].text, "item0");

        // Navigate down one more - should scroll
        state.handle(InputAction::CompletionDown);
        assert_eq!(state.selected_index(), Some(2));
        // Window should have scrolled
        assert_eq!(state.visible_selection_index(), Some(1)); // relative to scroll offset
        assert_eq!(state.filtered_completions()[0].text, "item1"); // scrolled by 1

        // Navigate down more
        state.handle(InputAction::CompletionDown);
        assert_eq!(state.selected_index(), Some(3));
        assert_eq!(state.filtered_completions()[0].text, "item2"); // scrolled so last 2 visible

        // Navigate down wraps to top
        state.handle(InputAction::CompletionDown);
        assert_eq!(state.selected_index(), Some(0));
        assert_eq!(state.visible_selection_index(), Some(0));
        assert_eq!(state.filtered_completions()[0].text, "item0"); // scroll reset to top

        // Navigate up from top wraps to bottom
        state.handle(InputAction::CompletionUp);
        assert_eq!(state.selected_index(), Some(3));
        // Scroll should be at end (items 2-3 visible)
        assert_eq!(state.filtered_completions()[0].text, "item2");
    }
}
