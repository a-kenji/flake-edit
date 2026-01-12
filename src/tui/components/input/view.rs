use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::model::InputState;
use crate::tui::components::footer::Footer;
use crate::tui::helpers::{context_span, diff_toggle_style, layouts};
use crate::tui::style::{BORDER_STYLE, INPUT_PROMPT, LABEL_STYLE_INVERSE, PLACEHOLDER_STYLE};

/// Input widget for text entry
pub struct Input<'a> {
    state: &'a InputState,
    prompt: &'a str,
    context: &'a str,
    label: Option<&'a str>,
    show_diff: bool,
}

impl<'a> Input<'a> {
    pub fn new(
        state: &'a InputState,
        prompt: &'a str,
        context: &'a str,
        label: Option<&'a str>,
        show_diff: bool,
    ) -> Self {
        Self {
            state,
            prompt,
            context,
            label,
            show_diff,
        }
    }

    /// Calculate cursor position for the given area
    pub fn cursor_position(&self, area: Rect) -> (u16, u16) {
        let (content_area, _) = layouts::fixed_content_with_footer(area, 3);
        let cursor_x = content_area.x + 2 + self.state.cursor() as u16;
        let cursor_y = content_area.y + 1;
        (cursor_x, cursor_y)
    }
}

impl Widget for Input<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (content_area, footer_area) = layouts::fixed_content_with_footer(area, 3);

        let display_text = if self.state.is_empty() {
            Line::from(vec![
                Span::raw(INPUT_PROMPT),
                Span::styled("Type here...", PLACEHOLDER_STYLE),
            ])
        } else {
            Line::from(vec![Span::raw(INPUT_PROMPT), Span::raw(self.state.text())])
        };
        let content = Paragraph::new(display_text).block(
            Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(BORDER_STYLE),
        );
        content.render(content_area, buf);

        let mut footer_spans = vec![context_span(self.context)];
        if let Some(lbl) = self.label {
            footer_spans.push(Span::raw(" "));
            footer_spans.push(Span::styled(format!(" {} ", lbl), LABEL_STYLE_INVERSE));
        }
        footer_spans.push(Span::raw(format!(" {}", self.prompt)));

        let (diff_label, diff_style) = diff_toggle_style(self.show_diff);
        Footer::new(
            footer_spans,
            vec![Span::styled(format!(" {} ", diff_label), diff_style)],
        )
        .render(footer_area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn create_test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        Terminal::new(backend).unwrap()
    }

    fn buffer_to_plain_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let mut lines = Vec::new();
        for y in 0..buffer.area.height {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push(buffer[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            lines.push(line.trim_end().to_string());
        }
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }

    #[test]
    fn test_render_input_empty() {
        let mut terminal = create_test_terminal(80, 4);
        let state = InputState::new(None);

        terminal
            .draw(|frame| {
                Input::new(&state, "Enter URI", "Add", None, false)
                    .render(frame.area(), frame.buffer_mut());
            })
            .unwrap();

        let output = buffer_to_plain_text(&terminal);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn test_render_input_with_text() {
        let mut terminal = create_test_terminal(80, 4);
        let state = InputState::new(Some("github:nixos/nixpkgs"));

        terminal
            .draw(|frame| {
                Input::new(&state, "Enter URI", "Add", None, true)
                    .render(frame.area(), frame.buffer_mut());
            })
            .unwrap();

        let output = buffer_to_plain_text(&terminal);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn test_render_input_with_label() {
        let mut terminal = create_test_terminal(80, 4);
        let state = InputState::new(Some("nixpkgs"));

        terminal
            .draw(|frame| {
                Input::new(&state, "for github:nixos/nixpkgs", "Add", Some("ID"), false)
                    .render(frame.area(), frame.buffer_mut());
            })
            .unwrap();

        let output = buffer_to_plain_text(&terminal);
        insta::assert_snapshot!(output);
    }
}
