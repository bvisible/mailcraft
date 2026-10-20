// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What is made, changed and deleted here of the contacts of a server's address books goes to that server, as what is
//! done to its events does ([`super::super::remote`]): the change is made here at once and noted
//! ([`ContactChange`], in [`Mailbox::contact_changes`]: [`note_contact_created`], [`note_contact_changed`],
//! [`note_contact_deleted`]); `sync.contactChanges` ([`contact_upload`]) says what the server is to be told, a host
//! carries it out, and `sync.applyContactChanges` ([`apply_contact_outcomes`]) takes in what the server said.
//!
//! - **What goes** is what a server's card holds of a contact ([`ContactFields`]): a new contact whole, a changed one
//!   as what differs from what the server has (the note's `base`), with what the parts it names were then (so that a
//!   host makes only that change to the server's card as it is now, and what someone else added there stays) and the
//!   server's mark of the contact (when it was made there: another contact a server gave the same name is never
//!   written to). Several changes before the server hears of any are one, and a change taken back is none. A new
//!   contact goes with a mark of its own (its card's `uid`): sent twice, it makes one card, and a download that has it
//!   before the answer came finds it again ([`before_merge`]).
//! - **Until the server was told**, a download leaves a contact whose change or deletion waits as it is here, and does
//!   not bring back one deleted here.
//! - **What the server said.** Done: a new contact has the server's name for it, and the note goes (what was changed
//!   since is noted afresh). Gone: a change has nothing to change there, and the contact goes here too; a deletion is
//!   done. Refused: a change is put back as the server has it, a deleted contact comes back, a new one is kept on this
//!   computer only (in a session that keeps nothing of its own, until the tab is closed: a contact the person made is
//!   never taken away because a server wouldn't have it); one sent again as a change of the card the server made of
//!   it the first time is that card, put back as it was sent then; each says so, once. Denied: tried once more, then
//!   refused. Failed: it waits, longer after every failure in a row; a new contact the server may have made (it may
//!   have run what it was sent; it is noted so as it goes, [`note_sending`]) keeps what it was sent with (what it was
//!   sent with this time, when nothing of it was there before), so that sending it again makes only what was changed
//!   since to that card, and one it refused to make, having made none of it, keeps nothing of the kind. A new contact,
//!   a change or a deletion the server has no room for (the account over its quota) waits however long it takes, and
//!   the owner is told once. One the server
//!   keeps failing with (a few times in a row, with this very change), or a new contact it keeps making without saying
//!   what it calls it (counted apart), is given up with a word, and the contact follows the server again (a new one is
//!   kept here, as a refused one is); another answer, or a new change here, starts the count again.
//! - **A copy.** A read that came before the answer to a new contact (one lost on the way) brings its card as a contact
//!   of its own. A note is that contact's own (a change of one of the two is never the other's), and deleting one of
//!   two contacts with one card deletes nothing there. A deletion waits for a new contact that may have been made on
//!   the server and is not due again ([`due_at`]): it may be of that one's card (after a few failures of it in a row,
//!   only one of a card with its name or an address of it waits). Once every answer of the job is in, a copy goes when
//!   nothing of it waits (what it changed went to the card), a deletion of it is not sent (it would delete the contact
//!   made here), and a contact made here whose card was deleted with its copy is made anew ([`apply_contact_outcomes`]).
//! - **An answer about a change that is no longer noted** (it was undone while it was on its way): the server did what
//!   the mailbox no longer says, so the opposite is noted; for a deletion, which can't be undone there, the contact goes
//!   here too, with a word.
//! - **Nothing is dropped without a word.** A note that no longer fits the mailbox (its contact is gone, its book may
//!   not be changed any more, another note about the same contact waits before it) is given up when the next upload is
//!   worked out, or when an answer is taken in ([`settle_contact_changes`]), and what it changed is put back (not a
//!   deletion, whose answer may have been lost after the server did it: the next read says).
//!
//! Like mail that arrives, taking an answer in is not an undo step; and once a server was told anything, the undo
//! history is forgotten, as it is for events.

use std::collections::HashSet;

use mailcraft_model::book::{
    CardMade, ContactChange, ContactChangeKind, ContactFields, ContactName, ContactOp, ContactOutcome, ContactsUpload, ContactsUploaded,
    MAX_CONTACT_CHANGES, MAX_CONTACT_OPS,
};
use mailcraft_model::remote::{ContactsDownload, EventResult, clean_key};
use mailcraft_model::{Contact, Id, Mailbox, Origin};
use mailcraft_time::Timestamp;
use serde_json::{Value, json};

use super::super::remote::{EventsWaiting, because, keeper, new_token, owner, wait_after};
use super::super::{bad, fail};
use super::{writable, writable_book_of};
use crate::{CmdError, CmdResult, Session, p};

/// Most outcomes taken from one answer, and most lines said about them.
const MAX_OUTCOMES: usize = 1_000;
const MAX_MESSAGES: usize = 20;
/// Longest text of a parameter error handed back.
const MAX_SHOWN: usize = 200;

/// Why a waiting change is given up ([`unfit`]).
const NO_ACCOUNT: &str = "its account is no longer in the mailbox";
const NO_CONTACT: &str = "the contact it is about is no longer in the contacts";
const NOT_ITS_OWN: &str = "it doesn't match the contact it names";
const NO_NAME: &str = "it doesn't say which contact it is about";
const STILL_HERE: &str = "the contact it deletes is still in the contacts";
const READ_ONLY: &str = "the contact's address book can't be changed any more";
const SAID_TWICE: &str = "another change about the same contact waits before it";
const TOO_MANY: &str = "too many changes were waiting";
/// And why one that was sent is: the server no longer has the contact; or it kept failing with it.
const GONE_THERE: &str = "the server no longer has the contact";
const STUCK: &str = "the server kept failing with it";
/// Why another change can't be noted.
const NO_ROOM: &str = "Too many contact changes are waiting to be sent to the server. Let Send/Receive get them through first.";

/// A change that waited for a server and was given up without being sent ([`settle_contact_changes`]), or one that the
/// server no longer has the contact for. What it changed here was put back to what the server has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactDropped {
    /// The account whose server the change was for.
    pub account: Id,
    pub kind: ContactChangeKind,
    /// Why, in words that say nothing of the contact: they go to the log.
    pub reason: &'static str,
    /// What the owner is told (it names the contact); empty when there is nothing to tell.
    pub message: String,
}

/// What a command answers of the changes that were given up: how many, why (each reason once), and what the owner is
/// told. Nothing of the contacts but their names, in the messages.
pub fn contact_dropped_json(dropped: &[ContactDropped]) -> Value {
    let mut reasons: Vec<String> = Vec::new();
    for one in dropped.iter().take(MAX_OUTCOMES) {
        let reason = format!("{:?}: {}", one.kind, one.reason);
        if reasons.len() < MAX_MESSAGES && !reasons.contains(&reason) {
            reasons.push(reason);
        }
    }
    let messages: Vec<&str> = dropped.iter().map(|one| one.message.as_str()).filter(|line| !line.is_empty()).take(MAX_MESSAGES).collect();
    json!({"count": dropped.len(), "reasons": reasons, "messages": messages})
}

/// A name for a line about a contact: short, and never empty.
fn called(contact: Option<&Contact>) -> String {
    let name: String = contact.map(Contact::name).unwrap_or_default().trim().chars().filter(|c| !c.is_control()).take(60).collect();
    if name.is_empty() { "a contact".into() } else { format!("\"{name}\"") }
}

/// The same, at the start of a sentence.
fn called_first(contact: Option<&Contact>) -> String {
    let name = called(contact);
    if name == "a contact" { "A contact".into() } else { name }
}

// ---- what is noted ----

/// Whether one more change to a contact may be noted: a mailbox that can't reach its server for a long time doesn't
/// pile them up without end.
fn room(mailbox: &Mailbox) -> Result<(), CmdError> {
    if mailbox.contact_changes.len() >= MAX_CONTACT_CHANGES { Err(fail(NO_ROOM)) } else { Ok(()) }
}

/// The contact `id` was made in an address book of a server (it names the book, and its account): the server is to be
/// told. Nothing to do for one of the mailbox's own; refused for a book whose contacts may not be changed.
pub(crate) fn note_contact_created(s: &mut Session, id: Id) -> Result<(), CmdError> {
    let Some(contact) = s.mailbox.contact(id) else { return Ok(()) };
    if contact.book == 0 {
        return Ok(());
    }
    let account = match writable_book_of(&s.mailbox, contact) {
        Some(book) => book.origin.account,
        None => return Err(fail(super::NO_ADDING)),
    };
    room(&s.mailbox)?;
    let token = new_token(id, s.rev() ^ s.now().unsigned_abs().rotate_left(32));
    let mailbox = &mut s.mailbox;
    let contact = mailbox.contact_mut(id)?;
    contact.origin = Origin { account, ..Default::default() };
    // (Its card's own mark, which an exported card keeps too.)
    if contact.uid.is_empty() {
        contact.uid.clone_from(&token);
    }
    mailbox.contact_changes.retain(|change| !(change.kind == ContactChangeKind::Create && change.contact == id));
    mailbox.contact_changes.push(ContactChange { account, kind: ContactChangeKind::Create, contact: id, token, ..Default::default() });
    Ok(())
}

/// A contact was changed here; `before` is what it was. When it is in a server's book, the server is to be told what
/// differs from its own copy: `before`, or what it was before an earlier change of this contact that is still waiting.
/// A change that takes everything back to that leaves nothing to tell. Something new to say doesn't wait out what the
/// last try earned, and starts the count of the server's failures with it again.
pub(crate) fn note_contact_changed(mailbox: &mut Mailbox, before: &Contact) -> Result<(), CmdError> {
    let Some(now) = mailbox.contact(before.id).cloned() else { return Ok(()) };
    if writable_book_of(mailbox, &now).is_none() {
        return Ok(());
    }
    let account = now.origin.account;
    // One the server doesn't have yet is sent as it is when it goes.
    if now.origin.id.is_empty() {
        let making = |change: &&mut ContactChange| change.kind == ContactChangeKind::Create && change.account == account && change.contact == now.id;
        if let Some(change) = mailbox.contact_changes.iter_mut().find(making) {
            (change.retry_at, change.stuck) = (0, 0);
        }
        return Ok(());
    }
    // (This contact's own: another here may have the same card, a copy a read brought of one made here.)
    let waiting = |change: &ContactChange| {
        change.kind == ContactChangeKind::Update && change.account == account && change.remote == now.origin.id && change.contact == now.id
    };
    match mailbox.contact_changes.iter().position(waiting) {
        Some(at) => {
            let back =
                mailbox.contact_changes.get(at).and_then(|change| change.base.as_deref()).is_some_and(|base| changed_fields(base, &now).is_empty());
            if back {
                mailbox.contact_changes.remove(at);
            } else if let Some(change) = mailbox.contact_changes.get_mut(at) {
                (change.retry_at, change.stuck) = (0, 0);
            }
        }
        None if changed_fields(before, &now).is_empty() => {}
        None => {
            room(mailbox)?;
            mailbox.contact_changes.push(ContactChange {
                account,
                kind: ContactChangeKind::Update,
                contact: now.id,
                remote: now.origin.id.clone(),
                base: Some(Box::new(before.clone())),
                ..Default::default()
            });
        }
    }
    Ok(())
}

/// A contact was deleted here; `before` is what it was. When it is in a server's book, the server is to delete it too;
/// one the server never had needs no telling (its note to make it goes), nor one whose card another contact here has
/// (a copy a read brought of one made here): the card stays, for that one, and only what waited of this one goes.
pub(crate) fn note_contact_deleted(mailbox: &mut Mailbox, before: &Contact) -> Result<(), CmdError> {
    if before.book == 0 || keeper(mailbox, &before.origin).is_none() {
        return Ok(());
    }
    let account = before.origin.account;
    if before.origin.id.is_empty() {
        mailbox.contact_changes.retain(|change| !(change.account == account && change.contact == before.id && change.remote.is_empty()));
        return Ok(());
    }
    let remote = before.origin.id.clone();
    let twin = mailbox.contacts.values().any(|c| c.id != before.id && c.origin.account == account && c.origin.id == remote);
    if twin {
        mailbox.contact_changes.retain(|change| !(change.account == account && change.contact == before.id));
        log::debug!("contacts: a contact whose card another here has was deleted here only");
        return Ok(());
    }
    // A change that was waiting is overtaken; what the server has is still what it was before it.
    let waiting = |change: &ContactChange| {
        change.kind == ContactChangeKind::Update && change.account == account && change.remote == remote && change.contact == before.id
    };
    let base = match mailbox.contact_changes.iter().position(waiting) {
        Some(at) => mailbox.contact_changes.remove(at).base,
        None => {
            room(mailbox)?;
            None
        }
    };
    let base = base.unwrap_or_else(|| Box::new(before.clone()));
    mailbox.contact_changes.retain(|change| !(change.account == account && change.remote == remote));
    mailbox.contact_changes.push(ContactChange {
        account,
        kind: ContactChangeKind::Delete,
        contact: before.id,
        remote,
        base: Some(base),
        ..Default::default()
    });
    Ok(())
}

// ---- what is sent ----

/// The name of a contact as a card holds it.
fn name_of(contact: &Contact) -> ContactName {
    ContactName { first: contact.first.trim().to_string(), last: contact.last.trim().to_string(), display: contact.display_name.trim().to_string() }
}

/// Everything a server's card holds of `contact`: what a new one is made with.
pub fn contact_fields(contact: &Contact) -> ContactFields {
    let mut fields = ContactFields {
        name: Some(name_of(contact)),
        company: Some(contact.company.trim().to_string()),
        title: Some(contact.title.trim().to_string()),
        emails: Some(contact.emails.clone()),
        phones: Some(contact.phones.clone()),
    };
    fields.tidy();
    fields
}

/// What a server's card holds of `now` that differs from `base`: what a change of a contact the server has tells it.
pub fn changed_fields(base: &Contact, now: &Contact) -> ContactFields {
    fn differs<T: PartialEq>(was: Option<T>, is: Option<T>) -> Option<T> {
        if was == is { None } else { is }
    }
    let (was, is) = (contact_fields(base), contact_fields(now));
    ContactFields {
        name: differs(was.name, is.name),
        company: differs(was.company, is.company),
        title: differs(was.title, is.title),
        emails: differs(was.emails, is.emails),
        phones: differs(was.phones, is.phones),
    }
}

/// `contact` with what `fields` say put into it: the contact as the server has it once it took them.
fn put_fields(contact: &mut Contact, fields: &ContactFields) {
    if let Some(name) = &fields.name {
        (contact.first, contact.last, contact.display_name) = (name.first.clone(), name.last.clone(), name.display.clone());
    }
    if let Some(company) = &fields.company {
        contact.company.clone_from(company);
    }
    if let Some(title) = &fields.title {
        contact.title.clone_from(title);
    }
    if let Some(emails) = &fields.emails {
        contact.emails.clone_from(emails);
    }
    if let Some(phones) = &fields.phones {
        contact.phones.clone_from(phones);
    }
}

/// Of `all`, the parts `named` names: what they were before the change that names them.
fn as_named(all: &ContactFields, named: &ContactFields) -> ContactFields {
    ContactFields {
        name: all.name.clone().filter(|_| named.name.is_some()),
        company: all.company.clone().filter(|_| named.company.is_some()),
        title: all.title.clone().filter(|_| named.title.is_some()),
        emails: all.emails.clone().filter(|_| named.emails.is_some()),
        phones: all.phones.clone().filter(|_| named.phones.is_some()),
    }
}

/// What the server calls the book `contact` is in, here; empty for none.
fn book_name(mailbox: &Mailbox, contact: &Contact) -> String {
    mailbox
        .address_book(contact.book)
        .filter(|book| book.origin.account == contact.origin.account)
        .map(|book| book.origin.id.clone())
        .unwrap_or_default()
}

/// How many failures in a row of a new contact that may have been made on its server (its answer didn't come) hold back
/// every deletion of its account; after them, only the deletion of a card that may be its own does.
const MAX_HOLDS: u32 = 3;

/// The new contacts of `account` that may have been made on its server (sent, and no answer said) and are not due
/// again yet: when each is due, how many times in a row it failed, and what it was sent with.
fn holders(mailbox: &Mailbox, account: Id, now: Timestamp) -> Vec<(Timestamp, u32, &Contact)> {
    mailbox
        .contact_changes
        .iter()
        .filter(|change| change.account == account && change.kind == ContactChangeKind::Create && change.retry_at > now)
        .filter_map(|change| change.base.as_deref().map(|sent| (change.retry_at, change.failures, sent)))
        .collect()
}

/// When a change is due. A deletion waits for each new contact of its account that may have been made on its server
/// and is not due again yet (`holders`): the card that one was made as may be the one deleted (a copy of it a read
/// brought), and the server is to say which first. After [`MAX_HOLDS`] failures of one in a row, only the deletion of
/// a card that may be its own (its name, or an address of it) waits for it.
fn due_at(change: &ContactChange, holders: &[(Timestamp, u32, &Contact)]) -> Timestamp {
    if change.kind != ContactChangeKind::Delete {
        return change.retry_at;
    }
    holders
        .iter()
        .filter(|(_, failures, sent)| *failures < MAX_HOLDS || may_be(sent, change.base.as_deref()))
        .map(|(at, ..)| *at)
        .fold(change.retry_at, Timestamp::max)
}

/// Whether the card a deletion is of (`deleted`: the contact as the server had it) may be the one a new contact was
/// made as (`sent`: what it was sent with): the same name or company, or an address or a number of both (its digits).
/// Unknown: it may be.
fn may_be(sent: &Contact, deleted: Option<&Contact>) -> bool {
    let Some(deleted) = deleted else { return true };
    let same = |a: &str, b: &str| !a.trim().is_empty() && a.trim().eq_ignore_ascii_case(b.trim());
    let digits = |number: &str| number.chars().filter(char::is_ascii_digit).collect::<String>();
    let same_number = |a: &str, b: &str| {
        let a = digits(a);
        !a.is_empty() && a == digits(b)
    };
    same(&sent.name(), &deleted.name())
        || same(&sent.company, &deleted.company)
        || sent.emails.iter().any(|a| deleted.emails.iter().any(|b| same(&a.value, &b.value)))
        || sent.phones.iter().any(|a| deleted.phones.iter().any(|b| same_number(&a.value, &b.value)))
}

/// How much of an account's contacts waits to go to its server: due now, all of them, and when the next held back is
/// due ([`EventsWaiting`], as for its events).
pub fn contacts_waiting(s: &Session, account: Id) -> EventsWaiting {
    let mut waiting = EventsWaiting::default();
    if !s.mailbox.contact_changes.iter().any(|change| change.account == account) {
        return waiting;
    }
    let (now, here) = (s.now(), contacts_here(&s.mailbox));
    let holders = holders(&s.mailbox, account, now);
    for change in s.mailbox.contact_changes.iter().filter(|change| change.account == account && unfit(&s.mailbox, change, &here).is_none()) {
        waiting.total += 1;
        let at = due_at(change, &holders);
        if at <= now {
            waiting.due += 1;
        } else {
            waiting.next = Some(waiting.next.map_or(at, |next| next.min(at)));
        }
    }
    waiting
}

/// What the server of `account` is to be told now of the contacts made, changed and deleted here in its books: the
/// changes that are due, oldest first, [`MAX_CONTACT_OPS`] at most. A deletion waits for a new contact that may have
/// been made on the server ([`due_at`]; a host sends the new ones first, and holds the deletions back while one has no
/// answer that names its card).
pub fn contact_upload(s: &Session, account: Id) -> Result<ContactsUpload, CmdError> {
    let owner = s.mailbox.account(account).ok_or_else(|| fail(format!("no account with id {account}")))?;
    let (mailbox, now) = (&s.mailbox, s.now());
    let here = contacts_here(mailbox);
    let holders = holders(mailbox, account, now);
    let due = |change: &&ContactChange| change.account == account && due_at(change, &holders) <= now;
    let mut ops = Vec::new();
    for change in mailbox.contact_changes.iter().filter(due) {
        if ops.len() >= MAX_CONTACT_OPS {
            break;
        }
        if unfit(mailbox, change, &here).is_some() {
            continue;
        }
        let contact = mailbox.contact(change.contact);
        let base = change.base.as_deref();
        let (fields, book, was) = match (change.kind, contact, base) {
            // (What it was sent with before, when it was: the card the server may have made of it has that.)
            (ContactChangeKind::Create, Some(contact), base) => (contact_fields(contact), book_name(mailbox, contact), base.map(contact_fields)),
            (ContactChangeKind::Update, Some(contact), Some(base)) => {
                let fields = changed_fields(base, contact);
                let was = as_named(&contact_fields(base), &fields);
                (fields, book_name(mailbox, contact), Some(was))
            }
            (ContactChangeKind::Delete, _, base) => (ContactFields::default(), base.map(|base| book_name(mailbox, base)).unwrap_or_default(), None),
            _ => continue,
        };
        // Changed and changed back: nothing to say.
        if change.kind == ContactChangeKind::Update && fields.is_empty() {
            continue;
        }
        // The server's mark of the contact the change is about, as the last read had it.
        let mark = if change.kind == ContactChangeKind::Create {
            String::new()
        } else {
            base.map(|base| base.origin.change_key.clone()).unwrap_or_default()
        };
        ops.push(ContactOp {
            kind: change.kind,
            contact: change.contact,
            remote: change.remote.clone(),
            token: change.token.clone(),
            book,
            fields,
            base: was,
            mark,
        });
    }
    Ok(ContactsUpload { ops, ..Default::default() }.for_account(owner))
}

/// The changes of `upload` are on their way to the server: a new contact among them may be made there from now on (its
/// answer lost, or this computer stopped before it came), so it is noted with what it is sent with, as one that may be
/// there: the next try looks for its card first ([`ContactChange::base`]). What the answer says settles it.
pub fn note_sending(s: &mut Session, upload: &ContactsUpload) {
    let mut noted = false;
    for op in upload.ops.iter().filter(|op| op.kind == ContactChangeKind::Create && op.base.is_none()) {
        let Some(mut sent) = s.mailbox.contact(op.contact).cloned() else { continue };
        put_fields(&mut sent, &op.fields);
        let making = |change: &&mut ContactChange| {
            change.kind == ContactChangeKind::Create && change.account == upload.account && change.token == op.token && change.base.is_none()
        };
        if let Some(change) = s.mailbox.contact_changes.iter_mut().find(making) {
            change.base = Some(Box::new(sent));
            noted = true;
        }
    }
    if noted {
        s.mailbox_changed();
    }
}

// ---- what no longer fits ----

/// What the mailbox has of each server's contacts, by account and by what the server calls them: for [`unfit`]. Nothing
/// is listed when no deletion waits (nothing would ask).
fn contacts_here(mailbox: &Mailbox) -> HashSet<(Id, &str)> {
    if !mailbox.contact_changes.iter().any(|change| change.kind == ContactChangeKind::Delete) {
        return HashSet::new();
    }
    mailbox.contacts.values().filter(|c| !c.origin.id.is_empty()).map(|c| (c.origin.account, c.origin.id.as_str())).collect()
}

/// Why a noted change no longer hangs together with the mailbox, when it doesn't: only one that does is sent (a file
/// from elsewhere could otherwise have the server told anything).
fn unfit(mailbox: &Mailbox, change: &ContactChange, here: &HashSet<(Id, &str)>) -> Option<&'static str> {
    if mailbox.account(change.account).is_none() {
        return Some(NO_ACCOUNT);
    }
    let named = |name: &str| !name.is_empty() && clean_key(name) == name;
    let of_account = |contact: &&Contact| contact.origin.account == change.account;
    let changeable = |contact: &Contact| writable_book_of(mailbox, contact).is_some();
    match change.kind {
        ContactChangeKind::Create if change.token.is_empty() => Some(NO_NAME),
        ContactChangeKind::Create => match mailbox.contact(change.contact).filter(of_account) {
            None => Some(NO_CONTACT),
            Some(contact) if !contact.origin.id.is_empty() => Some(NOT_ITS_OWN),
            Some(contact) if !changeable(contact) => Some(READ_ONLY),
            Some(_) => None,
        },
        ContactChangeKind::Update if !named(&change.remote) || change.base.is_none() => Some(NO_NAME),
        ContactChangeKind::Update => match mailbox.contact(change.contact).filter(of_account) {
            None => Some(NO_CONTACT),
            Some(contact) if contact.origin.id != change.remote => Some(NOT_ITS_OWN),
            Some(contact) if !changeable(contact) => Some(READ_ONLY),
            Some(_) => None,
        },
        ContactChangeKind::Delete if !named(&change.remote) => Some(NO_NAME),
        ContactChangeKind::Delete => {
            // The contact it deletes is one that was here, and isn't any more; and its book still lets it be deleted.
            let base = change.base.as_deref();
            let was_here = base.is_none_or(|base| base.origin.id == change.remote && base.origin.account == change.account);
            let deletable = base.is_none_or(|base| {
                mailbox
                    .address_book(base.book)
                    .is_some_and(|book| book.origin.account == change.account && writable(mailbox, book) && book.may_remove)
            });
            if !was_here {
                Some(NOT_ITS_OWN)
            } else if here.contains(&(change.account, change.remote.as_str())) {
                Some(STILL_HERE)
            } else if !deletable {
                Some(READ_ONLY)
            } else {
                None
            }
        }
    }
}

/// Whether a noted change has nothing to tell the server any more: the contact is, in everything a server is told, as
/// the server has it.
fn says_nothing(mailbox: &Mailbox, change: &ContactChange) -> bool {
    change.kind == ContactChangeKind::Update
        && match (change.base.as_deref(), mailbox.contact(change.contact)) {
            (Some(base), Some(contact)) => changed_fields(base, contact).is_empty(),
            _ => false,
        }
}

/// Give up the noted changes that no longer hang together with the mailbox, and all but the first about any one
/// contact; none without a word ([`give_up`]). `keeps_local`: whether a contact that can't be on its server stays
/// here, as one of the mailbox's own.
fn tidy_contact_changes(mailbox: &mut Mailbox, keeps_local: bool) -> Vec<ContactDropped> {
    if mailbox.contact_changes.is_empty() {
        return Vec::new();
    }
    /// A new contact is known by its id here, a deletion by what the server calls it, a change by both (a copy of a
    /// card made here, which a read brought before the answer, may wait with a change of its own).
    fn about(change: &ContactChange) -> (Id, &str, Id) {
        match change.kind {
            ContactChangeKind::Create => (change.account, "", change.contact),
            ContactChangeKind::Update => (change.account, change.remote.as_str(), change.contact),
            ContactChangeKind::Delete => (change.account, change.remote.as_str(), 0),
        }
    }
    let mut kept: Vec<ContactChange> = Vec::new();
    let mut given_up: Vec<(ContactChange, &'static str)> = Vec::new();
    let mut said_nothing = 0usize;
    {
        let here = contacts_here(mailbox);
        let mut seen: HashSet<(Id, &str, Id)> = HashSet::new();
        for change in &mailbox.contact_changes {
            let why = unfit(mailbox, change, &here).or_else(|| (!seen.insert(about(change))).then_some(SAID_TWICE));
            match why {
                Some(reason) => given_up.push((change.clone(), reason)),
                None if says_nothing(mailbox, change) => said_nothing += 1,
                None if kept.len() >= MAX_CONTACT_CHANGES => given_up.push((change.clone(), TOO_MANY)),
                None => kept.push(change.clone()),
            }
        }
    }
    if kept.len() == mailbox.contact_changes.len() {
        return Vec::new();
    }
    if said_nothing > 0 {
        log::debug!("contacts: {said_nothing} waiting changes had nothing left to send");
    }
    mailbox.contact_changes = kept;
    given_up.iter().take(MAX_OUTCOMES).map(|(change, reason)| give_up(mailbox, change, reason, keeps_local)).collect()
}

/// Where a contact that can't be on its server is kept, said at the end of a line: on this computer, or (a session that
/// keeps nothing of its own) here until the tab is closed.
const KEPT_HERE: &str = " It is kept on this computer only.";
const KEPT_FOR_NOW: &str = " It stays here until this tab is closed: copy what you need of it.";

/// Take a contact that can't be on its server out of its book and keep it as one of the mailbox's own: a contact the
/// person made is never taken away because its server wouldn't have it (in a session that keeps nothing of its own, it
/// stays until the tab is closed). Where it is kept, for the owner; nothing when it is no longer here.
fn keep_here(mailbox: &mut Mailbox, id: Id, keeps_local: bool) -> &'static str {
    match mailbox.contact_mut(id) {
        Ok(contact) => {
            contact.book = 0;
            contact.origin = Origin::default();
            if keeps_local { KEPT_HERE } else { KEPT_FOR_NOW }
        }
        Err(_) => "",
    }
}

/// Put a contact back as `base` says the server has it, under its id here; what is kept here only (its categories,
/// whether it is a favourite) stays as it is now.
fn put_back(mailbox: &mut Mailbox, id: Id, base: &Contact) {
    match mailbox.contact_mut(id) {
        Ok(contact) => {
            let (categories, favorite) = (std::mem::take(&mut contact.categories), contact.favorite);
            *contact = Contact { id, categories, favorite, ..base.clone() };
        }
        // (One that was deleted here comes back under the id it had when that is still free.)
        Err(_) => {
            mailbox.add_contact(Contact { id, ..base.clone() });
        }
    }
}

/// Give a waiting change up for `reason`: what it changed here is put back to what the server has, where that can be
/// told, and what became of it is said, for the owner (who is named the contact) and for the log (which is not).
fn give_up(mailbox: &mut Mailbox, change: &ContactChange, reason: &'static str, keeps_local: bool) -> ContactDropped {
    let here = mailbox.contact(change.contact).filter(|contact| contact.origin.account == change.account).cloned();
    let name = called_first(here.as_ref().or(change.base.as_deref()));
    let message = match (reason, change.kind) {
        // (The first of them still goes; an account that was removed took its contacts along.)
        (SAID_TWICE | NO_ACCOUNT, _) => String::new(),
        (READ_ONLY, ContactChangeKind::Update) => {
            if let (Some(base), Some(contact)) = (change.base.as_deref(), here.as_ref())
                && base.origin.id == contact.origin.id
            {
                put_back(mailbox, contact.id, base);
            }
            format!("{name}: its address book can't be changed any more, so the change made here was undone.")
        }
        (READ_ONLY, ContactChangeKind::Create) => {
            let kept = keep_here(mailbox, change.contact, keeps_local);
            format!("{name} was not sent to the server: its address book can't be changed any more.{kept}")
        }
        // (Not put back here: a deletion whose answer was lost may have been done there. The next read brings it back
        // if the server still has it.)
        (READ_ONLY, ContactChangeKind::Delete) => {
            format!(
                "{name}: its address book can't be changed any more, so it was not deleted there. It comes back with the next read if the server still has it."
            )
        }
        (NO_CONTACT, _) => format!("A change to {} was not sent: it is no longer in the contacts.", called(change.base.as_deref())),
        _ => "A contact change that was waiting to be sent no longer matched the contacts. It was not sent.".to_string(),
    };
    log::warn!("contacts: a waiting change was given up without being sent ({:?}): {reason}", change.kind);
    ContactDropped { account: change.account, kind: change.kind, reason, message }
}

/// Give up the waiting changes that can't be sent any more ([`tidy_contact_changes`]), outside of an answer or a
/// download: before a host is told what to send. What was given up, for the host to tell the owner and to log.
pub fn settle_contact_changes(s: &mut Session) -> Vec<ContactDropped> {
    let before = s.mailbox.contact_changes.len();
    let keeps_local = s.keeps_local;
    let dropped = tidy_contact_changes(&mut s.mailbox, keeps_local);
    if s.mailbox.contact_changes.len() != before || !dropped.is_empty() {
        s.mailbox_changed();
    }
    dropped
}

// ---- what the server said ----

/// How many times in a row the server may fail with one change in a way asking again is unlikely to get past
/// ([`ContactOutcome::lasting`]) before it is given up, with a word; and how many times it may say it made a new
/// contact without saying what it calls it.
const MAX_STUCK: u32 = 3;
const MAX_NAMELESS: u32 = 2;

/// What taking in the answers to a [`ContactsUpload`] came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactsApplied {
    /// Changes the server took on (or that needed no more telling).
    pub done: usize,
    /// Changes the server refused for good: put back as they were, or kept here only.
    pub refused: usize,
    /// Changes that couldn't be sent and wait to be tried again.
    pub failed: usize,
    /// The account may not change its contacts on the server: its owner is to be asked.
    pub denied: bool,
    /// What still waits, after this.
    pub waiting: usize,
    /// What the owner should hear, a line each.
    pub messages: Vec<String>,
    /// Changes given up without the server taking them, or about a contact the server no longer has.
    pub dropped: Vec<ContactDropped>,
}

impl ContactsApplied {
    fn say(&mut self, line: String) {
        if self.messages.len() < MAX_MESSAGES && !line.is_empty() && !self.messages.contains(&line) {
            self.messages.push(line);
        }
    }

    fn json(&self) -> Value {
        json!({
            "done": self.done,
            "refused": self.refused,
            "failed": self.failed,
            "denied": self.denied,
            "waiting": self.waiting,
            "messages": self.messages,
            "dropped": contact_dropped_json(&self.dropped),
        })
    }
}

/// What a change that couldn't be sent, or may not be, comes to ([`held_back`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    /// It waits, longer after each failure.
    Waits,
    /// The account was denied it a second time: refused.
    Denied,
    /// The server failed with it too many times in a row: given up.
    Stuck,
}

/// Note that a change couldn't be sent, or may not be: it waits, longer after each failure ([`Held`]).
fn held_back(change: &mut ContactChange, outcome: &ContactOutcome, now: Timestamp) -> Held {
    if outcome.result == EventResult::Denied {
        if change.denied {
            return Held::Denied;
        }
        change.denied = true;
    }
    change.failures = change.failures.saturating_add(1);
    change.retry_at = now.saturating_add(wait_after(change.failures, outcome.retry_after));
    change.error.clone_from(&outcome.error);
    // (In a row: another answer starts the count again.)
    change.nameless = 0;
    if !outcome.lasting {
        change.stuck = 0;
        return Held::Waits;
    }
    change.stuck = change.stuck.saturating_add(1);
    if change.stuck >= MAX_STUCK { Held::Stuck } else { Held::Waits }
}

/// What the answers are taken in with.
struct Taking<'a> {
    mailbox: &'a mut Mailbox,
    account: Id,
    now: Timestamp,
    keeps_local: bool,
    applied: ContactsApplied,
    /// The server was told something: the undo history is not what it has any more.
    told: bool,
    /// For a new mark: what is mixed into it.
    salt: u64,
    /// The contacts made here that the server said it made, with what it calls them; and what it calls the cards whose
    /// change it took: a copy of the same card a read brought before an answer is settled once every answer was taken
    /// in ([`Taking::settle_copies`]).
    made: Vec<(Id, String)>,
    changed: Vec<String>,
    /// What the server calls the cards it deleted in this answer.
    destroyed: HashSet<String>,
}

/// Take in what a server said to a [`ContactsUpload`] ([`ContactsUploaded`]): see the module.
pub fn apply_contact_outcomes(s: &mut Session, uploaded: &ContactsUploaded) -> Result<ContactsApplied, CmdError> {
    let account = owner(&s.mailbox, uploaded.account, &uploaded.email)?.id;
    let (now, keeps_local, salt) = (s.now(), s.keeps_local, s.rev());
    let (changes, contacts) = (s.mailbox.contact_changes.clone(), s.mailbox.contacts.clone());
    let mut taking = Taking {
        mailbox: &mut s.mailbox,
        account,
        now,
        keeps_local,
        applied: ContactsApplied::default(),
        told: false,
        salt,
        made: Vec::new(),
        changed: Vec::new(),
        destroyed: HashSet::new(),
    };
    for outcome in uploaded.outcomes.iter().take(MAX_OUTCOMES) {
        let mut outcome = outcome.clone();
        outcome.tidy();
        match outcome.op.kind {
            ContactChangeKind::Create => taking.created(&outcome),
            ContactChangeKind::Update => taking.updated(&outcome),
            ContactChangeKind::Delete => taking.deleted(&outcome),
        }
    }
    // Only once every answer is in: one about a copy of a contact made here may be among them.
    taking.settle_copies();
    let Taking { mut applied, told, .. } = taking;
    for dropped in tidy_contact_changes(&mut s.mailbox, keeps_local) {
        applied.say(dropped.message.clone());
        applied.dropped.push(dropped);
    }
    applied.waiting = s.mailbox.contact_changes.iter().filter(|change| change.account == account).count();
    if told {
        s.forget_history();
    }
    if told || s.mailbox.contact_changes != changes || s.mailbox.contacts != contacts {
        s.mailbox_changed();
    }
    Ok(applied)
}

impl Taking<'_> {
    /// The contact here the server calls `remote`, when one alone is.
    fn by_remote(&self, remote: &str) -> Option<Id> {
        let mut found = self.mailbox.contacts.values().filter(|c| c.origin.account == self.account && c.origin.id == remote).map(|c| c.id);
        let first = found.next();
        if found.next().is_some() { None } else { first }
    }

    /// The contact a note at `at` is about, while it is here; else the one the server calls `remote`.
    fn contact_of(&self, at: Option<usize>, remote: &str) -> Option<Id> {
        let noted = at.and_then(|at| self.mailbox.contact_changes.get(at)).map(|change| change.contact);
        noted.filter(|id| self.mailbox.contact(*id).is_some_and(|c| c.origin.id == remote)).or_else(|| self.by_remote(remote))
    }

    /// The server failed with the change at `at` too many times in a row ([`MAX_STUCK`]): it is given up, what it
    /// changed here put back where that can be told, and the owner told, once. The contact follows the server again.
    fn give_up_stuck(&mut self, at: usize, outcome: &ContactOutcome) {
        if at >= self.mailbox.contact_changes.len() {
            return;
        }
        let change = self.mailbox.contact_changes.remove(at);
        self.applied.refused += 1;
        let why = because(&outcome.error);
        let message = match change.kind {
            ContactChangeKind::Create => {
                let name = called_first(self.mailbox.contact(change.contact));
                let kept = keep_here(self.mailbox, change.contact, self.keeps_local);
                // (One whose answer didn't come may be there all the same.)
                let there = if change.base.is_some() {
                    " The server may have it already: then the next read brings it too, as a contact of its own."
                } else {
                    ""
                };
                format!("{name} could not be sent to the server, which keeps failing with it{why}.{kept}{there}")
            }
            ContactChangeKind::Update => {
                let name = called(self.mailbox.contact(change.contact).or(change.base.as_deref()));
                if let Some(base) = change.base.as_deref()
                    && self.mailbox.contact(change.contact).is_some_and(|c| c.origin.id == change.remote)
                {
                    put_back(self.mailbox, change.contact, base);
                }
                format!("The change to {name} could not be sent: the server keeps failing with it{why}. It was put back as the server has it.")
            }
            ContactChangeKind::Delete => {
                let name = called(change.base.as_deref());
                format!(
                    "Deleting {name} could not be sent: the server keeps failing with it{why}. It comes back with the next read if the server still has it."
                )
            }
        };
        log::warn!("contacts: a change the server kept failing with was given up ({:?})", change.kind);
        self.applied.say(message.clone());
        self.applied.dropped.push(ContactDropped { account: self.account, kind: change.kind, reason: STUCK, message });
    }

    /// Note whether the server has no room for the change at `at` (`outcome`): whether that is new, for the owner to be
    /// told once while it lasts.
    fn first_full(&mut self, at: usize, outcome: &ContactOutcome) -> bool {
        let Some(change) = self.mailbox.contact_changes.get_mut(at) else { return false };
        let first = outcome.full && !change.full;
        change.full = outcome.full;
        first
    }

    /// The contact `contact` as it was sent to be made (`op`): what the card made of it has.
    fn sent_as(&self, contact: Option<Id>, op: &ContactOp) -> Option<Box<Contact>> {
        contact.and_then(|id| self.mailbox.contact(id)).map(|here| {
            let mut sent = here.clone();
            put_fields(&mut sent, &op.fields);
            Box::new(sent)
        })
    }

    /// The answer to making a contact.
    fn created(&mut self, outcome: &ContactOutcome) {
        let (op, account, now) = (&outcome.op, self.account, self.now);
        let noted = |change: &ContactChange| {
            change.kind == ContactChangeKind::Create && change.account == account && !op.token.is_empty() && change.token == op.token
        };
        let at = self.mailbox.contact_changes.iter().position(noted);
        let contact =
            at.and_then(|at| self.mailbox.contact_changes.get(at)).map(|change| change.contact).filter(|id| self.mailbox.contact(*id).is_some());
        match outcome.result {
            EventResult::Done if outcome.remote.is_empty() => {
                // Made, but under what name wasn't said: it goes again (with its mark, which makes no second one, and
                // with what it was sent with, which the card made of it has). A server that says so again is not asked
                // for ever: the contact is kept here, and what the server made comes with the next read.
                let Some(at) = at else { return };
                let sent = self.sent_as(contact, op);
                let nameless = self.mailbox.contact_changes.get_mut(at).map(|change| {
                    change.base = sent;
                    change.failures = change.failures.saturating_add(1);
                    (change.nameless, change.stuck) = (change.nameless.saturating_add(1), 0);
                    change.retry_at = now.saturating_add(wait_after(change.failures, None));
                    change.nameless
                });
                if nameless.is_some_and(|n| n >= MAX_NAMELESS) {
                    self.mailbox.contact_changes.remove(at);
                    let name = called_first(contact.and_then(|id| self.mailbox.contact(id)));
                    if let Some(id) = contact {
                        keep_here(self.mailbox, id, self.keeps_local);
                    }
                    self.applied.say(format!(
                        "{name} was made on the server, which doesn't say what it calls it: it is kept on this computer, and the next read brings the server's card too."
                    ));
                }
                self.applied.failed += 1;
            }
            EventResult::Done => {
                self.told = true;
                self.applied.done += 1;
                let (Some(at), Some(id)) = (at, contact) else {
                    // Undone, or deleted again, while it was on its way: the server is to delete it.
                    let about = |change: &ContactChange| change.account == account && change.remote == outcome.remote;
                    let here = self.mailbox.contacts.values().any(|c| c.origin.account == account && c.origin.id == outcome.remote);
                    if !self.mailbox.contact_changes.iter().any(about) && !here {
                        self.mailbox.contact_changes.push(ContactChange {
                            account,
                            kind: ContactChangeKind::Delete,
                            remote: outcome.remote.clone(),
                            ..Default::default()
                        });
                    }
                    return;
                };
                let Ok(contact) = self.mailbox.contact_mut(id) else { return };
                contact.origin.id.clone_from(&outcome.remote);
                // What the server has is what was sent; what was changed here since is still to go.
                let mut sent = contact.clone();
                put_fields(&mut sent, &op.fields);
                let more = !changed_fields(&sent, contact).is_empty();
                self.mailbox.contact_changes.remove(at);
                if more {
                    self.mailbox.contact_changes.push(ContactChange {
                        account,
                        kind: ContactChangeKind::Update,
                        contact: id,
                        remote: outcome.remote.clone(),
                        base: Some(Box::new(sent)),
                        ..Default::default()
                    });
                }
                self.made.push((id, outcome.remote.clone()));
            }
            EventResult::Failed => {
                let Some(at) = at else { return };
                // The server may have made it (it was sent, and no answer came that says): what was sent the first
                // time is kept, so that sending it again makes only what was changed here since to the card made of
                // it. One it refused to make, and made none of before, may not be anywhere there: nothing of it is.
                let sent = self.sent_as(contact, op);
                // (Nothing of it was on the server before this try: it was never sent before, or its look found none.
                // A card made of it now has what was sent this time.)
                let fresh = op.base.is_none() || outcome.none_before;
                let first = self.first_full(at, outcome);
                let held = self.mailbox.contact_changes.get_mut(at).map(|change| {
                    match outcome.made {
                        CardMade::Perhaps if fresh || change.base.is_none() => change.base = sent,
                        CardMade::No => change.base = None,
                        CardMade::Unsaid if fresh => change.base = None,
                        _ => {}
                    }
                    held_back(change, outcome, now)
                });
                // No room for it on the server: said once while it lasts.
                if first {
                    let name = called(contact.and_then(|id| self.mailbox.contact(id)));
                    self.applied.say(format!("The server's mailbox is full: {name} waits here and goes as soon as there is room."));
                }
                self.applied.failed += 1;
                if held == Some(Held::Stuck) {
                    self.applied.failed -= 1;
                    self.give_up_stuck(at, outcome);
                }
            }
            EventResult::Denied | EventResult::Refused | EventResult::Gone => {
                let Some(at) = at else { return };
                let held = match self.mailbox.contact_changes.get_mut(at) {
                    Some(change) if outcome.result == EventResult::Denied => held_back(change, outcome, now),
                    _ => Held::Denied,
                };
                if held == Held::Waits {
                    self.applied.denied = true;
                    self.applied.failed += 1;
                    return;
                }
                let change = self.mailbox.contact_changes.remove(at);
                self.applied.refused += 1;
                // Sent again as a change of the card the server made of it the first time: it is that card, as the
                // server has it (what it was sent with then), not one of this computer's alone.
                if !outcome.remote.is_empty()
                    && let Some(id) = contact
                {
                    let name = called(self.mailbox.contact(id));
                    if let Some(base) = change.base.as_deref() {
                        put_back(self.mailbox, id, base);
                    }
                    if let Ok(here) = self.mailbox.contact_mut(id) {
                        here.origin = Origin { account, id: outcome.remote.clone(), ..Default::default() };
                    }
                    self.made.push((id, outcome.remote.clone()));
                    self.applied.say(format!("The server wouldn't take the change to {name}{}. It was put back as it was.", because(&outcome.error)));
                    return;
                }
                // What was written is not thrown away where it can be kept.
                let name = called(contact.and_then(|id| self.mailbox.contact(id)));
                let kept = contact.map(|id| keep_here(self.mailbox, id, self.keeps_local)).unwrap_or_default();
                self.applied.say(format!("The server wouldn't take {name}{}.{kept}", because(&outcome.error)));
            }
        }
    }

    /// The answer to changing a contact.
    fn updated(&mut self, outcome: &ContactOutcome) {
        let (op, account, now) = (&outcome.op, self.account, self.now);
        if op.remote.is_empty() {
            return;
        }
        let about = |change: &ContactChange| {
            change.account == account && change.remote == op.remote && change.kind != ContactChangeKind::Create && change.contact == op.contact
        };
        let at = self.mailbox.contact_changes.iter().position(about);
        let contact = self.contact_of(at, &op.remote);
        match outcome.result {
            EventResult::Done => {
                self.told = true;
                self.applied.done += 1;
                self.changed.push(op.remote.clone());
                let here = contact.and_then(|id| self.mailbox.contact(id)).cloned();
                match at {
                    // The server has what was sent now; what was changed here since is still to go (and what is to be
                    // put back, should a deletion be refused, is this).
                    Some(at) => {
                        let mut settled = false;
                        if let Some(change) = self.mailbox.contact_changes.get_mut(at) {
                            if let Some(base) = change.base.as_deref_mut() {
                                put_fields(base, &op.fields);
                            }
                            change.failures = 0;
                            change.stuck = 0;
                            change.retry_at = 0;
                            change.denied = false;
                            change.error.clear();
                            settled = change.kind == ContactChangeKind::Update
                                && match (change.base.as_deref(), here.as_ref()) {
                                    (Some(base), Some(contact)) => changed_fields(base, contact).is_empty(),
                                    _ => true,
                                };
                        }
                        if settled {
                            self.mailbox.contact_changes.remove(at);
                        }
                    }
                    // Undone while it was on its way: the server has the change and the mailbox doesn't, so what the
                    // mailbox says is to be sent after it.
                    None => {
                        let Some(contact) = here else { return };
                        let mut sent = contact.clone();
                        put_fields(&mut sent, &op.fields);
                        if !changed_fields(&sent, &contact).is_empty() && self.mailbox.contact_changes.len() < MAX_CONTACT_CHANGES {
                            self.mailbox.contact_changes.push(ContactChange {
                                account,
                                kind: ContactChangeKind::Update,
                                contact: contact.id,
                                remote: contact.origin.id.clone(),
                                base: Some(Box::new(sent)),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            EventResult::Gone => {
                // The server no longer has it (or has another contact under its name now): there is nothing to change,
                // here either.
                let Some(at) = at else { return };
                let deleting = self.mailbox.contact_changes.get(at).is_some_and(|change| change.kind == ContactChangeKind::Delete);
                self.mailbox.contact_changes.remove(at);
                self.applied.done += 1;
                if !deleting && let Some(id) = contact {
                    let name = called_first(self.mailbox.contact(id));
                    let _ = self.mailbox.remove_contact(id);
                    self.applied.refused += 1;
                    let message = format!("{name} is no longer on the server, so it couldn't be changed there. It was removed here too.");
                    log::warn!("contacts: a change was not taken: {GONE_THERE}");
                    self.applied.say(message.clone());
                    self.applied.dropped.push(ContactDropped { account, kind: ContactChangeKind::Update, reason: GONE_THERE, message });
                }
            }
            EventResult::Failed => {
                let Some(at) = at else { return };
                let first = self.first_full(at, outcome);
                let held = self
                    .mailbox
                    .contact_changes
                    .get_mut(at)
                    .filter(|change| change.kind == ContactChangeKind::Update)
                    .map(|change| held_back(change, outcome, now));
                if held.is_some() {
                    self.applied.failed += 1;
                }
                if first && held.is_some() {
                    let name = called(contact.and_then(|id| self.mailbox.contact(id)));
                    self.applied.say(format!("The server's mailbox is full: the change to {name} waits here and goes as soon as there is room."));
                }
                if held == Some(Held::Stuck) {
                    self.applied.failed -= 1;
                    self.give_up_stuck(at, outcome);
                }
            }
            EventResult::Denied | EventResult::Refused => {
                let Some(at) = at else { return };
                let Some(change) = self.mailbox.contact_changes.get_mut(at).filter(|change| change.kind == ContactChangeKind::Update) else { return };
                if outcome.result == EventResult::Denied && held_back(change, outcome, now) == Held::Waits {
                    self.applied.denied = true;
                    self.applied.failed += 1;
                    return;
                }
                let change = self.mailbox.contact_changes.remove(at);
                self.applied.refused += 1;
                let name = called(contact.and_then(|id| self.mailbox.contact(id)));
                if let (Some(base), Some(id)) = (change.base.as_deref(), contact) {
                    put_back(self.mailbox, id, base);
                }
                self.applied.say(format!("The server wouldn't take the change to {name}{}. It was put back as it was.", because(&outcome.error)));
            }
        }
    }

    /// The answer to deleting a contact.
    fn deleted(&mut self, outcome: &ContactOutcome) {
        let (op, account, now) = (&outcome.op, self.account, self.now);
        if op.remote.is_empty() {
            return;
        }
        let noted = |change: &ContactChange| change.kind == ContactChangeKind::Delete && change.account == account && change.remote == op.remote;
        let at = self.mailbox.contact_changes.iter().position(noted);
        match outcome.result {
            EventResult::Done | EventResult::Gone => {
                self.told |= outcome.result == EventResult::Done;
                self.applied.done += 1;
                self.destroyed.insert(op.remote.clone());
                if let Some(at) = at {
                    self.mailbox.contact_changes.remove(at);
                    return;
                }
                // Undone while it was on its way: the contact is here again and the server no longer has it. It is not
                // made anew there (it would be a new card, not the one that was deleted): it goes here too, with a word.
                // (A contact made here that the server said is this card is settled with the other answers.)
                if self.made.iter().any(|(_, remote)| *remote == op.remote) {
                    return;
                }
                self.mailbox.contact_changes.retain(|change| !(change.account == account && change.remote == op.remote));
                let Some(id) = self.by_remote(&op.remote) else { return };
                let name = called_first(self.mailbox.contact(id));
                let _ = self.mailbox.remove_contact(id);
                self.applied.say(format!(
                    "{name} was already deleted on the server when the deletion was undone here, so it is gone here too. The server may keep a copy of what was deleted."
                ));
            }
            EventResult::Failed => {
                let Some(at) = at else { return };
                if self.first_full(at, outcome) {
                    let name = called(self.mailbox.contact_changes.get(at).and_then(|change| change.base.as_deref()));
                    self.applied.say(format!("The server's mailbox is full: deleting {name} waits here and goes as soon as there is room."));
                }
                let held = self.mailbox.contact_changes.get_mut(at).map(|change| held_back(change, outcome, now));
                self.applied.failed += 1;
                if held == Some(Held::Stuck) {
                    self.applied.failed -= 1;
                    self.give_up_stuck(at, outcome);
                }
            }
            EventResult::Denied | EventResult::Refused => {
                let Some(at) = at else { return };
                let Some(change) = self.mailbox.contact_changes.get_mut(at) else { return };
                if outcome.result == EventResult::Denied && held_back(change, outcome, now) == Held::Waits {
                    self.applied.denied = true;
                    self.applied.failed += 1;
                    return;
                }
                let change = self.mailbox.contact_changes.remove(at);
                self.applied.refused += 1;
                let name = called(change.base.as_deref());
                // Back in the contacts, under the id it had when that is still free: the server still has it.
                let back = change.base.as_deref().is_some_and(|base| {
                    put_back(self.mailbox, change.contact, base);
                    true
                });
                let then = if back { " It is back in the contacts." } else { "" };
                self.applied.say(format!("The server wouldn't delete {name}{}.{then}", because(&outcome.error)));
            }
        }
    }

    /// Once every answer is in: a contact made here that the server said it made, whose card a read brought before the
    /// answer (one lost on the way) as a contact of its own, is that card.
    ///
    /// - The card was deleted there in this very answer, as that copy was deleted here (a host that sent both at once):
    ///   this contact, which the owner kept, is made anew (a new card, under a new mark).
    /// - A deletion of the copy that still waits is not sent: it would delete this contact's card.
    /// - Of the contacts here with one card ([`Taking::settle_twins`]), the one made here stays, and a copy goes once
    ///   nothing of it waits to be told (what it changed was taken there, and comes back with the read that follows).
    fn settle_copies(&mut self) {
        let account = self.account;
        let made = std::mem::take(&mut self.made);
        for (id, remote) in &made {
            if self.mailbox.contact(*id).is_none_or(|c| c.origin.id != *remote) {
                continue;
            }
            if self.destroyed.contains(remote) {
                let writable = self.mailbox.contact(*id).is_some_and(|contact| writable_book_of(self.mailbox, contact).is_some());
                if writable && self.mailbox.contact_changes.len() < MAX_CONTACT_CHANGES {
                    let token = new_token(*id, self.salt ^ self.now.unsigned_abs());
                    if let Ok(contact) = self.mailbox.contact_mut(*id) {
                        contact.origin = Origin { account, ..Default::default() };
                    }
                    self.mailbox.contact_changes.retain(|change| !(change.contact == *id && change.remote == *remote));
                    self.mailbox.contact_changes.push(ContactChange {
                        account,
                        kind: ContactChangeKind::Create,
                        contact: *id,
                        token,
                        ..Default::default()
                    });
                    log::debug!("contacts: a contact made here whose card was deleted with its copy is made anew");
                }
                continue;
            }
            let copy_deleted =
                |change: &ContactChange| change.account == account && change.kind == ContactChangeKind::Delete && change.remote == *remote;
            if self.mailbox.contact_changes.iter().any(copy_deleted) {
                self.mailbox.contact_changes.retain(|change| !copy_deleted(change));
                log::debug!("contacts: the deletion of a copy of a contact made here was not sent");
            }
            self.settle_twins(remote, Some(*id));
        }
        for remote in std::mem::take(&mut self.changed) {
            self.settle_twins(&remote, None);
        }
    }

    /// The contacts here that have the card `remote`: one stays (`keep`, the one made here; else the first), and so
    /// does every one a change of which waits, until it went. What the others kept here only (their categories,
    /// whether a favourite) goes to the one that stays.
    fn settle_twins(&mut self, remote: &str, keep: Option<Id>) {
        let account = self.account;
        let twins: Vec<Id> = self.mailbox.contacts.values().filter(|c| c.origin.account == account && c.origin.id == remote).map(|c| c.id).collect();
        let Some(keeper) = keep.filter(|id| twins.contains(id)).or_else(|| twins.first().copied()) else { return };
        for twin in twins.into_iter().filter(|twin| *twin != keeper) {
            if self.mailbox.contact_changes.iter().any(|change| change.contact == twin) {
                continue;
            }
            let kept = self.mailbox.contact(twin).map(|c| (c.categories.clone(), c.favorite));
            let _ = self.mailbox.remove_contact(twin);
            if let (Some((categories, favorite)), Ok(contact)) = (kept, self.mailbox.contact_mut(keeper)) {
                contact.favorite |= favorite;
                for category in categories {
                    if !contact.categories.contains(&category) {
                        contact.categories.push(category);
                    }
                }
            }
            log::debug!("contacts: a copy of a card another contact here has went, nothing of it waiting");
        }
    }
}

// ---- a download ----

/// Before a download of `account`'s contacts is merged ([`super::super::remote::merge_contacts`]): a contact made here
/// whose card the download has under the mark it was made with (its answer was lost on the way) is that card from now
/// on, its note to make it done (what differs here from the card is noted as a change). What it answers: what the
/// server calls the contacts whose change or deletion here it is yet to be told, which the merge leaves as they are
/// here (and doesn't bring back); and how many contacts were found again so.
pub(crate) fn before_merge(mailbox: &mut Mailbox, account: Id, download: &ContactsDownload) -> (HashSet<String>, usize) {
    let mut adopted = 0usize;
    let making: Vec<(String, Id)> = mailbox
        .contact_changes
        .iter()
        .filter(|change| change.account == account && change.kind == ContactChangeKind::Create && !change.token.is_empty())
        .map(|change| (change.token.clone(), change.contact))
        .collect();
    if !making.is_empty() {
        for remote in download.contacts.iter().take(mailcraft_model::remote::MAX_CONTACTS) {
            let mut remote = remote.clone();
            remote.tidy();
            let Some((token, id)) = making.iter().find(|(token, _)| !remote.uid.is_empty() && *token == remote.uid) else { continue };
            let taken = mailbox.contacts.values().any(|c| c.origin.account == account && c.origin.id == remote.id);
            if remote.id.is_empty() || taken {
                continue;
            }
            let Some(here) = mailbox.contact(*id).cloned().filter(|c| c.origin.account == account && c.origin.id.is_empty()) else { continue };
            // The card as the server has it, with what is kept here only.
            let mut card = super::super::remote::contact_from_remote(&remote);
            (card.id, card.book, card.uid) = (here.id, here.book, here.uid.clone());
            (card.categories, card.favorite) = (here.categories.clone(), here.favorite);
            card.origin = Origin { account, id: remote.id.clone(), ..card.origin };
            let mut found = here.clone();
            found.origin = card.origin.clone();
            mailbox
                .contact_changes
                .retain(|change| !(change.account == account && change.kind == ContactChangeKind::Create && &change.token == token));
            let more = !changed_fields(&card, &found).is_empty();
            if let Ok(contact) = mailbox.contact_mut(here.id) {
                *contact = found;
            }
            if more && mailbox.contact_changes.len() < MAX_CONTACT_CHANGES {
                mailbox.contact_changes.push(ContactChange {
                    account,
                    kind: ContactChangeKind::Update,
                    contact: here.id,
                    remote: remote.id.clone(),
                    base: Some(Box::new(card)),
                    ..Default::default()
                });
            }
            log::debug!("contacts: a contact made here was found again on the server by its mark");
            adopted += 1;
        }
    }
    let waiting = mailbox
        .contact_changes
        .iter()
        .filter(|change| change.account == account && change.kind != ContactChangeKind::Create && !change.remote.is_empty())
        .map(|change| change.remote.clone())
        .collect();
    (waiting, adopted)
}

// ---- the commands ----

pub(super) fn contact_changes_command(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::req_id(v, "account")?;
    // What can't be sent any more is given up first, and said: none of it goes without a word.
    let dropped = settle_contact_changes(s);
    let upload = contact_upload(s, account)?;
    note_sending(s, &upload);
    let mut answer = serde_json::to_value(upload).map_err(|e| fail(e.to_string()))?;
    if let Some(answer) = answer.as_object_mut() {
        answer.insert("dropped".into(), contact_dropped_json(&dropped));
    }
    Ok(answer)
}

pub(super) fn apply_contact_changes_command(s: &mut Session, v: &Value) -> CmdResult {
    let result = v.get("result").cloned().ok_or_else(|| bad("`result` is required"))?;
    let uploaded: ContactsUploaded = serde_json::from_value(result)
        .map_err(|e| bad(format!("`result`: {}", e.to_string().chars().filter(|c| !c.is_control()).take(MAX_SHOWN).collect::<String>())))?;
    Ok(apply_contact_outcomes(s, &uploaded)?.json())
}

#[cfg(test)]
mod tests;
