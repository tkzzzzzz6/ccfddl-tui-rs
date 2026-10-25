//! Timezone and datetime parsing for CCFDDL deadlines.
//!
//! Supported timezone strings:
//! - `AoE` (Anywhere on Earth, UTC-12)
//! - `UTC`
//! - `UTC+8`, `UTC-5` (offset validated to the range -12..=14)
//! - `PT` (US Pacific Time, DST aware). The Python implementation rejected
//!   this value, which silently turned ~34 real conference entries into
//!   "N/A / expired"; the Rust port resolves it properly.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use thiserror::Error;

pub const MIN_TZ_OFFSET: i32 = -12;
pub const MAX_TZ_OFFSET: i32 = 14;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimezoneError {
    #[error("Invalid timezone format: {0}")]
    InvalidFormat(String),
    #[error("Timezone offset {0} out of valid range [{MIN_TZ_OFFSET}, {MAX_TZ_OFFSET}]")]
    OffsetOutOfRange(i32),
    #[error("Failed to parse datetime '{0}'")]
    InvalidDatetime(String),
    #[error("Datetime '{0}' does not exist in timezone '{1}'")]
    NonexistentLocalTime(String, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfTimezone {
    Fixed(FixedOffset),
    Named(Tz),
}

fn fixed_hours(hours: i32) -> FixedOffset {
    FixedOffset::east_opt(hours * 3600).expect("offset within +-24h")
}

pub fn get_timezone(tz_str: &str) -> Result<ConfTimezone, TimezoneError> {
    match tz_str {
        "AoE" => return Ok(ConfTimezone::Fixed(fixed_hours(-12))),
        "UTC" => return Ok(ConfTimezone::Fixed(fixed_hours(0))),
        "PT" => return Ok(ConfTimezone::Named(chrono_tz::America::Los_Angeles)),
        _ => {}
    }

    let invalid = || TimezoneError::InvalidFormat(tz_str.to_string());
    let rest = tz_str.strip_prefix("UTC").ok_or_else(invalid)?;
    let (negative, digits) = if let Some(d) = rest.strip_prefix('+') {
        (false, d)
    } else if let Some(d) = rest.strip_prefix('-') {
        (true, d)
    } else {
        return Err(invalid());
    };

    if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let hours: i32 = digits.parse().map_err(|_| invalid())?;
    let offset = if negative { -hours } else { hours };

    if !(MIN_TZ_OFFSET..=MAX_TZ_OFFSET).contains(&offset) {
        return Err(TimezoneError::OffsetOutOfRange(offset));
    }
    Ok(ConfTimezone::Fixed(fixed_hours(offset)))
}

const DATETIME_FORMATS: [&str; 6] = [
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S",
    "%Y-%m-%d %H:%M",
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%dT%H:%M",
];

fn parse_naive(dt_str: &str) -> Option<NaiveDateTime> {
    let s = dt_str.trim();
    for format in DATETIME_FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, format) {
            return Some(dt);
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// Interpret `dt_str` as wall-clock time in `tz_str` and return the instant.
pub fn parse_datetime_with_tz(
    dt_str: &str,
    tz_str: &str,
) -> Result<DateTime<FixedOffset>, TimezoneError> {
    let tz = get_timezone(tz_str)?;
    let naive =
        parse_naive(dt_str).ok_or_else(|| TimezoneError::InvalidDatetime(dt_str.to_string()))?;
    let nonexistent =
        || TimezoneError::NonexistentLocalTime(dt_str.to_string(), tz_str.to_string());

    match tz {
        ConfTimezone::Fixed(offset) => offset
            .from_local_datetime(&naive)
            .single()
            .ok_or_else(nonexistent),
        ConfTimezone::Named(zone) => zone
            .from_local_datetime(&naive)
            .earliest()
            .map(|dt| dt.fixed_offset())
            .ok_or_else(nonexistent),
    }
}

pub fn parse_deadline_utc(dt_str: &str, tz_str: &str) -> Result<DateTime<Utc>, TimezoneError> {
    parse_datetime_with_tz(dt_str, tz_str).map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offset_secs(tz: &str) -> i32 {
        match get_timezone(tz).unwrap() {
            ConfTimezone::Fixed(o) => o.local_minus_utc(),
            ConfTimezone::Named(_) => panic!("expected fixed offset"),
        }
    }

    #[test]
    fn named_fixed_timezones() {
        assert_eq!(offset_secs("AoE"), -12 * 3600);
        assert_eq!(offset_secs("UTC"), 0);
        assert_eq!(offset_secs("UTC+8"), 8 * 3600);
        assert_eq!(offset_secs("UTC-5"), -5 * 3600);
        assert_eq!(offset_secs("UTC+0"), 0);
        assert_eq!(offset_secs("UTC+14"), 14 * 3600);
        assert_eq!(offset_secs("UTC-12"), -12 * 3600);
        assert_eq!(offset_secs("UTC+08"), 8 * 3600);
    }

    #[test]
    fn invalid_timezones() {
        for bad in [
            "", "GMT", "UTC+", "UTC8", "UTC+123", "utc+8", "UTC+5:30", "UTC +8", "PST",
        ] {
            assert!(
                matches!(get_timezone(bad), Err(TimezoneError::InvalidFormat(_))),
                "{bad} should be invalid"
            );
        }
        assert_eq!(
            get_timezone("UTC+15"),
            Err(TimezoneError::OffsetOutOfRange(15))
        );
        assert_eq!(
            get_timezone("UTC-13"),
            Err(TimezoneError::OffsetOutOfRange(-13))
        );
    }

    #[test]
    fn parse_aoe_deadline() {
        let dt = parse_deadline_utc("2025-01-15 23:59:59", "AoE").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-16T11:59:59+00:00");
    }

    #[test]
    fn parse_positive_offset() {
        let dt = parse_deadline_utc("2025-01-15 08:00:00", "UTC+8").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-15T00:00:00+00:00");
    }

    #[test]
    fn pt_respects_daylight_saving() {
        let winter = parse_deadline_utc("2025-01-15 12:00:00", "PT").unwrap();
        assert_eq!(winter.to_rfc3339(), "2025-01-15T20:00:00+00:00");
        let summer = parse_deadline_utc("2025-07-15 12:00:00", "PT").unwrap();
        assert_eq!(summer.to_rfc3339(), "2025-07-15T19:00:00+00:00");
    }

    #[test]
    fn pt_ambiguous_time_takes_first_occurrence() {
        let dt = parse_deadline_utc("2025-11-02 01:30:00", "PT").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-11-02T08:30:00+00:00");
    }

    #[test]
    fn pt_nonexistent_time_is_an_error() {
        let err = parse_deadline_utc("2025-03-09 02:30:00", "PT").unwrap_err();
        assert!(matches!(err, TimezoneError::NonexistentLocalTime(..)));
    }

    #[test]
    fn flexible_datetime_formats() {
        let expected = parse_deadline_utc("2025-03-01 10:30:00", "UTC").unwrap();
        for s in [
            "2025-03-01 10:30",
            "2025-03-01T10:30:00",
            "  2025-03-01 10:30:00  ",
        ] {
            assert_eq!(parse_deadline_utc(s, "UTC").unwrap(), expected, "{s}");
        }
        let midnight = parse_deadline_utc("2025-03-01", "UTC").unwrap();
        assert_eq!(midnight.to_rfc3339(), "2025-03-01T00:00:00+00:00");
        let fractional = parse_deadline_utc("2025-03-01 10:30:00.250", "UTC").unwrap();
        assert!(fractional > expected);
    }

    #[test]
    fn invalid_datetime_and_timezone_errors() {
        assert!(matches!(
            parse_deadline_utc("not a date", "UTC"),
            Err(TimezoneError::InvalidDatetime(_))
        ));
        assert!(matches!(
            parse_deadline_utc("2025-13-45 00:00:00", "UTC"),
            Err(TimezoneError::InvalidDatetime(_))
        ));
        assert!(matches!(
            parse_deadline_utc("2025-01-01 00:00:00", "Mars"),
            Err(TimezoneError::InvalidFormat(_))
        ));
        assert!(matches!(
            parse_deadline_utc("TBD", "UTC"),
            Err(TimezoneError::InvalidDatetime(_))
        ));
    }
}
