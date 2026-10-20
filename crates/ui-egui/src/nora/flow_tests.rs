// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): Nora's commands, and what becomes of her answers.
//! Nora's commands with a host that keeps every call for the test to answer: what is sent (and what
//! is not), where her draft and her version go, one undo step each, what the person is told, the
//! waits that end, and junk that is turned away. The sample mailbox's own people, and a hosting
//! page's mailbox of invented ones at `example.org`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

use mailcraft_engine::Session;
use mailcraft_model::rich::{ListKind, Paragraph, Run};
use mailcraft_model::sync::{GraphFetchResult, GraphFetched};
use mailcraft_model::{AccountKind, Id, RichBody, RichText};
use serde_json::{Value, json};

use super::super::testing::*;
use super::super::{DEADLINE_MS, GENERATE_EMAIL, NoraAnswer, PROOFREAD, QUICK_REPLY, SCAN_STYLE, TRANSLATE};
use super::{About, Kind};
use crate::jmap_job::JmapFetchJob;
use crate::{MailApp, Services};

#[test]
fn reply_by_the_servers_id_sends_the_id_and_no_text() {
    let (mut app, calls, _) = page(true);
    let e2 = id_of(&app, "E2");
    let asked = app.run("ui.noraReply", json!({"message": e2})).unwrap();
    assert_eq!((asked["composing"].clone(), asked["method"].clone()), (json!(true), json!(QUICK_REPLY)));
    let call = calls.last();
    assert_eq!(call.method, QUICK_REPLY);
    assert_eq!(call.args, json!({"account_name": ME, "email_id": "E2"}), "its id and the mailbox, nothing of its text");
    assert_eq!(app.session.compose.as_ref().map(|c| c.source), Some(Some(e2)), "the reply is open, waiting for her draft");
    assert_eq!(status(&app), "Nora is writing the reply…");
    calls.answer(200, json!({"message": {"success": true, "draft_text": "Hello Grace,\n\nYes, it is ready.", "language": "en"}}));
    app.poll_nora();
    assert!(body(&app).starts_with("Hello Grace,\n\nYes, it is ready.\n"), "{:?}", body(&app));
    calls.all_fit();
}

#[test]
fn reply_for_the_sample_sends_the_text_as_context() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("ui.noraReply", json!({})).unwrap();
    let call = calls.last();
    assert_eq!(call.method, GENERATE_EMAIL);
    assert_eq!(keys(&call.args), ["account_name", "prompt", "reply_context"].map(String::from).into());
    assert_eq!(call.args["account_name"], "", "the sample mailbox is no mailbox of the server");
    // As HTML the server reads as text: written as text, the sender's address would be taken for a tag there.
    let context = call.args["reply_context"].as_str().unwrap();
    let head =
        format!("<p>{}<br>Subject: {PLAIN}</p>\n<p>Avery,</p>\n<p>The roadmap review is moving", PRIYA.replace('<', "&lt;").replace('>', "&gt;"));
    assert!(context.starts_with(&head), "{context}");
    assert!(call.args["prompt"].as_str().unwrap().starts_with("Write the reply to the email being replied to"));
    calls.all_fit();
}

#[test]
fn another_account_than_the_pages_is_refused() {
    // A JMAP mailbox the page didn't hand over (the person added it in the page).
    let (mut app, calls, _) = page(true);
    app.services.nora.as_mut().unwrap().mailboxes = vec!["someone.else@example.org".into()];
    let said = app.run("ui.noraReply", json!({})).unwrap_err();
    assert_eq!(said, "Nora works on the mailboxes of your Neoffice account only.");
    assert!(app.execute("ui.noraTranslate", json!({"language": "fr"})).is_err());
    assert_eq!(calls.count(), 0, "nothing sent");
    assert!(app.session.compose.is_none(), "and no reply opened");
    // Mail of another kind of account is not sent either.
    let (mut app, calls, _) = sample(PLAIN);
    for account in &mut app.session.mailbox.accounts {
        account.kind = AccountKind::Imap;
    }
    assert_eq!(app.run("ui.noraReply", json!({})).unwrap_err(), "Nora works on the mailboxes of your Neoffice account only.");
    assert_eq!(app.execute("ui.noraTranslate", json!({"language": "de"})).unwrap_err(), "Nora works on the mailboxes of your Neoffice account only.");
    assert_eq!(calls.count(), 0);
    assert!(app.session.compose.is_none());
}

#[test]
fn a_page_message_without_an_id_opens_no_reply() {
    for id in ["", "E1/../E2", "E1 E2", "E\u{0}1", &"E".repeat(600)] {
        let (mut app, calls, _) = page(true);
        let e1 = id_of(&app, "E1");
        app.session.mailbox.message_mut(e1).unwrap().server.as_mut().unwrap().id = id.to_string();
        let said = app.run("ui.noraReply", json!({"message": e1})).unwrap_err();
        assert_eq!(said, "Nora couldn't find this message on the server: refresh the list and try again.", "{id:?}");
        assert_eq!(app.execute("ui.noraTranslate", json!({"message": e1, "language": "en"})).unwrap_err(), said);
        assert_eq!(calls.count(), 0, "no call, and no text instead of the id");
        assert!(app.session.compose.is_none(), "no reply opened for a draft that won't come");
    }
}

#[test]
fn the_draft_goes_above_the_signature_and_the_quote_in_one_undo_step() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("ui.noraReply", json!({})).unwrap();
    let before = body(&app);
    assert!(before.starts_with("\n\nAvery Quinn\nLumen Studio\n\n-----Original Message-----\n"), "{before:?}");
    let draft = "Bonjour Priya,\n\nJeudi me convient.\n\nAvery";
    calls.answer(200, json!({"message": {"success": true, "draft_text": draft}}));
    app.poll_nora();
    assert_eq!(body(&app), format!("{draft}{before}"), "the draft, then the reply as it was");
    assert_eq!(signature(&app).as_deref(), Some("Avery Quinn\nLumen Studio"), "the signature is still the signature");
    assert_eq!(said(&app), Some(("Nora's draft is in the reply: read it before sending.".into(), false, About::Writing)));
    assert_eq!(status(&app), "Nora's draft is in the reply: read it before sending.");
    assert!(!app.nora.busy());
    // One step takes it back, and it was the only one.
    app.run("compose.undo", json!({})).unwrap();
    assert_eq!(body(&app), before);
    assert!(app.execute("compose.undo", json!({})).is_err());
    calls.all_fit();
}

#[test]
fn a_draft_after_the_owner_typed_goes_below_a_blank_line() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("ui.noraReply", json!({})).unwrap();
    typed(&mut app, "Merci !");
    calls.answer(200, json!({"message": {"draft_text": "Bonjour Priya,\n\nC'est noté."}}));
    app.poll_nora();
    assert!(body(&app).starts_with("Bonjour Priya,\n\nC'est noté.\n\nMerci !\n\nAvery Quinn\nLumen Studio\n"), "{:?}", body(&app));
    assert_eq!(signature(&app).as_deref(), Some("Avery Quinn\nLumen Studio"));
}

#[test]
fn a_draft_into_a_body_whose_first_paragraph_is_the_signature() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("ui.noraReply", json!({})).unwrap();
    // A body that starts with the signature's own paragraphs (a script set it so).
    let marked = |text: &str| Paragraph { signature: true, ..Paragraph::new(vec![Run::plain(text)], ListKind::None) };
    let starts_with_signature = RichText::from_paragraphs(vec![marked("Avery Quinn"), marked("Lumen Studio"), Paragraph::default()]);
    // What a paste at the top would do to it: the draft's first line would join the signature.
    let mut pasted = RichBody::new(starts_with_signature.clone());
    pasted.paste(&RichText::from_plain("Bonjour")).unwrap();
    assert_eq!(pasted.text().signature().map(|s| s.text()).as_deref(), Some("BonjourAvery Quinn\nLumen Studio"));
    app.session.compose.as_mut().unwrap().body = RichBody::new(starts_with_signature);
    let before = body(&app);
    calls.answer(200, json!({"message": {"draft_text": "Bonjour"}}));
    app.poll_nora();
    assert_eq!(body(&app), before, "nothing goes into the signature");
    let why = "Your reply starts with its signature: leave a line above it for Nora's draft, and ask her again.";
    assert_eq!(said(&app), Some((why.into(), true, About::Writing)));
}

#[test]
fn a_refusal_leaves_the_reply_as_it_was_and_she_can_be_asked_again() {
    let (mut app, calls, message) = sample(PLAIN);
    app.run("ui.noraReply", json!({"all": true})).unwrap();
    assert_eq!(app.session.compose.as_ref().map(|c| c.mode), Some(mailcraft_engine::ComposeMode::ReplyAll));
    let before = body(&app);
    calls.answer(417, refusal(REFUSED));
    app.poll_nora();
    assert_eq!(body(&app), before);
    assert_eq!(said(&app), Some((format!("Nora couldn't write the reply: {REFUSED}"), true, About::Writing)));
    assert!(app.enabled("ui.noraReply"));
    app.run("ui.noraReply", json!({"message": message})).unwrap();
    assert_eq!(calls.count(), 2, "she can be asked again");
}

#[test]
fn a_draft_for_a_reply_that_was_closed_goes_nowhere() {
    let (mut app, calls, message) = sample(PLAIN);
    app.run("ui.noraReply", json!({})).unwrap();
    app.execute("compose.discard", json!({})).unwrap();
    calls.answer(200, json!({"message": {"draft_text": "Bonjour"}}));
    app.poll_nora();
    assert!(app.session.compose.is_none());
    assert_eq!(said(&app), Some(("The reply was closed before Nora's draft came back.".into(), false, About::Writing)));
    // A reply to another message is not hers either.
    app.run("ui.noraReply", json!({})).unwrap();
    let other = app.session.mailbox.messages.values().find(|m| m.subject == "Release checklist").map(|m| m.id).unwrap();
    app.execute("mail.reply", json!({"id": other})).unwrap();
    calls.answer(200, json!({"message": {"draft_text": "Bonjour"}}));
    app.poll_nora();
    assert!(!body(&app).contains("Bonjour"));
    assert_eq!(said(&app).map(|s| s.0), Some("The reply was closed before Nora's draft came back.".into()));
    // Nor is a reply to the same message that the person opened again by hand once hers was closed,
    // if only for a frame: a draft must not go out in a reply she was no longer asked for.
    app.execute("compose.discard", json!({})).unwrap();
    app.run("ui.noraReply", json!({"message": message})).unwrap();
    app.execute("compose.discard", json!({})).unwrap();
    app.poll_nora();
    app.execute("mail.reply", json!({"id": message})).unwrap();
    typed(&mut app, "My own words.");
    calls.answer(200, json!({"message": {"draft_text": "Bonjour"}}));
    app.poll_nora();
    assert!(body(&app).starts_with("My own words.\n") && !body(&app).contains("Bonjour"), "{:?}", body(&app));
    assert_eq!(said(&app).map(|s| s.0), Some("The reply was closed before Nora's draft came back.".into()));
    // The same for her version of the person's text: a message closed for a frame is not hers any more.
    app.run("ui.noraCorrect", json!({})).unwrap();
    app.execute("compose.discard", json!({})).unwrap();
    app.poll_nora();
    app.execute("mail.reply", json!({"id": message})).unwrap();
    typed(&mut app, "My own words.");
    calls.answer(200, json!({"message": {"corrected_text": "Her words.", "has_changes": true}}));
    app.poll_nora();
    assert!(body(&app).starts_with("My own words.\n"));
    assert_eq!(said(&app).map(|s| s.0), Some("The message was closed before Nora's version came back.".into()));
}

#[test]
fn an_answer_to_another_call_is_no_answer() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("ui.noraReply", json!({})).unwrap();
    let before = body(&app);
    {
        let all = calls.0.borrow();
        let (call, to) = all.last().unwrap();
        to.as_ref()
            .unwrap()
            .send(NoraAnswer { id: call.id + 1, status: 200, body: json!({"message": {"draft_text": "Not yours"}}).to_string() })
            .unwrap();
    }
    app.poll_nora();
    assert_eq!(body(&app), before);
    assert_eq!(said(&app).map(|s| s.0), Some("Nora couldn't be reached: the connection stopped before Nora answered".into()));
}

#[test]
fn the_keyboard_and_the_caret_stay_where_the_person_had_them() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("ui.noraReply", json!({})).unwrap();
    typed(&mut app, "Merci !");
    // The caret in the person's text, after "Merci".
    app.execute("compose.select", json!({"from": 5})).unwrap();
    calls.answer(200, json!({"message": {"draft_text": "Bonjour Priya,\n\nC'est noté."}}));
    app.poll_nora();
    let draft = "Bonjour Priya,\n\nC'est noté.\n\n";
    assert!(body(&app).starts_with(&format!("{draft}Merci !")));
    let at = draft.chars().count() + 5;
    assert_eq!(app.session.compose.as_ref().map(|c| c.body.selection()), Some(mailcraft_model::rich::Selection { anchor: at, head: at }));
    // Nobody's keyboard was in the body (no window here): it isn't taken there.
    assert!(!app.body_focus_wanted);
    // Her version over the person's text: a caret in it goes to its end; one past it stays past it.
    let typed_len = "Bonjour Priya,\n\nC'est noté.\n\nMerci !".chars().count();
    app.execute("compose.select", json!({"from": 3})).unwrap();
    app.run("ui.noraImprove", json!({})).unwrap();
    calls.answer(200, json!({"message": {"improved_text": "Bonjour Priya,\n\nC'est noté, merci !", "has_changes": true}}));
    app.poll_nora();
    let new_len = "Bonjour Priya,\n\nC'est noté, merci !".chars().count();
    assert!(body(&app).starts_with("Bonjour Priya,\n\nC'est noté, merci !\n"));
    assert_eq!(app.session.compose.as_ref().map(|c| c.body.selection().head), Some(new_len));
    assert!(typed_len > new_len);
}

/// The host's downloads of whole messages, kept for the test to answer.
type Downloads = Rc<RefCell<Vec<(JmapFetchJob, Sender<GraphFetchResult>)>>>;

fn downloads(app: &mut MailApp) -> Downloads {
    let asked: Downloads = Default::default();
    let kept = asked.clone();
    app.services.jmap_fetch = Some(Box::new(move |job: JmapFetchJob| {
        let (tx, rx) = mpsc::channel();
        kept.borrow_mut().push((job, tx));
        rx
    }));
    asked
}

fn whole_message(account: Id, id: &str) -> GraphFetchResult {
    let mime = format!(
        "From: Grace Hopper <grace@example.org>\r\nTo: {ME}\r\nSubject: The proof, {id}\r\nMessage-ID: <{id}@mail.example.org>\r\nDate: Wed, 14 Oct 2026 09:00:00 +0000\r\n\
MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello Ada,\r\n\r\nIs the proof ready?\r\n"
    );
    GraphFetchResult { account, email: ME.into(), fetched: vec![GraphFetched { id: id.into(), mime: mime.into_bytes() }], ..Default::default() }
}

#[test]
fn a_reply_that_waits_for_its_download_is_asked_when_it_opens() {
    let (mut app, calls, account) = page(false);
    let asked = downloads(&mut app);
    let e3 = id_of(&app, "E3");
    let pending = app.run("ui.noraReply", json!({"message": e3})).unwrap();
    assert_eq!(pending, json!({"pending": "download", "message": e3}));
    assert_eq!(calls.count(), 0, "no call before there is a reply to put the draft in");
    assert!(!app.enabled("ui.noraReply"), "one reply at a time");
    assert_eq!(app.execute("ui.noraReply", json!({"message": e3})).unwrap_err(), "Nora is already writing a reply.");
    // Nothing yet.
    app.poll_nora();
    app.drive_fetch();
    assert_eq!(calls.count(), 0);
    // The message comes; the reply opens; she is asked, by its id.
    let (_, to) = asked.borrow_mut().remove(0);
    to.send(whole_message(account, "E3")).unwrap();
    app.poll_nora();
    app.drive_fetch();
    assert!(app.session.compose.as_ref().is_some_and(|c| c.source == Some(e3)));
    app.poll_nora();
    assert_eq!(calls.count(), 1);
    assert_eq!(calls.last().args, json!({"account_name": ME, "email_id": "E3"}));
    calls.answer(200, json!({"message": {"draft_text": "Hello Grace,\n\nYes."}}));
    app.poll_nora();
    assert!(body(&app).starts_with("Hello Grace,\n\nYes.\n"));
    // A download that ends without a reply: nothing is asked, and she can be asked again.
    app.session.compose = None;
    let e2 = id_of(&app, "E2");
    app.run("ui.noraReply", json!({"message": e2})).unwrap();
    let (_, to) = asked.borrow_mut().remove(0);
    drop(to);
    app.drive_fetch();
    app.nora.age(5_000.0);
    app.poll_nora();
    assert!(!app.nora.busy() && calls.count() == 1);
    assert_eq!(said(&app).map(|s| s.0), Some("Nora couldn't write the reply: the message didn't download".into()), "and says so");
    assert!(app.enabled("ui.noraReply"));
    // A download that goes on keeps the reply waiting past the window's own deadline, to the browser's.
    app.session.compose = None;
    let e1 = id_of(&app, "E1");
    app.run("ui.noraReply", json!({"message": e1})).unwrap();
    app.nora.age(250_000.0);
    app.poll_nora();
    assert!(app.nora.asking(Kind::Reply), "still downloading");
    app.nora.age(100_000.0);
    app.poll_nora();
    assert!(!app.nora.asking(Kind::Reply), "given up at last");
}

#[test]
fn a_second_ask_while_she_writes_asks_nothing_more() {
    let (mut app, calls, _) = sample(PLAIN);
    assert!(app.enabled("ui.noraReply"));
    app.run("ui.noraReply", json!({})).unwrap();
    assert!(!app.enabled("ui.noraReply"));
    assert_eq!(app.run("ui.noraReply", json!({})).unwrap_err(), "Nora is already writing a reply.");
    // Nor is her text rewritten while her draft is coming.
    typed(&mut app, "Merci");
    assert!(!app.enabled("ui.noraCorrect"));
    assert_eq!(app.execute("ui.noraCorrect", json!({})).unwrap_err(), "Nora is already writing a reply.");
    assert_eq!(calls.count(), 1);
}

#[test]
fn an_answer_that_never_comes_ends_at_the_deadline() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("ui.noraReply", json!({})).unwrap();
    app.nora.age(DEADLINE_MS - 1_000.0);
    app.poll_nora();
    assert!(app.nora.asking(Kind::Reply), "still waiting");
    app.nora.age(2_000.0);
    app.poll_nora();
    assert!(!app.nora.busy());
    assert_eq!(said(&app), Some(("Nora couldn't write the reply: it took too long".into(), true, About::Writing)));
    // An answer that comes after that goes nowhere (it isn't even waited for), and she can be asked again.
    assert!(!calls.answer_late(200, json!({"message": {"draft_text": "Late"}})));
    app.poll_nora();
    assert!(!body(&app).contains("Late"));
    app.run("ui.noraReply", json!({})).unwrap();
    assert_eq!(calls.count(), 2);
}

#[test]
fn a_dropped_sender_ends_the_wait() {
    let (mut app, calls, message) = sample(PLAIN);
    app.run("ui.noraTranslate", json!({"language": "fr"})).unwrap();
    calls.drop_last();
    app.poll_nora();
    assert!(!app.nora.busy());
    let lost = "Nora couldn't be reached: the connection stopped before Nora answered";
    assert_eq!(said(&app), Some((lost.into(), true, About::Message(message))));
    assert!(app.nora.translation.is_none());
}

#[test]
fn translate_keeps_the_text_and_original_forgets_it() {
    let (mut app, calls, message) = sample(PLAIN);
    assert!(app.enabled("ui.noraTranslate") && !app.enabled("ui.noraOriginal"));
    let asked = app.run("ui.noraTranslate", json!({"language": "fr"})).unwrap();
    assert_eq!((asked["message"].as_u64(), asked["language"].as_str()), (Some(message), Some("fr")));
    let call = calls.last();
    assert_eq!(call.method, TRANSLATE);
    assert_eq!(keys(&call.args), ["account_name", "html_content", "target_language"].map(String::from).into());
    assert_eq!((call.args["target_language"].as_str(), call.args["account_name"].as_str()), (Some("fr"), Some("")));
    let html = call.args["html_content"].as_str().unwrap();
    assert!(html.starts_with("<p>Avery,</p>\n<p>The roadmap review is moving"), "{html}");
    assert_eq!(app.nora.translating(), Some(message));
    assert!(!app.enabled("ui.noraTranslate"), "one translation at a time");
    assert_eq!(app.execute("ui.noraTranslate", json!({"language": "de"})).unwrap_err(), "Nora is already translating a message.");
    calls.answer(
        200,
        json!({"message": {"translated_text": "Avery,\n\nLa revue est déplacée à jeudi.", "source_language": "en", "has_changes": true}}),
    );
    app.poll_nora();
    let t = app.nora.translation.as_ref().unwrap();
    assert_eq!((t.message, t.language), (message, "fr"));
    assert_eq!(t.shown.id, message);
    assert_eq!(t.shown.body, mailcraft_model::Body { text: "Avery,\n\nLa revue est déplacée à jeudi.".into(), html: None });
    let original = app.session.mailbox.messages.get(&message).unwrap();
    assert_eq!((&t.shown.subject, &t.shown.from), (&original.subject, &original.from), "the rest is the message's own");
    assert_eq!(status(&app), "Translated by Nora into French");
    assert!(app.enabled("ui.noraOriginal"));
    let back = app.run("ui.noraOriginal", json!({})).unwrap();
    assert_eq!(back["was"], true);
    assert!(app.nora.translation.is_none());
    assert!(app.session.mailbox.messages.get(&message).is_some_and(|m| m.body.text.starts_with("Avery,")), "the message itself never changed");
    calls.all_fit();
}

#[test]
fn a_message_already_in_that_language_is_said_so() {
    let (mut app, calls, message) = sample(PLAIN);
    app.run("ui.noraTranslate", json!({"language": "en"})).unwrap();
    calls.answer(200, json!({"message": {"translated_text": "Avery,", "source_language": "en", "target_language": "en", "has_changes": false}}));
    app.poll_nora();
    assert!(app.nora.translation.is_none());
    assert_eq!(said(&app), Some(("The message is already in English.".into(), false, About::Message(message))));
}

#[test]
fn a_refused_translation_shows_nothing() {
    let (mut app, calls, message) = sample(PLAIN);
    app.run("ui.noraTranslate", json!({"language": "de"})).unwrap();
    calls.answer(417, refusal(REFUSED));
    app.poll_nora();
    assert!(app.nora.translation.is_none());
    assert_eq!(said(&app), Some((format!("Nora couldn't translate the message: {REFUSED}"), true, About::Message(message))));
    // A translation is shown in the reading pane, which a message being written takes the place of.
    app.run("mail.new", json!({})).unwrap();
    assert!(!app.enabled("ui.noraTranslate"));
    assert!(app.execute("ui.noraTranslate", json!({"language": "de"})).is_err());
    assert_eq!(calls.count(), 1);
}

#[test]
fn translate_by_the_servers_id_sends_no_text() {
    let (mut app, calls, _) = page(true);
    app.run("ui.noraTranslate", json!({"language": "de"})).unwrap();
    assert_eq!(calls.last().method, TRANSLATE);
    assert_eq!(calls.last().args, json!({"target_language": "de", "account_name": ME, "email_id": "E1"}), "the id, nothing of the text");
}

#[test]
fn correct_sends_only_the_owners_text_and_puts_the_new_one_in_its_place() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("mail.reply", json!({})).unwrap();
    typed(&mut app, "bonjour priya,\n\njeudi me convient");
    let before = body(&app);
    app.run("ui.noraCorrect", json!({})).unwrap();
    let call = calls.last();
    assert_eq!(call.method, PROOFREAD);
    assert_eq!(call.args, json!({"html_content": "<p>bonjour priya,</p>\n<p>jeudi me convient</p>", "account_name": ""}));
    assert!(!call.args.to_string().contains("Avery Quinn") && !call.args.to_string().contains("roadmap"), "not the signature, not the original");
    assert_eq!(status(&app), "Nora is rewriting the text…");
    assert_eq!(app.nora.rewriting(), Some(super::Rewrite::Correct));
    calls.answer(200, json!({"message": {"success": true, "corrected_text": "Bonjour Priya,\n\nJeudi me convient.", "has_changes": true}}));
    app.poll_nora();
    let after = body(&app);
    assert!(after.starts_with("Bonjour Priya,\n\nJeudi me convient.\n\nAvery Quinn\nLumen Studio\n\n-----Original Message-----\n"), "{after:?}");
    assert_eq!(after.split_once("Avery Quinn").map(|(_, rest)| rest), before.split_once("Avery Quinn").map(|(_, rest)| rest), "the rest as it was");
    assert_eq!(signature(&app).as_deref(), Some("Avery Quinn\nLumen Studio"));
    assert_eq!(said(&app), Some(("Nora's version is in the message: Undo takes it back.".into(), false, About::Writing)));
    app.run("compose.undo", json!({})).unwrap();
    assert_eq!(body(&app), before, "one step takes it back");
    calls.all_fit();
}

#[test]
fn improve_and_translate_go_to_their_own_methods() {
    let (mut app, calls, _) = page(true);
    app.run("ui.noraReply", json!({})).unwrap();
    calls.answer(200, json!({"message": {"draft_text": "Hello Grace,\n\nYes."}}));
    app.poll_nora();
    app.run("ui.noraImprove", json!({})).unwrap();
    assert_eq!(calls.last().method, super::super::IMPROVE);
    assert_eq!(
        calls.last().args,
        json!({"html_content": "<p>Hello Grace,</p>\n<p>Yes.</p>", "account_name": ME}),
        "the page's mailbox, for its style"
    );
    calls.answer(200, json!({"message": {"improved_text": "Hello Grace,\n\nYes, the proof is ready.", "has_changes": true}}));
    app.poll_nora();
    assert!(body(&app).starts_with("Hello Grace,\n\nYes, the proof is ready.\n"));
    app.run("ui.noraTranslateDraft", json!({"language": "fr"})).unwrap();
    assert_eq!(
        calls.last().args,
        json!({"html_content": "<p>Hello Grace,</p>\n<p>Yes, the proof is ready.</p>", "target_language": "fr", "account_name": ME})
    );
    calls.answer(200, json!({"message": {"translated_text": "Bonjour Grace,\n\nOui, le bon à tirer est prêt.", "has_changes": true}}));
    app.poll_nora();
    assert!(body(&app).starts_with("Bonjour Grace,\n\nOui, le bon à tirer est prêt.\n"));
    calls.all_fit();
}

#[test]
fn text_typed_meanwhile_wins() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("mail.reply", json!({})).unwrap();
    typed(&mut app, "Bonjour, merci");
    app.run("ui.noraTranslateDraft", json!({"language": "de"})).unwrap();
    assert_eq!((calls.last().method, calls.last().args["target_language"].as_str()), (TRANSLATE, Some("de")));
    typed(&mut app, "Encore un mot. ");
    let before = body(&app);
    calls.answer(200, json!({"message": {"translated_text": "Guten Tag, danke", "has_changes": true}}));
    app.poll_nora();
    assert_eq!(body(&app), before);
    assert_eq!(said(&app), Some(("Your text changed meanwhile: Nora's version wasn't put in.".into(), false, About::Writing)));
    // A message closed meanwhile is not written into either.
    app.run("ui.noraCorrect", json!({})).unwrap();
    app.execute("compose.discard", json!({})).unwrap();
    app.run("mail.new", json!({})).unwrap();
    calls.answer(200, json!({"message": {"corrected_text": "Encore un mot. Bonjour, merci.", "has_changes": true}}));
    app.poll_nora();
    assert!(!body(&app).contains("Encore"));
    assert_eq!(said(&app).map(|s| s.0), Some("The message was closed before Nora's version came back.".into()));
}

#[test]
fn nothing_to_rewrite_and_nothing_to_change_are_said() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("mail.reply", json!({})).unwrap();
    assert_eq!(app.run("ui.noraImprove", json!({})).unwrap_err(), "Write your text first: Nora works on it.");
    assert_eq!(said(&app), Some(("Write your text first: Nora works on it.".into(), true, About::Writing)), "said in the band too");
    assert_eq!(calls.count(), 0, "an empty text isn't sent");
    // What is said of a message being written goes with it.
    app.execute("compose.discard", json!({})).unwrap();
    app.poll_nora();
    assert_eq!(said(&app), None, "the next message's band starts clean");
    app.run("mail.reply", json!({})).unwrap();
    typed(&mut app, &"x".repeat(8_001));
    assert_eq!(app.run("ui.noraCorrect", json!({})).unwrap_err(), "Your text is too long for Nora: 8000 characters at most.");
    assert_eq!(calls.count(), 0, "nor one longer than she takes");
    app.execute("compose.select", json!({"from": 0, "to": 8_001})).unwrap();
    app.execute("compose.type", json!({"text": "Merci."})).unwrap();
    app.run("ui.noraImprove", json!({})).unwrap();
    calls.answer(200, json!({"message": {"improved_text": "Merci.", "has_changes": false}}));
    app.poll_nora();
    assert_eq!(said(&app), Some(("Nora found nothing to change.".into(), false, About::Writing)));
    app.run("ui.noraTranslateDraft", json!({"language": "de"})).unwrap();
    calls.answer(200, json!({"message": {"translated_text": "Merci.", "source_language": "de", "has_changes": false}}));
    app.poll_nora();
    assert_eq!(said(&app).map(|s| s.0), Some("The message is already in German.".into()));
    // What the server cut is not put over the text.
    app.run("ui.noraCorrect", json!({})).unwrap();
    calls.answer(200, json!({"message": {"corrected_text": "Merci.\n\n[... text truncated for processing]", "has_changes": true}}));
    app.poll_nora();
    assert!(body(&app).starts_with("Merci.\n"));
    assert_eq!(said(&app), Some(("Nora couldn't rewrite the text: the server cut the text short".into(), true, About::Writing)));
}

#[test]
fn a_rewrite_is_refused_while_a_signature_is_open() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("mail.reply", json!({})).unwrap();
    typed(&mut app, "merci");
    app.execute("signature.open", json!({"name": "Standard"})).unwrap();
    assert!(!app.enabled("ui.noraCorrect") && !app.enabled("ui.noraImprove") && !app.enabled("ui.noraTranslateDraft"));
    assert!(app.execute("ui.noraCorrect", json!({})).is_err());
    assert_eq!(calls.count(), 0);
    app.execute("signature.close", json!({})).unwrap();
    // One asked before a signature is opened waits for it to be closed: the commands that write would write in it.
    app.run("ui.noraCorrect", json!({})).unwrap();
    app.execute("signature.open", json!({"name": "Standard"})).unwrap();
    let signature_text = app.session.signature_edit.as_ref().map(|e| e.body.plain());
    calls.answer(200, json!({"message": {"corrected_text": "Merci.", "has_changes": true}}));
    app.poll_nora();
    assert!(app.nora.asking(Kind::Rewrite), "it waits");
    assert_eq!(app.session.signature_edit.as_ref().map(|e| e.body.plain()), signature_text, "nothing went into the signature");
    app.execute("signature.close", json!({})).unwrap();
    app.poll_nora();
    assert!(body(&app).starts_with("Merci.\n"), "{:?}", body(&app));
}

#[test]
fn my_style_is_for_the_pages_mailboxes_only() {
    let (mut app, calls, _) = sample(PLAIN);
    app.run("mail.reply", json!({})).unwrap();
    assert!(!app.enabled("ui.noraLearnStyle"), "the sample mailbox has no style on the server");
    assert_eq!(app.run("ui.noraLearnStyle", json!({})).unwrap_err(), "Nora works on the mailboxes of your Neoffice account only.");
    assert_eq!(calls.count(), 0);
    let (mut app, _, account) = page(true);
    assert!(!app.enabled("ui.noraLearnStyle"), "nothing is being written");
    app.run("mail.new", json!({"account": account})).unwrap();
    assert!(app.enabled("ui.noraLearnStyle"));
}

#[test]
fn my_style_starts_and_says_how_it_went() {
    let (mut app, calls, account) = page(true);
    app.run("mail.new", json!({"account": account})).unwrap();
    let mut ask = |code: u16, body: Value| {
        app.run("ui.noraLearnStyle", json!({})).unwrap();
        assert_eq!((calls.last().method, calls.last().args), (SCAN_STYLE, json!({"account_name": ME})));
        assert_eq!(status(&app), "Starting style analysis…");
        // A second click while the server answers asks nothing more.
        let n = calls.count();
        assert_eq!(app.run("ui.noraLearnStyle", json!({})).unwrap_err(), "Nora is already learning your writing style.");
        assert_eq!(calls.count(), n);
        calls.answer(code, body);
        app.poll_nora();
        said(&app).unwrap()
    };
    let started = ask(200, json!({"message": {"success": true, "status": "Scanning"}}));
    assert_eq!(started, ("Nora is learning your writing style from your sent emails. This takes a minute.".into(), false, About::Writing));
    let running = ask(200, json!({"message": {"success": true, "status": "Scanning", "already_running": true}}));
    assert_eq!(running, ("Nora is already learning your writing style.".into(), false, About::Writing));
    let refused = ask(417, refusal("Access denied"));
    assert_eq!(refused, ("Nora couldn't learn your style: Access denied".into(), true, About::Writing));
}

#[test]
fn without_a_host_nothing_is_offered() {
    let mut app = quiet(MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default()));
    let id = app.session.mailbox.messages.values().find(|m| m.subject == PLAIN).map(|m| m.id).unwrap();
    app.execute("mail.select", json!({"id": id})).unwrap();
    app.run("mail.reply", json!({})).unwrap();
    for (command, ..) in crate::UI_COMMANDS.iter().filter(|c| c.0.starts_with("ui.nora")) {
        assert!(!app.enabled(command), "{command}");
        if *command != "ui.noraOriginal" {
            assert!(app.execute(command, json!({"language": "fr"})).is_err(), "{command}");
        }
    }
    assert!(!app.nora.busy());
}

/// Junk for every command, in every state, through `execute` and `run`: refused or done, never a panic, and never more
/// than one call of each kind.
#[test]
fn the_commands_take_anything_and_never_panic() {
    let junk = [
        json!(null),
        json!([]),
        json!("text"),
        json!(7),
        json!({}),
        json!({"message": "x"}),
        json!({"message": -1}),
        json!({"message": 0}),
        json!({"message": u64::MAX}),
        json!({"message": 1e300}),
        json!({"message": {"id": 1}}),
        json!({"all": "yes"}),
        json!({"all": 1}),
        json!({"language": null}),
        json!({"language": 5}),
        json!({"language": ""}),
        json!({"language": "FR"}),
        json!({"language": "fr\r\nX-Injected: 1"}),
        json!({"language": "x".repeat(10_000)}),
        json!({"language": ["fr"]}),
        json!({"language": "de", "message": 3, "all": true}),
    ];
    let commands: Vec<&str> = crate::UI_COMMANDS.iter().map(|c| c.0).filter(|id| id.starts_with("ui.nora")).collect();
    assert_eq!(commands.len(), 7);
    let states: [fn() -> (MailApp, Calls); 3] = [
        || {
            let (mut app, calls, _) = sample(PLAIN);
            app.execute("mail.select", json!({"ids": []})).unwrap();
            (app, calls)
        },
        || {
            let (app, calls, _) = sample(PLAIN);
            (app, calls)
        },
        || {
            let (mut app, calls, _) = page(true);
            app.run("mail.reply", json!({})).unwrap();
            typed(&mut app, "Hello");
            (app, calls)
        },
    ];
    for state in states {
        let (mut app, calls) = state();
        for command in &commands {
            for params in &junk {
                let _ = app.execute(command, params.clone());
                let _ = app.run(command, params.clone());
                app.poll_nora();
            }
        }
        calls.all_fit();
        assert!(app.nora.flights.len() <= 4);
    }
}

#[test]
fn the_answers_of_a_hostile_host_are_only_ever_words() {
    // Whatever a host hands back for a draft (a page's own HTML, a script, nothing at all), the reply keeps its
    // signature and the person is told something short.
    for (status, body) in [
        (200, "<html><script>alert(1)</script></html>".to_string()),
        (0, "x".repeat(5_000_000)),
        (200, json!({"message": {"draft_text": ["not", "text"]}}).to_string()),
        (200, json!({"message": {"draft_text": "\u{0}\u{1b}[2J\u{202e}"}}).to_string()),
        (999, String::new()),
    ] {
        let (mut app, calls, _) = sample(PLAIN);
        app.run("ui.noraReply", json!({})).unwrap();
        {
            let all = calls.0.borrow();
            let (call, to) = all.last().unwrap();
            to.as_ref().unwrap().send(NoraAnswer { id: call.id, status, body }).unwrap();
        }
        app.poll_nora();
        assert_eq!(signature(&app).as_deref(), Some("Avery Quinn\nLumen Studio"));
        let (text, ..) = said(&app).unwrap();
        assert!(text.chars().count() < 600 && !text.chars().any(char::is_control), "{text:?}");
    }
    // A draft of control characters alone pastes nothing harmful: they are not text the body takes.
    let (mut app, calls, _) = sample(PLAIN);
    app.run("ui.noraReply", json!({})).unwrap();
    calls.answer(200, json!({"message": {"draft_text": "Bonjour\u{0}\u{7} Priya\r\n\r\nMerci"}}));
    app.poll_nora();
    assert!(body(&app).starts_with("Bonjour Priya\n\nMerci\n"), "{:?}", body(&app));
}
