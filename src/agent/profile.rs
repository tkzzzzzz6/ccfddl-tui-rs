//! Research profile (`~/.ccfddl/profile.yaml`).
//!
//! The profile describes what the user works on and when a paper will be
//! ready. Parsing is lenient like the conference loader: unknown keys are
//! ignored and values of the wrong type fall back to defaults.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde_yaml_ng::Value;

use crate::data::favorites::app_dir;
use crate::model::{CATEGORIES, VALID_CCF_RANKS};

pub const DEFAULT_TARGET_RANKS: [&str; 2] = ["A", "B"];
pub const DEFAULT_MIN_DAYS_LEFT: i64 = 14;
pub const DEFAULT_HORIZON_DAYS: i64 = 365;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Free-text research interests, English or Chinese.
    pub interests: Vec<String>,
    /// Explicit CCF categories (`AI`, `DB`, ...). Empty means "infer from
    /// interests".
    pub categories: Vec<String>,
    /// Target CCF ranks, best first.
    pub target_ranks: Vec<String>,
    /// When the paper will be ready to submit.
    pub ready_date: Option<NaiveDate>,
    /// Without a ready date, skip deadlines closer than this.
    pub min_days_left: i64,
    /// Ignore deadlines further away than this.
    pub horizon_days: i64,
    pub preferred_places: Vec<String>,
    pub avoided_places: Vec<String>,
    /// Keep conferences whose deadline is not announced yet.
    pub include_tbd: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            interests: Vec::new(),
            categories: Vec::new(),
            target_ranks: DEFAULT_TARGET_RANKS.iter().map(|r| r.to_string()).collect(),
            ready_date: None,
            min_days_left: DEFAULT_MIN_DAYS_LEFT,
            horizon_days: DEFAULT_HORIZON_DAYS,
            preferred_places: Vec::new(),
            avoided_places: Vec::new(),
            include_tbd: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("{path}: {message}")]
    Read { path: String, message: String },
    #[error("{path}: invalid YAML: {message}")]
    Parse { path: String, message: String },
}

pub fn default_profile_path() -> PathBuf {
    app_dir().join("profile.yaml")
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect(),
        // A single comma separated string is accepted as a convenience.
        Some(Value::String(s)) => split_list(s),
        _ => Vec::new(),
    }
}

/// Split "a, b，c、d" into trimmed items (ASCII and CJK separators).
pub fn split_list(text: &str) -> Vec<String> {
    text.split([',', '，', '、', ';', '；'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn int_or(value: Option<&Value>, default: i64) -> i64 {
    match value {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(default),
        _ => default,
    }
}

/// Dates as people type them: `2027-03-01`, `2027/3/1`, `2027.3.1`,
/// `2027年3月1日`, `3月1日` / `3/1` (the next such day), `3个月后`, `2周后`,
/// `10天后`, `in 3 months`, `next month`. `today` anchors relative dates.
pub fn parse_flexible_date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    use chrono::{Datelike, Months};
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    for format in [
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%Y.%m.%d",
        "%Y年%m月%d日",
        "%Y年%m月%d号",
    ] {
        if let Ok(date) = NaiveDate::parse_from_str(&text, format) {
            return Some(date);
        }
    }
    let number = |s: &str| -> Option<u32> {
        let s = s.trim();
        s.parse().ok().or(match s {
            "一" => Some(1),
            "两" | "二" => Some(2),
            "三" => Some(3),
            "四" => Some(4),
            "五" => Some(5),
            "六" => Some(6),
            "七" => Some(7),
            "八" => Some(8),
            "九" => Some(9),
            "十" => Some(10),
            "半" => Some(0),
            _ => None,
        })
    };
    let relative = |n: u32, unit: &str| -> Option<NaiveDate> {
        match unit {
            "month" => today.checked_add_months(Months::new(n)),
            "week" => today.checked_add_days(chrono::Days::new(u64::from(n) * 7)),
            _ => today.checked_add_days(chrono::Days::new(u64::from(n))),
        }
    };
    if text == "半年后" || text == "in half a year" {
        return relative(6, "month");
    }
    if text == "next month" || text == "下个月" {
        return relative(1, "month");
    }
    for (suffix, unit) in [
        ("个月后", "month"),
        ("月后", "month"),
        ("周后", "week"),
        ("星期后", "week"),
        ("天后", "day"),
    ] {
        if let Some(n) = text.strip_suffix(suffix).and_then(number) {
            return relative(n, unit);
        }
    }
    if let Some(rest) = text.strip_prefix("in ") {
        let mut parts = rest.split_whitespace();
        if let (Some(n), Some(unit)) = (parts.next().and_then(number), parts.next()) {
            let unit = unit.trim_end_matches('s');
            if ["month", "week", "day"].contains(&unit) {
                return relative(n, unit);
            }
        }
    }
    // Month and day only: the next time that day comes round.
    let month_day = text
        .strip_suffix('日')
        .or_else(|| text.strip_suffix('号'))
        .and_then(|s| s.split_once('月'))
        .or_else(|| {
            text.split_once('/')
                .filter(|(m, d)| m.len() <= 2 && d.len() <= 2)
        });
    if let Some((m, d)) = month_day
        && let (Some(m), Some(d)) = (number(m), number(d))
    {
        for year in [today.year(), today.year() + 1] {
            if let Some(date) = NaiveDate::from_ymd_opt(year, m, d)
                && date >= today
            {
                return Some(date);
            }
        }
    }
    None
}

pub fn parse_date(text: &str) -> Option<NaiveDate> {
    let text = text.trim();
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(text, "%Y/%m/%d"))
        .ok()
}

/// Keep only valid CCF ranks, upper-cased and in A..N order.
pub fn normalize_ranks(ranks: &[String]) -> Vec<String> {
    let mut out: Vec<String> = VALID_CCF_RANKS
        .iter()
        .filter(|valid| ranks.iter().any(|r| r.trim().eq_ignore_ascii_case(valid)))
        .map(|r| r.to_string())
        .collect();
    out.dedup();
    out
}

/// Keep only known category codes, upper-cased.
pub fn normalize_categories(categories: &[String]) -> Vec<String> {
    CATEGORIES
        .iter()
        .filter(|c| {
            categories
                .iter()
                .any(|s| s.trim().eq_ignore_ascii_case(c.sub))
        })
        .map(|c| c.sub.to_string())
        .collect()
}

impl Profile {
    pub fn parse(content: &str, origin: &str) -> Result<Self, ProfileError> {
        let value: Value = serde_yaml_ng::from_str(content).map_err(|e| ProfileError::Parse {
            path: origin.to_string(),
            message: e.to_string(),
        })?;
        let mut profile = Profile::default();
        if !matches!(value, Value::Mapping(_)) {
            return Ok(profile);
        }
        profile.interests = string_list(value.get("interests"));
        profile.categories = normalize_categories(&string_list(value.get("categories")));
        let ranks = normalize_ranks(&string_list(value.get("target_ranks")));
        if !ranks.is_empty() {
            profile.target_ranks = ranks;
        }
        profile.ready_date = match value.get("ready_date") {
            Some(Value::String(s)) => parse_date(s),
            _ => None,
        };
        profile.min_days_left = int_or(value.get("min_days_left"), DEFAULT_MIN_DAYS_LEFT).max(0);
        profile.horizon_days = int_or(value.get("horizon_days"), DEFAULT_HORIZON_DAYS).max(1);
        profile.preferred_places = string_list(value.get("preferred_places"));
        profile.avoided_places = string_list(value.get("avoided_places"));
        if let Some(Value::Bool(b)) = value.get("include_tbd") {
            profile.include_tbd = *b;
        }
        Ok(profile)
    }

    /// Load a profile; a missing file yields `Ok(None)`.
    pub fn load(path: &Path) -> Result<Option<Self>, ProfileError> {
        match fs::read_to_string(path) {
            Ok(content) => Self::parse(&content, &path.display().to_string()).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ProfileError::Read {
                path: path.display().to_string(),
                message: e.to_string(),
            }),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_yaml())
    }

    pub fn is_empty(&self) -> bool {
        self.interests.is_empty() && self.categories.is_empty()
    }

    /// Serialize with comments, so the file doubles as documentation.
    pub fn to_yaml(&self) -> String {
        fn list(items: &[String]) -> String {
            if items.is_empty() {
                "[]".to_string()
            } else {
                let quoted: Vec<String> = items.iter().map(|s| yaml_string(s)).collect();
                format!("[{}]", quoted.join(", "))
            }
        }
        let codes: Vec<&str> = CATEGORIES.iter().map(|c| c.sub).collect();
        format!(
            "# ccfddl-tui research profile, used by `--recommend` and the agent panel.\n\
             # Research interests in English or Chinese, e.g. [\"multimodal LLM\", \"程序分析\"].\n\
             interests: {}\n\
             # CCF categories to consider ({}). Empty: inferred from interests.\n\
             categories: {}\n\
             # Target CCF ranks (A/B/C/N). Better ranks become stretch picks,\n\
             # worse ones safety picks.\n\
             target_ranks: {}\n\
             # When the paper will be ready (YYYY-MM-DD). Deadlines before it are skipped.\n\
             ready_date: {}\n\
             # Without ready_date, skip deadlines closer than this many days.\n\
             min_days_left: {}\n\
             # Ignore deadlines further away than this many days.\n\
             horizon_days: {}\n\
             # Place keywords to prefer or avoid, e.g. [\"Asia\", \"China\"].\n\
             preferred_places: {}\n\
             avoided_places: {}\n\
             # Keep conferences whose deadline is not announced yet.\n\
             include_tbd: {}\n",
            list(&self.interests),
            codes.join("/"),
            list(&self.categories),
            list(&self.target_ranks),
            self.ready_date
                .map(|d| format!("'{}'", d.format("%Y-%m-%d")))
                .unwrap_or_else(|| "null".to_string()),
            self.min_days_left,
            self.horizon_days,
            list(&self.preferred_places),
            list(&self.avoided_places),
            self.include_tbd,
        )
    }

    /// A copy with interests from `text` merged in (used for ad-hoc queries).
    pub fn with_extra_interests(&self, extra: &[String]) -> Self {
        let mut profile = self.clone();
        for item in extra {
            if !profile
                .interests
                .iter()
                .any(|i| i.eq_ignore_ascii_case(item))
            {
                profile.interests.push(item.clone());
            }
        }
        profile
    }

    /// One-paragraph description for prompts and `/profile`.
    pub fn summary(&self, zh: bool) -> String {
        let none = if zh { "未设置" } else { "not set" };
        let join = |items: &[String]| {
            if items.is_empty() {
                none.to_string()
            } else {
                items.join(", ")
            }
        };
        let ready = self
            .ready_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| none.to_string());
        if zh {
            format!(
                "研究方向: {}\n分类: {}\n目标等级: CCF {}\n论文就绪: {}\n最少剩余天数: {}  考虑范围: {} 天\n偏好地点: {}  避开地点: {}",
                join(&self.interests),
                join(&self.categories),
                join(&self.target_ranks),
                ready,
                self.min_days_left,
                self.horizon_days,
                join(&self.preferred_places),
                join(&self.avoided_places),
            )
        } else {
            format!(
                "Interests: {}\nCategories: {}\nTarget ranks: CCF {}\nPaper ready: {}\nMin days left: {}  Horizon: {} days\nPreferred places: {}  Avoided places: {}",
                join(&self.interests),
                join(&self.categories),
                join(&self.target_ranks),
                ready,
                self.min_days_left,
                self.horizon_days,
                join(&self.preferred_places),
                join(&self.avoided_places),
            )
        }
    }
}

fn yaml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flexible_dates() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d);
        let parse = |t: &str| parse_flexible_date(t, today);
        assert_eq!(parse("2027-03-01"), d(2027, 3, 1));
        assert_eq!(parse("2027/3/1"), d(2027, 3, 1));
        assert_eq!(parse("2027.3.1"), d(2027, 3, 1));
        assert_eq!(parse("2027年3月1日"), d(2027, 3, 1));
        assert_eq!(parse("3月1日"), d(2027, 3, 1));
        assert_eq!(parse("12月1号"), d(2026, 12, 1));
        assert_eq!(parse("11/15"), d(2026, 11, 15));
        assert_eq!(parse("3个月后"), d(2027, 1, 29));
        assert_eq!(parse("三个月后"), d(2027, 1, 29));
        assert_eq!(parse("半年后"), d(2027, 4, 29));
        assert_eq!(parse("2周后"), d(2026, 11, 12));
        assert_eq!(parse("10天后"), d(2026, 11, 8));
        assert_eq!(parse("in 3 months"), d(2027, 1, 29));
        assert_eq!(parse("in 1 week"), d(2026, 11, 5));
        assert_eq!(parse("next month"), d(2026, 11, 29));
        assert_eq!(parse("2027-13-01"), None);
        assert_eq!(parse("soon"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn parses_a_full_profile() {
        let profile = Profile::parse(
            "interests: [多模态大模型, program analysis]\n\
             categories: [ai, SE, bogus]\n\
             target_ranks: [b, A]\n\
             ready_date: '2030-03-01'\n\
             min_days_left: 7\n\
             preferred_places: Asia\n\
             include_tbd: false\n",
            "test",
        )
        .unwrap();
        assert_eq!(profile.interests, ["多模态大模型", "program analysis"]);
        // Unknown codes are dropped; known ones follow the sidebar order.
        assert_eq!(profile.categories, ["SE", "AI"]);
        assert_eq!(profile.target_ranks, ["A", "B"]);
        assert_eq!(profile.ready_date, NaiveDate::from_ymd_opt(2030, 3, 1));
        assert_eq!(profile.min_days_left, 7);
        assert_eq!(profile.preferred_places, ["Asia"]);
        assert!(!profile.include_tbd);
    }

    #[test]
    fn missing_or_odd_values_fall_back_to_defaults() {
        let profile = Profile::parse("interests: 3\ntarget_ranks: [Z]\n", "test").unwrap();
        assert_eq!(profile.target_ranks, DEFAULT_TARGET_RANKS);
        assert_eq!(profile.min_days_left, DEFAULT_MIN_DAYS_LEFT);
        assert_eq!(Profile::parse("", "test").unwrap(), Profile::default());
    }

    #[test]
    fn round_trips_through_yaml() {
        let profile = Profile {
            interests: vec!["LLM \"agents\"".into(), "程序分析".into()],
            categories: vec!["AI".into()],
            ready_date: NaiveDate::from_ymd_opt(2031, 1, 2),
            preferred_places: vec!["Asia".into()],
            ..Profile::default()
        };
        let parsed = Profile::parse(&profile.to_yaml(), "test").unwrap();
        assert_eq!(parsed, profile);
    }

    #[test]
    fn load_and_save_use_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/profile.yaml");
        assert_eq!(Profile::load(&path).unwrap(), None);
        let profile = Profile {
            interests: vec!["security".into()],
            ..Profile::default()
        };
        profile.save(&path).unwrap();
        assert_eq!(Profile::load(&path).unwrap(), Some(profile));
    }

    #[test]
    fn split_list_accepts_cjk_separators() {
        assert_eq!(split_list("a，b、c; d"), ["a", "b", "c", "d"]);
    }
}
