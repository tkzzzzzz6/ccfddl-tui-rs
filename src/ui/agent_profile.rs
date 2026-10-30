//! Opening, editing and saving the research-profile form.

use chrono::{Local, NaiveDate};
use ratatui::crossterm::event::KeyEvent;

use crate::agent::Lang;
use crate::agent::recommend::{Request, Tier, recommend};

use super::app::{App, Focus};
use super::profile_form::{Field, FormOutcome, ProfileForm};
use crate::agent::topics::{tag_name, tags_for_text};

/// Candidates counted by the form's preview.
const PREVIEW_TOP: usize = 8;

/// What the preview line shows for one draft.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub candidates: usize,
    /// The best pick of each tier.
    pub firsts: Vec<(Tier, String)>,
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

impl App {
    fn profile_lang(&self) -> Lang {
        self.language.into()
    }

    /// `/init`, `/profile`, `p`: edit the profile in the form.
    pub fn open_profile_form(&mut self) {
        let lang = self.profile_lang();
        if !self.agent.open {
            self.open_agent();
        }
        self.agent.ensure_profile(lang);
        self.agent.form = Some(ProfileForm::new(&self.agent.profile));
        self.agent.form_preview = None;
        self.focus = Focus::Agent;
    }

    pub fn cancel_profile_form(&mut self) {
        let lang = self.profile_lang();
        if self.agent.form.take().is_some() {
            self.agent
                .info(lang.pick("Profile unchanged.", "已取消，画像未修改。"));
        }
    }

    pub fn save_profile_form(&mut self) {
        let lang = self.profile_lang();
        let Some(form) = self.agent.form.as_mut() else {
            return;
        };
        let profile = match form.finish(today(), lang) {
            Ok(profile) => profile,
            Err(message) => {
                form.error = Some(message);
                return;
            }
        };
        let recommend_after = form.recommend_after_save;
        let changes = form.changes(today(), lang);
        if let Err(error) = profile.save(&self.agent.profile_path) {
            form.error = Some(match lang {
                Lang::Zh => format!("保存失败：{error}"),
                Lang::En => format!("Could not save: {error}"),
            });
            return;
        }
        self.agent.form = None;
        self.agent.profile = profile;
        self.agent.profile_exists = true;
        let mut text = match lang {
            Lang::Zh => format!("画像已保存到 {}", self.agent.profile_path.display()),
            Lang::En => format!("Profile saved to {}", self.agent.profile_path.display()),
        };
        if changes.is_empty() {
            text.push_str(lang.pick(" (no changes).", "（没有改动）。"));
        } else {
            text.push_str(lang.pick(":\n", "：\n"));
            text.push_str(&changes.join("\n"));
        }
        self.agent.info(text);
        if recommend_after {
            self.run_command("recommend");
        }
    }

    /// `s` on the cards, `/profile save`: write the follow-up conditions
    /// into the profile, after showing what would change.
    pub fn save_conditions_as_profile(&mut self) {
        let lang = self.profile_lang();
        self.agent.ensure_profile(lang);
        if self.agent.constraints.is_empty() {
            self.agent.info(lang.pick(
                "No conditions to save yet: describe your research or refine a recommendation first.",
                "还没有可保存的条件：先描述研究方向或追问推荐结果。",
            ));
            return;
        }
        let current = self.agent.profile.clone();
        let merged = self.agent.constraints.merge_into(&current, lang);
        self.open_profile_form();
        let mut form = ProfileForm::with_draft(&current, &merged);
        let changes = form.changes(today(), lang);
        form.notice = Some(if changes.is_empty() {
            lang.pick(
                "The profile already contains these conditions.",
                "画像已包含这些条件，无需修改。",
            )
            .to_string()
        } else {
            format!(
                "{}\n{}",
                lang.pick(
                    "From the conversation (Ctrl+S saves, Esc cancels):",
                    "将根据对话修改（Ctrl+S 保存，Esc 取消）：",
                ),
                changes.join("\n")
            )
        });
        self.agent.form = Some(form);
    }

    /// One line about the profile for the top of the panel.
    pub fn profile_status(&self) -> (String, bool) {
        let lang = self.profile_lang();
        let profile = &self.agent.profile;
        if !self.agent.profile_exists && profile.is_empty() {
            return (
                lang.pick(
                    "No research profile · p or click to set up",
                    "未设置研究画像 · 按 p 或点击设置",
                )
                .to_string(),
                false,
            );
        }
        let mut parts: Vec<String> = Vec::new();
        let topics: Vec<&str> = tags_for_text(&profile.interests.join(" , "))
            .iter()
            .map(|t| tag_name(t, lang.is_zh()))
            .take(3)
            .collect();
        if !topics.is_empty() {
            parts.push(topics.join(if lang.is_zh() { "、" } else { ", " }));
        } else if !profile.interests.is_empty() {
            parts.push(profile.interests.join(", "));
        }
        parts.push(format!("CCF {}", profile.target_ranks.join("/")));
        if let Some(date) = profile.ready_date {
            parts.push(match lang {
                Lang::Zh => format!("{date} 后"),
                Lang::En => format!("after {date}"),
            });
        }
        if !profile.preferred_places.is_empty() {
            parts.push(match lang {
                Lang::Zh => format!("偏好 {}", profile.preferred_places.len()),
                Lang::En => format!("{} preferred places", profile.preferred_places.len()),
            });
        }
        (
            format!("{}{}", lang.pick("Profile: ", "画像："), parts.join(" · ")),
            true,
        )
    }

    /// A key while the form is open.
    pub fn profile_form_key(&mut self, key: KeyEvent) {
        let Some(form) = self.agent.form.as_mut() else {
            return;
        };
        match form.handle_key(key) {
            FormOutcome::Continue => {}
            FormOutcome::Save => self.save_profile_form(),
            FormOutcome::Cancel => self.cancel_profile_form(),
        }
    }

    /// A click on a field (and optionally one of its choices).
    pub fn click_profile_form(&mut self, field: Field, option: Option<usize>) {
        let Some(form) = self.agent.form.as_mut() else {
            return;
        };
        form.focus(field);
        if let Some(option) = option {
            form.toggle(option);
        }
        self.focus = Focus::Agent;
    }

    /// Offline picks for the draft, recomputed only when it changes.
    pub fn profile_preview(&mut self) -> Option<Preview> {
        let form = self.agent.form.as_ref()?;
        let mut profile = form.preview_profile(today());
        if let Some((cached_for, preview)) = &self.agent.form_preview
            && *cached_for == profile
        {
            return Some(preview.clone());
        }
        // Tests and odd profiles: never preview beyond the scorer's range.
        profile.horizon_days = profile.horizon_days.max(1);
        let set = recommend(
            &Request {
                conferences: self.service.conferences(),
                favorites: self.service.favorites(),
                profile: &profile,
                now: chrono::Utc::now(),
                lang: self.profile_lang(),
            },
            PREVIEW_TOP,
        );
        let mut firsts: Vec<(Tier, String)> = Vec::new();
        for item in &set.items {
            if !firsts.iter().any(|(tier, _)| *tier == item.tier) {
                firsts.push((
                    item.tier,
                    format!("{} {}", item.facts.title, item.facts.year),
                ));
            }
        }
        let candidates = crate::agent::recommend::candidates(&Request {
            conferences: self.service.conferences(),
            favorites: self.service.favorites(),
            profile: &profile,
            now: chrono::Utc::now(),
            lang: self.profile_lang(),
        })
        .len();
        let preview = Preview { candidates, firsts };
        let key = form.preview_profile(today());
        self.agent.form_preview = Some((key, preview.clone()));
        Some(preview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Profile;
    use crate::ui::agent::Entry;
    use crate::ui::agent::tests::app_with;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    fn key(app: &mut App, code: KeyCode) {
        app.profile_form_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn init_opens_the_form_and_ctrl_s_saves() {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        app.agent.input.set("/init");
        app.submit_agent_input();
        assert!(app.agent.form.is_some());
        for c in "computer vision".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Down); // target ranks
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Char(' ')); // B off
        let preview = app.profile_preview().unwrap();
        assert!(preview.candidates >= 1);
        assert_eq!(preview.firsts[0].1, "CVPR 2099");

        app.profile_form_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.agent.form.is_none());
        let saved = Profile::load(&dir.path().join("profile.yaml"))
            .unwrap()
            .unwrap();
        assert_eq!(saved.interests, ["computer vision"]);
        assert_eq!(saved.target_ranks, ["A"]);
        assert_eq!(app.agent.profile, saved);
        assert!(matches!(
            app.agent.entries.last(),
            Some(Entry::Info(t)) if t.contains("Ranks: A, B → A")
        ));
    }

    #[test]
    fn a_bad_date_keeps_the_form_open() {
        let (mut app, dir) = app_with(None);
        app.open_profile_form();
        app.click_profile_form(Field::ReadyDate, None);
        for c in "someday".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        app.save_profile_form();
        let form = app.agent.form.as_ref().expect("still open");
        assert!(form.error.is_some());
        assert!(!dir.path().join("profile.yaml").exists());
    }

    #[test]
    fn escape_and_profile_command() {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        app.run_command("profile");
        assert!(app.agent.form.is_some());
        app.click_profile_form(Field::IncludeTbd, Some(0));
        assert!(!app.agent.form.as_ref().unwrap().draft.include_tbd);
        key(&mut app, KeyCode::Esc);
        assert!(app.agent.form.is_none());
        assert!(!dir.path().join("profile.yaml").exists());
        assert!(app.agent.profile.include_tbd);
        // Esc in the input reaches the form first, then closes the panel.
        app.open_profile_form();
        app.agent_escape();
        assert!(app.agent.form.is_none());
        assert!(app.agent.open);
    }

    fn wait(app: &mut App) {
        let start = std::time::Instant::now();
        while app.agent.is_busy() {
            app.poll_agent();
            assert!(start.elapsed().as_secs() < 10, "agent hung");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn conversation_conditions_become_the_profile_after_confirmation() {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        app.run_command("profile save");
        assert!(app.agent.form.is_none(), "nothing to save yet");

        app.agent.input.set("computer vision, 2099-01-31");
        app.submit_agent_input();
        wait(&mut app);
        app.agent.input.set("只要 A 类");
        app.submit_agent_input();
        wait(&mut app);

        app.focus = Focus::AgentCards;
        crate::ui::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        );
        let form = app.agent.form.as_ref().expect("form with the merged draft");
        let notice = form.notice.clone().unwrap();
        assert!(notice.contains("Ranks: A, B → A"), "{notice}");
        assert!(notice.contains("Ready: none → 2099-01-31"), "{notice}");
        assert!(
            !dir.path().join("profile.yaml").exists(),
            "not before Ctrl+S"
        );

        app.profile_form_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let saved = Profile::load(&dir.path().join("profile.yaml"))
            .unwrap()
            .unwrap();
        assert_eq!(saved.target_ranks, ["A"]);
        assert_eq!(
            saved.ready_date,
            chrono::NaiveDate::from_ymd_opt(2099, 1, 31)
        );
        assert!(!saved.interests.is_empty());
        let (status, has) = app.profile_status();
        assert!(has);
        assert!(status.contains("CCF A"), "{status}");
        assert!(status.contains("after 2099-01-31"), "{status}");
    }

    #[test]
    fn first_use_opens_the_form_then_recommends() {
        let (mut app, _dir) = app_with(None);
        app.agent.profile_exists = false;
        app.agent.profile = Profile {
            horizon_days: 100_000,
            ..Profile::default()
        };
        app.open_agent();
        let (status, has) = app.profile_status();
        assert!(!has);
        assert!(status.contains("No research profile"));

        app.run_quick_action(0);
        let form = app.agent.form.as_ref().expect("form first");
        assert!(form.recommend_after_save);
        assert!(form.notice.as_deref().unwrap().contains("No profile yet"));
        for c in "computer vision".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        app.profile_form_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.agent.profile_exists);
        wait(&mut app);
        assert!(
            app.agent.recommendations.is_some(),
            "recommended after saving"
        );

        // With a profile, shortcut 1 recommends straight away.
        app.agent.recommendations = None;
        app.run_quick_action(0);
        assert!(app.agent.form.is_none());
    }

    #[test]
    fn the_profile_line_is_drawn_and_clickable() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        let mut view = crate::ui::view::ViewState::new();
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|f| crate::ui::render::draw_with_view(f, &mut app, &mut view, chrono::Utc::now()))
            .unwrap();
        let line = view.layout.agent_profile_line.expect("profile line");
        let buffer = terminal.backend().buffer().clone();
        let text: String = (line.x..line.right())
            .map(|x| buffer[(x, line.y)].symbol().to_string())
            .collect();
        assert!(text.contains("Profile:"), "{text}");
        crate::ui::input::handle_mouse(
            &mut app,
            &mut view,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: line.x + 1,
                row: line.y,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert!(app.agent.form.is_some());

        // The form is drawn and its choices can be clicked.
        terminal
            .draw(|f| crate::ui::render::draw_with_view(f, &mut app, &mut view, chrono::Utc::now()))
            .unwrap();
        let &(area, field, _) = view
            .layout
            .agent_form
            .iter()
            .find(|(_, f, o)| *f == Field::TargetRanks && *o == Some(2))
            .expect("rank C is clickable");
        assert_eq!(field, Field::TargetRanks);
        crate::ui::input::handle_mouse(
            &mut app,
            &mut view,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(
            app.agent.form.as_ref().unwrap().draft.target_ranks,
            ["A", "B", "C"]
        );
    }
}
