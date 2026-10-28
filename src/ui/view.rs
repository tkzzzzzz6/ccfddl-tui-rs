//! Persistent view state and mouse hit regions.
//!
//! `App` owns conference/filter state. `ViewState` owns terminal-specific state
//! such as scroll offsets and the rectangles produced by the latest render.
//! Keeping them separate makes mouse routing testable without a real terminal.

use ratatui::layout::Rect;
use ratatui::widgets::TableState;

use super::app::{App, SidebarItem, SortColumn};

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

#[derive(Default)]
pub struct ViewState {
    pub table_state: TableState,
    pub layout: LayoutMap,
    pub sidebar_offset: usize,
    /// Number of sidebar visual rows visible in the last frame.
    pub sidebar_page: usize,
    /// Total number of visual rows, where two ranks may share one row.
    pub sidebar_visual_rows: usize,
}

impl ViewState {
    pub fn new() -> Self {
        Self::default()
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
}
