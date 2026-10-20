// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A person's notes from end to end: a job that asks for them, worked out by hand (as the window works it out: the
//! changes to send from `sync.noteChanges`, the state held), `mailcraft_jmap::sync_pim` over the in-process JMAP
//! server (driven by `block_on`, as a host on one thread drives it), then what came back taken in as the window takes
//! it: the outcomes of the changes first (`sync.applyNoteChanges`), then the notes read (`sync.applyNotes`), and the
//! state kept for the next job. Nothing here leaves this machine; every name, address and id is invented.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicBool;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::notes::server::{apply_note_outcomes, merge_notes, note_upload};
use mailcraft_jmap::standin::{Notes, StandIn, StoredNote, TOKEN};
use mailcraft_jmap::{Auth, Client, block_on, sync_pim};
use mailcraft_model::note::{Note, NotesAsk};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use mailcraft_model::{Id, Mailbox};
use serde_json::json;

const ADDRESS: &str = "ada@example.org";
const SESSION: &str = "https://jmap.standin.example/jmap/session";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// A session with one JMAP account, and a server with Ada's notes: "Offsite" pinned, "Groceries", and "Codes" Grace
/// shares with her for reading.
struct Rig {
    server: StandIn,
    s: Session,
    account: Id,
    /// The state of the notes at the last job that read them all, as the window keeps it.
    state: String,
}

impl Rig {
    fn new() -> Rig {
        let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
        let added = s.run("account.add", &json!({"email": ADDRESS, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
        let server = StandIn::new();
        server.with(|d| {
            d.notes = Notes::offered();
            d.notes.add(StoredNote::new("VN-1", "Offsite", "Lake walk\nCooking class").pinned());
            d.notes.add(StoredNote::new("VN-2", "Groceries", "Milk\nBread"));
            d.notes.add(StoredNote::new("VN-3", "Codes", "4711").shared_by("grace@example.org", "Grace Okafor", false));
        });
        Rig { server, account: added["id"].as_u64().unwrap(), s, state: String::new() }
    }

    /// A job that reads the notes, with what waits to go to the server and the state held (from a read ten minutes
    /// ago).
    fn job(&mut self) -> JmapPimRequest {
        let upload = note_upload(&self.s, self.account).unwrap();
        let changes = (!upload.ops.is_empty()).then_some(upload);
        let read_at = if self.state.is_empty() { 0 } else { NOW - 600 };
        JmapPimRequest {
            account: self.account,
            email: ADDRESS.into(),
            session: SESSION.into(),
            bearer: true,
            me: vec![ADDRESS.into()],
            now: NOW,
            notes: Some(NotesAsk { read: true, state: self.state.clone(), read_at, changes }),
            ..Default::default()
        }
    }

    /// One job, as a host does it, and its answer taken in as the window takes it: the outcomes, then the notes; and
    /// the state kept, or forgotten.
    fn run(&mut self) -> JmapPimResult {
        let request = self.job();
        let client = block_on(Client::connect(self.server.clone(), &request.session, &Auth::bearer(TOKEN))).unwrap();
        let mut result = block_on(sync_pim(&client, &request, &AtomicBool::new(false)));
        result.tidy();
        let notes = result.notes.clone().unwrap_or_default();
        if let Some(uploaded) = &notes.uploaded {
            apply_note_outcomes(&mut self.s, uploaded).unwrap();
        }
        if let Some(download) = &notes.download {
            merge_notes(&mut self.s, download).unwrap();
        }
        self.state = notes.state.unwrap_or_default();
        result
    }

    /// The titles of the account's notes here, in order.
    fn titles(&self) -> Vec<String> {
        let mut titles: Vec<String> =
            self.s.mailbox.notes.items.values().filter(|n| n.origin.account == self.account).map(|n| n.title.clone()).collect();
        titles.sort();
        titles
    }

    fn note(&self, title: &str) -> Note {
        self.s.mailbox.notes.items.values().find(|n| n.title == title).map(|n| n.as_ref().clone()).unwrap_or_else(|| panic!("no note {title}"))
    }

    fn on_server(&self, title: &str) -> usize {
        self.server.with(|d| d.notes.list.iter().filter(|note| note.title == title).count())
    }
}

#[test]
fn a_first_read_brings_the_notes_and_the_server() {
    let mut rig = Rig::new();
    let result = rig.run();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(rig.titles(), ["Codes", "Groceries", "Offsite"]);
    let server = rig.s.mailbox.notes.server(rig.account).cloned().unwrap();
    assert_eq!((server.max_title, server.max_text), (140, 100_000));
    let codes = rig.note("Codes");
    assert!(codes.shared && !codes.rights.write && !codes.rights.delete);
    assert_eq!(codes.owner_name, "Grace Okafor");
    assert!(codes.web_url.starts_with("https://") && !codes.version.is_empty());
    assert!(rig.note("Offsite").pinned);
    assert!(!rig.state.is_empty());
    // The engine says where they are, and refuses what the rights refuse.
    let id = codes.id;
    assert_eq!(rig.s.run("notes.get", &json!({"id": id})).unwrap()["where"], ADDRESS);
    assert!(rig.s.run("notes.edit", &json!({"id": id, "text": "Mine"})).is_err());
}

#[test]
fn a_note_made_here_is_on_the_server_and_comes_back_as_one() {
    let mut rig = Rig::new();
    rig.run();
    let made = rig.s.run("notes.new", &json!({"text": "Team lunch\nFriday at noon"})).unwrap()["id"].as_u64().unwrap();
    // Its first job loses the answer: the server made it all the same.
    rig.server.with(|d| d.lost_answers = 1);
    rig.run();
    assert_eq!(rig.on_server("Team lunch"), 1);
    let lunches =
        |rig: &Rig| -> Vec<Note> { rig.s.mailbox.notes.items.values().filter(|n| n.heading() == "Team lunch").map(|n| n.as_ref().clone()).collect() };
    assert_eq!(lunches(&rig).len(), 2, "the one that waits to go again, and the server's, which the read brought");
    // The next, a little later, sends it again with the same mark: the same note, and one here.
    rig.s.clock.now = Some(NOW + 120);
    rig.run();
    assert_eq!(rig.on_server("Team lunch"), 1, "the mark makes no second one");
    let lunch = lunches(&rig);
    assert_eq!(lunch.len(), 1, "{lunch:?}");
    assert_eq!(lunch[0].id, made, "the one made here, with the server's name");
    assert!(lunch[0].origin.id.starts_with("VN-") && rig.s.mailbox.notes.changes.is_empty());
    // Pinned and deleted here, it is so there.
    rig.s.run("notes.pin", &json!({"id": made})).unwrap();
    rig.run();
    assert!(rig.server.with(|d| d.notes.list.iter().any(|note| note.title == "Team lunch" && note.pinned)));
    rig.s.run("notes.delete", &json!({"id": made})).unwrap();
    rig.run();
    assert_eq!(rig.on_server("Team lunch"), 0);
    assert_eq!(rig.titles(), ["Codes", "Groceries", "Offsite"]);
}

#[test]
fn an_unchanged_state_changes_nothing_here() {
    let mut rig = Rig::new();
    rig.run();
    let (snapshot, rev) = (rig.s.mailbox.clone(), rig.s.rev());
    let before = rig.server.calls().len();
    let result = rig.run();
    assert!(result.notes.as_ref().is_some_and(|notes| notes.download.is_none()) && !rig.state.is_empty());
    assert_eq!(rig.server.calls().len() - before, 1, "one query for one id");
    assert!(rig.s.mailbox == snapshot && rig.s.rev() == rev, "nothing to take in, nothing to save");
    // A change sent from that very state keeps it: the state the server says after it is held.
    let groceries = rig.note("Groceries").id;
    rig.s.run("notes.archive", &json!({"id": groceries})).unwrap();
    let before = rig.server.calls().len();
    rig.run();
    assert_eq!(rig.server.calls().into_iter().skip(before).collect::<Vec<_>>(), ["Note/set", "Note/query"]);
    assert_eq!(rig.state, rig.server.with(|d| d.notes.state()));
}

#[test]
fn a_stale_edit_is_kept_as_a_copy() {
    let mut rig = Rig::new();
    rig.run();
    let groceries = rig.note("Groceries").id;
    rig.s.run("notes.edit", &json!({"id": groceries, "text": "Milk\nBread\nEggs"})).unwrap();
    // Meanwhile, on the phone.
    rig.server.with(|d| d.notes.change("VN-2", |note| note.text = "Milk\nCoffee".into()));
    let result = rig.run();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    // Both versions: the phone's in the note, Ada's as a note of its own, made on the server at the next job.
    assert_eq!(rig.note("Groceries").text, "Milk\nCoffee");
    assert_eq!(rig.note("Groceries (2)").text, "Milk\nBread\nEggs");
    rig.run();
    assert_eq!(rig.on_server("Groceries (2)"), 1);
    assert!(rig.s.mailbox.notes.changes.is_empty());
    assert_eq!(rig.titles(), ["Codes", "Groceries", "Groceries (2)", "Offsite"]);
}
