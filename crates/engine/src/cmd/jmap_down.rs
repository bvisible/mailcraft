// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Mail down from a JMAP server (RFC 8621).
//!
//! A JMAP run reads, for each mailbox it looks at, its newest messages: a *window*
//! ([`MessagePage`], one per mailbox). A window that holds every message of its mailbox is a
//! complete listing (`afresh`): what this mailbox had there and the window doesn't is marked gone
//! at once, and removed when the run ends whole, as a Graph folder listed afresh is. A window that
//! was cut short only adds and updates.
//!
//! - **A new message** comes as its headers and preview, not fetched ([`graph_down::new_message`]).
//! - **A message this mailbox has** comes as its keywords: read, flagged, answered and forwarded
//!   take the server's value, but for one changed here since the server last said (that goes up
//!   with the run's changes). It follows the server into the mailbox the window is of, unless it
//!   was moved here since: a move that has not reached the server stays, whether the window is
//!   complete or not, and goes up.
//! - **A message that is its marks alone and that this mailbox does not have** (the request said it did, and
//!   it was deleted here since) is passed over, not added blank.
//! - **A copy that waits to be destroyed** (a draft discarded here, a copy a run could not destroy: the queue of
//!   [`super::jmap_send`]) is passed over: it is not mail to bring down again. Nor is the copy of the draft that a
//!   message waiting in the Outbox was written as.
//! - **A message the server knows by a new id** (some servers name a message by where it is, so a
//!   move gives it a new id): one listed under an id this account doesn't know is first matched,
//!   by its `Message-ID`, to a message that waits for an id in this mailbox:
//!   - one with no id that the server was last known to have in it, wherever it lies here now (it
//!     was moved on since);
//!   - one lying in it here that this window or an earlier one marked gone from the mailbox the
//!     server had it in;
//!   - one lying in it here that the server last had in another mailbox: a move the owner made
//!     that the server may have made without a word coming back (the answer was lost).
//!
//!   A message this window lists under the id it has is not waiting for one. The match is made
//!   only when exactly one message waits and exactly one listed message of that `Message-ID` is new
//!   here, and when they were received at the same time and are as big (a move keeps both), where both say;
//!   otherwise the listed message is a new one.
//! - **A message that a complete window does not list, and that was moved here since** (it lies in another
//!   mailbox than the one the window is of, and the server may have made the move without a word coming
//!   back) is not gone: it is taken to be where it lies, under an id nobody has said, and is looked for there.
//! - **A message whose delete for good the server has not made** is not mail to bring down again; and a
//!   delete that waits without an id (the message had none) is given the id of the message it is for,
//!   which is the one this mailbox lists with its `Message-ID` (under the same rules as above), and it is
//!   not brought down either.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mailcraft_model::sync::{FolderListing, GraphMessage, MessagePage, ServerMessage};
use mailcraft_model::time::Timestamp;
use mailcraft_model::{Address, FlagStatus, FolderKind, Id, Mailbox, Message, Queued};

use super::graph_down::{self, Down, ServerIndex};

/// The longest server id taken in. RFC 8620 allows 255 octets; ids also go into requests and
/// addresses, so a longer one is refused rather than carried.
pub const MAX_ID: usize = 512;

/// Whether `id` can be a JMAP id (RFC 8620 §1.2: `A-Za-z0-9`, `-` and `_`).
pub fn plausible_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A `Message-ID` as it is compared: without blanks and angle brackets.
fn message_id(raw: &str) -> &str {
    raw.trim().trim_start_matches('<').trim_end_matches('>')
}

/// Take in one mailbox's window. A window for a mailbox this account doesn't have (yet) is
/// passed over: the next run brings it again.
pub fn take_window(mb: &mut Mailbox, index: &mut ServerIndex, account: Id, page: &MessagePage, down: &mut Down) {
    if !plausible_id(&page.folder) {
        return;
    }
    let Some(folder) = mb.folder_by_server(account, &page.folder) else { return };
    // The ids this window lists.
    let listed: HashSet<&str> = page.messages.iter().map(|m| m.server.id.as_str()).collect();
    if page.afresh {
        // Where each folder of the account is on the server, for a message that was moved here since.
        let server_of: HashMap<Id, &str> =
            mb.folders.iter().filter(|f| f.account == account).filter_map(|f| f.server.as_ref().map(|server| (f.id, server.id.as_str()))).collect();
        let mut moved_to: Vec<(Id, String)> = Vec::new();
        let mut gone: Vec<Id> = Vec::new();
        for m in mb.messages.values().filter(|m| m.account == account) {
            let Some(s) = m.server.as_ref().filter(|s| s.folder == page.folder && !s.gone) else { continue };
            // One this window does not list that lies in another mailbox here was moved here since, and the server may have
            // made the move without a word coming back: it is not gone, it is where it lies, and nobody has said its id.
            let there = (m.folder != folder && !s.id.is_empty() && !listed.contains(s.id.as_str()))
                .then(|| server_of.get(&m.folder).copied().filter(|to| plausible_id(to) && *to != page.folder))
                .flatten();
            match there {
                Some(to) => moved_to.push((m.id, to.to_string())),
                None => gone.push(m.id),
            }
        }
        for id in gone {
            if let Ok(m) = mb.message_mut(id)
                && let Some(s) = m.server.as_mut()
            {
                s.gone = true;
            }
        }
        for (id, to) in moved_to {
            if let Ok(m) = mb.message_mut(id)
                && let Some(s) = m.server.as_mut()
            {
                (s.id, s.folder) = (String::new(), to);
            }
        }
    }
    for id in page.removed.iter().filter(|id| plausible_id(id)) {
        let Some(m) = index.find(mb, account, id) else { continue };
        if let Ok(m) = mb.message_mut(m)
            && let Some(s) = m.server.as_mut().filter(|s| s.folder == page.folder && !s.gone)
        {
            s.gone = true;
            down.gone += 1;
        }
    }
    // Copies the server has that must not come back as mail: those that wait to be destroyed (a draft discarded here,
    // a copy that could not be destroyed, a message deleted for good here whose delete the server has not made
    // yet), and the draft of a message that waits to be sent.
    let doomed = super::jmap_send::doomed(mb, account);
    // The messages that wait for an id in this mailbox, by `Message-ID`: those a listed id nobody knows may be.
    let mut lost: HashMap<String, Vec<Id>> = HashMap::new();
    for m in mb.messages.values().filter(|m| m.account == account && !message_id(&m.message_id).is_empty()) {
        let Some(s) = m.server.as_ref() else { continue };
        let waits = if s.id.is_empty() {
            s.folder == page.folder
        } else {
            // (One this window lists under its id waits for nothing.)
            m.folder == folder && (s.gone || s.folder != page.folder) && !listed.contains(s.id.as_str())
        };
        if waits {
            lost.entry(message_id(&m.message_id).to_string()).or_default().push(m.id);
        }
    }
    // The messages this window lists under an id nobody has here, by `Message-ID`: one is taken for a message that waits
    // only when no other of them can be that message.
    let mut new_here: HashMap<&str, Vec<&GraphMessage>> = HashMap::new();
    for there in &page.messages {
        let id = there.server.id.as_str();
        if plausible_id(id) && !doomed.contains(id) && index.find(mb, account, id).is_none() {
            new_here.entry(message_id(&there.message_id)).or_default().push(there);
        }
    }
    for there in &page.messages {
        let id = there.server.id.as_str();
        if !plausible_id(id) || doomed.contains(id) {
            continue;
        }
        let here = index.find(mb, account, id).or_else(|| adopt(&mut lost, mb, there, &new_here));
        match here {
            Some(m) => {
                if update(mb, account, m, folder, &page.folder, there) {
                    down.changed += 1;
                }
                index.note(mb, account, id, m);
            }
            // A message the host sent as its marks alone, because the request said this mailbox has it, and that
            // it no longer does (it was deleted here since): no message to add. It would stay blank, as every
            // later run would send it as one the mailbox has. The next run asks for it whole.
            None if marks_only(there) => {}
            // A message that was deleted for good here while nothing had named it: now it is named, and the delete
            // can be made. It is not mail to bring down.
            None if named_by_a_delete(mb, account, &page.folder, there, &new_here) => {}
            None => {
                if let Ok(m) = mb.add_message(graph_down::new_message(folder, &page.folder, there)) {
                    index.note(mb, account, id, m);
                    down.new += 1;
                }
            }
        }
    }
}

/// The message that waits for an id which `there` is: the only one of the candidates (`lost`, by `Message-ID`) it can be
/// ([`alike`]), when no other message that is new here (`new_here`) can be that one. One that is taken is taken once.
fn adopt(lost: &mut HashMap<String, Vec<Id>>, mb: &Mailbox, there: &GraphMessage, new_here: &HashMap<&str, Vec<&GraphMessage>>) -> Option<Id> {
    let wanted = message_id(&there.message_id);
    let candidates = lost.get_mut(wanted)?;
    let size_of = |m: &mailcraft_model::Message| m.server.as_ref().map_or(0, |server| server.size);
    let mut fit = candidates.iter().copied().filter(|c| mb.message(*c).is_some_and(|m| alike(m.received, size_of(m), there)));
    let (Some(candidate), None) = (fit.next(), fit.next()) else { return None };
    let m = mb.message(candidate)?;
    let rivals = new_here.get(wanted).map_or(0, |all| all.iter().filter(|other| alike(m.received, size_of(m), other)).count());
    if rivals != 1 {
        return None;
    }
    candidates.retain(|c| *c != candidate);
    Some(candidate)
}

/// Whether a message here (received at `received`, `size` bytes on the server) and a listed one can be one message: a move
/// keeps the date a message was received and its size, where both say.
fn alike(received: Timestamp, size: u64, there: &GraphMessage) -> bool {
    (received == 0 || there.received == 0 || received == there.received) && (size == 0 || there.server.size == 0 || size == there.server.size)
}

/// Whether `there`, listed in the mailbox `folder`, is the message that a delete for good in the queue waits to name:
/// that delete (the only one, by `Message-ID`, of a message the server last had in this mailbox, [`alike`]) takes its id,
/// and so does the message kept with it, when no other message that is new here (`new_here`) can be that one.
fn named_by_a_delete(mb: &mut Mailbox, account: Id, folder: &str, there: &GraphMessage, new_here: &HashMap<&str, Vec<&GraphMessage>>) -> bool {
    let wanted = message_id(&there.message_id);
    if wanted.is_empty() {
        return false;
    }
    // (A delete that waits, as the message it is for was when it was kept: where the server had it, how long, how big.)
    let waiting = |q: &Queued| match q {
        Queued::DeleteMessage { account: a, id, kept: Some(kept) } => {
            *a == account
                && id.is_empty()
                && message_id(&kept.message_id) == wanted
                && kept.server.as_ref().is_some_and(|server| server.folder == folder && alike(kept.received, server.size, there))
        }
        _ => false,
    };
    let mut found = mb.queued.iter().enumerate().filter(|(_, q)| waiting(q)).map(|(n, _)| n);
    let (Some(n), None) = (found.next(), found.next()) else { return false };
    let Some(Queued::DeleteMessage { kept: Some(kept), .. }) = mb.queued.get(n) else { return false };
    let size = kept.server.as_ref().map_or(0, |server| server.size);
    let rivals = new_here.get(wanted).map_or(0, |all| all.iter().filter(|other| alike(kept.received, size, other)).count());
    if rivals != 1 {
        return false;
    }
    match mb.queued.get_mut(n) {
        Some(Queued::DeleteMessage { id, kept, .. }) => {
            *id = there.server.id.clone();
            // (If the server refuses to destroy it, the message is put back from what was kept, under its id.)
            if let Some(server) = kept.as_mut().and_then(|kept| kept.server.as_mut()) {
                server.id = there.server.id.clone();
            }
            true
        }
        _ => false,
    }
}

/// Whether a message of a window is its marks and nothing else: no `Message-ID`, subject, date, sender,
/// recipients, preview or size. That is how a host sends a message the request said this mailbox has (a
/// message the server has whole always has a size, if nothing else).
fn marks_only(m: &GraphMessage) -> bool {
    m.server.size == 0
        && m.message_id.is_empty()
        && m.subject.is_empty()
        && m.preview.is_empty()
        && m.sent == 0
        && m.received == 0
        && m.from == Address::default()
        && m.to.is_empty()
        && m.cc.is_empty()
}

/// Bring a message this mailbox has in step with what the server says of it now: its keywords,
/// its id and its mailbox. Whether anything changed.
fn update(mb: &mut Mailbox, account: Id, id: Id, folder: Id, server_folder: &str, there: &GraphMessage) -> bool {
    let Some(m) = mb.message(id) else { return false };
    // Listed again by a complete window, a message is no longer gone: that alone is no change.
    let mut before = m.clone();
    if let Some(s) = before.server.as_mut() {
        s.gone = false;
    }
    let last = m.server.clone().unwrap_or_default();
    // Where the server last had it, as this mailbox has that folder: a message still there here
    // follows the server; one moved here since stays where it was put (that move goes up). A gone mark
    // says nothing of that: a complete window marks all it lists before it takes them in.
    let was_here = mb.folder_by_server(account, &last.folder);
    let follows = was_here.is_none_or(|was| m.folder == was);
    let Ok(m) = mb.message_mut(id) else { return false };
    let now = &there.server;
    if m.read == last.read {
        m.read = now.read;
    }
    // JMAP has `$flagged` and nothing more: a follow-up flag marked complete here is no flag there, and
    // stays so until the server says the message is flagged again; a due date is kept while the message
    // stays flagged. Changed here is a different reading of "flagged", not a different status.
    let (flagged_here, flagged_there) = (m.flag == FlagStatus::Flagged, last.flag == FlagStatus::Flagged);
    if flagged_here == flagged_there && !(m.flag == FlagStatus::Complete && now.flag != FlagStatus::Flagged) {
        m.flag = now.flag;
    }
    if m.answered == last.answered {
        m.answered = now.answered;
    }
    if m.forwarded == last.forwarded {
        m.forwarded = now.forwarded;
    }
    if follows {
        m.folder = folder;
    }
    m.server = Some(ServerMessage {
        id: now.id.clone(),
        folder: server_folder.to_string(),
        read: now.read,
        flag: now.flag,
        answered: now.answered,
        forwarded: now.forwarded,
        gone: false,
        // Its size and the rest are what was learnt of it before, unless the window says.
        size: if now.size > 0 { now.size } else { last.size },
        ..last
    });
    *m != before
}

// //// Neoffice — what upstream's Exchange intake (`ews_down.rs`) takes from upstream's `jmap_down`, whose file this fork
// //// Neoffice — replaces with its own: the limits a server's words are held to, the one-line text, the people of a header, the
// //// Neoffice — lines of trouble, and the folders that stay when a complete listing leaves them out. Taken whole from upstream's
// //// Neoffice — v0.8.2+ `jmap_down.rs`; the fork's JMAP run does not use them (its own limits are in `jmap_mail.rs`).
/// Lines of trouble kept of one piece, and the longest, in characters.
const MAX_LINES: usize = 32;
const MAX_LINE: usize = 400;
/// Longest subject and `Message-ID` kept, in characters.
pub(crate) const MAX_TEXT: usize = 998;
/// Longest preview kept, in characters.
pub(crate) const MAX_PREVIEW: usize = 256;
/// Longest name and address kept, in characters, and most addresses of a `To` or a `Cc`.
const MAX_NAME: usize = 256;
const MAX_ADDRESS: usize = 320;
const MAX_PEOPLE: usize = 500;
/// Longest a busy server is waited for, in seconds: a day.
pub(crate) const MAX_BUSY: u32 = 86_400;
/// Most messages of a batch fetched whole, the largest taken (Download messages in full goes up to
/// 50 MB), and the most a batch brings in all, the one message past it taken still.
pub(crate) const MAX_FETCHED: usize = 100;
pub(crate) const MAX_MESSAGE: usize = 50 << 20;
pub(crate) const MAX_BATCH: usize = 64 << 20;

/// `text` on one line: words parted by one space (a control character parts them too), `most`
/// characters at the most.
pub(crate) fn line(text: &str, most: usize) -> String {
    let mut out = String::new();
    let mut length = 0usize;
    for word in text.split(|c: char| c.is_whitespace() || c.is_control()).filter(|word| !word.is_empty()) {
        if length > 0 {
            out.push(' ');
            length = length.saturating_add(1);
        }
        out.push_str(word);
        length = length.saturating_add(word.chars().count());
        if length >= most {
            break;
        }
    }
    out.chars().take(most).collect::<String>().trim_end().to_string()
}

/// An address as a server said it, on one line, or none: one whose address part holds what parts
/// addresses in a list or encloses one (`,` `;` `<` `>` `"`) would be two people, or another one,
/// once it is written into a reply.
pub(crate) fn person(a: &Address) -> Option<Address> {
    let (name, email) = (line(&a.name, MAX_NAME), line(&a.email, MAX_ADDRESS));
    if email.contains([',', ';', '<', '>', '"']) {
        return None;
    }
    (!name.is_empty() || !email.is_empty()).then_some(Address { name, email })
}

pub(crate) fn people(list: &[Address]) -> Vec<Address> {
    list.iter().take(MAX_PEOPLE).filter_map(person).collect()
}

/// The lines of trouble a piece tells of, held to their number and length.
pub(crate) fn lines(said: &[String]) -> Vec<String> {
    said.iter().take(MAX_LINES).map(|said| line(said, MAX_LINE)).filter(|said| !said.is_empty()).collect()
}

/// A draft that is here whole: it may have been changed here, and nothing goes up yet, so what the
/// server does with its copy doesn't take it away.
fn draft_here(m: &Message) -> bool {
    m.draft && !m.unfetched
}

/// Most folders followed upwards from one that stays, to the ones above it that stay with it.
const MAX_DEPTH: usize = 64;

/// Before a complete listing is taken in ([`super::graph_mail::take_listing`], which removes a
/// folder whose mailbox it doesn't have, with the mail in it): such a folder stays when it is one
/// of the account's standard folders, or holds mail that only this computer has (moved or
/// imported here, or a draft that is here whole: nothing goes up yet), and so do the folders above
/// it whose mailboxes are gone too. It stays as a folder of this computer alone, without what the
/// server had in that mailbox; and what of such a mailbox lies anywhere else here (moved there
/// here), or is a draft here whole, is this computer's own from now on. How many messages went.
pub(crate) fn keep_what_is_only_here(mb: &mut Mailbox, account: Id, listing: &FolderListing) -> usize {
    if !listing.complete {
        return 0;
    }
    let listed: HashSet<&str> = listing.folders.iter().map(|f| f.id.as_str()).collect();
    let only_here: HashSet<Id> =
        mb.messages.values().filter(|m| m.account == account && (m.server.is_none() || draft_here(m))).map(|m| m.folder).collect();
    // The folders whose mailbox is gone, by the id the server gave the mailbox; and which of them stay.
    let mut dropped: HashMap<String, Id> = HashMap::new();
    let mut kept: HashSet<Id> = HashSet::new();
    for f in mb.folders.iter().filter(|f| f.account == account) {
        let Some(server) = f.server.as_ref().filter(|server| !listed.contains(server.id.as_str())) else { continue };
        dropped.insert(server.id.clone(), f.id);
        if f.kind != FolderKind::Custom || only_here.contains(&f.id) {
            kept.insert(f.id);
        }
    }
    if dropped.is_empty() {
        return 0;
    }
    let gone_too: HashSet<Id> = dropped.values().copied().collect();
    for folder in kept.clone() {
        let mut at = mb.folder(folder).and_then(|f| f.parent);
        for _ in 0..MAX_DEPTH {
            let Some(parent) = at.filter(|parent| gone_too.contains(parent)) else { break };
            kept.insert(parent);
            at = mb.folder(parent).and_then(|f| f.parent);
        }
    }
    let before = mb.messages.len();
    // In a folder that stays: what lay in its mailbox goes with it, but a draft here whole.
    mb.messages.retain(|_, m| {
        let lay_there = m.server.as_ref().is_some_and(|server| dropped.get(&server.folder) == Some(&m.folder));
        !(kept.contains(&m.folder) && m.account == account && lay_there && !draft_here(m))
    });
    let gone = before.saturating_sub(mb.messages.len());
    // What else of those mailboxes is here, and not in a folder that goes (whose mail goes with it),
    // is this computer's now.
    let doomed: HashSet<Id> = gone_too.difference(&kept).copied().collect();
    for m in mb.messages.values_mut() {
        let of_one = m.account == account && m.server.as_ref().is_some_and(|server| dropped.contains_key(&server.folder));
        if of_one && !doomed.contains(&m.folder) {
            Arc::make_mut(m).server = None;
        }
    }
    for f in mb.folders.iter_mut().filter(|f| kept.contains(&f.id)) {
        f.server = None;
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_what_rfc_8620_allows_and_not_too_long() {
        for id in ["a", "Mabc_-09", "E1V2Zx", &"x".repeat(MAX_ID)] {
            assert!(plausible_id(id), "{id:?}");
        }
        for id in ["", " a", "a b", "a/b", "a.b", "é", "a\n", &"x".repeat(MAX_ID + 1)] {
            assert!(!plausible_id(id), "{id:?}");
        }
    }

    #[test]
    fn a_message_id_is_compared_without_brackets() {
        assert_eq!(message_id(" <abc@example.org> "), "abc@example.org");
        assert_eq!(message_id("abc@example.org"), "abc@example.org");
        assert_eq!(message_id("<>"), "");
    }
}
