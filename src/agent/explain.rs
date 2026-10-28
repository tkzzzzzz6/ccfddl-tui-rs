//! Offline answers to "why this conference?" questions.
//!
//! Without a model, a follow-up such as "为什么推荐 ACL 2027？" or "Evaluate
//! CVPR 2027 for my research" is answered from the rule engine: the card's
//! reasons, direction and risks, or the filter that kept the conference out
//! of the recommendations.

use super::Lang;
use super::recommend::{self, Request, Tier, facts_for, find_conference};
use super::topics::{subs_for_tags, tags_for_text, venue_tags};
use crate::model::Conference;

const QUESTION_CUES: [&str; 10] = [
    "why",
    "evaluate",
    "fit",
    "worth",
    "explain",
    "为什么",
    "评估",
    "适合",
    "值得",
    "怎么样",
];

/// `true` when the text asks about a conference rather than for new picks.
pub fn is_question(text: &str) -> bool {
    let lower = text.to_lowercase();
    QUESTION_CUES.iter().any(|cue| lower.contains(cue))
}

/// The conference whose title appears in `text` (longest match wins).
pub fn mentioned<'a>(conferences: &'a [Conference], text: &str) -> Option<&'a Conference> {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    let boundary = |i: usize| i >= bytes.len() || !bytes[i].is_ascii_alphanumeric();
    conferences
        .iter()
        .filter(|conf| conf.title.len() >= 2)
        .filter(|conf| {
            let title = conf.title.to_lowercase();
            lower.match_indices(&title).any(|(start, matched)| {
                (start == 0 || boundary(start - 1)) && boundary(start + matched.len())
            })
        })
        .max_by_key(|conf| conf.title.len())
}

/// A local answer for a question about one conference, if the text is one.
pub fn answer(req: &Request, text: &str) -> Option<String> {
    if !is_question(text) {
        return None;
    }
    let conf = mentioned(req.conferences, text)?;
    Some(explain(req, &conf.title))
}

fn days(n: i64, lang: Lang) -> String {
    match lang {
        Lang::Zh => format!("{n} 天"),
        Lang::En => format!("{n} days"),
    }
}

/// Explain how the rules see one conference for the request's profile.
pub fn explain(req: &Request, key: &str) -> String {
    let lang = req.lang;
    let Some(facts) = facts_for(req.conferences, key, req.now) else {
        let title = find_conference(req.conferences, key).map_or(key, |c| c.title.as_str());
        return match lang {
            Lang::Zh => format!("{title} 目前没有开放的截稿（已截止或尚无新一届信息）。"),
            Lang::En => format!("{title} has no open deadline right now."),
        };
    };
    let left = facts
        .days_left(req.now)
        .map(|n| days(n, lang))
        .unwrap_or_else(|| lang.pick("TBD", "待定").to_string());
    let mut lines = vec![match lang {
        Lang::Zh => format!(
            "{} {} · {} · 截稿 {}（还剩 {left}）",
            facts.title,
            facts.year,
            facts.rank_label(),
            facts.deadline_text
        ),
        Lang::En => format!(
            "{} {} · {} · deadline {} ({left} left)",
            facts.title,
            facts.year,
            facts.rank_label(),
            facts.deadline_text
        ),
    }];

    let sep = lang.pick("; ", "；");
    let candidate = recommend::candidates(req)
        .into_iter()
        .find(|c| c.facts.id == facts.id);
    match candidate {
        Some(c) => {
            let tier = match c.tier {
                Tier::Stretch => lang.pick("a stretch pick", "冲刺档"),
                Tier::Main => lang.pick("a main pick", "主投档"),
                Tier::Safe => lang.pick("a safe pick", "保底档"),
            };
            lines.push(match lang {
                Lang::Zh => format!("按你的画像属于{tier}（评分 {}）。", c.score),
                Lang::En => format!("For your profile this is {tier} (score {}).", c.score),
            });
            lines.push(format!(
                "{}{}",
                lang.pick("Why: ", "理由："),
                c.reasons.join(sep)
            ));
            lines.push(format!(
                "{}{}",
                lang.pick("Direction: ", "方向："),
                c.direction
            ));
            if !c.risks.is_empty() {
                lines.push(format!(
                    "{}{}",
                    lang.pick("Risks: ", "风险："),
                    c.risks.join(sep)
                ));
            }
        }
        None => {
            let reasons = exclusions(req, &facts);
            lines.push(match lang {
                Lang::Zh => format!("按你的画像它没有进入推荐：{}。", reasons.join("；")),
                Lang::En => format!(
                    "It is not among your recommendations: {}.",
                    reasons.join("; ")
                ),
            });
        }
    }
    lines.push(
        lang.pick(
            "(Offline rules. Connect a model with /setup for a fuller discussion.)",
            "（离线规则说明。用 /setup 配置模型后可以深入讨论。）",
        )
        .to_string(),
    );
    lines.join("\n")
}

/// Which of the scorer's filters rejected the conference.
fn exclusions(req: &Request, facts: &recommend::ConferenceFacts) -> Vec<String> {
    let lang = req.lang;
    let profile = req.profile;
    let mut out = Vec::new();
    let days_left = facts.days_left(req.now);
    if facts.is_tbd && !profile.include_tbd {
        out.push(
            lang.pick("the deadline is not announced", "截止时间尚未公布")
                .to_string(),
        );
    }
    if let Some(days) = days_left {
        if days > profile.horizon_days {
            out.push(match lang {
                Lang::Zh => format!("截稿在 {} 天之后，超出考虑范围", profile.horizon_days),
                Lang::En => format!(
                    "the deadline is more than {} days away",
                    profile.horizon_days
                ),
            });
        }
        match profile.ready_date {
            Some(ready) if facts.deadline.is_some_and(|d| d.date_naive() < ready) => {
                out.push(match lang {
                    Lang::Zh => format!("截稿早于论文就绪日期 {ready}"),
                    Lang::En => format!("it closes before your paper is ready ({ready})"),
                })
            }
            None if days < profile.min_days_left => out.push(match lang {
                Lang::Zh => format!("距离截稿不足 {} 天", profile.min_days_left),
                Lang::En => format!("fewer than {} days are left", profile.min_days_left),
            }),
            _ => {}
        }
    }
    let rank = |r: &str| match r {
        "A" => 0,
        "B" => 1,
        "C" => 2,
        _ => 3,
    };
    let targets: Vec<i32> = profile.target_ranks.iter().map(|r| rank(r)).collect();
    let worst = targets.iter().copied().max().unwrap_or(1);
    let this = rank(&facts.ccf);
    if this > worst + 1 || (this == 3 && !targets.contains(&3)) {
        out.push(match lang {
            Lang::Zh => format!(
                "等级 CCF-{} 不在目标（CCF {}）附近",
                facts.ccf,
                profile.target_ranks.join("/")
            ),
            Lang::En => format!(
                "CCF-{} is too far from your targets (CCF {})",
                facts.ccf,
                profile.target_ranks.join("/")
            ),
        });
    }
    let tags = tags_for_text(&profile.interests.join(" , "));
    let mut subs: std::collections::BTreeSet<String> = profile.categories.iter().cloned().collect();
    if subs.is_empty() {
        subs = subs_for_tags(&tags);
    }
    let has_topic = !tags.is_empty() || !subs.is_empty();
    if has_topic
        && !subs.contains(&facts.sub)
        && venue_tags(&facts.title, &facts.description).is_disjoint(&tags)
    {
        out.push(match lang {
            Lang::Zh => format!("研究方向与该会议（{} 类）不匹配", facts.sub),
            Lang::En => format!(
                "its topics ({} category) do not match your interests",
                facts.sub
            ),
        });
    }
    if out.is_empty() {
        out.push(
            lang.pick(
                "it scored below the other candidates",
                "综合评分低于其他候选",
            )
            .to_string(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Profile;
    use crate::agent::recommend::tests::{conferences, now};
    use crate::data::Favorites;

    fn ask(profile: &Profile, text: &str, lang: Lang) -> Option<String> {
        let confs = conferences();
        let dir = tempfile::tempdir().unwrap();
        let favorites = Favorites::empty_at(dir.path().join("f.json"));
        answer(
            &Request {
                conferences: &confs,
                favorites: &favorites,
                profile,
                now: now(),
                lang,
            },
            text,
        )
    }

    fn vision() -> Profile {
        Profile {
            interests: vec!["computer vision".into()],
            ..Profile::default()
        }
    }

    #[test]
    fn questions_need_a_cue_and_a_known_title() {
        assert!(is_question("为什么推荐 ACL 2030？"));
        assert!(is_question("Evaluate CVPR for me"));
        assert!(!is_question("multimodal LLM"));
        let confs = conferences();
        assert_eq!(mentioned(&confs, "why acl?").unwrap().title, "ACL");
        // Whole words only: "ICPR" does not count as "CVPR", "ACLX" not "ACL".
        assert!(mentioned(&confs, "why ACLX").is_none());
        assert_eq!(mentioned(&confs, "评估ICPR").unwrap().title, "ICPR");
        assert!(ask(&vision(), "multimodal LLM", Lang::En).is_none());
        assert!(ask(&vision(), "why Nonexistent?", Lang::En).is_none());
    }

    #[test]
    fn recommended_cards_are_explained_with_rules() {
        let text = ask(&vision(), "为什么推荐 CVPR 2030？", Lang::Zh).unwrap();
        assert!(text.contains("CVPR 2030 · CCF-A CORE A* THCPL A"), "{text}");
        assert!(text.contains("冲刺档"), "{text}");
        assert!(text.contains("理由：方向匹配"), "{text}");
        assert!(text.contains("风险："), "{text}");
        assert!(text.contains("/setup"), "{text}");
    }

    #[test]
    fn excluded_conferences_name_the_filter() {
        let text = ask(&vision(), "Evaluate SIGMOD for my research", Lang::En).unwrap();
        assert!(text.contains("not among your recommendations"), "{text}");
        assert!(text.contains("do not match your interests"), "{text}");

        let only_a = Profile {
            target_ranks: vec!["A".into()],
            ..vision()
        };
        let text = ask(&only_a, "why ICPR", Lang::En).unwrap();
        assert!(text.contains("CCF-C is too far"), "{text}");

        let text = ask(&vision(), "why OldConf", Lang::En).unwrap();
        assert!(text.contains("no open deadline"), "{text}");
    }
}
