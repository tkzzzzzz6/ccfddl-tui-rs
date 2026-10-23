//! Countdown formatting (ports `formatters.py` and `format_duration`).

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountdownColor {
    Red,
    Yellow,
    Blue,
    Green,
    Dim,
}

impl CountdownColor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Dim => "dim",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountdownText {
    pub text: String,
    pub color: CountdownColor,
}

impl CountdownText {
    fn new(text: impl Into<String>, color: CountdownColor) -> Self {
        Self { text: text.into(), color }
    }
}

pub fn get_countdown_color(days: i64) -> CountdownColor {
    match days {
        d if d < 1 => CountdownColor::Red,
        d if d < 7 => CountdownColor::Yellow,
        d if d < 30 => CountdownColor::Blue,
        _ => CountdownColor::Green,
    }
}

fn plural(count: i64, word: &str) -> String {
    if count == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

/// Live countdown text: `5 days 12:30:45` or `05:30:45` when under a day.
pub fn format_time_remaining(days: i64, total_seconds: i64) -> String {
    let minutes = total_seconds.rem_euclid(3600) / 60;
    let seconds = total_seconds.rem_euclid(60);

    if days >= 1 {
        let hours = total_seconds.div_euclid(3600).rem_euclid(24);
        format!("{days} {} {hours:02}:{minutes:02}:{seconds:02}", plural(days, "day"))
    } else {
        let total_hours = total_seconds.div_euclid(3600);
        format!("{total_hours:02}:{minutes:02}:{seconds:02}")
    }
}

pub fn format_deadline_countdown(
    deadline: Option<DateTime<Utc>>,
    is_tbd: bool,
    now: DateTime<Utc>,
) -> CountdownText {
    if is_tbd {
        return CountdownText::new("TBD", CountdownColor::Dim);
    }
    let Some(deadline) = deadline else {
        return CountdownText::new("N/A", CountdownColor::Dim);
    };
    if deadline <= now {
        return CountdownText::new("Expired", CountdownColor::Red);
    }
    let (text, color) = format_countdown_from_duration(deadline - now);
    CountdownText::new(text, color)
}

pub fn format_countdown_from_duration(remaining: Duration) -> (String, CountdownColor) {
    if remaining <= Duration::zero() {
        return ("Expired".to_string(), CountdownColor::Red);
    }
    let total_seconds = remaining.num_seconds();
    let total_days = remaining.num_days();
    (
        format_time_remaining(total_days, total_seconds),
        get_countdown_color(total_days),
    )
}

/// Coarse remaining time stored in a row: `05:30:12`, `12 days, 03:45`, `2 months, 5 days`.
pub fn format_duration(ddl_time: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let duration = ddl_time - now;
    if duration <= Duration::zero() {
        return "Expired".to_string();
    }

    let secs = duration.num_seconds();
    let total_days = secs / 86_400;
    let rem = secs % 86_400;
    let hours = rem / 3600;
    let minutes = rem % 3600 / 60;
    let seconds = rem % 60;
    let day_word = plural(total_days, "day");

    if total_days < 1 {
        return format!("{hours:02}:{minutes:02}:{seconds:02}");
    }
    if total_days < 30 {
        return format!("{total_days:02} {day_word}, {hours:02}:{minutes:02}");
    }

    let months = total_days / 30;
    let remaining_days = total_days % 30;
    let month_word = plural(months, "month");
    if remaining_days > 0 {
        // The Python original reuses the pluralisation of the *total* day count,
        // so this reads "1 month, 1 days". Kept as-is for output parity.
        format!("{months} {month_word}, {remaining_days} {day_word}")
    } else {
        format!("{months} {month_word}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 1, 15, 12, 0, 0).unwrap()
    }

    fn secs(days: i64, h: i64, m: i64, s: i64) -> i64 {
        ((days * 24 + h) * 60 + m) * 60 + s
    }

    #[test]
    fn color_thresholds() {
        use CountdownColor::*;
        assert_eq!(get_countdown_color(0), Red);
        assert_eq!(get_countdown_color(-1), Red);
        for d in [1, 3, 6] {
            assert_eq!(get_countdown_color(d), Yellow);
        }
        for d in [7, 15, 29] {
            assert_eq!(get_countdown_color(d), Blue);
        }
        for d in [30, 100] {
            assert_eq!(get_countdown_color(d), Green);
        }
        assert_eq!(Green.as_str(), "green");
    }

    #[test]
    fn time_remaining_less_than_a_day() {
        let text = format_time_remaining(0, secs(0, 5, 30, 45));
        assert_eq!(text, "05:30:45");
        assert!(!text.contains("day"));
    }

    #[test]
    fn time_remaining_more_than_a_day() {
        assert_eq!(format_time_remaining(5, secs(5, 12, 30, 45)), "5 days 12:30:45");
    }

    #[test]
    fn time_remaining_single_day() {
        let text = format_time_remaining(1, secs(1, 2, 30, 0));
        assert!(text.starts_with("1 day "), "{text}");
        assert_eq!(text, "1 day 02:30:00");
    }

    #[test]
    fn countdown_tbd_and_missing() {
        let tbd = format_deadline_countdown(None, true, now());
        assert_eq!((tbd.text.as_str(), tbd.color), ("TBD", CountdownColor::Dim));
        let na = format_deadline_countdown(None, false, now());
        assert_eq!((na.text.as_str(), na.color), ("N/A", CountdownColor::Dim));
    }

    #[test]
    fn countdown_expired() {
        let past = now() - Duration::hours(1);
        let c = format_deadline_countdown(Some(past), false, now());
        assert_eq!((c.text.as_str(), c.color), ("Expired", CountdownColor::Red));
        let exact = format_deadline_countdown(Some(now()), false, now());
        assert_eq!(exact.text, "Expired");
    }

    #[test]
    fn countdown_running() {
        let c = format_deadline_countdown(Some(now() + Duration::days(5)), false, now());
        assert_eq!(c.text, "5 days 00:00:00");
        assert_eq!(c.color, CountdownColor::Yellow);
        let soon = format_deadline_countdown(Some(now() + Duration::minutes(90)), false, now());
        assert_eq!((soon.text.as_str(), soon.color), ("01:30:00", CountdownColor::Red));
    }

    #[test]
    fn countdown_from_duration() {
        let (text, color) = format_countdown_from_duration(Duration::days(5) + Duration::hours(12));
        assert!(text.contains("5 days"));
        assert_eq!(color, CountdownColor::Yellow);
        let (text, color) = format_countdown_from_duration(Duration::seconds(-5));
        assert_eq!((text.as_str(), color), ("Expired", CountdownColor::Red));
    }

    #[test]
    fn duration_formats() {
        let n = now();
        assert_eq!(format_duration(n - Duration::seconds(1), n), "Expired");
        assert_eq!(format_duration(n, n), "Expired");
        assert_eq!(format_duration(n + Duration::milliseconds(500), n), "00:00:00");
        assert_eq!(format_duration(n + Duration::seconds(5 * 3600 + 30 * 60 + 12), n), "05:30:12");
        assert_eq!(format_duration(n + Duration::days(1), n), "01 day, 00:00");
        assert_eq!(
            format_duration(n + Duration::days(12) + Duration::hours(3) + Duration::minutes(45), n),
            "12 days, 03:45"
        );
        assert_eq!(format_duration(n + Duration::days(30), n), "1 month");
        assert_eq!(format_duration(n + Duration::days(60), n), "2 months");
        assert_eq!(format_duration(n + Duration::days(65), n), "2 months, 5 days");
        assert_eq!(format_duration(n + Duration::days(31), n), "1 month, 1 days");
    }
}
