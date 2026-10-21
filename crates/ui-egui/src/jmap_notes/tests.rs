// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's notes in the window's jobs: when they are asked for, when a change of them goes, and how the answer
//! is taken in. A host that keeps every job for the test to answer by hand. Invented people at `example.org`.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use mailcraft_engine::Session;
use mailcraft_model::note::{NoteOutcome, NoteResult, NotesAnswer, NotesDownload, NotesUploaded, RemoteNote};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult, PimOffers};
use mailcraft_model::{Id, Mailbox};
use serde_json::json;

use crate::jmap_pim::JmapPimJob;
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const ME: &str = "ada@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session";
const SECOND: f64 = 1_000.0;
const MINUTE: f64 = 60.0 * SECOND;
/// When the tests begin, by the window's clock (milliseconds).
const T0: f64 = 1_000_000.0;

type Asked = Arc<Mutex<Vec<(JmapPimRequest, Sender<JmapPimResult>)>>>;

/// A window with a JMAP account whose credential the host holds, Mail in front, and a host that keeps every job it is
/// handed. The account's id.
fn window() -> (MailApp, Id, Asked) {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.services.jmap = Some(Box::new(|_| mpsc::channel().1));
    app.services.jmap_host_credentials = true;
    let asked: Asked = Default::default();
    let log = asked.clone();
    app.services.jmap_pim = Some(Box::new(move |job: JmapPimJob| {
        let (tx, rx) = mpsc::channel();
        log.lock().unwrap().push((job.request, tx));
        rx
    }));
    let added = app.execute("account.add", json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}}));
    let id = added.unwrap()["id"].as_u64().unwrap();
    app.execute("view.mail", json!({})).unwrap();
    (app, id, asked)
}

fn jobs(asked: &Asked) -> usize {
    asked.lock().unwrap().len()
}

fn last(asked: &Asked) -> JmapPimRequest {
    asked.lock().unwrap().last().unwrap().0.clone()
}

/// Answer the last job with `notes` (and a session that offers calendars and contacts), and take it in.
fn answer(app: &mut MailApp, asked: &Asked, account: Id, notes: Option<NotesAnswer>) {
    let to = asked.lock().unwrap().last().unwrap().1.clone();
    let result =
        JmapPimResult { account, email: ME.into(), offers: Some(PimOffers { calendars: true, contacts: true }), notes, ..Default::default() };
    to.send(result).unwrap();
    app.poll_jmap_pim();
}

fn remote(id: &str, title: &str) -> RemoteNote {
    RemoteNote {
        id: id.into(),
        title: title.into(),
        text: format!("{title} words"),
        version: "2026-10-14T15:00:00.123Z".into(),
        ..Default::default()
    }
}

/// A server's notes, read whole at `state`.
fn read(account: Id, notes: Vec<RemoteNote>, state: &str) -> NotesAnswer {
    let download = NotesDownload { account, email: ME.into(), offered: true, complete: true, notes, ..Default::default() };
    NotesAnswer { download: Some(download), state: Some(state.into()), ..Default::default() }
}

/// A window whose first job read the server's two notes, "Offsite" and "Groceries", at state "s1"; the time it was
/// answered.
fn read_once() -> (MailApp, Id, Asked) {
    let (mut app, id, asked) = window();
    assert!(app.jmap_pim_tick_at(true, T0));
    answer(&mut app, &asked, id, Some(read(id, vec![remote("VN-1", "Offsite"), remote("VN-2", "Groceries")], "s1")));
    (app, id, asked)
}

#[test]
fn notes_are_asked_with_the_wide_job_and_at_once_when_notes_comes_to_the_front() {
    let (mut app, id, asked) = window();
    assert!(app.jmap_pim_tick_at(true, T0));
    let first = last(&asked).notes.expect("the first job asks for the notes");
    assert!(first.read && first.state.is_empty() && first.changes.is_none());
    answer(&mut app, &asked, id, Some(read(id, vec![remote("VN-1", "Offsite")], "s1")));
    assert_eq!(app.session.mailbox.notes.items.len(), 1);
    assert_eq!(app.jmap_notes_state(id), Some(("s1".into(), NOW)));
    // Mail in front: nothing more for a while.
    assert!(!app.jmap_pim_tick_at(true, T0 + MINUTE));
    // Notes comes to the front: at once, with the state held.
    app.execute("view.notes", json!({})).unwrap();
    assert!(app.jmap_pim_tick_at(true, T0 + MINUTE + SECOND));
    let ask = last(&asked).notes.expect("notes asked for");
    assert!(ask.read && ask.state == "s1" && ask.read_at == NOW);
    answer(&mut app, &asked, id, Some(NotesAnswer { state: Some("s1".into()), ..Default::default() }));
    // Then every two minutes while it is in front.
    let before = jobs(&asked);
    assert!(!app.jmap_pim_tick_at(true, T0 + 2.0 * MINUTE));
    assert!(app.jmap_pim_tick_at(true, T0 + 3.0 * MINUTE + 2.0 * SECOND));
    assert_eq!(jobs(&asked), before + 1);
    assert!(last(&asked).notes.is_some_and(|ask| ask.read));
}

#[test]
fn a_note_change_goes_after_three_quiet_seconds_and_not_the_same_twice_within_half_a_minute() {
    let (mut app, id, asked) = read_once();
    let made = app.execute("notes.new", json!({"title": "Lunch"})).unwrap()["id"].as_u64().unwrap();
    let t = T0 + 5.0 * SECOND;
    assert!(!app.jmap_pim_tick_at(true, t), "the mailbox just changed");
    assert!(!app.jmap_pim_tick_at(true, t + SECOND));
    assert!(app.jmap_pim_tick_at(true, t + 3.0 * SECOND));
    let ask = last(&asked).notes.expect("the change goes");
    let ops = ask.changes.expect("with the change").ops;
    assert_eq!((ops.len(), ops[0].note), (1, made));
    assert!(!ask.read, "Mail in front: the notes are not read again for it");
    // The very same changes don't go again within half a minute.
    assert!(app.notes_push(id, t + 13.0 * SECOND).is_none());
    assert!(app.notes_push(id, t + 34.0 * SECOND).is_some());
    // A change made meanwhile is another upload: it may go.
    app.execute("notes.edit", json!({"id": made, "text": "Friday"})).unwrap();
    app.notes_look(true, t + 14.0 * SECOND);
    assert!(app.notes_push(id, t + 18.0 * SECOND).is_some());
}

#[test]
fn the_answer_takes_the_outcomes_then_the_notes_and_keeps_the_state() {
    let (mut app, id, asked) = read_once();
    let made = app.execute("notes.new", json!({"title": "Lunch"})).unwrap()["id"].as_u64().unwrap();
    app.notes_look(true, T0 + SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 5.0 * SECOND));
    let op = last(&asked).notes.unwrap().changes.unwrap().ops[0].clone();
    // The server made it; the read after it has it too, at a new state.
    let done = NoteOutcome { op, result: NoteResult::Done, note: Some(remote("VN-9", "Lunch")), ..Default::default() };
    let uploaded = NotesUploaded { account: id, email: ME.into(), outcomes: vec![done], ..Default::default() };
    let mut notes = read(id, vec![remote("VN-1", "Offsite"), remote("VN-2", "Groceries"), remote("VN-9", "Lunch")], "s2");
    notes.uploaded = Some(uploaded);
    answer(&mut app, &asked, id, Some(notes));
    let lunch: Vec<u64> = app.session.mailbox.notes.items.values().filter(|note| note.title == "Lunch").map(|note| note.id).collect();
    assert_eq!(lunch, [made], "one note, the one made here");
    assert_eq!(app.session.mailbox.notes.note(made).map(|note| note.origin.id.as_str()), Some("VN-9"));
    assert!(app.session.mailbox.notes.changes.is_empty());
    assert_eq!(app.jmap_notes_state(id), Some(("s2".into(), NOW)));
    // What the engine says of a change the server refused goes to the status bar.
    let groceries = app.session.mailbox.notes.items.values().find(|note| note.origin.id == "VN-2").map(|note| note.id).unwrap();
    app.execute("notes.edit", json!({"id": groceries, "title": "Shopping"})).unwrap();
    app.notes_look(true, T0 + MINUTE);
    assert!(app.jmap_pim_tick_at(true, T0 + MINUTE + 4.0 * SECOND));
    let op = last(&asked).notes.unwrap().changes.unwrap().ops[0].clone();
    let refused = NoteOutcome { op, result: NoteResult::Refused, error: "Not permitted".into(), ..Default::default() };
    let notes = NotesAnswer {
        uploaded: Some(NotesUploaded { account: id, email: ME.into(), outcomes: vec![refused], ..Default::default() }),
        ..Default::default()
    };
    answer(&mut app, &asked, id, Some(notes));
    let said = app.status_msg.clone().map(|(text, _)| text).unwrap_or_default();
    assert!(said.contains("kept as a note of its own"), "{said}");
    assert_eq!(app.session.mailbox.notes.note(groceries).map(|note| note.title.as_str()), Some("Groceries"));
    assert!(app.session.mailbox.notes.items.values().any(|note| note.title == "Shopping (2)"), "her title, kept");
    assert_eq!(app.jmap_notes_state(id), None, "a job that brought no state forgets the one held");
}

#[test]
fn an_account_that_offers_no_notes_is_not_asked_for_them_and_its_notes_go() {
    let (mut app, id, asked) = read_once();
    assert!(app.session.mailbox.notes.server(id).is_some());
    // The server no longer offers them (the next wide job says so): its notes go.
    assert!(app.jmap_pim_tick_at(true, T0 + 11.0 * MINUTE));
    assert!(last(&asked).notes.is_some_and(|ask| ask.read));
    let gone = NotesDownload { account: id, email: ME.into(), offered: false, ..Default::default() };
    answer(&mut app, &asked, id, Some(NotesAnswer { download: Some(gone), ..Default::default() }));
    assert!(app.session.mailbox.notes.server(id).is_none() && app.session.mailbox.notes.items.is_empty());
    // Notes in front (the mailbox keeps its own here) asks nothing of that account's notes; the calendar's jobs go on
    // without them until the next wide one.
    app.execute("view.notes", json!({})).unwrap();
    app.execute("view.calendar", json!({})).unwrap();
    assert!(app.jmap_pim_tick_at(true, T0 + 12.0 * MINUTE));
    assert!(last(&asked).notes.is_none(), "{:?}", last(&asked).notes);
}

#[test]
fn a_job_that_carried_note_changes_and_says_nothing_of_them_came_to_nothing() {
    let (mut app, id, asked) = read_once();
    app.execute("notes.new", json!({"title": "Lunch"})).unwrap();
    app.notes_look(true, T0 + SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 5.0 * SECOND));
    assert!(last(&asked).notes.is_some_and(|ask| ask.changes.is_some()));
    // The answer says nothing of the notes.
    answer(&mut app, &asked, id, None);
    // Another change, ready to go: the server is left alone half a minute first.
    app.execute("notes.new", json!({"title": "Dinner"})).unwrap();
    app.notes_look(true, T0 + 6.0 * SECOND);
    let before = jobs(&asked);
    assert!(!app.jmap_pim_tick_at(true, T0 + 10.0 * SECOND));
    assert!(!app.jmap_pim_tick_at(true, T0 + 30.0 * SECOND));
    assert!(app.jmap_pim_tick_at(true, T0 + 36.0 * SECOND));
    assert_eq!(jobs(&asked), before + 1);
    assert_eq!(last(&asked).notes.and_then(|ask| ask.changes).map(|changes| changes.ops.len()), Some(2), "both go");
}

#[test]
fn a_hosting_tab_says_a_note_change_would_be_lost_while_it_waits() {
    let (mut app, id, asked) = read_once();
    app.execute("notes.new", json!({"title": "Lunch"})).unwrap();
    assert!(!app.notes_unsent(), "this computer keeps it: it goes later");
    app.session.keeps_local = false;
    assert!(app.notes_unsent(), "a hosting page's tab keeps nothing between visits");
    app.notes_look(true, T0 + SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 5.0 * SECOND));
    let op = last(&asked).notes.unwrap().changes.unwrap().ops[0].clone();
    let done = NoteOutcome { op, result: NoteResult::Done, note: Some(remote("VN-9", "Lunch")), ..Default::default() };
    let notes = NotesAnswer {
        uploaded: Some(NotesUploaded { account: id, email: ME.into(), outcomes: vec![done], ..Default::default() }),
        ..Default::default()
    };
    answer(&mut app, &asked, id, Some(notes));
    assert!(!app.notes_unsent(), "the server has it");
}

#[test]
fn a_new_note_the_server_refused_would_be_lost_with_a_hosting_page() {
    let (mut app, id, asked) = read_once();
    app.session.keeps_local = false;
    app.execute("notes.new", json!({"title": "Lunch"})).unwrap();
    app.notes_look(true, T0 + SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 5.0 * SECOND));
    let op = last(&asked).notes.unwrap().changes.unwrap().ops[0].clone();
    let refused = NoteOutcome { op, result: NoteResult::Refused, error: "Not permitted".into(), ..Default::default() };
    let notes = NotesAnswer {
        uploaded: Some(NotesUploaded { account: id, email: ME.into(), outcomes: vec![refused], ..Default::default() }),
        ..Default::default()
    };
    answer(&mut app, &asked, id, Some(notes));
    assert!(app.session.mailbox.notes.changes.is_empty());
    assert!(app.notes_unsent(), "kept here with why, and nowhere else");
}

#[test]
fn a_change_the_server_did_not_take_has_the_notes_read_again_soon() {
    let (mut app, id, asked) = read_once();
    let groceries = app.session.mailbox.notes.items.values().find(|note| note.origin.id == "VN-2").map(|note| note.id).unwrap();
    app.execute("notes.edit", json!({"id": groceries, "text": "Mine"})).unwrap();
    app.notes_look(true, T0 + SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 5.0 * SECOND));
    let op = last(&asked).notes.unwrap().changes.unwrap().ops[0].clone();
    let stale = NoteOutcome { op, result: NoteResult::Stale, error: "changed elsewhere".into(), ..Default::default() };
    let mut notes = NotesAnswer {
        uploaded: Some(NotesUploaded { account: id, email: ME.into(), outcomes: vec![stale], ..Default::default() }),
        ..Default::default()
    };
    notes.state = Some("s2".into());
    answer(&mut app, &asked, id, Some(notes));
    assert_eq!(app.jmap_notes_state(id), None, "the state held is the server's no longer");
    // The copy of her words goes, and the notes are read with it, whole.
    app.notes_look(true, T0 + 6.0 * SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 10.0 * SECOND));
    let ask = last(&asked).notes.unwrap();
    assert!(ask.read && ask.state.is_empty(), "{ask:?}");
}

#[test]
fn a_change_waiting_for_its_version_has_the_notes_read_whole_whatever_state_is_held() {
    let (mut app, _, asked) = read_once();
    let groceries = app.session.mailbox.notes.items.values().find(|note| note.origin.id == "VN-2").map(|note| note.id).unwrap();
    // An answer that didn't say the note's words left it no version; she edits it.
    if let Some(note) = app.session.mailbox.notes.note_mut(groceries) {
        note.version.clear();
    }
    app.execute("notes.edit", json!({"id": groceries, "text": "Mine"})).unwrap();
    app.notes_look(true, T0 + SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 5.0 * SECOND), "a job, for the version");
    let ask = last(&asked).notes.unwrap();
    assert!(ask.read && ask.state.is_empty(), "read whole: {ask:?}");
    assert!(ask.changes.is_none(), "the change waits for the version, without a failure");
}

#[test]
fn a_hosting_tab_keeps_notes_a_server_stopped_keeping_and_asks_before_it_is_left() {
    let (mut app, id, asked) = read_once();
    app.session.keeps_local = false;
    app.execute("notes.new", json!({"title": "Lunch"})).unwrap();
    assert!(app.jmap_pim_tick_at(true, T0 + 11.0 * MINUTE));
    let gone = NotesDownload { account: id, email: ME.into(), offered: false, ..Default::default() };
    answer(&mut app, &asked, id, Some(NotesAnswer { download: Some(gone), ..Default::default() }));
    assert!(app.session.mailbox.notes.items.values().any(|note| note.title == "Lunch" && note.origin.is_local()));
    assert!(app.session.mailbox.notes.changes.is_empty());
    assert!(app.notes_unsent(), "on this page alone: leaving it asks");
}

#[test]
fn a_change_waiting_for_its_version_has_the_notes_read_once_every_two_minutes_not_at_every_look() {
    let (mut app, id, asked) = read_once();
    let groceries = app.session.mailbox.notes.items.values().find(|note| note.origin.id == "VN-2").map(|note| note.id).unwrap();
    if let Some(note) = app.session.mailbox.notes.note_mut(groceries) {
        note.version.clear();
    }
    app.execute("notes.edit", json!({"id": groceries, "text": "Mine"})).unwrap();
    app.notes_look(true, T0 + SECOND);
    let before = jobs(&asked);
    // Four minutes of looks, every five seconds; each read comes back cut short, without the note.
    let mut t = T0 + 5.0 * SECOND;
    while t < T0 + 4.0 * MINUTE {
        if app.jmap_pim_tick_at(true, t) {
            let cut = NotesDownload { account: id, email: ME.into(), offered: true, complete: false, ..Default::default() };
            answer(&mut app, &asked, id, Some(NotesAnswer { download: Some(cut), ..Default::default() }));
        }
        t += 5.0 * SECOND;
    }
    assert_eq!(jobs(&asked) - before, 2, "at once, then two minutes later");
}

#[test]
fn what_goes_is_kept_as_sent_when_its_job_starts_and_the_mailbox_says_so() {
    let (mut app, _, asked) = read_once();
    let groceries = app.session.mailbox.notes.items.values().find(|note| note.origin.id == "VN-2").map(|note| note.id).unwrap();
    app.execute("notes.edit", json!({"id": groceries, "text": "Mine"})).unwrap();
    app.notes_look(true, T0 + SECOND);
    let rev = app.session.rev();
    assert!(app.jmap_pim_tick_at(true, T0 + 5.0 * SECOND));
    assert!(last(&asked).notes.is_some_and(|ask| ask.changes.is_some()));
    let sent = &app.session.mailbox.notes.changes[0].sent;
    assert_eq!(sent.first().and_then(|sent| sent.text.as_deref()), Some("Mine"));
    assert!(app.session.rev() > rev, "a change of the mailbox: it is saved with it");
    // (And that alone sends nothing again.)
    app.notes_look(true, T0 + 6.0 * SECOND);
    let id = app.session.mailbox.notes.changes[0].account;
    assert!(app.notes_push(id, T0 + 20.0 * SECOND).is_none());
}

#[test]
fn a_hosting_tab_asks_before_it_is_left_while_a_note_has_no_name_on_its_server() {
    let (mut app, id, asked) = read_once();
    app.session.keeps_local = false;
    app.execute("notes.new", json!({"title": "Lunch"})).unwrap();
    app.notes_look(true, T0 + SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 5.0 * SECOND));
    let op = last(&asked).notes.unwrap().changes.unwrap().ops[0].clone();
    // Made, the answer says, under a name it doesn't say: nothing waits, and the note has no name of the server's.
    let done = NoteOutcome { op, result: NoteResult::Done, note: None, ..Default::default() };
    let notes = NotesAnswer {
        uploaded: Some(NotesUploaded { account: id, email: ME.into(), outcomes: vec![done], ..Default::default() }),
        ..Default::default()
    };
    answer(&mut app, &asked, id, Some(notes));
    assert!(app.session.mailbox.notes.changes.is_empty());
    assert!(app.notes_unsent(), "as far as this page knows, nowhere but here");
}

#[test]
fn a_deletion_undone_after_its_answer_was_lost_has_the_notes_read_and_the_note_made_anew_if_it_is_gone() {
    let (mut app, id, asked) = read_once();
    let groceries = app.session.mailbox.notes.items.values().find(|note| note.origin.id == "VN-2").map(|note| note.id).unwrap();
    app.execute("notes.delete", json!({"id": groceries})).unwrap();
    app.notes_look(true, T0 + SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 5.0 * SECOND));
    let op = last(&asked).notes.unwrap().changes.unwrap().ops[0].clone();
    // The answer is lost; she undoes the deletion.
    let lost = NoteOutcome { op, result: NoteResult::Failed, error: "network: the connection broke".into(), ..Default::default() };
    let notes = NotesAnswer {
        uploaded: Some(NotesUploaded { account: id, email: ME.into(), outcomes: vec![lost], ..Default::default() }),
        ..Default::default()
    };
    answer(&mut app, &asked, id, Some(notes));
    app.execute("edit.undo", json!({})).unwrap();
    assert!(app.session.mailbox.notes.note(groceries).is_some());
    app.notes_look(true, T0 + 6.0 * SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 10.0 * SECOND), "read at once, to know whether the server deleted it");
    let ask = last(&asked).notes.unwrap();
    assert!(ask.read && ask.state.is_empty() && ask.changes.is_none(), "{ask:?}");
    // It did.
    answer(&mut app, &asked, id, Some(read(id, vec![remote("VN-1", "Offsite")], "s2")));
    let mine = app.session.mailbox.notes.note(groceries).cloned().expect("never lost");
    assert_eq!((mine.origin.id.as_str(), mine.text.as_str()), ("", "Groceries words"));
    app.notes_look(true, T0 + 11.0 * SECOND);
    assert!(app.jmap_pim_tick_at(true, T0 + 15.0 * SECOND));
    let made = last(&asked).notes.unwrap().changes.unwrap().ops;
    assert_eq!((made.len(), made[0].kind, made[0].note), (1, mailcraft_model::note::NoteChangeKind::Create, groceries), "made there anew");
}
