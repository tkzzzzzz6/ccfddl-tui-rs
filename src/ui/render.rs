//! Drawing: header, filter sidebar, conference table, details and help.
//!
//! Rendering also records the rectangles of interactive controls in
//! `ViewState`, allowing mouse input to use exactly the same layout.

use chrono::{DateTime, Duration, Utc};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, HighlightSpacing, Paragraph, Row, Scrollbar,
    ScrollbarOrientation, ScrollbarState, StatefulWidget, Table, Wrap,
};

use crate::agent::Tier;
use crate::data::ConferenceRow;
use crate::format::format_deadline_countdown;
use crate::model::CATEGORIES;

use super::agent_results::PanelSize;
use super::agent_view;
use super::app::{App, Focus, Language, SIDEBAR_WIDTH, SidebarItem, SortColumn};
use super::table_layout;
use super::theme;
use super::view::{LayoutMap, ViewState};

/// Column headers, in the order of `ConferenceTable.add_columns`.
pub const COLUMNS: [&str; 9] = [
    "★",
    "Title",
    "Sub",
    "CCF",
    "CORE",
    "THCPL",
    "Countdown",
    "Date",
    "Place",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TableColumn {
    Favorite,
    Title,
    Sub,
    CcfRank,
    CoreRank,
    ThcplRank,
    Countdown,
    Date,
    Place,
}

impl TableColumn {
    pub const ALL: [TableColumn; 9] = [
        Self::Favorite,
        Self::Title,
        Self::Sub,
        Self::CcfRank,
        Self::CoreRank,
        Self::ThcplRank,
        Self::Countdown,
        Self::Date,
        Self::Place,
    ];

    /// Stable name used in `layout.yaml`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Favorite => "favorite",
            Self::Title => "title",
            Self::Sub => "sub",
            Self::CcfRank => "ccf",
            Self::CoreRank => "core",
            Self::ThcplRank => "thcpl",
            Self::Countdown => "countdown",
            Self::Date => "date",
            Self::Place => "place",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.key() == key)
    }

    pub fn label(self, language: Language) -> &'static str {
        match (self, language) {
            (Self::Favorite, _) => "★",
            (Self::Title, Language::En) => "Title",
            (Self::Title, Language::Zh) => "会议",
            (Self::Sub, Language::En) => "Sub",
            (Self::Sub, Language::Zh) => "类别",
            (Self::CcfRank, _) => "CCF",
            (Self::CoreRank, _) => "CORE",
            (Self::ThcplRank, _) => "THCPL",
            (Self::Countdown, Language::En) => "Countdown",
            (Self::Countdown, Language::Zh) => "倒计时",
            (Self::Date, Language::En) => "Date",
            (Self::Date, Language::Zh) => "日期",
            (Self::Place, Language::En) => "Place",
            (Self::Place, Language::Zh) => "地点",
        }
    }

    fn sort_column(self) -> Option<SortColumn> {
        match self {
            Self::Title => Some(SortColumn::Title),
            Self::Sub => Some(SortColumn::Sub),
            Self::CcfRank => Some(SortColumn::CcfRank),
            Self::Countdown => Some(SortColumn::Countdown),
            Self::Date => Some(SortColumn::Date),
            Self::Place => Some(SortColumn::Place),
            _ => None,
        }
    }

    pub fn width(self, language: Language) -> Constraint {
        match self {
            Self::Favorite => Constraint::Length(1),
            Self::Title => Constraint::Max(36),
            Self::Sub if language == Language::Zh => Constraint::Length(4),
            Self::Sub => Constraint::Length(3),
            Self::CcfRank => Constraint::Length(3),
            Self::CoreRank => Constraint::Length(4),
            Self::ThcplRank => Constraint::Length(5),
            Self::Countdown => Constraint::Length(14),
            Self::Date => Constraint::Min(18),
            Self::Place => Constraint::Min(12),
        }
    }
}

pub fn columns_for_width(width: u16) -> Vec<TableColumn> {
    use TableColumn::*;
    if width >= 140 {
        vec![
            Favorite, Title, Sub, CcfRank, CoreRank, ThcplRank, Countdown, Date, Place,
        ]
    } else if width >= 100 {
        vec![Favorite, Title, Sub, CcfRank, Countdown, Date]
    } else {
        vec![Favorite, Title, CcfRank, Countdown]
    }
}

fn tr(language: Language, en: &'static str, zh: &'static str) -> &'static str {
    match language {
        Language::En => en,
        Language::Zh => zh,
    }
}

/// Backwards-compatible one-frame renderer used by examples and simple tests.
pub fn draw(frame: &mut Frame, app: &mut App, now: DateTime<Utc>) {
    let mut view = ViewState::new();
    draw_with_view(frame, app, &mut view, now);
}

/// Draw the whole UI and update the persistent scroll/hit-test state.
pub fn draw_with_view(frame: &mut Frame, app: &mut App, view: &mut ViewState, now: DateTime<Utc>) {
    let area = frame.area();
    view.layout = LayoutMap::default();
    if area.width < 50 || area.height < 8 {
        render_too_small(frame, area);
        return;
    }

    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    view.layout.header = header;
    view.layout.footer = footer;

    render_header(frame, app, header);
    render_body(frame, app, view, body, now);
    render_footer(frame, app, footer);

    if app.show_help {
        render_help(frame, app, view, area);
    }
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new("Terminal too small\nResize to at least 50x8")
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
        Span::styled(
            format!(" {title} "),
            theme::HEADER_STYLE.add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {subtitle}"), theme::HEADER_STYLE),
    ];
    if app.is_refreshing {
        spans.push(Span::styled(
            tr(app.language, "  |  Refreshing...", "  |  正在刷新..."),
            Style::new().fg(theme::ACCENT).bg(theme::HEADER_BG),
        ));
    } else if let Some(status) = &app.status {
        spans.push(Span::styled(format!("  |  {status}"), theme::HEADER_STYLE));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme::HEADER_STYLE),
        area,
    );
}

fn render_body(
    frame: &mut Frame,
    app: &mut App,
    view: &mut ViewState,
    area: Rect,
    now: DateTime<Utc>,
) {
    let show_details = app.show_details && area.height >= 16;
    let (main, details) = if show_details {
        let parts = Layout::vertical([Constraint::Min(7), Constraint::Length(9)]).split(area);
        (parts[0], Some(parts[1]))
    } else {
        (area, None)
    };
    view.layout.details = details;

    // The agent panel sits beside the table on wide terminals and replaces
    // it on narrow ones while it has focus.
    let (main, agent_area) = if !app.agent.open {
        (main, None)
    } else if app.agent.panel_size == PanelSize::Full && app.focus.in_agent() {
        (Rect::default(), Some(main))
    } else if main.width >= 120 {
        let panel_width = (main.width * 42 / 100).max(46);
        let [left, right] =
            Layout::horizontal([Constraint::Min(60), Constraint::Length(panel_width)]).areas(main);
        (left, Some(right))
    } else if app.focus.in_agent() {
        (Rect::default(), Some(main))
    } else {
        (main, None)
    };
    if let Some(agent_area) = agent_area {
        agent_view::render_agent(frame, app, view, agent_area, now);
    }
    if main.width == 0 {
        if let Some(details) = details {
            render_details(frame, app, details);
        }
        return;
    }

    let sidebar_width = if main.width < 100 { 24 } else { SIDEBAR_WIDTH };
    let [sidebar, table] =
        Layout::horizontal([Constraint::Length(sidebar_width), Constraint::Min(20)]).areas(main);
    view.layout.sidebar = sidebar;
    view.layout.table = table;

    render_sidebar(frame, app, view, sidebar);
    let columns = columns_for_width(main.width);
    render_table(frame, app, view, table, &columns, now);
    if let Some(details) = details {
        render_details(frame, app, details);
    }
}

/// Sidebar entries as display strings with their checkbox state.
fn sidebar_line(app: &App, item: &SidebarItem) -> Line<'static> {
    let checkbox = |on: bool| if on { "[x] " } else { "[ ] " };

    match item {
        SidebarItem::Header(title) => {
            let title = match (*title, app.language) {
                ("Options", Language::Zh) => "选项",
                (other, _) => other,
            };
            Line::from(Span::styled(title, theme::SECTION_STYLE))
        }
        SidebarItem::CategoriesToggle => {
            let arrow = if app.show_categories { "▾" } else { "▸" };
            Line::from(vec![
                Span::styled(format!("{arrow} "), theme::ACCENT),
                Span::styled(
                    format!(
                        "{} ({}/{})",
                        tr(app.language, "Categories", "分类"),
                        app.filters.subs.len(),
                        CATEGORIES.len()
                    ),
                    theme::SECTION_STYLE,
                ),
            ])
        }
        SidebarItem::Category(sub) => {
            let name =
                CATEGORIES
                    .iter()
                    .find(|c| c.sub == sub)
                    .map_or("", |c| match app.language {
                        Language::En => c.name_en,
                        Language::Zh => c.name,
                    });
            Line::from(vec![
                Span::styled(checkbox(app.is_category_enabled(sub)), theme::TEXT),
                Span::styled(format!("{sub} "), Style::new().fg(theme::sub_color(sub))),
                Span::raw(name.to_string()),
            ])
        }
        SidebarItem::CcfRank(rank) => Line::from(vec![
            Span::styled(checkbox(app.is_ccf_rank_enabled(rank)), theme::TEXT),
            Span::styled(rank.clone(), Style::new().fg(theme::ccf_rank_color(rank))),
        ]),
        SidebarItem::CoreRank(rank) => Line::from(vec![
            Span::styled(checkbox(app.is_core_rank_enabled(rank)), theme::TEXT),
            Span::styled(rank.clone(), Style::new().fg(theme::core_rank_color(rank))),
        ]),
        SidebarItem::ThcplRank(rank) => Line::from(vec![
            Span::styled(checkbox(app.is_thcpl_rank_enabled(rank)), theme::TEXT),
            Span::styled(rank.clone(), Style::new().fg(theme::thcpl_rank_color(rank))),
        ]),
        SidebarItem::ShowExpired => Line::from(vec![
            Span::styled(checkbox(app.filters.show_expired), theme::TEXT),
            Span::raw(tr(app.language, "Show Expired", "显示已截止")),
        ]),
        SidebarItem::Search => {
            let caret = if app.focus == Focus::Search { "_" } else { "" };
            let query = if app.search_input.is_empty() {
                Span::styled(
                    tr(app.language, "Search conferences...", "搜索会议..."),
                    theme::DIM,
                )
            } else {
                Span::styled(app.search_input.clone(), theme::SEARCH_STYLE)
            };
            let line = Line::from(vec![
                Span::styled("/ ", Style::new().fg(theme::ACCENT)),
                query,
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

fn is_rank_item(item: &SidebarItem) -> bool {
    matches!(
        item,
        SidebarItem::CcfRank(_) | SidebarItem::CoreRank(_) | SidebarItem::ThcplRank(_)
    )
}

/// Build visual sidebar rows. Rank checkboxes are packed two per row while
/// retaining one logical control per item for keyboard and mouse handling.
fn visual_sidebar_rows(app: &App) -> Vec<Vec<SidebarItem>> {
    let mut rows = Vec::new();
    let mut pending_rank = None;

    for item in app
        .sidebar_items()
        .into_iter()
        .filter(|item| !matches!(item, SidebarItem::Search))
    {
        if is_rank_item(&item) {
            if let Some(previous) = pending_rank.take() {
                rows.push(vec![previous, item]);
            } else {
                pending_rank = Some(item);
            }
        } else {
            if let Some(previous) = pending_rank.take() {
                rows.push(vec![previous]);
            }
            rows.push(vec![item]);
        }
    }
    if let Some(previous) = pending_rank {
        rows.push(vec![previous]);
    }
    rows
}

fn render_sidebar(frame: &mut Frame, app: &App, view: &mut ViewState, area: Rect) {
    let title = tr(app.language, "Filters", "筛选");
    let border = if matches!(app.focus, Focus::Sidebar | Focus::Search) {
        Style::new().fg(theme::PRIMARY)
    } else {
        theme::DIM
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(format!(" {title} "), theme::TITLE_STYLE))
        .border_style(border)
        .style(theme::SIDEBAR_STYLE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let [search_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
    view.layout.search = search_area;
    view.layout
        .sidebar_rows
        .push((search_area, SidebarItem::Search));
    frame.render_widget(
        Paragraph::new(sidebar_line(app, &SidebarItem::Search)).style(theme::SIDEBAR_STYLE),
        search_area,
    );

    let rows = visual_sidebar_rows(app);
    view.sidebar_visual_rows = rows.len();
    view.sidebar_page = list_area.height as usize;

    let controls = app.sidebar_controls();
    let cursor_item = controls.get(app.sidebar_cursor);
    let cursor_row = rows
        .iter()
        .position(|row| cursor_item.is_some_and(|cursor| row.contains(cursor)));
    view.sync_sidebar_cursor(cursor_row);

    let focused = app.focus == Focus::Sidebar;
    for (visual_index, row) in rows
        .iter()
        .enumerate()
        .skip(view.sidebar_offset)
        .take(view.sidebar_page)
    {
        let row_area = Rect::new(
            list_area.x,
            list_area.y + (visual_index - view.sidebar_offset) as u16,
            list_area.width,
            1,
        );
        let widths = if row.len() == 2 {
            Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
                .spacing(1)
                .split(row_area)
        } else {
            Layout::horizontal([Constraint::Min(1)]).split(row_area)
        };

        for (item, item_area) in row.iter().zip(widths.iter()) {
            let selected = focused && cursor_item.is_some_and(|cursor| cursor == item);
            let mut line = sidebar_line(app, item);
            if selected {
                line = line.style(theme::SELECTED_STYLE);
            }
            frame.render_widget(Paragraph::new(line).style(theme::SIDEBAR_STYLE), *item_area);
            view.layout.sidebar_rows.push((*item_area, item.clone()));
        }
    }

    if rows.len() > view.sidebar_page && list_area.height > 0 {
        let mut state = ScrollbarState::new(rows.len())
            .position(view.sidebar_offset)
            .viewport_content_length(view.sidebar_page);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(theme::ACCENT)
                .track_style(theme::DIM),
            list_area,
            &mut state,
        );
    }
}

/// Format one row into table cells.
fn compact_countdown(row: &ConferenceRow, now: DateTime<Utc>, language: Language) -> Span<'static> {
    let countdown = format_deadline_countdown(row.deadline, row.is_tbd, now);
    let text = if row.is_tbd {
        tr(language, "TBD", "待定").to_string()
    } else if let Some(deadline) = row.deadline {
        if deadline <= now {
            tr(language, "Expired", "已截止").to_string()
        } else {
            let remaining = deadline - now;
            let total_seconds = remaining.num_seconds();
            let days = remaining.num_days();
            let hours = total_seconds.div_euclid(3600).rem_euclid(24);
            let minutes = total_seconds.div_euclid(60).rem_euclid(60);
            let seconds = total_seconds.rem_euclid(60);
            if remaining < Duration::days(1) {
                format!(
                    "{:02}:{minutes:02}:{seconds:02}",
                    total_seconds.div_euclid(3600)
                )
            } else if remaining < Duration::days(30) {
                format!("{days}d {hours:02}:{minutes:02}:{seconds:02}")
            } else {
                format!("{days}d {hours:02}h")
            }
        }
    } else {
        "N/A".to_string()
    };
    Span::styled(text, theme::countdown_style(countdown.color))
}

fn row_cells(
    row: &ConferenceRow,
    columns: &[TableColumn],
    language: Language,
    now: DateTime<Utc>,
    tier: Option<Tier>,
) -> Vec<Cell<'static>> {
    let title_style = if row.is_running && !row.is_tbd {
        theme::BOLD
    } else {
        theme::DIM
    };

    columns
        .iter()
        .map(|column| {
            let line = match column {
                TableColumn::Favorite => {
                    let favorite = if row.is_favorite {
                        Span::styled("★", Style::new().fg(theme::WARNING))
                    } else {
                        Span::styled("☆", theme::DIM)
                    };
                    Line::from(favorite)
                }
                TableColumn::Title => {
                    let title = Span::styled(format!("{} {}", row.title, row.year), title_style);
                    match tier {
                        // Recommended in the round on screen: a dot in the
                        // tier's colour, like the cards.
                        Some(tier) => Line::from(vec![
                            Span::styled("● ", agent_view::tier_style(tier)),
                            title,
                        ]),
                        None => Line::from(title),
                    }
                }
                TableColumn::Sub => Line::from(Span::styled(
                    row.sub.clone(),
                    Style::new().fg(theme::sub_color(&row.sub)),
                )),
                TableColumn::CcfRank => Line::from(Span::styled(
                    row.rank.clone(),
                    Style::new().fg(theme::ccf_rank_color(&row.rank)),
                )),
                TableColumn::CoreRank => match &row.core_rank {
                    Some(rank) => Line::from(Span::styled(
                        rank.clone(),
                        Style::new().fg(theme::core_rank_color(rank)),
                    )),
                    None => Line::from(Span::styled("-", theme::DIM)),
                },
                TableColumn::ThcplRank => match &row.thcpl_rank {
                    Some(rank) => Line::from(Span::styled(
                        rank.clone(),
                        Style::new().fg(theme::thcpl_rank_color(rank)),
                    )),
                    None => Line::from(Span::styled("-", theme::DIM)),
                },
                TableColumn::Countdown => Line::from(compact_countdown(row, now, language)),
                TableColumn::Date => Line::from(row.date.clone()),
                TableColumn::Place => {
                    Line::from(Span::styled(row.place.clone(), theme::PLACE_STYLE))
                }
            };
            Cell::from(line)
        })
        .collect()
}

/// Column headers, with the active sort column marked by an arrow.
fn header_cells(app: &App, columns: &[TableColumn]) -> Vec<Cell<'static>> {
    columns
        .iter()
        .map(|column| {
            let active = column.sort_column() == Some(app.sort_column);
            let style = if active {
                theme::TITLE_STYLE
            } else {
                theme::SECTION_STYLE
            };
            let label = column.label(app.language);
            if active {
                let arrow = if app.sort_reverse { '▼' } else { '▲' };
                Cell::from(Line::from(vec![
                    Span::styled(format!("{label} "), style),
                    Span::styled(arrow.to_string(), style),
                ]))
            } else {
                Cell::from(Span::styled(label, style))
            }
        })
        .collect()
}

fn render_table(
    frame: &mut Frame,
    app: &App,
    view: &mut ViewState,
    area: Rect,
    columns: &[TableColumn],
    now: DateTime<Utc>,
) {
    let title = tr(app.language, "Conferences", "会议");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if app.focus == Focus::Table {
            Style::new().fg(theme::PRIMARY)
        } else {
            theme::DIM
        })
        .title(Span::styled(format!(" {title} "), theme::TITLE_STYLE));
    let inner = block.inner(area);
    view.layout.table_header = Rect::new(inner.x, inner.y, inner.width, 1);
    view.layout.table_rows = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(1),
    );
    view.sync_table(app);

    let geometry = table_layout::layout(columns, app.language, &view.column_widths, inner.width);
    view.table_hmax = geometry.max_offset();
    view.table_hoffset = view.table_hoffset.min(view.table_hmax);
    let offset = view.table_hoffset;

    let range = if app.visible.is_empty() {
        "0 / 0".to_string()
    } else {
        let start = view.table_state.offset() + 1;
        let end = (view.table_state.offset() + view.layout.table_rows.height as usize)
            .min(app.visible.len());
        format!("{start}-{end} / {}", app.visible.len())
    };
    let range_width = range.chars().count() as u16;
    frame.render_widget(block.title_bottom(Line::from(range).right_aligned()), area);

    let header = Row::new(header_cells(app, columns)).style(theme::SECTION_STYLE);
    let rows: Vec<Row> = app
        .visible
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let style = if index % 2 == 0 {
                Style::new().bg(theme::SURFACE)
            } else {
                Style::new().bg(theme::SURFACE_DARK)
            };
            let tier = app.recommended_tier(&row.title, row.year);
            Row::new(row_cells(row, columns, app.language, now, tier)).style(style)
        })
        .collect();
    let widths: Vec<Constraint> = geometry
        .columns
        .iter()
        .map(|c| Constraint::Length(c.width))
        .collect();
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(table_layout::SPACING)
        .flex(Flex::Start)
        .highlight_spacing(HighlightSpacing::Always)
        .row_highlight_style(theme::SELECTED_STYLE)
        .highlight_symbol(">");

    // Render the full-width table off screen, then copy the frozen columns
    // and the horizontally scrolled part into the frame.
    let content = Rect::new(0, 0, geometry.content_width.max(inner.width), inner.height);
    let mut buffer = Buffer::empty(content);
    StatefulWidget::render(table, content, &mut buffer, &mut view.table_state);
    let target = frame.buffer_mut();
    for y in 0..inner.height {
        for x in 0..inner.width {
            let source = geometry.to_content(x, offset);
            if source < content.width {
                target[(inner.x + x, inner.y + y)] = buffer[(source, y)].clone();
            }
        }
    }

    record_table_hits(view, &geometry, offset);
    if inner.height > 0 {
        let edge = inner.right();
        for (handle, _, _) in view.layout.table_handles.iter().filter(|h| h.0.x < edge) {
            let cell = &mut target[(handle.x, handle.y)];
            cell.set_symbol("│");
            if let Some(fg) = theme::DIM.fg {
                cell.set_fg(fg);
            }
        }
    }

    if app.visible.is_empty() {
        let message = if app.is_refreshing {
            tr(app.language, "Loading conferences...", "正在加载会议...")
        } else {
            tr(
                app.language,
                "No conferences match. Press Esc to reset filters.",
                "没有匹配的会议，按 Esc 重置筛选。",
            )
        };
        frame.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .style(theme::DIM),
            view.layout.table_rows,
        );
    } else if app.visible.len() > view.layout.table_rows.height as usize
        && view.layout.table_rows.height > 0
    {
        let rows_area = view.layout.table_rows;
        let mut state = ScrollbarState::new(app.visible.len())
            .position(view.table_state.offset())
            .viewport_content_length(rows_area.height as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(theme::ACCENT)
                .track_style(theme::DIM),
            rows_area,
            &mut state,
        );
        if rows_area.height > 2 {
            view.layout.table_vscroll = Some(Rect::new(
                rows_area.right() - 1,
                rows_area.y + 1,
                1,
                rows_area.height - 2,
            ));
        }
    }

    // The horizontal scrollbar lives on the bottom border, left of the range.
    if geometry.overflows() && area.width > range_width + 8 {
        let bar = Rect::new(
            area.x + 1,
            area.bottom() - 1,
            area.width - range_width - 4,
            1,
        );
        let scrollable = (geometry.content_width - geometry.pinned) as usize;
        let mut state = ScrollbarState::new(scrollable)
            .position(offset as usize)
            .viewport_content_length((geometry.viewport - geometry.pinned) as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                .thumb_style(theme::ACCENT)
                .track_style(theme::DIM),
            bar,
            &mut state,
        );
        view.layout.table_hscroll = Some(Rect::new(bar.x + 1, bar.y, bar.width - 2, 1));
    }
}

/// Sort targets and resize handles of the header, in screen coordinates.
fn record_table_hits(view: &mut ViewState, geometry: &table_layout::TableGeometry, offset: u16) {
    let header = view.layout.table_header;
    if header.height == 0 {
        return;
    }
    for placed in &geometry.columns {
        if let Some(sort) = placed.column.sort_column()
            && let Some((x, width)) = geometry.visible_span(placed.x, placed.width, offset)
        {
            view.layout
                .table_columns
                .push((Rect::new(header.x + x, header.y, width, 1), sort));
        }
        if placed.column == TableColumn::Favorite {
            continue;
        }
        let handle = placed.x + placed.width;
        // A column that ends at the right edge is resized from the border.
        let screen = geometry.to_screen(handle, offset).or_else(|| {
            let at_edge =
                handle >= geometry.pinned + offset && handle - offset == geometry.viewport;
            at_edge.then_some(geometry.viewport)
        });
        if let Some(x) = screen {
            view.layout.table_handles.push((
                Rect::new(header.x + x, header.y, 1, 1),
                placed.column,
                placed.width,
            ));
        }
    }
}

fn render_details(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::DIM)
        .title(Span::styled(
            format!(" {} ", tr(app.language, "Details", "详情")),
            theme::TITLE_STYLE,
        ))
        .style(Style::new().bg(theme::SURFACE));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(row) = app.selected_row() else {
        frame.render_widget(Paragraph::new("No row selected").style(theme::DIM), inner);
        return;
    };
    let conference = app
        .service
        .conferences()
        .iter()
        .find(|conference| conference.title == row.title);
    let year = conference.and_then(|conference| {
        conference
            .confs
            .iter()
            .find(|conference_year| conference_year.year == row.year)
    });

    let favorite = if row.is_favorite { "★" } else { "☆" };
    let mut lines = vec![
        Line::from(Span::styled(
            format!("{favorite} {} {}", row.title, row.year),
            theme::BOLD,
        )),
        Line::from(format!(
            "CCF {}  CORE {}  THCPL {}  {} {}",
            row.rank,
            row.core_rank.as_deref().unwrap_or("-"),
            row.thcpl_rank.as_deref().unwrap_or("-"),
            tr(app.language, "Category", "分类"),
            row.sub
        )),
        Line::from(format!(
            "{}: {}    {}: {}",
            tr(app.language, "Date", "日期"),
            row.date,
            tr(app.language, "Place", "地点"),
            row.place
        )),
    ];

    if let Some(year) = year {
        let deadlines = year
            .timeline
            .iter()
            .map(|timeline| timeline.deadline.clone())
            .collect::<Vec<_>>()
            .join(" | ");
        let abstracts = year
            .timeline
            .iter()
            .filter_map(|timeline| timeline.abstract_deadline.clone())
            .collect::<Vec<_>>()
            .join(" | ");
        let comments = year
            .timeline
            .iter()
            .filter_map(|timeline| timeline.comment.clone())
            .collect::<Vec<_>>()
            .join(" | ");
        lines.push(Line::from(format!(
            "{}: {deadlines} ({})",
            tr(app.language, "Deadline", "截止时间"),
            year.timezone
        )));
        if !abstracts.is_empty() {
            lines.push(Line::from(format!(
                "{}: {abstracts}",
                tr(app.language, "Abstract", "摘要截止")
            )));
        }
        lines.push(Line::from(format!("URL: {}", year.link)));
        if !comments.is_empty() {
            lines.push(Line::from(format!(
                "{}: {comments}",
                tr(app.language, "Notes", "备注")
            )));
        }
    } else {
        lines.push(Line::from(format!("URL: {}", row.link)));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let language = app.language;
    let keys: Vec<(&str, &str)> = match app.focus {
        Focus::Search => vec![
            ("Type", tr(language, "Search", "输入")),
            ("Enter", tr(language, "Apply", "应用")),
            ("Esc", tr(language, "Clear", "清空")),
            ("Tab", tr(language, "Table", "表格")),
        ],
        Focus::Sidebar => vec![
            ("↑↓", tr(language, "Move", "移动")),
            ("Space", tr(language, "Toggle", "切换")),
            ("/", tr(language, "Search", "搜索")),
            ("Tab", tr(language, "Table", "表格")),
            ("?", tr(language, "Help", "帮助")),
            ("q", tr(language, "Quit", "退出")),
        ],
        Focus::AgentCards => {
            let mut keys = vec![
                ("j/k", tr(language, "Move", "移动")),
                ("⏎", tr(language, "Show in table", "跳到表格")),
                ("f", tr(language, "Favorite", "收藏")),
                ("o", tr(language, "Open", "官网")),
                ("d", tr(language, "Details", "详情")),
                ("?", tr(language, "Ask", "追问")),
                ("v", tr(language, "Only these", "只看推荐")),
            ];
            if app.agent.rounds.len() > 1 {
                keys.push(("[ ]", tr(language, "Rounds", "轮次")));
            }
            keys.extend([
                ("i", tr(language, "Input", "输入")),
                ("F2", tr(language, "Size", "尺寸")),
                ("Tab", tr(language, "Table", "表格")),
            ]);
            keys
        }
        Focus::Agent if app.agent.completion_visible() => vec![
            ("↑↓", tr(language, "Choose", "选择")),
            ("Tab", tr(language, "Complete", "补全")),
            ("⏎", tr(language, "Send", "发送")),
            ("Esc", tr(language, "Close list", "关闭列表")),
        ],
        Focus::Agent if app.agent.setup.is_some() || app.agent.wizard.is_some() => vec![
            ("⏎", tr(language, "Answer", "回答")),
            ("Esc", tr(language, "Cancel", "取消")),
        ],
        Focus::Agent if app.quick_actions_visible() => vec![
            ("1-4", tr(language, "Shortcut", "快捷入口")),
            ("⏎", tr(language, "Send", "发送")),
            ("/", tr(language, "Commands", "命令")),
            ("Tab", tr(language, "Table", "表格")),
            ("Esc", tr(language, "Close", "关闭")),
        ],
        Focus::Agent => vec![
            ("⏎", tr(language, "Send", "发送")),
            ("↑↓", tr(language, "History", "历史")),
            ("/", tr(language, "Commands", "命令")),
            ("Tab", tr(language, "Cards / table", "卡片/表格")),
            ("PgUp/PgDn", tr(language, "Scroll", "滚动")),
            ("F2", tr(language, "Size", "尺寸")),
            ("Esc", tr(language, "Stop/Close", "中断/关闭")),
        ],
        Focus::Table => vec![
            ("j/k", tr(language, "Move", "移动")),
            ("⏎", tr(language, "Open", "打开")),
            ("/", tr(language, "Search", "搜索")),
            ("f", tr(language, "Favorite", "收藏")),
            ("d", tr(language, "Details", "详情")),
            ("a", tr(language, "Agent", "推荐助手")),
            ("Tab", tr(language, "Filters", "筛选")),
            ("?", tr(language, "Help", "帮助")),
            ("q", tr(language, "Quit", "退出")),
        ],
    };

    let mut spans = Vec::new();
    for (key, description) in keys {
        spans.push(Span::styled(
            format!(" {key} "),
            Style::new().fg(theme::TEXT).bg(theme::FOOTER_BG),
        ));
        spans.push(Span::styled(
            format!("{description}  "),
            theme::FOOTER_STYLE,
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme::FOOTER_STYLE),
        area,
    );
}

/// Keyboard and mouse help, rendered as an opaque modal with a clickable close
/// region. All other mouse input is blocked while this modal is visible.
fn render_help(frame: &mut Frame, app: &App, view: &mut ViewState, area: Rect) {
    let language = app.language;
    let sections: [(&str, &[(&str, &str)]); 3] = [
        (
            tr(language, "Navigation", "导航"),
            &[
                ("j/k", tr(language, "Move up/down", "上下移动")),
                ("g / G", tr(language, "First / last row", "首行 / 末行")),
                ("Click", tr(language, "Select row", "选择会议")),
                ("Wheel", tr(language, "Scroll pane", "滚动区域")),
                ("← / →", tr(language, "Scroll columns", "左右滚动列")),
            ],
        ),
        (
            tr(language, "Actions", "操作"),
            &[
                ("Enter", tr(language, "Open conference URL", "打开会议链接")),
                ("f", tr(language, "Toggle favorite", "切换收藏")),
                ("d", tr(language, "Toggle details", "展开 / 收起详情")),
                ("s / S", tr(language, "Sort / reverse", "排序 / 反向")),
                (
                    "a",
                    tr(language, "Ask the recommendation agent", "打开会议推荐助手"),
                ),
                (
                    "A",
                    tr(
                        language,
                        "Ask the agent about this row",
                        "让推荐助手评估当前会议",
                    ),
                ),
                ("l", tr(language, "Toggle language", "切换语言")),
                ("r", tr(language, "Refresh data", "刷新数据")),
                ("q", tr(language, "Quit", "退出")),
            ],
        ),
        (
            tr(language, "Filters & Mouse", "筛选与鼠标"),
            &[
                ("Tab", tr(language, "Focus sidebar", "聚焦侧栏")),
                ("/", tr(language, "Search titles", "搜索会议")),
                ("Space", tr(language, "Toggle checkbox", "切换复选框")),
                ("Header", tr(language, "Click to sort", "点击表头排序")),
                (
                    "Drag │",
                    tr(
                        language,
                        "Resize column (= resets)",
                        "拖动调整列宽（= 恢复）",
                    ),
                ),
                ("Esc", tr(language, "Reset filters", "重置筛选")),
            ],
        ),
    ];

    let content = 1
        + sections
            .iter()
            .map(|(_, rows)| rows.len() + 2)
            .sum::<usize>()
        + 3;
    let height = (content + 2) as u16;
    let width = 64.min(area.width.saturating_sub(2));
    let popup = centered(area, width, height.min(area.height.saturating_sub(2)));
    view.layout.help = Some(popup);

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        tr(language, "Keyboard & Mouse", "键盘与鼠标"),
        theme::TITLE_STYLE,
    ))];
    for (title, rows) in sections {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(title, theme::SECTION_STYLE)));
        for (key, description) in rows {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<7}"), Style::new().fg(theme::PRIMARY)),
                Span::raw(*description),
            ]));
        }
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        tr(language, "Press ? or Esc to close", "按 ? 或 Esc 关闭"),
        theme::DIM,
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(theme::BASE.bg(theme::SURFACE))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(theme::DIM)
                    .style(theme::BASE.bg(theme::SURFACE))
                    .title(Span::styled(
                        tr(language, " Help ", " 帮助 "),
                        theme::TITLE_STYLE,
                    )),
            ),
        popup,
    );

    let close = Rect::new(popup.right().saturating_sub(5), popup.y, 4, 1);
    view.layout.help_close = Some(close);
    frame.render_widget(
        Paragraph::new("[x]")
            .alignment(Alignment::Center)
            .style(theme::TITLE_STYLE),
        close,
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
        let lines = render(&mut app, 160, 24);

        assert!(contains(&lines, "CCF-Deadlines TUI"), "{lines:?}");
        assert!(contains(&lines, "Showing 2 of 2 conferences"), "{lines:?}");
        // Every column header is present.
        for column in COLUMNS {
            assert!(
                contains(&lines, column),
                "missing column {column}: {lines:?}"
            );
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
        assert!(
            contains_compact(&lines, "CCF-Deadlines 终端界面"),
            "{lines:?}"
        );
        assert!(contains_compact(&lines, "显示 2 / 2 个会议"), "{lines:?}");
        assert!(contains_compact(&lines, "筛选"), "{lines:?}");
        assert!(contains_compact(&lines, "倒计时"), "{lines:?}");
        assert!(contains_compact(&lines, "类别"), "{lines:?}");
        assert!(contains_compact(&lines, "显示已截止"), "{lines:?}");
    }

    #[test]
    fn help_overlay_shows_shortcuts() {
        let mut app = app();
        app.show_help = true;
        let lines = render(&mut app, 120, 40);

        assert!(contains(&lines, "Keyboard & Mouse"), "{lines:?}");
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
            .position(|line| line.contains("Keyboard & Mouse"))
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
        assert!(contains(&lines, "Keyboard & Mouse"), "{lines:?}");
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
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 40,
        };
        let popup = centered(area, 52, 20);
        assert_eq!(
            popup,
            Rect {
                x: 24,
                y: 10,
                width: 52,
                height: 20
            }
        );

        // Oversized requests are clamped, never negative.
        let small = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 4,
        };
        let clamped = centered(small, 52, 20);
        assert_eq!(clamped.width, 10);
        assert_eq!(clamped.height, 4);
    }
}
