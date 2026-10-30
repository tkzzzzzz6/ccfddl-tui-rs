//! `~/.ccfddl/agent-history.json`: every round of recommendations, so they
//! can be browsed, pinned, renamed and compared later.
//!
//! Like the session file, a round is stored as conference ids plus the
//! generated texts; facts are rebuilt from the current data on load, and
//! conferences whose deadline has passed since are kept (shown as closed).
//! The newest `limit` unpinned rounds are kept; pinned ones always are.

use std::fs;
use std::io;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use super::recommend::RecommendationSet;
use super::session_store::{restore_round, round_to_json, write_private};
use crate::model::Conference;

const VERSION: u64 = 1;
pub const DEFAULT_LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    pub id: u64,
    /// The request that produced it, or a name the user gave it.
    pub name: String,
    pub pinned: bool,
    pub set: RecommendationSet,
}

impl Round {
    /// Conferences whose deadline has passed.
    pub fn closed(&self, now: DateTime<Utc>) -> usize {
        self.set
            .items
            .iter()
            .filter(|i| i.facts.deadline.is_some_and(|d| d <= now))
            .count()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct History {
    /// Oldest first.
    pub rounds: Vec<Round>,
    pub next_id: u64,
    /// Unpinned rounds kept.
    pub limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            rounds: Vec::new(),
            next_id: 1,
            limit: DEFAULT_LIMIT,
        }
    }
}

impl History {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            ..Self::default()
        }
    }

    /// Add a round and return its id; the oldest unpinned rounds beyond the
    /// limit are dropped.
    pub fn push(&mut self, name: &str, set: RecommendationSet) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let name = name.trim();
        let name = if name.is_empty() {
            set.summary.clone()
        } else {
            name.to_string()
        };
        self.rounds.push(Round {
            id,
            name,
            pinned: false,
            set,
        });
        self.trim();
        id
    }

    pub fn trim(&mut self) {
        let unpinned = self.rounds.iter().filter(|r| !r.pinned).count();
        let mut excess = unpinned.saturating_sub(self.limit);
        self.rounds.retain(|r| {
            if excess > 0 && !r.pinned {
                excess -= 1;
                false
            } else {
                true
            }
        });
    }

    pub fn position(&self, id: u64) -> Option<usize> {
        self.rounds.iter().position(|r| r.id == id)
    }

    pub fn get(&self, id: u64) -> Option<&Round> {
        self.rounds.iter().find(|r| r.id == id)
    }

    pub fn rename(&mut self, id: u64, name: &str) -> bool {
        let name = name.trim();
        match self.rounds.iter_mut().find(|r| r.id == id) {
            Some(round) if !name.is_empty() => {
                round.name = name.to_string();
                true
            }
            _ => false,
        }
    }

    pub fn toggle_pin(&mut self, id: u64) -> Option<bool> {
        let round = self.rounds.iter_mut().find(|r| r.id == id)?;
        round.pinned = !round.pinned;
        let pinned = round.pinned;
        self.trim();
        Some(pinned)
    }

    pub fn remove(&mut self, id: u64) -> Option<Round> {
        let index = self.position(id)?;
        Some(self.rounds.remove(index))
    }

    /// `/history clear`: drop every unpinned round. Returns how many.
    pub fn clear_unpinned(&mut self) -> usize {
        let before = self.rounds.len();
        self.rounds.retain(|r| r.pinned);
        before - self.rounds.len()
    }
}

pub fn save(path: &Path, history: &History) -> io::Result<()> {
    let rounds: Vec<Value> = history
        .rounds
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "name": r.name,
                "pinned": r.pinned,
                "round": round_to_json(&r.set),
            })
        })
        .collect();
    let value = json!({
        "version": VERSION,
        "next_id": history.next_id,
        "rounds": rounds,
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&value).map_err(io::Error::other)?;
    write_private(path, &text)
}

/// Load the history. A missing file is an empty history; `Err` when the
/// file is unreadable or broken (the caller reports it and keeps the file).
pub fn load(
    path: &Path,
    conferences: &[Conference],
    now: DateTime<Utc>,
    limit: usize,
) -> Result<History, String> {
    let mut history = History::with_limit(limit);
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(history),
        Err(error) => return Err(error.to_string()),
    };
    let Value::Object(root) = serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())?
    else {
        return Err("not a JSON object".to_string());
    };
    let empty = Map::new();
    for entry in root
        .get("rounds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = entry.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let map = entry
            .get("round")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let (set, _) = restore_round(map, conferences, now);
        if set.items.is_empty() {
            continue;
        }
        history.rounds.push(Round {
            id,
            name: entry
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            pinned: entry
                .get("pinned")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            set,
        });
    }
    history.rounds.sort_by_key(|r| r.set.generated_at);
    let max_id = history.rounds.iter().map(|r| r.id).max().unwrap_or(0);
    history.next_id = root
        .get("next_id")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(max_id + 1);
    history.trim();
    Ok(history)
}

pub fn forget(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// `history_limit` at the top of `config.yaml`, or the default.
pub fn limit_from_config(path: &Path) -> usize {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text).ok())
        .and_then(|v| {
            v.get("history_limit")
                .and_then(serde_yaml_ng::Value::as_u64)
        })
        .map_or(DEFAULT_LIMIT, |n| (n as usize).clamp(1, 1000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::recommend::tests::{conferences, now};
    use crate::agent::recommend::{Request, recommend};
    use crate::agent::{Lang, Profile};
    use crate::data::Favorites;
    use chrono::Duration;

    fn sample(minutes: i64) -> RecommendationSet {
        let dir = tempfile::tempdir().unwrap();
        let favorites = Favorites::empty_at(dir.path().join("f.json"));
        let confs = conferences();
        let profile = Profile {
            interests: vec!["computer vision".into()],
            horizon_days: 100_000,
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

    #[test]
    fn rounds_are_kept_up_to_the_limit_except_pinned_ones() {
        let mut h = History::with_limit(3);
        let first = h.push("first", sample(0));
        assert_eq!(h.toggle_pin(first), Some(true));
        for i in 1..=5 {
            h.push(&format!("round {i}"), sample(i));
        }
        let names: Vec<&str> = h.rounds.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["first", "round 3", "round 4", "round 5"]);
        // Unpinning lets the limit apply to it too.
        h.toggle_pin(first);
        assert_eq!(h.rounds.len(), 3);
        assert!(h.get(first).is_none());
        // An empty name falls back to the summary.
        let id = h.push("  ", sample(9));
        assert_eq!(h.get(id).unwrap().name, h.get(id).unwrap().set.summary);
    }

    #[test]
    fn rename_remove_and_clear() {
        let mut h = History::default();
        let a = h.push("a", sample(0));
        let b = h.push("b", sample(1));
        assert!(h.rename(a, "vision ideas"));
        assert!(!h.rename(a, "   "));
        assert_eq!(h.get(a).unwrap().name, "vision ideas");
        h.toggle_pin(b);
        h.push("c", sample(2));
        assert_eq!(h.clear_unpinned(), 2);
        assert_eq!(h.rounds.len(), 1);
        assert!(h.remove(b).is_some());
        assert!(h.rounds.is_empty());
    }

    #[test]
    fn history_round_trips_and_keeps_closed_conferences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-history.json");
        let mut h = History::default();
        let id = h.push("computer vision", sample(0));
        h.toggle_pin(id);
        h.push("second", sample(5));
        save(&path, &h).unwrap();
        let loaded = load(&path, &conferences(), now(), DEFAULT_LIMIT).unwrap();
        assert_eq!(loaded, h);

        // After the first deadline, the conference is still there, closed.
        let first = h.rounds[0]
            .set
            .items
            .iter()
            .filter_map(|i| i.facts.deadline)
            .min()
            .unwrap();
        let later = first + Duration::seconds(1);
        let loaded = load(&path, &conferences(), later, DEFAULT_LIMIT).unwrap();
        assert_eq!(
            loaded.rounds[0].set.items.len(),
            h.rounds[0].set.items.len()
        );
        assert_eq!(loaded.rounds[0].closed(later), 1);
        assert_eq!(loaded.next_id, h.next_id);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn missing_broken_files_and_config_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-history.json");
        assert_eq!(load(&path, &conferences(), now(), 7).unwrap().limit, 7);
        fs::write(&path, "[1, 2").unwrap();
        assert!(load(&path, &conferences(), now(), 7).is_err());
        fs::write(&path, "{\"rounds\": [{\"id\": 3, \"round\": {}}, 7]}").unwrap();
        let h = load(&path, &conferences(), now(), 7).unwrap();
        assert!(h.rounds.is_empty(), "rounds without cards are skipped");
        assert_eq!(h.next_id, 1);
        forget(&path).unwrap();
        assert!(!path.exists());

        let config = dir.path().join("config.yaml");
        assert_eq!(limit_from_config(&config), DEFAULT_LIMIT);
        fs::write(&config, "history_limit: 12\nllm: {}\n").unwrap();
        assert_eq!(limit_from_config(&config), 12);
        fs::write(&config, "history_limit: 0\n").unwrap();
        assert_eq!(limit_from_config(&config), 1);
    }
}
