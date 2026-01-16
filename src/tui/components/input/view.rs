use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use std::collections::HashSet;

use super::model::{CompletionItem, InputState, MAX_VISIBLE_COMPLETIONS};
use crate::tui::components::footer::Footer;
use crate::tui::helpers::{context_span, diff_toggle_style, layouts};
use crate::tui::style::{
    BORDER_STYLE, COMPLETION_MATCH_STYLE, COMPLETION_SELECTED_MATCH_STYLE, DIMMED_STYLE,
    FOOTER_STYLE, HIGHLIGHT_STYLE, INPUT_PROMPT, LABEL_STYLE_INVERSE, PLACEHOLDER_STYLE,
};

/// Completion dropdown overlay widget
struct Completion<'a> {
    items: &'a [CompletionItem],
    selected: Option<usize>,
    anchor_x: u16,
}

impl<'a> Completion<'a> {
    fn new(items: &'a [CompletionItem], selected: Option<usize>, anchor_x: u16) -> Self {
        Self {
            items,
            selected,
            anchor_x,
        }
    }

    fn width(&self) -> u16 {
        let max_len = self
            .items
            .iter()
            .map(|item| {
                let desc_len = item
                    .description
                    .as_ref()
                    .map(|d| d.len() + 3) // " · " separator
                    .unwrap_or(0);
                item.text.len() + desc_len
            })
            .max()
            .unwrap_or(0);
        (max_len + 3) as u16 // 1 leading + 2 trailing padding
    }
}

impl Widget for Completion<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.items.is_empty() {
            return;
        }

        let width = self.width();
        let max_x = area.x + area.width;
        let items_to_show = self.items.len().min(MAX_VISIBLE_COMPLETIONS);

        for (i, item) in self.items.iter().take(items_to_show).enumerate() {
            let y = area.y + i as u16;
            let is_selected = Some(i) == self.selected;

            let (base_style, match_style) = if is_selected {
                (HIGHLIGHT_STYLE, COMPLETION_SELECTED_MATCH_STYLE)
            } else {
                (FOOTER_STYLE, COMPLETION_MATCH_STYLE)
            };

            let match_set: HashSet<u32> = item.match_indices.iter().copied().collect();
            let mut x = self.anchor_x;

            // Leading padding
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_char(' ');
                cell.set_style(base_style);
            }
            x += 1;

            // Completion text with match highlighting
            for (char_idx, ch) in item.text.chars().enumerate() {
                if x >= max_x || x >= self.anchor_x + width {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.reset();
                    cell.set_char(ch);
                    let style = if match_set.contains(&(char_idx as u32)) {
                        match_style
                    } else {
                        base_style
                    };
                    cell.set_style(style);
                }
                x += 1;
            }

            // Description (dimmed)
            if let Some(desc) = &item.description {
                for ch in " · ".chars().chain(desc.chars()) {
                    if x >= max_x {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.reset();
                        cell.set_char(ch);
                        cell.set_style(DIMMED_STYLE);
                    }
                    x += 1;
                }
            }

            // Trailing padding
            while x < (self.anchor_x + width).min(max_x) {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.reset();
                    cell.set_char(' ');
                    cell.set_style(base_style);
                }
                x += 1;
            }
        }
    }
}

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

    /// Calculate required height (fixed - completions overlay the footer)
    pub fn required_height(&self) -> u16 {
        4 // 3 for bordered content + 1 for footer
    }
}

impl Widget for Input<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (content_area, footer_area) = layouts::fixed_content_with_footer(area, 3);

        // Render input box
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

        // Render footer
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

        // Render completions overlay on border/footer area
        if self.state.has_visible_completions() {
            let anchor_x = content_area.x + 2 + self.state.completion_anchor() as u16;
            let overlay_area = Rect {
                x: area.x,
                y: footer_area.y.saturating_sub(1),
                width: area.width,
                height: MAX_VISIBLE_COMPLETIONS as u16,
            };
            Completion::new(
                self.state.filtered_completions(),
                self.state.visible_selection_index(),
                anchor_x,
            )
            .render(overlay_area, buf);
        }
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
