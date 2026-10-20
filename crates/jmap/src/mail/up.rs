// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The changes made to a JMAP account's messages, told to its server (RFC 8620 §5.3, RFC 8621 §4.6).
//!
//! The engine has worked out what differs between each message and what the server last said of it
//! ([`JmapChange`]); this makes those changes with `Email/set` and says what became of each
//! ([`JmapChangeDone`]). It runs first in a run, before the mailboxes are read, so that what is read is the
//! server after the changes.
//!
//! - **What is sent.** A message's marks (`$seen`, `$flagged`, `$answered`, `$forwarded`) as patches of
//!   `keywords`, only the ones that changed (`keywords/$seen: true`, or `null` to take it away); a move as
//!   patches of `mailboxIds` (the new mailbox `true`, the old one `null`); a delete for good as `destroy`. No
//!   other keyword of a message is named, and so none is touched. The marks of all the messages go first, then
//!   the moves and the deletes: a message's marks and its move are two updates, each made or refused by itself
//!   (a move into a mailbox that is full does not take the read mark with it), and the marks are made under the
//!   id the message has before the move changes it.
//! - **How much at once.** No more objects in a call than the session's `maxObjectsInSet` and `maxObjectsInGet`
//!   (the ids of the moved are read back with a `get`), never more than 500, one call to a request, one request
//!   after the other (a cancel is read before each). A call the server finds too large is halved.
//! - **What became of each change** is its own: `updated` is done; `notUpdated` and `notDestroyed` give a
//!   SetError, and `notFound` is *gone* (the server no longer has the message), a failure of the server's own
//!   (`serverFail`, `serverUnavailable`, `rateLimit`) is not an answer, and any other type is a refusal that
//!   says it, by its type. A change the server does not answer for, or a call that fails, is not in the
//!   result: the engine works it out again for the next run. A move that was not heard of (a call that failed
//!   without an error from the server, an answer that leaves it out, a failure of the server's own in the middle
//!   of it) may have been made all the same, and then a window of the mailbox it left would not list the message:
//!   so such a run is not whole (nothing is swept), unless the mailbox it was to go to is one the run's own listing
//!   of the mailboxes does not have. A move the server refused or could not make for being asked too often
//!   (`rateLimit`), a mark, or a delete that waits changes nothing the windows say.
//! - **A move's new id.** Some servers name a message by where it lies, so that a move gives it another id.
//!   When the answer names it (`updated: {old: {"id": new}}`) that is the id. When it does not, the ids of the
//!   moved messages are asked for, in the same request: an id that is still there is the message's, one that
//!   is not has changed to one nobody has said, and the outcome has an empty id (the engine finds the message
//!   again in the mailbox it was moved to, by its `Message-ID`).
//! - **A server that will not.** A call refused as a whole because the account cannot be changed (a read-only
//!   account, a method the server lacks) refuses every change in it, by the type of the error. The server
//!   busy, the credentials refused, a cancel, end the run, with what was answered handed over before.
//!
//! Everything a server answers is hostile: only the ids that were sent are looked up in what it says, a
//! SetError's text is never shown (only its type, cut to letters), and what is listed is counted to no more
//! than was asked.

use std::collections::{HashMap, HashSet};

use mailcraft_model::sync::{JmapChange, JmapChangeDone, JmapMailResult, JmapOutcome, MAX_MESSAGE_CHANGES};
use serde_json::{Map, Value, json};

use super::{MAX_SPLITS, Outcome, Run, Stop, WINDOW_BYTES};
use crate::{Error, Http, valid_id};

/// Most objects the host puts in one call, whatever the session says it takes.
const MAX_SET: usize = 500;
/// The most an answer to a call may be: the answers of 500 objects, and the ids of those moved.
const SET_BYTES: usize = WINDOW_BYTES;
/// Longest type of a SetError that is kept.
const MAX_TYPE: usize = 40;
/// Method errors that say the account's mail cannot be changed, so that every change of the call is refused.
const NO_CHANGES: [&str; 4] = ["accountReadOnly", "forbidden", "accountNotSupportedByMethod", "unknownMethod"];

/// An update of one message, as the changes of a request make it: its marks, or its move.
struct Update {
    id: String,
    /// The keywords to set (`true`) or take away (`false`), in the order the changes named them.
    keywords: Vec<(&'static str, bool)>,
    /// The mailbox it is taken out of and the one it is put in.
    moved: Option<(String, String)>,
    /// Where, in the request's list of changes, the changes this makes are.
    at: Vec<usize>,
}

impl Update {
    /// The patch (RFC 8620 §5.3): one entry for each keyword, and for a move the entry that puts the
    /// message in the new mailbox and the one that takes it out of the old.
    fn patch(&self) -> Value {
        let mut patch = Map::new();
        for (name, on) in &self.keywords {
            patch.insert(format!("keywords/{name}"), if *on { Value::Bool(true) } else { Value::Null });
        }
        if let Some((from, to)) = &self.moved {
            patch.insert(format!("mailboxIds/{to}"), Value::Bool(true));
            patch.insert(format!("mailboxIds/{from}"), Value::Null);
        }
        Value::Object(patch)
    }
}

/// A message to destroy, and where, in the request's list of changes, the changes that ask it are.
struct Destroy {
    id: String,
    at: Vec<usize>,
}

/// One of the objects of a call.
enum Item {
    Update(Update),
    Destroy(Destroy),
}

/// What the server made of an object.
enum Verdict {
    /// It no longer has the message.
    Gone,
    /// It could not just now: the change waits.
    Later(String),
    /// It will not, and says why by a type.
    Refused(String),
}

/// A SetError's `type`, as a token: letters and digits only, no longer than [`MAX_TYPE`].
fn token(kind: &str) -> String {
    let kept: String = kind.chars().filter(char::is_ascii_alphanumeric).take(MAX_TYPE).collect();
    if kept.is_empty() { "unknown".into() } else { kept }
}

fn verdict(error: &Value) -> Verdict {
    let kind = token(error.get("type").and_then(Value::as_str).unwrap_or_default());
    match kind.as_str() {
        "notFound" => Verdict::Gone,
        "rateLimit" | "serverFail" | "serverUnavailable" | "serverPartialFail" => Verdict::Later(kind),
        _ => Verdict::Refused(kind),
    }
}

/// What a change that the server made comes to: a move says the id the message has now.
fn done(change: &JmapChange, id_now: &str) -> JmapOutcome {
    match change {
        JmapChange::Move { .. } => JmapOutcome::Done { id: id_now.to_string() },
        _ => JmapOutcome::Done { id: String::new() },
    }
}

/// Put `outcome` for each of the changes at `at`.
fn put(settled: &mut [Option<JmapOutcome>], at: &[usize], outcome: impl Fn(usize) -> JmapOutcome) {
    for a in at {
        if let Some(slot) = settled.get_mut(*a) {
            *slot = Some(outcome(*a));
        }
    }
}

/// Settle the changes at `at` as `verdict` says; a change that waits is told in `out`.
fn settle(verdict: Verdict, at: &[usize], settled: &mut [Option<JmapOutcome>], out: &mut Outcome) {
    match verdict {
        Verdict::Gone => put(settled, at, |_| JmapOutcome::Gone),
        Verdict::Refused(why) => put(settled, at, |_| JmapOutcome::Refused { why: why.clone() }),
        Verdict::Later(kind) => {
            out.up.1.get_or_insert_with(|| format!("the server could not make them just now ({kind})"));
        }
    }
}

/// The objects the changes come to, in the two goes they are made in (the marks, then the moves and the deletes),
/// with the changes that need nothing from a server settled at once: a change about an id that cannot be a
/// server's is refused, one that names no mark or the same mailbox twice is done.
fn plan(changes: &[&JmapChange], settled: &mut [Option<JmapOutcome>]) -> (Vec<Item>, Vec<Item>) {
    let (mut marks, mut moves, mut destroys): (Vec<Update>, Vec<Update>, Vec<Destroy>) = (Vec::new(), Vec::new(), Vec::new());
    let (mut marks_of, mut moves_of, mut destroy_of): (HashMap<&str, usize>, HashMap<&str, usize>, HashMap<&str, usize>) =
        (HashMap::new(), HashMap::new(), HashMap::new());
    let mut mark = |at: usize, outcome: JmapOutcome| {
        if let Some(slot) = settled.get_mut(at) {
            *slot = Some(outcome);
        }
    };
    for (at, change) in changes.iter().enumerate() {
        let id = change.id();
        if !valid_id(id) {
            mark(at, JmapOutcome::Refused { why: "invalidId".into() });
            continue;
        }
        match change {
            JmapChange::Keywords { read, flagged, answered, forwarded, .. } => {
                let keywords = [("$seen", *read), ("$flagged", *flagged), ("$answered", *answered), ("$forwarded", *forwarded)];
                let keywords: Vec<(&'static str, bool)> = keywords.into_iter().filter_map(|(name, on)| on.map(|on| (name, on))).collect();
                if keywords.is_empty() {
                    mark(at, JmapOutcome::Done { id: String::new() });
                    continue;
                }
                let n = *marks_of.entry(id).or_insert_with(|| {
                    marks.push(Update { id: id.to_string(), keywords: Vec::new(), moved: None, at: Vec::new() });
                    marks.len().saturating_sub(1)
                });
                if let Some(update) = marks.get_mut(n) {
                    update.keywords.extend(keywords);
                    update.at.push(at);
                }
            }
            JmapChange::Move { from, to, .. } => {
                if !valid_id(from) || !valid_id(to) {
                    mark(at, JmapOutcome::Refused { why: "invalidId".into() });
                } else if from == to {
                    mark(at, JmapOutcome::Done { id: id.to_string() });
                } else {
                    let n = *moves_of.entry(id).or_insert_with(|| {
                        moves.push(Update { id: id.to_string(), keywords: Vec::new(), moved: None, at: Vec::new() });
                        moves.len().saturating_sub(1)
                    });
                    if let Some(update) = moves.get_mut(n) {
                        update.moved = Some((from.clone(), to.clone()));
                        update.at.push(at);
                    }
                }
            }
            JmapChange::Destroy { .. } => {
                let n = *destroy_of.entry(id).or_insert_with(|| {
                    destroys.push(Destroy { id: id.to_string(), at: Vec::new() });
                    destroys.len().saturating_sub(1)
                });
                if let Some(destroy) = destroys.get_mut(n) {
                    destroy.at.push(at);
                }
            }
        }
    }
    (marks.into_iter().map(Item::Update).collect(), moves.into_iter().map(Item::Update).chain(destroys.into_iter().map(Item::Destroy)).collect())
}

impl<H: Http> Run<'_, H> {
    /// Make the changes of the request, and hand on what became of each in a piece of its own, before anything
    /// is read. Nothing is asked of the server, and no piece is handed on, when the request holds none. An
    /// error is what ends the run (a cancel, the credentials refused, a busy server); trouble that is the
    /// changes' alone is told in `out` and the run goes on.
    pub(super) async fn make_changes(&self, out: &mut Outcome, emit: &mut dyn FnMut(JmapMailResult)) -> Result<(), Stop> {
        let changes: Vec<&JmapChange> = self.request.changes.iter().take(MAX_MESSAGE_CHANGES).collect();
        if changes.is_empty() {
            return Ok(());
        }
        let mut settled: Vec<Option<JmapOutcome>> = vec![None; changes.len()];
        let (marks, rest) = plan(&changes, &mut settled);
        // (The moved ids are read back with a `get`, which may take fewer objects than a `set`.)
        let size = self.client.max_objects_in_set().min(self.client.max_objects_in_get()).clamp(1, MAX_SET);
        let result = async {
            // The marks first: they are made under the ids the messages have before a move changes them. A server that
            // fails them is not asked for the rest.
            if self.send_items(&marks, size, &changes, &mut settled, out).await? {
                self.send_items(&rest, size, &changes, &mut settled, out).await?;
            }
            Ok(())
        }
        .await;
        // What was answered is handed on, whatever ended the work.
        let answered: Vec<JmapChangeDone> = changes
            .iter()
            .zip(settled.iter())
            .filter_map(|(change, outcome)| outcome.as_ref().map(|outcome| JmapChangeDone { change: (*change).clone(), outcome: outcome.clone() }))
            .collect();
        let waiting = settled.iter().filter(|outcome| outcome.is_none()).count();
        if result.is_ok() && waiting > 0 {
            out.up.0 = waiting;
            out.up.1.get_or_insert_with(|| "the server did not take them".to_string());
        }
        log::debug!("jmap changes: {} of {} answered", answered.len(), changes.len());
        if !answered.is_empty() {
            emit(self.piece(JmapMailResult { changes: answered, partial: true, ..Default::default() }));
        }
        result
    }

    /// The objects in calls of at most `size`, one call after the other. Whether they all went through: `false` when
    /// the server failed them, and what is left of them waits for the next run.
    async fn send_items(
        &self,
        items: &[Item],
        size: usize,
        changes: &[&JmapChange],
        settled: &mut [Option<JmapOutcome>],
        out: &mut Outcome,
    ) -> Result<bool, Stop> {
        let mut work: Vec<&[Item]> = items.chunks(size).rev().collect();
        while let Some(part) = work.pop() {
            self.check()?;
            match self.set(part, changes, settled, out).await {
                Ok(()) => {}
                // Too much for the server, whatever its session said: half and half.
                Err(Stop::Failed(Error::TooLarge(_))) if part.len() > 1 && out.splits < MAX_SPLITS => {
                    out.splits = out.splits.saturating_add(1);
                    let (first, second) = part.split_at(part.len() / 2);
                    work.push(second);
                    work.push(first);
                }
                // The rest wait for the next run; the mailboxes are read all the same.
                Err(Stop::Failed(error) | Stop::Down(error)) => {
                    log::info!("jmap changes: a call failed; the rest wait for the next run");
                    // A call the server answered with an error was not made (but for `serverPartialFail`: some of it was);
                    // any other may have been, without a word back.
                    if !matches!(&error, Error::Refused { kind, .. } if kind != "serverPartialFail") {
                        for item in part {
                            if let Item::Update(Update { moved: Some((_, to)), .. }) = item {
                                out.unsure_to.push(to.clone());
                            }
                        }
                    }
                    out.up.1.get_or_insert_with(|| self.said(&error));
                    return Ok(false);
                }
                Err(stop) => return Err(stop),
            }
        }
        Ok(true)
    }

    /// One call: the updates and destroys of `part`, and, when some of them move messages, a look at the
    /// moved ones by their old ids in the same request. What each came to is put in `settled`.
    async fn set(&self, part: &[Item], changes: &[&JmapChange], settled: &mut [Option<JmapOutcome>], out: &mut Outcome) -> Result<(), Stop> {
        let account = self.client.account_id();
        let (mut update, mut destroy, mut moved) = (Map::new(), Vec::new(), Vec::new());
        for item in part {
            match item {
                Item::Update(u) => {
                    update.insert(u.id.clone(), u.patch());
                    if u.moved.is_some() {
                        moved.push(u.id.as_str());
                    }
                }
                Item::Destroy(d) => destroy.push(d.id.as_str()),
            }
        }
        let mut args = Map::new();
        args.insert("accountId".into(), json!(account));
        if !update.is_empty() {
            args.insert("update".into(), Value::Object(update));
        }
        if !destroy.is_empty() {
            args.insert("destroy".into(), json!(destroy));
        }
        let mut calls = vec![("Email/set", Value::Object(args))];
        let look = !moved.is_empty() && self.client.max_calls_in_request() >= 2;
        if look {
            calls.push(("Email/get", json!({"accountId": account, "ids": moved, "properties": ["id"]})));
        }
        let mut answers = self.client.mail_within(calls, SET_BYTES).await.map_err(|e| self.call_stop(e))?;
        let said = match answers.take(0) {
            Ok(said) => said,
            Err(error) => return self.set_failed(part, error, settled),
        };
        // The moved messages that are still there under the id they had: those ids do not change with a move.
        let there = if look { answers.take(1).ok() } else { None };
        let stable: Option<HashSet<&str>> = there.as_ref().map(|got| {
            let list = got.get("list").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            list.iter().take(moved.len().saturating_mul(2)).filter_map(|email| email.get("id").and_then(Value::as_str)).collect()
        });
        let updated = said.get("updated").and_then(Value::as_object);
        let not_updated = said.get("notUpdated").and_then(Value::as_object);
        let not_destroyed = said.get("notDestroyed").and_then(Value::as_object);
        let destroyed: HashSet<&str> = said
            .get("destroyed")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .take(part.len().saturating_mul(2))
            .filter_map(Value::as_str)
            .collect();
        for item in part {
            match item {
                Item::Update(u) => {
                    if let Some(error) = not_updated.and_then(|members| members.get(&u.id)) {
                        let verdict = verdict(error);
                        // A server that fails after it has begun (it lost its mailbox's connection when the move was made) says
                        // as much by these; one that is asked too often (`rateLimit`) has not begun.
                        if let Some((_, to)) = &u.moved
                            && matches!(&verdict, Verdict::Later(kind) if kind != "rateLimit")
                        {
                            out.unsure_to.push(to.clone());
                        }
                        settle(verdict, &u.at, settled, out);
                    } else if let Some(entry) = updated.and_then(|members| members.get(&u.id)) {
                        let now = match entry.get("id").and_then(Value::as_str).filter(|id| valid_id(id)) {
                            Some(named) => named.to_string(),
                            None if stable.as_ref().is_some_and(|stable| stable.contains(u.id.as_str())) => u.id.clone(),
                            None => String::new(),
                        };
                        put(settled, &u.at, |a| changes.get(a).map_or(JmapOutcome::Done { id: String::new() }, |change| done(change, &now)));
                    } else if let Some((_, to)) = &u.moved {
                        // No word of it: it may have been made all the same, and a window of the mailbox it left would not list it.
                        out.unsure_to.push(to.clone());
                    }
                }
                Item::Destroy(d) => {
                    if let Some(error) = not_destroyed.and_then(|members| members.get(&d.id)) {
                        settle(verdict(error), &d.at, settled, out);
                    } else if destroyed.contains(d.id.as_str()) {
                        put(settled, &d.at, |_| JmapOutcome::Done { id: String::new() });
                    }
                }
            }
        }
        Ok(())
    }

    /// The server answered the call with an error. One that says the account cannot be changed refuses every
    /// change in it; a call that was too large is for the caller to halve; any other is trouble that ends the
    /// changes of this run.
    fn set_failed(&self, part: &[Item], error: Error, settled: &mut [Option<JmapOutcome>]) -> Result<(), Stop> {
        match &error {
            Error::Refused { kind, .. } if NO_CHANGES.contains(&kind.as_str()) => {
                let why = token(kind);
                for item in part {
                    let at = match item {
                        Item::Update(u) => &u.at,
                        Item::Destroy(d) => &d.at,
                    };
                    for a in at {
                        if let Some(slot) = settled.get_mut(*a) {
                            *slot = Some(JmapOutcome::Refused { why: why.clone() });
                        }
                    }
                }
                Ok(())
            }
            Error::Refused { kind, .. } if kind == "requestTooLarge" => {
                Err(Stop::Failed(Error::TooLarge("the server takes fewer objects in a call".into())))
            }
            _ => Err(Self::method_stop(error)),
        }
    }
}

#[cfg(test)]
mod tests;
