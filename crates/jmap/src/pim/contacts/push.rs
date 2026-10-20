// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What was made, changed and deleted here of the contacts of an account's address books, written to its server: the
//! part of a job before its contacts are read ([`push`]), so that what is read shows it.
//!
//! 1. **A new card sent before** whose answer didn't come may be on the server already: the card made of it is looked
//!    for by the mark it was made with (`ContactCard/query` by `uid`, RFC 9610 §3.3), and found, it is a change of
//!    that card, of what was changed here since it was sent ([`merged_json`]). A server that can't look for it there
//!    (it filters on no `uid`) has it sent as a new card again: its `uid` makes no second one where the server knows
//!    it. One that couldn't be looked for now (the look failed) is not sent, and goes again later: sent whole, it
//!    would take away from the card what someone else added there meanwhile.
//! 2. **The server's card first.** Every card a change or a deletion is about is read (a `ContactCard/get` of their
//!    names). One the server no longer has is gone; one it made at another time than the card the change is about
//!    (`created`, compared as instants: a server gave a new contact the name of one that was deleted) is another
//!    contact, and is never written to: the change is gone too.
//! 3. **The calls.** Each change is a create of a card whole, every property it says named (an empty one as empty), in
//!    its book and with the mark it was made with as its `uid`; an update of the whole properties it names, made to
//!    the server's card as it is now ([`merged_json`]: the addresses and numbers added or taken out here, and those
//!    someone else has there kept as they are), a name always with its parts and its full form, never a part of a
//!    property (a server that keeps only the basics refuses `emails/e1`); or a destroy. As many in a call as the
//!    session takes in a `set`, [`MAX_PER_CALL`] at most; a card two changes are about takes the first, and the second
//!    waits for the next job. A call the server refuses for its size, before running it (a 4xx that says so, or a
//!    413), is sent again in halves. In a job that sends a new card again, the new cards go first, and a change of the
//!    card one of them is answered with waits for the next job (a copy of it a read brought, changed or deleted here,
//!    is made to the card as it is then, and never deletes it); when one has no answer that names its card (lost, or
//!    not sent) nor says none was made, every change and deletion of a card the server has waits.
//! 4. **Outcomes**, one for each change tried, in the order they came: done (a new card with what the server calls it,
//!    as the engine names a card: [`card_key`](super::super::card_key)), gone, refused with the server's words (its
//!    markup taken out, [`MAX_WORDS`] characters at most), or failed with when to try again (a busy server, one that
//!    failed, a connection that broke, an answer too large to take, a change not reached). A failure that asking again
//!    is unlikely to get past (the server failed with this very card, or with the look for it, said nothing of it,
//!    won't make a new one for now, or a change alone is too large for it) says so ([`ContactOutcome::lasting`]); so
//!    does one of a new card what is known of a card made of it ([`ContactOutcome::made`]: perhaps, when it was sent
//!    and no answer said; none, when the server refused it and made none of it before). A card the server says is
//!    there already (`alreadyExists` with its id) is that card: it is read, and what was changed here since it was
//!    sent before is made to it. A new card sent again as a change of its card, refused, says which card (the contact
//!    is that one). A server that refuses the credentials, or is busy, ends the job: what was not tried waits as it
//!    was, with no outcome.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::book::{CardMade, ContactChangeKind, ContactFields, ContactOp, ContactOutcome, ContactsUploaded, MAX_CONTACT_OPS};
use mailcraft_model::remote::EventResult;
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use mailcraft_time::parse_iso;
use serde_json::{Map, Value, json};

use super::super::jscontact::{card_json, merged_json};
use super::super::{IDS_BYTES, ITEMS_BYTES, Stop, card_id, card_key, said, secrets_of, server_card_id};
use crate::{CONTACTS, CORE, Client, Error, Http, Secrets};

/// Most cards in one `ContactCard/set`, and read in one `get`.
pub const MAX_PER_CALL: usize = 50;
const PAGE: usize = 500;
/// The longest words of the server's kept, in characters.
pub const MAX_WORDS: usize = 200;
/// How long a server that failed is left alone before a change goes again, when it doesn't say, in seconds.
const RETRY: u32 = 60;
/// Most times in a job that a call is halved for being too large.
const MAX_SPLITS: usize = 16;
/// Most cards sent again that are looked for in one request.
const MAX_FINDS: usize = 16;
/// What every request names in `using`.
const USING: [&str; 2] = [CORE, CONTACTS];
/// What the server's card is read for before a change or a deletion: what a change writes, and when the card was
/// made and last changed.
const CARD_PROPERTIES: [&str; 8] = ["id", "name", "emails", "phones", "organizations", "titles", "created", "updated"];
/// Why a change is not written: the card is another contact's now.
const ANOTHER: &str = "the server has another contact under its name now";
/// Why a card the server's `get` said nothing of is not written to now: asking again is unlikely to get past it.
const UNSAID: &str = "the server's answer did not have it";

/// Write what `request.contact_changes` says to the account's address books (in `account` of the session) and put what
/// came of each change into `out.contacts_uploaded`. Whether the job may go on to read: not after a stop, a refusal of
/// the credentials or a busy server, which `out` then says.
pub(in crate::pim) async fn push<H: Http>(
    client: &Client<H>,
    request: &JmapPimRequest,
    account: &str,
    cancel: &AtomicBool,
    out: &mut JmapPimResult,
) -> bool {
    let Some(upload) = request.contact_changes.as_ref().filter(|upload| !upload.ops.is_empty()) else { return true };
    let ops: Vec<ContactOp> = upload
        .ops
        .iter()
        .take(MAX_CONTACT_OPS)
        .map(|op| {
            let mut op = op.clone();
            op.tidy();
            op
        })
        .collect();
    let mut writer = Writer {
        client,
        account,
        cancel,
        secrets: secrets_of(request, &client.secrets, account),
        outcomes: vec![None; ops.len()],
        ops,
        splits: 0,
        found: HashMap::new(),
        none_made: HashSet::new(),
        copies: HashMap::new(),
        gone: HashSet::new(),
        unread: HashMap::new(),
        again: Vec::new(),
        made: HashSet::new(),
    };
    let stop = writer.run().await.err();
    let token_refused = matches!(stop, Some(Stop::Auth));
    let going = matches!(stop, None | Some(Stop::Failed(_)));
    match stop {
        None | Some(Stop::Failed(_)) => {}
        Some(Stop::Cancelled) => out.stopped = true,
        Some(Stop::Auth) => out.auth_refused = true,
        Some(Stop::Busy(seconds, _)) => out.busy_for = Some(seconds),
    }
    let outcomes: Vec<ContactOutcome> = writer.outcomes.into_iter().flatten().collect();
    log::debug!("jmap pim: {} of {} change(s) of contacts answered", outcomes.len(), upload.ops.len().min(MAX_CONTACT_OPS));
    out.contacts_uploaded = Some(ContactsUploaded { account: request.account, email: request.email.clone(), outcomes, token_refused });
    going
}

/// One change made ready for a call: its place, the card it writes to (empty for a new one), and what is written
/// (nothing for a deletion).
#[derive(Clone)]
struct Ready {
    at: usize,
    target: String,
    body: Option<Value>,
}

/// The writes of one job.
struct Writer<'a, H> {
    client: &'a Client<H>,
    account: &'a str,
    cancel: &'a AtomicBool,
    secrets: Secrets,
    ops: Vec<ContactOp>,
    /// What became of each change, by its place.
    outcomes: Vec<Option<ContactOutcome>>,
    /// How many times a call was halved.
    splits: usize,
    /// New cards sent before that the server has, by their places: what it calls them; and those it has none of.
    found: HashMap<usize, String>,
    none_made: HashSet<usize>,
    /// The server's cards read, by their names; the names it said it doesn't have; and why others couldn't be read
    /// (and whether asking again is unlikely to get past it).
    copies: HashMap<String, Value>,
    gone: HashSet<String>,
    unread: HashMap<String, (String, bool)>,
    /// New cards the server says it had already (`alreadyExists`), by their places, with what it calls them: read,
    /// and what was changed here since they were sent written to them.
    again: Vec<(usize, String)>,
    /// What the server calls the cards it answered the new ones with, in this job.
    made: HashSet<String>,
}

/// What the server calls the card a change is about, for a change or a deletion: `None` for a key that names none.
fn target(op: &ContactOp) -> Option<String> {
    server_card_id(&op.remote).filter(|id| card_id(id))
}

/// The instant a card's `created` names, to the second: a date and a time with its offset (`Z` for UTC); `None` for
/// one without an offset, or for no time.
fn instant(text: &str) -> Option<i64> {
    let text = text.trim();
    if !text.contains(['T', 't']) {
        return None;
    }
    let (time, offset) = parse_iso(text)?;
    Some(time.civil_seconds().saturating_sub(i64::from(offset?).saturating_mul(60)))
}

/// Whether the server's `card` is another contact than the one `op` is about: it was made at another time than the one
/// the change knows (a server gave a new contact the name of one that was deleted), compared as instants (the same
/// time may be written another way: in another zone, with a fraction of a second), else as they are written. Unknown
/// either side: not another.
fn another(op: &ContactOp, card: &Value) -> bool {
    let made = card.get("created").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if op.mark.is_empty() || made.is_empty() {
        return false;
    }
    match (instant(&op.mark), instant(made)) {
        (Some(known), Some(now)) => known != now,
        _ => made != op.mark,
    }
}

/// A new card sent again, as a change of the card the server made of it: of what it was sent with before (`op.base`),
/// only what was changed here since, made to the card as it is now; all of it, when what it was sent with is not known.
fn resent(op: &ContactOp) -> ContactOp {
    let mut again = op.clone();
    again.kind = ContactChangeKind::Update;
    if let Some(sent) = &op.base {
        let fields = &op.fields;
        again.fields = ContactFields {
            name: fields.name.clone().filter(|now| sent.name.as_ref() != Some(now)),
            company: fields.company.clone().filter(|now| sent.company.as_ref() != Some(now)),
            title: fields.title.clone().filter(|now| sent.title.as_ref() != Some(now)),
            emails: fields.emails.clone().filter(|now| sent.emails.as_ref() != Some(now)),
            phones: fields.phones.clone().filter(|now| sent.phones.as_ref() != Some(now)),
        };
    }
    again
}

impl<H: Http> Writer<'_, H> {
    fn check(&self) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) { Err(Stop::Cancelled) } else { Ok(()) }
    }

    /// Say what became of the change at `at`, unless something was said already.
    fn answer(&mut self, at: usize, result: EventResult, remote: String, error: &str, retry_after: Option<u32>, lasting: bool) {
        let Some(op) = self.ops.get(at) else { return };
        if let Some(slot) = self.outcomes.get_mut(at).filter(|slot| slot.is_none()) {
            let mut outcome =
                ContactOutcome { op: op.clone(), result, remote, error: crate::tidy(error, MAX_WORDS), retry_after, lasting, made: CardMade::Unsaid };
            outcome.tidy();
            *slot = Some(outcome);
        }
    }

    /// The change at `at` couldn't be done now: to be tried again after `retry_after` (or [`RETRY`]).
    fn failed(&mut self, at: usize, why: &str, retry_after: Option<u32>, lasting: bool) {
        self.answer(at, EventResult::Failed, String::new(), why, Some(retry_after.unwrap_or(RETRY)), lasting);
    }

    /// A request about the changes at `affected` failed as a whole: a refused sign-in and a busy server end the job
    /// (what was not tried has no outcome, and waits as it was); anything else fails those changes, to go again.
    fn request_failed(&mut self, error: Error, affected: &[usize]) -> Result<(), Stop> {
        let words = said(&self.secrets, &error);
        match Stop::of(error, &self.secrets) {
            Stop::Auth => Err(Stop::Auth),
            Stop::Busy(seconds, why) => {
                for at in affected {
                    self.failed(*at, &words, Some(seconds), false);
                }
                Err(Stop::Busy(seconds, why))
            }
            _ => {
                for at in affected {
                    self.failed(*at, &words, None, false);
                }
                Ok(())
            }
        }
    }

    async fn run(&mut self) -> Result<(), Stop> {
        // What can't be written as it is said.
        let mut tried: Vec<usize> = Vec::new();
        for at in 0..self.ops.len() {
            let Some(op) = self.ops.get(at) else { continue };
            let refusal = match op.kind {
                ContactChangeKind::Create if op.token.is_empty() => Some("the contact has no mark to be made with"),
                ContactChangeKind::Create => None,
                _ if target(op).is_none() => Some("the contact's name on the server can't be read"),
                ContactChangeKind::Update if op.fields.is_empty() => Some("the change says nothing that could be sent"),
                _ => None,
            };
            match refusal {
                Some(why) => self.answer(at, EventResult::Refused, String::new(), why, None, false),
                None => tried.push(at),
            }
        }
        // New cards sent before: the cards the server made of them, where it can say (one that couldn't be looked for
        // now is answered, and not sent).
        self.find_made(&tried).await?;
        let resent: Vec<usize> = tried
            .iter()
            .copied()
            .filter(|at| {
                !self.found.contains_key(at) && self.ops.get(*at).is_some_and(|op| op.kind == ContactChangeKind::Create && op.base.is_some())
            })
            .collect();
        tried.retain(|at| self.outcomes.get(*at).is_some_and(Option::is_none));
        // One change of a card in a job, a new card sent again that was found first; a second waits for the next job,
        // with no outcome (it is made to what the first leaves), a second new card found as that card too among them.
        let mut claimed: HashSet<String> = HashSet::new();
        let mut found: Vec<(usize, String)> = self.found.iter().map(|(at, card)| (*at, card.clone())).collect();
        found.sort_unstable();
        for (at, card) in found {
            if !claimed.insert(card) {
                self.found.remove(&at);
                tried.retain(|one| *one != at);
            }
        }
        let (mut creates, mut others): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
        for at in tried {
            let Some(op) = self.ops.get(at) else { continue };
            if self.found.contains_key(&at) {
                others.push(at);
            } else if op.kind == ContactChangeKind::Create {
                creates.push(at);
            } else if let Some(card) = target(op)
                && claimed.insert(card)
            {
                others.push(at);
            }
        }
        // The server's cards the changes are about, as they are now.
        let wanted: Vec<String> = others.iter().filter_map(|at| self.card_at(*at)).collect();
        self.read_cards(wanted).await?;
        let creates: Vec<Ready> = creates.into_iter().filter_map(|at| self.prepare(at)).collect();
        let others: Vec<Ready> = others.into_iter().filter_map(|at| self.prepare(at)).collect();
        let size = MAX_PER_CALL.min(self.client.max_objects_in_set().max(1));
        if !resent.is_empty() {
            // A new card sent again as a new one may be one the server made the first time, that another change here is
            // about (a copy of it that a read brought): the new cards first; a change of the card one of them is
            // answered with waits for the next job, when it is made to the card as it is then. One that no answer
            // named (lost, or not sent) may be any card: every change or deletion of a card the server has waits.
            self.write(creates, size).await?;
            self.write_again(size).await?;
            let named = |outcome: &Option<ContactOutcome>| {
                outcome
                    .as_ref()
                    .is_some_and(|outcome| (outcome.result == EventResult::Done && !outcome.remote.is_empty()) || outcome.made == CardMade::No)
            };
            let unnamed = resent.iter().any(|at| !self.outcomes.get(*at).is_some_and(named));
            if unnamed {
                log::debug!("jmap pim: a new card sent again has no answer that names its card: the changes of cards wait");
            }
            let held = |one: &Ready| {
                self.made.contains(&one.target)
                    || (unnamed && !one.target.is_empty() && self.ops.get(one.at).is_some_and(|op| op.kind != ContactChangeKind::Create))
            };
            let others: Vec<Ready> = others.into_iter().filter(|one| !held(one)).collect();
            self.write(others, size).await?;
        } else {
            let mut all: Vec<Ready> = creates.into_iter().chain(others).collect();
            all.sort_by_key(|one| one.at);
            self.write(all, size).await?;
            self.write_again(size).await?;
        }
        Ok(())
    }

    /// The card the change at `at` writes to: the one found of a new card sent again, else the one it names.
    fn card_at(&self, at: usize) -> Option<String> {
        match self.found.get(&at) {
            Some(card) => Some(card.clone()),
            None => self.ops.get(at).filter(|op| op.kind != ContactChangeKind::Create).and_then(target),
        }
    }

    /// The new cards at `tried` that were sent before (whose answer didn't come) looked for by the mark they were made
    /// with ([`Writer::found`]). One the server says it has none of, or that a server says it can't look for so
    /// (`unsupportedFilter`), is sent as a new card again. One that couldn't be looked for now (the request failed, the
    /// server failed with the look, its answer can't be read) is not sent: it fails, to go again later (sent whole, it
    /// would take away from the card what someone else added there meanwhile).
    async fn find_made(&mut self, tried: &[usize]) -> Result<(), Stop> {
        let sent_before: Vec<usize> = tried
            .iter()
            .copied()
            .filter(|at| self.ops.get(*at).is_some_and(|op| op.kind == ContactChangeKind::Create && op.base.is_some()))
            .collect();
        let per = self.client.max_calls_in_request().clamp(1, MAX_FINDS);
        let mut can = true;
        for chunk in sent_before.chunks(per) {
            if !can {
                break;
            }
            self.check()?;
            let calls: Vec<(&str, Value)> = chunk
                .iter()
                .filter_map(|at| self.ops.get(*at))
                .map(|op| ("ContactCard/query", json!({"accountId": self.account, "filter": {"uid": op.token}, "limit": 2})))
                .collect();
            let mut answers = match self.client.call_within(&USING, calls, IDS_BYTES).await {
                Ok(answers) => answers,
                Err(error) => {
                    self.request_failed(error, chunk)?;
                    continue;
                }
            };
            for (n, at) in chunk.iter().enumerate() {
                match answers.take(n) {
                    Ok(answer) => match answer.get("ids").and_then(Value::as_array) {
                        Some(ids) => {
                            let ids: Vec<&str> = ids.iter().filter_map(Value::as_str).take(3).collect();
                            // (Two cards made with one mark: neither is taken for it.)
                            match ids.as_slice() {
                                [] => {
                                    self.none_made.insert(*at);
                                }
                                [one] if card_id(one) => {
                                    self.found.insert(*at, (*one).to_string());
                                }
                                _ => {}
                            }
                        }
                        None => self.failed(*at, "the server's answer to the look for its card can't be read", None, true),
                    },
                    // (Not asked again in this job.)
                    Err(Error::Refused { kind, .. }) if kind == "unsupportedFilter" => can = false,
                    // The server failed with the look itself: asking again may get past it once or twice, not for ever.
                    Err(error) => {
                        let words = said(&self.secrets, &error);
                        self.failed(*at, &words, None, true);
                    }
                }
            }
        }
        if !self.found.is_empty() {
            log::debug!("jmap pim: {} new card(s) sent before found on the server by their mark", self.found.len());
        }
        Ok(())
    }

    /// Read the server's cards of `ids` (each once), [`PAGE`] at a time.
    async fn read_cards(&mut self, ids: Vec<String>) -> Result<(), Stop> {
        let mut wanted: Vec<String> = Vec::new();
        for id in ids {
            if !wanted.contains(&id) {
                wanted.push(id);
            }
        }
        let size = PAGE.min(self.client.max_objects_in_get().max(1));
        for chunk in wanted.chunks(size) {
            self.check()?;
            let get = json!({"accountId": self.account, "ids": chunk, "properties": CARD_PROPERTIES});
            let mut answers = match self.client.call_within(&USING, vec![("ContactCard/get", get)], ITEMS_BYTES).await {
                Ok(answers) => answers,
                Err(error) => {
                    let affected = self.waiting_on(chunk);
                    let words = said(&self.secrets, &error);
                    self.request_failed(error, &affected)?;
                    self.unread.extend(chunk.iter().map(|id| (id.clone(), (words.clone(), false))));
                    continue;
                }
            };
            match answers.take(0) {
                Ok(answer) => {
                    let listed = answer.get("list").and_then(Value::as_array).cloned().unwrap_or_default();
                    for card in listed.into_iter().take(chunk.len()) {
                        let id = card.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                        if chunk.contains(&id) {
                            self.copies.insert(id, card);
                        }
                    }
                    let gone: HashSet<&str> =
                        answer.get("notFound").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
                    for id in chunk {
                        if gone.contains(id.as_str()) && !self.copies.contains_key(id) {
                            self.gone.insert(id.clone());
                        } else if !self.copies.contains_key(id) {
                            // (Neither there nor not found: a server that says so of a card says so again.)
                            self.unread.insert(id.clone(), (UNSAID.into(), true));
                        }
                    }
                }
                Err(error) => {
                    let words = said(&self.secrets, &error);
                    self.unread.extend(chunk.iter().map(|id| (id.clone(), (words.clone(), false))));
                }
            }
        }
        Ok(())
    }

    /// The changes not answered yet whose card is among `ids`.
    fn waiting_on(&self, ids: &[String]) -> Vec<usize> {
        (0..self.ops.len())
            .filter(|at| self.outcomes.get(*at).is_some_and(Option::is_none))
            .filter(|at| self.card_at(*at).is_some_and(|card| ids.contains(&card)))
            .collect()
    }

    /// The change at `at` made ready for a call, or answered (`None`).
    fn prepare(&mut self, at: usize) -> Option<Ready> {
        let op = self.ops.get(at)?.clone();
        let found = self.found.get(&at).cloned();
        if op.kind == ContactChangeKind::Create && found.is_none() {
            return Some(Ready { at, target: String::new(), body: Some(card_json(&op)) });
        }
        let card = self.card_at(at)?;
        if self.gone.contains(&card) {
            if found.is_some() {
                // Made the first time and deleted there since: made anew.
                return Some(Ready { at, target: String::new(), body: Some(card_json(&op)) });
            }
            self.answer(at, EventResult::Gone, String::new(), "the server no longer has the contact", None, false);
            return None;
        }
        self.made_to(at, &op, card)
    }

    /// The change at `at` (`op`) made to the server's card `card` as it was read; answered (`None`) when it can't be,
    /// or when nothing is left to write to it.
    fn made_to(&mut self, at: usize, op: &ContactOp, card: String) -> Option<Ready> {
        let Some(copy) = self.copies.get(&card).cloned() else {
            // Not read now: it waits, and goes once the server's card is known.
            let (why, lasting) = self.unread.get(&card).cloned().unwrap_or_else(|| ("the server's card couldn't be read".into(), false));
            if self.outcomes.get(at).is_some_and(Option::is_none) {
                self.failed(at, &why, None, lasting);
            }
            return None;
        };
        if another(op, &copy) {
            log::info!("jmap pim: a change of a contact was not written: {ANOTHER}");
            self.answer(at, EventResult::Gone, String::new(), ANOTHER, None, false);
            return None;
        }
        let body = match op.kind {
            ContactChangeKind::Delete => None,
            ContactChangeKind::Update => Some(merged_json(op, &copy)),
            ContactChangeKind::Create => {
                let again = resent(op);
                if again.fields.is_empty() {
                    // The card has what was sent, and nothing was changed here since.
                    self.answer(at, EventResult::Done, card_key(&card), "", None, false);
                    return None;
                }
                Some(merged_json(&again, &copy))
            }
        };
        Some(Ready { at, target: card, body })
    }

    /// New cards the server said it had already: each read, and what was changed here since it was sent made to it.
    async fn write_again(&mut self, size: usize) -> Result<(), Stop> {
        let again = std::mem::take(&mut self.again);
        if again.is_empty() {
            return Ok(());
        }
        for (_, card) in &again {
            self.copies.remove(card);
            self.unread.remove(card);
            self.gone.remove(card);
        }
        self.read_cards(again.iter().map(|(_, card)| card.clone()).collect()).await?;
        let mut ready: Vec<Ready> = Vec::new();
        for (at, card) in again {
            let Some(op) = self.ops.get(at).cloned() else { continue };
            if self.gone.contains(&card) {
                self.failed(at, "the card the server said it had is gone", None, false);
                continue;
            }
            if let Some(one) = self.made_to(at, &op, card) {
                ready.push(one);
            }
        }
        self.write(ready, size).await
    }

    /// The changes `ready` in calls of at most `size`, none writing twice to one card; a call the server refuses for
    /// its size (before running it) goes again in halves.
    async fn write(&mut self, ready: Vec<Ready>, size: usize) -> Result<(), Stop> {
        let mut work: Vec<Vec<Ready>> = ready.chunks(size).rev().map(<[Ready]>::to_vec).collect();
        while let Some(call) = work.pop() {
            self.check()?;
            if self.set(&call).await? {
                if call.len() > 1 && self.splits < MAX_SPLITS {
                    self.splits = self.splits.saturating_add(1);
                    let (first, second) = call.split_at(call.len() / 2);
                    work.push(second.to_vec());
                    work.push(first.to_vec());
                } else {
                    // A change alone too large for it: asking again won't make it smaller.
                    let alone = call.len() == 1;
                    for one in &call {
                        self.failed(one.at, "too many changes at once", None, alone);
                    }
                }
            }
        }
        Ok(())
    }

    /// One `ContactCard/set` of `call`, and its outcomes. `true` when the server refused it for its size before running
    /// it, and nothing of it was answered.
    async fn set(&mut self, call: &[Ready]) -> Result<bool, Stop> {
        if call.is_empty() {
            return Ok(false);
        }
        let (mut create, mut update, mut destroy) = (Map::new(), Map::new(), Vec::new());
        for one in call {
            match (self.ops.get(one.at).map(|op| op.kind), &one.body) {
                (Some(ContactChangeKind::Create), Some(body)) if one.target.is_empty() => {
                    create.insert(format!("k{}", one.at), body.clone());
                }
                (Some(_), Some(body)) => {
                    update.insert(one.target.clone(), body.clone());
                }
                (Some(_), None) => destroy.push(json!(one.target)),
                (None, _) => {}
            }
        }
        let mut args = json!({"accountId": self.account});
        if !create.is_empty() {
            args["create"] = Value::Object(create);
        }
        if !update.is_empty() {
            args["update"] = Value::Object(update);
        }
        if !destroy.is_empty() {
            args["destroy"] = Value::Array(destroy);
        }
        let affected: Vec<usize> = call.iter().map(|one| one.at).collect();
        let (status, answers) = self.client.call_with_status(&USING, vec![("ContactCard/set", args)], IDS_BYTES).await;
        let mut answers = match answers {
            Ok(answers) => answers,
            // Refused as a whole for its size, before it ran (a 4xx that says so; a 413, which a proxy before the server
            // says with no more): smaller calls may go.
            Err(Error::TooLarge(_)) if status.is_some_and(|status| (400..500).contains(&status)) => return Ok(true),
            Err(_) if status == Some(413) => return Ok(true),
            // Anything else (an answer too large to take, a connection lost): whether it went is not known. It goes
            // again: a new card with its mark, which makes no second one (and whose card may be on the server now); a
            // change made again to the card read anew; a deletion of a card that is gone, which is done.
            Err(error) => {
                let ran = !matches!(error, Error::Busy { .. } | Error::Auth(_));
                let went = self.request_failed(error, &affected);
                if ran {
                    for one in call.iter().filter(|one| one.target.is_empty()) {
                        self.made_is(one.at, CardMade::Perhaps);
                    }
                }
                return went.map(|()| false);
            }
        };
        let answer = match answers.take(0) {
            Ok(answer) => answer,
            Err(Error::Refused { kind, .. }) if kind == "requestTooLarge" => return Ok(true),
            Err(error) => {
                let words = said(&self.secrets, &error);
                let (result, retry) = match &error {
                    Error::Refused { kind, .. } if failed_for_now(kind) => (EventResult::Failed, Some(RETRY)),
                    Error::Refused { .. } => (EventResult::Refused, None),
                    _ => (EventResult::Failed, Some(RETRY)),
                };
                for one in call {
                    let remote = if result == EventResult::Refused { self.card_named(one) } else { String::new() };
                    self.answer(one.at, result, remote, &words, retry, false);
                }
                return Ok(false);
            }
        };
        let map = |key: &str| answer.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
        let (created, not_created, updated, not_updated, not_destroyed) =
            (map("created"), map("notCreated"), map("updated"), map("notUpdated"), map("notDestroyed"));
        let destroyed: HashSet<String> =
            answer.get("destroyed").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        for one in call {
            let Some(op) = self.ops.get(one.at).cloned() else { continue };
            let key = format!("k{}", one.at);
            let made = one.target.is_empty();
            let (done, error) = match (made, &one.body) {
                (true, _) => (created.get(&key).cloned(), not_created.get(&key).cloned()),
                (false, Some(_)) => (updated.contains_key(&one.target).then(|| json!({})), not_updated.get(&one.target).cloned()),
                (false, None) => (destroyed.contains(&one.target).then(|| json!({})), not_destroyed.get(&one.target).cloned()),
            };
            match (done, error) {
                (Some(done), _) => {
                    let remote = if made {
                        // (One the server named outside what it may call a card: the engine finds it at the next read.)
                        let id = done.get("id").and_then(Value::as_str).filter(|id| card_id(id));
                        if let Some(id) = id {
                            self.made.insert(id.to_string());
                        }
                        id.map(card_key).unwrap_or_default()
                    } else if op.kind == ContactChangeKind::Create {
                        // A new card the server had already, now with what was changed here since.
                        card_key(&one.target)
                    } else {
                        op.remote.clone()
                    };
                    self.answer(one.at, EventResult::Done, remote, "", None, false);
                }
                (None, Some(error)) => self.refused(one, &op, &error),
                (None, None) => {
                    self.failed(one.at, "the server's answer said nothing of it", None, true);
                    if made {
                        self.made_is(one.at, CardMade::Perhaps);
                    }
                }
            }
        }
        Ok(false)
    }

    /// The server didn't do the change `one`: `error` is its SetError (RFC 8620 §5.3).
    fn refused(&mut self, one: &Ready, op: &ContactOp, error: &Value) {
        let at = one.at;
        let kind = error.get("type").and_then(Value::as_str).unwrap_or_default();
        let words = error.get("description").and_then(Value::as_str).map(strip_tags).unwrap_or_default();
        let words = if words.trim().is_empty() { kind.to_string() } else { self.secrets.hide(&words) };
        if op.kind == ContactChangeKind::Create {
            // A card that is there already (a server that says so of one made before): that one, read, with what was
            // changed here since it was sent made to it.
            if one.target.is_empty()
                && kind == "alreadyExists"
                && let Some(existing) = error.get("existingId").and_then(Value::as_str).filter(|id| card_id(id))
            {
                self.made.insert(existing.to_string());
                self.again.push((at, existing.to_string()));
                return;
            }
            // The card made the first time, gone since it was found: made anew the next time.
            if !one.target.is_empty() && kind == "notFound" {
                self.failed(at, "the server no longer has the card it made of it", None, false);
                return;
            }
        }
        let retry = error.get("retryAfter").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
        // A new card the server refused to make (not one sent again as a change of the card it made of it).
        let new_card = op.kind == ContactChangeKind::Create && one.target.is_empty();
        match kind {
            "notFound" if !one.target.is_empty() => self.answer(at, EventResult::Gone, String::new(), &words, None, false),
            // The server failed with this very card: asking again may get past it once or twice, not for ever. So with
            // a new card it won't make for now (over its quota, say), unless it asks to be left alone a while.
            "serverFail" | "serverPartialFail" => self.failed(at, &words, retry, true),
            kind if failed_for_now(kind) => self.failed(at, &words, retry, new_card && !matches!(kind, "rateLimit" | "tooManyRequests")),
            _ => {
                let remote = self.card_named(one);
                self.answer(at, EventResult::Refused, remote, &words, None, false);
            }
        }
        // None was made of it, when it was never sent before or the server found none made with its mark: nothing
        // waits for it.
        if new_card && (op.base.is_none() || self.none_made.contains(&at)) {
            self.made_is(at, CardMade::No);
        }
    }

    /// Say what is known now of a card made of the new contact at `at`, in its answer.
    fn made_is(&mut self, at: usize, made: CardMade) {
        if let Some(Some(outcome)) = self.outcomes.get_mut(at) {
            outcome.made = made;
        }
    }

    /// For a new card sent again as a change of the card the server made of it (`one`): that card, as the engine names
    /// it, for a refusal to say (the contact is that card); else nothing.
    fn card_named(&self, one: &Ready) -> String {
        let resent = self.ops.get(one.at).is_some_and(|op| op.kind == ContactChangeKind::Create) && !one.target.is_empty();
        if resent { card_key(&one.target) } else { String::new() }
    }
}

/// Whether a server's refusal is one that asking again later may get past.
fn failed_for_now(kind: &str) -> bool {
    matches!(kind, "rateLimit" | "serverFail" | "serverUnavailable" | "serverPartialFail" | "tooManyRequests" | "overQuota")
}

/// Text from a server with its markup taken out.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match (c, in_tag) {
            ('<', _) => in_tag = true,
            ('>', true) => in_tag = false,
            (_, true) => {}
            (c, false) => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests;
