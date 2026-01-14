use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Span,
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use crate::tui::components::footer::Footer;
use crate::tui::helpers::{color_diff_lines, context_span, layouts};
use crate::tui::style::{BORDER_STYLE, HIGHLIGHT_STYLE};

/// Confirm widget that displays a diff and asks for confirmation
pub struct Confirm<'a> {
    diff: &'a str,
    context: &'a str,
}

impl<'a> Confirm<'a> {
    pub fn new(diff: &'a str, context: &'a str) -> Self {
        Self { diff, context }
    }
}

impl Widget for Confirm<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (content_area, footer_area) = layouts::content_with_footer(area);

        let content = Paragraph::new(color_diff_lines(self.diff))
            .block(
                Block::default()
                    .borders(Borders::TOP | Borders::BOTTOM)
                    .border_style(BORDER_STYLE),
            )
            .wrap(Wrap { trim: false });
        content.render(content_area, buf);

        Footer::new(
            vec![
                context_span(self.context),
                Span::raw(" Apply? "),
                Span::styled(" y ", HIGHLIGHT_STYLE),
                Span::raw("es "),
                Span::styled(" n ", HIGHLIGHT_STYLE),
                Span::raw("o "),
                Span::styled(" b ", HIGHLIGHT_STYLE),
                Span::raw("ack"),
            ],
            vec![],
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
    fn test_render_confirm() {
        let mut terminal = create_test_terminal(80, 10);
        let diff = r#"@@ -1,3 +1,3 @@
 inputs = {
-  nixpkgs.url = "github:nixos/nixpkgs";
+  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
 };"#;

        terminal
            .draw(|frame| {
                Confirm::new(diff, "Change").render(frame.area(), frame.buffer_mut());
            })
            .unwrap();

        let output = buffer_to_plain_text(&terminal);
        insta::assert_snapshot!(output);
    }
}
