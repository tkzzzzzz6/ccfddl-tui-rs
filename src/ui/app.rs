//! Application state and input handling.
//!
//! Mirrors `CCFDeadlinesApp` and `FilterSidebar` from the Python app: it owns
//! the filter state, the visible rows, the cursor, the language toggle and the
//! one-second countdown tick. Rendering lives in `render.rs`, so this module
//! stays pure logic and is unit-testable without a terminal.

use chrono::{DateTime, Utc};

use crate::data::query::{self, FilterState};
use crate::data::{ConferenceRow, DataService};
use crate::format::format_deadline_countdown;
use crate::model::CATEGORIES;

pub const SIDEBAR_WIDTH: u16 = 28;

pub const CCF_RANKS: [&str; 4] = ["A", "B", "C", "N"];
pub const CORE_RANKS: [&str; 5] = ["A*", "A", "B", "C", "N"];
pub const THCPL_RANKS: [&str; 3] = ["A", "B", "N"];

/// `(sub code, English name)` for every category, in sidebar order.
pub fn category_items() -> Vec<(&'static str, &'static str)> {
    CATEGORIES.iter().map(|c| (c.sub, c.name_en)).collect()
}

/// Which pane has keyboard focus, mirroring the Textual focus model.
///
/// Search is a separate mode so that printable keys (including `r`, `q` and
/// `?`) reach the text field instead of being swallowed by global shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Table,
    Sidebar,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    Zh,
}

impl Language {
    pub fn toggled(self) -> Self {
        match self {
            Self::En => Self::Zh,
            Self::Zh => Self::En,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::En => "CCF-Deadlines TUI",
            Self::Zh => "CCF-Deadlines 终端界面",
        }
    }
}

/// One row of the filter sidebar: either a section heading or a control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarItem {
    Header(&'static str),
    Category(String),
    CcfRank(String),
    CoreRank(String),
    ThcplRank(String),
    ShowExpired,
    Search,
}

impl SidebarItem {
    /// Headings are labels only; the sidebar cursor skips them.
    pub fn is_selectable(&self) -> bool {
        !matches!(self, Self::Header(_))
    }
}

pub struct App {
    pub service: DataService,
    pub filters: FilterState,
    pub rows: Vec<ConferenceRow>,
    pub visible: Vec<ConferenceRow>,
    pub selected: usize,
    pub focus: Focus,
    pub language: Language,
    pub status: Option<String>,
    pub show_help: bool,
    /// Index into the sidebar's selectable entries, so checkboxes can be
    /// operated from the keyboard the way the mouse does in the Textual app.
    pub sidebar_cursor: usize,
    /// Text typed into the sidebar search box, committed on Enter.
    pub search_input: String,
}

impl App {
    pub fn new(service: DataService) -> Self {
        let mut app = Self {
            service,
            filters: FilterState::default(),
            rows: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            focus: Focus::Table,
            language: Language::En,
            status: None,
            show_help: false,
                        sidebar_cursor: 0,
                        search_input: String::new(),
                    };
        app.reload(Utc::now());
        app
    }

    /// Re-derive rows from the service and re-apply the current filters.
    pub fn reload(&mut self, now: DateTime<Utc>) {
        self.rows = self.service.process_rows(now);
        self.refresh_visible();
    }

    /// Recompute the visible rows and clamp the cursor back into range.
    pub fn refresh_visible(&mut self) {
        self.visible = query::visible_rows(&self.rows, &self.filters);
        self.selected = if self.visible.is_empty() {
            0
        } else {
            self.selected.min(self.visible.len() - 1)
        };
    }

    pub fn selected_row(&self) -> Option<&ConferenceRow> {
        self.visible.get(self.selected)
    }

    pub fn total_count(&self) -> usize {
        self.rows.len()
    }

    pub fn toggle_language(&mut self) {
        self.language = self.language.toggled();
    }

    pub fn toggle_category(&mut self, sub: &str) {
        if !self.filters.subs.remove(sub) {
            self.filters.subs.insert(sub.to_string());
        }
        self.refresh_visible();
    }

    pub fn toggle_ccf_rank(&mut self, rank: &str) {
        if !self.filters.ccf_ranks.remove(rank) {
            self.filters.ccf_ranks.insert(rank.to_string());
        }
        self.refresh_visible();
    }

    pub fn toggle_core_rank(&mut self, rank: &str) {
        if !self.filters.core_ranks.remove(rank) {
            self.filters.core_ranks.insert(rank.to_string());
        }
        self.refresh_visible();
    }

    pub fn toggle_thcpl_rank(&mut self, rank: &str) {
        if !self.filters.thcpl_ranks.remove(rank) {
            self.filters.thcpl_ranks.insert(rank.to_string());
        }
        self.refresh_visible();
    }

    pub fn toggle_show_expired(&mut self) {
        self.filters.show_expired = !self.filters.show_expired;
        self.refresh_visible();
    }

    /// Commit the search box contents and re-filter.
    pub fn commit_search(&mut self) {
        self.filters.query = self.search_input.trim().to_lowercase();
        self.refresh_visible();
    }

    /// Reset every filter, matching `FilterSidebar.reset_filters`.
    pub fn reset_filters(&mut self) {
        self.filters = FilterState::default();
        self.search_input.clear();
        self.refresh_visible();
    }

    pub fn select_next(&mut self) {
        if !self.visible.is_empty() {
            self.selected = (self.selected + 1).min(self.visible.len() - 1);
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.visible.len().saturating_sub(1);
    }

    /// Toggle the favourite flag of the selected row and persist it.
    pub fn toggle_favorite(&mut self) {
        let Some(row) = self.selected_row().cloned() else {
            self.status = Some("No row selected".to_string());
            return;
        };

        let outcome = self.service.toggle_favorite(&row);
        for candidate in &mut self.rows {
            if candidate.title == row.title && candidate.year == row.year {
                candidate.is_favorite = outcome.is_favorite;
            }
        }
        self.status = match outcome.save_error {
            Some(error) => Some(format!("Favorites not saved: {error}")),
            None if outcome.is_favorite => Some(format!("{} added to favorites", row.title)),
            None => Some(format!("{} removed from favorites", row.title)),
        };
        self.refresh_visible();
                // Favourites jump to the top of the list, so keep the cursor on the row
                // the user acted on instead of leaving it on whatever slid into the
                // old slot.
                self.select_conference(&row.title, row.year);
            }

            /// Move the cursor onto a specific conference, if it is still visible.
            pub fn select_conference(&mut self, title: &str, year: i32) {
                if let Some(index) = self
                    .visible
                    .iter()
                    .position(|row| row.title == title && row.year == year)
                {
                    self.selected = index;
                }
            }

    /// Advance the clock by one tick, flipping rows whose deadline just passed.
    ///
    /// Python leaves `is_expired` frozen until the next reload, so expired
    /// conferences linger on screen for the rest of the session. Recomputing it
    /// here is what makes the live countdown honest.
    pub fn tick(&mut self, now: DateTime<Utc>) {
        let mut changed = false;
        for row in &mut self.rows {
            if row.is_tbd {
                continue;
            }
            let expired = row.deadline.is_some_and(|deadline| deadline <= now);
            if expired == row.is_expired {
                continue;
            }
            row.is_expired = expired;
            row.is_running = !expired;
            row.countdown = format_deadline_countdown(row.deadline, false, now).text;
            changed = true;
        }
        if changed {
            self.refresh_visible();
        }
    }

    /// Selectable sidebar entries, i.e. the ones the cursor can visit.
        pub fn sidebar_controls(&self) -> Vec<SidebarItem> {
            self.sidebar_items().into_iter().filter(SidebarItem::is_selectable).collect()
        }

        /// Move the sidebar cursor by `delta`, skipping section headings.
        pub fn move_sidebar_cursor(&mut self, delta: isize) {
            let total = self.sidebar_controls().len();
            if total == 0 {
                return;
            }
            let next = self.sidebar_cursor as isize + delta;
            self.sidebar_cursor = next.clamp(0, total as isize - 1) as usize;
        }

        /// Toggle whatever the sidebar cursor is currently on.
        pub fn toggle_sidebar_item(&mut self) {
            let Some(item) = self.sidebar_controls().get(self.sidebar_cursor).cloned() else {
                return;
            };
            match item {
                SidebarItem::Category(sub) => self.toggle_category(&sub),
                SidebarItem::CcfRank(rank) => self.toggle_ccf_rank(&rank),
                SidebarItem::CoreRank(rank) => self.toggle_core_rank(&rank),
                SidebarItem::ThcplRank(rank) => self.toggle_thcpl_rank(&rank),
                SidebarItem::ShowExpired => self.toggle_show_expired(),
                SidebarItem::Search => self.focus = Focus::Search,
                SidebarItem::Header(_) => {}
            }
        }

        /// Sidebar entries in display order.
        pub fn sidebar_items(&self) -> Vec<SidebarItem> {
        let mut items = vec![SidebarItem::Header("Categories")];
        items.extend(
            category_items()
                .into_iter()
                .map(|(sub, _)| SidebarItem::Category(sub.to_string())),
        );
        items.push(SidebarItem::Header("CCF Ranks"));
        items.extend(CCF_RANKS.iter().map(|r| SidebarItem::CcfRank((*r).to_string())));
        items.push(SidebarItem::Header("CORE Ranks"));
        items.extend(CORE_RANKS.iter().map(|r| SidebarItem::CoreRank((*r).to_string())));
        items.push(SidebarItem::Header("THCPL Ranks"));
        items.extend(THCPL_RANKS.iter().map(|r| SidebarItem::ThcplRank((*r).to_string())));
        items.push(SidebarItem::Header("Options"));
        items.push(SidebarItem::ShowExpired);
        items.push(SidebarItem::Header("Search"));
        items.push(SidebarItem::Search);
        items
    }

    pub fn is_category_enabled(&self, sub: &str) -> bool {
        self.filters.subs.contains(sub)
    }

    pub fn is_ccf_rank_enabled(&self, rank: &str) -> bool {
        self.filters.ccf_ranks.contains(rank)
    }

    pub fn is_core_rank_enabled(&self, rank: &str) -> bool {
        self.filters.core_ranks.contains(rank)
    }

    pub fn is_thcpl_rank_enabled(&self, rank: &str) -> bool {
        self.filters.thcpl_ranks.contains(rank)
    }

    /// `Showing 12 of 345 conferences` / `显示 12 / 345 个会议`.
    pub fn subtitle(&self) -> String {
        let local = if self.service.using_local_data() { " [LOCAL]" } else { "" };
        match self.language {
            Language::En => format!(
                "Showing {} of {} conferences{local}",
                self.visible.len(),
                self.total_count()
            ),
            Language::Zh => format!(
                "显示 {} / {} 个会议{local}",
                self.visible.len(),
                self.total_count()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{DataError, Favorites};
        use chrono::{Duration, TimeZone};
            use std::path::PathBuf;

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

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2029, 1, 1, 0, 0, 0).unwrap()
    }

    /// Builds an app from `result`, keeping cache/favorites under a leaked
    /// temp dir so the paths stay valid for the whole test.
    fn app_from(result: Result<String, DataError>) -> App {
        let dir = tempfile::tempdir().unwrap();
        let path: PathBuf = dir.path().to_path_buf();
        std::mem::forget(dir);

        let mut service = DataService::with_paths(
            None,
            path.join("cache.yml"),
            Favorites::empty_at(path.join("favorites.json")),
        );
        service.load_conferences_with(|| result).unwrap();
        App::new(service)
    }

    fn app() -> App {
        app_from(Ok(YAML.to_string()))
    }

    #[test]
    fn loads_rows_and_applies_default_filters() {
        let app = app();
        assert_eq!(app.total_count(), 3);
        // Expired rows are hidden by default.
        assert_eq!(app.visible.len(), 2);
        assert!(app.visible.iter().all(|r| !r.is_expired));
    }

    #[test]
    fn category_toggle_narrows_and_restores() {
        let mut app = app();
        assert!(app.is_category_enabled("AI"));

        app.toggle_category("AI");
        assert!(!app.is_category_enabled("AI"));
            // Every other category stays enabled, so the set is "all but AI".
            assert!(!app.filters.subs.contains("AI"));
            assert_eq!(app.filters.subs.len(), CATEGORIES.len() - 1);
            // CVPR is the only AI conference and it disappears.
            assert!(!app.visible.iter().any(|r| r.title == "CVPR"));

            app.toggle_category("AI");
            assert_eq!(app.visible.len(), 2);
        }

    #[test]
    fn rank_toggles_filter_rows() {
        let mut app = app();
        app.toggle_ccf_rank("A");
        assert!(!app.is_ccf_rank_enabled("A"));
        assert!(app.visible.iter().all(|r| r.rank == "B"));

        app.toggle_ccf_rank("A");
        assert!(app.is_ccf_rank_enabled("A"));
        assert_eq!(app.visible.len(), 2);
    }

    #[test]
    fn core_and_thcpl_toggles_are_independent() {
        let mut app = app();
        app.toggle_core_rank("A*");
        assert!(!app.is_core_rank_enabled("A*"));
        // ICDE has no CORE rank and therefore always passes the filter.
        assert!(app.visible.iter().all(|r| r.title == "ICDE"));

        app.toggle_core_rank("A*");
        app.toggle_thcpl_rank("A");
        assert!(app.visible.iter().all(|r| r.title == "ICDE"));

        app.toggle_thcpl_rank("A");
        assert_eq!(app.visible.len(), 2);
    }

    #[test]
    fn show_expired_reveals_finished_rows() {
        let mut app = app();
        assert!(!app.filters.show_expired);
        app.toggle_show_expired();
        assert!(app.visible.iter().any(|r| r.title == "OldConf"));
    }

    #[test]
    fn search_commits_trimmed_lowercased_and_filters() {
        let mut app = app();
        app.search_input = "  CVPR ".to_string();
        app.commit_search();
        assert_eq!(app.filters.query, "cvpr");
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.visible[0].title, "CVPR");
    }

    #[test]
    fn reset_restores_defaults() {
        let mut app = app();
        app.toggle_category("AI");
        app.toggle_show_expired();
        app.search_input = "cvpr".to_string();
        app.commit_search();

        app.reset_filters();
        assert_eq!(app.filters.subs, FilterState::default().subs);
        assert!(!app.filters.show_expired);
        assert!(app.filters.query.is_empty());
        assert!(app.search_input.is_empty());
        assert_eq!(app.visible.len(), 2);
    }

    #[test]
    fn selection_is_clamped_to_visible_rows() {
        let mut app = app();
        app.select_last();
        assert_eq!(app.selected, 1);
        app.select_next();
        assert_eq!(app.selected, 1);
        app.select_previous();
        assert_eq!(app.selected, 0);
        app.select_previous();
        assert_eq!(app.selected, 0);
        app.select_first();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn selection_clamps_when_rows_disappear() {
        let mut app = app();
        app.select_last();
        assert_eq!(app.selected, 1);

        app.search_input = "CVPR".to_string();
        app.commit_search();
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn empty_result_set_resets_selection() {
        let mut app = app();
        app.search_input = "zzzznotfound".to_string();
        app.commit_search();
        assert!(app.visible.is_empty());
        assert_eq!(app.selected, 0);
        assert!(app.selected_row().is_none());
    }

    #[test]
    fn favorite_toggle_persists_and_reorders() {
        let mut app = app();
            app.select_last();
            let target = app.selected_row().expect("a row is selected").clone();
            assert!(!target.is_favorite);

            app.toggle_favorite();
            assert!(app.service.is_favorite(&target.title, target.year));
            assert_eq!(
                app.status.as_deref(),
                Some(format!("{} added to favorites", target.title).as_str())
            );
            // Favourites sort to the top of the table, so the cursor now sits on it.
            assert!(app.visible[0].is_favorite);
            assert_eq!(app.visible[0].title, target.title);
            assert_eq!(app.selected, 0);

            app.toggle_favorite();
            assert!(!app.service.is_favorite(&target.title, target.year));
                        // Back in deadline order, but the cursor still tracks the same row.
                        assert_eq!(app.selected_row().map(|r| &r.title), Some(&target.title));
                    }

                    #[test]
                    fn cursor_follows_a_favorite_across_resorts() {
                        let mut app = app();
                        app.select_last();
                        let target = app.selected_row().unwrap().clone();

                        app.toggle_favorite();
                        assert_eq!(app.selected_row().unwrap().title, target.title);

                        // Removing it moves the row back down, and the cursor follows.
                        app.toggle_favorite();
                        assert_eq!(app.selected_row().unwrap().title, target.title);
                        assert!(!app.selected_row().unwrap().is_favorite);
                    }

    #[test]
    fn favorite_on_empty_selection_reports_status() {
        let mut app = app();
        app.search_input = "zzzznotfound".to_string();
        app.commit_search();
        app.toggle_favorite();
        assert_eq!(app.status.as_deref(), Some("No row selected"));
    }

    #[test]
    fn language_toggles_title_and_subtitle() {
        let mut app = app();
        assert_eq!(app.language.title(), "CCF-Deadlines TUI");
        assert!(app.subtitle().starts_with("Showing 2 of 3"));

        app.toggle_language();
        assert_eq!(app.language.title(), "CCF-Deadlines 终端界面");
        assert!(app.subtitle().starts_with("显示 2 / 3"));

        app.toggle_language();
        assert_eq!(app.language, Language::En);
    }

    #[test]
    fn local_data_indicator_appears_when_offline() {
        let app = app_from(Err(DataError::Remote("offline".into())));
        assert!(app.subtitle().ends_with("[LOCAL]"));
        // The embedded snapshot is large, so rows are still present.
        assert!(app.total_count() > 200);
    }

    #[test]
    fn tick_flips_rows_as_deadlines_pass() {
        let mut app = app();
        let deadline = app
            .rows
            .iter()
            .find(|r| r.title == "CVPR")
            .and_then(|r| r.deadline)
            .expect("CVPR has a deadline");

        // Well before the deadline nothing changes.
        app.tick(deadline - Duration::days(30));
        let cvpr = app.rows.iter().find(|r| r.title == "CVPR").unwrap();
        assert!(!cvpr.is_expired);
        assert!(cvpr.is_running);

        // Just after it, the row flips to expired and is filtered away.
        app.tick(deadline + Duration::seconds(1));
        let cvpr = app.rows.iter().find(|r| r.title == "CVPR").unwrap();
        assert!(cvpr.is_expired);
        assert!(!cvpr.is_running);
        assert_eq!(cvpr.countdown, "Expired");
        assert!(!app.visible.iter().any(|r| r.title == "CVPR"));
    }

    #[test]
    fn tick_is_idempotent_when_nothing_expires() {
        let mut app = app();
        let before = app.rows.clone();
        app.tick(now());
        app.tick(now());
        assert_eq!(app.rows, before);
    }

    #[test]
    fn sidebar_items_cover_every_control() {
        let app = app();
        let items = app.sidebar_items();
        let count = |f: fn(&SidebarItem) -> bool| items.iter().filter(|i| f(i)).count();

        assert_eq!(count(|i| matches!(i, SidebarItem::Category(_))), CATEGORIES.len());
        assert_eq!(count(|i| matches!(i, SidebarItem::CcfRank(_))), CCF_RANKS.len());
        assert_eq!(count(|i| matches!(i, SidebarItem::CoreRank(_))), CORE_RANKS.len());
        assert_eq!(count(|i| matches!(i, SidebarItem::ThcplRank(_))), THCPL_RANKS.len());
        assert_eq!(count(|i| matches!(i, SidebarItem::ShowExpired)), 1);
        assert_eq!(count(|i| matches!(i, SidebarItem::Search)), 1);
        assert_eq!(count(|i| matches!(i, SidebarItem::Header(_))), 6);
        assert_eq!(count(SidebarItem::is_selectable), items.len() - 6);
    }

    #[test]
    fn category_items_expose_english_names() {
        let items = category_items();
        assert_eq!(items.len(), CATEGORIES.len());
        assert_eq!(items[0], ("DS", "Computer Architecture"));
        assert!(items.iter().any(|(sub, name)| *sub == "DB" && *name == "Database"));
    }
}