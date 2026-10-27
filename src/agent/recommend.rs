//! Offline, rule-based conference recommendation.
//!
//! Every conference with an upcoming (or not yet announced) deadline is scored
//! on four signals that exist in the data set:
//!
//! - **topic**: the user's interest tags against the venue's tags and the CCF
//!   category (see [`super::topics`]);
//! - **rank**: the CCF rank against the target ranks (CORE A* adds a little);
//! - **time**: slack between when the paper is ready and the deadline;
//! - **preferences**: favorites and preferred / avoided places.
//!
//! Candidates are then split into stretch / main / safe tiers by rank relative
//! to the targets. Reasons, directions and risks are generated from templates,
//! so this module works without any network access.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, NaiveTime, Utc};

use super::Lang;
use super::profile::Profile;
use super::topics::{self, has_curated_hint, subs_for_tags, tag_name, tags_for_text, venue_tags};
use crate::data::Favorites;
use crate::model::{CATEGORIES, Conference, ConferenceYear, Timeline};
use crate::timezone::parse_deadline_utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Stretch,
    Main,
    Safe,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Stretch, Tier::Main, Tier::Safe];

    pub fn key(self) -> &'static str {
        match self {
            Self::Stretch => "stretch",
            Self::Main => "main",
            Self::Safe => "safe",
        }
    }

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::Stretch => lang.pick("Stretch", "冲刺"),
            Self::Main => lang.pick("Main", "主投"),
            Self::Safe => lang.pick("Safe", "保底"),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "stretch" | "reach" | "冲刺" => Some(Self::Stretch),
            "main" | "target" | "主投" => Some(Self::Main),
            "safe" | "safety" | "backup" | "保底" => Some(Self::Safe),
            _ => None,
        }
    }
}

/// Facts about one conference edition, straight from the data set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConferenceFacts {
    /// Unique key: the title, or `title@SUB` when a title is ambiguous.
    pub id: String,
    pub title: String,
    pub year: i32,
    pub sub: String,
    pub description: String,
    pub ccf: String,
    pub core: Option<String>,
    pub thcpl: Option<String>,
    pub deadline: Option<DateTime<Utc>>,
    /// The deadline as written in the data, with its timezone, or `TBD`.
    pub deadline_text: String,
    pub abstract_deadline: Option<DateTime<Utc>>,
    pub abstract_text: Option<String>,
    pub comment: Option<String>,
    pub place: String,
    pub date: String,
    pub link: String,
    pub is_tbd: bool,
}

impl ConferenceFacts {
    pub fn days_left(&self, now: DateTime<Utc>) -> Option<i64> {
        self.deadline.map(|deadline| (deadline - now).num_days())
    }

    pub fn rank_label(&self) -> String {
        let mut label = format!("CCF-{}", self.ccf);
        if let Some(core) = &self.core {
            label.push_str(&format!(" CORE {core}"));
        }
        if let Some(thcpl) = &self.thcpl {
            label.push_str(&format!(" THCPL {thcpl}"));
        }
        label
    }

    pub fn category_name(&self, lang: Lang) -> &'static str {
        CATEGORIES
            .iter()
            .find(|c| c.sub == self.sub)
            .map_or("", |c| lang.pick(c.name_en, c.name))
    }
}

/// A scored candidate with template-generated explanations.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub facts: ConferenceFacts,
    pub score: f64,
    pub tier: Tier,
    /// Interest tags that the venue also covers.
    pub matched_tags: Vec<&'static str>,
    pub venue_tags: Vec<&'static str>,
    /// Days between "paper ready" and the deadline.
    pub slack_days: Option<i64>,
    pub is_favorite: bool,
    pub reasons: Vec<String>,
    pub direction: String,
    pub risks: Vec<String>,
}

/// One recommendation shown to the user.
#[derive(Debug, Clone, PartialEq)]
pub struct Recommendation {
    pub facts: ConferenceFacts,
    pub tier: Tier,
    pub score: Option<f64>,
    pub reason: String,
    pub direction: String,
    pub risks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Rules,
    Llm(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecommendationSet {
    pub summary: String,
    pub items: Vec<Recommendation>,
    pub source: Source,
    pub generated_at: DateTime<Utc>,
}

/// Everything the scorer needs, borrowed from the caller.
pub struct Request<'a> {
    pub conferences: &'a [Conference],
    pub favorites: &'a Favorites,
    pub profile: &'a Profile,
    pub now: DateTime<Utc>,
    pub lang: Lang,
}

fn rank_order(rank: &str) -> i32 {
    match rank {
        "A" => 0,
        "B" => 1,
        "C" => 2,
        _ => 3,
    }
}

/// Titles that occur in more than one category need a qualified id.
fn ambiguous_titles(conferences: &[Conference]) -> BTreeSet<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for conf in conferences {
        *counts.entry(conf.title.to_lowercase()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(title, _)| title)
        .collect()
}

fn conference_id(conf: &Conference, ambiguous: &BTreeSet<String>) -> String {
    if ambiguous.contains(&conf.title.to_lowercase()) {
        format!("{}@{}", conf.title, conf.sub)
    } else {
        conf.title.clone()
    }
}

/// Find a conference by id, title (case-insensitive) or `title@SUB`.
pub fn find_conference<'a>(conferences: &'a [Conference], key: &str) -> Option<&'a Conference> {
    let key = key.trim();
    let (title, sub) = match key.rsplit_once('@') {
        Some((title, sub)) => (title.trim(), Some(sub.trim())),
        None => (key, None),
    };
    let mut matches = conferences.iter().filter(|conf| {
        conf.title.eq_ignore_ascii_case(title)
            && sub.is_none_or(|sub| conf.sub.eq_ignore_ascii_case(sub))
    });
    let first = matches.next()?;
    // Prefer an edition that is still open when the title is ambiguous.
    let now = Utc::now();
    if upcoming_edition(first, now).is_some() {
        return Some(first);
    }
    matches
        .find(|conf| upcoming_edition(conf, now).is_some())
        .or(Some(first))
}

/// An edition, the timeline entry that is still open, and its parsed
/// deadline (`None` for TBD).
pub type NextDeadline<'a> = (
    &'a ConferenceYear,
    Option<&'a Timeline>,
    Option<DateTime<Utc>>,
);

/// The next open deadline of a conference: the earliest future deadline across
/// all editions, otherwise a TBD timeline of a current or future edition.
pub fn upcoming_edition(conf: &Conference, now: DateTime<Utc>) -> Option<NextDeadline<'_>> {
    let mut best: Option<(&ConferenceYear, &Timeline, DateTime<Utc>)> = None;
    let mut tbd: Option<(&ConferenceYear, &Timeline)> = None;
    for year in &conf.confs {
        for timeline in &year.timeline {
            if timeline.deadline.eq_ignore_ascii_case("TBD") {
                if year.year >= now.year() && tbd.is_none() {
                    tbd = Some((year, timeline));
                }
                continue;
            }
            let Ok(deadline) = parse_deadline_utc(&timeline.deadline, &year.timezone) else {
                continue;
            };
            if deadline > now && best.is_none_or(|(_, _, b)| deadline < b) {
                best = Some((year, timeline, deadline));
            }
        }
    }
    match (best, tbd) {
        (Some((year, timeline, deadline)), _) => Some((year, Some(timeline), Some(deadline))),
        (None, Some((year, timeline))) => Some((year, Some(timeline), None)),
        (None, None) => None,
    }
}

fn build_facts(
    conf: &Conference,
    ambiguous: &BTreeSet<String>,
    now: DateTime<Utc>,
) -> Option<ConferenceFacts> {
    let (year, timeline, deadline) = upcoming_edition(conf, now)?;
    let is_tbd = deadline.is_none();
    let deadline_text = match (timeline, deadline) {
        (Some(t), Some(_)) => format!("{} ({})", t.deadline, year.timezone),
        _ => "TBD".to_string(),
    };
    let abstract_text = timeline.and_then(|t| t.abstract_deadline.clone());
    let abstract_deadline = abstract_text
        .as_deref()
        .and_then(|text| parse_deadline_utc(text, &year.timezone).ok());
    Some(ConferenceFacts {
        id: conference_id(conf, ambiguous),
        title: conf.title.clone(),
        year: year.year,
        sub: conf.sub.clone(),
        description: conf.description.clone(),
        ccf: conf.rank.ccf.clone(),
        core: conf.rank.core.clone(),
        thcpl: conf.rank.thcpl.clone(),
        deadline,
        deadline_text,
        abstract_deadline,
        abstract_text: abstract_text.map(|text| format!("{text} ({})", year.timezone)),
        comment: timeline.and_then(|t| t.comment.clone()),
        place: year.place.clone(),
        date: year.date.clone(),
        link: year.link.clone(),
        is_tbd,
    })
}

/// Facts for a conference key, if it still has an open deadline.
pub fn facts_for(
    conferences: &[Conference],
    key: &str,
    now: DateTime<Utc>,
) -> Option<ConferenceFacts> {
    let conf = find_conference(conferences, key)?;
    build_facts(conf, &ambiguous_titles(conferences), now)
}

/// All conferences with an open deadline, as facts (for search tools).
pub fn open_conferences(conferences: &[Conference], now: DateTime<Utc>) -> Vec<ConferenceFacts> {
    let ambiguous = ambiguous_titles(conferences);
    conferences
        .iter()
        .filter_map(|conf| build_facts(conf, &ambiguous, now))
        .collect()
}

/// What the user wants, derived once from the profile.
struct Wants {
    tags: BTreeSet<&'static str>,
    subs: BTreeSet<String>,
    targets: Vec<i32>,
    has_topic: bool,
}

impl Wants {
    fn from_profile(profile: &Profile) -> Self {
        let tags = tags_for_text(&profile.interests.join(" , "));
        let mut subs: BTreeSet<String> = profile.categories.iter().cloned().collect();
        if subs.is_empty() {
            subs = subs_for_tags(&tags);
        }
        let mut targets: Vec<i32> = profile.target_ranks.iter().map(|r| rank_order(r)).collect();
        if targets.is_empty() {
            targets = vec![0, 1];
        }
        targets.sort_unstable();
        targets.dedup();
        let has_topic = !tags.is_empty() || !subs.is_empty();
        Self {
            tags,
            subs,
            targets,
            has_topic,
        }
    }

    fn best(&self) -> i32 {
        self.targets[0]
    }

    fn worst(&self) -> i32 {
        *self.targets.last().unwrap_or(&1)
    }

    fn tier(&self, rank: i32) -> Tier {
        if rank < self.best() || (rank == self.best() && self.targets.len() > 1) {
            Tier::Stretch
        } else if self.targets.contains(&rank) {
            Tier::Main
        } else {
            Tier::Safe
        }
    }
}

fn place_matches(place: &str, keywords: &[String]) -> Option<String> {
    let lower = place.to_lowercase();
    keywords
        .iter()
        .find(|k| !k.trim().is_empty() && lower.contains(&k.trim().to_lowercase()))
        .cloned()
}

/// Score every open conference. Returned in descending score order.
pub fn candidates(req: &Request) -> Vec<Candidate> {
    let wants = Wants::from_profile(req.profile);
    let profile = req.profile;
    let ready_start = profile
        .ready_date
        .map(|d| d.and_time(NaiveTime::MIN).and_utc())
        .filter(|ready| *ready > req.now)
        .unwrap_or(req.now);

    let mut out: Vec<Candidate> = open_conferences(req.conferences, req.now)
        .into_iter()
        .filter_map(|facts| score(req, &wants, ready_start, facts))
        .collect();
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.facts.deadline.cmp(&b.facts.deadline))
            .then_with(|| a.facts.title.cmp(&b.facts.title))
    });
    out
}

fn score(
    req: &Request,
    wants: &Wants,
    ready_start: DateTime<Utc>,
    facts: ConferenceFacts,
) -> Option<Candidate> {
    let profile = req.profile;
    let lang = req.lang;

    // --- Hard filters -----------------------------------------------------
    if facts.is_tbd && !profile.include_tbd {
        return None;
    }
    let days_left = facts.days_left(req.now);
    let slack_days = facts.deadline.map(|d| (d - ready_start).num_days());
    if let Some(days) = days_left {
        if days > profile.horizon_days {
            return None;
        }
        if profile.ready_date.is_some() {
            if slack_days.is_some_and(|slack| slack < 0) {
                return None;
            }
        } else if days < profile.min_days_left {
            return None;
        }
    }
    let rank = rank_order(&facts.ccf);
    if rank > wants.worst() + 1 || (rank == 3 && !wants.targets.contains(&3)) {
        return None;
    }

    // --- Topic ------------------------------------------------------------
    let vtags = venue_tags(&facts.title, &facts.description);
    let matched: Vec<&'static str> = wants.tags.intersection(&vtags).copied().collect();
    let sub_match = wants.subs.contains(&facts.sub);
    let topic = if !wants.has_topic {
        0.5
    } else {
        let mut t = if sub_match { 0.5 } else { 0.0 };
        if sub_match
            && matched.is_empty()
            && has_curated_hint(&facts.title)
            && !wants.tags.is_empty()
        {
            // A well-known venue whose focus is elsewhere in the category.
            t = 0.3;
        }
        t + (matched.len() as f64 * 0.25).min(0.5)
    };
    if wants.has_topic && topic <= 0.0 {
        return None;
    }

    // --- Rank -------------------------------------------------------------
    let mut rank_score = if wants.targets.contains(&rank) {
        1.0
    } else if rank < wants.best() {
        0.8
    } else {
        0.5
    };
    if facts.core.as_deref() == Some("A*") {
        rank_score += 0.05;
    }

    // --- Time -------------------------------------------------------------
    let time = match slack_days {
        None => 0.5,
        Some(s) if s < 7 => 0.4,
        Some(s) if s <= 150 => 1.0,
        Some(s) => 1.0 - 0.5 * ((s - 150) as f64 / 215.0).min(1.0),
    };

    // --- Preferences ------------------------------------------------------
    let is_favorite = req.favorites.contains(&facts.title, facts.year);
    let preferred = place_matches(&facts.place, &profile.preferred_places);
    let avoided = place_matches(&facts.place, &profile.avoided_places);
    let mut total = 100.0 * (0.5 * topic.min(1.0) + 0.3 * rank_score + 0.2 * time);
    if is_favorite {
        total += 5.0;
    }
    if preferred.is_some() {
        total += 5.0;
    }
    if avoided.is_some() {
        total -= 20.0;
    }

    let tier = wants.tier(rank);
    let mut vtags_sorted: Vec<&'static str> = vtags.into_iter().collect();
    vtags_sorted.sort_unstable();

    // --- Explanations -----------------------------------------------------
    let mut reasons = Vec::new();
    let cat = facts.category_name(lang);
    if !matched.is_empty() {
        let names: Vec<&str> = matched.iter().map(|t| tag_name(t, lang.is_zh())).collect();
        reasons.push(match lang {
            Lang::Zh => format!("方向匹配：{}（{} 类）", names.join("、"), facts.sub),
            Lang::En => format!("Topic match: {} ({} category)", names.join(", "), facts.sub),
        });
    } else if sub_match {
        reasons.push(match lang {
            Lang::Zh => format!("属于你关注的「{cat}」类（{}）", facts.sub),
            Lang::En => format!("In your category {} ({cat})", facts.sub),
        });
    }
    let rank_note = match tier {
        Tier::Stretch => lang.pick(
            "above or at the top of your target, worth a reach",
            "达到或高于目标上限，适合冲刺",
        ),
        Tier::Main => lang.pick("matches your target", "符合目标等级"),
        Tier::Safe => lang.pick(
            "below your target, a safer option",
            "低于目标等级，作为保底",
        ),
    };
    reasons.push(format!(
        "{}{}{rank_note}",
        facts.rank_label(),
        lang.pick(": ", "：")
    ));
    match (days_left, slack_days) {
        (Some(days), Some(slack)) if profile.ready_date.is_some() => reasons.push(match lang {
            Lang::Zh => format!(
                "截稿 {}，还剩 {days} 天，论文就绪后余 {slack} 天",
                facts.deadline_text
            ),
            Lang::En => format!(
                "Deadline {}, {days} days left, {slack} days after your paper is ready",
                facts.deadline_text
            ),
        }),
        (Some(days), _) => reasons.push(match lang {
            Lang::Zh => format!("截稿 {}，还剩 {days} 天", facts.deadline_text),
            Lang::En => format!("Deadline {}, {days} days left", facts.deadline_text),
        }),
        _ => {}
    }
    if is_favorite {
        reasons.push(
            lang.pick("Already in your favorites", "已在收藏中")
                .to_string(),
        );
    }
    if let Some(keyword) = &preferred {
        reasons.push(match lang {
            Lang::Zh => format!("地点 {} 符合偏好「{keyword}」", facts.place),
            Lang::En => format!("{} matches preferred place \"{keyword}\"", facts.place),
        });
    }

    let direction = direction(lang, tier, &matched, &vtags_sorted, &wants.tags, cat);

    let mut risks = Vec::new();
    if facts.is_tbd {
        risks.push(
            lang.pick(
                "Deadline not announced yet; watch the website",
                "截止时间尚未公布，需关注官网",
            )
            .to_string(),
        );
    }
    if let Some(slack) = slack_days
        && slack < 14
    {
        risks.push(match lang {
            Lang::Zh => format!("时间紧：留给写作和实验的时间只有 {slack} 天"),
            Lang::En => format!("Tight schedule: only {slack} days to finish"),
        });
    }
    if let (Some(abs), Some(text)) = (facts.abstract_deadline, &facts.abstract_text)
        && facts.deadline.is_some_and(|d| abs < d)
        && abs > req.now
    {
        risks.push(match lang {
            Lang::Zh => format!("需先在 {text} 前提交摘要"),
            Lang::En => format!("Abstract due first: {text}"),
        });
    }
    if let Some(keyword) = &avoided {
        risks.push(match lang {
            Lang::Zh => format!("地点 {} 命中你要避开的「{keyword}」", facts.place),
            Lang::En => format!("{} matches avoided place \"{keyword}\"", facts.place),
        });
    }
    if let Some(comment) = &facts.comment {
        risks.push(match lang {
            Lang::Zh => format!("备注：{comment}"),
            Lang::En => format!("Note: {comment}"),
        });
    }

    Some(Candidate {
        facts,
        score: (total * 10.0).round() / 10.0,
        tier,
        matched_tags: matched,
        venue_tags: vtags_sorted,
        slack_days,
        is_favorite,
        reasons,
        direction,
        risks,
    })
}

fn direction(
    lang: Lang,
    tier: Tier,
    matched: &[&'static str],
    venue: &[&'static str],
    wanted: &BTreeSet<&'static str>,
    category: &str,
) -> String {
    let zh = lang.is_zh();
    let names = |tags: &[&'static str]| -> Vec<&'static str> {
        tags.iter().map(|t| tag_name(t, zh)).collect()
    };
    let sep = lang.pick(", ", "、");
    let focus = if !matched.is_empty() {
        match lang {
            Lang::Zh => format!("以「{}」为主线", names(matched).join(sep)),
            Lang::En => format!("Lead with {}", names(matched).join(sep)),
        }
    } else if let Some(first) = wanted.iter().next() {
        match lang {
            Lang::Zh => format!("从「{}」与{category}的交叉切入", tag_name(first, zh)),
            Lang::En => format!(
                "Frame {} as a contribution to {category}",
                tag_name(first, zh)
            ),
        }
    } else {
        lang.pick("Pick the strongest result you have", "选择你最有把握的成果")
            .to_string()
    };
    let venue_note = if venue.is_empty() {
        String::new()
    } else {
        match lang {
            Lang::Zh => format!("（该会关注：{}）", names(venue).join(sep)),
            Lang::En => format!(" (venue covers: {})", names(venue).join(sep)),
        }
    };
    let tier_note = match tier {
        Tier::Stretch => lang.pick(
            "needs a clear novelty claim and thorough comparisons",
            "需要明确的创新点和充分的对比实验",
        ),
        Tier::Main => lang.pick(
            "highlight the method contribution and solid baselines",
            "突出方法贡献并补齐主流基线",
        ),
        Tier::Safe => lang.pick(
            "an application, system or extended-experiment paper fits well",
            "可侧重应用落地、系统实现或扩展实验",
        ),
    };
    match lang {
        Lang::Zh => format!("{focus}{venue_note}；{tier_note}"),
        Lang::En => format!("{focus}{venue_note}; {tier_note}"),
    }
}

/// Keep a mix of tiers: roughly 30% stretch, 40% main, 30% safe, filling any
/// gap with the next best candidates.
pub fn select(candidates: Vec<Candidate>, top: usize) -> Vec<Candidate> {
    let top = top.max(1);
    let caps: BTreeMap<Tier, usize> = [
        (Tier::Stretch, (top * 3).div_ceil(10).max(1)),
        (Tier::Main, (top * 4).div_ceil(10).max(1)),
        (Tier::Safe, (top * 3).div_ceil(10).max(1)),
    ]
    .into_iter()
    .collect();
    let mut counts: BTreeMap<Tier, usize> = BTreeMap::new();
    let mut picked = Vec::new();
    let mut rest = Vec::new();
    for candidate in candidates {
        let count = counts.entry(candidate.tier).or_default();
        if picked.len() < top && *count < caps[&candidate.tier] {
            *count += 1;
            picked.push(candidate);
        } else {
            rest.push(candidate);
        }
    }
    for candidate in rest {
        if picked.len() >= top {
            break;
        }
        picked.push(candidate);
    }
    picked.sort_by(|a, b| a.tier.cmp(&b.tier).then(b.score.total_cmp(&a.score)));
    picked
}

impl Candidate {
    pub fn into_recommendation(self, lang: Lang) -> Recommendation {
        Recommendation {
            facts: self.facts,
            tier: self.tier,
            score: Some(self.score),
            reason: self.reasons.join(lang.pick("; ", "；")),
            direction: self.direction,
            risks: self.risks,
        }
    }
}

/// The complete offline recommendation.
pub fn recommend(req: &Request, top: usize) -> RecommendationSet {
    let all = candidates(req);
    let total = all.len();
    let picked = select(all, top);
    let lang = req.lang;
    let tags = tags_for_text(&req.profile.interests.join(" , "));
    let tag_names: Vec<&str> = tags
        .iter()
        .map(|t| topics::tag_name(t, lang.is_zh()))
        .collect();
    let summary = if picked.is_empty() {
        lang.pick(
            "No open conference matches the profile. Try more interests, wider target ranks or a longer horizon.",
            "没有符合画像的可投会议。可以补充研究方向、放宽目标等级或延长考虑范围。",
        )
        .to_string()
    } else if tag_names.is_empty() {
        match lang {
            Lang::Zh => format!(
                "未识别到研究方向，以下按等级和时间从 {total} 个可投会议中挑选。用 /init 或 profile.yaml 补充方向可获得更准的推荐。"
            ),
            Lang::En => format!(
                "No research topic recognised; picked from {total} open conferences by rank and timing. Add interests with /init or profile.yaml for better results."
            ),
        }
    } else {
        match lang {
            Lang::Zh => format!(
                "根据方向「{}」从 {total} 个候选会议中挑选，按冲刺 / 主投 / 保底分档。",
                tag_names.join("、")
            ),
            Lang::En => format!(
                "Picked from {total} candidates for {}, grouped into stretch / main / safe.",
                tag_names.join(", ")
            ),
        }
    };
    RecommendationSet {
        summary,
        items: picked
            .into_iter()
            .map(|c| c.into_recommendation(lang))
            .collect(),
        source: Source::Rules,
        generated_at: req.now,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::data::source::parse_conferences;
    use chrono::{NaiveDate, TimeZone};

    pub(crate) const YAML: &str = "
- title: CVPR
  description: IEEE/CVF Conference on Computer Vision and Pattern Recognition
  sub: AI
  rank: {ccf: A, core: 'A*', thcpl: A}
  confs:
  - year: 2030
    timezone: UTC-12
    place: Seattle, USA
    date: June 2030
    link: https://cvpr.example
    timeline:
    - deadline: '2030-03-10 23:59:59'
      abstract_deadline: '2030-03-01 23:59:59'
- title: ICME
  description: The IEEE International Conference on Multimedia & Expo
  sub: CG
  rank: {ccf: B}
  confs:
  - year: 2030
    timezone: UTC
    place: Beijing, China
    timeline:
    - deadline: '2030-04-01 00:00:00'
- title: ACL
  description: Annual Meeting of the Association for Computational Linguistics
  sub: AI
  rank: {ccf: A}
  confs:
  - year: 2030
    timezone: UTC
    timeline:
    - deadline: '2030-02-10 00:00:00'
- title: ICPR
  description: International Conference on Pattern Recognition
  sub: AI
  rank: {ccf: C}
  confs:
  - year: 2030
    timezone: UTC
    timeline:
    - deadline: '2030-05-01 00:00:00'
- title: SIGMOD
  description: ACM Conference on Management of Data
  sub: DB
  rank: {ccf: A}
  confs:
  - year: 2030
    timezone: UTC
    timeline:
    - deadline: '2030-03-01 00:00:00'
- title: OldConf
  description: Old
  sub: AI
  rank: {ccf: A}
  confs:
  - year: 2020
    timezone: UTC
    timeline:
    - deadline: '2020-01-01 00:00:00'
- title: SomeNConf
  description: Vision workshop
  sub: AI
  rank: {ccf: N}
  confs:
  - year: 2030
    timezone: UTC
    timeline:
    - deadline: '2030-03-05 00:00:00'
";

    pub(crate) fn conferences() -> Vec<Conference> {
        parse_conferences(YAML, "test").unwrap()
    }

    pub(crate) fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()
    }

    fn favorites() -> Favorites {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("favorites.json");
        std::mem::forget(dir);
        Favorites::empty_at(path)
    }

    fn run(profile: &Profile, lang: Lang) -> Vec<Candidate> {
        let confs = conferences();
        let favorites = favorites();
        candidates(&Request {
            conferences: &confs,
            favorites: &favorites,
            profile,
            now: now(),
            lang,
        })
    }

    fn vision_profile() -> Profile {
        Profile {
            interests: vec!["多模态".into(), "computer vision".into()],
            ..Profile::default()
        }
    }

    #[test]
    fn topic_match_ranks_vision_venues_first() {
        let list = run(&vision_profile(), Lang::En);
        let titles: Vec<&str> = list.iter().map(|c| c.facts.title.as_str()).collect();
        assert_eq!(titles[0], "CVPR", "{titles:?}");
        assert!(titles.contains(&"ICME"));
        // Database venues are out of scope, expired and CCF-N venues dropped.
        assert!(!titles.contains(&"SIGMOD"));
        assert!(!titles.contains(&"OldConf"));
        assert!(!titles.contains(&"SomeNConf"));
    }

    #[test]
    fn tiers_follow_target_ranks() {
        let list = run(&vision_profile(), Lang::En);
        let tier = |title: &str| list.iter().find(|c| c.facts.title == title).unwrap().tier;
        assert_eq!(tier("CVPR"), Tier::Stretch);
        assert_eq!(tier("ICME"), Tier::Main);
        assert_eq!(tier("ICPR"), Tier::Safe);

        let only_a = Profile {
            target_ranks: vec!["A".into()],
            ..vision_profile()
        };
        let list = run(&only_a, Lang::En);
        assert_eq!(
            list.iter().find(|c| c.facts.title == "CVPR").unwrap().tier,
            Tier::Main
        );
        // C is two levels below A and is no longer offered.
        assert!(list.iter().all(|c| c.facts.title != "ICPR"));
    }

    #[test]
    fn ready_date_skips_deadlines_before_it() {
        let profile = Profile {
            ready_date: NaiveDate::from_ymd_opt(2030, 3, 5),
            ..vision_profile()
        };
        let list = run(&profile, Lang::En);
        let titles: Vec<&str> = list.iter().map(|c| c.facts.title.as_str()).collect();
        assert!(!titles.contains(&"ACL"), "{titles:?}");
        let cvpr = list.iter().find(|c| c.facts.title == "CVPR").unwrap();
        assert_eq!(cvpr.slack_days, Some(6));
        assert!(cvpr.risks.iter().any(|r| r.contains("Tight")));
    }

    #[test]
    fn explanations_cite_data_facts_in_both_languages() {
        let list = run(&vision_profile(), Lang::Zh);
        let cvpr = list.iter().find(|c| c.facts.title == "CVPR").unwrap();
        let text = cvpr.reasons.join("|");
        assert!(text.contains("CCF-A CORE A*"), "{text}");
        assert!(text.contains("2030-03-10 23:59:59 (UTC-12)"), "{text}");
        assert!(text.contains("计算机视觉"), "{text}");
        assert!(
            cvpr.risks.iter().any(|r| r.contains("摘要")),
            "{:?}",
            cvpr.risks
        );
        assert!(cvpr.direction.contains("冲刺") || cvpr.direction.contains("创新"));
    }

    #[test]
    fn places_adjust_the_score() {
        let base = run(&vision_profile(), Lang::En);
        let icme =
            |list: &[Candidate]| list.iter().find(|c| c.facts.title == "ICME").unwrap().score;
        let preferred = Profile {
            preferred_places: vec!["china".into()],
            ..vision_profile()
        };
        let avoided = Profile {
            avoided_places: vec!["Beijing".into()],
            ..vision_profile()
        };
        assert!(icme(&run(&preferred, Lang::En)) > icme(&base));
        assert!(icme(&run(&avoided, Lang::En)) < icme(&base));
    }

    #[test]
    fn empty_profile_still_recommends_by_rank_and_time() {
        let list = run(&Profile::default(), Lang::En);
        assert!(list.iter().any(|c| c.facts.title == "SIGMOD"));
    }

    #[test]
    fn select_mixes_tiers() {
        let list = run(&vision_profile(), Lang::En);
        let picked = select(list, 3);
        let tiers: Vec<Tier> = picked.iter().map(|c| c.tier).collect();
        assert_eq!(tiers, [Tier::Stretch, Tier::Main, Tier::Safe]);
    }

    #[test]
    fn recommend_builds_a_summary() {
        let confs = conferences();
        let favorites = favorites();
        let profile = vision_profile();
        let set = recommend(
            &Request {
                conferences: &confs,
                favorites: &favorites,
                profile: &profile,
                now: now(),
                lang: Lang::En,
            },
            5,
        );
        assert_eq!(set.source, Source::Rules);
        assert!(set.summary.contains("computer vision"), "{}", set.summary);
        assert!(!set.items.is_empty());
    }

    #[test]
    fn find_conference_is_case_insensitive_and_supports_sub() {
        let confs = conferences();
        assert_eq!(find_conference(&confs, "cvpr").unwrap().title, "CVPR");
        assert_eq!(find_conference(&confs, "ICME@cg").unwrap().title, "ICME");
        assert!(find_conference(&confs, "nope").is_none());
        assert!(facts_for(&confs, "OldConf", now()).is_none());
    }
}
