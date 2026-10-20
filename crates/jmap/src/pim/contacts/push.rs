// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What was made, changed and deleted here of the contacts of an account's address books, written to its server: the
//! part of a job before its contacts are read ([`push`]), so that what is read shows it.
//!
//! 1. **The server's card first.** Every card a change or a deletion is about is read (a `ContactCard/get` of their
//!    names). One the server no longer has is gone; one it made after the card the change is about (`created` is
//!    another than the change's mark: a server gave a new contact the name of one that was deleted) is another contact,
//!    and is never written to: the change is gone too.
//! 2. **The calls.** Each change is a create of a card whole, in its book and with the mark it was made with as its
//!    `uid` (sent again, it makes no second card); an update of the whole properties it names, made to the server's
//!    card as it is now ([`merged_json`]: the addresses and numbers added or taken out here, and those someone else
//!    added there meanwhile kept), a name always with its parts and its full form, never a part of a property (a
//!    server that keeps only the basics refuses `emails/e1`); or a destroy. As many in a call as the session takes in
//!    a `set`, [`MAX_PER_CALL`] at most; a card two changes are about takes the first, and the second waits for the
//!    next job. A call the server refuses for its size, before running it, is sent again in halves.
//! 3. **Outcomes**, one for each change tried, in the order they came: done (a new card with what the server calls it,
//!    as the engine names a card: [`card_key`](super::super::card_key)), gone, refused with the server's words (its
//!    markup taken out, [`MAX_WORDS`] characters at most), or failed with when to try again (a busy server, one that
//!    failed, a connection that broke, an answer too large to take, a change not reached). A failure that asking again
//!    is unlikely to get past (the server failed with this very card, said nothing of it, or a change alone is too
//!    large for it) says so ([`ContactOutcome::lasting`]). A card the server says is there already (`alreadyExists`
//!    with its id) is that card, and what was sent of it is written to it. A server that refuses the credentials, or is
//!    busy, ends the job: what was not tried waits as it was, with no outcome.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::book::{ContactChangeKind, ContactOp, ContactOutcome, ContactsUploaded, MAX_CONTACT_OPS};
use mailcraft_model::remote::EventResult;
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
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
/// What every request names in `using`.
const USING: [&str; 2] = [CORE, CONTACTS];
/// What the server's card is read for before a change or a deletion: what a change writes, and when the card was
/// made and last changed.
const CARD_PROPERTIES: [&str; 8] = ["id", "name", "emails", "phones", "organizations", "titles", "created", "updated"];
/// Why a change is not written: the card is another contact's now.
const ANOTHER: &str = "the server has another contact under its name now";

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
        copies: HashMap::new(),
        gone: HashSet::new(),
        unread: HashMap::new(),
        again: Vec::new(),
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
    /// The server's cards read, by their names; the names it said it doesn't have; and why others couldn't be read.
    copies: HashMap<String, Value>,
    gone: HashSet<String>,
    unread: HashMap<String, String>,
    /// New cards the server says it had already (`alreadyExists`): what was sent of them is written to the card it has.
    again: Vec<Ready>,
}

/// What the server calls the card a change is about, for a change or a deletion: `None` for a key that names none.
fn target(op: &ContactOp) -> Option<String> {
    server_card_id(&op.remote).filter(|id| card_id(id))
}

/// Whether the server's `card` is another contact than the one `op` is about: it was made at another time than the one
/// the change knows (a server gave a new contact the name of one that was deleted). Unknown either side: not another.
fn another(op: &ContactOp, card: &Value) -> bool {
    let made = card.get("created").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    !op.mark.is_empty() && !made.is_empty() && made != op.mark
}

impl<H: Http> Writer<'_, H> {
    fn check(&self) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) { Err(Stop::Cancelled) } else { Ok(()) }
    }

    /// Say what became of the change at `at`, unless something was said already.
    fn answer(&mut self, at: usize, result: EventResult, remote: String, error: &str, retry_after: Option<u32>, lasting: bool) {
        let Some(op) = self.ops.get(at) else { return };
        if let Some(slot) = self.outcomes.get_mut(at).filter(|slot| slot.is_none()) {
            let mut outcome = ContactOutcome { op: op.clone(), result, remote, error: crate::tidy(error, MAX_WORDS), retry_after, lasting };
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
        // What can't be written as it is said; and a card a change before it in this job writes to already.
        let mut ready: Vec<usize> = Vec::new();
        let mut written: HashSet<String> = HashSet::new();
        for at in 0..self.ops.len() {
            let Some(op) = self.ops.get(at) else { continue };
            let refusal = match op.kind {
                ContactChangeKind::Create if op.token.is_empty() => Some("the contact has no mark to be made with"),
                ContactChangeKind::Create => None,
                _ if target(op).is_none() => Some("the contact's name on the server can't be read"),
                ContactChangeKind::Update if op.fields.is_empty() => Some("the change says nothing that could be sent"),
                _ => None,
            };
            if let Some(why) = refusal {
                self.answer(at, EventResult::Refused, String::new(), why, None, false);
                continue;
            }
            // (The second waits for the next job, with no outcome: it is made to what the first leaves.)
            if let Some(card) = target(op).filter(|_| op.kind != ContactChangeKind::Create)
                && !written.insert(card)
            {
                continue;
            }
            ready.push(at);
        }
        // The server's cards the changes and deletions are about.
        let wanted: Vec<String> =
            ready.iter().filter_map(|at| self.ops.get(*at)).filter(|op| op.kind != ContactChangeKind::Create).filter_map(target).collect();
        self.read_cards(wanted).await?;
        // Each change made ready, or answered.
        let mut prepared: Vec<Ready> = Vec::new();
        for at in ready {
            if let Some(one) = self.prepare(at) {
                prepared.push(one);
            }
        }
        let size = MAX_PER_CALL.min(self.client.max_objects_in_set().max(1));
        self.write(prepared, size).await?;
        // New cards the server had already: what was sent of them, written to the card it has.
        let again = std::mem::take(&mut self.again);
        if !again.is_empty() {
            self.write(again, size).await?;
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
                    self.unread.extend(chunk.iter().map(|id| (id.clone(), words.clone())));
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
                            self.unread.insert(id.clone(), "the server's answer did not have it".into());
                        }
                    }
                }
                Err(error) => {
                    let words = said(&self.secrets, &error);
                    self.unread.extend(chunk.iter().map(|id| (id.clone(), words.clone())));
                }
            }
        }
        Ok(())
    }

    /// The changes not answered yet whose card is among `ids`.
    fn waiting_on(&self, ids: &[String]) -> Vec<usize> {
        (0..self.ops.len())
            .filter(|at| self.outcomes.get(*at).is_some_and(Option::is_none))
            .filter(|at| self.ops.get(*at).and_then(target).is_some_and(|card| ids.contains(&card)))
            .collect()
    }

    /// The change at `at` made ready for a call, or answered (`None`).
    fn prepare(&mut self, at: usize) -> Option<Ready> {
        let op = self.ops.get(at)?.clone();
        if op.kind == ContactChangeKind::Create {
            return Some(Ready { at, target: String::new(), body: Some(card_json(&op)) });
        }
        let card = target(&op)?;
        if self.gone.contains(&card) {
            self.answer(at, EventResult::Gone, String::new(), "the server no longer has the contact", None, false);
            return None;
        }
        let Some(copy) = self.copies.get(&card).cloned() else {
            // Not read now: it waits, and goes once the server's card is known.
            let why = self.unread.get(&card).cloned().unwrap_or_else(|| "the server's card couldn't be read".into());
            if self.outcomes.get(at).is_some_and(Option::is_none) {
                self.failed(at, &why, None, false);
            }
            return None;
        };
        if another(&op, &copy) {
            log::info!("jmap pim: a change of a contact was not written: {ANOTHER}");
            self.answer(at, EventResult::Gone, String::new(), ANOTHER, None, false);
            return None;
        }
        let body = match op.kind {
            ContactChangeKind::Delete => None,
            _ => Some(merged_json(&op, &copy)),
        };
        Some(Ready { at, target: card, body })
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
            // Refused as a whole for its size, before it ran (a 4xx): smaller calls may go.
            Err(Error::TooLarge(_)) if status.is_some_and(|status| (400..500).contains(&status)) => return Ok(true),
            // Anything else (an answer too large to take, a connection lost): whether it went is not known. It goes
            // again: a new card with its mark, which makes no second one; a change made again to the card read anew;
            // a deletion of a card that is gone, which is done.
            Err(error) => return self.request_failed(error, &affected).map(|()| false),
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
                for at in affected {
                    self.answer(at, result, String::new(), &words, retry, false);
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
                        done.get("id").and_then(Value::as_str).filter(|id| card_id(id)).map(card_key).unwrap_or_default()
                    } else if op.kind == ContactChangeKind::Create {
                        // A new card the server had already, now written as it was sent.
                        card_key(&one.target)
                    } else {
                        op.remote.clone()
                    };
                    self.answer(one.at, EventResult::Done, remote, "", None, false);
                }
                (None, Some(error)) => self.refused(one, &op, &error),
                (None, None) => self.failed(one.at, "the server's answer said nothing of it", None, true),
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
        // A card that is there already (a server that says so of one made before): that one, with what was sent
        // written to it.
        if op.kind == ContactChangeKind::Create
            && one.target.is_empty()
            && kind == "alreadyExists"
            && let Some(existing) = error.get("existingId").and_then(Value::as_str).filter(|id| card_id(id))
        {
            let mut again = op.clone();
            again.kind = ContactChangeKind::Update;
            again.base = None;
            self.again.push(Ready { at, target: existing.to_string(), body: Some(card_json(&again)) });
            return;
        }
        let retry = error.get("retryAfter").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
        match kind {
            "notFound" if !one.target.is_empty() => self.answer(at, EventResult::Gone, String::new(), &words, None, false),
            // The server failed with this very card: asking again may get past it once or twice, not for ever.
            "serverFail" | "serverPartialFail" => self.failed(at, &words, retry, true),
            kind if failed_for_now(kind) => self.failed(at, &words, retry, false),
            _ => self.answer(at, EventResult::Refused, String::new(), &words, None, false),
        }
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
