//! Data models for CCFDDL conference data.
//!
//! Parsing is deliberately lenient (same as the Python `from_dict` helpers):
//! values of the wrong type fall back to defaults instead of failing the file.

use serde_yaml_ng::Value;

pub const VALID_CCF_RANKS: [&str; 4] = ["A", "B", "C", "N"];
pub const VALID_CORE_RANKS: [&str; 5] = ["A*", "A", "B", "C", "N"];
pub const VALID_THCPL_RANKS: [&str; 3] = ["A", "B", "N"];
pub const VALID_YEAR_RANGE: (i64, i64) = (1990, 2100);

fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Sequence(s)) => !s.is_empty(),
        Some(Value::Mapping(m)) => !m.is_empty(),
        Some(Value::Tagged(_)) => true,
    }
}

fn validate_rank(value: Option<&Value>, valid: &[&str]) -> String {
    match value {
        Some(Value::String(s)) if valid.contains(&s.as_str()) => s.clone(),
        _ => "N".to_string(),
    }
}

fn validate_year(value: Option<&Value>) -> i32 {
    let in_range = |y: i64| (VALID_YEAR_RANGE.0..=VALID_YEAR_RANGE.1).contains(&y);
    let year = match value {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    };
    match year {
        Some(y) if in_range(y) => y as i32,
        _ => 0,
    }
}

fn ensure_str(value: Option<&Value>, default: &str) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(true)) => "True".to_string(),
        Some(Value::Bool(false)) => "False".to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => default.to_string(),
    }
}

fn optional_str(value: Option<&Value>) -> Option<String> {
    is_truthy(value).then(|| ensure_str(value, ""))
}

fn ensure_list(value: Option<&Value>) -> &[Value] {
    match value {
        Some(Value::Sequence(items)) => items,
        _ => &[],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rank {
    pub ccf: String,
    pub core: Option<String>,
    pub thcpl: Option<String>,
}

impl Rank {
    pub fn from_value(data: &Value) -> Self {
        let core = data.get("core");
        let thcpl = data.get("thcpl");
        Self {
            ccf: validate_rank(data.get("ccf"), &VALID_CCF_RANKS),
            core: is_truthy(core).then(|| validate_rank(core, &VALID_CORE_RANKS)),
            thcpl: is_truthy(thcpl).then(|| validate_rank(thcpl, &VALID_THCPL_RANKS)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeline {
    pub deadline: String,
    pub abstract_deadline: Option<String>,
    pub comment: Option<String>,
}

impl Timeline {
    pub fn new(deadline: impl Into<String>) -> Self {
        Self {
            deadline: deadline.into(),
            abstract_deadline: None,
            comment: None,
        }
    }

    pub fn from_value(data: &Value) -> Self {
        Self {
            deadline: ensure_str(data.get("deadline"), "TBD"),
            abstract_deadline: optional_str(data.get("abstract_deadline")),
            comment: optional_str(data.get("comment")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConferenceYear {
    pub year: i32,
    pub id: String,
    pub link: String,
    pub timeline: Vec<Timeline>,
    pub timezone: String,
    pub date: String,
    pub place: String,
}

impl ConferenceYear {
    pub fn from_value(data: &Value) -> Self {
        Self {
            year: validate_year(data.get("year")),
            id: ensure_str(data.get("id"), ""),
            link: ensure_str(data.get("link"), ""),
            timeline: ensure_list(data.get("timeline"))
                .iter()
                .map(Timeline::from_value)
                .collect(),
            timezone: ensure_str(data.get("timezone"), "UTC"),
            date: ensure_str(data.get("date"), ""),
            place: ensure_str(data.get("place"), ""),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conference {
    pub title: String,
    pub description: String,
    pub sub: String,
    pub rank: Rank,
    pub dblp: String,
    pub confs: Vec<ConferenceYear>,
}

impl Conference {
    pub fn from_value(data: &Value) -> Self {
        Self {
            title: ensure_str(data.get("title"), ""),
            description: ensure_str(data.get("description"), ""),
            sub: ensure_str(data.get("sub"), ""),
            rank: Rank::from_value(data.get("rank").unwrap_or(&Value::Null)),
            dblp: ensure_str(data.get("dblp"), ""),
            confs: ensure_list(data.get("confs"))
                .iter()
                .map(ConferenceYear::from_value)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Category {
    pub name: &'static str,
    pub name_en: &'static str,
    pub sub: &'static str,
}

pub const CATEGORIES: [Category; 10] = [
    Category { name: "计算机体系结构/并行与分布计算/存储系统", name_en: "Computer Architecture", sub: "DS" },
    Category { name: "计算机网络", name_en: "Network System", sub: "NW" },
    Category { name: "网络与信息安全", name_en: "Network and System Security", sub: "SC" },
    Category { name: "软件工程/系统软件/程序设计语言", name_en: "Software Engineering", sub: "SE" },
    Category { name: "数据库/数据挖掘/内容检索", name_en: "Database", sub: "DB" },
    Category { name: "计算机科学理论", name_en: "Computing Theory", sub: "CT" },
    Category { name: "计算机图形学与多媒体", name_en: "Graphics", sub: "CG" },
    Category { name: "人工智能", name_en: "Artificial Intelligence", sub: "AI" },
    Category { name: "人机交互与普适计算", name_en: "Computer-Human Interaction", sub: "HI" },
    Category { name: "交叉/综合/新兴", name_en: "Interdiscipline", sub: "MX" },
];

pub fn get_category_by_sub(sub: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.sub == sub)
}

pub fn is_valid_sub(sub: &str) -> bool {
    get_category_by_sub(sub).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Value {
        serde_yaml_ng::from_str(yaml).unwrap()
    }

    #[test]
    fn rank_valid_values() {
        let rank = Rank::from_value(&parse("{ccf: A, core: 'A*', thcpl: B}"));
        assert_eq!(rank.ccf, "A");
        assert_eq!(rank.core.as_deref(), Some("A*"));
        assert_eq!(rank.thcpl.as_deref(), Some("B"));
    }

    #[test]
    fn rank_invalid_values_become_n() {
        let rank = Rank::from_value(&parse("{ccf: Z, core: X, thcpl: 'A*'}"));
        assert_eq!(rank.ccf, "N");
        assert_eq!(rank.core.as_deref(), Some("N"));
        assert_eq!(rank.thcpl.as_deref(), Some("N"));
    }

    #[test]
    fn rank_missing_or_falsy_optional_ranks_are_none() {
        let rank = Rank::from_value(&parse("{ccf: B, core: '', thcpl: null}"));
        assert_eq!(rank.ccf, "B");
        assert_eq!(rank.core, None);
        assert_eq!(rank.thcpl, None);
        assert_eq!(Rank::from_value(&Value::Null).ccf, "N");
    }

    #[test]
    fn year_validation() {
        let year = |s: &str| validate_year(Some(&parse(s)));
        assert_eq!(year("2025"), 2025);
        assert_eq!(year("'2025'"), 2025);
        assert_eq!(year("1989"), 0);
        assert_eq!(year("2101"), 0);
        assert_eq!(year("abc"), 0);
        assert_eq!(year("2025.0"), 0);
        assert_eq!(validate_year(None), 0);
    }

    #[test]
    fn timeline_defaults_and_optionals() {
        let t = Timeline::from_value(&parse("{abstract_deadline: '2024-01-01 00:00:00', comment: Main}"));
        assert_eq!(t.deadline, "TBD");
        assert_eq!(t.abstract_deadline.as_deref(), Some("2024-01-01 00:00:00"));
        assert_eq!(t.comment.as_deref(), Some("Main"));
        assert_eq!(Timeline::from_value(&parse("42")).deadline, "TBD");
    }

    #[test]
    fn conference_from_value_full() {
        let conf = Conference::from_value(&parse(
            r#"
title: CVPR
description: Vision
sub: AI
rank: {ccf: A, core: 'A*', thcpl: A}
dblp: cvpr
confs:
- year: 2025
  id: cvpr25
  link: https://cvpr2025.thecvf.com/
  timeline:
  - deadline: '2024-11-15 23:59:59'
    comment: Main deadline
  timezone: UTC-8
  date: June 15-20, 2025
  place: Seattle, USA
"#,
        ));
        assert_eq!(conf.title, "CVPR");
        assert_eq!(conf.confs.len(), 1);
        assert_eq!(conf.confs[0].year, 2025);
        assert_eq!(conf.confs[0].timezone, "UTC-8");
        assert_eq!(conf.confs[0].timeline[0].deadline, "2024-11-15 23:59:59");
    }

    #[test]
    fn conference_from_garbage_does_not_panic() {
        let conf = Conference::from_value(&parse("[1, 2, 3]"));
        assert_eq!(conf.title, "");
        assert_eq!(conf.rank.ccf, "N");
        assert!(conf.confs.is_empty());

        let conf = Conference::from_value(&parse("{confs: not-a-list, rank: 5}"));
        assert!(conf.confs.is_empty());
        assert_eq!(conf.rank.ccf, "N");
    }

    #[test]
    fn year_defaults() {
        let cy = ConferenceYear::from_value(&parse("{year: 2025}"));
        assert_eq!(cy.timezone, "UTC");
        assert!(cy.timeline.is_empty());
    }

    #[test]
    fn categories() {
        assert_eq!(CATEGORIES.len(), 10);
        assert_eq!(get_category_by_sub("DB").unwrap().name_en, "Database");
        assert!(get_category_by_sub("XX").is_none());
        assert!(is_valid_sub("AI"));
    }
}
