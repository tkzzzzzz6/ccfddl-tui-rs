//! Renders one full frame with the embedded snapshot and prints it, so the
//! layout can be eyeballed without attaching to a terminal.
//!
//! Run with: `cargo run --example snapshot` (add `-- --help` for the overlay).

use chrono::Utc;
use ccfddl_tui::data::{DataError, DataService, Favorites};
use ccfddl_tui::ui;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn main() {
    let dir = std::env::temp_dir().join("ccfddl-snapshot");
    std::fs::create_dir_all(&dir).unwrap();

    let mut service = DataService::with_paths(
        None,
        dir.join("cache.yml"),
        Favorites::empty_at(dir.join("favorites.json")),
    );
    // Offline: fall back to the embedded snapshot.
    service
        .load_conferences_with(|| Err(DataError::Remote("offline".into())))
        .expect("embedded snapshot must parse");

    let mut app = ui::App::new(service);
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|a| a == "--help") {
            app.show_help = true;
        }
        if args.iter().any(|a| a == "--sidebar-bottom") {
            // Scroll the sidebar to its last entries (search box).
            app.sidebar_cursor = app.sidebar_controls().len().saturating_sub(1);
        }
        if args.iter().any(|a| a == "--zh") {
            app.toggle_language();
        }
        if args.iter().any(|a| a == "--focus-sidebar") {
            app.focus = ccfddl_tui::ui::Focus::Sidebar;
        }

    let (width, height) = (120u16, 32u16);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| ui::render::draw(frame, &mut app, Utc::now()))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    for y in 0..buffer.area.height {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        println!("{line}");
    }
}