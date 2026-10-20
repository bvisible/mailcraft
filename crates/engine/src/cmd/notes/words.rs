// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What the notes' commands say to a person: why one is refused, and what became of a change a server was told. They
//! are English here, as the engine writes everything; the window reads them back in its language (`run_words`, whose
//! census holds this file to its rows).

/// Notes are offered nowhere here: a tab of a hosting page whose server keeps none, which keeps nothing of its own.
pub const NO_NOTES_HERE: &str = "Notes aren't kept here: this page's server offers none.";
/// A note the person may read and not change (one shared with them for reading).
pub const NOTE_READ_ONLY: &str = "Your rights do not let you change this note";
/// A note shared with the person, which only its owner deletes.
pub const NOTE_NOT_YOURS: &str = "Only its owner deletes a note: this one was shared with you";
/// A note whose text has more than words (formatting, a checklist, attachments), or that is kept cut here.
pub const NOTE_TEXT_ELSEWHERE: &str = "This note's text is changed where it was written: open it on the web";
/// A text longer than a note holds.
pub const NOTE_TOO_LONG: &str = "A note holds 100,000 characters at most";
/// A note with no web address to open.
pub const NOTE_NO_LINK: &str = "This note has no web address";
/// More changes waiting than are ever kept.
pub const TOO_MANY_NOTE_CHANGES: &str = "Too many note changes are waiting to be sent. Let them get through first.";
