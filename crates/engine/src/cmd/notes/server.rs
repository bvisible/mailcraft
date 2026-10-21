// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A server's notes in the engine: read into the mailbox (`sync.applyNotes`, [`merge_notes`]); what is made, changed
//! and deleted of them here noted for the server ([`note_created`], [`note_changed`], [`note_deleted`]) and handed to a
//! host (`sync.noteChanges`, [`note_upload`]); and what the server said of each taken in (`sync.applyNoteChanges`,
//! [`apply_note_outcomes`]).
//!
//! - **What goes** is worked out when it goes: a new note whole, with the mark it is made with (sent twice, it makes
//!   one note); a changed one as what differs from what the server had (the change's `base`), with the version it was
//!   made from when its words changed; a deletion. Several changes before the server hears of any are one, a change
//!   taken back is none, and a note made and deleted before it went tells the server nothing.
//! - **Nothing typed is lost.** A change of words the server refuses because it has a newer version (stale), or makes
//!   to a note it no longer has, is kept as a note of its own, and the person is told; the note itself is put back to
//!   what the server has.
//! - **Until the server was told**, a read leaves a note whose change waits as it is here (the fields it changes; the
//!   others as the server has them), and doesn't bring back one deleted here.
//! - **An answer about a change that is no longer noted** (it was undone while it was on its way): the server did what
//!   the mailbox no longer says, so the opposite is noted.
//! - **A change that went and got no answer** stays noted with what went ([`NoteChange::sent`]), through an undo and
//!   words typed back: the server may have written it, so what the note says goes there over it, and a read takes the
//!   server's copy of what went for hers, never for someone else's words. A deletion that went without an answer and
//!   was undone waits to know whether the server deleted the note ([`NoteChange::undeleted`]): if it did, the note is
//!   made there anew.
//! - **A server that stops keeping notes**, or an account whose address or server is changed, lets its notes go; what
//!   waited to go stays, as this computer's own, or held by a page that keeps none ([`NoteHold`]) until the very same
//!   server keeps notes again: then each goes back as the change of that very note it was, never as a copy.
//!
//! Like mail that arrives, taking a server's answer or its notes in is not an undo step; once a server was told
//! anything, the undo history is forgotten.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mailcraft_model::note::{
    MAX_NOTE_CHANGES, MAX_NOTE_OPS, MAX_NOTES, Note, NoteChange, NoteChangeKind, NoteFields, NoteHold, NoteOp, NoteOutcome, NoteResult, NoteRights,
    NotesDownload, NotesUpload, NotesUploaded, RemoteNote,
};
use mailcraft_model::{Id, Mailbox, Origin};
use serde_json::{Value, json};

use super::super::remote::{EventsWaiting, new_token, owner, wait_after};
use super::super::{bad, fail};
use super::words::{
    CHANGE_PUT_BACK, KEPT_AS_COPY, KEPT_REFUSED, KEPT_WHEN_GONE, KEPT_WHEN_UNSHARED, MADE_ANEW, NO_LONGER_KEPT, NO_LONGER_KEPT_PAGE,
    NO_ROOM_FOR_COPY, NOT_DELETED, NOT_SAVED, TOO_MANY_NOTE_CHANGES, UNDO_NOT_IN_STEP, fill,
};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// A version older than any a server gives: what a change of words is told against when the version its words were read
/// at isn't known, so that a server that has other words refuses it as stale rather than write over them.
pub const OLDEST_VERSION: &str = "1970-01-01T00:00:00Z";

/// The version a change made from `version` is told against: that one, or [`OLDEST_VERSION`] when none is known.
fn older(version: &str) -> String {
    if version.is_empty() { OLDEST_VERSION.to_string() } else { version.to_string() }
}

/// A title for a note of its own made of the person's words over `heading`'s note: "Offsite (2)", "(3)" when that one
/// is taken, and so on, in `most` characters at most; and whether the heading had to be cut to fit (the whole of it then
/// goes first in the note's text, so that nothing of it is lost).
pub fn copy_title(mailbox: &Mailbox, heading: &str, most: usize) -> (String, bool) {
    let heading = heading.trim();
    let mut last = (String::new(), false);
    for n in 2..1_000u32 {
        let suffix = format!(" ({n})");
        let room = most.saturating_sub(suffix.chars().count());
        let kept: String = heading.chars().take(room).collect();
        last = (format!("{}{suffix}", kept.trim_end()), kept.chars().count() < heading.chars().count());
        if !mailbox.notes.items.values().any(|note| note.title == last.0) {
            break;
        }
    }
    last
}

/// Most outcomes taken from one answer, and most lines said of them.
const MAX_OUTCOMES: usize = 500;
const MAX_MESSAGES: usize = 20;
/// The longest text of a parameter error handed back.
const MAX_SHOWN: usize = 200;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("sync.applyNotes", "Apply Downloaded Notes", "Send / Receive", apply_notes_command)
            .pure()
            .params(r#"{"download": NotesDownload} — what a host read of an account's notes"#),
        CommandSpec::new("sync.noteChanges", "Note Changes to Send", "Send / Receive", note_changes_command)
            .pure()
            .params(
                r#"{"account": number} — the changes of that account's notes its server is to be told now, and `needRead`: whether its notes are to be read whole first (a change of words waits for the version a read brings)"#,
            ),
        CommandSpec::new("sync.applyNoteChanges", "Apply Sent Note Changes", "Send / Receive", apply_note_changes_command)
            .pure()
            .params(r#"{"result": NotesUploaded} — what the server made of them"#),
    ]
}

// ---- what is noted ----

/// Whether one more change of a note may be noted.
pub(super) fn room(mailbox: &Mailbox) -> Result<(), CmdError> {
    if mailbox.notes.changes.len() >= MAX_NOTE_CHANGES { Err(fail(TOO_MANY_NOTE_CHANGES)) } else { Ok(()) }
}

/// The fields of a note a server is told of: everything for a new one.
fn fields_of(note: &Note) -> NoteFields {
    NoteFields { title: Some(note.title.clone()), text: Some(note.text.clone()), pinned: Some(note.pinned), archived: Some(note.archived) }
}

/// What of `now` differs from `base`.
fn changed(base: &Note, now: &Note) -> NoteFields {
    NoteFields {
        title: (base.title != now.title).then(|| now.title.clone()),
        text: (base.text != now.text).then(|| now.text.clone()),
        pinned: (base.pinned != now.pinned).then_some(now.pinned),
        archived: (base.archived != now.archived).then_some(now.archived),
    }
}

/// Of `base`, the parts `named` names: what they were before the change that names them.
fn as_was(base: &Note, named: &NoteFields) -> NoteFields {
    NoteFields {
        title: named.title.as_ref().map(|_| base.title.clone()),
        text: named.text.as_ref().map(|_| base.text.clone()),
        pinned: named.pinned.map(|_| base.pinned),
        archived: named.archived.map(|_| base.archived),
    }
}

/// Of what went of `change` and was never answered ([`NoteChange::sent`]), the parts that differ from `note` now, as
/// `note` has them: the server may hold what went, and is to be told what the note says.
fn unanswered(change: &NoteChange, note: &Note) -> NoteFields {
    let mut parts = NoteFields::default();
    for sent in &change.sent {
        if sent.title.as_ref().is_some_and(|title| *title != note.title) {
            parts.title = Some(note.title.clone());
        }
        if sent.text.as_ref().is_some_and(|text| *text != note.text) {
            parts.text = Some(note.text.clone());
        }
        if sent.pinned.is_some_and(|pinned| pinned != note.pinned) {
            parts.pinned = Some(note.pinned);
        }
        if sent.archived.is_some_and(|archived| archived != note.archived) {
            parts.archived = Some(note.archived);
        }
    }
    parts
}

/// `fields` and, of `more`, the parts `fields` doesn't name.
fn with(fields: NoteFields, more: NoteFields) -> NoteFields {
    NoteFields {
        title: fields.title.or(more.title),
        text: fields.text.or(more.text),
        pinned: fields.pinned.or(more.pinned),
        archived: fields.archived.or(more.archived),
    }
}

/// `note` with what `fields` say made to it.
fn put(note: &mut Note, fields: &NoteFields) {
    if let Some(title) = &fields.title {
        note.title.clone_from(title);
    }
    if let Some(text) = &fields.text {
        note.text.clone_from(text);
    }
    if let Some(pinned) = fields.pinned {
        note.pinned = pinned;
    }
    if let Some(archived) = fields.archived {
        note.archived = archived;
    }
}

fn waiting_create(change: &NoteChange, note: Id) -> bool {
    change.kind == NoteChangeKind::Create && change.note == note
}

/// The note `id` was made here, in the notes of a server: the server is to be told, with a mark of its own.
pub(super) fn note_created(mailbox: &mut Mailbox, id: Id, salt: u64) {
    let Some(note) = mailbox.notes.note(id) else { return };
    if note.origin.is_local() || !note.origin.id.is_empty() || mailbox.notes.changes.iter().any(|change| waiting_create(change, id)) {
        return;
    }
    let account = note.origin.account;
    let token = new_token(id, salt);
    if let Some(note) = mailbox.notes.note_mut(id) {
        note.token.clone_from(&token);
        note.error.clear();
    }
    mailbox.notes.changes.push(NoteChange { account, kind: NoteChangeKind::Create, note: id, token, ..Default::default() });
}

/// The note `id`, whose server no longer has it, is one of the person's own, to be made there anew with its words: a new
/// note of that server's, as if it had been made here.
fn anew(mailbox: &mut Mailbox, id: Id, salt: u64) {
    if let Some(note) = mailbox.notes.note_mut(id) {
        note.origin.id.clear();
        note.version.clear();
        note.error.clear();
        (note.shared, note.rights) = (false, NoteRights::default());
        note.owner_name.clear();
        note.web_url.clear();
    }
    note_created(mailbox, id, salt);
}

/// A server's note was changed here; `before` is what it was. The server is to be told what differs from its own copy:
/// `before`, or what it was before an earlier change that still waits. A change that takes everything back to that
/// leaves nothing to tell. One the server doesn't have yet is sent as it is when it goes (and one whose making was
/// refused goes again).
pub(super) fn note_changed(mailbox: &mut Mailbox, before: &Note, salt: u64) {
    let Some(now) = mailbox.notes.note(before.id).cloned() else { return };
    if now.origin.is_local() {
        return;
    }
    if now.origin.id.is_empty() {
        if let Some(change) = mailbox.notes.changes.iter_mut().find(|change| waiting_create(change, now.id)) {
            change.retry_at = 0;
        } else {
            note_created(mailbox, now.id, salt);
        }
        return;
    }
    let waiting = |change: &NoteChange| change.kind == NoteChangeKind::Update && change.note == now.id && change.remote == now.origin.id;
    match mailbox.notes.changes.iter().position(waiting) {
        Some(at) => {
            let back = mailbox.notes.changes.get(at).and_then(|change| change.base.as_deref()).is_some_and(|base| changed(base, &now).is_empty());
            // (Unless what went of it was never answered and isn't what the note says now: the server may have written it,
            // and is to be told what the note says, her undo or words typed back.)
            // (Nor one that waits to know whether the server deleted the note: [`NoteChange::undeleted`].)
            let unsure = mailbox.notes.changes.get(at).is_some_and(|change| change.undeleted || !unanswered(change, &now).is_empty());
            if back && !unsure {
                let change = mailbox.notes.changes.remove(at);
                // The note is as the server had it when it was read: so is its version (one a read brought meanwhile
                // is of other words).
                if let (Some(base), Some(note)) = (change.base.as_deref(), mailbox.notes.note_mut(now.id)) {
                    note.version.clone_from(&base.version);
                }
            } else if let Some(change) = mailbox.notes.changes.get_mut(at) {
                change.retry_at = 0;
            }
        }
        None if changed(before, &now).is_empty() => {}
        None => mailbox.notes.changes.push(NoteChange {
            account: now.origin.account,
            kind: NoteChangeKind::Update,
            note: now.id,
            remote: now.origin.id.clone(),
            base: Some(Box::new(before.clone())),
            ..Default::default()
        }),
    }
}

/// A server's note was deleted here; `before` is what it was. The server is to delete it too; one it never had needs no
/// telling (its note to make it goes).
pub(super) fn note_deleted(mailbox: &mut Mailbox, before: &Note) {
    if before.origin.is_local() {
        return;
    }
    if before.origin.id.is_empty() {
        mailbox.notes.changes.retain(|change| !waiting_create(change, before.id));
        return;
    }
    // A change that waited is overtaken: what the server has is still what it was before it.
    let waiting = |change: &NoteChange| change.kind == NoteChangeKind::Update && change.note == before.id;
    let base = mailbox.notes.changes.iter().find(|change| waiting(change)).and_then(|change| change.base.clone());
    mailbox.notes.changes.retain(|change| change.note != before.id);
    mailbox.notes.changes.push(NoteChange {
        account: before.origin.account,
        kind: NoteChangeKind::Delete,
        note: before.id,
        remote: before.origin.id.clone(),
        base: Some(base.unwrap_or_else(|| Box::new(before.clone()))),
        ..Default::default()
    });
}

/// An undo or redo goes from `now` to `back`: a change of a server's note that went and was never answered stays noted in
/// `back`, with what went (the server may have written it), so that what `back` says goes there over it, as soon as the
/// server may be asked (as a change typed does).
///
/// A deletion that went without an answer, undone: the note is here again, and the server may have deleted it. A change
/// of it is noted that waits to know ([`NoteChange::undeleted`]): if the server did delete it, it is made there anew with
/// her words; if not, nothing is made twice.
pub(crate) fn keep_unanswered(back: &mut Mailbox, now: &Mailbox) -> Vec<String> {
    let mut said = Vec::new();
    for change in now.notes.changes.iter().filter(|change| change.kind == NoteChangeKind::Delete && !change.sent.is_empty()) {
        let Some(id) = back.notes.by_remote(change.account, &change.remote).map(|note| note.id) else { continue };
        let deleted = back.notes.changes.iter().any(|kept| kept.kind == NoteChangeKind::Delete && kept.remote == change.remote);
        if deleted {
            continue;
        }
        let same = |kept: &&mut NoteChange| kept.kind == NoteChangeKind::Update && kept.note == id && kept.remote == change.remote;
        let full = back.notes.changes.len() >= MAX_NOTE_CHANGES;
        match back.notes.changes.iter_mut().find(same) {
            Some(kept) => (kept.undeleted, kept.retry_at) = (true, 0),
            None if full => {
                log::warn!("notes: a deletion undone after it went unanswered couldn't be noted: too many changes wait");
                said.push(UNDO_NOT_IN_STEP.to_string());
            }
            None => {
                let base = change.base.clone().or_else(|| back.notes.note(id).map(|note| Box::new(note.clone())));
                back.notes.changes.push(NoteChange {
                    account: change.account,
                    kind: NoteChangeKind::Update,
                    note: id,
                    remote: change.remote.clone(),
                    base,
                    undeleted: true,
                    ..Default::default()
                });
            }
        }
    }
    for change in now.notes.changes.iter().filter(|change| change.kind == NoteChangeKind::Update && !change.sent.is_empty()) {
        let here = back.notes.note(change.note).is_some_and(|note| note.origin.account == change.account && note.origin.id == change.remote);
        if !here {
            continue;
        }
        let same = |kept: &&mut NoteChange| kept.kind == NoteChangeKind::Update && kept.note == change.note && kept.remote == change.remote;
        let full = back.notes.changes.len() >= MAX_NOTE_CHANGES;
        match back.notes.changes.iter_mut().find(same) {
            Some(kept) => {
                for sent in &change.sent {
                    if !kept.sent.contains(sent) {
                        kept.sent.push(sent.clone());
                    }
                }
                let over = kept.sent.len().saturating_sub(mailcraft_model::note::MAX_UNANSWERED);
                kept.sent.drain(..over);
                kept.retry_at = 0;
            }
            None if full => {
                log::warn!("notes: a change undone after it went unanswered couldn't be noted: too many changes wait");
                said.push(UNDO_NOT_IN_STEP.to_string());
            }
            None => back.notes.changes.push(NoteChange { retry_at: 0, ..change.clone() }),
        }
    }
    said.dedup();
    said
}

// ---- what is sent ----

/// Whether a noted change still hangs together with the mailbox: its account and its server are here, and the note it
/// is about (not a deletion's).
fn fits(mailbox: &Mailbox, change: &NoteChange) -> bool {
    mailbox.account(change.account).is_some()
        && mailbox.notes.server(change.account).is_some()
        && match change.kind {
            NoteChangeKind::Create => {
                mailbox.notes.note(change.note).is_some_and(|note| note.origin.account == change.account && note.origin.id.is_empty())
            }
            NoteChangeKind::Update => mailbox
                .notes
                .note(change.note)
                .is_some_and(|note| note.origin.account == change.account && note.origin.id == change.remote && change.base.is_some()),
            NoteChangeKind::Delete => !change.remote.is_empty(),
        }
}

/// Give up the changes that no longer hang together with the mailbox (their note, account or server is gone): for the
/// log, how many.
pub fn settle_note_changes(s: &mut Session) -> usize {
    let before = s.mailbox.notes.changes.len();
    let kept: Vec<NoteChange> = s.mailbox.notes.changes.iter().filter(|change| fits(&s.mailbox, change)).cloned().collect();
    let dropped = before - kept.len();
    if dropped > 0 {
        log::warn!("notes: {dropped} waiting change(s) no longer matched the notes and were given up");
        s.mailbox.notes.changes = kept;
        s.mailbox_changed();
    }
    dropped
}

/// Whether a change waits for a read rather than for a moment: a change of words whose version isn't known, or one of
/// a note whose deletion was undone with nothing more to tell ([`notes_need_read`]).
fn waits_for_read(mailbox: &Mailbox, change: &NoteChange) -> bool {
    if change.kind != NoteChangeKind::Update {
        return false;
    }
    let (Some(note), Some(base)) = (mailbox.notes.note(change.note), change.base.as_deref()) else { return false };
    let fields = with(changed(base, note), unanswered(change, note));
    (fields.has_words() && base.version.is_empty()) || (change.undeleted && fields.is_empty())
}

/// How much of an account's notes waits to go to its server: due now, all of it, and when the next held back is due.
/// (What waits for a read is not due: [`notes_need_read`].)
pub fn notes_waiting(s: &Session, account: Id) -> EventsWaiting {
    let mut waiting = EventsWaiting::default();
    let now = s.now();
    for change in s.mailbox.notes.changes.iter().filter(|change| change.account == account && fits(&s.mailbox, change)) {
        waiting.total += 1;
        if waits_for_read(&s.mailbox, change) {
            // (Not due: nothing of it can go before a read, which [`notes_need_read`] asks for.)
        } else if change.retry_at <= now {
            waiting.due += 1;
        } else {
            waiting.next = Some(waiting.next.map_or(change.retry_at, |next| next.min(change.retry_at)));
        }
    }
    waiting
}

/// Whether `account`'s notes are to be read whole before what waits can go: a change of words whose version isn't
/// known (an answer that didn't say the note's words, a version lost) waits for the one a read brings; so does a note
/// whose deletion was undone after it went without an answer ([`NoteChange::undeleted`]).
pub fn notes_need_read(s: &Session, account: Id) -> bool {
    s.mailbox.notes.changes.iter().filter(|change| change.account == account && change.kind == NoteChangeKind::Update).any(|change| {
        let note = s.mailbox.notes.note(change.note);
        let words = note.zip(change.base.as_deref()).is_some_and(|(note, base)| with(changed(base, note), unanswered(change, note)).has_words());
        // (And a note whose deletion was undone after it went without an answer: whether the server deleted it.)
        change.undeleted || (words && change.base.as_deref().is_some_and(|base| base.version.is_empty()))
    })
}

/// What goes to the server now ([`note_upload`]) is on its way: what each change of it carries is kept with the change
/// until an answer comes, as what the server may have written already.
pub fn notes_sending(s: &mut Session, upload: &NotesUpload) {
    let mut kept = false;
    for op in upload.ops.iter().filter(|op| op.kind == NoteChangeKind::Update) {
        let going = |change: &&mut NoteChange| change.account == upload.account && change.kind == NoteChangeKind::Update && change.note == op.note;
        if let Some(change) = s.mailbox.notes.changes.iter_mut().find(going) {
            // (The last few; the same once.)
            change.sent.retain(|sent| *sent != op.fields);
            change.sent.push(op.fields.clone());
            if change.sent.len() > mailcraft_model::note::MAX_UNANSWERED {
                change.sent.remove(0);
            }
            kept = true;
        }
    }
    // A deletion: that it went.
    for op in upload.ops.iter().filter(|op| op.kind == NoteChangeKind::Delete) {
        let going =
            |change: &&mut NoteChange| change.account == upload.account && change.kind == NoteChangeKind::Delete && change.remote == op.remote;
        if let Some(change) = s.mailbox.notes.changes.iter_mut().find(going).filter(|change| change.sent.is_empty()) {
            change.sent.push(NoteFields::default());
            kept = true;
        }
    }
    // (Kept with the mailbox, which is saved: a window closed before the answer still knows it went.)
    if kept {
        s.mailbox_changed();
    }
}

/// What the server of `account` is to be told now of its notes made, changed and deleted here: the changes that are
/// due, oldest first, [`MAX_NOTE_OPS`] at most.
pub fn note_upload(s: &Session, account: Id) -> Result<NotesUpload, CmdError> {
    let owner = s.mailbox.account(account).ok_or_else(|| fail(format!("no account with id {account}")))?;
    let now = s.now();
    let mut ops = Vec::new();
    for change in s.mailbox.notes.changes.iter().filter(|change| change.account == account && change.retry_at <= now) {
        if ops.len() >= MAX_NOTE_OPS {
            break;
        }
        if !fits(&s.mailbox, change) {
            continue;
        }
        let note = s.mailbox.notes.note(change.note);
        let op = match (change.kind, note, change.base.as_deref()) {
            (NoteChangeKind::Create, Some(note), _) => {
                NoteOp { kind: NoteChangeKind::Create, note: note.id, token: change.token.clone(), fields: fields_of(note), ..Default::default() }
            }
            (NoteChangeKind::Update, Some(note), Some(base)) => {
                // What differs from the server's copy, and what went without an answer and isn't what the note says now.
                let fields = with(changed(base, note), unanswered(change, note));
                if fields.is_empty() {
                    continue;
                }
                // A change of words goes with the version it was made from, or not before a read brings it
                // ([`notes_need_read`]): without one, it would be written over whatever is there.
                if fields.has_words() && base.version.is_empty() {
                    continue;
                }
                let version = if fields.has_words() { base.version.clone() } else { String::new() };
                let earlier = change.sent.clone();
                NoteOp { kind: NoteChangeKind::Update, note: note.id, remote: change.remote.clone(), fields, version, earlier, ..Default::default() }
            }
            (NoteChangeKind::Delete, _, _) => {
                NoteOp { kind: NoteChangeKind::Delete, note: change.note, remote: change.remote.clone(), ..Default::default() }
            }
            _ => continue,
        };
        ops.push(op);
    }
    Ok(NotesUpload { account: owner.id, email: owner.email.trim().to_string(), ops })
}

// ---- a download ----

/// What taking in a download of notes came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotesMerged {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    /// What the owner should hear, a line each.
    pub messages: Vec<String>,
}

impl NotesMerged {
    fn json(&self) -> Value {
        json!({"added": self.added, "updated": self.updated, "removed": self.removed, "unchanged": self.unchanged, "messages": self.messages})
    }
}

/// The server of `account` keeps no notes (any more), or the account is now another's (its address or server changed):
/// its notes go, and what waited to go there is kept, never dropped: on this computer as its own, or, on a page that
/// keeps none of its own, held until the page is left (it asks before) or the very same server keeps notes again
/// ([`NoteHold`]).
fn let_go(s: &mut Session, account: Id) -> NotesMerged {
    let mut merged = NotesMerged::default();
    let keeps_local = s.keeps_local;
    let (email, session) = s.mailbox.account(account).map(|a| (a.email.trim().to_string(), session_of(a))).unwrap_or_default();
    let notes = &mut s.mailbox.notes;
    let had = notes.servers.iter().any(|server| server.account == account) || notes.items.values().any(|note| note.origin.account == account);
    if !had {
        return merged;
    }
    // (A new note, one changed here, and one the server refused to make, here alone: the words that waited to go; and
    // what each change was made over, the mark a new one went with, and what went without an answer.)
    let mut waiting: HashMap<Id, NoteHold> = HashMap::new();
    for change in notes.changes.iter().filter(|change| change.account == account && change.kind != NoteChangeKind::Delete) {
        let hold = match change.kind {
            NoteChangeKind::Update => NoteHold { base: change.base.clone(), sent: change.sent.clone(), ..Default::default() },
            _ => NoteHold { token: change.token.clone(), ..Default::default() },
        };
        waiting.insert(change.note, hold);
    }
    for note in notes.items.values().filter(|note| note.origin.account == account && note.origin.id.is_empty()) {
        waiting.entry(note.id).or_insert_with(|| NoteHold { token: note.token.clone(), ..Default::default() });
    }
    notes.servers.retain(|server| server.account != account);
    notes.changes.retain(|change| change.account != account);
    let ours: Vec<Id> = notes.items.values().filter(|note| note.origin.account == account).map(|note| note.id).collect();
    for id in ours {
        let Some(hold) = waiting.get(&id) else {
            notes.items.remove(&id);
            merged.removed += 1;
            continue;
        };
        if let Some(note) = notes.note_mut(id) {
            let held = (!keeps_local).then(|| {
                let remote = if hold.base.is_some() { note.origin.id.clone() } else { String::new() };
                Box::new(NoteHold { account, email: email.clone(), session: session.clone(), remote, ..hold.clone() })
            });
            let mut own = Note {
                id,
                title: note.title.clone(),
                text: note.text.clone(),
                pinned: note.pinned,
                archived: note.archived,
                held,
                ..Default::default()
            };
            (own.created, own.updated) = (note.created, note.updated);
            *note = own;
        }
    }
    if !waiting.is_empty() {
        merged.messages.push(if keeps_local { NO_LONGER_KEPT } else { NO_LONGER_KEPT_PAGE }.to_string());
    }
    s.mailbox_changed();
    merged
}

/// What a hold says the server of an account is: its JMAP session, or none.
pub fn session_of(account: &mailcraft_model::Account) -> String {
    account.jmap.as_ref().map(|jmap| jmap.session.trim().to_string()).unwrap_or_default()
}

/// The notes a page held of `account`'s server, given back to it now that it keeps notes again, when it is the very same
/// server (the account's address and session as they were): a new one is to be made there; one it had, a change of it
/// that waits to know whether it is still there ([`NoteChange::undeleted`], [`NoteChange::resumed`]), told against the
/// oldest version so that it never goes over other words. As many as there is room to note; how many.
fn give_back(mailbox: &mut Mailbox, account: Id, salt: u64) -> usize {
    let mut held = held_here(mailbox, account);
    held.sort_unstable();
    let mut given = 0;
    for id in held {
        if mailbox.notes.changes.len() >= MAX_NOTE_CHANGES {
            break;
        }
        let Some(hold) = mailbox.notes.note_mut(id).and_then(|note| note.held.take()) else { continue };
        let NoteHold { remote, base, token, sent, .. } = *hold;
        // (A copy of it a read brought anyway, with nothing of its own waiting, is the server's: hers takes its place.)
        let twin = mailbox.notes.by_remote(account, &remote).map(|note| note.id).filter(|twin| *twin != id && !remote.is_empty());
        if let Some(twin) = twin.filter(|twin| !mailbox.notes.changes.iter().any(|change| change.note == *twin)) {
            mailbox.notes.items.remove(&twin);
        }
        let had = !remote.is_empty() && mailbox.notes.by_remote(account, &remote).is_none();
        match base.filter(|_| had) {
            Some(mut base) => {
                let Some(note) = mailbox.notes.note_mut(id) else { continue };
                // Hers as she left it, the server's as it was.
                let mut back = Note { id, origin: Origin { account, id: remote.clone(), ..Default::default() }, ..(*base).clone() };
                (back.title, back.text, back.pinned, back.archived) = (note.title.clone(), note.text.clone(), note.pinned, note.archived);
                (back.created, back.updated) = (note.created, note.updated);
                back.token.clear();
                back.error.clear();
                let mine = changed(&base, &back);
                *note = back;
                // Nothing of hers left: it is the server's note, as any other. Otherwise the change goes, never over
                // other words; made anew if the server deleted it only when her words are in it.
                if !mine.is_empty() || !sent.is_empty() {
                    base.version = OLDEST_VERSION.to_string();
                    mailbox.notes.changes.push(NoteChange {
                        account,
                        kind: NoteChangeKind::Update,
                        note: id,
                        remote,
                        base: Some(base),
                        sent,
                        undeleted: mine.has_words(),
                        resumed: true,
                        ..Default::default()
                    });
                }
            }
            None => {
                if let Some(note) = mailbox.notes.note_mut(id) {
                    note.origin.account = account;
                }
                if token.is_empty() {
                    note_created(mailbox, id, salt);
                } else {
                    // Sent with its mark again: the server makes no second one.
                    if let Some(note) = mailbox.notes.note_mut(id) {
                        note.token.clone_from(&token);
                    }
                    mailbox.notes.changes.push(NoteChange { account, kind: NoteChangeKind::Create, note: id, token, ..Default::default() });
                }
            }
        }
        given += 1;
    }
    given
}

/// The notes a page holds of `account`'s server as it is now (its address and session as they were then).
fn held_here(mailbox: &Mailbox, account: Id) -> Vec<Id> {
    let Some(owner) = mailbox.account(account) else { return Vec::new() };
    let (email, session) = (owner.email.trim().to_string(), session_key(&session_of(owner)));
    let same = |hold: &NoteHold| hold.account == account && hold.email.eq_ignore_ascii_case(&email) && session_key(&hold.session) == session;
    mailbox.notes.items.values().filter(|note| note.origin.is_local() && note.held.as_deref().is_some_and(same)).map(|note| note.id).collect()
}

/// A JMAP session's address as two are compared: without the spaces around it or a slash at its end, its scheme and
/// host in small letters.
pub fn session_key(session: &str) -> String {
    let session = session.trim().trim_end_matches('/');
    match session.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
            format!("{}://{}/{path}", scheme.to_ascii_lowercase(), host.to_ascii_lowercase())
        }
        None => session.to_string(),
    }
}

/// A job's answer that says which server it asked (`session`) is `owner`'s only while that is still its server: one
/// moved to another since would take another server's notes in, and send them back there under its names. As
/// [`owner`] checks the address.
fn same_server(owner: &mailcraft_model::Account, session: &str) -> Result<(), CmdError> {
    if session.trim().is_empty() || session_key(session) == session_key(&session_of(owner)) {
        Ok(())
    } else {
        Err(fail(format!("this answer was from another server than {}'s now", owner.name)))
    }
}

/// `account`'s address or server was changed: it is another's now, and what was the old server's of notes is let go as
/// when a server stops keeping notes ([`let_go`]); nothing of it goes to the new one. What is said of it, for the log.
pub fn account_moved(s: &mut Session, account: Id) -> Vec<String> {
    // (A job of its notes under way is of the server it had: its answer is set aside, wherever the account goes next.)
    let moves = s.notes_moves.entry(account).or_default();
    *moves = moves.saturating_add(1);
    let said = let_go(s, account).messages;
    // (Said in the status bar: where her waiting notes are now.)
    if !said.is_empty() {
        s.sync_status = said.join(" ");
    }
    said
}

/// Take in what a server has of an account's notes ([`NotesDownload`]): see the module. Nothing is changed when this
/// returns an error.
pub fn merge_notes(s: &mut Session, download: &NotesDownload) -> Result<NotesMerged, CmdError> {
    let owner = owner(&s.mailbox, download.account, &download.email)?;
    same_server(owner, &download.session)?;
    let account = owner.id;
    let mut download = download.clone();
    download.tidy();
    let mut merged = NotesMerged::default();
    let keeps_local = s.keeps_local;
    if !download.offered {
        return Ok(let_go(s, account));
    }
    let mut server = download.server.clone();
    server.account = account;
    match s.mailbox.notes.servers.iter_mut().find(|kept| kept.account == account) {
        Some(kept) => *kept = server,
        None => s.mailbox.notes.servers.push(server),
    }
    // The notes a page held when this very server stopped keeping notes (it keeps none of its own) go back to it.
    let salt = s.rev();
    let held = if keeps_local { 0 } else { give_back(&mut s.mailbox, account, salt) };
    // (Those it couldn't give back yet are that server's notes: a read doesn't bring them in beside them.)
    let still_held: HashSet<String> = held_here(&s.mailbox, account)
        .into_iter()
        .filter_map(|id| s.mailbox.notes.note(id).and_then(|note| note.held.as_ref()).map(|hold| hold.remote.clone()))
        .filter(|remote| !remote.is_empty())
        .collect();
    let notes = &s.mailbox.notes;
    // The notes a change of waits for (they are as they are here), and those deleted here (they stay deleted).
    let waiting: HashSet<Id> = notes.changes.iter().filter(|change| change.account == account).map(|change| change.note).collect();
    let deleted: HashSet<String> = notes
        .changes
        .iter()
        .filter(|change| change.account == account && change.kind == NoteChangeKind::Delete)
        .map(|change| change.remote.clone())
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    for remote in &download.notes {
        seen.insert(remote.id.clone());
        if deleted.contains(&remote.id) || still_held.contains(&remote.id) {
            continue;
        }
        let here =
            s.mailbox.notes.items.values().filter(|note| note.origin.account == account && note.origin.id == remote.id).map(|note| note.id).min();
        let Some(id) = here else {
            if s.mailbox.notes.items.len() < MAX_NOTES {
                let id = s.mailbox.alloc_id();
                s.mailbox.notes.items.insert(id, Arc::new(remote.note(account, id)));
                merged.added += 1;
            }
            continue;
        };
        let Some(old) = s.mailbox.notes.note(id).cloned() else { continue };
        let mut fresh = remote.note(account, id);
        (fresh.token, fresh.error) = (old.token.clone(), old.error.clone());
        if waiting.contains(&id) {
            // What a waiting change changes stays as it is here; the rest is the server's, and so is what the change
            // is told against (but the version it was made from).
            let Some(at) = s.mailbox.notes.changes.iter().position(|change| change.note == id && change.base.is_some()) else { continue };
            let Some(base) = s.mailbox.notes.changes.get(at).and_then(|change| change.base.as_deref()).cloned() else { continue };
            // (Hers too: what went without an answer and isn't what the note says now, the server's copy of which may be
            // her own write.)
            let unsure = s.mailbox.notes.changes.get(at).map(|change| unanswered(change, &old)).unwrap_or_default();
            let mine = with(changed(&base, &old), unsure);
            // Words changed here that the server changed meanwhile too were written over other words: the change keeps
            // the version it was made from, so that the server refuses it as stale rather than take it over the
            // others'. Otherwise what it is told against is the server's note now, version and all.
            let moved = (mine.title.is_some() && fresh.title != base.title) || (mine.text.is_some() && fresh.text != base.text);
            let mut new_base = fresh.clone();
            put(&mut new_base, &as_was(&base, &mine));
            if moved {
                new_base.version = older(&base.version);
            }
            put(&mut fresh, &mine);
            // (The note's version is the one of the words it is changed from.)
            fresh.version.clone_from(&new_base.version);
            if let Some(change) = s.mailbox.notes.changes.get_mut(at) {
                change.base = Some(Box::new(new_base));
            }
        }
        if fresh == old {
            merged.unchanged += 1;
        } else if let Some(note) = s.mailbox.notes.note_mut(id) {
            *note = fresh;
            merged.updated += 1;
        }
    }
    // (No longer there: not in a whole read, or not in a list read whole, whether or not all its notes could be read.)
    let listed: Option<HashSet<String>> = download.listed.as_ref().map(|listed| listed.iter().cloned().collect());
    let there = |remote: &str| seen.contains(remote) || listed.as_ref().is_some_and(|listed| listed.contains(remote));
    let gone_there = |remote: &str| (download.complete && !seen.contains(remote)) || listed.as_ref().is_some_and(|listed| !listed.contains(remote));
    // A note whose deletion was undone after it went without an answer (or that a page held), still there: the server
    // never deleted it, and nothing is made anew. What was changed of it since still goes; a change with nothing to
    // tell goes.
    let kept_there: Vec<Id> = s
        .mailbox
        .notes
        .changes
        .iter()
        .filter(|change| change.account == account && change.undeleted && there(&change.remote) && !gone_there(&change.remote))
        .map(|change| change.note)
        .collect();
    for id in &kept_there {
        let note = s.mailbox.notes.note(*id).cloned();
        s.mailbox.notes.changes.retain_mut(|change| {
            if change.note != *id || !change.undeleted {
                return true;
            }
            change.undeleted = false;
            let Some(note) = note.as_ref() else { return true };
            change.base.as_deref().is_some_and(|base| !with(changed(base, note), unanswered(change, note)).is_empty())
        });
    }
    let notes = &mut s.mailbox.notes;
    // A whole read: what isn't there any more goes, unless a change of it waits; two notes with one name of the
    // server's leave one, the first (the one made here), unless a change of the other waits.
    let mut kept_names: HashSet<String> = HashSet::new();
    let mut versioned = false;
    let mut deleted_there: Vec<Id> = Vec::new();
    let ours: Vec<(Id, String)> = notes
        .items
        .values()
        .filter(|note| note.origin.account == account && !note.origin.id.is_empty())
        .map(|note| (note.id, note.origin.id.clone()))
        .collect();
    for (id, remote) in ours {
        if waiting.contains(&id) {
            // A change of words waiting for the version a read brings, of a note a whole read no longer finds (deleted
            // there, or no longer shared): no read brings one. It goes with the oldest version, never over other words;
            // the server says the note is gone, and her words are kept as a note of her own.
            if gone_there(&remote) {
                // (One whose deletion she undid, or that a page held, deleted there after all: made anew with her
                // words, below.)
                if notes.changes.iter().any(|change| change.account == account && change.note == id && change.undeleted) {
                    deleted_there.push(id);
                    kept_names.insert(remote);
                    continue;
                }
                let of = |change: &&mut NoteChange| change.account == account && change.note == id && change.kind == NoteChangeKind::Update;
                for change in notes.changes.iter_mut().filter(of) {
                    if let Some(base) = change.base.as_deref_mut().filter(|base| base.version.is_empty()) {
                        base.version = OLDEST_VERSION.to_string();
                        versioned = true;
                    }
                }
            }
            kept_names.insert(remote);
            continue;
        }
        let gone = download.complete && !seen.contains(&remote);
        let twin = !kept_names.insert(remote);
        if gone || twin {
            notes.items.remove(&id);
            merged.removed += 1;
        }
    }
    for id in &deleted_there {
        let title = called(s.mailbox.notes.note(*id));
        // (Held by a page, and changed there by her: her version, kept; a deletion she undid: made anew.)
        let held = s.mailbox.notes.changes.iter().any(|change| change.note == *id && change.resumed);
        let shared = s.mailbox.notes.note(*id).is_some_and(|note| note.shared);
        s.mailbox.notes.changes.retain(|change| change.note != *id);
        anew(&mut s.mailbox, *id, salt);
        let line = match (held, shared) {
            (false, _) => MADE_ANEW,
            (true, true) => KEPT_WHEN_UNSHARED,
            (true, false) => KEPT_WHEN_GONE,
        };
        merged.messages.push(fill(line, &[("title", &title)]));
    }
    let undeleted = !kept_there.is_empty() || !deleted_there.is_empty();
    if merged.added + merged.updated + merged.removed > 0 || versioned || held > 0 || undeleted {
        s.mailbox_changed();
    }
    Ok(merged)
}

// ---- what the server said ----

/// What taking in the answers to a [`NotesUpload`] came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotesApplied {
    /// Changes the server took (or that needed no more telling).
    pub done: usize,
    /// Changes it refused: put back, or kept here with why.
    pub refused: usize,
    /// Changes of words made from an older version: kept as notes of their own.
    pub stale: usize,
    /// Changes that couldn't be sent and wait to be tried again.
    pub failed: usize,
    /// What still waits, after this.
    pub waiting: usize,
    /// What the owner should hear, a line each.
    pub messages: Vec<String>,
}

impl NotesApplied {
    fn say(&mut self, line: String) {
        if self.messages.len() < MAX_MESSAGES && !line.is_empty() && !self.messages.contains(&line) {
            self.messages.push(line);
        }
    }

    fn json(&self) -> Value {
        json!({
            "done": self.done,
            "refused": self.refused,
            "stale": self.stale,
            "failed": self.failed,
            "waiting": self.waiting,
            "messages": self.messages,
        })
    }
}

/// A note's title for a line about it: quoted, short, and never empty.
fn called(note: Option<&Note>) -> String {
    let title: String = note.map(Note::heading).unwrap_or_default().chars().filter(|c| !c.is_control()).take(60).collect();
    // (A note with no words is named by none, in any language.)
    let title = if title.trim().is_empty() { "\u{2026}" } else { title.trim() };
    format!("\"{title}\"")
}

/// Take in what a server said to a [`NotesUpload`] ([`NotesUploaded`]): see the module.
pub fn apply_note_outcomes(s: &mut Session, uploaded: &NotesUploaded) -> Result<NotesApplied, CmdError> {
    let owner = owner(&s.mailbox, uploaded.account, &uploaded.email)?;
    same_server(owner, &uploaded.session)?;
    let account = owner.id;
    let (now, salt) = (s.now(), s.rev());
    let before = s.mailbox.notes.clone();
    let mut taking = Taking { mailbox: &mut s.mailbox, account, now, salt, applied: NotesApplied::default(), told: false };
    for outcome in uploaded.outcomes.iter().take(MAX_OUTCOMES) {
        let mut outcome = outcome.clone();
        outcome.tidy();
        match outcome.op.kind {
            NoteChangeKind::Create => taking.created(&outcome),
            NoteChangeKind::Update => taking.updated(&outcome),
            NoteChangeKind::Delete => taking.deleted(&outcome),
        }
    }
    let Taking { mut applied, told, .. } = taking;
    applied.waiting = s.mailbox.notes.changes.iter().filter(|change| change.account == account).count();
    if told {
        s.forget_history();
    }
    if told || s.mailbox.notes != before {
        s.mailbox_changed();
    }
    Ok(applied)
}

struct Taking<'a> {
    mailbox: &'a mut Mailbox,
    account: Id,
    now: mailcraft_time::Timestamp,
    salt: u64,
    applied: NotesApplied,
    told: bool,
}

impl Taking<'_> {
    fn position(&self, wanted: impl Fn(&NoteChange) -> bool) -> Option<usize> {
        let account = self.account;
        self.mailbox.notes.changes.iter().position(|change| change.account == account && wanted(change))
    }

    /// The change at `at` couldn't be sent now: it waits, longer after each failure.
    fn failed(&mut self, at: Option<usize>, outcome: &NoteOutcome) {
        let Some(change) = at.and_then(|at| self.mailbox.notes.changes.get_mut(at)) else { return };
        change.failures = change.failures.saturating_add(1);
        change.retry_at = self.now.saturating_add(wait_after(change.failures, outcome.retry_after));
        change.error.clone_from(&outcome.error);
        self.applied.failed += 1;
    }

    /// A new note of this server's, here: the person's words of `from`, under a title of their own ([`copy_title`]), to
    /// be made there; `None` when there is no room for one (the notes or the changes are as many as are kept).
    fn keep_as_new(&mut self, from: &Note) -> Option<(Id, String)> {
        if self.mailbox.notes.changes.len() >= MAX_NOTE_CHANGES || self.mailbox.notes.items.len() >= MAX_NOTES {
            return None;
        }
        let (most, most_text) = self
            .mailbox
            .notes
            .server(self.account)
            .map_or((mailcraft_model::note::MAX_NOTE_TITLE, mailcraft_model::note::MAX_NOTE_TEXT), |server| (server.max_title, server.max_text));
        let (title, cut) = copy_title(self.mailbox, from.heading(), most);
        // A title that had to be cut goes whole as the first line of the text (one from the text is in it already).
        let text = if cut && !from.title.trim().is_empty() { format!("{}\n{}", from.title.trim(), from.text) } else { from.text.clone() };
        if text.chars().count() > most_text {
            return None;
        }
        let id = self.mailbox.alloc_id();
        let note = Note {
            id,
            origin: Origin { account: self.account, ..Default::default() },
            title: title.clone(),
            text,
            pinned: from.pinned,
            archived: from.archived,
            created: self.now,
            updated: self.now,
            ..Default::default()
        };
        self.mailbox.notes.items.insert(id, Arc::new(note));
        note_created(self.mailbox, id, self.salt);
        Some((id, title))
    }

    /// What the server's answer to a change says of the note, taken by the note here: its version, whether it is rich,
    /// and the person's rights (the answer says no more: the note's link, owner and category stay as they were read).
    fn take_changed(note: &mut Note, said: &RemoteNote) {
        let fresh = said.note(note.origin.account, note.id);
        (note.version, note.rich) = (fresh.version, fresh.rich);
        note.rights = NoteRights { write_text: fresh.rights.write_text && !note.cut, ..fresh.rights };
        if fresh.updated != 0 {
            note.updated = fresh.updated;
        }
        note.error.clear();
    }

    /// What the server says of a note it made, taken by the note here (not its words).
    fn take_server_side(note: &mut Note, said: &RemoteNote) {
        let fresh = said.note(note.origin.account, note.id);
        (note.version, note.rights, note.rich, note.web_url) = (fresh.version, fresh.rights, fresh.rich, fresh.web_url);
        (note.shared, note.owner_name, note.category) = (fresh.shared, fresh.owner_name, fresh.category);
        if fresh.created != 0 {
            note.created = fresh.created;
        }
        if fresh.updated != 0 {
            note.updated = fresh.updated;
        }
        note.error.clear();
    }

    fn created(&mut self, outcome: &NoteOutcome) {
        let op = &outcome.op;
        let at = self.position(|change| change.kind == NoteChangeKind::Create && !op.token.is_empty() && change.token == op.token);
        let note = at.and_then(|at| self.mailbox.notes.changes.get(at)).map(|change| change.note).filter(|id| self.mailbox.notes.note(*id).is_some());
        match outcome.result {
            NoteResult::Done => {
                self.told = true;
                self.applied.done += 1;
                let Some(said) = outcome.note.as_ref().filter(|said| !said.id.is_empty()) else {
                    // Made, under a name that wasn't said: the next read brings it.
                    if let Some(at) = at {
                        self.mailbox.notes.changes.remove(at);
                    }
                    return;
                };
                let (Some(at), Some(id)) = (at, note) else {
                    // Undone, or deleted again, while it was on its way: the server is to delete it. Not while a note
                    // here still carries the mark it was made with (one a page holds, say): that is no undo, and what
                    // was made is hers.
                    let marked = !op.token.is_empty()
                        && self
                            .mailbox
                            .notes
                            .items
                            .values()
                            .any(|note| note.token == op.token || note.held.as_deref().is_some_and(|hold| hold.token == op.token));
                    if !marked
                        && self.mailbox.notes.by_remote(self.account, &said.id).is_none()
                        && self.mailbox.notes.changes.len() < MAX_NOTE_CHANGES
                    {
                        let base = Box::new(said.note(self.account, 0));
                        self.mailbox.notes.changes.push(NoteChange {
                            account: self.account,
                            kind: NoteChangeKind::Delete,
                            remote: said.id.clone(),
                            base: Some(base),
                            ..Default::default()
                        });
                    }
                    return;
                };
                self.mailbox.notes.changes.remove(at);
                let Some(note) = self.mailbox.notes.note_mut(id) else { return };
                note.origin.id.clone_from(&said.id);
                // An answer that names the note and says nothing more of it (no version): its words stay as they are
                // here, and the next read brings the rest.
                if said.version.is_empty() {
                    note.error.clear();
                    return;
                }
                let here = note.clone();
                Self::take_server_side(note, said);
                // Nothing changed here since it went: the words are the server's (a title it gave from the first line).
                let mut sent = here.clone();
                put(&mut sent, &op.fields);
                if changed(&sent, &here).is_empty() {
                    (note.title, note.text) = (said.title.clone(), said.text.clone());
                } else {
                    // What was changed here since still goes; of the words, what wasn't is the server's (a title it gave
                    // from the first line stays, never wiped by the one sent without one).
                    if here.title == sent.title {
                        note.title.clone_from(&said.title);
                    }
                    if here.text == sent.text {
                        note.text.clone_from(&said.text);
                    }
                    let mut base = said.note(self.account, id);
                    base.token.clear();
                    self.mailbox.notes.changes.push(NoteChange {
                        account: self.account,
                        kind: NoteChangeKind::Update,
                        note: id,
                        remote: said.id.clone(),
                        base: Some(Box::new(base)),
                        ..Default::default()
                    });
                }
                // A copy of it a read brought before the answer goes, when nothing of it waits.
                let changes = &self.mailbox.notes.changes;
                let twins: Vec<Id> = self
                    .mailbox
                    .notes
                    .items
                    .values()
                    .filter(|other| other.id != id && other.origin.account == self.account && other.origin.id == said.id)
                    .filter(|other| !changes.iter().any(|change| change.note == other.id))
                    .map(|other| other.id)
                    .collect();
                for twin in twins {
                    self.mailbox.notes.items.remove(&twin);
                }
            }
            NoteResult::Failed => self.failed(at, outcome),
            NoteResult::Gone | NoteResult::Refused | NoteResult::Stale => {
                let Some(at) = at else { return };
                self.mailbox.notes.changes.remove(at);
                self.applied.refused += 1;
                let title = called(note.and_then(|id| self.mailbox.notes.note(id)));
                if let Some(note) = note.and_then(|id| self.mailbox.notes.note_mut(id)) {
                    note.error.clone_from(&outcome.error);
                }
                self.applied.say(fill(NOT_SAVED, &[("title", &title), ("said", &outcome.error)]));
            }
        }
    }

    fn updated(&mut self, outcome: &NoteOutcome) {
        let op = &outcome.op;
        if op.remote.is_empty() {
            return;
        }
        let at = self.position(|change| change.kind == NoteChangeKind::Update && change.remote == op.remote && change.note == op.note);
        match outcome.result {
            NoteResult::Done => {
                self.told = true;
                self.applied.done += 1;
                let Some(at) = at else {
                    // Undone while it was on its way: what the mailbox says goes after it.
                    let Some(here) = self.mailbox.notes.note(op.note).filter(|note| note.origin.id == op.remote).cloned() else { return };
                    let mut sent = here.clone();
                    put(&mut sent, &op.fields);
                    if let Some(said) = outcome.note.as_ref().filter(|said| !said.version.is_empty()) {
                        // Words the server has that the note here doesn't, of a part the change didn't name: another
                        // device's. The opposite goes with the version of the words here, to go stale there rather
                        // than write over them.
                        let other = (op.fields.title.is_none() && said.title != here.title) || (op.fields.text.is_none() && said.text != here.text);
                        sent.version = if other { older(&here.version) } else { said.version.clone() };
                    }
                    if !changed(&sent, &here).is_empty() && self.mailbox.notes.changes.len() < MAX_NOTE_CHANGES {
                        self.mailbox.notes.changes.push(NoteChange {
                            account: self.account,
                            kind: NoteChangeKind::Update,
                            note: here.id,
                            remote: op.remote.clone(),
                            base: Some(Box::new(sent)),
                            ..Default::default()
                        });
                    }
                    return;
                };
                let Some(here) = self.mailbox.notes.note(op.note).cloned() else {
                    self.mailbox.notes.changes.remove(at);
                    return;
                };
                let Some(change) = self.mailbox.notes.changes.get_mut(at) else { return };
                (change.failures, change.retry_at) = (0, 0);
                change.error.clear();
                // (The server has the note: its deletion, undone, never happened there.)
                change.undeleted = false;
                let Some(base) = change.base.as_deref_mut() else {
                    self.mailbox.notes.changes.remove(at);
                    return;
                };
                // What the server was told: its note as it was read, with what this change made to it.
                let read_at = base.version.clone();
                put(base, &op.fields);
                let (sent_title, sent_text) = (base.title.clone(), base.text.clone());
                // The answer says the note's version with its words: the server's note now. The words are taken with
                // their version, never a version without its words.
                let said = outcome.note.as_ref().filter(|said| !said.version.is_empty());
                if let Some(said) = said {
                    Self::take_changed(base, said);
                    (base.title, base.text) = (said.title.clone(), said.text.clone());
                    // Words typed here since this went, of a part the server changed meanwhile too (another device): they
                    // were typed over other words, and go with the version they were read at, to go stale there.
                    let over = (here.title != sent_title && said.title != sent_title) || (here.text != sent_text && said.text != sent_text);
                    if over {
                        base.version = older(&read_at);
                    }
                } else {
                    // An answer that doesn't say the note's words leaves no version known for them: the notes are read
                    // before more words go ([`notes_need_read`]).
                    base.version.clear();
                }
                change.sent.clear();
                let base = base.clone();
                if let (Some(said), Some(note)) = (said, self.mailbox.notes.note_mut(op.note)) {
                    // Of the note here, what wasn't changed since this went is the server's now; what was stays.
                    if note.title == sent_title {
                        note.title.clone_from(&said.title);
                    }
                    if note.text == sent_text {
                        note.text.clone_from(&said.text);
                    }
                    Self::take_changed(note, said);
                    note.version.clone_from(&base.version);
                } else if let Some(note) = self.mailbox.notes.note_mut(op.note) {
                    note.version.clear();
                }
                // Nothing more to tell once the note here is the server's.
                let settled = self.mailbox.notes.note(op.note).is_none_or(|now| changed(&base, now).is_empty());
                if settled {
                    self.mailbox.notes.changes.remove(at);
                }
            }
            NoteResult::Failed => self.failed(at, outcome),
            NoteResult::Stale | NoteResult::Gone | NoteResult::Refused => {
                let Some(at) = at else { return };
                let change = self.mailbox.notes.changes.remove(at);
                let Some(here) = self.mailbox.notes.note(op.note).cloned() else { return };
                let title = called(Some(&here));
                // The person's words that the server didn't take, as they are here now.
                let words = change.base.as_deref().is_some_and(|base| changed(base, &here).has_words());
                match outcome.result {
                    NoteResult::Gone if words || change.undeleted => {
                        // Deleted there, or no longer shared with her: the note here, with her words, is one of her own,
                        // to be made there anew (and so is one whose deletion she undid).
                        self.applied.done += 1;
                        anew(self.mailbox, op.note, self.salt);
                        let line = if !words && !change.resumed {
                            MADE_ANEW
                        } else if here.shared {
                            KEPT_WHEN_UNSHARED
                        } else {
                            KEPT_WHEN_GONE
                        };
                        self.applied.say(fill(line, &[("title", &title)]));
                    }
                    NoteResult::Gone => {
                        // Deleted there, and nothing of hers to keep: it goes here too.
                        self.applied.done += 1;
                        self.mailbox.notes.items.remove(&op.note);
                    }
                    result => {
                        if result == NoteResult::Stale {
                            self.applied.stale += 1;
                        } else {
                            self.applied.refused += 1;
                        }
                        // Nothing typed is lost: her words are a note of their own, and the note is put back as the
                        // server had it, with the version of those words, until the next read brings what it has now.
                        let copy = if words { self.keep_as_new(&here) } else { None };
                        match (words, copy) {
                            (true, None) => {
                                // No room for one: the change waits, and the note keeps her words.
                                // (What went before this, unanswered, may still be there.)
                                let mut change = change;
                                change.sent = op.earlier.iter().filter(|earlier| **earlier != op.fields).cloned().collect();
                                change.failures = change.failures.saturating_add(1);
                                change.retry_at = self.now.saturating_add(wait_after(change.failures, None));
                                change.error.clone_from(&outcome.error);
                                self.mailbox.notes.changes.push(change);
                                self.applied.say(fill(NO_ROOM_FOR_COPY, &[("title", &title), ("said", &outcome.error)]));
                                return;
                            }
                            (_, Some((_, copy))) => {
                                let copy = format!("\"{copy}\"");
                                let said = outcome.error.trim().trim_end_matches('.');
                                let line = if result == NoteResult::Stale {
                                    fill(KEPT_AS_COPY, &[("title", &title), ("copy", &copy)])
                                } else {
                                    fill(KEPT_REFUSED, &[("title", &title), ("said", said), ("copy", &copy)])
                                };
                                self.applied.say(line);
                            }
                            (false, _) if result == NoteResult::Refused => {
                                self.applied.say(fill(CHANGE_PUT_BACK, &[("title", &title), ("said", &outcome.error)]));
                            }
                            (false, _) => {}
                        }
                        if let (Some(base), Some(note)) = (change.base.as_deref(), self.mailbox.notes.note_mut(op.note)) {
                            let (id, origin) = (note.id, note.origin.clone());
                            *note = Note { id, origin, version: older(&base.version), ..base.clone() };
                        }
                    }
                }
            }
        }
    }

    fn deleted(&mut self, outcome: &NoteOutcome) {
        let op = &outcome.op;
        if op.remote.is_empty() {
            return;
        }
        let at = self.position(|change| change.kind == NoteChangeKind::Delete && change.remote == op.remote);
        match outcome.result {
            NoteResult::Done | NoteResult::Gone => {
                self.told |= outcome.result == NoteResult::Done;
                self.applied.done += 1;
                if let Some(at) = at {
                    self.mailbox.notes.changes.remove(at);
                    return;
                }
                // Undone while it was on its way: the note is here again and the server no longer has it. Nothing
                // typed is lost: it is made there anew.
                let Some(id) = self.mailbox.notes.by_remote(self.account, &op.remote).map(|note| note.id) else { return };
                let title = called(self.mailbox.notes.note(id));
                self.mailbox.notes.changes.retain(|change| change.note != id);
                if let Some(note) = self.mailbox.notes.note_mut(id) {
                    note.origin.id.clear();
                    note.version.clear();
                }
                note_created(self.mailbox, id, self.salt);
                self.applied.say(fill(MADE_ANEW, &[("title", &title)]));
            }
            NoteResult::Failed => self.failed(at, outcome),
            NoteResult::Refused | NoteResult::Stale => {
                let Some(at) = at else { return };
                let change = self.mailbox.notes.changes.remove(at);
                self.applied.refused += 1;
                let title = called(change.base.as_deref());
                // Back in the notes, under the id it had when that is still free: the server still has it.
                if let Some(base) = change.base.as_deref() {
                    let id = if self.mailbox.notes.note(change.note).is_none() && change.note != 0 { change.note } else { self.mailbox.alloc_id() };
                    let mut back = base.clone();
                    back.id = id;
                    if self.mailbox.notes.by_remote(self.account, &op.remote).is_none() {
                        self.mailbox.notes.items.insert(id, Arc::new(back));
                    }
                }
                self.applied.say(fill(NOT_DELETED, &[("title", &title), ("said", &outcome.error)]));
            }
        }
    }
}

// ---- the commands ----

fn shown(e: &serde_json::Error) -> String {
    e.to_string().chars().filter(|c| !c.is_control()).take(MAX_SHOWN).collect()
}

fn apply_notes_command(s: &mut Session, v: &Value) -> CmdResult {
    let download = v.get("download").cloned().ok_or_else(|| bad("`download` is required"))?;
    let download: NotesDownload = serde_json::from_value(download).map_err(|e| bad(format!("`download`: {}", shown(&e))))?;
    Ok(merge_notes(s, &download)?.json())
}

fn note_changes_command(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::req_id(v, "account")?;
    settle_note_changes(s);
    let upload = note_upload(s, account)?;
    let mut said = serde_json::to_value(upload).map_err(|e| fail(e.to_string()))?;
    // (Whether the notes are to be read whole first: a change of words waits for the version a read brings.)
    if let Some(said) = said.as_object_mut() {
        said.insert("needRead".into(), json!(notes_need_read(s, account)));
    }
    Ok(said)
}

fn apply_note_changes_command(s: &mut Session, v: &Value) -> CmdResult {
    let result = v.get("result").cloned().ok_or_else(|| bad("`result` is required"))?;
    let uploaded: NotesUploaded = serde_json::from_value(result).map_err(|e| bad(format!("`result`: {}", shown(&e))))?;
    Ok(apply_note_outcomes(s, &uploaded)?.json())
}

#[cfg(test)]
mod tests;
