// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The loopback server's answers, the demo it serves, and the server itself, which only runs when asked for by hand.

use std::sync::atomic::AtomicBool;

use mailcraft_model::remote::CalendarAsk;
use mailcraft_model::sync::{JmapMailRequest, JmapPimRequest, JmapScope};
use mailcraft_time::{Date, DateTime};
use serde_json::{Value, json};

use super::super::{DEMO_PERSON, Pim, SESSION, crowd, demo};
use super::*;
use crate::{Auth, Client, sync_mail, sync_pim};

/// Tuesday 2026-10-20, the week the clocks go back in Zurich.
fn today() -> Date {
    Date::new(2026, 10, 20).unwrap()
}

/// A request as a browser sends it, and what came back: the status, the headers, the body.
fn ask(stand_in: &StandIn, trouble: &mut Trouble, method: &str, target: &str, token: Option<&str>, body: &str) -> Option<(u16, String, Vec<u8>)> {
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let raw = format!(
        "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:8790\r\nOrigin: http://127.0.0.1:8771\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let answer = super::answer(stand_in, trouble, raw.as_bytes())?;
    let end = answer.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(answer[..end].to_vec()).unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    Some((status, head, answer[end + 4..].to_vec()))
}

fn json_of(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap()
}

#[test]
fn the_loopback_server_answers_a_session_and_an_api_call_with_cors_headers() {
    let stand_in = demo(today());
    let mut trouble = Trouble::default();
    let (status, head, body) = ask(&stand_in, &mut trouble, "GET", "/jmap/session", Some(TOKEN), "").unwrap();
    assert_eq!(status, 200);
    for header in [
        "Access-Control-Allow-Origin: *",
        "Access-Control-Allow-Headers: Authorization, Content-Type, Accept",
        "Access-Control-Expose-Headers: Retry-After",
        "Connection: close",
    ] {
        assert!(head.contains(header), "{header} in {head}");
    }
    let session = json_of(&body);
    assert!(session["capabilities"][crate::CALENDARS].is_object() && session["apiUrl"] == "/jmap/api", "{session}");
    let call =
        json!({"using": [crate::CORE, crate::CALENDARS], "methodCalls": [["Calendar/get", {"accountId": super::super::ACCOUNT}, "c0"]]}).to_string();
    let (status, head, body) = ask(&stand_in, &mut trouble, "POST", "/jmap/api", Some(TOKEN), &call).unwrap();
    assert_eq!(status, 200);
    assert!(head.contains("Access-Control-Allow-Origin: *"));
    assert_eq!(json_of(&body)["methodResponses"][0][1]["list"].as_array().map(Vec::len), Some(3));
    // Without the credential, refused as the stand-in refuses it, with the same headers.
    let (status, head, _) = ask(&stand_in, &mut trouble, "GET", "/jmap/session", None, "").unwrap();
    assert!(status == 401 && head.contains("Access-Control-Allow-Origin: *"));
}

#[test]
fn an_options_request_is_answered() {
    let stand_in = demo(today());
    let mut trouble = Trouble::default();
    let (status, head, body) = ask(&stand_in, &mut trouble, "OPTIONS", "/jmap/api", None, "").unwrap();
    assert_eq!((status, body.len()), (204, 0));
    assert!(head.contains("Access-Control-Allow-Methods: GET, POST, OPTIONS"), "{head}");
    assert!(stand_in.seen().is_empty(), "a preflight is not the stand-in's");
}

#[test]
fn a_slow_or_down_server_does_what_the_query_string_says() {
    let stand_in = demo(today());
    let mut trouble = Trouble::default();
    // Slow: the answer waits.
    ask(&stand_in, &mut trouble, "GET", "/__trouble?slow=30", None, "").unwrap();
    let began = std::time::Instant::now();
    assert_eq!(ask(&stand_in, &mut trouble, "GET", "/jmap/session", Some(TOKEN), "").unwrap().0, 200);
    assert!(began.elapsed() >= Duration::from_millis(30));
    // Down: closed without an answer, until it is up again.
    ask(&stand_in, &mut trouble, "GET", "/__trouble?down", None, "").unwrap();
    assert!(ask(&stand_in, &mut trouble, "GET", "/jmap/session", Some(TOKEN), "").is_none());
    ask(&stand_in, &mut trouble, "GET", "/__trouble?up", None, "").unwrap();
    assert_eq!(trouble.slow_ms, 0);
    assert_eq!(ask(&stand_in, &mut trouble, "GET", "/jmap/session", Some(TOKEN), "").unwrap().0, 200);
    // Expire: the token is no longer good, and the new one is given to whoever asks, as a page's renewal would.
    ask(&stand_in, &mut trouble, "GET", "/__trouble?expire", None, "").unwrap();
    assert_eq!(ask(&stand_in, &mut trouble, "GET", "/jmap/session", Some(TOKEN), "").unwrap().0, 401);
    let fresh = String::from_utf8(ask(&stand_in, &mut trouble, "GET", "/__token", None, "").unwrap().2).unwrap();
    assert_eq!(fresh, format!("{TOKEN}-1"));
    assert_eq!(ask(&stand_in, &mut trouble, "GET", "/jmap/session", Some(&fresh), "").unwrap().0, 200);
    // What it was asked to call, for whoever looks.
    let log = json_of(&ask(&stand_in, &mut trouble, "GET", "/__log", None, "").unwrap().2);
    assert!(log.is_array());
    // A request that is no request is closed.
    assert!(super::answer(&stand_in, &mut trouble, b"garbage").is_none());
}

#[test]
fn the_demo_data_reads_back_as_the_server_would_serve_it() {
    let stand_in = demo(today());
    let client = block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let now = DateTime::at_midnight(today()).to_timestamp(0) + 9 * 3_600;
    // The window's first job of the account: the list, and the wide stretch of the main calendar and the others.
    let asks = ["", &Pim::calendar_id("team"), &Pim::calendar_id("grace")]
        .map(|calendar| CalendarAsk { calendar: calendar.to_string(), series: Vec::new() });
    let request = JmapPimRequest {
        account: 1,
        email: DEMO_PERSON.into(),
        session: SESSION.into(),
        me: vec![DEMO_PERSON.into()],
        zone: "Europe/Zurich".into(),
        now,
        from: now - 31 * 86_400,
        to: now + 366 * 86_400,
        lists: true,
        asks: asks.to_vec(),
        ..Default::default()
    };
    let result = block_on(sync_pim(&client, &request, &AtomicBool::new(false)));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let list = result.calendars.as_ref().unwrap();
    assert_eq!(list.calendars.iter().filter(|c| c.main).map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Ada Marlow"]);
    assert_eq!(list.calendars.len(), 3);
    let main = result.events.as_ref().unwrap();
    assert!(main.complete && main.events.len() == 9, "{} events", main.events.len());
    // The meeting she organizes, with its three guests, and the one she is invited to, unanswered.
    let budget = main.events.iter().find(|e| e.title == "Budget review").unwrap();
    assert_eq!((budget.is_organizer, budget.attendees.len(), budget.reminder_minutes, budget.location.as_str()), (Some(true), 3, Some(15), "Room 4"));
    let planning = main.events.iter().find(|e| e.title == "Quarterly planning").unwrap();
    assert_eq!((planning.is_organizer, planning.response), (Some(false), mailcraft_model::Response::None));
    assert!(main.events.iter().any(|e| e.title == "Dentist" && e.start_zone.is_empty() && e.private));
    assert!(main.events.iter().any(|e| e.title == "Book fair" && e.all_day));
    // The weekly meeting: seven occurrences, at nine in Zurich either side of the night the clocks go back.
    let team = result.more_events.iter().find(|d| d.calendar == Pim::calendar_id("team")).unwrap();
    let mut starts: Vec<i64> = team.events.iter().map(|e| e.start).collect();
    starts.sort();
    assert_eq!(starts.len(), 7);
    let zurich = mailcraft_time::Zone::named("Europe/Zurich").unwrap();
    assert!(starts.iter().all(|start| (zurich.wall(*start).hour, zurich.wall(*start).minute) == (9, 0)));
    assert_eq!(starts[1] - starts[0], 7 * 86_400, "summer time, a week");
    assert_eq!(starts[2] - starts[1], 7 * 86_400 + 3_600, "the week the clocks go back is an hour longer");
    let grace = result.more_events.iter().find(|d| d.calendar == Pim::calendar_id("grace")).unwrap();
    assert_eq!(grace.events.len(), 2);
    // A crowd for a look at what many events cost: all of them read, in pages.
    crowd(&stand_in, today(), 600);
    let crowded = block_on(sync_pim(&client, &request, &AtomicBool::new(false)));
    assert_eq!(crowded.events.as_ref().map(|d| (d.events.len(), d.complete)), Some((609, true)));
    // And the mail is there for a mail run.
    let mail = JmapMailRequest {
        account: 1,
        email: DEMO_PERSON.into(),
        session: SESSION.into(),
        window: 50,
        list_folders: true,
        scope: JmapScope::All,
        ..Default::default()
    };
    let mut pieces = Vec::new();
    block_on(sync_mail(&client, &mail, &AtomicBool::new(false), &mut |piece| pieces.push(piece)));
    assert_eq!(pieces.iter().flat_map(|p| &p.pages).map(|page| page.messages.len()).sum::<usize>(), 4);
}

/// The server itself, for a browser to talk to: run by hand (see the module).
#[test]
#[ignore = "a server on this computer's loopback for a browser to talk to, until it is stopped: run by hand (loopback.rs)"]
fn serve_the_stand_in_on_loopback() {
    let port = std::env::var("MAILCRAFT_STANDIN_PORT").ok().and_then(|p| p.parse::<u16>().ok()).unwrap_or(8790);
    let today = std::env::var("MAILCRAFT_STANDIN_TODAY")
        .ok()
        .and_then(|day| Date::parse_iso(&day))
        .unwrap_or_else(|| DateTime::from_timestamp(mailcraft_time::now(), 0).date);
    let stand_in = demo(today);
    let more = std::env::var("MAILCRAFT_STANDIN_EVENTS").ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0).min(20_000);
    crowd(&stand_in, today, more);
    println!(
        "the stand-in, with the demo around {} and {more} more events, on http://127.0.0.1:{port}/jmap/session (token {TOKEN}); stop it with Ctrl-C",
        today.iso()
    );
    serve(&stand_in, port).unwrap();
}
