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
//!   of its server that it says it is in ([`book_here`]); one whose change made here waits stays as it is here until
//!   the server was told ([`written`]).
//! - **What the owner may do.** A contact of a book whose contacts the account may not change refuses Edit and
//!   Delete, with the reason ([`change_refusal`]); one of a book whose contacts it may change and not delete refuses
//!   Delete alone, with its own ([`delete_refusal`]). In a book whose contacts it may change, what is made, changed and
//!   deleted here goes to the server ([`written`]); of a contact, a server's card holds its name, its company, its job
//!   title, its e-mail addresses and its phone numbers, and a change to anything else is refused with the reason
//!   ([`unkept`]), as is a display name of its own in a book whose server keeps only the basics. The other contacts a
//!   server keeps (those of a server that has no books) are refused as before ([`super::remote::changeable`]): changes
//!   made here are not sent back. The reasons are fixed words, which a greyed control says on hover too.
//! - **Where a new contact goes** ([`target_book`]): to the book a caller names, else to the main book of the account
//!   on screen whose contacts the account may change, else to another of its books it may change, else to such a book
//!   of another account ([`default_book`]), else into the mailbox's own contacts. A session that keeps nothing of its
//!   own between visits ([`Session::keeps_local`]) refuses that last, which would be gone at the next visit
//!   ([`adding_refusal`]).
//!
//! **The seam** with a model of collections of another kind (a CardDAV server's, or another model of books in the
//! engine): a host hands its collections over as [`RemoteBook`]s and its cards as contacts that name them
//! (`RemoteContact::book`), and carries out what [`written`] says is to go. Nothing here knows what kind of server
//! listed them, and nothing outside this file and `book.rs` knows what a book is but by its id.

use std::collections::HashSet;

use mailcraft_model::book::{BooksDownload, MAX_BOOKS, RemoteBook};
use mailcraft_model::{AddressBook, Contact, Id, Mailbox, Origin};
use serde_json::{Value, json};

use super::{bad, fail};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub mod written;

pub(crate) use written::before_merge;
pub use written::{
    ContactDropped, ContactsApplied, apply_contact_outcomes, changed_fields, contact_dropped_json, contact_fields, contact_upload, contacts_waiting,
    note_sending, note_unsent, settle_contact_changes,
};

/// Why the owner may not make a contact here: the servers' address books are ones the account may only read.
pub const NO_ADDING: &str = "Your rights do not let you add contacts";
/// Why the owner may not make a contact here: the session keeps none of its own, and no server lists an address book.
pub const NO_BOOKS: &str = "This page keeps no contacts of its own, and its server has no address book to put new ones in";
/// Why the owner may not make a contact here yet: the session keeps none of its own, and the servers' address books are
/// still to come ([`Session::books_listed`]).
pub const NO_BOOKS_YET: &str = "The address books are still being read from the server";
/// Why the owner may not change a contact: its address book is one the account may only read.
pub const NO_CHANGING: &str = "Your rights do not let you change this contact";
/// Why the owner may not delete a contact: the account may only read its book's contacts, or change them and not
/// delete them.
pub const NO_DELETING: &str = "Your rights do not let you delete this contact";
/// Why a part of a contact can't be set here: its address book keeps no more of a contact than that.
pub const NO_KEEPING: &str = "Its address book keeps only a contact's name, company, job title, e-mail addresses and phone numbers";
/// Why a contact can't be put in another address book by a change.
pub const NO_MOVING: &str = "A contact can't be moved to another address book here";
/// What a book the server gives no name is called here.
const SERVER_BOOK: &str = "Contacts";
/// Longest text of a parameter error handed back.
const MAX_SHOWN: usize = 200;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("sync.applyBooks", "Apply Downloaded Address Books", "Send / Receive", apply_books_command)
            .pure()
            .params(r#"{"download": BooksDownload} — the account's address books as a host brought them from its server: {"account": number, "email"?: string, "complete": bool, "books": [{"id": string, "name": string, "readOnly": bool, "mayRemove"?: bool, "main"?: bool, "plain"?: bool}]} — each is a book of that account here, with the account's rights there; one the server no longer lists goes when the list is complete, and its contacts are in no book until the contacts come down again. Answers {"added", "updated", "removed", "complete"}"#),
        CommandSpec::new("sync.contactChanges", "Contact Changes to Send", "Send / Receive", written::contact_changes_command)
            .pure()
            .params(r#"{"account": number} — what that account's server is to be told of contacts made, changed or deleted here in its address books: {"account", "email", "ops": [{"kind": "create"|"update"|"delete", "contact": number, "remote"?: string, "token"?: string, "book"?: string, "fields": {"name"?: {"first", "last", "display"?}, "company"?: string, "title"?: string, "emails"?: [{"label", "value"}], "phones"?: [{"label", "value"}]}}], "dropped": {"count": number, "reasons": [string], "messages": [string]}} — "dropped": the waiting changes that can't be sent any more and were given up just now (what they changed was put back), for a host to log and to tell the owner"#),
        CommandSpec::new("sync.applyContactChanges", "Apply Sent Contact Changes", "Send / Receive", written::apply_contact_changes_command)
            .pure()
            .params(r#"{"result": ContactsUploaded} — what a host heard from the server: {"account": number, "email"?: string, "outcomes": [{"op": ContactOp, "result": "done"|"gone"|"refused"|"denied"|"failed", "remote"?: string, "error"?: string, "retryAfter"?: seconds}]} — answers {"done", "refused", "failed", "denied", "waiting", "messages": [string], "dropped": {"count", "reasons": [string], "messages": [string]}}"#),
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
    s.books_listed = true;
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

/// Forget the address books of `account`'s server, and what waited to be told to it of their contacts, for an account
/// that is no longer followed: how many of either went. (Its contacts go with the rest of what came from that server.)
pub fn forget_books(mailbox: &mut Mailbox, account: Id) -> usize {
    let before = mailbox.address_books.len() + mailbox.contact_changes.len();
    mailbox.address_books.retain(|book| book.origin.account != account);
    mailbox.contact_changes.retain(|change| change.account != account);
    before - mailbox.address_books.len() - mailbox.contact_changes.len()
}

/// The book a contact is in, while the mailbox has the account whose server keeps it.
fn kept_in<'a>(mailbox: &'a Mailbox, contact: &Contact) -> Option<&'a AddressBook> {
    super::remote::keeper(mailbox, &contact.origin)?;
    mailbox.book_of(contact)
}

/// Whether the contacts of `book` may be changed here: the account may change them there, and the mailbox still has it.
pub fn writable(mailbox: &Mailbox, book: &AddressBook) -> bool {
    !book.read_only && super::remote::keeper(mailbox, &book.origin).is_some()
}

/// The book `contact` is in when it is one whose contacts may be changed here: what is changed of it goes to that
/// book's server.
pub fn writable_book_of<'a>(mailbox: &'a Mailbox, contact: &Contact) -> Option<&'a AddressBook> {
    kept_in(mailbox, contact).filter(|book| writable(mailbox, book))
}

/// The address book a new contact goes to when none is named ([`target_book`]): the main book of the account on
/// screen whose contacts may be changed, else another such book of it, else the same of the other accounts, in their
/// order. `None` when there is none: the contact is the mailbox's own.
pub fn default_book(s: &Session) -> Option<&AddressBook> {
    let mailbox = &s.mailbox;
    let on_screen = mailbox.folder(s.view.folder).map(|folder| folder.account);
    let order = on_screen.into_iter().chain(mailbox.accounts.iter().map(|account| account.id));
    order.into_iter().find_map(|account| {
        let books = || mailbox.address_books.iter().filter(move |book| book.origin.account == account && writable(mailbox, book));
        books().find(|book| book.main).or_else(|| books().next())
    })
}

/// Why `contact` may not be changed here for its address book's sake: the account may only read the book's contacts.
/// `None` when its book says nothing against it (or it is in none); what else may stand in the way is
/// [`super::remote::changeable`]'s to say.
pub fn change_refusal(mailbox: &Mailbox, contact: &Contact) -> Option<&'static str> {
    kept_in(mailbox, contact).filter(|book| book.read_only).map(|_| NO_CHANGING)
}

/// Why `contact` may not be deleted here for its address book's sake: the account may only read the book's contacts,
/// or may change them and not delete them. (Deleting is what is refused either way, and said so.)
pub fn delete_refusal(mailbox: &Mailbox, contact: &Contact) -> Option<&'static str> {
    let book = kept_in(mailbox, contact)?;
    (book.read_only || !book.may_remove).then_some(NO_DELETING)
}

/// Why a new contact can't be made: no address book of a server takes it ([`default_book`]) and the session keeps
/// nothing of its own between visits ([`Session::keeps_local`]), so it would be kept nowhere. The account's rights
/// when a server lists books it may only read; that no server lists one, when none does; that they are still being
/// read, before any server said ([`Session::books_listed`]).
pub fn adding_refusal(s: &Session) -> Option<&'static str> {
    if s.keeps_local || default_book(s).is_some() {
        return None;
    }
    let listed = s.mailbox.address_books.iter().any(|book| super::remote::keeper(&s.mailbox, &book.origin).is_some());
    Some(if listed {
        NO_ADDING
    } else if s.books_listed {
        NO_BOOKS
    } else {
        NO_BOOKS_YET
    })
}

/// The address book a new contact made with the params `v` goes to, by its id here; `None` for the mailbox's own
/// contacts. `book` names one (0 for the mailbox's own); without it, [`default_book`]. Refused, with the reason, for a
/// book whose contacts may not be changed here, and for the mailbox's own in a session that keeps nothing of its own.
pub fn target_book(s: &Session, v: &Value) -> Result<Option<Id>, CmdError> {
    let named = match v.get("book") {
        None | Some(Value::Null) => None,
        Some(_) => Some(p::u64(v, "book").ok_or_else(|| bad("`book` is the id of an address book, or 0 for the mailbox's own contacts"))?),
    };
    let target = match named {
        None => default_book(s).map(|book| book.id),
        Some(0) => None,
        Some(id) => {
            let book = s.mailbox.address_book(id).ok_or_else(|| fail(format!("no address book with id {id}")))?;
            if !writable(&s.mailbox, book) {
                return Err(fail(NO_ADDING));
            }
            Some(id)
        }
    };
    if target.is_none() && !s.keeps_local {
        return Err(fail(adding_refusal(s).unwrap_or(NO_ADDING)));
    }
    Ok(target)
}

/// What a server's card holds of a contact here is all that may be set of one in a server's address book: why not,
/// when `after` sets anything else differently from `before` (a postal address, a birthday, notes…), or a display name
/// of its own in a book whose server keeps only the basics (it names a contact by its first and last name).
pub fn unkept(book: &AddressBook, before: &Contact, after: &Contact) -> Option<&'static str> {
    let beyond = before.middle != after.middle
        || before.nickname != after.nickname
        || before.department != after.department
        || before.addresses != after.addresses
        || before.websites != after.websites
        || before.birthday != after.birthday
        || before.notes != after.notes
        || (book.plain && before.display_name != after.display_name);
    beyond.then_some(NO_KEEPING)
}

/// Make `contact` in the book `book` (its id here; `None`: the mailbox's own), noted for that book's server: the
/// contact's id. What the book would not keep is refused ([`unkept`]).
pub(crate) fn add_contact(s: &mut Session, book: Option<Id>, mut contact: Contact) -> Result<Id, CmdError> {
    let Some(book) = book.and_then(|id| s.mailbox.address_book(id)).cloned() else {
        return Ok(s.mailbox.add_contact(Contact { book: 0, origin: Origin::default(), ..contact }));
    };
    if let Some(why) = unkept(&book, &Contact::default(), &contact) {
        return Err(fail(why));
    }
    contact.book = book.id;
    contact.origin = Origin { account: book.origin.account, ..Default::default() };
    let id = s.mailbox.add_contact(contact);
    written::note_contact_created(s, id)?;
    Ok(id)
}

/// Put `after` in the place of the contact it is (by its id), and note for its book's server what changed. In a book of
/// a server, what the book would not keep is refused ([`unkept`]); and no contact changes its book by a change.
pub(crate) fn put_edited(s: &mut Session, after: Contact, v: &Value) -> Result<(), CmdError> {
    let before = s.mailbox.contact(after.id).cloned().ok_or_else(|| fail("the contact is gone"))?;
    if v.get("book").is_some_and(|book| !book.is_null()) && p::u64(v, "book") != Some(before.book) {
        return Err(fail(NO_MOVING));
    }
    if let Some(book) = writable_book_of(&s.mailbox, &before)
        && let Some(why) = unkept(book, &before, &after)
    {
        return Err(fail(why));
    }
    *s.mailbox.contact_mut(before.id)? = Contact { book: before.book, origin: before.origin.clone(), ..after };
    written::note_contact_changed(&mut s.mailbox, &before)
}

/// Note for its book's server that `before` (the contact as it was) was deleted here.
pub(crate) fn note_deleted(mailbox: &mut Mailbox, before: &Contact) -> Result<(), CmdError> {
    written::note_contact_deleted(mailbox, before)
}

/// Whether running `id` with `params` writes to a server's address book, which everyone who reads that book sees: a
/// contact made in one, or one of one changed or deleted. For an agent, that is the owner's to allow
/// ([`crate::agent::classify`]).
pub fn writes_to_a_book(s: &Session, id: &str, params: &Value) -> bool {
    match id {
        "people.new" | "people.fromMessage" => target_book(s, params).is_ok_and(|book| book.is_some()),
        "people.edit" | "people.delete" => {
            let named = p::id(params, "id").or(s.sel.contact);
            named.and_then(|id| s.mailbox.contact(id)).is_some_and(|contact| writable_book_of(&s.mailbox, contact).is_some())
        }
        _ => false,
    }
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
