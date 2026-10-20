// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A calendar whose server keeps only the basics of an event (`Calendar::plain`, as a JMAP server of a hosting page
//! says it keeps): what it refuses here, and why, and what it doesn't; and the owner's yes to the mail a change makes,
//! which goes to such a server with the change. Invented people at `example.org`.

use mailcraft_model::remote::{CalendarsDownload, EventChangeKind, EventsDownload, RemoteCalendar, RemoteEvent};
use mailcraft_model::{Address, Attendee, Id};
use mailcraft_time::{Date, DateTime, Timestamp};
use serde_json::{Value, json};

use super::{PLAIN_FREE_BUSY, PLAIN_KEEPS_NO};
use crate::cmd::remote::{apply_calendars, apply_events, event_upload};
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

#[test]
fn one_occurrence_of_somebody_elses_meeting_is_not_answered_alone_by_mail_in_a_plain_calendar() {
    for plain in [true, false] {
        let (mut s, _, _) = session(plain);
        let account = s.mailbox.accounts.iter().find(|a| a.email == "ada@example.org").map(|a| a.id).unwrap();
        // Bo's weekly meeting, Tuesdays at nine, Ada invited.
        let series = RemoteEvent {
            id: "V3".into(),
            title: "Weekly sync".into(),
            start: at(13, 9),
            end: at(13, 10),
            organizer: Some(Address::new("Bo", "bo@example.org")),
            attendees: vec![Attendee { address: Address::new("Ada", "ada@example.org"), ..Default::default() }],
            is_organizer: Some(false),
            kind: mailcraft_model::remote::RemoteEventKind::SeriesMaster,
            recurrence: Some(mailcraft_model::Recurrence { count: Some(4), ..mailcraft_model::Recurrence::every(mailcraft_model::Freq::Weekly) }),
            series_day: Date::new(2026, 10, 13),
            ..Default::default()
        };
        let download = EventsDownload {
            account,
            email: "ada@example.org".into(),
            from: at(1, 0),
            to: at(31, 0),
            complete: false,
            events: vec![series],
            ..Default::default()
        };
        apply_events(&mut s, &download).unwrap();
        let weekly = s.mailbox.events.values().find(|e| e.title == "Weekly sync").map(|e| e.id).unwrap();
        let one = |send: bool| json!({"id": weekly, "response": "decline", "scope": "occurrence", "occurrence": "2026-10-20", "send": send});
        if plain {
            assert_eq!(refused(&mut s, "calendar.respond", one(true)), super::PLAIN_WHOLE_ANSWER);
            let declined = json!({"id": weekly, "decline": true, "scope": "occurrence", "occurrence": "2026-10-20", "send": true});
            assert_eq!(refused(&mut s, "calendar.delete", declined), super::PLAIN_WHOLE_ANSWER);
            assert!(s.mailbox.event_changes.is_empty());
            // Kept here and on the copy, without telling Bo: as before.
            s.run("calendar.respond", &one(false)).unwrap();
        } else {
            s.run("calendar.respond", &one(true)).unwrap();
        }
        assert_eq!(s.mailbox.event_changes.len(), 1, "plain: {plain}");
        // The whole series is answered and told as before.
        s.run("calendar.respond", &json!({"id": weekly, "response": "accept", "send": true})).unwrap();
    }
}

#[test]
fn the_owners_yes_to_the_mail_goes_to_the_server_with_each_change_she_gave_it_for() {
    let (mut s, mine, _) = session(true);
    let account = s.mailbox.accounts.iter().find(|a| a.email == "ada@example.org").map(|a| a.id).unwrap();
    // Her own meeting with Bo.
    let review = RemoteEvent {
        id: "V9".into(),
        title: "Review".into(),
        start: at(21, 9),
        end: at(21, 10),
        organizer: Some(Address::new("Ada", "ada@example.org")),
        attendees: vec![Attendee { address: Address::new("Bo", "bo@example.org"), ..Default::default() }],
        is_organizer: Some(true),
        ..Default::default()
    };
    let download =
        EventsDownload { account, email: "ada@example.org".into(), from: at(1, 0), to: at(31, 0), events: vec![review], ..Default::default() };
    apply_events(&mut s, &download).unwrap();
    let review = s.mailbox.events.values().find(|e| e.title == "Review").map(|e| e.id).unwrap();
    // Her appointment moved: nobody to ask, and no yes goes with it (whoever the server's copy may invite by now).
    s.run("calendar.move", &json!({"id": mine, "start": "2026-10-19T10:00"})).unwrap();
    // Her meeting moved, and a new one, each with her yes to the mail.
    s.run("calendar.move", &json!({"id": review, "start": "2026-10-21T10:00", "send": true})).unwrap();
    s.run("calendar.newMeeting", &json!({"title": "Kick-off", "start": "2026-10-22T09:00", "attendees": "cy@example.org", "send": true})).unwrap();
    let said = |s: &crate::Session| -> Vec<(EventChangeKind, String, bool, bool)> {
        let mut ops: Vec<_> = event_upload(s, account).unwrap().ops.into_iter().map(|op| (op.kind, op.remote, op.send, op.cancel)).collect();
        ops.sort_by(|a, b| a.1.cmp(&b.1));
        ops
    };
    assert_eq!(
        said(&s),
        [
            (EventChangeKind::Create, String::new(), true, false),
            (EventChangeKind::Update, "V1".into(), false, false),
            (EventChangeKind::Update, "V9".into(), true, false)
        ]
    );
    // Called off with her yes: the deletion carries it.
    s.run("calendar.delete", &json!({"id": review, "send": true})).unwrap();
    assert!(said(&s).contains(&(EventChangeKind::Delete, "V9".into(), true, true)), "{:?}", said(&s));
}

#[test]
fn a_change_of_who_is_invited_says_who_was_invited_before_it() {
    let (mut s, mine, _) = session(true);
    let account = s.mailbox.accounts.iter().find(|a| a.email == "ada@example.org").map(|a| a.id).unwrap();
    s.run("calendar.edit", &json!({"id": mine, "attendees": "bo@example.org", "send": true})).unwrap();
    s.run("calendar.edit", &json!({"id": mine, "title": "Budget review (moved)", "send": true})).unwrap();
    let ops = event_upload(&s, account).unwrap().ops;
    let invited = |list: &Option<Vec<Attendee>>| list.iter().flatten().map(|a| a.address.email.clone()).collect::<Vec<_>>();
    assert_eq!((invited(&ops[0].fields.attendees), ops[0].invited_before.as_ref().map(Vec::len)), (vec!["bo@example.org".to_string()], Some(0)));
    // A change that leaves who is invited as it was says nothing of them.
    let (mut s, mine, _) = session(true);
    s.run("calendar.edit", &json!({"id": mine, "title": "Budget review (moved)"})).unwrap();
    assert_eq!(event_upload(&s, account).unwrap().ops[0].invited_before, None);
}
