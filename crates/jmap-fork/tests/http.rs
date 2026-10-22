// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Native tests of the `http` module, rule by rule (see SPEC.md and NOTES.md).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use mailcraft_jmap_fork::http::{Auth, Error, Http, Request, Response, Result, retry_after};

const SECRET: &str = "SeCrEt-ToKeN";

// ---- Error texts --------------------------------------------------------------------------------------------------------

#[test]
fn error_texts_are_exact() {
    let cases = [
        (Error::Io("timed out".into()), "network: timed out"),
        (Error::Tls("bad certificate".into()), "secure connection: bad certificate"),
        (Error::Protocol("not JSON".into()), "unexpected server response: not JSON"),
        (Error::Bad("unknown method".into()), "the server rejected the request: unknown method"),
        (Error::Auth("HTTP 401".into()), "sign-in failed: HTTP 401"),
        (Error::Busy { status: 503, retry_after: Some(30) }, "the server is busy (HTTP 503)"),
        (Error::Busy { status: 429, retry_after: None }, "the server is busy (HTTP 429)"),
        (Error::TooLarge("over 10 bytes".into()), "too large: over 10 bytes"),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn refused_shows_the_description_else_the_kind() {
    let with_description = Error::Refused { kind: "urn:ietf:params:jmap:error:limit".into(), description: "Too many calls in one request".into() };
    assert_eq!(with_description.to_string(), "the server refused: Too many calls in one request");
    let without = Error::Refused { kind: "urn:ietf:params:jmap:error:limit".into(), description: String::new() };
    assert_eq!(without.to_string(), "the server refused: urn:ietf:params:jmap:error:limit");
}

#[test]
fn errors_compare_and_clone() {
    let error = Error::Busy { status: 503, retry_after: Some(5) };
    assert_eq!(error.clone(), error);
    assert_ne!(error, Error::Io("x".into()));
    let as_std: &dyn std::error::Error = &error;
    assert!(as_std.source().is_none());
}

// ---- Request ------------------------------------------------------------------------------------------------------------

#[test]
fn get_request() {
    let req = Request::get("https://h/.well-known/jmap", 1024);
    assert_eq!(req.method, "GET");
    assert_eq!(req.url, "https://h/.well-known/jmap");
    assert!(req.headers.is_empty());
    assert!(req.body.is_empty());
    assert_eq!(req.max, 1024);
}

#[test]
fn post_request_adds_content_type() {
    let req = Request::post("https://h/api", "application/json", b"{\"using\":[]}".to_vec(), 7);
    assert_eq!(req.method, "POST");
    assert_eq!(req.url, "https://h/api");
    assert_eq!(req.headers, [("Content-Type".to_owned(), "application/json".to_owned())]);
    assert_eq!(req.body, b"{\"using\":[]}");
    assert_eq!(req.max, 7);
}

#[test]
fn headers_are_appended_in_order() {
    let req = Request::post("https://h/upload", "image/png", vec![1, 2, 3], 10)
        .header("Authorization", "Bearer x")
        .header("Accept", "application/json")
        .header("Accept", "text/plain");
    let names: Vec<(&str, &str)> = req.headers.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    assert_eq!(names, [("Content-Type", "image/png"), ("Authorization", "Bearer x"), ("Accept", "application/json"), ("Accept", "text/plain"),]);
}

#[test]
fn default_request_is_empty() {
    let req = Request::default();
    assert!(req.method.is_empty() && req.url.is_empty() && req.headers.is_empty() && req.body.is_empty());
    assert_eq!(req.max, 0);
}

#[test]
fn request_debug_shows_method_and_origin_only() {
    let req = Request::post(
        &format!("https://jmap.example.com:8443/api/{SECRET}?access_token={SECRET}"),
        "application/json",
        SECRET.as_bytes().to_vec(),
        100,
    )
    .header("Authorization", &format!("Bearer {SECRET}"));
    let shown = format!("{req:?}");
    assert_eq!(shown, "Request { method: \"POST\", to: \"https://jmap.example.com:8443\" }");
    assert!(!shown.contains(SECRET));
    assert!(!shown.contains("Authorization"));
    assert!(!shown.contains("/api"));

    let get = Request::get("http://h/x", 1);
    assert_eq!(format!("{get:?}"), "Request { method: \"GET\", to: \"http://h\" }");
}

#[test]
fn request_debug_with_an_address_that_does_not_parse() {
    for url in [format!("not an address {SECRET}"), format!("ftp://h/{SECRET}"), format!("https://user:{SECRET}@h/"), String::new()] {
        let req = Request::get(&url, 1);
        let shown = format!("{req:?}");
        assert_eq!(shown, "Request { method: \"GET\", to: \"(not a web address)\" }");
        assert!(!shown.contains(SECRET));
    }
}

// ---- Response -----------------------------------------------------------------------------------------------------------

#[test]
fn response_debug_shows_status_and_body_length_only() {
    let resp = Response { status: 200, body: SECRET.as_bytes().to_vec(), retry_after: Some(9) };
    let shown = format!("{resp:?}");
    assert_eq!(shown, "Response { status: 200, body_len: 12 }");
    assert!(!shown.contains(SECRET));
    assert_eq!(format!("{:?}", Response::default()), "Response { status: 0, body_len: 0 }");
}

#[test]
fn responses_compare() {
    let a = Response { status: 503, body: Vec::new(), retry_after: Some(3) };
    assert_eq!(a.clone(), a);
    assert_ne!(a, Response { retry_after: None, ..a.clone() });
}

// ---- Http trait ---------------------------------------------------------------------------------------------------------

/// Polls a future that never waits (enough for a transport that answers at once).
fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the future waited"),
    }
}

/// A transport that echoes what it got: the status is the body length, the body the method.
struct Echo;

impl Http for Echo {
    async fn send(&self, req: Request) -> Result<Response> {
        if req.url.is_empty() {
            return Err(Error::Io("no address".into()));
        }
        Ok(Response { status: u16::try_from(req.body.len()).unwrap_or(u16::MAX), body: req.method.into_bytes(), retry_after: None })
    }
}

/// A generic caller, as the JMAP client is.
async fn call(transport: &impl Http, req: Request) -> Result<Response> {
    transport.send(req).await
}

#[test]
fn a_transport_implements_http() {
    let answer = ready(call(&Echo, Request::post("https://h/", "text/plain", vec![0; 3], 9))).unwrap();
    assert_eq!(answer.status, 3);
    assert_eq!(answer.body, b"POST");
    assert_eq!(ready(call(&Echo, Request::default())), Err(Error::Io("no address".into())));
}

// ---- retry_after --------------------------------------------------------------------------------------------------------

/// `Sun, 06 Nov 1994 08:49:37 GMT`, the example date of RFC 9110 §5.6.7.
const EXAMPLE_DATE: &str = "Sun, 06 Nov 1994 08:49:37 GMT";
const EXAMPLE_TIME: i64 = 784_111_777;

#[test]
fn retry_after_delay_seconds() {
    assert_eq!(retry_after("120", 0), Some(120));
    assert_eq!(retry_after("0", 0), Some(0));
    assert_eq!(retry_after("007", 0), Some(7));
    assert_eq!(retry_after("3599", 0), Some(3599));
    assert_eq!(retry_after("3600", 0), Some(3600));
    assert_eq!(retry_after("3601", 0), Some(3600));
    assert_eq!(retry_after("86400", 0), Some(3600));
    assert_eq!(retry_after("99999999999999999999999999", 0), Some(3600));
    // Delay-seconds do not depend on the clock.
    assert_eq!(retry_after("5", i64::MAX), Some(5));
    assert_eq!(retry_after("5", i64::MIN), Some(5));
}

#[test]
fn retry_after_ignores_surrounding_optional_whitespace() {
    assert_eq!(retry_after(" 120 ", 0), Some(120));
    assert_eq!(retry_after("\t5", 0), Some(5));
    assert_eq!(retry_after(&format!("  {EXAMPLE_DATE}\t"), EXAMPLE_TIME - 10), Some(10));
}

#[test]
fn retry_after_refuses_other_numbers() {
    for bad in ["", " ", "-1", "+5", "1.5", "1e3", "12a", "a12", "1 2", "0x10", "١٢", "５"] {
        assert_eq!(retry_after(bad, 0), None, "{bad:?}");
    }
}

#[test]
fn retry_after_http_date() {
    assert_eq!(retry_after(EXAMPLE_DATE, EXAMPLE_TIME - 120), Some(120));
    assert_eq!(retry_after(EXAMPLE_DATE, EXAMPLE_TIME - 1), Some(1));
    assert_eq!(retry_after(EXAMPLE_DATE, EXAMPLE_TIME), Some(0));
    assert_eq!(retry_after(EXAMPLE_DATE, EXAMPLE_TIME - 3600), Some(3600));
    assert_eq!(retry_after(EXAMPLE_DATE, EXAMPLE_TIME - 3601), Some(3600));
    assert_eq!(retry_after(EXAMPLE_DATE, 0), Some(3600));
    assert_eq!(retry_after(EXAMPLE_DATE, i64::MIN), Some(3600));
    assert_eq!(retry_after("Fri, 31 Dec 9999 23:59:59 GMT", EXAMPLE_TIME), Some(3600));
    assert_eq!(retry_after("Thu, 01 Jan 1970 00:01:00 GMT", 0), Some(60));
    assert_eq!(retry_after("Tue, 29 Feb 2028 00:00:30 GMT", 1_835_395_200), Some(30));
}

#[test]
fn retry_after_date_in_the_past_is_none() {
    assert_eq!(retry_after(EXAMPLE_DATE, EXAMPLE_TIME + 1), None);
    assert_eq!(retry_after(EXAMPLE_DATE, i64::MAX), None);
    assert_eq!(retry_after("Thu, 01 Jan 1970 00:00:00 GMT", 1), None);
}

#[test]
fn retry_after_only_imf_fixdate() {
    for bad in [
        // The obsolete RFC 850 and asctime forms.
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
        // Near misses.
        "Sun, 06 Nov 1994 08:49:37 gmt",
        "Sun, 06 Nov 1994 08:49:37 UTC",
        "Sun, 06 Nov 1994 08:49:37 +0000",
        "sun, 06 Nov 1994 08:49:37 GMT",
        "Sun, 06 NOV 1994 08:49:37 GMT",
        "Sun, 6 Nov 1994 08:49:37 GMT",
        "Sun, 06 Nov 94 08:49:37 GMT",
        "Sun, 06 Nov 1994 8:49:37 GMT",
        "Sun 06 Nov 1994 08:49:37 GMT",
        "Sun,  06 Nov 1994 08:49:37 GMT",
        "Sun, 06 Nov 1994 08:49 GMT",
        "Sun, 06 Nov 1994 08.49.37 GMT",
        "Xyz, 06 Nov 1994 08:49:37 GMT",
        "Sun, 06 Nox 1994 08:49:37 GMT",
        "Sun, 31 Nov 1994 08:49:37 GMT",
        "Sun, 29 Feb 1900 08:49:37 GMT",
        "Sun, 06 Nov 1994 25:49:37 GMT",
        "Sun, 06 Nov 1994 08:61:37 GMT",
        "Sun, 06 Nov 1994 08:49:61 GMT",
        "Sun, 06 Nov 1994 08:49:37 GMT, Mon, 07 Nov 1994 08:49:37 GMT",
        "tomorrow",
    ] {
        assert_eq!(retry_after(bad, 0), None, "{bad:?}");
    }
}

#[test]
fn retry_after_accepts_a_leap_second_and_a_mismatched_day_name() {
    assert_eq!(retry_after("Thu, 31 Dec 1998 23:59:60 GMT", 915_148_800 - 10), Some(10));
    assert_eq!(retry_after("Mon, 06 Nov 1994 08:49:37 GMT", EXAMPLE_TIME - 2), Some(2));
}

// ---- Auth ---------------------------------------------------------------------------------------------------------------

fn auth_refused(auth: &Auth, secrets: &[&str]) -> String {
    match auth.header() {
        Err(Error::Auth(reason)) => {
            // Short values ("a", "user") can appear in any English sentence; the tests pass long ones.
            for secret in secrets.iter().filter(|s| s.len() >= 6) {
                assert!(!reason.contains(secret), "{reason:?} repeats a secret");
            }
            reason
        }
        other => panic!("{auth:?} gave {other:?}"),
    }
}

/// RFC 7617 §2 and §2.1 examples.
#[test]
fn basic_header() {
    assert_eq!(Auth::basic("Aladdin", "open sesame").header().unwrap(), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
    assert_eq!(Auth::basic("test", "123\u{a3}").header().unwrap(), "Basic dGVzdDoxMjPCow==");
    assert_eq!(Auth::basic("user", "").header().unwrap(), "Basic dXNlcjo=");
    assert_eq!(Auth::basic("", "").header().unwrap(), "Basic Og==");
    // A colon in the password is fine: the first colon ends the user.
    assert_eq!(Auth::basic("a", "b:c").header().unwrap(), "Basic YTpiOmM=");
    assert_eq!(Auth::basic(String::from("me@example.com"), String::from("pässwörd")).header().unwrap(), "Basic bWVAZXhhbXBsZS5jb206cMOkc3N3w7ZyZA==");
}

#[test]
fn basic_refuses_a_user_with_a_colon() {
    auth_refused(&Auth::basic("us:er", SECRET), &["us:er", SECRET]);
    auth_refused(&Auth::basic(":", SECRET), &[SECRET]);
}

#[test]
fn basic_refuses_control_characters() {
    let user = "alice-the-user";
    for (user, password) in [
        (format!("{user}\u{0}"), SECRET.to_owned()),
        (format!("{user}\n"), SECRET.to_owned()),
        (user.to_owned(), format!("{SECRET}\r\n")),
        (user.to_owned(), format!("{SECRET}\u{7f}")),
        (user.to_owned(), format!("{SECRET}\u{85}")),
        (user.to_owned(), format!("\t{SECRET}")),
    ] {
        let reason = auth_refused(&Auth::basic(user.as_str(), password.as_str()), &[user.as_str(), SECRET]);
        assert!(reason.contains("control character"), "{reason}");
    }
}

#[test]
fn basic_size_limit() {
    let user = "u".repeat(16 * 1024);
    let password = "p".repeat(16 * 1024);
    assert!(Auth::basic(user.as_str(), password.as_str()).header().is_ok());
    let longer = format!("{password}p");
    auth_refused(&Auth::basic(user.as_str(), longer.as_str()), &[&longer]);
    auth_refused(&Auth::basic("", "p".repeat(32 * 1024 + 1)), &[]);
}

#[test]
fn bearer_header() {
    // RFC 6750 §2.1 example token.
    assert_eq!(Auth::bearer("mF_9.B5f-4.1JqM").header().unwrap(), "Bearer mF_9.B5f-4.1JqM");
    assert_eq!(Auth::bearer("aZ09-._~+/").header().unwrap(), "Bearer aZ09-._~+/");
    assert_eq!(Auth::bearer("abc==").header().unwrap(), "Bearer abc==");
    assert_eq!(Auth::bearer(String::from("x")).header().unwrap(), "Bearer x");
}

#[test]
fn bearer_refuses_what_is_not_a_b64token() {
    for token in ["", "=", "==", "a=b", "a b", "a,b", "a\"b", "é", "a;b", "a:b", "tok\\en", "a==b"] {
        auth_refused(&Auth::bearer(token), &[]);
        let longer = format!("{SECRET}{token}");
        if !token.is_empty() && token != "=" && token != "==" {
            auth_refused(&Auth::bearer(longer.as_str()), &[SECRET, &longer]);
        }
    }
}

#[test]
fn bearer_refuses_control_characters() {
    for token in [format!("{SECRET}\n"), format!("\r{SECRET}"), format!("{SECRET}\u{0}"), format!("{SECRET}\u{7f}")] {
        let reason = auth_refused(&Auth::bearer(token.as_str()), &[SECRET]);
        assert!(reason.contains("control character"), "{reason}");
    }
}

#[test]
fn bearer_size_limit() {
    let at_limit = "t".repeat(32 * 1024);
    assert_eq!(Auth::bearer(at_limit.as_str()).header().unwrap().len(), "Bearer ".len() + 32 * 1024);
    let over = format!("{at_limit}t");
    auth_refused(&Auth::bearer(over.as_str()), &[&over]);
}

#[test]
fn auth_debug_never_shows_the_secret() {
    let basic = Auth::basic("alice@example.com", SECRET);
    let shown = format!("{basic:?}");
    assert_eq!(shown, "Basic(\"alice@example.com\")");
    assert!(!shown.contains(SECRET));
    let bearer = Auth::bearer(SECRET);
    let shown = format!("{bearer:?}");
    assert_eq!(shown, "Bearer");
    assert!(!format!("{bearer:#?}").contains(SECRET));
}

#[test]
fn auth_compares_and_clones() {
    let basic = Auth::basic("a", "b");
    assert_eq!(basic.clone(), basic);
    assert_eq!(basic, Auth::Basic { user: "a".into(), password: "b".into() });
    assert_ne!(basic, Auth::basic("a", "c"));
    assert_eq!(Auth::bearer("t"), Auth::Bearer("t".into()));
}
