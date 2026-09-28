//! What a provider's response headers say: the id support needs to find a
//! request, and how long to wait before sending it again.
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The header TypeSafe names each request by.
pub const REQUEST_ID: &str = "x-typesafe-request-id";
/// The longest request id kept; a longer or stranger value is not an id.
const MAX_REQUEST_ID_BYTES: usize = 128;

/// A request id fit to print: letters, digits and `._:-`. Anything else is
/// dropped, so a header cannot carry text into messages or logs.
pub fn request_id(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()
        && value.len() <= MAX_REQUEST_ID_BYTES
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c)))
    .then(|| value.to_owned())
}

/// How long the provider asks to wait before a retry: `retry-after-ms`, which
/// TypeSafe's SDKs honor first, else `Retry-After` in seconds or as an HTTP
/// date. A date in the past asks for no wait.
pub fn retry_after(ms: Option<&str>, value: Option<&str>, now: SystemTime) -> Option<Duration> {
    if let Some(wait) = ms
        .and_then(|ms| ms.trim().parse::<f64>().ok())
        .and_then(|ms| Duration::try_from_secs_f64(ms / 1000.0).ok())
    {
        return Some(wait);
    }
    let value = value?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    http_date(value).map(|date| date.duration_since(now).unwrap_or_default())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const SECONDS_PER_DAY: u64 = 86_400;
/// Days from 0000-03-01 to 1970-01-01 in the proleptic Gregorian calendar.
const EPOCH_DAYS: u64 = 719_468;

/// An HTTP date in the form HTTP requires senders to use, as in
/// `Sun, 06 Nov 1994 08:49:37 GMT`.
fn http_date(text: &str) -> Option<SystemTime> {
    let parts: Vec<&str> = text.split_ascii_whitespace().collect();
    if parts.len() != 6 || parts[5] != "GMT" {
        return None;
    }
    let day: u64 = parts[1].parse().ok()?;
    let month = MONTHS.iter().position(|m| *m == parts[2])? as u64 + 1;
    let year: u64 = parts[3].parse().ok().filter(|year| *year >= 1970)?;
    let clock: Vec<u64> = parts[4]
        .split(':')
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    let [hour, minute, second] = clock[..] else {
        return None;
    };
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_since_epoch(year, month, day);
    Some(
        UNIX_EPOCH
            + Duration::from_secs(days * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second),
    )
}

/// Days from 1970-01-01 to a date from 1970 on: Howard Hinnant's
/// `days_from_civil`, counting years from March so leap days fall last.
fn days_since_epoch(year: u64, month: u64, day: u64) -> u64 {
    let year = if month <= 2 { year - 1 } else { year };
    let (era, year_of_era) = (year / 400, year % 400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - EPOCH_DAYS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_id_is_kept_only_when_it_is_safe_to_print() {
        assert_eq!(
            request_id(Some(" req_01J9-abc.1:2 ")).as_deref(),
            Some("req_01J9-abc.1:2")
        );
        for value in [
            "",
            "id with space",
            "id\r\nX-Injected: 1",
            "\u{1b}[31m",
            &"a".repeat(129),
        ] {
            assert_eq!(request_id(Some(value)), None, "{value:?}");
        }
        assert_eq!(request_id(None), None);
    }

    #[test]
    fn retry_after_ms_wins_then_seconds_then_an_http_date() {
        let now = UNIX_EPOCH + Duration::from_secs(784_111_777);
        let wait = |ms, value| retry_after(ms, value, now);
        assert_eq!(
            wait(Some("1500"), Some("9")),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(
            wait(Some("12.5"), None),
            Some(Duration::from_micros(12_500))
        );
        assert_eq!(wait(Some("-1"), Some("9")), Some(Duration::from_secs(9)));
        assert_eq!(
            wait(Some("soon"), Some(" 2 ")),
            Some(Duration::from_secs(2))
        );
        // 784111777 is Sun, 06 Nov 1994 08:49:37 GMT.
        assert_eq!(
            wait(None, Some("Sun, 06 Nov 1994 08:50:07 GMT")),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            wait(None, Some("Sun, 06 Nov 1994 08:49:00 GMT")),
            Some(Duration::ZERO)
        );
        for value in [
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 32 Nov 1994 08:49:37 GMT",
            "tomorrow",
        ] {
            assert_eq!(wait(None, Some(value)), None, "{value}");
        }
        assert_eq!(wait(None, None), None);
    }

    #[test]
    fn http_dates_count_leap_days() {
        let seconds = |text| {
            http_date(text)
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        };
        assert_eq!(seconds("Thu, 01 Jan 1970 00:00:00 GMT"), 0);
        assert_eq!(seconds("Tue, 29 Feb 2000 00:00:00 GMT"), 951_782_400);
        assert_eq!(seconds("Mon, 28 Sep 2026 12:00:00 GMT"), 1_790_596_800);
    }
}
