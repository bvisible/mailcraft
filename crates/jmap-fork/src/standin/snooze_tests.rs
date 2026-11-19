// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! « Plus tard » on the stand-in, as a browser test reads it: put away, refused in the past, one record for twice, back at
//! its time unread to where it came from, and called off when it is no longer in the folder.

use serde_json::{Map, Value, json};

use super::super::{Profile, StandIn};
use super::FOLDER;

fn args(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

fn put(stand_in: &StandIn, email: &str, until: &str) -> Value {
    stand_in.with(|d| d.snooze_method("NeomailSnooze/set", &args(json!({"create": {"k": {"emailId": email, "until": until}}})))).ok().unwrap()
}

fn folder_of(stand_in: &StandIn, email: &str) -> Option<String> {
    stand_in.with(|d| {
        let e = d.email(email)?;
        d.mailboxes.iter().find(|m| e.mailboxes.contains(&m.id)).map(|m| m.name.clone())
    })
}

#[test]
fn put_away_then_back_at_its_time_unread_to_where_it_came_from() {
    let stand_in = StandIn::new();
    let email = stand_in.with(|d| {
        d.snoozes.on = true;
        d.profile = Profile::by_place();
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        d.deliver(&inbox, b"From: a@example.org\r\nSubject: Plan\r\nMessage-ID: <Plan@example.org>\r\n\r\nHi\r\n", &["$seen"], "2026-11-19T08:00:00Z")
    });
    let past = put(&stand_in, &email, "2020-01-01T00:00:00Z");
    assert_eq!(past["notCreated"]["k"]["type"], "invalidProperties");
    let later = mailcraft_time::format_iso_utc(mailcraft_time::now() + 3_600);
    let made = put(&stand_in, &email, &later);
    let moved = made["created"]["k"]["emailId"].as_str().unwrap().to_string();
    assert_ne!(moved, email, "a server that names mail by where it lies");
    assert_eq!(folder_of(&stand_in, &moved).as_deref(), Some(FOLDER));
    // Twice: one record, the later time.
    let again = put(&stand_in, &moved, &mailcraft_time::format_iso_utc(mailcraft_time::now() + 7_200));
    assert_eq!(again["created"]["k"]["id"], made["created"]["k"]["id"]);
    assert_eq!(stand_in.with(|d| d.snoozes.records.len()), 1);
    let listed = stand_in.with(|d| d.snooze_method("NeomailSnooze/get", &Map::new())).ok().unwrap();
    assert_eq!(listed["list"].as_array().unwrap().len(), 1);
    assert!(listed["mailboxId"].is_string());
    // Not yet; then at its time.
    assert_eq!(stand_in.with(|d| d.snoozes_due(mailcraft_time::now())), 0);
    assert_eq!(stand_in.with(|d| d.snoozes_due(mailcraft_time::now() + 7_200)), 1);
    let back =
        stand_in.with(|d| d.emails.iter().find(|e| e.raw.starts_with(b"From: a@")).map(|e| (e.id.clone(), e.keywords.contains("$seen")))).unwrap();
    assert_eq!(folder_of(&stand_in, &back.0).as_deref(), Some("Inbox"));
    assert!(!back.1, "unread");
    assert_eq!(stand_in.with(|d| d.snoozes_due(mailcraft_time::now() + 9_000)), 0, "once");
}

#[test]
fn moved_out_by_hand_it_is_called_off_and_does_not_come_back() {
    let stand_in = StandIn::new();
    let (email, trash) = stand_in.with(|d| {
        d.snoozes.on = true;
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        let trash = d.mailbox("Trash", None, Some("trash"));
        (d.deliver(&inbox, b"From: a@example.org\r\nMessage-ID: <x@example.org>\r\n\r\nHi\r\n", &[], "2026-11-19T08:00:00Z"), trash)
    });
    let made = put(&stand_in, &email, &mailcraft_time::format_iso_utc(mailcraft_time::now() + 60));
    let moved = made["created"]["k"]["emailId"].as_str().unwrap().to_string();
    stand_in.with(|d| d.move_to(&moved, &trash));
    assert_eq!(stand_in.with(|d| d.snoozes_due(mailcraft_time::now() + 120)), 0);
    assert_eq!(stand_in.with(|d| d.snoozes.records[0].state.clone()), "calledOff");
    assert_eq!(folder_of(&stand_in, &moved).as_deref(), Some("Trash"));
}

#[test]
fn a_request_naming_the_capability_is_taken_when_it_is_on_and_refused_when_it_is_not() {
    use crate::{Http as _, Request, block_on};
    let stand_in = StandIn::new();
    let ask = |stand_in: &StandIn| {
        let body = json!({"using": ["urn:ietf:params:jmap:core", super::SNOOZE], "methodCalls": [["NeomailSnooze/get", {"accountId": "A1"}, "c0"]]});
        let url = format!("{}{}", super::super::ORIGIN, super::super::API_PATH);
        let request = Request::post(&url, "application/json", serde_json::to_vec(&body).unwrap(), 1 << 20)
            .header("Authorization", &format!("Bearer {}", super::super::TOKEN));
        block_on(stand_in.send(request)).map(|r| r.status).ok()
    };
    assert_eq!(ask(&stand_in), Some(400), "not offered: an unknown capability");
    stand_in.with(|d| d.snoozes.on = true);
    assert_eq!(ask(&stand_in), Some(200));
}
