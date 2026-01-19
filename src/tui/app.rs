//! Unified TUI application model
//!
//! This module provides a unified state machine for the TUI that:
//! - Manages the complete workflow (Add/Change/Remove)
//! - Handles screen transitions internally
//! - Maintains global state (show_diff) across screens
//! - Is fully testable with pure update() and render() functions

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;

use crate::cache::CacheConfig;
use crate::change::Change;
use crate::cli::Command;
use crate::lock::NestedInput;

use super::completions::uri_completion_items;
use super::components::confirm::ConfirmAction;
use super::components::input::{Input, InputAction, InputResult, InputState};
use super::components::list::{ListAction, ListResult, ListState};
use super::workflow::{AddPhase, ConfirmResultAction, FollowPhase, WorkflowData};

// Re-export workflow types that are part of the public API
pub use super::workflow::{AppResult, MultiSelectResultData, SingleSelectResult, UpdateResult};

const MAX_LIST_HEIGHT: u16 = 12;

#[derive(Debug, Clone)]
pub struct App {
    context: String,
    flake_text: String,
    show_diff: bool,
    cache_config: CacheConfig,
    screen: Screen,
    data: WorkflowData,
}

/// The current screen being displayed
#[derive(Debug, Clone)]
pub enum Screen {
    Input(InputScreen),
    List(ListScreen),
    Confirm(ConfirmScreen),
}

/// Input screen state
#[derive(Debug, Clone)]
pub struct InputScreen {
    pub state: InputState,
    pub prompt: String,
    pub label: Option<String>,
}

/// List screen state (unified single and multi-select)
#[derive(Debug, Clone)]
pub struct ListScreen {
    pub state: ListState,
    pub items: Vec<String>,
    pub prompt: String,
}

impl ListScreen {
    pub fn single(items: Vec<String>, prompt: impl Into<String>, show_diff: bool) -> Self {
        let len = items.len();
        Self {
            state: ListState::new(len, false, show_diff),
            items,
            prompt: prompt.into(),
        }
    }

    pub fn multi(items: Vec<String>, prompt: impl Into<String>, show_diff: bool) -> Self {
        let len = items.len();
        Self {
            state: ListState::new(len, true, show_diff),
            items,
            prompt: prompt.into(),
        }
    }
}

/// Confirm screen state
#[derive(Debug, Clone)]
pub struct ConfirmScreen {
    pub diff: String,
}

impl App {
    /// Create a new App for the Add workflow
    ///
    /// The workflow asks for URI first, then ID (with inferred ID as default).
    /// Optionally provide a prefill_uri if the user provided a URI on the command line.
    pub fn add(
        context: impl Into<String>,
        flake_text: impl Into<String>,
        prefill_uri: Option<&str>,
        cache_config: CacheConfig,
    ) -> Self {
        let completions = uri_completion_items(None, &cache_config);
        Self {
            context: context.into(),
            flake_text: flake_text.into(),
            show_diff: false,
            cache_config,
            screen: Screen::Input(InputScreen {
                state: InputState::with_completions(prefill_uri, completions),
                prompt: "Enter flake URI".into(),
                label: None,
            }),
            data: WorkflowData::Add {
                phase: AddPhase::Uri,
                uri: None,
                id: None,
            },
        }
    }

    /// Create a new App for the Change workflow
    ///
    /// Takes a list of (id, current_uri) pairs. The current_uri is used to
    /// prefill the input field when an item is selected.
    pub fn change(
        context: impl Into<String>,
        flake_text: impl Into<String>,
        inputs: Vec<(String, String)>,
        cache_config: CacheConfig,
    ) -> Self {
        let input_ids: Vec<String> = inputs.iter().map(|(id, _)| id.clone()).collect();
        let input_uris: std::collections::HashMap<String, String> = inputs.into_iter().collect();
        Self {
            context: context.into(),
            flake_text: flake_text.into(),
            show_diff: false,
            cache_config,
            screen: Screen::List(ListScreen::single(
                input_ids.clone(),
                "Select input to change",
                false,
            )),
            data: WorkflowData::Change {
                selected_input: None,
                uri: None,
                input_uris,
                all_inputs: input_ids,
            },
        }
    }

    /// Create a new App for the Remove workflow
    pub fn remove(
        context: impl Into<String>,
        flake_text: impl Into<String>,
        inputs: Vec<String>,
    ) -> Self {
        Self {
            context: context.into(),
            flake_text: flake_text.into(),
            show_diff: false,
            cache_config: CacheConfig::default(),
            screen: Screen::List(ListScreen::multi(
                inputs.clone(),
                "Select inputs to remove",
                false,
            )),
            data: WorkflowData::Remove {
                selected_inputs: Vec::new(),
                all_inputs: inputs,
            },
        }
    }

    /// Create a new App for the Change workflow when ID is already known
    ///
    /// This is a single-screen variant that just asks for the new URI.
    pub fn change_uri(
        context: impl Into<String>,
        flake_text: impl Into<String>,
        id: impl Into<String>,
        current_uri: Option<&str>,
        show_diff: bool,
        cache_config: CacheConfig,
    ) -> Self {
        let id_string = id.into();
        let completions = uri_completion_items(Some(&id_string), &cache_config);
        Self {
            context: context.into(),
            flake_text: flake_text.into(),
            show_diff,
            cache_config,
            screen: Screen::Input(InputScreen {
                state: InputState::with_completions(current_uri, completions),
                prompt: format!("for {}", id_string),
                label: Some("URI".into()),
            }),
            data: WorkflowData::Change {
                selected_input: Some(id_string),
                uri: None,
                input_uris: std::collections::HashMap::new(),
                all_inputs: Vec::new(),
            },
        }
    }

    /// Create a standalone single-select App (for Pin/Unpin workflows)
    pub fn select_one(
        context: impl Into<String>,
        prompt: impl Into<String>,
        items: Vec<String>,
        initial_diff: bool,
    ) -> Self {
        Self {
            context: context.into(),
            flake_text: String::new(),
            show_diff: initial_diff,
            cache_config: CacheConfig::default(),
            screen: Screen::List(ListScreen::single(items, prompt, initial_diff)),
            data: WorkflowData::SelectOne {
                selected_input: None,
            },
        }
    }

    /// Create a standalone multi-select App (for Update workflow)
    pub fn select_many(
        context: impl Into<String>,
        prompt: impl Into<String>,
        items: Vec<String>,
        initial_diff: bool,
    ) -> Self {
        Self {
            context: context.into(),
            flake_text: String::new(),
            show_diff: initial_diff,
            cache_config: CacheConfig::default(),
            screen: Screen::List(ListScreen::multi(items, prompt, initial_diff)),
            data: WorkflowData::SelectMany {
                selected_inputs: Vec::new(),
            },
        }
    }

    /// Create a standalone confirmation App with pre-computed diff
    pub fn confirm(context: impl Into<String>, diff: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            flake_text: String::new(),
            show_diff: true,
            cache_config: CacheConfig::default(),
            screen: Screen::Confirm(ConfirmScreen { diff: diff.into() }),
            data: WorkflowData::ConfirmOnly { action: None },
        }
    }

    /// Create a new App for the Follow workflow
    ///
    /// Shows a list of nested inputs to select from, then a list of targets.
    /// `nested_inputs` contains the nested input paths with their existing follows info
    /// `top_level_inputs` are the available targets like "nixpkgs", "flake-utils"
    pub fn follow(
        context: impl Into<String>,
        flake_text: impl Into<String>,
        nested_inputs: Vec<NestedInput>,
        top_level_inputs: Vec<String>,
    ) -> Self {
        // Convert to display strings for the UI
        let display_items: Vec<String> = nested_inputs
            .iter()
            .map(|i| i.to_display_string())
            .collect();
        Self {
            context: context.into(),
            flake_text: flake_text.into(),
            show_diff: false,
            cache_config: CacheConfig::default(),
            screen: Screen::List(ListScreen::single(
                display_items,
                "Select input to add follows",
                false,
            )),
            data: WorkflowData::Follow {
                phase: FollowPhase::SelectInput,
                selected_input: None,
                selected_target: None,
                nested_inputs,
                top_level_inputs,
            },
        }
    }

    /// Create a new App for the Follow workflow when input is already selected
    ///
    /// Shows only the target selection list.
    pub fn follow_target(
        context: impl Into<String>,
        flake_text: impl Into<String>,
        input: impl Into<String>,
        top_level_inputs: Vec<String>,
    ) -> Self {
        let input = input.into();
        Self {
            context: context.into(),
            flake_text: flake_text.into(),
            show_diff: false,
            cache_config: CacheConfig::default(),
            screen: Screen::List(ListScreen::single(
                top_level_inputs.clone(),
                format!("Select target for {input}"),
                false,
            )),
            data: WorkflowData::Follow {
                phase: FollowPhase::SelectTarget,
                selected_input: Some(input),
                selected_target: None,
                nested_inputs: Vec::<NestedInput>::new(),
                top_level_inputs,
            },
        }
    }

    /// Create an App from a CLI Command.
    ///
    /// This is the main entry point for creating TUI apps from parsed CLI arguments.
    /// Returns `None` if the command doesn't need interactive TUI (e.g., all args provided,
    /// or it's a non-interactive command like List).
    ///
    /// # Arguments
    /// * `command` - The parsed CLI command
    /// * `flake_text` - The content of the flake.nix file
    /// * `inputs` - List of (id, uri) pairs representing current inputs
    /// * `diff` - Whether diff mode is enabled
    /// * `cache_config` - Cache configuration for completions
    pub fn from_command(
        command: &Command,
        flake_text: impl Into<String>,
        inputs: Vec<(String, String)>,
        diff: bool,
        cache_config: CacheConfig,
    ) -> Option<Self> {
        let flake_text = flake_text.into();
        let input_ids: Vec<String> = inputs.iter().map(|(id, _)| id.clone()).collect();

        match command {
            // Add: interactive if no id+uri provided
            Command::Add { id, uri, .. } => {
                if id.is_some() && uri.is_some() {
                    None // Non-interactive: all args provided
                } else {
                    // prefill_uri is the first positional arg (id field) when uri is None
                    let prefill = id.as_deref();
                    Some(Self::add("Add", flake_text, prefill, cache_config).with_diff(diff))
                }
            }

            // Remove: interactive if no id provided
            Command::Remove { id } => {
                if id.is_some() {
                    None
                } else {
                    Some(Self::remove("Remove", flake_text, input_ids).with_diff(diff))
                }
            }

            // Change: multiple interactive modes
            Command::Change { id, uri, .. } => {
                if id.is_some() && uri.is_some() {
                    None // Non-interactive: all args provided
                } else if let Some(id) = id {
                    // ID provided but no URI: show URI input for that specific input
                    let current_uri = inputs
                        .iter()
                        .find(|(i, _)| i == id)
                        .map(|(_, u)| u.as_str());
                    Some(Self::change_uri(
                        "Change",
                        flake_text,
                        id,
                        current_uri,
                        diff,
                        cache_config,
                    ))
                } else {
                    // No args: show input selection list
                    Some(Self::change("Change", flake_text, inputs, cache_config).with_diff(diff))
                }
            }

            // Pin: interactive if no id provided
            Command::Pin { id, .. } => {
                if id.is_some() {
                    None
                } else {
                    Some(Self::select_one(
                        "Pin",
                        "Select input to pin",
                        input_ids,
                        diff,
                    ))
                }
            }

            // Unpin: interactive if no id provided
            Command::Unpin { id } => {
                if id.is_some() {
                    None
                } else {
                    Some(Self::select_one(
                        "Unpin",
                        "Select input to unpin",
                        input_ids,
                        diff,
                    ))
                }
            }

            // Update: interactive if no id provided
            Command::Update { id, .. } => {
                if id.is_some() {
                    None
                } else {
                    Some(Self::select_many(
                        "Update",
                        "Space select, U all, ^D diff",
                        input_ids,
                        diff,
                    ))
                }
            }

            // These commands handle their own interactivity or don't need TUI
            Command::List { .. }
            | Command::Completion { .. }
            | Command::Follow { .. }
            | Command::AddFollow { .. }
            | Command::Config { .. } => None,
        }
    }

    pub fn show_diff(&self) -> bool {
        self.show_diff
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    /// Get the Change that would be applied based on current workflow state.
    /// Useful for testing to verify what modification the TUI would produce.
    pub fn pending_change(&self) -> Change {
        self.build_change()
    }

    /// Compute the diff string for the current change against the flake text.
    /// Returns the unified diff showing what would change.
    ///
    /// This looks at the current screen state (including list selections)
    /// to compute a live preview of what would happen.
    pub fn pending_diff(&self) -> String {
        let change = self.build_preview_change();
        self.compute_diff(&change)
    }

    /// Build a Change based on current screen state for live preview.
    /// Unlike build_change(), this looks at current screen input/selections.
    fn build_preview_change(&self) -> Change {
        match &self.screen {
            // For Input screens, use current input text
            Screen::Input(screen) => {
                let current_text = screen.state.text();
                if current_text.is_empty() {
                    return Change::None;
                }
                match &self.data {
                    WorkflowData::Add { phase, uri, .. } => match phase {
                        AddPhase::Uri => Change::Add {
                            id: None,
                            uri: Some(current_text.to_string()),
                            flake: true,
                        },
                        AddPhase::Id => Change::Add {
                            id: Some(current_text.to_string()),
                            uri: uri.clone(),
                            flake: true,
                        },
                    },
                    WorkflowData::Change { selected_input, .. } => Change::Change {
                        id: selected_input.clone(),
                        uri: Some(current_text.to_string()),
                        ref_or_rev: None,
                    },
                    _ => self.build_change(),
                }
            }
            // For List screens, use current selections
            Screen::List(screen) => {
                let selected_items: Vec<String> = screen
                    .state
                    .selected_indices()
                    .iter()
                    .filter_map(|&i| screen.items.get(i).cloned())
                    .collect();

                if !selected_items.is_empty() {
                    return match &self.data {
                        WorkflowData::Remove { .. } => Change::Remove {
                            ids: selected_items.into_iter().map(|s| s.into()).collect(),
                        },
                        WorkflowData::Follow {
                            phase,
                            selected_input,
                            ..
                        } => {
                            // During SelectInput phase, we can't preview yet
                            // During SelectTarget phase, use selected_input + current selection
                            if *phase == FollowPhase::SelectTarget {
                                if let Some(input) = selected_input {
                                    let target =
                                        selected_items.into_iter().next().unwrap_or_default();
                                    Change::Follows {
                                        input: input.clone().into(),
                                        target,
                                    }
                                } else {
                                    Change::None
                                }
                            } else {
                                // SelectInput phase - no preview possible
                                Change::None
                            }
                        }
                        _ => self.build_change(),
                    };
                }
                if matches!(
                    &self.data,
                    WorkflowData::Follow { .. } | WorkflowData::Change { .. }
                ) {
                    return Change::None;
                }
                self.build_change()
            }
            Screen::Confirm(_) => self.build_change(),
        }
    }

    /// Set the initial diff mode
    pub fn with_diff(mut self, show_diff: bool) -> Self {
        self.show_diff = show_diff;
        // Also update ListState if we're on a list screen
        if let Screen::List(ref mut screen) = self.screen {
            screen.state =
                ListState::new(screen.items.len(), screen.state.multi_select(), show_diff);
        }
        self
    }

    pub fn update(&mut self, key: KeyEvent) -> UpdateResult {
        let screen = self.screen.clone();
        match screen {
            Screen::Input(s) => self.update_input(s, key),
            Screen::List(s) => self.update_list(s, key),
            Screen::Confirm(_) => self.update_confirm(key),
        }
    }

    fn update_input(&mut self, mut screen: InputScreen, key: KeyEvent) -> UpdateResult {
        let action = InputAction::from_key(key);
        match action {
            InputAction::ToggleDiff => {
                self.show_diff = !self.show_diff;
                UpdateResult::Continue
            }
            _ => {
                if let Some(result) = screen.state.handle(action) {
                    match result {
                        InputResult::Submit(text) => self.handle_input_submit(text),
                        InputResult::Cancel => {
                            // In Add workflow, Escape from ID input goes back to URI input
                            if let WorkflowData::Add { phase, uri, .. } = &mut self.data
                                && *phase == AddPhase::Id
                            {
                                *phase = AddPhase::Uri;
                                self.screen = Screen::Input(InputScreen {
                                    state: InputState::with_completions(
                                        uri.as_deref(),
                                        uri_completion_items(None, &self.cache_config),
                                    ),
                                    prompt: "Enter flake URI".into(),
                                    label: None,
                                });
                                return UpdateResult::Continue;
                            }
                            // In Change workflow, Escape from URI input goes back to list
                            if let WorkflowData::Change { all_inputs, .. } = &self.data
                                && !all_inputs.is_empty()
                            {
                                self.screen = Screen::List(ListScreen::single(
                                    all_inputs.clone(),
                                    "Select input to change",
                                    self.show_diff,
                                ));
                                return UpdateResult::Continue;
                            }
                            UpdateResult::Cancelled
                        }
                    }
                } else {
                    if let Screen::Input(s) = &mut self.screen {
                        s.state = screen.state;
                    }
                    UpdateResult::Continue
                }
            }
        }
    }

    fn update_list(&mut self, mut screen: ListScreen, key: KeyEvent) -> UpdateResult {
        let action = ListAction::from_key(key);
        if let Some(result) = screen.state.handle(action) {
            match result {
                ListResult::Select(indices, show_diff) => {
                    self.show_diff = show_diff;
                    let items: Vec<String> =
                        indices.iter().map(|&i| screen.items[i].clone()).collect();
                    self.handle_list_submit(indices, items)
                }
                ListResult::Cancel => {
                    // For Follow workflow, Escape from SelectTarget goes back to SelectInput
                    if let WorkflowData::Follow {
                        phase,
                        nested_inputs,
                        ..
                    } = &mut self.data
                        && *phase == FollowPhase::SelectTarget
                        && !nested_inputs.is_empty()
                    {
                        *phase = FollowPhase::SelectInput;
                        let display_items: Vec<String> = nested_inputs
                            .iter()
                            .map(|i| i.to_display_string())
                            .collect();
                        self.screen = Screen::List(ListScreen::single(
                            display_items,
                            "Select input to add follows",
                            self.show_diff,
                        ));
                        return UpdateResult::Continue;
                    }
                    UpdateResult::Cancelled
                }
            }
        } else {
            if let Screen::List(s) = &mut self.screen {
                s.state = screen.state;
            }
            UpdateResult::Continue
        }
    }

    fn update_confirm(&mut self, key: KeyEvent) -> UpdateResult {
        let action = ConfirmAction::from_key(key);
        match action {
            ConfirmAction::Apply => {
                if let WorkflowData::ConfirmOnly { action, .. } = &mut self.data {
                    *action = Some(ConfirmResultAction::Apply);
                }
                UpdateResult::Done
            }
            ConfirmAction::Back => {
                // For ConfirmOnly workflow, Back is a result, not a navigation
                if let WorkflowData::ConfirmOnly { action, .. } = &mut self.data {
                    *action = Some(ConfirmResultAction::Back);
                    UpdateResult::Done
                } else {
                    self.go_back();
                    UpdateResult::Continue
                }
            }
            ConfirmAction::Exit => {
                if let WorkflowData::ConfirmOnly { action, .. } = &mut self.data {
                    *action = Some(ConfirmResultAction::Exit);
                }
                UpdateResult::Cancelled
            }
            ConfirmAction::None => UpdateResult::Continue,
        }
    }

    fn handle_input_submit(&mut self, text: String) -> UpdateResult {
        match &mut self.data {
            WorkflowData::Add { phase, uri, id } => match phase {
                AddPhase::Uri => {
                    let (inferred_id, normalized_uri) = Self::parse_uri_and_infer_id(&text);
                    *uri = Some(normalized_uri);
                    *phase = AddPhase::Id;
                    self.screen = Screen::Input(InputScreen {
                        state: InputState::new(inferred_id.as_deref()),
                        prompt: format!("for {}", text),
                        label: Some("ID".into()),
                    });
                    UpdateResult::Continue
                }
                AddPhase::Id => {
                    *id = Some(text);
                    self.transition_to_confirm()
                }
            },
            WorkflowData::Change { uri, .. } => {
                *uri = Some(text);
                self.transition_to_confirm()
            }
            // These workflows don't use input screens
            WorkflowData::Remove { .. }
            | WorkflowData::SelectOne { .. }
            | WorkflowData::SelectMany { .. }
            | WorkflowData::ConfirmOnly { .. }
            | WorkflowData::Follow { .. } => UpdateResult::Continue,
        }
    }

    fn handle_list_submit(&mut self, indices: Vec<usize>, items: Vec<String>) -> UpdateResult {
        match &mut self.data {
            WorkflowData::Change {
                selected_input,
                input_uris,
                ..
            } => {
                // Single-select: take first item
                let item = items.into_iter().next().unwrap_or_default();
                let current_uri = input_uris.get(&item).map(|s| s.as_str());
                *selected_input = Some(item.clone());
                self.screen = Screen::Input(InputScreen {
                    state: InputState::with_completions(
                        current_uri,
                        uri_completion_items(Some(&item), &self.cache_config),
                    ),
                    prompt: "Enter new URI".into(),
                    label: Some(item),
                });
                UpdateResult::Continue
            }
            WorkflowData::SelectOne { selected_input } => {
                // Single-select: take first item
                *selected_input = items.into_iter().next();
                UpdateResult::Done
            }
            WorkflowData::Remove {
                selected_inputs, ..
            } => {
                *selected_inputs = items;
                self.transition_to_confirm()
            }
            WorkflowData::SelectMany { selected_inputs } => {
                *selected_inputs = items;
                UpdateResult::Done
            }
            WorkflowData::Follow {
                phase,
                selected_input,
                selected_target,
                nested_inputs,
                top_level_inputs,
            } => {
                match phase {
                    FollowPhase::SelectInput => {
                        // Use index to look up the path from nested_inputs
                        let index = indices.first().copied().unwrap_or(0);
                        let path = nested_inputs
                            .get(index)
                            .map(|i| i.path.clone())
                            .unwrap_or_default();
                        *selected_input = Some(path.clone());
                        *phase = FollowPhase::SelectTarget;
                        self.screen = Screen::List(ListScreen::single(
                            top_level_inputs.clone(),
                            format!("Select target for {path}"),
                            self.show_diff,
                        ));
                        UpdateResult::Continue
                    }
                    FollowPhase::SelectTarget => {
                        let item = items.into_iter().next().unwrap_or_default();
                        *selected_target = Some(item);
                        self.transition_to_confirm()
                    }
                }
            }
            _ => UpdateResult::Continue,
        }
    }

    fn transition_to_confirm(&mut self) -> UpdateResult {
        if !self.show_diff {
            return UpdateResult::Done;
        }

        let change = self.build_change();
        let diff_str = self.compute_diff(&change);
        self.screen = Screen::Confirm(ConfirmScreen { diff: diff_str });
        UpdateResult::Continue
    }

    fn go_back(&mut self) {
        match &mut self.data {
            WorkflowData::Add { phase, id, uri } => {
                *phase = AddPhase::Id;
                self.screen = Screen::Input(InputScreen {
                    state: InputState::new(id.as_deref()),
                    prompt: format!("for {}", uri.as_deref().unwrap_or("")),
                    label: Some("ID".into()),
                });
            }
            WorkflowData::Change {
                selected_input,
                uri,
                ..
            } => {
                self.screen = Screen::Input(InputScreen {
                    state: InputState::with_completions(
                        uri.as_deref(),
                        uri_completion_items(selected_input.as_deref(), &self.cache_config),
                    ),
                    prompt: "Enter new URI".into(),
                    label: selected_input.clone(),
                });
            }
            WorkflowData::Remove { all_inputs, .. } => {
                self.screen = Screen::List(ListScreen::multi(
                    all_inputs.clone(),
                    "Select inputs to remove",
                    self.show_diff,
                ));
            }
            WorkflowData::Follow {
                phase,
                nested_inputs,
                top_level_inputs,
                ..
            } => {
                // go_back is called from confirm screen, so we need to go back to
                // the target selection list (SelectTarget phase)
                if *phase == FollowPhase::SelectTarget {
                    self.screen = Screen::List(ListScreen::single(
                        top_level_inputs.clone(),
                        "Select target to follow",
                        self.show_diff,
                    ));
                } else if !nested_inputs.is_empty() {
                    // SelectInput phase - go back to input selection
                    let display_items: Vec<String> = nested_inputs
                        .iter()
                        .map(|i| i.to_display_string())
                        .collect();
                    self.screen = Screen::List(ListScreen::single(
                        display_items,
                        "Select input to add follows",
                        self.show_diff,
                    ));
                }
            }
            // Standalone workflows don't have a "back" concept - they're single screen
            WorkflowData::SelectOne { .. }
            | WorkflowData::SelectMany { .. }
            | WorkflowData::ConfirmOnly { .. } => {}
        }
    }

    fn build_change(&self) -> Change {
        self.data.build_change()
    }

    fn compute_diff(&self, change: &Change) -> String {
        super::workflow::compute_diff(&self.flake_text, change)
    }

    fn parse_uri_and_infer_id(uri: &str) -> (Option<String>, String) {
        super::workflow::parse_uri_and_infer_id(uri)
    }

    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        match &self.screen {
            Screen::Input(screen) => {
                let input = Input::new(
                    &screen.state,
                    &screen.prompt,
                    &self.context,
                    screen.label.as_deref(),
                    self.show_diff,
                );
                Some(input.cursor_position(area))
            }
            _ => None,
        }
    }

    pub fn terminal_height(&self) -> u16 {
        match &self.screen {
            Screen::Input(screen) => {
                let input = Input::new(
                    &screen.state,
                    &screen.prompt,
                    &self.context,
                    screen.label.as_deref(),
                    self.show_diff,
                );
                input.required_height()
            }
            Screen::List(s) => super::helpers::list_height(s.items.len(), MAX_LIST_HEIGHT),
            Screen::Confirm(s) => super::helpers::diff_height(s.diff.lines().count()),
        }
    }

    pub fn extract_result(self) -> Option<AppResult> {
        match self.data {
            WorkflowData::Add { .. }
            | WorkflowData::Change { .. }
            | WorkflowData::Remove { .. }
            | WorkflowData::Follow { .. } => {
                let change = self.build_change();
                if matches!(change, Change::None) {
                    None
                } else {
                    Some(AppResult::Change(change))
                }
            }
            WorkflowData::SelectOne { selected_input } => selected_input.map(|item| {
                AppResult::SingleSelect(SingleSelectResult {
                    item,
                    show_diff: self.show_diff,
                })
            }),
            WorkflowData::SelectMany { selected_inputs } => {
                if selected_inputs.is_empty() {
                    None
                } else {
                    Some(AppResult::MultiSelect(MultiSelectResultData {
                        items: selected_inputs,
                        show_diff: self.show_diff,
                    }))
                }
            }
            WorkflowData::ConfirmOnly { action } => action.map(AppResult::Confirm),
        }
    }
}
