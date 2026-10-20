// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The address books of a JMAP account's server in the engine: the books taken in, the contacts placed in them, what
//! the owner may do with each, a session that keeps nothing, and the file. The server's side is written out by hand, as
//! a host brings it (`mailcraft-jmap`'s reading of JMAP for Contacts). Invented people at `example.org`.

use mailcraft_model::book::{BooksDownload, RemoteBook};
use mailcraft_model::remote::{ContactsDownload, RemoteContact};
use mailcraft_model::{Contact, Id, Labeled, Mailbox, Origin};
use mailcraft_time::{Date, DateTime, Timestamp};
use serde_json::{Value, json};

use super::{NO_ADDING, NO_CHANGING, NO_DELETING, apply_books, book_here, merge_books};
use crate::agent::{self, Access};
use crate::cmd::remote::{apply_contacts, forget};
use crate::{CmdError, Session};

const ME: &str = "ada@example.org";
/// What the server of a mail program that keeps a team's contacts says of one it keeps (`changeable`).
const NOT_SENT_BACK: &str =
    "This contact is kept by the server of ada@example.org. Changes made here are not sent back yet, so it can't be changed or deleted here.";

fn at(year: i32, month: u8, day: u8, hour: u8) -> Timestamp {
    DateTime::new(Date::new(year, month, day).unwrap(), hour, 0, 0).unwrap().to_timestamp(0)
}

/// The sample mailbox on Wednesday 2026-10-14 with a JMAP account, and that account's id.
fn session() -> (Session, Id) {
    let now = at(2026, 10, 14, 16);
    let mut s = Session::at(crate::sample::mailbox(now, 0), now, 0);
    let added = s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap/session"}})).unwrap();
    (s, added["id"].as_u64().unwrap())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Why a command was refused: by the command (`Failed`) or before it ran (`Disabled`, a greyed control's reason).
fn refused(s: &mut Session, id: &str, params: Value) -> String {
    match s.run(id, &params) {
        Err(CmdError::Failed(why) | CmdError::Disabled(why)) => why,
        other => panic!("{id} {params}: expected a refusal, got {other:?}"),
    }
}

/// Why a command's control is greyed now, if it is.
fn greyed(s: &Session, id: &str) -> Option<&'static str> {
    s.registry.get(id).and_then(|spec| (spec.enabled)(s))
}

fn book(id: &str, name: &str, read_only: bool) -> RemoteBook {
    RemoteBook { id: id.into(), name: name.into(), read_only, may_remove: !read_only, ..Default::default() }
}

fn books(account: Id, books: Vec<RemoteBook>) -> BooksDownload {
    BooksDownload { account, email: ME.into(), complete: true, books }
}

fn card(id: &str, first: &str, email: &str, book: &str) -> RemoteContact {
    RemoteContact { id: id.into(), book: book.into(), first: first.into(), emails: vec![Labeled::new("", email)], ..Default::default() }
}

fn cards(account: Id, contacts: Vec<RemoteContact>) -> ContactsDownload {
    ContactsDownload { account, email: ME.into(), complete: true, contacts }
}

/// The contact the server calls `remote`.
fn kept(s: &Session, account: Id, remote: &str) -> Contact {
    let found = s.mailbox.contacts.values().find(|c| c.origin.account == account && c.origin.id == remote);
    found.map(|c| c.as_ref().clone()).unwrap_or_else(|| panic!("no contact {remote}"))
}

/// The server's name of the book a contact is in here, or `""` for none.
fn book_of(s: &Session, account: Id, remote: &str) -> String {
    let contact = kept(s, account, remote);
    s.mailbox.book_of(&contact).map(|book| book.origin.id.clone()).unwrap_or_default()
}

/// The books of `account` here, by the server's name, with whether each may only be read.
fn listed(s: &Session, account: Id) -> Vec<(String, String, bool)> {
    s.mailbox.address_books.iter().filter(|b| b.origin.account == account).map(|b| (b.origin.id.clone(), b.name.clone(), b.read_only)).collect()
}

/// The team's book, which Ada may change, and Grace's, which she may only read; a contact in each.
fn the_two_books(s: &mut Session, account: Id) {
    let mut team = book("team", "Team", false);
    team.main = true;
    apply_books(s, &books(account, vec![team, book("grace", "Grace's", true)])).unwrap();
    apply_contacts(s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace")])).unwrap();
}

#[test]
fn a_download_with_books_makes_the_books_and_places_each_contact_in_one() {
    let (mut s, account) = session();
    let (step, rev) = (s.undo_label(), s.rev());
    let mut team = book("team", "Team", false);
    team.main = true;
    team.plain = true;
    let answer = run(&mut s, "sync.applyBooks", json!({"download": books(account, vec![team, book("grace", "", true)])}));
    assert_eq!(answer, json!({"added": 2, "updated": 0, "removed": 0, "complete": true}));
    assert_eq!(listed(&s, account), [("team".into(), "Team".into(), false), ("grace".into(), "Contacts".into(), true)]);
    let team = s.mailbox.address_book_on_server(account, "team").cloned().unwrap();
    assert!(team.main && team.plain && team.may_remove && team.id != 0);
    // Taking a list in is no undo step, and is said to whoever shows the mailbox.
    assert!(s.undo_label() == step && s.rev() > rev);
    // Each contact is in the book it says, and one in a book the mailbox doesn't have, or none, is in none.
    let download = cards(
        account,
        vec![
            card("c-bob", "Bob", "bob@example.org", "team"),
            card("c-cy", "Cy", "cy@example.org", "grace"),
            card("c-dee", "Dee", "dee@example.org", "elsewhere"),
            card("c-eve", "Eve", "eve@example.org", ""),
        ],
    );
    apply_contacts(&mut s, &download).unwrap();
    let placed: Vec<String> = ["c-bob", "c-cy", "c-dee", "c-eve"].iter().map(|c| book_of(&s, account, c)).collect();
    assert_eq!(placed, ["team", "grace", "", ""]);
    assert_eq!(kept(&s, account, "c-bob").book, team.id);
    assert_eq!(book_here(&s.mailbox, account, "grace"), s.mailbox.address_book_on_server(account, "grace").unwrap().id);
    assert_eq!(book_here(&s.mailbox, account, "elsewhere"), 0);
    // Again: nothing changes, and nothing is to be saved for it.
    let (snapshot, rev) = (s.mailbox.clone(), s.rev());
    assert_eq!(
        apply_books(&mut s, &books(account, vec![book("grace", "", true), RemoteBook { main: true, plain: true, ..book("team", "Team", false) }]))
            .unwrap()["updated"],
        0
    );
    apply_contacts(&mut s, &download).unwrap();
    assert!(s.mailbox == snapshot && s.rev() == rev);
    // A book whose rights change there changes here.
    apply_books(&mut s, &books(account, vec![book("team", "Team", true), book("grace", "", true)])).unwrap();
    assert_eq!(listed(&s, account)[0], ("team".into(), "Team".into(), true));
    // Somebody else's list, or nobody's, changes nothing.
    let snapshot = s.mailbox.clone();
    let stranger = BooksDownload { email: "eve@example.org".into(), ..books(account, Vec::new()) };
    assert_eq!(refused(&mut s, "sync.applyBooks", json!({"download": stranger})), "this download was for another account, not for ada@example.org");
    assert!(merge_books(&mut s.mailbox, &books(424_242, Vec::new())).is_err());
    assert!(matches!(s.run("sync.applyBooks", &json!({})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("sync.applyBooks", &json!({"download": {"books": 5}})), Err(CmdError::Params(_))));
    assert!(s.mailbox == snapshot);
    assert!(s.mailbox.clone().repair().is_empty());
}

#[test]
fn a_book_the_server_no_longer_lists_goes_with_its_contacts_on_a_whole_download() {
    let (mut s, account) = session();
    the_two_books(&mut s, account);
    // Grace's book is no more: it goes, and its contact is in none until the contacts come down.
    let answer = apply_books(&mut s, &books(account, vec![book("team", "Team", false)])).unwrap();
    assert_eq!(answer["removed"], 1);
    assert_eq!(listed(&s, account), [("team".into(), "Team".into(), false)]);
    assert_eq!(book_of(&s, account, "c-cy"), "");
    // The contacts, all of them, without Cy: Cy goes.
    apply_contacts(&mut s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team")])).unwrap();
    assert!(!s.mailbox.contacts.values().any(|c| c.first == "Cy"));
    assert_eq!(book_of(&s, account, "c-bob"), "team");
}

#[test]
fn an_incomplete_download_removes_nothing() {
    let (mut s, account) = session();
    the_two_books(&mut s, account);
    let snapshot = s.mailbox.clone();
    // A list of the books that isn't all of them, and contacts that aren't all of them: nothing goes.
    let team = RemoteBook { main: true, ..book("team", "Team", false) };
    apply_books(&mut s, &BooksDownload { complete: false, ..books(account, vec![team]) }).unwrap();
    apply_contacts(&mut s, &ContactsDownload { complete: false, ..cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team")]) }).unwrap();
    assert!(s.mailbox == snapshot, "Grace's book and Cy are still here");
    assert_eq!(book_of(&s, account, "c-cy"), "grace");
}

#[test]
fn a_contact_that_moves_to_another_book_on_the_server_follows_it() {
    let (mut s, account) = session();
    the_two_books(&mut s, account);
    let before = kept(&s, account, "c-cy");
    run(&mut s, "people.favorite", json!({"id": before.id, "value": true}));
    // On the server, Cy is in the team's book now; Grace's book goes, and Cy with it from there, not from here.
    apply_books(&mut s, &books(account, vec![book("team", "Team", false)])).unwrap();
    apply_contacts(&mut s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "team")]))
        .unwrap();
    let after = kept(&s, account, "c-cy");
    assert_eq!((after.id, after.favorite), (before.id, true), "the same contact, with what is kept here");
    assert_eq!(book_of(&s, account, "c-cy"), "team");
    // And between two books that both stay.
    the_two_books(&mut s, account);
    assert_eq!(book_of(&s, account, "c-cy"), "grace");
}

#[test]
fn a_read_only_book_marks_its_contacts_and_edit_and_delete_refuse_with_the_reason() {
    let (mut s, account) = session();
    the_two_books(&mut s, account);
    let cy = kept(&s, account, "c-cy");
    assert!(s.mailbox.book_of(&cy).is_some_and(|book| book.read_only));
    let snapshot = s.mailbox.clone();
    // Named, as an agent names it: the command says why.
    assert_eq!(refused(&mut s, "people.edit", json!({"id": cy.id, "first": "Cyril"})), NO_CHANGING);
    assert_eq!(refused(&mut s, "people.delete", json!({"id": cy.id})), NO_CHANGING);
    // Selected, as the ribbon has it: the controls are greyed with the reason, and say it when pressed.
    run(&mut s, "people.select", json!({"id": cy.id}));
    assert_eq!((greyed(&s, "people.edit"), greyed(&s, "people.delete")), (Some(NO_CHANGING), Some(NO_CHANGING)));
    assert_eq!(refused(&mut s, "people.edit", json!({"first": "Cyril"})), NO_CHANGING);
    assert_eq!(refused(&mut s, "people.delete", json!({})), NO_CHANGING);
    assert!(s.mailbox == snapshot);
    // What is kept here only may still be set: whether it is a favourite, and its categories.
    run(&mut s, "people.favorite", json!({"id": cy.id, "value": true}));
    run(&mut s, "people.categorize", json!({"id": cy.id, "category": "Red category"}));
    // A contact of the mailbox's own is as ever, and nothing is said against it.
    let own = run(&mut s, "people.new", json!({"first": "Fay", "email": "fay@example.org", "book": 0}))["id"].as_u64().unwrap();
    assert_eq!((greyed(&s, "people.edit"), greyed(&s, "people.delete")), (None, None));
    run(&mut s, "people.edit", json!({"id": own, "last": "Park"}));
    run(&mut s, "people.delete", json!({"id": own}));
    // Nothing is selected: the controls say that, as before.
    s.sel.contact = None;
    assert_eq!(greyed(&s, "people.edit"), Some("select a contact first"));
}

#[test]
fn a_book_whose_cards_cannot_be_deleted_refuses_delete_only() {
    let (mut s, account) = session();
    let changes_only = RemoteBook { may_remove: false, ..book("team", "Team", false) };
    apply_books(&mut s, &books(account, vec![changes_only])).unwrap();
    apply_contacts(&mut s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team")])).unwrap();
    let bob = kept(&s, account, "c-bob").id;
    run(&mut s, "people.select", json!({"id": bob}));
    assert_eq!((greyed(&s, "people.edit"), greyed(&s, "people.delete")), (None, Some(NO_DELETING)));
    assert_eq!(refused(&mut s, "people.delete", json!({"id": bob})), NO_DELETING);
    // Changing it is the account's right: the change goes to the server (`written`).
    run(&mut s, "people.edit", json!({"id": bob, "first": "Robert"}));
    assert_eq!(kept(&s, account, "c-bob").first, "Robert");
    assert_eq!(s.mailbox.contact_changes.len(), 1);
    // A book whose contacts may be deleted too says nothing against Delete either, and the deletion goes.
    apply_books(&mut s, &books(account, vec![book("team", "Team", false)])).unwrap();
    assert_eq!(greyed(&s, "people.delete"), None);
    run(&mut s, "people.delete", json!({"id": bob}));
    assert!(s.mailbox.contact(bob).is_none());
    assert_eq!(s.mailbox.contact_changes.iter().map(|c| c.remote.as_str()).collect::<Vec<_>>(), ["c-bob"]);
}

#[test]
fn a_graph_contact_is_refused_with_its_present_words() {
    let now = at(2026, 10, 14, 16);
    let mut s = Session::at(crate::sample::mailbox(now, 0), now, 0);
    let added = run(&mut s, "account.add", json!({"email": ME, "incoming": {"host": "outlook.example.org"}, "auth": "oauth2"}));
    let account = added["id"].as_u64().unwrap();
    // Microsoft's contacts come in no book.
    apply_contacts(&mut s, &cards(account, vec![RemoteContact { book: String::new(), ..card("c-bob", "Bob", "bob@example.org", "") }])).unwrap();
    let bob = kept(&s, account, "c-bob");
    assert!(bob.book == 0 && s.mailbox.book_of(&bob).is_none());
    run(&mut s, "people.select", json!({"id": bob.id}));
    assert_eq!((greyed(&s, "people.edit"), greyed(&s, "people.delete")), (None, None), "as before: refused when pressed");
    assert_eq!(refused(&mut s, "people.edit", json!({"id": bob.id, "first": "Robert"})), NOT_SENT_BACK);
    assert_eq!(refused(&mut s, "people.delete", json!({"id": bob.id})), NOT_SENT_BACK);
}

#[test]
fn two_contacts_with_one_origin_leave_one() {
    let (mut s, account) = session();
    the_two_books(&mut s, account);
    // A second copy of Bob, as a create whose answer was lost would make one, under the server's own name for him.
    let twin = s.mailbox.add_contact(Contact {
        first: "Bob".into(),
        emails: vec![Labeled::new("", "bob@example.org")],
        origin: Origin { account, id: "c-bob".into(), ..Default::default() },
        ..Default::default()
    });
    assert_eq!(s.mailbox.contacts.values().filter(|c| c.origin.id == "c-bob").count(), 2);
    apply_contacts(&mut s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace")]))
        .unwrap();
    let bobs: Vec<Id> = s.mailbox.contacts.values().filter(|c| c.origin.id == "c-bob").map(|c| c.id).collect();
    assert_eq!(bobs, [twin], "the next whole download leaves one, the later");
    assert_eq!(book_of(&s, account, "c-bob"), "team");
}

#[test]
fn a_tab_that_keeps_nothing_refuses_a_local_contact_with_the_reason() {
    let (mut s, account) = session();
    the_two_books(&mut s, account);
    let message = *s.mailbox.messages.keys().next().unwrap();
    s.keeps_local = false;
    // A book Ada may change takes a new contact, there: nothing is kept here only.
    assert_eq!(greyed(&s, "people.new"), None);
    let made = run(&mut s, "people.new", json!({"first": "Gus", "email": "gus@example.org"}))["id"].as_u64().unwrap();
    assert_eq!(book_of(&s, account, ""), "team", "made in the team's book");
    assert!(s.mailbox.contact(made).is_some_and(|c| c.origin.account == account));
    assert_eq!(refused(&mut s, "people.new", json!({"first": "Hal", "book": 0})), NO_ADDING, "not the mailbox's own");
    run(&mut s, "people.delete", json!({"id": made}));
    // None does: the team's book is read-only now.
    let team = RemoteBook { main: true, ..book("team", "Team", true) };
    apply_books(&mut s, &books(account, vec![team, book("grace", "Grace's", true)])).unwrap();
    let snapshot = s.mailbox.clone();
    // New Contact: greyed with the reason, and refused however it is asked for.
    assert_eq!(greyed(&s, "people.new"), Some(NO_ADDING));
    assert!(!s.enabled("people.new"));
    assert_eq!(refused(&mut s, "people.new", json!({"first": "Gus", "email": "gus@example.org"})), NO_ADDING);
    // Add to Contacts: greyed with the selection, and refused for a message that is named too.
    s.sel.messages = vec![message];
    assert_eq!(greyed(&s, "people.fromMessage"), Some(NO_ADDING));
    assert_eq!(refused(&mut s, "people.fromMessage", json!({})), NO_ADDING);
    let sender = s.mailbox.message(message).map(|m| m.from.email.clone()).unwrap();
    if s.mailbox.contact_by_email(&sender).is_none() {
        assert_eq!(refused(&mut s, "people.fromMessage", json!({"id": message})), NO_ADDING);
    }
    assert!(s.mailbox == snapshot, "nothing was made that would be gone at the next visit");
    // Without a selection, Add to Contacts says what it lacks first.
    s.sel.messages.clear();
    assert_eq!(greyed(&s, "people.fromMessage"), Some("select a message first"));
    // The contacts of the server are still found and shown.
    assert_eq!(run(&mut s, "people.list", json!({"query": "Bob"}))["contacts"][0]["email"], "bob@example.org");
    // A session that keeps what it makes makes it, as one of the mailbox's own.
    s.keeps_local = true;
    assert_eq!(greyed(&s, "people.new"), None);
    let made = run(&mut s, "people.new", json!({"first": "Gus", "email": "gus@example.org"}));
    assert!(s.mailbox.contact(made["id"].as_u64().unwrap()).is_some_and(|c| c.origin.is_local() && c.book == 0));
    s.sel.messages = vec![message];
    assert_eq!(greyed(&s, "people.fromMessage"), None);
}

#[test]
fn forget_removes_the_accounts_books() {
    let (mut s, account) = session();
    the_two_books(&mut s, account);
    let other =
        run(&mut s, "account.add", json!({"email": "bo@example.org", "kind": "jmap", "jmap": {"session": "https://bo.example.org/jmap"}}))["id"]
            .as_u64()
            .unwrap();
    apply_books(&mut s, &BooksDownload { email: "bo@example.org".into(), ..books(other, vec![book("team", "Bo's team", true)]) }).unwrap();
    let step = s.undo_label();
    assert_eq!(run(&mut s, "sync.forgetRemote", json!({"account": account})), json!({"events": 0, "contacts": 2, "calendars": 0}));
    assert!(listed(&s, account).is_empty());
    assert_eq!(listed(&s, other), [("team".into(), "Bo's team".into(), true)], "another account's stay");
    assert!(s.undo_label() == step);
    // Its books alone are worth forgetting too; and then there is nothing left to forget.
    apply_books(&mut s, &books(account, vec![book("team", "Team", false)])).unwrap();
    let rev = s.rev();
    forget(&mut s, account).unwrap();
    assert!(listed(&s, account).is_empty() && s.rev() > rev);
    let rev = s.rev();
    forget(&mut s, account).unwrap();
    assert_eq!(s.rev(), rev);
}

#[test]
fn sync_apply_books_is_never_for_an_agent() {
    assert_eq!(agent::access("sync.applyBooks"), Some(Access::Never));
    let (s, account) = session();
    let params = json!({"download": books(account, vec![book("team", "Team", false)])});
    assert_eq!(agent::classify(&s, "sync.applyBooks", &params), Access::Never);
    // Listed for agents as such.
    let listed = s.registry.describe();
    let entry = listed.as_array().unwrap().iter().find(|c| c["id"] == "sync.applyBooks").unwrap();
    assert_eq!((entry["agent"].as_str(), entry["mutates"].as_bool()), (Some("never"), Some(false)));
}

#[test]
fn a_mailbox_with_books_round_trips_through_the_file() {
    let (mut s, account) = session();
    the_two_books(&mut s, account);
    let dir = std::env::temp_dir().join(format!("mailcraft-books-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mailbox.mailcraft");
    crate::store::save(&path, &s.mailbox).unwrap();
    let opened = crate::store::open(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let read: &Mailbox = &opened.mailbox;
    assert_eq!(read.address_books, s.mailbox.address_books);
    let cy = read.contacts.values().find(|c| c.origin.id == "c-cy").unwrap();
    assert_eq!(read.book_of(cy).map(|b| (b.origin.id.as_str(), b.read_only)), Some(("grace", true)));
}
