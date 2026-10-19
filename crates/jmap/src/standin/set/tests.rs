// //// Neoffice — added file (no upstream equivalent)
//! `Email/set` of the in-process server, held to the standard: the patch grammar, what a patch may make of an
//! email, the answer of each object, and the two kinds of server.

use proptest::prelude::*;
use serde_json::{Value, json};

use super::super::{ACCOUNT, SESSION, StandIn, TOKEN};
use super::*;
use crate::{Auth, Client, Error, block_on};

fn mail(subject: &str) -> Vec<u8> {
    format!("From: Zoe Park <zoe@example.org>\r\nTo: ada@example.org\r\nSubject: {subject}\r\nMessage-ID: <{subject}@example.org>\r\n\r\nHello.\r\n")
        .into_bytes()
}

struct Ids {
    inbox: String,
    archive: String,
    first: String,
    second: String,
    third: String,
}

/// An Inbox of three emails (the first read, the third flagged) and an empty Archive.
fn filled(profile: Profile) -> (StandIn, Ids) {
    let stand_in = StandIn::new();
    let ids = stand_in.with(|d| {
        d.profile = profile;
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        let archive = d.mailbox("Archive", None, Some("archive"));
        let first = d.deliver(&inbox, &mail("first"), &["$seen"], "2026-10-14T09:00:00Z");
        let second = d.deliver(&inbox, &mail("second"), &[], "2026-10-15T06:00:00Z");
        let third = d.deliver(&inbox, &mail("third"), &["$flagged"], "2026-10-15T07:30:00Z");
        Ids { inbox, archive, first, second, third }
    });
    (stand_in, ids)
}

/// The one `Email/set` call of a request: its answer, or the type of the error it ended in.
fn set(stand_in: &StandIn, args: Value) -> Result<Value, String> {
    let client = block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let mut args = args;
    if let Some(members) = args.as_object_mut() {
        members.entry("accountId").or_insert(json!(ACCOUNT));
    }
    let answers = block_on(client.mail(vec![("Email/set", args)])).unwrap();
    match answers.get(0) {
        Ok(answer) => Ok(answer.clone()),
        Err(Error::Refused { kind, .. }) => Err(kind),
        Err(other) => panic!("{other}"),
    }
}

fn keywords(stand_in: &StandIn, id: &str) -> Vec<String> {
    stand_in.with(|d| d.email(id).map(|e| e.keywords.iter().cloned().collect()).unwrap_or_default())
}

fn mailboxes(stand_in: &StandIn, id: &str) -> Vec<String> {
    stand_in.with(|d| d.email(id).map(|e| e.mailboxes.iter().cloned().collect()).unwrap_or_default())
}

/// What one update came to: the type of its SetError, or `ok`.
fn update_one(stand_in: &StandIn, id: &str, patch: Value) -> String {
    let answer = set(stand_in, json!({"update": {id: patch}})).unwrap();
    match answer["notUpdated"][id]["type"].as_str() {
        Some(kind) => kind.to_string(),
        None => {
            assert!(answer["updated"].as_object().is_some_and(|updated| updated.contains_key(id)), "{answer}");
            "ok".to_string()
        }
    }
}

#[test]
fn keywords_are_set_and_taken_away_one_at_a_time_and_nothing_else_is_touched() {
    let (stand_in, ids) = filled(Profile::standard());
    stand_in.with(|d| assert!(d.set_keyword(&ids.second, "$custom", true)));
    let before = stand_in.with(|d| d.state());
    let answer = set(
        &stand_in,
        json!({"update": {&ids.second: {"keywords/$seen": true, "keywords/$flagged": true}, &ids.third: {"keywords/$flagged": null}}}),
    )
    .unwrap();
    assert_eq!(answer["updated"], json!({&ids.second: null, &ids.third: null}));
    // What is empty is null, as the standard has it; the state moved on.
    assert_eq!(
        (&answer["notUpdated"], &answer["destroyed"], &answer["notDestroyed"], &answer["created"]),
        (&Value::Null, &Value::Null, &Value::Null, &Value::Null)
    );
    assert_eq!((answer["oldState"].as_str(), answer["newState"].as_str() != Some(before.as_str())), (Some(before.as_str()), true));
    assert_eq!(keywords(&stand_in, &ids.second), ["$custom", "$flagged", "$seen"], "the one it was not told of stays");
    assert!(keywords(&stand_in, &ids.third).is_empty());
    assert_eq!(keywords(&stand_in, &ids.first), ["$seen"], "an email not named is not touched");
    // Taking away what is not there, and a keyword in any case, change nothing and break nothing.
    assert_eq!(update_one(&stand_in, &ids.first, json!({"keywords/$answered": null, "keywords/$SEEN": null})), "ok");
    assert!(keywords(&stand_in, &ids.first).is_empty(), "keywords are not told apart by case");
    assert_eq!(update_one(&stand_in, &ids.first, json!({"keywords/$Forwarded": true})), "ok");
    assert_eq!(keywords(&stand_in, &ids.first), ["$forwarded"], "and are kept in lower case");
    // A patch that changes nothing leaves the state where it was.
    let state = stand_in.with(|d| d.state());
    assert_eq!(update_one(&stand_in, &ids.first, json!({})), "ok");
    assert_eq!(update_one(&stand_in, &ids.first, json!({"keywords/$forwarded": true})), "ok");
    assert_eq!(stand_in.with(|d| d.state()), state);
}

#[test]
fn a_move_puts_the_email_in_the_new_mailbox_and_takes_it_out_of_the_old() {
    let (stand_in, ids) = filled(Profile::standard());
    let patch = json!({format!("mailboxIds/{}", ids.archive): true, format!("mailboxIds/{}", ids.inbox): null, "keywords/$seen": true});
    let answer = set(&stand_in, json!({"update": {&ids.second: patch}})).unwrap();
    assert_eq!(answer["updated"], json!({&ids.second: null}));
    assert_eq!(mailboxes(&stand_in, &ids.second), std::slice::from_ref(&ids.archive));
    assert_eq!(keywords(&stand_in, &ids.second), ["$seen"], "the marks and the move are one change");
    // The id stays: a server of the standard kind does not name an email by where it lies.
    assert!(stand_in.with(|d| d.email(&ids.second).is_some()));
    // Put in a mailbox without taking it out of the other: in two, which a server of this kind allows.
    assert_eq!(update_one(&stand_in, &ids.first, json!({format!("mailboxIds/{}", ids.archive): true})), "ok");
    assert_eq!(mailboxes(&stand_in, &ids.first), [ids.inbox.clone(), ids.archive.clone()]);
    // Taken out of the last one it is in, it is in none, which no email is.
    assert_eq!(update_one(&stand_in, &ids.third, json!({format!("mailboxIds/{}", ids.inbox): null})), "invalidProperties");
    assert_eq!(mailboxes(&stand_in, &ids.third), std::slice::from_ref(&ids.inbox), "a refused patch changes nothing");
    // A mailbox that is none.
    assert_eq!(update_one(&stand_in, &ids.third, json!({"mailboxIds/Mnope": true})), "invalidProperties");
}

#[test]
fn the_whole_of_keywords_or_mailbox_ids_can_be_set_by_a_server_of_the_standard_kind() {
    let (stand_in, ids) = filled(Profile::standard());
    assert_eq!(update_one(&stand_in, &ids.first, json!({"keywords": {"$flagged": true, "$Answered": true}})), "ok");
    assert_eq!(keywords(&stand_in, &ids.first), ["$answered", "$flagged"]);
    assert_eq!(update_one(&stand_in, &ids.first, json!({"keywords": null})), "ok");
    assert!(keywords(&stand_in, &ids.first).is_empty());
    assert_eq!(update_one(&stand_in, &ids.first, json!({"mailboxIds": {&ids.archive: true}})), "ok");
    assert_eq!(mailboxes(&stand_in, &ids.first), std::slice::from_ref(&ids.archive));
    for (patch, kind) in [
        (json!({"keywords": {"$seen": false}}), "invalidProperties"),
        (json!({"keywords": ["$seen"]}), "invalidProperties"),
        (json!({"mailboxIds": {}}), "invalidProperties"),
        (json!({"mailboxIds": null}), "invalidProperties"),
        (json!({"mailboxIds": {&ids.archive: 1}}), "invalidProperties"),
    ] {
        assert_eq!(update_one(&stand_in, &ids.first, patch.clone()), kind, "{patch}");
    }
    assert_eq!(mailboxes(&stand_in, &ids.first), std::slice::from_ref(&ids.archive));
}

#[test]
fn a_patch_is_held_to_the_grammar() {
    let (stand_in, ids) = filled(Profile::standard());
    let archive = format!("mailboxIds/{}", ids.archive);
    let cases: Vec<(Value, &str)> = vec![
        // A path inside another, whichever way round they come.
        (json!({"keywords": {"$seen": true}, "keywords/$seen": true}), "invalidPatch"),
        (json!({"keywords/$seen": true, "keywords/$seen/more": true}), "invalidPatch"),
        // Not a path: a leading slash, an empty part, a bad escape, nothing.
        (json!({"/keywords/$seen": true}), "invalidPatch"),
        (json!({"keywords//$seen": true}), "invalidPatch"),
        (json!({"keywords/$a~2b": true}), "invalidPatch"),
        (json!({"keywords/$a~": true}), "invalidPatch"),
        (json!({"": true}), "invalidPatch"),
        // Below a boolean there is nothing to change.
        (json!({"keywords/$seen/x": true}), "invalidPatch"),
        // What an email's own is not changed, and what is none is none.
        (json!({"size": 5}), "invalidProperties"),
        (json!({"receivedAt": "2026-01-01T00:00:00Z"}), "invalidProperties"),
        (json!({"id": "E9"}), "invalidProperties"),
        (json!({"subject": "x"}), "invalidProperties"),
        (json!({"nothing": 1}), "invalidProperties"),
        (json!({"nothing/at/all": 1}), "invalidProperties"),
        // A keyword is true, or null to take it away; a mailbox likewise.
        (json!({"keywords/$seen": false}), "invalidProperties"),
        (json!({"keywords/$seen": 1}), "invalidProperties"),
        (json!({archive.clone(): "yes"}), "invalidProperties"),
        // A keyword is 1 to 255 of the characters an IMAP atom has.
        (json!({"keywords/a b": true}), "invalidProperties"),
        (json!({"keywords/a(b": true}), "invalidProperties"),
        (json!({"keywords/a\"b": true}), "invalidProperties"),
        (json!({"keywords/é": true}), "invalidProperties"),
        (json!({format!("keywords/{}", "k".repeat(256)): true}), "invalidProperties"),
        // ~1 is a slash and ~0 a tilde, in a name: `$a/b` and `$a~b` are keywords.
        (json!({"keywords/$a~1b": true}), "ok"),
        (json!({"keywords/$a~0b": true}), "ok"),
        (json!({format!("keywords/{}", "k".repeat(255)): true}), "ok"),
    ];
    for (patch, kind) in cases {
        assert_eq!(update_one(&stand_in, &ids.second, patch.clone()), kind, "{patch}");
    }
    assert!(keywords(&stand_in, &ids.second).contains(&"$a/b".to_string()) && keywords(&stand_in, &ids.second).contains(&"$a~b".to_string()));
    // A patch that is no object, and one of too many entries.
    assert_eq!(update_one(&stand_in, &ids.second, json!([1])), "invalidPatch");
    assert_eq!(update_one(&stand_in, &ids.second, json!("keywords/$seen")), "invalidPatch");
    let many: Map<String, Value> = (0..65).map(|n| (format!("keywords/k{n}"), json!(true))).collect();
    assert_eq!(update_one(&stand_in, &ids.second, Value::Object(many)), "tooLarge");
    let some: Map<String, Value> = (0..64).map(|n| (format!("keywords/j{n}"), json!(true))).collect();
    assert_eq!(update_one(&stand_in, &ids.second, Value::Object(some)), "ok");
    // More keywords than it keeps.
    let more: Map<String, Value> = (0..40).map(|n| (format!("keywords/m{n}"), json!(true))).collect();
    assert_eq!(update_one(&stand_in, &ids.second, Value::Object(more)), "tooManyKeywords");
}

#[test]
fn a_server_that_follows_the_entries_alone_refuses_the_rest() {
    let (stand_in, ids) = filled(Profile::by_place());
    for (patch, kind) in [
        (json!({"keywords": {"$flagged": true}}), "invalidPatch"),
        (json!({"mailboxIds": {&ids.inbox: true}}), "invalidPatch"),
        (json!({"keywords/$seen/x": true}), "invalidPatch"),
        (json!({"keywords/$seen": false}), "invalidProperties"),
        (json!({"keywords/$flagged": true}), "ok"),
    ] {
        assert_eq!(update_one(&stand_in, &ids.second, patch.clone()), kind, "{patch}");
    }
    // One mailbox to an email: put in another without taking it out of the first is too many; taken out of
    // the only one is none.
    assert_eq!(update_one(&stand_in, &ids.second, json!({format!("mailboxIds/{}", ids.archive): true})), "tooManyMailboxes");
    assert_eq!(update_one(&stand_in, &ids.second, json!({format!("mailboxIds/{}", ids.inbox): null})), "invalidProperties");
    assert_eq!(mailboxes(&stand_in, &ids.second), std::slice::from_ref(&ids.inbox));
}

#[test]
fn a_server_that_names_mail_by_place_gives_a_moved_email_another_id_and_does_not_say_which() {
    let (stand_in, ids) = filled(Profile::by_place());
    let blob = stand_in.with(|d| d.email(&ids.second).map(|e| e.blob.clone())).unwrap();
    let move_second = json!({format!("mailboxIds/{}", ids.archive): true, format!("mailboxIds/{}", ids.inbox): null});
    let answer = set(&stand_in, json!({"update": {&ids.second: move_second, &ids.third: {"keywords/$seen": true}}})).unwrap();
    // This kind of server answers `{}` and `[]` for what it did nothing with, and nothing of what is created.
    assert_eq!(answer["updated"], json!({&ids.second: null, &ids.third: null}));
    assert_eq!((&answer["notUpdated"], &answer["destroyed"], &answer["notDestroyed"]), (&json!({}), &json!([]), &json!({})));
    assert!(answer.get("created").is_none() && answer.get("notCreated").is_none(), "{answer}");
    // The old id names nothing now; the email lies in the Archive under another id and another blob.
    assert!(stand_in.with(|d| d.email(&ids.second).is_none()));
    let moved = stand_in.with(|d| d.emails.iter().find(|e| e.mailboxes.contains(&ids.archive)).cloned()).unwrap();
    assert_ne!((moved.id.as_str(), moved.blob.as_str()), (ids.second.as_str(), blob.as_str()));
    assert_eq!(moved.raw, mail("second"));
    // A change of marks alone does not rename it.
    assert!(stand_in.with(|d| d.email(&ids.third).is_some_and(|e| e.keywords.contains("$seen"))));
    // The old id names nothing, and this kind of server does not look: a later change of it, and a destroy, are
    // answered as made.
    assert_eq!(update_one(&stand_in, &ids.second, json!({"keywords/$seen": true})), "ok");
    let gone = set(&stand_in, json!({"destroy": [&ids.second]})).unwrap();
    assert_eq!((&gone["destroyed"], &gone["notDestroyed"]), (&json!([&ids.second]), &json!({})));
    // One that follows the standard says what is so: the old id of a move is `notFound`.
    let (standard, ids) = filled(Profile::standard());
    assert!(standard.with(|d| d.email(&ids.second).is_some()));
    set(&standard, json!({"destroy": [&ids.second]})).unwrap();
    assert_eq!(update_one(&standard, &ids.second, json!({"keywords/$seen": true})), "notFound");
    let again = set(&standard, json!({"destroy": [&ids.second]})).unwrap();
    assert_eq!(again["notDestroyed"], json!({&ids.second: {"type": "notFound", "description": "no such email"}}));
}

#[test]
fn a_server_that_names_mail_by_place_can_say_the_new_id() {
    let (stand_in, ids) = filled(Profile { says_moved_id: true, ..Profile::by_place() });
    let move_second = json!({format!("mailboxIds/{}", ids.archive): true, format!("mailboxIds/{}", ids.inbox): null});
    let answer = set(&stand_in, json!({"update": {&ids.second: move_second, &ids.third: {"keywords/$seen": true}}})).unwrap();
    let new = answer["updated"][&ids.second]["id"].as_str().unwrap().to_string();
    assert!(stand_in.with(|d| d.email(&new).is_some_and(|e| e.mailboxes.contains(&ids.archive))));
    assert_eq!(answer["updated"][&ids.third], Value::Null, "only a move gives a new id");
    // Moved to where it is: not a move.
    let again = set(&stand_in, json!({"update": {&new: {format!("mailboxIds/{}", ids.archive): true}}})).unwrap();
    assert_eq!(again["updated"], json!({&new: null}));
}

#[test]
fn emails_are_destroyed_one_by_one_and_each_has_its_own_answer() {
    let (stand_in, ids) = filled(Profile::standard());
    let blob = stand_in.with(|d| d.email(&ids.first).map(|e| e.blob.clone())).unwrap();
    let answer = set(&stand_in, json!({"destroy": [&ids.first, "Enot-there", &ids.second, &ids.first]})).unwrap();
    assert_eq!(answer["destroyed"], json!([&ids.first, &ids.second]), "an id twice is destroyed once");
    assert_eq!(answer["notDestroyed"], json!({"Enot-there": {"type": "notFound", "description": "no such email"}}));
    assert_eq!(answer["updated"], Value::Null);
    assert_eq!(stand_in.with(|d| d.emails.iter().map(|e| e.id.clone()).collect::<Vec<_>>()), std::slice::from_ref(&ids.third));
    assert!(stand_in.with(|d| d.emails.iter().all(|e| e.blob != blob)), "its blob goes with it");
    // An email updated and destroyed in one call is destroyed, and the update says so.
    let both = set(&stand_in, json!({"update": {&ids.third: {"keywords/$seen": true}}, "destroy": [&ids.third]})).unwrap();
    assert_eq!(both["destroyed"], json!([&ids.third]));
    assert_eq!(both["notUpdated"][&ids.third]["type"], "willDestroy");
    assert!(stand_in.with(|d| d.emails.is_empty()));
}

#[test]
fn the_call_is_held_to_its_arguments_its_state_and_its_limits() {
    let (stand_in, ids) = filled(Profile::standard());
    for (args, kind) in [
        (json!({"update": {&ids.first: {}}, "extra": 1}), "invalidArguments"),
        (json!({"update": [1]}), "invalidArguments"),
        (json!({"update": "x"}), "invalidArguments"),
        (json!({"destroy": {"x": 1}}), "invalidArguments"),
        (json!({"destroy": ["ok", 5]}), "invalidArguments"),
        (json!({"destroy": ["a b"]}), "invalidArguments"),
        (json!({"update": {"a/b": {}}}), "invalidArguments"),
        (json!({"update": {"": {}}}), "invalidArguments"),
        (json!({"ifInState": 5}), "invalidArguments"),
        (json!({"create": 5}), "invalidArguments"),
        (json!({"accountId": "A9"}), "accountNotFound"),
        (json!({"accountId": 3}), "invalidArguments"),
        (json!({"ifInState": "not the state"}), "stateMismatch"),
    ] {
        assert_eq!(set(&stand_in, args.clone()).unwrap_err(), kind, "{args}");
    }
    // The state it is in is let through, as is none.
    let state = stand_in.with(|d| d.state());
    assert!(set(&stand_in, json!({"ifInState": state, "update": {&ids.first: {"keywords/$flagged": true}}})).is_ok());
    assert!(
        set(&stand_in, json!({"ifInState": state, "update": {&ids.first: {"keywords/$flagged": null}}})).is_err(),
        "it is not in that state any more"
    );
    assert_eq!(keywords(&stand_in, &ids.first), ["$flagged", "$seen"]);
    assert!(set(&stand_in, json!({"ifInState": null, "update": null, "destroy": null, "create": null})).is_ok());
    // Nothing to do is an answer all the same.
    let nothing = set(&stand_in, json!({})).unwrap();
    assert_eq!((&nothing["updated"], &nothing["destroyed"]), (&Value::Null, &Value::Null));
    // No more objects than the session says, created, updated and destroyed together.
    stand_in.with(|d| d.max_set = 3);
    let client = block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    assert_eq!(client.max_objects_in_set(), 3);
    assert!(set(&stand_in, json!({"update": {&ids.first: {}, &ids.second: {}}, "destroy": [&ids.third]})).is_ok());
    assert_eq!(set(&stand_in, json!({"update": {&ids.first: {}, &ids.second: {}}, "destroy": [&ids.third, "E7"]})).unwrap_err(), "requestTooLarge");
    assert_eq!(set(&stand_in, json!({"create": {"a": {}, "b": {}}, "destroy": [&ids.third, "E7"]})).unwrap_err(), "requestTooLarge");
    // Emails are imported, not created.
    stand_in.with(|d| d.max_set = 500);
    let made = set(&stand_in, json!({"create": {"c1": {"mailboxIds": {&ids.inbox: true}}}})).unwrap();
    assert_eq!(made["notCreated"]["c1"]["type"], "forbidden");
    stand_in.with(|d| d.profile = Profile::by_place());
    assert_eq!(set(&stand_in, json!({"create": {"c1": {}}})).unwrap_err(), "invalidArguments");
}

#[test]
fn a_method_that_is_not_in_the_request_s_capabilities_is_unknown() {
    let (stand_in, ids) = filled(Profile::standard());
    let client = block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let args = json!({"accountId": ACCOUNT, "update": {&ids.first: {"keywords/$flagged": true}}});
    let answers = block_on(client.call(&[crate::CORE], vec![("Email/set", args)])).unwrap();
    assert!(matches!(answers.get(0), Err(Error::Refused { kind, .. }) if kind == "unknownMethod"));
    assert_eq!(keywords(&stand_in, &ids.first), ["$seen"]);
}

#[test]
fn an_email_a_test_has_refused_is_refused_with_the_type_it_named_and_the_others_go_through() {
    let (stand_in, ids) = filled(Profile::standard());
    stand_in.with(|d| {
        d.refused_sets.insert(ids.first.clone(), "forbidden");
        d.refused_sets.insert(ids.second.clone(), "serverFail");
    });
    let answer = set(
        &stand_in,
        json!({"update": {&ids.first: {"keywords/$flagged": true}, &ids.second: {"keywords/$flagged": true}, &ids.third: {"keywords/$seen": true}}}),
    )
    .unwrap();
    assert_eq!(answer["updated"], json!({&ids.third: null}));
    assert_eq!(
        (answer["notUpdated"][&ids.first]["type"].as_str(), answer["notUpdated"][&ids.second]["type"].as_str()),
        (Some("forbidden"), Some("serverFail"))
    );
    assert_eq!(keywords(&stand_in, &ids.first), ["$seen"]);
    let answer = set(&stand_in, json!({"destroy": [&ids.first, &ids.third]})).unwrap();
    assert_eq!((answer["destroyed"].clone(), answer["notDestroyed"][&ids.first]["type"].clone()), (json!([&ids.third]), json!("forbidden")));
}

#[test]
fn an_answer_can_be_lost_after_the_work_is_done() {
    let (stand_in, ids) = filled(Profile::standard());
    stand_in.with(|d| d.lost_answers = 1);
    let client = block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let args = json!({"accountId": ACCOUNT, "update": {&ids.second: {"keywords/$flagged": true}}});
    let lost = block_on(client.mail(vec![("Email/set", args.clone())]));
    assert!(matches!(lost, Err(Error::Io(_))), "{lost:?}");
    assert_eq!(keywords(&stand_in, &ids.second), ["$flagged"], "it was done");
    // Only the next request: the one after is answered.
    assert!(block_on(client.mail(vec![("Email/set", args)])).is_ok());
    assert_eq!(stand_in.api_requests(), 2);
    // The session is no request to the API, and is never lost.
    stand_in.with(|d| d.lost_answers = 1);
    assert!(block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).is_ok());
}

#[test]
fn what_it_was_asked_counts_the_updates_and_the_destroys() {
    let (stand_in, ids) = filled(Profile::standard());
    set(&stand_in, json!({"update": {&ids.first: {}, &ids.second: {}}, "destroy": [&ids.third]})).unwrap();
    set(&stand_in, json!({"destroy": ["E1"]})).unwrap();
    let asked: Vec<(Option<usize>, Option<usize>)> =
        stand_in.asked().iter().filter(|a| a.method == "Email/set").map(|a| (a.updates, a.destroys)).collect();
    assert_eq!(asked, [(Some(2), Some(1)), (None, Some(1))]);
}

proptest! {
    /// Whatever a client sends as a patch, the server answers, never panics, and an email in a mailbox that
    /// exists comes out of it in at least one.
    #[test]
    fn any_patch_is_answered_and_leaves_the_email_somewhere(path in "[ -~]{0,24}", value in prop_oneof![Just(json!(true)), Just(json!(null)), Just(json!(false)), Just(json!({"a": true})), Just(json!(["x"])), Just(json!("s"))], plain in any::<bool>()) {
        let (stand_in, ids) = filled(if plain { Profile::by_place() } else { Profile::standard() });
        let patch = json!({path: value});
        let _ = set(&stand_in, json!({"update": {&ids.second: patch}}));
        let at = stand_in.with(|d| d.emails.iter().filter(|e| e.raw == mail("second")).map(|e| e.mailboxes.len()).collect::<Vec<_>>());
        prop_assert_eq!(at.len(), 1);
        prop_assert!(at[0] >= 1);
    }
}
