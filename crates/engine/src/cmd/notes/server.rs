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
//!
//! Like mail that arrives, taking a server's answer or its notes in is not an undo step; once a server was told
//! anything, the undo history is forgotten.

use std::collections::HashSet;
use std::sync::Arc;

use mailcraft_model::note::{
    MAX_NOTE_CHANGES, MAX_NOTE_OPS, MAX_NOTES, Note, NoteChange, NoteChangeKind, NoteFields, NoteOp, NoteOutcome, NoteResult, NotesDownload,
    NotesUpload, NotesUploaded, RemoteNote,
};
use mailcraft_model::{Id, Mailbox, Origin};
use serde_json::{Value, json};

use super::super::remote::{EventsWaiting, new_token, owner, wait_after};
use super::super::{bad, fail};
use super::words::{
    CHANGE_PUT_BACK, KEPT_AS_COPY, KEPT_WHEN_GONE, MADE_ANEW, NO_LONGER_KEPT, NO_LONGER_KEPT_LOST, NOT_DELETED, NOT_SAVED, TOO_MANY_NOTE_CHANGES,
    fill,
};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

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
            .params(r#"{"account": number} — the changes of that account's notes its server is to be told now"#),
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
            if back {
                mailbox.notes.changes.remove(at);
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

/// How much of an account's notes waits to go to its server: due now, all of it, and when the next held back is due.
pub fn notes_waiting(s: &Session, account: Id) -> EventsWaiting {
    let mut waiting = EventsWaiting::default();
    let now = s.now();
    for change in s.mailbox.notes.changes.iter().filter(|change| change.account == account && fits(&s.mailbox, change)) {
        waiting.total += 1;
        if change.retry_at <= now {
            waiting.due += 1;
        } else {
            waiting.next = Some(waiting.next.map_or(change.retry_at, |next| next.min(change.retry_at)));
        }
    }
    waiting
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
                let fields = changed(base, note);
                if fields.is_empty() {
                    continue;
                }
                let version = if fields.has_words() { base.version.clone() } else { String::new() };
                NoteOp { kind: NoteChangeKind::Update, note: note.id, remote: change.remote.clone(), fields, version, ..Default::default() }
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

/// Take in what a server has of an account's notes ([`NotesDownload`]): see the module. Nothing is changed when this
/// returns an error.
pub fn merge_notes(s: &mut Session, download: &NotesDownload) -> Result<NotesMerged, CmdError> {
    let account = owner(&s.mailbox, download.account, &download.email)?.id;
    let mut download = download.clone();
    download.tidy();
    let mut merged = NotesMerged::default();
    let keeps_local = s.keeps_local;
    let notes = &mut s.mailbox.notes;
    if !download.offered {
        // The server no longer keeps notes (or never did): its notes go, and what waited to be made there is kept on
        // this computer where that is kept.
        let had = notes.servers.iter().any(|server| server.account == account) || notes.items.values().any(|note| note.origin.account == account);
        if !had {
            return Ok(merged);
        }
        let waiting: HashSet<Id> = notes
            .changes
            .iter()
            .filter(|change| change.account == account && change.kind == NoteChangeKind::Create)
            .map(|change| change.note)
            .collect();
        notes.servers.retain(|server| server.account != account);
        notes.changes.retain(|change| change.account != account);
        let ours: Vec<Id> = notes.items.values().filter(|note| note.origin.account == account).map(|note| note.id).collect();
        for id in ours {
            if keeps_local && waiting.contains(&id) {
                if let Some(note) = notes.note_mut(id) {
                    let mut own = Note {
                        id,
                        title: note.title.clone(),
                        text: note.text.clone(),
                        pinned: note.pinned,
                        archived: note.archived,
                        ..Default::default()
                    };
                    (own.created, own.updated) = (note.created, note.updated);
                    *note = own;
                }
            } else {
                notes.items.remove(&id);
                merged.removed += 1;
            }
        }
        if !waiting.is_empty() {
            merged.messages.push(if keeps_local { NO_LONGER_KEPT } else { NO_LONGER_KEPT_LOST }.to_string());
        }
        s.mailbox_changed();
        return Ok(merged);
    }
    let mut server = download.server.clone();
    server.account = account;
    match notes.servers.iter_mut().find(|kept| kept.account == account) {
        Some(kept) => *kept = server,
        None => notes.servers.push(server),
    }
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
        if deleted.contains(&remote.id) {
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
            let mine = changed(&base, &old);
            let mut new_base = fresh.clone();
            put(&mut new_base, &as_was(&base, &mine));
            new_base.version.clone_from(&base.version);
            put(&mut fresh, &mine);
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
    let notes = &mut s.mailbox.notes;
    // A whole read: what isn't there any more goes, unless a change of it waits; two notes with one name of the
    // server's leave one, the first (the one made here), unless a change of the other waits.
    let mut kept_names: HashSet<String> = HashSet::new();
    let ours: Vec<(Id, String)> = notes
        .items
        .values()
        .filter(|note| note.origin.account == account && !note.origin.id.is_empty())
        .map(|note| (note.id, note.origin.id.clone()))
        .collect();
    for (id, remote) in ours {
        if waiting.contains(&id) {
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
    if merged.added + merged.updated + merged.removed > 0 {
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
    if title.trim().is_empty() { "A note".into() } else { format!("\"{}\"", title.trim()) }
}

/// Take in what a server said to a [`NotesUpload`] ([`NotesUploaded`]): see the module.
pub fn apply_note_outcomes(s: &mut Session, uploaded: &NotesUploaded) -> Result<NotesApplied, CmdError> {
    let account = owner(&s.mailbox, uploaded.account, &uploaded.email)?.id;
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

    /// A new note of this server's, here: the person's words, under `title`, to be made there.
    fn keep_as_new(&mut self, from: &Note, title: String) -> Option<Id> {
        if self.mailbox.notes.changes.len() >= MAX_NOTE_CHANGES || self.mailbox.notes.items.len() >= MAX_NOTES {
            return None;
        }
        let id = self.mailbox.alloc_id();
        let note = Note {
            id,
            origin: Origin { account: self.account, ..Default::default() },
            title,
            text: from.text.clone(),
            pinned: from.pinned,
            archived: from.archived,
            created: self.now,
            updated: self.now,
            ..Default::default()
        };
        self.mailbox.notes.items.insert(id, Arc::new(note));
        note_created(self.mailbox, id, self.salt);
        Some(id)
    }

    /// What the server says of a note it made or changed, taken by the note here (not its words).
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
                    // Undone, or deleted again, while it was on its way: the server is to delete it.
                    if self.mailbox.notes.by_remote(self.account, &said.id).is_none() && self.mailbox.notes.changes.len() < MAX_NOTE_CHANGES {
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
                let here = note.clone();
                Self::take_server_side(note, said);
                // Nothing changed here since it went: the words are the server's (a title it gave from the first line).
                let mut sent = here.clone();
                put(&mut sent, &op.fields);
                if changed(&sent, &here).is_empty() {
                    (note.title, note.text) = (said.title.clone(), said.text.clone());
                } else {
                    // What was changed here since still goes.
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
                    if let Some(said) = &outcome.note {
                        sent.version.clone_from(&said.version);
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
                let here = self.mailbox.notes.note(op.note).cloned();
                let mut settled = false;
                if let Some(change) = self.mailbox.notes.changes.get_mut(at) {
                    if let Some(base) = change.base.as_deref_mut() {
                        put(base, &op.fields);
                        if let Some(said) = &outcome.note {
                            Self::take_server_side(base, said);
                        }
                    }
                    (change.failures, change.retry_at) = (0, 0);
                    change.error.clear();
                    settled = match (change.base.as_deref(), here.as_ref()) {
                        (Some(base), Some(here)) => changed(base, here).is_empty(),
                        _ => true,
                    };
                }
                if settled {
                    self.mailbox.notes.changes.remove(at);
                }
                if let (Some(said), Some(note)) = (&outcome.note, self.mailbox.notes.note_mut(op.note)) {
                    Self::take_server_side(note, said);
                }
            }
            NoteResult::Failed => self.failed(at, outcome),
            NoteResult::Stale | NoteResult::Gone => {
                let Some(at) = at else { return };
                let change = self.mailbox.notes.changes.remove(at);
                let Some(here) = self.mailbox.notes.note(op.note).cloned() else { return };
                let stale = outcome.result == NoteResult::Stale;
                let title = called(Some(&here));
                if op.fields.has_words() {
                    // Nothing typed is lost: the person's version is a note of its own.
                    let copy_title = if stale { format!("{} (2)", here.heading()) } else { here.heading().to_string() };
                    let copy_title: String = copy_title.chars().take(mailcraft_model::note::MAX_NOTE_TITLE).collect();
                    self.keep_as_new(&here, copy_title.clone());
                    let line = if stale {
                        fill(KEPT_AS_COPY, &[("title", &title), ("copy", &format!("\"{copy_title}\""))])
                    } else {
                        fill(KEPT_WHEN_GONE, &[("title", &title)])
                    };
                    self.applied.say(line);
                }
                if stale {
                    self.applied.stale += 1;
                    // The note itself is as the server had it, until the next read brings what it has now.
                    if let (Some(base), Some(note)) = (change.base.as_deref(), self.mailbox.notes.note_mut(op.note)) {
                        let (id, origin) = (note.id, note.origin.clone());
                        *note = Note { id, origin, version: String::new(), ..base.clone() };
                    }
                } else {
                    self.applied.done += 1;
                    self.mailbox.notes.items.remove(&op.note);
                }
            }
            NoteResult::Refused => {
                let Some(at) = at else { return };
                let change = self.mailbox.notes.changes.remove(at);
                self.applied.refused += 1;
                let title = called(self.mailbox.notes.note(op.note));
                if let (Some(base), Some(note)) = (change.base.as_deref(), self.mailbox.notes.note_mut(op.note)) {
                    let (id, origin) = (note.id, note.origin.clone());
                    *note = Note { id, origin, ..base.clone() };
                }
                self.applied.say(fill(CHANGE_PUT_BACK, &[("title", &title), ("said", &outcome.error)]));
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
    serde_json::to_value(upload).map_err(|e| fail(e.to_string()))
}

fn apply_note_changes_command(s: &mut Session, v: &Value) -> CmdResult {
    let result = v.get("result").cloned().ok_or_else(|| bad("`result` is required"))?;
    let uploaded: NotesUploaded = serde_json::from_value(result).map_err(|e| bad(format!("`result`: {}", shown(&e))))?;
    Ok(apply_note_outcomes(s, &uploaded)?.json())
}

#[cfg(test)]
mod tests;
