//! Terminal UI: state, key handling and rendering.
//!
//! This module is the Rust counterpart of the Textual widgets in the Python
//! package (`app.py`, `widgets/conference_table.py`, `widgets/filters.py`,
//! `widgets/countdown.py`). The data layer under [`crate::data`] is shared with
//! the Python implementation and is verified against it by `tests/golden.rs`.

pub mod app;
pub mod input;
pub mod render;
pub mod theme;

use std::io;
use std::time::{Duration, Instant};

use chrono::Utc;
use ratatui::crossterm::event::{self, Event};
use ratatui::DefaultTerminal;

pub use app::{App, Focus, Language};
pub use input::Outcome;

/// How often the countdown column is recomputed. The Python app uses a
/// one-second `set_interval` on the table, so this matches it exactly.
const TICK: Duration = Duration::from_secs(1);

/// Run the TUI until the user quits.
///
/// The terminal is restored even when the loop exits early, so a panic or an
/// error never leaves the shell in raw mode.
pub fn run(service: crate::data::DataService) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, App::new(service));
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, mut app: App) -> anyhow::Result<()> {
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|frame| render::draw(frame, &mut app, Utc::now()))?;

        // A short timeout keeps the countdown ticking without busy-waiting.
        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => match input::handle_key(&mut app, key) {
                    Outcome::Quit => break,
                    Outcome::Reload => {
                        app.status = Some("Refreshing...".to_string());
                        app.reload(Utc::now());
                        app.status = None;
                    }
                    Outcome::Continue => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if last_tick.elapsed() >= TICK {
            last_tick = Instant::now();
            app.tick(Utc::now());
        }
    }
    Ok(())
}

/// Install the panic hook and alternate screen, returning the terminal.
///
/// Exposed so tests and embedders can set up the same environment.
pub fn init_terminal() -> io::Result<DefaultTerminal> {
    ratatui::try_init()
}