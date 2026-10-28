//! First-screen shortcuts of the agent panel: four common starting points,
//! triggered with the number keys 1-4 or a click.

use chrono::Utc;

use crate::agent::Lang;
use crate::agent::profile::Profile;
use crate::agent::recommend::{self, RecommendationSet, Request, Source};

use super::agent::Entry;
use super::app::App;

/// Days ahead for the "upcoming CCF-A deadlines" shortcut.
pub const UPCOMING_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickAction {
    Recommend,
    UpcomingA,
    Profile,
    Model,
}

impl QuickAction {
    pub const ALL: [QuickAction; 4] = [
        QuickAction::Recommend,
        QuickAction::UpcomingA,
        QuickAction::Profile,
        QuickAction::Model,
    ];

    pub fn label(self, app: &App) -> String {
        let lang: Lang = app.language.into();
        let has_profile = !app.agent.profile.is_empty();
        match self {
            Self::Recommend => lang
                .pick("Recommend for my profile", "按我的画像推荐")
                .to_string(),
            Self::UpcomingA => match lang {
                Lang::Zh => format!("未来 {UPCOMING_DAYS} 天截稿的 CCF-A 会议"),
                Lang::En => format!("CCF-A deadlines in the next {UPCOMING_DAYS} days"),
            },
            Self::Profile if has_profile => lang
                .pick("Edit my research profile", "修改研究画像")
                .to_string(),
            Self::Profile => lang
                .pick("Set up my research profile", "设置研究画像")
                .to_string(),
            Self::Model if app.agent.llm.is_some() => {
                lang.pick("Switch model", "切换模型").to_string()
            }
            Self::Model => lang.pick("Connect a model", "配置模型").to_string(),
        }
    }

    /// The command it runs, shown next to the label.
    pub fn hint(self, app: &App) -> &'static str {
        match self {
            Self::Recommend => "/recommend",
            Self::UpcomingA => "",
            Self::Profile => "/init",
            Self::Model if app.agent.llm.is_some() => "/model",
            Self::Model => "/setup",
        }
    }
}

impl App {
    /// Shortcuts show until the first request, with an empty input and no
    /// questionnaire running.
    pub fn quick_actions_visible(&self) -> bool {
        self.agent.open
            && self.agent.input.is_empty()
            && self.agent.setup.is_none()
            && self.agent.wizard.is_none()
            && !self.agent.is_busy()
            && !self
                .agent
                .entries
                .iter()
                .any(|entry| matches!(entry, Entry::User(_)))
    }

    /// Run shortcut `index` (0-based). Returns `false` if there is none.
    pub fn run_quick_action(&mut self, index: usize) -> bool {
        let Some(action) = QuickAction::ALL.get(index).copied() else {
            return false;
        };
        match action {
            QuickAction::Recommend => self.run_command("recommend"),
            QuickAction::UpcomingA => self.show_upcoming_a(),
            QuickAction::Profile => self.run_command("init"),
            QuickAction::Model if self.agent.llm.is_some() => self.run_command("model"),
            QuickAction::Model => self.run_command("setup"),
        }
        true
    }

    /// Open CCF-A deadlines in the next [`UPCOMING_DAYS`] days, as cards,
    /// computed locally so it also works offline.
    pub fn show_upcoming_a(&mut self) {
        let lang: Lang = self.language.into();
        let label = QuickAction::UpcomingA.label(self);
        self.agent.entries.push(Entry::User(label));
        let profile = Profile {
            target_ranks: self.agent.profile.target_ranks.clone(),
            horizon_days: UPCOMING_DAYS,
            min_days_left: 0,
            include_tbd: false,
            ..Profile::default()
        };
        let now = Utc::now();
        let mut items: Vec<_> = recommend::candidates(&Request {
            conferences: self.service.conferences(),
            favorites: self.service.favorites(),
            profile: &profile,
            now,
            lang,
        })
        .into_iter()
        .filter(|candidate| candidate.facts.ccf == "A")
        .map(|candidate| candidate.into_recommendation(lang))
        .collect();
        items.sort_by_key(|item| item.facts.deadline);
        let summary = match (items.len(), lang) {
            (0, Lang::Zh) => format!("未来 {UPCOMING_DAYS} 天内没有 CCF-A 会议截稿。"),
            (0, Lang::En) => format!("No CCF-A deadline in the next {UPCOMING_DAYS} days."),
            (n, Lang::Zh) => {
                format!("未来 {UPCOMING_DAYS} 天内有 {n} 个 CCF-A 会议截稿，按截稿时间排序。")
            }
            (n, Lang::En) => {
                format!("{n} CCF-A deadlines in the next {UPCOMING_DAYS} days, soonest first.")
            }
        };
        self.agent.entries.push(Entry::Assistant(summary.clone()));
        self.agent.scroll = 0;
        if items.is_empty() {
            return;
        }
        self.agent.push_recommendations(RecommendationSet {
            summary,
            conditions: Vec::new(),
            items,
            source: Source::Rules,
            generated_at: now,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::agent::tests::app_with;

    #[test]
    fn shortcuts_show_until_the_first_request() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        assert!(app.quick_actions_visible());
        app.agent.input.set("x");
        assert!(!app.quick_actions_visible());
        app.agent.input.clear();
        assert!(app.run_quick_action(2)); // /init
        assert!(!app.quick_actions_visible());
        assert!(!app.run_quick_action(9));
    }

    #[test]
    fn labels_follow_the_state() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        assert_eq!(QuickAction::Model.label(&app), "Connect a model");
        assert_eq!(QuickAction::Model.hint(&app), "/setup");
        app.agent.profile.interests = vec!["vision".into()];
        assert_eq!(QuickAction::Profile.label(&app), "Edit my research profile");
        app.toggle_language();
        assert!(QuickAction::UpcomingA.label(&app).contains("CCF-A"));
    }

    #[test]
    fn upcoming_lists_only_open_ccf_a_deadlines() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        app.show_upcoming_a();
        // Fixture deadlines are in 2099: nothing within 30 days.
        assert!(app.agent.recommendations.is_none());
        assert!(
            matches!(app.agent.entries.last(), Some(Entry::Assistant(t)) if t.contains("No CCF-A"))
        );
    }

    #[test]
    fn upcoming_sorts_ccf_a_by_deadline() {
        use crate::data::{DataService, Favorites};
        use crate::ui::agent::AgentPanel;

        let day = |n: i64| {
            (Utc::now() + chrono::Duration::days(n))
                .format("%Y-%m-%d 23:59:59")
                .to_string()
        };
        let yaml = format!(
            "
- {{title: LATE, description: Late, sub: AI, rank: {{ccf: A}}, confs: [{{year: 2099, timezone: UTC, timeline: [{{deadline: '{}'}}]}}]}}
- {{title: SOON, description: Soon, sub: DB, rank: {{ccf: A}}, confs: [{{year: 2099, timezone: UTC, timeline: [{{deadline: '{}'}}]}}]}}
- {{title: BRANK, description: B, sub: AI, rank: {{ccf: B}}, confs: [{{year: 2099, timezone: UTC, timeline: [{{deadline: '{}'}}]}}]}}
- {{title: FAR, description: Far, sub: AI, rank: {{ccf: A}}, confs: [{{year: 2099, timezone: UTC, timeline: [{{deadline: '{}'}}]}}]}}
",
            day(20),
            day(5),
            day(3),
            day(60)
        );
        let dir = tempfile::tempdir().unwrap();
        let mut service = DataService::with_paths(
            None,
            dir.path().join("cache.yml"),
            Favorites::empty_at(dir.path().join("f.json")),
        );
        service.load_conferences_with(move || Ok(yaml)).unwrap();
        let mut app = App::with_agent(
            service,
            AgentPanel::new(dir.path().join("profile.yaml"), None),
        );
        app.open_agent();
        app.show_upcoming_a();
        let titles: Vec<String> = app
            .agent
            .recommendations
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|item| item.facts.title.clone())
            .collect();
        assert_eq!(titles, ["SOON", "LATE"]);
    }
}
