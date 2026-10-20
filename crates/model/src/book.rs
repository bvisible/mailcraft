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
//! **Written back.** A contact made, changed or deleted here in a book whose contacts the account may change is noted
//! for its server ([`ContactChange`], in [`crate::Mailbox::contact_changes`]), as an event is: the change is made here
//! at once, a host tells the server ([`ContactsUpload`]) and brings back what it made of each ([`ContactsUploaded`]),
//! and the note stays until the server took the change or refused it for good. What is sent of a contact is the part
//! of it a server's card holds ([`ContactFields`]): its name, its company, its job title, its e-mail addresses and its
//! phone numbers.
//!
//! Plain data, and lenient: a mailbox written before there were books reads as one that has none, and what a server
//! or a file says of them is tidied before it is kept.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mailcraft_time::Timestamp;
use serde::{Deserialize, Serialize};

use crate::remote::{EventResult, clean_key};
use crate::{Account, Contact, Id, Labeled, MAX_NAME, Mailbox, Origin};

/// Most address books one account is taken to have; what is beyond is left out.
pub const MAX_REMOTE_BOOKS: usize = 50;
/// Most address books a mailbox keeps, every account together.
pub const MAX_BOOKS: usize = 200;
/// Most changes to contacts that may wait for servers at once ([`crate::Mailbox::contact_changes`]), and most that one
/// [`ContactsUpload`] carries: the rest go with the next.
pub const MAX_CONTACT_CHANGES: usize = 5_000;
pub const MAX_CONTACT_OPS: usize = 50;
/// Longest mark a new contact is sent with ([`ContactChange::token`]), and longest words of a server's kept.
pub const MAX_CONTACT_TOKEN: usize = 64;
const MAX_WHY: usize = 300;
/// Longest one-line field of a contact that is sent, and most addresses or numbers of one.
const MAX_LINE: usize = 500;
const MAX_ENTRIES: usize = 50;

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn is_zero_time(t: &Timestamp) -> bool {
    *t == 0
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

/// What is done to a contact that a server keeps in an address book.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContactChangeKind {
    /// A contact made here that the server doesn't have yet.
    #[default]
    Create,
    /// A contact the server has that was changed here.
    Update,
    /// A contact the server has that was deleted here.
    Delete,
}

/// A change made here to a contact of an address book that an account's server keeps, which the server is yet to be
/// told. The change itself is made in the mailbox already; this is the note that it is to go to the server too. A host
/// carries it out ([`ContactsUpload`]), and it stays listed until the server has taken it, or has refused it for good
/// ([`ContactsUploaded`]).
///
/// What is sent is worked out when it goes, from the contact as it is then: a new contact whole, a changed one as what
/// differs from `base`. So several changes to one contact before the server hears of any are one change, and one that
/// is taken back is none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContactChange {
    pub account: Id,
    pub kind: ContactChangeKind,
    /// The contact in the mailbox; for a deletion, the id it had.
    pub contact: Id,
    /// What the server calls the contact. Empty for one it doesn't have yet.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote: String,
    /// For a new contact: the mark it is sent with (its card's `uid`). Sending it a second time (the answer to the first
    /// was lost) then makes no second one, and it is known again when a download brings it before the answer.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    /// For a change or a deletion: the contact as the server has it. What is sent is what differs from this, and this
    /// is what is put back when the server refuses for good.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Box<Contact>>,
    /// How many times in a row the server couldn't be told.
    #[serde(skip_serializing_if = "is_zero")]
    pub failures: u32,
    /// Not to be tried again before this (Unix seconds); 0 for at once.
    #[serde(skip_serializing_if = "is_zero_time")]
    pub retry_at: Timestamp,
    /// The server has said once that the account may not do this: it is tried once more, and given up when it is
    /// refused again.
    #[serde(skip_serializing_if = "is_false")]
    pub denied: bool,
    /// Why it didn't go through the last time, for the owner to read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// How many times in a row the server failed with it in a way that asking again is unlikely to get past (it
    /// failed with this very change, or said nothing of it): given up after a few, with a word.
    #[serde(skip_serializing_if = "is_zero")]
    pub stuck: u32,
}

/// A contact's name as a server's card holds it: one whole, sent together ([`ContactFields::name`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContactName {
    pub first: String,
    pub last: String,
    /// The name it is shown by, when it is not its first and last name ([`Contact::display_name`]); empty otherwise.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub display: String,
}

/// What a server is told of a contact: everything for a new one, and for one it has only what was changed here
/// (`None` leaves a part as the server has it: somebody else may have changed it meanwhile). The parts are those a
/// server's card holds of a contact here: the rest of a contact is not sent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContactFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<ContactName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    /// The job title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The e-mail addresses, the primary one first, with their labels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emails: Option<Vec<Labeled>>,
    /// The phone numbers, with their labels (`mobile`, `work`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phones: Option<Vec<Labeled>>,
}

impl ContactFields {
    /// Nothing is said: there is nothing to tell the server.
    pub fn is_empty(&self) -> bool {
        *self == ContactFields::default()
    }

    /// Which of them are said, by the names a card has for them: for a log line, which may say what kind of thing was
    /// sent and never what it was.
    pub fn names(&self) -> Vec<&'static str> {
        let said = [
            (self.name.is_some(), "name"),
            (self.company.is_some(), "organizations"),
            (self.title.is_some(), "titles"),
            (self.emails.is_some(), "emails"),
            (self.phones.is_some(), "phones"),
        ];
        said.into_iter().filter(|(is, _)| *is).map(|(_, name)| name).collect()
    }

    /// Cut to the size a server is sent: names one line each, empty entries gone, an e-mail address one or none,
    /// lists capped.
    pub fn tidy(&mut self) {
        let line = |text: &str| one_line(text, MAX_LINE);
        if let Some(name) = &mut self.name {
            (name.first, name.last, name.display) = (line(&name.first), line(&name.last), line(&name.display));
        }
        self.company = self.company.take().map(|text| line(&text));
        self.title = self.title.take().map(|text| line(&text));
        let entries = |list: Vec<Labeled>, keep: &dyn Fn(&str) -> bool| -> Vec<Labeled> {
            list.into_iter()
                .map(|entry| Labeled { label: one_line(&entry.label, MAX_NAME), value: line(&entry.value) })
                .filter(|entry| keep(&entry.value))
                .take(MAX_ENTRIES)
                .collect()
        };
        self.emails = self.emails.take().map(|list| entries(list, &|address| address.contains('@') && !address.contains(char::is_whitespace)));
        self.phones = self.phones.take().map(|list| entries(list, &|number| !number.is_empty()));
    }
}

/// One thing a server is asked to do to a contact ([`ContactsUpload`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContactOp {
    pub kind: ContactChangeKind,
    /// The contact in the mailbox the change is about (for the answer to find it again).
    pub contact: Id,
    /// What the server calls the contact, for a change or a deletion.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote: String,
    /// The mark a new contact is made with ([`ContactChange::token`]).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    /// What the server calls the address book the contact is in: the one a new contact goes into.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub book: String,
    /// A new contact whole, or what changed of one the server has; nothing for a deletion.
    pub fields: ContactFields,
    /// For a change: what the parts it names were as the server had them when they were changed here, so that only
    /// what was changed here (an address added, a number taken out) is made to the server's card as it is now, and
    /// what someone else added there meanwhile stays. `None`: the parts named are written as they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<ContactFields>,
    /// For a change or a deletion: the server's mark of the contact the change is about (when it was made there,
    /// [`crate::Origin::change_key`]). A card that has another is another contact, which a server gave the same name
    /// once this one was gone: it is never written to. Empty when unknown.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mark: String,
}

impl ContactOp {
    /// Cut to size, as everything from outside is: an id that can't be one is none.
    pub fn tidy(&mut self) {
        self.remote = clean_key(&self.remote);
        self.token = clean_key(&self.token).chars().take(MAX_CONTACT_TOKEN).collect();
        self.book = clean_key(&self.book);
        self.fields.tidy();
        if let Some(base) = &mut self.base {
            base.tidy();
        }
        self.mark = clean_key(&self.mark);
    }
}

/// The changes to the contacts of an account's address books that its server is to be told, oldest first. The engine
/// says what they are (`sync.contactChanges`); a host carries them out and brings back a [`ContactsUploaded`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContactsUpload {
    pub account: Id,
    /// That account's address, as in [`BooksDownload::email`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    pub ops: Vec<ContactOp>,
}

impl ContactsUpload {
    /// The upload as one for `account`.
    pub fn for_account(mut self, account: &Account) -> ContactsUpload {
        self.account = account.id;
        self.email = account.email.trim().to_string();
        self
    }
}

/// What became of one [`ContactOp`], with the op itself: the mailbox may have moved on since it was asked for. The
/// answer words are an event's ([`EventResult`]): done, gone, refused, denied, failed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContactOutcome {
    pub op: ContactOp,
    pub result: EventResult,
    /// What the server calls the contact now, for one that was made.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote: String,
    /// Why not, in words for the owner; never anything that was sent.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// How many seconds the server asked to be left alone, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u32>,
    /// For a change that failed: the server answered, and failed with this very change (or said nothing of it), which
    /// asking again is unlikely to get past; not for a server that was busy or a connection that broke.
    #[serde(skip_serializing_if = "is_false")]
    pub lasting: bool,
}

impl ContactOutcome {
    /// Cut to size, as everything from outside is.
    pub fn tidy(&mut self) {
        self.op.tidy();
        self.remote = clean_key(&self.remote);
        self.error = one_line(&self.error, MAX_WHY);
    }
}

/// What a server made of a [`ContactsUpload`]: an outcome for every op that was tried, in order. An op without one was
/// not tried (the server stopped taking the sign-in, or was too busy), and waits as before.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContactsUploaded {
    pub account: Id,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    pub outcomes: Vec<ContactOutcome>,
    /// The server stopped taking the sign-in part-way: a new one is needed for the rest.
    #[serde(skip_serializing_if = "is_false")]
    pub token_refused: bool,
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
    // What waits to be told to a server is for an account here, and no more of it than is ever noted.
    let waiting = mailbox.contact_changes.len();
    mailbox.contact_changes.retain(|change| accounts.contains(&change.account));
    mailbox.contact_changes.truncate(MAX_CONTACT_CHANGES);
    if mailbox.contact_changes.len() < waiting {
        fixed.push(format!(
            "{} change(s) of contacts waiting for a server that no account here keeps dropped",
            waiting - mailbox.contact_changes.len()
        ));
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

    #[test]
    fn a_change_to_a_contact_round_trips_and_what_goes_up_is_tidied() {
        // A mailbox with nothing waiting writes nothing of it, and reads as before.
        let (mut mailbox, account) = mailbox();
        assert!(!serde_json::to_string(&mailbox).unwrap().contains("contactChanges"));
        let base = Contact {
            id: 7,
            first: "Grace".into(),
            book: 9,
            origin: Origin { account, id: "c-grace".into(), ..Default::default() },
            ..Default::default()
        };
        mailbox.contact_changes.push(ContactChange {
            account,
            kind: ContactChangeKind::Update,
            contact: 7,
            remote: "c-grace".into(),
            base: Some(Box::new(base)),
            failures: 2,
            retry_at: 1_791_993_660,
            error: "the server failed".into(),
            ..Default::default()
        });
        let json = serde_json::to_string(&mailbox).unwrap();
        assert!(json.contains(r#""contactChanges":[{"account":"#) && json.contains(r#""kind":"update""#), "{json}");
        assert_eq!(serde_json::from_str::<Mailbox>(&json).unwrap(), mailbox);
        assert_eq!(round_trip(&ContactChange::default()), r#"{"account":0,"kind":"create","contact":0}"#);
        // A list that isn't one, or entries that aren't changes, read as nothing.
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["contactChanges"] = json!([7, {"kind": "explode"}, {"account": account, "kind": "delete", "remote": "c-x"}]);
        let lenient: Mailbox = serde_json::from_value(value).unwrap();
        assert_eq!(lenient.contact_changes.iter().map(|c| c.kind).collect::<Vec<_>>(), [ContactChangeKind::Delete]);
        // What a host is handed and brings back, from outside: names of many lines, an address that is none, ids that
        // can't be ids, words of a thousand lines.
        let mut op = ContactOp {
            kind: ContactChangeKind::Create,
            contact: 7,
            remote: "two words".into(),
            token: "t".repeat(500),
            book: " neoffice ".into(),
            fields: ContactFields {
                name: Some(ContactName { first: "Gr\nace".into(), last: "O".repeat(2_000), display: String::new() }),
                company: Some("Okafor\r\nJoinery".into()),
                title: None,
                emails: Some(vec![Labeled::new("work", "grace@example.org"), Labeled::new("", "not an address"), Labeled::new("x\ny", "  ")]),
                phones: Some((0..200).map(|n| Labeled::new("mobile", format!("+41 79 555 {n:02}"))).collect()),
            },
            base: Some(ContactFields { company: Some("Okafor\nJoinery".into()), ..Default::default() }),
            mark: "2026-10-01T08:00:00Z\n".into(),
        };
        op.tidy();
        assert_eq!((op.base.as_ref().and_then(|b| b.company.as_deref()), op.mark.as_str()), (Some("Okafor Joinery"), "2026-10-01T08:00:00Z"));
        assert!(op.remote.is_empty() && op.token.len() == MAX_CONTACT_TOKEN && op.book == "neoffice");
        let name = op.fields.name.as_ref().unwrap();
        assert!(name.first == "Gr ace" && name.last.chars().count() == MAX_LINE);
        assert_eq!(op.fields.company.as_deref(), Some("Okafor  Joinery"));
        assert_eq!(op.fields.emails, Some(vec![Labeled::new("work", "grace@example.org")]));
        assert_eq!(op.fields.phones.as_ref().map(Vec::len), Some(MAX_ENTRIES));
        assert_eq!(op.fields.names(), ["name", "organizations", "emails", "phones"]);
        assert!(ContactFields::default().is_empty() && !op.fields.is_empty());
        let mut outcome = ContactOutcome {
            op: op.clone(),
            result: EventResult::Refused,
            remote: "c\u{0}x".into(),
            error: "no\n".repeat(1_000),
            retry_after: Some(5),
            lasting: true,
        };
        outcome.tidy();
        assert!(outcome.remote.is_empty() && !outcome.error.contains('\n') && outcome.error.chars().count() <= MAX_WHY);
        let uploaded = ContactsUploaded { account, email: "ada@neoffice.example.org".into(), outcomes: vec![outcome], token_refused: true };
        assert!(round_trip(&uploaded).contains(r#""result":"refused""#));
        round_trip(&ContactsUpload { ops: vec![op], ..Default::default() }.for_account(&mailbox.accounts[1]));
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
