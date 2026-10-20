// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What was made, changed and deleted here of the contacts of an account's address books, written to its server: the
//! part of a job before its contacts are read ([`push`]), so that what is read shows it.
//!
//! 1. **The calls.** Each change is a create of a card whole, in its book and with the mark it was made with as its
//!    `uid` (sent again, it makes no second card); an update of the whole properties it names, a name always with its
//!    parts and its full form, never a part of a property (a server that keeps only the basics refuses `emails/e1`);
//!    or a destroy. As many in a call as the session takes in a `set`, [`MAX_PER_CALL`] at most, none writing twice to
//!    one card. A call the server finds too large is sent again in halves.
//! 2. **Outcomes**, one for each change tried, in the order they came: done (a new card with what the server calls it,
//!    as the engine names a card: [`card_key`](super::super::card_key)), gone, refused with the server's words (its
//!    markup taken out, [`MAX_WORDS`] characters at most), or failed with when to try again (a busy server, one that
//!    failed, a connection that broke, a change not reached). A card the server says is there already
//!    (`alreadyExists` with its id) is that card. A server that refuses the credentials ends the job: what it had not
//!    answered waits as it was.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::book::{ContactChangeKind, ContactOp, ContactOutcome, ContactsUploaded, MAX_CONTACT_OPS};
use mailcraft_model::remote::EventResult;
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use serde_json::{Map, Value, json};

use super::super::jscontact::card_json;
use super::super::{IDS_BYTES, Stop, card_id, card_key, said, secrets_of, server_card_id};
use crate::{CONTACTS, CORE, Client, Error, Http, Secrets};

/// Most cards in one `ContactCard/set`.
pub const MAX_PER_CALL: usize = 50;
/// The longest words of the server's kept, in characters.
pub const MAX_WORDS: usize = 200;
/// How long a server that failed is left alone before a change goes again, when it doesn't say, in seconds.
const RETRY: u32 = 60;
/// Most times in a job that a call is halved for being too large.
const MAX_SPLITS: usize = 16;
/// What every request names in `using`.
const USING: [&str; 2] = [CORE, CONTACTS];

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
    let mut writer =
        Writer { client, account, cancel, secrets: secrets_of(request, &client.secrets, account), outcomes: vec![None; ops.len()], ops, splits: 0 };
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
}

/// What the server calls the card a change is about, for a change or a deletion: `None` for a key that names none.
fn target(op: &ContactOp) -> Option<String> {
    server_card_id(&op.remote).filter(|id| card_id(id))
}

impl<H: Http> Writer<'_, H> {
    fn check(&self) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) { Err(Stop::Cancelled) } else { Ok(()) }
    }

    /// Say what became of the change at `at`, unless something was said already.
    fn answer(&mut self, at: usize, result: EventResult, remote: String, error: &str, retry_after: Option<u32>) {
        let Some(op) = self.ops.get(at) else { return };
        if let Some(slot) = self.outcomes.get_mut(at).filter(|slot| slot.is_none()) {
            let mut outcome = ContactOutcome { op: op.clone(), result, remote, error: crate::tidy(error, MAX_WORDS), retry_after };
            outcome.tidy();
            *slot = Some(outcome);
        }
    }

    fn failed(&mut self, at: usize, why: &str, retry_after: Option<u32>) {
        self.answer(at, EventResult::Failed, String::new(), why, Some(retry_after.unwrap_or(RETRY)));
    }

    /// Every change not answered yet: failed, to be tried again after `retry_after`.
    fn fail_the_rest(&mut self, why: &str, retry_after: Option<u32>) {
        for at in 0..self.ops.len() {
            self.failed(at, why, retry_after);
        }
    }

    async fn run(&mut self) -> Result<(), Stop> {
        // What can't be written as it is said.
        let mut ready: Vec<usize> = Vec::new();
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
                Some(why) => self.answer(at, EventResult::Refused, String::new(), why, None),
                None => ready.push(at),
            }
        }
        let size = MAX_PER_CALL.min(self.client.max_objects_in_set().max(1));
        let mut work: Vec<Vec<usize>> = calls(&self.ops, ready, size).into_iter().rev().collect();
        while let Some(call) = work.pop() {
            self.check()?;
            if self.set(&call).await? {
                // Too large for the server: in halves, until it takes them (or one alone is too large).
                if call.len() > 1 && self.splits < MAX_SPLITS {
                    self.splits = self.splits.saturating_add(1);
                    let (first, second) = call.split_at(call.len() / 2);
                    work.push(second.to_vec());
                    work.push(first.to_vec());
                } else {
                    for at in call {
                        self.failed(at, "too many changes at once", None);
                    }
                }
            }
        }
        Ok(())
    }

    /// One `ContactCard/set` of the changes at `call`, and their outcomes. `true` when the server found the call too
    /// large, and nothing of it was answered.
    async fn set(&mut self, call: &[usize]) -> Result<bool, Stop> {
        let (mut create, mut update, mut destroy) = (Map::new(), Map::new(), Vec::new());
        for at in call {
            let Some(op) = self.ops.get(*at) else { continue };
            match (op.kind, target(op)) {
                (ContactChangeKind::Create, _) => {
                    create.insert(format!("k{at}"), card_json(op));
                }
                (ContactChangeKind::Update, Some(id)) => {
                    update.insert(id, card_json(op));
                }
                (ContactChangeKind::Delete, Some(id)) => destroy.push(json!(id)),
                (_, None) => {}
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
        let mut answers = match self.client.call_within(&USING, vec![("ContactCard/set", args)], IDS_BYTES).await {
            Ok(answers) => answers,
            Err(Error::TooLarge(_)) => return Ok(true),
            // Whether it went is not known: it goes again (a new card with its mark, which makes no second one).
            Err(error) => {
                let words = said(&self.secrets, &error);
                return match Stop::of(error, &self.secrets) {
                    Stop::Auth => Err(Stop::Auth),
                    Stop::Busy(seconds, why) => {
                        self.fail_the_rest(&words, Some(seconds));
                        Err(Stop::Busy(seconds, why))
                    }
                    _ => {
                        for at in call {
                            self.failed(*at, &words, None);
                        }
                        Ok(false)
                    }
                };
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
                for at in call {
                    self.answer(*at, result, String::new(), &words, retry);
                }
                return Ok(false);
            }
        };
        let map = |key: &str| answer.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
        let (created, not_created, updated, not_updated, not_destroyed) =
            (map("created"), map("notCreated"), map("updated"), map("notUpdated"), map("notDestroyed"));
        let destroyed: HashSet<String> =
            answer.get("destroyed").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        for at in call {
            let Some(op) = self.ops.get(*at).cloned() else { continue };
            let id = target(&op).unwrap_or_default();
            let key = format!("k{at}");
            let (done, error) = match op.kind {
                ContactChangeKind::Create => (created.get(&key).cloned(), not_created.get(&key).cloned()),
                ContactChangeKind::Update => (updated.contains_key(&id).then(|| json!({})), not_updated.get(&id).cloned()),
                ContactChangeKind::Delete => (destroyed.contains(&id).then(|| json!({})), not_destroyed.get(&id).cloned()),
            };
            match (done, error) {
                (Some(done), _) => {
                    let remote = match op.kind {
                        // (One the server named outside what it may call a card: the engine finds it at the next read.)
                        ContactChangeKind::Create => {
                            done.get("id").and_then(Value::as_str).filter(|id| card_id(id)).map(card_key).unwrap_or_default()
                        }
                        _ => op.remote.clone(),
                    };
                    self.answer(*at, EventResult::Done, remote, "", None);
                }
                (None, Some(error)) => self.refused(*at, &op, &error),
                (None, None) => self.failed(*at, "the server's answer said nothing of it", None),
            }
        }
        Ok(false)
    }

    /// The server didn't do the change at `at`: `error` is its SetError (RFC 8620 §5.3).
    fn refused(&mut self, at: usize, op: &ContactOp, error: &Value) {
        let kind = error.get("type").and_then(Value::as_str).unwrap_or_default();
        let words = error.get("description").and_then(Value::as_str).map(strip_tags).unwrap_or_default();
        let words = if words.trim().is_empty() { kind.to_string() } else { self.secrets.hide(&words) };
        // A card that is there already (a server that says so of one made before): that one.
        if op.kind == ContactChangeKind::Create
            && kind == "alreadyExists"
            && let Some(existing) = error.get("existingId").and_then(Value::as_str).filter(|id| card_id(id))
        {
            self.answer(at, EventResult::Done, card_key(existing), "", None);
            return;
        }
        let retry = error.get("retryAfter").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
        match kind {
            "notFound" if op.kind != ContactChangeKind::Create => self.answer(at, EventResult::Gone, String::new(), &words, None),
            kind if failed_for_now(kind) => self.failed(at, &words, retry),
            _ => self.answer(at, EventResult::Refused, String::new(), &words, None),
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

/// The changes at `ready`, in calls of at most `size`, none of which writes twice to one card.
fn calls(ops: &[ContactOp], ready: Vec<usize>, size: usize) -> Vec<Vec<usize>> {
    let mut calls: Vec<Vec<usize>> = Vec::new();
    let of = |at: usize| ops.get(at).filter(|op| op.kind != ContactChangeKind::Create).and_then(target);
    for at in ready {
        let mine = of(at);
        let fits = |call: &Vec<usize>| call.len() < size && (mine.is_none() || !call.iter().any(|other| of(*other) == mine));
        match calls.iter_mut().find(|call| fits(call)) {
            Some(call) => call.push(at),
            None => calls.push(vec![at]),
        }
    }
    calls
}

#[cfg(test)]
mod tests;
