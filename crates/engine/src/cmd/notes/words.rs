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
/// A text longer than a note holds, or a title: `{max}` is what the note's server takes (or this computer keeps).
pub const NOTE_TOO_LONG: &str = "A note holds {max} characters at most";
pub const NOTE_TITLE_TOO_LONG: &str = "A note's title holds {max} characters at most";
/// A note with no web address to open.
pub const NOTE_NO_LINK: &str = "This note has no web address";
/// More changes waiting than are ever kept.
pub const TOO_MANY_NOTE_CHANGES: &str = "Too many note changes are waiting to be sent. Let them get through first.";

/// What became of a change a server was told, said to the person: `{title}` is the note's, quoted; `{said}` the
/// server's words, as it said them.
pub const KEPT_AS_COPY: &str = "{title} was changed elsewhere meanwhile: your version is kept as a note of its own, {copy}.";
pub const KEPT_WHEN_GONE: &str = "{title} was deleted elsewhere meanwhile: your version is kept as a note of its own.";
pub const NOT_SAVED: &str = "{title} couldn't be saved on the server: {said}";
pub const CHANGE_PUT_BACK: &str = "The change to {title} was put back as the server has it: {said}";
/// A change of words the server refused (a note shared for reading since, say): the person's words are a note of their
/// own.
pub const KEPT_REFUSED: &str = "The server wouldn't take the change to {title}: {said}. Your version is kept as a note of its own, {copy}.";
/// Words typed into a note that can no longer be changed so (read only now, or its text changed where it was written):
/// `{why}` is the refusal; they are a note of their own.
pub const KEPT_NOT_WRITABLE: &str = "{title} can no longer be changed here ({why}): your version is kept as a note of its own, {copy}.";
/// A change of words that went stale, or was refused, when no note of its own can be made for the person's words (the
/// notes are as many as are kept): it waits, and the note keeps them.
pub const NO_ROOM_FOR_COPY: &str =
    "{title} couldn't be saved on the server ({said}), and there is no room for a note of its own: your version stays here, unsent.";
pub const NOT_DELETED: &str = "The server wouldn't delete {title}, which is back in the notes: {said}";
pub const MADE_ANEW: &str = "{title} was already deleted on the server when its deletion was undone here: it is made there anew.";
/// A server that no longer keeps notes: what waited to go to it is kept on this computer, or (a session that keeps
/// nothing of its own) on this page, unsent, until it is left (it asks before).
pub const NO_LONGER_KEPT: &str = "The server no longer keeps notes here: the notes that waited to go are kept on this computer.";
pub const NO_LONGER_KEPT_PAGE: &str =
    "The server no longer keeps notes here: the notes that waited to go stay on this page, unsent, until it is left.";
/// Words typed into a note shared with the person that is no longer shared with her: they are a note of her own.
pub const KEPT_WHEN_UNSHARED: &str = "{title} is no longer shared with you: your version is kept as a note of its own.";

/// `n` with its thousands grouped, as English writes it (`100,000`); the window writes it in its own language's way.
pub fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `template` with each `{name}` filled with its value, in one pass.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, from) = rest.split_at(open);
        out.push_str(before);
        match from.get(1..).and_then(|inner| inner.split_once('}')) {
            Some((name, after)) if values.iter().any(|(key, _)| *key == name) => {
                out.push_str(values.iter().find(|(key, _)| *key == name).map(|(_, value)| *value).unwrap_or_default());
                rest = after;
            }
            _ => {
                out.push('{');
                rest = from.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}
