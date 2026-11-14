// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What changed in a JMAP account's mail since a state the engine keeps (RFC 8620 §5.2, RFC 8621 §4.3), asked instead of
//! reading every window again.
//!
//! A run whose request carries a state ([`JmapMailRequest::email_state`]), from a run that read all of the mail or took
//! in all that changed, asks the server what became of the emails since, in every mailbox at once:
//!
//! - **One request**, when nothing waits to be sent or filed first: `Mailbox/get` (the run's listing, as always),
//!   `Email/changes` from the state ([`MAX_CHANGES`] at most), `Email/get` of the `created` by a back-reference, with what
//!   a message list shows of them, and `Email/get` of the `updated`, their keywords and mailboxes. With something to send
//!   first, the mailboxes are listed before it, and the changes are a request of three calls after it, so that the copies
//!   filed come with them. An answer too large for one request is asked for again without the created emails, which are
//!   then read [`MAX_CHUNK`](super::MAX_CHUNK) at a time, halved as a window's are when a call for them fails.
//! - **What is taken in.** A created email goes whole into the mailbox whose window would list it ([`home`]), and so does
//!   an updated one the engine hasn't got (it was moved from a mailbox no window of the engine reaches, or is older than
//!   the window): one it has goes as its marks into that mailbox, which also moves it there. A destroyed one is in the
//!   piece's [`JmapMailResult::destroyed`]: the engine marks it gone wherever it lies, and removes it when the run is
//!   whole. On a server that names an email by where it lies, a move is the old id destroyed and a new one created: the
//!   engine finds the message again by its `Message-ID` where it was moved here, and otherwise it is a new message.
//!   Each round of changes is a piece, never [`MessagePage::afresh`]: nothing is taken for gone that a destroy did not name.
//! - **More.** `hasMoreChanges`: the next round asks from the `newState` of this one, [`MAX_ROUNDS`] at most; past them, every
//!   window is read, which costs less than so many changes. The state of the last round is the run's when it is whole.
//! - **When the server can't say.** `cannotCalculateChanges` (a state it doesn't know, or too old, or more changes in one go
//!   than were asked for), a `newState` that can't be asked about (`"-"`), or an answer that is not one: every window is read,
//!   as a run that reads them all does, and the state of its first window is the run's. A server that has no such method
//!   (`unknownMethod`), or will not take the state (`invalidArguments`): the windows of the request's scope are read, and the
//!   engine forgets the state. Any other failure ends the run, which is then not whole: the next one asks from the same
//!   state (a change is heard twice rather than missed).
//! - **Emails that can't be had.** One the server can not give whole even alone is said in a line and passed over, without
//!   holding the state back (it would be asked for again at every run, and the changes since with it); one an answer leaves
//!   out without saying it is gone, or a call given up on, holds it back: the run is not whole.
//!
//! What comes back is hostile, as everywhere in a run: ids are held to the alphabet of an id, lists to what was asked for.

use std::collections::HashSet;

use mailcraft_model::sync::{GraphMessage, JmapMailResult, JmapMailState, MessagePage};
use serde_json::{Value, json};

use super::message::{self, Marks, home};
use super::{EMAIL_PROPERTIES, LIST_BYTES, MAILBOX_PROPERTIES, Outcome, Places, Run, Stop, WINDOW_PROPERTIES, halvable};
use crate::{Error, Http, result_of, valid_id};

/// The most changes one `Email/changes` is asked for (and never more than the server's `maxObjectsInGet`: the created and
/// the updated are read back by one `get` each). Fewer may make a server refuse a state whose changes came together.
pub(super) const MAX_CHANGES: usize = 500;
/// The calls of one round: a server that takes fewer to a request is read window by window.
pub(super) const CALLS: usize = 3;
/// The most rounds of changes in a run.
const MAX_ROUNDS: usize = 10;
/// The most an answer of a round may be, in bytes: the created emails' header fields (500 of them), and their marks
/// alone when the created emails are read apart.
const CHANGES_BYTES: usize = 12 << 20;
const MARKS_BYTES: usize = 4 << 20;
/// The answer to a call for emails by their ids, whole.
const WHOLE_BYTES: usize = 8 << 20;

/// What became of asking what changed.
pub(super) enum Followed {
    /// All of it was taken in: no window is read.
    Taken,
    /// The server can't say: every window is read.
    ReadAll,
    /// The server has no such thing: the windows of the request's scope are read, and the state is forgotten.
    Unknown,
    /// The changes could not be had (an error of the server's, an answer that is none): the windows of the request's scope
    /// are read, the state is kept, and the run is not whole.
    Failed(Error),
}

/// The answers of one round: `Email/changes`, `Email/get` of the created (`None`: they are read apart), of the updated.
pub(super) struct Round {
    changes: Result<Value, Error>,
    created: Option<Result<Value, Error>>,
    updated: Result<Value, Error>,
}

/// What one round says.
enum Took {
    /// Taken in, to this state, and nothing more is to come.
    All(String),
    /// Taken in, to this state, and more is to come from it.
    More(String),
    Cannot,
    Unknown,
}

impl<H: Http> Run<'_, H> {
    /// `Mailbox/get` and the first round of changes since `since`, in one request: the mailboxes, and the round. An
    /// answer too large, or a server that takes too few calls to a request: the mailboxes alone, and no round.
    pub(super) async fn list_and_changes(&self, since: &str, out: &mut Outcome) -> Result<(Option<Places>, Option<Round>), Stop> {
        self.check()?;
        if self.client.max_calls_in_request() <= CALLS {
            return Ok((self.list(out).await?, None));
        }
        let mut calls = vec![("Mailbox/get", json!({"accountId": self.client.account_id(), "properties": MAILBOX_PROPERTIES}))];
        calls.extend(self.round_calls(since, 1, false));
        match self.client.mail_within(calls, LIST_BYTES.saturating_add(CHANGES_BYTES)).await {
            Ok(mut answers) => {
                let places = self.places_of(answers.take(0), out)?;
                let round = Round { changes: answers.take(1), created: Some(answers.take(2)), updated: answers.take(3) };
                Ok((places, Some(round)))
            }
            Err(error) => match self.changes_stop(error) {
                Stop::Failed(_) => {
                    self.lean.set(true);
                    Ok((self.list(out).await?, None))
                }
                stop => Err(stop),
            },
        }
    }

    /// Why a request of changes failed as a whole: as for any request ([`Run::call_stop`]), but a server that answered and
    /// refused it (a gateway that will not take one of its calls, say) is this step's alone, as an answer too large is.
    fn changes_stop(&self, error: Error) -> Stop {
        match error {
            Error::Bad(_) | Error::Refused { .. } => Stop::Failed(error),
            other => self.call_stop(other),
        }
    }

    /// Take in what changed since `since`, the first round's answers `first` when they came with the mailboxes.
    pub(super) async fn follow(
        &self,
        since: &str,
        first: Option<Round>,
        places: Option<&Places>,
        out: &mut Outcome,
        emit: &mut dyn FnMut(JmapMailResult),
    ) -> Result<Followed, Stop> {
        let mut since = since.to_string();
        let mut next = first;
        for _ in 0..MAX_ROUNDS {
            let round = match next.take() {
                Some(round) => round,
                None => match self.round(&since).await {
                    Ok(round) => round,
                    Err(Stop::Failed(error)) => return Ok(Followed::Failed(error)),
                    Err(stop) => return Err(stop),
                },
            };
            let took = match self.take_round(&since, round, places, out, emit).await {
                Ok(took) => took,
                Err(Stop::Failed(error)) => return Ok(Followed::Failed(error)),
                Err(stop) => return Err(stop),
            };
            match took {
                Took::All(state) => {
                    *self.state.borrow_mut() = Some(state);
                    return Ok(Followed::Taken);
                }
                Took::More(state) => since = state,
                Took::Cannot => {
                    log::info!("jmap mail: the server can't say what changed since the state kept; every window is read");
                    return Ok(Followed::ReadAll);
                }
                Took::Unknown => {
                    log::info!("jmap mail: the server says no changes; the windows are read");
                    return Ok(Followed::Unknown);
                }
            }
        }
        log::info!("jmap mail: more changes than {MAX_ROUNDS} rounds take; every window is read");
        Ok(Followed::ReadAll)
    }

    /// The calls of a round, the first of them being call number `at` of its request: the created emails are asked for
    /// whole unless `lean`.
    fn round_calls(&self, since: &str, at: usize, lean: bool) -> Vec<(&'static str, Value)> {
        let account = self.client.account_id();
        let mut calls = vec![("Email/changes", json!({"accountId": account, "sinceState": since, "maxChanges": self.max_changes}))];
        if !lean {
            let created = result_of(at, "Email/changes", "/created");
            calls.push(("Email/get", json!({"accountId": account, "#ids": created, "properties": EMAIL_PROPERTIES})));
        }
        let updated = result_of(at, "Email/changes", "/updated");
        calls.push(("Email/get", json!({"accountId": account, "#ids": updated, "properties": WINDOW_PROPERTIES})));
        calls
    }

    /// A round of its own: asked again without the created emails when its answer is too large, and so from then on in
    /// the run.
    async fn round(&self, since: &str) -> Result<Round, Stop> {
        loop {
            self.check()?;
            let lean = self.lean.get();
            let most = if lean { MARKS_BYTES } else { CHANGES_BYTES };
            match self.client.mail_within(self.round_calls(since, 0, lean), most).await {
                Ok(mut answers) if lean => return Ok(Round { changes: answers.take(0), created: None, updated: answers.take(1) }),
                Ok(mut answers) => return Ok(Round { changes: answers.take(0), created: Some(answers.take(1)), updated: answers.take(2) }),
                Err(error) => match self.changes_stop(error) {
                    Stop::Failed(_) if !lean => self.lean.set(true),
                    stop => return Err(stop),
                },
            }
        }
    }

    /// Take in one round asked from `since`: a piece with what it brought, and what comes next.
    async fn take_round(
        &self,
        since: &str,
        round: Round,
        places: Option<&Places>,
        out: &mut Outcome,
        emit: &mut dyn FnMut(JmapMailResult),
    ) -> Result<Took, Stop> {
        let changes = match round.changes {
            Ok(changes) => changes,
            Err(Error::Refused { kind, .. }) if kind == "cannotCalculateChanges" => return Ok(Took::Cannot),
            Err(Error::Refused { kind, .. }) if kind == "unknownMethod" || kind == "invalidArguments" => return Ok(Took::Unknown),
            Err(error) => return Err(Self::method_stop(error)),
        };
        let Some(state) = changes.get("newState").and_then(Value::as_str).filter(|state| JmapMailState::usable(state)) else {
            return Ok(Took::Cannot);
        };
        let more = changes.get("hasMoreChanges").and_then(Value::as_bool).unwrap_or(false);
        // (More, and the same state: a server that would be asked forever.)
        if more && state == since {
            return Ok(Took::Cannot);
        }
        // The ids of a list, each once, if it is one and holds no more than were asked for.
        let ids = |key: &str| -> Option<Vec<&str>> {
            let list = changes.get(key)?.as_array().filter(|list| list.len() <= self.max_changes)?;
            let mut seen = HashSet::new();
            Some(list.iter().filter_map(Value::as_str).filter(|id| valid_id(id) && seen.insert(*id)).collect())
        };
        let (Some(mut created), Some(mut updated), Some(destroyed)) = (ids("created"), ids("updated"), ids("destroyed")) else {
            return Ok(Took::Cannot);
        };
        // An id is in one list (RFC 8620 §5.2): gone wins, then made.
        created.retain(|id| !destroyed.contains(id));
        updated.retain(|id| !destroyed.contains(id) && !created.contains(id));
        let claimable = places.map(|places| &places.claimable);
        let mut found: Vec<(String, GraphMessage)> = Vec::new();
        // To be read whole: the created emails the round has not brought, and the updated ones the engine hasn't got.
        let mut whole: Vec<&str> = Vec::new();
        let whole_of =
            |id: &str, mailbox: &str, email: &Value| (mailbox.to_string(), message::whole(id, mailbox, email, Marks::of(email.get("keywords"))));
        match &round.created {
            Some(Ok(answer)) => {
                if taken(answer, &created, claimable, out, |id, mailbox, email| found.push(whole_of(id, mailbox, email))).is_none() {
                    whole.extend(&created);
                }
            }
            // (The call failed, or the round left it out: they are read apart.)
            _ => whole.extend(&created),
        }
        match &round.updated {
            Ok(answer) => {
                // (One the engine has in another mailbox comes whole too: that mailbox may be gone with this round's listing.)
                let listed = taken(answer, &updated, claimable, out, |id, mailbox, email| {
                    if self.has_in(id, mailbox) {
                        found.push((mailbox.to_string(), message::known(id, mailbox, Marks::of(email.get("keywords")))));
                    } else {
                        whole.push(id);
                    }
                });
                if listed.is_none() {
                    self.short(out, updated.len(), "the server's answer did not have them");
                }
            }
            Err(error) => match Self::method_stop(error.clone()) {
                Stop::Failed(error) => self.short(out, updated.len(), &self.said(&error)),
                stop => return Err(stop),
            },
        }
        found.extend(self.read_whole(&whole, claimable, out).await?);
        let mut pages: Vec<MessagePage> = Vec::new();
        for (mailbox, message) in found {
            match pages.iter_mut().find(|page| page.folder == mailbox) {
                Some(page) => page.messages.push(message),
                None => pages.push(MessagePage { folder: mailbox, messages: vec![message], ..Default::default() }),
            }
        }
        log::debug!(
            "jmap mail: changes since the state kept: {} created, {} updated, {} destroyed{}",
            created.len(),
            updated.len(),
            destroyed.len(),
            if more { ", and more to come" } else { "" }
        );
        if !pages.is_empty() || !destroyed.is_empty() {
            let destroyed = destroyed.iter().map(|id| id.to_string()).collect();
            emit(self.piece(JmapMailResult { pages, destroyed, partial: true, ..Default::default() }));
        }
        Ok(if more { Took::More(state.to_string()) } else { Took::All(state.to_string()) })
    }

    /// Whether the engine has the email `id` in `mailbox`.
    fn has_in(&self, id: &str, mailbox: &str) -> bool {
        self.known.get(mailbox).is_some_and(|ids| ids.contains(id))
    }

    /// `count` emails a page lacks though the server did not say they are gone: the run is not whole.
    fn short(&self, out: &mut Outcome, count: usize, why: &str) {
        if count > 0 {
            out.details.0 = out.details.0.saturating_add(count);
            out.details.1.get_or_insert_with(|| why.to_string());
        }
    }

    /// The emails `wanted`, whole, each with the mailbox whose window would list it: [`MAX_CHUNK`](super::MAX_CHUNK) to a
    /// call (and no more than the server's `maxObjectsInGet`), a call that fails halved until the email that breaks it is
    /// found, which is passed over and said.
    async fn read_whole(&self, wanted: &[&str], claimable: Option<&HashSet<String>>, out: &mut Outcome) -> Result<Vec<(String, GraphMessage)>, Stop> {
        let mut got = Vec::new();
        let mut work: Vec<&[&str]> = wanted.chunks(self.chunk).rev().collect();
        // The emails that failed alone, and whether the server gave any other.
        let (mut alone, mut answered): (Vec<&str>, bool) = (Vec::new(), false);
        while let Some(ids) = work.pop() {
            self.check()?;
            match self.anywhere(ids, claimable, out).await {
                Ok(found) => {
                    answered = true;
                    got.extend(found);
                }
                Err(stop) if halvable(&stop, ids.len(), out.splits) => {
                    out.splits = out.splits.saturating_add(1);
                    let (newer, older) = ids.split_at(ids.len() / 2);
                    work.push(older);
                    work.push(newer);
                }
                Err(Stop::Failed(_)) if ids.len() == 1 => alone.extend(ids),
                Err(Stop::Failed(error)) => {
                    let rest = ids.len().saturating_add(work.iter().map(|ids| ids.len()).sum::<usize>());
                    self.short(out, rest, &self.said(&error));
                    break;
                }
                Err(stop) => return Err(stop),
            }
        }
        // Each is asked for once more: one that fails again while the server gives the others is the email that can't be had
        // (said, and passed over); otherwise it is a server that can't just now, and the run is not whole.
        for id in alone {
            self.check()?;
            match self.anywhere(&[id], claimable, out).await {
                Ok(found) => got.extend(found),
                Err(Stop::Failed(error)) if answered => {
                    out.unreadable.0 = out.unreadable.0.saturating_add(1);
                    out.unreadable.1.get_or_insert_with(|| self.said(&error));
                }
                Err(Stop::Failed(error)) => self.short(out, 1, &self.said(&error)),
                Err(stop) => return Err(stop),
            }
        }
        Ok(got)
    }

    /// One call for the emails `ids`, whole.
    async fn anywhere(&self, ids: &[&str], claimable: Option<&HashSet<String>>, out: &mut Outcome) -> Result<Vec<(String, GraphMessage)>, Stop> {
        let args = json!({"accountId": self.client.account_id(), "ids": ids, "properties": EMAIL_PROPERTIES});
        let mut answers = self.client.mail_within(vec![("Email/get", args)], WHOLE_BYTES).await.map_err(|e| self.call_stop(e))?;
        let answer = answers.take(0).map_err(Self::method_stop)?;
        let mut found = Vec::new();
        let listed = taken(&answer, ids, claimable, out, |id, mailbox, email| {
            found.push((mailbox.to_string(), message::whole(id, mailbox, email, Marks::of(email.get("keywords")))));
        });
        match listed {
            Some(()) => Ok(found),
            None => Err(Stop::Failed(Error::Protocol("the answer has no list of emails".into()))),
        }
    }
}

/// Hand each email of `answer` (an `Email/get`) that `ids` names to `each`, once, with the mailbox whose window would list
/// it (one that lies in none of the run's list, the first it names); and count against the run the ids the answer neither
/// lists nor says are gone. `None`: the answer has no list.
fn taken<'v>(
    answer: &'v Value,
    ids: &[&str],
    claimable: Option<&HashSet<String>>,
    out: &mut Outcome,
    mut each: impl FnMut(&'v str, &'v str, &'v Value),
) -> Option<()> {
    let list = answer.get("list").and_then(Value::as_array)?;
    let asked: HashSet<&str> = ids.iter().copied().collect();
    let gone: HashSet<&str> =
        answer.get("notFound").and_then(Value::as_array).into_iter().flatten().take(ids.len()).filter_map(Value::as_str).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    for email in list.iter().take(ids.len()) {
        let Some(id) = email.get("id").and_then(Value::as_str).filter(|id| asked.contains(id) && !seen.contains(*id)) else { continue };
        seen.insert(id);
        // (One that lies in no mailbox is none to show.)
        let mailbox_ids = email.get("mailboxIds");
        let Some(mailbox) = home(mailbox_ids, claimable).or_else(|| home(mailbox_ids, None)) else { continue };
        each(id, mailbox, email);
    }
    let short = ids.iter().filter(|id| !seen.contains(**id) && !gone.contains(**id)).count();
    if short > 0 {
        out.details.0 = out.details.0.saturating_add(short);
        out.details.1.get_or_insert_with(|| "the server's answer did not have them".to_string());
    }
    Some(())
}
