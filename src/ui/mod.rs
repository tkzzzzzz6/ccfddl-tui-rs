//! Terminal UI: state, key handling and rendering.
//!
//! This module is the Rust counterpart of the Textual widgets in the Python
//! package (`app.py`, `widgets/conference_table.py`, `widgets/filters.py`,
//! `widgets/countdown.py`). The data layer under [`crate::data`] is shared with
//! the Python implementation and is verified against it by `tests/golden.rs`.

pub mod agent;
pub mod agent_view;
pub mod app;
pub mod browser;
pub mod input;
pub mod refresh;
pub mod render;
pub mod theme;
pub mod view;

use std::io;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use chrono::Utc;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;

pub use app::{App, Focus, Language};
pub use input::Outcome;
pub use view::ViewState;

/// How often the countdown column is recomputed. The Python app uses a
/// one-second `set_interval` on the table, so this matches it exactly.
const TICK: Duration = Duration::from_secs(1);

/// Runs the TUI with mouse capture enabled. The guard disables capture on
/// normal return, errors and unwinding; `ratatui::restore` then restores the
/// screen and input mode.
struct MouseCaptureGuard;

impl MouseCaptureGuard {
    fn enable() -> io::Result<Self> {
        execute!(io::stdout(), EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for MouseCaptureGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture);
    }
}

/// Run the TUI until the user quits.
///
/// The terminal is restored even when the loop exits early, so a panic or an
/// error never leaves the shell in raw mode.
/// The agent panel reads its research profile from `profile_path` and uses
/// a model when `CCFDDL_API_KEY` (or a local `CCFDDL_BASE_URL`) is set.
pub fn run(
    service: crate::data::DataService,
    profile_path: std::path::PathBuf,
) -> anyhow::Result<()> {
    let agent = agent::AgentPanel::new(profile_path, crate::agent::llm::LlmConfig::from_env());
    let mut terminal = ratatui::init();
    let result = match MouseCaptureGuard::enable() {
        Ok(_mouse) => event_loop(&mut terminal, App::with_agent(service, agent)),
        Err(error) => Err(error.into()),
    };
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, mut app: App) -> anyhow::Result<()> {
    let mut last_tick = Instant::now();
    let mut view = ViewState::new();
    let mut refresh_receiver = None;
    start_refresh(&mut app, &mut refresh_receiver);

    loop {
        poll_refresh(&mut app, &mut refresh_receiver);
        app.poll_agent();
        terminal.draw(|frame| render::draw_with_view(frame, &mut app, &mut view, Utc::now()))?;

        // A short timeout keeps the countdown ticking without busy-waiting;
        // refresh completion is polled more frequently while a worker is active.
        let tick_timeout = TICK.saturating_sub(last_tick.elapsed());
        let timeout = if app.agent.is_running() {
            // Streamed agent text should appear promptly.
            tick_timeout.min(Duration::from_millis(40))
        } else if app.is_refreshing {
            tick_timeout.min(Duration::from_millis(100))
        } else {
            tick_timeout
        };
        if event::poll(timeout)? {
            let outcome = match event::read()? {
                Event::Key(key) => input::handle_key(&mut app, key),
                Event::Mouse(mouse) => input::handle_mouse(&mut app, &mut view, mouse),
                Event::Resize(_, _) => {
                    view.layout = Default::default();
                    Outcome::Continue
                }
                _ => Outcome::Continue,
            };
            match outcome {
                Outcome::Quit => break,
                Outcome::Reload => start_refresh(&mut app, &mut refresh_receiver),
                Outcome::Continue => {}
            }
        }

        if last_tick.elapsed() >= TICK {
            last_tick = Instant::now();
            app.tick(Utc::now());
        }
    }
    Ok(())
}

fn start_refresh(app: &mut App, receiver: &mut Option<Receiver<refresh::RefreshMessage>>) {
    if app.is_refreshing {
        return;
    }
    app.is_refreshing = true;
    app.status = None;
    *receiver = Some(refresh::spawn(&app.service));
}

fn poll_refresh(app: &mut App, receiver: &mut Option<Receiver<refresh::RefreshMessage>>) {
    let mut clear = false;
    if let Some(current) = receiver.as_ref() {
        match current.try_recv() {
            Ok(message) => {
                app.finish_refresh(message);
                clear = true;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                app.is_refreshing = false;
                app.status = Some("Refresh worker stopped unexpectedly".to_string());
                clear = true;
            }
        }
    }
    if clear {
        *receiver = None;
    }
}

/// Install the panic hook and alternate screen, returning the terminal.
///
/// Exposed so tests and embedders can set up the same environment.
pub fn init_terminal() -> io::Result<DefaultTerminal> {
    ratatui::try_init()
}
