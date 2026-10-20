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
    NotesDownload { account, email: ME.into(), offered: true, server: NoteServer::default(), complete: true, notes }
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
    // What goes is the change made here alone, from the version it was made from.
    let going = ops(&mut s, account);
    assert_eq!(going.len(), 1);
    assert_eq!(
        (going[0].fields.title.as_deref(), going[0].fields.pinned, going[0].version.as_str()),
        (Some("Offsite in May"), None, "2026-10-20T08:00:00.100Z")
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
    // In a session that keeps nothing of its own, it is not kept, and that is said.
    let (mut s, account) = session();
    s.keeps_local = false;
    run(&mut s, "notes.new", json!({"title": "Not sent yet"}));
    let said = run(&mut s, "sync.applyNotes", json!({"download": NotesDownload { account, email: ME.into(), ..Default::default() }}));
    assert_eq!(said["messages"], json!([super::super::words::NO_LONGER_KEPT_LOST]));
    assert!(s.mailbox.notes.items.is_empty());
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
        note: Some(remote("VN-1", "Offsite", "2026-10-20T11:00:00.000Z")),
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
    assert_eq!((note.text.as_str(), note.version.as_str()), ("Offsite words", ""), "as the server had it, until the next read");
}

#[test]
fn a_change_of_words_to_a_note_deleted_elsewhere_is_kept_as_a_new_note() {
    let (mut s, account) = session();
    let offsite = by_remote(&s, "VN-1");
    run(&mut s, "notes.edit", json!({"id": offsite, "text": "Mine"}));
    let said = answer(&mut s, account, NoteResult::Gone, "", None);
    assert_eq!(said["messages"], json!(["\"Offsite\" was deleted elsewhere meanwhile: your version is kept as a note of its own."]));
    assert!(s.mailbox.notes.note(offsite).is_none());
    assert!(s.mailbox.notes.items.values().any(|note| note.title == "Offsite" && note.text == "Mine" && note.origin.id.is_empty()));
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
    assert_eq!(said["messages"], json!(["The change to \"Mine now\" was put back as the server has it: Not permitted"]));
    assert_eq!(s.mailbox.notes.note(offsite).map(|note| note.title.as_str()), Some("Offsite"));
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
    let done = NoteOutcome { op: first[0].clone(), result: NoteResult::Done, note: Some(remote("VN-1", "Renamed", "v2")), ..Default::default() };
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
