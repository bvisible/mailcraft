// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): Nora's ready replies, with a host that keeps every call for the test.
//! Nora's ready replies with a host that keeps every call for the test to answer: when they are asked for (and when
//! not), what goes out (the message's id, never its text), what the card becomes, what Edit and Send write, that a
//! message never goes out with a field of hers still empty, the person's yes and their « not now ». A hosting page's
//! mailbox of invented people at `example.org`.

use craft_nora_ui::proposal::ProposalAction;
use mailcraft_model::Address;
use mailcraft_model::stream::Stream;
use serde_json::{Value, json};

use super::super::testing::*;
use super::super::{MY_STYLE, READY_REPLIES};
use super::{Shown, WAIT_MS, act, consented, open_style, unfilled_in_compose};
use crate::MailApp;

/// A hosting page's window whose first message has been read a second: its ready replies asked for.
fn asked() -> (MailApp, Calls) {
    let (mut app, calls, _) = page(true);
    app.poll_nora();
    app.nora.ready.age(WAIT_MS + 1.0);
    app.poll_nora();
    assert_eq!(calls.last().method, READY_REPLIES);
    (app, calls)
}

fn proposal() -> Value {
    json!({"message": {
        "success": true,
        "language": "en",
        "draft": {"segments": [{"t": "Hello Grace,\n\nThe proof is ready on "}, {"f": 0}, {"t": ".\n\nAda"}]},
        "short_replies": [{"label": "Say it is ready", "segments": [{"t": "It is ready."}]}],
        "fields": [{"label": "Date", "kind": "date"}],
        "style": {"learnt": true, "learning": false},
        "cached": false
    }})
}

#[test]
fn the_ready_replies_are_asked_once_a_message_has_been_read_a_second() {
    let (mut app, calls, _) = page(true);
    app.poll_nora();
    assert_eq!(calls.count(), 0, "not at once: no call for the messages gone through with the arrow keys");
    app.nora.ready.age(WAIT_MS + 1.0);
    app.poll_nora();
    let call = calls.last();
    assert_eq!(call.method, READY_REPLIES);
    assert_eq!(call.args, json!({"account_name": ME, "email_id": "E1"}), "its id and the mailbox, nothing of its text");
    app.poll_nora();
    assert_eq!(calls.count(), 1, "once");
    calls.all_fit();
}

#[test]
fn the_answer_is_the_card_s_proposal() {
    let (mut app, calls) = asked();
    calls.answer(200, proposal());
    app.poll_nora();
    match &app.nora.ready.shown {
        Some(Shown::Proposal { proposal, style }) => {
            assert_eq!(proposal.fields[0].label, "Date");
            assert_eq!(proposal.short[0].label, "Say it is ready");
            assert!(style.learnt);
        }
        other => panic!("not a proposal: {other:?}"),
    }
}

#[test]
fn an_answer_for_a_message_no_longer_read_is_not_the_card_s() {
    let (mut app, calls) = asked();
    let e2 = id_of(&app, "E2");
    app.execute("mail.select", json!({"id": e2})).unwrap();
    app.poll_nora();
    calls.answer(200, proposal());
    app.poll_nora();
    assert_eq!(app.nora.ready.message, Some(e2));
    assert_eq!(app.nora.ready.shown, None, "E2's own card, not yet asked for");
}

#[test]
fn edit_opens_the_reply_with_the_text_and_send_waits_for_its_fields() {
    let (mut app, calls) = asked();
    calls.answer(200, proposal());
    app.poll_nora();
    let e1 = id_of(&app, "E1");
    act(&mut app, e1, ProposalAction::Edit("Hello Grace,\n\nThe proof is ready on ⟦Date⟧.\n\nAda".into()));
    assert_eq!(app.session.compose.as_ref().map(|c| c.source), Some(Some(e1)), "the reply to E1 is open");
    assert!(body(&app).starts_with("Hello Grace,\n\nThe proof is ready on ⟦Date⟧."), "{:?}", body(&app));
    let refused = app.run("compose.send", json!({})).unwrap_err();
    assert!(refused.contains("\"Date\""), "{refused}");
    assert!(app.session.compose.is_some(), "still being written");
    assert!(unfilled_in_compose(&app).is_some());
    // Filled in by hand, the marker typed over: nothing holds it back any more.
    let marker = "Hello Grace,\n\nThe proof is ready on ⟦Date⟧.\n\nAda".chars().count();
    app.execute("compose.select", json!({"from": 0, "to": marker})).unwrap();
    app.execute("compose.type", json!({"text": "Hello Grace,\n\nThe proof is ready on Friday.\n\nAda"})).unwrap();
    assert_eq!(unfilled_in_compose(&app), None, "{:?}", body(&app));
}

#[test]
fn send_writes_the_reply_and_sends_it() {
    let (mut app, calls) = asked();
    // The page's window sends from its JMAP account (a run it hands over: here, one that never answers).
    app.services.jmap = Some(Box::new(|_| std::sync::mpsc::channel().1));
    calls.answer(200, proposal());
    app.poll_nora();
    let e1 = id_of(&app, "E1");
    act(&mut app, e1, ProposalAction::Send("Hello Grace,\n\nThe proof is ready on Friday 16 October.\n\nAda".into()));
    assert!(app.session.compose.is_none(), "sent: {}", status(&app));
    let sent = app.session.mailbox.messages.values().find(|m| m.body.text.contains("ready on Friday 16 October"));
    assert!(sent.is_some_and(|m| m.subject.contains("The proof, E1")), "the reply to E1 went out");
    calls.all_fit();
    assert!(app.nora.ready.dismissed.contains(&e1), "its card is done with");
}

#[test]
fn the_first_time_the_person_is_asked_and_their_yes_asks_again() {
    let (mut app, calls) = asked();
    calls.answer(200, json!({"message": {"success": true, "consent_needed": true}}));
    app.poll_nora();
    assert_eq!(app.nora.ready.shown, Some(Shown::Consent));
    consented(&mut app, true);
    let call = calls.last();
    assert_eq!((call.method, call.args.clone()), (MY_STYLE, json!({"action": "enable"})));
    calls.answer(200, json!({"message": {"success": true, "ready_replies": true, "consented": true, "style": {"learnt": false, "learning": true}}}));
    app.poll_nora();
    app.poll_nora();
    assert_eq!(calls.count(), 3);
    assert_eq!(calls.last().method, READY_REPLIES, "asked again, now that they are on");
    calls.all_fit();
}

#[test]
fn not_now_asks_nothing_more_this_time() {
    let (mut app, calls) = asked();
    calls.answer(200, json!({"message": {"success": true, "consent_needed": true}}));
    app.poll_nora();
    consented(&mut app, false);
    assert_eq!(calls.last().args, json!({"action": "decline"}));
    let e2 = id_of(&app, "E2");
    app.execute("mail.select", json!({"id": e2})).unwrap();
    app.poll_nora();
    app.nora.ready.age(WAIT_MS + 1.0);
    app.poll_nora();
    assert_eq!(calls.count(), 2, "no ready replies asked for E2");
}

#[test]
fn nothing_is_asked_where_no_reply_is_wanted() {
    let cases: [(&str, fn(&mut MailApp)); 5] = [
        ("a newsletter", |app| app.session.mailbox.message_mut(id_of(app, "E1")).unwrap().stream = Some(Stream::Newsletter)),
        ("a notification", |app| app.session.mailbox.message_mut(id_of(app, "E1")).unwrap().stream = Some(Stream::Notification)),
        ("answered already", |app| app.session.mailbox.message_mut(id_of(app, "E1")).unwrap().answered = true),
        ("from the mailbox itself", |app| app.session.mailbox.message_mut(id_of(app, "E1")).unwrap().from = Address::new("Ada", ME)),
        ("a host that doesn't take them (Neodesk's shell)", |app| app.services.nora.as_mut().unwrap().chat = false),
    ];
    for (case, change) in cases {
        let (mut app, calls, _) = page(true);
        change(&mut app);
        app.poll_nora();
        app.nora.ready.age(WAIT_MS + 1.0);
        app.poll_nora();
        assert_eq!(calls.count(), 0, "{case}");
    }
}

#[test]
fn what_the_guard_held_back_is_said_and_a_failure_shows_nothing() {
    let (mut app, calls) = asked();
    calls.answer(200, json!({"message": {"success": true, "blocked": true, "message": REFUSED}}));
    app.poll_nora();
    assert_eq!(app.nora.ready.shown, Some(Shown::Blocked(REFUSED.to_string())));

    let (mut app, calls) = asked();
    calls.answer(500, json!({"exception": "Traceback ... LLM service error"}));
    app.poll_nora();
    assert_eq!(app.nora.ready.shown, Some(Shown::Nothing), "the person reads their mail as before");
}

#[test]
fn my_style_opens_and_reads_what_nora_keeps() {
    let (mut app, calls, _) = page(true);
    assert!(app.enabled("ui.noraStyle"));
    app.run("ui.noraStyle", json!({})).unwrap();
    assert!(app.nora.ready.style.open);
    let call = calls.last();
    assert_eq!((call.method, call.args.clone()), (MY_STYLE, json!({"action": "get"})));
    calls.answer(200, json!({"message": {"success": true, "ready_replies": true, "points": ["You write \"vous\"."], "style": {"learnt": true}}}));
    app.poll_nora();
    let view = app.nora.ready.style.view.clone().unwrap();
    assert_eq!(view["points"][0], "You write \"vous\".");
    assert!(!app.nora.ready.quiet, "on: the ready replies are asked for");
    // Where they don't go out, « My style » is not offered and the band learns the style as before.
    app.services.nora.as_mut().unwrap().chat = false;
    assert!(!app.enabled("ui.noraStyle"));
    assert!(open_style(&mut app).is_err());
}
