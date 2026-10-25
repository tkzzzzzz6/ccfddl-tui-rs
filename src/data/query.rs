//! Filtering, fuzzy search and default ordering (ports `DataService.filter_*`,
//! `fuzzy_search`, `sort_rows` and `CCFDeadlinesApp._apply_filters`).

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use super::row::ConferenceRow;
use super::search::similarity_ratio;
use crate::model::CATEGORIES;

pub const FUZZY_MATCH_THRESHOLD: f64 = 0.3;

fn set_of(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|v| (*v).to_string()).collect()
}

/// Active filters. An empty set means "no filtering" for that dimension,
/// which matches the Python behaviour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterState {
    pub subs: BTreeSet<String>,
    pub ccf_ranks: BTreeSet<String>,
    pub core_ranks: BTreeSet<String>,
    pub thcpl_ranks: BTreeSet<String>,
    pub show_expired: bool,
    pub query: String,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            subs: CATEGORIES.iter().map(|c| c.sub.to_string()).collect(),
            ccf_ranks: set_of(&["A", "B", "C", "N"]),
            core_ranks: set_of(&["A*", "A", "B", "C", "N"]),
            thcpl_ranks: set_of(&["A", "B", "N"]),
            show_expired: false,
            query: String::new(),
        }
    }
}

pub fn filter_by_sub(rows: Vec<ConferenceRow>, subs: &BTreeSet<String>) -> Vec<ConferenceRow> {
    if subs.is_empty() {
        return rows;
    }
    rows.into_iter().filter(|r| subs.contains(&r.sub)).collect()
}

pub fn filter_by_rank(rows: Vec<ConferenceRow>, ranks: &BTreeSet<String>) -> Vec<ConferenceRow> {
    if ranks.is_empty() {
        return rows;
    }
    rows.into_iter()
        .filter(|r| ranks.contains(&r.rank))
        .collect()
}

/// Rows without a CORE/THCPL rank always pass that rank filter.
fn passes_optional_rank(rank: &Option<String>, selected: &BTreeSet<String>) -> bool {
    selected.is_empty() || rank.as_ref().is_none_or(|r| selected.contains(r))
}

pub fn apply_filters(rows: Vec<ConferenceRow>, filters: &FilterState) -> Vec<ConferenceRow> {
    let rows = filter_by_sub(rows, &filters.subs);
    let rows = filter_by_rank(rows, &filters.ccf_ranks);
    rows.into_iter()
        .filter(|r| passes_optional_rank(&r.core_rank, &filters.core_ranks))
        .filter(|r| passes_optional_rank(&r.thcpl_rank, &filters.thcpl_ranks))
        .filter(|r| filters.show_expired || !r.is_expired)
        .collect()
}

/// Substring matches score above 1.0 (shorter titles first); everything else
/// is scored by similarity ratio. Results are ordered by descending score.
pub fn fuzzy_search(rows: Vec<ConferenceRow>, query: &str) -> Vec<ConferenceRow> {
    if query.is_empty() {
        return rows;
    }
    let query = query.to_lowercase();

    let mut scored: Vec<(ConferenceRow, f64)> = rows
        .into_iter()
        .filter_map(|row| {
            let title = row.title.to_lowercase();
            let score = if title.contains(&query) {
                1.0 + 1.0 / (title.chars().count() as f64 + 1.0)
            } else {
                similarity_ratio(&query, &title)
            };
            (score > FUZZY_MATCH_THRESHOLD).then_some((row, score))
        })
        .collect();

    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().map(|(row, _)| row).collect()
}

fn deadline_key(row: &ConferenceRow) -> DateTime<Utc> {
    row.deadline.unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// Default order: favorites, then running by urgency, then TBD, then finished
/// (TBD and finished newest year first). Sorting is stable.
pub fn sort_rows(rows: Vec<ConferenceRow>) -> Vec<ConferenceRow> {
    let mut favorites = Vec::new();
    let mut running = Vec::new();
    let mut tbd = Vec::new();
    let mut finished = Vec::new();

    for row in rows {
        if row.is_favorite {
            favorites.push(row);
        } else if row.is_tbd {
            tbd.push(row);
        } else if row.is_running {
            running.push(row);
        } else {
            finished.push(row);
        }
    }

    favorites.sort_by_key(deadline_key);
    running.sort_by_key(deadline_key);
    tbd.sort_by_key(|r| std::cmp::Reverse(r.year));
    finished.sort_by_key(|r| std::cmp::Reverse(r.year));

    favorites
        .into_iter()
        .chain(running)
        .chain(tbd)
        .chain(finished)
        .collect()
}

/// Full pipeline used by the UI. Without a query, the default conference
/// ordering applies. During search, relevance order is preserved instead of
/// being overwritten by the deadline ordering.
pub fn visible_rows(all: &[ConferenceRow], filters: &FilterState) -> Vec<ConferenceRow> {
    let rows = apply_filters(all.to_vec(), filters);
    if filters.query.is_empty() {
        sort_rows(rows)
    } else {
        fuzzy_search(rows, &filters.query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 1, 15, 12, 0, 0).unwrap()
    }

    struct Spec {
        title: &'static str,
        year: i32,
        sub: &'static str,
        rank: &'static str,
        days: Option<i64>,
        tbd: bool,
        fav: bool,
    }

    impl Spec {
        fn new(title: &'static str) -> Self {
            Self {
                title,
                year: 2025,
                sub: "AI",
                rank: "A",
                days: None,
                tbd: false,
                fav: false,
            }
        }
        fn year(mut self, year: i32) -> Self {
            self.year = year;
            self
        }
        fn sub(mut self, sub: &'static str) -> Self {
            self.sub = sub;
            self
        }
        fn rank(mut self, rank: &'static str) -> Self {
            self.rank = rank;
            self
        }
        fn days(mut self, days: i64) -> Self {
            self.days = Some(days);
            self
        }
        fn tbd(mut self) -> Self {
            self.tbd = true;
            self
        }
        fn fav(mut self) -> Self {
            self.fav = true;
            self
        }

        fn build(self) -> ConferenceRow {
            let running = self.tbd || self.days.is_some_and(|d| d > 0);
            ConferenceRow {
                title: self.title.into(),
                year: self.year,
                sub: self.sub.into(),
                rank: self.rank.into(),
                core_rank: Some("A".into()),
                thcpl_rank: Some("A".into()),
                deadline: self.days.map(|d| now() + Duration::days(d)),
                countdown: String::new(),
                place: "Test".into(),
                date: "2025".into(),
                link: "https://example.com".into(),
                is_running: running,
                is_tbd: self.tbd,
                is_expired: !running,
                is_favorite: self.fav,
            }
        }
    }

    fn titles(rows: &[ConferenceRow]) -> Vec<&str> {
        rows.iter().map(|r| r.title.as_str()).collect()
    }

    fn mixed() -> Vec<ConferenceRow> {
        vec![
            Spec::new("Finished").days(-5).year(2024).build(),
            Spec::new("Tbd2023").tbd().year(2023).build(),
            Spec::new("Soon").days(3).build(),
            Spec::new("Later").days(40).build(),
            Spec::new("Tbd2025").tbd().year(2025).build(),
            Spec::new("OldFinished").days(-50).year(2022).build(),
        ]
    }

    #[test]
    fn filter_by_sub_cases() {
        let rows = vec![
            Spec::new("A1").sub("AI").build(),
            Spec::new("D1").sub("DB").build(),
            Spec::new("N1").sub("NW").build(),
        ];
        let subs = |v: &[&str]| set_of(v);
        assert_eq!(titles(&filter_by_sub(rows.clone(), &subs(&["AI"]))), ["A1"]);
        assert_eq!(
            titles(&filter_by_sub(rows.clone(), &subs(&["AI", "DB"]))),
            ["A1", "D1"]
        );
        assert_eq!(filter_by_sub(rows.clone(), &subs(&[])).len(), 3);
        assert!(filter_by_sub(rows.clone(), &subs(&["XX"])).is_empty());
        assert!(filter_by_sub(vec![], &subs(&["AI"])).is_empty());
    }

    #[test]
    fn filter_by_rank_cases() {
        let rows = vec![
            Spec::new("A").rank("A").build(),
            Spec::new("B").rank("B").build(),
            Spec::new("C").rank("C").build(),
        ];
        assert_eq!(
            titles(&filter_by_rank(rows.clone(), &set_of(&["A", "C"]))),
            ["A", "C"]
        );
        assert_eq!(filter_by_rank(rows.clone(), &set_of(&[])).len(), 3);
        assert!(filter_by_rank(rows, &set_of(&["N"])).is_empty());
    }

    #[test]
    fn apply_filters_default_hides_expired() {
        let rows = apply_filters(mixed(), &FilterState::default());
        assert_eq!(titles(&rows), ["Tbd2023", "Soon", "Later", "Tbd2025"]);
        let all = apply_filters(
            mixed(),
            &FilterState {
                show_expired: true,
                ..Default::default()
            },
        );
        assert_eq!(all.len(), 6);
    }

    #[test]
    fn optional_ranks_pass_when_missing() {
        let mut none_ranks = Spec::new("NoRanks").days(5).build();
        none_ranks.core_rank = None;
        none_ranks.thcpl_rank = None;
        let mut core_b = Spec::new("CoreB").days(5).build();
        core_b.core_rank = Some("B".into());

        let filters = FilterState {
            core_ranks: set_of(&["A*"]),
            thcpl_ranks: set_of(&["B"]),
            ..Default::default()
        };
        let out = apply_filters(
            vec![none_ranks, core_b, Spec::new("Both").days(5).build()],
            &filters,
        );
        assert_eq!(titles(&out), ["NoRanks"]);
    }

    #[test]
    fn empty_rank_sets_disable_filtering() {
        let filters = FilterState {
            subs: set_of(&[]),
            ccf_ranks: set_of(&[]),
            core_ranks: set_of(&[]),
            thcpl_ranks: set_of(&[]),
            show_expired: true,
            query: String::new(),
        };
        assert_eq!(apply_filters(mixed(), &filters).len(), 6);
    }

    #[test]
    fn fuzzy_exact_partial_and_case() {
        let rows = vec![
            Spec::new("CVPR").days(5).build(),
            Spec::new("NeurIPS").days(5).build(),
            Spec::new("ICML").days(5).build(),
        ];
        assert_eq!(titles(&fuzzy_search(rows.clone(), "CVPR")), ["CVPR"]);
        assert_eq!(titles(&fuzzy_search(rows.clone(), "cvpr")), ["CVPR"]);
        assert_eq!(titles(&fuzzy_search(rows.clone(), "Neur")), ["NeurIPS"]);
        assert_eq!(fuzzy_search(rows.clone(), "").len(), 3);
        assert!(fuzzy_search(rows, "ZZZZZZZZZZNOTFOUND12345").is_empty());
        assert!(fuzzy_search(vec![], "CVPR").is_empty());
    }

    #[test]
    fn fuzzy_ranks_shorter_substring_match_first() {
        let rows = vec![
            Spec::new("International Conference on Computer Vision").build(),
            Spec::new("Computer Vision Workshop").build(),
        ];
        let out = fuzzy_search(rows, "Computer Vision");
        assert_eq!(
            titles(&out),
            [
                "Computer Vision Workshop",
                "International Conference on Computer Vision"
            ]
        );
    }

    #[test]
    fn fuzzy_substring_beats_similarity_and_tolerates_typos() {
        let rows = vec![
            Spec::new("ICDE").build(),
            Spec::new("ICDCS").build(),
            Spec::new("Zzzz").build(),
        ];
        let out = fuzzy_search(rows, "icde");
        assert_eq!(titles(&out)[0], "ICDE");
        let typo = fuzzy_search(vec![Spec::new("NeurIPS").build()], "neurpis");
        assert_eq!(titles(&typo), ["NeurIPS"]);
    }

    #[test]
    fn visible_rows_preserves_search_relevance() {
        let rows = vec![
            Spec::new("DASFAA").days(32).build(),
            Spec::new("ISCAS").days(1).build(),
            Spec::new("DSN").days(39).build(),
        ];
        let filters = FilterState {
            query: "dasfaa".to_string(),
            ..Default::default()
        };

        let searched = visible_rows(&rows, &filters);
        assert_eq!(titles(&searched)[0], "DASFAA");

        // The deadline ordering still applies when there is no search query.
        let unsearched = visible_rows(&rows, &FilterState::default());
        assert_eq!(titles(&unsearched)[0], "ISCAS");
    }

    #[test]
    fn sort_groups_in_order() {
        let sorted = sort_rows(mixed());
        assert_eq!(
            titles(&sorted),
            [
                "Soon",
                "Later",
                "Tbd2025",
                "Tbd2023",
                "Finished",
                "OldFinished"
            ]
        );
    }

    #[test]
    fn sort_running_by_deadline_ascending() {
        let rows = vec![
            Spec::new("c").days(30).build(),
            Spec::new("a").days(2).build(),
            Spec::new("b").days(9).build(),
        ];
        assert_eq!(titles(&sort_rows(rows)), ["a", "b", "c"]);
    }

    #[test]
    fn sort_favorites_first_then_by_deadline() {
        let rows = vec![
            Spec::new("Regular").days(1).build(),
            Spec::new("FavLater").days(30).fav().build(),
            Spec::new("FavSoon").days(5).fav().build(),
            Spec::new("FavTbd").tbd().fav().build(),
        ];
        assert_eq!(
            titles(&sort_rows(rows)),
            ["FavSoon", "FavLater", "FavTbd", "Regular"]
        );
    }

    #[test]
    fn sort_is_stable_for_equal_keys() {
        let rows = vec![
            Spec::new("t1").tbd().year(2024).build(),
            Spec::new("t2").tbd().year(2024).build(),
            Spec::new("t3").tbd().year(2025).build(),
        ];
        assert_eq!(titles(&sort_rows(rows)), ["t3", "t1", "t2"]);
        assert!(sort_rows(vec![]).is_empty());
    }

    #[test]
    fn visible_rows_pipeline() {
        let rows = vec![
            Spec::new("CVPR").days(10).build(),
            Spec::new("CVPR Workshop").days(2).build(),
            Spec::new("SIGMOD").sub("DB").days(1).build(),
            Spec::new("OldCVPR").days(-3).year(2020).build(),
        ];
        // Search keeps relevance order: the shorter exact substring match
        // comes first instead of being re-sorted by deadline.
        let filters = FilterState {
            query: "cvpr".into(),
            ..Default::default()
        };
        assert_eq!(
            titles(&visible_rows(&rows, &filters)),
            ["CVPR", "CVPR Workshop"]
        );

        let filters = FilterState {
            query: "cvpr".into(),
            show_expired: true,
            ..Default::default()
        };
        assert_eq!(visible_rows(&rows, &filters).len(), 3);
    }
}
