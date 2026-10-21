// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The browser-free half of the `fetch` transport: what the browser request is built from, how a failure
//! becomes an [`Error`], and the body-size accounting. Kept apart so it can be tested natively.

use mailcraft_jmap::http::{Error, Request, Result};

/// Most characters of a browser message kept in an [`Error::Io`].
const MAX_REASON_CHARS: usize = 300;

/// The reason given when the exchange was aborted because it took too long.
const TIMED_OUT: &str = "timed out";

/// The reason given when a rejected promise carries no message.
const NO_MESSAGE: &str = "the request failed";

/// What the browser request is built from.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan<'a> {
    /// The method, as the caller gave it.
    pub(crate) method: &'a str,
    /// The full address.
    pub(crate) url: &'a str,
    /// The header fields, in order.
    pub(crate) headers: &'a [(String, String)],
    /// The body to send, or `None` for no body at all.
    pub(crate) body: Option<&'a [u8]>,
}

/// The plan for `req`: its method, address and headers as they are, and its body unless it is empty on a
/// `GET` or `HEAD` (the Fetch Standard refuses any body, even an empty one, on those two methods).
pub(crate) fn plan(req: &Request) -> Plan<'_> {
    let bodiless = req.method.eq_ignore_ascii_case("GET") || req.method.eq_ignore_ascii_case("HEAD");
    let body = if req.body.is_empty() && bodiless { None } else { Some(req.body.as_slice()) };
    Plan { method: &req.method, url: &req.url, headers: &req.headers, body }
}

/// The error for a rejected `fetch` or body read: "timed out" when our own time limit aborted it, else the
/// browser's message cut to 300 characters.
pub(crate) fn failure(timed_out: bool, message: Option<&str>) -> Error {
    if timed_out {
        return Error::Io(TIMED_OUT.to_owned());
    }
    let message = message.map(str::trim).filter(|message| !message.is_empty()).unwrap_or(NO_MESSAGE);
    Error::Io(message.chars().take(MAX_REASON_CHARS).collect())
}

/// The body length after one more chunk, or [`Error::TooLarge`] once it would pass `max`.
pub(crate) fn add_chunk(total: usize, chunk: usize, max: usize) -> Result<usize> {
    match total.checked_add(chunk) {
        Some(sum) if sum <= max => Ok(sum),
        _ => Err(Error::TooLarge(format!("the response body is over {max} bytes"))),
    }
}

/// Unix seconds from a JavaScript time in milliseconds (`Date.now()`).
pub(crate) fn unix_seconds(ms: f64) -> i64 {
    // `as` saturates on overflow and maps NaN to 0, which is what we want for a clock reading.
    (ms / 1000.0).floor() as i64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn plan_keeps_method_address_and_header_order() {
        let req = Request::post("https://h/api", "application/json", b"{}".to_vec(), 10)
            .header("Authorization", "Bearer x")
            .header("Accept", "application/json");
        let plan = plan(&req);
        assert_eq!(plan.method, "POST");
        assert_eq!(plan.url, "https://h/api");
        let names: Vec<&str> = plan.headers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Content-Type", "Authorization", "Accept"]);
        assert_eq!(plan.body, Some(&b"{}"[..]));
    }

    #[test]
    fn plan_sends_no_body_for_an_empty_get_or_head() {
        assert_eq!(plan(&Request::get("https://h/", 1)).body, None);
        let mut head = Request::get("https://h/", 1);
        head.method = "head".to_owned();
        assert_eq!(plan(&head).body, None);
        let mut lower_get = Request::get("https://h/", 1);
        lower_get.method = "get".to_owned();
        assert_eq!(plan(&lower_get).body, None);
    }

    #[test]
    fn plan_keeps_an_empty_body_on_other_methods_and_any_non_empty_body() {
        let empty_post = Request::post("https://h/", "text/plain", Vec::new(), 1);
        assert_eq!(plan(&empty_post).body, Some(&b""[..]));
        let mut delete = Request::get("https://h/", 1);
        delete.method = "DELETE".to_owned();
        assert_eq!(plan(&delete).body, Some(&b""[..]));
        let mut get_with_body = Request::get("https://h/", 1);
        get_with_body.body = b"x".to_vec();
        assert_eq!(plan(&get_with_body).body, Some(&b"x"[..]));
    }

    #[test]
    fn failure_says_timed_out_when_aborted_by_the_time_limit() {
        assert_eq!(failure(true, Some("The user aborted a request.")), Error::Io("timed out".to_owned()));
        assert_eq!(failure(true, None), Error::Io("timed out".to_owned()));
    }

    #[test]
    fn failure_keeps_the_browser_message_cut_to_300_characters() {
        assert_eq!(failure(false, Some("Failed to fetch")), Error::Io("Failed to fetch".to_owned()));
        assert_eq!(failure(false, Some("  Load failed \n")), Error::Io("Load failed".to_owned()));
        let long = "é".repeat(400);
        let Error::Io(reason) = failure(false, Some(&long)) else {
            panic!("not an Io error");
        };
        assert_eq!(reason.chars().count(), 300);
        assert!(reason.chars().all(|c| c == 'é'));
    }

    #[test]
    fn failure_without_a_message_has_a_fixed_reason() {
        assert_eq!(failure(false, None), Error::Io("the request failed".to_owned()));
        assert_eq!(failure(false, Some("   ")), Error::Io("the request failed".to_owned()));
    }

    #[test]
    fn chunks_are_counted_up_to_the_limit() {
        assert_eq!(add_chunk(0, 0, 0), Ok(0));
        assert_eq!(add_chunk(0, 10, 10), Ok(10));
        assert_eq!(add_chunk(4, 6, 10), Ok(10));
        assert!(matches!(add_chunk(4, 7, 10), Err(Error::TooLarge(_))));
        assert!(matches!(add_chunk(10, 1, 10), Err(Error::TooLarge(_))));
        assert!(matches!(add_chunk(usize::MAX, 1, usize::MAX), Err(Error::TooLarge(_))));
        assert_eq!(add_chunk(0, 1, 0).unwrap_err().to_string(), "too large: the response body is over 0 bytes");
    }

    #[test]
    fn clock_readings_become_unix_seconds() {
        assert_eq!(unix_seconds(0.0), 0);
        assert_eq!(unix_seconds(1_999.9), 1);
        assert_eq!(unix_seconds(784_111_777_000.0), 784_111_777);
        assert_eq!(unix_seconds(-1.0), -1);
        assert_eq!(unix_seconds(f64::NAN), 0);
        assert_eq!(unix_seconds(f64::INFINITY), i64::MAX);
    }
}
