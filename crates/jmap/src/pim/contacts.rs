// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! An account's address books and the contacts in them, read for the engine (JMAP for Contacts, RFC 9610; cards as
//! JSContact, RFC 9553): [`read`], the last part of a job ([`super::sync_pim`]), after what was changed of them here
//! was written ([`push`]).
//!
//! **What is asked of the server**, in as few requests as it takes:
//!
//! 1. When the request holds the state of the window's last whole read, and it is less than an hour old
//!    ([`JmapPimRequest::contacts_state_to_ask`]): `AddressBook/get` (every book) and a `ContactCard/query` for one
//!    card, in one request. When the server still says that state, nothing has changed since: that is all, and the
//!    books come back without the contacts.
//! 2. Otherwise, or when the state changed: `AddressBook/get` (unless the first request read it) and
//!    `ContactCard/query` from the start, for as many as [`READ_CARDS`] and with the total, in one request; further
//!    pages from where it ended when the server set a lower limit ([`Pager`](super::Pager)); then `ContactCard/get` of
//!    what it found, as many in a request as the server takes in a `get`, for the properties the engine takes (not
//!    `notes`, which a server that keeps only the basics doesn't serve). The query asks for no order, filter or anchor:
//!    a server that keeps only the basics has none of them but its own order.
//!
//! A server that takes fewer calls in a request is asked the books and the query in turn.
//!
//! **What comes back.** The books ([`BooksDownload`], complete when they were listed): each with its name, read-only
//! unless the account may write its cards (`myRights.mayWrite`, and [`WRITE_CONTACTS`](super::WRITE_CONTACTS) open),
//! its cards deletable where it may write them and the server does not narrow that (`myRights.mayRemoveItems`), the
//! main one (`isDefault`), and plain when the session says a card keeps
//! no more than its name, e-mail addresses, phone numbers, organizations and titles, or, when it says nothing of it,
//! when the request says the server is one of those ([`JmapPimRequest::plain_unless_said`]).
//!
//! Then the contacts ([`ContactsDownload`]): complete when the query was read whole and not cut at [`READ_CARDS`],
//! every card it found was read (or the server said it was gone since), the server's state did not change from one
//! page to the next, and the job was not stopped. Each card is in one book ([`super::jscontact`]); a card that is no
//! person or has nothing to show is left out, as the engine would leave it out. With complete contacts comes the state
//! they were read at ([`JmapPimResult::contacts_state`]); after any trouble no state comes, and the next job reads
//! everything.
//!
//! **When something goes wrong**, as for the calendars: the server refusing the credentials ends the job with
//! `auth_refused`, one that is busy with `busy_for`, a stop with `stopped`. A request that fails in another way, or a
//! query the server answers with an error, ends the reading of the contacts with a line, and what was read before it
//! is handed over, in a download that adds and updates and removes nothing. A list of books the server answers with an
//! error is a line. A `get` that fails is asked again in halves, by its ids, until the card that breaks it is found;
//! that one is left out. The lines name no contact and no book, and what a server says in them is scrubbed as the
//! calendars' lines are.

pub(super) mod push;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::book::BooksDownload;
use mailcraft_model::remote::{ContactsDownload, RemoteContact};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use serde_json::{Value, json};

use super::jscontact::{self, Listed};
use super::{IDS_BYTES, ITEMS_BYTES, Pager, Stop, WRITE_CONTACTS, counted, said, secrets_of};
use crate::{CONTACTS, CORE, Client, Error, Http, Secrets};

/// Most cards one job reads, by the server's order (a server that keeps only the basics lists them by name): an
/// instance with more shows the first ones, and its download is not complete.
pub const READ_CARDS: usize = 5_000;
/// Most cards asked for in one `get`, whatever the server takes.
const GET: usize = 500;
/// The most a list of address books may be, in bytes.
const LIST_BYTES: usize = 1 << 20;
/// Most times in a job that a `get` is halved to find the card that breaks it.
const MAX_SPLITS: usize = 32;
/// The longest state of the cards that is believed, in bytes.
const MAX_STATE: usize = 256;
/// What every request names in `using`.
const USING: [&str; 2] = [CORE, CONTACTS];
/// What a card is read for.
const CARD_PROPERTIES: [&str; 9] = ["id", "uid", "addressBookIds", "kind", "name", "emails", "phones", "organizations", "titles"];
/// What a card keeps when a server keeps only the basics of one: a session that says it writes no more is one.
const BASICS: [&str; 5] = ["emails", "name", "organizations", "phones", "titles"];

/// Read the address books of the account `request` is for, and their cards, as the module says, into `out`; the books
/// are in the account `account` of the session.
pub(super) async fn read<H: Http>(client: &Client<H>, request: &JmapPimRequest, account: &str, cancel: &AtomicBool, out: &mut JmapPimResult) {
    let job = Job { client, request, account, cancel, secrets: secrets_of(request, &client.secrets, account) };
    let mut got = Gathered::new();
    let stop = job.run(&mut got).await.err();
    match stop {
        None => {}
        Some(Stop::Cancelled) => out.stopped = true,
        Some(Stop::Auth) => out.auth_refused = true,
        Some(Stop::Busy(seconds, why)) => {
            out.busy_for = Some(seconds);
            got.halted = Some(why);
        }
        Some(Stop::Failed(error)) => got.halted = Some(said(&job.secrets, &error)),
    }
    job.finish(got, out);
}

/// What the job has read so far.
struct Gathered {
    /// The books listed, when they were; and why they could not be, when they were asked for.
    listed: Option<Vec<Listed>>,
    listing: Option<String>,
    /// The server still says the state the request held: nothing more is read.
    unchanged: Option<String>,
    /// The state the first page of the query said, and whether a later page said another.
    state: Option<String>,
    moved: bool,
    pager: Pager,
    /// A page of the query was answered: there are cards to hand over, if only none.
    answered: bool,
    /// The cards read, in the order they came, each once.
    cards: Vec<RemoteContact>,
    seen: HashSet<String>,
    /// Cards the query found that could not be read, and why the first could not.
    unread: (usize, Option<String>),
    /// Cards that were no person to show.
    passed_over: usize,
    /// Why the reading ended early, as a line.
    halted: Option<String>,
    /// How many times a `get` was halved.
    splits: usize,
}

impl Gathered {
    fn new() -> Gathered {
        Gathered {
            listed: None,
            listing: None,
            unchanged: None,
            state: None,
            moved: false,
            pager: Pager::new(READ_CARDS, READ_CARDS).cards(),
            answered: false,
            cards: Vec::new(),
            seen: HashSet::new(),
            unread: (0, None),
            passed_over: 0,
            halted: None,
            splits: 0,
        }
    }
}

/// One reading of an account's address books.
struct Job<'a, H> {
    client: &'a Client<H>,
    request: &'a JmapPimRequest,
    account: &'a str,
    cancel: &'a AtomicBool,
    /// What a line must not show.
    secrets: Secrets,
}

impl<H: Http> Job<'_, H> {
    fn check(&self) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) { Err(Stop::Cancelled) } else { Ok(()) }
    }

    fn call_stop(&self, error: Error) -> Stop {
        Stop::of(error, &self.secrets)
    }

    fn list_call(&self) -> (&'static str, Value) {
        ("AddressBook/get", json!({"accountId": self.account, "ids": null}))
    }

    /// A query of `limit` cards from `position`, with the total when `total`.
    fn query_call(&self, position: usize, limit: usize, total: bool) -> (&'static str, Value) {
        ("ContactCard/query", json!({"accountId": self.account, "position": position, "limit": limit, "calculateTotal": total}))
    }

    /// The whole reading: the state alone when it may do, else the books and every page, then the cards.
    async fn run(&self, got: &mut Gathered) -> Result<(), Stop> {
        self.check()?;
        let mut listed = false;
        if let Some(held) = self.request.contacts_state_to_ask() {
            let [list, query] = self.ask(self.list_call(), self.query_call(0, 1, false), IDS_BYTES).await?;
            listed = true;
            self.take_list(got, list);
            match query {
                Ok(answer) if state_of(&answer).as_deref() == Some(held) => {
                    got.unchanged = Some(held.to_string());
                    return Ok(());
                }
                // Another state, or none: everything is read below.
                Ok(_) => {}
                Err(error) => return Err(Stop::Failed(error)),
            }
        }
        // The books (unless the state was asked about with them) and the first page; then the other pages.
        while got.pager.done().is_none() {
            self.check()?;
            let at = got.pager.position();
            let query = self.query_call(at, got.pager.limit(), at == 0);
            let found = if listed {
                let mut answers = self.client.call_within(&USING, vec![query], IDS_BYTES).await.map_err(|e| self.call_stop(e))?;
                answers.take(0)
            } else {
                listed = true;
                let [list, found] = self.ask(self.list_call(), query, IDS_BYTES).await?;
                self.take_list(got, list);
                found
            };
            let found = found.map_err(Stop::Failed)?;
            match (state_of(&found), &got.state) {
                (state, None) if got.pager.position() == 0 => got.state = state,
                (state, kept) if state.as_ref() != kept.as_ref() => got.moved = true,
                _ => {}
            }
            got.pager.page(&found);
            got.answered = true;
        }
        let ids: Vec<String> = got.pager.ids().to_vec();
        self.read_cards(got, ids).await
    }

    /// The books' call and the query's, in one request when the server takes two calls in one, else one after the
    /// other (the query's answer `bound` bytes at most): their answers, in that order.
    async fn ask(&self, list: (&'static str, Value), query: (&'static str, Value), bound: usize) -> Result<[crate::Result<Value>; 2], Stop> {
        if self.client.max_calls_in_request() >= 2 {
            let mut answers = self.client.call_within(&USING, vec![list, query], bound.max(LIST_BYTES)).await.map_err(|e| self.call_stop(e))?;
            return Ok([answers.take(0), answers.take(1)]);
        }
        let mut listed = self.client.call_within(&USING, vec![list], LIST_BYTES).await.map_err(|e| self.call_stop(e))?;
        self.check()?;
        let mut found = self.client.call_within(&USING, vec![query], bound).await.map_err(|e| self.call_stop(e))?;
        Ok([listed.take(0), found.take(0)])
    }

    /// The list of books as it came, or why it didn't.
    fn take_list(&self, got: &mut Gathered, answer: crate::Result<Value>) {
        match answer {
            Ok(answer) => match answer.get("list").and_then(Value::as_array) {
                Some(list) => {
                    let mut listed: Vec<Listed> = Vec::new();
                    for book in list.iter().take(mailcraft_model::book::MAX_REMOTE_BOOKS).filter_map(jscontact::book_of) {
                        if !listed.iter().any(|have| have.book.id == book.book.id) {
                            listed.push(book);
                        }
                    }
                    got.listed = Some(listed);
                }
                None => got.listing = Some("the answer has no list of address books".into()),
            },
            Err(error) => got.listing = Some(said(&self.secrets, &error)),
        }
    }

    /// The books the account may write cards in, by their ids: a card's own book is the first of them it is in.
    fn writable(got: &Gathered) -> HashSet<String> {
        got.listed.iter().flatten().filter(|listed| listed.writable).map(|listed| listed.book.id.clone()).collect()
    }

    /// `ids` asked for by themselves, as many at a time as the server takes and in halves when a call for them fails,
    /// until the card that breaks it is found: that one is left out, the rest are read.
    async fn read_cards(&self, got: &mut Gathered, ids: Vec<String>) -> Result<(), Stop> {
        let size = self.client.max_objects_in_get().clamp(1, GET);
        let mut work: Vec<Vec<String>> = ids.chunks(size).rev().map(<[String]>::to_vec).collect();
        let writable = Self::writable(got);
        while let Some(chunk) = work.pop() {
            self.check()?;
            let get = json!({"accountId": self.account, "ids": chunk, "properties": CARD_PROPERTIES});
            let mut answers = self.client.call_within(&USING, vec![("ContactCard/get", get)], ITEMS_BYTES).await.map_err(|e| self.call_stop(e))?;
            match answers.take(0) {
                Ok(answer) => self.take_cards(got, &chunk, &answer, &writable),
                Err(_) if chunk.len() > 1 && got.splits < MAX_SPLITS => {
                    got.splits = got.splits.saturating_add(1);
                    let (first, second) = chunk.split_at(chunk.len() / 2);
                    work.push(second.to_vec());
                    work.push(first.to_vec());
                }
                Err(error) if chunk.len() == 1 => unread(got, 1, said(&self.secrets, &error)),
                // The server keeps failing: what is left waits for the next job.
                Err(error) => {
                    let left = chunk.len().saturating_add(work.iter().map(Vec::len).sum::<usize>());
                    unread(got, left, said(&self.secrets, &error));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// The cards of a `get`'s answer, of those asked for (`ids`): each once; what it left out without saying it was
    /// gone is counted as unread.
    fn take_cards(&self, got: &mut Gathered, ids: &[String], answer: &Value, writable: &HashSet<String>) {
        let Some(list) = answer.get("list").and_then(Value::as_array) else {
            unread(got, ids.len(), "the answer has no list of cards".into());
            return;
        };
        let asked: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let gone: HashSet<&str> = answer.get("notFound").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        let mut came: HashSet<String> = HashSet::new();
        for value in list.iter().take(ids.len()) {
            let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
            if !asked.contains(id) || came.contains(id) {
                continue;
            }
            came.insert(id.to_string());
            match jscontact::card_of(value, writable) {
                Ok(card) if got.seen.insert(card.id.clone()) => got.cards.push(card),
                Ok(_) => {}
                Err(missing) => {
                    log::debug!("jmap pim: a card of the server's answer was passed over (no {missing:?})");
                    got.passed_over = got.passed_over.saturating_add(1);
                }
            }
        }
        let left_out = ids.iter().filter(|id| !came.contains(id.as_str()) && !gone.contains(id.as_str())).count();
        if left_out > 0 {
            unread(got, left_out, "the server's answer did not have them".into());
        }
    }

    /// The answer, from what was read.
    fn finish(&self, got: Gathered, out: &mut JmapPimResult) {
        let (account, email) = (self.request.account, self.request.email.clone());
        if let Some(listed) = &got.listed {
            let plain = match self.client.contacts_writable() {
                Some(written) => written.iter().all(|property| BASICS.contains(&property.as_str())),
                None => self.request.plain_unless_said,
            };
            let books = listed
                .iter()
                .map(|listed| mailcraft_model::book::RemoteBook {
                    read_only: !(listed.writable && WRITE_CONTACTS),
                    may_remove: listed.removes && WRITE_CONTACTS,
                    plain,
                    ..listed.book.clone()
                })
                .collect();
            out.books = Some(BooksDownload { account, email: email.clone(), complete: true, books });
        }
        out.errors.extend(lines(&got));
        if let Some(held) = got.unchanged {
            out.contacts_state = Some(held);
            return;
        }
        // No page of the query was answered: there is nothing to say of the cards.
        if !got.answered {
            return;
        }
        let complete =
            got.pager.done() == Some(true) && got.unread.0 == 0 && got.halted.is_none() && !got.moved && !self.cancel.load(Ordering::Relaxed);
        if got.passed_over > 0 {
            log::debug!("jmap pim: {} card(s) that are no person to show were left out", got.passed_over);
        }
        out.contacts_state = got.state.filter(|state| complete && state.len() <= MAX_STATE);
        out.contacts = Some(ContactsDownload { account, email, complete, contacts: got.cards });
    }
}

/// The state a `/query` says of what it lists (`queryState`), when it says one.
fn state_of(answer: &Value) -> Option<String> {
    answer.get("queryState").and_then(Value::as_str).filter(|state| !state.is_empty() && state.len() <= MAX_STATE).map(str::to_string)
}

fn unread(got: &mut Gathered, count: usize, why: String) {
    got.unread.0 = got.unread.0.saturating_add(count);
    got.unread.1.get_or_insert(why);
}

/// The lines of trouble worth telling the owner.
fn lines(got: &Gathered) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(why) = &got.listing {
        lines.push(format!("Couldn't list the address books on the server: {why}"));
    }
    if let Some(why) = &got.halted {
        lines.push(format!("Couldn't bring the contacts down from the server: {why}"));
    }
    if let (n @ 1.., Some(why)) = (got.unread.0, &got.unread.1) {
        lines.push(format!("Couldn't read {} of the address books: {why}", counted(n, "contact", "contacts")));
    }
    lines
}

#[cfg(test)]
mod tests;
