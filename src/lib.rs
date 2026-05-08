pub mod app;
pub mod config;
pub mod preview;
pub mod system;
pub mod ui;

use std::{io, time::Duration};

use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::App;

/// Restores terminal state on drop — even if the app panics.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        // Do NOT enable mouse capture — we never use mouse events and it breaks
        // the user's ability to right-click / select text in the terminal.
        let _ = execute!(
            stdout,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

pub fn run() -> Result<()> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        // Mouse capture intentionally omitted — we have no mouse handlers
        // and enabling it disables native terminal text selection / right-click.
        crossterm::cursor::Hide
    )?;

    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new()?;

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if app.should_quit {
            break;
        }

        // Tick the debounce timer on every iteration so image/text jobs are
        // dispatched ~80 ms after the user stops moving the cursor.
        app.tick_debounce();

        // Poll with a short timeout so the debounce tick fires frequently
        // enough to feel responsive without burning the CPU.
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    app.handle_key(key);
                }
            }
        }
    }

    terminal.show_cursor()?;
    Ok(())
}
