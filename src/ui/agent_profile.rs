//! Opening, editing and saving the research-profile form.

use chrono::{Local, NaiveDate};
use ratatui::crossterm::event::KeyEvent;

use crate::agent::Lang;
use crate::agent::recommend::{Request, Tier, recommend};

use super::app::{App, Focus};
use super::profile_form::{Field, FormOutcome, ProfileForm};

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
}
