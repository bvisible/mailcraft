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
//! - **Held**: the server refused it, with the reason in plain words, or the answer was lost and it **may have
//!   gone**: either way it stays in the Outbox, says what happened, and is not sent again until the owner
//!   says so (Send Again). A message is never sent twice by a retry: when in doubt, it is held.
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

use mailcraft_model::sync::{JmapDraftDone, JmapDraftUp, JmapSend, MAX_JMAP_DRAFTS, MAX_JMAP_SENDS, SendDone, SendOutcome, ServerMessage};
use mailcraft_model::{FolderKind, Id, Mailbox, Message, Queued};

use super::jmap_down::plausible_id;
use crate::Session;

/// A line of the owner's, from text that came from elsewhere: no control character, at most `most` characters.
fn plain(text: &str, most: usize) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(most).collect::<String>().trim().to_string()
}

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

/// Take in what became of one message sent. Whether anything changed; a line for the owner when it is held.
/// Only a message of the account's Outbox is taken: an answer that names any other message changes nothing.
pub fn send_done(mb: &mut Mailbox, account: Id, done: &SendDone, said: &mut Vec<String>) -> bool {
    let outbox = mb.folder_of_kind(account, FolderKind::Outbox);
    let Some(m) = mb.message(done.message).filter(|m| m.account == account && Some(m.folder) == outbox) else { return false };
    let subject: String = plain(&m.subject, 60);
    let subject = if subject.is_empty() { "(no subject)".to_string() } else { subject };
    match &done.outcome {
        SendOutcome::Waits => false,
        // Gone: the server filed it in Sent, and that copy comes down. What it answers or forwards is marked so now.
        SendOutcome::Sent => {
            mb.mark_answered(done.message);
            mb.forget_message(done.message).is_ok()
        }
        SendOutcome::Held { lost, why } => {
            let (held, line) = if *lost {
                (
                    "It may have been sent: the server's answer was lost. Before you send it again, ask a recipient whether it arrived; the copy in Sent was filed first, so it doesn't tell."
                        .to_string(),
                    format!("'{subject}' may have been sent: the server's answer was lost. It is held in the Outbox."),
                )
            } else {
                let why = plain(why, 300);
                let why = if why.is_empty() { "the server wouldn't take it".to_string() } else { why };
                (
                    format!("It wasn't sent: {why}. Change it, or send it again."),
                    format!("Couldn't send '{subject}': {why}. It is held in the Outbox."),
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

/// Take in what became of one draft filed. Whether anything changed; a line for the owner when the server won't
/// have it.
///
/// - **Filed**: the draft is the server's copy from now on (`id`, in the mailbox the host said), and waits no
///   more unless it was saved again while the run was under way (then the next run replaces the copy just made).
/// - **Refused**: the draft waits until it is saved again, and that is said once.
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
        said.push(format!("Couldn't save the draft '{subject}' on the server: {why}."));
        if let Ok(m) = mb.message_mut(done.message)
            && let Some(sd) = m.server_draft.as_mut()
        {
            sd.pending = false;
        }
        return true;
    }
    if !plausible_id(&done.id) {
        return false;
    }
    let Ok(m) = mb.message_mut(done.message) else { return false };
    // The server's copy: the one just filed. (Its marks are those of a draft saved here, read; what the owner
    // changes of them later is worked out from this.)
    m.server = Some(ServerMessage { id: done.id.clone(), folder: done.folder.clone(), read: true, ..Default::default() });
    if let Some(sd) = m.server_draft.as_mut() {
        sd.pending = sd.pending && sd.saves != done.saves;
    }
    true
}

/// A copy the server has of a message that is gone here: its destruction waits in the queue until a run has told
/// the server. Whether it was queued (not when it already is).
fn queue_destroy(mb: &mut Mailbox, account: Id, id: &str) -> bool {
    if mb.queued.iter().any(|q| matches!(q, Queued::DeleteMessage { account: a, id: i, .. } if *a == account && i == id)) {
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
