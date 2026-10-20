// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A calendar whose server keeps only the basics of an event (`Calendar::plain`, as a JMAP server of a hosting page
//! says it keeps): what it refuses here, and why, and what it doesn't. Invented people at `example.org`.

use mailcraft_model::remote::{CalendarsDownload, EventsDownload, RemoteCalendar, RemoteEvent};
use mailcraft_model::{Address, Attendee, Id};
use mailcraft_time::{Date, DateTime, Timestamp};
use serde_json::{Value, json};

use super::{PLAIN_FREE_BUSY, PLAIN_KEEPS_NO};
use crate::cmd::remote::{apply_calendars, apply_events};
use crate::{CmdError, Session};

fn at(day: u8, hour: u8) -> Timestamp {
    DateTime::new(Date::new(2026, 10, day).unwrap(), hour, 0, 0).unwrap().to_timestamp(0)
}

/// A JMAP account whose server keeps only the basics (`plain`), or one that keeps everything: its main calendar, with
/// an appointment of the owner's and a meeting of Bo's she is invited to. The session, the appointment and the meeting.
fn session(plain: bool) -> (Session, Id, Id) {
    let now = at(14, 16);
    let mut s = Session::at(crate::sample::mailbox(now, 0), now, 0);
    let added = s.run(
        "account.add",
        &json!({"email": "ada@example.org", "kind": "jmap", "jmap": {"session": "https://jmap.example.org/session", "bearer": true}}),
    );
    let account = added.unwrap()["id"].as_u64().unwrap();
    let list = CalendarsDownload {
        account,
        email: "ada@example.org".into(),
        complete: true,
        calendars: vec![RemoteCalendar { id: "C1".into(), name: "Ada".into(), main: true, plain, ..Default::default() }],
    };
    apply_calendars(&mut s, &list).unwrap();
    let mine = RemoteEvent {
        id: "V1".into(),
        title: "Budget review".into(),
        start: at(19, 9),
        end: at(19, 10),
        is_organizer: Some(true),
        ..Default::default()
    };
    let theirs = RemoteEvent {
        id: "V2".into(),
        title: "Planning".into(),
        start: at(20, 9),
        end: at(20, 10),
        organizer: Some(Address::new("Bo", "bo@example.org")),
        attendees: vec![Attendee { address: Address::new("Ada", "ada@example.org"), ..Default::default() }],
        is_organizer: Some(false),
        ..Default::default()
    };
    let download = EventsDownload {
        account,
        email: "ada@example.org".into(),
        from: at(1, 0),
        to: at(31, 0),
        complete: true,
        events: vec![mine, theirs],
        ..Default::default()
    };
    apply_events(&mut s, &download).unwrap();
    let id = |title: &str| s.mailbox.events.values().find(|e| e.title == title).map(|e| e.id).unwrap();
    let (mine, theirs) = (id("Budget review"), id("Planning"));
    (s, mine, theirs)
}

fn refused(s: &mut Session, id: &str, params: Value) -> String {
    match s.run(id, &params) {
        Err(CmdError::Failed(why)) => why,
        other => panic!("{id} {params}: expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_plain_calendar_refuses_categories_importance_options_teams_and_proposals_with_a_reason() {
    let (mut s, mine, theirs) = session(true);
    for (command, params) in [
        ("calendar.categorize", json!({"id": mine, "category": "Red"})),
        ("calendar.importance", json!({"id": mine, "value": "high"})),
        ("calendar.teamsMeeting", json!({"id": mine, "value": true})),
        ("calendar.edit", json!({"id": mine, "categories": ["Red"]})),
        ("calendar.edit", json!({"id": mine, "importance": "low"})),
        ("calendar.new", json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00", "categories": ["Red"]})),
        ("calendar.propose", json!({"id": theirs, "start": "2026-10-20T11:00", "end": "2026-10-20T12:00", "send": true})),
        (
            "calendar.respond",
            json!({"id": theirs, "response": "tentative", "send": true, "propose": {"start": "2026-10-20T11:00", "end": "2026-10-20T12:00"}}),
        ),
        ("calendar.forward", json!({"id": theirs, "to": "eve@example.org", "send": true})),
    ] {
        assert_eq!(refused(&mut s, command, params.clone()), PLAIN_KEEPS_NO, "{command} {params}");
    }
    assert!(s.mailbox.event_changes.is_empty(), "nothing noted for the server");
    // What it keeps goes as before.
    s.run("calendar.edit", &json!({"id": mine, "title": "Budget review (moved)", "location": "Room 5"})).unwrap();
    s.run("calendar.new", &json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00"})).unwrap();
    assert_eq!(s.mailbox.event_changes.len(), 2);
}

#[test]
fn show_as_other_than_free_or_busy_is_refused_in_a_plain_calendar() {
    let (mut s, mine, theirs) = session(true);
    for value in ["tentative", "outOfOffice", "workingElsewhere"] {
        assert_eq!(refused(&mut s, "calendar.showAs", json!({"id": mine, "value": value})), PLAIN_FREE_BUSY, "{value}");
    }
    assert_eq!(refused(&mut s, "calendar.edit", json!({"id": mine, "showAs": "outOfOffice"})), PLAIN_FREE_BUSY);
    s.run("calendar.showAs", &json!({"id": mine, "value": "free"})).unwrap();
    assert_eq!(s.mailbox.event(mine).map(|e| e.show_as), Some(mailcraft_model::ShowAs::Free));
    // A tentative answer shows busy there: that is what the server keeps.
    s.run("calendar.respond", &json!({"id": theirs, "response": "tentative"})).unwrap();
    assert_eq!(s.mailbox.event(theirs).map(|e| e.show_as), Some(mailcraft_model::ShowAs::Busy));
}

#[test]
fn a_graph_calendar_is_not_plain() {
    let (mut s, mine, _) = session(false);
    s.run("calendar.categorize", &json!({"id": mine, "category": "Red"})).unwrap();
    s.run("calendar.showAs", &json!({"id": mine, "value": "tentative"})).unwrap();
    s.run("calendar.importance", &json!({"id": mine, "value": "high"})).unwrap();
    let e = s.mailbox.event(mine).unwrap();
    assert_eq!(
        (e.categories.as_slice(), e.show_as, e.importance),
        (["Red".to_string()].as_slice(), mailcraft_model::ShowAs::Tentative, mailcraft_model::Importance::High)
    );
    // A calendar of this computer's is never plain.
    let own = s.mailbox.calendars.iter().find(|c| c.origin.account == 0).map(|c| c.id).unwrap();
    assert!(!super::plain(&s, own));
}
