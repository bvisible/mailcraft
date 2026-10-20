// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What is made, changed and deleted here in a server's address book, noted for the server and settled by what it
//! says. The server's side is written out by hand, as a host brings it (`mailcraft-jmap`'s writing of JMAP for
//! Contacts). Invented people at `example.org`.

use mailcraft_model::book::{
    BooksDownload, ContactChange, ContactChangeKind, ContactFields, ContactName, ContactOutcome, ContactsUploaded, RemoteBook,
};
use mailcraft_model::remote::{ContactsDownload, EventResult, RemoteContact};
use mailcraft_model::{Contact, Id, Labeled};
use mailcraft_time::{Date, DateTime, Timestamp};
use proptest::prelude::*;
use serde_json::{Value, json};

use super::super::{NO_ADDING, NO_CHANGING, NO_DELETING, NO_KEEPING, NO_MOVING, apply_books};
use crate::agent::{self, Access, Class};
use crate::cmd::remote::apply_contacts;
use crate::{CmdError, Session};

const ME: &str = "ada@example.org";

fn at(year: i32, month: u8, day: u8, hour: u8) -> Timestamp {
    DateTime::new(Date::new(year, month, day).unwrap(), hour, 0, 0).unwrap().to_timestamp(0)
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(s: &mut Session, id: &str, params: Value) -> String {
    match s.run(id, &params) {
        Err(CmdError::Failed(why) | CmdError::Disabled(why)) => why,
        other => panic!("{id} {params}: expected a refusal, got {other:?}"),
    }
}

fn book(id: &str, name: &str, read_only: bool) -> RemoteBook {
    RemoteBook { id: id.into(), name: name.into(), read_only, may_remove: !read_only, plain: true, ..Default::default() }
}

fn card(id: &str, first: &str, email: &str, book: &str) -> RemoteContact {
    RemoteContact { id: id.into(), book: book.into(), first: first.into(), emails: vec![Labeled::new("", email)], ..Default::default() }
}

fn cards(account: Id, contacts: Vec<RemoteContact>) -> ContactsDownload {
    ContactsDownload { account, email: ME.into(), complete: true, contacts }
}

/// The sample mailbox on Wednesday 2026-10-14 with a JMAP account whose server has the team's book (Ada may change
/// its contacts, the main one) with Bob in it, and Grace's (she may only read) with Cy; that account's id.
fn session() -> (Session, Id) {
    let now = at(2026, 10, 14, 16);
    let mut s = Session::at(crate::sample::mailbox(now, 0), now, 0);
    let added = s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap/session"}})).unwrap();
    let account = added["id"].as_u64().unwrap();
    let team = RemoteBook { main: true, ..book("team", "Team", false) };
    apply_books(&mut s, &BooksDownload { account, email: ME.into(), complete: true, books: vec![team, book("grace", "Grace's", true)] }).unwrap();
    apply_contacts(&mut s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace")]))
        .unwrap();
    (s, account)
}

/// The contact the server calls `remote`.
fn kept(s: &Session, remote: &str) -> Contact {
    let found = s.mailbox.contacts.values().find(|c| c.origin.id == remote);
    found.map(|c| c.as_ref().clone()).unwrap_or_else(|| panic!("no contact {remote}"))
}

fn named(s: &Session, first: &str) -> Option<Contact> {
    s.mailbox.contacts.values().find(|c| c.first == first).map(|c| c.as_ref().clone())
}

/// What `sync.contactChanges` says is to go now.
fn upload(s: &mut Session, account: Id) -> Value {
    run(s, "sync.contactChanges", json!({"account": account}))
}

fn ops(s: &mut Session, account: Id) -> Vec<Value> {
    upload(s, account)["ops"].as_array().cloned().unwrap_or_default()
}

/// The server's answer `result` to every op that is to go now.
fn answer(s: &mut Session, account: Id, result: EventResult, remote: &str, error: &str) -> Value {
    let upload = upload(s, account);
    answer_to(s, account, &upload, result, remote, error)
}

fn answer_to(s: &mut Session, account: Id, upload: &Value, result: EventResult, remote: &str, error: &str) -> Value {
    let outcomes: Vec<ContactOutcome> = upload["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| ContactOutcome {
            op: serde_json::from_value(op.clone()).unwrap(),
            result,
            remote: remote.into(),
            error: error.into(),
            ..Default::default()
        })
        .collect();
    let uploaded = ContactsUploaded { account, email: ME.into(), outcomes, token_refused: false };
    run(s, "sync.applyContactChanges", json!({"result": uploaded}))
}

fn kinds(s: &Session) -> Vec<ContactChangeKind> {
    s.mailbox.contact_changes.iter().map(|change| change.kind).collect()
}

#[test]
fn a_contact_made_in_a_writable_book_is_noted_and_kept_until_the_server_says() {
    let (mut s, account) = session();
    let team = s.mailbox.address_book_on_server(account, "team").map(|b| b.id).unwrap();
    // Without a book named, the main one Ada may change.
    let id = run(&mut s, "people.new", json!({"first": "Gus", "last": "Hale", "email": "gus@example.org", "company": "Hale & Sons"}))["id"]
        .as_u64()
        .unwrap();
    let gus = s.mailbox.contact(id).cloned().unwrap();
    assert_eq!((gus.book, gus.origin.account, gus.origin.id.as_str()), (team, account, ""));
    let change = s.mailbox.contact_changes.first().cloned().unwrap();
    assert_eq!((change.kind, change.contact, change.token.len()), (ContactChangeKind::Create, id, 36));
    assert_eq!(gus.uid, change.token, "its card's mark, which an export keeps too");
    // What goes: the card whole, into the team's book.
    let ops = ops(&mut s, account);
    assert_eq!(ops.len(), 1);
    assert_eq!(
        (ops[0]["kind"].as_str(), ops[0]["book"].as_str(), ops[0]["token"].as_str()),
        (Some("create"), Some("team"), Some(change.token.as_str()))
    );
    let fields: ContactFields = serde_json::from_value(ops[0]["fields"].clone()).unwrap();
    assert_eq!(fields.name, Some(ContactName { first: "Gus".into(), last: "Hale".into(), display: String::new() }));
    assert_eq!((fields.company.as_deref(), fields.title.as_deref()), (Some("Hale & Sons"), Some("")));
    assert_eq!(fields.emails, Some(vec![Labeled::new("work", "gus@example.org")]));
    // A download before the server answered leaves it as it is.
    apply_contacts(&mut s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace")]))
        .unwrap();
    assert_eq!(s.mailbox.contact(id), Some(&gus));
    assert_eq!(kinds(&s), [ContactChangeKind::Create]);
    // Done: it is the server's card from now on, and the history before is not what the server has any more.
    assert!(s.can_undo());
    let said = answer(&mut s, account, EventResult::Done, "c-gus", "");
    assert_eq!((said["done"].as_u64(), said["waiting"].as_u64()), (Some(1), Some(0)));
    assert_eq!(s.mailbox.contact(id).map(|c| c.origin.id.as_str()), Some("c-gus"));
    assert!(s.mailbox.contact_changes.is_empty() && !s.can_undo());
    // A book named: Grace's may only be read, the mailbox's own (0) is kept here, one that isn't there is said so.
    let grace = s.mailbox.address_book_on_server(account, "grace").map(|b| b.id).unwrap();
    assert_eq!(refused(&mut s, "people.new", json!({"first": "Hal", "book": grace})), NO_ADDING);
    let own = run(&mut s, "people.new", json!({"first": "Hal", "book": 0}))["id"].as_u64().unwrap();
    assert!(s.mailbox.contact(own).is_some_and(|c| c.book == 0 && c.origin.is_local()));
    assert!(refused(&mut s, "people.new", json!({"first": "Hal", "book": 999_999})).contains("no address book"));
    assert!(matches!(s.run("people.new", &json!({"first": "Hal", "book": "team"})), Err(CmdError::Params(_))));
    assert!(s.mailbox.contact_changes.is_empty(), "nothing of those goes to the server");
    // Add to Contacts goes the same way.
    let message =
        s.mailbox.messages.values().find(|m| m.from.is_valid() && s.mailbox.contact_by_email(&m.from.email).is_none()).map(|m| m.id).unwrap();
    let added = run(&mut s, "people.fromMessage", json!({"id": message}));
    let sender = s.mailbox.contact(added["id"].as_u64().unwrap()).cloned().unwrap();
    assert_eq!((sender.book, kinds(&s)), (team, vec![ContactChangeKind::Create]));
}

#[test]
fn a_contact_changed_notes_only_what_changed_against_its_base() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    let change = s.mailbox.contact_changes.first().cloned().unwrap();
    assert_eq!((change.kind, change.remote.as_str(), change.base.as_deref()), (ContactChangeKind::Update, "c-bob", Some(&bob)));
    let fields: ContactFields = serde_json::from_value(ops(&mut s, account)[0]["fields"].clone()).unwrap();
    assert_eq!(fields, ContactFields { title: Some("Joiner".into()), ..Default::default() });
    // A name goes whole, with its parts.
    run(&mut s, "people.edit", json!({"id": bob.id, "last": "Lindqvist"}));
    let fields: ContactFields = serde_json::from_value(ops(&mut s, account)[0]["fields"].clone()).unwrap();
    assert_eq!(fields.name, Some(ContactName { first: "Bob".into(), last: "Lindqvist".into(), display: String::new() }));
    assert_eq!(fields.title.as_deref(), Some("Joiner"));
    assert!(fields.emails.is_none() && fields.phones.is_none() && fields.company.is_none());
    // What is kept here only goes nowhere.
    run(&mut s, "people.favorite", json!({"id": bob.id, "value": true}));
    run(&mut s, "people.categorize", json!({"id": bob.id, "category": "Red category"}));
    assert_eq!(s.mailbox.contact_changes.len(), 1);
}

#[test]
fn two_changes_to_one_contact_are_one_note() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    run(&mut s, "people.edit", json!({"id": bob.id, "phones": [{"label": "mobile", "value": "+41 79 555 01 01"}]}));
    assert_eq!(kinds(&s), [ContactChangeKind::Update]);
    let fields: ContactFields = serde_json::from_value(ops(&mut s, account)[0]["fields"].clone()).unwrap();
    assert_eq!((fields.title.as_deref(), fields.phones), (Some("Joiner"), Some(vec![Labeled::new("mobile", "+41 79 555 01 01")])));
    // Taken back, there is nothing to tell.
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "", "phones": []}));
    assert!(s.mailbox.contact_changes.is_empty());
    assert!(ops(&mut s, account).is_empty());
}

#[test]
fn a_contact_made_and_deleted_before_it_went_is_never_sent() {
    let (mut s, account) = session();
    let id = run(&mut s, "people.new", json!({"first": "Gus", "email": "gus@example.org"}))["id"].as_u64().unwrap();
    run(&mut s, "people.edit", json!({"id": id, "title": "Joiner"}));
    assert_eq!(kinds(&s), [ContactChangeKind::Create], "a change before it went goes with it");
    run(&mut s, "people.delete", json!({"id": id}));
    assert!(s.mailbox.contact_changes.is_empty() && ops(&mut s, account).is_empty());
}

#[test]
fn a_deleted_contact_stays_deleted_through_a_download_until_the_server_was_told() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    run(&mut s, "people.delete", json!({"id": bob.id}));
    let change = s.mailbox.contact_changes.first().cloned().unwrap();
    assert_eq!((change.kind, change.remote.as_str()), (ContactChangeKind::Delete, "c-bob"));
    assert_eq!(change.base.as_deref(), Some(&bob), "what the server has is still what it was before the change that waited");
    let download = cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace")]);
    apply_contacts(&mut s, &download).unwrap();
    assert!(named(&s, "Bob").is_none(), "not brought back while its deletion waits");
    let ops = ops(&mut s, account);
    assert_eq!((ops.len(), ops[0]["kind"].as_str(), ops[0]["remote"].as_str()), (1, Some("delete"), Some("c-bob")));
    answer(&mut s, account, EventResult::Done, "", "");
    assert!(s.mailbox.contact_changes.is_empty());
    // Told: what comes down from then on is what the server has.
    apply_contacts(&mut s, &cards(account, vec![card("c-cy", "Cy", "cy@example.org", "grace")])).unwrap();
    assert!(named(&s, "Bob").is_none());
}

#[test]
fn a_contact_that_waits_is_kept_through_a_download_that_changes_it() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    let here = s.mailbox.contact(bob.id).cloned().unwrap();
    // Somebody else changed him meanwhile; and a whole download without him: he stays as he is here, until the server
    // answered the change.
    let changed = RemoteContact { company: "Elsewhere".into(), ..card("c-bob", "Bob", "bob@example.org", "team") };
    let merged = apply_contacts(&mut s, &cards(account, vec![changed, card("c-cy", "Cy", "cy@example.org", "grace")])).unwrap();
    assert_eq!((s.mailbox.contact(bob.id), merged["updated"].as_u64()), (Some(&here), Some(0)));
    apply_contacts(&mut s, &cards(account, vec![card("c-cy", "Cy", "cy@example.org", "grace")])).unwrap();
    assert_eq!(s.mailbox.contact(bob.id), Some(&here));
    // Answered: the next download is what the server has.
    answer(&mut s, account, EventResult::Done, "", "");
    let after = RemoteContact { title: "Joiner".into(), company: "Elsewhere".into(), ..card("c-bob", "Bob", "bob@example.org", "team") };
    apply_contacts(&mut s, &cards(account, vec![after, card("c-cy", "Cy", "cy@example.org", "grace")])).unwrap();
    assert_eq!(s.mailbox.contact(bob.id).map(|c| (c.title.as_str(), c.company.as_str())), Some(("Joiner", "Elsewhere")));
}

#[test]
fn a_contact_made_here_is_found_again_by_its_token_in_the_next_download() {
    let (mut s, account) = session();
    let id = run(&mut s, "people.new", json!({"first": "Gus", "email": "gus@example.org"}))["id"].as_u64().unwrap();
    let token = s.mailbox.contact_changes[0].token.clone();
    // The answer was lost; the next download has the card under that mark.
    let gus = RemoteContact {
        uid: token.clone(),
        emails: vec![Labeled::new("work", "gus@example.org")],
        ..card("c-gus", "Gus", "gus@example.org", "team")
    };
    let all = |gus: RemoteContact| {
        cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace"), gus])
    };
    let rev = s.rev();
    apply_contacts(&mut s, &all(gus.clone())).unwrap();
    let found: Vec<&Contact> = s.mailbox.contacts.values().filter(|c| c.first == "Gus").map(|c| c.as_ref()).collect();
    assert_eq!(found.iter().map(|c| (c.id, c.origin.id.as_str())).collect::<Vec<_>>(), [(id, "c-gus")], "the same contact, and no second one");
    assert!(s.mailbox.contact_changes.is_empty() && s.rev() > rev);
    // Changed here before the card came down: what differs is noted as a change of the card.
    let id = run(&mut s, "people.new", json!({"first": "Hal", "email": "hal@example.org"}))["id"].as_u64().unwrap();
    let token = s.mailbox.contact_changes[0].token.clone();
    run(&mut s, "people.edit", json!({"id": id, "title": "Joiner"}));
    let hal = RemoteContact { uid: token, emails: vec![Labeled::new("work", "hal@example.org")], ..card("c-hal", "Hal", "hal@example.org", "team") };
    let mut download = all(gus);
    download.contacts.push(hal);
    apply_contacts(&mut s, &download).unwrap();
    assert_eq!(s.mailbox.contact(id).map(|c| (c.origin.id.as_str(), c.title.as_str())), Some(("c-hal", "Joiner")));
    let fields: ContactFields = serde_json::from_value(ops(&mut s, account)[0]["fields"].clone()).unwrap();
    assert_eq!(fields, ContactFields { title: Some("Joiner".into()), ..Default::default() });
    answer(&mut s, account, EventResult::Done, "", "");
    // A server that keeps no mark of a card (its uid is its name): the download brings the card as a contact of its
    // own, and the answer that comes after says it is the one made here: one is left, the one made here.
    let id = run(&mut s, "people.new", json!({"first": "Ivy", "email": "ivy@example.org"}))["id"].as_u64().unwrap();
    let mut download = all(card("c-gus", "Gus", "gus@example.org", "team"));
    download.contacts.push(card("c-hal", "Hal", "hal@example.org", "team"));
    download.contacts.push(card("c-ivy", "Ivy", "ivy@example.org", "team"));
    apply_contacts(&mut s, &download).unwrap();
    assert_eq!(s.mailbox.contacts.values().filter(|c| c.first == "Ivy").count(), 2);
    answer(&mut s, account, EventResult::Done, "c-ivy", "");
    let ivy: Vec<(Id, &str)> = s.mailbox.contacts.values().filter(|c| c.first == "Ivy").map(|c| (c.id, c.origin.id.as_str())).collect();
    assert_eq!(ivy, [(id, "c-ivy")]);
}

#[test]
fn done_records_the_servers_id_and_takes_the_note() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    let sent = upload(&mut s, account);
    // Changed again while the change was on its way: only that is still to go.
    run(&mut s, "people.edit", json!({"id": bob.id, "company": "Lindqvist Joinery"}));
    answer_to(&mut s, account, &sent, EventResult::Done, "", "");
    let fields: ContactFields = serde_json::from_value(ops(&mut s, account)[0]["fields"].clone()).unwrap();
    assert_eq!(fields, ContactFields { company: Some("Lindqvist Joinery".into()), ..Default::default() });
    answer(&mut s, account, EventResult::Done, "", "");
    assert!(s.mailbox.contact_changes.is_empty());
    // A new one made, and the server's answer says what it calls it; one that doesn't say waits and goes again with
    // its mark.
    let id = run(&mut s, "people.new", json!({"first": "Gus", "email": "gus@example.org"}))["id"].as_u64().unwrap();
    let said = answer(&mut s, account, EventResult::Done, "", "");
    assert_eq!((said["failed"].as_u64(), kinds(&s)), (Some(1), vec![ContactChangeKind::Create]));
    s.clock.now = s.clock.now.map(|now| now + 60);
    answer(&mut s, account, EventResult::Done, "c-gus", "");
    assert_eq!(s.mailbox.contact(id).map(|c| c.origin.id.as_str()), Some("c-gus"));
    assert!(s.mailbox.contact_changes.is_empty());
}

#[test]
fn refused_puts_a_changed_contact_back_to_its_base_and_says_so_once() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    run(&mut s, "people.favorite", json!({"id": bob.id, "value": true}));
    let sent = upload(&mut s, account);
    let said = answer_to(&mut s, account, &sent, EventResult::Refused, "", "Your rights do not allow it.");
    assert_eq!(
        said["messages"],
        json!(["The server wouldn't take the change to \"Bob\" (Your rights do not allow it.). It was put back as it was."])
    );
    let back = s.mailbox.contact(bob.id).cloned().unwrap();
    assert_eq!((back.title.as_str(), back.favorite), ("", true), "as the server has it, and what is kept here only as it is");
    assert!(s.mailbox.contact_changes.is_empty());
    // Said once: the same answer again has nothing to say.
    let again = answer_to(&mut s, account, &sent, EventResult::Refused, "", "Your rights do not allow it.");
    assert_eq!(again["messages"], json!([]));
    // A deletion refused (a document names him): he is back.
    run(&mut s, "people.delete", json!({"id": bob.id}));
    let said = answer(&mut s, account, EventResult::Refused, "", "Cannot delete or cancel because Contact Bob is linked with Issue ISS-0001");
    assert_eq!(
        said["messages"][0],
        "The server wouldn't delete \"Bob\" (Cannot delete or cancel because Contact Bob is linked with Issue ISS-0001). It is back in the contacts."
    );
    assert_eq!(s.mailbox.contact(bob.id).map(|c| c.origin.id.as_str()), Some("c-bob"));
}

#[test]
fn refused_removes_a_contact_the_server_never_had_and_says_so() {
    let (mut s, account) = session();
    s.keeps_local = false;
    let id = run(&mut s, "people.new", json!({"company": "Hale & Sons"}))["id"].as_u64().unwrap();
    let said = answer(&mut s, account, EventResult::Refused, "", "A contact needs a name or an address.");
    assert_eq!(said["messages"], json!(["The server wouldn't take \"Hale & Sons\" (A contact needs a name or an address.). It was removed here."]));
    assert!(s.mailbox.contact(id).is_none() && s.mailbox.contact_changes.is_empty());
    // Where what is made here is kept, it stays, as the mailbox's own.
    s.keeps_local = true;
    let id = run(&mut s, "people.new", json!({"company": "Hale & Sons"}))["id"].as_u64().unwrap();
    let said = answer(&mut s, account, EventResult::Refused, "", "");
    assert_eq!(said["messages"], json!(["The server wouldn't take \"Hale & Sons\". It is kept on this computer only."]));
    assert!(s.mailbox.contact(id).is_some_and(|c| c.book == 0 && c.origin.is_local()));
}

#[test]
fn gone_removes_the_contact_here() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    let said = answer(&mut s, account, EventResult::Gone, "", "");
    assert_eq!(said["messages"], json!(["\"Bob\" is no longer on the server, so it couldn't be changed there. It was removed here too."]));
    assert_eq!(said["dropped"]["count"], 1);
    assert!(s.mailbox.contact(bob.id).is_none() && s.mailbox.contact_changes.is_empty());
    // A deletion of one that is gone there already is done.
    let cy = kept(&s, "c-cy");
    apply_books(
        &mut s,
        &BooksDownload { account, email: ME.into(), complete: true, books: vec![book("team", "Team", false), book("grace", "Grace's", false)] },
    )
    .unwrap();
    run(&mut s, "people.delete", json!({"id": cy.id}));
    let said = answer(&mut s, account, EventResult::Gone, "", "");
    assert_eq!((said["done"].as_u64(), said["messages"].clone()), (Some(1), json!([])));
    assert!(s.mailbox.contact_changes.is_empty());
}

#[test]
fn failed_waits_with_the_back_off_and_is_sent_again() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    let now = s.now();
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    let said = answer(&mut s, account, EventResult::Failed, "", "the server failed");
    assert_eq!((said["failed"].as_u64(), said["waiting"].as_u64()), (Some(1), Some(1)));
    let change = s.mailbox.contact_changes[0].clone();
    assert_eq!((change.failures, change.retry_at, change.error.as_str()), (1, now + 30, "the server failed"));
    assert!(ops(&mut s, account).is_empty(), "held back for half a minute");
    assert_eq!(super::contacts_waiting(&s, account).next, Some(now + 30));
    // Twice as long after each failure more.
    s.clock.now = Some(now + 30);
    answer(&mut s, account, EventResult::Failed, "", "");
    assert_eq!(s.mailbox.contact_changes[0].retry_at, now + 30 + 60);
    s.clock.now = Some(now + 90);
    assert_eq!(ops(&mut s, account).len(), 1, "due again");
    // A change made meanwhile goes at once.
    s.clock.now = Some(now + 100);
    answer(&mut s, account, EventResult::Failed, "", "");
    run(&mut s, "people.edit", json!({"id": bob.id, "company": "Lindqvist Joinery"}));
    assert_eq!(ops(&mut s, account).len(), 1);
    // Denied once waits; denied again gives up.
    let said = answer(&mut s, account, EventResult::Denied, "", "");
    assert!(said["denied"].as_bool() == Some(true) && s.mailbox.contact_changes[0].denied);
    s.clock.now = Some(now + 10_000);
    answer(&mut s, account, EventResult::Denied, "", "");
    assert!(s.mailbox.contact_changes.is_empty());
    assert_eq!(s.mailbox.contact(bob.id).map(|c| c.title.as_str()), Some(""), "put back");
}

#[test]
fn a_note_that_no_longer_fits_the_mailbox_is_given_up_with_a_word() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    // From a file from elsewhere: a change of a contact that isn't here, one of an account that isn't, one that names
    // nothing, the deletion of one that is still here, and a change that says nothing (Bob is as the server has him).
    let base = |c: &Contact| Some(Box::new(c.clone()));
    let gone = Contact { id: 999_999, first: "Zed".into(), ..bob.clone() };
    s.mailbox.contact_changes = vec![
        ContactChange { account, kind: ContactChangeKind::Update, contact: 999_999, remote: "c-zed".into(), base: base(&gone), ..Default::default() },
        ContactChange { account: 424_242, kind: ContactChangeKind::Delete, remote: "c-x".into(), ..Default::default() },
        ContactChange { account, kind: ContactChangeKind::Create, contact: bob.id, ..Default::default() },
        ContactChange { account, kind: ContactChangeKind::Delete, contact: bob.id, remote: "c-bob".into(), base: base(&bob), ..Default::default() },
        ContactChange { account, kind: ContactChangeKind::Update, contact: bob.id, remote: "c-bob".into(), base: base(&bob), ..Default::default() },
    ];
    let answer = upload(&mut s, account);
    assert_eq!(answer["dropped"]["count"], 4, "the one that says nothing just goes");
    let messages = answer["dropped"]["messages"].as_array().unwrap();
    assert!(messages.iter().any(|m| m == "A change to \"Zed\" was not sent: it is no longer in the contacts."), "{messages:?}");
    assert!(messages.iter().any(|m| m == "A contact change that was waiting to be sent no longer matched the contacts. It was not sent."));
    assert!(answer["ops"].as_array().unwrap().is_empty() && s.mailbox.contact_changes.is_empty());
    assert!(s.mailbox.contact(bob.id).is_some(), "the deletion of one that is still here is not sent");
}

#[test]
fn a_contact_of_a_book_that_became_read_only_is_undone_with_a_word() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    let gus = run(&mut s, "people.new", json!({"first": "Gus", "email": "gus@example.org"}))["id"].as_u64().unwrap();
    // Ada's rights in the team's book were narrowed on the server.
    apply_books(
        &mut s,
        &BooksDownload { account, email: ME.into(), complete: true, books: vec![RemoteBook { main: true, ..book("team", "Team", true) }] },
    )
    .unwrap();
    let dropped = super::settle_contact_changes(&mut s);
    let messages: Vec<&str> = dropped.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "\"Bob\": its address book can't be changed any more, so the change made here was undone.",
            "\"Gus\" was not sent to the server: its address book can't be changed any more. It is kept on this computer only.",
        ]
    );
    assert_eq!(s.mailbox.contact(bob.id).map(|c| c.title.as_str()), Some(""));
    assert!(s.mailbox.contact(gus).is_some_and(|c| c.book == 0 && c.origin.is_local()));
    assert!(s.mailbox.contact_changes.is_empty());
    // And the commands say why.
    assert_eq!(refused(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"})), NO_CHANGING);
}

#[test]
fn a_card_cannot_be_deleted_where_the_book_forbids_it() {
    let (mut s, account) = session();
    let changes_only = RemoteBook { main: true, may_remove: false, ..book("team", "Team", false) };
    apply_books(&mut s, &BooksDownload { account, email: ME.into(), complete: true, books: vec![changes_only, book("grace", "Grace's", true)] })
        .unwrap();
    let bob = kept(&s, "c-bob");
    assert_eq!(refused(&mut s, "people.delete", json!({"id": bob.id})), NO_DELETING);
    // Changed, though: that goes.
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    assert_eq!(kinds(&s), [ContactChangeKind::Update]);
    // A deletion noted before the book said so is not sent; he is not made again here (its answer may have been lost
    // after the server did it): the next read brings him back, if the server still has him.
    let base = Some(Box::new(s.mailbox.contact(bob.id).cloned().unwrap()));
    s.mailbox.remove_contact(bob.id).unwrap();
    s.mailbox.contact_changes =
        vec![ContactChange { account, kind: ContactChangeKind::Delete, contact: bob.id, remote: "c-bob".into(), base, ..Default::default() }];
    let answer = upload(&mut s, account);
    assert_eq!(
        answer["dropped"]["messages"],
        json!([
            "\"Bob\": its address book can't be changed any more, so it was not deleted there. It comes back with the next read if the server still has it."
        ])
    );
    assert!(s.mailbox.contact(bob.id).is_none() && answer["ops"].as_array().unwrap().is_empty());
    apply_contacts(&mut s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace")]))
        .unwrap();
    assert!(named(&s, "Bob").is_some(), "back with the read");
}

#[test]
fn what_a_plain_book_does_not_keep_is_refused_with_the_reason() {
    let (mut s, _) = session();
    let bob = kept(&s, "c-bob");
    for (key, value) in [
        ("notes", json!("Met at the fair")),
        ("birthday", json!("1980-02-02")),
        ("website", json!("https://bob.example.org")),
        ("department", json!("Joinery")),
        ("displayName", json!("Bobby")),
        ("address", json!({"street": "Rue du Lac 4", "city": "Lausanne"})),
    ] {
        assert_eq!(refused(&mut s, "people.edit", json!({"id": bob.id, key: value.clone()})), NO_KEEPING, "{key}");
        assert_eq!(refused(&mut s, "people.new", json!({"first": "Gus", key: value})), NO_KEEPING, "{key}");
    }
    assert!(s.mailbox.contact_changes.is_empty());
    // Nothing is moved to another book by a change.
    assert_eq!(refused(&mut s, "people.edit", json!({"id": bob.id, "book": 0})), NO_MOVING);
    // The mailbox's own contacts keep everything, as ever.
    let own = run(&mut s, "people.new", json!({"first": "Gus", "notes": "Met at the fair", "book": 0}))["id"].as_u64().unwrap();
    run(&mut s, "people.edit", json!({"id": own, "birthday": "1980-02-02", "displayName": "Gussie"}));
    assert!(s.mailbox.contact_changes.is_empty());
}

#[test]
fn undoing_a_change_that_went_notes_the_opposite() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    // A change on its way, undone here meanwhile: the server has it, so its opposite goes.
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    let sent = upload(&mut s, account);
    assert!(s.undo());
    assert!(s.mailbox.contact_changes.is_empty() && s.mailbox.contact(bob.id).is_some_and(|c| c.title.is_empty()));
    answer_to(&mut s, account, &sent, EventResult::Done, "", "");
    let fields: ContactFields = serde_json::from_value(ops(&mut s, account)[0]["fields"].clone()).unwrap();
    assert_eq!(fields, ContactFields { title: Some(String::new()), ..Default::default() });
    answer(&mut s, account, EventResult::Done, "", "");
    // A new one undone: the server made it, so it deletes it.
    run(&mut s, "people.new", json!({"first": "Gus", "email": "gus@example.org"}));
    let sent = upload(&mut s, account);
    assert!(s.undo());
    answer_to(&mut s, account, &sent, EventResult::Done, "c-gus", "");
    let ops = ops(&mut s, account);
    assert_eq!((ops[0]["kind"].as_str(), ops[0]["remote"].as_str()), (Some("delete"), Some("c-gus")));
    answer(&mut s, account, EventResult::Done, "", "");
    // A deletion undone: the server deleted it, so it is gone here too, with a word; nothing is made anew there (a bare
    // new card is not the one that was deleted).
    run(&mut s, "people.delete", json!({"id": bob.id}));
    let sent = upload(&mut s, account);
    assert!(s.undo());
    let said = answer_to(&mut s, account, &sent, EventResult::Done, "", "");
    assert!(s.mailbox.contact(bob.id).is_none() && s.mailbox.contact_changes.is_empty());
    assert_eq!(
        said["messages"],
        json!([
            "\"Bob\" was already deleted on the server when the deletion was undone here, so it is gone here too. The server may keep a copy of what was deleted."
        ])
    );
}

#[test]
fn the_contact_commands_are_never_for_an_agent_and_a_change_to_a_server_book_is_send_and_its_deletion_delete() {
    for id in ["sync.contactChanges", "sync.applyContactChanges"] {
        assert_eq!(agent::access(id), Some(Access::Never), "{id}");
    }
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    let own = run(&mut s, "people.new", json!({"first": "Fay", "book": 0}))["id"].as_u64().unwrap();
    let classify = |s: &Session, id: &str, params: Value| agent::classify(s, id, &params);
    assert_eq!(classify(&s, "sync.contactChanges", json!({"account": account})), Access::Never);
    assert_eq!(classify(&s, "people.new", json!({"first": "Gus"})), Access::Owner(Class::Send), "it goes to the team's book");
    assert_eq!(classify(&s, "people.new", json!({"first": "Gus", "book": 0})), Access::Safe);
    assert_eq!(classify(&s, "people.edit", json!({"id": bob.id, "title": "Joiner"})), Access::Owner(Class::Send));
    assert_eq!(classify(&s, "people.edit", json!({"id": own, "title": "Joiner"})), Access::Safe);
    assert_eq!(classify(&s, "people.delete", json!({"id": bob.id})), Access::Owner(Class::Delete));
    assert_eq!(classify(&s, "people.delete", json!({"id": own})), Access::Safe);
    // A selected contact counts as a named one.
    s.sel.contact = Some(bob.id);
    assert_eq!(classify(&s, "people.delete", json!({})), Access::Owner(Class::Delete));
    // Whatever path an agent took there, what it wasn't allowed is undone.
    s.agent = Some(vec![Class::Delete]);
    let why = refused(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    assert!(why.contains("agent"), "{why}");
    s.agent = Some(vec![Class::Send]);
    let why = refused(&mut s, "people.delete", json!({"id": bob.id}));
    assert!(why.contains("agent"), "{why}");
    assert!(s.mailbox.contact_changes.is_empty() && s.mailbox.contact(bob.id).is_some());
    s.agent = Some(vec![Class::Send, Class::Delete]);
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    // Listed for agents as such.
    let listed = s.registry.describe();
    for id in ["sync.contactChanges", "sync.applyContactChanges"] {
        let entry = listed.as_array().unwrap().iter().find(|c| c["id"] == id).unwrap();
        assert_eq!((entry["agent"].as_str(), entry["mutates"].as_bool()), (Some("never"), Some(false)), "{id}");
    }
}

#[test]
fn forget_takes_the_waiting_changes_of_the_account_along() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    run(&mut s, "sync.forgetRemote", json!({"account": account}));
    assert!(s.mailbox.contact_changes.is_empty() && s.mailbox.address_books.is_empty());
    // A file whose changes name an account it doesn't have is put right when it is opened.
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob").id;
    run(&mut s, "people.edit", json!({"id": bob, "title": "Joiner"}));
    let stray = ContactChange { account: 424_242, ..s.mailbox.contact_changes[0].clone() };
    s.mailbox.contact_changes.push(stray);
    let fixed = s.mailbox.repair();
    assert!(fixed.iter().any(|line| line.contains("change(s) of contacts")), "{fixed:?}");
    assert_eq!(s.mailbox.contact_changes.iter().map(|c| c.account).collect::<Vec<_>>(), [account]);
}

proptest! {
    #[test]
    fn the_contact_commands_take_anything_and_never_panic(text in any::<String>(), n in any::<u64>(), m in any::<i64>(), result in 0usize..5) {
        let (mut s, account) = session();
        let bob = kept(&s, "c-bob");
        let _ = s.run("people.edit", &json!({"id": bob.id, "title": text.clone(), "book": n}));
        let _ = s.run("people.new", &json!({"first": text.clone(), "book": n, "emails": [{"label": text.clone(), "value": text.clone()}]}));
        let _ = s.run("people.new", &json!({"first": text.clone(), "phones": [{"label": text.clone(), "value": text.clone()}]}));
        let _ = s.run("people.edit", &json!({"id": bob.id, "last": text.clone(), "phones": [text.clone()]}));
        let _ = s.run("sync.contactChanges", &json!({"account": n}));
        let upload = s.run("sync.contactChanges", &json!({"account": account})).unwrap();
        let results = [EventResult::Done, EventResult::Gone, EventResult::Refused, EventResult::Denied, EventResult::Failed];
        let mut ops: Vec<Value> = upload["ops"].as_array().cloned().unwrap_or_default();
        ops.push(json!({"kind": "update", "contact": n, "remote": text.clone(), "token": text.clone(), "fields": {"name": {"first": text.clone()}, "emails": [{"value": text.clone()}]}}));
        ops.push(json!({"kind": "delete", "remote": "c-cy"}));
        let outcomes: Vec<Value> = ops
            .into_iter()
            .map(|op| json!({"op": op, "result": results[result], "remote": text.clone(), "error": text.clone(), "retryAfter": n % 100_000}))
            .collect();
        let _ = s.run("sync.applyContactChanges", &json!({"result": {"account": account, "email": ME, "outcomes": outcomes}}));
        let _ = s.run("sync.applyContactChanges", &json!({"result": {"account": account, "outcomes": [{"op": {"kind": text.clone()}}]}}));
        let _ = s.run("sync.applyContactChanges", &json!({"result": text.clone()}));
        s.clock.now = Some(m.clamp(0, 4_000_000_000));
        let _ = s.run("sync.contactChanges", &json!({"account": account}));
        let _ = s.undo();
        let _ = s.run("sync.contactChanges", &json!({"account": account}));
        prop_assert!(s.mailbox.contact_changes.len() <= mailcraft_model::book::MAX_CONTACT_CHANGES);
        let json = serde_json::to_string(&s.mailbox).unwrap();
        prop_assert_eq!(serde_json::from_str::<mailcraft_model::Mailbox>(&json).unwrap(), s.mailbox.clone());
    }
}

/// Gus made here, his first answer lost on the way, and the read that came before the next try, which brought his card
/// as a contact of its own (a server that keeps no mark of a card: its uid is its name). Gus here, the copy, and
/// what the next try sends.
fn lost_then_copied(s: &mut Session, account: Id) -> (Id, Id) {
    let id = run(s, "people.new", json!({"first": "Gus", "email": "gus@example.org"}))["id"].as_u64().unwrap();
    let first = upload(s, account);
    answer_to(s, account, &first, EventResult::Failed, "", "the connection was lost");
    let gus = RemoteContact { emails: vec![Labeled::new("work", "gus@example.org")], ..card("c-gus", "Gus", "gus@example.org", "team") };
    apply_contacts(s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace"), gus]))
        .unwrap();
    let copy = s.mailbox.contacts.values().find(|c| c.first == "Gus" && c.id != id).map(|c| c.id).expect("the copy the read brought");
    // The next try is due a minute on.
    s.clock.now = s.clock.now.map(|now| now + 120);
    (id, copy)
}

/// The outcomes of `upload` whose op is of `kind`, all `result`, the made ones called `remote`.
fn answer_only(s: &mut Session, account: Id, upload: &Value, kind: &str, result: EventResult, remote: &str) -> Value {
    let mut some = upload.clone();
    some["ops"] = json!(upload["ops"].as_array().unwrap().iter().filter(|op| op["kind"] == kind).cloned().collect::<Vec<Value>>());
    answer_to(s, account, &some, result, remote, "")
}

#[test]
fn a_copy_changed_before_the_answer_came_keeps_its_change() {
    let (mut s, account) = session();
    let (gus, copy) = lost_then_copied(&mut s, account);
    // A number added to the copy, before the next try.
    run(&mut s, "people.edit", json!({"id": copy, "phones": [{"label": "mobile", "value": "+41 79 555 02 02"}]}));
    let second = upload(&mut s, account);
    let kinds: Vec<&str> = second["ops"].as_array().unwrap().iter().filter_map(|op| op["kind"].as_str()).collect();
    assert_eq!(kinds, ["create", "update"], "made again, and the copy's change, in one call");
    // The server took both: the card is Gus's, with the number.
    answer_to(&mut s, account, &second, EventResult::Done, "c-gus", "");
    let left: Vec<Id> = s.mailbox.contacts.values().filter(|c| c.first == "Gus").map(|c| c.id).collect();
    assert_eq!(left, [gus], "one Gus, the one made here");
    assert!(s.mailbox.contact_changes.is_empty(), "nothing is queued that would take the number away again: {:?}", s.mailbox.contact_changes);
    // The read after it shows the card as the server has it.
    let with_phone = RemoteContact { phones: vec![Labeled::new("mobile", "+41 79 555 02 02")], ..card("c-gus", "Gus", "gus@example.org", "team") };
    apply_contacts(
        &mut s,
        &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace"), with_phone]),
    )
    .unwrap();
    assert_eq!(s.mailbox.contact(gus).map(|c| c.phones.len()), Some(1));
    // A copy whose change the server couldn't take yet stays, with its change, until it goes.
    let (mut s, account) = session();
    let (gus, copy) = lost_then_copied(&mut s, account);
    run(&mut s, "people.edit", json!({"id": copy, "title": "Joiner"}));
    let second = upload(&mut s, account);
    answer_only(&mut s, account, &second, "create", EventResult::Done, "c-gus");
    assert!(s.mailbox.contact(copy).is_some() && s.mailbox.contact(gus).is_some());
    let waiting: Vec<(ContactChangeKind, Id)> = s.mailbox.contact_changes.iter().map(|c| (c.kind, c.contact)).collect();
    assert_eq!(waiting, [(ContactChangeKind::Update, copy)]);
    let fields: ContactFields = serde_json::from_value(ops(&mut s, account)[0]["fields"].clone()).unwrap();
    assert_eq!(fields, ContactFields { title: Some("Joiner".into()), ..Default::default() });
}

#[test]
fn a_copy_deleted_before_the_answer_came_doesnt_take_the_contact_made_here_along() {
    // The copy's deletion went in the same call as the try that made Gus again: the card is gone there, and Gus, whom
    // the owner kept, is made anew.
    let (mut s, account) = session();
    let (gus, copy) = lost_then_copied(&mut s, account);
    run(&mut s, "people.delete", json!({"id": copy}));
    let second = upload(&mut s, account);
    answer_to(&mut s, account, &second, EventResult::Done, "c-gus", "");
    let here = s.mailbox.contact(gus).cloned().expect("Gus stays");
    assert!(here.origin.id.is_empty(), "made anew: the card he had is gone");
    let waiting: Vec<(ContactChangeKind, Id)> = s.mailbox.contact_changes.iter().map(|c| (c.kind, c.contact)).collect();
    assert_eq!(waiting, [(ContactChangeKind::Create, gus)]);
    apply_contacts(&mut s, &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace")]))
        .unwrap();
    assert!(s.mailbox.contact(gus).is_some(), "a read without the card keeps him: he is to be made");
    // The copy's deletion hadn't gone yet: it is not sent, for it would delete Gus's own card.
    let (mut s, account) = session();
    let (gus, copy) = lost_then_copied(&mut s, account);
    run(&mut s, "people.delete", json!({"id": copy}));
    let second = upload(&mut s, account);
    answer_only(&mut s, account, &second, "create", EventResult::Done, "c-gus");
    assert!(s.mailbox.contact_changes.is_empty(), "{:?}", s.mailbox.contact_changes);
    assert_eq!(s.mailbox.contact(gus).map(|c| c.origin.id.as_str()), Some("c-gus"));
    let gus_card = RemoteContact { emails: vec![Labeled::new("work", "gus@example.org")], ..card("c-gus", "Gus", "gus@example.org", "team") };
    apply_contacts(
        &mut s,
        &cards(account, vec![card("c-bob", "Bob", "bob@example.org", "team"), card("c-cy", "Cy", "cy@example.org", "grace"), gus_card]),
    )
    .unwrap();
    assert_eq!(s.mailbox.contacts.values().filter(|c| c.first == "Gus").map(|c| c.id).collect::<Vec<_>>(), [gus]);
}

#[test]
fn a_new_contact_the_server_keeps_making_without_a_name_is_left_to_the_next_read() {
    let (mut s, account) = session();
    let id = run(&mut s, "people.new", json!({"first": "Gus", "email": "gus@example.org"}))["id"].as_u64().unwrap();
    let said = answer(&mut s, account, EventResult::Done, "", "");
    assert_eq!((said["failed"].as_u64(), kinds(&s)), (Some(1), vec![ContactChangeKind::Create]), "once: it goes again");
    s.clock.now = s.clock.now.map(|now| now + 120);
    let said = answer(&mut s, account, EventResult::Done, "", "");
    assert!(s.mailbox.contact_changes.is_empty() && s.mailbox.contact(id).is_none(), "not asked for ever");
    assert_eq!(
        said["messages"],
        json!(["\"Gus\" was made on the server, which doesn't say what it calls it: it comes with the next read, as the server has it."])
    );
}

#[test]
fn a_change_the_server_keeps_failing_with_is_given_up_with_a_word_and_a_busy_server_never() {
    let (mut s, account) = session();
    let bob = kept(&s, "c-bob");
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    // A server that is busy, or a connection that breaks, is waited out however long it takes.
    for _ in 0..10 {
        answer(&mut s, account, EventResult::Failed, "", "the server is busy");
        s.clock.now = s.clock.now.map(|now| now + 1_000);
    }
    assert_eq!(kinds(&s), [ContactChangeKind::Update]);
    // One that fails with this very change (a hook of the server's that breaks on it): three times, then it is given
    // up and he follows the server again.
    let lasting = |s: &mut Session| {
        let upload = upload(s, account);
        let outcomes: Vec<ContactOutcome> = upload["ops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|op| ContactOutcome {
                op: serde_json::from_value(op.clone()).unwrap(),
                result: EventResult::Failed,
                error: "serverFail".into(),
                lasting: true,
                ..Default::default()
            })
            .collect();
        let uploaded = ContactsUploaded { account, email: ME.into(), outcomes, token_refused: false };
        let said = run(s, "sync.applyContactChanges", json!({"result": uploaded}));
        s.clock.now = s.clock.now.map(|now| now + 1_000);
        said
    };
    lasting(&mut s);
    lasting(&mut s);
    assert_eq!(kinds(&s), [ContactChangeKind::Update]);
    let said = lasting(&mut s);
    assert!(s.mailbox.contact_changes.is_empty());
    assert_eq!(s.mailbox.contact(bob.id).map(|c| c.title.as_str()), Some(""), "as the server has him");
    assert_eq!(
        said["messages"],
        json!(["The change to \"Bob\" could not be sent: the server keeps failing with it (serverFail). It was put back as the server has it."])
    );
}

#[test]
fn an_agents_redo_that_would_write_to_a_servers_book_needs_the_owners_yes() {
    let (mut s, _) = session();
    let bob = kept(&s, "c-bob");
    // The owner changes Bob, then takes it back.
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    assert!(s.undo());
    assert!(s.mailbox.contact_changes.is_empty());
    // An agent without the yes to write for everyone may not bring the change back: it would go to the server.
    s.agent = Some(vec![Class::Delete]);
    let why = refused(&mut s, "edit.redo", json!({}));
    assert!(why.contains("agent"), "{why}");
    assert!(s.mailbox.contact_changes.is_empty());
    // Nor bring back a deletion without the yes to delete.
    s.agent = None;
    run(&mut s, "people.delete", json!({"id": bob.id}));
    assert!(s.undo());
    s.agent = Some(vec![Class::Send]);
    let why = refused(&mut s, "edit.redo", json!({}));
    assert!(why.contains("agent"), "{why}");
    assert!(s.mailbox.contact(bob.id).is_some());
    // An undo that takes a waiting change back writes nothing there: it may.
    s.agent = None;
    run(&mut s, "people.edit", json!({"id": bob.id, "title": "Joiner"}));
    s.agent = Some(Vec::new());
    run(&mut s, "edit.undo", json!({}));
    assert!(s.mailbox.contact_changes.is_empty());
}
