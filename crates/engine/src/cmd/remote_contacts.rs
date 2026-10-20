// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The address books of an account's server (JMAP for Contacts, RFC 9610) and what the owner may do with the contacts
//! in them: the engine's side of [`mailcraft_model::book`].
//!
//! - **The books** are taken in as the server lists them (`sync.applyBooks`, [`apply_books`]): each is an
//!   [`AddressBook`] here under the server's name, with the account's rights there. One the server no longer lists goes
//!   when the list is all of them, and the contacts that were in it are in no book until a download of the contacts
//!   says where they are now; that download, when it is all there is and names none of them, removes them. Taking the
//!   list in is not an undo step, as mail that arrives is not.
//! - **The contacts** come down as before (`sync.applyContacts`, [`super::remote::merge_contacts`]), each in the book
//!   of its server that it says it is in ([`book_here`]).
//! - **What the owner may do.** A contact of a book whose contacts the account may not change refuses Edit and
//!   Delete, with the reason ([`change_refusal`]); one of a book whose contacts it may change and not delete refuses
//!   Delete alone, with its own ([`delete_refusal`]). The other contacts a server keeps are refused as before
//!   ([`super::remote::changeable`]): changes made here are not sent back yet. A session that keeps nothing of its own
//!   between visits ([`Session::keeps_local`]) refuses a new contact of the mailbox's own, which would be gone at the
//!   next visit, as no book of a server takes a new contact yet ([`adding_refusal`]). The reasons are fixed words, which
//!   a greyed control says on hover too.
//!
//! **The seam** with a model of collections of another kind (a CardDAV server's, or another model of books in the
//! engine): a host hands its collections over as [`RemoteBook`]s and its cards as contacts that name them
//! (`RemoteContact::book`). Nothing here knows what kind of server listed them, and nothing outside this file and
//! `book.rs` knows what a book is but by its id.

use std::collections::HashSet;

use mailcraft_model::book::{BooksDownload, MAX_BOOKS, RemoteBook};
use mailcraft_model::{AddressBook, Contact, Id, Mailbox, Origin};
use serde_json::{Value, json};

use super::bad;
use crate::{CmdError, CmdResult, CommandSpec, Session};

/// Why the owner may not make a contact here: there is no address book this one could go to that keeps it.
pub const NO_ADDING: &str = "Your rights do not let you add contacts";
/// Why the owner may not change (nor delete) a contact: its address book is one the account may only read.
pub const NO_CHANGING: &str = "Your rights do not let you change this contact";
/// Why the owner may not delete a contact: the account may change the contacts of its book, not delete them.
pub const NO_DELETING: &str = "Your rights do not let you delete this contact";
/// What a book the server gives no name is called here.
const SERVER_BOOK: &str = "Contacts";
/// Longest text of a parameter error handed back.
const MAX_SHOWN: usize = 200;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("sync.applyBooks", "Apply Downloaded Address Books", "Send / Receive", apply_books_command)
            .pure()
            .params(r#"{"download": BooksDownload} — the account's address books as a host brought them from its server: {"account": number, "email"?: string, "complete": bool, "books": [{"id": string, "name": string, "readOnly": bool, "mayRemove"?: bool, "main"?: bool, "plain"?: bool}]} — each is a book of that account here, with the account's rights there; one the server no longer lists goes when the list is complete, and its contacts are in no book until the contacts come down again. Answers {"added", "updated", "removed", "complete"}"#),
    ]
}

/// What taking in a list of address books changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BooksMerged {
    pub added: usize,
    pub updated: usize,
    /// Gone because the server no longer lists them.
    pub removed: usize,
    /// Contacts that were in a book that went, and are in none now.
    pub unbooked: usize,
}

impl BooksMerged {
    /// Whether the mailbox is any different for it.
    pub fn changed(&self) -> bool {
        self.added + self.updated + self.removed + self.unbooked > 0
    }
}

/// Take the address books of an account's server into the mailbox, as the server lists them (see the module).
/// Nothing is changed when this returns an error. Doing it twice changes nothing the second time.
pub fn merge_books(mailbox: &mut Mailbox, download: &BooksDownload) -> Result<BooksMerged, CmdError> {
    let account = super::remote::owner(mailbox, download.account, &download.email)?.id;
    let mut download = download.clone();
    download.tidy();
    let mut merged = BooksMerged::default();
    let listed: HashSet<&str> = download.books.iter().map(|remote| remote.id.as_str()).collect();
    for remote in &download.books {
        let found = mailbox.address_books.iter_mut().find(|book| book.origin.account == account && book.origin.id == remote.id);
        if let Some(book) = found {
            let fresh = book_from_remote(book.id, account, remote);
            if *book != fresh {
                *book = fresh;
                merged.updated += 1;
            }
        } else if mailbox.address_books.len() < MAX_BOOKS {
            let id = mailbox.alloc_id();
            mailbox.address_books.push(book_from_remote(id, account, remote));
            merged.added += 1;
        } else {
            log::debug!("contacts: a mailbox keeps {MAX_BOOKS} address books at most; one more was left out");
        }
    }
    // One main book an account: the one the list says, when it says one.
    if download.books.iter().any(|remote| remote.main) {
        for book in
            mailbox.address_books.iter_mut().filter(|book| book.origin.account == account && book.main && !listed.contains(book.origin.id.as_str()))
        {
            book.main = false;
            merged.updated += 1;
        }
    }
    if download.complete {
        let gone: Vec<Id> = mailbox
            .address_books
            .iter()
            .filter(|book| book.origin.account == account && !listed.contains(book.origin.id.as_str()))
            .map(|book| book.id)
            .collect();
        if !gone.is_empty() {
            mailbox.address_books.retain(|book| !gone.contains(&book.id));
            merged.removed = gone.len();
            // Where its contacts are now, the contacts' own download says.
            let held: Vec<Id> = mailbox.contacts.values().filter(|contact| gone.contains(&contact.book)).map(|contact| contact.id).collect();
            for id in held {
                if let Ok(contact) = mailbox.contact_mut(id) {
                    contact.book = 0;
                    merged.unbooked += 1;
                }
            }
        }
    }
    Ok(merged)
}

/// A book of the server as the mailbox keeps it, under the id `id` here.
fn book_from_remote(id: Id, account: Id, remote: &RemoteBook) -> AddressBook {
    AddressBook {
        id,
        name: if remote.name.is_empty() { SERVER_BOOK.to_string() } else { remote.name.clone() },
        origin: Origin { account, id: remote.id.clone(), ..Default::default() },
        read_only: remote.read_only,
        may_remove: remote.may_remove && !remote.read_only,
        main: remote.main,
        plain: remote.plain,
    }
}

/// Take in the address books a host brought ([`merge_books`]). Not an undo step.
pub fn apply_books(s: &mut Session, download: &BooksDownload) -> CmdResult {
    let merged = merge_books(&mut s.mailbox, download)?;
    if merged.changed() {
        s.mailbox_changed();
    }
    Ok(json!({"added": merged.added, "updated": merged.updated, "removed": merged.removed, "complete": download.complete}))
}

/// The book of `account` here that its server calls `server_id`, by its id here: what a contact the server says is in
/// that book is in. 0 for none: the server names no book, or one the mailbox doesn't have (yet).
pub fn book_here(mailbox: &Mailbox, account: Id, server_id: &str) -> Id {
    mailbox.address_book_on_server(account, server_id).map_or(0, |book| book.id)
}

/// Forget the address books of `account`'s server, for an account that is no longer followed: how many went. (Its
/// contacts go with the rest of what came from that server.)
pub fn forget_books(mailbox: &mut Mailbox, account: Id) -> usize {
    let before = mailbox.address_books.len();
    mailbox.address_books.retain(|book| book.origin.account != account);
    before - mailbox.address_books.len()
}

/// The book a contact is in, while the mailbox has the account whose server keeps it.
fn kept_in<'a>(mailbox: &'a Mailbox, contact: &Contact) -> Option<&'a AddressBook> {
    super::remote::keeper(mailbox, &contact.origin)?;
    mailbox.book_of(contact)
}

/// Why `contact` may not be changed here for its address book's sake: the account may only read the book's contacts.
/// `None` when its book says nothing against it (or it is in none); what else may stand in the way is
/// [`super::remote::changeable`]'s to say.
pub fn change_refusal(mailbox: &Mailbox, contact: &Contact) -> Option<&'static str> {
    kept_in(mailbox, contact).filter(|book| book.read_only).map(|_| NO_CHANGING)
}

/// Why `contact` may not be deleted here for its address book's sake: the account may only read the book's contacts,
/// or may change them and not delete them.
pub fn delete_refusal(mailbox: &Mailbox, contact: &Contact) -> Option<&'static str> {
    let book = kept_in(mailbox, contact)?;
    if book.read_only {
        Some(NO_CHANGING)
    } else if !book.may_remove {
        Some(NO_DELETING)
    } else {
        None
    }
}

/// Why a new contact can't be made: the session keeps nothing of its own between visits ([`Session::keeps_local`]),
/// and no address book of a server takes a new contact yet, so it would be kept nowhere.
pub fn adding_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_ADDING)
}

fn apply_books_command(s: &mut Session, v: &Value) -> CmdResult {
    let download = v.get("download").cloned().ok_or_else(|| bad("`download` is required"))?;
    // What can't be read is said in a few words: the error may repeat what it choked on.
    let download: BooksDownload = serde_json::from_value(download)
        .map_err(|e| bad(format!("`download`: {}", e.to_string().chars().filter(|c| !c.is_control()).take(MAX_SHOWN).collect::<String>())))?;
    apply_books(s, &download)
}

#[cfg(test)]
mod tests;
