use ratatui::{
    style::Style,
    text::{Line, Span},
};

use super::style::{
    DIFF_ADD_STYLE, DIFF_HUNK_STYLE, DIFF_REMOVE_STYLE, HIGHLIGHT_COLOR, HIGHLIGHT_STYLE,
    LABEL_STYLE,
};

/// Color diff lines for display
pub fn color_diff_lines(diff: &str) -> Vec<Line<'_>> {
    diff.lines()
        .map(|line| {
            if line.starts_with('+') && !line.starts_with("+++") {
                Line::styled(line, DIFF_ADD_STYLE)
            } else if line.starts_with('-') && !line.starts_with("---") {
                Line::styled(line, DIFF_REMOVE_STYLE)
            } else if line.starts_with("@@") {
                Line::styled(line, DIFF_HUNK_STYLE)
            } else {
                Line::raw(line)
            }
        })
        .collect()
}

/// Returns (label, style) for diff toggle display
pub fn diff_toggle_style(show_diff: bool) -> (&'static str, Style) {
    if show_diff {
        ("Diff: On", HIGHLIGHT_STYLE)
    } else {
        ("Diff: Off", LABEL_STYLE)
    }
}

/// Returns (label, style) for follow toggle display
pub fn follow_toggle_style(show_follows: bool) -> (&'static str, Style) {
    if show_follows {
        ("Follow: On", HIGHLIGHT_STYLE)
    } else {
        ("Follow: Off", LABEL_STYLE)
    }
}

/// Calculate height for inline list widget
/// Accounts for top/bottom borders (+2) and footer bar (+1)
pub fn list_height(item_count: usize, max_height: u16) -> u16 {
    (item_count as u16 + 3).min(max_height)
}

/// Calculate height for diff preview widget
/// Accounts for borders (+2) and footer (+1), with min/max bounds
pub fn diff_height(line_count: usize) -> u16 {
    (line_count as u16 + 3).clamp(6, 20)
}

/// Create a styled context label span for footer
pub fn context_span(context: &str) -> Span<'_> {
    Span::styled(format!(" {} ", context), LABEL_STYLE)
}

/// Create a checkbox line for multi-select lists
pub fn checkbox_line<'a>(text: &'a str, selected: bool) -> Line<'a> {
    if selected {
        Line::from(vec![
            Span::styled("[x] ", HIGHLIGHT_STYLE),
            Span::styled(text, Style::new().fg(HIGHLIGHT_COLOR)),
        ])
    } else {
        Line::from(vec![Span::raw("[ ] "), Span::raw(text)])
    }
}

/// Standard layout helpers for consistent widget structure
pub mod layouts {
    use ratatui::layout::{Constraint, Layout, Rect};
    use std::rc::Rc;

    /// Split area into expandable content area and fixed footer bar
    pub fn content_with_footer(area: Rect) -> (Rect, Rect) {
        let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
        (chunks[0], chunks[1])
    }

    /// Split area into fixed-height content area and footer bar
    pub fn fixed_content_with_footer(area: Rect, content_height: u16) -> (Rect, Rect) {
        let chunks = Layout::vertical([Constraint::Length(content_height), Constraint::Length(1)])
            .split(area);
        (chunks[0], chunks[1])
    }

    /// Split footer into left and right columns
    pub fn footer_columns(area: Rect) -> Rc<[Rect]> {
        Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).split(area)
    }

    /// Split area into main content area and diff preview area
    /// Returns (main_area, diff_area)
    pub fn content_with_diff_preview(area: Rect, diff_lines: usize) -> (Rect, Rect) {
        // Reserve space for diff preview (borders + content lines)
        // Ensure main content gets at least 4 lines
        let diff_height = (diff_lines as u16 + 2).max(4);
        let chunks =
            Layout::vertical([Constraint::Min(4), Constraint::Length(diff_height)]).split(area);
        (chunks[0], chunks[1])
    }
}
