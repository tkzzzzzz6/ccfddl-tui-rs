//! Working with recommendation results: earlier rounds, follow-up questions
//! about a card, asking about a table row, and the panel size.

use crate::agent::{Lang, RecommendationSet, Tier};

use super::agent::AgentPanel;
use super::app::{App, Focus};

/// Narrowest agent panel when it sits beside the table.
pub const PANEL_MIN_WIDTH: u16 = 30;
/// Columns moved by one `<` / `>` press.
pub const PANEL_STEP: i32 = 4;

/// Recommendation rounds kept for `[` / `]`.
pub const ROUNDS_KEPT: usize = 5;

/// How the recommendation cards are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardsView {
    /// Two lines per card, details of the selected one with `d`.
    #[default]
    Expanded,
    /// One line per card, two columns on wide panels.
    Compact,
    /// A single summary line.
    Collapsed,
}

impl CardsView {
    pub fn key(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Compact => "compact",
            Self::Collapsed => "collapsed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [Self::Expanded, Self::Compact, Self::Collapsed]
            .into_iter()
            .find(|v| v.key() == key)
    }

    pub fn next(self) -> Self {
        match self {
            Self::Expanded => Self::Compact,
            Self::Compact => Self::Collapsed,
            Self::Collapsed => Self::Expanded,
        }
    }
}

/// How much room the agent panel takes while it has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelSize {
    /// Beside the table on wide terminals, full width on narrow ones.
    #[default]
    Half,
    /// The whole body, even on wide terminals.
    Full,
}

impl AgentPanel {
    /// Show a new set and remember it, keeping the last [`ROUNDS_KEPT`].
    pub fn push_recommendations(&mut self, set: RecommendationSet) {
        self.rounds.push(set.clone());
        if self.rounds.len() > ROUNDS_KEPT {
            self.rounds.remove(0);
        }
        self.round_cursor = self.rounds.len() - 1;
        self.recommendations = Some(set);
        self.card_cursor = 0;
        self.turn_cards = true;
        // A new round is meant to be seen: never arrive collapsed.
        self.cards_view = match self.cards_view_pref {
            CardsView::Collapsed => CardsView::Compact,
            view => view,
        };
    }
}

impl App {
    fn results_lang(&self) -> Lang {
        self.language.into()
    }

    /// Tier of a conference in the round on screen.
    pub fn recommended_tier(&self, title: &str, year: i32) -> Option<Tier> {
        self.agent.recommendations.as_ref().and_then(|set| {
            set.items
                .iter()
                .find(|item| item.facts.title == title && item.facts.year == year)
                .map(|item| item.tier)
        })
    }

    /// Re-filter after the round on screen changed, keeping the selection.
    pub fn recommendations_changed(&mut self) {
        if self.only_recommended {
            let selected = self.selected_row().map(|row| (row.title.clone(), row.year));
            self.refresh_visible();
            if let Some((title, year)) = selected {
                self.select_conference(&title, year);
            }
        }
    }

    /// `v` / `/only`: show only the recommended conferences in the table.
    pub fn toggle_only_recommended(&mut self) {
        let lang = self.results_lang();
        let has_cards = self
            .agent
            .recommendations
            .as_ref()
            .is_some_and(|set| !set.items.is_empty());
        if !self.only_recommended && !has_cards {
            self.status = Some(
                lang.pick(
                    "No recommendations yet: press a and ask the agent first.",
                    "还没有推荐结果：先按 a 让助手推荐。",
                )
                .to_string(),
            );
            return;
        }
        let selected = self.selected_row().map(|row| (row.title.clone(), row.year));
        self.only_recommended = !self.only_recommended;
        self.refresh_visible();
        match selected {
            Some((title, year))
                if self.recommended_tier(&title, year).is_some() || !self.only_recommended =>
            {
                self.select_conference(&title, year)
            }
            _ => self.selected = 0,
        }
    }

    /// Ctrl+O, `z`, a click on the cards' title: expanded → compact →
    /// collapsed. The choice is remembered.
    pub fn cycle_cards_view(&mut self) {
        let next = self.agent.cards_view.next();
        self.agent.cards_view = next;
        self.agent.cards_view_pref = next;
        if next == CardsView::Collapsed && self.focus == Focus::AgentCards {
            self.focus = Focus::Agent;
        }
        self.agent.save_session();
    }

    /// Move to the cards, unfolding them if they are collapsed.
    pub fn focus_cards(&mut self) {
        if !self.has_cards() {
            return;
        }
        if self.agent.cards_view == CardsView::Collapsed {
            self.agent.cards_view = CardsView::Compact;
        }
        self.focus = Focus::AgentCards;
    }

    /// `[` / `]`: show an earlier or later round of recommendations.
    pub fn show_round(&mut self, delta: isize) {
        let count = self.agent.rounds.len();
        if count < 2 {
            return;
        }
        let next = (self.agent.round_cursor as isize + delta).clamp(0, count as isize - 1) as usize;
        if next == self.agent.round_cursor {
            return;
        }
        self.agent.round_cursor = next;
        self.agent.recommendations = Some(self.agent.rounds[next].clone());
        self.agent.card_cursor = 0;
        self.recommendations_changed();
    }

    /// `?` on a card: go to the input with a question about it.
    pub fn ask_about_card(&mut self) {
        let lang = self.results_lang();
        let Some(card) = self.agent.selected_card() else {
            return;
        };
        let question = match lang {
            Lang::Zh => format!("为什么推荐 {} {}？", card.facts.title, card.facts.year),
            Lang::En => format!("Why {} {}?", card.facts.title, card.facts.year),
        };
        self.focus_agent_input();
        self.agent.input.set(&question);
        self.agent.input_edited();
    }

    /// `A` in the table: open the agent with a question about the selected
    /// conference, ready to edit and send.
    pub fn ask_about_selected_row(&mut self) {
        let lang = self.results_lang();
        let Some((title, year)) = self.selected_row().map(|row| (row.title.clone(), row.year))
        else {
            return;
        };
        self.open_agent();
        let question = match lang {
            Lang::Zh => format!("评估 {title} {year} 是否适合我的研究"),
            Lang::En => format!("Evaluate {title} {year} for my research"),
        };
        self.agent.input.set(&question);
        self.agent.input_edited();
    }

    /// Set the side-by-side panel width, within the bounds of the last frame.
    pub fn set_panel_width(&mut self, width: u16) {
        let (min, max) = self
            .agent
            .panel_bounds
            .unwrap_or((PANEL_MIN_WIDTH, u16::MAX));
        self.agent.panel_width = Some(width.clamp(min, max.max(min)));
    }

    /// `<` / `>` on the cards: a wider or narrower panel.
    pub fn resize_panel(&mut self, delta: i32) {
        let Some((min, max)) = self.agent.panel_bounds else {
            return;
        };
        let current = self
            .agent
            .panel_width
            .unwrap_or(self.agent.panel_shown_width) as i32;
        let current = current.clamp(min as i32, max.max(min) as i32);
        self.set_panel_width((current + delta).max(0) as u16);
    }

    /// `y` on the cards: copy the selected card as Markdown.
    pub fn copy_card(&mut self) {
        let lang = self.results_lang();
        let Some(card) = self.agent.selected_card() else {
            return;
        };
        let text = crate::agent::report::card_markdown(card, chrono::Utc::now(), lang);
        let title = format!("{} {}", card.facts.title, card.facts.year);
        let message = match ((self.agent.clipboard)(&text), lang) {
            (Ok(()), Lang::Zh) => format!(
                "已复制 {title}（Markdown，经 OSC 52 写入剪贴板；若粘贴为空，说明终端不支持，可用 /export）。"
            ),
            (Ok(()), Lang::En) => format!(
                "Copied {title} as Markdown (via OSC 52; if nothing pastes, the terminal does not support it, use /export)."
            ),
            (Err(error), Lang::Zh) => format!("复制失败：{error}"),
            (Err(error), Lang::En) => format!("Copy failed: {error}"),
        };
        self.agent.info(message);
    }

    /// F2 / Ctrl+\: half → full → hidden. Hidden keeps the conversation;
    /// `a` brings the panel back at half size.
    pub fn cycle_panel_size(&mut self) {
        let lang = self.results_lang();
        self.agent.panel_width = None;
        match self.agent.panel_size {
            PanelSize::Half => {
                self.agent.panel_size = PanelSize::Full;
                self.status = Some(
                    lang.pick("Agent: full screen", "推荐助手：全屏")
                        .to_string(),
                );
            }
            PanelSize::Full => {
                self.agent.panel_size = PanelSize::Half;
                self.agent.open = false;
                self.focus = Focus::Table;
                self.status = Some(
                    lang.pick(
                        "Agent hidden; press a to bring it back",
                        "推荐助手已隐藏，按 a 恢复",
                    )
                    .to_string(),
                );
            }
        }
    }

    /// Title for the cards header: `round 2/3 · 14:05` when several rounds
    /// are kept.
    pub fn round_label(&self) -> Option<String> {
        let count = self.agent.rounds.len();
        if count < 2 {
            return None;
        }
        let set = self.agent.rounds.get(self.agent.round_cursor)?;
        let time = set
            .generated_at
            .with_timezone(&chrono::Local)
            .format("%H:%M");
        Some(match self.results_lang() {
            Lang::Zh => format!("第 {}/{count} 轮 · {time}", self.agent.round_cursor + 1),
            Lang::En => format!("round {}/{count} · {time}", self.agent.round_cursor + 1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::recommend::Source;
    use crate::ui::agent::tests::app_with;
    use chrono::Utc;

    fn set(summary: &str, from: &App) -> RecommendationSet {
        let mut set = from
            .agent
            .recommendations
            .clone()
            .expect("a first set to copy");
        set.summary = summary.to_string();
        set.generated_at = Utc::now();
        set.source = Source::Rules;
        set
    }

    fn app_with_cards() -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        app.agent.profile.horizon_days = 100_000;
        app.agent.input.set("computer vision database");
        app.submit_agent_input();
        while app.agent.is_busy() {
            app.poll_agent();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(app.agent.rounds.len(), 1);
        (app, dir)
    }

    #[test]
    fn rounds_are_kept_and_browsed() {
        let (mut app, _dir) = app_with_cards();
        assert_eq!(app.round_label(), None);
        for i in 0..ROUNDS_KEPT + 1 {
            let next = set(&format!("round {i}"), &app);
            app.agent.push_recommendations(next);
        }
        assert_eq!(app.agent.rounds.len(), ROUNDS_KEPT);
        assert_eq!(app.agent.round_cursor, ROUNDS_KEPT - 1);
        assert!(app.round_label().unwrap().starts_with("round 5/5"));

        app.agent.card_cursor = 1;
        app.show_round(-1);
        assert_eq!(
            app.agent.recommendations.as_ref().unwrap().summary,
            "round 4"
        );
        assert_eq!(app.agent.card_cursor, 0);
        app.show_round(-10);
        assert_eq!(
            app.agent.recommendations.as_ref().unwrap().summary,
            "round 1"
        );
        app.show_round(1);
        assert!(app.round_label().unwrap().starts_with("round 2/5"));
    }

    #[test]
    fn question_mark_prefills_a_follow_up() {
        let (mut app, _dir) = app_with_cards();
        app.focus = Focus::AgentCards;
        let title = app.agent.selected_card().unwrap().facts.title.clone();
        app.ask_about_card();
        assert_eq!(app.focus, Focus::Agent);
        assert_eq!(app.agent.input, format!("Why {title} 2099?").as_str());
        // Offline, sending it is answered locally from the rules.
        let cards_before = app.agent.rounds.len();
        app.submit_agent_input();
        assert!(!app.agent.is_busy());
        let answer = match app.agent.entries.last() {
            Some(crate::ui::agent::Entry::Assistant(text)) => text.clone(),
            other => panic!("{other:?}"),
        };
        assert!(answer.contains(&title), "{answer}");
        assert!(answer.contains("Why:"), "{answer}");
        assert_eq!(app.agent.rounds.len(), cards_before, "no new cards");
    }

    #[test]
    fn table_rows_can_be_asked_about() {
        let (mut app, _dir) = app_with(None);
        app.select_conference("ICDE", 2099);
        app.ask_about_selected_row();
        assert!(app.agent.open);
        assert_eq!(app.focus, Focus::Agent);
        assert_eq!(app.agent.input, "Evaluate ICDE 2099 for my research");
    }

    #[test]
    fn panel_size_cycles_half_full_hidden() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        app.cycle_panel_size();
        assert_eq!(app.agent.panel_size, PanelSize::Full);
        app.cycle_panel_size();
        assert!(!app.agent.open);
        assert_eq!(app.focus, Focus::Table);
        assert_eq!(app.agent.panel_size, PanelSize::Half);
        // The conversation survives hiding.
        assert!(!app.agent.entries.is_empty());
    }

    #[test]
    fn table_can_show_only_the_recommendations() {
        let (mut app, _dir) = app_with_cards();
        // The fixture has two conferences; keep one in the round.
        let mut one = set("one", &app);
        one.items.truncate(1);
        let kept = one.items[0].facts.title.clone();
        app.agent.push_recommendations(one);
        assert_eq!(app.visible.len(), 2);
        assert!(app.recommended_tier(&kept, 2099).is_some());

        app.toggle_only_recommended();
        assert!(app.only_recommended);
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.visible[0].title, kept);
        assert!(app.subtitle().contains("recommendations only"));

        // Browsing to the earlier round (both conferences) follows along.
        app.show_round(-1);
        assert_eq!(app.visible.len(), 2);
        app.show_round(1);
        assert_eq!(app.visible.len(), 1);

        // v again, or Esc in the table, shows everything.
        app.toggle_only_recommended();
        assert_eq!(app.visible.len(), 2);
        app.toggle_only_recommended();
        app.reset_filters();
        assert!(!app.only_recommended);
        assert_eq!(app.visible.len(), 2);
    }

    #[test]
    fn only_recommended_needs_cards() {
        let (mut app, _dir) = app_with(None);
        app.toggle_only_recommended();
        assert!(!app.only_recommended);
        assert!(
            app.status
                .as_deref()
                .unwrap()
                .contains("No recommendations")
        );
    }

    #[test]
    fn recommended_rows_are_marked_in_the_table() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (mut app, _dir) = app_with_cards();
        app.focus = Focus::Table;
        let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
        terminal
            .draw(|frame| crate::ui::render::draw(frame, &mut app, Utc::now()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: String = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol().to_string())
            .collect();
        assert!(text.contains("● CVPR 2099"), "{text}");
    }

    fn render(app: &mut App, view: &mut crate::ui::view::ViewState, width: u16) {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
        terminal
            .draw(|frame| crate::ui::render::draw_with_view(frame, app, view, Utc::now()))
            .unwrap();
    }

    #[test]
    fn the_panel_divider_can_be_dragged_and_keys_resize_it() {
        use crate::ui::input::handle_mouse;
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let (mut app, _dir) = app_with_cards();
        let mut view = crate::ui::view::ViewState::new();
        render(&mut app, &mut view, 160);
        let divider = view.layout.agent_divider.expect("side-by-side panel");
        let before = view.layout.agent.unwrap().width;
        assert_eq!(app.agent.panel_shown_width, before);

        let y = divider.y + 3;
        handle_mouse(
            &mut app,
            &mut view,
            mouse(MouseEventKind::Down(MouseButton::Left), divider.x, y),
        );
        handle_mouse(
            &mut app,
            &mut view,
            mouse(MouseEventKind::Drag(MouseButton::Left), divider.x - 10, y),
        );
        handle_mouse(
            &mut app,
            &mut view,
            mouse(MouseEventKind::Up(MouseButton::Left), divider.x - 10, y),
        );
        assert_eq!(app.agent.panel_width, Some(before + 10));
        render(&mut app, &mut view, 160);
        assert_eq!(view.layout.agent.unwrap().width, before + 10);

        // Bounds: the table keeps 60 columns, the panel at least 30.
        app.set_panel_width(1000);
        render(&mut app, &mut view, 160);
        assert_eq!(view.layout.agent.unwrap().width, 160 - 60);
        app.set_panel_width(0);
        assert_eq!(app.agent.panel_width, Some(PANEL_MIN_WIDTH));

        app.agent.panel_width = None;
        render(&mut app, &mut view, 160);
        let shown = app.agent.panel_shown_width;
        app.resize_panel(PANEL_STEP);
        assert_eq!(app.agent.panel_width, Some(shown + PANEL_STEP as u16));
        app.resize_panel(-PANEL_STEP);
        assert_eq!(app.agent.panel_width, Some(shown));
        // F2 starts over from the default width.
        app.cycle_panel_size();
        assert_eq!(app.agent.panel_width, None);
    }

    #[test]
    fn y_copies_the_selected_card_as_markdown() {
        use std::sync::{Arc, Mutex};
        let (mut app, _dir) = app_with_cards();
        let copied = Arc::new(Mutex::new(String::new()));
        let sink = copied.clone();
        app.agent.clipboard = Arc::new(move |text: &str| {
            *sink.lock().unwrap() = text.to_string();
            Ok(())
        });
        app.copy_card();
        let text = copied.lock().unwrap().clone();
        let card = app.agent.selected_card().unwrap();
        assert!(text.starts_with(&format!("### {} {}", card.facts.title, card.facts.year)));
        assert!(text.contains("- Deadline: "));
        assert!(text.contains("- Why: "));
        let last = |app: &App| match app.agent.entries.last() {
            Some(crate::ui::agent::Entry::Info(text)) => text.clone(),
            other => panic!("{other:?}"),
        };
        assert!(last(&app).contains("Copied"));

        app.agent.clipboard = Arc::new(|_: &str| Err(std::io::Error::other("no tty")));
        app.copy_card();
        assert!(last(&app).contains("no tty"));
    }
}
