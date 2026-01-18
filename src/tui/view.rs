//! TUI rendering logic for the App.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use super::app::{App, Screen};
use super::components::confirm::Confirm;
use super::components::input::Input;
use super::components::list::List;
use super::helpers::{color_diff_lines, layouts};
use super::style::BORDER_STYLE;

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self.screen() {
            Screen::Input(screen) => {
                let mut input = Input::new(
                    &screen.state,
                    &screen.prompt,
                    self.context(),
                    screen.label.as_deref(),
                    self.show_diff(),
                );
                // Add follows indicator for Add workflow
                if let Some(follows_enabled) = self.follows_enabled() {
                    input = input.with_follows(follows_enabled);
                }

                if self.show_diff() {
                    let diff = self.pending_diff();
                    let diff_lines = diff.lines().count();
                    let (main_area, diff_area) =
                        layouts::content_with_diff_preview(area, diff_lines);
                    input.render(main_area, buf);
                    render_diff_preview(&diff, diff_area, buf);
                } else {
                    input.render(area, buf);
                }
            }
            Screen::List(screen) => {
                List::new(&screen.state, &screen.items, &screen.prompt, self.context())
                    .render(area, buf);
            }
            Screen::Confirm(screen) => {
                Confirm::new(&screen.diff, self.context()).render(area, buf);
            }
        }
    }
}

/// Render a diff preview in the given area.
fn render_diff_preview(diff: &str, area: Rect, buf: &mut Buffer) {
    let content = Paragraph::new(color_diff_lines(diff))
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(BORDER_STYLE),
        )
        .wrap(Wrap { trim: false });
    content.render(area, buf);
}
