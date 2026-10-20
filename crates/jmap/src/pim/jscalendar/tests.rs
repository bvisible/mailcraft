// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JSCalendar event and a calendar into what the engine takes, and what hostile ones come to.

use mailcraft_model::remote::RemoteEventKind;
use mailcraft_model::{Address, Attendee, Response, Rgb, ShowAs};
use mailcraft_time::Zone;
use proptest::prelude::*;
use serde_json::{Value, json};

use super::*;

const ME: &str = "ada@example.org";

fn me() -> Vec<String> {
    vec![ME.to_string(), "ada.lovelace@example.org".to_string()]
}

/// An event of an hour on Monday 2026-10-19, 09:00 in Zurich (07:00 UTC), as a server that gives instants serves it.
fn event(change: impl FnOnce(&mut Value)) -> Value {
    let mut event = json!({
        "@type": "Event",
        "id": "V1",
        "uid": "budget-review-1",
        "calendarIds": {"C1": true},
        "title": "Budget review",
        "start": "2026-10-19T09:00:00",
        "timeZone": "Europe/Zurich",
        "duration": "PT1H",
        "showWithoutTime": false,
        "utcStart": "2026-10-19T07:00:00Z",
        "utcEnd": "2026-10-19T08:00:00Z",
        "status": "confirmed",
    });
    change(&mut event);
    event
}

fn read(value: &Value) -> Read {
    event_of(value, &me(), Some(Zone::named("Europe/Zurich").unwrap())).unwrap()
}

/// 2026-10-19 at `hour`:`minute` UTC.
fn utc(day: u8, hour: i64, minute: i64) -> i64 {
    mailcraft_time::DateTime::at_midnight(mailcraft_time::Date::new(2026, 10, day).unwrap()).to_timestamp(0) + hour * 3_600 + minute * 60
}

#[test]
fn a_timed_event_reads_from_its_utc_times_and_keeps_its_zone_name() {
    let got = read(&event(|_| {}));
    assert_eq!((got.event.start, got.event.end, got.event.all_day), (utc(19, 7, 0), utc(19, 8, 0), false));
    assert_eq!((got.event.id.as_str(), got.event.uid.as_str(), got.event.token.as_str()), ("V1", "budget-review-1", "budget-review-1"));
    assert_eq!((got.event.start_zone.as_str(), got.event.end_zone.as_str()), ("Europe/Zurich", ""));
    assert_eq!((got.event.title.as_str(), got.event.kind, got.calendars), ("Budget review", RemoteEventKind::Single, vec!["C1".to_string()]));
    // The server's instants are believed over the clock: they are what it worked out with its zone database.
    let skewed = read(&event(|e| e["utcStart"] = json!("2026-10-19T06:30:00Z")));
    assert_eq!(skewed.event.start, utc(19, 6, 30));
    // Without them, the clock of its zone and its length: the days on the clock, then the time that passes.
    let worked = read(&event(|e| {
        e.as_object_mut().unwrap().remove("utcStart");
        e.as_object_mut().unwrap().remove("utcEnd");
        e["start"] = json!("2026-10-24T09:00:00");
        e["duration"] = json!("P1DT1H");
    }));
    // Saturday 09:00 summer time is 07:00 UTC; a day later on the clock is Sunday 09:00 winter time, 08:00 UTC; and an hour.
    assert_eq!((worked.event.start, worked.event.end), (utc(24, 7, 0), utc(25, 9, 0)));
    // A zone the time crate doesn't know is no zone: the event is a floating one, at the server's instants.
    let unknown = read(&event(|e| e["timeZone"] = json!("Mars/Olympus_Mons")));
    assert_eq!((unknown.event.start, unknown.event.start_zone.as_str()), (utc(19, 7, 0), ""));
}

#[test]
fn a_whole_day_event_is_its_days_from_midnight_to_midnight() {
    let day = |change: fn(&mut Value)| {
        read(&event(|e| {
            e["showWithoutTime"] = json!(true);
            e["start"] = json!("2026-10-20T00:00:00");
            e["duration"] = json!("P1D");
            e.as_object_mut().unwrap().remove("utcStart");
            e.as_object_mut().unwrap().remove("utcEnd");
            change(e);
        }))
        .event
    };
    let one = day(|_| {});
    assert_eq!((one.start, one.end, one.all_day, one.start_zone.as_str()), (utc(20, 0, 0), utc(21, 0, 0), true, ""));
    // Several days, a week, a length that is not whole days, none, and a date without a time.
    assert_eq!(day(|e| e["duration"] = json!("P3D")).end, utc(23, 0, 0));
    assert_eq!(day(|e| e["duration"] = json!("P1W")).end, utc(27, 0, 0));
    assert_eq!(day(|e| e["duration"] = json!("P1DT2H")).end, utc(22, 0, 0));
    assert_eq!(day(|e| e["duration"] = json!("PT0S")).end, utc(21, 0, 0), "at least its day");
    assert_eq!(day(|e| e["duration"] = json!(null)).end, utc(21, 0, 0));
    assert_eq!(day(|e| e["start"] = json!("2026-10-20")).start, utc(20, 0, 0));
    // The instants a server adds to one are not its days, and are not read.
    let with_instants = day(|e| e["utcStart"] = json!("2026-10-19T22:00:00Z"));
    assert_eq!(with_instants.start, utc(20, 0, 0));
}

#[test]
fn a_floating_event_takes_the_servers_utc_times_and_has_no_zone() {
    let floating = read(&event(|e| e["timeZone"] = json!(null)));
    assert_eq!((floating.event.start, floating.event.end, floating.event.start_zone.as_str()), (utc(19, 7, 0), utc(19, 8, 0), ""));
    // Without instants: by the clock of the zone the calendar is shown in here.
    let bare = event(|e| {
        e["timeZone"] = json!(null);
        e.as_object_mut().unwrap().remove("utcStart");
        e.as_object_mut().unwrap().remove("utcEnd");
    });
    assert_eq!(read(&bare).event.start, utc(19, 7, 0), "09:00 in Zurich");
    let in_utc = event_of(&bare, &me(), None).unwrap();
    assert_eq!(in_utc.event.start, utc(19, 9, 0), "09:00 UTC when no zone is known here either");
}

fn participants(list: Value) -> Value {
    event(|e| e["participants"] = list)
}

#[test]
fn the_organizer_is_known_by_the_servers_order_of_addresses() {
    let organizer = |participant: Value| read(&participants(json!({"p1": participant}))).event.organizer;
    let owner = json!({"owner": true});
    let at = |email: &str| Some(Address { name: "Grace Hopper".into(), email: email.into() });
    let named = |extra: Value| {
        let mut p = json!({"name": "Grace Hopper", "roles": owner});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    assert_eq!(
        organizer(named(
            json!({"calendarAddress": "mailto:grace@example.org", "email": "g@example.org", "sendTo": {"imip": "mailto:i@example.org"}})
        )),
        at("grace@example.org")
    );
    assert_eq!(organizer(named(json!({"email": "g@example.org", "sendTo": {"imip": "mailto:i@example.org"}}))), at("g@example.org"));
    assert_eq!(organizer(named(json!({"sendTo": {"imip": "mailto:i@example.org"}, "scheduleId": "mailto:s@example.org"}))), at("i@example.org"));
    assert_eq!(organizer(named(json!({"scheduleId": "mailto:s@example.org"}))), at("s@example.org"));
    // `organizerCalendarAddress` says who it is, and the participant with that address names them.
    let said = read(&event(|e| {
        e["organizerCalendarAddress"] = json!("mailto:grace@example.org");
        e["participants"] = json!({"a": {"name": "Bob", "email": "bob@example.org", "roles": {"attendee": true}}, "b": {"name": "Grace Hopper", "calendarAddress": "mailto:grace@example.org"}});
    }));
    assert_eq!(said.event.organizer, at("grace@example.org"));
    // The organizer is not one of the people invited, even when the server lists them as one too.
    assert_eq!(said.event.attendees.iter().map(|a| a.address.email.as_str()).collect::<Vec<_>>(), ["bob@example.org"]);
    let both = read(&participants(
        json!({"o": {"email": "grace@example.org", "roles": {"owner": true, "attendee": true}}, "x": {"email": "bob@example.org"}}),
    ));
    assert_eq!(both.event.attendees.len(), 1);
}

#[test]
fn the_person_is_the_participant_with_one_of_their_addresses() {
    let invited = read(&event(|e| {
        e["organizerCalendarAddress"] = json!("mailto:grace@example.org");
        e["participants"] = json!({
            "g": {"email": "grace@example.org", "roles": {"owner": true}, "participationStatus": "accepted"},
            "me": {"email": "Ada.Lovelace@Example.org", "roles": {"attendee": true}, "participationStatus": "tentative"},
            "b": {"email": "bob@example.org", "roles": {"attendee": true, "optional": true}, "participationStatus": "declined"},
            "c": {"name": "Room 4", "roles": {"attendee": true}},
            "i": {"email": "info@example.org", "roles": {"informational": true}},
        });
    }));
    let e = &invited.event;
    assert_eq!((e.response, e.is_organizer), (Response::Tentative, Some(false)));
    assert_eq!(
        e.attendees,
        [
            Attendee { address: Address::new("", "bob@example.org"), optional: true, response: Response::Declined },
            Attendee { address: Address::new("Room 4", ""), optional: false, response: Response::None },
            Attendee { address: Address::new("", "Ada.Lovelace@Example.org"), optional: false, response: Response::Tentative },
        ],
        "the informational participant is no guest"
    );
    // The owner organizes it.
    let mine = read(&event(|e| {
        e["organizerCalendarAddress"] = json!("mailto:ada@example.org");
        e["participants"] =
            json!({"a": {"email": ME, "roles": {"owner": true}}, "b": {"email": "bob@example.org", "participationStatus": "needs-action"}});
    }));
    assert_eq!((mine.event.is_organizer, mine.event.attendees.len(), mine.event.attendees[0].response), (Some(true), 1, Response::None));
}

#[test]
fn is_organizer_is_none_when_nobody_organizes() {
    let alone = read(&event(|_| {}));
    assert_eq!((alone.event.is_organizer, alone.event.organizer.clone(), alone.event.attendees.len()), (None, None, 0));
    let guests_only = read(&participants(json!({"b": {"email": "bob@example.org", "roles": {"attendee": true}}})));
    assert_eq!((guests_only.event.is_organizer, guests_only.event.attendees.len()), (None, 1));
}

#[test]
fn free_busy_privacy_tentative_and_cancelled_are_read() {
    let shown = |change: fn(&mut Value)| read(&event(change)).event;
    assert_eq!(shown(|_| {}).show_as, ShowAs::Busy, "busy unless said");
    assert_eq!(shown(|e| e["freeBusyStatus"] = json!("free")).show_as, ShowAs::Free);
    assert_eq!(shown(|e| e["freeBusyStatus"] = json!("busy")).show_as, ShowAs::Busy);
    assert_eq!(shown(|e| e["status"] = json!("tentative")).show_as, ShowAs::Tentative);
    assert_eq!(shown(|e| e["freeBusyStatus"] = json!(7)).show_as, ShowAs::Busy);
    assert!(shown(|e| e["privacy"] = json!("private")).private && shown(|e| e["privacy"] = json!("secret")).private);
    assert!(!shown(|e| e["privacy"] = json!("public")).private);
    assert!(shown(|e| e["status"] = json!("cancelled")).cancelled && !shown(|_| {}).cancelled);
    // When it was changed is the version it is (and when).
    let updated = shown(|e| e["updated"] = json!("2026-10-18T10:00:00Z"));
    assert_eq!((updated.change_key.as_str(), updated.modified), ("2026-10-18T10:00:00Z", utc(18, 10, 0)));
    assert_eq!(shown(|e| e["sequence"] = json!(3)).change_key, "3");
}

#[test]
fn the_first_alert_before_the_start_is_the_reminder() {
    let alerted = |alerts: Value| read(&event(|e| e["alerts"] = alerts)).event.reminder_minutes;
    let offset = |offset: &str| json!({"@type": "Alert", "trigger": {"@type": "OffsetTrigger", "offset": offset}});
    assert_eq!(alerted(json!({"a": offset("-PT15M")})), Some(15));
    assert_eq!(alerted(json!({"a": offset("-P1D")})), Some(1_440));
    assert_eq!(alerted(json!({"a": offset("PT0S")})), Some(0), "at the start");
    // One after the start, an absolute one, one from the end: not a reminder.
    assert_eq!(alerted(json!({"a": offset("PT10M")})), None);
    assert_eq!(alerted(json!({"a": {"trigger": {"@type": "AbsoluteTrigger", "when": "2026-10-19T06:00:00Z"}}})), None);
    assert_eq!(alerted(json!({"a": {"trigger": {"offset": "-PT5M", "relativeTo": "end"}}})), None);
    // The first of those that are, by the order of their names; a trigger with no type is an offset.
    assert_eq!(alerted(json!({"a": offset("PT10M"), "b": {"trigger": {"offset": "-PT30M"}}, "c": offset("-PT5M")})), Some(30));
    assert_eq!(alerted(json!({"a": "soon", "b": {"trigger": "soon"}})), None);
    assert_eq!(alerted(json!(null)), None);
}

#[test]
fn a_location_is_the_first_one_with_a_name() {
    let placed = |locations: Value| read(&event(|e| e["locations"] = locations)).event.location;
    assert_eq!(placed(json!({"l1": {"@type": "Location", "name": "Room 4"}})), "Room 4");
    assert_eq!(placed(json!({"a": {"name": "  "}, "b": {"coordinates": "geo:0,0"}, "c": {"name": "Lakeside Hall"}})), "Lakeside Hall");
    assert_eq!(placed(json!({"a": {"name": 7}})), "");
    assert_eq!(placed(json!([{"name": "a list is no map"}])), "");
}

#[test]
fn an_occurrence_names_its_series_and_a_description_in_html_loses_its_tags() {
    let one = read(&event(|e| {
        e["id"] = json!("OWyJWMSIsIjIwMjYtMTAtMTlUMDk6MDA6MDAiXQ");
        e["recurrenceId"] = json!("2026-10-19T09:00:00");
        e["baseEventId"] = json!("V1");
    }));
    assert_eq!((one.event.kind, one.event.series.as_str()), (RemoteEventKind::Occurrence, "V1"));
    // Without a series that is one, it stands alone.
    let loose = read(&event(|e| {
        e["recurrenceId"] = json!("2026-10-19T09:00:00");
        e["baseEventId"] = json!("not an id");
    }));
    assert_eq!((loose.event.kind, loose.event.series.as_str()), (RemoteEventKind::Single, ""));
    let html = read(&event(|e| {
        e["description"] = json!("<p>Agenda:</p><ul><li>Budget &amp; plan</li></ul>");
        e["descriptionContentType"] = json!("text/html; charset=utf-8");
    }));
    assert_eq!(html.event.description, "Agenda: Budget & plan");
    let plain = read(&event(|e| e["description"] = json!("Keep <this> as it is")));
    assert_eq!(plain.event.description, "Keep <this> as it is");
}

#[test]
fn colours_read_as_rgb_and_names_and_junk_is_none() {
    assert_eq!(color("#2E8B57"), Some(Rgb(0x2E, 0x8B, 0x57)));
    assert_eq!(color("#2e8b57"), Some(Rgb(0x2E, 0x8B, 0x57)));
    assert_eq!(color("#f80"), Some(Rgb(0xFF, 0x88, 0x00)));
    assert_eq!(color(" Teal "), Some(Rgb(0x00, 0x80, 0x80)));
    for junk in ["", "#", "#12345", "#1234567", "#gggggg", "rgb(1,2,3)", "chartreuse-ish", "#é12", "\u{0}"] {
        assert_eq!(color(junk), None, "{junk:?}");
    }
    let listed = calendar_of(&json!({"id": "C1", "name": "Team", "color": "#ff0000", "isDefault": true, "myRights": {"mayWriteAll": true}})).unwrap();
    assert_eq!((listed.calendar.id.as_str(), listed.calendar.name.as_str(), listed.calendar.color), ("C1", "Team", Some(Rgb(0xFF, 0, 0))));
    assert!(listed.default && listed.writable);
    // Writing its own only is not writing it (the server writes where all may be written).
    let own = calendar_of(&json!({"id": "C2", "myRights": {"mayWriteOwn": true, "mayWriteAll": false}})).unwrap();
    assert!(!own.writable && !own.default && own.calendar.color.is_none());
    for bad in [json!({"name": "no id"}), json!({"id": ""}), json!({"id": "two words"}), json!("C1"), json!(null)] {
        assert_eq!(calendar_of(&bad), None, "{bad}");
    }
}

#[test]
fn an_event_without_an_id_a_start_or_a_readable_end_is_passed_over_and_counted() {
    let zone = Some(Zone::UTC);
    let skip = |change: fn(&mut Value)| event_of(&event(change), &me(), zone).err();
    assert_eq!(skip(|e| e["id"] = json!(null)), Some(Missing::Id));
    assert_eq!(skip(|e| e["id"] = json!("V 1")), Some(Missing::Id));
    assert_eq!(
        skip(|e| {
            e["utcStart"] = json!("soon");
            e["start"] = json!("soon");
        }),
        Some(Missing::Start)
    );
    assert_eq!(
        skip(|e| {
            e["utcEnd"] = json!(null);
            e["duration"] = json!("forever");
        }),
        Some(Missing::End)
    );
    assert_eq!(
        skip(|e| {
            e["showWithoutTime"] = json!(true);
            e["start"] = json!(20261020);
        }),
        Some(Missing::Start)
    );
    // An end before the start ends at the start.
    let backwards = event_of(&event(|e| e["utcEnd"] = json!("2026-10-19T06:00:00Z")), &me(), zone).unwrap();
    assert_eq!(backwards.event.end, backwards.event.start);
}

/// Junk of every shape for a value.
fn junk() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>().prop_map(|n| json!(n)),
        ".{0,40}".prop_map(Value::from),
        Just(json!("\u{202e}gnp.exe\u{0}\r\n")),
        Just(json!("-P99999999W")),
        Just(json!("Europe/Zurich/../../etc")),
    ];
    leaf.prop_recursive(4, 32, 6, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::from),
            proptest::collection::btree_map(".{0,12}", inner, 0..6).prop_map(|m| json!(m))
        ]
    })
}

/// The properties an event is read by.
const FIELDS: [&str; 21] = [
    "id",
    "uid",
    "title",
    "start",
    "timeZone",
    "duration",
    "showWithoutTime",
    "utcStart",
    "utcEnd",
    "participants",
    "locations",
    "alerts",
    "calendarIds",
    "organizerCalendarAddress",
    "status",
    "recurrenceId",
    "baseEventId",
    "description",
    "descriptionContentType",
    "updated",
    "sequence",
];

proptest! {
    #[test]
    fn hostile_events_never_panic(fields in proptest::collection::btree_map(proptest::sample::select(FIELDS.to_vec()), junk(), 0..12)) {
        let value = json!(fields);
        if let Ok(read) = event_of(&value, &me(), Some(Zone::UTC)) {
            prop_assert!(read.event.end >= read.event.start);
            prop_assert!(!read.event.title.chars().any(char::is_control));
            prop_assert!(read.event.attendees.len() <= 500);
        }
        let _ = calendar_of(&value);
    }
}

#[test]
fn hostile_events_of_every_size_never_panic() {
    let crowd: serde_json::Map<String, Value> =
        (0..5_000).map(|n| (format!("p{n:05}"), json!({"email": format!("p{n}@example.org"), "roles": {"attendee": true}}))).collect();
    let big = "é".repeat(1 << 20);
    let mut nested = json!("deep");
    for _ in 0..200 {
        nested = json!({"name": nested});
    }
    let hostile = event(|e| {
        e["participants"] = Value::Object(crowd);
        e["title"] = json!(big.clone());
        e["description"] = json!(format!("<{big}"));
        e["descriptionContentType"] = json!("text/html");
        e["locations"] = json!({"l": nested});
        e["timeZone"] = json!("\u{202e}Europe/Zurich\u{0}");
        e["duration"] = json!("P999999999W");
    });
    let read = read(&hostile);
    assert_eq!(read.event.attendees.len(), 500, "as many as an event keeps");
    assert!(read.event.title.chars().count() <= 500 && read.event.description.chars().count() <= 100_000);
    assert_eq!(read.event.start_zone, "");
    // A whole-day event that would last for ever lasts ten years.
    let long = read_day("P999999999W");
    assert_eq!(long.end - long.start, 3_660 * 86_400);
}

fn read_day(length: &str) -> mailcraft_model::remote::RemoteEvent {
    read(&event(|e| {
        e["showWithoutTime"] = json!(true);
        e["duration"] = json!(length);
    }))
    .event
}
