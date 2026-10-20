// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What an agent may do with notes, as with People's contacts (D8): reading them is safe, and so is changing the
//! mailbox's own. A note of a server is written there for the person, where their phone and their desk read it: making
//! or changing one is the owner's to allow, as mail that goes out is (Send). Deleting a note is the owner's to allow
//! (Delete). What a host does with a server's notes (`sync.*`) is never an agent's.

use mailcraft_model::Mailbox;
use mailcraft_model::note::{NoteChangeKind, NoteFields};
use serde_json::Value;

use crate::agent::{Access, Class};
use crate::{Session, p};

/// A notes' command's class by id; `None` for a command that isn't one of the notes'.
pub fn access(id: &str) -> Option<Access> {
    Some(match id {
        "view.notes" | "notes.list" | "notes.get" | "notes.select" | "notes.link" => Access::Safe,
        // (A server's note asks: `classify`.)
        "notes.new" | "notes.edit" | "notes.pin" | "notes.archive" => Access::Safe,
        "notes.delete" => Access::Owner(Class::Delete),
        "sync.applyNotes" | "sync.noteChanges" | "sync.applyNoteChanges" => Access::Never,
        _ => return None,
    })
}

/// What only the params and the mailbox tell: a note made, changed, pinned or archived on a server is Send.
pub fn classify(s: &Session, id: &str, params: &Value) -> Option<Access> {
    let on_server = match id {
        "notes.new" => {
            let named = p::id(params, "account").filter(|account| *account != 0);
            named.map_or_else(|| !s.mailbox.notes.servers.is_empty(), |account| s.mailbox.notes.server(account).is_some())
        }
        "notes.edit" | "notes.pin" | "notes.archive" => {
            let note = p::id(params, "id").or(s.sel.note).and_then(|id| s.mailbox.notes.note(id));
            note.is_some_and(|note| note.origin.is_remote())
        }
        _ => return None,
    };
    on_server.then_some(Access::Owner(Class::Send))
}

/// The question to the owner about a notes' command an agent wants, when it is one.
pub fn describe(s: &Session, id: &str, params: &Value) -> Option<String> {
    let note = p::id(params, "id").or(s.sel.note).and_then(|id| s.mailbox.notes.note(id));
    let called = |title: &str| {
        let title: String = title.chars().filter(|c| !c.is_control()).take(80).collect();
        if title.trim().is_empty() { "a note".to_string() } else { format!("the note \u{201c}{}\u{201d}", title.trim()) }
    };
    let named = note.map(|note| called(note.heading())).unwrap_or_else(|| "a note".to_string());
    Some(match id {
        "notes.new" => format!("write {} to your notes on the server", called(p::str(params, "title").unwrap_or_default())),
        "notes.edit" => format!("change {named} in your notes on the server"),
        "notes.pin" => format!("pin or unpin {named} in your notes on the server"),
        "notes.archive" => format!("archive or unarchive {named} in your notes on the server"),
        "notes.delete" => format!("delete {named}"),
        _ => return None,
    })
}

/// What a command changed of the notes that its grant didn't cover: a server's note written (Send), or one deleted
/// there (Delete). A change that rides on one already waiting counts as one: what goes is the note as it is when it
/// goes.
pub fn overstep(before: &Mailbox, after: &Mailbox, allow: &[Class]) -> Option<Class> {
    let (was, now) = (&before.notes, &after.notes);
    let noted = |kind: NoteChangeKind| now.changes.iter().any(|change| change.kind == kind && !was.changes.contains(change));
    let words = |mailbox: &Mailbox, id| {
        mailbox.notes.note(id).map(|note| NoteFields {
            title: Some(note.title.clone()),
            text: Some(note.text.clone()),
            pinned: Some(note.pinned),
            archived: Some(note.archived),
        })
    };
    let rides = now
        .changes
        .iter()
        .filter(|change| change.kind != NoteChangeKind::Delete)
        .any(|change| words(before, change.note) != words(after, change.note));
    if !allow.contains(&Class::Send) && (noted(NoteChangeKind::Create) || noted(NoteChangeKind::Update) || rides) {
        return Some(Class::Send);
    }
    if !allow.contains(&Class::Delete) && noted(NoteChangeKind::Delete) {
        return Some(Class::Delete);
    }
    None
}
