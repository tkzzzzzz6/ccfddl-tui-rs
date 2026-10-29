//! Offline follow-ups: conditions picked out of free text and kept across
//! turns ("只要 A 类", "多推荐几个", "排除 ICCV", "不要 TBD", "只看国内",
//! "半年内", ...).
//!
//! With a model the conversation itself carries this context; offline, each
//! request is rebuilt from the profile plus these accumulated [`Constraints`].

use chrono::NaiveDate;

use super::Lang;
use super::profile::{Profile, parse_date};
use super::recommend::ConferenceFacts;
use super::topics::tags_for_text;
use crate::model::{CATEGORIES, Conference};

/// Default number of offline picks.
pub const DEFAULT_TOP: usize = super::session::DEFAULT_TOP;
const MAX_TOP: usize = 30;
const MORE_STEP: usize = 4;

/// Conditions accumulated over offline turns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Constraints {
    /// Research description from the user, added to the profile interests.
    pub interests: Option<String>,
    pub ready_date: Option<NaiveDate>,
    /// Only these CCF ranks (A..N); empty keeps the profile's targets.
    pub only_ranks: Vec<String>,
    /// Only these CCF categories (`AI`, `DB`, ...).
    pub categories: Vec<String>,
    pub top: Option<usize>,
    /// Titles that must not be recommended.
    pub excluded: Vec<String>,
    pub no_tbd: bool,
    pub preferred_places: Vec<String>,
    pub avoided_places: Vec<String>,
    pub horizon_days: Option<i64>,
}

const NEGATIONS: [&str; 17] = [
    "不要",
    "去掉",
    "排除",
    "除了",
    "不考虑",
    "别推荐",
    "别给",
    "不去",
    "避开",
    "不想去",
    "exclude",
    "without",
    "except",
    "avoid",
    "no ",
    "not ",
    "drop ",
];
const ONLY: [&str; 9] = [
    "只要",
    "只看",
    "只推荐",
    "仅",
    "只",
    "only",
    "just",
    "限定",
    "limit to",
];
const RESEARCH: [&str; 9] = [
    "我做",
    "我的方向",
    "研究",
    "方向是",
    "working on",
    "work on",
    "my research",
    "i do ",
    "my topic",
];

/// Regions: Chinese name, English name, aliases, keywords of the data's
/// `place` field.
pub type Region = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

const PLACES: [Region; 10] = [
    (
        "国内",
        "China",
        &["国内", "中国", "china"],
        &["China", "Hong Kong", "Macau", "Macao"],
    ),
    (
        "美国",
        "the US",
        &["美国", "the us", "usa", "united states", "america"],
        &["USA", "United States", "U.S.A"],
    ),
    (
        "亚洲",
        "Asia",
        &["亚洲", "asia"],
        &[
            "China",
            "Japan",
            "Korea",
            "Singapore",
            "Hong Kong",
            "Taiwan",
            "Macau",
            "Thailand",
            "Vietnam",
            "India",
            "Malaysia",
            "Indonesia",
        ],
    ),
    (
        "欧洲",
        "Europe",
        &["欧洲", "europe"],
        &[
            "France",
            "Germany",
            "Italy",
            "Spain",
            "Portugal",
            "Netherlands",
            "Belgium",
            "Austria",
            "Switzerland",
            "Sweden",
            "Norway",
            "Denmark",
            "Finland",
            "Ireland",
            "UK",
            "United Kingdom",
            "Greece",
            "Czech",
            "Poland",
            "Hungary",
        ],
    ),
    ("日本", "Japan", &["日本", "japan"], &["Japan"]),
    ("加拿大", "Canada", &["加拿大", "canada"], &["Canada"]),
    (
        "北美",
        "North America",
        &["北美", "north america"],
        &["USA", "United States", "U.S.A", "Canada", "Mexico"],
    ),
    (
        "新加坡",
        "Singapore",
        &["新加坡", "singapore"],
        &["Singapore"],
    ),
    ("韩国", "Korea", &["韩国", "korea"], &["Korea"]),
    (
        "澳新",
        "Australia / NZ",
        &["澳洲", "澳大利亚", "新西兰", "australia", "new zealand"],
        &["Australia", "New Zealand"],
    ),
];

/// Known regions, for the profile form's place choices.
pub fn regions() -> &'static [Region] {
    &PLACES
}

/// Category aliases (codes are matched as whole words separately).
const CATEGORY_ALIASES: [(&str, &[&str]); 10] = [
    (
        "DS",
        &["体系结构", "并行", "分布式", "存储", "architecture"],
    ),
    ("NW", &["计算机网络", "networking"]),
    ("SC", &["信息安全", "安全", "security"]),
    ("SE", &["软件工程", "software engineering"]),
    ("DB", &["数据库", "数据挖掘", "database", "data mining"]),
    ("CT", &["理论", "theory"]),
    ("CG", &["图形", "多媒体", "graphics", "multimedia"]),
    ("AI", &["人工智能", "artificial intelligence"]),
    ("HI", &["人机交互", "普适计算", "hci", "human-computer"]),
    ("MX", &["交叉", "综合", "interdisciplinary"]),
];

fn lower(text: &str) -> String {
    text.to_lowercase()
}

/// Split text into clauses so a negation only affects its own clause.
fn clauses(text: &str) -> Vec<String> {
    lower(text)
        .split(['，', ',', '。', ';', '；', '!', '！', '?', '？', '\n'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn has_any(text: &str, cues: &[&str]) -> bool {
    cues.iter().any(|cue| text.contains(cue))
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// `needle` (lowercase) occurs in `hay` as a whole ASCII word.
fn has_word(hay: &str, needle: &str) -> bool {
    let bytes = hay.as_bytes();
    hay.match_indices(needle).any(|(start, m)| {
        let end = start + m.len();
        (start == 0 || !is_word_byte(bytes[start - 1]))
            && (end >= bytes.len() || !is_word_byte(bytes[end]))
    })
}

fn chinese_number(text: &str) -> Option<usize> {
    const DIGITS: [(&str, usize); 11] = [
        ("十", 10),
        ("一", 1),
        ("两", 2),
        ("二", 2),
        ("三", 3),
        ("四", 4),
        ("五", 5),
        ("六", 6),
        ("七", 7),
        ("八", 8),
        ("九", 9),
    ];
    DIGITS
        .iter()
        .find(|(c, _)| text.starts_with(c))
        .map(|(_, n)| *n)
}

/// The number written right before `unit` (digits or a Chinese numeral).
fn number_before(clause: &str, unit: &str) -> Option<usize> {
    let (before, _) = clause.split_once(unit)?;
    let before = before.trim_end();
    let digits: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if !digits.is_empty() {
        return digits.parse().ok();
    }
    let last = before.chars().last()?;
    chinese_number(&last.to_string())
}

/// The number written right after `word` ("top 10", "within 60 days").
fn number_after(clause: &str, word: &str) -> Option<usize> {
    let (_, after) = clause.split_once(word)?;
    let digits: String = after
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// CCF ranks named in a clause: `A类`, `A/B 类`, `CCF-A`, `rank B`.
fn ranks_in(clause: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut add = |r: char| {
        let r = r.to_ascii_uppercase().to_string();
        if !found.contains(&r) {
            found.push(r);
        }
    };
    let chars: Vec<char> = clause.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c != '类' {
            continue;
        }
        // Walk back over letters and separators: "a/b 类", "a、b类".
        let mut j = i;
        let mut letters = Vec::new();
        while j > 0 {
            let p = chars[j - 1];
            if matches!(p, 'a' | 'b' | 'c') {
                // Only a standalone letter, not the end of a word.
                if j >= 2 && chars[j - 2].is_ascii_alphanumeric() {
                    break;
                }
                letters.push(p);
            } else if !matches!(p, ' ' | '/' | '、' | '和' | '或' | '+' | '-') {
                break;
            }
            j -= 1;
        }
        letters.into_iter().rev().for_each(&mut add);
    }
    for prefix in ["ccf-", "ccf ", "rank ", "class "] {
        for (start, m) in clause.match_indices(prefix) {
            if start > 0 && is_word_byte(clause.as_bytes()[start - 1]) {
                continue;
            }
            let rest = &clause[start + m.len()..];
            let mut it = rest.chars();
            if let Some(r @ ('a' | 'b' | 'c')) = it.next()
                && !it.next().is_some_and(|c| c.is_ascii_alphanumeric())
            {
                add(r);
            }
        }
    }
    found.sort();
    found
}

fn categories_in(clause: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for category in CATEGORIES {
        let code = category.sub.to_lowercase();
        let aliases = CATEGORY_ALIASES
            .iter()
            .find(|(sub, _)| *sub == category.sub)
            .map_or(&[][..], |(_, a)| *a);
        if has_word(clause, &code) || aliases.iter().any(|a| clause.contains(a)) {
            found.push(category.sub.to_string());
        }
    }
    found
}

fn places_in(clause: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, _, aliases, keywords) in PLACES {
        let hit = aliases.iter().any(|alias| {
            if alias.is_ascii() {
                has_word(clause, alias.trim())
            } else {
                clause.contains(alias)
            }
        });
        if hit {
            for k in keywords {
                if !out.iter().any(|o| o == k) {
                    out.push(k.to_string());
                }
            }
        }
    }
    out
}

fn titles_in<'a>(clause: &str, conferences: &'a [Conference]) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for conf in conferences {
        let title = conf.title.to_lowercase();
        if title.len() >= 2 && has_word(clause, &title) && !out.contains(&conf.title.as_str()) {
            out.push(&conf.title);
        }
    }
    out
}

fn horizon_in(clause: &str) -> Option<i64> {
    if clause.contains("半年") || clause.contains("half a year") {
        return Some(182);
    }
    if clause.contains("一年内") || clause.contains("within a year") {
        return Some(365);
    }
    let cn = [("个月", 30), ("月内", 30), ("周", 7), ("天", 1)];
    if clause.contains('内') || clause.contains("以内") {
        for (unit, days) in cn {
            if let Some(n) = number_before(clause, unit) {
                return Some(n as i64 * days);
            }
        }
    }
    if let Some(n) = number_after(clause, "within") {
        let after = clause.split_once("within").map_or("", |(_, a)| a);
        let days = if after.contains("month") {
            30
        } else if after.contains("week") {
            7
        } else {
            1
        };
        return Some(n as i64 * days);
    }
    None
}

fn top_in(clause: &str, current: usize) -> Option<usize> {
    for word in ["top", "recommend", "推荐"] {
        if let Some(n) = number_after(clause, word) {
            return Some(n);
        }
    }
    for unit in ["个", " conferences", " venues", " picks"] {
        if let Some(n) = number_before(clause, unit) {
            return Some(n);
        }
    }
    let more = ["多推荐", "多给", "多一些", "多来", "再来", "more"];
    if has_any(clause, &more) {
        return Some(current + MORE_STEP);
    }
    if clause.contains("少一些") || clause.contains("fewer") {
        return Some(current.saturating_sub(MORE_STEP).max(3));
    }
    None
}

fn push_unique(list: &mut Vec<String>, items: impl IntoIterator<Item = String>) {
    for item in items {
        if !list.iter().any(|i| i.eq_ignore_ascii_case(&item)) {
            list.push(item);
        }
    }
}

impl Constraints {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn top(&self) -> usize {
        self.top.unwrap_or(DEFAULT_TOP)
    }

    /// Read `text` as a follow-up to `self`. `current` lists the titles of
    /// the round on screen, for "换一批" / "others".
    pub fn refine(&self, text: &str, conferences: &[Conference], current: &[String]) -> Self {
        let mut next = self.clone();
        let mut refined = false;
        let all = lower(text);

        for clause in clauses(text) {
            let negated = has_any(&clause, &NEGATIONS);
            let only = has_any(&clause, &ONLY);

            let ranks = ranks_in(&clause);
            if !ranks.is_empty() {
                refined = true;
                if negated {
                    let base: Vec<String> = if next.only_ranks.is_empty() {
                        ["A", "B", "C"].iter().map(|s| s.to_string()).collect()
                    } else {
                        next.only_ranks.clone()
                    };
                    next.only_ranks = base.into_iter().filter(|r| !ranks.contains(r)).collect();
                } else {
                    next.only_ranks = ranks;
                }
            }

            if only {
                let categories = categories_in(&clause);
                if !categories.is_empty() {
                    refined = true;
                    next.categories = categories;
                }
            }

            if has_any(&clause, &["tbd", "待定", "未公布", "没公布"]) {
                refined = true;
                let include = has_any(
                    &clause,
                    &["包括", "包含", "也行", "也可以", "include", "keep"],
                );
                next.no_tbd = negated || !include;
            }

            let places = places_in(&clause);
            if !places.is_empty() {
                refined = true;
                if negated {
                    next.preferred_places.retain(|p| !places.contains(p));
                    push_unique(&mut next.avoided_places, places);
                } else {
                    next.avoided_places.retain(|p| !places.contains(p));
                    push_unique(&mut next.preferred_places, places);
                }
            }

            if let Some(days) = horizon_in(&clause) {
                refined = true;
                next.horizon_days = Some(days.max(1));
            } else if let Some(top) = top_in(&clause, next.top()) {
                refined = true;
                next.top = Some(top.clamp(1, MAX_TOP));
            }

            if has_any(
                &clause,
                &[
                    "换一批",
                    "换几个",
                    "换一些",
                    "其他的",
                    "别的",
                    "others",
                    "another batch",
                    "different ones",
                ],
            ) {
                refined = true;
                push_unique(&mut next.excluded, current.iter().cloned());
            } else if negated {
                let titles = titles_in(&clause, conferences);
                if !titles.is_empty() {
                    refined = true;
                    push_unique(&mut next.excluded, titles.into_iter().map(str::to_string));
                }
            }
        }

        if let Some(date) = text
            .split(|c: char| !(c.is_ascii_digit() || c == '-' || c == '/'))
            .find_map(parse_date)
        {
            refined = true;
            next.ready_date = Some(date);
        }

        // A research description replaces the previous one; a short
        // follow-up such as "只看 AI" does not. Conference names are not
        // research topics ("排除 CVPR").
        let mut topic_text = all.clone();
        for title in titles_in(&all, conferences) {
            topic_text = topic_text.replace(&title.to_lowercase(), " ");
        }
        let describes_research = !refined || has_any(&all, &RESEARCH) || self.interests.is_none();
        if describes_research && !tags_for_text(&topic_text).is_empty() {
            next.interests = Some(text.trim().to_string());
        }
        next
    }

    /// The profile for this request.
    pub fn apply(&self, profile: &Profile) -> Profile {
        let mut profile = match &self.interests {
            Some(text) => profile.with_extra_interests(std::slice::from_ref(text)),
            None => profile.clone(),
        };
        if let Some(date) = self.ready_date {
            profile.ready_date = Some(date);
        }
        if !self.only_ranks.is_empty() {
            profile.target_ranks = self.only_ranks.clone();
        }
        if !self.categories.is_empty() {
            profile.categories = self.categories.clone();
        }
        if self.no_tbd {
            profile.include_tbd = false;
        }
        push_unique(
            &mut profile.preferred_places,
            self.preferred_places.iter().cloned(),
        );
        push_unique(
            &mut profile.avoided_places,
            self.avoided_places.iter().cloned(),
        );
        if let Some(days) = self.horizon_days {
            profile.horizon_days = days;
        }
        profile
    }

    /// The profile with these conditions written into it, for
    /// `/profile save`: recognised topics join the interests (as topic
    /// names, not the whole sentence), the other conditions replace or
    /// extend the matching fields. Exclusions and the number of picks are
    /// per-conversation and stay out.
    pub fn merge_into(&self, profile: &Profile, lang: Lang) -> Profile {
        let mut out = profile.clone();
        if let Some(text) = &self.interests {
            let names: Vec<String> = tags_for_text(text)
                .iter()
                .map(|t| super::topics::tag_name(t, lang.is_zh()).to_string())
                .filter(|n| !n.is_empty())
                .collect();
            // Skip topics the profile already covers in other words.
            let known = tags_for_text(&out.interests.join(" , "));
            let fresh: Vec<String> = names
                .into_iter()
                .filter(|n| tags_for_text(n).iter().any(|t| !known.contains(t)))
                .collect();
            push_unique(&mut out.interests, fresh);
        }
        if let Some(date) = self.ready_date {
            out.ready_date = Some(date);
        }
        if !self.only_ranks.is_empty() {
            out.target_ranks = self.only_ranks.clone();
        }
        if !self.categories.is_empty() {
            out.categories = self.categories.clone();
        }
        if self.no_tbd {
            out.include_tbd = false;
        }
        if let Some(days) = self.horizon_days {
            out.horizon_days = days;
        }
        out.avoided_places.retain(|p| {
            !self
                .preferred_places
                .iter()
                .any(|q| q.eq_ignore_ascii_case(p))
        });
        out.preferred_places.retain(|p| {
            !self
                .avoided_places
                .iter()
                .any(|q| q.eq_ignore_ascii_case(p))
        });
        push_unique(
            &mut out.preferred_places,
            self.preferred_places.iter().cloned(),
        );
        push_unique(&mut out.avoided_places, self.avoided_places.iter().cloned());
        out
    }

    /// Hard filters the scorer does not apply by itself.
    pub fn keeps(&self, facts: &ConferenceFacts) -> bool {
        (self.only_ranks.is_empty() || self.only_ranks.contains(&facts.ccf))
            && (self.categories.is_empty() || self.categories.contains(&facts.sub))
            && !self
                .excluded
                .iter()
                .any(|t| t.eq_ignore_ascii_case(&facts.title))
            && !self
                .avoided_places
                .iter()
                .any(|p| facts.place.to_lowercase().contains(&p.to_lowercase()))
    }

    /// Short labels for the "conditions" line, e.g. `仅 CCF-A`.
    pub fn labels(&self, lang: Lang) -> Vec<String> {
        let zh = lang.is_zh();
        let mut out = Vec::new();
        if let Some(text) = &self.interests {
            let tags = tags_for_text(text);
            let names: Vec<&str> = tags
                .iter()
                .map(|t| super::topics::tag_name(t, zh))
                .collect();
            if !names.is_empty() {
                out.push(names.join(if zh { "、" } else { ", " }));
            }
        }
        if let Some(date) = self.ready_date {
            out.push(if zh {
                format!("{date} 后")
            } else {
                format!("after {date}")
            });
        }
        if !self.only_ranks.is_empty() {
            let ranks = self.only_ranks.join("/");
            out.push(if zh {
                format!("仅 CCF-{ranks}")
            } else {
                format!("CCF-{ranks} only")
            });
        }
        if !self.categories.is_empty() {
            let names: Vec<&str> = self
                .categories
                .iter()
                .map(|sub| {
                    CATEGORIES
                        .iter()
                        .find(|c| c.sub == sub)
                        .map_or(sub.as_str(), |c| if zh { c.sub } else { c.name_en })
                })
                .collect();
            out.push(if zh {
                format!("仅 {}", names.join("/"))
            } else {
                format!("only {}", names.join("/"))
            });
        }
        if let Some(top) = self.top {
            out.push(if zh {
                format!("{top} 个")
            } else {
                format!("{top} picks")
            });
        }
        if let Some(days) = self.horizon_days {
            out.push(if zh {
                format!("{days} 天内")
            } else {
                format!("within {days} days")
            });
        }
        if self.no_tbd {
            out.push(lang.pick("no TBD", "不含待定").to_string());
        }
        if !self.preferred_places.is_empty() {
            out.push(place_label(&self.preferred_places, lang, false));
        }
        if !self.avoided_places.is_empty() {
            out.push(place_label(&self.avoided_places, lang, true));
        }
        if !self.excluded.is_empty() {
            let shown: Vec<&str> = self.excluded.iter().take(4).map(String::as_str).collect();
            let more = self.excluded.len().saturating_sub(shown.len());
            let mut text = shown.join(", ");
            if more > 0 {
                text.push_str(&format!(" +{more}"));
            }
            out.push(if zh {
                format!("排除 {text}")
            } else {
                format!("excluding {text}")
            });
        }
        out
    }
}

fn place_label(places: &[String], lang: Lang, avoid: bool) -> String {
    // Show the region name when the keywords are one of the known groups.
    let names: Vec<&str> = PLACES
        .iter()
        .filter(|(_, _, _, keywords)| keywords.iter().all(|k| places.iter().any(|p| p == k)))
        .map(|(zh, en, _, _)| lang.pick(en, zh))
        .collect();
    // Prefer the most specific region names; fall back to the keywords.
    let name = if names.is_empty() {
        places.join("/")
    } else {
        names.join("/")
    };
    match (lang, avoid) {
        (Lang::Zh, false) => format!("优先{name}"),
        (Lang::Zh, true) => format!("避开{name}"),
        (Lang::En, false) => format!("prefer {name}"),
        (Lang::En, true) => format!("avoid {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::recommend::tests::conferences;

    fn refine(prev: &Constraints, text: &str) -> Constraints {
        prev.refine(
            text,
            &conferences(),
            &["CVPR".to_string(), "ICME".to_string()],
        )
    }

    fn ranks(c: &Constraints) -> Vec<&str> {
        c.only_ranks.iter().map(String::as_str).collect()
    }

    #[test]
    fn a_research_description_sets_interests_and_date() {
        let c = refine(
            &Constraints::default(),
            "我做多模态大模型，2027-03-01 能写完",
        );
        assert_eq!(
            c.interests.as_deref(),
            Some("我做多模态大模型，2027-03-01 能写完")
        );
        assert_eq!(c.ready_date, parse_date("2027-03-01"));
        assert!(c.only_ranks.is_empty());
    }

    #[test]
    fn rank_follow_ups_in_both_languages() {
        let base = refine(&Constraints::default(), "computer vision");
        assert_eq!(ranks(&refine(&base, "只要 A 类")), ["A"]);
        assert_eq!(ranks(&refine(&base, "只看A类的")), ["A"]);
        assert_eq!(ranks(&refine(&base, "A/B 类就行")), ["A", "B"]);
        assert_eq!(ranks(&refine(&base, "a、b类")), ["A", "B"]);
        assert_eq!(ranks(&refine(&base, "CCF-A only")), ["A"]);
        assert_eq!(ranks(&refine(&base, "only rank b please")), ["B"]);
        assert_eq!(ranks(&refine(&base, "不要 C 类")), ["A", "B"]);
        assert_eq!(ranks(&refine(&base, "no ccf-c")), ["A", "B"]);
        // The research interests are kept by short follow-ups.
        assert_eq!(refine(&base, "只要 A 类").interests, base.interests);
        // Words that merely contain a letter do not count.
        assert!(refine(&base, "abc类似的方向").only_ranks.is_empty());
    }

    #[test]
    fn category_follow_ups_need_an_only_cue() {
        let base = Constraints::default();
        assert_eq!(refine(&base, "只看 AI").categories, ["AI"]);
        assert_eq!(refine(&base, "只看数据库方向的").categories, ["DB"]);
        assert_eq!(
            refine(&base, "only software engineering").categories,
            ["SE"]
        );
        assert!(refine(&base, "我做 AI 安全").categories.is_empty());
    }

    #[test]
    fn count_follow_ups() {
        let base = Constraints::default();
        assert_eq!(
            refine(&base, "多推荐几个").top,
            Some(DEFAULT_TOP + MORE_STEP)
        );
        assert_eq!(
            refine(&base, "show me more").top,
            Some(DEFAULT_TOP + MORE_STEP)
        );
        assert_eq!(refine(&base, "推荐 10 个").top, Some(10));
        assert_eq!(refine(&base, "推荐五个").top, Some(5));
        assert_eq!(refine(&base, "top 12").top, Some(12));
        assert_eq!(refine(&base, "top 999").top, Some(MAX_TOP));
        let more = refine(&refine(&base, "推荐 10 个"), "再来几个");
        assert_eq!(more.top, Some(10 + MORE_STEP));
    }

    #[test]
    fn exclusions_and_new_batches() {
        let base = Constraints::default();
        assert_eq!(refine(&base, "排除 CVPR").excluded, ["CVPR"]);
        assert_eq!(refine(&base, "without icme please").excluded, ["ICME"]);
        let batch = refine(&base, "换一批");
        assert_eq!(batch.excluded, ["CVPR", "ICME"]);
        assert_eq!(refine(&base, "give me different ones").excluded.len(), 2);
        // Mentioning a conference without a negation excludes nothing.
        assert!(refine(&base, "CVPR 怎么样").excluded.is_empty());
    }

    #[test]
    fn tbd_places_and_time_windows() {
        let base = Constraints::default();
        assert!(refine(&base, "不要 TBD").no_tbd);
        assert!(refine(&base, "no TBD ones").no_tbd);
        assert!(!refine(&refine(&base, "不要待定的"), "包括待定").no_tbd);

        let china = refine(&base, "只看国内");
        assert!(china.preferred_places.contains(&"China".to_string()));
        let avoid = refine(&china, "不去美国");
        assert!(avoid.avoided_places.contains(&"USA".to_string()));
        assert!(avoid.preferred_places.contains(&"China".to_string()));
        assert!(
            refine(&base, "avoid the US")
                .avoided_places
                .contains(&"USA".to_string())
        );

        assert_eq!(refine(&base, "三个月内").horizon_days, Some(90));
        assert_eq!(refine(&base, "半年内截稿的").horizon_days, Some(182));
        assert_eq!(refine(&base, "2周内").horizon_days, Some(14));
        assert_eq!(refine(&base, "within 60 days").horizon_days, Some(60));
        assert_eq!(refine(&base, "within 2 months").horizon_days, Some(60));
    }

    #[test]
    fn conditions_accumulate_across_turns() {
        let mut c = refine(&Constraints::default(), "computer vision, 2030-03-01");
        c = refine(&c, "只要 A 类");
        c = refine(&c, "排除 ICME");
        c = refine(&c, "多推荐几个");
        assert!(c.interests.is_some());
        assert_eq!(ranks(&c), ["A"]);
        assert_eq!(c.excluded, ["ICME"]);
        assert_eq!(c.top(), DEFAULT_TOP + MORE_STEP);
        // A new research description replaces the old one, keeps the rest.
        let next = refine(&c, "我做数据库");
        assert_eq!(next.interests.as_deref(), Some("我做数据库"));
        assert_eq!(ranks(&next), ["A"]);
    }

    #[test]
    fn apply_and_keeps_shape_the_request() {
        let c = Constraints {
            only_ranks: vec!["A".into()],
            excluded: vec!["CVPR".into()],
            no_tbd: true,
            horizon_days: Some(90),
            ..Default::default()
        };
        let profile = c.apply(&Profile::default());
        assert_eq!(profile.target_ranks, ["A"]);
        assert!(!profile.include_tbd);
        assert_eq!(profile.horizon_days, 90);
        let facts = crate::agent::recommend::open_conferences(
            &conferences(),
            crate::agent::recommend::tests::now(),
        );
        assert!(
            facts
                .iter()
                .filter(|f| c.keeps(f))
                .all(|f| f.ccf == "A" && f.title != "CVPR")
        );
    }

    #[test]
    fn conditions_merge_into_a_profile() {
        let c = refine(
            &refine(
                &Constraints::default(),
                "我做多模态大模型，2027-03-01 能写完",
            ),
            "只要 A 类，只看国内，排除 CVPR，半年内，多推荐几个",
        );
        let base = Profile {
            interests: vec!["program analysis".into()],
            avoided_places: vec!["China".into()],
            ..Profile::default()
        };
        let merged = c.merge_into(&base, Lang::Zh);
        assert_eq!(merged.interests[0], "program analysis");
        assert!(merged.interests.len() > 1, "{:?}", merged.interests);
        // Topic names, not the sentence, and they are recognised again.
        assert!(merged.interests.iter().all(|i| !i.contains("2027")));
        assert!(
            merged.interests[1..]
                .iter()
                .all(|i| !tags_for_text(i).is_empty())
        );
        assert_eq!(merged.ready_date, parse_date("2027-03-01"));
        assert_eq!(merged.target_ranks, ["A"]);
        assert_eq!(merged.horizon_days, 182);
        assert!(merged.preferred_places.contains(&"China".to_string()));
        assert!(merged.avoided_places.is_empty());
        // Merging twice adds nothing new.
        assert_eq!(c.merge_into(&merged, Lang::Zh), merged);
    }

    #[test]
    fn labels_describe_the_conditions() {
        let c = refine(
            &refine(&Constraints::default(), "我做计算机视觉，2030-03-01"),
            "只要 A 类，排除 CVPR，不去美国",
        );
        let zh = c.labels(Lang::Zh).join(" · ");
        assert!(zh.contains("2030-03-01 后"), "{zh}");
        assert!(zh.contains("仅 CCF-A"), "{zh}");
        assert!(zh.contains("排除 CVPR"), "{zh}");
        assert!(zh.contains("避开美国"), "{zh}");
        let en = c.labels(Lang::En).join(" · ");
        assert!(en.contains("CCF-A only"), "{en}");
        assert!(en.contains("avoid"), "{en}");
        assert!(Constraints::default().labels(Lang::En).is_empty());
    }
}
