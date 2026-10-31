//! Timestamps exchanged with macOS tools, as seconds since the Unix epoch.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Parses the UTC timestamps printed by `simctl` (`2026-09-24T21:48:39Z`)
/// and `mdls` (`2026-10-30 08:22:39 +0000`). Other offsets are rejected
/// rather than silently misread.
pub fn parse_utc(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, rest) = text.split_at_checked(10)?;
    let time = rest.strip_prefix(['T', ' '])?;
    let (clock, zone) = time.split_at_checked(8)?;
    if !matches!(zone.trim(), "Z" | "+0000" | "+00:00") {
        return None;
    }

    let mut date_parts = date.split('-').map(str::parse::<i64>);
    let (year, month, day) = (
        date_parts.next()?.ok()?,
        date_parts.next()?.ok()?,
        date_parts.next()?.ok()?,
    );
    let mut clock_parts = clock.split(':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        clock_parts.next()?.ok()?,
        clock_parts.next()?.ok()?,
        clock_parts.next()?.ok()?,
    );
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(seconds).ok()
}

/// Today's date (UTC) as `2026-10-31`, for file names.
pub fn today() -> String {
    let (year, month, day) = civil_from_days((unix_seconds(SystemTime::now()) / 86_400) as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

/// L'inverse de [`days_from_civil`], tjs Howard Hinnant.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_round_trip() {
        for days in [0, 19_000, 20_757, -1, 365 * 400] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(20_757), (2026, 10, 31));
    }

    #[test]
    fn parses_simctl_timestamps() {
        assert_eq!(parse_utc("2026-09-24T21:48:39Z"), Some(1_790_286_519));
    }

    #[test]
    fn parses_mdls_timestamps() {
        assert_eq!(parse_utc("1970-01-02 00:00:01 +0000"), Some(86_401));
    }

    #[test]
    fn handles_leap_years() {
        assert_eq!(parse_utc("2024-03-01T00:00:00Z"), Some(1_709_251_200));
    }

    #[test]
    fn rejects_other_offsets_and_garbage() {
        assert_eq!(parse_utc("2026-09-24T21:48:39+0200"), None);
        assert_eq!(parse_utc("(null)"), None);
        assert_eq!(parse_utc("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_utc(""), None);
    }
}
