use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, List as RatatuiList, ListItem, ListState, StatefulWidget, Widget},
};

use super::model::ListState as SelectionState;
use crate::tui::components::footer::Footer;
use crate::tui::helpers::{checkbox_line, context_span, diff_toggle_style, layouts};
use crate::tui::style::{BORDER_STYLE, HIGHLIGHT_STYLE, HIGHLIGHT_SYMBOL};

/// Unified list widget for single and multi-select
pub struct List<'a> {
    state: &'a SelectionState,
    items: &'a [String],
    prompt: &'a str,
    context: &'a str,
}

impl<'a> List<'a> {
    pub fn new(
        state: &'a SelectionState,
        items: &'a [String],
        prompt: &'a str,
        context: &'a str,
    ) -> Self {
        Self {
            state,
            items,
            prompt,
            context,
        }
    }
}

impl Widget for List<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (content_area, footer_area) = layouts::content_with_footer(area);

        let mut list_state = ListState::default();
        list_state.select(Some(self.state.cursor()));

        let list_items: Vec<ListItem> = if self.state.multi_select() {
            self.items
                .iter()
                .enumerate()
                .map(|(i, item)| ListItem::new(checkbox_line(item, self.state.is_selected(i))))
                .collect()
        } else {
            self.items
                .iter()
                .map(|item| ListItem::new(Line::raw(item.as_str())))
                .collect()
        };

        let mut list = RatatuiList::new(list_items)
            .block(
                Block::default()
                    .borders(Borders::TOP | Borders::BOTTOM)
                    .border_style(BORDER_STYLE),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL);

        // Single-select uses highlight style, multi-select doesn't
        if !self.state.multi_select() {
            list = list.highlight_style(HIGHLIGHT_STYLE);
        }

        StatefulWidget::render(list, content_area, buf, &mut list_state);

        // Footer with optional selection count for multi-select
        let count_info = if self.state.multi_select() && self.state.selected_count() > 0 {
            format!(" ({} selected)", self.state.selected_count())
        } else {
            String::new()
        };

        let (diff_label, diff_style) = diff_toggle_style(self.state.show_diff());
        Footer::new(
            vec![
                context_span(self.context),
                Span::raw(format!(" {}{}", self.prompt, count_info)),
            ],
            vec![Span::styled(format!(" {} ", diff_label), diff_style)],
        )
        .render(footer_area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::components::list::ListAction;
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
    fn test_render_single_select() {
        let mut terminal = create_test_terminal(80, 8);
        let items = vec![
            "nixpkgs".to_string(),
            "home-manager".to_string(),
            "flake-utils".to_string(),
        ];
        let state = SelectionState::new(items.len(), false, false);

        terminal
            .draw(|frame| {
                List::new(&state, &items, "Select input", "Change")
                    .render(frame.area(), frame.buffer_mut());
            })
            .unwrap();

        let output = buffer_to_plain_text(&terminal);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn test_render_single_select_with_diff_on() {
        let mut terminal = create_test_terminal(80, 8);
        let items = vec!["nixpkgs".to_string(), "home-manager".to_string()];
        let mut state = SelectionState::new(items.len(), false, true);
        state.handle(ListAction::Down);

        terminal
            .draw(|frame| {
                List::new(&state, &items, "Select input", "Change")
                    .render(frame.area(), frame.buffer_mut());
            })
            .unwrap();

        let output = buffer_to_plain_text(&terminal);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn test_render_multi_select() {
        let mut terminal = create_test_terminal(80, 8);
        let items = vec![
            "nixpkgs".to_string(),
            "home-manager".to_string(),
            "flake-utils".to_string(),
        ];
        let mut state = SelectionState::new(items.len(), true, false);
        state.handle(ListAction::Toggle);

        terminal
            .draw(|frame| {
                List::new(&state, &items, "Select inputs", "Update")
                    .render(frame.area(), frame.buffer_mut());
            })
            .unwrap();

        let output = buffer_to_plain_text(&terminal);
        insta::assert_snapshot!(output);
    }
}
