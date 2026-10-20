// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Address books: the collections an account's server keeps its contacts in (JMAP for Contacts, RFC 9610), with what
//! the account may do in each ([`AddressBook`]), and how a host hands them over ([`BooksDownload`]).
//!
//! A contact that a server keeps names the book it is in ([`crate::Contact::book`]); one of a server that has no books
//! (Microsoft Graph's contacts) names none. The rights are the book's, as its server says them: whether its cards may
//! be changed here ([`AddressBook::read_only`]) and, in a book whose cards may be, whether they may be deleted
//! ([`AddressBook::may_remove`]).
//!
//! **The seam.** All there is of books is here and in the engine's `cmd/remote_contacts.rs`. The rest of the model
//! knows two things of them: a mailbox's list ([`crate::Mailbox::address_books`]) and a contact's book. A collection
//! model of another shape (a CardDAV server's, say) is mapped onto this one by what its server calls a collection and
//! by its rights, and a model that takes this one's place is a change of these two files and those two fields.
//!
//! Plain data, and lenient: a mailbox written before there were books reads as one that has none, and what a server
//! or a file says of them is tidied before it is kept.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::remote::clean_key;
use crate::{Account, Contact, Id, MAX_NAME, Mailbox, Origin};

/// Most address books one account is taken to have; what is beyond is left out.
pub const MAX_REMOTE_BOOKS: usize = 50;
/// Most address books a mailbox keeps, every account together.
pub const MAX_BOOKS: usize = 200;

fn is_false(b: &bool) -> bool {
    !*b
}

/// No book: what a contact of the mailbox's own, or of a server without books, names.
pub(crate) fn no_book(book: &Id) -> bool {
    *book == 0
}

/// An address book that an account's server keeps: the contacts in it are copies of the server's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AddressBook {
    pub id: Id,
    /// The name its server gives it.
    pub name: String,
    /// The account whose server keeps it, and what that server calls it ([`Origin::id`]: never empty).
    pub origin: Origin,
    /// Its contacts may be read here and not changed: the account's rights there say so, or what is changed here
    /// would not reach the server.
    #[serde(skip_serializing_if = "is_false")]
    pub read_only: bool,
    /// Its contacts may be deleted, in a book whose contacts may be changed. Not until a server says so.
    #[serde(skip_serializing_if = "is_false")]
    pub may_remove: bool,
    /// It is the account's main address book: the one a new contact of the account goes to.
    #[serde(skip_serializing_if = "is_false")]
    pub main: bool,
    /// Its server keeps only the basics of a contact: the first and last name, the company, the job title, the e-mail
    /// addresses and the phone numbers, and of a number's label no more than whether it is a mobile one. Whatever else
    /// a contact has here would not reach it.
    #[serde(skip_serializing_if = "is_false")]
    pub plain: bool,
}

/// An address book as a server has it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteBook {
    /// What the server calls it; a book without that is not kept.
    pub id: String,
    pub name: String,
    /// The account may read its contacts and not change them.
    pub read_only: bool,
    /// The account may delete its contacts (in a book it may change).
    #[serde(skip_serializing_if = "is_false")]
    pub may_remove: bool,
    /// It is the account's main address book.
    #[serde(skip_serializing_if = "is_false")]
    pub main: bool,
    /// Its server keeps only the basics of a contact ([`AddressBook::plain`]).
    #[serde(skip_serializing_if = "is_false")]
    pub plain: bool,
}

/// An account's address books, as its server lists them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BooksDownload {
    /// The account whose books they are ([`BooksDownload::for_account`]).
    pub account: Id,
    /// That account's address: a download is not taken in by another account that happens to have the id.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// Every book the server has is here. When not, none is removed for not being here.
    pub complete: bool,
    pub books: Vec<RemoteBook>,
}

impl BooksDownload {
    /// The download as one for `account`.
    pub fn for_account(mut self, account: &Account) -> BooksDownload {
        self.account = account.id;
        self.email = account.email.trim().to_string();
        self
    }

    /// Cut to size: books without an id that is one, or said twice, go; names are one line; the list is capped; a book
    /// that may not be changed is not one whose contacts may be deleted; and one book at most is the main one.
    pub fn tidy(&mut self) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut main = false;
        for mut book in std::mem::take(&mut self.books) {
            book.id = clean_key(&book.id);
            if book.id.is_empty() || seen.contains(&book.id) || seen.len() >= MAX_REMOTE_BOOKS {
                continue;
            }
            seen.insert(book.id.clone());
            book.name = one_line(&book.name, MAX_NAME);
            book.may_remove = book.may_remove && !book.read_only;
            book.main = book.main && !main;
            main |= book.main;
            self.books.push(book);
        }
    }
}

/// One line of text from outside: control characters read as spaces, at most `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let short: String = flat.trim().chars().take(max).collect();
    short.trim_end().to_string()
}

impl Mailbox {
    /// The address book with the id `id`.
    pub fn address_book(&self, id: Id) -> Option<&AddressBook> {
        if id == 0 { None } else { self.address_books.iter().find(|book| book.id == id) }
    }

    /// The address book a contact is in: one of its server's, while the mailbox has the account.
    pub fn book_of(&self, contact: &Contact) -> Option<&AddressBook> {
        self.address_book(contact.book).filter(|book| contact.origin.is_remote() && book.origin.account == contact.origin.account)
    }

    /// The address book of `account` that its server calls `server_id`.
    pub fn address_book_on_server(&self, account: Id, server_id: &str) -> Option<&AddressBook> {
        if server_id.is_empty() {
            return None;
        }
        self.address_books.iter().find(|book| book.origin.account == account && book.origin.id == server_id)
    }
}

/// Keep a mailbox's books whole, as [`Mailbox::repair`] keeps every part of it: each has an id that is one (`sane`;
/// a new one otherwise), is a server's (an account the mailbox has, and a name the server gives it that can be one),
/// and is there once (a second book with the id of one before it goes, and so does a second for the same book of the
/// same server, whose contacts are then the first one's); one book of an account at most is its main one; and a
/// contact names a book of its own account, or none. What was set right, a line each.
pub(crate) fn repair(mailbox: &mut Mailbox, sane: &dyn Fn(&Id) -> bool) -> Vec<String> {
    let mut fixed = Vec::new();
    let accounts: HashSet<Id> = mailbox.accounts.iter().map(|account| account.id).collect();
    let listed = std::mem::take(&mut mailbox.address_books);
    let before = listed.len();
    let mut kept: Vec<AddressBook> = Vec::new();
    // The contacts of a book that went go to the book kept in its place, or to none (0).
    let mut moved: HashMap<Id, Id> = HashMap::new();
    let mut renumbered = 0usize;
    for mut book in listed {
        let named = !book.origin.id.is_empty() && clean_key(&book.origin.id) == book.origin.id;
        if !accounts.contains(&book.origin.account) || !named || kept.len() >= MAX_BOOKS {
            moved.insert(book.id, 0);
            continue;
        }
        if let Some(first) = kept.iter().find(|have| have.origin.account == book.origin.account && have.origin.id == book.origin.id) {
            moved.insert(book.id, first.id);
            continue;
        }
        // (Its contacts can't be told from the first one's: they stay with that one.)
        if kept.iter().any(|have| have.id == book.id) {
            continue;
        }
        if book.id == 0 || !sane(&book.id) {
            let fresh = mailbox.alloc_id();
            if book.id != 0 {
                moved.insert(book.id, fresh);
            }
            book.id = fresh;
            renumbered += 1;
        }
        book.name = one_line(&book.name, MAX_NAME);
        book.may_remove = book.may_remove && !book.read_only;
        book.main = book.main && !kept.iter().any(|have| have.main && have.origin.account == book.origin.account);
        kept.push(book);
    }
    if kept.len() < before {
        fixed.push(format!("{} address book(s) that no account here keeps, or listed twice, removed", before - kept.len()));
    }
    if renumbered > 0 {
        fixed.push(format!("{renumbered} address book id(s) corrected"));
    }
    let homes: HashMap<Id, Id> = kept.iter().map(|book| (book.id, book.origin.account)).collect();
    moved.retain(|from, _| !homes.contains_key(from));
    mailbox.address_books = kept;
    let mut cleared = 0usize;
    for contact in mailbox.contacts.values_mut() {
        if contact.book == 0 {
            continue;
        }
        let to = moved.get(&contact.book).copied().unwrap_or(contact.book);
        let to = if contact.origin.is_remote() && homes.get(&to) == Some(&contact.origin.account) { to } else { 0 };
        if to != contact.book {
            Arc::make_mut(contact).book = to;
            cleared += 1;
        }
    }
    if cleared > 0 {
        fixed.push(format!("{cleared} contact(s) taken out of an address book of another account, or of none"));
    }
    fixed
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;
    use crate::remote::RemoteContact;

    fn round_trip<T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> String {
        let json = serde_json::to_string(value).unwrap();
        assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), value, "{json}");
        json
    }

    /// A mailbox with a local account and a server's (id 5 is not taken), and its account id.
    fn mailbox() -> (Mailbox, Id) {
        let mut mailbox = Mailbox::with_local_account("Ada Marlow", "ada@example.org");
        let account = mailbox.add_account(Account { email: "ada@neoffice.example.org".into(), ..Default::default() });
        (mailbox, account)
    }

    fn book(id: Id, account: Id, server: &str) -> AddressBook {
        AddressBook { id, name: format!("Book {server}"), origin: Origin { account, id: server.into(), ..Default::default() }, ..Default::default() }
    }

    fn kept(mailbox: &mut Mailbox, account: Id, book: Id, name: &str) -> Id {
        mailbox.add_contact(Contact {
            first: name.into(),
            book,
            origin: Origin { account, id: format!("card-{name}"), ..Default::default() },
            ..Default::default()
        })
    }

    #[test]
    fn a_mailbox_from_before_address_books_reads_as_before() {
        // A mailbox and a contact written before there were books: none, and nothing of them is written back.
        let (mut mailbox, account) = mailbox();
        let id = kept(&mut mailbox, account, 0, "Grace");
        let json = serde_json::to_string(&mailbox).unwrap();
        assert!(!json.contains("addressBooks") && !json.contains("\"book\""), "{json}");
        let read: Mailbox = serde_json::from_str(&json).unwrap();
        assert!(read.address_books.is_empty() && read == mailbox);
        assert_eq!(read.contact(id).map(|c| c.book), Some(0));
        // A list that isn't one, or entries that aren't books, read as nothing rather than as a broken file.
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["addressBooks"] = json!(5);
        assert!(serde_json::from_value::<Mailbox>(value.clone()).unwrap().address_books.is_empty());
        value["addressBooks"] = json!([7, {"id": "x"}, {"id": 9, "name": "Team", "origin": {"account": account, "id": "team"}}]);
        let lenient: Mailbox = serde_json::from_value(value).unwrap();
        assert_eq!(lenient.address_books.iter().map(|b| b.id).collect::<Vec<_>>(), [9]);
    }

    #[test]
    fn a_book_round_trips_and_junk_is_tidied() {
        let mut team = book(9, 3, "team");
        team.plain = true;
        team.main = true;
        let json = round_trip(&team);
        assert!(json.contains(r#""main":true"#) && json.contains(r#""plain":true"#) && !json.contains("readOnly") && !json.contains("mayRemove"));
        let read_only = AddressBook { read_only: true, ..book(10, 3, "grace") };
        assert!(round_trip(&read_only).contains(r#""readOnly":true"#));
        // What a server lists: ids that can't be ids, a book said twice, two main ones, names of many lines, rights
        // that contradict each other, and more books than any account has.
        let mut download = BooksDownload {
            account: 3,
            email: "ada@neoffice.example.org".into(),
            complete: true,
            books: vec![
                RemoteBook { id: "neoffice".into(), name: "Neoffice\r\ncontacts".into(), main: true, may_remove: true, ..Default::default() },
                RemoteBook { id: "neoffice".into(), name: "Again".into(), ..Default::default() },
                RemoteBook { id: "two words".into(), ..Default::default() },
                RemoteBook { id: "x".repeat(2_000), ..Default::default() },
                RemoteBook { id: "shared".into(), name: "é".repeat(500), read_only: true, may_remove: true, main: true, ..Default::default() },
            ],
        };
        download.books.extend((0..2 * MAX_REMOTE_BOOKS).map(|n| RemoteBook { id: format!("b{n}"), ..Default::default() }));
        download.tidy();
        assert_eq!(download.books.len(), MAX_REMOTE_BOOKS);
        let first = &download.books[0];
        assert_eq!((first.id.as_str(), first.name.as_str(), first.main, first.may_remove), ("neoffice", "Neoffice  contacts", true, true));
        let shared = &download.books[1];
        assert_eq!((shared.id.as_str(), shared.name.chars().count(), shared.main, shared.may_remove), ("shared", MAX_NAME, false, false));
        assert!(download.books.iter().filter(|b| b.main).count() == 1 && !download.books.iter().any(|b| b.id.contains(' ')));
        // Tidied once is tidied, and what isn't said reads as nothing.
        let once = download.clone();
        download.tidy();
        assert_eq!(download, once);
        round_trip(&download);
        assert_eq!(serde_json::from_str::<BooksDownload>("{}").unwrap(), BooksDownload::default());
        assert_eq!(round_trip(&RemoteBook::default()), r#"{"id":"","name":"","readOnly":false}"#);
        // A contact of a server's says which of its books it is in; one of a server without books says nothing of it.
        let mut card = RemoteContact { id: "c1".into(), book: " neoffice ".into(), ..Default::default() };
        card.tidy();
        assert_eq!(card.book, "neoffice");
        assert!(!serde_json::to_string(&RemoteContact { id: "c1".into(), ..Default::default() }).unwrap().contains("book"));
        let mut junk = RemoteContact { id: "c1".into(), book: "two\nlines".into(), ..Default::default() };
        junk.tidy();
        assert!(junk.book.is_empty());
    }

    #[test]
    fn repair_drops_a_book_id_that_names_nothing_and_clears_a_contacts_unknown_book() {
        let (mut mailbox, account) = mailbox();
        let local = mailbox.accounts[0].id;
        let team = mailbox.alloc_id();
        let gone = mailbox.alloc_id();
        let twice = mailbox.alloc_id();
        mailbox.address_books = vec![
            AddressBook { main: true, ..book(team, account, "team") },
            // Of an account the mailbox doesn't have, and with nothing the server calls it.
            book(gone, 424_242, "old"),
            book(gone + 100, account, ""),
            // The same book of the same server again, under another id: its contacts are the first one's.
            AddressBook { main: true, ..book(twice, account, "team") },
            // Another book that has the first one's id.
            book(team, account, "other"),
            // An id that can't be one, and none.
            book(1 << 60, account, "big"),
            book(0, account, "zero"),
        ];
        let in_team = kept(&mut mailbox, account, team, "Ada");
        let in_twice = kept(&mut mailbox, account, twice, "Bob");
        let in_gone = kept(&mut mailbox, account, gone, "Cy");
        let in_big = kept(&mut mailbox, account, 1 << 60, "Dee");
        let nowhere = kept(&mut mailbox, account, 777_777, "Eve");
        // A contact of the mailbox's own, and one of another account, that name the team's book.
        let own = mailbox.add_contact(Contact { first: "Fay".into(), book: team, ..Default::default() });
        let other = kept(&mut mailbox, local, team, "Gus");
        let fixed = mailbox.repair();
        assert!(fixed.iter().any(|line| line.contains("address book(s)")), "{fixed:?}");
        let books: Vec<(&str, bool)> = mailbox.address_books.iter().map(|b| (b.origin.id.as_str(), b.main)).collect();
        assert_eq!(books, [("team", true), ("big", false), ("zero", false)]);
        let big = mailbox.address_book_on_server(account, "big").map(|b| b.id).unwrap();
        assert!(big != 1 << 60 && big != 0 && mailbox.address_book_on_server(account, "zero").is_some_and(|b| b.id != 0));
        let book_of = |id: Id| mailbox.contact(id).map(|c| c.book).unwrap();
        assert_eq!((book_of(in_team), book_of(in_twice), book_of(in_big)), (team, team, big));
        assert_eq!((book_of(in_gone), book_of(nowhere), book_of(own), book_of(other)), (0, 0, 0, 0));
        // Ids given since are none of theirs, and a mailbox that is whole stays as it is.
        let next = mailbox.alloc_id();
        assert!(mailbox.address_books.iter().all(|b| b.id < next));
        assert!(mailbox.clone().repair().is_empty(), "{:?}", mailbox.clone().repair());
        assert_eq!(mailbox.book_of(mailbox.contact(in_team).unwrap()).map(|b| b.origin.id.as_str()), Some("team"));
        assert!(mailbox.address_book(0).is_none() && mailbox.address_book_on_server(account, "").is_none());
    }

    proptest! {
        #[test]
        fn hostile_books_never_panic(text in any::<String>(), id in any::<u64>(), account in 0u64..8, flags in any::<[bool; 4]>()) {
            let remote = RemoteBook { id: text.clone(), name: text.clone(), read_only: flags[0], may_remove: flags[1], main: flags[2], plain: flags[3] };
            let mut download = BooksDownload { account, email: text.clone(), complete: flags[0], books: vec![remote.clone(), remote] };
            download.tidy();
            prop_assert!(download.books.len() <= 1);
            prop_assert!(download.books.iter().all(|b| b.name.chars().count() <= MAX_NAME && !(b.read_only && b.may_remove)));
            let (mut mailbox, server) = mailbox();
            let origin = Origin { account: if flags[0] { server } else { account }, id: text.clone(), ..Default::default() };
            mailbox.address_books = vec![AddressBook { id, name: text.clone(), origin, ..Default::default() }];
            // A contact as a server's are: of its account, under the server's name for it.
            let card = Origin { account: server, id: "c1".into(), ..Default::default() };
            mailbox.add_contact(Contact { first: "Ada".into(), book: id, origin: card, ..Default::default() });
            mailbox.repair();
            prop_assert!(mailbox.clone().repair().is_empty());
            let json = serde_json::to_string(&mailbox).unwrap();
            prop_assert_eq!(serde_json::from_str::<Mailbox>(&json).unwrap(), mailbox);
        }
    }
}
