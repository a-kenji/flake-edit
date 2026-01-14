use std::io::{self, Stdout, Write};

use crossterm::{
    cursor, execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};

pub struct InlineTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    inline_mode: bool,
    height: u16,
}

impl InlineTerminal {
    pub fn new(height: u16) -> io::Result<Self> {
        let (_, term_height) = terminal::size()?;

        let inline_mode = height < term_height;

        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();

        if !inline_mode {
            execute!(stdout, EnterAlternateScreen)?;
        }

        let backend = CrosstermBackend::new(stdout);
        let viewport = if inline_mode {
            Viewport::Inline(height)
        } else {
            Viewport::Fullscreen
        };

        let terminal = Terminal::with_options(backend, TerminalOptions { viewport })?;

        Ok(Self {
            terminal,
            inline_mode,
            height,
        })
    }

    pub fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    /// Resize the terminal to a new height.
    /// This clears the current viewport and creates a new one.
    pub fn resize(&mut self, new_height: u16) -> io::Result<()> {
        if new_height == self.height {
            return Ok(());
        }

        let mut stdout = io::stdout();

        // Clear current inline area
        if self.inline_mode {
            execute!(
                stdout,
                cursor::MoveUp(self.height),
                Clear(ClearType::FromCursorDown),
            )?;
        }

        // Check if we need to switch modes
        let (_, term_height) = terminal::size()?;
        let new_inline_mode = new_height < term_height;

        // Handle mode transitions
        if self.inline_mode && !new_inline_mode {
            execute!(stdout, EnterAlternateScreen)?;
        } else if !self.inline_mode && new_inline_mode {
            execute!(stdout, LeaveAlternateScreen)?;
        }

        // Create new terminal with new viewport
        let backend = CrosstermBackend::new(stdout);
        let viewport = if new_inline_mode {
            Viewport::Inline(new_height)
        } else {
            Viewport::Fullscreen
        };

        self.terminal = Terminal::with_options(backend, TerminalOptions { viewport })?;
        self.inline_mode = new_inline_mode;
        self.height = new_height;

        Ok(())
    }
}

impl Drop for InlineTerminal {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        if self.inline_mode {
            let _ = execute!(
                stdout,
                cursor::MoveUp(self.height),
                Clear(ClearType::FromCursorDown),
            );
        } else {
            let _ = execute!(stdout, LeaveAlternateScreen);
        }
        let _ = stdout.flush();
    }
}
