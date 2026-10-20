// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! `Email/set` of the in-process server: emails updated and destroyed (RFC 8620 §5.3, RFC 8621 §4.6).
//!
//! Strict, as the rest of the stand-in is, so that a client that sends something a server would refuse is
//! caught here and not by a person's mail:
//!
//! - **The call.** `accountId`, `ifInState`, `create`, `update` and `destroy` and no other argument
//!   (`invalidArguments`); `ifInState` that is not the state is `stateMismatch`; no more objects in all than
//!   the session's `maxObjectsInSet` (`requestTooLarge`); an id is held to the alphabet of an id.
//! - **A patch** is an object whose keys are paths of the email without the leading `/` (RFC 6901, with `~0`
//!   and `~1`), none of them inside another (`invalidPatch`). Only `keywords` and `mailboxIds` can be
//!   changed (`invalidProperties` for an `id`, a `size`… or a property that is none), by the whole
//!   property or by one entry of it (`keywords/$seen`, `mailboxIds/M1`), the value `true` or `null` to take it
//!   away. A server that follows the standard takes either form; a layer over a mailbox
//!   ([`Profile::plain_patches`]) takes the entries alone.
//! - **What it makes of the email.** A keyword is 1 to 255 characters of ASCII but ` ( ) { ] % * " \`, and
//!   is kept in lower case; no more than 100 (`tooManyKeywords`); the email is in at least one mailbox that
//!   exists (`invalidProperties`), and in no more than the account allows (`tooManyMailboxes`). On a server
//!   that names an email by where it lies ([`Profile::one_mailbox`]) an email that is moved is another id
//!   and another blob; the answer says which id when [`Profile::says_moved_id`].
//! - **Each object has its own answer**: `updated` (with `null`, or the properties the server set that the client
//!   does not know) or `notUpdated`, `destroyed` or `notDestroyed`, with a SetError (`notFound` for an email that
//!   is no more, unless the server does not look: [`Profile::unchecked_ids`]; `willDestroy` for one that is
//!   destroyed in the same call, `tooLarge` for a patch of more than 64 entries, `forbidden`… for an email a test
//!   had it refuse: [`Data::refused_sets`], or its moves alone: [`Data::refused_moves`]). `create` is refused: emails
//!   are imported, not created.
//!
//! The state changes with every change to an email, whoever makes it, as everything the stand-in serves.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use super::{ACCOUNT, Data, Mailbox, Outcome, Profile, account, allow_only, invalid, refuse};
use crate::valid_id;

/// Longest keyword (RFC 8621 §4.1.1), most keywords an email carries, most entries in a patch and most
/// parts of a path.
const MAX_KEYWORD: usize = 255;
const MAX_KEYWORDS: usize = 100;
const MAX_PATCH: usize = 64;
const MAX_PARTS: usize = 8;
const MAX_PATH: usize = 1_024;
/// The properties of an email that are its own and are not changed by a client.
const NOT_CHANGED: &[&str] =
    &["id", "blobId", "threadId", "size", "receivedAt", "preview", "hasAttachment", "headers", "subject", "from", "to", "cc", "messageId", "sentAt"];

/// A SetError (RFC 8620 §5.3): the `type` and a word of why.
struct SetError {
    kind: &'static str,
    description: String,
}

fn error(kind: &'static str, description: impl Into<String>) -> SetError {
    SetError { kind, description: description.into() }
}

impl SetError {
    fn json(&self) -> Value {
        json!({"type": self.kind, "description": self.description})
    }
}

/// What a patch makes of an email.
struct Patched {
    keywords: BTreeSet<String>,
    mailboxes: BTreeSet<String>,
}

impl Data {
    /// `Email/set`: the emails updated and destroyed, each answered by itself.
    pub(super) fn email_set(&mut self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "ifInState", "create", "update", "destroy"])?;
        account(args)?;
        let before = self.state();
        match args.get("ifInState") {
            None | Some(Value::Null) => {}
            Some(Value::String(state)) if *state == before => {}
            Some(Value::String(_)) => return Err(refuse("stateMismatch", "the state is not the one the call expected")),
            Some(_) => return Err(invalid("ifInState is a string")),
        }
        let none = Map::new();
        let create = match args.get("create") {
            None | Some(Value::Null) => &none,
            Some(Value::Object(members)) => members,
            Some(_) => return Err(invalid("create is an object")),
        };
        let update = match args.get("update") {
            None | Some(Value::Null) => &none,
            Some(Value::Object(members)) => members,
            Some(_) => return Err(invalid("update is an object")),
        };
        let destroy: Vec<&str> = match args.get("destroy") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => {
                let ids = items.iter().map(|item| item.as_str().filter(|id| valid_id(id))).collect::<Option<Vec<&str>>>();
                ids.ok_or_else(|| invalid("an id is 1 to 512 characters of A-Z a-z 0-9 - _"))?
            }
            Some(_) => return Err(invalid("destroy is a list")),
        };
        if update.keys().any(|id| !valid_id(id)) {
            return Err(invalid("an id is 1 to 512 characters of A-Z a-z 0-9 - _"));
        }
        let objects = create.len().saturating_add(update.len()).saturating_add(destroy.len());
        if objects > self.max_set {
            return Err(refuse("requestTooLarge", format!("{objects} objects, and {} are taken at a time", self.max_set)));
        }
        // A layer over a mailbox has no way to make an email from a description of one.
        if self.profile.plain_patches && !create.is_empty() {
            return Err(invalid("messages are imported, not created"));
        }
        let not_created: Map<String, Value> =
            create.keys().map(|cid| (cid.clone(), error("forbidden", "emails are imported, not created").json())).collect();

        let destroying: BTreeSet<&str> = destroy.iter().copied().collect();
        let (mut updated, mut not_updated) = (Map::new(), Map::new());
        for (id, patch) in update {
            let outcome = if destroying.contains(id.as_str()) {
                Err(error("willDestroy", "the email is destroyed in this call"))
            } else {
                self.update_one(id, patch)
            };
            match outcome {
                Ok(said) => updated.insert(id.clone(), said),
                Err(e) => not_updated.insert(id.clone(), e.json()),
            };
        }
        let (mut destroyed, mut not_destroyed) = (Vec::new(), Map::new());
        for id in destroying {
            match self.destroy_one(id) {
                Ok(()) => destroyed.push(Value::from(id)),
                Err(e) => {
                    not_destroyed.insert(id.to_string(), e.json());
                }
            }
        }
        let after = self.state();
        let plain = self.profile.plain_patches;
        // (A server of the standard kind says `null` for what is empty; the other always says all.)
        let map_or_null = |map: Map<String, Value>| if map.is_empty() && !plain { Value::Null } else { Value::Object(map) };
        let list_or_null = if destroyed.is_empty() && !plain { Value::Null } else { Value::Array(destroyed) };
        let mut out = json!({
            "accountId": ACCOUNT,
            "oldState": before,
            "newState": after,
            "updated": map_or_null(updated),
            "destroyed": list_or_null,
            "notUpdated": map_or_null(not_updated),
            "notDestroyed": map_or_null(not_destroyed),
        });
        if let (false, Some(members)) = (plain, out.as_object_mut()) {
            members.insert("created".into(), Value::Null);
            members.insert("notCreated".into(), map_or_null(not_created));
        }
        Ok(out)
    }

    /// One email's patch, and what the server says of it: `null`, or the id it gave a moved email.
    fn update_one(&mut self, id: &str, patch: &Value) -> Result<Value, SetError> {
        if let Some(kind) = self.refused_sets.get(id) {
            return Err(error(kind, "the server will not change this email just now"));
        }
        if let Some(kind) = self.refused_moves.get(id)
            && patch.as_object().is_some_and(|entries| entries.keys().any(|path| path == "mailboxIds" || path.starts_with("mailboxIds/")))
        {
            return Err(error(kind, "the server will not move this email just now"));
        }
        let Some(at) = self.emails.iter().position(|e| e.id == id) else {
            // A server that does not look is told of a message that is no more, and says it was made.
            return if self.profile.unchecked_ids { Ok(Value::Null) } else { Err(error("notFound", "no such email")) };
        };
        let Value::Object(patch) = patch else {
            return Err(error("invalidPatch", "a patch is an object"));
        };
        let patched = match self.emails.get(at) {
            Some(email) => patched(&self.profile, &self.mailboxes, &email.keywords, &email.mailboxes, patch)?,
            None => return Err(error("notFound", "no such email")),
        };
        let moved = self.emails.get(at).is_some_and(|email| email.mailboxes != patched.mailboxes);
        let changed = self.emails.get(at).is_some_and(|email| email.mailboxes != patched.mailboxes || email.keywords != patched.keywords);
        // A server that names an email by where it lies gives it, and its blob, a name for its new place.
        let fresh = (moved && self.profile.one_mailbox).then(|| (self.id("B"), self.id("E")));
        let Some(email) = self.emails.get_mut(at) else {
            return Err(error("notFound", "no such email"));
        };
        email.keywords = patched.keywords;
        email.mailboxes = patched.mailboxes;
        let said = match fresh {
            Some((blob, new)) => {
                (email.blob, email.id) = (blob, new.clone());
                if self.profile.says_moved_id { json!({"id": new}) } else { Value::Null }
            }
            None => Value::Null,
        };
        if changed {
            self.changed();
        }
        Ok(said)
    }

    fn destroy_one(&mut self, id: &str) -> Result<(), SetError> {
        if let Some(kind) = self.refused_sets.get(id) {
            return Err(error(kind, "the server will not destroy this email just now"));
        }
        if !self.remove(id) && !self.profile.unchecked_ids {
            return Err(error("notFound", "no such email"));
        }
        Ok(())
    }
}

/// What `patch` makes of an email that has these keywords and lies in these mailboxes, or why it is no patch
/// of it.
fn patched(
    profile: &Profile,
    known: &[Mailbox],
    keywords: &BTreeSet<String>,
    mailboxes: &BTreeSet<String>,
    patch: &Map<String, Value>,
) -> Result<Patched, SetError> {
    if patch.len() > MAX_PATCH {
        return Err(error("tooLarge", format!("a patch of more than {MAX_PATCH} entries")));
    }
    let mut paths: Vec<(Vec<String>, &Value)> = Vec::new();
    for (path, value) in patch {
        paths.push((parts(path).ok_or_else(|| error("invalidPatch", "not a path of the email"))?, value));
    }
    // No path is inside another, and none is given twice.
    for (a, (inner, _)) in paths.iter().enumerate() {
        if paths.iter().enumerate().any(|(b, (outer, _))| a != b && inner.starts_with(outer)) {
            return Err(error("invalidPatch", "a path is inside another"));
        }
    }
    let (mut keywords, mut mailboxes) = (keywords.clone(), mailboxes.clone());
    for (parts, value) in &paths {
        match parts.as_slice() {
            [property] if property == "keywords" && !profile.plain_patches => keywords = keyword_set(value)?,
            [property] if property == "mailboxIds" && !profile.plain_patches => mailboxes = mailbox_set(value)?,
            [property, name] if property == "keywords" => {
                let name = keyword(name)?;
                match value {
                    Value::Bool(true) => {
                        keywords.insert(name);
                    }
                    Value::Null => keywords.retain(|k| *k != name),
                    _ => return Err(error("invalidProperties", "a keyword is true, or null to take it away")),
                }
            }
            [property, id] if property == "mailboxIds" => match value {
                Value::Bool(true) => {
                    mailboxes.insert(id.clone());
                }
                Value::Null => {
                    mailboxes.remove(id);
                }
                _ => return Err(error("invalidProperties", "a mailbox is true, or null to take it away")),
            },
            // The whole of a property on a server that takes the entries alone, or a path below a boolean.
            [property, ..] if property == "keywords" || property == "mailboxIds" => {
                return Err(error("invalidPatch", "a keyword or a mailbox is set or taken away by itself"));
            }
            [property, ..] if NOT_CHANGED.contains(&property.as_str()) => {
                return Err(error("invalidProperties", "this property of an email is the server's"));
            }
            _ => return Err(error("invalidProperties", "no such property")),
        }
    }
    if keywords.len() > MAX_KEYWORDS {
        return Err(error("tooManyKeywords", "the email has more keywords than the server keeps"));
    }
    if mailboxes.is_empty() {
        return Err(error("invalidProperties", "an email is in at least one mailbox"));
    }
    if mailboxes.iter().any(|id| !known.iter().any(|mailbox| mailbox.id == *id)) {
        return Err(error("invalidProperties", "no such mailbox"));
    }
    if profile.one_mailbox && mailboxes.len() > 1 {
        return Err(error("tooManyMailboxes", "an email is in one mailbox only"));
    }
    Ok(Patched { keywords, mailboxes })
}

/// The parts of a path of the email (RFC 6901 without the first `/`), or `None` for a path that is none.
fn parts(path: &str) -> Option<Vec<String>> {
    if path.len() > MAX_PATH {
        return None;
    }
    let mut out = Vec::new();
    for part in path.split('/') {
        let mut text = String::new();
        let mut chars = part.chars();
        while let Some(c) = chars.next() {
            if c != '~' {
                text.push(c);
                continue;
            }
            match chars.next() {
                Some('0') => text.push('~'),
                Some('1') => text.push('/'),
                _ => return None,
            }
        }
        if text.is_empty() || out.len() >= MAX_PARTS {
            return None;
        }
        out.push(text);
    }
    Some(out)
}

/// A keyword as RFC 8621 §4.1.1 has it, in lower case.
fn keyword(name: &str) -> Result<String, SetError> {
    let allowed = |c: char| c.is_ascii_graphic() && !matches!(c, '(' | ')' | '{' | ']' | '%' | '*' | '"' | '\\');
    if name.is_empty() || name.len() > MAX_KEYWORD || !name.chars().all(allowed) {
        return Err(error("invalidProperties", "not a keyword"));
    }
    Ok(name.to_ascii_lowercase())
}

/// The whole of `keywords`: an object whose values are all `true` (`null`: none).
fn keyword_set(value: &Value) -> Result<BTreeSet<String>, SetError> {
    match value {
        Value::Null => Ok(BTreeSet::new()),
        Value::Object(members) if members.values().all(|on| *on == Value::Bool(true)) => members.keys().map(|name| keyword(name)).collect(),
        _ => Err(error("invalidProperties", "keywords are an object whose values are true")),
    }
}

/// The whole of `mailboxIds`, in the same form.
fn mailbox_set(value: &Value) -> Result<BTreeSet<String>, SetError> {
    match value {
        Value::Null => Ok(BTreeSet::new()),
        Value::Object(members) if members.values().all(|on| *on == Value::Bool(true)) => Ok(members.keys().cloned().collect()),
        _ => Err(error("invalidProperties", "mailboxIds are an object whose values are true")),
    }
}

#[cfg(test)]
mod tests;
