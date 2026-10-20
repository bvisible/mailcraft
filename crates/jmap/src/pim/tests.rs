// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What the calendar readers share: the session's offers, the span a server expands, durations and times, and the
//! paging of a `/query`, over scripted answers.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;

use super::*;
use crate::{Auth, CALENDARS, CONTACTS, CORE, Error, MAIL, Request, Response, block_on};

const SESSION: &str = "https://jmap.example.org/jmap/session";

/// A session that offers what `extra` adds to a mail one.
fn session(extra: impl FnOnce(&mut Value)) -> Value {
    let mut doc = json!({
        "capabilities": {CORE: {"maxCallsInRequest": 16, "maxObjectsInGet": 500}, MAIL: {}},
        "accounts": {"A1": {"name": "ada@example.org", "accountCapabilities": {MAIL: {}}}},
        "primaryAccounts": {MAIL: "A1"},
        "apiUrl": "https://jmap.example.org/api/method/jmap.api",
        "downloadUrl": "/jmap/download/{accountId}/{blobId}/{name}?type={type}",
        "uploadUrl": "/jmap/upload/{accountId}/",
        "state": "1",
    });
    extra(&mut doc);
    doc
}

/// A session that offers calendars and address books in account `A1`, with what `capability` says of the calendars.
fn offering(capability: Value) -> Value {
    session(|doc| {
        doc["capabilities"][CALENDARS] = json!({});
        doc["capabilities"][CONTACTS] = json!({});
        doc["primaryAccounts"][CALENDARS] = json!("A1");
        doc["primaryAccounts"][CONTACTS] = json!("A1");
        doc["accounts"]["A1"]["accountCapabilities"][CALENDARS] = capability;
        doc["accounts"]["A1"]["accountCapabilities"][CONTACTS] = json!({"maxAddressBooksPerCard": 1, "mayCreateAddressBook": false});
    })
}

/// An `Http` that serves `doc` as the session and answers every call with `answer` (the call's name and arguments
/// in, its result out), keeping the bodies it was sent.
#[derive(Clone)]
struct Scripted {
    doc: Value,
    answer: Rc<dyn Fn(&str, &Value) -> Value>,
    sent: Rc<RefCell<Vec<Value>>>,
    /// Pads every answer to this many bytes (an answer too large to be taken).
    pad: usize,
}

impl Scripted {
    fn new(doc: Value, answer: impl Fn(&str, &Value) -> Value + 'static) -> Scripted {
        Scripted { doc, answer: Rc::new(answer), sent: Rc::default(), pad: 0 }
    }

    fn sent(&self) -> Vec<Value> {
        self.sent.borrow().clone()
    }
}

impl Http for Scripted {
    async fn send(&self, req: Request) -> Result<Response> {
        let body = if req.method == "GET" {
            serde_json::to_vec(&self.doc).unwrap()
        } else {
            let asked: Value = serde_json::from_slice(&req.body).unwrap();
            self.sent.borrow_mut().push(asked.clone());
            // An answer that is `{"error": type}` is that method error.
            let respond = |call: &Value| match (self.answer)(call[0].as_str().unwrap(), &call[1]) {
                Value::Object(error) if error.contains_key("error") => json!(["error", {"type": error["error"]}, call[2]]),
                result => json!([call[0], result, call[2]]),
            };
            let responses: Vec<Value> = asked["methodCalls"].as_array().unwrap().iter().map(respond).collect();
            let mut answer = json!({"methodResponses": responses, "sessionState": "1"});
            if self.pad > 0 {
                answer["padding"] = json!("x".repeat(self.pad));
            }
            serde_json::to_vec(&answer).unwrap()
        };
        Ok(Response { status: 200, body, retry_after: None })
    }
}

fn connect(http: &Scripted) -> Result<Client<Scripted>> {
    block_on(Client::connect(http.clone(), SESSION, &Auth::bearer("tk")))
}

#[test]
fn a_session_that_offers_contacts_and_calendars_names_their_accounts() {
    let http = Scripted::new(offering(json!({"maxExpandedQueryDuration": "P500D"})), |_, _| json!({}));
    let client = connect(&http).unwrap();
    assert!(client.offers_calendars() && client.offers_contacts());
    assert_eq!((client.calendars_account(), client.contacts_account()), (Some("A1"), Some("A1")));
    // In accounts of their own, when the session says so.
    let apart = offering(json!({})).tap(|doc| {
        doc["primaryAccounts"][CALENDARS] = json!("A-cal");
        doc["primaryAccounts"][CONTACTS] = json!("A-book");
    });
    let client = connect(&Scripted::new(apart, |_, _| json!({}))).unwrap();
    assert_eq!((client.calendars_account(), client.contacts_account(), client.account_id()), (Some("A-cal"), Some("A-book"), "A1"));
    // What is shown of a client says that, and nothing of the accounts.
    let shown = format!("{client:?}");
    assert!(shown.contains("calendars: true") && !shown.contains("A-cal"), "{shown}");
}

#[test]
fn a_session_without_them_offers_neither_and_mail_still_works() {
    let http = Scripted::new(session(|_| {}), |_, _| json!({"list": []}));
    let client = connect(&http).unwrap();
    assert!(!client.offers_calendars() && !client.offers_contacts());
    assert_eq!((client.calendars_account(), client.contacts_account()), (None, None));
    assert_eq!((client.calendars_span(), client.calendars_writable(), client.contacts_writable()), (500 * 86_400, None, None));
    // A primary account named for a capability the session doesn't have is not an offer either.
    let named = session(|doc| doc["primaryAccounts"][CALENDARS] = json!("A1"));
    assert!(!connect(&Scripted::new(named, |_, _| json!({}))).unwrap().offers_calendars());
    // And mail goes on as before.
    let answers = block_on(client.mail(vec![("Mailbox/get", json!({"accountId": "A1"}))])).unwrap();
    assert!(answers.get(0).is_ok());
}

#[test]
fn an_account_id_that_is_not_an_id_is_not_offered() {
    for id in [json!(""), json!("x".repeat(513)), json!("A\u{0}1"), json!("A 1"), json!(7), json!(null), json!({"id": "A1"})] {
        let doc = offering(json!({})).tap(|doc| doc["primaryAccounts"][CALENDARS] = id.clone());
        let client = connect(&Scripted::new(doc, |_, _| json!({}))).unwrap();
        assert!(!client.offers_calendars(), "{id}");
        assert!(client.offers_contacts(), "the other is offered as it is: {id}");
    }
}

#[test]
fn the_span_a_server_expands_is_read_and_clamped() {
    let span = |said: Value| {
        let client = connect(&Scripted::new(offering(json!({"maxExpandedQueryDuration": said})), |_, _| json!({}))).unwrap();
        client.calendars_span()
    };
    assert_eq!(span(json!("P500D")), 500 * 86_400);
    assert_eq!(span(json!("P52W")), 364 * 86_400);
    assert_eq!(span(json!("PT1H")), 86_400, "held to a day");
    assert_eq!(span(json!("P3Y")), 2 * 365 * 86_400, "held to two years");
    for junk in [json!("junk"), json!(""), json!("P"), json!("-P5D"), json!(500), json!(null), json!("P99999999999D")] {
        assert_eq!(span(junk.clone()), 500 * 86_400, "{junk}");
    }
    // Absent, and with no capability for the account at all.
    assert_eq!(span(json!(null)), 500 * 86_400);
    let bare = offering(json!({})).tap(|doc| doc["accounts"]["A1"]["accountCapabilities"] = json!({}));
    assert_eq!(connect(&Scripted::new(bare, |_, _| json!({}))).unwrap().calendars_span(), 500 * 86_400);
}

#[test]
fn what_the_server_writes_is_read_from_the_session() {
    let doc = offering(json!({"writableProperties": ["title", "start", "", "two words", 7, "x".repeat(65), "duration"]}))
        .tap(|doc| doc["accounts"]["A1"]["accountCapabilities"][CONTACTS]["writableProperties"] = json!(["name", "emails"]));
    let client = connect(&Scripted::new(doc, |_, _| json!({}))).unwrap();
    assert_eq!(client.calendars_writable(), Some(&["title".to_string(), "start".to_string(), "duration".to_string()][..]));
    assert_eq!(client.contacts_writable(), Some(&["name".to_string(), "emails".to_string()][..]));
    // Not said, or not a list: nothing is known of it.
    let unsaid = connect(&Scripted::new(offering(json!({"writableProperties": "title"})), |_, _| json!({}))).unwrap();
    assert_eq!((unsaid.calendars_writable(), unsaid.contacts_writable()), (None, None));
    // A host may know a server by where its API is.
    assert!(client.api_path_ends_with("/api/method/jmap.api") && !client.api_path_ends_with("/other.api"));
}

#[test]
fn durations_read_and_write_in_every_form() {
    let span = |days, seconds| Some(Span { days, seconds });
    assert_eq!(duration("PT1H30M"), span(0, 5_400));
    assert_eq!(duration("P1D"), span(1, 0));
    assert_eq!(duration("P1DT2H"), span(1, 7_200));
    assert_eq!(duration("PT0S"), span(0, 0));
    assert_eq!(duration("P1W"), span(7, 0));
    assert_eq!(duration("P1W2D"), span(9, 0));
    assert_eq!(duration("PT1M30.5S"), span(0, 90));
    assert_eq!(duration(" P2D "), span(2, 0));
    for junk in ["", "P", "PT", "1D", "P1", "P1H", "PT1D", "P1D1W", "PT1M1H", "P1Y", "-P1D", "P1.5D", "PT.5S", "P1DT", "PT1H1H", "pt1h", "P١D"] {
        assert_eq!(duration(junk), None, "{junk}");
    }
    // Too long to be a number, and too large to be counted in seconds.
    assert_eq!(duration("P1234567890D"), None);
    assert_eq!(duration("P999999999W"), span(6_999_999_993, 0));
    assert_eq!(span_seconds("P999999999W"), Some(6_999_999_993 * 86_400));
    assert_eq!(span_seconds("P1Y2M"), Some((365 + 62) * 86_400));
    assert_eq!(span_seconds("P1M1Y"), None, "years before months");
    // Signed, for an alert's offset.
    assert_eq!(signed_seconds("-PT15M"), Some(-900));
    assert_eq!(signed_seconds("+P1D"), Some(86_400));
    assert_eq!(signed_seconds("PT0S"), Some(0));
    assert_eq!(signed_seconds("--PT1M"), None);
    // Written back as JSCalendar has them, and read back as they were.
    for (days, seconds, text) in
        [(0, 5_400, "PT1H30M"), (1, 0, "P1D"), (1, 7_200, "P1DT2H"), (0, 0, "PT0S"), (0, 59, "PT59S"), (3, 3_601, "P3DT1H1S")]
    {
        let written = duration_text(Span { days, seconds });
        assert_eq!(written, text);
        assert_eq!(duration(&written), Some(Span { days, seconds }));
    }
}

#[test]
fn times_read_in_utc_and_on_the_clock() {
    assert_eq!(utc_date("2026-10-20T09:00:00Z"), Some(1_792_486_800));
    assert_eq!(utc_date("2026-10-20T09:00:00.250Z"), Some(1_792_486_800));
    for junk in ["2026-10-20T09:00:00", "2026-10-20T09:00:00+02:00", "2026-10-20Z", "2026-13-20T09:00:00Z", "", "Z", "soon"] {
        assert_eq!(utc_date(junk), None, "{junk}");
    }
    assert_eq!(utc_text(1_792_486_800), "2026-10-20T09:00:00Z");
    let clock = local_date_time("2026-10-25T02:30:00").unwrap();
    assert_eq!(local_text(clock), "2026-10-25T02:30:00");
    for junk in ["2026-10-25T02:30:00Z", "2026-10-25T02:30:00+01:00", "2026-10-25", "02:30"] {
        assert_eq!(local_date_time(junk), None, "{junk}");
    }
    assert_eq!(mailto("mailto:Ada%40example.org"), "Ada@example.org");
    assert_eq!(mailto(" MAILTO:bob@example.org "), "bob@example.org");
    assert_eq!(mailto("carol@example.org"), "carol@example.org");
    assert!(mine(&["Ada@Example.org".into()], " ada@example.org") && !mine(&["ada@example.org".into()], "") && !mine(&[], "ada@example.org"));
}

/// A server that has `count` ids (`e0`, `e1`…) for a query, pages them by `position` and `limit` (at most `most` a
/// page, said back when it set the limit lower), says the total when asked, and (`ignores_position`) may answer
/// every page from the start.
fn listing(count: usize, most: usize, ignores_position: bool, says_total: bool) -> impl Fn(&str, &Value) -> Value {
    move |_, args| {
        let position = if ignores_position { 0 } else { args["position"].as_u64().unwrap_or(0) as usize };
        let asked = args["limit"].as_u64().map(|n| n as usize);
        let limit = asked.unwrap_or(most).min(most);
        let ids: Vec<String> = (position..count).take(limit).map(|n| format!("e{n}")).collect();
        let mut answer = json!({"accountId": "A1", "queryState": "q", "canCalculateChanges": false, "position": position, "ids": ids});
        if says_total && args["calculateTotal"] == json!(true) {
            answer["total"] = json!(count);
        }
        if asked.is_none_or(|asked| asked > most) {
            answer["limit"] = json!(most);
        }
        answer
    }
}

fn query(http: &Scripted, first_limit: usize, cap: usize) -> Result<Queried> {
    let client = connect(http)?;
    let args = json!({"accountId": "A1", "filter": {"inAddressBook": "b1"}}).as_object().cloned().unwrap();
    block_on(query_all(&client, &[CORE, CONTACTS], "ContactCard/query", args, first_limit, cap))
}

/// The positions and limits each page was asked with.
fn pages(http: &Scripted) -> Vec<(u64, u64)> {
    http.sent()
        .iter()
        .map(|body| (body["methodCalls"][0][1]["position"].as_u64().unwrap(), body["methodCalls"][0][1]["limit"].as_u64().unwrap()))
        .collect()
}

#[test]
fn query_all_pages_by_position_until_a_short_page() {
    // Without a total: the third page is short, and that is all of it.
    let http = Scripted::new(offering(json!({})), listing(450, 5_000, false, false));
    let got = query(&http, 200, 5_000).unwrap();
    assert_eq!((got.ids.len(), got.total, got.whole), (450, None, true));
    assert_eq!(got.ids.first().map(String::as_str), Some("e0"));
    assert_eq!(got.ids.last().map(String::as_str), Some("e449"));
    assert_eq!(pages(&http), [(0, 200), (200, 200), (400, 200)]);
    // Every request names the core and the capability of the method, and the first asks for the total.
    for body in http.sent() {
        assert_eq!(body["using"], json!([CORE, CONTACTS]));
    }
    assert_eq!(http.sent()[0]["methodCalls"][0][1]["calculateTotal"], json!(true));
    assert_eq!(http.sent()[1]["methodCalls"][0][1]["calculateTotal"], json!(false));
    // An empty answer is whole at once.
    let none = Scripted::new(offering(json!({})), listing(0, 5_000, false, true));
    assert_eq!(query(&none, 200, 5_000).unwrap(), Queried { ids: Vec::new(), total: Some(0), whole: true });
}

#[test]
fn query_all_stops_at_the_total_and_at_the_cap() {
    // A total that a full page reaches: no empty page is asked for after it.
    let http = Scripted::new(offering(json!({})), listing(400, 5_000, false, true));
    let got = query(&http, 200, 5_000).unwrap();
    assert_eq!((got.ids.len(), got.total, got.whole), (400, Some(400), true));
    assert_eq!(pages(&http), [(0, 200), (200, 200)]);
    // The cap: what is past it is not read, and what was read is not all.
    let http = Scripted::new(offering(json!({})), listing(1_000, 5_000, false, true));
    let got = query(&http, 200, 300).unwrap();
    assert_eq!((got.ids.len(), got.total, got.whole), (300, Some(1_000), false));
    assert_eq!(pages(&http), [(0, 200), (200, 100)], "the last page asks for what is left of the cap");
    // A cap the total is no more than: all of it.
    let http = Scripted::new(offering(json!({})), listing(300, 5_000, false, true));
    assert!(query(&http, 200, 300).unwrap().whole);
}

#[test]
fn query_all_stops_at_a_page_that_brings_nothing_new() {
    // A server that ignores `position` and says so: the second page says it starts where the first did.
    let http = Scripted::new(offering(json!({})), listing(500, 5_000, true, false));
    let got = query(&http, 200, 5_000).unwrap();
    assert_eq!((got.ids.len(), got.whole), (200, false));
    assert_eq!(http.sent().len(), 2);
    // One that ignores it and says nothing of it: the second page brings nothing new.
    let silent = Scripted::new(offering(json!({})), |_, _| json!({"ids": (0..200).map(|n| format!("e{n}")).collect::<Vec<_>>()}));
    let got = query(&silent, 200, 5_000).unwrap();
    assert_eq!((got.ids.len(), got.whole), (200, false));
    assert_eq!(silent.sent().len(), 2);
}

#[test]
fn query_all_says_when_it_is_not_whole() {
    // A total the pages never reach (the server listed less than it counts).
    let short = Scripted::new(offering(json!({})), |_, args| {
        let ids: Vec<String> = if args["position"] == json!(0) { (0..50).map(|n| format!("e{n}")).collect() } else { Vec::new() };
        json!({"ids": ids, "total": 80, "position": args["position"]})
    });
    let got = query(&short, 200, 5_000).unwrap();
    assert_eq!((got.ids.len(), got.total, got.whole), (50, Some(80), false));
    // Ids that are none, an id twice, and a list that is not one.
    let flawed = Scripted::new(offering(json!({})), |_, _| json!({"ids": ["e1", "", "e2", "e1", 7, "two words"]}));
    let got = query(&flawed, 200, 5_000).unwrap();
    assert_eq!((got.ids, got.whole), (vec!["e1".to_string(), "e2".to_string()], false));
    let broken = Scripted::new(offering(json!({})), |_, _| json!({"ids": "e1"}));
    assert!(!query(&broken, 200, 5_000).unwrap().whole);
    let empty = Scripted::new(offering(json!({})), |_, _| json!({}));
    let got = query(&empty, 200, 5_000).unwrap();
    assert!(!got.whole && got.ids.is_empty(), "an answer with no list of ids is none");
    // A method error is the whole query's.
    let refused = Scripted::new(offering(json!({})), |_, _| json!({"error": "unsupportedFilter"}));
    assert!(matches!(query(&refused, 200, 5_000), Err(Error::Refused { kind, .. }) if kind == "unsupportedFilter"));
}

#[test]
fn a_page_cut_by_the_servers_own_limit_is_followed() {
    // Asked for 5,000 at a time, the server answers 500 and says so: a page of 500 is not the last.
    let http = Scripted::new(offering(json!({})), listing(1_200, 500, false, true));
    let got = query(&http, 5_000, 5_000).unwrap();
    assert_eq!((got.ids.len(), got.total, got.whole), (1_200, Some(1_200), true));
    assert_eq!(pages(&http), [(0, 5_000), (500, 500), (1_000, 500)], "then no more than it takes");
}

#[test]
fn a_pim_answer_over_its_bound_is_refused() {
    let mut http = Scripted::new(offering(json!({})), listing(10, 5_000, false, true));
    http.pad = IDS_BYTES;
    assert!(matches!(query(&http, 200, 5_000), Err(Error::TooLarge(_))), "a list of ids larger than 4 MB");
    // The bounds themselves.
    assert_eq!((IDS_BYTES, ITEMS_BYTES), (4 << 20, 8 << 20));
}

#[test]
fn events_are_written_and_the_door_of_contacts_stays_shut_until_they_are() {
    const { assert!(WRITE_EVENTS && !WRITE_CONTACTS) };
}

/// `value` changed by `change`, for building a session from another.
trait Tap: Sized {
    fn tap(self, change: impl FnOnce(&mut Self)) -> Self;
}

impl Tap for Value {
    fn tap(mut self, change: impl FnOnce(&mut Self)) -> Self {
        change(&mut self);
        self
    }
}
