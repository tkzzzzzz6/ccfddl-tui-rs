//! End-to-end UI interaction tests using ratatui's TestBackend.

use ccfddl_tui::data::{DataService, Favorites};
use ccfddl_tui::ui::app::{App, Focus, SidebarItem, SortColumn};
use ccfddl_tui::ui::input::{handle_mouse, handle_view_key};
use ccfddl_tui::ui::render::{TableColumn, columns_for_width, draw_with_view};
use ccfddl_tui::ui::table_layout::{HSCROLL_STEP, load_widths};
use ccfddl_tui::ui::view::{LayoutMap, ViewState};
use chrono::Utc;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

const YAML: &str = "
- title: CVPR
  sub: AI
  rank: {ccf: A, core: 'A*', thcpl: A}
  confs:
  - year: 2030
    link: https://example.com/cvpr
    timezone: UTC
    date: June 10-15, 2030
    place: Paris, France
    timeline:
    - deadline: '2030-06-01 00:00:00'
      abstract_deadline: '2030-05-20 00:00:00'
      comment: Main track
- title: ICDE
  sub: DB
  rank: {ccf: B}
  confs:
  - year: 2030
    link: https://example.com/icde
    timezone: UTC
    date: July 1-4, 2030
    place: Rome, Italy
    timeline:
    - deadline: '2030-07-01 00:00:00'
- title: OldConf
  sub: DB
  rank: {ccf: C}
  confs:
  - year: 2020
    link: https://example.com/old
    timezone: UTC
    timeline:
    - deadline: '2020-01-01 00:00:00'
";

fn app_with_yaml(yaml: String) -> App {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);
    let mut service = DataService::with_paths(
        None,
        path.join("cache.yml"),
        Favorites::empty_at(path.join("favorites.json")),
    );
    service.load_conferences_with(|| Ok(yaml)).unwrap();
    App::new(service)
}

fn app() -> App {
    app_with_yaml(YAML.to_string())
}

fn render(app: &mut App, view: &mut ViewState, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| draw_with_view(frame, app, view, Utc::now()))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn click(x: u16, y: u16) -> MouseEvent {
    mouse(MouseEventKind::Down(MouseButton::Left), x, y)
}

fn click_rect(area: Rect) -> MouseEvent {
    click(area.x + area.width / 2, area.y + area.height / 2)
}

fn sidebar_hit(view: &ViewState, target: &SidebarItem) -> MouseEvent {
    let area = view
        .layout
        .sidebar_rows
        .iter()
        .find(|(_, item)| item == target)
        .map(|(area, _)| *area)
        .expect("sidebar control is rendered");
    click_rect(area)
}

fn header_hit(view: &ViewState, column: SortColumn) -> MouseEvent {
    let area = view
        .layout
        .table_columns
        .iter()
        .find(|(_, candidate)| *candidate == column)
        .map(|(area, _)| *area)
        .expect("sortable header is rendered");
    click_rect(area)
}

#[test]
fn clicking_a_category_checkbox_toggles_the_filter() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);

    let event = sidebar_hit(&view, &SidebarItem::Category("AI".to_string()));
    handle_mouse(&mut app, &mut view, event);

    assert!(!app.is_category_enabled("AI"));
    assert!(!app.visible.iter().any(|row| row.title == "CVPR"));
    assert_eq!(app.focus, Focus::Sidebar);
}

#[test]
fn clicking_show_expired_reveals_old_rows() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);

    let event = sidebar_hit(&view, &SidebarItem::ShowExpired);
    handle_mouse(&mut app, &mut view, event);

    assert!(app.filters.show_expired);
    assert!(app.visible.iter().any(|row| row.title == "OldConf"));
}

#[test]
fn clicking_search_focuses_the_input() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);

    let event = click_rect(view.layout.search);
    handle_mouse(&mut app, &mut view, event);
    assert_eq!(app.focus, Focus::Search);
}

#[test]
fn clicking_a_header_sorts_and_preserves_the_selected_conference() {
    let mut app = app();
    app.select_last();
    let selected = app.selected_row().unwrap().title.clone();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 160, 32);

    let event = header_hit(&view, SortColumn::Title);
    handle_mouse(&mut app, &mut view, event);
    assert_eq!(app.sort_column, SortColumn::Title);
    assert!(!app.sort_reverse);
    assert_eq!(app.selected_row().unwrap().title, selected);

    render(&mut app, &mut view, 160, 32);
    let event = header_hit(&view, SortColumn::Title);
    handle_mouse(&mut app, &mut view, event);
    assert!(app.sort_reverse);
    assert_eq!(app.selected_row().unwrap().title, selected);
}

#[test]
fn row_click_selects_and_second_click_toggles_details() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 140, 32);

    let second_row = click(view.layout.table_rows.x + 2, view.layout.table_rows.y + 1);
    handle_mouse(&mut app, &mut view, second_row);
    assert_eq!(app.selected, 1);
    assert!(!app.show_details);

    render(&mut app, &mut view, 140, 32);
    let same_row = click(view.layout.table_rows.x + 2, view.layout.table_rows.y + 1);
    handle_mouse(&mut app, &mut view, same_row);
    assert!(app.show_details);

    let lines = render(&mut app, &mut view, 140, 32);
    assert!(view.layout.details.is_some());
    assert!(
        lines.iter().any(|line| line.contains("ICDE 2030")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("https://example.com/icde")),
        "{lines:?}"
    );
}

#[test]
fn details_show_deadlines_abstracts_and_comments() {
    let mut app = app();
    app.show_details = true;
    let mut view = ViewState::new();
    let lines = render(&mut app, &mut view, 160, 36);
    let text = lines.join("\n");

    assert!(text.contains("2030-06-01 00:00:00"), "{text}");
    assert!(text.contains("2030-05-20 00:00:00"), "{text}");
    assert!(text.contains("Main track"), "{text}");
    assert!(text.contains("Paris, France"), "{text}");
}

#[test]
fn table_wheel_moves_selection_without_changing_focus() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);

    let event = mouse(
        MouseEventKind::ScrollDown,
        view.layout.table_rows.x + 2,
        view.layout.table_rows.y,
    );
    handle_mouse(&mut app, &mut view, event);
    assert_eq!(app.selected, 1);
    assert_eq!(app.focus, Focus::Table);

    let event = mouse(
        MouseEventKind::ScrollUp,
        view.layout.table_rows.x + 2,
        view.layout.table_rows.y,
    );
    handle_mouse(&mut app, &mut view, event);
    assert_eq!(app.selected, 0);
}

#[test]
fn scrolled_table_click_uses_the_rendered_offset() {
    let yaml = (0..60)
        .map(|index| {
            format!(
                "- title: Conf{index:02}\n  sub: AI\n  rank: {{ccf: A}}\n  confs:\n  - year: 2030\n    timezone: UTC\n    timeline:\n    - deadline: '2030-06-01 00:00:00'\n"
            )
        })
        .collect::<String>();
    let mut app = app_with_yaml(yaml);
    let mut view = ViewState::new();
    render(&mut app, &mut view, 100, 12);

    view.scroll_table(&mut app, 20);
    render(&mut app, &mut view, 100, 12);
    assert!(view.table_state.offset() > 0);

    let event = click(view.layout.table_rows.x + 2, view.layout.table_rows.y);
    handle_mouse(&mut app, &mut view, event);
    assert_eq!(app.selected, view.table_state.offset());
    assert!(app.selected > 0);
}

#[test]
fn sidebar_wheel_scrolls_independently() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 100, 14);
    assert!(view.sidebar_visual_rows > view.sidebar_page);

    let selected = app.selected;
    let event = mouse(
        MouseEventKind::ScrollDown,
        view.layout.sidebar.x + 2,
        view.layout.sidebar.y + 4,
    );
    handle_mouse(&mut app, &mut view, event);

    assert!(view.sidebar_offset > 0);
    assert_eq!(app.selected, selected);
}

#[test]
fn categories_can_be_collapsed_with_the_mouse() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);

    let event = sidebar_hit(&view, &SidebarItem::CategoriesToggle);
    handle_mouse(&mut app, &mut view, event);
    assert!(!app.show_categories);

    render(&mut app, &mut view, 120, 32);
    assert!(
        !view
            .layout
            .sidebar_rows
            .iter()
            .any(|(_, item)| matches!(item, SidebarItem::Category(_)))
    );
}

#[test]
fn help_blocks_background_clicks_and_close_works() {
    let mut app = app();
    app.show_help = true;
    let mut view = ViewState::new();
    render(&mut app, &mut view, 140, 40);

    let selected = app.selected;
    // Choose a point that is both over the table and inside the modal. It must
    // not fall through to the row underneath.
    let help = view.layout.help.unwrap();
    let background = click(help.x + 5, help.y + 5);
    handle_mouse(&mut app, &mut view, background);
    assert_eq!(app.selected, selected);
    assert!(app.show_help);

    let close = click_rect(view.layout.help_close.unwrap());
    handle_mouse(&mut app, &mut view, close);
    assert!(!app.show_help);
}

#[test]
fn clicking_outside_help_also_closes_it() {
    let mut app = app();
    app.show_help = true;
    let mut view = ViewState::new();
    render(&mut app, &mut view, 140, 40);

    handle_mouse(&mut app, &mut view, click(0, 0));
    assert!(!app.show_help);
}

#[test]
fn responsive_column_sets_and_hit_maps_follow_width() {
    assert_eq!(columns_for_width(160).len(), 9);
    assert_eq!(columns_for_width(120).len(), 6);
    assert_eq!(columns_for_width(90).len(), 4);

    let mut app = app();
    let mut view = ViewState::new();
    for (width, expected_hit_regions) in [(80, 3), (100, 5), (120, 5), (160, 6)] {
        render(&mut app, &mut view, width, 32);
        assert_eq!(view.layout.table_columns.len(), expected_hit_regions);
        assert!(
            view.layout
                .table_columns
                .iter()
                .all(|(area, _)| area.right() <= width)
        );
    }
}

#[test]
fn title_column_is_capped_on_wide_layouts() {
    let mut app = app();
    let mut view = ViewState::new();

    for width in [120, 160] {
        render(&mut app, &mut view, width, 32);
        let title_area = view
            .layout
            .table_columns
            .iter()
            .find(|(_, column)| *column == SortColumn::Title)
            .map(|(area, _)| *area)
            .unwrap();
        assert!(
            title_area.width <= 36,
            "title width at {width}: {title_area:?}"
        );
    }
}

#[test]
fn empty_results_show_an_explicit_reset_hint() {
    let mut app = app();
    app.search_input = "no-such-conference".to_string();
    app.commit_search();
    let mut view = ViewState::new();
    let lines = render(&mut app, &mut view, 120, 32);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("No conferences match")),
        "{lines:?}"
    );
}

#[test]
fn right_clicks_and_motion_do_not_change_selection() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);
    let selected = app.selected;
    let rows = view.layout.table_rows;

    for kind in [
        MouseEventKind::Down(MouseButton::Right),
        MouseEventKind::Moved,
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_mouse(&mut app, &mut view, mouse(kind, rows.x + 1, rows.y + 1));
    }
    assert_eq!(app.selected, selected);
}

#[test]
fn resize_replaces_rather_than_reuses_stale_hit_regions() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 160, 32);
    let old = std::mem::take(&mut view.layout.table_columns);

    view.layout = LayoutMap::default();
    assert!(view.layout.table_rows.is_empty());
    render(&mut app, &mut view, 90, 32);
    assert_ne!(view.layout.table_columns.len(), old.len());
}

fn many_rows(count: usize) -> App {
    let mut yaml = String::new();
    for i in 0..count {
        yaml.push_str(&format!(
            "
- title: CONF{i:03}
  sub: AI
  rank: {{ccf: A}}
  confs:
  - year: 2030
    timezone: UTC
    date: June 10-15, 2030
    place: Paris, France
    timeline:
    - deadline: '2030-06-01 00:00:00'
"
        ));
    }
    app_with_yaml(yaml)
}

fn handle(view: &ViewState, column: TableColumn) -> (Rect, u16) {
    view.layout
        .table_handles
        .iter()
        .find(|(_, candidate, _)| *candidate == column)
        .map(|(area, _, width)| (*area, *width))
        .expect("resize handle is rendered")
}

fn drag_handle(app: &mut App, view: &mut ViewState, column: TableColumn, by: i32) {
    let (area, _) = handle(view, column);
    handle_mouse(app, view, click(area.x, area.y));
    let to = (area.x as i32 + by) as u16;
    handle_mouse(
        app,
        view,
        mouse(MouseEventKind::Drag(MouseButton::Left), to, area.y),
    );
    handle_mouse(
        app,
        view,
        mouse(MouseEventKind::Up(MouseButton::Left), to, area.y),
    );
}

#[test]
fn header_shows_resize_handles_between_columns() {
    let mut app = app();
    let mut view = ViewState::new();
    let lines = render(&mut app, &mut view, 120, 32);
    let (area, width) = handle(&view, TableColumn::Title);
    assert!(width > 0);
    let header = &lines[area.y as usize];
    assert_eq!(header.chars().nth(area.x as usize), Some('│'), "{header}");
    // The handle sits in the gap right after the title's sort target.
    let title = view
        .layout
        .table_columns
        .iter()
        .find(|(_, c)| *c == SortColumn::Title)
        .unwrap()
        .0;
    assert_eq!(title.right(), area.x);
    assert!(
        !view
            .layout
            .table_handles
            .iter()
            .any(|(_, c, _)| *c == TableColumn::Favorite)
    );
}

#[test]
fn dragging_a_handle_resizes_and_saves_the_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout.yaml");
    let mut app = app();
    let mut view = ViewState::new().with_layout_file(path.clone());
    render(&mut app, &mut view, 120, 32);
    let (_, before) = handle(&view, TableColumn::Title);

    drag_handle(&mut app, &mut view, TableColumn::Title, -6);
    assert_eq!(
        view.column_widths.get(&TableColumn::Title),
        Some(&(before - 6))
    );
    assert!(view.drag.is_none());
    render(&mut app, &mut view, 120, 32);
    assert_eq!(handle(&view, TableColumn::Title).1, before - 6);
    assert_eq!(
        load_widths(&path).get(&TableColumn::Title),
        Some(&(before - 6))
    );

    // Saved widths come back in a new view.
    let restored = ViewState::new().with_layout_file(path);
    assert_eq!(restored.column_widths, view.column_widths);
}

#[test]
fn a_column_cannot_be_dragged_below_its_header() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);
    drag_handle(&mut app, &mut view, TableColumn::Countdown, -50);
    assert_eq!(
        view.column_widths.get(&TableColumn::Countdown),
        Some(&"Countdown".len().try_into().unwrap())
    );
}

#[test]
fn double_clicking_a_handle_restores_the_default_width() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);
    let (_, default) = handle(&view, TableColumn::Title);
    drag_handle(&mut app, &mut view, TableColumn::Title, -8);
    render(&mut app, &mut view, 120, 32);

    let (area, _) = handle(&view, TableColumn::Title);
    handle_mouse(&mut app, &mut view, click(area.x, area.y));
    handle_mouse(&mut app, &mut view, click(area.x, area.y));
    assert!(view.column_widths.is_empty());
    render(&mut app, &mut view, 120, 32);
    assert_eq!(handle(&view, TableColumn::Title).1, default);
}

#[test]
fn equals_key_resets_all_widths_and_arrows_scroll_sideways() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);
    drag_handle(&mut app, &mut view, TableColumn::Title, 60);
    render(&mut app, &mut view, 120, 32);
    assert!(view.table_hmax > 0);

    assert!(handle_view_key(&app, &mut view, key(KeyCode::Right)));
    assert_eq!(view.table_hoffset, HSCROLL_STEP.min(view.table_hmax));
    assert!(handle_view_key(&app, &mut view, key(KeyCode::Left)));
    assert_eq!(view.table_hoffset, 0);
    // Other keys and other focus areas are left to `handle_key`.
    assert!(!handle_view_key(&app, &mut view, key(KeyCode::Char('j'))));
    app.focus = Focus::Sidebar;
    assert!(!handle_view_key(&app, &mut view, key(KeyCode::Right)));
    app.focus = Focus::Table;

    assert!(handle_view_key(&app, &mut view, key(KeyCode::Char('='))));
    assert!(view.column_widths.is_empty());
    render(&mut app, &mut view, 120, 32);
    assert_eq!(view.table_hmax, 0);
}

#[test]
fn clicking_a_header_still_sorts_after_resizing() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);
    drag_handle(&mut app, &mut view, TableColumn::Title, -4);
    render(&mut app, &mut view, 120, 32);
    let event = header_hit(&view, SortColumn::Countdown);
    handle_mouse(&mut app, &mut view, event);
    assert_eq!(app.sort_column, SortColumn::Countdown);
}

#[test]
fn wide_columns_scroll_horizontally_with_the_title_frozen() {
    let mut app = app();
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);
    drag_handle(&mut app, &mut view, TableColumn::Date, 40);
    let lines = render(&mut app, &mut view, 120, 32);
    let track = view.layout.table_hscroll.expect("horizontal scrollbar");
    assert_eq!(track.y, view.layout.table.bottom() - 1);
    assert!(lines.iter().any(|l| l.contains("CVPR 2030")));
    assert!(lines.iter().any(|l| l.contains("Sub")));

    // Shift + wheel scrolls sideways, the plain wheel still moves rows.
    let rows = view.layout.table_rows;
    let mut wheel = mouse(MouseEventKind::ScrollDown, rows.x + 3, rows.y);
    wheel.modifiers = KeyModifiers::SHIFT;
    handle_mouse(&mut app, &mut view, wheel);
    assert_eq!(view.table_hoffset, HSCROLL_STEP);
    let lines = render(&mut app, &mut view, 120, 32);
    assert!(lines.iter().any(|l| l.contains("CVPR 2030")), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("Sub")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("June 10-15, 2030")));

    // Dragging the scrollbar to its end shows the last column.
    handle_mouse(&mut app, &mut view, click(track.x, track.y));
    assert_eq!(view.table_hoffset, 0);
    handle_mouse(
        &mut app,
        &mut view,
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            track.right() - 1,
            track.y,
        ),
    );
    assert_eq!(view.table_hoffset, view.table_hmax);
    handle_mouse(
        &mut app,
        &mut view,
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            track.right(),
            track.y,
        ),
    );
    let lines = render(&mut app, &mut view, 120, 32);
    assert!(lines.iter().any(|l| l.contains("CVPR 2030")), "{lines:#?}");
}

#[test]
fn vertical_scrollbar_can_be_clicked_and_dragged() {
    let mut app = many_rows(80);
    let mut view = ViewState::new();
    render(&mut app, &mut view, 120, 32);
    let track = view.layout.table_vscroll.expect("vertical scrollbar");
    let page = view.layout.table_rows.height as usize;

    handle_mouse(&mut app, &mut view, click(track.x, track.bottom() - 1));
    assert_eq!(view.table_state.offset(), 80 - page);
    assert!(app.selected >= 80 - page);
    assert_eq!(app.focus, Focus::Table);

    handle_mouse(
        &mut app,
        &mut view,
        mouse(MouseEventKind::Drag(MouseButton::Left), track.x, track.y),
    );
    assert_eq!(view.table_state.offset(), 0);
    assert!(app.selected < page);
    handle_mouse(
        &mut app,
        &mut view,
        mouse(MouseEventKind::Up(MouseButton::Left), track.x, track.y),
    );
    assert!(view.drag.is_none());
    let lines = render(&mut app, &mut view, 120, 32);
    assert!(lines.iter().any(|l| l.contains("CONF000")));
}
