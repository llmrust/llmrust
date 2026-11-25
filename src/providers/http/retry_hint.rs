//! Strict Retry-After parsing and request-scoped carriage.
//!
//! The provider's existing `chat`/`stream`/`embed` signatures stay unchanged.
//! Their default `*_with_retry_after` companions establish a nested task-local
//! scope, while the shared compatible transport records the current response's
//! hint into that scope. The returned tuple is the retry decision's only input;
//! no provider instance owns a mutable hint slot.

use std::cell::RefCell;
use std::time::Duration;

use reqwest::header::RETRY_AFTER;

/// Maximum wait accepted from an upstream hint.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

tokio::task_local! {
    static REQUEST_RETRY_AFTER: RefCell<Option<Duration>>;
}

/// Run one provider call in an isolated hint scope and return its bound hint.
pub(crate) async fn scoped_call<F, T>(future: F) -> (T, Option<Duration>)
where
    F: std::future::Future<Output = T>,
{
    REQUEST_RETRY_AFTER
        .scope(RefCell::new(None), async {
            let output = future.await;
            let hint = REQUEST_RETRY_AFTER.with(|slot| slot.borrow_mut().take());
            (output, hint)
        })
        .await
}

/// Clear the active call's hint before a new transport attempt.
pub(crate) fn clear_response() {
    let _ = REQUEST_RETRY_AFTER.try_with(|slot| *slot.borrow_mut() = None);
}

/// Capture and parse the current response's Retry-After value, if any.
pub(crate) fn capture_response(response: &reqwest::Response) {
    let hint = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_retry_after(value, now_unix_secs()));
    // Direct legacy `chat()` calls have no scope and deliberately keep no state.
    let _ = REQUEST_RETRY_AFTER.try_with(|slot| *slot.borrow_mut() = hint);
}

pub(crate) fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse RFC 9110 delay-seconds or a strict IMF-fixdate HTTP-date.
///
/// Signed values, impossible calendar dates, non-GMT dates, and malformed
/// fields return `None`. Past dates clamp to zero and all waits cap at 60 s.
pub fn parse_retry_after(value: &str, now_unix_secs: i64) -> Option<Duration> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        let seconds = value.parse::<u64>().ok()?;
        return Some(Duration::from_secs(seconds).min(MAX_RETRY_AFTER));
    }
    let target = http_date_to_unix(value)?;
    let delta = target.saturating_sub(now_unix_secs);
    if delta <= 0 {
        return Some(Duration::ZERO);
    }
    Some(Duration::from_secs(delta as u64).min(MAX_RETRY_AFTER))
}

/// Parse exactly `Sun, 06 Nov 1994 08:49:37 GMT` and validate its calendar.
fn http_date_to_unix(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 29
        || bytes[3] != b','
        || bytes[4] != b' '
        || bytes[7] != b' '
        || bytes[11] != b' '
        || bytes[16] != b' '
        || bytes[19] != b':'
        || bytes[22] != b':'
        || bytes[25] != b' '
        || &bytes[26..] != b"GMT"
        || !bytes[..3].iter().all(u8::is_ascii_alphabetic)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..11].iter().all(u8::is_ascii_alphabetic)
        || !bytes[12..16].iter().all(u8::is_ascii_digit)
        || !bytes[17..19].iter().all(u8::is_ascii_digit)
        || !bytes[20..22].iter().all(u8::is_ascii_digit)
        || !bytes[23..25].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let weekday = std::str::from_utf8(&bytes[..3]).ok()?;
    if !matches!(
        weekday,
        "Sun" | "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat"
    ) {
        return None;
    }
    let day = two_digits(&bytes[5..7])?;
    let month = match std::str::from_utf8(&bytes[8..11]).ok()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = four_digits(&bytes[12..16])?;
    let hour = two_digits(&bytes[17..19])?;
    let minute = two_digits(&bytes[20..22])?;
    let second = two_digits(&bytes[23..25])?;
    if day == 0
        || day > i64::from(days_in_month(year, month))
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day as u32);
    Some(
        days.saturating_mul(86_400)
            .saturating_add(hour.saturating_mul(3_600))
            .saturating_add(minute.saturating_mul(60))
            .saturating_add(second),
    )
}

fn two_digits(bytes: &[u8]) -> Option<i64> {
    Some(i64::from(bytes[0] - b'0') * 10 + i64::from(bytes[1] - b'0'))
}

fn four_digits(bytes: &[u8]) -> Option<i64> {
    Some(
        bytes
            .iter()
            .fold(0_i64, |value, byte| value * 10 + i64::from(byte - b'0')),
    )
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Howard Hinnant's civil-date conversion, relative to 1970-01-01.
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year / 400;
    let year_of_era = year - era * 400;
    let month_prime = ((month + 9) % 12) as i64;
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
