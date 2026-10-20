// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The stand-in's calendars and address books, held to what a server that keeps only the basics serves, and a
//! client held to them.

use serde_json::{Value, json};

use super::super::{ACCOUNT, SESSION, StandIn, TOKEN};
use super::*;
use crate::{Auth, CALENDARS, CONTACTS, CORE, Client, Http, MAIL, Request, block_on};

fn connect(stand_in: &StandIn) -> Client<StandIn> {
    block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap()
}

/// Method calls as the API has them, and the HTTP status and body of the answer.
fn raw(stand_in: &StandIn, using: Value, calls: Value) -> (u16, Value) {
    let body = serde_json::to_vec(&json!({"using": using, "methodCalls": calls})).unwrap();
    let req =
        Request::post("https://jmap.standin.example/jmap/api", "application/json", body, 1 << 20).header("Authorization", &format!("Bearer {TOKEN}"));
    let resp = block_on(stand_in.send(req)).unwrap();
    (resp.status, serde_json::from_slice(&resp.body).unwrap_or(Value::Null))
}

#[test]
fn the_stand_in_offers_calendars_and_address_books_only_when_asked() {
    let stand_in = StandIn::new();
    let client = connect(&stand_in);
    assert!(!client.offers_calendars() && !client.offers_contacts(), "a stand-in for mail stays one");
    // A request may not name what the session doesn't offer.
    let (status, body) = raw(&stand_in, json!([CORE, MAIL, CALENDARS]), json!([["Mailbox/get", {"accountId": ACCOUNT}, "c0"]]));
    assert_eq!((status, body["type"].as_str()), (400, Some("urn:ietf:params:jmap:error:unknownCapability")));
    stand_in.with(|d| d.pim = Pim::offered());
    let client = connect(&stand_in);
    assert!(client.offers_calendars() && client.offers_contacts());
    assert_eq!((client.calendars_account(), client.contacts_account()), (Some(ACCOUNT), Some(ACCOUNT)));
    assert_eq!(client.calendars_span(), 500 * 86_400);
    assert_eq!(client.calendars_writable().map(<[String]>::len), Some(WRITABLE_EVENT.len()));
    assert!(client.calendars_writable().is_some_and(|written| !written.iter().any(|p| p == "keywords")), "no categories are kept");
    assert_eq!(client.contacts_writable().map(<[String]>::len), Some(WRITABLE_CARD.len()));
    let (status, _) = raw(&stand_in, json!([CORE, MAIL, CALENDARS, CONTACTS]), json!([["Mailbox/get", {"accountId": ACCOUNT}, "c0"]]));
    assert_eq!(status, 200);
    // A session that doesn't say what it writes, and a stretch it says otherwise.
    stand_in.with(|d| {
        d.pim.says_writable = false;
        d.pim.span = "P90D".into();
    });
    let client = connect(&stand_in);
    assert_eq!((client.calendars_writable(), client.contacts_writable(), client.calendars_span()), (None, None, 90 * 86_400));
}

/// What one call of calendars answered: its result, or the `type` of its error.
fn one(stand_in: &StandIn, name: &str, args: Value) -> Result<Value, String> {
    let (status, body) = raw(stand_in, json!([CORE, CALENDARS]), json!([[name, args, "c0"]]));
    assert_eq!(status, 200, "{body}");
    let answer = body["methodResponses"][0].clone();
    if answer[0] == "error" { Err(answer[1]["type"].as_str().unwrap_or_default().to_string()) } else { Ok(answer[1].clone()) }
}

/// A stand-in with the server's own fixtures: a weekly series of three in Zurich, a summer morning, a day off, and
/// the night summer time ends.
fn fixtures() -> StandIn {
    let stand_in = StandIn::new();
    stand_in.with(|d| {
        d.pim = Pim::offered();
        d.pim.add_calendar(Calendar { flagged: true, ..Calendar::new("own", "Ada", true) });
        d.pim.add_event(
            Event::timed("weekly", "own", "Site meeting", "2030-01-14T09:00:00", Some("Europe/Zurich"), "PT1H30M")
                .repeating(json!({"@type": "RecurrenceRule", "frequency": "weekly", "count": 3})),
        );
        d.pim.add_event(Event::timed("summer", "own", "Summer café", "2030-07-15T09:00:00", Some("Europe/Zurich"), "PT1H"));
        d.pim.add_event(Event::whole_days("day", "own", "Closed", "2030-01-20", 1));
        d.pim.add_event(Event::timed("dst", "own", "Night", "2030-10-27T01:00:00", Some("Europe/Zurich"), "PT3H"));
    });
    stand_in
}

fn window(after: &str, before: &str) -> Value {
    json!({"accountId": ACCOUNT, "filter": {"after": after, "before": before}, "expandRecurrences": true})
}

const JANUARY: (&str, &str) = ("2030-01-01T00:00:00Z", "2030-02-01T00:00:00Z");

#[test]
fn each_occurrence_comes_apart_with_its_utc_times() {
    let stand_in = fixtures();
    let found = one(&stand_in, "CalendarEvent/query", window(JANUARY.0, JANUARY.1)).unwrap();
    let ids: Vec<String> = found["ids"].as_array().unwrap().iter().map(|id| id.as_str().unwrap().to_string()).collect();
    assert_eq!(ids.len(), 4, "three occurrences and the day off: {found}");
    let got = one(&stand_in, "CalendarEvent/get", json!({"accountId": ACCOUNT, "ids": ids})).unwrap();
    let list = got["list"].as_array().unwrap();
    let weekly: Vec<&Value> = list.iter().filter(|e| e["title"] == "Site meeting").collect();
    let starts: Vec<&str> = weekly.iter().map(|e| e["utcStart"].as_str().unwrap()).collect();
    assert_eq!(starts, ["2030-01-14T08:00:00Z", "2030-01-21T08:00:00Z", "2030-01-28T08:00:00Z"]);
    assert_eq!(weekly[0]["utcEnd"], "2030-01-14T09:30:00Z", "an hour and a half, winter time");
    // An occurrence is named by its series and its start; its series by its own id.
    assert_eq!(weekly[0]["id"], json!(Pim::occurrence_id("weekly", "2030-01-14T09:00:00")));
    assert_eq!(weekly[0]["baseEventId"], json!(Pim::event_id("weekly")));
    assert_eq!(weekly[0]["recurrenceId"], "2030-01-14T09:00:00");
    assert!(weekly.iter().all(|e| e.get("recurrenceRule").is_none()));
    let day = list.iter().find(|e| e["title"] == "Closed").unwrap();
    assert!(day["showWithoutTime"] == true && day.get("utcStart").is_none(), "a day keeps its days");
    // The night summer time ends: three hours as they pass, from 01:00 summer time.
    let night = one(&stand_in, "CalendarEvent/get", json!({"accountId": ACCOUNT, "ids": [Pim::event_id("dst")]})).unwrap();
    assert_eq!(
        (night["list"][0]["utcStart"].as_str(), night["list"][0]["utcEnd"].as_str()),
        (Some("2030-10-26T23:00:00Z"), Some("2030-10-27T02:00:00Z"))
    );
    // Without expanding, the series is one event, read with its rule.
    let unexpanded =
        one(&stand_in, "CalendarEvent/query", json!({"accountId": ACCOUNT, "filter": {"after": JANUARY.0, "before": JANUARY.1}})).unwrap();
    assert!(unexpanded["ids"].as_array().unwrap().contains(&json!(Pim::event_id("weekly"))));
    let master = one(&stand_in, "CalendarEvent/get", json!({"accountId": ACCOUNT, "ids": [Pim::event_id("weekly")]})).unwrap();
    assert_eq!(master["list"][0]["recurrenceRule"]["count"], 3);
}

#[test]
fn a_query_requires_a_window_of_500_days_at_most_and_no_other_filter() {
    let stand_in = fixtures();
    for filter in [
        json!({}),
        json!({"after": JANUARY.0}),
        json!({"after": "2030-01-01T00:00:00Z", "before": "2032-01-01T00:00:00Z"}),
        json!({"text": "x", "after": JANUARY.0, "before": JANUARY.1}),
        json!("all"),
    ] {
        assert_eq!(
            one(&stand_in, "CalendarEvent/query", json!({"accountId": ACCOUNT, "filter": filter})),
            Err("unsupportedFilter".into()),
            "{filter}"
        );
    }
    let refused = |extra: Value| {
        let mut args = window(JANUARY.0, JANUARY.1);
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        one(&stand_in, "CalendarEvent/query", args).err()
    };
    assert_eq!(refused(json!({"anchor": "V1"})), Some("invalidArguments".into()));
    assert_eq!(refused(json!({"sort": [{"property": "start"}]})), Some("unsupportedSort".into()));
    assert_eq!(refused(json!({"limit": -1})), Some("invalidArguments".into()));
    assert_eq!(refused(json!({"position": "1"})), Some("invalidArguments".into()));
    assert_eq!(refused(json!({"collapseThreads": true})), Some("invalidArguments".into()));
    assert_eq!(
        one(&stand_in, "CalendarEvent/query", json!({"accountId": ACCOUNT, "filter": {"after": "soon", "before": JANUARY.1}})),
        Err("invalidArguments".into())
    );
    // Its limit is said back when it set it, and a position from the end counts from the end.
    let all = one(&stand_in, "CalendarEvent/query", window(JANUARY.0, JANUARY.1)).unwrap();
    assert_eq!((all["limit"].as_u64(), all["canCalculateChanges"].as_bool()), (Some(5_000), Some(false)));
    let mut last = window(JANUARY.0, JANUARY.1);
    last["position"] = json!(-1);
    last["calculateTotal"] = json!(true);
    let last = one(&stand_in, "CalendarEvent/query", last).unwrap();
    assert_eq!((last["ids"].as_array().map(Vec::len), last["total"].as_u64()), (Some(1), Some(4)));
    assert_eq!(last["ids"][0], all["ids"][3]);
    // A calendar it doesn't have finds nothing; one it has, its events.
    let mut named = window(JANUARY.0, JANUARY.1);
    named["filter"]["inCalendars"] = json!([Pim::calendar_id("nobody's")]);
    assert_eq!(one(&stand_in, "CalendarEvent/query", named.clone()).unwrap()["ids"], json!([]));
    named["filter"]["inCalendars"] = json!([Pim::calendar_id("own")]);
    assert_eq!(one(&stand_in, "CalendarEvent/query", named).unwrap()["ids"].as_array().map(Vec::len), Some(4));
}

#[test]
fn a_get_of_every_event_is_refused_and_properties_are_kept() {
    let stand_in = fixtures();
    assert_eq!(one(&stand_in, "CalendarEvent/get", json!({"accountId": ACCOUNT, "ids": null})), Err("requestTooLarge".into()));
    let ids: Vec<String> = (0..501).map(|n| format!("V{n}")).collect();
    assert_eq!(one(&stand_in, "CalendarEvent/get", json!({"accountId": ACCOUNT, "ids": ids})), Err("requestTooLarge".into()));
    assert_eq!(one(&stand_in, "CalendarEvent/get", json!({"accountId": ACCOUNT, "ids": ["V1"], "properties": [7]})), Err("invalidArguments".into()));
    let got = one(
        &stand_in,
        "CalendarEvent/get",
        json!({"accountId": ACCOUNT, "ids": [Pim::event_id("summer"), "Vnobody", "Oxx"], "properties": ["title"]}),
    )
    .unwrap();
    assert_eq!(got["list"][0].as_object().unwrap().keys().collect::<Vec<_>>(), ["@type", "id", "title"]);
    assert_eq!(got["notFound"], json!(["Vnobody", "Oxx"]));
    // An occurrence the series doesn't have, or leaves out, is not found.
    stand_in.with(|d| d.pim.events[0].excluded.push("2030-01-21T09:00:00".into()));
    let ids = [
        Pim::occurrence_id("weekly", "2030-01-21T09:00:00"),
        Pim::occurrence_id("weekly", "2030-01-22T09:00:00"),
        Pim::occurrence_id("weekly", "2030-01-28T09:00:00"),
    ];
    let got = one(&stand_in, "CalendarEvent/get", json!({"accountId": ACCOUNT, "ids": ids})).unwrap();
    assert_eq!(got["list"].as_array().map(Vec::len), Some(1));
    let master = one(&stand_in, "CalendarEvent/get", json!({"accountId": ACCOUNT, "ids": [Pim::event_id("weekly")]})).unwrap();
    assert_eq!(master["list"][0]["recurrenceOverrides"], json!({"2030-01-21T09:00:00": {"excluded": true}}));
    // And events it can't read now fail the whole call.
    stand_in.with(|d| {
        d.pim.broken_events.insert(Pim::event_id("summer"));
    });
    assert_eq!(one(&stand_in, "CalendarEvent/get", json!({"accountId": ACCOUNT, "ids": [Pim::event_id("summer")]})), Err("serverFail".into()));
}

#[test]
fn one_calendar_is_the_default_and_colours_are_rrggbb() {
    let stand_in = StandIn::new();
    stand_in.with(|d| {
        d.pim = Pim::offered();
        d.pim.add_calendar(Calendar { flagged: true, color: Some("#F80".into()), ..Calendar::new("holidays", "Holidays", false) });
        d.pim.add_calendar(Calendar { flagged: true, color: Some("rgb(1,2,3)".into()), ..Calendar::new("work", "Work", true) });
        d.pim.add_calendar(Calendar { flagged: true, color: Some("Navy".into()), ..Calendar::new("home", "Home", true) });
    });
    let listed = one(&stand_in, "Calendar/get", json!({"accountId": ACCOUNT, "ids": null})).unwrap();
    let list = listed["list"].as_array().unwrap();
    let defaults: Vec<&str> = list.iter().filter(|c| c["isDefault"] == true).map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(defaults, ["Work"], "the first flagged of those whose events may be written");
    assert_eq!(list.iter().map(|c| c["color"].clone()).collect::<Vec<_>>(), [json!("#ff8800"), json!(null), json!("#000080")]);
    assert_eq!(list[0]["myRights"].as_object().map(|r| r.len()), Some(8), "the eight rights of the draft");
    assert_eq!((list[0]["myRights"]["mayWriteAll"].as_bool(), list[1]["myRights"]["mayWriteAll"].as_bool()), (Some(false), Some(true)));
    let some = one(&stand_in, "Calendar/get", json!({"accountId": ACCOUNT, "ids": [Pim::calendar_id("home"), "Cnone"]})).unwrap();
    assert_eq!((some["list"].as_array().map(Vec::len), some["notFound"].clone()), (Some(1), json!(["Cnone"])));
    assert_eq!(one(&stand_in, "Calendar/get", json!({"accountId": "A2"})), Err("accountNotFound".into()));
}

#[test]
fn the_methods_of_calendars_are_unknown_where_none_are_offered() {
    let stand_in = StandIn::new();
    stand_in.with(|d| d.pim = Pim { calendars: false, ..Pim::offered() });
    let (status, body) = raw(&stand_in, json!([CORE, MAIL]), json!([["Calendar/get", {"accountId": ACCOUNT}, "c0"]]));
    assert_eq!((status, body["methodResponses"][0][1]["type"].as_str()), (200, Some("unknownMethod")));
}

/// What one call of address books answered: its result, or the `type` of its error.
fn card_call(stand_in: &StandIn, name: &str, args: Value) -> Result<Value, String> {
    let (status, body) = raw(stand_in, json!([CORE, CONTACTS]), json!([[name, args, "c0"]]));
    assert_eq!(status, 200, "{body}");
    let answer = body["methodResponses"][0].clone();
    if answer[0] == "error" { Err(answer[1]["type"].as_str().unwrap_or_default().to_string()) } else { Ok(answer[1].clone()) }
}

#[test]
fn cards_are_served_as_a_server_that_keeps_only_the_basics_serves_them() {
    let stand_in = StandIn::new();
    stand_in.with(|d| {
        d.pim = Pim::offered();
        d.pim.add_book(Book { default: true, ..Book::new("neoffice", "Neoffice", true) });
        d.pim.add_book(Book { may_remove: false, ..Book::new("sales", "Sales", true) });
        d.pim.add_book(Book::new("partners", "Partners", false));
        d.pim.add_card(
            Card::person("Grace Okafor", "neoffice", "Grace", "Okafor", "grace@example.org")
                .with_email("g.okafor@example.org")
                .with_phone("+41 44 555 01 01", false)
                .with_phone("+41 79 555 01 02", true)
                .working("Engineer", "Okafor Engineering"),
        );
        d.pim.add_card(Card::company("Lindqvist Joinery", "partners", "Lindqvist Joinery", "joinery@example.org"));
    });
    // The books, with the rights such a server says.
    let books = card_call(&stand_in, "AddressBook/get", json!({"accountId": ACCOUNT, "ids": null})).unwrap();
    let rights: Vec<(Value, Value, Value)> = books["list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| (b["isDefault"].clone(), b["myRights"]["mayWrite"].clone(), b["myRights"]["mayRemoveItems"].clone()))
        .collect();
    assert_eq!(
        rights,
        [(json!(true), json!(true), json!(true)), (json!(false), json!(true), json!(false)), (json!(false), json!(false), json!(false))]
    );
    let some = card_call(&stand_in, "AddressBook/get", json!({"accountId": ACCOUNT, "ids": ["sales", "gone"]})).unwrap();
    assert_eq!((some["list"].as_array().map(Vec::len), some["notFound"].clone()), (Some(1), json!(["gone"])));
    // A card: its id is the contact's name, its name in components, the primary address preferred, the primary mobile
    // number a mobile and the others voice, one organization, one title.
    let got = card_call(&stand_in, "ContactCard/get", json!({"accountId": ACCOUNT, "ids": ["Grace Okafor", "Nobody"]})).unwrap();
    let grace = &got["list"][0];
    assert_eq!(
        (grace["id"].clone(), grace["uid"].clone(), grace["addressBookIds"].clone()),
        (json!("Grace Okafor"), json!("Grace Okafor"), json!({"neoffice": true}))
    );
    assert_eq!(grace["name"]["full"], "Grace Okafor");
    assert_eq!(
        grace["name"]["components"],
        json!([{"@type": "NameComponent", "kind": "given", "value": "Grace"}, {"@type": "NameComponent", "kind": "surname", "value": "Okafor"}])
    );
    assert_eq!((grace["emails"]["e1"]["pref"].clone(), grace["emails"]["e2"].get("pref").cloned()), (json!(1), None));
    assert_eq!(
        (grace["phones"]["p1"]["features"].clone(), grace["phones"]["p2"]["features"].clone()),
        (json!({"voice": true}), json!({"mobile": true}))
    );
    assert_eq!((grace["organizations"]["o1"]["name"].clone(), grace["titles"]["t1"]["kind"].clone()), (json!("Okafor Engineering"), json!("title")));
    assert_eq!(got["notFound"], json!(["Nobody"]));
    assert!(got["state"].as_str().is_some_and(|state| !state.is_empty()));
    // A company of its own: called by its company, with no components.
    let joinery = card_call(&stand_in, "ContactCard/get", json!({"accountId": ACCOUNT, "ids": ["Lindqvist Joinery"]})).unwrap();
    assert_eq!(
        (joinery["list"][0]["name"]["full"].clone(), joinery["list"][0]["name"]["components"].clone()),
        (json!("Lindqvist Joinery"), json!([]))
    );
    // `properties` keeps what it knows of what was named, the id always; what it doesn't know is no error.
    let few =
        card_call(&stand_in, "ContactCard/get", json!({"accountId": ACCOUNT, "ids": ["Grace Okafor"], "properties": ["emails", "notes"]})).unwrap();
    let keys: Vec<&String> = few["list"][0].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["@type", "emails", "id", "version"]);
    // Every card at once only while there are no more than a `get` takes.
    assert!(card_call(&stand_in, "ContactCard/get", json!({"accountId": ACCOUNT, "ids": null})).is_ok());
    stand_in.with(|d| d.max_get = 1);
    assert_eq!(card_call(&stand_in, "ContactCard/get", json!({"accountId": ACCOUNT, "ids": null})), Err("requestTooLarge".into()));
    // The query: by id, a page of them, the total, the limit it set, a position from the end, and the state.
    stand_in.with(|d| d.pim.card_limit = 1);
    let page = card_call(&stand_in, "ContactCard/query", json!({"accountId": ACCOUNT, "calculateTotal": true})).unwrap();
    assert_eq!((page["ids"].clone(), page["total"].clone(), page["limit"].clone()), (json!(["Grace Okafor"]), json!(2), json!(1)));
    assert!(page["queryState"].as_str().is_some() && page["canCalculateChanges"] == false);
    let last = card_call(&stand_in, "ContactCard/query", json!({"accountId": ACCOUNT, "position": -1, "limit": 1})).unwrap();
    assert_eq!((last["ids"].clone(), last["position"].clone(), last.get("limit").cloned()), (json!(["Lindqvist Joinery"]), json!(1), None));
    assert_eq!(card_call(&stand_in, "ContactCard/query", json!({"accountId": ACCOUNT, "limit": -1})), Err("invalidArguments".into()));
    // The state changes when a card does.
    let before = page["queryState"].clone();
    stand_in.with(|d| d.pim.cards[0].last = "Okafor-Mensah".into());
    let after = card_call(&stand_in, "ContactCard/query", json!({"accountId": ACCOUNT})).unwrap();
    assert!(after["queryState"] != before);
    stand_in.with(|d| d.pim.says_state = false);
    assert!(card_call(&stand_in, "ContactCard/query", json!({"accountId": ACCOUNT})).unwrap().get("queryState").is_none());
}

#[test]
fn the_methods_of_address_books_are_unknown_where_none_are_offered() {
    let stand_in = StandIn::new();
    stand_in.with(|d| d.pim = Pim { contacts: false, ..Pim::offered() });
    let (status, body) = raw(&stand_in, json!([CORE, CALENDARS]), json!([["ContactCard/query", {"accountId": ACCOUNT}, "c0"]]));
    assert_eq!((status, body["methodResponses"][0][1]["type"].as_str()), (200, Some("unknownMethod")));
    // Offered, a method of address books needs their capability named.
    stand_in.with(|d| d.pim = Pim::offered());
    let (_, body) = raw(&stand_in, json!([CORE, CALENDARS]), json!([["AddressBook/get", {"accountId": ACCOUNT}, "c0"]]));
    assert_eq!(body["methodResponses"][0][1]["type"].as_str(), Some("unknownMethod"));
}
