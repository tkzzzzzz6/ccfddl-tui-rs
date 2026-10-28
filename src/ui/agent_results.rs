//! Working with recommendation results: earlier rounds, follow-up questions
//! about a card, asking about a table row, and the panel size.

use crate::agent::{Lang, RecommendationSet};

use super::agent::AgentPanel;
use super::app::{App, Focus};

/// Recommendation rounds kept for `[` / `]`.
pub const ROUNDS_KEPT: usize = 5;

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
    }
}

impl App {
    fn results_lang(&self) -> Lang {
        self.language.into()
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

    /// F2 / Ctrl+\: half → full → hidden. Hidden keeps the conversation;
    /// `a` brings the panel back at half size.
    pub fn cycle_panel_size(&mut self) {
        let lang = self.results_lang();
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
}
