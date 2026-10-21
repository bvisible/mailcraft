// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A person's notes over JMAP, the last part of a job ([`super::sync_pim`]): what was changed of them here written to
//! the server, then the notes read ([`sync`]). JMAP has no standard for notes: this is a data type of the hosting
//! page's server in its shape (`Note/get`, `Note/query`, `Note/set`; `Note/changes` not kept), under a capability of
//! its own, [`NOTES`]. The capability names that server, the one place in this crate that does: it is the name the
//! server gives it, as a vendor's capability is named by its vendor (RFC 8620 §2).
//!
//! **What the session says.** The account of the notes (`primaryAccounts`), the longest title and text the server
//! takes (`maxSizeTitle`, held between 1 and 1,000, 140 when it says nothing; `maxSizeText`, between 1 and 100,000,
//! 100,000 when it says nothing: [`limits`]), and what it writes of a note (`writableProperties`). A session without the
//! capability is said in the answer as a server that keeps no notes (a download that is not `offered`): what the
//! mailbox has of them goes.
//!
//! 1. **The changes go first**, in `Note/set` calls of as many as the session takes in a `set`, [`MAX_PER_SET`] at most:
//!    a new note whole (`title`, `text`, `isPinned`, `isArchived`) with the mark it was made with as its `uid`, which
//!    the server knows for a week (sent again after an answer that was lost, it is the note made then); a change as the
//!    whole properties it names, with the `updated` it was made from when it changes the title or the text (the server
//!    refuses it, `stateMismatch`, when the note was changed since: the engine keeps both versions); a deletion. What
//!    the server made of each is an outcome ([`NoteOutcome`]): made, changed or deleted is done (with the note as the
//!    answer says it); `stateMismatch` is stale; `notFound` is gone (refused, for a new note); `forbidden`,
//!    `invalidProperties` and any other refusal is refused, with the server's words (their markup taken out, the
//!    secrets hidden, [`MAX_WORDS`] characters at most); a server that is busy, failed, or can't be reached, and a
//!    change its answer says nothing of, failed, to go again after the time it asked for or a minute. A call refused for
//!    its size is sent again in halves. A refused sign-in ends the job (`token_refused`: what was not tried waits), a
//!    busy server too.
//! 2. **Then the notes are read**, when the window asks, or when the server didn't take a change as it was sent (the
//!    window's notes are then other than the server's). When the window holds the state of its last whole read and it is less than an hour
//!    old ([`NotesAsk::state_to_ask`]), one `Note/query` for one id asks whether the server still says it: then nothing
//!    has changed, and nothing more is read. (After changes the server took, every one, from that very state, the
//!    state it said after them is the one asked about.) Otherwise `Note/query` from the start, archived notes too,
//!    page after page ([`Pager`]), [`MAX_SERVER_NOTES`] at most; then `Note/get` of what it found, as many in a call as
//!    the session takes in a `get`, [`MAX_PER_GET`] at most, for the thirteen properties of a note. A `get` that fails
//!    is asked again in halves, by its ids, until the note that breaks it is found: that one is left out. One job reads
//!    no more than [`TEXT_BUDGET`] characters of text: the notes after that wait for the next one.
//! 3. **What comes back** ([`NotesAnswer`]): the outcomes of the changes, with the server's states before and after
//!    them; the notes read ([`NotesDownload`]), complete when the query was read whole, every note it found was read
//!    (or the server said it was gone since), the budget was not reached, the server said the same state on every page,
//!    and the job was not stopped; with what the list called them when it was read whole at one state, even if some
//!    couldn't be read (`listed`: one not in it is gone); and the state the window's notes are at now: the one they were
//!    all read at, or the one held, still the server's. After any trouble no state comes, and the next job reads
//!    everything.
//!
//! **The lines** of trouble name no note: "Couldn't send the note changes: …", "Couldn't read the notes: …", "3 notes
//! couldn't be read: …".

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::note::{
    MAX_NOTE_OPS, MAX_SERVER_NOTES, NoteChangeKind, NoteOp, NoteOutcome, NoteResult, NoteServer, NotesAnswer, NotesDownload, NotesUpload,
    NotesUploaded, RemoteNote,
};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use serde_json::{Map, Value, json};

use super::{IDS_BYTES, ITEMS_BYTES, Pager, Stop, said, secrets_of, utc_date};
use crate::{CORE, Client, Error, Http, Secrets, valid_id};

/// The capability of a person's notes, as the hosting page's server names it.
pub const NOTES: &str = "https://neoffice.me/jmap/notes";
/// Most notes in one `Note/set`, and read in one `Note/get`.
pub const MAX_PER_SET: usize = 50;
pub const MAX_PER_GET: usize = 100;
/// Most characters of text one job reads.
pub const TEXT_BUDGET: usize = 8_000_000;
/// The longest words of the server's kept, in characters.
pub const MAX_WORDS: usize = 300;
/// How long a server that failed is left alone before a change goes again, when it doesn't say, in seconds.
const RETRY: u32 = 60;
/// Most times in a job that a call is halved.
const MAX_SPLITS: usize = 32;
/// The longest state of the notes that is believed, in bytes.
const MAX_STATE: usize = 256;
/// What every request names in `using`.
const USING: [&str; 2] = [CORE, NOTES];
/// What a note is read for.
const NOTE_PROPERTIES: [&str; 13] =
    ["id", "title", "text", "isRich", "isPinned", "isArchived", "category", "created", "updated", "isShared", "ownerName", "myRights", "webUrl"];
/// Why a change was not done: the server's answer says nothing of it.
const UNSAID: &str = "the server's answer said nothing of it";
/// Why a change of words is not sent: the version its words were read at isn't known (it would be written over
/// whatever is there). The notes are read, and it goes after.
const VERSION_UNKNOWN: &str = "the version its words were read at isn't known yet";

/// The longest title and text the session says the server takes, in characters: `maxSizeTitle` between 1 and 1,000
/// (140 when it says nothing), `maxSizeText` between 1 and 100,000 (100,000 when it says nothing).
pub(crate) fn limits(capability: Option<&Value>) -> (usize, usize) {
    (crate::limit(capability, "maxSizeTitle", 140, 1_000), crate::limit(capability, "maxSizeText", 100_000, 100_000))
}

/// Write the changes `request.notes` carries, then read the notes as it asks, into `out.notes` (see the module); the
/// notes are in the account of the session that offers them. A request that asks nothing of them is left alone.
pub(super) async fn sync<H: Http>(client: &Client<H>, request: &JmapPimRequest, cancel: &AtomicBool, out: &mut JmapPimResult) {
    let Some(ask) = request.notes.as_ref() else { return };
    let (account, email) = (request.account, request.email.clone());
    let Some(server_account) = client.notes_account() else {
        log::debug!("jmap pim: the session offers no notes");
        let gone = NotesDownload { account, email, offered: false, ..Default::default() };
        out.notes = Some(NotesAnswer { download: Some(gone), ..Default::default() });
        return;
    };
    let job = Job { client, account: server_account, cancel, secrets: secrets_of(request, &client.secrets, server_account) };
    let mut answer = NotesAnswer::default();
    let mut lines: Vec<String> = Vec::new();
    let mut held = ask.state_to_ask(request.now).map(str::to_string);
    let mut stop = None;
    // A job that only writes reads too when the server didn't take a change as it was sent: the window's notes are
    // other than the server's (a stale one put back, a note gone), and what the server has now is read at once.
    let mut read_again = false;
    if let Some(upload) = ask.changes.as_ref().filter(|upload| !upload.ops.is_empty()) {
        let (uploaded, stopped, line, read) = job.write(upload, request).await;
        lines.extend(line);
        // After changes the server took, all of them, from the very state held, the window's notes are at the state it
        // said after them. A change it didn't take as it was sent (stale, gone, refused) leaves the window's notes
        // other than the server's: they are read again. (Any other state held is only asked about: a server that says
        // another reads everything.)
        let kept = uploaded.outcomes.iter().all(|outcome| matches!(outcome.result, NoteResult::Done | NoteResult::Failed));
        held = match held {
            Some(_) if !kept || stopped.is_some() => None,
            Some(state) if uploaded.old_state == state && !uploaded.new_state.is_empty() => Some(uploaded.new_state.clone()),
            other => other,
        };
        read_again = !kept || read;
        // A change waits for the version of its words: the state held says nothing of it, and the notes are read whole.
        if read {
            held = None;
        }
        answer.uploaded = Some(uploaded);
        stop = stopped;
    }
    if stop.is_none() && (ask.read || read_again) {
        let mut got = Gathered::new();
        let read = job.read(held.as_deref(), &mut got).await.err();
        match read {
            None => {}
            Some(Stop::Cancelled) => out.stopped = true,
            Some(Stop::Auth) => out.auth_refused = true,
            Some(Stop::Busy(seconds, why)) => {
                out.busy_for = Some(seconds);
                got.halted = Some(why);
            }
            Some(Stop::Failed(error)) => got.halted = Some(said(&job.secrets, &error)),
        }
        lines.extend(got.lines());
        answer.state = job.finish(got, &mut answer.download, request, held.as_deref());
    } else {
        match stop {
            Some(Stop::Cancelled) => out.stopped = true,
            Some(Stop::Auth) => out.auth_refused = true,
            Some(Stop::Busy(seconds, _)) => out.busy_for = Some(seconds),
            Some(Stop::Failed(_)) | None => {}
        }
        // Nothing was read: the state held, when it is still the window's.
        answer.state = if stop.is_none() { held } else { None };
    }
    log::debug!(
        "jmap pim: {} note change(s) answered, {} note(s) read (complete: {}), state {}",
        answer.uploaded.as_ref().map_or(0, |uploaded| uploaded.outcomes.len()),
        answer.download.as_ref().map_or(0, |download| download.notes.len()),
        answer.download.as_ref().is_some_and(|download| download.complete),
        if answer.state.is_some() { "known" } else { "unknown" }
    );
    out.errors.extend(lines);
    out.notes = Some(answer);
}

/// One job's work on the notes.
struct Job<'a, H> {
    client: &'a Client<H>,
    /// The session's account of the notes.
    account: &'a str,
    cancel: &'a AtomicBool,
    /// What a line must not show.
    secrets: Secrets,
}

/// What the writing has come to.
struct Writes {
    ops: Vec<NoteOp>,
    outcomes: Vec<Option<NoteOutcome>>,
    /// The server's state before the first call and after the last; and whether each call began where the one before
    /// ended (nobody else changed the notes between them, and every call ran or didn't, as far as is known).
    old_state: Option<String>,
    new_state: Option<String>,
    chained: bool,
    splits: usize,
    /// Why a call couldn't be made, for the line.
    trouble: Option<String>,
    /// The notes are to be read in this job, whatever was asked: a change waits for a version only a read brings.
    read: bool,
}

impl Writes {
    /// Say what became of the change at `at`, unless something was said already.
    fn answer(&mut self, at: usize, result: NoteResult, note: Option<RemoteNote>, words: &str, retry_after: Option<u32>) {
        let Some(op) = self.ops.get(at) else { return };
        if let Some(slot) = self.outcomes.get_mut(at).filter(|slot| slot.is_none()) {
            let mut outcome = NoteOutcome { op: op.clone(), result, note, error: crate::tidy(words, MAX_WORDS), retry_after };
            outcome.tidy();
            *slot = Some(outcome);
        }
    }

    fn failed(&mut self, at: usize, words: &str, retry_after: Option<u32>) {
        self.answer(at, NoteResult::Failed, None, words, Some(retry_after.unwrap_or(RETRY)));
    }

    /// The states one call's answer says: the chain holds while each begins where the last ended.
    fn states(&mut self, answer: &Value) {
        let state =
            |key: &str| answer.get(key).and_then(Value::as_str).filter(|state| !state.is_empty() && state.len() <= MAX_STATE).map(str::to_string);
        let (old, new) = (state("oldState"), state("newState"));
        match (&self.new_state, &old) {
            (None, _) if self.old_state.is_none() => self.old_state.clone_from(&old),
            (Some(last), Some(old)) if last == old => {}
            _ => self.chained = false,
        }
        if old.is_none() || new.is_none() {
            self.chained = false;
        }
        self.new_state = new;
    }
}

/// What came of one call.
enum Sent {
    Answered,
    /// Refused for its size before it ran: smaller calls may go.
    TooLarge,
}

/// A change as a `Note/set` writes it: the properties it names, whole.
fn note_json(op: &NoteOp) -> Value {
    let mut body = Map::new();
    if let Some(title) = &op.fields.title {
        body.insert("title".into(), json!(title));
    }
    if let Some(text) = &op.fields.text {
        body.insert("text".into(), json!(text));
    }
    if let Some(pinned) = op.fields.pinned {
        body.insert("isPinned".into(), json!(pinned));
    }
    if let Some(archived) = op.fields.archived {
        body.insert("isArchived".into(), json!(archived));
    }
    match op.kind {
        NoteChangeKind::Create if !op.token.is_empty() => {
            body.insert("uid".into(), json!(op.token));
        }
        // The version a change of its words was made from: the server refuses it when the note changed since.
        NoteChangeKind::Update if op.fields.has_words() && !op.version.is_empty() => {
            body.insert("updated".into(), json!(op.version));
        }
        _ => {}
    }
    Value::Object(body)
}

/// A note of a `Note/get`'s answer, or of a create's: `None` for one whose id can't be one.
fn note_of(value: &Value) -> Option<RemoteNote> {
    let id = value.get("id").and_then(Value::as_str).filter(|id| valid_id(id))?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let rights = value.get("myRights");
    // What the server doesn't say of the rights: all of them for the person's own note, none for one shared with them.
    let shared = flag("isShared");
    let right = |key: &str| rights.and_then(|rights| rights.get(key)).and_then(Value::as_bool).unwrap_or(!shared);
    let version = text("updated");
    Some(RemoteNote {
        id: id.to_string(),
        title: text("title"),
        text: text("text"),
        rich: flag("isRich"),
        cut: false,
        pinned: flag("isPinned"),
        archived: flag("isArchived"),
        category: text("category"),
        created: utc_date(&text("created")).unwrap_or(0),
        updated: utc_date(&version).unwrap_or(0),
        version,
        shared: flag("isShared"),
        owner_name: text("ownerName"),
        may_write: right("mayWrite"),
        may_write_text: right("mayWriteText"),
        may_delete: right("mayDelete"),
        web_url: text("webUrl"),
    })
}

/// What a change's answer says of the note (`updated`, `title`, `text`, `isRich`, `myRights`), as a note of the server's
/// named `id`; `None` when it says nothing of it.
fn changed_of(id: &str, value: &Value) -> Option<RemoteNote> {
    let mut said = value.as_object()?.clone();
    // A version is taken with its words or not at all.
    if !["updated", "title", "text"].iter().all(|key| said.get(*key).is_some_and(Value::is_string)) {
        return None;
    }
    said.insert("id".into(), json!(id));
    note_of(&Value::Object(said))
}

/// Whether `note` (the server's) has exactly what one earlier sending of `op` carried, and every part `op` names went
/// then: its version is her own write, not someone else's.
fn ours(op: &NoteOp, note: &RemoteNote) -> bool {
    let fields = &op.fields;
    op.earlier.iter().any(|earlier| {
        let named = (fields.title.is_none() || earlier.title.is_some())
            && (fields.text.is_none() || earlier.text.is_some())
            && (fields.pinned.is_none() || earlier.pinned.is_some())
            && (fields.archived.is_none() || earlier.archived.is_some());
        named
            && earlier.title.as_ref().is_none_or(|title| *title == note.title)
            && earlier.text.as_ref().is_none_or(|text| *text == note.text)
            && earlier.pinned.is_none_or(|pinned| pinned == note.pinned)
            && earlier.archived.is_none_or(|archived| archived == note.archived)
    })
}

/// Text from a server with its markup taken out.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match (c, in_tag) {
            ('<', _) => in_tag = true,
            ('>', true) => in_tag = false,
            (_, true) => {}
            (c, false) => out.push(c),
        }
    }
    out
}

/// Whether a server's refusal is one that asking again later may get past.
/// What comes of a resend over her own write ([`Job::send_again`]) that the server refused with a SetError of `kind`:
/// `None` leaves it stale, as it was (a newer version there); trouble for now is tried again later.
fn after_resend(kind: &str) -> Option<NoteResult> {
    match kind {
        // Someone wrote meanwhile: a conflict, as it was.
        "stateMismatch" => None,
        "notFound" => Some(NoteResult::Gone),
        kind if failed_for_now(kind) => Some(NoteResult::Failed),
        _ => Some(NoteResult::Refused),
    }
}

fn failed_for_now(kind: &str) -> bool {
    matches!(kind, "rateLimit" | "serverFail" | "serverUnavailable" | "serverPartialFail" | "tooManyRequests" | "overQuota")
}

impl<H: Http> Job<'_, H> {
    fn check(&self) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) { Err(Stop::Cancelled) } else { Ok(()) }
    }

    fn call_stop(&self, error: Error) -> Stop {
        Stop::of(error, &self.secrets)
    }

    /// The server's words of a SetError, as the person reads them.
    fn words(&self, error: &Value) -> String {
        let kind = error.get("type").and_then(Value::as_str).unwrap_or_default();
        let words = error.get("description").and_then(Value::as_str).map(strip_tags).unwrap_or_default();
        if words.trim().is_empty() { crate::tidy(kind, MAX_WORDS) } else { crate::tidy(&self.secrets.hide(&words), MAX_WORDS) }
    }

    /// Write the changes of `upload` (see the module): what came of each, why the job stops when it does, and a line.
    async fn write(&self, upload: &NotesUpload, request: &JmapPimRequest) -> (NotesUploaded, Option<Stop>, Option<String>, bool) {
        let ops: Vec<NoteOp> = upload
            .ops
            .iter()
            .take(MAX_NOTE_OPS)
            .map(|op| {
                let mut op = op.clone();
                op.tidy();
                op
            })
            .collect();
        let mut w =
            Writes { outcomes: vec![None; ops.len()], ops, old_state: None, new_state: None, chained: true, splits: 0, trouble: None, read: false };
        let size = self.client.max_objects_in_set().clamp(1, MAX_PER_SET);
        let places: Vec<usize> = (0..w.ops.len()).collect();
        let mut work: Vec<Vec<usize>> = places.chunks(size).rev().map(<[usize]>::to_vec).collect();
        let mut stop = None;
        while let Some(call) = work.pop() {
            if let Err(stopped) = self.check() {
                stop = Some(stopped);
                break;
            }
            match self.set(&call, &mut w).await {
                Ok(Sent::Answered) => {}
                Ok(Sent::TooLarge) if call.len() > 1 && w.splits < MAX_SPLITS => {
                    w.splits = w.splits.saturating_add(1);
                    let (first, second) = call.split_at(call.len() / 2);
                    work.push(second.to_vec());
                    work.push(first.to_vec());
                }
                Ok(Sent::TooLarge) => {
                    let words = said(&self.secrets, &Error::TooLarge("the change".into()));
                    for at in call {
                        w.failed(at, &words, None);
                    }
                }
                Err(stopped) => {
                    stop = Some(stopped);
                    break;
                }
            }
        }
        let token_refused = matches!(stop, Some(Stop::Auth));
        let line = w.trouble.take().map(|why| format!("Couldn't send the note changes: {why}"));
        let read = w.read;
        let new_state = if w.chained { w.new_state.clone().unwrap_or_default() } else { String::new() };
        let uploaded = NotesUploaded {
            account: request.account,
            email: request.email.clone(),
            outcomes: w.outcomes.into_iter().flatten().collect(),
            old_state: w.old_state.unwrap_or_default(),
            new_state,
            token_refused,
        };
        (uploaded, stop, line, read)
    }

    /// One `Note/set` of the changes at `call`, and their outcomes.
    async fn set(&self, call: &[usize], w: &mut Writes) -> Result<Sent, Stop> {
        let (mut create, mut update, mut destroy) = (Map::new(), Map::new(), Vec::new());
        // (A note two changes are about takes the first; the second waits for the next job.)
        let mut named: HashSet<String> = HashSet::new();
        let mut sent: Vec<usize> = Vec::new();
        for &at in call {
            let Some(op) = w.ops.get(at) else { continue };
            match op.kind {
                NoteChangeKind::Create => {
                    create.insert(format!("k{at}"), note_json(op));
                }
                _ if !valid_id(&op.remote) => {
                    // A name no server gives: there is no such note there.
                    w.answer(at, NoteResult::Gone, None, "", None);
                    continue;
                }
                _ if !named.insert(op.remote.clone()) => continue,
                // A change of words without the version they were read at would be written over whatever is there:
                // not sent; the notes are read, and it goes after with that version.
                NoteChangeKind::Update if op.fields.has_words() && op.version.is_empty() => {
                    w.failed(at, VERSION_UNKNOWN, None);
                    w.read = true;
                    continue;
                }
                NoteChangeKind::Update => {
                    update.insert(op.remote.clone(), note_json(op));
                }
                NoteChangeKind::Delete => destroy.push(json!(op.remote)),
            }
            sent.push(at);
        }
        if sent.is_empty() {
            return Ok(Sent::Answered);
        }
        let args = json!({"accountId": self.account, "create": create, "update": update, "destroy": destroy});
        let (status, answers) = self.client.call_with_status(&USING, vec![("Note/set", args)], ITEMS_BYTES).await;
        let mut answers = match answers {
            Ok(answers) => answers,
            Err(Error::TooLarge(_)) if status.is_some_and(|status| (400..500).contains(&status)) => return Ok(Sent::TooLarge),
            Err(_) if status == Some(413) => return Ok(Sent::TooLarge),
            Err(error) => {
                // Whether it ran is not known: each goes again (a new note with its mark, which makes no second one).
                w.chained = false;
                let words = said(&self.secrets, &error);
                return match self.call_stop(error) {
                    Stop::Auth => Err(Stop::Auth),
                    Stop::Busy(seconds, why) => {
                        for &at in &sent {
                            w.failed(at, &words, Some(seconds));
                        }
                        Err(Stop::Busy(seconds, why))
                    }
                    _ => {
                        for &at in &sent {
                            w.failed(at, &words, None);
                        }
                        w.trouble.get_or_insert(words);
                        Ok(Sent::Answered)
                    }
                };
            }
        };
        let answer = match answers.take(0) {
            Ok(answer) => answer,
            Err(Error::Refused { kind, .. }) if kind == "requestTooLarge" => return Ok(Sent::TooLarge),
            Err(error) => {
                // The method refused as a whole ran nothing; an answer that is no answer to it may have run.
                let words = said(&self.secrets, &error);
                let refused = matches!(&error, Error::Refused { kind, .. } if !failed_for_now(kind));
                if !matches!(&error, Error::Refused { .. }) {
                    w.chained = false;
                }
                for &at in &sent {
                    if refused { w.answer(at, NoteResult::Refused, None, &words, None) } else { w.failed(at, &words, None) }
                }
                w.trouble.get_or_insert(words);
                return Ok(Sent::Answered);
            }
        };
        w.states(&answer);
        let map = |key: &str| answer.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
        let (created, not_created, updated, not_updated, not_destroyed) =
            (map("created"), map("notCreated"), map("updated"), map("notUpdated"), map("notDestroyed"));
        let destroyed: HashSet<&str> = answer.get("destroyed").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        let tried = sent.clone();
        for at in sent {
            let Some(op) = w.ops.get(at).cloned() else { continue };
            let key = format!("k{at}");
            let (done, error) = match op.kind {
                NoteChangeKind::Create => (created.get(&key).map(note_of), not_created.get(&key)),
                NoteChangeKind::Update => (updated.get(&op.remote).map(|said| changed_of(&op.remote, said)), not_updated.get(&op.remote)),
                NoteChangeKind::Delete => (destroyed.contains(op.remote.as_str()).then_some(None), not_destroyed.get(&op.remote)),
            };
            match (done, error) {
                (Some(note), _) => w.answer(at, NoteResult::Done, note, "", None),
                (None, Some(error)) => {
                    let kind = error.get("type").and_then(Value::as_str).unwrap_or_default();
                    let words = self.words(error);
                    let retry = error.get("retryAfter").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
                    match kind {
                        "stateMismatch" => w.answer(at, NoteResult::Stale, None, &words, None),
                        "notFound" if op.kind != NoteChangeKind::Create => w.answer(at, NoteResult::Gone, None, &words, None),
                        kind if failed_for_now(kind) => w.failed(at, &words, retry),
                        _ => w.answer(at, NoteResult::Refused, None, &words, None),
                    }
                }
                (None, None) => w.failed(at, UNSAID, None),
            }
        }
        self.done_after_all(&tried, w).await;
        Ok(Sent::Answered)
    }

    /// Of the changes at `tried`, those refused as stale whose very words the note there has were written by an earlier
    /// try whose answer was lost (the server wrote it, the connection broke): they are done, with the note as it is
    /// there, never a conflict with themselves. When the note can't be read now, one that went before without an answer
    /// is tried again later (the server's newer version may be her own write); the others stay stale, and the person's
    /// words are kept as a note of their own.
    async fn done_after_all(&self, tried: &[usize], w: &mut Writes) {
        let stale = |at: &usize| {
            w.outcomes
                .get(*at)
                .and_then(Option::as_ref)
                .is_some_and(|outcome| outcome.result == NoteResult::Stale && outcome.op.kind == NoteChangeKind::Update)
        };
        let stale: Vec<usize> = tried.iter().copied().filter(stale).collect();
        if stale.is_empty() {
            return;
        }
        let ids: Vec<String> = stale.iter().filter_map(|at| w.ops.get(*at).map(|op| op.remote.clone())).collect();
        let get = json!({"accountId": self.account, "ids": ids, "properties": NOTE_PROPERTIES});
        let answer = match self.client.call_within(&USING, vec![("Note/get", get)], ITEMS_BYTES).await {
            Ok(mut answers) => answers.take(0),
            Err(error) => Err(error),
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                let words = said(&self.secrets, &error);
                for at in stale {
                    if w.ops.get(at).is_some_and(|op| !op.earlier.is_empty()) {
                        Self::again_later(w, at, &words);
                    }
                }
                return;
            }
        };
        let there: Vec<RemoteNote> = answer.get("list").and_then(Value::as_array).into_iter().flatten().filter_map(note_of).collect();
        for at in stale {
            let Some(op) = w.ops.get(at).cloned() else { continue };
            let Some(note) = there.iter().find(|note| note.id == op.remote) else { continue };
            let fields = &op.fields;
            let same = fields.title.as_ref().is_none_or(|title| *title == note.title)
                && fields.text.as_ref().is_none_or(|text| *text == note.text)
                && fields.pinned.is_none_or(|pinned| pinned == note.pinned)
                && fields.archived.is_none_or(|archived| archived == note.archived);
            if same && let Some(slot) = w.outcomes.get_mut(at) {
                *slot = None;
                w.answer(at, NoteResult::Done, Some(note.clone()), "", None);
                continue;
            }
            // The note there has exactly what an earlier sending of it carried (its answer lost) and every part this
            // one names went then: the server's newer version is her own write. What she typed since goes over it,
            // checked against that version; never without one (it would be written over whatever is there by then).
            if ours(&op, note) {
                let mut said = note.clone();
                said.tidy();
                if said.version.is_empty() {
                    Self::again_later(w, at, VERSION_UNKNOWN);
                } else {
                    self.send_again(at, &op, &said.version, w).await;
                }
            }
        }
    }

    /// The change at `at` sent again over the version `version`, the server's copy of her own earlier write: done when
    /// the server takes it; stale as before when the server refuses it (someone wrote meanwhile); and tried again later
    /// when no answer says what came of it (the server may have written it: never a conflict with herself).
    async fn send_again(&self, at: usize, op: &NoteOp, version: &str, w: &mut Writes) {
        let again = NoteOp { version: version.to_string(), ..op.clone() };
        let args = json!({"accountId": self.account, "update": {op.remote.clone(): note_json(&again)}});
        let answer = match self.client.call_within(&USING, vec![("Note/set", args)], ITEMS_BYTES).await {
            Ok(mut answers) => answers.take(0),
            Err(error) => Err(error),
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                w.chained = false;
                Self::again_later(w, at, &said(&self.secrets, &error));
                return;
            }
        };
        w.states(&answer);
        if let Some(said) = answer.get("updated").and_then(|updated| updated.get(&op.remote)) {
            if let Some(slot) = w.outcomes.get_mut(at) {
                *slot = None;
            }
            w.answer(at, NoteResult::Done, changed_of(&op.remote, said), "", None);
            return;
        }
        // Refused: stale as before (her words are kept as a note of their own). Neither: nothing is known.
        let Some(error) = answer.get("notUpdated").and_then(|refused| refused.get(&op.remote)) else {
            Self::again_later(w, at, UNSAID);
            return;
        };
        let kind = error.get("type").and_then(Value::as_str).unwrap_or_default();
        if let Some(result) = after_resend(kind) {
            if let Some(slot) = w.outcomes.get_mut(at) {
                *slot = None;
            }
            let words = self.words(error);
            if result == NoteResult::Failed { w.failed(at, &words, None) } else { w.answer(at, result, None, &words, None) }
        }
    }

    /// The change at `at`, answered already, tried again later instead.
    fn again_later(w: &mut Writes, at: usize, words: &str) {
        if let Some(slot) = w.outcomes.get_mut(at) {
            *slot = None;
        }
        w.failed(at, words, None);
    }

    /// Read the notes into `got` (see the module): only whether the server still says `held`, when there is one.
    async fn read(&self, held: Option<&str>, got: &mut Gathered) -> Result<(), Stop> {
        self.check()?;
        if let Some(held) = held {
            let query = json!({"accountId": self.account, "limit": 1});
            let mut answers = self.client.call_within(&USING, vec![("Note/query", query)], IDS_BYTES).await.map_err(|e| self.call_stop(e))?;
            let answer = answers.take(0).map_err(Stop::Failed)?;
            if state_of(&answer).as_deref() == Some(held) {
                got.unchanged = true;
                return Ok(());
            }
        }
        while got.pager.done().is_none() {
            self.check()?;
            let at = got.pager.position();
            let query = json!({"accountId": self.account, "position": at, "limit": got.pager.limit(), "calculateTotal": at == 0});
            let mut answers = self.client.call_within(&USING, vec![("Note/query", query)], IDS_BYTES).await.map_err(|e| self.call_stop(e))?;
            let found = answers.take(0).map_err(Stop::Failed)?;
            match (state_of(&found), &got.state) {
                (state, None) if at == 0 => got.state = state,
                (state, kept) if state.as_ref() != kept.as_ref() => got.moved = true,
                _ => {}
            }
            got.pager.page(&found);
            got.answered = true;
        }
        let ids: Vec<String> = got.pager.ids().to_vec();
        self.read_notes(got, ids).await
    }

    /// `ids` asked for by themselves, as many at a time as the server takes and in halves when a call for them fails,
    /// until the note that breaks it is found: that one is left out, the rest are read; until the budget of text is
    /// spent.
    async fn read_notes(&self, got: &mut Gathered, ids: Vec<String>) -> Result<(), Stop> {
        let size = self.client.max_objects_in_get().clamp(1, MAX_PER_GET);
        let mut work: Vec<Vec<String>> = ids.chunks(size).rev().map(<[String]>::to_vec).collect();
        while let Some(chunk) = work.pop() {
            self.check()?;
            let get = json!({"accountId": self.account, "ids": chunk, "properties": NOTE_PROPERTIES});
            let answer = match self.client.call_within(&USING, vec![("Note/get", get)], ITEMS_BYTES).await {
                Ok(mut answers) => answers.take(0),
                // An answer too large to take is a call to ask in halves; any other failure of the request stops.
                Err(error @ Error::TooLarge(_)) => Err(error),
                Err(error) => return Err(self.call_stop(error)),
            };
            match answer {
                Ok(answer) => {
                    if !self.take_notes(got, &chunk, &answer) {
                        let left = work.iter().map(Vec::len).sum::<usize>();
                        got.unread(left, OVER_BUDGET);
                        return Ok(());
                    }
                }
                Err(_) if chunk.len() > 1 && got.splits < MAX_SPLITS => {
                    got.splits = got.splits.saturating_add(1);
                    let (first, second) = chunk.split_at(chunk.len() / 2);
                    work.push(second.to_vec());
                    work.push(first.to_vec());
                }
                Err(error) if chunk.len() == 1 => got.unread(1, &said(&self.secrets, &error)),
                // The server keeps failing: what is left waits for the next job.
                Err(error) => {
                    let left = chunk.len().saturating_add(work.iter().map(Vec::len).sum::<usize>());
                    got.unread(left, &said(&self.secrets, &error));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// The notes of a `get`'s answer, of those asked for (`ids`): each once; what it left out without saying it was gone
    /// is counted as unread. `false` once the budget of text is spent (the notes of `ids` after it are counted too).
    fn take_notes(&self, got: &mut Gathered, ids: &[String], answer: &Value) -> bool {
        let Some(list) = answer.get("list").and_then(Value::as_array) else {
            got.unread(ids.len(), MISSING);
            return true;
        };
        let asked: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let gone: HashSet<&str> = answer.get("notFound").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        let mut came: HashSet<String> = HashSet::new();
        for value in list.iter().take(ids.len()) {
            let Some(note) = note_of(value).filter(|note| asked.contains(note.id.as_str()) && !came.contains(&note.id)) else { continue };
            let length = note.text.chars().count().saturating_add(note.title.chars().count());
            if length > got.budget {
                got.over_budget = true;
                let left = ids.iter().filter(|id| !came.contains(id.as_str()) && !gone.contains(id.as_str())).count();
                got.unread(left, OVER_BUDGET);
                return false;
            }
            got.budget -= length;
            came.insert(note.id.clone());
            if got.seen.insert(note.id.clone()) {
                got.notes.push(note);
            }
        }
        let left_out = ids.iter().filter(|id| !came.contains(id.as_str()) && !gone.contains(id.as_str())).count();
        if left_out > 0 {
            got.unread(left_out, MISSING);
        }
        true
    }

    /// The download, from what was read (into `download`), and the state the window's notes are at: `held` when the
    /// server still says it, the state they were all read at, or none.
    fn finish(&self, got: Gathered, download: &mut Option<NotesDownload>, request: &JmapPimRequest, held: Option<&str>) -> Option<String> {
        if got.unchanged {
            return held.map(str::to_string);
        }
        if !got.answered {
            return None;
        }
        let complete = got.pager.done() == Some(true)
            && got.unread.0 == 0
            && got.halted.is_none()
            && !got.moved
            && !got.over_budget
            && !self.cancel.load(Ordering::Relaxed);
        // The list read whole, at one state, even when some of its notes couldn't be read: one not in it is gone.
        let listed = (got.pager.done() == Some(true) && !got.moved && !self.cancel.load(Ordering::Relaxed)).then(|| got.pager.ids().to_vec());
        let (max_title, max_text) = self.client.notes_limits();
        let server = NoteServer {
            account: request.account,
            max_title,
            max_text,
            writable: self.client.notes_writable().map(<[String]>::to_vec).unwrap_or_default(),
        };
        *download =
            Some(NotesDownload { account: request.account, email: request.email.clone(), offered: true, server, complete, notes: got.notes, listed });
        got.state.filter(|_| complete)
    }
}

/// Why notes of the server's were not read: its answer did not have them (a row of the mail's says so already).
const MISSING: &str = "the server's answer did not have them";

/// Why the notes after the budget are not read.
const OVER_BUDGET: &str = "the notes hold more text than one reading takes, and these are not shown";

/// The state a `Note/query` says (`queryState`), when it says one.
fn state_of(answer: &Value) -> Option<String> {
    answer.get("queryState").and_then(Value::as_str).filter(|state| !state.is_empty() && state.len() <= MAX_STATE).map(str::to_string)
}

/// What the reading has come to.
struct Gathered {
    /// The server still says the state the window holds: nothing more is read.
    unchanged: bool,
    /// The state the first page said, and whether a later page said another.
    state: Option<String>,
    moved: bool,
    pager: Pager,
    /// A page of the query was answered: there are notes to hand over, if only none.
    answered: bool,
    notes: Vec<RemoteNote>,
    seen: HashSet<String>,
    /// Notes the query found that could not be read, and why the first could not.
    unread: (usize, Option<String>),
    /// Characters of text still to be read in this job, and whether it was spent.
    budget: usize,
    over_budget: bool,
    /// Why the reading ended early.
    halted: Option<String>,
    splits: usize,
}

impl Gathered {
    fn new() -> Gathered {
        Gathered {
            unchanged: false,
            state: None,
            moved: false,
            pager: Pager::new(MAX_SERVER_NOTES, MAX_SERVER_NOTES),
            answered: false,
            notes: Vec::new(),
            seen: HashSet::new(),
            unread: (0, None),
            budget: TEXT_BUDGET,
            over_budget: false,
            halted: None,
            splits: 0,
        }
    }

    fn unread(&mut self, count: usize, why: &str) {
        if count > 0 {
            self.unread.0 = self.unread.0.saturating_add(count);
            self.unread.1.get_or_insert_with(|| why.to_string());
        }
    }

    /// The lines of trouble worth telling the owner.
    fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(why) = &self.halted {
            lines.push(format!("Couldn't read the notes: {why}"));
        }
        match (self.unread.0, &self.unread.1) {
            (1, Some(why)) => lines.push(format!("1 note couldn't be read: {why}")),
            (n @ 2.., Some(why)) => lines.push(format!("{n} notes couldn't be read: {why}")),
            _ => {}
        }
        lines
    }
}

#[cfg(test)]
mod tests;
