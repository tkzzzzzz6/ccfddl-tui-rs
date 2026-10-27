//! One user turn of the agent: the LLM tool loop, or the offline fallback.

use std::sync::atomic::{AtomicBool, Ordering};

use super::Lang;
use super::llm::{ChatMessage, LlmClient, LlmError};
use super::profile::parse_date;
use super::recommend::{self, RecommendationSet, Request};
use super::tools::{self, ToolContext};
use super::topics::tags_for_text;

/// Upper bound on model calls per turn, so a confused model cannot loop.
pub const MAX_STEPS: usize = 8;
pub const DEFAULT_TOP: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// Streamed assistant text.
    Delta(String),
    ToolStarted {
        name: String,
        arguments: String,
    },
    ToolFinished {
        name: String,
        summary: String,
    },
    Recommendations(RecommendationSet),
    /// The turn ended; `history` is the conversation to send next time.
    Finished {
        history: Vec<ChatMessage>,
    },
    Failed(String),
    Cancelled,
}

pub fn system_prompt(ctx: &ToolContext) -> String {
    let language = match ctx.lang {
        Lang::Zh => "Simplified Chinese",
        Lang::En => "English",
    };
    format!(
        "You are the conference recommendation assistant inside ccfddl-tui, a terminal app that tracks deadlines of CCF-ranked computer science conferences. Today is {today} (UTC).\n\
         \n\
         Rules:\n\
         1. Deadlines, ranks, places and dates must come from tool results. Never invent conferences or deadlines.\n\
         2. For recommendation requests, call rank_candidates first (pass interests, target ranks or a ready date the user mentioned), use get_conference or search_conferences if you need more detail, then call submit_recommendations exactly once with 3-8 items, covering stretch / main / safe tiers when candidates allow.\n\
         3. In each item, `reason` explains why the venue fits this user, citing tool facts (topic match, rank, days left); `direction` gives a concrete submission angle, track or contribution type for that venue; `risks` lists timing or fit risks. Anything not from tools, such as a venue's reviewing culture, must be marked as general experience.\n\
         4. After submit_recommendations, stop: the UI renders the cards, so do not repeat them.\n\
         5. For other questions (one deadline, comparing two venues), answer briefly using the tools.\n\
         6. Reply in {language}. Keep text short: it is shown in a terminal.\n\
         \n\
         Saved profile:\n{profile}",
        today = ctx.now.format("%Y-%m-%d"),
        profile = ctx.profile.summary(false),
    )
}

/// Run one turn against a model.
pub fn run_llm_turn(
    client: &mut dyn LlmClient,
    ctx: &mut ToolContext,
    mut history: Vec<ChatMessage>,
    user_text: &str,
    emit: &mut dyn FnMut(AgentEvent),
    cancel: &AtomicBool,
) {
    history.push(ChatMessage::user(user_text));
    let specs = tools::specs();
    let model = client.model().to_string();

    for _ in 0..MAX_STEPS {
        let mut messages = Vec::with_capacity(history.len() + 1);
        messages.push(ChatMessage::system(system_prompt(ctx)));
        messages.extend(history.iter().cloned());

        let completion = {
            let mut on_delta = |text: &str| emit(AgentEvent::Delta(text.to_string()));
            client.complete(&messages, &specs, &mut on_delta, cancel)
        };
        let completion = match completion {
            Ok(completion) => completion,
            Err(LlmError::Cancelled) => {
                emit(AgentEvent::Cancelled);
                return;
            }
            Err(error) => {
                emit(AgentEvent::Failed(error.to_string()));
                return;
            }
        };

        history.push(ChatMessage::assistant(
            completion.content.clone(),
            completion.tool_calls.clone(),
        ));
        if completion.tool_calls.is_empty() {
            emit(AgentEvent::Finished { history });
            return;
        }

        let mut submitted = false;
        for call in &completion.tool_calls {
            if cancel.load(Ordering::Relaxed) {
                emit(AgentEvent::Cancelled);
                return;
            }
            emit(AgentEvent::ToolStarted {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            });
            let output = tools::execute(ctx, call, &model);
            emit(AgentEvent::ToolFinished {
                name: call.name.clone(),
                summary: output.summary.clone(),
            });
            history.push(ChatMessage::tool_result(&call.id, output.content));
            if let Some(set) = output.recommendations {
                emit(AgentEvent::Recommendations(set));
                submitted = true;
            }
        }
        if submitted {
            emit(AgentEvent::Finished { history });
            return;
        }
    }

    emit(AgentEvent::Delta(
        ctx.lang
            .pick(
                "\n(Stopped after too many tool calls.)",
                "\n（工具调用次数过多，已停止。）",
            )
            .to_string(),
    ));
    emit(AgentEvent::Finished { history });
}

/// Rule-based answer for when no model is configured. Interests and a
/// `YYYY-MM-DD` ready date are picked out of the user's text.
pub fn run_offline_turn(
    ctx: &ToolContext,
    user_text: &str,
    top: usize,
    emit: &mut dyn FnMut(AgentEvent),
) {
    let mut profile = ctx.profile.clone();
    if !tags_for_text(user_text).is_empty() {
        profile = profile.with_extra_interests(&[user_text.trim().to_string()]);
    }
    if let Some(date) = user_text
        .split(|c: char| !(c.is_ascii_digit() || c == '-' || c == '/'))
        .find_map(parse_date)
    {
        profile.ready_date = Some(date);
    }
    emit(AgentEvent::ToolStarted {
        name: "rank_candidates".to_string(),
        arguments: String::new(),
    });
    let set = recommend::recommend(
        &Request {
            conferences: &ctx.conferences,
            favorites: &ctx.favorites,
            profile: &profile,
            now: ctx.now,
            lang: ctx.lang,
        },
        top,
    );
    emit(AgentEvent::ToolFinished {
        name: "rank_candidates".to_string(),
        summary: format!("{} picks", set.items.len()),
    });
    emit(AgentEvent::Delta(
        ctx.lang
            .pick(
                "Offline mode (no CCFDDL_API_KEY): rule-based recommendations.",
                "离线模式（未设置 CCFDDL_API_KEY）：以下是规则推荐。",
            )
            .to_string(),
    ));
    emit(AgentEvent::Recommendations(set));
    emit(AgentEvent::Finished {
        history: Vec::new(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Profile;
    use crate::agent::llm::Completion;
    use crate::agent::llm::tests::{ScriptedClient, call};
    use crate::agent::recommend::tests::{conferences, now};
    use crate::agent::tools::SUBMIT;
    use crate::data::Favorites;
    use serde_json::json;

    fn ctx(lang: Lang) -> ToolContext {
        let dir = tempfile::tempdir().unwrap();
        let favorites = Favorites::empty_at(dir.path().join("f.json"));
        std::mem::forget(dir);
        ToolContext::new(conferences(), favorites, Profile::default(), now(), lang)
    }

    fn collect(f: impl FnOnce(&mut dyn FnMut(AgentEvent))) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        f(&mut |event| events.push(event));
        events
    }

    #[test]
    fn tool_loop_ends_with_submitted_recommendations() {
        let mut client = ScriptedClient::new(vec![
            Completion {
                content: "Let me look.".into(),
                tool_calls: vec![call("rank_candidates", json!({ "interests": ["vision"] }))],
            },
            Completion {
                content: String::new(),
                tool_calls: vec![call(
                    SUBMIT,
                    json!({
                        "summary": "CVPR first",
                        "items": [{ "id": "CVPR", "tier": "stretch", "reason": "r", "direction": "d" }]
                    }),
                )],
            },
        ]);
        let seen = client.seen.clone();
        let mut ctx = ctx(Lang::En);
        let events = collect(|emit| {
            run_llm_turn(
                &mut client,
                &mut ctx,
                Vec::new(),
                "recommend vision venues",
                emit,
                &AtomicBool::new(false),
            )
        });

        assert!(matches!(&events[0], AgentEvent::Delta(text) if text == "Let me"));
        assert!(events.iter().any(
            |e| matches!(e, AgentEvent::ToolStarted { name, .. } if name == "rank_candidates")
        ));
        let set = events
            .iter()
            .find_map(|e| match e {
                AgentEvent::Recommendations(set) => Some(set.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(set.items[0].facts.title, "CVPR");
        let AgentEvent::Finished { history } = events.last().unwrap() else {
            panic!("{events:?}");
        };
        // user, assistant+call, tool, assistant+submit, tool
        assert_eq!(history.len(), 5);

        // The second request carried the tool result back to the model, and
        // every request starts with the system prompt.
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[1][0].content.contains("ccfddl-tui"));
        assert!(
            seen[1]
                .iter()
                .any(|m| m.content.contains("total_candidates"))
        );
    }

    #[test]
    fn plain_answer_finishes_the_turn() {
        let mut client = ScriptedClient::new(vec![Completion {
            content: "Hello!".into(),
            tool_calls: vec![],
        }]);
        let mut ctx = ctx(Lang::Zh);
        let events = collect(|emit| {
            run_llm_turn(
                &mut client,
                &mut ctx,
                Vec::new(),
                "hi",
                emit,
                &AtomicBool::new(false),
            )
        });
        assert!(
            matches!(events.last(), Some(AgentEvent::Finished { history }) if history.len() == 2)
        );
    }

    #[test]
    fn errors_and_cancellation_are_reported() {
        let mut client = ScriptedClient::new(vec![]);
        let mut ctx = ctx(Lang::En);
        let events = collect(|emit| {
            run_llm_turn(
                &mut client,
                &mut ctx,
                Vec::new(),
                "hi",
                emit,
                &AtomicBool::new(false),
            )
        });
        assert!(matches!(events.last(), Some(AgentEvent::Failed(_))));

        let events = collect(|emit| {
            run_llm_turn(
                &mut client,
                &mut ctx,
                Vec::new(),
                "hi",
                emit,
                &AtomicBool::new(true),
            )
        });
        assert_eq!(events.last(), Some(&AgentEvent::Cancelled));
    }

    #[test]
    fn runaway_tool_loops_are_capped() {
        let replies = (0..MAX_STEPS + 2)
            .map(|_| Completion {
                content: String::new(),
                tool_calls: vec![call("get_profile", json!({}))],
            })
            .collect();
        let mut client = ScriptedClient::new(replies);
        let seen = client.seen.clone();
        let mut ctx = ctx(Lang::En);
        let events = collect(|emit| {
            run_llm_turn(
                &mut client,
                &mut ctx,
                Vec::new(),
                "loop",
                emit,
                &AtomicBool::new(false),
            )
        });
        assert_eq!(seen.lock().unwrap().len(), MAX_STEPS);
        assert!(matches!(events.last(), Some(AgentEvent::Finished { .. })));
    }

    #[test]
    fn offline_turn_reads_interests_and_dates_from_text() {
        let ctx = ctx(Lang::Zh);
        let events = collect(|emit| {
            run_offline_turn(&ctx, "我做计算机视觉，2030-03-05 能写完", DEFAULT_TOP, emit)
        });
        let set = events
            .iter()
            .find_map(|e| match e {
                AgentEvent::Recommendations(set) => Some(set.clone()),
                _ => None,
            })
            .unwrap();
        let titles: Vec<&str> = set.items.iter().map(|i| i.facts.title.as_str()).collect();
        assert!(titles.contains(&"CVPR"), "{titles:?}");
        // ACL closes before the ready date.
        assert!(!titles.contains(&"ACL"), "{titles:?}");
        assert!(set.summary.contains("计算机视觉"), "{}", set.summary);
    }
}
