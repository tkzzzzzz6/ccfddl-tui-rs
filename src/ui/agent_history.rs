//! Opening the `/history` list and carrying out what it asks for.

use chrono::Utc;
use ratatui::crossterm::event::KeyEvent;

use crate::agent::Lang;
use crate::agent::report::export_markdown;

use super::app::{App, Focus};
use super::history_list::{HistoryList, ListAction, compare};

impl App {
    fn history_lang(&self) -> Lang {
        self.language.into()
    }

    /// `/history`, `h` on the cards: the list of saved rounds.
    pub fn open_history(&mut self) {
        let lang = self.history_lang();
        if !self.agent.open {
            self.open_agent();
        }
        if self.agent.round_history.rounds.is_empty() {
            self.agent
                .info(lang.pick("No saved recommendations yet.", "还没有保存的推荐。"));
            return;
        }
        let current = self.current_round_id();
        self.agent.history_list = Some(HistoryList::at(&self.agent.round_history, current));
        self.agent.form = None;
        self.focus = Focus::Agent;
    }

    /// The history id of the round on screen.
    pub fn current_round_id(&self) -> Option<u64> {
        self.agent.recommendations.as_ref()?;
        self.agent
            .round_history
            .rounds
            .get(self.agent.round_cursor)
            .map(|r| r.id)
    }

    pub fn history_list_key(&mut self, key: KeyEvent) {
        let Some(list) = self.agent.history_list.as_mut() else {
            return;
        };
        let action = list.handle_key(&self.agent.round_history, key);
        self.history_action(action);
    }

    /// A click on row `position` of the list: select, or open if selected.
    pub fn click_history_row(&mut self, position: usize) {
        let Some(list) = self.agent.history_list.as_mut() else {
            return;
        };
        if list.cursor == position && list.renaming.is_none() {
            if let Some(id) = list.selected(&self.agent.round_history) {
                self.history_action(ListAction::Open(id));
            }
        } else {
            list.cursor = position;
            list.confirm_delete = None;
            list.comparison = None;
            list.clamp(&self.agent.round_history);
        }
        self.focus = Focus::Agent;
    }

    fn history_action(&mut self, action: ListAction) {
        let lang = self.history_lang();
        match action {
            ListAction::None => {}
            ListAction::Close => self.agent.history_list = None,
            ListAction::Open(id) => {
                self.agent.history_list = None;
                self.open_round(id);
                if let Some(round) = self.agent.round_history.get(id) {
                    let closed = round.closed(Utc::now());
                    let mut text = match lang {
                        Lang::Zh => {
                            format!("已打开「{}」（{} 个推荐", round.name, round.set.items.len())
                        }
                        Lang::En => {
                            format!("Opened \"{}\" ({} picks", round.name, round.set.items.len())
                        }
                    };
                    if closed > 0 {
                        text.push_str(&match lang {
                            Lang::Zh => format!("，{closed} 个已截止"),
                            Lang::En => format!(", {closed} closed"),
                        });
                    }
                    text.push_str(lang.pick(").", "）。"));
                    self.agent.info(text);
                }
            }
            ListAction::TogglePin(id) => {
                self.agent.round_history.toggle_pin(id);
                self.agent.save_history();
            }
            ListAction::Rename(id, name) => {
                if self.agent.round_history.rename(id, &name) {
                    self.agent.save_history();
                }
            }
            ListAction::Delete(id) => {
                self.agent.round_history.remove(id);
                self.agent.save_history();
                self.after_history_change();
                if self.agent.round_history.rounds.is_empty() {
                    self.agent.history_list = None;
                } else if let Some(list) = self.agent.history_list.as_mut() {
                    list.clamp(&self.agent.round_history);
                }
            }
            ListAction::Export(id) => {
                let Some(round) = self.agent.round_history.get(id) else {
                    return;
                };
                let set = round.set.clone();
                let dir = self
                    .agent
                    .profile_path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default();
                match export_markdown(&set, &dir, Utc::now(), lang) {
                    Ok(path) => {
                        self.status = Some(match lang {
                            Lang::Zh => format!("已导出到 {}", path.display()),
                            Lang::En => format!("Exported to {}", path.display()),
                        })
                    }
                    Err(error) => self.status = Some(error.to_string()),
                }
            }
            ListAction::Compare(id) => {
                let (Some(round), Some(current)) = (
                    self.agent.round_history.get(id),
                    self.agent.recommendations.as_ref(),
                ) else {
                    return;
                };
                let comparison = compare(&round.set, current);
                if let Some(list) = self.agent.history_list.as_mut() {
                    list.comparison = Some((id, comparison));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::agent::tests::app_with;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::layout::Rect;

    fn press(app: &mut App, code: KeyCode) {
        app.history_list_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn app_with_rounds() -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        app.agent.profile.horizon_days = 100_000;
        for (i, text) in ["computer vision", "databases"].iter().enumerate() {
            app.agent.input.set(text);
            app.submit_agent_input();
            while app.agent.is_busy() {
                app.poll_agent();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // Distinct times, so the order is well defined.
            let last = app.agent.round_history.rounds.len() - 1;
            app.agent.round_history.rounds[last].set.generated_at +=
                chrono::Duration::seconds(i as i64);
        }
        (app, dir)
    }

    #[test]
    fn history_opens_on_the_round_on_screen_and_opens_others() {
        let (mut app, _dir) = app_with_rounds();
        app.run_command("history");
        let list = app.agent.history_list.as_ref().expect("list");
        let current = app.current_round_id();
        assert_eq!(list.selected(&app.agent.round_history), current);

        press(&mut app, KeyCode::Down); // the older round
        press(&mut app, KeyCode::Enter);
        assert!(app.agent.history_list.is_none());
        assert_eq!(app.agent.round_cursor, 0);
        assert_eq!(app.agent.round_history.rounds[0].name, "computer vision");
        assert!(matches!(
            app.agent.entries.last(),
            Some(crate::ui::agent::Entry::Info(t)) if t.contains("Opened \"computer vision\"")
        ));
    }

    #[test]
    fn pin_rename_delete_export_and_compare() {
        let (mut app, dir) = app_with_rounds();
        app.focus = Focus::AgentCards;
        crate::ui::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
        );
        assert!(app.agent.history_list.is_some(), "h on the cards");

        press(&mut app, KeyCode::Char('p'));
        let newest = app.agent.round_history.rounds[1].id;
        assert!(app.agent.round_history.get(newest).unwrap().pinned);

        press(&mut app, KeyCode::Char('r'));
        app.history_list_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "DB".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.agent.round_history.get(newest).unwrap().name, "DB");
        let saved = std::fs::read_to_string(dir.path().join("agent-history.json")).unwrap();
        assert!(saved.contains("\"DB\""));

        press(&mut app, KeyCode::Char('e'));
        assert!(app.status.as_deref().unwrap().contains("Exported"));

        // Compare the older round with the one on screen.
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('c'));
        assert!(
            app.agent
                .history_list
                .as_ref()
                .unwrap()
                .comparison
                .is_some()
        );
        press(&mut app, KeyCode::Esc);
        assert!(
            app.agent
                .history_list
                .as_ref()
                .unwrap()
                .comparison
                .is_none()
        );

        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.agent.round_history.rounds.len(), 2);
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.agent.round_history.rounds.len(), 1);
        press(&mut app, KeyCode::Esc);
        assert!(app.agent.history_list.is_none());
    }

    #[test]
    fn an_empty_history_says_so() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        app.run_command("history");
        assert!(app.agent.history_list.is_none());
        assert!(matches!(
            app.agent.entries.last(),
            Some(crate::ui::agent::Entry::Info(t)) if t.contains("No saved")
        ));
    }

    #[test]
    fn rows_are_drawn_and_clickable() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let (mut app, _dir) = app_with_rounds();
        app.open_history();
        let mut view = crate::ui::view::ViewState::new();
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        let mut draw = |app: &mut App, view: &mut crate::ui::view::ViewState| {
            terminal
                .draw(|f| crate::ui::render::draw_with_view(f, app, view, Utc::now()))
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let text = draw(&mut app, &mut view);
        assert!(text.contains("Saved recommendations (2)"), "{text}");
        assert!(text.contains("databases"), "{text}");
        assert!(text.contains("Preview:"), "{text}");
        assert_eq!(view.layout.agent_history_rows.len(), 2);

        let click = |app: &mut App, view: &mut crate::ui::view::ViewState, area: Rect| {
            crate::ui::input::handle_mouse(
                app,
                view,
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x + 3,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                },
            );
        };
        let (second, position) = view.layout.agent_history_rows[1];
        click(&mut app, &mut view, second);
        assert_eq!(app.agent.history_list.as_ref().unwrap().cursor, position);
        draw(&mut app, &mut view);
        let (second, _) = view.layout.agent_history_rows[1];
        click(&mut app, &mut view, second);
        assert!(app.agent.history_list.is_none(), "a second click opens it");
        assert_eq!(app.agent.round_cursor, 0);
    }
}
