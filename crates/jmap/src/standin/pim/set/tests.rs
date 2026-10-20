// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The stand-in's `CalendarEvent/set`, held to what a server that keeps only the basics takes, as strictly.

use serde_json::{Value, json};

use super::super::super::{ACCOUNT, SESSION, StandIn, TOKEN};
use super::super::Calendar;
use super::*;
use crate::{CALENDARS, CORE, Http, Request, block_on};

const ME: &str = "ada@example.org";

/// Ada's calendar (the default, hers to write) and Grace's (hers to read), a meeting of Grace's Ada is invited to,
/// a weekly one of her own, and her lunch.
fn stand_in() -> StandIn {
    let stand_in = StandIn::new();
    stand_in.with(|d| {
        d.pim = Pim::offered();
        d.pim.own = vec![ME.into()];
        d.pim.add_calendar(Calendar { flagged: true, ..Calendar::new("ada", "Ada", true) });
        d.pim.add_calendar(Calendar::new("grace", "Grace", false));
        d.pim.add_event(
            Event::timed("planning", "ada", "Planning", "2026-10-20T10:00:00", Some("Europe/Zurich"), "PT1H")
                .with("organizerCalendarAddress", json!("mailto:grace@example.org"))
                .with(
                    "participants",
                    json!({
                        "grace": {"calendarAddress": "mailto:grace@example.org", "roles": {"owner": true}, "participationStatus": "accepted"},
                        "ada": {"calendarAddress": "mailto:ada@example.org", "roles": {"attendee": true}, "participationStatus": "needs-action", "expectReply": true},
                    }),
                ),
        );
        d.pim.add_event(
            Event::timed("weekly", "ada", "Site meeting", "2026-10-12T09:00:00", Some("Europe/Zurich"), "PT1H")
                .repeating(json!({"frequency": "weekly", "count": 3})),
        );
        d.pim.add_event(Event::timed("lunch", "ada", "Lunch", "2026-10-19T12:00:00", Some("Europe/Zurich"), "PT1H"));
    });
    stand_in
}

/// One `CalendarEvent/set`: its result, or the `type` of its error.
fn set(stand_in: &StandIn, args: Value) -> Result<Value, String> {
    let mut args = args;
    args["accountId"] = json!(ACCOUNT);
    let body = serde_json::to_vec(&json!({"using": [CORE, CALENDARS], "methodCalls": [["CalendarEvent/set", args, "c0"]]})).unwrap();
    let req =
        Request::post("https://jmap.standin.example/jmap/api", "application/json", body, 1 << 20).header("Authorization", &format!("Bearer {TOKEN}"));
    let resp = block_on(stand_in.send(req)).unwrap();
    let answer: Value = serde_json::from_slice(&resp.body).unwrap();
    let answer = answer["methodResponses"][0].clone();
    if answer[0] == "error" { Err(answer[1]["type"].as_str().unwrap_or_default().to_string()) } else { Ok(answer[1].clone()) }
}

fn new_event(extra: Value) -> Value {
    let mut event = json!({"title": "Dentist", "start": "2026-10-22T16:00:00", "timeZone": "Europe/Zurich", "duration": "PT45M"});
    event.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    event
}

fn kind(error: &Value) -> &str {
    error["type"].as_str().unwrap_or_default()
}

#[test]
fn a_set_is_held_to_what_the_server_writes() {
    let stand_in = stand_in();
    let _ = SESSION;
    let made = set(
        &stand_in,
        json!({"create": {
            "ok": new_event(json!({"uid": "mc-1"})),
            "slash": new_event(json!({"locations/l1/name": "Room"})),
            "keywords": new_event(json!({"keywords": {"Red": true}})),
            "occurrence": new_event(json!({"recurrenceId": "2026-10-22T16:00:00"})),
            "draft": new_event(json!({"isDraft": true})),
            "boss": new_event(json!({"organizerCalendarAddress": "mailto:boss@example.org"})),
            "short": {"title": "No time"},
            "elsewhere": new_event(json!({"calendarIds": {"Cbm9uZQ": true}})),
            "readonly": new_event(json!({"calendarIds": {Pim::calendar_id("grace"): true}})),
            "badzone": new_event(json!({"timeZone": "Mars/Olympus"})),
            "busy": new_event(json!({"freeBusyStatus": "tentative"})),
        }}),
    )
    .unwrap();
    assert_eq!(made["created"]["ok"]["uid"], "mc-1");
    let refused: Vec<(&str, &str)> = made["notCreated"].as_object().unwrap().iter().map(|(k, e)| (k.as_str(), kind(e))).collect();
    assert_eq!(
        refused,
        [
            ("badzone", "invalidProperties"),
            ("boss", "forbidden"),
            ("busy", "invalidProperties"),
            ("draft", "invalidProperties"),
            ("elsewhere", "invalidProperties"),
            ("keywords", "invalidProperties"),
            ("occurrence", "invalidProperties"),
            ("readonly", "forbidden"),
            ("short", "invalidProperties"),
            ("slash", "invalidProperties"),
        ]
    );
    assert_eq!(made["notCreated"]["keywords"]["description"], "Not written here: keywords");
    assert!(made["notCreated"]["slash"]["description"].as_str().unwrap().contains("whole property"));
    // What the server keeps for itself may come back as it was read.
    let echoed =
        set(&stand_in, json!({"update": {Pim::event_id("lunch"): {"title": "Lunch (late)", "utcStart": "x", "@type": "Event", "uid": "u"}}}))
            .unwrap();
    assert_eq!(echoed["updated"], json!({Pim::event_id("lunch"): null}));
    // The call as a whole.
    assert_eq!(set(&stand_in, json!({"ifInState": "0", "update": {}})), Err("invalidArguments".into()));
    assert_eq!(set(&stand_in, json!({"create": []})), Err("invalidArguments".into()));
    let many: Vec<String> = (0..501).map(|n| format!("V{n}")).collect();
    assert_eq!(set(&stand_in, json!({"destroy": many})), Err("requestTooLarge".into()));
}

#[test]
fn fifty_events_a_call_and_the_rest_are_asked_to_wait() {
    let stand_in = stand_in();
    let creates: serde_json::Map<String, Value> = (0..51).map(|n| (format!("k{n:02}"), new_event(json!({"uid": format!("n{n}")})))).collect();
    let made = set(&stand_in, json!({"create": creates})).unwrap();
    assert_eq!(made["created"].as_object().unwrap().len(), 50);
    let waits: Vec<&str> = made["notCreated"].as_object().unwrap().values().map(kind).collect();
    assert_eq!(waits, ["rateLimit"]);
}

#[test]
fn a_create_is_known_again_by_its_uid_and_goes_to_the_default_calendar() {
    let stand_in = stand_in();
    let first = set(&stand_in, json!({"create": {"a": new_event(json!({"uid": "mc-again"}))}})).unwrap();
    let again = set(&stand_in, json!({"create": {"b": new_event(json!({"uid": "mc-again"}))}})).unwrap();
    assert_eq!(first["created"]["a"]["id"], again["created"]["b"]["id"]);
    let made: Vec<Event> = stand_in.with(|d| d.pim.events.iter().filter(|e| e.props.get("uid") == Some(&json!("mc-again"))).cloned().collect());
    assert_eq!(made.len(), 1);
    assert_eq!(
        (made[0].calendars.as_slice(), made[0].props["organizerCalendarAddress"].as_str()),
        (["ada".to_string()].as_slice(), Some("mailto:ada@example.org"))
    );
}

#[test]
fn an_occurrence_is_changed_alone_and_takes_neither_zone_nor_calendar() {
    let stand_in = stand_in();
    let second = Pim::occurrence_id("weekly", "2026-10-19T09:00:00");
    let changed = set(
        &stand_in,
        json!({"update": {second.clone(): {"title": "Site meeting (room B)", "timeZone": "America/New_York", "start": "2026-10-19T11:00:00"}}}),
    )
    .unwrap();
    assert_eq!(changed["notUpdated"], json!({}));
    let weekly = stand_in.with(|d| d.pim.events.iter().find(|e| e.raw == "weekly").cloned()).unwrap();
    assert_eq!(weekly.zone.as_deref(), Some("Europe/Zurich"), "the series keeps its zone");
    assert_eq!(
        weekly.overrides["2026-10-19T09:00:00"],
        *json!({"title": "Site meeting (room B)", "start": "2026-10-19T11:00:00"}).as_object().unwrap()
    );
    // Deleted alone: the series leaves it out; then it is no more.
    let gone = set(&stand_in, json!({"destroy": [second.clone()]})).unwrap();
    assert_eq!(gone["destroyed"], json!([second]));
    let again = set(&stand_in, json!({"destroy": [second.clone()]})).unwrap();
    assert_eq!(kind(&again["notDestroyed"][&second]), "notFound");
    let weekly = stand_in.with(|d| d.pim.events.iter().find(|e| e.raw == "weekly").cloned()).unwrap();
    assert_eq!((weekly.excluded.as_slice(), weekly.overrides.len()), (["2026-10-19T09:00:00".to_string()].as_slice(), 0));
}

/// The people of Grace's meeting as a client sends them back, with Ada's answer `status`.
fn answered(stand_in: &StandIn, status: &str) -> Value {
    let mut people = stand_in.with(|d| d.pim.events.iter().find(|e| e.raw == "planning").unwrap().props["participants"].clone());
    people["ada"]["participationStatus"] = json!(status);
    // As a client spells them: another case, what the server works out again left out.
    people["ada"]["calendarAddress"] = json!("MAILTO:ADA@example.org");
    people["ada"].as_object_mut().unwrap().remove("expectReply");
    people
}

#[test]
fn an_answer_sent_is_kept_and_told_to_the_organizer_and_one_not_sent_is_kept_only() {
    let stand_in = stand_in();
    let planning = Pim::event_id("planning");
    let sent =
        set(&stand_in, json!({"update": {planning.clone(): {"participants": answered(&stand_in, "accepted")}}, "sendSchedulingMessages": true}))
            .unwrap();
    assert_eq!(sent["notUpdated"], json!({}));
    let told = stand_in.with(|d| d.pim.scheduled.clone());
    assert_eq!(told, [Scheduled { what: "reply", event: "planning".into(), occurrence: None, to: vec!["grace@example.org".into()] }]);
    let status = |stand_in: &StandIn| {
        stand_in.with(|d| d.pim.events.iter().find(|e| e.raw == "planning").unwrap().props["participants"]["ada"]["participationStatus"].clone())
    };
    assert_eq!(status(&stand_in), "accepted");
    // Not asked to be sent: written on the copy, nobody told.
    stand_in.with(|d| d.pim.scheduled.clear());
    set(&stand_in, json!({"update": {planning.clone(): {"participants": answered(&stand_in, "tentative")}}})).unwrap();
    assert_eq!((status(&stand_in), stand_in.with(|d| d.pim.scheduled.len())), (json!("tentative"), 0));
    // A server that doesn't tell organizers (before it did): kept, nobody told.
    stand_in.with(|d| d.pim.tells_the_organizer = false);
    set(&stand_in, json!({"update": {planning.clone(): {"participants": answered(&stand_in, "declined")}}, "sendSchedulingMessages": true})).unwrap();
    assert_eq!((status(&stand_in), stand_in.with(|d| d.pim.scheduled.len())), (json!("declined"), 0));
}

#[test]
fn one_occurrence_of_somebody_elses_invitation_is_not_answered_alone() {
    let stand_in = stand_in();
    // Grace's meeting repeats, weekly.
    stand_in.with(|d| {
        let planning = d.pim.events.iter_mut().find(|e| e.raw == "planning").unwrap();
        planning.rule = Some(json!({"frequency": "weekly", "count": 3}));
    });
    let second = Pim::occurrence_id("planning", "2026-10-27T10:00:00");
    let refused =
        set(&stand_in, json!({"update": {second.clone(): {"participants": answered(&stand_in, "declined")}}, "sendSchedulingMessages": true}))
            .unwrap();
    assert_eq!(kind(&refused["notUpdated"][&second]), "forbidden");
    assert!(refused["notUpdated"][&second]["description"].as_str().unwrap().contains("answered as a whole"));
    // Kept on the copy of that occurrence alone, when nobody is to be told.
    let kept = set(&stand_in, json!({"update": {second.clone(): {"participants": answered(&stand_in, "declined")}}})).unwrap();
    assert_eq!(kept["notUpdated"], json!({}));
    let planning = stand_in.with(|d| d.pim.events.iter().find(|e| e.raw == "planning").cloned()).unwrap();
    assert_eq!(planning.overrides["2026-10-27T10:00:00"]["participants"]["ada"]["participationStatus"], "declined");
    assert_eq!(planning.props["participants"]["ada"]["participationStatus"], "needs-action");
}

#[test]
fn the_organizer_and_the_draft_state_are_the_events_own() {
    let stand_in = stand_in();
    let lunch = Pim::event_id("lunch");
    let other = set(&stand_in, json!({"update": {lunch.clone(): {"organizerCalendarAddress": "mailto:boss@example.org"}}})).unwrap();
    assert_eq!(kind(&other["notUpdated"][&lunch]), "forbidden");
    let draft = set(&stand_in, json!({"update": {lunch.clone(): {"isDraft": true}}})).unwrap();
    assert_eq!(kind(&draft["notUpdated"][&lunch]), "invalidProperties");
    // A meeting of somebody else's in a calendar Ada may only read is not hers to change.
    stand_in.with(|d| d.pim.add_event(Event::timed("review", "grace", "Review", "2026-10-21T15:00:00", None, "PT1H")));
    let review = Pim::event_id("review");
    let refused = set(&stand_in, json!({"update": {review.clone(): {"title": "x"}}, "destroy": [review.clone()]})).unwrap();
    assert_eq!((kind(&refused["notUpdated"][&review]), kind(&refused["notDestroyed"][&review])), ("forbidden", "forbidden"));
}
