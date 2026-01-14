use ratatui::style::{Color, Modifier, Style};

pub const APP_NAME: &str = "flake-edit";

pub const BORDER_COLOR: Color = Color::DarkGray;
pub const HIGHLIGHT_COLOR: Color = Color::Cyan;
pub const PLACEHOLDER_COLOR: Color = Color::DarkGray;
pub const LABEL_BG_COLOR: Color = Color::DarkGray;
pub const LABEL_FG_COLOR: Color = Color::White;
pub const FOOTER_BG_COLOR: Color = Color::Rgb(40, 40, 40);
pub const FOOTER_FG_COLOR: Color = Color::Gray;

pub const BORDER_STYLE: Style = Style::new().fg(BORDER_COLOR);
pub const HIGHLIGHT_STYLE: Style = Style::new()
    .fg(HIGHLIGHT_COLOR)
    .add_modifier(Modifier::BOLD);
pub const PLACEHOLDER_STYLE: Style = Style::new().fg(PLACEHOLDER_COLOR);

/// Style for highlighted label boxes (like command context or app name)
pub const LABEL_STYLE: Style = Style::new()
    .bg(LABEL_BG_COLOR)
    .fg(LABEL_FG_COLOR)
    .add_modifier(Modifier::BOLD);

/// Inverse style for secondary labels (like ID)
pub const LABEL_STYLE_INVERSE: Style = Style::new()
    .fg(LABEL_BG_COLOR)
    .bg(LABEL_FG_COLOR)
    .add_modifier(Modifier::BOLD);

/// Style for the footer bar background
pub const FOOTER_STYLE: Style = Style::new().bg(FOOTER_BG_COLOR).fg(FOOTER_FG_COLOR);

// Diff coloring styles
pub const DIFF_ADD_COLOR: Color = Color::Green;
pub const DIFF_REMOVE_COLOR: Color = Color::Red;
pub const DIFF_HUNK_COLOR: Color = Color::Cyan;

pub const DIFF_ADD_STYLE: Style = Style::new().fg(DIFF_ADD_COLOR);
pub const DIFF_REMOVE_STYLE: Style = Style::new().fg(DIFF_REMOVE_COLOR);
pub const DIFF_HUNK_STYLE: Style = Style::new().fg(DIFF_HUNK_COLOR);

pub const HIGHLIGHT_SYMBOL: &str = ">> ";
pub const INPUT_PROMPT: &str = "❯ ";
