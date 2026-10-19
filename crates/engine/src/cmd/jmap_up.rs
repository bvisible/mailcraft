// //// Neoffice — added file (no upstream equivalent)
//! Changes of messages up to a JMAP server (RFC 8621 §4.6), as Microsoft Graph's go up
//! ([`super::graph_up`]).
//!
//! What goes up is the difference between each message of a JMAP account and what its server last said
//! of it ([`ServerMessage`]): the marks that differ (read, flagged, replied, forwarded: `$seen`,
//! `$flagged`, `$answered`, `$forwarded`) as one change of the message, a different folder as a move. What
//! can't be worked out by comparing, a delete for good, waits in the mailbox's queue, and nothing else does:
//! a crash between a change here and its going up loses nothing, as the difference is still there.
//!
//! - **Delete** is a move into the server's Deleted Items; Shift+Delete and emptying Deleted Items (or
//!   Junk E-mail) destroy the message.
//! - **A message with no id** (it was moved, and the server gave it a new one that nobody has said yet, until a
//!   window of the mailbox it went to shows it) can't be named, so nothing is sent for it: its changes wait
//!   until it is found again ([`super::jmap_down`]), and the end of a run says how many are not (a mailbox
//!   bigger than the window shows only its newest messages, so an old one moved there may never be). A message
//!   deleted for good meanwhile can't be destroyed yet: its delete waits in the queue without an id, and the
//!   window that names the message gives the delete its id and keeps the message from coming down again. (If
//!   no window ever does, the server keeps its copy, out of sight here.)
//! - **A folder the server doesn't have** (one made here: a JMAP account's folders are the server's) is no
//!   place to move a message to: the move waits, and so does the message where it is.
//!
//! What comes back is taken in by server id: a change that was made becomes the last synced state, a move
//! gives the message the id the server named (or none, until it is found again); one the server refused is
//! put back to how the server has it and said once per run and kind of change; one the server no longer
//! has the message for leaves here with it, at the end of the run.

use std::collections::{BTreeMap, HashMap, HashSet};

use mailcraft_model::sync::{JmapChange, JmapChangeDone, JmapOutcome, ServerMessage};
use mailcraft_model::{FlagStatus, Id, Mailbox, Message, Queued};

use super::jmap_down::plausible_id;
use crate::Session;

/// The changes of messages made in `account` that its server hasn't heard of: at most `limit` spelled out,
/// and how many there are.
pub fn message_changes(s: &Session, account: Id, limit: usize) -> (Vec<JmapChange>, usize) {
    let mb = &s.mailbox;
    let (mut changes, mut count) = (Vec::new(), 0usize);
    let mut note = |change: &dyn Fn() -> JmapChange| {
        if changes.len() < limit {
            changes.push(change());
        }
        count += 1;
    };
    // What waits in the Outbox is sent, not moved.
    let outbox = mb.folder_of_kind(account, mailcraft_model::FolderKind::Outbox);
    for m in mb.messages.values().filter(|m| m.account == account && Some(m.folder) != outbox) {
        let Some(server) = m.server.as_ref().filter(|server| !server.gone && plausible_id(&server.id)) else { continue };
        if let Some(marks) = marks_of(m, server) {
            note(&|| marks.clone());
        }
        // Where the server has the folder it lies in here; a folder it doesn't have, or a mailbox the message
        // is not known to be in, is no move.
        if let Some(to) = destination(mb, account, m.folder).filter(|to| *to != server.folder && plausible_id(&server.folder)) {
            note(&|| JmapChange::Move { message: m.id, id: server.id.clone(), from: server.folder.clone(), to: to.clone() });
        }
    }
    for queued in mb.queued.iter().filter(|q| q.account() == account) {
        // (A copy and a folder deleted have no counterpart here: a JMAP account has neither.)
        if let Queued::DeleteMessage { id, .. } = queued
            && plausible_id(id)
        {
            note(&|| JmapChange::Destroy { id: id.clone() });
        }
    }
    (changes, count)
}

/// The server's id of the folder `folder` of `account`, when the server has it.
fn destination(mb: &Mailbox, account: Id, folder: Id) -> Option<String> {
    let f = mb.folder(folder).filter(|f| f.account == account)?;
    f.server.as_ref().map(|server| server.id.clone()).filter(|id| plausible_id(id))
}

/// At most `limit` of the mailboxes (their server ids) that the changes of a run will leave messages waiting in to be
/// found again, and those that already wait: where a message was moved to and has no id yet (the mailbox the server
/// has it in, which is not where it lies here if it was moved again since), where a message marked gone lies, or where
/// the server last had a message whose delete for good waits for its id. The moves of this run come first. A run that reads them finds those messages by their `Message-ID`
/// ([`super::jmap_down`]) in the same run that moved them.
pub fn to_find_again(s: &Session, account: Id, changes: &[JmapChange], limit: usize) -> Vec<String> {
    let mb = &s.mailbox;
    let moved_to = changes.iter().filter_map(|change| if let JmapChange::Move { to, .. } = change { Some(to.clone()) } else { None });
    let waiting = mb.messages.values().filter(|m| m.account == account).filter_map(|m| match m.server.as_ref() {
        Some(server) if server.gone => destination(mb, account, m.folder),
        Some(server) if server.id.is_empty() => Some(server.folder.clone()),
        _ => None,
    });
    // And where the messages lie whose delete for good waits for an id.
    let deleting = mb.queued.iter().filter_map(|q| match q {
        Queued::DeleteMessage { account: a, id, kept: Some(kept) } if *a == account && id.is_empty() => {
            kept.server.as_ref().map(|server| server.folder.clone())
        }
        _ => None,
    });
    let mut out: Vec<String> = Vec::new();
    for id in moved_to.chain(waiting).chain(deleting) {
        if out.len() >= limit {
            break;
        }
        if plausible_id(&id) && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// The marks of `m` that differ from what `server` last said, as one change; none when they all agree.
fn marks_of(m: &Message, server: &ServerMessage) -> Option<JmapChange> {
    // JMAP has `$flagged` and nothing more: a follow-up flag marked complete is, there, no flag.
    let flagged = m.flag == FlagStatus::Flagged;
    let read = (m.read != server.read).then_some(m.read);
    let flag = (flagged != (server.flag == FlagStatus::Flagged)).then_some(flagged);
    let answered = (m.answered != server.answered).then_some(m.answered);
    let forwarded = (m.forwarded != server.forwarded).then_some(m.forwarded);
    if read.is_none() && flag.is_none() && answered.is_none() && forwarded.is_none() {
        return None;
    }
    Some(JmapChange::Keywords { message: m.id, id: server.id.clone(), read, flagged: flag, answered, forwarded })
}

/// A kind of change, for saying refusals once per run and kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Read(bool),
    Flag,
    Replied,
    Several,
    Move,
    Delete,
}

/// Why the server said no.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Why {
    Gone,
    Denied,
    Full,
    Invalid,
}

impl Why {
    fn of(outcome: &JmapOutcome) -> Option<Why> {
        match outcome {
            JmapOutcome::Done { .. } => None,
            JmapOutcome::Gone => Some(Why::Gone),
            // (SetError types, RFC 8620 §5.3, and the method errors that say an account can't be changed.)
            JmapOutcome::Refused { why } => Some(match why.as_str() {
                "forbidden" | "accountReadOnly" | "accountNotSupportedByMethod" | "unknownMethod" => Why::Denied,
                "overQuota" => Why::Full,
                _ => Why::Invalid,
            }),
        }
    }

    fn words(self, many: bool) -> &'static str {
        match (self, many) {
            (Why::Gone, false) => "it no longer exists",
            (Why::Gone, true) => "they no longer exist",
            (Why::Denied, false) => "this account may not change it",
            (Why::Denied, true) => "this account may not change them",
            (Why::Full, _) => "the mailbox is full",
            (Why::Invalid, _) => "it can't make that change",
        }
    }
}

/// The refusals of a run, by kind and reason: how many, the first message's subject, and the folder they were to go
/// to (when that was one and the same).
#[derive(Debug, Default)]
pub struct Refusals {
    said: BTreeMap<(Kind, Why), (usize, String, Option<String>)>,
}

impl Refusals {
    fn note(&mut self, kind: Kind, why: Why, subject: &str, to: Option<String>) {
        // A delete for good of a message nothing is known of here is of "a message".
        let subject = if kind == Kind::Delete && subject.trim().is_empty() { String::new() } else { shown(subject) };
        let entry = self.said.entry((kind, why)).or_insert_with(|| (0, subject, to.clone()));
        entry.0 += 1;
        if entry.2 != to {
            entry.2 = None;
        }
    }

    /// One line for each kind and reason: "Couldn't move 'Invoice 42' to Archive: the server says it no longer
    /// exists."
    pub fn lines(&self) -> Vec<String> {
        self.said
            .iter()
            .map(|((kind, why), (n, subject, to))| {
                let many = *n > 1;
                let what = if many { format!("{n} messages") } else { format!("'{subject}'") };
                let to = to.as_ref().map(|to| format!(" to {to}")).unwrap_or_default();
                let doing = match kind {
                    Kind::Read(true) => format!("mark {what} as read"),
                    Kind::Read(false) => format!("mark {what} as unread"),
                    Kind::Flag => format!("change the flag of {what}"),
                    Kind::Replied => format!("change the replied or forwarded mark of {what}"),
                    Kind::Several => format!("change {what}"),
                    Kind::Move => format!("move {what}{to}"),
                    Kind::Delete if subject.is_empty() && !many => "delete a message for good".to_string(),
                    Kind::Delete => format!("delete {what} for good"),
                };
                match (kind, why) {
                    (Kind::Delete, Why::Denied) => format!("Couldn't {doing}: the server doesn't allow it."),
                    _ => format!("Couldn't {doing}: the server says {}.", why.words(many)),
                }
            })
            .collect()
    }
}

/// A subject as a line about it shows it.
fn shown(subject: &str) -> String {
    let subject: String = subject.chars().filter(|c| !c.is_control()).take(60).collect();
    if subject.trim().is_empty() { "(no subject)".to_string() } else { subject }
}

/// The name of the folder of `account` the server calls `id`, for a line about it.
fn folder_name(mb: &Mailbox, account: Id, id: &str) -> Option<String> {
    let folder = mb.folder_by_server(account, id)?;
    mb.folder(folder).map(|f| f.name.chars().filter(|c| !c.is_control()).take(60).collect())
}

/// The message of `account` called `id` on the server, when it is still `message` here.
fn mine<'m>(mb: &'m mut Mailbox, account: Id, message: Id, id: &str) -> Option<&'m mut Message> {
    let m = mb.message_mut(message).ok()?;
    (m.account == account && m.server.as_ref().is_some_and(|s| s.id == id)).then_some(m)
}

/// Take in what became of one change of a message. Whether anything changed here.
pub fn message_done(mb: &mut Mailbox, account: Id, done: &JmapChangeDone, refusals: &mut Refusals) -> bool {
    let why = Why::of(&done.outcome);
    match &done.change {
        JmapChange::Keywords { message, id, read, flagged, answered, forwarded } => {
            let marks = [read.is_some(), flagged.is_some(), answered.is_some() || forwarded.is_some()];
            let kind = match marks {
                _ if marks.iter().filter(|p| **p).count() > 1 => Kind::Several,
                [true, ..] => Kind::Read(read.unwrap_or_default()),
                [_, true, _] => Kind::Flag,
                _ => Kind::Replied,
            };
            let Some(m) = mine(mb, account, *message, id) else { return false };
            let Some(mut server) = m.server.clone() else { return false };
            match &done.outcome {
                JmapOutcome::Done { .. } => {
                    if let Some(read) = read {
                        server.read = *read;
                    }
                    if let Some(flagged) = flagged {
                        server.flag = if *flagged { FlagStatus::Flagged } else { FlagStatus::None };
                    }
                    if let Some(answered) = answered {
                        server.answered = *answered;
                    }
                    if let Some(forwarded) = forwarded {
                        server.forwarded = *forwarded;
                    }
                }
                _ => {
                    // Put back to how the server has it, but for what was changed again here.
                    if read.is_some_and(|read| m.read == read) {
                        m.read = server.read;
                    }
                    if flagged.is_some_and(|flagged| (m.flag == FlagStatus::Flagged) == flagged) {
                        m.flag = server.flag;
                    }
                    if answered.is_some_and(|answered| m.answered == answered) {
                        m.answered = server.answered;
                    }
                    if forwarded.is_some_and(|forwarded| m.forwarded == forwarded) {
                        m.forwarded = server.forwarded;
                    }
                    server.gone |= why == Some(Why::Gone);
                    let subject = m.subject.clone();
                    if let Some(why) = why {
                        refusals.note(kind, why, &subject, None);
                    }
                }
            }
            if let Ok(m) = mb.message_mut(*message) {
                m.server = Some(server);
            }
            true
        }
        JmapChange::Move { message, id, to, .. } => {
            let Some(m) = mine(mb, account, *message, id) else { return false };
            let Some(server) = m.server.as_mut() else { return false };
            match &done.outcome {
                // In its new mailbox, under the id the server named, or under none until a window of that mailbox
                // shows it (some servers name a message by where it lies).
                JmapOutcome::Done { id: now } => {
                    server.folder = to.clone();
                    server.id = if plausible_id(now) { now.clone() } else { String::new() };
                }
                JmapOutcome::Gone => {
                    server.gone = true;
                    let subject = m.subject.clone();
                    refusals.note(Kind::Move, Why::Gone, &subject, folder_name(mb, account, to));
                }
                JmapOutcome::Refused { .. } => {
                    let was = server.folder.clone();
                    let subject = m.subject.clone();
                    if let Some(back) = mb.folder_by_server(account, &was)
                        && let Ok(m) = mb.message_mut(*message)
                    {
                        m.folder = back;
                    }
                    if let Some(why) = why {
                        refusals.note(Kind::Move, why, &subject, folder_name(mb, account, to));
                    }
                }
            }
            true
        }
        JmapChange::Destroy { .. } => {
            destroys_done(mb, account, &[done], refusals);
            true
        }
    }
}

/// Take in what became of deletes for good (the other `dones` are left alone): each is off the queue, and one the
/// server refused is put back and said. All in one pass over the queue and one look at the messages, as emptying
/// Deleted Items queues thousands.
pub fn destroys_done(mb: &mut Mailbox, account: Id, dones: &[&JmapChangeDone], refusals: &mut Refusals) {
    // (A delete for good that waits for an id, in the queue, is nobody's to take in.)
    let ids: HashSet<&str> = dones
        .iter()
        .filter_map(|done| match &done.change {
            JmapChange::Destroy { id } if !id.is_empty() => Some(id.as_str()),
            _ => None,
        })
        .collect();
    if ids.is_empty() {
        return;
    }
    // What the queue kept of each message (the first that kept one), moved out of it.
    let mut kept: HashMap<String, Option<Box<Message>>> = HashMap::new();
    for queued in std::mem::take(&mut mb.queued) {
        match queued {
            Queued::DeleteMessage { account: a, id, kept: was } if a == account && ids.contains(id.as_str()) => {
                let slot = kept.entry(id).or_default();
                if slot.is_none() {
                    *slot = was;
                }
            }
            other => mb.queued.push(other),
        }
    }
    let mut present: Option<HashSet<String>> = None;
    for done in dones {
        let (JmapChange::Destroy { id }, JmapOutcome::Refused { .. }, Some(why)) = (&done.change, &done.outcome, Why::of(&done.outcome)) else {
            continue;
        };
        let kept = kept.remove(id.as_str()).flatten();
        let subject = put_back(mb, account, id, kept, &mut present);
        refusals.note(Kind::Delete, why, &subject, None);
    }
}

/// A message deleted for good here whose delete the server refused, put back as the server has it: in its server
/// folder, from what was kept of it here, with the marks the server last had. Nothing kept (a queue from before) and
/// nothing can be put back: the server's copy comes down again with the window of its mailbox. The subject, for
/// saying so. `present` holds the server ids of the account's messages once they were looked at.
fn put_back(mb: &mut Mailbox, account: Id, id: &str, kept: Option<Box<Message>>, present: &mut Option<HashSet<String>>) -> String {
    let Some(kept) = kept else { return String::new() };
    let subject = kept.subject.clone();
    // Back already (it came down again meanwhile): nothing to do.
    let present = present.get_or_insert_with(|| {
        mb.messages.values().filter(|m| m.account == account).filter_map(|m| m.server.as_ref().map(|server| server.id.clone())).collect()
    });
    if present.contains(id) {
        return subject;
    }
    let mut m = *kept;
    let Some(server) = m.server.clone().filter(|server| server.id == id) else { return subject };
    let Some(folder) = mb.folder_by_server(account, &server.folder) else { return subject };
    (m.folder, m.uid, m.synced) = (folder, 0, None);
    (m.read, m.flag, m.answered, m.forwarded) = (server.read, server.flag, server.answered, server.forwarded);
    m.server = Some(ServerMessage { gone: false, ..server });
    if mb.add_message(m).is_ok() {
        present.insert(id.to_string());
    }
    subject
}

#[cfg(test)]
#[path = "jmap_up_tests.rs"]
mod tests;
