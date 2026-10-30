//! The `/history` list: browse, open, pin, rename, delete, export, filter
//! and compare saved rounds of recommendations. Pure logic over the
//! [`History`]; drawn by `history_list_view.rs`, wired up in
//! `agent_history.rs`.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::agent::RecommendationSet;
use crate::agent::history_store::History;

use super::line_input::LineInput;

/// Conferences only in one of two rounds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Comparison {
    /// In the selected round, not in the round on screen.
    pub added: Vec<String>,
    /// On screen, not in the selected round.
    pub removed: Vec<String>,
    pub common: usize,
}

pub fn compare(selected: &RecommendationSet, current: &RecommendationSet) -> Comparison {
    let key = |set: &RecommendationSet| -> Vec<String> {
        set.items
            .iter()
            .map(|i| format!("{} {}", i.facts.title, i.facts.year))
            .collect()
    };
    let (a, b) = (key(selected), key(current));
    Comparison {
        added: a.iter().filter(|t| !b.contains(t)).cloned().collect(),
        removed: b.iter().filter(|t| !a.contains(t)).cloned().collect(),
        common: a.iter().filter(|t| b.contains(t)).count(),
    }
}

/// What a key asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListAction {
    None,
    Close,
    /// Show this round (by id) as the cards and close the list.
    Open(u64),
    TogglePin(u64),
    Rename(u64, String),
    Delete(u64),
    Export(u64),
    Compare(u64),
}

#[derive(Debug, Clone, Default)]
pub struct HistoryList {
    /// Position in [`HistoryList::visible`].
    pub cursor: usize,
    pub filter: LineInput,
    /// Typing goes to the filter.
    pub filtering: bool,
    /// Typing goes to the new name of the selected round.
    pub renaming: Option<LineInput>,
    /// `d` was pressed once on this round; a second `d` deletes it.
    pub confirm_delete: Option<u64>,
    /// The comparison shown under the list.
    pub comparison: Option<(u64, Comparison)>,
}

impl HistoryList {
    /// A list with the cursor on round `id` (the one on screen).
    pub fn at(history: &History, id: Option<u64>) -> Self {
        let mut list = Self::default();
        if let Some(id) = id {
            list.cursor = list
                .visible(history)
                .iter()
                .position(|i| history.rounds[*i].id == id)
                .unwrap_or(0);
        }
        list
    }

    /// Indexes into `history.rounds`: pinned first, then newest first,
    /// narrowed by the filter (name, conditions or conference titles).
    pub fn visible(&self, history: &History) -> Vec<usize> {
        let needle = self.filter.text().trim().to_lowercase();
        let mut out: Vec<usize> = (0..history.rounds.len())
            .filter(|i| {
                if needle.is_empty() {
                    return true;
                }
                let round = &history.rounds[*i];
                round.name.to_lowercase().contains(&needle)
                    || round
                        .set
                        .conditions
                        .iter()
                        .any(|c| c.to_lowercase().contains(&needle))
                    || round
                        .set
                        .items
                        .iter()
                        .any(|item| item.facts.title.to_lowercase().contains(&needle))
            })
            .collect();
        out.sort_by(|a, b| {
            let (ra, rb) = (&history.rounds[*a], &history.rounds[*b]);
            rb.pinned
                .cmp(&ra.pinned)
                .then(rb.set.generated_at.cmp(&ra.set.generated_at))
        });
        out
    }

    /// The selected round's id.
    pub fn selected(&self, history: &History) -> Option<u64> {
        let visible = self.visible(history);
        visible
            .get(self.cursor.min(visible.len().saturating_sub(1)))
            .map(|i| history.rounds[*i].id)
    }

    pub fn clamp(&mut self, history: &History) {
        let count = self.visible(history).len();
        self.cursor = self.cursor.min(count.saturating_sub(1));
    }

    fn move_cursor(&mut self, history: &History, delta: isize) {
        let count = self.visible(history).len() as isize;
        if count == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, count - 1) as usize;
        self.confirm_delete = None;
        self.comparison = None;
    }

    pub fn handle_key(&mut self, history: &History, key: KeyEvent) -> ListAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let selected = self.selected(history);

        if let Some(name) = self.renaming.as_mut() {
            match key.code {
                KeyCode::Esc => self.renaming = None,
                KeyCode::Enter => {
                    let name = name.text();
                    self.renaming = None;
                    if let Some(id) = selected {
                        return ListAction::Rename(id, name);
                    }
                }
                _ => {
                    name.handle_key(key);
                }
            }
            return ListAction::None;
        }

        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filtering = false;
                }
                KeyCode::Enter | KeyCode::Down | KeyCode::Tab => self.filtering = false,
                _ => {
                    if self.filter.handle_key(key) {
                        self.cursor = 0;
                    }
                }
            }
            return ListAction::None;
        }

        // A pending delete is confirmed by a second `d` only.
        let confirm = self.confirm_delete.take();
        match key.code {
            KeyCode::Esc if self.comparison.is_some() => self.comparison = None,
            KeyCode::Esc | KeyCode::Char('q') => return ListAction::Close,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(history, -1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(history, 1),
            KeyCode::Home | KeyCode::Char('g') => self.move_cursor(history, isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_cursor(history, isize::MAX / 2),
            KeyCode::Char('/') => self.filtering = true,
            _ if ctrl => {}
            KeyCode::Enter => {
                if let Some(id) = selected {
                    return ListAction::Open(id);
                }
            }
            KeyCode::Char('p') => {
                if let Some(id) = selected {
                    return ListAction::TogglePin(id);
                }
            }
            KeyCode::Char('r') => {
                if let Some(id) = selected {
                    let mut input = LineInput::new();
                    if let Some(round) = history.get(id) {
                        input.set(&round.name);
                    }
                    self.renaming = Some(input);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(id) = selected {
                    if confirm == Some(id) {
                        return ListAction::Delete(id);
                    }
                    self.confirm_delete = Some(id);
                }
            }
            KeyCode::Char('e') => {
                if let Some(id) = selected {
                    return ListAction::Export(id);
                }
            }
            KeyCode::Char('c') => {
                if let Some(id) = selected {
                    return ListAction::Compare(id);
                }
            }
            _ => {}
        }
        ListAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::recommend::tests::{conferences, now};
    use crate::agent::recommend::{Request, recommend};
    use crate::agent::{Lang, Profile};
    use crate::data::Favorites;
    use chrono::Duration;

    fn set(interest: &str, minutes: i64) -> RecommendationSet {
        let dir = tempfile::tempdir().unwrap();
        let favorites = Favorites::empty_at(dir.path().join("f.json"));
        let confs = conferences();
        let profile = Profile {
            interests: vec![interest.into()],
            horizon_days: 100_000,
            target_ranks: vec!["A".into(), "B".into(), "C".into()],
            ..Profile::default()
        };
        let mut set = recommend(
            &Request {
                conferences: &confs,
                favorites: &favorites,
                profile: &profile,
                now: now(),
                lang: Lang::En,
            },
            8,
        );
        set.generated_at = now() + Duration::minutes(minutes);
        set
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn history() -> (History, [u64; 3]) {
        let mut h = History::default();
        let a = h.push("vision", set("computer vision", 0));
        let b = h.push("databases", set("databases", 1));
        let c = h.push("multimedia", set("multimedia", 2));
        (h, [a, b, c])
    }

    #[test]
    fn newest_first_with_pinned_on_top() {
        let (mut h, [a, b, c]) = history();
        let list = HistoryList::default();
        let ids = |list: &HistoryList, h: &History| -> Vec<u64> {
            list.visible(h).iter().map(|i| h.rounds[*i].id).collect()
        };
        assert_eq!(ids(&list, &h), [c, b, a]);
        h.toggle_pin(a);
        assert_eq!(ids(&list, &h), [a, c, b]);
        let at = HistoryList::at(&h, Some(b));
        assert_eq!(at.selected(&h), Some(b));
    }

    #[test]
    fn filter_matches_names_and_conference_titles() {
        let (h, [a, ..]) = history();
        let mut list = HistoryList::default();
        list.handle_key(&h, key(KeyCode::Char('/')));
        assert!(list.filtering);
        for c in "visi".chars() {
            list.handle_key(&h, key(KeyCode::Char(c)));
        }
        assert_eq!(list.visible(&h).len(), 1);
        assert_eq!(list.selected(&h), Some(a));
        // Typing while filtering never triggers actions.
        assert_eq!(
            list.handle_key(&h, key(KeyCode::Char('d'))),
            ListAction::None
        );
        list.handle_key(&h, key(KeyCode::Esc));
        assert!(!list.filtering);
        assert_eq!(list.visible(&h).len(), 3);

        list.filter.set("cvpr");
        let matches = list.visible(&h);
        assert!(!matches.is_empty());
        assert!(matches.iter().all(|i| {
            h.rounds[*i]
                .set
                .items
                .iter()
                .any(|item| item.facts.title == "CVPR")
        }));
    }

    #[test]
    fn keys_produce_actions() {
        let (h, [_, b, c]) = history();
        let mut list = HistoryList::default();
        assert_eq!(
            list.handle_key(&h, key(KeyCode::Enter)),
            ListAction::Open(c)
        );
        list.handle_key(&h, key(KeyCode::Down));
        assert_eq!(
            list.handle_key(&h, key(KeyCode::Char('p'))),
            ListAction::TogglePin(b)
        );
        assert_eq!(
            list.handle_key(&h, key(KeyCode::Char('e'))),
            ListAction::Export(b)
        );
        assert_eq!(
            list.handle_key(&h, key(KeyCode::Char('c'))),
            ListAction::Compare(b)
        );

        // Delete needs two presses in a row.
        assert_eq!(
            list.handle_key(&h, key(KeyCode::Char('d'))),
            ListAction::None
        );
        assert_eq!(list.confirm_delete, Some(b));
        assert_eq!(
            list.handle_key(&h, key(KeyCode::Char('d'))),
            ListAction::Delete(b)
        );
        list.handle_key(&h, key(KeyCode::Char('d')));
        list.handle_key(&h, key(KeyCode::Char('j')));
        assert_eq!(list.confirm_delete, None, "moving cancels");

        list.handle_key(&h, key(KeyCode::Up));
        list.handle_key(&h, key(KeyCode::Char('r')));
        let input = list.renaming.as_ref().unwrap();
        assert_eq!(input.text(), "databases");
        list.handle_key(&h, KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "DB ideas".chars() {
            list.handle_key(&h, key(KeyCode::Char(c)));
        }
        assert_eq!(
            list.handle_key(&h, key(KeyCode::Enter)),
            ListAction::Rename(b, "DB ideas".into())
        );
        assert_eq!(list.handle_key(&h, key(KeyCode::Esc)), ListAction::Close);
    }

    #[test]
    fn comparison_lists_added_and_removed_conferences() {
        let a = set("computer vision", 0);
        let mut b = a.clone();
        let moved = b.items.remove(0);
        let cmp = compare(&a, &b);
        assert_eq!(
            cmp.added,
            [format!("{} {}", moved.facts.title, moved.facts.year)]
        );
        assert!(cmp.removed.is_empty());
        assert_eq!(cmp.common, b.items.len());
        let back = compare(&b, &a);
        assert_eq!(back.removed.len(), 1);
    }
}
