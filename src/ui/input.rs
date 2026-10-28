//! Key handling: maps terminal input onto the actions of the Python app.
//!
//! Split out from [`App`](super::app::App) so the bindings can be exercised
//! without constructing a terminal.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

use super::app::{App, Focus};
use super::view::{Drag, SCROLL_STEP, ViewState};

/// What the caller should do after a key was handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// Re-read the data source (`r` in the Python app).
    Reload,
    Quit,
}

/// Handle one key press. Returns [`Outcome::Quit`] for `q`/`Ctrl-C`.
///
/// `crossterm` emits both Press and Release on some platforms; only Press is
/// acted on so a single press is not handled twice.
pub fn handle_key(app: &mut App, key: KeyEvent) -> Outcome {
    if key.kind == KeyEventKind::Release {
        return Outcome::Continue;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Outcome::Quit;
    }

    // The help overlay swallows input until it is dismissed.
    if app.show_help {
        match key.code {
            KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                app.show_help = false;
            }
            _ => {}
        }
        return Outcome::Continue;
    }

    match app.focus {
        Focus::Search => search_key(app, key),
        Focus::Sidebar => sidebar_key(app, key),
        Focus::Table => table_key(app, key),
        Focus::Agent => agent_key(app, key),
        Focus::AgentCards => agent_cards_key(app, key),
    }
}

fn table_key(app: &mut App, key: KeyEvent) -> Outcome {
    match key.code {
        KeyCode::Char('q') => return Outcome::Quit,
        KeyCode::Char('r') => return Outcome::Reload,
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Tab | KeyCode::BackTab => app.focus = Focus::Sidebar,
        // `/` focuses the search field, the way it works in the Textual app.
        KeyCode::Char('/') => app.focus = Focus::Search,
        KeyCode::Char('j') | KeyCode::Down => app.select_next(),
        KeyCode::Char('k') | KeyCode::Up => app.select_previous(),
        KeyCode::Char('g') | KeyCode::Home => app.select_first(),
        KeyCode::Char('G') | KeyCode::End => app.select_last(),
        KeyCode::Char('f') => app.toggle_favorite(),
        KeyCode::Char('l') => app.toggle_language(),
        KeyCode::Char('d') => app.toggle_details(),
        KeyCode::Char('s') => app.cycle_sort_column(),
        KeyCode::Char('a') => app.open_agent(),
        KeyCode::Char('A') => app.ask_about_selected_row(),
        KeyCode::Char('v') => app.toggle_only_recommended(),
        KeyCode::Char('S') => app.toggle_sort_direction(),
        KeyCode::Enter => {
            app.open_selected_url();
        }
        KeyCode::Esc => app.reset_filters(),
        _ => {}
    }
    Outcome::Continue
}

fn sidebar_key(app: &mut App, key: KeyEvent) -> Outcome {
    match key.code {
        KeyCode::Char('q') => return Outcome::Quit,
        KeyCode::Char('r') => return Outcome::Reload,
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Tab | KeyCode::BackTab => app.focus = Focus::Table,
        KeyCode::Char('/') => app.focus = Focus::Search,
        KeyCode::Esc => {
            app.reset_filters();
            app.focus = Focus::Table;
        }
        KeyCode::Char('a') => app.open_agent(),
        // Space/Enter toggle the checkbox under the sidebar cursor.
        KeyCode::Char(' ') | KeyCode::Enter => app.toggle_sidebar_item(),
        KeyCode::Down | KeyCode::Char('j') => app.move_sidebar_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_sidebar_cursor(-1),
        _ => {}
    }
    Outcome::Continue
}

/// Text-entry mode: every printable key belongs to the query.
fn search_key(app: &mut App, key: KeyEvent) -> Outcome {
    match key.code {
        KeyCode::Esc => {
            app.search_input.clear();
            app.commit_search();
            app.focus = Focus::Table;
        }
        KeyCode::Enter => {
            app.commit_search();
            app.focus = Focus::Table;
        }
        KeyCode::Backspace => {
            app.search_input.pop();
            app.commit_search();
        }
        KeyCode::Char(c) => app.search_input.push(c),
        _ => {}
    }
    Outcome::Continue
}

/// Agent input: editing keys go to the line editor, like the search field.
/// Up/Down browse the input history (or the `/command` popup); Tab moves on
/// to the cards. Number keys run the first-screen shortcuts.
fn agent_key(app: &mut App, key: KeyEvent) -> Outcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let popup = app.agent.completion_visible();
    match key.code {
        KeyCode::Esc if popup => app.agent.dismiss_completion(),
        KeyCode::Esc => app.agent_escape(),
        KeyCode::Enter => app.submit_agent_input(),
        KeyCode::Tab if popup => app.agent.accept_completion(),
        KeyCode::Tab => app.focus_next_from_input(),
        KeyCode::BackTab => app.focus = Focus::Table,
        KeyCode::Up if popup => app.agent.move_completion(-1),
        KeyCode::Down if popup => app.agent.move_completion(1),
        KeyCode::Up => app.agent.history_prev(),
        KeyCode::Down => app.agent.history_next(),
        KeyCode::PageUp => app.scroll_transcript(AGENT_PAGE),
        KeyCode::PageDown => app.scroll_transcript(-AGENT_PAGE),
        KeyCode::Char('l') if ctrl => app.toggle_language(),
        _ if is_panel_size_key(&key) => app.cycle_panel_size(),
        KeyCode::Char(c @ '1'..='4') if !ctrl && app.quick_actions_visible() => {
            app.run_quick_action(c as usize - '1' as usize);
        }
        _ => {
            if app.agent.input.handle_key(key) {
                app.agent.input_edited();
            }
        }
    }
    Outcome::Continue
}

/// F2, or Ctrl+\ (which some terminals report as Ctrl+4).
fn is_panel_size_key(key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    matches!(key.code, KeyCode::F(2)) || (ctrl && matches!(key.code, KeyCode::Char('\\' | '4')))
}

/// Recommendation cards: the same keys as the conference table.
fn agent_cards_key(app: &mut App, key: KeyEvent) -> Outcome {
    if is_panel_size_key(&key) {
        app.cycle_panel_size();
        return Outcome::Continue;
    }
    match key.code {
        KeyCode::Char('q') => return Outcome::Quit,
        KeyCode::Char('j') | KeyCode::Down => app.move_card(1),
        KeyCode::Char('k') | KeyCode::Up => app.move_card(-1),
        KeyCode::Char('g') | KeyCode::Home => app.move_card(isize::MIN / 2),
        KeyCode::Char('G') | KeyCode::End => app.move_card(isize::MAX / 2),
        KeyCode::Enter => app.jump_to_card(),
        KeyCode::Char('f') => app.toggle_card_favorite(),
        KeyCode::Char('o') => {
            app.open_card_url();
        }
        KeyCode::Char('d') => app.toggle_card_details(),
        KeyCode::Char('?') => app.ask_about_card(),
        KeyCode::Char('v') => app.toggle_only_recommended(),
        KeyCode::Char('[') => app.show_round(-1),
        KeyCode::Char(']') => app.show_round(1),
        KeyCode::Char('l') => app.toggle_language(),
        KeyCode::Char('/') => {
            app.focus_agent_input();
            if app.agent.input.is_empty() {
                app.agent.input.insert_char('/');
                app.agent.input_edited();
            }
        }
        KeyCode::Char('i') | KeyCode::Esc | KeyCode::BackTab => app.focus_agent_input(),
        KeyCode::Tab => app.focus = Focus::Table,
        KeyCode::PageUp => app.scroll_transcript(AGENT_PAGE),
        KeyCode::PageDown => app.scroll_transcript(-AGENT_PAGE),
        _ => {}
    }
    Outcome::Continue
}

/// A bracketed paste: inserted at the cursor, never submitted.
pub fn handle_paste(app: &mut App, text: &str) -> Outcome {
    match app.focus {
        Focus::Agent => {
            app.agent.input.insert_str(text);
            app.agent.input_edited();
        }
        Focus::Search => {
            let line: String = text
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            app.search_input.push_str(&line);
        }
        Focus::Table | Focus::Sidebar | Focus::AgentCards => {}
    }
    Outcome::Continue
}

/// Transcript lines moved by PageUp / PageDown.
const AGENT_PAGE: isize = 8;

/// Route a mouse event through the hit map produced by the latest render.
/// Keyboard and mouse actions deliberately share the same `App` methods.
pub fn handle_mouse(app: &mut App, view: &mut ViewState, mouse: MouseEvent) -> Outcome {
    let point = Position {
        x: mouse.column,
        y: mouse.row,
    };

    if app.show_help {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            let close_clicked = view
                .layout
                .help_close
                .is_some_and(|area| area.contains(point));
            let outside_clicked = view.layout.help.is_some_and(|area| !area.contains(point));
            if close_clicked || outside_clicked {
                app.show_help = false;
            }
        }
        return Outcome::Continue;
    }

    match mouse.kind {
        MouseEventKind::Drag(MouseButton::Left) => {
            drag_to(app, view, point);
            return Outcome::Continue;
        }
        MouseEventKind::Up(MouseButton::Left) => {
            view.end_drag();
            return Outcome::Continue;
        }
        _ => {}
    }

    let shift = mouse.modifiers.contains(KeyModifiers::SHIFT);
    let horizontal = match mouse.kind {
        MouseEventKind::ScrollLeft => -1,
        MouseEventKind::ScrollRight => 1,
        MouseEventKind::ScrollUp if shift => -1,
        MouseEventKind::ScrollDown if shift => 1,
        _ => 0,
    };
    if horizontal != 0 {
        if view.layout.table.contains(point) {
            view.scroll_columns(horizontal);
        }
        return Outcome::Continue;
    }

    let scroll = match mouse.kind {
        MouseEventKind::ScrollUp => -SCROLL_STEP,
        MouseEventKind::ScrollDown => SCROLL_STEP,
        _ => 0,
    };
    if scroll != 0 {
        if view.layout.agent_transcript.contains(point) {
            app.scroll_transcript(-scroll);
        } else if view.layout.agent_cards_area.contains(point) {
            app.move_card(scroll.signum());
        } else if view.layout.table.contains(point) {
            view.scroll_table(app, scroll);
        } else if view.layout.sidebar.contains(point) {
            view.scroll_sidebar(scroll);
        }
        return Outcome::Continue;
    }

    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return Outcome::Continue;
    }

    if let Some(&(_, index)) = view
        .layout
        .agent_completions
        .iter()
        .find(|(area, _)| area.contains(point))
    {
        app.agent.completion_cursor = index;
        app.agent.accept_completion();
        app.focus = Focus::Agent;
        return Outcome::Continue;
    }
    if let Some(&(_, index)) = view
        .layout
        .agent_cards
        .iter()
        .find(|(area, _)| area.contains(point))
    {
        // First click selects a card, a second click shows it in the table.
        if app.focus == Focus::AgentCards && app.agent.card_cursor == index {
            app.jump_to_card();
            view.sync_table(app);
        } else {
            app.agent.card_cursor = index;
            app.focus = Focus::AgentCards;
        }
        return Outcome::Continue;
    }
    if let Some(&(_, index)) = view
        .layout
        .agent_quick
        .iter()
        .find(|(area, _)| area.contains(point))
    {
        app.focus = Focus::Agent;
        app.run_quick_action(index);
        return Outcome::Continue;
    }
    if view.layout.agent.is_some_and(|area| area.contains(point)) {
        app.focus = if view.layout.agent_cards_area.contains(point) && app.has_cards() {
            Focus::AgentCards
        } else {
            Focus::Agent
        };
        return Outcome::Continue;
    }

    if view.layout.search.contains(point) {
        app.focus = Focus::Search;
        if let Some(index) = app
            .sidebar_controls()
            .iter()
            .position(|item| matches!(item, super::app::SidebarItem::Search))
        {
            app.sidebar_cursor = index;
        }
        return Outcome::Continue;
    }

    if let Some(&(_, column, width)) = view
        .layout
        .table_handles
        .iter()
        .find(|(area, _, _)| area.contains(point))
    {
        app.focus = Focus::Table;
        if view.handle_clicked(column) {
            view.reset_column_width(column);
        } else {
            view.drag = Some(Drag::Column {
                column,
                origin_x: point.x,
                width,
            });
        }
        return Outcome::Continue;
    }
    if let Some(track) = view.layout.table_vscroll.filter(|t| t.contains(point)) {
        app.focus = Focus::Table;
        view.set_voffset_at(app, track, point.y);
        view.drag = Some(Drag::VerticalScroll);
        return Outcome::Continue;
    }
    if let Some(track) = view.layout.table_hscroll.filter(|t| t.contains(point)) {
        view.set_hoffset_at(track, point.x);
        view.drag = Some(Drag::HorizontalScroll);
        return Outcome::Continue;
    }

    for (area, column) in &view.layout.table_columns {
        if area.contains(point) {
            app.focus = Focus::Table;
            app.sort_by_column(*column);
            view.sync_table(app);
            return Outcome::Continue;
        }
    }

    for (area, item) in &view.layout.sidebar_rows {
        if !area.contains(point) || !item.is_selectable() {
            continue;
        }
        app.focus = Focus::Sidebar;
        if let Some(index) = app
            .sidebar_controls()
            .iter()
            .position(|candidate| candidate == item)
        {
            app.sidebar_cursor = index;
        }
        app.toggle_sidebar_item();
        return Outcome::Continue;
    }

    let rows = view.layout.table_rows;
    if rows.contains(point) {
        let visible_index = view.table_state.offset() + (point.y - rows.y) as usize;
        if visible_index < app.visible.len() {
            app.focus = Focus::Table;
            if app.selected == visible_index {
                app.toggle_details();
            } else {
                app.selected = visible_index;
            }
            view.sync_table(app);
        }
    }
    Outcome::Continue
}

/// Continue a drag started by a click on a handle or a scrollbar.
fn drag_to(app: &mut App, view: &mut ViewState, point: Position) {
    match view.drag {
        Some(Drag::Column {
            column,
            origin_x,
            width,
        }) => {
            let wanted = (width as i32 + point.x as i32 - origin_x as i32).max(1) as u16;
            let min = super::table_layout::min_width(column, app.language);
            view.set_column_width(column, wanted.max(min));
        }
        Some(Drag::VerticalScroll) => {
            if let Some(track) = view.layout.table_vscroll {
                view.set_voffset_at(app, track, point.y);
            }
        }
        Some(Drag::HorizontalScroll) => {
            if let Some(track) = view.layout.table_hscroll {
                view.set_hoffset_at(track, point.x);
            }
        }
        None => {}
    }
}

/// Keys that act on the table's on-screen layout rather than on `App`:
/// horizontal scrolling and resetting column widths. Returns `true` when
/// the key was used.
pub fn handle_view_key(app: &App, view: &mut ViewState, key: KeyEvent) -> bool {
    if key.kind == KeyEventKind::Release || app.show_help || app.focus != Focus::Table {
        return false;
    }
    match key.code {
        KeyCode::Left | KeyCode::Char('h') => view.scroll_columns(-1),
        KeyCode::Right => view.scroll_columns(1),
        KeyCode::Char('=') => view.reset_column_widths(),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::app::Language;
    use super::*;
    use crate::data::{DataService, Favorites};
    use crate::model::CATEGORIES;
    use ratatui::crossterm::event::KeyEventState;

    const YAML: &str = "
- title: CVPR
  sub: AI
  rank: {ccf: A, core: 'A*', thcpl: A}
  confs:
  - year: 2030
    timezone: UTC
    timeline:
    - deadline: '2030-06-01 00:00:00'
- title: ICDE
  sub: DB
  rank: {ccf: B}
  confs:
  - year: 2030
    timezone: UTC
    timeline:
    - deadline: '2030-07-01 00:00:00'
- title: OldConf
  sub: DB
  rank: {ccf: C}
  confs:
  - year: 2020
    timezone: UTC
    timeline:
    - deadline: '2020-01-01 00:00:00'
";

    fn app() -> App {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let mut service = DataService::with_paths(
            None,
            path.join("cache.yml"),
            Favorites::empty_at(path.join("favorites.json")),
        );
        service
            .load_conferences_with(|| Ok(YAML.to_string()))
            .unwrap();
        App::with_agent(
            service,
            crate::ui::agent::AgentPanel::new(path.join("profile.yaml"), None),
        )
    }

    fn press(app: &mut App, code: KeyCode) -> Outcome {
        handle_key(
            app,
            KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        )
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let mut app = app();
        assert_eq!(press(&mut app, KeyCode::Char('q')), Outcome::Quit);
        assert_eq!(
            handle_key(
                &mut app,
                KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }
            ),
            Outcome::Quit
        );
    }

    #[test]
    fn release_events_are_ignored() {
        let mut app = app();
        press(&mut app, KeyCode::Down);
        let selected = app.selected;

        // A release for the same key must not move the cursor again.
        handle_key(
            &mut app,
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Release,
                state: KeyEventState::NONE,
            },
        );
        assert_eq!(app.selected, selected);
    }

    #[test]
    fn navigation_keys_move_and_clamp() {
        let mut app = app();
        assert_eq!(app.selected, 0);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected, 1);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected, 1);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected, 0);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected, 0);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.selected, 1);
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn f_toggles_favorite_of_selected_row() {
        let mut app = app();
        press(&mut app, KeyCode::Char('f'));
        let row = app.selected_row().unwrap();
        assert!(app.service.is_favorite(&row.title, row.year));
    }

    #[test]
    fn l_toggles_language_and_r_requests_reload() {
        let mut app = app();
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.language, Language::Zh);
        assert_eq!(press(&mut app, KeyCode::Char('r')), Outcome::Reload);
    }

    #[test]
    fn help_overlay_opens_and_swallows_input() {
        let mut app = app();
        press(&mut app, KeyCode::Char('?'));
        assert!(app.show_help);

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected, 0);

        press(&mut app, KeyCode::Esc);
        assert!(!app.show_help);

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn q_while_help_open_closes_instead_of_quitting() {
        let mut app = app();
        press(&mut app, KeyCode::Char('?'));
        assert_eq!(press(&mut app, KeyCode::Char('q')), Outcome::Continue);
        assert!(!app.show_help);
    }

    #[test]
    fn tab_switches_focus_in_both_directions() {
        let mut app = app();
        assert_eq!(app.focus, Focus::Table);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Sidebar);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Table);
    }

    #[test]
    fn search_mode_captures_printable_keys() {
        let mut app = app();
        // '/' focuses the field from the table.
        press(&mut app, KeyCode::Char('/'));
        assert_eq!(app.focus, Focus::Search);

        // Keys that are global shortcuts elsewhere are literal text here.
        for c in "qr?".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.search_input, "qr?");
    }

    #[test]
    fn search_mode_builds_and_applies_a_query() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Sidebar);
        press(&mut app, KeyCode::Char('/')); // sidebar -> search
        assert_eq!(app.focus, Focus::Search);

        for c in "cvpr".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.search_input, "cvpr");
        // The query is applied on Enter.
        assert_eq!(app.visible.len(), 2);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.filters.query, "cvpr");
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.focus, Focus::Table);
    }

    #[test]
    fn backspace_edits_query_live() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        for c in "cvprx".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.search_input, "cvpr");
        assert_eq!(app.filters.query, "cvpr");
        assert_eq!(app.visible.len(), 1);
    }

    #[test]
    fn escape_clears_the_query_and_returns_to_table() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        for c in "cvpr".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.visible.len(), 1);

        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Esc);
        assert!(app.search_input.is_empty());
        assert!(app.filters.query.is_empty());
        assert_eq!(app.visible.len(), 2);
        assert_eq!(app.focus, Focus::Table);
    }

    #[test]
    fn escape_resets_filters_and_returns_to_table() {
        let mut app = app();
        app.toggle_category("AI");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Table);
        assert!(app.is_category_enabled("AI"));
    }

    #[test]
    fn sidebar_cursor_toggles_checkboxes() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);

        // Search is pinned first; move directly to the first category.
        let ds = app
            .sidebar_controls()
            .iter()
            .position(
                |item| matches!(item, super::super::app::SidebarItem::Category(sub) if sub == "DS"),
            )
            .unwrap();
        app.sidebar_cursor = ds;
        press(&mut app, KeyCode::Char(' '));
        assert!(!app.is_category_enabled("DS"));

        // Moving down visits the next category, headings are skipped.
        press(&mut app, KeyCode::Down);
        assert_eq!(app.sidebar_cursor, ds + 1);
        press(&mut app, KeyCode::Char(' '));
        assert!(!app.is_category_enabled("NW"));
    }

    #[test]
    fn sidebar_cursor_clamps_at_both_ends() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);

        press(&mut app, KeyCode::Up);
        assert_eq!(app.sidebar_cursor, 0);

        for _ in 0..(CATEGORIES.len() + 20) {
            press(&mut app, KeyCode::Down);
        }
        assert_eq!(app.sidebar_cursor, app.sidebar_controls().len() - 1);
    }

    #[test]
    fn sidebar_search_entry_switches_to_search_mode() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);
        // Search is the first, permanently visible sidebar control.
        assert_eq!(app.sidebar_cursor, 0);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(app.focus, Focus::Search);
    }

    #[test]
    fn enter_handles_the_selected_url_without_desktop_side_effects() {
        let mut app = app();
        // Invalid URLs exercise the Enter wiring without launching a browser
        // on developer machines or CI runners.
        app.rows[0].link = "file:///etc/passwd".to_string();
        app.refresh_visible();

        assert_eq!(press(&mut app, KeyCode::Enter), Outcome::Continue);
        assert!(
            app.status
                .as_deref()
                .is_some_and(|status| status.contains("Invalid URL")),
            "unexpected status: {:?}",
            app.status
        );
    }

    #[test]
    fn s_cycles_the_sort_column_and_shift_s_reverses() {
        let mut app = app();
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.sort_column, crate::ui::app::SortColumn::Title);
        press(&mut app, KeyCode::Char('S'));
        assert!(app.sort_reverse);
        press(&mut app, KeyCode::Char('s'));
        // Cycling resets the direction.
        assert!(!app.sort_reverse);
    }

    fn press_ctrl(app: &mut App, c: char) -> Outcome {
        handle_key(
            app,
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        )
    }

    #[test]
    fn a_opens_the_agent_and_typing_goes_to_its_input() {
        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.focus, Focus::Agent);
        assert!(app.agent.open);
        // Global shortcuts are literal text in the agent input.
        for c in "qr?a".chars() {
            assert_eq!(press(&mut app, KeyCode::Char(c)), Outcome::Continue);
        }
        assert_eq!(app.agent.input, "qr?a");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.agent.input, "qr?");
        // Ctrl chords are not typed.
        press_ctrl(&mut app, 'f');
        assert_eq!(app.agent.input, "qr?");

        // Esc clears the input, a second Esc closes the panel.
        press(&mut app, KeyCode::Esc);
        assert!(app.agent.input.is_empty());
        press(&mut app, KeyCode::Esc);
        assert!(!app.agent.open);
        assert_eq!(app.focus, Focus::Table);
    }

    fn press_mod(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Outcome {
        handle_key(
            app,
            KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        )
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn agent_input_supports_cursor_editing() {
        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "多模型");
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Left);
        type_text(&mut app, "态大");
        assert_eq!(app.agent.input, "多态大模型");
        press(&mut app, KeyCode::Home);
        press(&mut app, KeyCode::Delete);
        press(&mut app, KeyCode::End);
        type_text(&mut app, " for vision");
        press_ctrl(&mut app, 'w');
        assert_eq!(app.agent.input, "态大模型 for ");
        press_ctrl(&mut app, 'u');
        assert!(app.agent.input.is_empty());
    }

    #[test]
    fn up_and_down_browse_sent_inputs() {
        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "/help");
        press(&mut app, KeyCode::Esc); // close the popup
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "/profile");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "draft");
        press(&mut app, KeyCode::Up);
        assert_eq!(app.agent.input, "/profile");
        press(&mut app, KeyCode::Up);
        assert_eq!(app.agent.input, "/help");
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.agent.input, "draft");
    }

    #[test]
    fn slash_popup_completes_with_tab_and_closes_with_esc() {
        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "/ex");
        assert!(app.agent.completion_visible());
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.agent.input, "/export ");
        assert_eq!(app.focus, Focus::Agent, "Tab completed instead of leaving");

        press_ctrl(&mut app, 'u');
        type_text(&mut app, "/");
        press(&mut app, KeyCode::Down);
        assert_eq!(app.agent.completion_cursor, 1);
        // Esc only closes the popup; the input and the panel stay.
        press(&mut app, KeyCode::Esc);
        assert!(!app.agent.completion_visible());
        assert_eq!(app.agent.input, "/");
        assert!(app.agent.open);
        // Without a popup Tab goes back to the table.
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Table);
    }

    #[test]
    fn paste_inserts_without_sending() {
        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "ab");
        press(&mut app, KeyCode::Left);
        handle_paste(&mut app, "line one\nline two\n");
        assert_eq!(app.agent.input, "aline one line two b");
        assert!(
            !app.agent
                .entries
                .iter()
                .any(|e| matches!(e, crate::ui::agent::Entry::User(_)))
        );

        app.focus = Focus::Search;
        handle_paste(&mut app, "cv\npr");
        assert_eq!(app.search_input, "cv pr");
        app.focus = Focus::Table;
        assert_eq!(handle_paste(&mut app, "x"), Outcome::Continue);
    }

    /// Opens the agent and runs an offline request that yields several cards.
    fn app_with_cards() -> App {
        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        // The fixture deadlines are years away.
        app.agent.profile.horizon_days = 100_000;
        type_text(&mut app, "computer vision database");
        press(&mut app, KeyCode::Enter);
        let start = std::time::Instant::now();
        while app.agent.is_busy() {
            app.poll_agent();
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let count = app
            .agent
            .recommendations
            .as_ref()
            .map_or(0, |s| s.items.len());
        assert!(count >= 2, "{count}");
        app
    }

    #[test]
    fn tab_moves_from_input_to_cards_to_table() {
        let mut app = app_with_cards();
        assert_eq!(app.focus, Focus::Agent);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::AgentCards);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Table);
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.focus, Focus::Agent);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Focus::Table);

        // Without cards, Tab goes straight to the table.
        let mut fresh = self::app();
        press(&mut fresh, KeyCode::Char('a'));
        press(&mut fresh, KeyCode::Tab);
        assert_eq!(fresh.focus, Focus::Table);
    }

    #[test]
    fn card_keys_mirror_the_table() {
        let mut app = app_with_cards();
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.agent.card_cursor, 1);
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Up);
        assert_eq!(app.agent.card_cursor, 0);
        press(&mut app, KeyCode::Char('G'));
        let last = app.agent.recommendations.as_ref().unwrap().items.len() - 1;
        assert_eq!(app.agent.card_cursor, last);
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.agent.card_cursor, 0);

        let title = app.agent.selected_card().unwrap().facts.title.clone();
        press(&mut app, KeyCode::Char('f'));
        assert!(app.service.is_favorite(&title, 2030));
        assert!(app.agent.input.is_empty(), "f is not typed in the cards");

        // Cards start compact; d shows the details.
        assert!(!app.agent.card_details);
        press(&mut app, KeyCode::Char('d'));
        assert!(app.agent.card_details);

        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Table);
        assert_eq!(app.selected_row().unwrap().title, title);
    }

    #[test]
    fn leaving_the_cards_returns_to_the_input() {
        let mut app = app_with_cards();
        for key in [KeyCode::Char('i'), KeyCode::Esc, KeyCode::BackTab] {
            app.focus = Focus::AgentCards;
            press(&mut app, key);
            assert_eq!(app.focus, Focus::Agent, "{key:?}");
        }
        app.focus = Focus::AgentCards;
        press(&mut app, KeyCode::Char('/'));
        assert_eq!(app.focus, Focus::Agent);
        assert_eq!(app.agent.input, "/");
        assert!(app.agent.completion_visible());

        app.focus = Focus::AgentCards;
        assert_eq!(press(&mut app, KeyCode::Char('q')), Outcome::Quit);
    }

    #[test]
    fn old_ctrl_and_alt_card_chords_are_gone() {
        let mut app = app_with_cards();
        press_ctrl(&mut app, 'f');
        press_mod(&mut app, KeyCode::Down, KeyModifiers::ALT);
        assert_eq!(app.agent.card_cursor, 0);
        let title = app.agent.selected_card().unwrap().facts.title.clone();
        assert!(!app.service.is_favorite(&title, 2030));
        // Plain Up is history.
        press(&mut app, KeyCode::Up);
        assert_eq!(app.agent.input, "computer vision database");
    }

    #[test]
    fn result_keys_follow_up_rounds_and_table_questions() {
        let mut app = app_with_cards();
        press(&mut app, KeyCode::Tab);
        let title = app.agent.selected_card().unwrap().facts.title.clone();
        press(&mut app, KeyCode::Char('?'));
        assert_eq!(app.focus, Focus::Agent);
        assert_eq!(app.agent.input, format!("Why {title} 2030?").as_str());

        // A second round, then browse back.
        press_ctrl(&mut app, 'u');
        type_text(&mut app, "database");
        press(&mut app, KeyCode::Enter);
        while app.agent.is_busy() {
            app.poll_agent();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(app.agent.rounds.len(), 2);
        app.focus = Focus::AgentCards;
        press(&mut app, KeyCode::Char('['));
        assert_eq!(app.agent.round_cursor, 0);
        press(&mut app, KeyCode::Char(']'));
        assert_eq!(app.agent.round_cursor, 1);

        // `A` in the table asks about the selected row.
        app.focus = Focus::Table;
        let row = app.selected_row().unwrap().title.clone();
        press(&mut app, KeyCode::Char('A'));
        assert_eq!(app.focus, Focus::Agent);
        assert!(app.agent.input.text().contains(&row));
    }

    #[test]
    fn f2_and_ctrl_backslash_cycle_the_panel_size() {
        use crate::ui::agent_results::PanelSize;

        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::F(2));
        assert_eq!(app.agent.panel_size, PanelSize::Full);
        press_ctrl(&mut app, '\\');
        assert!(!app.agent.open);
        assert_eq!(app.focus, Focus::Table);
        press(&mut app, KeyCode::Char('a'));
        assert!(app.agent.open);
        // Some terminals report Ctrl+\ as Ctrl+4.
        press_ctrl(&mut app, '4');
        assert_eq!(app.agent.panel_size, PanelSize::Full);
        assert!(app.agent.input.is_empty(), "the chord is not typed");
    }

    #[test]
    fn number_keys_run_shortcuts_only_on_an_empty_first_screen() {
        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        assert!(app.quick_actions_visible());
        press(&mut app, KeyCode::Char('3'));
        assert!(app.agent.wizard.is_some(), "3 starts /init");

        let mut fresh = self::app();
        press(&mut fresh, KeyCode::Char('a'));
        type_text(&mut fresh, "top 3");
        assert_eq!(fresh.agent.input, "top 3");
        assert!(fresh.agent.wizard.is_none());
    }

    #[test]
    fn tab_leaves_the_agent_open_and_a_returns_to_it() {
        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Table);
        assert!(app.agent.open);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.focus, Focus::Agent);
    }

    #[test]
    fn ctrl_c_still_quits_from_the_agent() {
        let mut app = app();
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(press_ctrl(&mut app, 'c'), Outcome::Quit);
    }

    #[test]
    fn reload_is_available_from_the_sidebar_too() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);
        assert_eq!(press(&mut app, KeyCode::Char('r')), Outcome::Reload);
    }
}
