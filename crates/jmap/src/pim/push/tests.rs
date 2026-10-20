// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What is changed here of a calendar, written to the in-process server as strictly as a server that keeps only the
//! basics takes it: what goes, in how many calls, what comes back, and what trouble comes to. Invented people at
//! `example.org`.

use std::sync::atomic::AtomicBool;

use mailcraft_model::remote::{
    CalendarAsk, EventChangeKind, EventFields, EventOp, EventOutcome, EventReminder, EventRepeat, EventResult, EventTime, EventWall, EventZones,
    EventsUpload,
};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use mailcraft_model::{Address, Freq, Recurrence, Response, ShowAs};
use mailcraft_time::{Date, DateTime};
use serde_json::{Value, json};

use super::*;
use crate::pim::sync_pim;
use crate::standin::{Calendar, Effect, Event, Fault, Pim, SESSION, StandIn, TOKEN};
use crate::{Auth, block_on};

const ME: &str = "ada@example.org";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const DAY: i64 = 86_400;

/// 2026-10-`day` at `hour`:`minute` UTC.
fn utc(day: u8, hour: i64, minute: i64) -> i64 {
    DateTime::at_midnight(Date::new(2026, 10, day).unwrap()).to_timestamp(0) + hour * 3_600 + minute * 60
}

fn person(name: &str, email: &str, roles: Value, status: &str) -> Value {
    json!({"@type": "Participant", "name": name, "calendarAddress": format!("mailto:{email}"), "roles": roles, "participationStatus": status})
}

/// A server where Ada may write her calendar (the default) and the team's, and only read Grace's: a meeting she
/// organizes with Bob and Carol, one of Grace's she is invited to, her lunch, and a weekly site meeting.
fn server() -> StandIn {
    let server = StandIn::new();
    server.with(|d| {
        let pim = &mut d.pim;
        *pim = Pim::offered();
        pim.own = vec![ME.into()];
        pim.add_calendar(Calendar { flagged: true, ..Calendar::new("ada", "Ada", true) });
        pim.add_calendar(Calendar::new("team", "Team", true));
        pim.add_calendar(Calendar::new("grace", "Grace", false));
        let zurich = Some("Europe/Zurich");
        pim.add_event(
            Event::timed("budget", "ada", "Budget review", "2026-10-19T09:00:00", zurich, "PT1H")
                .with("organizerCalendarAddress", json!(format!("mailto:{ME}")))
                .with("locations", json!({"l1": {"@type": "Location", "name": "Room 4"}}))
                .with(
                    "participants",
                    json!({
                        "ada": person("Ada", ME, json!({"owner": true, "attendee": true}), "accepted"),
                        "bob": person("Bob", "bob@example.org", json!({"attendee": true}), "accepted"),
                        "carol": person("Carol", "carol@example.org", json!({"attendee": true}), "needs-action"),
                    }),
                ),
        );
        pim.add_event(
            Event::timed("planning", "ada", "Quarterly planning", "2026-10-20T10:00:00", zurich, "PT2H")
                .with("organizerCalendarAddress", json!("mailto:grace@example.org"))
                .with(
                    "participants",
                    json!({
                        "grace": person("Grace", "grace@example.org", json!({"owner": true}), "accepted"),
                        "ada": person("Ada", ME, json!({"attendee": true}), "needs-action"),
                    }),
                ),
        );
        pim.add_event(Event::timed("lunch", "ada", "Lunch", "2026-10-19T12:00:00", zurich, "PT1H"));
        pim.add_event(Event::timed("design", "grace", "Design review", "2026-10-21T15:00:00", zurich, "PT1H"));
        pim.add_event(
            Event::timed("site", "ada", "Site meeting", "2026-10-12T09:00:00", zurich, "PT1H")
                .repeating(json!({"@type": "RecurrenceRule", "frequency": "weekly", "count": 6})),
        );
    });
    server
}

/// A job that writes `ops` and reads nothing.
fn job(ops: Vec<EventOp>) -> JmapPimRequest {
    JmapPimRequest {
        account: 7,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        me: vec![ME.into()],
        zone: "Europe/Zurich".into(),
        now: NOW,
        from: NOW - 7 * DAY,
        to: NOW + 14 * DAY,
        changes: Some(EventsUpload { account: 7, email: ME.into(), ops, zone: "Europe/Zurich".into() }),
        ..Default::default()
    }
}

fn run(server: &StandIn, request: &JmapPimRequest) -> JmapPimResult {
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    block_on(sync_pim(&client, request, &AtomicBool::new(false)))
}

fn outcomes(result: &JmapPimResult) -> Vec<(EventResult, String)> {
    result.uploaded.as_ref().unwrap().outcomes.iter().map(|o| (o.result, o.error.clone())).collect()
}

fn zones() -> Option<EventZones> {
    Some(EventZones { start: "Europe/Zurich".into(), end: "Europe/Zurich".into() })
}

/// A new appointment: an hour from `start` (UTC), with the mark `token`.
fn create(token: &str, title: &str, start: i64) -> EventOp {
    EventOp {
        kind: EventChangeKind::Create,
        event: 100,
        token: token.into(),
        fields: EventFields {
            title: Some(title.into()),
            location: Some(String::new()),
            description: Some(String::new()),
            time: Some(EventTime { start, end: start + 3_600, all_day: false }),
            show_as: Some(ShowAs::Busy),
            private: Some(false),
            reminder: Some(EventReminder { on: true, minutes: 15 }),
            zones: zones(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A change of the event the server calls `remote`.
fn update(remote: &str, fields: EventFields) -> EventOp {
    EventOp { kind: EventChangeKind::Update, event: 200, remote: remote.into(), fields, ..Default::default() }
}

fn title(title: &str) -> EventFields {
    EventFields { title: Some(title.into()), ..Default::default() }
}

/// The stand-in's event `raw`, as the store has it.
fn stored(server: &StandIn, raw: &str) -> Option<Event> {
    server.with(|d| d.pim.events.iter().find(|e| e.raw == raw).cloned())
}

#[test]
fn a_create_comes_back_done_with_its_v_id_and_uid() {
    let server = server();
    let result = run(&server, &job(vec![create("mc-1", "Dentist", utc(22, 14, 0))]));
    let done = &result.uploaded.as_ref().unwrap().outcomes[0];
    assert_eq!((done.result, done.uid.as_str(), done.zone.as_str()), (EventResult::Done, "mc-1", "Europe/Zurich"));
    assert!(done.remote.starts_with('V'), "{done:?}");
    let made = server.with(|d| d.pim.events.iter().find(|e| e.props.get("uid") == Some(&json!("mc-1"))).cloned()).unwrap();
    assert_eq!((made.start.as_str(), made.zone.as_deref(), made.duration.as_str()), ("2026-10-22T16:00:00", Some("Europe/Zurich"), "PT1H"));
    assert_eq!(
        (made.calendars.as_slice(), made.props["title"].as_str()),
        (["ada".to_string()].as_slice(), Some("Dentist")),
        "in the default calendar"
    );
    assert!(result.errors.is_empty() && !result.auth_refused, "{result:?}");
}

#[test]
fn a_new_event_in_another_calendar_goes_there_and_not_into_one_that_can_only_be_read() {
    let server = server();
    let team = EventOp { calendar: Pim::calendar_id("team"), ..create("mc-2", "Stand-up", utc(22, 8, 0)) };
    let grace = EventOp { calendar: Pim::calendar_id("grace"), ..create("mc-3", "Coffee", utc(22, 9, 0)) };
    let result = run(&server, &job(vec![team, grace]));
    let said = outcomes(&result);
    assert_eq!(said[0].0, EventResult::Done);
    assert_eq!(said[1], (EventResult::Refused, "Your rights do not allow it.".to_string()));
    let made = server.with(|d| d.pim.events.iter().find(|e| e.props.get("uid") == Some(&json!("mc-2"))).cloned()).unwrap();
    assert_eq!(made.calendars, ["team"]);
}

#[test]
fn a_whole_day_and_a_series_are_written_as_the_server_keeps_them() {
    let server = server();
    let day = EventOp {
        fields: EventFields {
            time: Some(EventTime { start: utc(23, 0, 0), end: utc(25, 0, 0), all_day: true }),
            zones: None,
            ..create("d", "Fair", 0).fields
        },
        ..create("mc-day", "Fair", 0)
    };
    let first = Date::new(2026, 10, 26).unwrap();
    let mut series = create("mc-weekly", "Yoga", utc(26, 17, 0));
    series.fields.repeat = Some(EventRepeat { rule: Some(Recurrence { count: Some(4), ..Recurrence::every(Freq::Weekly) }), first });
    series.fields.wall = Some(EventWall {
        start: DateTime { date: first, hour: 18, minute: 0, second: 0 },
        end: DateTime { date: first, hour: 19, minute: 0, second: 0 },
    });
    let result = run(&server, &job(vec![day, series]));
    assert!(outcomes(&result).iter().all(|(r, _)| *r == EventResult::Done), "{result:?}");
    let by_uid = |uid: &str| server.with(|d| d.pim.events.iter().find(|e| e.props.get("uid") == Some(&json!(uid))).cloned()).unwrap();
    let fair = by_uid("mc-day");
    assert_eq!((fair.all_day, fair.start.as_str(), fair.duration.as_str(), fair.zone.as_deref()), (true, "2026-10-23T00:00:00", "P2D", None));
    let yoga = by_uid("mc-weekly");
    assert_eq!((yoga.start.as_str(), yoga.zone.as_deref()), ("2026-10-26T18:00:00", Some("Europe/Zurich")));
    assert_eq!(yoga.rule.as_ref().unwrap()["count"], 4);
}

#[test]
fn a_change_carries_only_the_properties_that_changed_and_the_rest_stays() {
    let server = server();
    let result = run(&server, &job(vec![update(&Pim::event_id("lunch"), title("Lunch with Dan"))]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done);
    let lunch = stored(&server, "lunch").unwrap();
    assert_eq!(
        (lunch.props["title"].as_str(), lunch.start.as_str(), lunch.duration.as_str()),
        (Some("Lunch with Dan"), "2026-10-19T12:00:00", "PT1H")
    );
}

#[test]
fn a_guest_added_in_the_desk_meanwhile_is_kept_by_a_change_of_the_title_and_of_the_place() {
    let server = server();
    // Somebody adds Dan in the desk, after the window read the meeting.
    server.with(|d| {
        let budget = d.pim.events.iter_mut().find(|e| e.raw == "budget").unwrap();
        budget.props["participants"]["dan"] = person("Dan", "dan@example.org", json!({"attendee": true}), "needs-action");
    });
    let fields = EventFields { title: Some("Budget review (2027)".into()), location: Some("Room 5".into()), ..Default::default() };
    let result = run(&server, &job(vec![update(&Pim::event_id("budget"), fields)]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done);
    let budget = stored(&server, "budget").unwrap();
    assert!(budget.props["participants"].get("dan").is_some(), "Dan stays");
    assert_eq!(budget.props["locations"]["l1"]["name"], "Room 5");
}

#[test]
fn a_guest_added_here_leaves_one_added_on_the_server_meanwhile_where_it_is() {
    let server = server();
    // Eve is invited in the desk after the window read the meeting; then Ada invites Dan here, with her yes.
    server.with(|d| {
        let budget = d.pim.events.iter_mut().find(|e| e.raw == "budget").unwrap();
        budget.props["participants"]["eve"] = person("Eve", "eve@example.org", json!({"attendee": true}), "needs-action");
    });
    let invite = |name: &str, email: &str| mailcraft_model::Attendee { address: Address::new(name, email), ..Default::default() };
    let before = vec![invite("Bob", "bob@example.org"), invite("Carol", "carol@example.org")];
    let now = [before.clone(), vec![invite("Dan", "dan@example.org")]].concat();
    let op = EventOp {
        send: true,
        invited_before: Some(before),
        ..update(&Pim::event_id("budget"), EventFields { attendees: Some(now), ..Default::default() })
    };
    let result = run(&server, &job(vec![op]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done, "{result:?}");
    let people = stored(&server, "budget").unwrap().props["participants"].as_object().unwrap().clone();
    let mut addresses: Vec<String> = people.values().map(|p| p["calendarAddress"].as_str().or(p["email"].as_str()).unwrap().to_string()).collect();
    addresses.sort();
    assert_eq!(
        addresses,
        ["dan@example.org", "mailto:ada@example.org", "mailto:bob@example.org", "mailto:carol@example.org", "mailto:eve@example.org"]
    );
    // The update goes to everybody invited: Eve is told of a change, never of a cancellation.
    let told = server.with(|d| d.pim.scheduled.clone());
    assert_eq!(told.iter().map(|t| t.what).collect::<Vec<_>>(), ["update"]);
    assert!(told[0].to.contains(&"eve@example.org".to_string()), "{told:?}");
}

#[test]
fn a_change_of_the_people_of_an_event_with_more_than_are_read_is_refused() {
    let server = server();
    server.with(|d| {
        let budget = d.pim.events.iter_mut().find(|e| e.raw == "budget").unwrap();
        for n in 0..1_001 {
            budget.props["participants"][format!("g{n}")] =
                person("Guest", &format!("guest{n}@example.org"), json!({"attendee": true}), "needs-action");
        }
    });
    let attendees = vec![mailcraft_model::Attendee { address: Address::new("Dan", "dan@example.org"), ..Default::default() }];
    let op = EventOp { send: true, ..update(&Pim::event_id("budget"), EventFields { attendees: Some(attendees), ..Default::default() }) };
    let said = outcomes(&run(&server, &job(vec![op])));
    assert_eq!(said[0].0, EventResult::Refused);
    assert!(said[0].1.contains("more than 1,000 people"), "{said:?}");
    assert!(server.asked().iter().all(|a| a.method != "CalendarEvent/set"), "nothing written");
    // Its title is still changed.
    assert_eq!(outcomes(&run(&server, &job(vec![update(&Pim::event_id("budget"), title("All hands"))])))[0].0, EventResult::Done);
}

#[test]
fn scheduling_messages_only_for_the_organizer_of_a_meeting_and_in_calls_of_their_own() {
    let server = server();
    let ops = vec![
        // Ada's meeting, retitled, and she said to tell them: the people invited are told.
        EventOp { send: true, ..update(&Pim::event_id("budget"), title("Budget review (room 5)")) },
        // Her lunch, retitled: nobody to tell.
        update(&Pim::event_id("lunch"), title("Lunch (late)")),
        // Grace's meeting, a reminder of Ada's own: nobody told (she doesn't organize it).
        update(&Pim::event_id("planning"), EventFields { reminder: Some(EventReminder { on: true, minutes: 30 }), ..Default::default() }),
    ];
    let result = run(&server, &job(ops));
    assert!(outcomes(&result).iter().all(|(r, _)| *r == EventResult::Done), "{result:?}");
    let sets: Vec<_> = server.asked().into_iter().filter(|a| a.method == "CalendarEvent/set").map(|a| a.updates).collect();
    assert_eq!(sets, [Some(2), Some(1)], "the silent ones, then the one that writes to people");
    let told = server.with(|d| d.pim.scheduled.clone());
    assert_eq!(told.len(), 1);
    assert_eq!((told[0].what, told[0].event.as_str()), ("update", "budget"));
    assert_eq!(told[0].to, ["bob@example.org", "carol@example.org"]);
    // A change of how it shows in her own calendar tells nobody, even of her meeting.
    server.with(|d| d.pim.scheduled.clear());
    run(&server, &job(vec![update(&Pim::event_id("budget"), EventFields { show_as: Some(ShowAs::Free), ..Default::default() })]));
    assert!(server.with(|d| d.pim.scheduled.is_empty()));
    // Called off: a cancellation; deleted without a word (somebody else's copy, or a meeting called off without): none.
    let cancel =
        EventOp { kind: EventChangeKind::Delete, event: 200, remote: Pim::event_id("budget"), cancel: true, send: true, ..Default::default() };
    let quiet = EventOp { kind: EventChangeKind::Delete, event: 201, remote: Pim::event_id("planning"), ..Default::default() };
    let result = run(&server, &job(vec![cancel, quiet]));
    assert!(outcomes(&result).iter().all(|(r, _)| *r == EventResult::Done), "{result:?}");
    let told = server.with(|d| d.pim.scheduled.clone());
    assert_eq!(told.iter().map(|t| (t.what, t.event.as_str())).collect::<Vec<_>>(), [("cancellation", "budget")]);
    assert!(stored(&server, "budget").is_none() && stored(&server, "planning").is_none());
}

#[test]
fn nobody_is_written_to_without_the_persons_yes_whoever_the_servers_copy_invites() {
    let server = server();
    // Bob is invited to Ada's lunch from her phone, after the window read it: here it is no meeting, and nobody was asked.
    server.with(|d| {
        let lunch = d.pim.events.iter_mut().find(|e| e.raw == "lunch").unwrap();
        lunch.props.insert("organizerCalendarAddress".into(), json!(format!("mailto:{ME}")));
        lunch.props.insert(
            "participants".into(),
            json!({
                "ada": person("Ada", ME, json!({"owner": true, "attendee": true}), "accepted"),
                "bob": person("Bob", "bob@example.org", json!({"attendee": true}), "needs-action"),
            }),
        );
    });
    let moved =
        EventFields { time: Some(EventTime { start: utc(19, 11, 0), end: utc(19, 12, 0), all_day: false }), zones: zones(), ..Default::default() };
    let invited = |op: EventOp| EventOp {
        fields: EventFields {
            attendees: Some(vec![mailcraft_model::Attendee { address: Address::new("Bob", "bob@example.org"), ..Default::default() }]),
            ..op.fields
        },
        ..op
    };
    let ops = vec![
        // Moved here: the server's copy has Bob, and still nobody said to tell him.
        update(&Pim::event_id("lunch"), moved),
        // Her own meeting retitled, and a new one with Bob, neither with her yes (a mailbox file that says nothing).
        update(&Pim::event_id("budget"), title("Budget review (draft)")),
        invited(create("mc-quiet", "Coffee", utc(22, 9, 0))),
    ];
    let result = run(&server, &job(ops));
    assert!(outcomes(&result).iter().all(|(r, _)| *r == EventResult::Done), "{result:?}");
    assert!(server.with(|d| d.pim.scheduled.is_empty()), "nobody written to: {:?}", server.with(|d| d.pim.scheduled.clone()));
    assert_eq!(stored(&server, "lunch").unwrap().start, "2026-10-19T13:00:00", "the change itself went");
    let sets = server.asked().into_iter().filter(|a| a.method == "CalendarEvent/set").count();
    assert_eq!(sets, 1, "one quiet call");
    // With her yes, the same change of her meeting tells its people.
    run(&server, &job(vec![EventOp { send: true, ..update(&Pim::event_id("budget"), title("Budget review (final)")) }]));
    assert_eq!(server.with(|d| d.pim.scheduled.iter().map(|t| t.what).collect::<Vec<_>>()), ["update"]);
    // Called off by a change that says so and not that she said yes: deleted, and nobody told.
    let called_off = EventOp { kind: EventChangeKind::Delete, event: 200, remote: Pim::event_id("budget"), cancel: true, ..Default::default() };
    assert_eq!(outcomes(&run(&server, &job(vec![called_off])))[0].0, EventResult::Done);
    assert!(stored(&server, "budget").is_none());
    assert_eq!(server.with(|d| d.pim.scheduled.len()), 1, "no cancellation");
}

#[test]
fn a_change_that_writes_to_people_whose_answer_was_lost_is_not_written_and_mailed_twice() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("CalendarEvent/set", Effect::LoseAnswer)));
    let invite = |name: &str, email: &str| mailcraft_model::Attendee { address: Address::new(name, email), ..Default::default() };
    let before = vec![invite("Bob", "bob@example.org"), invite("Carol", "carol@example.org")];
    let fields = EventFields {
        title: Some("Budget review (final)".into()),
        location: Some("Room 5".into()),
        description: Some("Bring the figures.".into()),
        time: Some(EventTime { start: utc(19, 8, 0), end: utc(19, 9, 0), all_day: false }),
        zones: zones(),
        show_as: Some(ShowAs::Free),
        private: Some(true),
        reminder: Some(EventReminder { on: true, minutes: 30 }),
        attendees: Some([before.clone(), vec![invite("Dan", "dan@example.org")]].concat()),
        ..Default::default()
    };
    let op = EventOp { send: true, invited_before: Some(before), ..update(&Pim::event_id("budget"), fields) };
    // The server did it, and wrote to the people; the answer was lost on the way.
    let first = run(&server, &job(vec![op.clone()]));
    assert_eq!(outcomes(&first)[0].0, EventResult::Failed);
    assert_eq!(server.with(|d| d.pim.scheduled.len()), 1);
    // Sent again: the server's copy says it all already. Done, nothing written, nobody written to a second time.
    let before_again = server.asked().len();
    let again = run(&server, &job(vec![op]));
    let done = &again.uploaded.as_ref().unwrap().outcomes[0];
    assert_eq!((done.result, done.remote.as_str()), (EventResult::Done, Pim::event_id("budget").as_str()), "{done:?}");
    assert_eq!(server.with(|d| d.pim.scheduled.len()), 1);
    assert!(server.asked().into_iter().skip(before_again).all(|a| a.method != "CalendarEvent/set"));
    // A change the copy doesn't say yet still goes.
    let other = EventOp { send: true, ..update(&Pim::event_id("budget"), title("Budget review (final, really)")) };
    assert_eq!(outcomes(&run(&server, &job(vec![other])))[0].0, EventResult::Done);
    assert_eq!(server.with(|d| d.pim.scheduled.len()), 2);
}

#[test]
fn categories_importance_response_options_proposals_and_online_are_never_sent_and_an_op_with_nothing_else_is_refused() {
    let server = server();
    let only = EventFields {
        categories: Some(vec!["Red".into()]),
        importance: Some(mailcraft_model::Importance::High),
        request_responses: Some(false),
        allow_proposals: Some(false),
        online: Some(mailcraft_model::OnlineProvider::Teams),
        ..Default::default()
    };
    let with_title = EventFields { title: Some("Lunch (Teams)".into()), ..only.clone() };
    let forward = EventOp {
        kind: EventChangeKind::Forward,
        event: 9,
        remote: Pim::event_id("budget"),
        token: "f".into(),
        send: true,
        to: vec![Address::new("", "eve@example.org")],
        ..Default::default()
    };
    let result = run(&server, &job(vec![update(&Pim::event_id("budget"), only), update(&Pim::event_id("lunch"), with_title), forward]));
    let said = outcomes(&result);
    assert_eq!(said[0], (EventResult::Refused, write::KEEPS_NO.to_string()));
    assert_eq!(said[1].0, EventResult::Done, "the rest of the change goes");
    assert_eq!((said[2].0, said[2].1.contains("forwards no meetings")), (EventResult::Refused, true));
    assert_eq!(stored(&server, "lunch").unwrap().props["title"], "Lunch (Teams)");
}

#[test]
fn a_create_answered_already_exists_is_taken_as_that_event() {
    let server = server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let cancel = AtomicBool::new(false);
    let ops = vec![create("mc-1", "Dentist", utc(22, 14, 0)), update("V1", title("x")), update("V2", title("y")), update("V3", title("z"))];
    let mut writer = Writer {
        client: &client,
        account: "A1",
        cancel: &cancel,
        secrets: Secrets::default(),
        shown: Zone::UTC,
        me: vec![ME.into()],
        outcomes: vec![None; ops.len()],
        ops: ops.clone(),
        copies: HashMap::new(),
        gone: HashSet::new(),
        unread: HashMap::new(),
    };
    writer.refused(0, &ops[0], &json!({"type": "alreadyExists", "existingId": "Vmade"}), "Europe/Zurich");
    writer.refused(1, &ops[1], &json!({"type": "invalidProperties", "description": "<b>title</b> is   too long"}), "");
    writer.refused(2, &ops[2], &json!({"type": "rateLimit", "retryAfter": 120}), "");
    writer.refused(3, &ops[3], &json!({"type": "notFound"}), "");
    let said: Vec<EventOutcome> = writer.outcomes.into_iter().flatten().collect();
    assert_eq!((said[0].result, said[0].remote.as_str()), (EventResult::Done, "Vmade"));
    assert_eq!((said[1].result, said[1].error.as_str()), (EventResult::Refused, "title is too long"), "the server's words, without markup");
    assert_eq!((said[2].result, said[2].retry_after), (EventResult::Failed, Some(120)));
    assert_eq!(said[3].result, EventResult::Gone);
}

#[test]
fn not_found_of_an_update_or_a_destroy_is_gone() {
    let server = server();
    let gone = EventOp { kind: EventChangeKind::Delete, event: 3, remote: Pim::event_id("nobody"), ..Default::default() };
    let result = run(&server, &job(vec![update(&Pim::event_id("deleted"), title("x")), gone]));
    assert_eq!(outcomes(&result).iter().map(|o| o.0).collect::<Vec<_>>(), [EventResult::Gone, EventResult::Gone]);
    assert!(server.asked().iter().all(|a| a.method != "CalendarEvent/set"), "nothing to write");
}

#[test]
fn forbidden_and_invalid_properties_are_refused_with_the_facades_words_stripped_and_cut() {
    let server = server();
    server.with(|d| {
        d.pim.refused_events.insert(Pim::event_id("lunch"), "invalidProperties");
    });
    let result = run(&server, &job(vec![update(&Pim::event_id("design"), title("x")), update(&Pim::event_id("lunch"), title("y"))]));
    assert_eq!(
        outcomes(&result),
        [(EventResult::Refused, "Your rights do not allow it.".to_string()), (EventResult::Refused, "refused on demand".to_string())]
    );
    assert_eq!(stored(&server, "design").unwrap().props["title"], "Design review");
}

#[test]
fn rate_limit_server_fail_and_a_dead_transport_are_failed_with_a_retry() {
    let server = server();
    server.with(|d| {
        d.pim.refused_events.insert(Pim::event_id("lunch"), "rateLimit");
    });
    let result = run(&server, &job(vec![update(&Pim::event_id("lunch"), title("y"))]));
    assert_eq!((outcomes(&result)[0].0, result.uploaded.as_ref().unwrap().outcomes[0].retry_after), (EventResult::Failed, Some(60)));
    // The whole call failing.
    server.with(|d| d.faults.push_back(Fault::on("CalendarEvent/set", Effect::MethodError("serverFail"))));
    let result = run(&server, &job(vec![update(&Pim::event_id("budget"), title("y"))]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Failed);
    // A connection lost on the way: not known whether it went, so it goes again.
    server.with(|d| d.faults.push_back(Fault::on("CalendarEvent/set", Effect::LoseAnswer)));
    let result = run(&server, &job(vec![update(&Pim::event_id("budget"), title("z"))]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Failed);
    // A busy server: what was not written waits as long as it asked, and nothing is read.
    server.with(|d| {
        d.refuse_next.push_back(None);
        d.refuse_next.push_back(Some((503, Some(45))));
    });
    let reading = JmapPimRequest { asks: vec![CalendarAsk::default()], ..job(vec![update(&Pim::event_id("budget"), title("w"))]) };
    let result = run(&server, &reading);
    assert_eq!((outcomes(&result)[0].0, result.uploaded.as_ref().unwrap().outcomes[0].retry_after), (EventResult::Failed, Some(45)));
    assert_eq!((result.busy_for, result.events.is_none()), (Some(45), true));
}

#[test]
fn a_401_mid_job_ends_the_job_and_leaves_the_rest_unanswered() {
    let server = server();
    // The copies are read; then the server ends the session.
    server.with(|d| d.drop_token_after = Some(1));
    let result = run(&server, &job(vec![update(&Pim::event_id("lunch"), title("y")), create("mc-9", "Gym", utc(22, 6, 0))]));
    assert!(result.auth_refused && result.uploaded.as_ref().unwrap().token_refused, "{result:?}");
    assert!(result.uploaded.as_ref().unwrap().outcomes.is_empty(), "nothing answered: everything waits");
    assert_eq!(stored(&server, "lunch").unwrap().props["title"], "Lunch");
}

#[test]
fn a_create_whose_answer_was_lost_is_found_again_by_its_uid_and_not_made_twice() {
    let server = server();
    server.with(|d| d.lost_answers = 1);
    let op = create("mc-lost", "Physio", utc(23, 15, 0));
    let first = run(&server, &job(vec![op.clone()]));
    assert_eq!(outcomes(&first)[0].0, EventResult::Failed);
    let again = run(&server, &job(vec![op]));
    let done = &again.uploaded.as_ref().unwrap().outcomes[0];
    assert_eq!(done.result, EventResult::Done);
    let made: Vec<String> =
        server.with(|d| d.pim.events.iter().filter(|e| e.props.get("uid") == Some(&json!("mc-lost"))).map(|e| Pim::event_id(&e.raw)).collect());
    assert_eq!(made, std::slice::from_ref(&done.remote), "one event, the one the first made");
}

#[test]
fn at_most_fifty_events_a_call_and_the_rest_wait() {
    let server = server();
    let ops: Vec<EventOp> = (0..60).map(|n| create(&format!("mc-n{n}"), "Slot", utc(26, 6, 0) + n * 1_800)).collect();
    let result = run(&server, &job(ops));
    assert_eq!(result.uploaded.as_ref().unwrap().outcomes.len(), 50, "fifty tried");
    assert!(outcomes(&result).iter().all(|(r, _)| *r == EventResult::Done));
    let sets: Vec<_> = server.seen().into_iter().filter(|s| s.calls.iter().any(|c| c == "CalendarEvent/set")).collect();
    assert_eq!(sets.len(), 1);
    // A session that takes fewer in a set is written in more calls.
    server.with(|d| d.max_set = 20);
    let ops: Vec<EventOp> = (0..30).map(|n| create(&format!("mc-m{n}"), "Slot", utc(27, 6, 0) + n * 1_800)).collect();
    let before = server.seen().len();
    let result = run(&server, &job(ops));
    assert!(outcomes(&result).iter().all(|(r, _)| *r == EventResult::Done));
    let sets = server.seen().into_iter().skip(before).filter(|s| s.calls.iter().any(|c| c == "CalendarEvent/set")).count();
    assert_eq!(sets, 2);
}

#[test]
fn free_busy_privacy_and_a_reminder_are_written_and_read_back() {
    let server = server();
    let mut op = create("mc-rb", "Retreat", utc(22, 7, 0));
    (op.fields.show_as, op.fields.private, op.fields.reminder) = (Some(ShowAs::Free), Some(true), Some(EventReminder { on: true, minutes: 45 }));
    let reading = JmapPimRequest { asks: vec![CalendarAsk::default()], ..job(vec![op]) };
    let result = run(&server, &reading);
    assert_eq!(outcomes(&result)[0].0, EventResult::Done);
    // The same job reads after it wrote: the event is there as it was made.
    let read = result.events.unwrap().events.into_iter().find(|e| e.title == "Retreat").unwrap();
    assert_eq!((read.show_as, read.private, read.reminder_minutes, read.token.as_str()), (ShowAs::Free, true, Some(45), "mc-rb"));
    assert_eq!((read.start, read.end), (utc(22, 7, 0), utc(22, 8, 0)));
}

#[test]
fn a_reminder_set_here_leaves_the_other_alerts_of_the_server_as_they_are() {
    let server = server();
    let alerts = json!({
        "a1": {"@type": "Alert", "trigger": {"@type": "OffsetTrigger", "offset": "-PT15M", "relativeTo": "start"}, "action": "display"},
        "a2": {"@type": "Alert", "trigger": {"@type": "OffsetTrigger", "offset": "-P1D", "relativeTo": "start"}, "action": "email"},
        "a3": {"@type": "Alert", "trigger": {"@type": "OffsetTrigger", "offset": "-PT2H", "relativeTo": "start"}, "action": "display"},
    });
    server.with(|d| {
        let lunch = d.pim.events.iter_mut().find(|e| e.raw == "lunch").unwrap();
        lunch.props.insert("alerts".into(), alerts.clone());
    });
    let reminder =
        |on: bool, minutes: u32| update(&Pim::event_id("lunch"), EventFields { reminder: Some(EventReminder { on, minutes }), ..Default::default() });
    assert_eq!(outcomes(&run(&server, &job(vec![reminder(true, 45)])))[0].0, EventResult::Done);
    let kept = stored(&server, "lunch").unwrap().props["alerts"].clone();
    assert_eq!(kept["a1"]["trigger"]["offset"], "-PT45M");
    assert_eq!((kept["a2"].clone(), kept["a3"].clone()), (alerts["a2"].clone(), alerts["a3"].clone()));
    // Off: the reminder alone goes.
    assert_eq!(outcomes(&run(&server, &job(vec![reminder(false, 0)])))[0].0, EventResult::Done);
    let left = stored(&server, "lunch").unwrap().props["alerts"].clone();
    assert_eq!(left, json!({"a2": alerts["a2"], "a3": alerts["a3"]}));
}

#[test]
fn a_series_is_changed_by_its_master_and_keeps_its_zone_and_clock() {
    let server = server();
    // An hour later, on its clock: ten from the first Monday on.
    let first = Date::new(2026, 10, 12).unwrap();
    let fields = EventFields {
        time: Some(EventTime { start: utc(12, 8, 0), end: utc(12, 9, 0), all_day: false }),
        wall: Some(EventWall {
            start: DateTime { date: first, hour: 10, minute: 0, second: 0 },
            end: DateTime { date: first, hour: 11, minute: 0, second: 0 },
        }),
        zones: zones(),
        ..Default::default()
    };
    let result = run(&server, &job(vec![update(&Pim::event_id("site"), fields)]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done, "{result:?}");
    let site = stored(&server, "site").unwrap();
    assert_eq!((site.start.as_str(), site.zone.as_deref(), site.duration.as_str()), ("2026-10-12T10:00:00", Some("Europe/Zurich"), "PT1H"));
    assert!(site.rule.is_some(), "it still repeats");
    // Read again: ten either side of the night the clocks go back (08:00 UTC, then 09:00).
    let wide = JmapPimRequest { asks: vec![CalendarAsk::default()], changes: None, ..job(Vec::new()) };
    let read = run(&server, &wide);
    let mut starts: Vec<i64> = read.events.unwrap().events.iter().filter(|e| e.series == Pim::event_id("site")).map(|e| e.start).collect();
    starts.sort();
    assert_eq!(starts, [utc(12, 8, 0), utc(19, 8, 0), utc(26, 9, 0)]);
}

// One occurrence of a series, the series, and answers.

/// One occurrence of the weekly site meeting as the engine knows it: by its series and when the series has it (the
/// `day` of October at `hour` UTC).
fn of_site(kind: EventChangeKind, day: u8, hour: i64, fields: EventFields) -> EventOp {
    EventOp { kind, event: 300, series: Pim::event_id("site"), occurrence: utc(day, hour, 0), fields, ..Default::default() }
}

/// What the server keeps of the site meeting's occurrence of `at`.
fn overridden(server: &StandIn, at: &str) -> Option<serde_json::Map<String, Value>> {
    stored(server, "site").and_then(|site| site.overrides.get(at).cloned())
}

#[test]
fn one_occurrence_is_found_by_its_series_and_its_start_to_the_minute() {
    let server = server();
    // The 19th at nine in Zurich is 07:00 UTC.
    let result = run(&server, &job(vec![of_site(EventChangeKind::Update, 19, 7, title("Site meeting (room B)"))]));
    let done = &result.uploaded.as_ref().unwrap().outcomes[0];
    assert_eq!((done.result, done.remote.as_str()), (EventResult::Done, Pim::occurrence_id("site", "2026-10-19T09:00:00").as_str()), "{done:?}");
    assert_eq!(overridden(&server, "2026-10-19T09:00:00").unwrap()["title"], "Site meeting (room B)");
    // Looked for in the series' calendar, around that time, each occurrence apart.
    let query = server.asked().into_iter().find(|a| a.method == "CalendarEvent/query").unwrap();
    assert_eq!(query.limit, Some(200));
    assert_eq!(stored(&server, "site").unwrap().props["title"], "Site meeting", "the others stay as they are");
}

#[test]
fn no_occurrence_found_is_gone() {
    let server = server();
    let ops = vec![
        // A Tuesday the series doesn't have, and the Monday two minutes off.
        of_site(EventChangeKind::Update, 20, 7, title("x")),
        EventOp { occurrence: utc(19, 7, 2), ..of_site(EventChangeKind::Delete, 19, 7, EventFields::default()) },
        // A series the server doesn't have.
        EventOp { series: Pim::event_id("nobody"), ..of_site(EventChangeKind::Delete, 19, 7, EventFields::default()) },
    ];
    let result = run(&server, &job(ops));
    assert_eq!(outcomes(&result).iter().map(|o| o.0).collect::<Vec<_>>(), [EventResult::Gone; 3]);
    assert!(stored(&server, "site").unwrap().overrides.is_empty() && stored(&server, "site").unwrap().excluded.is_empty());
}

#[test]
fn an_occurrence_is_updated_by_its_own_id_with_neither_zone_nor_calendar() {
    let server = server();
    // Moved to eleven in Zurich, by an event that names its zone.
    let fields = EventFields {
        time: Some(EventTime { start: utc(19, 9, 0), end: utc(19, 10, 0), all_day: false }),
        zones: Some(EventZones { start: "America/New_York".into(), end: String::new() }),
        ..Default::default()
    };
    let result = run(&server, &job(vec![of_site(EventChangeKind::Update, 19, 7, fields)]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done);
    let written = server.with(|d| d.pim.written.clone());
    let (id, patch) = written.last().unwrap();
    assert_eq!(id, &Pim::occurrence_id("site", "2026-10-19T09:00:00"));
    assert_eq!(
        patch,
        &json!({"start": "2026-10-19T11:00:00", "duration": "PT1H", "showWithoutTime": false}),
        "on the series' clock, no zone, no calendar"
    );
    assert_eq!(overridden(&server, "2026-10-19T09:00:00").unwrap()["start"], "2026-10-19T11:00:00");
}

#[test]
fn an_occurrence_is_deleted_by_its_own_id_and_the_server_excludes_it() {
    let server = server();
    // The 26th, winter time: nine in Zurich is 08:00 UTC.
    let result = run(&server, &job(vec![of_site(EventChangeKind::Delete, 26, 8, EventFields::default())]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done);
    assert_eq!(stored(&server, "site").unwrap().excluded, ["2026-10-26T09:00:00"]);
}

#[test]
fn the_whole_series_is_deleted_by_its_master() {
    let server = server();
    let all = EventOp { kind: EventChangeKind::Delete, event: 300, remote: Pim::event_id("site"), ..Default::default() };
    let result = run(&server, &job(vec![all]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done);
    assert!(stored(&server, "site").is_none());
}

#[test]
fn a_second_occurrence_is_still_found_after_the_first_was_changed() {
    let server = server();
    run(&server, &job(vec![of_site(EventChangeKind::Update, 19, 7, title("Site meeting (room B)"))]));
    let result = run(&server, &job(vec![of_site(EventChangeKind::Update, 26, 8, title("Site meeting (room C)"))]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done);
    assert_eq!(overridden(&server, "2026-10-19T09:00:00").unwrap()["title"], "Site meeting (room B)");
    assert_eq!(overridden(&server, "2026-10-26T09:00:00").unwrap()["title"], "Site meeting (room C)");
}

#[test]
fn one_occurrence_is_looked_for_in_the_minutes_around_it_and_a_crowded_answer_is_tried_again_never_gone() {
    let server = server();
    run(&server, &job(vec![of_site(EventChangeKind::Update, 19, 7, title("Site meeting (room B)"))]));
    let query = server.asked().into_iter().find(|a| a.method == "CalendarEvent/query").unwrap();
    let minute = |seconds: i64| crate::pim::utc_text(utc(19, 7, 0) + seconds);
    assert_eq!((query.after, query.before, query.total), (Some(minute(-60)), Some(minute(61)), true), "the minute either side, and the total");
    // A day so busy that the answer can't hold everything around that time: the change waits, and nothing is taken for gone.
    server.with(|d| {
        for n in 0..250 {
            d.pim.add_event(Event::timed(&format!("busy{n}"), "ada", "Busy", "2026-10-26T08:59:30", Some("Europe/Zurich"), "PT1H"));
        }
    });
    let result = run(&server, &job(vec![of_site(EventChangeKind::Update, 26, 8, title("Site meeting (room C)"))]));
    let said = &result.uploaded.as_ref().unwrap().outcomes[0];
    assert_eq!((said.result, said.retry_after), (EventResult::Failed, Some(60)), "{said:?}");
    assert!(overridden(&server, "2026-10-26T09:00:00").is_none(), "nothing written");
}

#[test]
fn an_event_that_says_it_is_an_occurrence_and_names_no_series_is_not_written() {
    let server = server();
    server.with(|d| {
        d.pim.add_event(
            Event::timed("loose", "ada", "Loose", "2026-10-21T12:00:00", Some("Europe/Zurich"), "PT1H")
                .with("recurrenceId", json!("2026-10-21T12:00:00")),
        );
        d.pim.add_event(
            Event::timed("odd", "ada", "Odd", "2026-10-22T12:00:00", Some("Europe/Zurich"), "PT1H")
                .with("recurrenceId", json!("2026-10-22T12:00:00"))
                .with("baseEventId", json!("not an id!")),
        );
    });
    let result = run(&server, &job(vec![update(&Pim::event_id("loose"), title("x")), update(&Pim::event_id("odd"), title("y"))]));
    for (result, why) in outcomes(&result) {
        assert_eq!(result, EventResult::Refused);
        assert!(why.contains("occurrence of a series"), "{why}");
    }
    assert!(server.asked().iter().all(|a| a.method != "CalendarEvent/set"));
    assert_eq!(stored(&server, "loose").unwrap().props["title"], "Loose");
}

#[test]
fn an_occurrence_is_never_the_series_itself_nor_one_another_series_names() {
    let zurich = Zone::named("Europe/Zurich").unwrap();
    let series = Pim::event_id("site");
    // When the series has it: the 19th at nine in Zurich.
    let at = utc(19, 7, 0);
    let candidate = |id: &str, base: &str, recurrence: Value| json!({"id": id, "baseEventId": base, "recurrenceId": recurrence, "utcStart": crate::pim::utc_text(at), "start": "2026-10-19T09:00:00"});
    let own = Pim::occurrence_id("site", "2026-10-19T09:00:00");
    let pick = |found: Vec<Value>| pick_occurrence(found, &series, at, zurich, false).map(|(id, _)| id);
    let wrong = vec![
        // The series itself, as a server that lists it among what it expands might.
        candidate(&series, &series, Value::Null),
        candidate(&series, &series, json!("2026-10-19T09:00:00")),
        // No recurrence id: no occurrence.
        candidate("Vother", &series, Value::Null),
        // An id that names another series, or another time than its own.
        candidate(&Pim::occurrence_id("plan", "2026-10-19T09:00:00"), &series, json!("2026-10-19T09:00:00")),
        candidate(&Pim::occurrence_id("site", "2026-10-12T09:00:00"), &series, json!("2026-10-19T09:00:00")),
        // Another series' occurrence.
        candidate(&Pim::occurrence_id("plan", "2026-10-19T09:00:00"), &Pim::event_id("plan"), json!("2026-10-19T09:00:00")),
    ];
    assert_eq!(pick(wrong.clone()), None);
    let mut all = wrong;
    all.push(candidate(&own, &series, json!("2026-10-19T09:00:00")));
    assert_eq!(pick(all), Some(own.clone()));
    // An occurrence a server names by an id of its own (not by its series and its start) is one too.
    assert_eq!(pick(vec![candidate("Vocc19", &series, json!("2026-10-19T09:00:00"))]), Some("Vocc19".to_string()));
}

/// Ada's answer `response` to Grace's planning meeting, told to Grace when `send`.
fn respond(remote: &str, response: Response, send: bool) -> EventOp {
    EventOp { kind: EventChangeKind::Respond, event: 400, remote: remote.into(), response, send, ..Default::default() }
}

fn status_of_ada(server: &StandIn) -> Value {
    stored(server, "planning").unwrap().props["participants"]["ada"]["participationStatus"].clone()
}

#[test]
fn an_answer_is_the_servers_participants_map_with_the_persons_status_set() {
    let server = server();
    let result = run(&server, &job(vec![respond(&Pim::event_id("planning"), Response::Accepted, false)]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done);
    let (id, patch) = server.with(|d| d.pim.written.last().cloned()).unwrap();
    assert_eq!(id, Pim::event_id("planning"));
    assert_eq!(patch.as_object().unwrap().keys().collect::<Vec<_>>(), ["participants"], "nothing else");
    assert_eq!(patch["participants"]["grace"], stored(&server, "planning").unwrap().props["participants"]["grace"]);
    assert_eq!(status_of_ada(&server), "accepted");
    // Somebody's meeting Ada is not invited to has nobody to answer for.
    let lunch = run(&server, &job(vec![respond(&Pim::event_id("lunch"), Response::Accepted, false)]));
    assert_eq!(outcomes(&lunch)[0], (EventResult::Refused, "you are not among the people invited to it".to_string()));
}

#[test]
fn an_answer_with_send_asks_the_server_to_tell_the_organizer_and_one_without_does_not() {
    let server = server();
    run(&server, &job(vec![respond(&Pim::event_id("planning"), Response::Tentative, false)]));
    assert!(server.with(|d| d.pim.scheduled.is_empty()), "kept on her copy, nobody told");
    assert_eq!(status_of_ada(&server), "tentative");
    run(&server, &job(vec![respond(&Pim::event_id("planning"), Response::Declined, true)]));
    let told = server.with(|d| d.pim.scheduled.clone());
    assert_eq!(told.iter().map(|t| (t.what, t.to.clone())).collect::<Vec<_>>(), [("reply", vec!["grace@example.org".to_string()])]);
    assert_eq!(status_of_ada(&server), "declined");
}

#[test]
fn an_answer_to_one_occurrence_is_that_occurrence_alone_and_to_the_series_is_the_master() {
    let server = server();
    // Grace's planning repeats, Tuesdays at ten.
    server.with(|d| d.pim.events.iter_mut().find(|e| e.raw == "planning").unwrap().rule = Some(json!({"frequency": "weekly", "count": 3})));
    let one = |response: Response, send: bool| EventOp {
        kind: EventChangeKind::Respond,
        event: 400,
        series: Pim::event_id("planning"),
        occurrence: utc(27, 9, 0),
        response,
        send,
        ..Default::default()
    };
    // Kept on her copy of that Tuesday alone.
    let kept = run(&server, &job(vec![one(Response::Declined, false)]));
    assert_eq!(outcomes(&kept)[0].0, EventResult::Done);
    let planning = stored(&server, "planning").unwrap();
    assert_eq!(planning.overrides["2026-10-27T10:00:00"]["participants"]["ada"]["participationStatus"], "declined");
    assert_eq!(status_of_ada(&server), "needs-action", "the series as it was");
    // Told to Grace: such a server answers an invitation as a whole, and says so.
    let told = run(&server, &job(vec![one(Response::Tentative, true)]));
    let said = outcomes(&told);
    assert_eq!(said[0].0, EventResult::Refused);
    assert!(said[0].1.contains("answered as a whole"), "{said:?}");
    // The series: the master, and Grace is told.
    let series = run(&server, &job(vec![respond(&Pim::event_id("planning"), Response::Accepted, true)]));
    assert_eq!(outcomes(&series)[0].0, EventResult::Done);
    assert_eq!(status_of_ada(&server), "accepted");
    assert_eq!(server.with(|d| d.pim.scheduled.len()), 1);
}

#[test]
fn a_proposal_sent_with_an_answer_and_a_forward_are_refused_with_their_reason() {
    let server = server();
    let proposing = EventOp {
        proposed: Some(EventTime { start: utc(20, 12, 0), end: utc(20, 13, 0), all_day: false }),
        ..respond(&Pim::event_id("planning"), Response::Tentative, true)
    };
    let forward = EventOp {
        kind: EventChangeKind::Forward,
        event: 9,
        remote: Pim::event_id("planning"),
        token: "f".into(),
        send: true,
        to: vec![Address::new("", "eve@example.org")],
        ..Default::default()
    };
    let result = run(&server, &job(vec![proposing, forward]));
    let said = outcomes(&result);
    assert_eq!((said[0].0, said[0].1.contains("another time")), (EventResult::Refused, true), "{said:?}");
    assert_eq!((said[1].0, said[1].1.contains("forwards no meetings")), (EventResult::Refused, true));
    assert_eq!(status_of_ada(&server), "needs-action", "nothing written");
    assert!(server.asked().iter().all(|a| a.method != "CalendarEvent/set"));
}

#[test]
fn a_write_to_an_event_of_a_series_is_never_sent_before_the_master_was_read() {
    let server = server();
    // An occurrence the window holds as an event of its own, by its own id: its series is read before it is written.
    let own = Pim::occurrence_id("site", "2026-10-19T09:00:00");
    let result = run(&server, &job(vec![update(&own, title("Site meeting (room B)"))]));
    assert_eq!(outcomes(&result)[0].0, EventResult::Done);
    let methods: Vec<(String, Option<usize>)> = server.asked().into_iter().map(|a| (a.method, a.ids)).collect();
    assert_eq!(
        methods,
        [("CalendarEvent/get".to_string(), Some(1)), ("CalendarEvent/get".to_string(), Some(1)), ("CalendarEvent/set".to_string(), None)],
        "the occurrence, its series, then the write"
    );
    // A series that can't be read just now: nothing is written, and the change waits.
    server.with(|d| {
        d.pim.broken_events.insert(Pim::event_id("site"));
    });
    let before = server.asked().len();
    let waits = run(&server, &job(vec![update(&own, title("Site meeting (room C)"))]));
    assert_eq!(outcomes(&waits)[0].0, EventResult::Failed);
    assert!(server.asked().into_iter().skip(before).all(|a| a.method != "CalendarEvent/set"));
    // One occurrence known by its time: its series is read first too, and one the server doesn't have is gone.
    server.with(|d| d.pim.broken_events.clear());
    server.with(|d| d.pim.events.retain(|e| e.raw != "site"));
    let gone = run(&server, &job(vec![of_site(EventChangeKind::Delete, 26, 8, EventFields::default()), update(&own, title("x"))]));
    assert_eq!(outcomes(&gone).iter().map(|o| o.0).collect::<Vec<_>>(), [EventResult::Gone, EventResult::Gone]);
}
