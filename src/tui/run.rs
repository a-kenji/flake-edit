//! TUI run loop for executing interactive workflows.

use std::io;

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::widgets::Widget;

use super::app::{App, AppResult, UpdateResult};
use super::backend::InlineTerminal;

/// Run the app to completion, returning the result based on workflow type.
///
/// Returns `None` if the user cancelled, or `Some(AppResult)` with the
/// appropriate result type for the workflow.
pub fn run(mut app: App) -> io::Result<Option<AppResult>> {
    let height = app.terminal_height();
    let mut term = InlineTerminal::new(height)?;

    loop {
        // Resize terminal if needed (e.g., when transitioning to Confirm screen)
        let new_height = app.terminal_height();
        if new_height != term.height() {
            term.resize(new_height)?;
        }

        term.terminal().draw(|frame| {
            let area = frame.area();
            (&app).render(area, frame.buffer_mut());
            if let Some((x, y)) = app.cursor_position(area) {
                frame.set_cursor_position((x, y));
            }
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match app.update(key) {
                UpdateResult::Continue => {}
                UpdateResult::Done => return Ok(app.extract_result()),
                UpdateResult::Cancelled => return Ok(None),
            }
        }
    }
}
