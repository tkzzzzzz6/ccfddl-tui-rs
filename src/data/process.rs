//! Turns raw conferences into display rows (ports `_find_best_conf_year` / `_create_row`).

use chrono::{DateTime, Utc};

use super::favorites::Favorites;
use super::row::ConferenceRow;
use crate::format::format_duration;
use crate::model::{Conference, ConferenceYear};
use crate::timezone::parse_deadline_utc;

fn is_tbd(deadline: &str) -> bool {
    deadline.eq_ignore_ascii_case("TBD")
}

pub fn process_rows(
    conferences: &[Conference],
    now: DateTime<Utc>,
    favorites: &Favorites,
) -> Vec<ConferenceRow> {
    conferences
        .iter()
        .filter_map(|conf| find_best_conf_year(conf, now, favorites))
        .collect()
}

/// Pick the edition/deadline to show for a conference:
/// 1. the earliest upcoming deadline across all editions;
/// 2. otherwise a TBD deadline (real deadlines always win over TBD);
/// 3. otherwise the most recent edition (shown as finished).
pub fn find_best_conf_year(
    conf: &Conference,
    now: DateTime<Utc>,
    favorites: &Favorites,
) -> Option<ConferenceRow> {
    if conf.confs.is_empty() {
        return None;
    }

    let mut best_year: Option<&ConferenceYear> = None;
    let mut best_deadline: Option<DateTime<Utc>> = None;
    let mut best_is_tbd = false;

    for conf_year in &conf.confs {
        for timeline in &conf_year.timeline {
            if is_tbd(&timeline.deadline) {
                if best_deadline.is_none() && !best_is_tbd {
                    best_year = Some(conf_year);
                    best_is_tbd = true;
                }
                continue;
            }

            match parse_deadline_utc(&timeline.deadline, &conf_year.timezone) {
                Ok(deadline) => {
                    if deadline <= now {
                        continue;
                    }
                    if best_deadline.is_none_or(|best| deadline < best) {
                        best_deadline = Some(deadline);
                        best_year = Some(conf_year);
                        best_is_tbd = false;
                    }
                }
                Err(e) => log::warn!(
                    "Failed to parse deadline '{}' for {}: {e}",
                    timeline.deadline,
                    conf.title
                ),
            }
        }
    }

    if best_year.is_none() {
        // Most recent year; the first one wins on ties, like a stable descending sort.
        let mut latest = &conf.confs[0];
        for conf_year in &conf.confs[1..] {
            if conf_year.year > latest.year {
                latest = conf_year;
            }
        }
        best_year = Some(latest);

        if let Some(first) = latest.timeline.first() {
            if is_tbd(&first.deadline) {
                best_is_tbd = true;
            } else if let Ok(deadline) = parse_deadline_utc(&first.deadline, &latest.timezone) {
                best_deadline = Some(deadline);
            }
        }
    }

    best_year.map(|conf_year| create_row(conf, conf_year, best_deadline, best_is_tbd, now, favorites))
}

pub fn create_row(
    conf: &Conference,
    conf_year: &ConferenceYear,
    deadline: Option<DateTime<Utc>>,
    is_tbd: bool,
    now: DateTime<Utc>,
    favorites: &Favorites,
) -> ConferenceRow {
    let (countdown, is_running) = if is_tbd {
        ("TBD".to_string(), true)
    } else {
        match deadline {
            None => ("N/A".to_string(), false),
            Some(d) if d > now => (format_duration(d, now), true),
            Some(_) => ("Finished".to_string(), false),
        }
    };

    ConferenceRow {
        title: conf.title.clone(),
        year: conf_year.year,
        sub: conf.sub.clone(),
        rank: conf.rank.ccf.clone(),
        core_rank: conf.rank.core.clone(),
        thcpl_rank: conf.rank.thcpl.clone(),
        deadline,
        countdown,
        place: conf_year.place.clone(),
        date: conf_year.date.clone(),
        link: conf_year.link.clone(),
        is_running,
        is_tbd,
        is_expired: !is_running && !is_tbd,
        is_favorite: favorites.contains(&conf.title, conf_year.year),
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::model::{Rank, Timeline};
    use chrono::TimeZone;

    /// 2025-01-15 12:00:00 UTC
    pub fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 1, 15, 12, 0, 0).unwrap()
    }

    pub fn no_favorites() -> Favorites {
        Favorites::empty_at(std::env::temp_dir().join("ccfddl-tui-test-never-written.json"))
    }

    pub fn year(
        year: i32,
        timezone: &str,
        deadlines: &[&str],
    ) -> ConferenceYear {
        ConferenceYear {
            year,
            id: format!("conf{year}"),
            link: "https://example.com".into(),
            timeline: deadlines.iter().map(|d| Timeline::new(*d)).collect(),
            timezone: timezone.into(),
            date: format!("June {year}"),
            place: "City".into(),
        }
    }

    pub fn conference(title: &str, confs: Vec<ConferenceYear>) -> Conference {
        Conference {
            title: title.into(),
            description: format!("{title} description"),
            sub: "AI".into(),
            rank: Rank { ccf: "A".into(), core: Some("A*".into()), thcpl: Some("A".into()) },
            dblp: title.to_lowercase(),
            confs,
        }
    }

    /// CVPR 2025, deadline 2024-11-15 23:59:59 UTC-8 (already past at `fixed_now`).
    pub fn sample_conference() -> Conference {
        conference("CVPR", vec![year(2025, "UTC-8", &["2024-11-15 23:59:59"])])
    }

    pub fn sample_conference_tbd() -> Conference {
        conference("NeurIPS", vec![year(2025, "UTC-12", &["TBD"])])
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use chrono::{Duration, TimeZone};

    fn fmt(dt: DateTime<Utc>) -> String {
        dt.format("%Y-%m-%d %H:%M:%S").to_string()
    }

    #[test]
    fn upcoming_deadline_is_selected() {
        let now = fixed_now();
        let deadline = fmt(now + Duration::days(30));
        let conf = conference("CVPR", vec![year(2025, "UTC", &[&deadline])]);

        let row = find_best_conf_year(&conf, now, &no_favorites()).unwrap();
        assert_eq!(row.title, "CVPR");
        assert!(row.is_running);
        assert!(!row.is_tbd);
        assert!(!row.is_expired);
        assert_eq!(row.deadline, Some(now + Duration::days(30)));
        assert_eq!(row.countdown, "1 month");
    }

    #[test]
    fn tbd_deadline() {
        let row =
            find_best_conf_year(&sample_conference_tbd(), fixed_now(), &no_favorites()).unwrap();
        assert!(row.is_tbd);
        assert!(row.is_running);
        assert!(!row.is_expired);
        assert_eq!(row.countdown, "TBD");
        assert_eq!(row.deadline, None);
    }

    #[test]
    fn tbd_is_case_insensitive() {
        let conf = conference("X", vec![year(2025, "UTC", &["tbd"])]);
        assert!(find_best_conf_year(&conf, fixed_now(), &no_favorites()).unwrap().is_tbd);
    }

    #[test]
    fn no_confs_gives_none() {
        let conf = conference("EmptyConf", vec![]);
        assert!(find_best_conf_year(&conf, fixed_now(), &no_favorites()).is_none());
    }

    #[test]
    fn expired_falls_back_to_most_recent_year() {
        let now = fixed_now();
        let past = fmt(now - Duration::days(30));
        let conf = conference(
            "ExpiredConf",
            vec![year(2023, "UTC", &["2023-01-01 00:00:00"]), year(2024, "UTC", &[&past])],
        );

        let row = find_best_conf_year(&conf, now, &no_favorites()).unwrap();
        assert_eq!(row.year, 2024);
        assert!(!row.is_running);
        assert!(row.is_expired);
        assert_eq!(row.countdown, "Finished");
        assert_eq!(row.deadline, Some(now - Duration::days(30)));
    }

    #[test]
    fn fallback_year_tie_prefers_first_listed() {
        let mut a = year(2024, "UTC", &["2024-01-01 00:00:00"]);
        a.id = "first".into();
        let mut b = year(2024, "UTC", &["2024-02-01 00:00:00"]);
        b.id = "second".into();
        let conf = conference("Tie", vec![a, b]);
        let row = find_best_conf_year(&conf, fixed_now(), &no_favorites()).unwrap();
        assert_eq!(row.date, "June 2024");
        assert_eq!(row.deadline.unwrap().to_rfc3339(), "2024-01-01T00:00:00+00:00");
    }

    #[test]
    fn expired_with_tbd_first_timeline_is_tbd() {
        let conf = conference("X", vec![year(2024, "UTC", &["TBD", "2020-01-01 00:00:00"])]);
        let row = find_best_conf_year(&conf, fixed_now(), &no_favorites()).unwrap();
        assert!(row.is_tbd);
    }

    #[test]
    fn year_without_timeline_is_na_and_expired() {
        let conf = conference("X", vec![year(2024, "UTC", &[])]);
        let row = find_best_conf_year(&conf, fixed_now(), &no_favorites()).unwrap();
        assert_eq!(row.countdown, "N/A");
        assert!(!row.is_running);
        assert!(row.is_expired);
    }

    #[test]
    fn earliest_upcoming_timeline_wins() {
        let now = fixed_now();
        let later = fmt(now + Duration::days(60));
        let sooner = fmt(now + Duration::days(30));
        let conf = conference("SIGMOD", vec![year(2025, "UTC", &[&later, &sooner])]);

        let row = find_best_conf_year(&conf, now, &no_favorites()).unwrap();
        assert_eq!(row.deadline, Some(now + Duration::days(30)));
    }

    #[test]
    fn earliest_across_years_wins() {
        let now = fixed_now();
        let conf = conference(
            "ICML",
            vec![
                year(2026, "UTC", &["2025-06-01 00:00:00"]),
                year(2025, "UTC", &["2025-03-01 00:00:00"]),
            ],
        );
        let row = find_best_conf_year(&conf, now, &no_favorites()).unwrap();
        assert_eq!(row.year, 2025);
    }

    #[test]
    fn real_deadline_beats_tbd_in_either_order() {
        let now = fixed_now();
        let real = "2025-06-01 00:00:00";
        for timelines in [vec!["TBD", real], vec![real, "TBD"]] {
            let conf = conference("X", vec![year(2025, "UTC", &timelines)]);
            let row = find_best_conf_year(&conf, now, &no_favorites()).unwrap();
            assert!(!row.is_tbd, "{timelines:?}");
            assert!(row.deadline.is_some());
        }
        let conf = conference(
            "X",
            vec![year(2025, "UTC", &["TBD"]), year(2026, "UTC", &[real])],
        );
        let row = find_best_conf_year(&conf, now, &no_favorites()).unwrap();
        assert_eq!((row.year, row.is_tbd), (2026, false));
    }

    #[test]
    fn unparseable_deadlines_are_skipped() {
        let conf = conference(
            "X",
            vec![year(2025, "UTC", &["garbage", "2025-06-01 00:00:00"]), year(2026, "Mars", &["2026-01-01 00:00:00"])],
        );
        let row = find_best_conf_year(&conf, fixed_now(), &no_favorites()).unwrap();
        assert_eq!(row.year, 2025);
    }

    #[test]
    fn deadline_exactly_now_counts_as_past() {
        let conf = conference("X", vec![year(2025, "UTC", &["2025-01-15 12:00:00"])]);
        let row = find_best_conf_year(&conf, fixed_now(), &no_favorites()).unwrap();
        assert!(!row.is_running);
        assert_eq!(row.countdown, "Finished");
    }

    #[test]
    fn pt_timezone_resolves_instead_of_failing() {
        let conf = conference("VLDB", vec![year(2025, "PT", &["2025-03-01 12:00:00"])]);
        let row = find_best_conf_year(&conf, fixed_now(), &no_favorites()).unwrap();
        assert!(row.is_running);
        assert_eq!(row.deadline.unwrap().to_rfc3339(), "2025-03-01T20:00:00+00:00");
    }

    #[test]
    fn create_row_running() {
        let conf = sample_conference();
        let now = fixed_now();
        let row = create_row(&conf, &conf.confs[0], Some(now + Duration::days(10)), false, now, &no_favorites());
        assert_eq!(
            (row.title.as_str(), row.year, row.sub.as_str(), row.rank.as_str()),
            ("CVPR", 2025, "AI", "A")
        );
        assert_eq!(row.core_rank.as_deref(), Some("A*"));
        assert!(row.is_running && !row.is_tbd && !row.is_expired);
        assert_eq!(row.countdown, "10 days, 00:00");
    }

    #[test]
    fn create_row_tbd() {
        let conf = sample_conference_tbd();
        let row = create_row(&conf, &conf.confs[0], None, true, fixed_now(), &no_favorites());
        assert!(row.is_tbd && row.is_running && !row.is_expired);
        assert_eq!(row.countdown, "TBD");
    }

    #[test]
    fn create_row_finished() {
        let conf = sample_conference();
        let now = fixed_now();
        let row = create_row(&conf, &conf.confs[0], Some(now - Duration::days(10)), false, now, &no_favorites());
        assert!(!row.is_running && !row.is_tbd && row.is_expired);
        assert_eq!(row.countdown, "Finished");
    }

    #[test]
    fn create_row_marks_favorites() {
        let dir = tempfile::tempdir().unwrap();
        let mut favorites = Favorites::empty_at(dir.path().join("f.json"));
        favorites.toggle("CVPR", 2025);

        let conf = sample_conference();
        let row = create_row(&conf, &conf.confs[0], None, false, fixed_now(), &favorites);
        assert!(row.is_favorite);
        let other = conference("CVPR", vec![year(2026, "UTC", &[])]);
        let row = create_row(&other, &other.confs[0], None, false, fixed_now(), &favorites);
        assert!(!row.is_favorite);
    }

    #[test]
    fn process_rows_empty_and_filters_confless() {
        let now = fixed_now();
        assert!(process_rows(&[], now, &no_favorites()).is_empty());

        let confs = vec![
            conference("Empty", vec![]),
            sample_conference_tbd(),
            conference("Dated", vec![year(2026, "AoE", &["2026-03-01 23:59:59"])]),
        ];
        let rows = process_rows(&confs, now, &no_favorites());
        assert_eq!(rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["NeurIPS", "Dated"]);
        assert_eq!(
            rows[1].deadline,
            Some(Utc.with_ymd_and_hms(2026, 3, 2, 11, 59, 59).unwrap())
        );
    }
}
