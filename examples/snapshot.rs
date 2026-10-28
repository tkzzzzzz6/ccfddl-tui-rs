//! Renders one full frame with the embedded snapshot and prints it, so the
//! layout can be eyeballed without attaching to a terminal.
//!
//! Run with: `cargo run --example snapshot` (add `-- --help` for the overlay).

use ccfddl_tui::data::{DataError, DataService, Favorites};
use ccfddl_tui::ui;
use chrono::Utc;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

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

    let args: Vec<String> = std::env::args().skip(1).collect();
    // The agent panel reads (and /init writes) its profile in the temp dir.
    let mut app = ui::App::with_agent(
        service,
        ui::agent::AgentPanel::new(dir.join("profile.yaml"), None),
    );
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
    if args.iter().any(|a| a == "--details") {
        app.show_details = true;
    }
    if args.iter().any(|a| a == "--collapse-categories") {
        app.show_categories = false;
    }
    if let Some(index) = args.iter().position(|arg| arg == "--search")
        && let Some(query) = args.get(index + 1)
    {
        app.search_input = query.clone();
        app.commit_search();
    }
    if args.iter().any(|a| a == "--sort-title") {
        app.cycle_sort_column();
        app.status = None;
    }
    if args.iter().any(|a| a == "--sort-title-desc") {
        app.cycle_sort_column();
        app.toggle_sort_direction();
        app.status = None;
    }

    // `--agent "question"` opens the panel and runs one offline turn.
    if let Some(index) = args.iter().position(|arg| arg == "--agent") {
        app.open_agent();
        if let Some(question) = args.get(index + 1).filter(|q| !q.starts_with("--")) {
            app.agent.input.set(question);
            app.submit_agent_input();
            while app.agent.is_running() {
                app.poll_agent();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
    // `--type "/ex"` leaves text in the input (shows the command popup).
    if let Some(index) = args.iter().position(|arg| arg == "--type")
        && let Some(text) = args.get(index + 1)
    {
        app.open_agent();
        app.agent.input.set(text);
        app.agent.input_edited();
    }
    if args.iter().any(|a| a == "--card-2") {
        app.move_card(1);
    }

    let (width, height) = if args.iter().any(|a| a == "--wide") {
        (160u16, 44u16)
    } else {
        (120u16, 32u16)
    };
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
