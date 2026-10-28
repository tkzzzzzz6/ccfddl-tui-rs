//! Tools exposed to the model. Every tool reads the local data set, so the
//! model can only cite deadlines, ranks and places that actually exist.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::Lang;
use super::llm::{ToolCall, ToolSpec};
use super::profile::{Profile, normalize_categories, normalize_ranks, parse_date};
use super::recommend::{
    self, Candidate, ConferenceFacts, Recommendation, RecommendationSet, Request, Source, Tier,
    facts_for, find_conference, open_conferences,
};
use super::topics::{self, subs_for_tags, tags_for_text, venue_tags};
use crate::data::Favorites;
use crate::model::Conference;

pub const SUBMIT: &str = "submit_recommendations";

/// Snapshot of everything the tools may read, owned by the worker thread.
#[derive(Clone)]
pub struct ToolContext {
    pub conferences: Vec<Conference>,
    pub favorites: Favorites,
    pub profile: Profile,
    pub now: DateTime<Utc>,
    pub lang: Lang,
    /// Profile after the model's overrides in `rank_candidates`.
    pub effective_profile: Profile,
}

impl ToolContext {
    pub fn new(
        conferences: Vec<Conference>,
        favorites: Favorites,
        profile: Profile,
        now: DateTime<Utc>,
        lang: Lang,
    ) -> Self {
        Self {
            conferences,
            favorites,
            effective_profile: profile.clone(),
            profile,
            now,
            lang,
        }
    }

    fn candidates(&self, profile: &Profile) -> Vec<Candidate> {
        recommend::candidates(&Request {
            conferences: &self.conferences,
            favorites: &self.favorites,
            profile,
            now: self.now,
            lang: self.lang,
        })
    }
}

/// Result of running one tool.
pub struct ToolOutput {
    /// JSON sent back to the model.
    pub content: String,
    /// One line for the transcript.
    pub summary: String,
    /// Set by `submit_recommendations`.
    pub recommendations: Option<RecommendationSet>,
}

impl ToolOutput {
    fn json(value: Value, summary: String) -> Self {
        Self {
            content: value.to_string(),
            summary,
            recommendations: None,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            content: json!({ "error": message }).to_string(),
            summary: format!("error: {message}"),
            recommendations: None,
        }
    }
}

pub fn specs() -> Vec<ToolSpec> {
    let string_list = json!({ "type": "array", "items": { "type": "string" } });
    vec![
        ToolSpec {
            name: "get_profile",
            description: "Read the user's saved research profile (interests, categories, target CCF ranks, paper-ready date, place preferences) and today's date.",
            parameters: json!({ "type": "object", "properties": {} }),
        },
        ToolSpec {
            name: "rank_candidates",
            description: "Score all conferences with an open deadline using the offline rule engine (topic match, CCF rank vs targets, time slack, preferences). Returns candidates with tier (stretch/main/safe), score, deadline facts, rule-based reasons and risks. Pass fields to override the saved profile for this request, e.g. interests the user just mentioned. Call this first.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "interests": string_list.clone(),
                    "categories": { "type": "array", "items": { "type": "string", "enum": ["DS","NW","SC","SE","DB","CT","CG","AI","HI","MX"] } },
                    "target_ranks": { "type": "array", "items": { "type": "string", "enum": ["A","B","C","N"] } },
                    "ready_date": { "type": "string", "description": "YYYY-MM-DD when the paper will be ready" },
                    "top": { "type": "integer", "minimum": 1, "maximum": 30 }
                }
            }),
        },
        ToolSpec {
            name: "search_conferences",
            description: "Search conferences that still have an open deadline by keyword (title, full name or topic), CCF category and rank. Sorted by deadline.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "categories": string_list.clone(),
                    "ccf_ranks": string_list.clone(),
                    "max_days_left": { "type": "integer" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
                }
            }),
        },
        ToolSpec {
            name: "get_conference",
            description: "Full details of one conference by id or title: all deadlines of the next edition, abstract deadline, notes, place, dates, link and topics.",
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        },
        ToolSpec {
            name: SUBMIT,
            description: "Submit the final recommendations to the user's screen. Use ids returned by other tools. Deadlines and ranks are filled in from the data automatically, so do not repeat them; focus on why the venue fits, a concrete submission direction, and risks.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "2-3 sentence overall strategy" },
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "tier": { "type": "string", "enum": ["stretch", "main", "safe"] },
                                "reason": { "type": "string" },
                                "direction": { "type": "string" },
                                "risks": string_list
                            },
                            "required": ["id", "tier", "reason", "direction"]
                        }
                    }
                },
                "required": ["summary", "items"]
            }),
        },
    ]
}

fn str_list(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => super::profile::split_list(s),
        _ => Vec::new(),
    }
}

fn facts_json(facts: &ConferenceFacts, now: DateTime<Utc>) -> Value {
    json!({
        "id": facts.id,
        "title": facts.title,
        "year": facts.year,
        "name": facts.description,
        "category": facts.sub,
        "ccf": facts.ccf,
        "core": facts.core,
        "thcpl": facts.thcpl,
        "deadline": facts.deadline_text,
        "days_left": facts.days_left(now),
        "abstract_deadline": facts.abstract_text,
        "place": facts.place,
        "dates": facts.date,
    })
}

pub fn execute(ctx: &mut ToolContext, call: &ToolCall, model: &str) -> ToolOutput {
    let args: Value = match serde_json::from_str(if call.arguments.trim().is_empty() {
        "{}"
    } else {
        &call.arguments
    }) {
        Ok(value) => value,
        Err(e) => return ToolOutput::error(format!("invalid JSON arguments: {e}")),
    };
    match call.name.as_str() {
        "get_profile" => get_profile(ctx),
        "rank_candidates" => rank_candidates(ctx, &args),
        "search_conferences" => search_conferences(ctx, &args),
        "get_conference" => get_conference(ctx, &args),
        SUBMIT => submit(ctx, &args, model),
        other => ToolOutput::error(format!("unknown tool {other}")),
    }
}

fn get_profile(ctx: &ToolContext) -> ToolOutput {
    let profile = &ctx.profile;
    let tags = tags_for_text(&profile.interests.join(" , "));
    let value = json!({
        "today": ctx.now.format("%Y-%m-%d").to_string(),
        "profile_saved": !profile.is_empty(),
        "interests": profile.interests,
        "recognised_topics": tags.iter().map(|t| topics::tag_name(t, false)).collect::<Vec<_>>(),
        "categories": if profile.categories.is_empty() { subs_for_tags(&tags).into_iter().collect() } else { profile.categories.clone() },
        "target_ranks": profile.target_ranks,
        "ready_date": profile.ready_date.map(|d| d.to_string()),
        "min_days_left": profile.min_days_left,
        "horizon_days": profile.horizon_days,
        "preferred_places": profile.preferred_places,
        "avoided_places": profile.avoided_places,
    });
    let summary = if profile.is_empty() {
        "no saved profile".to_string()
    } else {
        format!("{} interest(s)", profile.interests.len())
    };
    ToolOutput::json(value, summary)
}

fn rank_candidates(ctx: &mut ToolContext, args: &Value) -> ToolOutput {
    let mut profile = ctx.profile.clone();
    let interests = str_list(args, "interests");
    if !interests.is_empty() {
        profile = profile.with_extra_interests(&interests);
    }
    let categories = normalize_categories(&str_list(args, "categories"));
    if !categories.is_empty() {
        profile.categories = categories;
    }
    let ranks = normalize_ranks(&str_list(args, "target_ranks"));
    if !ranks.is_empty() {
        profile.target_ranks = ranks;
    }
    if let Some(date) = args
        .get("ready_date")
        .and_then(Value::as_str)
        .and_then(parse_date)
    {
        profile.ready_date = Some(date);
    }
    let top = args
        .get("top")
        .and_then(Value::as_u64)
        .unwrap_or(15)
        .clamp(1, 30) as usize;

    let all = ctx.candidates(&profile);
    let total = all.len();
    let items: Vec<Value> = all
        .iter()
        .take(top)
        .map(|c| {
            let mut value = facts_json(&c.facts, ctx.now);
            value["tier"] = json!(c.tier.key());
            value["score"] = json!(c.score);
            value["slack_days"] = json!(c.slack_days);
            value["matched_topics"] = json!(
                c.matched_tags
                    .iter()
                    .map(|t| topics::tag_name(t, false))
                    .collect::<Vec<_>>()
            );
            value["venue_topics"] = json!(
                c.venue_tags
                    .iter()
                    .map(|t| topics::tag_name(t, false))
                    .collect::<Vec<_>>()
            );
            value["favorite"] = json!(c.is_favorite);
            value["rule_reasons"] = json!(c.reasons);
            value["rule_risks"] = json!(c.risks);
            value
        })
        .collect();
    let tags = tags_for_text(&profile.interests.join(" , "));
    let shown = items.len();
    let value = json!({
        "total_candidates": total,
        "recognised_topics": tags.iter().map(|t| topics::tag_name(t, false)).collect::<Vec<_>>(),
        "target_ranks": profile.target_ranks,
        "candidates": items,
    });
    ctx.effective_profile = profile;
    ToolOutput::json(value, format!("{shown} of {total} candidates"))
}

fn search_conferences(ctx: &ToolContext, args: &Value) -> ToolOutput {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let query_tags = tags_for_text(&query);
    let categories: BTreeSet<String> = normalize_categories(&str_list(args, "categories"))
        .into_iter()
        .collect();
    let ranks: BTreeSet<String> = normalize_ranks(&str_list(args, "ccf_ranks"))
        .into_iter()
        .collect();
    let max_days = args.get("max_days_left").and_then(Value::as_i64);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 50) as usize;

    let mut hits: Vec<ConferenceFacts> = open_conferences(&ctx.conferences, ctx.now)
        .into_iter()
        .filter(|f| categories.is_empty() || categories.contains(&f.sub))
        .filter(|f| ranks.is_empty() || ranks.contains(&f.ccf))
        .filter(|f| max_days.is_none_or(|max| f.days_left(ctx.now).is_some_and(|d| d <= max)))
        .filter(|f| {
            query.is_empty()
                || f.title.to_lowercase().contains(&query)
                || f.description.to_lowercase().contains(&query)
                || !venue_tags(&f.title, &f.description).is_disjoint(&query_tags)
        })
        .collect();
    hits.sort_by(|a, b| match (a.deadline, b.deadline) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.title.cmp(&b.title),
    });
    let total = hits.len();
    let items: Vec<Value> = hits
        .iter()
        .take(limit)
        .map(|f| facts_json(f, ctx.now))
        .collect();
    let shown = items.len();
    ToolOutput::json(
        json!({ "total": total, "results": items }),
        format!("{shown} of {total} results"),
    )
}

fn get_conference(ctx: &ToolContext, args: &Value) -> ToolOutput {
    let Some(id) = args.get("id").and_then(Value::as_str) else {
        return ToolOutput::error("missing id");
    };
    let Some(conf) = find_conference(&ctx.conferences, id) else {
        return ToolOutput::error(format!("no conference called {id}"));
    };
    let facts = facts_for(&ctx.conferences, id, ctx.now);
    let edition = facts
        .as_ref()
        .and_then(|f| conf.confs.iter().find(|y| y.year == f.year));
    let mut value = match &facts {
        Some(f) => facts_json(f, ctx.now),
        None => json!({ "title": conf.title, "open": false, "note": "no open deadline" }),
    };
    if let Some(edition) = edition {
        value["timezone"] = json!(edition.timezone);
        value["link"] = json!(edition.link);
        value["all_deadlines"] = edition
            .timeline
            .iter()
            .map(|t| {
                json!({
                    "deadline": t.deadline,
                    "abstract_deadline": t.abstract_deadline,
                    "comment": t.comment,
                })
            })
            .collect();
    }
    value["topics"] = json!(
        venue_tags(&conf.title, &conf.description)
            .iter()
            .map(|t| topics::tag_name(t, false))
            .collect::<Vec<_>>()
    );
    value["favorite"] = json!(
        facts
            .as_ref()
            .is_some_and(|f| ctx.favorites.contains(&f.title, f.year))
    );
    ToolOutput::json(value, conf.title.clone())
}

fn submit(ctx: &ToolContext, args: &Value, model: &str) -> ToolOutput {
    let summary = args
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let rule_candidates: BTreeMap<String, Candidate> = ctx
        .candidates(&ctx.effective_profile)
        .into_iter()
        .map(|c| (c.facts.id.to_lowercase(), c))
        .collect();

    let mut items = Vec::new();
    let mut rejected = Vec::new();
    let mut seen = BTreeSet::new();
    for item in args
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = item
            .get("id")
            .or_else(|| item.get("title"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(facts) = facts_for(&ctx.conferences, id, ctx.now) else {
            rejected.push(id.to_string());
            continue;
        };
        if !seen.insert(facts.id.clone()) {
            continue;
        }
        let rule = rule_candidates.get(&facts.id.to_lowercase());
        let tier = item
            .get("tier")
            .and_then(Value::as_str)
            .and_then(Tier::parse)
            .or(rule.map(|c| c.tier))
            .unwrap_or(Tier::Main);
        let text = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        // Data-derived risks (abstract deadlines, tight slack) always stay.
        let mut risks: Vec<String> = rule.map(|c| c.risks.clone()).unwrap_or_default();
        for risk in str_list(item, "risks") {
            if !risk.trim().is_empty() && !risks.contains(&risk) {
                risks.push(risk);
            }
        }
        let reason = text("reason");
        let direction = text("direction");
        items.push(Recommendation {
            reason: if reason.is_empty() {
                rule.map(|c| c.reasons.join("; ")).unwrap_or_default()
            } else {
                reason
            },
            direction: if direction.is_empty() {
                rule.map(|c| c.direction.clone()).unwrap_or_default()
            } else {
                direction
            },
            facts,
            tier,
            score: rule.map(|c| c.score),
            risks,
        });
    }
    items.sort_by_key(|item| item.tier);

    if items.is_empty() {
        return ToolOutput::error(format!(
            "none of the ids exist or have an open deadline: {}",
            rejected.join(", ")
        ));
    }
    let count = items.len();
    let mut content = json!({ "accepted": count, "status": "shown to the user" });
    if !rejected.is_empty() {
        content["rejected_ids"] = json!(rejected);
    }
    ToolOutput {
        content: content.to_string(),
        summary: if rejected.is_empty() {
            format!("{count} recommendations")
        } else {
            format!(
                "{count} recommendations, dropped unknown: {}",
                rejected.join(", ")
            )
        },
        recommendations: Some(RecommendationSet {
            summary,
            conditions: Vec::new(),
            items,
            source: Source::Llm(model.to_string()),
            generated_at: ctx.now,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm::tests::call;
    use crate::agent::recommend::tests::{conferences, now};

    fn ctx() -> ToolContext {
        let dir = tempfile::tempdir().unwrap();
        let favorites = Favorites::empty_at(dir.path().join("f.json"));
        std::mem::forget(dir);
        ToolContext::new(
            conferences(),
            favorites,
            Profile {
                interests: vec!["computer vision".into()],
                ..Profile::default()
            },
            now(),
            Lang::En,
        )
    }

    fn run(ctx: &mut ToolContext, name: &str, args: Value) -> (Value, ToolOutput) {
        let out = execute(ctx, &call(name, args), "test-model");
        (serde_json::from_str(&out.content).unwrap(), out)
    }

    #[test]
    fn specs_have_unique_names() {
        let names: BTreeSet<&str> = specs().iter().map(|s| s.name).collect();
        assert_eq!(names.len(), specs().len());
        assert!(names.contains(SUBMIT));
    }

    #[test]
    fn profile_tool_reports_topics() {
        let (value, _) = run(&mut ctx(), "get_profile", json!({}));
        assert_eq!(value["today"], "2030-01-01");
        assert_eq!(value["recognised_topics"][0], "computer vision");
    }

    #[test]
    fn rank_candidates_accepts_overrides() {
        let mut ctx = ctx();
        let (value, out) = run(
            &mut ctx,
            "rank_candidates",
            json!({ "interests": ["database"], "target_ranks": ["A"], "top": 3 }),
        );
        let titles: Vec<&str> = value["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["title"].as_str().unwrap())
            .collect();
        assert!(titles.contains(&"SIGMOD"), "{titles:?}");
        assert!(out.summary.contains("candidates"));
        assert_eq!(ctx.effective_profile.target_ranks, ["A"]);
    }

    #[test]
    fn search_filters_by_query_rank_and_time() {
        let mut ctx = ctx();
        let (value, _) = run(&mut ctx, "search_conferences", json!({ "query": "vision" }));
        let titles: Vec<&str> = value["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["title"].as_str().unwrap())
            .collect();
        assert!(titles.contains(&"CVPR"));
        assert!(!titles.contains(&"OldConf"));

        let (value, _) = run(
            &mut ctx,
            "search_conferences",
            json!({ "ccf_ranks": ["A"], "max_days_left": 45 }),
        );
        assert_eq!(value["total"], 1, "{value}");
        assert_eq!(value["results"][0]["title"], "ACL");
    }

    #[test]
    fn get_conference_returns_all_deadlines() {
        let (value, _) = run(&mut ctx(), "get_conference", json!({ "id": "cvpr" }));
        assert_eq!(value["link"], "https://cvpr.example");
        assert_eq!(
            value["all_deadlines"][0]["abstract_deadline"],
            "2030-03-01 23:59:59"
        );
        let (value, out) = run(&mut ctx(), "get_conference", json!({ "id": "nope" }));
        assert!(value["error"].is_string());
        assert!(out.summary.starts_with("error"));
    }

    #[test]
    fn submit_validates_ids_and_keeps_data_facts() {
        let mut ctx = ctx();
        let (value, out) = run(
            &mut ctx,
            SUBMIT,
            json!({
                "summary": "Go for CVPR",
                "items": [
                    { "id": "ICME", "tier": "main", "reason": "fits", "direction": "multimodal" },
                    { "id": "cvpr", "tier": "stretch", "reason": "top venue", "direction": "3D", "risks": ["crowded"] },
                    { "id": "Imaginary", "tier": "safe", "reason": "x", "direction": "y" },
                    { "id": "OldConf", "tier": "safe", "reason": "x", "direction": "y" }
                ]
            }),
        );
        assert_eq!(value["accepted"], 2);
        assert_eq!(value["rejected_ids"], json!(["Imaginary", "OldConf"]));
        let set = out.recommendations.unwrap();
        assert_eq!(set.source, Source::Llm("test-model".into()));
        // Sorted by tier, facts from data, rule risks merged with the model's.
        assert_eq!(set.items[0].facts.title, "CVPR");
        assert_eq!(
            set.items[0].facts.deadline_text,
            "2030-03-10 23:59:59 (UTC-12)"
        );
        assert!(set.items[0].risks.iter().any(|r| r.contains("Abstract")));
        assert!(set.items[0].risks.contains(&"crowded".to_string()));
    }

    #[test]
    fn submit_with_only_unknown_ids_is_an_error() {
        let (value, out) = run(
            &mut ctx(),
            SUBMIT,
            json!({ "summary": "", "items": [{ "id": "nope", "tier": "main", "reason": "", "direction": "" }] }),
        );
        assert!(value["error"].is_string());
        assert!(out.recommendations.is_none());
    }

    #[test]
    fn bad_arguments_and_unknown_tools_are_reported_to_the_model() {
        let mut ctx = ctx();
        let out = execute(
            &mut ctx,
            &ToolCall {
                id: "1".into(),
                name: "get_profile".into(),
                arguments: "{not json".into(),
            },
            "m",
        );
        assert!(out.content.contains("invalid JSON"));
        let (value, _) = run(&mut ctx, "rm_rf", json!({}));
        assert!(value["error"].as_str().unwrap().contains("unknown tool"));
    }
}
