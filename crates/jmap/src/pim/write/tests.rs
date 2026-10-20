// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! An event made, changed or answered here, as JSCalendar writes it. Invented people at `example.org`.

use mailcraft_model::remote::{EventChangeKind, EventFields, EventOp, EventReminder, EventRepeat, EventTime, EventWall, EventZones};
use mailcraft_model::{Address, Attendee, Freq, Recurrence, Response, ShowAs};
use mailcraft_time::{Date, DateTime, Weekday, Zone};
use serde_json::{Value, json};

use super::*;

const ME: &str = "ada@example.org";

fn zurich() -> Zone {
    Zone::named("Europe/Zurich").unwrap()
}

/// 2026-10-`day` at `hour`:`minute` UTC.
fn utc(day: u8, hour: i64, minute: i64) -> i64 {
    DateTime::at_midnight(Date::new(2026, 10, day).unwrap()).to_timestamp(0) + hour * 3_600 + minute * 60
}

fn timed(start: i64, end: i64) -> EventTime {
    EventTime { start, end, all_day: false }
}

/// A new appointment as the engine says it: an hour on Monday 2026-10-19 from nine in Zurich (07:00 UTC).
fn new_event(change: impl FnOnce(&mut EventOp)) -> EventOp {
    let mut op = EventOp {
        kind: EventChangeKind::Create,
        event: 7,
        token: "mc-token-7".into(),
        fields: EventFields {
            title: Some("Budget review".into()),
            location: Some("Room 4".into()),
            description: Some("Bring the figures.".into()),
            time: Some(timed(utc(19, 7, 0), utc(19, 8, 0))),
            show_as: Some(ShowAs::Busy),
            private: Some(false),
            reminder: Some(EventReminder { on: true, minutes: 15 }),
            zones: Some(EventZones { start: "Europe/Zurich".into(), end: "Europe/Zurich".into() }),
            ..Default::default()
        },
        ..Default::default()
    };
    change(&mut op);
    op
}

fn made(op: &EventOp) -> (Value, bool, String) {
    create(op, Clock::Own(zurich())).unwrap()
}

#[test]
fn a_new_event_is_written_whole_with_its_uid_zone_and_exact_duration() {
    let (event, invites, zone) = made(&new_event(|_| {}));
    assert_eq!(
        event,
        json!({
            "@type": "Event",
            "uid": "mc-token-7",
            "title": "Budget review",
            "description": "Bring the figures.",
            "locations": {"l1": {"@type": "Location", "name": "Room 4"}},
            "start": "2026-10-19T09:00:00",
            "timeZone": "Europe/Zurich",
            "duration": "PT1H",
            "showWithoutTime": false,
            "freeBusyStatus": "busy",
            "privacy": "public",
            "alerts": {"a1": {"@type": "Alert", "trigger": {"@type": "OffsetTrigger", "offset": "-PT15M", "relativeTo": "start"}, "action": "display"}},
        })
    );
    assert!(!invites, "nobody to invite");
    assert_eq!(zone, "Europe/Zurich");
    // An event of no zone of its own is written on the clock of the calendar's.
    let (floating, _, zone) = create(&new_event(|op| op.fields.zones = None), Clock::Own(Zone::named("America/New_York").unwrap())).unwrap();
    assert_eq!(
        (floating["start"].as_str(), floating["timeZone"].as_str(), zone.as_str()),
        (Some("2026-10-19T03:00:00"), Some("America/New_York"), "America/New_York")
    );
    // Ninety minutes are an hour and a half.
    let (long, ..) = made(&new_event(|op| op.fields.time = Some(timed(utc(19, 7, 0), utc(19, 8, 30)))));
    assert_eq!(long["duration"], "PT1H30M");
    // Nothing written that such a server doesn't keep.
    let (kept, ..) = made(&new_event(|op| {
        op.fields.categories = Some(vec!["Red".into()]);
        op.fields.importance = Some(mailcraft_model::Importance::High);
        op.fields.request_responses = Some(false);
        op.fields.online = Some(mailcraft_model::OnlineProvider::Teams);
    }));
    for key in ["keywords", "priority", "categories", "virtualLocations"] {
        assert!(kept.get(key).is_none(), "{key}");
    }
}

#[test]
fn a_new_event_in_the_main_calendar_names_none_and_in_another_names_it() {
    let (main, ..) = made(&new_event(|_| {}));
    assert!(main.get("calendarIds").is_none(), "the server's default");
    let (team, ..) = made(&new_event(|op| op.calendar = "Cteam".into()));
    assert_eq!(team["calendarIds"], json!({"Cteam": true}));
}

#[test]
fn a_whole_day_event_is_written_as_its_days() {
    let days = EventTime { start: utc(20, 0, 0), end: utc(23, 0, 0), all_day: true };
    let (event, _, zone) = made(&new_event(|op| op.fields.time = Some(days)));
    assert_eq!(
        (event["start"].as_str(), event["duration"].as_str(), event["showWithoutTime"].as_bool(), &event["timeZone"]),
        (Some("2026-10-20T00:00:00"), Some("P3D"), Some(true), &Value::Null)
    );
    assert_eq!(zone, "");
    // At least one day.
    let (one, ..) = made(&new_event(|op| op.fields.time = Some(EventTime { start: utc(20, 0, 0), end: utc(20, 0, 0), all_day: true })));
    assert_eq!(one["duration"], "P1D");
}

#[test]
fn the_repeated_hour_of_the_night_the_clocks_go_back_is_written_in_utc_and_said() {
    // 2026-10-25: half past two in Zurich comes twice, at 00:30 and 01:30 UTC. The first is told on the clock.
    let (first, _, zone) = made(&new_event(|op| op.fields.time = Some(timed(utc(25, 0, 30), utc(25, 1, 0)))));
    assert_eq!(
        (first["start"].as_str(), first["timeZone"].as_str(), zone.as_str()),
        (Some("2026-10-25T02:30:00"), Some("Europe/Zurich"), "Europe/Zurich")
    );
    // The second can't be: in UTC, and the zone it went in says so.
    let (second, _, zone) = made(&new_event(|op| op.fields.time = Some(timed(utc(25, 1, 30), utc(25, 2, 0)))));
    assert_eq!(
        (second["start"].as_str(), second["timeZone"].as_str(), second["duration"].as_str()),
        (Some("2026-10-25T01:30:00"), Some(UTC_ZONE), Some("PT30M"))
    );
    assert_eq!(zone, UTC_ZONE);
    // A zone the server has no name for (an offset) is UTC too.
    let (offset, _, zone) = made(&new_event(|op| op.fields.zones = Some(EventZones { start: "UTC+05:30".into(), end: String::new() })));
    assert_eq!((offset["start"].as_str(), zone.as_str()), (Some("2026-10-19T07:00:00"), UTC_ZONE));
}

#[test]
fn a_day_or_more_across_a_change_of_clock_is_written_as_days() {
    // Nine on Saturday (summer time, 07:00 UTC) to nine on Sunday (winter time, 08:00 UTC): a day on the clock.
    let (event, ..) = made(&new_event(|op| op.fields.time = Some(timed(utc(24, 7, 0), utc(25, 8, 0)))));
    assert_eq!((event["start"].as_str(), event["duration"].as_str()), (Some("2026-10-24T09:00:00"), Some("P1D")));
    // And to eleven on Monday: two days and two hours.
    let (longer, ..) = made(&new_event(|op| op.fields.time = Some(timed(utc(24, 7, 0), utc(26, 10, 0)))));
    assert_eq!(longer["duration"], "P2DT2H");
    // Three hours across the change are the three hours that pass.
    let (night, ..) = made(&new_event(|op| op.fields.time = Some(timed(utc(24, 23, 0), utc(25, 2, 0)))));
    assert_eq!((night["start"].as_str(), night["duration"].as_str()), (Some("2026-10-25T01:00:00"), Some("PT3H")));
}

#[test]
fn free_busy_privacy_and_a_reminder_are_written() {
    let (free, ..) = made(&new_event(|op| {
        op.fields.show_as = Some(ShowAs::Free);
        op.fields.private = Some(true);
        op.fields.reminder = Some(EventReminder { on: false, minutes: 15 });
    }));
    assert_eq!((free["freeBusyStatus"].as_str(), free["privacy"].as_str(), &free["alerts"]), (Some("free"), Some("private"), &json!({})));
    // Out of the office is busy to such a server.
    let (away, ..) = made(&new_event(|op| op.fields.show_as = Some(ShowAs::OutOfOffice)));
    assert_eq!(away["freeBusyStatus"], "busy");
    // The afternoon before a day off: eighteen hours.
    let (early, ..) = made(&new_event(|op| op.fields.reminder = Some(EventReminder { on: true, minutes: 18 * 60 })));
    assert_eq!(early["alerts"]["a1"]["trigger"]["offset"], "-PT18H");
}

#[test]
fn the_rule_is_written_from_the_model() {
    let first = Date::new(2026, 10, 19).unwrap();
    let rule = |r: Recurrence| rule_json(&r, first);
    assert_eq!(
        rule(Recurrence { count: Some(6), ..Recurrence::every(Freq::Weekly) }),
        json!({"@type": "RecurrenceRule", "frequency": "weekly", "count": 6})
    );
    assert_eq!(
        rule(Recurrence { interval: 2, by_day: vec![Weekday::Mon, Weekday::Wed], until: Date::new(2026, 12, 31), ..Recurrence::every(Freq::Weekly) }),
        json!({"@type": "RecurrenceRule", "frequency": "weekly", "interval": 2, "byDay": [{"@type": "NDay", "day": "mo"}, {"@type": "NDay", "day": "we"}], "until": "2026-12-31T23:59:59"})
    );
    assert_eq!(
        rule(Recurrence { by_day: vec![Weekday::Mon], nth: Some(3), ..Recurrence::every(Freq::Monthly) }),
        json!({"@type": "RecurrenceRule", "frequency": "monthly", "byDay": [{"@type": "NDay", "day": "mo", "nthOfPeriod": 3}]})
    );
    assert_eq!(
        rule(Recurrence { by_day: vec![Weekday::Mon], nth: Some(-1), ..Recurrence::every(Freq::Yearly) }),
        json!({"@type": "RecurrenceRule", "frequency": "yearly", "byDay": [{"@type": "NDay", "day": "mo", "nthOfPeriod": -1}], "byMonth": ["10"]})
    );
    assert_eq!(rule(Recurrence::every(Freq::Daily)), json!({"@type": "RecurrenceRule", "frequency": "daily"}));
    // A rule written is the rule read.
    for written in [
        Recurrence { count: Some(6), ..Recurrence::every(Freq::Weekly) },
        Recurrence { interval: 3, by_day: vec![Weekday::Fri], nth: Some(-1), ..Recurrence::every(Freq::Monthly) },
        Recurrence { by_day: vec![Weekday::Mon, Weekday::Tue], until: Date::new(2027, 1, 31), ..Recurrence::every(Freq::Daily) },
    ] {
        let start = DateTime { date: first, hour: 9, minute: 0, second: 0 };
        let read = crate::pim::jscalendar::rule_of(rule(written.clone()).as_object().unwrap(), start, false).unwrap();
        assert_eq!(read, Recurrence { interval: written.interval.max(1), ..written });
    }
    // A new series carries its rule, on its clock.
    let (series, ..) = made(&new_event(|op| {
        op.fields.repeat = Some(EventRepeat { rule: Some(Recurrence { count: Some(6), ..Recurrence::every(Freq::Weekly) }), first });
        op.fields.wall = Some(EventWall {
            start: DateTime { date: first, hour: 9, minute: 0, second: 0 },
            end: DateTime { date: first, hour: 10, minute: 0, second: 0 },
        });
    }));
    assert_eq!((series["start"].as_str(), series["recurrenceRule"]["count"].as_u64()), (Some("2026-10-19T09:00:00"), Some(6)));
}

/// The server's copy of a meeting Ada organizes, with Bob and Carol invited, in Room 4 and a second place.
fn copy() -> Copy {
    json!({
        "id": "V1",
        "organizerCalendarAddress": "mailto:ada@example.org",
        "locations": {"l1": {"@type": "Location", "name": "Room 4"}, "l2": {"@type": "Location", "name": "", "coordinates": "geo:46.5,6.6"}},
        "participants": {
            "ada": {"@type": "Participant", "calendarAddress": "mailto:ada@example.org", "roles": {"owner": true, "attendee": true}, "participationStatus": "accepted"},
            "bob": {"@type": "Participant", "calendarAddress": "mailto:bob@example.org", "roles": {"attendee": true}, "participationStatus": "accepted"},
            "carol": {"@type": "Participant", "email": "carol@example.org", "roles": {"attendee": true}, "participationStatus": "tentative"},
        },
    })
    .as_object()
    .unwrap()
    .clone()
}

fn change(fields: EventFields) -> EventOp {
    EventOp { kind: EventChangeKind::Update, event: 7, remote: "V1".into(), fields, ..Default::default() }
}

fn patched(fields: EventFields) -> Value {
    patch(&change(fields), Some(&copy()), Clock::Own(zurich()), &[ME.to_string()]).unwrap().0
}

#[test]
fn a_change_carries_only_the_properties_that_changed_each_whole() {
    assert_eq!(patched(EventFields { title: Some("Budget review (moved)".into()), ..Default::default() }), json!({"title": "Budget review (moved)"}));
    let moved = patched(EventFields {
        time: Some(timed(utc(19, 12, 0), utc(19, 13, 0))),
        zones: Some(EventZones { start: "Europe/Zurich".into(), end: "Europe/Zurich".into() }),
        ..Default::default()
    });
    assert_eq!(moved, json!({"start": "2026-10-19T14:00:00", "timeZone": "Europe/Zurich", "duration": "PT1H", "showWithoutTime": false}));
    assert_eq!(patched(EventFields { reminder: Some(EventReminder { on: false, minutes: 0 }), ..Default::default() }), json!({"alerts": {}}));
    // A series that no longer repeats.
    let once = patched(EventFields { repeat: Some(EventRepeat { rule: None, first: Date::new(2026, 10, 19).unwrap() }), ..Default::default() });
    assert_eq!(once, json!({"recurrenceRule": null}));
    // Nothing such a server keeps: no change at all.
    let kept_no = EventFields { categories: Some(vec!["Red".into()]), importance: Some(mailcraft_model::Importance::High), ..Default::default() };
    assert_eq!(patch(&change(kept_no), Some(&copy()), Clock::Own(zurich()), &[ME.to_string()]), None);
    // Never a key with a slash.
    let all = patched(EventFields {
        title: Some("x".into()),
        location: Some("Room 5".into()),
        attendees: Some(vec![Attendee { address: Address::new("Bob", "bob@example.org"), ..Default::default() }]),
        ..Default::default()
    });
    assert!(all.as_object().unwrap().keys().all(|key| !key.contains('/')));
}

#[test]
fn locations_and_participants_are_the_servers_map_with_the_persons_change_made_to_it() {
    // The first place with a name is the place: renamed, the other stays as the server has it.
    let renamed = patched(EventFields { location: Some("Room 5".into()), ..Default::default() });
    assert_eq!(renamed["locations"]["l1"]["name"], "Room 5");
    assert_eq!(renamed["locations"]["l2"], copy()["locations"]["l2"]);
    // No place: the named one goes.
    let none = patched(EventFields { location: Some(String::new()), ..Default::default() });
    assert_eq!(none["locations"].as_object().unwrap().keys().collect::<Vec<_>>(), ["l2"]);
    // Carol no longer invited, Dan invited, Bob optional: the organizer and Bob's own entry stay as the server has them.
    let now = EventFields {
        attendees: Some(vec![
            Attendee { address: Address::new("Bob", "bob@example.org"), optional: true, response: Response::None },
            Attendee { address: Address::new("Dan Moreau", "Dan@Example.org"), ..Default::default() },
        ]),
        ..Default::default()
    };
    let people = patch(&EventOp { invited_before: Some(bob_and_carol()), ..change(now) }, Some(&copy()), Clock::Own(zurich()), &[ME.to_string()]);
    let people = people.unwrap().0;
    let people = people["participants"].as_object().unwrap();
    let mut keys: Vec<&String> = people.keys().collect();
    keys.sort();
    assert_eq!(keys, ["ada", "bob", "p1"]);
    assert_eq!(people["ada"], copy()["participants"]["ada"]);
    assert_eq!((people["bob"]["participationStatus"].as_str(), people["bob"]["roles"]["optional"].as_bool()), (Some("accepted"), Some(true)));
    assert_eq!(
        people["p1"],
        json!({"@type": "Participant", "name": "Dan Moreau", "email": "Dan@Example.org", "sendTo": {"imip": "mailto:Dan@Example.org"}, "roles": {"attendee": true}, "participationStatus": "needs-action", "expectReply": true})
    );
}

/// Who the window had invited to the meeting of [`copy`] when the change was made: Bob and Carol.
fn bob_and_carol() -> Vec<Attendee> {
    vec![
        Attendee { address: Address::new("Bob", "bob@example.org"), ..Default::default() },
        Attendee { address: Address::new("Carol", "carol@example.org"), ..Default::default() },
    ]
}

#[test]
fn a_change_of_the_people_takes_away_only_who_went_and_keeps_whoever_the_server_has_besides() {
    // Since the window read the meeting: Eve was invited on the server, and a room and somebody informed are there, whom
    // the window never shows.
    let mut theirs = copy();
    theirs["participants"].as_object_mut().unwrap().extend(
        json!({
            "eve": {"@type": "Participant", "calendarAddress": "mailto:eve@example.org", "roles": {"attendee": true}, "participationStatus": "needs-action"},
            "room": {"@type": "Participant", "name": "Room 4", "email": "room4@example.org", "kind": "location", "roles": {"chair": true}},
            "info": {"@type": "Participant", "email": "info@example.org", "roles": {"informational": true}},
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let with = |people: Vec<Attendee>, before: Option<Vec<Attendee>>| {
        let op = EventOp { invited_before: before, ..change(EventFields { attendees: Some(people), ..Default::default() }) };
        let (body, _) = patch(&op, Some(&theirs), Clock::Own(zurich()), &[ME.to_string()]).unwrap();
        let mut keys: Vec<String> = body["participants"].as_object().unwrap().keys().cloned().collect();
        keys.sort();
        (keys, body)
    };
    // Dan added here: everybody stays, Dan comes.
    let mut added = bob_and_carol();
    added.push(Attendee { address: Address::new("Dan", "dan@example.org"), ..Default::default() });
    let (keys, body) = with(added, Some(bob_and_carol()));
    assert_eq!(keys, ["ada", "bob", "carol", "eve", "info", "p1", "room"]);
    assert_eq!(body["participants"]["p1"]["email"], "dan@example.org");
    for kept in ["eve", "room", "info", "bob", "carol"] {
        assert_eq!(body["participants"][kept], theirs["participants"][kept], "{kept} as the server has them");
    }
    // Carol taken away here: she alone goes.
    let (keys, _) = with(vec![Attendee { address: Address::new("Bob", "bob@example.org"), ..Default::default() }], Some(bob_and_carol()));
    assert_eq!(keys, ["ada", "bob", "eve", "info", "room"]);
    // Not known who was invited before: nobody goes, and nobody's role changes; only who is not there comes.
    let only = vec![Attendee { address: Address::new("Bob", "bob@example.org"), optional: true, response: Response::None }];
    let (keys, body) = with(only, None);
    assert_eq!(keys, ["ada", "bob", "carol", "eve", "info", "room"]);
    assert_eq!(body["participants"]["bob"], theirs["participants"]["bob"]);
    // Somebody invited before and taken away on the server meanwhile is not brought back by another change.
    let mut without_carol = copy();
    without_carol["participants"].as_object_mut().unwrap().remove("carol");
    let op = EventOp {
        invited_before: Some(bob_and_carol()),
        ..change(EventFields {
            attendees: Some([bob_and_carol(), vec![Attendee { address: Address::new("Dan", "dan@example.org"), ..Default::default() }]].concat()),
            ..Default::default()
        })
    };
    let (body, _) = patch(&op, Some(&without_carol), Clock::Own(zurich()), &[ME.to_string()]).unwrap();
    let mut keys: Vec<&String> = body["participants"].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["ada", "bob", "p1"]);
}

#[test]
fn a_copy_with_more_people_than_are_read_is_told_apart() {
    let mut crowded = copy();
    let many: serde_json::Map<String, Value> =
        (0..=MAX_COPY_PEOPLE).map(|n| (format!("x{n}"), json!({"email": format!("guest{n}@example.org"), "roles": {"attendee": true}}))).collect();
    crowded.insert("participants".into(), Value::Object(many));
    assert!(too_many_people(&crowded) && !too_many_people(&copy()));
}

#[test]
fn an_occurrence_is_written_on_its_series_clock_with_no_zone() {
    // An occurrence of a series in New York, moved to 14:00 there, the calendar shown in Zurich.
    let ny = Zone::named("America/New_York").unwrap();
    let op = change(EventFields {
        time: Some(timed(utc(19, 18, 0), utc(19, 19, 0))),
        zones: Some(EventZones { start: "America/New_York".into(), end: String::new() }),
        title: Some("Call".into()),
        ..Default::default()
    });
    let (body, zone) = patch(&op, Some(&copy()), Clock::Series(Some(ny), zurich()), &[ME.to_string()]).unwrap();
    assert_eq!(body, json!({"title": "Call", "start": "2026-10-19T14:00:00", "duration": "PT1H", "showWithoutTime": false}));
    assert_eq!(zone, "America/New_York");
    // A series of no zone: on the clock of the calendar's.
    let (floating, _) = patch(&op, None, Clock::Series(None, zurich()), &[]).unwrap();
    assert_eq!(floating["start"], "2026-10-19T20:00:00");
}

#[test]
fn an_answer_is_the_servers_people_with_the_persons_own_answer() {
    let mut theirs = copy();
    theirs.insert("organizerCalendarAddress".into(), json!("mailto:bob@example.org"));
    let answered = answer(&theirs, Response::Declined, &["ADA@example.org".to_string()]).unwrap();
    let people = answered["participants"].as_object().unwrap();
    assert_eq!(people["ada"]["participationStatus"], "declined");
    assert_eq!((people["bob"].clone(), people["carol"].clone()), (theirs["participants"]["bob"].clone(), theirs["participants"]["carol"].clone()));
    assert_eq!(answered.as_object().unwrap().len(), 1, "nothing else");
    // Not among them, or no answer: none.
    assert_eq!(answer(&theirs, Response::Accepted, &["eve@example.org".to_string()]), None);
    assert_eq!(answer(&theirs, Response::None, &[ME.to_string()]), None);
    // Who organizes, whether others come, whether it is a draft.
    assert_eq!(meeting(&copy(), &[ME.to_string()]), (true, true, false));
    assert_eq!(meeting(&theirs, &[ME.to_string()]), (false, true, false));
    let mut alone = copy();
    alone.remove("participants");
    alone.insert("isDraft".into(), json!(true));
    assert_eq!(meeting(&alone, &[ME.to_string()]), (true, false, true));
}

#[test]
fn hostile_copies_never_panic() {
    let junk = [json!(null), json!(7), json!("x"), json!([1, 2]), json!({"l1": 3}), json!({"p": {"roles": "owner", "email": 5}})];
    for value in junk {
        let mut copy = Copy::new();
        for key in ["locations", "participants", "organizerCalendarAddress", "isDraft"] {
            copy.insert(key.into(), value.clone());
        }
        let fields = EventFields {
            location: Some("Room".into()),
            attendees: Some(vec![Attendee { address: Address::new("", "x@example.org"), ..Default::default() }]),
            ..Default::default()
        };
        let _ = patch(&change(fields), Some(&copy), Clock::Own(zurich()), &[ME.to_string()]);
        let _ = answer(&copy, Response::Accepted, &[ME.to_string()]);
        let _ = meeting(&copy, &[ME.to_string()]);
    }
}
