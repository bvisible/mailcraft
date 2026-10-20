// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's address books, from end to end: a job that brings the contacts, worked out by hand (as the window
//! works it out), `mailcraft_jmap::sync_pim` over the in-process JMAP server (driven by `block_on`, as a host on one
//! thread drives it), then the engine's `sync.applyBooks` and `sync.applyContacts` for what came back, the state of the
//! contacts kept for the next job as the window keeps it. The server names its cards as one that calls a card by its
//! contact's name does. Nothing here leaves this machine; every name, address and id is invented.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicBool;

use mailcraft_engine::cmd::remote::apply_contacts;
use mailcraft_engine::cmd::remote_contacts::{NO_CHANGING, apply_books};
use mailcraft_engine::{CmdError, Session};
use mailcraft_jmap::standin::{Book, Card, Pim, StandIn, TOKEN};
use mailcraft_jmap::{Auth, Client, block_on, sync_pim};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use mailcraft_model::{Contact, Id, Mailbox};
use serde_json::json;

const ADDRESS: &str = "ada@example.org";
const SESSION: &str = "https://jmap.standin.example/jmap/session";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// A session with one JMAP account, and a server with the team's book (the default, which Ada may write) and the
/// partners' (which she may only read), and a few people in them.
struct Rig {
    server: StandIn,
    s: Session,
    account: Id,
    /// The state of the contacts at the last job that brought them all, as the window keeps it.
    state: String,
}

impl Rig {
    fn new() -> Rig {
        let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
        let added = s.run("account.add", &json!({"email": ADDRESS, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
        let server = StandIn::new();
        server.with(|d| {
            d.pim = Pim::offered();
            d.pim.add_book(Book { default: true, ..Book::new("neoffice", "Neoffice", true) });
            d.pim.add_book(Book::new("partners", "Partners", false));
            d.pim.add_card(Card::person("Grace Okafor", "neoffice", "Grace", "Okafor", "grace@example.org").working("Engineer", ""));
            d.pim.add_card(Card::person("Bob Lindqvist", "neoffice", "Bob", "Lindqvist", "bob@example.org").with_phone("+41 79 555 01 02", true));
            d.pim.add_card(Card::company("Lindqvist Joinery", "neoffice", "Lindqvist Joinery", "joinery@example.org"));
            d.pim.add_card(Card::person("Carol Duval", "partners", "Carol", "Duval", "carol@example.org"));
        });
        Rig { server, account: added["id"].as_u64().unwrap(), s, state: String::new() }
    }

    /// A job that brings the contacts alone, with the state held (from a read ten minutes ago).
    fn job(&self) -> JmapPimRequest {
        JmapPimRequest {
            account: self.account,
            email: ADDRESS.into(),
            session: SESSION.into(),
            bearer: true,
            me: vec![ADDRESS.into()],
            now: NOW,
            contacts: true,
            contacts_state: self.state.clone(),
            contacts_read: if self.state.is_empty() { 0 } else { NOW - 600 },
            ..Default::default()
        }
    }

    /// One job, as a host does it, and its answer taken in as the window takes it: the books, then the contacts; and
    /// the state kept, or forgotten.
    fn run(&mut self, request: &JmapPimRequest) -> JmapPimResult {
        let client = block_on(Client::connect(self.server.clone(), &request.session, &Auth::bearer(TOKEN))).unwrap();
        let result = block_on(sync_pim(&client, request, &AtomicBool::new(false)));
        if let Some(books) = &result.books {
            apply_books(&mut self.s, books).unwrap();
        }
        if let Some(contacts) = &result.contacts {
            apply_contacts(&mut self.s, contacts).unwrap();
        }
        self.state = result.contacts_state.clone().unwrap_or_default();
        result
    }

    /// The names of the contacts of the account's server here, in order.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.s.mailbox.contacts.values().filter(|c| c.origin.account == self.account).map(|c| c.name()).collect();
        names.sort();
        names
    }

    fn contact(&self, name: &str) -> Contact {
        self.s.mailbox.contacts.values().find(|c| c.name() == name).map(|c| c.as_ref().clone()).unwrap_or_else(|| panic!("no contact {name}"))
    }

    /// The server's name of the book `name`'s contact is in here.
    fn book_of(&self, name: &str) -> String {
        let contact = self.contact(name);
        self.s.mailbox.book_of(&contact).map(|book| book.origin.id.clone()).unwrap_or_default()
    }
}

#[test]
fn a_first_read_brings_the_book_and_its_cards() {
    let mut rig = Rig::new();
    let result = rig.run(&rig.job());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(rig.names(), ["Bob Lindqvist", "Carol Duval", "Grace Okafor", "Lindqvist Joinery"]);
    // Both books here, read-only as every book is until changes go back, and keeping only the basics.
    let books: Vec<(String, bool, bool, bool)> =
        rig.s.mailbox.address_books.iter().map(|b| (b.origin.id.clone(), b.read_only, b.main, b.plain)).collect();
    assert_eq!(books, [("neoffice".into(), true, true, true), ("partners".into(), true, false, true)]);
    assert_eq!((rig.book_of("Grace Okafor"), rig.book_of("Carol Duval")), ("neoffice".to_string(), "partners".to_string()));
    let bob = rig.contact("Bob Lindqvist");
    assert_eq!((bob.phones[0].label.as_str(), bob.emails[0].value.as_str()), ("mobile", "bob@example.org"));
    assert_eq!(rig.contact("Grace Okafor").title, "Engineer");
    assert!(rig.contact("Lindqvist Joinery").first.is_empty(), "a company of its own, under its company's name");
    // A contact the server keeps under a name with a space is the same contact at every read.
    assert!(!rig.state.is_empty());
    let grace = rig.contact("Grace Okafor").id;
    rig.state.clear();
    rig.run(&rig.job());
    assert_eq!(rig.contact("Grace Okafor").id, grace);
    // The book's rights hold here: its contacts can't be changed, and the engine says why.
    let refused = rig.s.run("people.edit", &json!({"id": grace, "first": "Gracie"}));
    assert_eq!(refused, Err(CmdError::Failed(NO_CHANGING.into())));
    // And the To field finds them.
    let found = mailcraft_engine::cmd::compose::suggest(&rig.s, "lindq", 6);
    assert_eq!(found.iter().map(|s| s.email.as_str()).collect::<Vec<_>>(), ["bob@example.org", "joinery@example.org"]);
}

#[test]
fn a_card_deleted_on_the_server_goes_at_the_next_whole_read() {
    let mut rig = Rig::new();
    rig.run(&rig.job());
    rig.server.with(|d| d.pim.cards.retain(|c| c.id != "Bob Lindqvist"));
    let result = rig.run(&rig.job());
    assert!(result.contacts.as_ref().is_some_and(|d| d.complete), "the state changed: everything was read");
    assert_eq!(rig.names(), ["Carol Duval", "Grace Okafor", "Lindqvist Joinery"]);
}

#[test]
fn a_read_cut_short_adds_and_updates_and_removes_nothing() {
    let mut rig = Rig::new();
    rig.run(&rig.job());
    rig.server.with(|d| {
        d.pim.cards.retain(|c| c.id != "Bob Lindqvist");
        d.pim.cards[0].title = "Chief engineer".into();
        d.pim.add_card(Card::person("Dan Moreau", "neoffice", "Dan", "Moreau", "dan@example.org"));
        // One card the server can't read just now: the read is not whole.
        d.pim.broken_cards.insert("Carol Duval".into());
    });
    let result = rig.run(&rig.job());
    assert!(result.contacts.as_ref().is_some_and(|d| !d.complete) && !result.errors.is_empty());
    assert_eq!(rig.names(), ["Bob Lindqvist", "Carol Duval", "Dan Moreau", "Grace Okafor", "Lindqvist Joinery"], "Bob stays: nothing goes");
    assert_eq!(rig.contact("Grace Okafor").title, "Chief engineer", "what was read is taken in");
    assert!(rig.state.is_empty(), "and the next job reads everything");
}

#[test]
fn a_card_the_server_renames_is_the_same_contact() {
    let mut rig = Rig::new();
    rig.run(&rig.job());
    let before = rig.contact("Grace Okafor");
    rig.s.run("people.favorite", &json!({"id": before.id, "value": true})).unwrap();
    rig.server.with(|d| {
        let grace = d.pim.cards.iter_mut().find(|c| c.id == "Grace Okafor").unwrap();
        grace.last = "Okafor-Mensah".into();
    });
    rig.run(&rig.job());
    let after = rig.contact("Grace Okafor-Mensah");
    assert_eq!((after.id, after.favorite), (before.id, true), "the same contact, with what is kept here");
    assert_eq!(rig.names().len(), 4);
}

#[test]
fn an_unchanged_state_changes_nothing_here() {
    let mut rig = Rig::new();
    rig.run(&rig.job());
    let (snapshot, rev) = (rig.s.mailbox.clone(), rig.s.rev());
    let before = rig.server.calls().len();
    let result = rig.run(&rig.job());
    assert!(result.contacts.is_none() && !rig.state.is_empty());
    assert_eq!(rig.server.calls().len() - before, 2, "the books, and one card's query");
    assert!(rig.s.mailbox == snapshot && rig.s.rev() == rev, "nothing to take in, nothing to save");
}
