use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::tui::helpers::layouts;
use crate::tui::style::{APP_NAME, FOOTER_STYLE, LABEL_STYLE};

/// Footer widget with left and right content plus app name
pub struct Footer<'a> {
    left_spans: Vec<Span<'a>>,
    right_spans: Vec<Span<'a>>,
}

impl<'a> Footer<'a> {
    pub fn new(left_spans: Vec<Span<'a>>, right_spans: Vec<Span<'a>>) -> Self {
        Self {
            left_spans,
            right_spans,
        }
    }
}

impl Widget for Footer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let footer_left = Line::from(self.left_spans);

        let mut right = self.right_spans;
        if !right.is_empty() {
            right.push(Span::raw(" "));
        }
        right.push(Span::styled(format!(" {} ", APP_NAME), LABEL_STYLE));
        let footer_right = Line::from(right).right_aligned();

        let footer_cols = layouts::footer_columns(area);
        Paragraph::new(footer_left)
            .style(FOOTER_STYLE)
            .render(footer_cols[0], buf);
        Paragraph::new(footer_right)
            .style(FOOTER_STYLE)
            .render(footer_cols[1], buf);
    }
}
