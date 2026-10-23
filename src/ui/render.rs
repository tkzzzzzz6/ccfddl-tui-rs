//! Drawing: header, filter sidebar, conference table, footer and help screen.
//!
//! The layout mirrors the Textual app — a fixed-width sidebar on the left and a
//! DataTable filling the rest, with the header above and the footer below.

use chrono::{DateTime, Utc};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::data::ConferenceRow;
use crate::format::format_deadline_countdown;
use crate::model::CATEGORIES;

use super::app::{App, Focus, Language, SidebarItem, SIDEBAR_WIDTH};
use super::theme;

/// Column headers, in the order of `ConferenceTable.add_columns`.
pub const COLUMNS: [&str; 9] = [
    "★", "Title", "Sub", "CCF", "CORE", "THCPL", "Countdown", "Date", "Place",
];

/// Draw the whole UI for the current frame.
pub fn draw(frame: &mut Frame, app: &mut App, now: DateTime<Utc>) {
    let area = frame.area();
    if area.width < 40 || area.height < 8 {
        render_too_small(frame, area);
        return;
    }

    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);

    render_header(frame, app, header);
    render_body(frame, app, body, now);
    render_footer(frame, app, footer);

    if app.show_help {
        render_help(frame, area);
    }
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new("Terminal too small\nResize to at least 40x8")
            .style(theme::DIM)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let subtitle = app.subtitle();
    let title = app.language.title();

    let mut spans = vec![
        Span::styled(format!(" {title} "), theme::HEADER_STYLE.add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {subtitle}"), theme::HEADER_STYLE),
    ];
    if let Some(status) = &app.status {
        spans.push(Span::styled(format!("  |  {status}"), theme::HEADER_STYLE));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme::HEADER_STYLE), area);
}

fn render_body(frame: &mut Frame, app: &mut App, area: Rect, now: DateTime<Utc>) {
    let [sidebar, table] =
        Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(20)]).areas(area);
    render_sidebar(frame, app, sidebar);
    render_table(frame, app, table, now);
}

/// Sidebar entries as display strings with their checkbox state.
fn sidebar_line(app: &App, item: &SidebarItem) -> Line<'static> {
    let checkbox = |on: bool| if on { "[x] " } else { "[ ] " };

    match item {
        SidebarItem::Header(title) => Line::from(Span::styled(*title, theme::SECTION_STYLE)),
        SidebarItem::Category(sub) => {
            let name = CATEGORIES
                .iter()
                .find(|c| c.sub == sub)
                .map_or("", |c| c.name_en);
            Line::from(vec![
                Span::styled(checkbox(app.is_category_enabled(sub)), theme::TEXT),
                Span::styled(format!("{sub} "), Style::new().fg(theme::sub_color(sub))),
                Span::raw(name.to_string()),
            ])
        }
        SidebarItem::CcfRank(rank) => Line::from(vec![
            Span::styled(checkbox(app.is_ccf_rank_enabled(rank)), theme::TEXT),
            Span::styled(format!("CCF {rank}"), Style::new().fg(theme::ccf_rank_color(rank))),
        ]),
        SidebarItem::CoreRank(rank) => Line::from(vec![
            Span::styled(checkbox(app.is_core_rank_enabled(rank)), theme::TEXT),
            Span::styled(
                format!("CORE {rank}"),
                Style::new().fg(theme::core_rank_color(rank)),
            ),
        ]),
        SidebarItem::ThcplRank(rank) => Line::from(vec![
            Span::styled(checkbox(app.is_thcpl_rank_enabled(rank)), theme::TEXT),
            Span::styled(
                format!("THCPL {rank}"),
                Style::new().fg(theme::thcpl_rank_color(rank)),
            ),
        ]),
        SidebarItem::ShowExpired => Line::from(vec![
            Span::styled(checkbox(app.filters.show_expired), theme::TEXT),
            Span::raw("Show Expired"),
        ]),
        SidebarItem::Search => {
                    // The caret only shows while the field has focus.
                    let caret = if app.focus == Focus::Search { "_" } else { "" };
                    let line = Line::from(vec![
                        Span::styled("> ", theme::DIM),
                        Span::styled(app.search_input.clone(), theme::SEARCH_STYLE),
                        Span::styled(caret.to_string(), theme::DIM),
                    ]);
                    if app.focus == Focus::Search {
                        line.style(theme::SELECTED_STYLE)
                    } else {
                        line
                    }
                }
    }
}

fn render_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let title = match app.language {
        Language::En => "Filters",
        Language::Zh => "筛选",
    };
    let border = if app.focus == Focus::Sidebar {
        Style::new().fg(theme::PRIMARY)
    } else {
        theme::DIM
    };

    let items = app.sidebar_items();
    let inner_height = area.height.saturating_sub(2) as usize;
    // Keep the highlighted control visible when the sidebar overflows.
        let controls = app.sidebar_controls();
        let cursor_item = controls.get(app.sidebar_cursor);
        let cursor_index = items
            .iter()
            .position(|item| Some(item) == cursor_item)
            .unwrap_or(0);
        let offset = cursor_index.saturating_sub(inner_height.saturating_sub(1));

        let focused = app.focus == Focus::Sidebar;
        let lines: Vec<Line> = items
            .iter()
            .enumerate()
            .skip(offset)
            .take(inner_height.max(1))
            .map(|(index, item)| {
                let line = sidebar_line(app, item);
                if focused && index == cursor_index {
                    line.style(theme::SELECTED_STYLE)
                } else {
                    line
                }
            })
            .collect();

    frame.render_widget(
        Paragraph::new(lines)
            .style(theme::SIDEBAR_STYLE)
            .block(Block::default().borders(Borders::ALL).title(Span::styled(
                format!(" {title} "),
                theme::TITLE_STYLE,
            ))
            .border_style(border)),
        area,
    );
}

/// Format one row into table cells.
fn row_cells(row: &ConferenceRow, now: DateTime<Utc>) -> Vec<Cell<'static>> {
    let favorite = if row.is_favorite {
        Span::styled("★", Style::new().fg(theme::WARNING))
    } else {
        Span::styled("☆", theme::DIM)
    };

    // Running rows are highlighted, finished ones dimmed (`_format_title`).
    let title_style = if row.is_running && !row.is_tbd {
        theme::BOLD
    } else {
        theme::DIM
    };

    let countdown = format_deadline_countdown(row.deadline, row.is_tbd, now);

    let core = match &row.core_rank {
        Some(rank) => Span::styled(rank.clone(), Style::new().fg(theme::core_rank_color(rank))),
        None => Span::styled("-", theme::DIM),
    };
    let thcpl = match &row.thcpl_rank {
        Some(rank) => Span::styled(rank.clone(), Style::new().fg(theme::thcpl_rank_color(rank))),
        None => Span::styled("-", theme::DIM),
    };

    vec![
        Cell::from(Line::from(favorite)),
        Cell::from(Line::from(Span::styled(
            format!("{} {}", row.title, row.year),
            title_style,
        ))),
        Cell::from(Line::from(Span::styled(
            row.sub.clone(),
            Style::new().fg(theme::sub_color(&row.sub)),
        ))),
        Cell::from(Line::from(Span::styled(
            row.rank.clone(),
            Style::new().fg(theme::ccf_rank_color(&row.rank)),
        ))),
        Cell::from(Line::from(core)),
        Cell::from(Line::from(thcpl)),
        Cell::from(Line::from(Span::styled(
            countdown.text,
            theme::countdown_style(countdown.color),
        ))),
        Cell::from(row.date.clone()),
        Cell::from(Line::from(Span::styled(row.place.clone(), theme::PLACE_STYLE))),
    ]
}

fn render_table(frame: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    let header = Row::new(COLUMNS.iter().map(|c| Cell::from(*c))).style(theme::SECTION_STYLE);

    let rows: Vec<Row> = app
        .visible
        .iter()
        .map(|row| Row::new(row_cells(row, now)).style(theme::BASE))
        .collect();

    let widths = [
        Constraint::Length(1),
        Constraint::Min(18),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Min(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL))
        .row_highlight_style(theme::SELECTED_STYLE)
        .highlight_symbol("");

    let mut state = ratatui::widgets::TableState::new().with_selected(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let keys: &[(&str, &str)] = match app.focus {
            Focus::Search => &[
                ("Type", "Search titles"),
                ("Enter", "Apply"),
                ("Esc", "Clear"),
            ],
            Focus::Sidebar => &[
            ("↑↓", "Move"),
            ("Space", "Toggle"),
            ("Tab", "Table"),
            ("Enter", "Search"),
            ("Esc", "Reset"),
            ("?", "Help"),
            ("q", "Quit"),
        ],
        Focus::Table => &[
            ("j/k", "Move"),
            ("g/G", "Ends"),
            ("f", "Favorite"),
            ("Tab", "Filters"),
            ("l", "Language"),
            ("r", "Refresh"),
            ("Esc", "Reset"),
            ("?", "Help"),
            ("q", "Quit"),
        ],
    };

    let line = Line::from(
        keys.iter()
            .map(|(key, desc)| Span::raw(format!(" {key} {desc}")))
            .collect::<Vec<Span>>(),
    );
    frame.render_widget(Paragraph::new(line).style(theme::FOOTER_STYLE), area);
}

/// The `?` overlay, ported from `HelpScreen`.
fn render_help(frame: &mut Frame, area: Rect) {
    let sections: [(&str, &[(&str, &str)]); 3] = [
        ("Navigation", &[("j/k", "Move up/down"), ("g / G", "First / last row")]),
        ("Actions", &[("Enter", "Open conference URL"), ("f", "Toggle favorite"),
                      ("l", "Toggle language"), ("r", "Refresh data"), ("q", "Quit")]),
            ("Filter", &[("Tab", "Focus sidebar"), ("/", "Search titles"),
                         ("Space", "Toggle checkbox"), ("Esc", "Reset filters")]),
    ];

        // Title + per-section (blank, heading, rows) + blank + closing hint, plus
        // the two rows the border takes.
        let content = 1 + sections.iter().map(|(_, rows)| rows.len() + 2).sum::<usize>() + 2;
        let height = (content + 2) as u16;
        let width = 52.min(area.width.saturating_sub(2));
    let popup = centered(area, width, height.min(area.height.saturating_sub(2)));

    let mut lines: Vec<Line> = vec![Line::from(Span::styled("Keyboard Shortcuts", theme::TITLE_STYLE))];
    for (title, rows) in sections {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(title, theme::SECTION_STYLE)));
        for (key, desc) in rows {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<6}"), Style::new().fg(theme::PRIMARY)),
                Span::raw(*desc),
            ]));
        }
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled("Press ? or Esc to close", theme::DIM)));

    // Clear first so the table underneath does not show through the popup.
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .style(theme::BASE.bg(theme::SURFACE))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme::DIM)
                        .style(theme::BASE.bg(theme::SURFACE))
                        .title(Span::styled(" Help ", theme::TITLE_STYLE)),
                ),
            popup,
        );
}

/// A `width` x `height` rectangle centred inside `area`.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{DataError, DataService, Favorites};
    use crate::ui::app::App;
    use chrono::TimeZone;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

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
";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2029, 1, 1, 0, 0, 0).unwrap()
    }

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
        App::new(service)
    }

    /// Render into a test terminal and return the buffer as plain text lines.
    fn render(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app, now())).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn contains(lines: &[String], needle: &str) -> bool {
        lines.iter().any(|line| line.contains(needle))
    }

        /// Like [`contains`], but ignores the padding spaces that the terminal
        /// inserts after double-width glyphs (CJK text occupies two cells).
        fn contains_compact(lines: &[String], needle: &str) -> bool {
            let compact: String = lines
                .iter()
                .flat_map(|line| line.chars())
                .filter(|c| !c.is_whitespace())
                .collect();
            let needle: String = needle.chars().filter(|c| !c.is_whitespace()).collect();
            compact.contains(&needle)
        }

    #[test]
    fn draws_header_table_and_footer() {
        let mut app = app();
        let lines = render(&mut app, 100, 24);

        assert!(contains(&lines, "CCF-Deadlines TUI"), "{lines:?}");
        assert!(contains(&lines, "Showing 2 of 2 conferences"), "{lines:?}");
        // Every column header is present.
        for column in COLUMNS {
            assert!(contains(&lines, column), "missing column {column}: {lines:?}");
        }
        // Rows are rendered with title + year.
        assert!(contains(&lines, "CVPR 2030"), "{lines:?}");
        assert!(contains(&lines, "ICDE 2030"), "{lines:?}");
        // Footer hints are rendered.
        assert!(contains(&lines, "Favorite"), "{lines:?}");
        assert!(contains(&lines, "Quit"), "{lines:?}");
    }

    #[test]
    fn sidebar_lists_filters_with_checkboxes() {
        let mut app = app();
        let lines = render(&mut app, 120, 40);

        assert!(contains(&lines, "Filters"), "{lines:?}");
        assert!(contains(&lines, "Categories"), "{lines:?}");
        assert!(contains(&lines, "[x] AI"), "{lines:?}");
        assert!(contains(&lines, "CCF Ranks"), "{lines:?}");
        assert!(contains(&lines, "CORE Ranks"), "{lines:?}");
        assert!(contains(&lines, "THCPL Ranks"), "{lines:?}");
        assert!(contains(&lines, "Show Expired"), "{lines:?}");
        // "Show Expired" starts disabled.
        assert!(contains(&lines, "[ ] Show Expired"), "{lines:?}");
        assert!(contains(&lines, "Search"), "{lines:?}");
    }

    #[test]
    fn unchecked_filter_is_reflected_in_sidebar() {
        let mut app = app();
        app.toggle_category("AI");
        let lines = render(&mut app, 120, 40);
        assert!(contains(&lines, "[ ] AI"), "{lines:?}");
        assert!(contains(&lines, "Showing 1 of 2 conferences"), "{lines:?}");
    }

    #[test]
    fn footer_changes_with_focus() {
        let mut app = app();
        app.focus = Focus::Sidebar;
        let sidebar = render(&mut app, 120, 30);
        assert!(contains(&sidebar, "Toggle"), "{sidebar:?}");

        app.focus = Focus::Table;
        let table = render(&mut app, 120, 30);
        assert!(contains(&table, "Filters"), "{table:?}");
    }

    #[test]
    fn language_switch_is_visible() {
        let mut app = app();
        app.toggle_language();
        let lines = render(&mut app, 120, 30);
            assert!(contains_compact(&lines, "CCF-Deadlines 终端界面"), "{lines:?}");
            assert!(contains_compact(&lines, "显示 2 / 2 个会议"), "{lines:?}");
            assert!(contains_compact(&lines, "筛选"), "{lines:?}");
    }

    #[test]
    fn help_overlay_shows_shortcuts() {
        let mut app = app();
        app.show_help = true;
        let lines = render(&mut app, 120, 40);

        assert!(contains(&lines, "Keyboard Shortcuts"), "{lines:?}");
        assert!(contains(&lines, "Toggle favorite"), "{lines:?}");
        assert!(contains(&lines, "Press ? or Esc to close"), "{lines:?}");
        // The underlying UI is still drawn underneath.
        assert!(contains(&lines, "CCF-Deadlines TUI"), "{lines:?}");
    }

        #[test]
        fn help_overlay_covers_the_table_behind_it() {
            let mut app = app();
            app.show_help = true;
            let lines = render(&mut app, 120, 40);

            // Row data must not bleed through the popup: the overlay is opaque, so
            // the rows it spans are replaced by help content or blank space.
            let overlay_start = lines
                .iter()
                .position(|line| line.contains("Keyboard Shortcuts"))
                .expect("overlay is drawn");
            let spanned = &lines[overlay_start..(overlay_start + 6).min(lines.len())];
            assert!(
                spanned.iter().all(|line| !line.contains("CVPR 2030")),
                "table shows through the help overlay: {spanned:?}"
            );
        }

        #[test]
        fn help_overlay_fits_a_small_terminal() {
            let mut app = app();
            app.show_help = true;
            // Must not panic even when there is no room for the popup.
            let lines = render(&mut app, 60, 12);
            assert!(contains(&lines, "Keyboard Shortcuts"), "{lines:?}");
        }

    #[test]
    fn too_small_terminal_shows_hint() {
        let mut app = app();
        let lines = render(&mut app, 20, 5);
        assert!(contains(&lines, "Terminal too small"), "{lines:?}");
    }

    #[test]
    fn empty_result_set_does_not_panic() {
        let mut app = app();
        app.search_input = "zzzznotfound".to_string();
        app.commit_search();
        let lines = render(&mut app, 120, 30);
        assert!(contains(&lines, "Showing 0 of 2 conferences"), "{lines:?}");
    }

    #[test]
    fn offline_data_is_flagged_as_local() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let mut service = DataService::with_paths(
            None,
            path.join("cache.yml"),
            Favorites::empty_at(path.join("favorites.json")),
        );
        service
            .load_conferences_with(|| Err(DataError::Remote("offline".into())))
            .unwrap();

        let mut app = App::new(service);
        let lines = render(&mut app, 120, 30);
        assert!(contains(&lines, "[LOCAL]"), "{lines:?}");
    }

    #[test]
    fn centered_rect_fits_inside_area() {
        let area = Rect { x: 0, y: 0, width: 100, height: 40 };
        let popup = centered(area, 52, 20);
        assert_eq!(popup, Rect { x: 24, y: 10, width: 52, height: 20 });

        // Oversized requests are clamped, never negative.
        let small = Rect { x: 0, y: 0, width: 10, height: 4 };
        let clamped = centered(small, 52, 20);
        assert_eq!(clamped.width, 10);
        assert_eq!(clamped.height, 4);
    }
}