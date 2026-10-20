// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The notes' commands: where notes are offered, what may be done to each, and what an agent may do. Invented people at
//! `example.org`.

use std::sync::Arc;

use mailcraft_model::note::{MAX_NOTE_TEXT, Note, NoteRights, NoteServer};
use mailcraft_model::{Account, Id, Origin};
use serde_json::{Value, json};

use super::*;
use crate::agent::{self, Access, Class};
use crate::{CmdError, Session};

const ME: &str = "ada@example.org";

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(s: &mut Session, id: &str, params: Value) -> String {
    match s.run(id, &params) {
        Err(CmdError::Failed(why) | CmdError::Disabled(why)) => why,
        other => panic!("{id} {params}: expected a refusal, got {other:?}"),
    }
}

/// A session with an account whose server keeps notes, and that account.
fn with_server() -> (Session, Id) {
    let mut s = Session::new(crate::sample::mailbox(1_791_993_600, 0));
    let account = s.mailbox.add_account(Account { email: ME.into(), ..Default::default() });
    s.mailbox.notes.servers.push(NoteServer { account, ..Default::default() });
    (s, account)
}

/// A note of the server's, as a download would have made it.
fn server_note(s: &mut Session, account: Id, remote: &str, title: &str, rights: NoteRights) -> Id {
    let id = s.mailbox.alloc_id();
    let note = Note {
        id,
        origin: Origin { account, id: remote.into(), ..Default::default() },
        title: title.into(),
        text: "Words".into(),
        version: "2026-10-20T08:00:00.123Z".into(),
        rights,
        ..Default::default()
    };
    s.mailbox.notes.items.insert(id, Arc::new(note));
    id
}

fn title(s: &Session, id: Id) -> String {
    s.mailbox.notes.note(id).map(|note| note.title.clone()).unwrap_or_default()
}

#[test]
fn notes_are_offered_where_kept_here_or_where_a_server_keeps_them() {
    // On this computer: offered, and a new note is the mailbox's own.
    let mut s = Session::new(crate::sample::mailbox(1_791_993_600, 0));
    assert!(offered(&s));
    run(&mut s, "view.notes", json!({}));
    assert_eq!(s.view.module, Module::Notes);
    let id = run(&mut s, "notes.new", json!({"title": "Groceries", "text": "Milk"}))["id"].as_u64().unwrap();
    assert!(s.mailbox.notes.note(id).is_some_and(|note| note.origin.is_local()));
    // Where a server keeps notes, a new one goes to it, or to the account named.
    let (mut s, account) = with_server();
    let id = run(&mut s, "notes.new", json!({"title": "Offsite"}))["id"].as_u64().unwrap();
    assert_eq!(s.mailbox.notes.note(id).map(|note| note.origin.account), Some(account));
    let why = refused(&mut s, "notes.new", json!({"title": "x", "account": 4_242}));
    assert!(why.contains("no server keeps notes"), "{why}");
}

#[test]
fn a_hosting_pages_tab_without_a_notes_server_offers_no_notes() {
    let mut s = Session::new(crate::sample::mailbox(1_791_993_600, 0));
    s.keeps_local = false;
    assert!(!offered(&s));
    for (id, params) in [("view.notes", json!({})), ("notes.new", json!({"title": "Lost"})), ("notes.list", json!({}))] {
        assert_eq!(refused(&mut s, id, params), NO_NOTES_HERE, "{id}");
    }
    assert_ne!(s.view.module, Module::Notes);
    assert!(s.mailbox.notes.items.is_empty());
    // Once its server keeps notes, they are offered there.
    let account = s.mailbox.add_account(Account { email: ME.into(), ..Default::default() });
    s.mailbox.notes.servers.push(NoteServer { account, ..Default::default() });
    assert!(offered(&s));
    run(&mut s, "view.notes", json!({}));
}

#[test]
fn a_local_note_is_made_edited_pinned_archived_and_deleted_and_undo_takes_each_back() {
    let mut s = Session::new(crate::sample::mailbox(1_791_993_600, 0));
    let id = run(&mut s, "notes.new", json!({"title": "Groceries", "text": "Milk\nBread"}))["id"].as_u64().unwrap();
    assert_eq!(s.sel.note, Some(id));
    run(&mut s, "notes.edit", json!({"text": "Milk\nBread\nEggs"}));
    assert_eq!(s.mailbox.notes.note(id).map(|note| note.text.lines().count()), Some(3));
    run(&mut s, "notes.pin", json!({}));
    run(&mut s, "notes.archive", json!({"value": true}));
    let note = s.mailbox.notes.note(id).cloned().unwrap();
    assert!(note.pinned && note.archived);
    // The lists: the archived one is in the archive, and found by its words.
    let listed = |s: &mut Session, params: Value| -> Vec<u64> {
        run(s, "notes.list", params)["notes"].as_array().unwrap().iter().filter_map(|n| n["id"].as_u64()).collect()
    };
    assert!(listed(&mut s, json!({})).is_empty());
    assert_eq!(listed(&mut s, json!({"archived": true, "query": "eggs"})), [id]);
    assert_eq!(run(&mut s, "notes.get", json!({"id": id}))["where"], "on this computer");
    run(&mut s, "notes.delete", json!({}));
    assert!(s.mailbox.notes.note(id).is_none());
    // Undo takes each back, one step at a time.
    assert!(s.undo());
    assert!(s.mailbox.notes.note(id).is_some_and(|note| note.archived));
    assert!(s.undo());
    assert!(s.mailbox.notes.note(id).is_some_and(|note| !note.archived && note.pinned));
    assert!(s.undo());
    assert!(s.mailbox.notes.note(id).is_some_and(|note| !note.pinned));
    assert!(s.undo());
    assert_eq!(s.mailbox.notes.note(id).map(|note| note.text.as_str()), Some("Milk\nBread"));
    assert!(s.undo());
    assert!(s.mailbox.notes.note(id).is_none());
    // A note without a link says so.
    let id = run(&mut s, "notes.new", json!({"text": "Untitled words"}))["id"].as_u64().unwrap();
    assert_eq!(refused(&mut s, "notes.link", json!({"id": id})), NOTE_NO_LINK);
}

#[test]
fn a_title_is_one_line_and_a_title_or_text_over_the_limit_is_refused() {
    let mut s = Session::new(crate::sample::mailbox(1_791_993_600, 0));
    let id = run(&mut s, "notes.new", json!({"title": "Two\nlines"}))["id"].as_u64().unwrap();
    assert_eq!(title(&s, id), "Two lines");
    assert_eq!(refused(&mut s, "notes.edit", json!({"title": "t".repeat(500)})), "A note's title holds 140 characters at most");
    assert_eq!(title(&s, id), "Two lines", "never cut");
    assert_eq!(refused(&mut s, "notes.edit", json!({"text": "x".repeat(MAX_NOTE_TEXT + 1)})), "A note holds 100000 characters at most");
    assert_eq!(refused(&mut s, "notes.new", json!({"text": "x".repeat(MAX_NOTE_TEXT + 1)})), "A note holds 100000 characters at most");
    // A server that takes shorter ones refuses beyond its limit, and says its limit.
    let (mut s, account) = with_server();
    if let Some(server) = s.mailbox.notes.servers.iter_mut().find(|server| server.account == account) {
        (server.max_title, server.max_text) = (20, 10);
    }
    assert_eq!(refused(&mut s, "notes.new", json!({"title": "A title longer than twenty"})), "A note's title holds 20 characters at most");
    run(&mut s, "notes.new", json!({"title": "Short"}));
    assert_eq!(refused(&mut s, "notes.edit", json!({"text": "eleven char"})), "A note holds 10 characters at most");
}

#[test]
fn a_rich_notes_text_is_refused_and_its_title_may_change() {
    let (mut s, account) = with_server();
    let id = server_note(&mut s, account, "VN-1", "Quarterly plan", NoteRights { write_text: false, ..Default::default() });
    if let Some(note) = s.mailbox.notes.note_mut(id) {
        note.rich = true;
        note.web_url = "https://desk.example.org/app/notes?note=VN-1".into();
    }
    assert_eq!(refused(&mut s, "notes.edit", json!({"id": id, "text": "plain"})), NOTE_TEXT_ELSEWHERE);
    run(&mut s, "notes.edit", json!({"id": id, "title": "Quarterly plan (draft)"}));
    assert_eq!(title(&s, id), "Quarterly plan (draft)");
    assert_eq!(run(&mut s, "notes.link", json!({"id": id}))["url"], "https://desk.example.org/app/notes?note=VN-1");
    assert_eq!(run(&mut s, "notes.get", json!({"id": id}))["where"], ME);
}

#[test]
fn a_note_shared_for_reading_refuses_every_change_and_one_shared_for_writing_refuses_delete() {
    let (mut s, account) = with_server();
    let read_only = server_note(&mut s, account, "VN-2", "Room booking codes", NoteRights { write: false, write_text: false, delete: false });
    for (id, params) in [
        ("notes.edit", json!({"id": read_only, "title": "Mine now"})),
        ("notes.edit", json!({"id": read_only, "text": "Mine now"})),
        ("notes.pin", json!({"id": read_only})),
        ("notes.archive", json!({"id": read_only})),
    ] {
        assert_eq!(refused(&mut s, id, params), NOTE_READ_ONLY, "{id}");
    }
    assert_eq!(refused(&mut s, "notes.delete", json!({"id": read_only})), NOTE_NOT_YOURS);
    let writable = server_note(&mut s, account, "VN-3", "Shared plan", NoteRights { delete: false, ..Default::default() });
    run(&mut s, "notes.edit", json!({"id": writable, "text": "Changed together"}));
    assert_eq!(refused(&mut s, "notes.delete", json!({"id": writable})), NOTE_NOT_YOURS);
    // Nothing changed of what was refused.
    assert_eq!(title(&s, read_only), "Room booking codes");
}

#[test]
fn agents_read_freely_writes_to_a_servers_note_ask_and_deletes_are_delete_class() {
    let (mut s, account) = with_server();
    let theirs = server_note(&mut s, account, "VN-4", "Offsite", NoteRights::default());
    let own = s.mailbox.alloc_id();
    s.mailbox.notes.items.insert(own, Arc::new(Note { id: own, title: "Mine".into(), ..Default::default() }));
    for id in ["view.notes", "notes.list", "notes.get", "notes.select", "notes.link"] {
        assert_eq!(agent::classify(&s, id, &json!({"id": theirs})), Access::Safe, "{id}");
    }
    for id in ["notes.edit", "notes.pin", "notes.archive"] {
        assert_eq!(agent::classify(&s, id, &json!({"id": theirs})), Access::Owner(Class::Send), "{id}");
        assert_eq!(agent::classify(&s, id, &json!({"id": own})), Access::Safe, "{id} of the mailbox's own");
    }
    assert_eq!(agent::classify(&s, "notes.new", &json!({})), Access::Owner(Class::Send), "it goes to the server");
    assert_eq!(agent::classify(&s, "notes.delete", &json!({"id": own})), Access::Owner(Class::Delete));
    for id in ["sync.applyNotes", "sync.noteChanges", "sync.applyNoteChanges"] {
        assert_eq!(agent::access(id), Some(Access::Never), "{id}");
    }
    // As a server's contact is.
    assert!(agent::describe(&s, "notes.edit", &json!({"id": theirs})).contains("Offsite"));
    // The window's opening of a note's link in the system's browser is the owner's to allow, as a meeting's is.
    assert_eq!(agent::access("ui.openNote"), Some(Access::Owner(Class::Send)));
    assert!(agent::describe(&s, "ui.openNote", &json!({"id": theirs})).contains("on the web"));
}

#[test]
fn notes_edit_asks_an_agent_as_people_edit_does() {
    // The mailbox's own: safe, as a contact of its own is; a server's: written for the person there, Send, as a contact
    // of a server's address book is.
    assert_eq!(agent::access("notes.edit"), agent::access("people.edit"));
    let (mut s, account) = with_server();
    let theirs = server_note(&mut s, account, "VN-9", "Offsite", NoteRights::default());
    assert_eq!(agent::classify(&s, "notes.edit", &json!({"id": theirs})), Access::Owner(Class::Send));
}
