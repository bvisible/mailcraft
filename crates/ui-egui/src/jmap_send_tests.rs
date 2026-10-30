// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's Outbox and drafts in the window: a message written here goes with the run that starts by
//! itself, its copy shows in Sent, a refusal holds it until the owner sends it again, an answer that was lost
//! holds it for good, and a draft is saved by itself and filed. The host answers by hand. Invented people at
//! `example.org`.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use mailcraft_engine::Session;
use mailcraft_model::sync::{
    FolderListing, GraphFolder, GraphMessage, JmapDraftDone, JmapMailRequest, JmapMailResult, JmapScope, MessagePage, SendDone, SendOutcome,
    ServerMessage,
};
use mailcraft_model::{Address, FolderKind, Id, Mailbox};
use serde_json::json;

use super::JmapJob;
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const ME: &str = "ada@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session?account=ada@example.org";

/// One run the host was asked for, and the way to answer it.
struct Run {
    request: JmapMailRequest,
    to: Sender<JmapMailResult>,
}

type Asked = Arc<Mutex<Vec<Run>>>;

/// A window with a JMAP account whose host holds the credential, rounds that start by themselves, and a first
/// run done: the folders are the server's. The account's id and the runs the host is asked for from now on.
fn window() -> (MailApp, Id, Asked) {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = true;
    app.session.prefs.sync_minutes = 5;
    let asked: Asked = Default::default();
    let log = asked.clone();
    app.services.jmap = Some(Box::new(move |job: JmapJob| {
        let (tx, rx) = mpsc::channel();
        log.lock().unwrap().push(Run { request: job.request, to: tx });
        rx
    }));
    app.services.jmap_host_credentials = true;
    let id = app.execute("account.add", json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    // The first run: the folders.
    app.execute("ui.sync", json!({"account": id})).unwrap();
    let folder = |id: &str, name: &str, kind: FolderKind| GraphFolder { id: id.into(), name: name.into(), kind, ..Default::default() };
    let folders = vec![
        folder("Min", "INBOX", FolderKind::Inbox),
        folder("Mdr", "Drafts", FolderKind::Drafts),
        folder("Mse", "Sent", FolderKind::Sent),
        folder("Mtr", "Trash", FolderKind::Deleted),
    ];
    answer(&asked, 0, JmapMailResult { listing: Some(FolderListing { folders, complete: true }), ..piece(true) });
    answer(&asked, 0, JmapMailResult { whole: true, ..piece(false) });
    app.poll_jmap();
    assert!(!app.syncing());
    // The timed round is not due: only what the window finds waiting starts a run, unless a test says otherwise.
    app.last_auto_sync = crate::now_ms();
    (app, id, asked)
}

fn piece(partial: bool) -> JmapMailResult {
    JmapMailResult { email: ME.into(), partial, ..Default::default() }
}

fn answer(asked: &Asked, run: usize, piece: JmapMailResult) {
    asked.lock().unwrap()[run].to.send(piece).unwrap();
}

fn request(asked: &Asked, run: usize) -> JmapMailRequest {
    asked.lock().unwrap()[run].request.clone()
}

fn jobs(asked: &Asked) -> usize {
    asked.lock().unwrap().len()
}

/// The automatic rounds, until the one that sends what waits has begun (the window looks every few seconds).
fn round(app: &mut MailApp) {
    app.push_checked = 0.0;
    app.auto_sync();
    app.auto_sync();
}

/// Write a message in the account and press Send: its id in the Outbox.
fn send(app: &mut MailApp, account: Id, subject: &str) -> Id {
    app.execute("mail.new", json!({"to": "bob@example.org", "subject": subject, "body": "Hi", "account": account})).unwrap();
    app.run("compose.send", json!({})).unwrap()["id"].as_u64().unwrap()
}

fn count(app: &MailApp, account: Id, kind: FolderKind) -> usize {
    app.session.mailbox.folder_of_kind(account, kind).map_or(0, |f| app.session.mailbox.messages_in(f).count())
}

/// What the server holds in Sent after it sent a message: the copy, in a window of its own.
fn sent_window(subject: &str) -> JmapMailResult {
    let copy = GraphMessage {
        server: ServerMessage { id: "S1".into(), folder: "Mse".into(), read: true, size: 100, ..Default::default() },
        message_id: format!("<{subject}@example.org>"),
        subject: subject.into(),
        from: Address::new("Ada", ME),
        to: vec![Address::new("Bob", "bob@example.org")],
        sent: NOW,
        received: NOW,
        preview: "Hi".into(),
        ..Default::default()
    };
    JmapMailResult { pages: vec![MessagePage { folder: "Mse".into(), messages: vec![copy], afresh: true, ..Default::default() }], ..piece(true) }
}

fn sent_as(message: Id, outcome: SendOutcome) -> JmapMailResult {
    JmapMailResult { sent: vec![SendDone { message, outcome, draft: None, draft_left: String::new() }], ..piece(true) }
}

// ---------------------------------------------------------------------------------------------------------

#[test]
fn a_message_written_here_goes_with_the_next_run_and_its_copy_shows_in_sent() {
    let (mut app, id, asked) = window();
    let message = send(&mut app, id, "Hello");
    // Send queued it: it is in the Outbox, the window is not told it can't, and the account has something to send.
    assert_eq!((count(&app, id, FolderKind::Outbox), count(&app, id, FolderKind::Sent)), (1, 0));
    assert!(app.session.compose.is_none());
    assert!(!app.status_msg.as_ref().is_some_and(|(text, _)| text.contains("isn't available")), "{:?}", app.status_msg);
    assert_eq!(app.waiting_to_go(id), 1);

    // A run begins by itself, for the account, with the message; it reads the Inbox and the Sent mailbox.
    round(&mut app);
    assert_eq!(jobs(&asked), 2);
    let asks = request(&asked, 1);
    assert_eq!(asks.sends.iter().map(|send| (send.message, send.mime.subject.as_str())).collect::<Vec<_>>(), [(message, "Hello")]);
    assert_eq!((asks.background, asks.list_folders, asks.scope.clone()), (true, false, JmapScope::Light { also: vec!["Mse".into()] }));
    assert!(app.syncing());

    // The host sends it, files its copy in Sent, and reads the window of Sent.
    answer(&asked, 1, sent_as(message, SendOutcome::Sent));
    answer(&asked, 1, sent_window("Hello"));
    answer(&asked, 1, JmapMailResult { whole: true, ..piece(false) });
    app.poll_jmap();
    assert!(!app.syncing());
    assert_eq!((count(&app, id, FolderKind::Outbox), count(&app, id, FolderKind::Sent)), (0, 1));
    assert_eq!(app.waiting_to_go(id), 0);
    let sent = app.session.mailbox.folder_of_kind(id, FolderKind::Sent).unwrap();
    assert_eq!(app.session.mailbox.messages_in(sent).next().map(|m| m.subject.clone()), Some("Hello".to_string()));
    // Nothing waits, so nothing starts; and a timed round carries nothing to send again.
    round(&mut app);
    assert_eq!(jobs(&asked), 2);
    app.last_auto_sync = 0.0;
    round(&mut app);
    assert!(request(&asked, 2).sends.is_empty());
}

#[test]
fn a_message_the_server_refused_stays_held_in_the_outbox_until_the_owner_sends_it_again() {
    let (mut app, id, asked) = window();
    let message = send(&mut app, id, "Hello");
    round(&mut app);
    answer(&asked, 1, sent_as(message, SendOutcome::Held { lost: false, why: "the server doesn't accept bob@example.org".into() }));
    answer(&asked, 1, JmapMailResult { whole: true, ..piece(false) });
    app.poll_jmap();
    assert!(!app.syncing());
    // It is where it was, with what happened, and the owner is told.
    let m = app.session.mailbox.message(message).unwrap();
    assert_eq!(m.held, "It wasn't sent: the server doesn't accept bob@example.org. Change it, or send it again.");
    assert_eq!(count(&app, id, FolderKind::Outbox), 1);
    // (What the status line and the problems report say leaves the address out: they go to the log as well.)
    assert!(app.session.sync_status.starts_with("Couldn't send 'Hello': the server doesn't accept (address)."), "{}", app.session.sync_status);
    assert!(app.sync_problem_lines().iter().any(|line| line.contains("Couldn't send 'Hello'")), "{:?}", app.sync_problem_lines());
    // Nothing waits for a run, and no round takes it along.
    assert_eq!(app.waiting_to_go(id), 0);
    for _ in 0..3 {
        app.last_auto_sync = 0.0;
        round(&mut app);
        if app.syncing() {
            let run = jobs(&asked) - 1;
            assert!(request(&asked, run).sends.is_empty(), "held mail is not sent by itself");
            answer(&asked, run, JmapMailResult { whole: true, ..piece(false) });
            app.poll_jmap();
        }
    }
    // Send Again is the owner's say-so: it goes with the next run.
    app.run("mail.sendAgain", json!({"id": message})).unwrap();
    assert_eq!(app.waiting_to_go(id), 1);
    app.last_auto_sync = 0.0;
    round(&mut app);
    let last = jobs(&asked) - 1;
    assert_eq!(request(&asked, last).sends.iter().map(|send| send.message).collect::<Vec<_>>(), [message]);
}

#[test]
fn a_message_whose_answer_was_lost_is_held_as_one_that_may_have_gone_and_no_round_sends_it_again() {
    let (mut app, id, asked) = window();
    let message = send(&mut app, id, "Hello");
    round(&mut app);
    answer(&asked, 1, sent_as(message, SendOutcome::Held { lost: true, why: String::new() }));
    // (The copy the server filed first is in Sent all the same.)
    answer(&asked, 1, sent_window("Hello"));
    answer(&asked, 1, JmapMailResult { whole: true, ..piece(false) });
    app.poll_jmap();
    let m = app.session.mailbox.message(message).unwrap();
    assert!(m.held.starts_with("It may have been sent"), "{}", m.held);
    assert!(app.session.sync_status.contains("may have been sent"), "{}", app.session.sync_status);
    assert_eq!((count(&app, id, FolderKind::Outbox), count(&app, id, FolderKind::Sent)), (1, 1));
    for _ in 0..4 {
        app.last_auto_sync = 0.0;
        round(&mut app);
        if app.syncing() {
            let run = jobs(&asked) - 1;
            assert!(request(&asked, run).sends.is_empty());
            answer(&asked, run, JmapMailResult { whole: true, ..piece(false) });
            app.poll_jmap();
        }
    }
}

#[test]
fn a_message_that_could_not_be_tried_waits_and_is_not_asked_for_every_few_seconds() {
    let (mut app, id, asked) = window();
    let message = send(&mut app, id, "Hello");
    round(&mut app);
    // The host could not reach the server: nothing is said of the message, only of the run.
    answer(&asked, 1, JmapMailResult { errors: vec!["The server couldn't be reached.".into()], partial: true, ..piece(true) });
    answer(&asked, 1, JmapMailResult { whole: true, ..piece(false) });
    app.poll_jmap();
    assert!(app.session.mailbox.message(message).unwrap().held.is_empty());
    assert_eq!(app.waiting_to_go(id), 1);
    // It is not asked for again at once: what didn't go through is left for a few minutes.
    app.push_checked = 0.0;
    app.auto_sync();
    app.auto_sync();
    assert_eq!(jobs(&asked), 2, "no run starts for it again at once");
}

// ---------------------------------------------------------------------------------------------------------

#[test]
fn a_draft_saved_here_is_filed_by_the_next_run_and_replaced_by_the_one_after() {
    let (mut app, id, asked) = window();
    app.execute("mail.new", json!({"to": "bob@example.org", "subject": "Later", "body": "Hi", "account": id})).unwrap();
    let draft = app.run("compose.saveDraft", json!({})).unwrap()["draft"].as_u64().unwrap();
    assert_eq!(app.waiting_to_go(id), 1, "a run is worth starting for it");
    round(&mut app);
    let asks = request(&asked, 1);
    assert_eq!(asks.drafts.iter().map(|d| (d.message, d.saves, d.replaces.as_str())).collect::<Vec<_>>(), [(draft, 1, "")]);
    assert_eq!(asks.scope, JmapScope::Light { also: vec!["Mdr".into()] });
    answer(
        &asked,
        1,
        JmapMailResult {
            drafts: vec![JmapDraftDone { message: draft, saves: 1, id: "E5".into(), folder: "Mdr".into(), refused: None }],
            ..piece(true)
        },
    );
    answer(&asked, 1, JmapMailResult { whole: true, ..piece(false) });
    app.poll_jmap();
    assert_eq!(app.waiting_to_go(id), 0);
    assert_eq!(app.session.mailbox.message(draft).unwrap().server.as_ref().map(|s| s.id.as_str()), Some("E5"));

    // Edited and saved again: the next run files it in place of the copy the server has.
    app.run("mail.editDraft", json!({"id": draft})).unwrap();
    app.run("compose.set", json!({"subject": "Later, edited"})).unwrap();
    app.run("compose.saveDraft", json!({})).unwrap();
    app.last_auto_sync = 0.0;
    round(&mut app);
    let asks = request(&asked, 2);
    assert_eq!(asks.drafts.iter().map(|d| (d.saves, d.replaces.as_str(), d.mime.subject.as_str())).collect::<Vec<_>>(), [(2, "E5", "Later, edited")]);
}

#[test]
fn a_draft_being_written_is_saved_by_itself_every_five_minutes_while_it_has_changes() {
    let (mut app, id, _) = window();
    let ctx = egui::Context::default();
    app.execute("mail.new", json!({"to": "bob@example.org", "subject": "Slowly", "account": id})).unwrap();
    let drafts = app.session.mailbox.folder_of_kind(id, FolderKind::Drafts).unwrap();
    let kept = |app: &MailApp| app.session.mailbox.messages_in(drafts).count();
    // Nothing changed since the window opened it: nothing to save.
    app.draft_saved_at = crate::now_ms() - 6.0 * 60_000.0;
    app.draft_autosave_tick(&ctx);
    assert_eq!(kept(&app), 0);
    // Changed, and the five minutes are over.
    app.run("compose.set", json!({"body": "More words"})).unwrap();
    assert!(app.session.compose.as_ref().is_some_and(|c| c.dirty));
    app.draft_saved_at = crate::now_ms() - 60_000.0;
    app.draft_autosave_tick(&ctx);
    assert_eq!(kept(&app), 0, "not yet");
    app.draft_saved_at = crate::now_ms() - 6.0 * 60_000.0;
    app.draft_autosave_tick(&ctx);
    assert_eq!(kept(&app), 1);
    assert_eq!(app.waiting_to_go(id), 1, "and it goes up with the next run");
    // An account whose server keeps no drafts of ours is not saved for by itself.
    let imap =
        app.execute("account.add", json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    app.run("compose.discard", json!({})).unwrap();
    app.execute("mail.new", json!({"to": "carol@example.org", "subject": "Elsewhere", "account": imap})).unwrap();
    app.run("compose.set", json!({"body": "Words"})).unwrap();
    app.draft_saved_at = crate::now_ms() - 6.0 * 60_000.0;
    app.draft_autosave_tick(&ctx);
    let imap_drafts = app.session.mailbox.folder_of_kind(imap, FolderKind::Drafts).unwrap();
    assert_eq!(app.session.mailbox.messages_in(imap_drafts).count(), 0);
}
