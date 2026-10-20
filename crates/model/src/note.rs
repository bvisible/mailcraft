// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Notes: a title and a text, pinned or archived, kept in the mailbox ([`Notes`], in [`crate::Mailbox::notes`]).
//!
//! A note is the mailbox's own (its origin's account is 0), or a server's: an account whose server keeps notes
//! ([`NoteServer`]) has them read into the mailbox ([`NotesDownload`]), and what is made, changed and deleted of them
//! here is noted for that server ([`NoteChange`]), as an event or a contact of a server is: the change is made here at
//! once, a host tells the server ([`NotesUpload`]) and brings back what it made of each ([`NotesUploaded`]), and the
//! note of the change stays until the server took it or refused it.
//!
//! A server's note may be one shared with the person (another owns it), one whose text has more than words
//! (formatting, a checklist, attachments: its text is read here and changed where it was written, [`Note::rich`]), or
//! one longer than is kept here ([`Note::cut`]): what may be done to each is said by its rights ([`NoteRights`]).
//!
//! **Nothing typed is lost.** A change of a server's note's words carries the version it was made from
//! ([`Note::version`]): a server that has a newer one refuses it, and the person's words are kept as a note of their
//! own.
//!
//! Plain data, and lenient: a mailbox written before there were notes reads as one that has none, and what a server or
//! a file says of them is tidied before it is kept ([`RemoteNote::tidy`], [`repair`]).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use mailcraft_time::Timestamp;
use serde::{Deserialize, Serialize};

use crate::remote::clean_key;
use crate::{Id, Mailbox, OnlineMeeting, Origin};

/// Most notes a mailbox keeps, and most of one server's.
pub const MAX_NOTES: usize = 10_000;
pub const MAX_SERVER_NOTES: usize = 5_000;
/// The longest title and text of a note, in characters.
pub const MAX_NOTE_TITLE: usize = 140;
pub const MAX_NOTE_TEXT: usize = 100_000;
/// Most changes of notes that may wait for servers at once, and most that one [`NotesUpload`] carries.
pub const MAX_NOTE_CHANGES: usize = 500;
pub const MAX_NOTE_OPS: usize = 50;
/// The longest version of a server's note kept ([`Note::version`]), mark a new one is sent with, category, name of an
/// owner, and words of why a change didn't go.
const MAX_VERSION: usize = 64;
const MAX_TOKEN: usize = 64;
const MAX_CATEGORY: usize = 140;
const MAX_OWNER: usize = 200;
const MAX_WHY: usize = 300;
/// Most properties a server says it writes of a note, and the longest name of one.
const MAX_WRITABLE: usize = 20;
const MAX_PROPERTY: usize = 40;

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn is_zero_time(t: &Timestamp) -> bool {
    *t == 0
}

fn is_true(b: &bool) -> bool {
    *b
}

/// One line of text from outside: control characters read as spaces, at most `max` characters; and whether it was
/// cut.
fn one_line(text: &str, max: usize) -> (String, bool) {
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let trimmed = flat.trim();
    let cut = trimmed.chars().nth(max).is_some();
    let short: String = trimmed.chars().take(max).collect();
    (short.trim_end().to_string(), cut)
}

/// A note's text from outside: lines ended by `\n`, no control character but a tab, at most [`MAX_NOTE_TEXT`]
/// characters; and whether it was cut.
pub fn clean_text(text: &str) -> (String, bool) {
    let mut out = String::new();
    let mut count = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let c = match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                '\n'
            }
            '\n' | '\t' => c,
            c if c.is_control() => continue,
            c => c,
        };
        if count >= MAX_NOTE_TEXT {
            return (out, true);
        }
        out.push(c);
        count += 1;
    }
    (out, false)
}

/// A note's title from outside: one line, at most [`MAX_NOTE_TITLE`] characters; and whether it was cut.
pub fn clean_title(title: &str) -> (String, bool) {
    one_line(title, MAX_NOTE_TITLE)
}

/// What may be done to a note ([`Note::rights`]): all of it for the mailbox's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteRights {
    /// Its title changed, it pinned or archived.
    #[serde(skip_serializing_if = "is_true")]
    pub write: bool,
    /// Its text changed (not a rich note's, nor one cut here).
    #[serde(skip_serializing_if = "is_true")]
    pub write_text: bool,
    /// It deleted (its owner only).
    #[serde(skip_serializing_if = "is_true")]
    pub delete: bool,
}

impl Default for NoteRights {
    fn default() -> NoteRights {
        NoteRights { write: true, write_text: true, delete: true }
    }
}

impl NoteRights {
    fn is_all(&self) -> bool {
        *self == NoteRights::default()
    }
}

/// A note.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Note {
    pub id: Id,
    /// The account whose server keeps it, and what the server calls it; the mailbox's own has account 0.
    pub origin: Origin,
    /// One line, [`MAX_NOTE_TITLE`] characters at most; empty for one that has none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub archived: bool,
    /// Its text has more than words where it was written (formatting, a checklist, attachments): it is read here as
    /// words, and changed there.
    #[serde(skip_serializing_if = "is_false")]
    pub rich: bool,
    /// It was longer than is kept here, and is kept cut: its text is not changed here.
    #[serde(skip_serializing_if = "is_false")]
    pub cut: bool,
    /// The category its server gave it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// When it was made and last changed (Unix seconds).
    #[serde(skip_serializing_if = "is_zero_time")]
    pub created: Timestamp,
    #[serde(skip_serializing_if = "is_zero_time")]
    pub updated: Timestamp,
    /// For a server's: its version as the server last said it (its `updated`, as it was written), which a change of
    /// its words is made from.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// Somebody else owns it, and shared it with the person; who.
    #[serde(skip_serializing_if = "is_false")]
    pub shared: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub owner_name: String,
    #[serde(skip_serializing_if = "NoteRights::is_all")]
    pub rights: NoteRights,
    /// Where it is opened on the web (https only); empty for none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub web_url: String,
    /// For a new one of a server's: the mark it is sent with, so that sent twice it makes one note.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    /// Why its last change could not be saved, for the person to read; empty when nothing is wrong.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// What may be shown of a note: what it is, never what it says.
impl std::fmt::Debug for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Note")
            .field("id", &self.id)
            .field("account", &self.origin.account)
            .field("title", &self.title.chars().count())
            .field("text", &self.text.chars().count())
            .field("pinned", &self.pinned)
            .field("archived", &self.archived)
            .field("rich", &self.rich)
            .field("shared", &self.shared)
            .finish_non_exhaustive()
    }
}

impl Note {
    /// Its title, else the first line of its text that says something.
    pub fn heading(&self) -> &str {
        let title = self.title.trim();
        if !title.is_empty() {
            return title;
        }
        self.text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default()
    }

    /// Whether its text may be changed here: its rights say so, and it is neither rich nor cut.
    pub fn text_writable(&self) -> bool {
        self.rights.write && self.rights.write_text && !self.rich && !self.cut
    }

    /// Cut to what is kept, as everything from outside is.
    pub fn tidy(&mut self) {
        let (title, title_cut) = clean_title(&self.title);
        let (text, text_cut) = clean_text(&self.text);
        (self.title, self.text) = (title, text);
        self.cut |= title_cut || text_cut;
        self.origin.id = clean_key(&self.origin.id);
        self.category = one_line(&self.category, MAX_CATEGORY).0;
        self.version = clean_version(&self.version);
        self.owner_name = one_line(&self.owner_name, MAX_OWNER).0;
        self.web_url = OnlineMeeting::clean_link(&self.web_url).unwrap_or_default();
        self.token = clean_token(&self.token);
        self.error = one_line(&self.error, MAX_WHY).0;
        if self.origin.is_local() {
            // (The mailbox's own: nobody else's, and all the person's to change.)
            (self.shared, self.rich, self.rights) = (false, false, NoteRights::default());
            self.owner_name.clear();
            self.version.clear();
        }
    }
}

/// A version as a server says it: one line, [`MAX_VERSION`] bytes at most, else none.
fn clean_version(version: &str) -> String {
    let version = version.trim();
    if version.len() > MAX_VERSION || version.chars().any(char::is_control) { String::new() } else { version.to_string() }
}

/// The mark of a new note: letters, digits and hyphens, [`MAX_TOKEN`] at most, else none.
fn clean_token(token: &str) -> String {
    let token = token.trim();
    let fits = token.len() <= MAX_TOKEN && token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if fits { token.to_string() } else { String::new() }
}

/// An account whose server keeps notes, with what it says of them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteServer {
    pub account: Id,
    /// The longest title and text it takes.
    pub max_title: usize,
    pub max_text: usize,
    /// What it says it writes of a note (`isPinned`, `text`…), when it said.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub writable: Vec<String>,
}

impl Default for NoteServer {
    fn default() -> NoteServer {
        NoteServer { account: 0, max_title: MAX_NOTE_TITLE, max_text: MAX_NOTE_TEXT, writable: Vec::new() }
    }
}

impl NoteServer {
    /// Cut to what is kept: limits between 1 and the model's, a few property names of one word.
    pub fn tidy(&mut self) {
        self.max_title = self.max_title.clamp(1, MAX_NOTE_TITLE);
        self.max_text = self.max_text.clamp(1, MAX_NOTE_TEXT);
        let mut kept: Vec<String> = Vec::new();
        for name in std::mem::take(&mut self.writable) {
            let fits = (1..=MAX_PROPERTY).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric());
            if fits && !kept.contains(&name) && kept.len() < MAX_WRITABLE {
                kept.push(name);
            }
        }
        self.writable = kept;
    }
}

/// What is done to a note that a server keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteChangeKind {
    /// A note made here that the server doesn't have yet.
    #[default]
    Create,
    /// A note the server has that was changed here.
    Update,
    /// A note the server has that was deleted here.
    Delete,
}

/// A change made here to a note that an account's server keeps, which the server is yet to be told. The change is made
/// in the mailbox already; this is the note that it is to go to the server too. What is sent is worked out when it
/// goes, from the note as it is then: a new note whole, a changed one as what differs from `base`. So several changes
/// before the server hears of any are one, and one taken back is none.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteChange {
    pub account: Id,
    pub kind: NoteChangeKind,
    /// The note here; for a deletion, the id it had.
    pub note: Id,
    /// What the server calls it; empty for one it doesn't have yet.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote: String,
    /// For a new note: the mark it is sent with (sent twice, it makes one note).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    /// For a change or a deletion: the note as the server had it when it was first changed here. What is sent is what
    /// differs from it; what is put back when the server refuses; and its version is the one the change was made from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Box<Note>>,
    /// How many times in a row the server couldn't be told, and not before when it is tried again (Unix seconds).
    #[serde(skip_serializing_if = "is_zero")]
    pub failures: u32,
    #[serde(skip_serializing_if = "is_zero_time")]
    pub retry_at: Timestamp,
    /// Why it didn't go through the last time.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

impl std::fmt::Debug for NoteChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoteChange")
            .field("account", &self.account)
            .field("kind", &self.kind)
            .field("note", &self.note)
            .field("base", &self.base.is_some())
            .field("failures", &self.failures)
            .finish_non_exhaustive()
    }
}

/// What a server is told of a note: everything for a new one, what was changed of one it has (`None` leaves a part
/// as the server has it).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
}

impl std::fmt::Debug for NoteFields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoteFields")
            .field("title", &self.title.is_some())
            .field("text", &self.text.is_some())
            .field("pinned", &self.pinned)
            .field("archived", &self.archived)
            .finish()
    }
}

impl NoteFields {
    /// Nothing is said.
    pub fn is_empty(&self) -> bool {
        *self == NoteFields::default()
    }

    /// Whether it says what the note says (its title or its text), which a version guards.
    pub fn has_words(&self) -> bool {
        self.title.is_some() || self.text.is_some()
    }

    /// Cut to what a server is sent.
    pub fn tidy(&mut self) {
        self.title = self.title.take().map(|title| clean_title(&title).0);
        self.text = self.text.take().map(|text| clean_text(&text).0);
    }
}

/// One thing a server is asked to do to a note ([`NotesUpload`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteOp {
    pub kind: NoteChangeKind,
    /// The note here the change is about (for the answer to find it again).
    pub note: Id,
    /// What the server calls it, for a change or a deletion.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote: String,
    /// The mark a new note is made with.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    pub fields: NoteFields,
    /// For a change of its words: the version it was made from; empty otherwise (a pin or an archive carries none).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
}

impl NoteOp {
    /// Cut to size, as everything from outside is.
    pub fn tidy(&mut self) {
        self.remote = clean_key(&self.remote);
        self.token = clean_token(&self.token);
        self.fields.tidy();
        self.version = clean_version(&self.version);
    }
}

/// The changes to notes that a server is to be told, oldest first. The engine says what they are
/// (`sync.noteChanges`); a host carries them out and brings back a [`NotesUploaded`].
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotesUpload {
    pub account: Id,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    pub ops: Vec<NoteOp>,
}

impl std::fmt::Debug for NotesUpload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotesUpload").field("account", &self.account).field("ops", &self.ops.len()).finish()
    }
}

/// What became of one [`NoteOp`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteResult {
    /// The server did it.
    Done,
    /// The server has no such note (it was deleted elsewhere).
    Gone,
    /// The server has a newer version of it than the change was made from: its words were not written.
    Stale,
    /// The server won't do it, for a reason that asking again won't change.
    Refused,
    /// It couldn't be done now: it is tried again.
    #[default]
    Failed,
}

/// What became of one [`NoteOp`], with the op itself (the mailbox may have moved on since it was asked for).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteOutcome {
    pub op: NoteOp,
    pub result: NoteResult,
    /// The note as the server has it after a create or a change, as far as its answer said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<RemoteNote>,
    /// Why not, in words for the owner; never anything that was sent.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// How many seconds the server asked to be left alone, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u32>,
}

impl NoteOutcome {
    /// Cut to size, as everything from outside is.
    pub fn tidy(&mut self) {
        self.op.tidy();
        if let Some(note) = &mut self.note {
            note.tidy();
        }
        self.error = one_line(&self.error, MAX_WHY).0;
    }
}

/// What a server made of a [`NotesUpload`]: an outcome for every op that was tried, in order (one without an outcome
/// was not tried, and waits as before), and the state of the notes before and after.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotesUploaded {
    pub account: Id,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    pub outcomes: Vec<NoteOutcome>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub old_state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub new_state: String,
    /// The server stopped taking the sign-in part-way: a new one is needed for the rest.
    #[serde(skip_serializing_if = "is_false")]
    pub token_refused: bool,
}

impl std::fmt::Debug for NotesUploaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotesUploaded").field("account", &self.account).field("outcomes", &self.outcomes.len()).finish_non_exhaustive()
    }
}

impl NotesUploaded {
    /// Cut to size: as many outcomes as an upload has ops, each tidied; states that can be ones.
    pub fn tidy(&mut self) {
        self.outcomes.truncate(MAX_NOTE_OPS);
        self.outcomes.iter_mut().for_each(NoteOutcome::tidy);
        self.old_state = clean_version(&self.old_state);
        self.new_state = clean_version(&self.new_state);
    }
}

/// A note as a server has it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteNote {
    /// What the server calls it. A note without one is not kept.
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "is_false")]
    pub rich: bool,
    /// It was longer than is kept here (its text came cut).
    #[serde(skip_serializing_if = "is_false")]
    pub cut: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub archived: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub category: String,
    #[serde(skip_serializing_if = "is_zero_time")]
    pub created: Timestamp,
    #[serde(skip_serializing_if = "is_zero_time")]
    pub updated: Timestamp,
    /// Its `updated` as the server wrote it: the version a change of its words is made from.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(skip_serializing_if = "is_false")]
    pub shared: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub owner_name: String,
    /// What the person may do to it there (all of it, when the server says nothing).
    #[serde(skip_serializing_if = "is_true")]
    pub may_write: bool,
    #[serde(skip_serializing_if = "is_true")]
    pub may_write_text: bool,
    #[serde(skip_serializing_if = "is_true")]
    pub may_delete: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub web_url: String,
}

impl Default for RemoteNote {
    fn default() -> RemoteNote {
        RemoteNote {
            id: String::new(),
            title: String::new(),
            text: String::new(),
            rich: false,
            cut: false,
            pinned: false,
            archived: false,
            category: String::new(),
            created: 0,
            updated: 0,
            version: String::new(),
            shared: false,
            owner_name: String::new(),
            may_write: true,
            may_write_text: true,
            may_delete: true,
            web_url: String::new(),
        }
    }
}

impl std::fmt::Debug for RemoteNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteNote")
            .field("title", &self.title.chars().count())
            .field("text", &self.text.chars().count())
            .field("rich", &self.rich)
            .field("shared", &self.shared)
            .finish_non_exhaustive()
    }
}

impl RemoteNote {
    /// Cut to what is kept: an id that can be one, a title of one line, a text that is text (cut: [`RemoteNote::cut`]),
    /// a link that is https, a version of one short line.
    pub fn tidy(&mut self) {
        self.id = clean_key(&self.id);
        let (title, title_cut) = clean_title(&self.title);
        let (text, text_cut) = clean_text(&self.text);
        (self.title, self.text) = (title, text);
        self.cut |= title_cut || text_cut;
        self.category = one_line(&self.category, MAX_CATEGORY).0;
        self.version = clean_version(&self.version);
        self.owner_name = one_line(&self.owner_name, MAX_OWNER).0;
        self.web_url = OnlineMeeting::clean_link(&self.web_url).unwrap_or_default();
        // (Its text may be changed only where it may be written at all.)
        self.may_write_text &= self.may_write;
    }

    /// The note here it is for `account`, under `id`: what the server says of it, with what it may be done to.
    pub fn note(&self, account: Id, id: Id) -> Note {
        let rights = NoteRights { write: self.may_write, write_text: self.may_write_text && !self.rich && !self.cut, delete: self.may_delete };
        Note {
            id,
            origin: Origin { account, id: self.id.clone(), ..Default::default() },
            title: self.title.clone(),
            text: self.text.clone(),
            pinned: self.pinned,
            archived: self.archived,
            rich: self.rich,
            cut: self.cut,
            category: self.category.clone(),
            created: self.created,
            updated: self.updated,
            version: self.version.clone(),
            shared: self.shared,
            owner_name: self.owner_name.clone(),
            rights,
            web_url: self.web_url.clone(),
            token: String::new(),
            error: String::new(),
        }
    }
}

/// An account's notes as its server has them, or that it keeps none.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotesDownload {
    pub account: Id,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// The server keeps notes. False: it no longer does (or never did), and what this mailbox has of its notes goes.
    pub offered: bool,
    /// What it says of them: limits and what it writes.
    pub server: NoteServer,
    /// Every note it has is here: one that isn't is no longer there.
    #[serde(skip_serializing_if = "is_false")]
    pub complete: bool,
    pub notes: Vec<RemoteNote>,
}

impl std::fmt::Debug for NotesDownload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotesDownload")
            .field("account", &self.account)
            .field("offered", &self.offered)
            .field("complete", &self.complete)
            .field("notes", &self.notes.len())
            .finish()
    }
}

impl NotesDownload {
    /// Cut to size: as many notes as are kept of a server, each tidied, one per id (a list cut short is not complete).
    pub fn tidy(&mut self) {
        self.server.tidy();
        if self.notes.len() > MAX_SERVER_NOTES {
            self.notes.truncate(MAX_SERVER_NOTES);
            self.complete = false;
        }
        let mut seen: HashSet<String> = HashSet::new();
        for note in &mut self.notes {
            note.tidy();
        }
        self.notes.retain(|note| !note.id.is_empty() && seen.insert(note.id.clone()));
    }
}

/// What a job is asked of an account's notes: to read them (with the state they were last read whole at, and when),
/// and the changes to write first.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotesAsk {
    #[serde(skip_serializing_if = "is_false")]
    pub read: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub state: String,
    #[serde(skip_serializing_if = "is_zero_time")]
    pub read_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<NotesUpload>,
}

impl std::fmt::Debug for NotesAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotesAsk")
            .field("read", &self.read)
            .field("state", &!self.state.is_empty())
            .field("changes", &self.changes.as_ref().map(|changes| changes.ops.len()))
            .finish()
    }
}

/// What a job brought back of an account's notes: what it made of the changes, and the notes read (or, when the state
/// held is still theirs, only that state).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotesAnswer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded: Option<NotesUploaded>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<NotesDownload>,
    /// The state the notes were read whole at, or the one held, still theirs; `None` when it is not known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl std::fmt::Debug for NotesAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotesAnswer")
            .field("uploaded", &self.uploaded.as_ref().map(|uploaded| uploaded.outcomes.len()))
            .field("download", &self.download.as_ref().map(|download| download.notes.len()))
            .field("state", &self.state.is_some())
            .finish()
    }
}

impl NotesAnswer {
    /// Cut to size, as everything from outside is.
    pub fn tidy(&mut self) {
        if let Some(uploaded) = &mut self.uploaded {
            uploaded.tidy();
        }
        if let Some(download) = &mut self.download {
            download.tidy();
        }
        self.state = self.state.take().map(|state| clean_version(&state)).filter(|state| !state.is_empty());
    }
}

/// The notes a mailbox keeps: its own and its servers', the servers that keep notes, and what waits to be told to
/// them.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Notes {
    /// By their ids (written as a list of them).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", with = "items_list")]
    pub items: BTreeMap<Id, Arc<Note>>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "crate::sync::lenient_list")]
    pub servers: Vec<NoteServer>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "crate::sync::lenient_list")]
    pub changes: Vec<NoteChange>,
}

impl std::fmt::Debug for Notes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notes").field("items", &self.items.len()).field("servers", &self.servers.len()).field("changes", &self.changes.len()).finish()
    }
}

impl Notes {
    /// Nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.servers.is_empty() && self.changes.is_empty()
    }

    pub fn note(&self, id: Id) -> Option<&Note> {
        self.items.get(&id).map(Arc::as_ref)
    }

    pub fn note_mut(&mut self, id: Id) -> Option<&mut Note> {
        self.items.get_mut(&id).map(Arc::make_mut)
    }

    /// The server of `account`, when it keeps notes.
    pub fn server(&self, account: Id) -> Option<&NoteServer> {
        self.servers.iter().find(|server| server.account == account)
    }

    /// The note of `account`'s server that it calls `remote`.
    pub fn by_remote(&self, account: Id, remote: &str) -> Option<&Note> {
        if remote.is_empty() {
            return None;
        }
        self.items.values().map(Arc::as_ref).find(|note| note.origin.account == account && note.origin.id == remote)
    }

    /// The ids of the notes, for the count that gives new ones.
    pub fn ids(&self) -> impl Iterator<Item = &Id> {
        self.items.keys()
    }
}

/// The notes written as a list (each says its id), and read leniently: what isn't a note is passed over.
mod items_list {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use serde::{Deserializer, Serializer};

    use super::Note;
    use crate::Id;

    pub(super) fn serialize<S: Serializer>(items: &BTreeMap<Id, Arc<Note>>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(items.values())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<Id, Arc<Note>>, D::Error> {
        let list: Vec<Note> = crate::sync::lenient_list(deserializer)?;
        Ok(list.into_iter().map(|note| (note.id, Arc::new(note))).collect())
    }
}

/// Read a mailbox's notes leniently: something that isn't notes reads as none.
pub(crate) fn lenient<'de, D>(deserializer: D) -> Result<Notes, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Maybe {
        Is(Notes),
        Not(serde::de::IgnoredAny),
    }
    Ok(match Maybe::deserialize(deserializer)? {
        Maybe::Is(notes) => notes,
        Maybe::Not(_) => Notes::default(),
    })
}

/// Forget what the mailbox has of the notes of `accounts` (the accounts are going): their notes, their servers and what
/// waits for them.
pub(crate) fn forget_accounts(mailbox: &mut Mailbox, accounts: &HashSet<Id>) {
    let notes = &mut mailbox.notes;
    notes.items.retain(|_, note| !accounts.contains(&note.origin.account));
    notes.servers.retain(|server| !accounts.contains(&server.account));
    notes.changes.retain(|change| !accounts.contains(&change.account));
}

/// Make a mailbox's notes hang together, whatever a file held: ids from the mailbox's count (`sane`), each note
/// tidied, a server's notes only of accounts here and one per what the server calls it (the first kept), changes only
/// of accounts here and of notes that are here (a deletion's is not), no more than is ever kept, one server an
/// account. What was fixed.
pub(crate) fn repair(mailbox: &mut Mailbox, sane: &dyn Fn(&Id) -> bool) -> Vec<String> {
    let mut fixed = Vec::new();
    if mailbox.notes.is_empty() {
        return fixed;
    }
    let accounts: HashSet<Id> = mailbox.accounts.iter().map(|account| account.id).collect();
    let items = std::mem::take(&mut mailbox.notes.items);
    let before = items.len();
    let mut kept: BTreeMap<Id, Arc<Note>> = BTreeMap::new();
    let mut origins: HashSet<(Id, String)> = HashSet::new();
    let mut renumbered = 0usize;
    for (key, mut note) in items {
        if kept.len() >= MAX_NOTES {
            break;
        }
        let item = Arc::make_mut(&mut note);
        item.tidy();
        if item.origin.is_remote()
            && (!accounts.contains(&item.origin.account)
                || (!item.origin.id.is_empty() && !origins.insert((item.origin.account, item.origin.id.clone()))))
        {
            continue;
        }
        let id = if key == 0 || !sane(&key) || kept.contains_key(&key) {
            renumbered += 1;
            mailbox.alloc_id()
        } else {
            key
        };
        item.id = id;
        kept.insert(id, note);
    }
    let removed = before.saturating_sub(kept.len());
    if removed > 0 {
        fixed.push(format!("{removed} note(s) of no account here, kept twice, or beyond what is kept, removed"));
    }
    if renumbered > 0 {
        fixed.push(format!("{renumbered} note id(s) corrected"));
    }
    mailbox.notes.items = kept;
    let servers = std::mem::take(&mut mailbox.notes.servers);
    for mut server in servers {
        server.tidy();
        if accounts.contains(&server.account) && mailbox.notes.server(server.account).is_none() {
            mailbox.notes.servers.push(server);
        }
    }
    let waiting = mailbox.notes.changes.len();
    let notes = &mailbox.notes.items;
    let fits = |change: &NoteChange| {
        accounts.contains(&change.account)
            && match change.kind {
                NoteChangeKind::Create | NoteChangeKind::Update => notes.get(&change.note).is_some_and(|note| note.origin.account == change.account),
                NoteChangeKind::Delete => !change.remote.is_empty(),
            }
    };
    let mut changes: Vec<NoteChange> = std::mem::take(&mut mailbox.notes.changes).into_iter().filter(fits).collect();
    changes.truncate(MAX_NOTE_CHANGES);
    for change in &mut changes {
        change.remote = clean_key(&change.remote);
        change.token = clean_token(&change.token);
        change.error = one_line(&change.error, MAX_WHY).0;
        if let Some(base) = &mut change.base {
            base.tidy();
        }
    }
    if changes.len() < waiting {
        fixed.push(format!("{} change(s) of notes that no longer fit the mailbox dropped", waiting - changes.len()));
    }
    mailbox.notes.changes = changes;
    fixed
}

#[cfg(test)]
mod tests;
