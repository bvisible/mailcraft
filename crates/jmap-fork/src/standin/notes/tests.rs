// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The stand-in's notes, held to what the hosting page's server takes and serves. Invented people at `example.org`.

use proptest::prelude::*;
use serde_json::{Value, json};

use super::super::{ACCOUNT, SESSION, StandIn, TOKEN};
use super::*;
use crate::{CORE, Http, MAIL, Request, block_on};

/// Method calls as the API has them, and the HTTP status and body of the answer.
fn raw(stand_in: &StandIn, using: Value, calls: Value) -> (u16, Value) {
    let body = serde_json::to_vec(&json!({"using": using, "methodCalls": calls})).unwrap();
    let req =
        Request::post("https://jmap.standin.example/jmap/api", "application/json", body, 1 << 20).header("Authorization", &format!("Bearer {TOKEN}"));
    let resp = block_on(stand_in.send(req)).unwrap();
    (resp.status, serde_json::from_slice(&resp.body).unwrap_or(Value::Null))
}

/// What one call of notes answered: its result, or the `type` of its error.
fn one(stand_in: &StandIn, name: &str, args: Value) -> Result<Value, String> {
    let (status, body) = raw(stand_in, json!([CORE, NOTES]), json!([[name, args, "c0"]]));
    assert_eq!(status, 200, "{body}");
    let answer = body["methodResponses"][0].clone();
    if answer[0] == "error" { Err(answer[1]["type"].as_str().unwrap_or_default().to_string()) } else { Ok(answer[1].clone()) }
}

fn session(stand_in: &StandIn) -> Value {
    let req = Request::get(SESSION, 1 << 20).header("Authorization", &format!("Bearer {TOKEN}"));
    serde_json::from_slice(&block_on(stand_in.send(req)).unwrap().body).unwrap()
}

/// A stand-in whose person has: "Offsite" pinned, "Groceries", a rich "Plan", "Codes" Grace shares for reading,
/// "Shared plan" Bob shares for writing, an archived "Old", and Carol's "Private", which the person doesn't see.
fn fixtures() -> StandIn {
    let stand_in = StandIn::new();
    stand_in.with(|d| {
        let notes = &mut d.notes;
        *notes = Notes::offered();
        notes.add(StoredNote::new("VN-1", "Offsite", "Lake walk").pinned());
        notes.add(StoredNote::new("VN-2", "Groceries", "Milk\nBread"));
        notes.add(StoredNote::new("VN-3", "Plan", "Goals").rich());
        notes.add(StoredNote::new("VN-4", "Codes", "4711").shared_by("grace@example.org", "Grace Okafor", false));
        notes.add(StoredNote::new("VN-5", "Shared plan", "Steps").shared_by("bob@example.org", "Bob Lindqvist", true));
        notes.add(StoredNote::new("VN-6", "Old", "Done").archived());
        notes.add(StoredNote::new("VN-7", "Private", "Mine").of_someone_else("carol@example.org", "Carol Duval"));
    });
    stand_in
}

fn note(stand_in: &StandIn, id: &str) -> Value {
    one(stand_in, "Note/get", json!({"accountId": ACCOUNT, "ids": [id]})).unwrap()["list"][0].clone()
}

fn set(stand_in: &StandIn, args: Value) -> Value {
    let mut args = args;
    args["accountId"] = json!(ACCOUNT);
    one(stand_in, "Note/set", args).unwrap()
}

#[test]
fn the_session_offers_notes_with_their_limits_when_turned_on() {
    let stand_in = StandIn::new();
    assert!(session(&stand_in)["capabilities"].get(NOTES).is_none(), "a stand-in for mail stays one");
    let (status, body) = raw(&stand_in, json!([CORE, MAIL, NOTES]), json!([["Mailbox/get", {"accountId": ACCOUNT}, "c0"]]));
    assert_eq!((status, body["type"].as_str()), (400, Some("urn:ietf:params:jmap:error:unknownCapability")));
    stand_in.with(|d| d.notes = Notes::offered());
    let said = session(&stand_in);
    assert_eq!(said["capabilities"][NOTES], json!({}));
    assert_eq!(said["primaryAccounts"][NOTES], ACCOUNT);
    assert_eq!(
        said["accounts"][ACCOUNT]["accountCapabilities"][NOTES],
        json!({"maxSizeTitle": 140, "maxSizeText": 100_000, "writableProperties": ["isArchived", "isPinned", "text", "title"]})
    );
    // A method of notes needs the capability in `using`.
    let (_, body) = raw(&stand_in, json!([CORE, MAIL]), json!([["Note/query", {"accountId": ACCOUNT}, "c0"]]));
    assert_eq!(body["methodResponses"][0][1]["type"], "unknownMethod");
    // Changes are not kept.
    for name in ["Note/changes", "Note/queryChanges"] {
        assert_eq!(one(&stand_in, name, json!({"accountId": ACCOUNT, "sinceState": "x"})), Err("cannotCalculateChanges".into()));
    }
}

#[test]
fn query_takes_only_is_archived_and_orders_pinned_then_latest() {
    let stand_in = fixtures();
    let ids = |args: Value| -> Vec<String> {
        let mut args = args;
        args["accountId"] = json!(ACCOUNT);
        serde_json::from_value(one(&stand_in, "Note/query", args).unwrap()["ids"].clone()).unwrap()
    };
    // Pinned first, then the latest changed: the notes were added in order, one after the other.
    assert_eq!(ids(json!({})), ["VN-1", "VN-6", "VN-5", "VN-4", "VN-3", "VN-2"]);
    assert_eq!(ids(json!({"filter": {"isArchived": false}})), ["VN-1", "VN-5", "VN-4", "VN-3", "VN-2"]);
    assert_eq!(ids(json!({"filter": {"isArchived": true}})), ["VN-6"]);
    stand_in.with(|d| d.notes.change("VN-2", |note| note.text = "Milk".into()));
    assert_eq!(ids(json!({"filter": {"isArchived": false}, "limit": 2})), ["VN-1", "VN-2"]);
    assert_eq!(ids(json!({"position": -2})), ["VN-4", "VN-3"]);
    let answer = one(&stand_in, "Note/query", json!({"accountId": ACCOUNT, "calculateTotal": true})).unwrap();
    assert_eq!((answer["total"].as_u64(), answer["limit"].as_u64(), answer["canCalculateChanges"].as_bool()), (Some(6), Some(5_000), Some(false)));
    assert_eq!(answer["queryState"].as_str().map(str::len), Some(16));
    for (args, refused) in [
        (json!({"filter": {"text": "milk"}}), "unsupportedFilter"),
        (json!({"filter": {"isArchived": "yes"}}), "invalidArguments"),
        (json!({"sort": [{"property": "updated"}]}), "unsupportedSort"),
        (json!({"anchor": "VN-1"}), "invalidArguments"),
        (json!({"limit": -1}), "invalidArguments"),
        (json!({"accountId": "A2"}), "accountNotFound"),
    ] {
        let mut args = args;
        args.as_object_mut().unwrap().entry("accountId").or_insert(json!(ACCOUNT));
        assert_eq!(one(&stand_in, "Note/query", args.clone()), Err(refused.to_string()), "{args}");
    }
    // The state changes with a note, and with a share.
    let state = |s: &StandIn| s.with(|d| d.notes.state());
    let before = state(&stand_in);
    stand_in.with(|d| d.notes.list.iter_mut().filter(|note| note.id == "VN-4").for_each(|note| note.share = Some(true)));
    assert_ne!(state(&stand_in), before);
    stand_in.with(|d| d.notes.says_state = false);
    assert!(one(&stand_in, "Note/query", json!({"accountId": ACCOUNT})).unwrap().get("queryState").is_none());
}

#[test]
fn get_reads_what_the_person_sees_and_says_not_found_otherwise() {
    let stand_in = fixtures();
    let answer = one(&stand_in, "Note/get", json!({"accountId": ACCOUNT, "ids": ["VN-4", "VN-7", "nothing"]})).unwrap();
    assert_eq!(answer["notFound"], json!(["VN-7", "nothing"]));
    let codes = &answer["list"][0];
    assert_eq!(codes["title"], "Codes");
    assert_eq!((codes["isShared"].as_bool(), codes["ownerName"].as_str()), (Some(true), Some("Grace Okafor")));
    assert_eq!(codes["myRights"], json!({"mayWrite": false, "mayWriteText": false, "mayDelete": false}));
    assert_eq!(codes["webUrl"], "https://jmap.standin.example/app/notes?note=VN-4");
    assert!(codes["updated"].as_str().is_some_and(|v| v.ends_with('Z') && v.contains('.')), "{codes}");
    assert!(codes["created"].as_str().is_some_and(|v| v.ends_with('Z') && !v.contains('.')), "{codes}");
    let plan = note(&stand_in, "VN-3");
    assert_eq!(
        (plan["isRich"].as_bool(), plan["myRights"]["mayWriteText"].as_bool(), plan["myRights"]["mayDelete"].as_bool()),
        (Some(true), Some(false), Some(true))
    );
    assert_eq!(note(&stand_in, "VN-5")["myRights"], json!({"mayWrite": true, "mayWriteText": true, "mayDelete": false}));
    // All of them, while a get takes them; with the properties named, and the id.
    let all = one(&stand_in, "Note/get", json!({"accountId": ACCOUNT, "ids": null, "properties": ["title", "colour"]})).unwrap();
    assert_eq!(all["list"].as_array().map(Vec::len), Some(6));
    assert_eq!(all["list"][0], json!({"id": "VN-1", "title": "Offsite"}));
    stand_in.with(|d| d.max_get = 3);
    assert_eq!(one(&stand_in, "Note/get", json!({"accountId": ACCOUNT})), Err("requestTooLarge".into()));
    assert_eq!(one(&stand_in, "Note/get", json!({"accountId": ACCOUNT, "ids": ["a", "b", "c", "d"]})), Err("requestTooLarge".into()));
    // A note that breaks the get breaks it all.
    stand_in.with(|d| d.notes.broken.insert("VN-2".into()));
    assert_eq!(one(&stand_in, "Note/get", json!({"accountId": ACCOUNT, "ids": ["VN-1", "VN-2"]})), Err("serverFail".into()));
    assert!(one(&stand_in, "Note/get", json!({"accountId": ACCOUNT, "ids": ["VN-1"]})).is_ok());
}

#[test]
fn set_is_as_strict_as_the_facade() {
    let stand_in = fixtures();
    let refusal = |answer: &Value, side: &str, key: &str| answer[side][key]["type"].as_str().unwrap_or_default().to_string();
    let answer = set(
        &stand_in,
        json!({"create": {
            "a": {"title": "Two\nlines"},
            "b": {"title": "t".repeat(141)},
            "c": {"text": "x".repeat(100_001)},
            "d": {"title": "Fine", "colour": "red"},
            "e": {"isPinned": "yes"},
            "f": {"title": "Kept", "uid": "u-1", "@type": "Note", "isRich": true, "updated": "2020-01-01T00:00:00Z"},
            "g": {"text": "\n  First line  \nSecond"},
        }}),
    );
    for key in ["a", "b", "c", "d", "e"] {
        assert_eq!(refusal(&answer, "notCreated", key), "invalidProperties", "{key}: {answer}");
    }
    let kept = &answer["created"]["f"];
    assert_eq!((kept["title"].as_str(), kept["isRich"].as_bool(), kept["myRights"]["mayDelete"].as_bool()), (Some("Kept"), Some(false), Some(true)));
    assert_eq!(answer["created"]["g"]["title"], "First line", "named by its first line");
    assert_ne!(answer["oldState"], answer["newState"]);
    // A patch of a part, ifInState, too many, the wrong shape.
    let answer = set(&stand_in, json!({"update": {"VN-2": {"title/x": "y"}, "VN-1": {"isArchived": 1}}}));
    assert_eq!(
        (refusal(&answer, "notUpdated", "VN-2"), refusal(&answer, "notUpdated", "VN-1")),
        ("invalidProperties".into(), "invalidProperties".into())
    );
    let fails = |args: Value| {
        let mut args = args;
        args["accountId"] = json!(ACCOUNT);
        one(&stand_in, "Note/set", args).err()
    };
    assert_eq!(fails(json!({"ifInState": "abc", "destroy": []})), Some("invalidArguments".into()));
    assert_eq!(fails(json!({"create": []})), Some("invalidArguments".into()));
    stand_in.with(|d| d.max_set = 2);
    assert_eq!(fails(json!({"destroy": ["VN-1", "VN-2", "VN-6"]})), Some("requestTooLarge".into()));
    // Unknown notes are not found; what a test lines up is refused so.
    stand_in.with(|d| {
        d.max_set = 500;
        d.notes.refused.insert("VN-2".into(), "serverFail");
    });
    let answer = set(&stand_in, json!({"update": {"VN-7": {"title": "x"}, "VN-2": {"title": "x"}}, "destroy": ["VN-99"]}));
    assert_eq!(refusal(&answer, "notUpdated", "VN-7"), "notFound");
    assert_eq!(refusal(&answer, "notUpdated", "VN-2"), "serverFail");
    assert_eq!(refusal(&answer, "notDestroyed", "VN-99"), "notFound");
    // What it was sent is kept, in order.
    let written = stand_in.with(|d| d.notes.written.iter().map(|(what, _)| what.clone()).collect::<Vec<_>>());
    assert_eq!(written.last().map(String::as_str), Some("destroy VN-99"));
}

#[test]
fn a_stale_version_is_a_state_mismatch_and_a_pin_alone_never_is() {
    let stand_in = fixtures();
    let read = note(&stand_in, "VN-2")["updated"].as_str().unwrap().to_string();
    // Changed elsewhere since it was read.
    stand_in.with(|d| d.notes.change("VN-2", |note| note.text = "Milk\nBread\nEggs".into()));
    let answer = set(&stand_in, json!({"update": {"VN-2": {"text": "Milk", "updated": read}}}));
    assert_eq!(answer["notUpdated"]["VN-2"]["type"], "stateMismatch");
    assert_eq!(stand_in.with(|d| d.notes.note("VN-2").map(|note| note.text.clone())), Some("Milk\nBread\nEggs".into()));
    // A pin alone gives no version and goes.
    let answer = set(&stand_in, json!({"update": {"VN-2": {"isPinned": true}}}));
    assert_eq!(answer["updated"]["VN-2"].get("isPinned"), None, "pinned is not echoed");
    assert!(stand_in.with(|d| d.notes.note("VN-2").is_some_and(|note| note.pinned)));
    // From the version it now has, the words go; the answer is the new version.
    let now = note(&stand_in, "VN-2")["updated"].as_str().unwrap().to_string();
    let answer = set(&stand_in, json!({"update": {"VN-2": {"text": "Milk", "updated": now}}}));
    let said = &answer["updated"]["VN-2"];
    assert_eq!(said["text"], "Milk");
    assert_ne!(said["updated"].as_str(), Some(now.as_str()));
    assert_eq!(said["updated"], note(&stand_in, "VN-2")["updated"]);
    // A version that isn't a date.
    assert_eq!(
        set(&stand_in, json!({"update": {"VN-2": {"title": "x", "updated": "yesterday"}}}))["notUpdated"]["VN-2"]["type"],
        "invalidProperties"
    );
    // A whole note sent back as it is changes nothing.
    let whole = note(&stand_in, "VN-1");
    let before = stand_in.with(|d| d.notes.state());
    assert!(set(&stand_in, json!({"update": {"VN-1": whole}}))["updated"].get("VN-1").is_some());
    assert_eq!(stand_in.with(|d| d.notes.state()), before);
}

#[test]
fn only_the_owner_destroys() {
    let stand_in = fixtures();
    let answer = set(&stand_in, json!({"destroy": ["VN-4", "VN-5", "VN-2"]}));
    assert_eq!(answer["destroyed"], json!(["VN-2"]));
    assert_eq!(
        (answer["notDestroyed"]["VN-4"]["type"].as_str(), answer["notDestroyed"]["VN-5"]["type"].as_str()),
        (Some("forbidden"), Some("forbidden"))
    );
    // One shared for writing is changed; one shared for reading is not.
    let answer = set(&stand_in, json!({"update": {"VN-5": {"text": "Together"}, "VN-4": {"isPinned": true}}}));
    assert!(answer["updated"].get("VN-5").is_some());
    assert_eq!(answer["notUpdated"]["VN-4"]["type"], "forbidden");
}

#[test]
fn a_rich_notes_words_are_forbidden_its_title_is_not() {
    let stand_in = fixtures();
    let answer = set(&stand_in, json!({"update": {"VN-3": {"text": "Plain now"}}}));
    assert_eq!(answer["notUpdated"]["VN-3"]["type"], "forbidden");
    let answer = set(&stand_in, json!({"update": {"VN-3": {"title": "Plan (draft)", "text": "Goals"}}}));
    assert_eq!(answer["updated"]["VN-3"]["title"], "Plan (draft)", "its words as they are are no change");
    assert!(stand_in.with(|d| d.notes.note("VN-3").is_some_and(|note| note.rich && note.text == "Goals")));
}

#[test]
fn a_create_with_a_known_uid_is_the_same_note() {
    let stand_in = fixtures();
    let first = set(&stand_in, json!({"create": {"k": {"title": "Lunch", "uid": "mark-1"}}}))["created"]["k"].clone();
    let again = set(&stand_in, json!({"create": {"k2": {"title": "Lunch again", "uid": "mark-1"}}}))["created"]["k2"].clone();
    assert_eq!(first["id"], again["id"]);
    assert_eq!(again["title"], "Lunch", "nothing more is written");
    assert_eq!(stand_in.with(|d| d.notes.list.iter().filter(|note| note.title.starts_with("Lunch")).count()), 1);
    // Once deleted, or a week on, the mark makes a new one.
    stand_in.with(|d| d.notes.clock += 8 * 86_400 * 1_000);
    let later = set(&stand_in, json!({"create": {"k3": {"title": "Lunch", "uid": "mark-1"}}}))["created"]["k3"].clone();
    assert_ne!(later["id"], first["id"]);
    // A mark a test lines up is refused so.
    stand_in.with(|d| d.notes.refused.insert("mark-2".into(), "overQuota"));
    assert_eq!(set(&stand_in, json!({"create": {"k": {"title": "x", "uid": "mark-2"}}}))["notCreated"]["k"]["type"], "overQuota");
}

#[test]
fn a_version_has_its_milliseconds_and_reads_back() {
    assert_eq!(version(1_790_000_001_250), "2026-09-21T14:13:21.250Z");
    assert_eq!(version(1_790_000_001_000), "2026-09-21T14:13:21Z");
    for millis in [1_790_000_001_250, 1_790_000_001_000, 0, 1_790_000_001_007] {
        assert_eq!(moment(&version(millis)), Some(millis));
    }
    assert_eq!(moment("2026-09-21T14:13:21.25Z"), Some(1_790_000_001_250));
    for bad in ["", "2026-09-21T14:13:21", "2026-09-21T14:13:21.Z", "2026-09-21T14:13:21.x5Z", "yesterday"] {
        assert_eq!(moment(bad), None, "{bad}");
    }
}

proptest! {
    #[test]
    fn whatever_a_method_of_notes_is_sent_it_answers_and_never_panics(
        name in proptest::sample::select(vec!["Note/get", "Note/query", "Note/set", "Note/changes", "Note/queryChanges"]),
        args in crate::tests::any_json(),
        patch in crate::tests::any_json(),
    ) {
        let stand_in = fixtures();
        let mut shaped = json!({"accountId": ACCOUNT, "create": {"k": patch.clone()}, "update": {"VN-1": patch.clone(), "VN-3": patch}});
        for call in [args, shaped.take()] {
            let (status, body) = raw(&stand_in, json!([CORE, NOTES]), json!([[name, call, "c0"]]));
            prop_assert_eq!(status, 200);
            prop_assert_eq!(body["methodResponses"].as_array().map(Vec::len), Some(1));
        }
    }
}
