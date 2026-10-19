// //// Neoffice — added file (no upstream equivalent)
//! Sending and drafts of an account kept in step with a JMAP server (RFC 8621 §7 and §4.8).
//!
//! **Sending.** What waits in a JMAP account's Outbox goes with its run ([`sends`]): each message whole, held
//! ones left out (a held message is the owner's to send again, [`mail.sendAgain`](super::graph_send)). The
//! host files a copy in the server's Sent and has the server send it. What became of each message comes back
//! as a [`SendDone`] ([`send_done`]):
//!
//! - **Sent**: the message leaves the Outbox (the copy in Sent comes down with the window of that mailbox),
//!   and the original it answers or forwards is marked so (#154).
//! - **Held**: the server refused it, with the reason in plain words, or the answer was lost (or was a
//!   failure that may have come after the sending) and it **may have gone**: either way it stays in the Outbox,
//!   says what happened, and is not sent again until the owner says so (Send Again). A message is never sent
//!   twice by a retry: when in doubt, it is held. Nothing the owner does can let it go again by itself: the
//!   history that Undo goes back through is forgotten (by the caller) whenever a message went or was held, since
//!   an older state of the mailbox has it in the Outbox, not held.
//! - **Waits**: not tried, or not taken (the network, a busy server, a limit on sending): it goes with the next
//!   run, and nothing is said of it here.
//!
//! **Drafts.** A draft saved here ([`ServerDraft::pending`]) goes up with the next run as a copy filed in the
//! server's Drafts ([`drafts`]), and the copy an earlier save made is destroyed once the new one is in. What
//! the host filed comes back as a [`JmapDraftDone`] ([`draft_done`]): the draft knows the server's copy by
//! its id from then on, and waits no more unless it was saved again meanwhile. A draft **sent** hands the
//! server's copy of it to the message that was made of it ([`ServerDraft::sent_from`]), which the host
//! destroys when the message has gone; a draft **discarded** has its server copy queued for destruction
//! ([`discard`], [`Queued::DeleteMessage`]).
//!
//! **Copies nobody wants.** A copy the host could not destroy (the draft of a message that went, the copy an
//! earlier save of a draft replaced, a copy filed in Sent for a message that was not sent) comes back as
//! unwanted ([`unwanted`]) and waits in the same queue. A copy that waits there, and the draft copy of a message
//! that waits in the Outbox, are not brought down as mail ([`doomed`]): the owner already has them as what they
//! are. Who destroys what waits in the queue is the part of a run that sends the owner's changes up.

use std::collections::HashSet;

use mailcraft_model::sync::{JmapDraftDone, JmapDraftUp, JmapSend, MAX_JMAP_DRAFTS, MAX_JMAP_SENDS, SendDone, SendOutcome, ServerMessage};
use mailcraft_model::{FolderKind, Id, Mailbox, Message, Queued};

use super::jmap_down::plausible_id;
use crate::Session;

/// A line of the owner's, from text that came from elsewhere: no control character, at most `most` characters.
fn plain(text: &str, most: usize) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(most).collect::<String>().trim().to_string()
}

/// `text` with every word that has an `@` in it replaced by "(address)". The lines a run says go to the log as
/// well as to the owner, and an address is not for the log: it stays in what the message says of itself
/// ([`Message::held`]), which only the owner reads.
fn no_address(text: &str) -> String {
    fn flush(word: &mut String, out: &mut String) {
        out.push_str(if word.contains('@') { "(address)" } else { word.as_str() });
        word.clear();
    }
    let (mut out, mut word) = (String::with_capacity(text.len()), String::new());
    for c in text.chars() {
        if c.is_whitespace() || matches!(c, ',' | ';' | '"' | '\'' | '<' | '>' | '(' | ')') {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// The most changes that wait in the mailbox's queue: what is past it is not queued (a server that never lets a
/// copy be destroyed must not grow the file without end).
const MAX_QUEUED: usize = 10_000;
/// The most unwanted copies a piece of a run is read for.
const MAX_UNWANTED: usize = 100;

/// What `account` has waiting to be sent: every message of its Outbox that isn't held, whole, the oldest first
/// and at most [`MAX_JMAP_SENDS`] of them (the rest go with the next run). What a message keeps on disk is read
/// for it (R6.1); one whose content can't be read waits in the Outbox, with a line in the log, rather than going
/// without its files.
pub fn sends(s: &Session, account: Id) -> Vec<JmapSend> {
    let mb = &s.mailbox;
    let Some(outbox) = mb.folder_of_kind(account, FolderKind::Outbox) else { return Vec::new() };
    let mut waiting: Vec<Id> = mb.messages_in(outbox).filter(|m| m.account == account && m.held.is_empty()).map(|m| m.id).collect();
    waiting.sort_unstable();
    waiting
        .into_iter()
        .take(MAX_JMAP_SENDS)
        .filter_map(|id| match s.loaded_message(id) {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("message {id} stays in the Outbox this time: {e}");
                None
            }
        })
        .map(|m| JmapSend {
            message: m.id,
            // The draft it was written as, when the server has one.
            draft_copy: m.server_draft.as_ref().map(|d| d.sent_from.clone()).filter(|id| plausible_id(id)).unwrap_or_default(),
            mime: Box::new(m.as_ref().clone()),
        })
        .collect()
}

/// The drafts of `account` that wait to go up: those of its Drafts saved here since the server last had them,
/// whole, the oldest first and at most [`MAX_JMAP_DRAFTS`] of them. A draft that was moved out of Drafts (it is
/// in Deleted Items, say) is not made again there.
pub fn drafts(s: &Session, account: Id) -> Vec<JmapDraftUp> {
    let mb = &s.mailbox;
    let Some(folder) = mb.folder_of_kind(account, FolderKind::Drafts) else { return Vec::new() };
    let mut waiting: Vec<Id> = mb.messages_in(folder).filter(|m| is_waiting(m, account)).map(|m| m.id).collect();
    waiting.sort_unstable();
    waiting
        .into_iter()
        .take(MAX_JMAP_DRAFTS)
        .filter_map(|id| match s.loaded_message(id) {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("draft {id} waits to go up: {e}");
                None
            }
        })
        .filter_map(|m| {
            let sd = m.server_draft.as_ref()?;
            // The server's copy of an earlier save, if it still has it.
            let replaces =
                m.server.as_ref().filter(|server| !server.gone && plausible_id(&server.id)).map(|server| server.id.clone()).unwrap_or_default();
            Some(JmapDraftUp { message: m.id, saves: sd.saves, replaces, mime: Box::new(m.as_ref().clone()) })
        })
        .collect()
}

/// Whether `m` is a draft of `account` saved here that the server doesn't have in its latest form.
fn is_waiting(m: &Message, account: Id) -> bool {
    m.account == account && m.draft && m.server_draft.as_ref().is_some_and(|d| d.pending)
}

/// How many drafts of `account` wait to go up (cheap: nothing is read).
pub fn pending_drafts(mb: &Mailbox, account: Id) -> usize {
    mb.folder_of_kind(account, FolderKind::Drafts).map_or(0, |folder| mb.messages_in(folder).filter(|m| is_waiting(m, account)).count())
}

/// Take in what became of one message sent. Whether anything changed; a line for the owner when it is held, or
/// when it went (or may have) although the owner had taken it out of the Outbox meanwhile.
pub fn send_done(mb: &mut Mailbox, account: Id, done: &SendDone, said: &mut Vec<String>) -> bool {
    let outbox = mb.folder_of_kind(account, FolderKind::Outbox);
    let Some(m) = mb.message(done.message).filter(|m| m.account == account) else {
        // Deleted from here while the run was under way: all that is left to say is that it went, if it did.
        return left_the_outbox(mb, done, None, said);
    };
    let subject = plain(&m.subject, 60);
    let subject = if subject.is_empty() { "(no subject)".to_string() } else { subject };
    if Some(m.folder) != outbox {
        return left_the_outbox(mb, done, Some(&subject), said);
    }
    match &done.outcome {
        SendOutcome::Waits => false,
        // Gone: the server filed it in Sent, and that copy comes down. What it answers or forwards is marked so now.
        SendOutcome::Sent => {
            mb.mark_answered(done.message);
            mb.forget_message(done.message).is_ok()
        }
        SendOutcome::Held { lost, why } => {
            let why = plain(why, 300);
            let (held, line) = if *lost {
                // (What the host could say of why it isn't known; the answer was lost when it says nothing.)
                let doubt = if why.is_empty() { "the server's answer was lost".to_string() } else { why };
                (
                    format!(
                        "It may have been sent: {doubt}. Before you send it again, ask a recipient whether it arrived; the copy in Sent was filed first, so it doesn't tell."
                    ),
                    format!("'{subject}' may have been sent: {}. It is held in the Outbox.", no_address(&doubt)),
                )
            } else {
                let why = if why.is_empty() { "the server wouldn't take it".to_string() } else { why };
                (
                    format!("It wasn't sent: {why}. Change it, or send it again."),
                    format!("Couldn't send '{subject}': {}. It is held in the Outbox.", no_address(&why)),
                )
            };
            said.push(line);
            if let Ok(m) = mb.message_mut(done.message) {
                m.held = held;
            }
            true
        }
    }
}

/// What became of a message that is no longer in the account's Outbox, because the owner took it out (to change
/// it, or for good) while the run was under way. Nothing is done to it, but a message that went, or may have, is
/// said so: the owner is told, and the original it answers is marked.
fn left_the_outbox(mb: &mut Mailbox, done: &SendDone, subject: Option<&str>, said: &mut Vec<String>) -> bool {
    let what = match &done.outcome {
        SendOutcome::Sent => "was sent",
        SendOutcome::Held { lost: true, .. } => "may have been sent",
        SendOutcome::Held { .. } | SendOutcome::Waits => return false,
    };
    let name = subject.map_or_else(|| "A message".to_string(), |subject| format!("'{subject}'"));
    said.push(format!("{name} {what}, although it was taken out of the Outbox meanwhile. Look in Sent."));
    if subject.is_some() && matches!(done.outcome, SendOutcome::Sent) {
        mb.mark_answered(done.message);
    }
    true
}

/// Take in what became of one draft filed. Whether anything changed; a line for the owner when the server won't
/// have it.
///
/// - **Filed**: the draft is the server's copy from now on (`id`, in the mailbox the host said), and waits no
///   more unless it was saved again while the run was under way (then the next run replaces the copy just made).
/// - **Refused**: the draft waits until it is saved again, and that is said once (a draft saved again while the
///   run was under way still waits: that version was not the one refused).
/// - **Gone meanwhile** (discarded while the run was under way): the copy the run filed is not wanted, and its
///   destruction is queued.
pub fn draft_done(mb: &mut Mailbox, account: Id, done: &JmapDraftDone, said: &mut Vec<String>) -> bool {
    let Some(m) = mb.message(done.message).filter(|m| m.account == account && m.draft && m.server_draft.is_some()) else {
        // Not a draft here any more (or never of this account): a copy filed for it is nobody's.
        if mb.message(done.message).is_none() && plausible_id(&done.id) {
            return queue_destroy(mb, account, &done.id);
        }
        return false;
    };
    let subject = plain(&m.subject, 60);
    let subject = if subject.is_empty() { "(no subject)".to_string() } else { subject };
    if let Some(why) = &done.refused {
        let why = plain(why, 300);
        said.push(format!("Couldn't save the draft '{subject}' on the server: {}.", no_address(&why)));
        if let Ok(m) = mb.message_mut(done.message)
            && let Some(sd) = m.server_draft.as_mut()
            && sd.saves == done.saves
        {
            sd.pending = false;
        }
        return true;
    }
    if !plausible_id(&done.id) {
        return false;
    }
    // The mailbox the copy is in: the one the host said, when that can be one, else the account's Drafts.
    let folder = if plausible_id(&done.folder) {
        done.folder.clone()
    } else {
        mb.folder_of_kind(account, FolderKind::Drafts)
            .and_then(|f| mb.folder(f))
            .and_then(|f| f.server.as_ref())
            .map(|server| server.id.clone())
            .unwrap_or_default()
    };
    let Ok(m) = mb.message_mut(done.message) else { return false };
    // The server's copy: the one just filed. (Its marks are those of a draft saved here, read; what the owner
    // changes of them later is worked out from this.)
    m.server = Some(ServerMessage { id: done.id.clone(), folder, read: true, ..Default::default() });
    if let Some(sd) = m.server_draft.as_mut() {
        sd.pending = sd.pending && sd.saves != done.saves;
    }
    true
}

/// Take in the copies the server has that nobody wants and that the host could not destroy: each waits in the
/// queue until it is destroyed. Whether any was queued.
pub fn unwanted(mb: &mut Mailbox, account: Id, ids: &[String]) -> bool {
    let mut queued = false;
    for id in ids.iter().take(MAX_UNWANTED).filter(|id| plausible_id(id)) {
        queued |= queue_destroy(mb, account, id);
    }
    queued
}

/// The server ids of the copies of `account` that must not come down as mail: those whose destruction waits in
/// the queue, and the copy of the draft that a message waiting to be sent was written as (the host destroys it
/// when the message has gone).
pub fn doomed(mb: &Mailbox, account: Id) -> HashSet<String> {
    let queued = mb.queued.iter().filter_map(|q| match q {
        Queued::DeleteMessage { account: a, id, .. } if *a == account => Some(id.as_str()),
        _ => None,
    });
    let written_as = mb.messages.values().filter(|m| m.account == account).filter_map(|m| m.server_draft.as_ref()).map(|d| d.sent_from.as_str());
    queued.chain(written_as).filter(|id| !id.is_empty()).map(str::to_string).collect()
}

/// A copy the server has of a message that is gone here: its destruction waits in the queue until a run has told
/// the server. Whether it was queued (not when it already is, nor when the queue is full).
fn queue_destroy(mb: &mut Mailbox, account: Id, id: &str) -> bool {
    if mb.queued.len() >= MAX_QUEUED
        || mb.queued.iter().any(|q| matches!(q, Queued::DeleteMessage { account: a, id: i, .. } if *a == account && i == id))
    {
        return false;
    }
    mb.queued.push(Queued::DeleteMessage { account, id: id.to_string(), kept: None });
    true
}

/// The draft `id` is being discarded: when the server has a copy of it, that copy is destroyed too. (Called before
/// the draft is removed; a draft the server has not got, or that is another kind of account's, queues nothing.)
pub fn discard(mb: &mut Mailbox, id: Id) {
    let Some((account, copy)) = mb
        .message(id)
        .filter(|m| m.draft)
        .and_then(|m| m.server.as_ref().filter(|server| !server.gone && plausible_id(&server.id)).map(|server| (m.account, server.id.clone())))
    else {
        return;
    };
    if mb.account(account).is_some_and(|a| a.kind == mailcraft_model::AccountKind::Jmap) {
        queue_destroy(mb, account, &copy);
    }
}

#[cfg(test)]
#[path = "jmap_send_tests.rs"]
mod tests;
