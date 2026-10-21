// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A server's notes: read in, what is done here noted for the server, and what it said taken in. The server's side is
//! written out by hand, as a host brings it (`mailcraft-jmap`'s notes). Invented people at `example.org`.

use mailcraft_model::Account;
use mailcraft_model::note::{NoteChangeKind, NoteOp, NoteOutcome, NoteResult, NoteServer, NotesDownload, NotesUploaded, RemoteNote};
use serde_json::{Value, json};

use crate::agent::{self, Class};
use crate::{CmdError, Session};

const ME: &str = "ada@neoffice.example.org";

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn remote(id: &str, title: &str, version: &str) -> RemoteNote {
    RemoteNote { id: id.into(), title: title.into(), text: format!("{title} words"), version: version.into(), ..Default::default() }
}

fn download(account: u64, notes: Vec<RemoteNote>) -> NotesDownload {
    NotesDownload { account, email: ME.into(), offered: true, server: NoteServer::default(), complete: true, notes, listed: None }
}

/// A session whose account's server keeps two notes, "Offsite" and "Groceries"; the account.
fn session() -> (Session, u64) {
    let mut s = Session::new(crate::sample::mailbox(1_791_993_600, 0));
    let account = s.mailbox.add_account(Account { email: ME.into(), ..Default::default() });
    let notes = vec![remote("VN-1", "Offsite", "2026-10-20T08:00:00.100Z"), remote("VN-2", "Groceries", "2026-10-20T08:00:00.200Z")];
    run(&mut s, "sync.applyNotes", json!({"download": download(account, notes)}));
    (s, account)
}

fn by_remote(s: &Session, remote: &str) -> u64 {
    s.mailbox.notes.items.values().find(|note| note.origin.id == remote).map(|note| note.id).unwrap_or_else(|| panic!("no note {remote}"))
}

fn ops(s: &mut Session, account: u64) -> Vec<NoteOp> {
    serde_json::from_value(run(s, "sync.noteChanges", json!({"account": account}))["ops"].clone()).unwrap()
}

/// The server's answer `result` to every op that goes now; a done create named `made`.
fn answer(s: &mut Session, account: u64, result: NoteResult, error: &str, made: Option<RemoteNote>) -> Value {
    let outcomes: Vec<NoteOutcome> =
        ops(s, account).into_iter().map(|op| NoteOutcome { op, result, error: error.into(), note: made.clone(), ..Default::default() }).collect();
    let uploaded = NotesUploaded { account, email: ME.into(), outcomes, ..Default::default() };
    run(s, "sync.applyNoteChanges", json!({"result": uploaded}))
}

fn kinds(s: &Session) -> Vec<NoteChangeKind> {
    s.mailbox.notes.changes.iter().map(|change| change.kind).collect()
}

#[test]
fn a_download_adds_updates_removes_and_leaves_a_note_that_waits() {
    let (mut s, account) = session();
    assert_eq!(s.mailbox.notes.items.len(), 2);
    assert!(s.mailbox.notes.server(account).is_some());
    let offsite = by_remote(&s, "VN-1");
    // Offsite changed here (its change waits); Groceries changed in the desk; a third note made there.
    run(&mut s, "notes.edit", json!({"id": offsite, "title": "Offsite in May"}));
    let mut groceries = remote("VN-2", "Groceries", "2026-10-20T09:00:00.000Z");
    groceries.text = "Milk".into();
    let mut offsite_there = remote("VN-1", "Offsite", "2026-10-20T09:00:00.000Z");
    offsite_there.pinned = true;
    let merged =
        run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![offsite_there, groceries, remote("VN-3", "Codes", "v3")])}));
    assert_eq!((merged["added"].as_u64(), merged["updated"].as_u64()), (Some(1), Some(2)), "{merged}");
    let offsite = s.mailbox.notes.note(offsite).cloned().unwrap();
    assert_eq!(offsite.title, "Offsite in May", "the change that waits is as it is here");
    assert!(offsite.pinned, "and the rest is the server's");
    // What goes is the change made here alone; the server changed no words of it meanwhile (only the pin), so it is
    // told against the version just read.
    let going = ops(&mut s, account);
    assert_eq!(going.len(), 1);
    assert_eq!(
        (going[0].fields.title.as_deref(), going[0].fields.pinned, going[0].version.as_str()),
        (Some("Offsite in May"), None, "2026-10-20T09:00:00.000Z")
    );
    assert_eq!(s.mailbox.notes.note(by_remote(&s, "VN-2")).map(|note| note.text.as_str()), Some("Milk"));
    // A whole read without Groceries takes it away; Offsite, whose change waits, stays.
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![])}));
    assert!(s.mailbox.notes.items.values().all(|note| note.origin.id == "VN-1"));
}

#[test]
fn an_incomplete_download_removes_nothing() {
    let (mut s, account) = session();
    let mut partial = download(account, vec![remote("VN-1", "Offsite", "v")]);
    partial.complete = false;
    run(&mut s, "sync.applyNotes", json!({"download": partial}));
    assert_eq!(s.mailbox.notes.items.len(), 2);
}

#[test]
fn a_new_note_goes_to_the_server_that_keeps_notes_and_waits_to_be_sent() {
    let (mut s, account) = session();
    let id = run(&mut s, "notes.new", json!({"title": "Team lunch", "text": "Friday"}))["id"].as_u64().unwrap();
    assert_eq!(kinds(&s), [NoteChangeKind::Create]);
    let going = ops(&mut s, account);
    assert_eq!(going.len(), 1);
    let note = s.mailbox.notes.note(id).unwrap();
    assert_eq!(
        (going[0].kind, going[0].token.as_str(), going[0].fields.title.as_deref()),
        (NoteChangeKind::Create, note.token.as_str(), Some("Team lunch"))
    );
    assert_eq!(going[0].token.len(), 36);
    assert_eq!(run(&mut s, "notes.list", json!({}))["notes"][0]["saved"], false);
    // A read before the answer leaves it as it is.
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "v"), remote("VN-2", "Groceries", "v")])}));
    assert!(s.mailbox.notes.note(id).is_some());
}

#[test]
fn changes_to_one_note_before_it_is_sent_are_one_change() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "One"}));
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Two"}));
    run(&mut s, "notes.pin", json!({"id": offsite}));
    assert_eq!(kinds(&s), [NoteChangeKind::Update]);
    let going = ops(&mut s, account);
    assert_eq!((going[0].fields.text.as_deref(), going[0].fields.pinned), (Some("Two"), Some(true)));
    // Taken back to what the server has: nothing is to go.
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Offsite words"}));
    run(&mut s, "notes.pin", json!({"id": offsite}));
    assert!(s.mailbox.notes.changes.is_empty());
    // A new note changed before it went is sent once, as it is then.
    let id = run(&mut s, "notes.new", json!({"title": "Draft"}))["id"].as_u64().unwrap();
    run(&mut s, "notes.edit", json!({"id": id, "text": "Final"}));
    assert_eq!(kinds(&s), [NoteChangeKind::Create]);
    assert_eq!(ops(&mut s, account)[0].fields.text.as_deref(), Some("Final"));
}

#[test]
fn a_note_made_and_deleted_before_it_was_sent_tells_the_server_nothing() {
    let (mut s, account) = session();
    let id = run(&mut s, "notes.new", json!({"title": "Oops"}))["id"].as_u64().unwrap();
    run(&mut s, "notes.delete", json!({"id": id}));
    assert!(s.mailbox.notes.changes.is_empty());
    // A server's note changed and then deleted: one deletion, against what the server had.
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "title": "Changed"}));
    run(&mut s, "notes.delete", json!({"id": offsite}));
    assert_eq!(kinds(&s), [NoteChangeKind::Delete]);
    assert_eq!(s.mailbox.notes.changes[0].base.as_deref().map(|base| base.title.as_str()), Some("Offsite"));
    assert_eq!(ops(&mut s, account)[0].remote, "VN-1");
    // A read before the server was told doesn't bring it back.
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "v"), remote("VN-2", "Groceries", "v")])}));
    assert!(s.mailbox.notes.items.values().all(|note| note.origin.id != "VN-1"));
}

#[test]
fn a_server_that_no_longer_offers_notes_takes_them_away_with_a_word() {
    let (mut s, account) = session();
    let waiting = run(&mut s, "notes.new", json!({"title": "Not sent yet"}))["id"].as_u64().unwrap();
    let gone = NotesDownload { account, email: ME.into(), offered: false, ..Default::default() };
    let said = run(&mut s, "sync.applyNotes", json!({"download": gone}));
    assert_eq!(said["messages"], json!([super::super::words::NO_LONGER_KEPT]));
    assert!(s.mailbox.notes.servers.is_empty() && s.mailbox.notes.changes.is_empty());
    assert_eq!(s.mailbox.notes.items.len(), 1, "the one that waited is kept, as the mailbox's own");
    assert!(s.mailbox.notes.note(waiting).is_some_and(|note| note.origin.is_local()));
    // In a session that keeps nothing of its own, it stays on the page, unsent, until the page is left (it asks first).
    let (mut s, account) = session();
    s.keeps_local = false;
    run(&mut s, "notes.new", json!({"title": "Not sent yet"}));
    let said = run(&mut s, "sync.applyNotes", json!({"download": NotesDownload { account, email: ME.into(), ..Default::default() }}));
    assert_eq!(said["messages"], json!([super::super::words::NO_LONGER_KEPT_PAGE]));
    assert!(s.mailbox.notes.items.values().all(|note| note.origin.is_local() && note.title == "Not sent yet"));
    assert!(super::super::offered(&s), "and shown");
}

#[test]
fn done_gives_a_new_note_its_server_id_and_version() {
    let (mut s, account) = session();
    let id = run(&mut s, "notes.new", json!({"text": "Ideas\nfor May"}))["id"].as_u64().unwrap();
    let mut made = remote("VN-9", "Ideas", "2026-10-20T10:00:00.000Z");
    made.text = "Ideas\nfor May".into();
    made.web_url = "https://desk.example.org/app/notes?note=VN-9".into();
    answer(&mut s, account, NoteResult::Done, "", Some(made));
    let note = s.mailbox.notes.note(id).cloned().unwrap();
    assert_eq!(
        (note.origin.id.as_str(), note.version.as_str(), note.title.as_str()),
        ("VN-9", "2026-10-20T10:00:00.000Z", "Ideas"),
        "the title the server gave it"
    );
    assert!(note.web_url.starts_with("https://") && s.mailbox.notes.changes.is_empty());
    // A copy of it a read brought before the answer is the same note.
    let (mut s, account) = session();
    let id = run(&mut s, "notes.new", json!({"title": "Lunch"}))["id"].as_u64().unwrap();
    run(
        &mut s,
        "sync.applyNotes",
        json!({"download": download(account, vec![remote("VN-1", "Offsite", "v"), remote("VN-2", "Groceries", "v"), remote("VN-9", "Lunch", "v")])}),
    );
    assert_eq!(s.mailbox.notes.items.values().filter(|note| note.title == "Lunch").count(), 2);
    answer(&mut s, account, NoteResult::Done, "", Some(remote("VN-9", "Lunch", "v")));
    let lunches: Vec<u64> = s.mailbox.notes.items.values().filter(|note| note.title == "Lunch").map(|note| note.id).collect();
    assert_eq!(lunches, [id], "one, the one made here");
}

#[test]
fn an_answer_to_a_change_takes_its_version_and_rights_and_leaves_the_rest_as_read() {
    let (mut s, account) = session();
    let mut offsite = remote("VN-1", "Offsite", "2026-10-20T09:00:00.000Z");
    (offsite.web_url, offsite.owner_name, offsite.category) =
        ("https://desk.example.org/app/notes?note=VN-1".into(), "Grace Okafor".into(), "Work".into());
    offsite.shared = true;
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![offsite, remote("VN-2", "Groceries", "v")])}));
    let id = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": id, "title": "Offsite in May"}));
    // The server's answer to a change says its version, its words and the rights, no more.
    let said = RemoteNote {
        id: "VN-1".into(),
        title: "Offsite in May".into(),
        text: "Offsite words".into(),
        version: "2026-10-20T10:00:00.250Z".into(),
        may_delete: false,
        ..Default::default()
    };
    answer(&mut s, account, NoteResult::Done, "", Some(said));
    let note = s.mailbox.notes.note(id).cloned().unwrap();
    assert_eq!((note.version.as_str(), note.rights.delete), ("2026-10-20T10:00:00.250Z", false));
    assert_eq!(
        (note.web_url.as_str(), note.owner_name.as_str(), note.category.as_str(), note.shared),
        ("https://desk.example.org/app/notes?note=VN-1", "Grace Okafor", "Work", true)
    );
}

#[test]
fn a_change_made_while_the_first_was_on_its_way_still_goes() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "First"}));
    let first = ops(&mut s, account);
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Second"}));
    let done = NoteOutcome {
        op: first[0].clone(),
        result: NoteResult::Done,
        note: Some(RemoteNote { text: "First".into(), ..remote("VN-1", "Offsite", "2026-10-20T11:00:00.000Z") }),
        ..Default::default()
    };
    run(&mut s, "sync.applyNoteChanges", json!({"result": NotesUploaded { account, email: ME.into(), outcomes: vec![done], ..Default::default() }}));
    let going = ops(&mut s, account);
    assert_eq!((going.len(), going[0].fields.text.as_deref(), going[0].version.as_str()), (1, Some("Second"), "2026-10-20T11:00:00.000Z"));
}

#[test]
fn a_stale_change_of_words_is_kept_as_a_new_note_and_said() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "My plan"}));
    let said = answer(&mut s, account, NoteResult::Stale, "The note was changed elsewhere: read it again.", None);
    assert_eq!(said["stale"].as_u64(), Some(1));
    assert_eq!(said["messages"], json!(["\"Offsite\" was changed elsewhere meanwhile: your version is kept as a note of its own, \"Offsite (2)\"."]));
    let copy = s.mailbox.notes.items.values().find(|note| note.title == "Offsite (2)").cloned().expect("the person's version");
    assert_eq!((copy.text.as_str(), copy.origin.account, copy.origin.id.as_str()), ("My plan", account, ""));
    assert_eq!(kinds(&s), [NoteChangeKind::Create], "it goes to the server as a note of its own");
    let note = s.mailbox.notes.note(offsite).unwrap();
    assert_eq!(
        (note.text.as_str(), note.version.as_str()),
        ("Offsite words", "2026-10-20T08:00:00.100Z"),
        "as the server had it, with the version those words are from, until the next read"
    );
}

#[test]
fn a_change_of_words_to_a_note_deleted_elsewhere_is_kept_as_a_new_note() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Mine"}));
    let said = answer(&mut s, account, NoteResult::Gone, "", None);
    assert_eq!(said["messages"], json!(["\"Offsite\" was deleted elsewhere meanwhile: your version is kept as a note of its own."]));
    // The note here, with her words, is one of its own now, to be made there anew.
    assert!(s.mailbox.notes.note(offsite).is_some_and(|note| note.title == "Offsite" && note.text == "Mine" && note.origin.id.is_empty()));
    assert_eq!(kinds(&s), [NoteChangeKind::Create]);
    s.mailbox.notes.changes.clear();
    // A pin alone of a note gone there: it goes here too.
    let groceries = by_remote(&s, "VN-2");
    run(&mut s, "notes.pin", json!({"id": groceries}));
    s.clock.now = s.clock.now.map(|now| now + 10);
    let outcome = NoteOutcome {
        op: ops(&mut s, account).into_iter().find(|op| op.kind == NoteChangeKind::Update).unwrap(),
        result: NoteResult::Gone,
        ..Default::default()
    };
    run(
        &mut s,
        "sync.applyNoteChanges",
        json!({"result": NotesUploaded { account, email: ME.into(), outcomes: vec![outcome], ..Default::default() }}),
    );
    assert!(s.mailbox.notes.note(groceries).is_none());
}

#[test]
fn a_refused_create_stays_with_why_and_a_refused_change_is_put_back() {
    let (mut s, account) = session();
    let id = run(&mut s, "notes.new", json!({"title": "Lunch"}))["id"].as_u64().unwrap();
    let said = answer(&mut s, account, NoteResult::Refused, "A title is one line of 140 characters at most.", None);
    assert_eq!(said["messages"], json!(["\"Lunch\" couldn't be saved on the server: A title is one line of 140 characters at most."]));
    let note = s.mailbox.notes.note(id).cloned().unwrap();
    assert!(note.origin.id.is_empty() && !note.error.is_empty() && s.mailbox.notes.changes.is_empty(), "kept here, with why");
    // Changed again, it goes again.
    run(&mut s, "notes.edit", json!({"id": id, "title": "Lunch on Friday"}));
    assert_eq!(kinds(&s), [NoteChangeKind::Create]);
    s.mailbox.notes.changes.clear();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "title": "Mine now"}));
    let said = answer(&mut s, account, NoteResult::Refused, "Not permitted", None);
    assert_eq!(
        said["messages"],
        json!(["The server wouldn't take the change to \"Mine now\": Not permitted. Your version is kept as a note of its own, \"Mine now (2)\"."])
    );
    assert_eq!(s.mailbox.notes.note(offsite).map(|note| note.title.as_str()), Some("Offsite"), "put back as the server has it");
    assert!(s.mailbox.notes.items.values().any(|note| note.title == "Mine now (2)" && note.origin.id.is_empty()), "nothing typed is lost");
    // A refused pin is put back, with the server's words.
    s.mailbox.notes.changes.clear();
    run(&mut s, "notes.pin", json!({"id": offsite}));
    let said = answer(&mut s, account, NoteResult::Refused, "Not permitted", None);
    assert_eq!(said["messages"], json!(["The change to \"Offsite\" was put back as the server has it: Not permitted"]));
    assert!(s.mailbox.notes.note(offsite).is_some_and(|note| !note.pinned));
    // A refused deletion brings the note back.
    run(&mut s, "notes.delete", json!({"id": offsite}));
    let said = answer(&mut s, account, NoteResult::Refused, "Only its owner deletes a note.", None);
    assert_eq!(said["messages"], json!(["The server wouldn't delete \"Offsite\", which is back in the notes: Only its owner deletes a note."]));
    assert!(s.mailbox.notes.items.values().any(|note| note.origin.id == "VN-1"));
}

#[test]
fn a_failed_change_waits_longer_each_time() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "title": "Later"}));
    let mut waits = Vec::new();
    for _ in 0..3 {
        answer(&mut s, account, NoteResult::Failed, "the server failed", None);
        let change = s.mailbox.notes.changes[0].clone();
        waits.push(change.retry_at - s.now());
        assert!(ops(&mut s, account).is_empty(), "held back");
        s.clock.now = Some(change.retry_at);
    }
    assert!(waits.windows(2).all(|w| w[1] > w[0]), "{waits:?}");
    assert_eq!(run(&mut s, "notes.get", json!({"id": offsite}))["saved"], false);
}

#[test]
fn an_answer_for_an_undone_change_notes_the_opposite() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "title": "Renamed"}));
    let first = ops(&mut s, account);
    assert!(s.undo());
    assert!(s.mailbox.notes.changes.is_empty());
    let said = RemoteNote { text: "Offsite words".into(), ..remote("VN-1", "Renamed", "v2") };
    let done = NoteOutcome { op: first[0].clone(), result: NoteResult::Done, note: Some(said), ..Default::default() };
    run(&mut s, "sync.applyNoteChanges", json!({"result": NotesUploaded { account, email: ME.into(), outcomes: vec![done], ..Default::default() }}));
    let going = ops(&mut s, account);
    assert_eq!((going.len(), going[0].fields.title.as_deref(), going[0].version.as_str()), (1, Some("Offsite"), "v2"));
    // A new note undone while it was on its way: the server is to delete it.
    let (mut s, account) = session();
    run(&mut s, "notes.new", json!({"title": "Undone"}));
    let first = ops(&mut s, account);
    assert!(s.undo());
    let done = NoteOutcome { op: first[0].clone(), result: NoteResult::Done, note: Some(remote("VN-7", "Undone", "v")), ..Default::default() };
    run(&mut s, "sync.applyNoteChanges", json!({"result": NotesUploaded { account, email: ME.into(), outcomes: vec![done], ..Default::default() }}));
    assert_eq!(
        ops(&mut s, account).iter().map(|op| (op.kind, op.remote.clone())).collect::<Vec<_>>(),
        [(NoteChangeKind::Delete, "VN-7".to_string())]
    );
}

#[test]
fn the_undo_history_is_forgotten_once_the_server_was_told() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "title": "Told"}));
    answer(&mut s, account, NoteResult::Done, "", Some(remote("VN-1", "Told", "v2")));
    assert!(!s.undo(), "nothing to undo: the server has it");
}

#[test]
fn two_notes_with_one_origin_leave_one() {
    let (mut s, account) = session();
    let first = by_remote(&s, "VN-1");
    let twin = s.mailbox.alloc_id();
    let mut copy = s.mailbox.notes.note(first).cloned().unwrap();
    copy.id = twin;
    s.mailbox.notes.items.insert(twin, std::sync::Arc::new(copy));
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "v"), remote("VN-2", "Groceries", "v")])}));
    assert!(s.mailbox.notes.note(first).is_some() && s.mailbox.notes.note(twin).is_none());
}

#[test]
fn an_agents_change_never_rides_on_a_change_that_waits_and_writes_ask() {
    let (mut s, _) = session();
    let offsite = by_remote(&s, "VN-1");
    // The owner changes it twice, while the server can't be reached: one note of a change waits.
    run(&mut s, "notes.edit", json!({"id": offsite, "title": "One"}));
    run(&mut s, "notes.edit", json!({"id": offsite, "title": "Two"}));
    let before = s.mailbox.clone();
    let mut after = before.clone();
    if let Some(note) = after.notes.note_mut(offsite) {
        note.text = "An agent's words".into();
    }
    assert_eq!(agent::overstep(&before, &after, &[]), Some(Class::Send));
    // An agent with no class takes the second back: refused, nothing changed.
    s.agent = Some(Vec::new());
    let why = match s.run("edit.undo", &json!({})) {
        Err(CmdError::Failed(why) | CmdError::Disabled(why)) => why,
        other => panic!("{other:?}"),
    };
    assert!(why.contains("agent"), "{why}");
    assert_eq!(s.mailbox.notes.note(offsite).map(|note| note.title.as_str()), Some("Two"));
    // A deletion of a server's note is Delete.
    s.agent = None;
    let before = s.mailbox.clone();
    run(&mut s, "notes.delete", json!({"id": offsite}));
    assert_eq!(agent::overstep(&before, &s.mailbox, &[Class::Send]), Some(Class::Delete));
}

/// The note here `id`.
fn note(s: &Session, id: u64) -> mailcraft_model::note::Note {
    s.mailbox.notes.note(id).cloned().unwrap_or_else(|| panic!("no note {id}"))
}

#[test]
fn a_pin_answered_with_words_written_elsewhere_takes_them_with_their_version() {
    let (mut s, account) = session();
    let groceries = by_remote(&s, "VN-2");
    // Read here at the first version; the phone wrote "Milk\nCoffee" since; Ada pins it.
    run(&mut s, "notes.pin", json!({"id": groceries}));
    let said = RemoteNote { text: "Milk\nCoffee".into(), pinned: true, ..remote("VN-2", "Groceries", "v3") };
    answer(&mut s, account, NoteResult::Done, "", Some(said));
    let kept = note(&s, groceries);
    assert_eq!((kept.text.as_str(), kept.version.as_str(), kept.pinned), ("Milk\nCoffee", "v3", true), "the version with its own words");
    // What she types now is made over the words she sees.
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk\nCoffee\nEggs"}));
    let going = ops(&mut s, account);
    assert_eq!((going[0].fields.text.as_deref(), going[0].version.as_str()), (Some("Milk\nCoffee\nEggs"), "v3"));
}

#[test]
fn words_typed_while_a_pin_was_on_its_way_go_stale_when_the_answer_brings_other_words() {
    let (mut s, account) = session();
    let groceries = by_remote(&s, "VN-2");
    run(&mut s, "notes.pin", json!({"id": groceries}));
    let first = ops(&mut s, account);
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Groceries words\nEggs"}));
    let said = RemoteNote { text: "Milk\nCoffee".into(), pinned: true, ..remote("VN-2", "Groceries", "v3") };
    let done = NoteOutcome { op: first[0].clone(), result: NoteResult::Done, note: Some(said), ..Default::default() };
    run(&mut s, "sync.applyNoteChanges", json!({"result": NotesUploaded { account, email: ME.into(), outcomes: vec![done], ..Default::default() }}));
    assert_eq!(note(&s, groceries).text, "Groceries words\nEggs", "her words stay here");
    let going = ops(&mut s, account);
    assert_eq!(going.len(), 1);
    assert_eq!(
        (going[0].fields.text.as_deref(), going[0].version.as_str()),
        (Some("Groceries words\nEggs"), "2026-10-20T08:00:00.200Z"),
        "made over the words she read: the server refuses it as stale, never over the phone's"
    );
}

#[test]
fn a_change_taken_back_after_a_read_gives_the_note_back_the_version_its_words_are_from() {
    let (mut s, account) = session();
    let (groceries, read_at) = (by_remote(&s, "VN-2"), "2026-10-20T08:00:00.200Z");
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Mine"}));
    // A read brings the phone's words, newer, while her change waits.
    let phone = RemoteNote { text: "Phone".into(), ..remote("VN-2", "Groceries", "v2") };
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "2026-10-20T08:00:00.100Z"), phone])}));
    assert_eq!((note(&s, groceries).text.as_str(), note(&s, groceries).version.as_str()), ("Mine", read_at));
    // She types the words back as she read them: nothing to tell, and the note's version is still the one of those words.
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Groceries words"}));
    assert!(s.mailbox.notes.changes.is_empty());
    assert_eq!(note(&s, groceries).version, read_at);
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Again"}));
    assert_eq!(ops(&mut s, account)[0].version, read_at, "stale at the server, never written over the phone's");
}

#[test]
fn a_pin_that_waits_takes_the_version_a_read_brings() {
    let (mut s, account) = session();
    let groceries = by_remote(&s, "VN-2");
    run(&mut s, "notes.pin", json!({"id": groceries}));
    // A read: the same words, a newer version (the desk archived something, or nothing that matters here).
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "v1"), remote("VN-2", "Groceries", "v2")])}));
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "More"}));
    let going = ops(&mut s, account);
    assert_eq!((going.len(), going[0].fields.pinned, going[0].version.as_str()), (1, Some(true), "v2"), "no false conflict");
}

#[test]
fn a_conflict_with_no_room_for_a_copy_keeps_the_change_and_says_so() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Mine"}));
    while s.mailbox.notes.items.len() < mailcraft_model::note::MAX_NOTES {
        let id = s.mailbox.alloc_id();
        s.mailbox.notes.items.insert(id, std::sync::Arc::new(mailcraft_model::note::Note { id, ..Default::default() }));
    }
    let said = answer(&mut s, account, NoteResult::Stale, "The note was changed elsewhere: read it again.", None);
    assert_eq!(
        said["messages"],
        json!([
            "\"Offsite\" couldn't be saved on the server (The note was changed elsewhere: read it again.), and there is no room for a note of its own: your version stays here, unsent."
        ])
    );
    assert_eq!(note(&s, offsite).text, "Mine");
    assert_eq!(kinds(&s), [NoteChangeKind::Update], "it waits");
}

#[test]
fn a_server_that_no_longer_offers_notes_keeps_a_waiting_change_here() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Mine"}));
    let gone = NotesDownload { account, email: ME.into(), offered: false, ..Default::default() };
    let said = run(&mut s, "sync.applyNotes", json!({"download": gone}));
    assert_eq!(said["messages"], json!([super::super::words::NO_LONGER_KEPT]));
    assert!(s.mailbox.notes.note(offsite).is_some_and(|note| note.origin.is_local() && note.text == "Mine"));
}

#[test]
fn a_new_notes_answer_that_names_it_alone_leaves_its_words() {
    let (mut s, account) = session();
    let id = run(&mut s, "notes.new", json!({"title": "Ideas", "text": "For May"}))["id"].as_u64().unwrap();
    answer(&mut s, account, NoteResult::Done, "", Some(RemoteNote { id: "VN-9".into(), ..Default::default() }));
    let made = note(&s, id);
    assert_eq!((made.origin.id.as_str(), made.title.as_str(), made.text.as_str()), ("VN-9", "Ideas", "For May"));
}

#[test]
fn conflicts_on_one_note_number_their_copies() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    for words in ["One", "Two"] {
        run(&mut s, "notes.edit", json!({"id": offsite, "text": words}));
        answer(&mut s, account, NoteResult::Stale, "changed elsewhere", None);
        s.mailbox.notes.changes.retain(|change| change.kind != NoteChangeKind::Create);
    }
    let mut copies: Vec<String> =
        s.mailbox.notes.items.values().filter(|note| note.title.starts_with("Offsite (")).map(|note| note.title.clone()).collect();
    copies.sort();
    assert_eq!(copies, ["Offsite (2)", "Offsite (3)"]);
}

#[test]
fn a_note_without_words_is_named_in_a_line_without_english_words() {
    let (mut s, account) = session();
    run(&mut s, "notes.new", json!({}));
    let said = answer(&mut s, account, NoteResult::Refused, "Not permitted", None);
    assert_eq!(said["messages"], json!(["\"…\" couldn't be saved on the server: Not permitted"]));
}

#[test]
fn an_answer_that_brings_another_devices_words_of_a_part_not_changed_here_settles_the_change() {
    let (mut s, account) = session();
    let groceries = by_remote(&s, "VN-2");
    run(&mut s, "notes.edit", json!({"id": groceries, "title": "Shopping"}));
    // The server took the title; the phone had changed the text meanwhile, and the answer says so.
    let said = RemoteNote { title: "Shopping".into(), text: "Milk\nCoffee".into(), ..remote("VN-2", "Groceries", "v3") };
    answer(&mut s, account, NoteResult::Done, "", Some(said));
    assert_eq!((note(&s, groceries).text.as_str(), note(&s, groceries).version.as_str()), ("Milk\nCoffee", "v3"));
    assert!(s.mailbox.notes.changes.is_empty(), "nothing waits: the note here is the server's");
}

#[test]
fn a_refused_new_note_stays_when_the_server_stops_offering_notes() {
    for keeps_local in [true, false] {
        let (mut s, account) = session();
        s.keeps_local = keeps_local;
        let id = run(&mut s, "notes.new", json!({"title": "Lunch", "text": "Friday"}))["id"].as_u64().unwrap();
        answer(&mut s, account, NoteResult::Refused, "Not permitted", None);
        assert!(s.mailbox.notes.changes.is_empty(), "refused: nothing waits, the note is here with why");
        let gone = NotesDownload { account, email: ME.into(), offered: false, ..Default::default() };
        let said = run(&mut s, "sync.applyNotes", json!({"download": gone}));
        assert!(s.mailbox.notes.note(id).is_some_and(|note| note.origin.is_local() && note.text == "Friday"), "kept: {keeps_local}");
        assert_eq!(said["messages"].as_array().map(Vec::len), Some(1), "and said: {said}");
    }
}

#[test]
fn a_copy_whose_title_had_to_be_cut_has_the_whole_title_as_its_first_line() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    let long = "T".repeat(140);
    run(&mut s, "notes.edit", json!({"id": offsite, "title": long, "text": "Mine"}));
    answer(&mut s, account, NoteResult::Stale, "changed elsewhere", None);
    let copy = s.mailbox.notes.items.values().find(|note| note.title.ends_with(" (2)")).cloned().expect("a copy");
    assert_eq!(copy.title.chars().count(), 140);
    assert_eq!(copy.text, format!("{long}\nMine"), "nothing of her title lost");
}

#[test]
fn an_answer_to_a_change_without_its_words_leaves_no_version_and_the_notes_are_read() {
    let (mut s, account) = session();
    let groceries = by_remote(&s, "VN-2");
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk"}));
    answer(&mut s, account, NoteResult::Done, "", None);
    assert_eq!(note(&s, groceries).version, "", "a version is never kept for words it isn't of");
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk\nEggs"}));
    assert!(ops(&mut s, account).is_empty(), "not sent without a version");
    assert!(super::notes_need_read(&s, account), "the notes are read for it");
    assert_eq!(run(&mut s, "sync.noteChanges", json!({"account": account}))["needRead"], true, "and a host is told so");
    // The read gives it the version of the words it was made over.
    let read = RemoteNote { text: "Milk".into(), ..remote("VN-2", "Groceries", "v5") };
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "2026-10-20T08:00:00.100Z"), read])}));
    let going = ops(&mut s, account);
    assert_eq!((going.len(), going[0].version.as_str()), (1, "v5"));
    assert!(!super::notes_need_read(&s, account));
}

#[test]
fn words_kept_when_a_note_is_no_longer_shared_with_her_say_so() {
    let (mut s, account) = session();
    let mut shared = remote("VN-3", "Codes", "v1");
    (shared.shared, shared.owner_name) = (true, "Grace Okafor".into());
    run(
        &mut s,
        "sync.applyNotes",
        json!({"download": download(account, vec![remote("VN-1", "Offsite", "v"), remote("VN-2", "Groceries", "v"), shared])}),
    );
    let codes = by_remote(&s, "VN-3");
    run(&mut s, "notes.edit", json!({"id": codes, "text": "4711 and 0815"}));
    let said = answer(&mut s, account, NoteResult::Gone, "", None);
    assert_eq!(said["messages"], json!(["\"Codes\" is no longer shared with you: your version is kept as a note of its own."]));
    let mine = note(&s, codes);
    assert!(!mine.shared && mine.owner_name.is_empty() && mine.rights.delete && mine.origin.id.is_empty(), "{mine:?}");
}

#[test]
fn what_was_sent_of_a_change_goes_with_it_until_an_answer_comes() {
    let (mut s, account) = session();
    let groceries = by_remote(&s, "VN-2");
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk"}));
    let first = note_upload_now(&mut s, account);
    super::notes_sending(&mut s, &first);
    // Its answer was lost; she types more.
    answer(&mut s, account, NoteResult::Failed, "network: the connection broke", None);
    s.clock.now = s.clock.now.map(|now| now + 600);
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk\nEggs"}));
    let going = ops(&mut s, account);
    assert_eq!(going[0].earlier.first().and_then(|earlier| earlier.text.as_deref()), Some("Milk"), "what may be on the server already");
    // An answer settles it.
    let said = RemoteNote { text: "Milk\nEggs".into(), ..remote("VN-2", "Groceries", "v9") };
    answer(&mut s, account, NoteResult::Done, "", Some(said));
    assert!(s.mailbox.notes.changes.iter().all(|change| change.sent.is_empty()));
}

fn note_upload_now(s: &mut Session, account: u64) -> mailcraft_model::note::NotesUpload {
    serde_json::from_value(run(s, "sync.noteChanges", json!({"account": account}))).unwrap()
}

#[test]
fn a_change_of_words_without_a_version_of_a_note_gone_from_the_server_goes_and_is_kept() {
    let (mut s, account) = session();
    let groceries = by_remote(&s, "VN-2");
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk"}));
    answer(&mut s, account, NoteResult::Done, "", None);
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk\nEggs"}));
    assert!(ops(&mut s, account).is_empty(), "it waits for the version a read brings");
    // A whole read without it: deleted there meanwhile, and no read will ever bring its version.
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "2026-10-20T08:00:00.100Z")])}));
    assert!(!super::notes_need_read(&s, account), "no read is waited for again");
    let going = ops(&mut s, account);
    assert_eq!((going.len(), going[0].version.as_str()), (1, super::OLDEST_VERSION), "it goes, never over other words");
    let said = answer(&mut s, account, NoteResult::Gone, "", None);
    let kept = note(&s, groceries);
    assert_eq!((kept.text.as_str(), kept.origin.id.as_str()), ("Milk\nEggs", ""), "her words, a note of her own: {said}");
    assert_eq!(kinds(&s), [NoteChangeKind::Create]);
}

/// Groceries' text changed to "Milk" and sent, its answer lost (the server may have written it); ten minutes later.
fn milk_sent_and_lost() -> (Session, u64, u64) {
    let (mut s, account) = session();
    let groceries = by_remote(&s, "VN-2");
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk"}));
    let first = note_upload_now(&mut s, account);
    super::notes_sending(&mut s, &first);
    answer(&mut s, account, NoteResult::Failed, "network: the connection broke", None);
    s.clock.now = s.clock.now.map(|now| now + 600);
    (s, account, groceries)
}

#[test]
fn a_change_typed_back_after_a_lost_answer_still_goes_over_what_may_be_there() {
    let (mut s, account, groceries) = milk_sent_and_lost();
    // She types the words back as she read them.
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Groceries words"}));
    let going = ops(&mut s, account);
    assert_eq!(going.len(), 1, "never dropped: the server may hold what she took back");
    assert_eq!(
        (going[0].fields.text.as_deref(), going[0].version.as_str(), going[0].earlier.len()),
        (Some("Groceries words"), "2026-10-20T08:00:00.200Z", 1)
    );
    let said = RemoteNote { text: "Groceries words".into(), ..remote("VN-2", "Groceries", "v4") };
    answer(&mut s, account, NoteResult::Done, "", Some(said));
    assert!(s.mailbox.notes.changes.is_empty());
}

#[test]
fn an_undo_after_a_lost_answer_still_goes_over_what_may_be_there() {
    let (mut s, account, groceries) = milk_sent_and_lost();
    assert!(s.undo());
    assert_eq!(note(&s, groceries).text, "Groceries words");
    let going = ops(&mut s, account);
    assert_eq!(going.len(), 1, "the undo goes there too: the server may hold the words undone");
    assert_eq!((going[0].fields.text.as_deref(), going[0].earlier.len()), (Some("Groceries words"), 1));
    // Redone: her words again, and what went before still with them.
    assert!(s.redo());
    let going = ops(&mut s, account);
    assert_eq!((going[0].fields.text.as_deref(), going[0].earlier.len()), (Some("Milk"), 1));
}

#[test]
fn every_sending_unanswered_goes_with_a_change_and_a_stale_answer_keeps_those_before_it() {
    let (mut s, account, groceries) = milk_sent_and_lost();
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk\nEggs"}));
    let second = note_upload_now(&mut s, account);
    super::notes_sending(&mut s, &second);
    let texts = |sent: &[mailcraft_model::note::NoteFields]| sent.iter().filter_map(|sent| sent.text.clone()).collect::<Vec<_>>();
    assert_eq!(texts(&s.mailbox.notes.changes[0].sent), ["Milk", "Milk\nEggs"], "either may be there");
    // The second is refused as stale, and there is no room for a copy: the first may still be there.
    while s.mailbox.notes.items.len() < mailcraft_model::note::MAX_NOTES {
        let id = s.mailbox.alloc_id();
        s.mailbox.notes.items.insert(id, std::sync::Arc::new(mailcraft_model::note::Note { id, ..Default::default() }));
    }
    let stale = NoteOutcome { op: second.ops[0].clone(), result: NoteResult::Stale, error: "changed elsewhere".into(), ..Default::default() };
    run(&mut s, "sync.applyNoteChanges", json!({"result": NotesUploaded { account, email: ME.into(), outcomes: vec![stale], ..Default::default() }}));
    assert_eq!(texts(&s.mailbox.notes.changes[0].sent), ["Milk"], "what went before and was never answered");
}

#[test]
fn a_title_the_server_gave_a_new_note_stays_when_more_was_typed_before_the_answer() {
    let (mut s, account) = session();
    let id = run(&mut s, "notes.new", json!({"text": "Milk"}))["id"].as_u64().unwrap();
    let first = ops(&mut s, account);
    run(&mut s, "notes.edit", json!({"id": id, "text": "Milk\nEggs"}));
    // The server titled it from its first line.
    let made = RemoteNote { id: "VN-9".into(), title: "Milk".into(), text: "Milk".into(), version: "v1".into(), ..Default::default() };
    let done = NoteOutcome { op: first[0].clone(), result: NoteResult::Done, note: Some(made), ..Default::default() };
    run(&mut s, "sync.applyNoteChanges", json!({"result": NotesUploaded { account, email: ME.into(), outcomes: vec![done], ..Default::default() }}));
    let kept = note(&s, id);
    assert_eq!((kept.title.as_str(), kept.text.as_str()), ("Milk", "Milk\nEggs"));
    let going = ops(&mut s, account);
    assert_eq!(going.len(), 1);
    assert_eq!(
        (going[0].fields.title.as_deref(), going[0].fields.text.as_deref(), going[0].version.as_str()),
        (None, Some("Milk\nEggs"), "v1"),
        "what she typed goes; the server's title is never wiped"
    );
}

#[test]
fn notes_a_page_kept_when_its_server_stopped_go_there_again_when_it_keeps_notes_again() {
    for keeps_local in [false, true] {
        let (mut s, account) = session();
        s.keeps_local = keeps_local;
        let id = run(&mut s, "notes.new", json!({"title": "Lunch", "text": "Friday"}))["id"].as_u64().unwrap();
        let gone = NotesDownload { account, email: ME.into(), offered: false, ..Default::default() };
        run(&mut s, "sync.applyNotes", json!({"download": gone}));
        assert!(note(&s, id).origin.is_local());
        run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "v")])}));
        let back = note(&s, id);
        if keeps_local {
            assert!(back.origin.is_local() && s.mailbox.notes.changes.is_empty(), "this computer keeps it as its own");
            continue;
        }
        assert_eq!((back.origin.account, back.origin.id.as_str()), (account, ""), "the server's again");
        let going = ops(&mut s, account);
        assert_eq!((going.len(), going[0].kind, going[0].fields.text.as_deref()), (1, NoteChangeKind::Create, Some("Friday")));
    }
}

#[test]
fn a_read_before_the_resend_keeps_words_typed_back_over_her_own_lost_write() {
    let (mut s, account, groceries) = milk_sent_and_lost();
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Groceries words"}));
    // A read brings what the server wrote of the lost sending, before the change goes again.
    let there = RemoteNote { text: "Milk".into(), ..remote("VN-2", "Groceries", "v4") };
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "2026-10-20T08:00:00.100Z"), there])}));
    assert_eq!(note(&s, groceries).text, "Groceries words", "what she typed back stays");
    let going = ops(&mut s, account);
    assert_eq!(
        (going.len(), going[0].fields.text.as_deref(), going[0].version.as_str()),
        (1, Some("Groceries words"), "2026-10-20T08:00:00.200Z"),
        "told against the words she read: the server's copy of her own write is known by what went"
    );
}

/// Offsite deleted and the deletion sent, its answer lost (the server may have deleted it); ten minutes later the
/// deletion is undone.
fn deletion_lost_then_undone() -> (Session, u64, u64) {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.delete", json!({"id": offsite}));
    let going = note_upload_now(&mut s, account);
    super::notes_sending(&mut s, &going);
    answer(&mut s, account, NoteResult::Failed, "network: the connection broke", None);
    s.clock.now = s.clock.now.map(|now| now + 600);
    assert!(s.undo());
    let back = note(&s, offsite);
    assert_eq!((back.origin.id.as_str(), back.text.as_str()), ("VN-1", "Offsite words"));
    (s, account, offsite)
}

#[test]
fn a_deletion_undone_after_its_answer_was_lost_is_made_anew_when_the_server_deleted_the_note() {
    let (mut s, account, offsite) = deletion_lost_then_undone();
    // A whole read without it: the server did delete it.
    let said = run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-2", "Groceries", "v")])}));
    let mine = note(&s, offsite);
    assert_eq!((mine.origin.id.as_str(), mine.title.as_str(), mine.text.as_str()), ("", "Offsite", "Offsite words"), "never lost: {said}");
    assert_eq!(said["messages"], json!(["\"Offsite\" was already deleted on the server when its deletion was undone here: it is made there anew."]));
    let going = ops(&mut s, account);
    assert_eq!(
        (going.len(), going[0].kind, going[0].fields.text.as_deref()),
        (1, NoteChangeKind::Create, Some("Offsite words")),
        "a new note of hers, with her words"
    );
    assert!(!super::notes_need_read(&s, account));
}

#[test]
fn a_deletion_undone_after_its_answer_was_lost_makes_nothing_twice_when_the_server_kept_the_note() {
    let (mut s, account, offsite) = deletion_lost_then_undone();
    assert!(super::notes_need_read(&s, account), "whether the server deleted it is read at once");
    assert!(ops(&mut s, account).is_empty(), "nothing goes before that is known");
    // A whole read with it: the server never deleted it.
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![remote("VN-1", "Offsite", "v2"), remote("VN-2", "Groceries", "v")])}));
    assert_eq!(note(&s, offsite).origin.id, "VN-1");
    assert!(s.mailbox.notes.changes.is_empty() && ops(&mut s, account).is_empty(), "nothing is made, nothing waits");
    assert_eq!(s.mailbox.notes.items.values().filter(|note| note.title == "Offsite").count(), 1);
    assert!(!super::notes_need_read(&s, account));
    // What she types since goes as a change of that very note.
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Lake walk"}));
    let going = ops(&mut s, account);
    assert_eq!((going.len(), going[0].kind, going[0].version.as_str()), (1, NoteChangeKind::Update, "v2"));
}

#[test]
fn a_deletion_undone_after_its_answer_was_lost_and_typed_into_keeps_her_words_either_way() {
    // Typed into before the read, the server having deleted it: her words go as a change, and come back as a note of
    // her own made anew.
    let (mut s, account, offsite) = deletion_lost_then_undone();
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Lake walk"}));
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Offsite words"}));
    assert!(!s.mailbox.notes.changes.is_empty(), "typed back: still to be checked");
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Lake walk"}));
    answer(&mut s, account, NoteResult::Gone, "", None);
    let mine = note(&s, offsite);
    assert_eq!((mine.origin.id.as_str(), mine.text.as_str()), ("", "Lake walk"));
    assert_eq!(kinds(&s), [NoteChangeKind::Create]);
}

/// Groceries changed here after an answer that left it no version: a change of words that waits for one.
fn waiting_for_a_version() -> (Session, u64, u64) {
    let (mut s, account) = session();
    let groceries = by_remote(&s, "VN-2");
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk"}));
    answer(&mut s, account, NoteResult::Done, "", None);
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk\nEggs"}));
    assert!(super::notes_need_read(&s, account));
    (s, account, groceries)
}

#[test]
fn a_change_without_a_version_of_a_note_a_whole_list_no_longer_has_goes_though_some_note_could_not_be_read() {
    let (mut s, account, _) = waiting_for_a_version();
    // A read cut short whose list wasn't whole either: nothing is known of it.
    let mut cut = download(account, vec![]);
    cut.complete = false;
    run(&mut s, "sync.applyNotes", json!({"download": cut}));
    assert!(super::notes_need_read(&s, account) && ops(&mut s, account).is_empty(), "still waiting");
    // A read whose list is whole, without Groceries, though Offsite couldn't be read: deleted there.
    let mut cut = download(account, vec![]);
    (cut.complete, cut.listed) = (false, Some(vec!["VN-1".into()]));
    run(&mut s, "sync.applyNotes", json!({"download": cut}));
    assert!(!super::notes_need_read(&s, account));
    let going = ops(&mut s, account);
    assert_eq!((going.len(), going[0].version.as_str()), (1, super::OLDEST_VERSION));
}

#[test]
fn a_page_holding_notes_of_a_server_that_stopped_gives_them_back_as_they_were_never_as_copies() {
    let (mut s, account) = session();
    s.keeps_local = false;
    let (offsite, groceries) = (by_remote(&s, "VN-1"), by_remote(&s, "VN-2"));
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Mine"}));
    run(&mut s, "notes.pin", json!({"id": groceries}));
    let gone = NotesDownload { account, email: ME.into(), offered: false, ..Default::default() };
    run(&mut s, "sync.applyNotes", json!({"download": gone}));
    assert!(note(&s, offsite).origin.is_local() && note(&s, groceries).origin.is_local(), "held on the page");
    // Offered again: the server still has both, as they were.
    let both = vec![remote("VN-1", "Offsite", "2026-10-20T08:00:00.100Z"), remote("VN-2", "Groceries", "2026-10-20T08:00:00.200Z")];
    run(&mut s, "sync.applyNotes", json!({"download": download(account, both)}));
    assert_eq!(s.mailbox.notes.items.len(), 2, "no copies");
    assert_eq!((note(&s, offsite).origin.id.as_str(), note(&s, groceries).origin.id.as_str()), ("VN-1", "VN-2"));
    let mut going = ops(&mut s, account);
    going.sort_by_key(|op| op.remote.clone());
    let said: Vec<_> = going.iter().map(|op| (op.kind, op.fields.text.as_deref(), op.fields.pinned, op.version.as_str())).collect();
    assert_eq!(
        said,
        [(NoteChangeKind::Update, Some("Mine"), None, "2026-10-20T08:00:00.100Z"), (NoteChangeKind::Update, None, Some(true), ""),],
        "her changes, to the very notes"
    );
}

#[test]
fn a_held_note_its_server_deleted_meanwhile_is_made_anew_and_one_changed_there_is_a_copy() {
    let (mut s, account) = session();
    s.keeps_local = false;
    let (offsite, groceries) = (by_remote(&s, "VN-1"), by_remote(&s, "VN-2"));
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Mine"}));
    run(&mut s, "notes.edit", json!({"id": groceries, "text": "Milk"}));
    let gone = NotesDownload { account, email: ME.into(), offered: false, ..Default::default() };
    run(&mut s, "sync.applyNotes", json!({"download": gone}));
    // Offsite deleted there; Groceries changed there by the phone.
    let phone = RemoteNote { text: "Phone".into(), ..remote("VN-2", "Groceries", "v9") };
    let said = run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![phone])}));
    let made = note(&s, offsite);
    assert_eq!((made.origin.id.as_str(), made.text.as_str()), ("", "Mine"), "made anew: {said}");
    assert_eq!(said["messages"], json!(["\"Offsite\" was deleted elsewhere meanwhile: your version is kept as a note of its own."]));
    assert_eq!(note(&s, groceries).text, "Milk", "her words stay until the server says");
    let mut going = ops(&mut s, account);
    going.sort_by_key(|op| op.kind == NoteChangeKind::Update);
    assert_eq!((going[0].kind, going[0].fields.text.as_deref()), (NoteChangeKind::Create, Some("Mine")));
    assert_eq!(
        (going[1].kind, going[1].fields.text.as_deref(), going[1].version.as_str()),
        (NoteChangeKind::Update, Some("Milk"), super::OLDEST_VERSION),
        "never over the phone's: stale there, and kept as a copy"
    );
}

#[test]
fn another_address_for_an_account_lets_its_notes_go_and_never_sends_hers_to_the_other() {
    for keeps_local in [false, true] {
        let (mut s, account) = session();
        s.keeps_local = keeps_local;
        let offsite = by_remote(&s, "VN-1");
        run(&mut s, "notes.edit", json!({"id": offsite, "text": "Mine"}));
        run(&mut s, "account.edit", json!({"id": account, "email": "grace@example.org"}));
        assert!(s.mailbox.notes.changes.is_empty(), "nothing waits for another person's server");
        let kept = note(&s, offsite);
        assert!(kept.origin.is_local() && kept.text == "Mine", "her words kept: {keeps_local}");
        assert!(s.mailbox.notes.server(account).is_none());
        // The other address's server offers notes: hers don't go there.
        let other = NotesDownload { account, email: "grace@example.org".into(), offered: true, complete: true, ..Default::default() };
        run(&mut s, "sync.applyNotes", json!({"download": other}));
        assert!(note(&s, offsite).origin.is_local() && s.mailbox.notes.changes.is_empty(), "{keeps_local}");
    }
}

#[test]
fn a_held_note_waits_while_too_many_changes_wait_and_goes_back_once_there_is_room() {
    let (mut s, account) = session();
    s.keeps_local = false;
    let id = run(&mut s, "notes.new", json!({"title": "Lunch", "text": "Friday"}))["id"].as_u64().unwrap();
    let gone = NotesDownload { account, email: ME.into(), offered: false, ..Default::default() };
    run(&mut s, "sync.applyNotes", json!({"download": gone}));
    let full: Vec<mailcraft_model::note::NoteChange> = (0..mailcraft_model::note::MAX_NOTE_CHANGES)
        .map(|n| mailcraft_model::note::NoteChange { account, kind: NoteChangeKind::Delete, remote: format!("VN-x{n}"), ..Default::default() })
        .collect();
    s.mailbox.notes.changes = full;
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![])}));
    assert!(s.mailbox.notes.changes.len() <= mailcraft_model::note::MAX_NOTE_CHANGES);
    assert!(note(&s, id).origin.is_local(), "still held: no room to note it");
    s.mailbox.notes.changes.clear();
    run(&mut s, "sync.applyNotes", json!({"download": download(account, vec![])}));
    assert_eq!((note(&s, id).origin.account, kinds(&s)), (account, vec![NoteChangeKind::Create]));
}

#[test]
fn a_change_waiting_for_a_read_is_not_due_and_the_window_does_not_look_every_second_for_it() {
    let (s, account, _) = waiting_for_a_version();
    let waiting = super::notes_waiting(&s, account);
    assert_eq!((waiting.total, waiting.due), (1, 0), "it waits, for a read, not for a moment");
}
