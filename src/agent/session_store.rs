//! `~/.ccfddl/agent-session.json`: the agent panel's input history, the
//! offline follow-up conditions and how the cards were shown, so they
//! survive a restart. Recommendation rounds live in `agent-history.json`
//! (see `history_store`); a round found here (written by 0.3) is migrated.
//!
//! Rounds are stored as conference ids plus the generated texts; deadlines,
//! ranks and places are rebuilt from the current data when they are read.
//! API keys and the conversation itself are never written.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Map, Value, json};

use super::recommend::{Recommendation, RecommendationSet, Source, Tier, facts_for_edition};
use super::refine::Constraints;
use crate::data::favorites::app_dir;
use crate::model::Conference;

const VERSION: u64 = 1;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SavedSession {
    pub input_history: Vec<String>,
    /// A round saved by 0.3, to migrate into the history; never written.
    pub round: Option<RecommendationSet>,
    pub constraints: Constraints,
    /// How the cards were last shown (`expanded`, `compact`, `collapsed`).
    pub cards_view: Option<String>,
}

pub fn default_session_path() -> PathBuf {
    app_dir().join("agent-session.json")
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn constraints_to_json(c: &Constraints) -> Value {
    json!({
        "interests": c.interests,
        "ready_date": c.ready_date.map(|d| d.to_string()),
        "only_ranks": c.only_ranks,
        "categories": c.categories,
        "top": c.top,
        "excluded": c.excluded,
        "no_tbd": c.no_tbd,
        "preferred_places": c.preferred_places,
        "avoided_places": c.avoided_places,
        "horizon_days": c.horizon_days,
    })
}

fn constraints_from_json(value: Option<&Value>) -> Constraints {
    let Some(Value::Object(map)) = value else {
        return Constraints::default();
    };
    Constraints {
        interests: map
            .get("interests")
            .and_then(Value::as_str)
            .map(str::to_string),
        ready_date: map
            .get("ready_date")
            .and_then(Value::as_str)
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()),
        only_ranks: strings(map.get("only_ranks")),
        categories: strings(map.get("categories")),
        top: map
            .get("top")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 30) as usize),
        excluded: strings(map.get("excluded")),
        no_tbd: map.get("no_tbd").and_then(Value::as_bool).unwrap_or(false),
        preferred_places: strings(map.get("preferred_places")),
        avoided_places: strings(map.get("avoided_places")),
        horizon_days: map.get("horizon_days").and_then(Value::as_i64),
    }
}

pub(crate) fn round_to_json(set: &RecommendationSet) -> Value {
    let source = match &set.source {
        Source::Rules => json!({"kind": "rules"}),
        Source::Llm(model) => json!({"kind": "llm", "model": model}),
    };
    let items: Vec<Value> = set
        .items
        .iter()
        .map(|item| {
            json!({
                "id": item.facts.id,
                "title": item.facts.title,
                "year": item.facts.year,
                "tier": item.tier.key(),
                "score": item.score,
                "reason": item.reason,
                "direction": item.direction,
                "risks": item.risks,
            })
        })
        .collect();
    json!({
        "summary": set.summary,
        "conditions": set.conditions,
        "source": source,
        "generated_at": set.generated_at.to_rfc3339(),
        "items": items,
    })
}

/// Rebuild a saved round from current data. Conferences whose deadline has
/// passed since are kept (their facts show it); only those no longer in
/// the data are dropped, and their number is returned.
pub(crate) fn restore_round(
    map: &Map<String, Value>,
    conferences: &[Conference],
    now: DateTime<Utc>,
) -> (RecommendationSet, usize) {
    let source = match map.get("source") {
        Some(Value::Object(s)) if s.get("kind").and_then(Value::as_str) == Some("llm") => {
            Source::Llm(
                s.get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        }
        _ => Source::Rules,
    };
    let saved = map
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut items = Vec::new();
    for item in &saved {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let year = item.get("year").and_then(Value::as_i64).unwrap_or(0) as i32;
        let Some(tier) = item
            .get("tier")
            .and_then(Value::as_str)
            .and_then(Tier::parse)
        else {
            continue;
        };
        // The same edition, open or not.
        let Some(facts) = facts_for_edition(conferences, id, year, now) else {
            continue;
        };
        let text = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        items.push(Recommendation {
            facts,
            tier,
            score: item.get("score").and_then(Value::as_f64),
            reason: text("reason"),
            direction: text("direction"),
            risks: strings(item.get("risks")),
        });
    }
    let dropped = saved.len() - items.len();
    let set = RecommendationSet {
        summary: map
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        conditions: strings(map.get("conditions")),
        items,
        source,
        generated_at: map
            .get("generated_at")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map_or(now, |d| d.with_timezone(&Utc)),
    };
    (set, dropped)
}

pub fn save(path: &Path, session: &SavedSession) -> io::Result<()> {
    let value = json!({
        "version": VERSION,
        "input_history": session.input_history,
        "constraints": constraints_to_json(&session.constraints),
        "cards_view": session.cards_view,
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&value).map_err(io::Error::other)?;
    write_private(path, &text)
}

#[cfg(unix)]
pub(crate) fn write_private(path: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())
}

#[cfg(not(unix))]
pub(crate) fn write_private(path: &Path, text: &str) -> io::Result<()> {
    fs::write(path, text)
}

/// What was restored.
#[derive(Debug, Clone, PartialEq)]
pub struct Restored {
    pub session: SavedSession,
}

/// Load the session. `Ok(None)` when there is none; `Err` when the file is
/// unreadable or broken (the caller reports it and starts fresh).
pub fn load(
    path: &Path,
    conferences: &[Conference],
    now: DateTime<Utc>,
) -> Result<Option<Restored>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let Value::Object(root) = serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())?
    else {
        return Err("not a JSON object".to_string());
    };
    let mut input_history = strings(root.get("input_history"));
    let excess = input_history.len().saturating_sub(100);
    input_history.drain(..excess);
    let round = match root.get("round") {
        Some(Value::Object(map)) => {
            let (set, _) = restore_round(map, conferences, now);
            (!set.items.is_empty()).then_some(set)
        }
        _ => None,
    };
    Ok(Some(Restored {
        session: SavedSession {
            input_history,
            round,
            constraints: constraints_from_json(root.get("constraints")),
            cards_view: root
                .get("cards_view")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
    }))
}

pub fn forget(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
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

    fn sample() -> RecommendationSet {
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
        set.conditions = vec!["CCF-A only".into()];
        set
    }

    #[test]
    fn a_session_round_trips_without_rounds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s/agent-session.json");
        let session = SavedSession {
            input_history: vec!["computer vision".into(), "只要 A 类".into()],
            round: Some(sample()),
            constraints: Constraints {
                interests: Some("computer vision".into()),
                only_ranks: vec!["A".into()],
                top: Some(5),
                ready_date: NaiveDate::from_ymd_opt(2030, 1, 1),
                ..Default::default()
            },
            cards_view: Some("compact".into()),
        };
        save(&path, &session).unwrap();
        let restored = load(&path, &conferences(), now()).unwrap().unwrap();
        assert_eq!(restored.session.input_history, session.input_history);
        assert_eq!(restored.session.constraints, session.constraints);
        assert_eq!(restored.session.cards_view.as_deref(), Some("compact"));
        // Rounds belong to agent-history.json now.
        assert!(restored.session.round.is_none());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_round_written_by_0_3_is_read_for_migration_closed_ones_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-session.json");
        let round = sample();
        let legacy = json!({
            "version": 1,
            "input_history": ["computer vision"],
            "round": round_to_json(&round),
        });
        fs::write(&path, legacy.to_string()).unwrap();
        let restored = load(&path, &conferences(), now()).unwrap().unwrap();
        let back = restored.session.round.unwrap();
        assert_eq!(back.items, round.items);
        assert_eq!(back.conditions, round.conditions);
        assert_eq!(back.summary, round.summary);

        // After a deadline passed the conference is kept, with that date.
        let first = round
            .items
            .iter()
            .filter_map(|i| i.facts.deadline)
            .min()
            .unwrap();
        let later = first + Duration::seconds(1);
        let restored = load(&path, &conferences(), later).unwrap().unwrap();
        let set = restored.session.round.unwrap();
        assert_eq!(set.items.len(), round.items.len());
        assert!(set.items.iter().any(|i| i.facts.deadline == Some(first)));
    }

    #[test]
    fn missing_broken_and_forgotten_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-session.json");
        assert_eq!(load(&path, &conferences(), now()), Ok(None));
        fs::write(&path, "{not json").unwrap();
        assert!(load(&path, &conferences(), now()).is_err());
        fs::write(&path, "{\"input_history\": [\"a\", 3], \"round\": 7}").unwrap();
        let restored = load(&path, &conferences(), now()).unwrap().unwrap();
        assert_eq!(restored.session.input_history, ["a"]);
        assert!(restored.session.round.is_none());
        forget(&path).unwrap();
        assert!(!path.exists());
        forget(&path).unwrap();
    }
}
