//! Persistent view state and mouse hit regions.
//!
//! `App` owns conference/filter state. `ViewState` owns terminal-specific state
//! such as scroll offsets and the rectangles produced by the latest render.
//! Keeping them separate makes mouse routing testable without a real terminal.

use std::path::PathBuf;
use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::widgets::TableState;

use super::app::{App, SidebarItem, SortColumn};
use super::render::TableColumn;
use super::table_layout::{self, ColumnWidths, HSCROLL_STEP};

pub const SCROLL_STEP: isize = 3;

/// Clickable regions from the most recently rendered frame.
#[derive(Debug, Default)]
pub struct LayoutMap {
    pub header: Rect,
    pub sidebar: Rect,
    pub table: Rect,
    pub table_header: Rect,
    pub table_rows: Rect,
    pub footer: Rect,
    pub search: Rect,
    pub details: Option<Rect>,
    pub help: Option<Rect>,
    pub help_close: Option<Rect>,
    pub sidebar_rows: Vec<(Rect, SidebarItem)>,
    pub table_columns: Vec<(Rect, SortColumn)>,
    /// Column resize handles in the table header: area, column, its width.
    pub table_handles: Vec<(Rect, TableColumn, u16)>,
    /// Track of the vertical table scrollbar (arrows excluded).
    pub table_vscroll: Option<Rect>,
    /// Track of the horizontal table scrollbar (arrows excluded).
    pub table_hscroll: Option<Rect>,
    /// The whole agent panel, when visible.
    pub agent: Option<Rect>,
    pub agent_transcript: Rect,
    pub agent_cards_area: Rect,
    pub agent_input: Rect,
    /// Rendered lines of each recommendation card, by card index.
    pub agent_cards: Vec<(Rect, usize)>,
    /// Rows of the slash-command popup, by match index.
    pub agent_completions: Vec<(Rect, usize)>,
    /// First-screen shortcuts, by index.
    pub agent_quick: Vec<(Rect, usize)>,
}

/// A mouse drag in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    /// Resizing `column`, which was `width` wide when grabbed at `origin_x`.
    Column {
        column: TableColumn,
        origin_x: u16,
        width: u16,
    },
    VerticalScroll,
    HorizontalScroll,
}

/// Two clicks on the same handle within this time reset the column.
const DOUBLE_CLICK_MS: u128 = 400;

#[derive(Default)]
pub struct ViewState {
    pub table_state: TableState,
    pub layout: LayoutMap,
    pub sidebar_offset: usize,
    /// Number of sidebar visual rows visible in the last frame.
    pub sidebar_page: usize,
    /// Total number of visual rows, where two ranks may share one row.
    pub sidebar_visual_rows: usize,
    /// Custom column widths (dragged by the user).
    pub column_widths: ColumnWidths,
    /// Where `column_widths` is saved; `None` keeps them in memory (tests).
    pub layout_path: Option<PathBuf>,
    /// Horizontal scroll offset of the table, in columns of content.
    pub table_hoffset: u16,
    /// Largest horizontal offset in the last frame.
    pub table_hmax: u16,
    pub drag: Option<Drag>,
    last_handle_click: Option<(Instant, TableColumn)>,
}

impl ViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load saved column widths from `path` and save changes back to it.
    pub fn with_layout_file(mut self, path: PathBuf) -> Self {
        self.column_widths = table_layout::load_widths(&path);
        self.layout_path = Some(path);
        self
    }

    fn save_layout(&self) {
        if let Some(path) = &self.layout_path {
            // Best effort: a read-only home must not break the UI.
            let _ = table_layout::save_widths(path, &self.column_widths);
        }
    }

    pub fn set_column_width(&mut self, column: TableColumn, width: u16) {
        self.column_widths.insert(column, width.max(1));
    }

    /// Finish a drag; a resized column is saved.
    pub fn end_drag(&mut self) {
        if matches!(self.drag.take(), Some(Drag::Column { .. })) {
            self.save_layout();
        }
    }

    /// Returns `true` when this click on `column`'s handle is a double click.
    pub fn handle_clicked(&mut self, column: TableColumn) -> bool {
        let now = Instant::now();
        let double = self.last_handle_click.is_some_and(|(at, previous)| {
            previous == column && now.duration_since(at).as_millis() <= DOUBLE_CLICK_MS
        });
        self.last_handle_click = (!double).then_some((now, column));
        double
    }

    pub fn reset_column_width(&mut self, column: TableColumn) {
        if self.column_widths.remove(&column).is_some() {
            self.save_layout();
        }
    }

    pub fn reset_column_widths(&mut self) {
        self.table_hoffset = 0;
        if !self.column_widths.is_empty() {
            self.column_widths.clear();
            self.save_layout();
        }
    }

    pub fn scroll_columns(&mut self, delta: isize) {
        let step = delta * HSCROLL_STEP as isize;
        let max = self.table_hmax as isize;
        self.table_hoffset = (self.table_hoffset as isize + step).clamp(0, max) as u16;
    }

    pub fn set_hoffset_at(&mut self, track: Rect, x: u16) {
        self.table_hoffset = proportional(track.x, track.width, x, self.table_hmax as usize) as u16;
    }

    /// Move the viewport so `y` on the scrollbar track maps to the same
    /// fraction of the rows; the selection keeps its place on screen.
    pub fn set_voffset_at(&mut self, app: &mut App, track: Rect, y: u16) {
        let page = self.layout.table_rows.height as usize;
        let rows = app.visible.len();
        if rows == 0 || page == 0 {
            return;
        }
        let max = rows.saturating_sub(page);
        let offset = proportional(track.y, track.height, y, max);
        let old = self.table_state.offset();
        let on_screen = app.selected.saturating_sub(old).min(page - 1);
        app.selected = (offset + on_screen).min(rows - 1);
        *self.table_state.offset_mut() = offset;
        self.sync_table(app);
    }

    /// Synchronize table selection and keep the offset within the result set.
    pub fn sync_table(&mut self, app: &App) {
        self.table_state
            .select((!app.visible.is_empty()).then_some(app.selected));
        self.clamp_table_offset(app.visible.len());
    }

    pub fn clamp_table_offset(&mut self, row_count: usize) {
        let page = self.layout.table_rows.height as usize;
        let max_offset = row_count.saturating_sub(page);
        let offset = self.table_state.offset().min(max_offset);
        *self.table_state.offset_mut() = offset;
    }

    /// Wheel scrolling moves the selection with the viewport. This keeps the
    /// selected row visible and works with ratatui's own TableState scrolling.
    pub fn scroll_table(&mut self, app: &mut App, delta: isize) {
        app.move_selection(delta);
        self.sync_table(app);
    }

    pub fn clamp_sidebar_offset(&mut self) {
        let max_offset = self.sidebar_visual_rows.saturating_sub(self.sidebar_page);
        self.sidebar_offset = self.sidebar_offset.min(max_offset);
    }

    pub fn scroll_sidebar(&mut self, delta: isize) {
        let max_offset = self.sidebar_visual_rows.saturating_sub(self.sidebar_page) as isize;
        self.sidebar_offset = (self.sidebar_offset as isize + delta).clamp(0, max_offset) as usize;
    }

    /// Scroll the keyboard-focused sidebar control into view without changing
    /// the current scroll position unnecessarily.
    pub fn sync_sidebar_cursor(&mut self, cursor_row: Option<usize>) {
        self.clamp_sidebar_offset();
        let Some(cursor_row) = cursor_row else {
            return;
        };
        if self.sidebar_page == 0 {
            self.sidebar_offset = 0;
            return;
        }
        if cursor_row < self.sidebar_offset {
            self.sidebar_offset = cursor_row;
        } else if cursor_row >= self.sidebar_offset + self.sidebar_page {
            self.sidebar_offset = cursor_row + 1 - self.sidebar_page;
        }
        self.clamp_sidebar_offset();
    }
}

/// Map `pos` on a track starting at `start` with `len` cells to `0..=max`.
fn proportional(start: u16, len: u16, pos: u16, max: usize) -> usize {
    if len <= 1 || max == 0 {
        return 0;
    }
    let along = pos.saturating_sub(start).min(len - 1) as usize;
    (along * max + (len as usize - 1) / 2) / (len as usize - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_scroll_is_clamped_to_content() {
        let mut view = ViewState::new();
        view.sidebar_visual_rows = 20;
        view.sidebar_page = 5;

        view.scroll_sidebar(-1);
        assert_eq!(view.sidebar_offset, 0);
        view.scroll_sidebar(4);
        assert_eq!(view.sidebar_offset, 4);
        view.scroll_sidebar(100);
        assert_eq!(view.sidebar_offset, 15);
    }

    #[test]
    fn sidebar_cursor_is_scrolled_into_view() {
        let mut view = ViewState::new();
        view.sidebar_visual_rows = 20;
        view.sidebar_page = 5;

        view.sync_sidebar_cursor(Some(8));
        assert_eq!(view.sidebar_offset, 4);
        view.sync_sidebar_cursor(Some(2));
        assert_eq!(view.sidebar_offset, 2);
        view.sync_sidebar_cursor(Some(19));
        assert_eq!(view.sidebar_offset, 15);
    }

    #[test]
    fn table_offset_is_clamped_after_filtering() {
        let mut view = ViewState::new();
        view.layout.table_rows = Rect::new(0, 0, 20, 5);
        *view.table_state.offset_mut() = 50;
        view.clamp_table_offset(12);
        assert_eq!(view.table_state.offset(), 7);

        view.clamp_table_offset(3);
        assert_eq!(view.table_state.offset(), 0);
    }

    #[test]
    fn track_positions_map_to_the_whole_range() {
        assert_eq!(proportional(10, 11, 10, 50), 0);
        assert_eq!(proportional(10, 11, 20, 50), 50);
        assert_eq!(proportional(10, 11, 15, 50), 25);
        assert_eq!(proportional(10, 11, 99, 50), 50);
        assert_eq!(proportional(10, 1, 10, 50), 0);
    }

    #[test]
    fn horizontal_scroll_is_clamped() {
        let mut view = ViewState::new();
        view.table_hmax = 10;
        view.scroll_columns(1);
        assert_eq!(view.table_hoffset, 4);
        view.scroll_columns(5);
        assert_eq!(view.table_hoffset, 10);
        view.scroll_columns(-9);
        assert_eq!(view.table_hoffset, 0);
    }

    #[test]
    fn double_clicks_are_per_column() {
        let mut view = ViewState::new();
        assert!(!view.handle_clicked(TableColumn::Title));
        assert!(!view.handle_clicked(TableColumn::Date));
        assert!(!view.handle_clicked(TableColumn::Title));
        assert!(view.handle_clicked(TableColumn::Title));
        // A third click starts over.
        assert!(!view.handle_clicked(TableColumn::Title));
    }
}
