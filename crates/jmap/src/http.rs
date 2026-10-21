// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The transport-neutral request, response and error types of the JMAP client, and the [`Http`] trait a
//! transport implements.
//!
//! Also the pure pieces the transports share: [`retry_after`] (RFC 9110 §10.2.3) and the `Authorization`
//! header of [`Auth`] (RFC 7617 Basic, RFC 6750 Bearer).

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::url::Url;

/// What went wrong with an exchange. The `Display` texts are fixed: the program translates them by their
/// English wording.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    /// The request never got an answer (network failure, timeout).
    #[error("network: {0}")]
    Io(String),
    /// The secure connection failed.
    #[error("secure connection: {0}")]
    Tls(String),
    /// An answer the protocol does not allow, or an address that is not valid.
    #[error("unexpected server response: {0}")]
    Protocol(String),
    /// The server answered with a JMAP or problem-details error type and its human description.
    #[error("the server refused: {}", refused(.kind, .description))]
    Refused {
        /// The error type, e.g. `urn:ietf:params:jmap:error:limit`.
        kind: String,
        /// The human-readable description the server gave, possibly empty.
        description: String,
    },
    /// HTTP 400-class, other than authentication and limits.
    #[error("the server rejected the request: {0}")]
    Bad(String),
    /// HTTP 401 or 403, or credentials that cannot be sent.
    #[error("sign-in failed: {0}")]
    Auth(String),
    /// HTTP 429 or 503 (also 502 and 504).
    #[error("the server is busy (HTTP {status})")]
    Busy {
        /// The HTTP status code.
        status: u16,
        /// The `Retry-After` delay in seconds, when the server gave one.
        retry_after: Option<u32>,
    },
    /// A body over the size the caller allowed.
    #[error("too large: {0}")]
    TooLarge(String),
}

/// The description when the server gave one, else the error type.
fn refused<'a>(kind: &'a str, description: &'a str) -> &'a str {
    if description.is_empty() { kind } else { description }
}

/// The result of an exchange.
pub type Result<T> = std::result::Result<T, Error>;

/// One HTTP request, as the JMAP client hands it to a transport.
#[derive(Clone, Default)]
pub struct Request {
    /// The method, e.g. `"GET"` or `"POST"`.
    pub method: String,
    /// The full address.
    pub url: String,
    /// The header fields, in the order they were added.
    pub headers: Vec<(String, String)>,
    /// The body, empty for none.
    pub body: Vec<u8>,
    /// The most bytes of response body the caller takes.
    pub max: usize,
}

impl Request {
    /// A `GET` of `url`, taking at most `max` bytes of response body.
    pub fn get(url: &str, max: usize) -> Request {
        Request { method: "GET".to_owned(), url: url.to_owned(), headers: Vec::new(), body: Vec::new(), max }
    }

    /// A `POST` of `body` to `url` with the given `Content-Type`, taking at most `max` bytes of response body.
    pub fn post(url: &str, content_type: &str, body: Vec<u8>, max: usize) -> Request {
        Request { method: "POST".to_owned(), url: url.to_owned(), headers: vec![("Content-Type".to_owned(), content_type.to_owned())], body, max }
    }

    /// The same request with one more header field, appended after the others.
    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

impl fmt::Debug for Request {
    /// Writes the method and the origin of the address only: headers, path, query and body can carry secrets.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let to = match Url::parse(&self.url) {
            Ok(url) => url.origin(),
            Err(_) => "(not a web address)".to_owned(),
        };
        f.debug_struct("Request").field("method", &self.method).field("to", &to).finish()
    }
}

/// One HTTP response, as a transport hands it back. The status is not interpreted.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Response {
    /// The HTTP status code.
    pub status: u16,
    /// The body, at most the request's `max` bytes.
    pub body: Vec<u8>,
    /// The `Retry-After` delay in seconds, capped at 3600; `None` when absent or unreadable.
    pub retry_after: Option<u32>,
}

impl fmt::Debug for Response {
    /// Writes the status and the body length only: the body can carry secrets.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response").field("status", &self.status).field("body_len", &self.body.len()).finish()
    }
}

/// A transport: sends one request and returns the server's answer, whatever its status.
#[allow(async_fn_in_trait)]
pub trait Http {
    /// Send `req` and return the response. Errors only when no usable answer came back (see [`Error`]).
    async fn send(&self, req: Request) -> Result<Response>;
}

/// The longest delay [`retry_after`] returns, in seconds.
const MAX_RETRY_AFTER: u32 = 3600;

/// Seconds to wait from a Retry-After value: delta-seconds (digits only) or an HTTP-date (IMF-fixdate, RFC 9110
/// §5.6.7) compared with `now` (Unix seconds); capped at 3600; None for anything else or a date in the past.
///
/// Surrounding spaces and tabs (optional whitespace around a field value, RFC 9110 §5.5) are ignored.
pub fn retry_after(value: &str, now: i64) -> Option<u32> {
    let value = value.trim_matches([' ', '\t']);
    if value.is_empty() {
        return None;
    }
    if value.bytes().all(|b| b.is_ascii_digit()) {
        let seconds = value.bytes().fold(0u32, |total, digit| total.saturating_mul(10).saturating_add(u32::from(digit - b'0')));
        return Some(seconds.min(MAX_RETRY_AFTER));
    }
    let at = imf_fixdate(value)?;
    let delay = i128::from(at) - i128::from(now);
    if delay < 0 {
        return None;
    }
    let capped = delay.min(i128::from(MAX_RETRY_AFTER));
    u32::try_from(capped).ok()
}

/// The Unix time of an IMF-fixdate such as `Sun, 06 Nov 1994 08:49:37 GMT` (RFC 9110 §5.6.7). Names are
/// case-sensitive; the day name is not checked against the date.
fn imf_fixdate(text: &str) -> Option<i64> {
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    if text.len() != 29 || !text.is_ascii() {
        return None;
    }
    let field = |from: usize, to: usize| text.get(from..to);
    let separators = [(3, ","), (4, " "), (7, " "), (11, " "), (16, " "), (19, ":"), (22, ":"), (25, " ")];
    for (at, expected) in separators {
        if field(at, at + 1)? != expected {
            return None;
        }
    }
    if !DAYS.contains(&field(0, 3)?) || field(26, 29)? != "GMT" {
        return None;
    }
    let day = digits(field(5, 7)?)?;
    let month = MONTHS.iter().position(|name| Some(*name) == field(8, 11))?;
    let month = u32::try_from(month).ok()? + 1;
    let year = digits(field(12, 16)?)?;
    let hour = digits(field(17, 19)?)?;
    let minute = digits(field(20, 22)?)?;
    let second = digits(field(23, 25)?)?;
    // RFC 9110 allows second 60 (a leap second).
    if day == 0 || day > days_in_month(year, month) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    Some(days * 86_400 + i64::from(hour * 3600 + minute * 60 + second))
}

/// The value of a run of ASCII digits (at most four here, so no overflow).
fn digits(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The number of days in `month` (1..=12) of `year` in the proleptic Gregorian calendar.
fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days from 1970-01-01 to the given proleptic Gregorian date (`month` 1..=12, `day` 1..=31).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    // Count from 0000-03-01, so that the leap day ends the year, in 400-year eras of 146 097 days.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    // 719 468 days separate 0000-03-01 from 1970-01-01.
    era * 146_097 + day_of_era - 719_468
}

/// Most bytes of user name plus password, or of a token, that [`Auth::header`] sends.
const MAX_SECRET: usize = 32 * 1024;

/// The credentials the client signs in with.
#[derive(Clone, PartialEq, Eq)]
pub enum Auth {
    /// HTTP Basic (RFC 7617).
    Basic {
        /// The user name; may not hold `:`.
        user: String,
        /// The password.
        password: String,
    },
    /// An OAuth bearer token (RFC 6750).
    Bearer(String),
}

impl Auth {
    /// Basic credentials.
    pub fn basic(user: impl Into<String>, password: impl Into<String>) -> Auth {
        Auth::Basic { user: user.into(), password: password.into() }
    }

    /// A bearer token.
    pub fn bearer(token: impl Into<String>) -> Auth {
        Auth::Bearer(token.into())
    }

    /// The Authorization header value: "Basic " + base64(user ":" password) (RFC 7617; a user holding ':' is refused),
    /// or "Bearer " + token (RFC 6750 b64token characters only). Refused with Error::Auth (a reason that never
    /// repeats the secret) when a value holds a control character, or user + password or the token is over 32 KiB.
    pub fn header(&self) -> Result<String> {
        match self {
            Auth::Basic { user, password } => {
                if user.len().saturating_add(password.len()) > MAX_SECRET {
                    return Err(refuse("the user name and password are too long"));
                }
                if user.chars().any(char::is_control) || password.chars().any(char::is_control) {
                    return Err(refuse("the user name or password holds a control character"));
                }
                if user.contains(':') {
                    return Err(refuse("the user name holds a colon, which Basic sign-in cannot send"));
                }
                Ok(format!("Basic {}", STANDARD.encode(format!("{user}:{password}"))))
            }
            Auth::Bearer(token) => {
                if token.len() > MAX_SECRET {
                    return Err(refuse("the token is too long"));
                }
                if token.chars().any(char::is_control) {
                    return Err(refuse("the token holds a control character"));
                }
                if !is_b64token(token) {
                    return Err(refuse("the token holds characters a bearer token cannot"));
                }
                Ok(format!("Bearer {token}"))
            }
        }
    }
}

impl fmt::Debug for Auth {
    /// Writes the kind (and the user name for Basic), never the password or the token.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::Basic { user, .. } => f.debug_tuple("Basic").field(user).finish(),
            Auth::Bearer(_) => f.write_str("Bearer"),
        }
    }
}

/// An authentication error with a fixed reason.
fn refuse(reason: &str) -> Error {
    Error::Auth(reason.to_owned())
}

/// RFC 6750 §2.1: `b64token = 1*( ALPHA / DIGIT / "-" / "." / "_" / "~" / "+" / "/" ) *"="`.
fn is_b64token(token: &str) -> bool {
    let body = token.trim_end_matches('=');
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'+' | b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        assert_eq!(days_from_civil(1994, 11, 6), 9_075);
    }

    #[test]
    fn month_lengths() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2023, 4), 30);
        assert_eq!(days_in_month(2023, 12), 31);
        assert_eq!(days_in_month(2023, 13), 0);
    }

    #[test]
    fn fixdates() {
        assert_eq!(imf_fixdate("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784_111_777));
        assert_eq!(imf_fixdate("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(imf_fixdate("Wed, 31 Dec 1969 23:59:59 GMT"), Some(-1));
        assert_eq!(imf_fixdate("Tue, 29 Feb 2000 12:00:00 GMT"), Some(951_825_600));
        assert_eq!(imf_fixdate("Thu, 31 Dec 1998 23:59:60 GMT"), Some(915_148_800));
        assert_eq!(imf_fixdate("Mon, 29 Feb 2001 12:00:00 GMT"), None);
        assert_eq!(imf_fixdate("Sun, 06 Nov 1994 24:00:00 GMT"), None);
        assert_eq!(imf_fixdate("Sun, 06 Nov 1994 08:60:00 GMT"), None);
        assert_eq!(imf_fixdate("Sun, 00 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(imf_fixdate("sun, 06 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(imf_fixdate("Sun, 06 nov 1994 08:49:37 GMT"), None);
        assert_eq!(imf_fixdate("Sun, 06 Nov 1994 08:49:37 UTC"), None);
        assert_eq!(imf_fixdate("Sun, 6 Nov 1994 08:49:37 GMT "), None);
        assert_eq!(imf_fixdate("Sun, 06 Nov 1994 08:49:37 GMTX"), None);
        assert_eq!(imf_fixdate("Sun, 06 Nov 1994 08:49:3é GMT"), None);
        assert_eq!(imf_fixdate("Sun; 06 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(imf_fixdate("Sun, 06 Nov 1994 08:49:+7 GMT"), None);
    }
}
