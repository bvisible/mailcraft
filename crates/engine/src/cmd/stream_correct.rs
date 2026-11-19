// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a person puts a message of a hosting page's Inbox under another tab
// //// Neoffice — (Principal, Notifications, Newsletters), and, on their yes, its sender's next ones too; the server keeps it.

//! « Classer dans » in a tab of a hosting page whose mailbox's server takes a person's choice of tab (the Neoffice server's
//! `NeomailStream/set`, frappe_webmail's `mailcraft_stream`, offered where its capability says `correct: true`;
//! neoffice-maintenance#3142):
//!
//! - `mail.offerStreamCorrection` says, for a mailbox, whether its server takes that choice: the hosting page reads it from
//!   the capability each time it asks the server for the tabs (`apps/mailcraft-web/src/streams.rs`).
//! - `mail.correctStream` puts the messages under the tab chosen here at once, them alone, and asks the server
//!   ([`Correction`], taken by the host); a message already under that tab (the tab ticked chosen again) asks nothing. Then
//!   the window offers the same for their senders' next ones ([`Proposal`]: « Faire de même pour les prochains e-mails de … ? »,
//!   as Gmail asks after a move between its tabs): an address may write both a person's mail and automatic notices, and one
//!   choice must not carry them all (the screen test of 19.11.2026, Jérémy's choice). On the person's yes, `"sender": true`:
//!   the other messages of each sender in the same Inbox go with them, and the server keeps the choice for the sender, for this
//!   person and this mailbox (its next answers say it).
//! - `mail.streamCorrected` is the server's answer: a message it did not take is put back under the tab it had, with the
//!   sender's others that went with it, and the person is told.
//!
//! None is an edit of the owner's mailbox (the stream is a word of the host's, kept nowhere here): no step of the history,
//! nothing written to the mailbox file or sent by a run. Nothing here does I/O.

use std::collections::HashSet;

use mailcraft_model::{FolderKind, Id, Message, Stream};
use serde_json::{Value, json};

use super::jmap_down::plausible_id;
use super::{bad, fail, select_after_removal, targets};
use crate::{CmdResult, CommandSpec, Filter, Session, p};

/// The most messages one `mail.correctStream` takes: what the server takes in one call (its `maxIds`).
pub const MAX_CORRECTIONS: usize = 200;

/// What a hosting page's tab knows of its servers' taking a choice of tab, and what it is to tell them. Never saved.
#[derive(Clone, Debug, Default)]
pub struct Hosted {
    /// The accounts whose server takes a person's choice of tab, as the host says them (`mail.offerStreamCorrection`).
    pub correctable: HashSet<Id>,
    /// What is to be told the servers, and what was told and has no answer yet, oldest first.
    pub asks: Vec<Correction>,
    /// What the person is to be told of answers that refused, for the window to say.
    pub said: Vec<String>,
    /// The offer of the same for the senders' next ones, after a choice made for messages alone.
    pub proposal: Option<Proposal>,
}

/// After messages were put under a tab alone: the same for their senders' next ones, as the window offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// The messages chosen (what `"sender": true` is asked for).
    pub ids: Vec<Id>,
    pub stream: Stream,
    /// Their senders as the window names them (the name, else the address), each once.
    pub senders: Vec<String>,
    /// When it was made (seconds since 1970): the window lets it go after a while.
    pub at: i64,
}

/// One choice of tab to tell a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correction {
    pub account: Id,
    pub message: Id,
    /// The message's id on its server.
    pub email_id: String,
    pub stream: Stream,
    /// For its sender too (its other messages and next ones), on the person's yes; else for the message alone.
    pub sender: bool,
    /// The messages put under that tab here with it (it first, then, for its sender, its sender's others in the Inbox that
    /// were not under it yet), and the stream each had: what a refusal puts back.
    pub was: Vec<(Id, Option<Stream>)>,
    /// Taken by the host: its answer is awaited.
    pub sent: bool,
}

impl Hosted {
    /// The choices not taken yet, now taken (the host sends them).
    pub fn take(&mut self) -> Vec<Correction> {
        let mut out = Vec::new();
        for ask in self.asks.iter_mut().filter(|a| !a.sent) {
            ask.sent = true;
            out.push(ask.clone());
        }
        out
    }

    /// Whether anything is told a server or waits to be.
    pub fn asking(&self) -> bool {
        !self.asks.is_empty()
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("mail.offerStreamCorrection", "Offer Stream Correction", "Folder › Actions", offer)
            .params(
                r#"{"account": string (the mailbox's address), "offered": boolean} — whether the mailbox's server takes a person's choice of tab for a message of its Inbox (the hosting page's, from the server's capability)"#,
            )
            .pure(),
        CommandSpec::new("mail.correctStream", "Move to Tab", "Message › Actions", correct)
            .params(
                r#"{"ids"?: [number] (the messages selected when left out), "stream": "person" | "notification" | "newsletter", "sender"?: boolean} — put messages of the Inbox under another tab (Main, Notifications, Newsletters), them alone; with "sender": true (the person's yes to the next ones from their senders), their senders' other messages there with them, and the mailbox's server keeps the choice for each sender"#,
            )
            .selection(selection_refusal)
            .pure(),
        CommandSpec::new("mail.streamCorrected", "Stream Corrected", "Folder › Actions", corrected)
            .params(r#"{"message": number, "ok": boolean} — what a mailbox's server answered when told a person's choice of tab (the hosting page's)"#)
            .pure(),
    ]
}

/// The address a sender is known by, as the server weighs it: without blanks, in lower case.
fn sender(m: &Message) -> String {
    m.from.email.trim().to_lowercase()
}

/// A sender as the person reads it: the name, else the address.
fn sender_name(m: &Message) -> String {
    let name = m.from.name.trim();
    if name.is_empty() { m.from.email.trim().to_string() } else { name.to_string() }
}

fn in_inbox(s: &Session, m: &Message) -> bool {
    s.mailbox.folder(m.folder).is_some_and(|f| f.kind == FolderKind::Inbox)
}

/// Why `m` can't be put under another tab here, or none.
fn refused(s: &Session, m: &Message) -> Option<&'static str> {
    if s.keeps_local || !s.hosted_stream.correctable.contains(&m.account) {
        return Some("this mailbox's server doesn't keep a choice of tab");
    }
    if m.draft || !in_inbox(s, m) {
        return Some("only a message of the Inbox goes under a tab");
    }
    if !m.server.as_ref().is_some_and(|server| !server.gone && plausible_id(&server.id)) {
        return Some("this message isn't on its server yet: try again in a moment");
    }
    None
}

/// Why « Move to Tab » can't run on `ids`, or none.
pub fn refusal(s: &Session, ids: &[Id]) -> Option<&'static str> {
    if ids.is_empty() {
        return Some("select a message first");
    }
    if ids.len() > MAX_CORRECTIONS {
        return Some("too many messages at once: choose fewer");
    }
    ids.iter().find_map(|id| s.mailbox.message(*id).map_or(Some("select a message first"), |m| refused(s, m)))
}

fn selection_refusal(s: &Session) -> Option<&'static str> {
    refusal(s, &s.sel.messages)
}

/// Whether « Move to Tab » is offered for `ids`: each a message of an Inbox whose server keeps a choice of tab.
pub fn offered(s: &Session, ids: &[Id]) -> bool {
    refusal(s, ids).is_none()
}

/// Whether some mailbox of this tab takes a choice of tab: the window offers it.
pub fn available(s: &Session) -> bool {
    !s.keeps_local && !s.hosted_stream.correctable.is_empty()
}

/// The tab all of `ids` are under, when they share one (a message nobody said anything of is under the main one).
pub fn current(s: &Session, ids: &[Id]) -> Option<Stream> {
    let mut streams = ids.iter().filter_map(|id| s.mailbox.message(*id)).map(Message::stream_or_person);
    let first = streams.next()?;
    streams.all(|other| other == first).then_some(first)
}

/// Whether the list in view shows a message under `stream`: the tabs show their own; any other filter shows them all.
fn shown(s: &Session, stream: Stream) -> bool {
    match s.view.filter {
        Filter::Main => stream == Stream::Person,
        Filter::Notifications => stream == Stream::Notification,
        Filter::Newsletters => stream == Stream::Newsletter,
        _ => true,
    }
}

fn offer(s: &mut Session, v: &Value) -> CmdResult {
    p::only(v, &["account", "offered"])?;
    let email = p::req_str(v, "account")?.trim().to_lowercase();
    let offered = p::opt_bool(v, "offered")?.ok_or_else(|| bad("`offered` is true or false"))?;
    // The mailbox's own accounts, not a shared one opened through another: its server answers for its own mail.
    let accounts: Vec<Id> =
        s.mailbox.accounts.iter().filter(|a| a.shared_of.is_none() && a.email.trim().eq_ignore_ascii_case(&email)).map(|a| a.id).collect();
    for account in &accounts {
        if offered {
            s.hosted_stream.correctable.insert(*account);
        } else {
            // It does not any more (its site turned it off): nothing is told it, and what was chosen here stays shown.
            s.hosted_stream.correctable.remove(account);
            s.hosted_stream.asks.retain(|a| a.account != *account);
            s.hosted_stream.proposal = None;
        }
    }
    Ok(json!({"accounts": accounts.len()}))
}

fn correct(s: &mut Session, v: &Value) -> CmdResult {
    p::only(v, &["ids", "id", "stream", "sender"])?;
    let stream = Stream::from_code(p::req_str(v, "stream")?).ok_or_else(|| bad("`stream` is person, notification or newsletter"))?;
    let whole = p::opt_bool(v, "sender")?.unwrap_or(false);
    let mut seen = HashSet::new();
    let ids: Vec<Id> = targets(s, v)?.into_iter().filter(|id| seen.insert(*id)).collect();
    // Every message is looked at before anything is changed.
    if let Some(why) = refusal(s, &ids) {
        return Err(fail(why));
    }
    let before = s.rows();
    let mut moved: HashSet<Id> = HashSet::new();
    let mut asks = Vec::with_capacity(ids.len());
    for id in &ids {
        let Some(m) = s.mailbox.message(*id) else { continue };
        let (account, folder, from) = (m.account, m.folder, sender(m));
        let email_id = m.server.as_ref().map(|server| server.id.clone()).unwrap_or_default();
        // It first, then, for its sender, its sender's others in this Inbox (not a draft, not one another message of this
        // choice took).
        let mut these = vec![*id];
        if whole && !from.is_empty() {
            let mut others: Vec<Id> = s
                .mailbox
                .messages
                .values()
                .filter(|o| o.id != *id && o.account == account && o.folder == folder && !o.draft && sender(o) == from)
                .map(|o| o.id)
                .collect();
            others.sort_unstable();
            these.extend(others);
        }
        let mut was = Vec::new();
        for mid in these {
            if moved.contains(&mid) {
                continue;
            }
            moved.insert(mid);
            let Ok(m) = s.mailbox.message_mut(mid) else { continue };
            // Already shown under that tab (a message nobody said anything of is under the main one): nothing moves.
            if m.stream_or_person() == stream {
                continue;
            }
            was.push((mid, m.stream));
            m.stream = Some(stream);
        }
        // Nothing moved (the tab ticked chosen again): the server is not asked, and its model hears nothing (#3142, the screen test).
        // For the sender, it is asked even so: the choice for the sender is news to it.
        if was.is_empty() && !whole {
            continue;
        }
        asks.push(Correction { account, message: *id, email_id, stream, sender: whole, was, sent: false });
    }
    let changed: Vec<Id> = asks.iter().flat_map(|a| a.was.iter().map(|(id, _)| *id)).collect();
    // The offer of the same for the senders' next ones: after a choice for messages alone; their yes ends it.
    let offer = if whole { None } else { proposal(s, &asks, stream) };
    if whole || offer.is_some() {
        s.hosted_stream.proposal = offer;
    }
    // What left the tab in view leaves the list: the selection goes on as after a move.
    if !shown(s, stream) {
        select_after_removal(s, &before, &changed);
    }
    let count = asks.len();
    s.hosted_stream.asks.extend(asks);
    Ok(json!({"changed": changed.len(), "asked": count, "stream": stream.code()}))
}

/// The offer for the senders of `asks` (made for messages alone), or none when no sender has an address.
fn proposal(s: &Session, asks: &[Correction], stream: Stream) -> Option<Proposal> {
    let mut ids = Vec::new();
    let mut senders: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for ask in asks {
        let Some(m) = s.mailbox.message(ask.message) else { continue };
        if sender(m).is_empty() {
            continue;
        }
        ids.push(ask.message);
        if seen.insert(sender(m)) {
            senders.push(sender_name(m));
        }
    }
    (!ids.is_empty()).then(|| Proposal { ids, stream, senders, at: s.now() })
}

fn corrected(s: &mut Session, v: &Value) -> CmdResult {
    p::only(v, &["message", "ok"])?;
    let message = p::req_id(v, "message")?;
    let ok = p::opt_bool(v, "ok")?.ok_or_else(|| bad("`ok` is true or false"))?;
    let Some(at) = s.hosted_stream.asks.iter().position(|a| a.message == message && a.sent) else {
        // Not awaited (the server was turned off meanwhile): nothing to do.
        return Ok(json!({"known": false}));
    };
    let ask = s.hosted_stream.asks.remove(at);
    if ok {
        return Ok(json!({"known": true, "back": 0}));
    }
    // Refused: nothing to offer for its sender.
    if s.hosted_stream.proposal.as_ref().is_some_and(|p| p.ids.contains(&message)) {
        s.hosted_stream.proposal = None;
    }
    // Put back what went with it, unless another choice moved it since.
    let mut back = 0;
    for (id, was) in &ask.was {
        if let Ok(m) = s.mailbox.message_mut(*id)
            && m.stream == Some(ask.stream)
            && !s.hosted_stream.asks.iter().any(|a| a.was.iter().any(|(other, _)| other == id))
        {
            m.stream = *was;
            back += 1;
        }
    }
    let subject = s.mailbox.message(message).map(|m| m.subject.trim().to_string()).unwrap_or_default();
    let lang = s.interface_language;
    let subject = if subject.is_empty() { lang.t("(no subject)").to_string() } else { subject };
    s.hosted_stream
        .said
        .push(mailcraft_i18n::fmt(lang.t("'{subject}' could not be put under that tab: it stays where it was."), &[("subject", &subject)]));
    Ok(json!({"known": true, "back": back}))
}

#[cfg(test)]
#[path = "stream_correct_tests.rs"]
mod tests;
