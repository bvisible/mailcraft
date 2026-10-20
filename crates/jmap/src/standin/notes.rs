// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A person's notes, when a test turns them on ([`Data::notes`](super::Data::notes)): `Note/query`, `Note/get` and
//! `Note/set` as strictly as the hosting page's server takes them ([`NOTES`]), and `Note/changes` and
//! `Note/queryChanges`, which it doesn't keep.
//!
//! - **The session** then names the capability, its account (the mail's, [`ACCOUNT`](super::ACCOUNT)) and what the
//!   account says of it: the longest title and text (`maxSizeTitle`, `maxSizeText`) and the properties it writes
//!   (`writableProperties`). A request may name the capability only then, and a method of notes needs it.
//! - **What the person sees** is their own notes and those shared with them ([`StoredNote::share`]); the others are
//!   not there (`notFound`).
//! - **`Note/query`** lists them as the desk's list has them: pinned first, then the latest changed, then by id. The
//!   one filter is `{isArchived}` (`unsupportedFilter` otherwise), with no sort (`unsupportedSort`) and no anchor; from
//!   a `position` that may be negative, as many as [`Notes::limit`] at a time (the limit said back when it set it),
//!   with the total when asked and the state of the notes (`queryState`, which changes when a note the person sees is
//!   added, changed or removed, or a share of one with them), unless [`Notes::says_state`] is off. It can't calculate
//!   changes.
//! - **`Note/get`** reads the notes named, or every one while there are no more than a `get` takes (`requestTooLarge`
//!   otherwise): the title, the words of the text, whether it is rich, pinned or archived, its category, when it was
//!   made and last changed (`updated`, to the millisecond when it has any: the version of the note), whether it is
//!   shared with the person and by whom, the person's rights (`mayWrite`; `mayWriteText`, not for a rich note;
//!   `mayDelete`, the owner's alone), and its address in the desk (`webUrl`); `properties` keeps those it knows of the
//!   ones named (and the id).
//! - **`Note/set`** makes, changes and deletes notes, as many as a `set` takes (`requestTooLarge`), with no
//!   `ifInState`; each refused on its own:
//!   - What is written is a whole property of `title`, `text`, `isPinned`, `isArchived`; what the server keeps for
//!     itself (every property it reads, `uid`, `@type`) may be sent and is left as it is; anything else, a key with a
//!     `/`, a title of more than one line or of more than [`Notes::max_title`] characters, a text of more than
//!     [`Notes::max_text`] are `invalidProperties`.
//!   - A new note without a title is named by the first line of its text; one made with a `uid` seen in the last
//!     seven days is the note made then, and nothing more is written.
//!   - A change that gives the `updated` it was made from is refused when the note has changed since
//!     (`stateMismatch`); only what differs is written; a rich note's text is `forbidden` (its title is not), and so is
//!     a change by a person it is shared with for reading.
//!   - Only the owner deletes a note (`forbidden`).
//!   - What was created is the whole note; what was updated, its `updated`, `title`, `text`, `isRich` and `myRights`.
//! - **Trouble on demand**: notes whose `get` fails ([`Notes::broken`]), and a note (by its id, or a new one by its
//!   `uid`) whose write is answered with the SetError a test names ([`Notes::refused`]). What it was sent is kept, in
//!   order ([`Notes::written`]).

use std::collections::{BTreeMap, BTreeSet};

use mailcraft_time::{Timestamp, format_iso_utc};
use serde_json::{Map, Value, json};

use super::{ACCOUNT, ORIGIN, Outcome, account, allow_only, invalid, refuse};
use crate::pct;
use crate::pim::notes::NOTES;

/// What the server writes of a note, as its session says.
pub const WRITABLE_NOTE: [&str; 4] = ["isArchived", "isPinned", "text", "title"];
/// Every property of a note it serves.
const NOTE_PROPERTIES: [&str; 13] =
    ["id", "title", "text", "isRich", "isPinned", "isArchived", "category", "created", "updated", "isShared", "ownerName", "myRights", "webUrl"];
/// Most notes one query lists, the longest title and text, as the server has them.
pub const MAX_NOTES: usize = 5_000;
const MAX_TITLE: usize = 140;
const MAX_TEXT: usize = 100_000;
/// How long a new note is known by its `uid`, in milliseconds.
const UID_KEPT: i64 = 7 * 86_400 * 1_000;
/// Where its clock starts (milliseconds), and how far a write moves it: a whole second once in a thousand, so a
/// version has its milliseconds, as the server's mostly have.
const NOTES_CLOCK: i64 = 1_790_000_000_000;
const TICK: i64 = 1_237;
/// The longest `uid` kept, and most writes kept as they came.
const MAX_UID: usize = 255;
const MAX_WRITTEN: usize = 10_000;
/// The person's own name, as the server says it of their own notes.
pub const PERSON_NAME: &str = "Ada Marlow";
/// The person's login on the server, who owns their notes.
const PERSON: &str = super::USER;

/// A note the stand-in has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredNote {
    /// Its name in the store (`VN-2026-00001`), its id in the API.
    pub id: String,
    pub title: String,
    /// The words of its text.
    pub text: String,
    /// It has formatting, a checklist, attachments: its text is changed where it was written.
    pub rich: bool,
    pub pinned: bool,
    pub archived: bool,
    pub category: Option<String>,
    /// When it was made and last changed, in milliseconds.
    pub created: i64,
    pub updated: i64,
    /// Who owns it ([`PERSON`] for the person's own) and their name.
    pub owner: String,
    pub owner_name: String,
    /// A colleague's note shared with the person: `Some(true)` for writing, `Some(false)` for reading; `None` for one
    /// that isn't (the person sees it only when it is theirs).
    pub share: Option<bool>,
}

impl StoredNote {
    /// The person's own plain note.
    pub fn new(id: &str, title: &str, text: &str) -> StoredNote {
        StoredNote {
            id: id.into(),
            title: title.into(),
            text: text.into(),
            rich: false,
            pinned: false,
            archived: false,
            category: None,
            created: 0,
            updated: 0,
            owner: PERSON.into(),
            owner_name: PERSON_NAME.into(),
            share: None,
        }
    }

    /// A colleague's, shared with the person for writing or for reading.
    pub fn shared_by(mut self, owner: &str, name: &str, writable: bool) -> StoredNote {
        (self.owner, self.owner_name, self.share) = (owner.into(), name.into(), Some(writable));
        self
    }

    /// A colleague's the person doesn't see.
    pub fn of_someone_else(mut self, owner: &str, name: &str) -> StoredNote {
        (self.owner, self.owner_name, self.share) = (owner.into(), name.into(), None);
        self
    }

    pub fn rich(mut self) -> StoredNote {
        self.rich = true;
        self
    }

    pub fn pinned(mut self) -> StoredNote {
        self.pinned = true;
        self
    }

    pub fn archived(mut self) -> StoredNote {
        self.archived = true;
        self
    }

    fn own(&self) -> bool {
        self.owner == PERSON
    }

    fn visible(&self) -> bool {
        self.own() || self.share.is_some()
    }

    fn may_write(&self) -> bool {
        self.own() || self.share == Some(true)
    }
}

/// The person's notes on the stand-in (see the module).
#[derive(Clone, Debug)]
pub struct Notes {
    /// Whether the session offers them. Off at first.
    pub offered: bool,
    pub list: Vec<StoredNote>,
    /// Most ids one query lists ([`MAX_NOTES`] at first).
    pub limit: usize,
    /// Whether a query says the notes' state.
    pub says_state: bool,
    /// The longest title and text it takes, as its session says them.
    pub max_title: usize,
    pub max_text: usize,
    /// Notes whose `get` fails (`serverFail` for the whole call), by id.
    pub broken: BTreeSet<String>,
    /// Notes (by id, or a new one by its `uid`) whose write is answered with this SetError `type`.
    pub refused: BTreeMap<String, &'static str>,
    /// The notes made with a `uid`: `uid` → (id, when).
    pub made: BTreeMap<String, (String, i64)>,
    /// The store's clock, in milliseconds.
    pub clock: i64,
    /// What `Note/set` was sent, in order: `create`, `update <id>` or `destroy <id>`, and what came with it.
    pub written: Vec<(String, Value)>,
    next: u32,
}

impl Default for Notes {
    fn default() -> Notes {
        Notes {
            offered: false,
            list: Vec::new(),
            limit: MAX_NOTES,
            says_state: true,
            max_title: MAX_TITLE,
            max_text: MAX_TEXT,
            broken: BTreeSet::new(),
            refused: BTreeMap::new(),
            made: BTreeMap::new(),
            clock: NOTES_CLOCK,
            written: Vec::new(),
            next: 0,
        }
    }
}

/// A version as the server writes it: UTC, with the milliseconds when there are any.
pub fn version(millis: i64) -> String {
    let seconds: Timestamp = millis.div_euclid(1_000);
    let fraction = millis.rem_euclid(1_000);
    let text = format_iso_utc(seconds);
    match text.strip_suffix('Z') {
        Some(head) if fraction != 0 => format!("{head}.{fraction:03}Z"),
        _ => text,
    }
}

/// The instant a version says, in milliseconds: UTC (`Z`), with or without a fraction.
fn moment(text: &str) -> Option<i64> {
    let head = text.strip_suffix(['Z', 'z'])?;
    let (whole, fraction) = match head.split_once('.') {
        Some((whole, fraction)) if !fraction.is_empty() && fraction.len() <= 9 && fraction.bytes().all(|b| b.is_ascii_digit()) => (whole, fraction),
        Some(_) => return None,
        None => (head, ""),
    };
    let seconds = crate::pim::utc_date(&format!("{whole}Z"))?;
    let millis: i64 = format!("{fraction:0<3}").get(..3)?.parse().ok()?;
    seconds.checked_mul(1_000)?.checked_add(millis)
}

fn set_error(kind: &str, description: impl Into<String>) -> Value {
    json!({"type": kind, "description": description.into()})
}

fn not_found() -> Value {
    json!({"type": "notFound"})
}

/// What a note or a patch writes: its title, text, and whether it is pinned or archived, each when it names it.
#[derive(Default)]
struct Fields {
    title: Option<String>,
    text: Option<String>,
    pinned: Option<bool>,
    archived: Option<bool>,
}

impl Notes {
    /// Notes offered, with none in them.
    pub fn offered() -> Notes {
        Notes { offered: true, ..Notes::default() }
    }

    /// Add a note; its id. One made at no time is made now.
    pub fn add(&mut self, mut note: StoredNote) -> String {
        if note.created == 0 {
            note.created = self.tick();
        }
        note.updated = note.updated.max(note.created);
        let id = note.id.clone();
        self.list.push(note);
        id
    }

    /// Change a note as another of the person's devices would (the desk, the phone): it is changed now. Whether it was
    /// there.
    pub fn change(&mut self, id: &str, how: impl FnOnce(&mut StoredNote)) -> bool {
        let now = self.tick();
        let Some(note) = self.list.iter_mut().find(|note| note.id == id) else { return false };
        how(note);
        note.updated = now;
        true
    }

    pub fn note(&self, id: &str) -> Option<&StoredNote> {
        self.list.iter().find(|note| note.id == id)
    }

    /// The store's clock, a little on: when what is made or changed now is.
    fn tick(&mut self) -> i64 {
        self.clock = self.clock.saturating_add(TICK);
        self.clock
    }

    fn new_id(&mut self) -> String {
        loop {
            self.next = self.next.saturating_add(1);
            let id = format!("VN-2026-{:05}", self.next);
            if self.note(&id).is_none() || self.next == u32::MAX {
                return id;
            }
        }
    }

    pub(super) fn add_to(&self, session: &mut Value, account: &str) {
        if self.offered {
            session["capabilities"][NOTES] = json!({});
            session["accounts"][account]["accountCapabilities"][NOTES] =
                json!({"maxSizeTitle": self.max_title, "maxSizeText": self.max_text, "writableProperties": WRITABLE_NOTE});
            session["primaryAccounts"][NOTES] = json!(account);
        }
    }

    /// Whether a request may name `capability` in `using`: the session offers it.
    pub(super) fn knows(&self, capability: &str) -> bool {
        capability == NOTES && self.offered
    }

    /// Whether `name` is a method of notes (offered).
    pub(super) fn has(&self, name: &str) -> bool {
        self.offered && matches!(name, "Note/get" | "Note/query" | "Note/set" | "Note/changes" | "Note/queryChanges")
    }

    /// A method of notes; `most_get` and `most_set` what a `get` and a `set` take.
    pub(super) fn method(&mut self, name: &str, args: &Map<String, Value>, most_get: usize, most_set: usize) -> Outcome<Value> {
        match name {
            "Note/query" => self.query(args),
            "Note/get" => self.get(args, most_get),
            "Note/set" => self.set(args, most_set),
            _ => {
                account(args)?;
                Err(refuse("cannotCalculateChanges", "Read the notes again."))
            }
        }
    }

    /// The notes the person sees, of one archive state when given, as the desk's list orders them.
    fn visible(&self, archived: Option<bool>) -> Vec<&StoredNote> {
        let mut seen: Vec<&StoredNote> =
            self.list.iter().filter(|note| note.visible() && archived.is_none_or(|archived| note.archived == archived)).collect();
        seen.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated.cmp(&a.updated)).then(a.id.cmp(&b.id)));
        seen
    }

    /// The notes' state: it changes when one the person sees is added, changed or removed, or a share of one with them.
    pub fn state(&self) -> String {
        let mut seen: Vec<(&str, i64, Option<bool>)> =
            self.list.iter().filter(|note| note.visible()).map(|note| (note.id.as_str(), note.updated, note.share)).collect();
        seen.sort_unstable();
        // FNV-1a over what they are.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in format!("{seen:?}").bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }

    fn query(&self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "filter", "sort", "position", "anchor", "anchorOffset", "limit", "calculateTotal"])?;
        account(args)?;
        if !matches!(args.get("anchor"), None | Some(Value::Null)) {
            return Err(invalid("anchor"));
        }
        let unsupported = || refuse("unsupportedFilter", "Only isArchived is supported.");
        let archived = match args.get("filter") {
            None | Some(Value::Null) => None,
            Some(Value::Object(filter)) if filter.keys().all(|key| key == "isArchived") => match filter.get("isArchived") {
                None | Some(Value::Null) => None,
                Some(Value::Bool(archived)) => Some(*archived),
                Some(_) => return Err(invalid("isArchived is true or false")),
            },
            Some(_) => return Err(unsupported()),
        };
        match args.get("sort") {
            None | Some(Value::Null) => {}
            Some(Value::Array(sort)) if sort.is_empty() => {}
            Some(_) => return Err(refuse("unsupportedSort", "The notes come as the desk's list has them.")),
        }
        let number = |key: &str| -> Outcome<Option<i64>> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(value) => value.as_i64().map(Some).ok_or_else(|| invalid(key.to_string())),
            }
        };
        let position = number("position")?.unwrap_or(0);
        let asked = number("limit")?;
        if asked.is_some_and(|limit| limit < 0) {
            return Err(invalid("limit"));
        }
        let most = self.limit.min(MAX_NOTES);
        let limit = asked.map_or(most, |limit| usize::try_from(limit).unwrap_or(usize::MAX).min(most));
        let found: Vec<&str> = self.visible(archived).into_iter().map(|note| note.id.as_str()).collect();
        let total = found.len();
        let position = if position < 0 {
            total.saturating_sub(usize::try_from(position.unsigned_abs()).unwrap_or(usize::MAX))
        } else {
            usize::try_from(position).unwrap_or(usize::MAX)
        };
        let ids: Vec<&str> = found.into_iter().skip(position).take(limit).collect();
        let mut out = json!({"accountId": ACCOUNT, "canCalculateChanges": false, "position": position, "ids": ids});
        if self.says_state {
            out["queryState"] = json!(self.state());
        }
        if args.get("calculateTotal").and_then(Value::as_bool) == Some(true) {
            out["total"] = json!(total);
        }
        if asked.is_none_or(|asked| usize::try_from(asked).unwrap_or(usize::MAX) > most) {
            out["limit"] = json!(most);
        }
        Ok(out)
    }

    /// A note as the server serves it, with the properties `wanted` (and the id), every one when `None`.
    fn served(note: &StoredNote, wanted: Option<&[String]>) -> Value {
        let mut out = json!({
            "id": note.id,
            "title": note.title,
            "text": note.text,
            "isRich": note.rich,
            "isPinned": note.pinned,
            "isArchived": note.archived,
            "category": note.category,
            "created": version(note.created.div_euclid(1_000) * 1_000),
            "updated": version(note.updated),
            "isShared": !note.own(),
            "ownerName": note.owner_name,
            "myRights": {"mayWrite": note.may_write(), "mayWriteText": note.may_write() && !note.rich, "mayDelete": note.own()},
            "webUrl": format!("{ORIGIN}/app/notes?note={}", pct(&note.id)),
        });
        if let (Some(wanted), Value::Object(map)) = (wanted, &mut out) {
            map.retain(|key, _| key == "id" || wanted.iter().any(|w| w == key));
        }
        out
    }

    fn get(&self, args: &Map<String, Value>, most: usize) -> Outcome<Value> {
        allow_only(args, &["accountId", "ids", "properties"])?;
        account(args)?;
        let ids: Vec<String> = match args.get("ids") {
            None | Some(Value::Null) => {
                let every = self.visible(None);
                if every.len() > most {
                    return Err(refuse("requestTooLarge", "Ask for the notes by id (Note/query)."));
                }
                every.into_iter().map(|note| note.id.clone()).collect()
            }
            Some(Value::Array(ids)) if ids.len() > most => return Err(refuse("requestTooLarge", "ids")),
            // What isn't a name names nothing.
            Some(Value::Array(ids)) => ids.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            Some(_) => return Err(invalid("ids")),
        };
        let wanted: Option<Vec<String>> = match args.get("properties") {
            None | Some(Value::Null) => None,
            Some(Value::Array(names)) => {
                let names: Option<Vec<String>> = names.iter().map(|n| n.as_str().map(str::to_string)).collect();
                let names = names.ok_or_else(|| invalid("properties"))?;
                Some(names.into_iter().filter(|name| NOTE_PROPERTIES.contains(&name.as_str())).collect())
            }
            Some(_) => return Err(invalid("properties")),
        };
        if ids.iter().any(|id| self.broken.contains(id)) {
            return Err(refuse("serverFail", "a note can't be read just now"));
        }
        let (mut list, mut not_found) = (Vec::new(), Vec::new());
        for id in &ids {
            match self.list.iter().find(|note| &note.id == id && note.visible()) {
                Some(note) => list.push(Notes::served(note, wanted.as_deref())),
                None => not_found.push(id.clone()),
            }
        }
        Ok(json!({"accountId": ACCOUNT, "state": self.state(), "list": list, "notFound": not_found}))
    }

    /// What a new note or a patch writes, held to what the server takes (`Err`: the SetError).
    fn fields(&self, given: &Value) -> Result<Fields, Value> {
        let bad = |words: String| set_error("invalidProperties", words);
        let Some(given) = given.as_object() else { return Err(bad("Only title, text, isPinned and isArchived can be written.".into())) };
        if given.keys().any(|key| key.contains('/')) {
            return Err(bad("A whole property replaces it: no patch of a part.".into()));
        }
        let known = |key: &str| WRITABLE_NOTE.contains(&key) || NOTE_PROPERTIES.contains(&key) || key == "uid" || key == "@type";
        if given.keys().any(|key| !known(key)) {
            return Err(bad("Only title, text, isPinned and isArchived can be written.".into()));
        }
        let mut fields = Fields::default();
        match given.get("title") {
            None => {}
            Some(Value::Null) => fields.title = Some(String::new()),
            Some(Value::String(title)) => {
                let title = title.trim();
                if title.chars().count() > self.max_title || title.contains('\n') {
                    return Err(bad(format!("A title is one line of {} characters at most.", self.max_title)));
                }
                fields.title = Some(title.to_string());
            }
            Some(_) => return Err(bad("A title is text.".into())),
        }
        match given.get("text") {
            None => {}
            Some(Value::Null) => fields.text = Some(String::new()),
            Some(Value::String(text)) if text.chars().count() > self.max_text => {
                return Err(bad(format!("A note holds {} characters at most.", self.max_text)));
            }
            Some(Value::String(text)) => fields.text = Some(text.clone()),
            Some(_) => return Err(bad("A text is text.".into())),
        }
        for (key, slot) in [("isPinned", &mut fields.pinned), ("isArchived", &mut fields.archived)] {
            match given.get(key) {
                None => {}
                Some(Value::Bool(value)) => *slot = Some(*value),
                Some(_) => return Err(bad(format!("{key} is true or false."))),
            }
        }
        Ok(fields)
    }

    fn written(&mut self, what: String, value: &Value) {
        if self.written.len() < MAX_WRITTEN {
            self.written.push((what, value.clone()));
        }
    }

    fn set(&mut self, args: &Map<String, Value>, most: usize) -> Outcome<Value> {
        allow_only(args, &["accountId", "ifInState", "create", "update", "destroy"])?;
        account(args)?;
        let empty = Map::new();
        let object = |key: &str| match args.get(key) {
            None | Some(Value::Null) => Some(&empty),
            Some(Value::Object(map)) => Some(map),
            Some(_) => None,
        };
        let destroys: Option<&[Value]> = match args.get("destroy") {
            None | Some(Value::Null) => Some(&[]),
            Some(Value::Array(list)) => Some(list),
            Some(_) => None,
        };
        let (Some(creates), Some(updates), Some(destroys)) = (object("create"), object("update"), destroys) else {
            return Err(invalid("create, update, destroy"));
        };
        if creates.len().saturating_add(updates.len()).saturating_add(destroys.len()) > most {
            return Err(refuse("requestTooLarge", "maxObjectsInSet"));
        }
        if !matches!(args.get("ifInState"), None | Some(Value::Null)) {
            return Err(invalid("ifInState"));
        }
        let old_state = self.state();
        let (mut created, mut not_created) = (Map::new(), Map::new());
        for (key, given) in creates {
            self.written("create".into(), given);
            match self.create(given) {
                Ok(id) => {
                    let made = self.note(&id).map_or_else(|| json!({"id": id}), |note| Notes::served(note, None));
                    created.insert(key.clone(), made);
                }
                Err(error) => {
                    not_created.insert(key.clone(), error);
                }
            }
        }
        let (mut updated, mut not_updated) = (Map::new(), Map::new());
        for (id, patch) in updates {
            self.written(format!("update {id}"), patch);
            match self.update(id, patch) {
                Ok(()) => {
                    let echo = ["updated", "title", "text", "isRich", "myRights"].map(String::from);
                    let said = self.note(id).map_or(Value::Null, |note| Notes::served(note, Some(&echo)));
                    let said = match said {
                        // (The server echoes these, not the id.)
                        Value::Object(mut map) => {
                            map.remove("id");
                            Value::Object(map)
                        }
                        other => other,
                    };
                    updated.insert(id.clone(), said);
                }
                Err(error) => {
                    not_updated.insert(id.clone(), error);
                }
            }
        }
        let (mut destroyed, mut not_destroyed) = (Vec::new(), Map::new());
        for id in destroys.iter().filter_map(Value::as_str) {
            self.written(format!("destroy {id}"), &Value::Null);
            match self.destroy(id) {
                Ok(()) => destroyed.push(id.to_string()),
                Err(error) => {
                    not_destroyed.insert(id.to_string(), error);
                }
            }
        }
        Ok(json!({
            "accountId": ACCOUNT,
            "oldState": old_state,
            "newState": self.state(),
            "created": created,
            "updated": updated,
            "destroyed": destroyed,
            "notCreated": not_created,
            "notUpdated": not_updated,
            "notDestroyed": not_destroyed,
        }))
    }

    fn create(&mut self, given: &Value) -> Result<String, Value> {
        let fields = self.fields(given)?;
        let uid: String = given.get("uid").and_then(Value::as_str).unwrap_or_default().chars().take(MAX_UID).collect();
        if let Some(kind) = (!uid.is_empty()).then(|| self.refused.get(&uid).copied()).flatten() {
            return Err(set_error(kind, "the server can't make this note just now"));
        }
        // A note made with this mark lately, still there and the person's: the same note.
        let lately = |(id, when): &&(String, i64)| self.clock.saturating_sub(*when) <= UID_KEPT && self.note(id).is_some_and(StoredNote::own);
        if let Some((id, _)) = self.made.get(&uid).filter(|_| !uid.is_empty()).filter(lately) {
            return Ok(id.clone());
        }
        let text = fields.text.unwrap_or_default();
        let title = fields.title.filter(|title| !title.is_empty());
        let title = title.unwrap_or_else(|| text.split('\n').map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_string());
        let id = self.new_id();
        let now = self.tick();
        let note = StoredNote {
            title: title.chars().take(self.max_title).collect(),
            pinned: fields.pinned.unwrap_or(false),
            archived: fields.archived.unwrap_or(false),
            created: now,
            updated: now,
            ..StoredNote::new(&id, "", &text)
        };
        self.list.push(note);
        if !uid.is_empty() {
            self.made.insert(uid, (id.clone(), now));
        }
        Ok(id)
    }

    fn update(&mut self, id: &str, patch: &Value) -> Result<(), Value> {
        let Some(note) = self.list.iter().find(|note| note.id == id && note.visible()).cloned() else { return Err(not_found()) };
        if let Some(kind) = self.refused.get(id) {
            return Err(set_error(kind, "the server can't change this note just now"));
        }
        let fields = self.fields(patch)?;
        match patch.get("updated") {
            None | Some(Value::Null) => {}
            Some(read) => match read.as_str().and_then(moment) {
                None => return Err(set_error("invalidProperties", "updated is a date.")),
                Some(read) if read != note.updated => {
                    return Err(set_error("stateMismatch", "The note was changed elsewhere: read it again."));
                }
                Some(_) => {}
            },
        }
        // Only what differs is written.
        let title = fields.title.filter(|title| *title != note.title);
        let text = fields.text.filter(|text| *text != note.text);
        let pinned = fields.pinned.filter(|pinned| *pinned != note.pinned);
        let archived = fields.archived.filter(|archived| *archived != note.archived);
        if text.is_some() && note.rich {
            return Err(set_error("forbidden", "This note is changed where it was written: open it there."));
        }
        if title.is_none() && text.is_none() && pinned.is_none() && archived.is_none() {
            return Ok(());
        }
        if !note.may_write() {
            return Err(set_error("forbidden", "Not permitted"));
        }
        let now = self.tick();
        if let Some(kept) = self.list.iter_mut().find(|kept| kept.id == id) {
            if let Some(title) = title {
                kept.title = title;
            }
            if let Some(text) = text {
                kept.text = text;
            }
            if let Some(pinned) = pinned {
                kept.pinned = pinned;
            }
            if let Some(archived) = archived {
                kept.archived = archived;
            }
            kept.updated = now;
        }
        Ok(())
    }

    fn destroy(&mut self, id: &str) -> Result<(), Value> {
        let Some(note) = self.list.iter().find(|note| note.id == id && note.visible()) else { return Err(not_found()) };
        if let Some(kind) = self.refused.get(id) {
            return Err(set_error(kind, "the server can't delete this note just now"));
        }
        if !note.own() {
            return Err(set_error("forbidden", "Only its owner deletes a note."));
        }
        self.list.retain(|note| note.id != id);
        Ok(())
    }
}

/// The person's notes of the demo, as of `morning` (milliseconds): a pinned one, a list, a rich one, one a colleague
/// shares for reading, an archived one.
pub(super) fn demo_notes(notes: &mut Notes, morning: i64) {
    *notes = Notes { clock: morning, ..Notes::offered() };
    notes.add(StoredNote::new("VN-2026-00101", "Team offsite ideas", "Lake walk\nCooking class\nBoat trip, if the weather holds").pinned());
    notes.add(StoredNote::new("VN-2026-00102", "Groceries", "Milk\nBread\nCoffee beans"));
    notes.add(StoredNote::new("VN-2026-00103", "Quarterly plan", "Goals for the quarter, with a checklist and the budget sheet attached").rich());
    notes.add(StoredNote::new("VN-2026-00104", "Room booking codes", "Large room: 4711\nSmall room: 0815").shared_by(
        "grace@example.org",
        "Grace Okafor",
        false,
    ));
    notes.add(StoredNote::new("VN-2026-00105", "Old checklist", "Renew the parking card").archived());
}

/// Add `count` short notes to the person's, for a look at what many notes cost a window.
pub fn crowd_notes(notes: &mut Notes, count: usize) {
    for n in 0..count {
        notes.add(StoredNote::new(&format!("VN-2026-C{n:05}"), &format!("Note {n}"), &format!("Words of note {n}")));
    }
}

#[cfg(test)]
mod tests;
