use ratatui::style::{Color, Modifier, Style};

pub(crate) const APP_NAME: &str = "flake-edit";

pub(crate) const BORDER_COLOR: Color = Color::DarkGray;
pub(crate) const HIGHLIGHT_COLOR: Color = Color::Cyan;
pub(crate) const PLACEHOLDER_COLOR: Color = Color::DarkGray;
pub(crate) const LABEL_BG_COLOR: Color = Color::DarkGray;
pub(crate) const LABEL_FG_COLOR: Color = Color::White;
pub(crate) const FOOTER_BG_COLOR: Color = Color::Rgb(40, 40, 40);
pub(crate) const FOOTER_FG_COLOR: Color = Color::Gray;

pub(crate) const BORDER_STYLE: Style = Style::new().fg(BORDER_COLOR);
pub(crate) const HIGHLIGHT_STYLE: Style = Style::new()
    .fg(HIGHLIGHT_COLOR)
    .add_modifier(Modifier::BOLD);
pub(crate) const PLACEHOLDER_STYLE: Style = Style::new().fg(PLACEHOLDER_COLOR);

/// Style for highlighted label boxes (like command context or app name)
pub(crate) const LABEL_STYLE: Style = Style::new()
    .bg(LABEL_BG_COLOR)
    .fg(LABEL_FG_COLOR)
    .add_modifier(Modifier::BOLD);

/// Inverse style for secondary labels (like ID)
pub(crate) const LABEL_STYLE_INVERSE: Style = Style::new()
    .fg(LABEL_BG_COLOR)
    .bg(LABEL_FG_COLOR)
    .add_modifier(Modifier::BOLD);

/// Style for the footer bar background
pub(crate) const FOOTER_STYLE: Style = Style::new().bg(FOOTER_BG_COLOR).fg(FOOTER_FG_COLOR);

/// Style for matched characters in completions (cyan on grey background)
pub(crate) const COMPLETION_MATCH_STYLE: Style =
    Style::new().bg(FOOTER_BG_COLOR).fg(HIGHLIGHT_COLOR);

/// Style for matched characters in selected completion (cyan with grey background badge)
pub(crate) const COMPLETION_SELECTED_MATCH_STYLE: Style = Style::new()
    .bg(FOOTER_BG_COLOR)
    .fg(HIGHLIGHT_COLOR)
    .add_modifier(Modifier::BOLD);

/// Dimmed style for secondary text like descriptions
pub(crate) const DIMMED_STYLE: Style = Style::new().fg(Color::DarkGray);

// Diff coloring styles
pub(crate) const DIFF_ADD_COLOR: Color = Color::Green;
pub(crate) const DIFF_REMOVE_COLOR: Color = Color::Red;
pub(crate) const DIFF_HUNK_COLOR: Color = Color::Cyan;

pub(crate) const DIFF_ADD_STYLE: Style = Style::new().fg(DIFF_ADD_COLOR);
pub(crate) const DIFF_REMOVE_STYLE: Style = Style::new().fg(DIFF_REMOVE_COLOR);
pub(crate) const DIFF_HUNK_STYLE: Style = Style::new().fg(DIFF_HUNK_COLOR);

pub(crate) const HIGHLIGHT_SYMBOL: &str = ">> ";
pub(crate) const INPUT_PROMPT: &str = "❯ ";
