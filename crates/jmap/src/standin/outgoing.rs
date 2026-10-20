// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What the stand-in does for sending mail and filing drafts: the upload of a message (RFC 8620 §6.1),
//! `Email/import` (RFC 8621 §4.8), `Identity/get` (§6.1) and `EmailSubmission/set` (§7.5), held to the
//! standard as the rest of the stand-in is; what it files is destroyed by `Email/set` (`set.rs`).
//!
//! - **Upload.** `POST /jmap/upload/{accountId}/` with the credentials and a `Content-Type`; a body over
//!   `maxSizeUpload` is a 413. The answer is a 201 with the blob, its type and its size. What is uploaded
//!   waits in [`Data::uploads`] until an `Email/import` takes it.
//! - **Import.** One mailbox at least, that the stand-in has; keywords; an arrival time of its own. The blob
//!   must be an upload (`invalidProperties`, naming `blobId`, otherwise). The import takes the upload: it is
//!   gone afterwards.
//! - **Submission.** The identity must be one the stand-in lists, the email one it has; the envelope's
//!   `mailFrom` must be the identity's address (`forbiddenMailFrom`) and so must the message's `From`
//!   (`forbiddenFrom`); there must be recipients (`noRecipients`), not too many (`tooManyRecipients`), and each
//!   must be an address (`invalidRecipients`). Without an envelope it is made from the message's headers, as
//!   RFC 8621 §7 has it. A submission that goes is recorded in [`Data::delivered`] as it left, **without any
//!   Bcc field** ("the server MUST remove any Bcc header field present on the message during delivery",
//!   §7.5); the email is not copied anywhere.
//! - **Trouble on demand.** [`Data::faults`] holds one-off failures for a test to line up: a call that is
//!   refused, a creation refused with a SetError of the type a test names, a submission that goes and is
//!   then said to have failed, an answer that is lost after the call ran, a gateway that gives up with a
//!   status after it ran, an answer that is not JMAP.
//!
//! A server that is a layer in front of an IMAP and an SMTP server holds all this to more, see [`super::facade`].

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use super::{ACCOUNT, Data, Email, Outcome, UPLOAD_PATH, account, allow_only, header, invalid, problem, refuse};
use crate::{Error, Request, valid_id};

/// The first moment an email the stand-in files arrives; the next is a second later. After every email a test
/// has delivered with `Data::deliver` (those arrive in 2026, early), so that a window lists a message that was
/// sent or saved first.
const ARRIVALS_FROM: i64 = 1_796_083_200;
/// The most recipients a submission may name before the standard server refuses it.
pub(super) const MAX_RECIPIENTS: usize = 100;
/// The most octets of a message the server sends (`tooLarge` past it).
const MAX_SENT: usize = 25_000_000;
/// Most entries of a refusal's list of addresses.
const MAX_LISTED: usize = 100;

/// Who the account may send as (RFC 8621 §6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub email: String,
}

/// A message that was uploaded and not yet filed: the octets, and the type the client said they are.
#[derive(Clone, PartialEq, Eq)]
pub struct Upload {
    pub kind: String,
    pub octets: Vec<u8>,
}

/// What may be shown of an upload: not the message.
impl std::fmt::Debug for Upload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upload").field("kind", &self.kind).field("octets", &self.octets.len()).finish()
    }
}

/// A message the server sent: who it was sent from and to, as the envelope said, and the message as it left.
#[derive(Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The email that was submitted (its id on the server).
    pub email: String,
    pub from: String,
    pub recipients: Vec<String>,
    /// The octets that left, without any Bcc header field.
    pub raw: Vec<u8>,
}

/// What may be shown of a delivery: counts, not the message nor who it was for.
impl std::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Delivery").field("recipients", &self.recipients.len()).field("octets", &self.raw.len()).finish_non_exhaustive()
    }
}

/// What a fault does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The call is not run; its answer is a method error of this `type`.
    MethodError(&'static str),
    /// For a call that creates (`Email/import`, `EmailSubmission/set`): the creation is refused with this
    /// SetError `type` and `description`; nothing is made.
    SetError(&'static str, &'static str),
    /// For `EmailSubmission/set`: the server sends the message, and then says the creation failed with this
    /// SetError `type` and `description` (a server whose goodbye to the mail server it handed the message to
    /// failed, say). The message has left; the answer says it has not.
    SentThenSetError(&'static str, &'static str),
    /// The request is not run at all: the server answers with this HTTP status (a server that is busy, or a
    /// gateway that is down).
    RefuseWith(u16),
    /// The request runs, and its answer never comes: the connection is lost.
    LoseAnswer,
    /// The request runs, and the answer is this HTTP status instead (a gateway that gave up, a failure after
    /// the work was done).
    Status(u16),
    /// The request runs, and the answer is not JMAP at all.
    Garbage,
}

/// A failure lined up for the next call of the method `on`, or the next `upload`; it happens once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub on: &'static str,
    pub effect: Effect,
}

impl Fault {
    pub fn on(on: &'static str, effect: Effect) -> Fault {
        Fault { on, effect }
    }
}

/// A SetError a creation ends in.
pub(super) struct SetError {
    kind: &'static str,
    description: String,
    /// What else the SetError says (`properties`, `invalidRecipients`, `maxRecipients`).
    more: Map<String, Value>,
}

impl SetError {
    pub(super) fn new(kind: &'static str, description: impl Into<String>) -> SetError {
        SetError { kind, description: description.into(), more: Map::new() }
    }

    pub(super) fn with(mut self, key: &str, value: Value) -> SetError {
        self.more.insert(key.to_string(), value);
        self
    }

    fn json(self) -> Value {
        let mut out = self.more;
        out.insert("type".into(), json!(self.kind));
        out.insert("description".into(), json!(self.description));
        Value::Object(out)
    }
}

/// The text of the header fields named `name` in a message, each unfolded: one entry for each such field.
pub(super) fn header_fields(raw: &[u8], name: &str) -> Vec<String> {
    let text = String::from_utf8_lossy(raw);
    let mut found: Vec<String> = Vec::new();
    let mut on = false;
    for line in text.split('\n').map(|line| line.trim_end_matches('\r')) {
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if on && let Some(last) = found.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
            continue;
        }
        on = line.split_once(':').is_some_and(|(field, _)| field.eq_ignore_ascii_case(name));
        if on && let Some((_, value)) = line.split_once(':') {
            found.push(value.trim().to_string());
        }
    }
    found
}

/// A message with its `Bcc` header fields taken out (and their continuation lines), the rest byte for byte.
pub(super) fn without_bcc(raw: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let (mut in_head, mut skipping) = (true, false);
    for line in raw.split_inclusive(|b| *b == b'\n') {
        let content = line.strip_suffix(b"\n").map_or(line, |l| l.strip_suffix(b"\r").unwrap_or(l));
        if in_head {
            if content.is_empty() {
                in_head = false;
            } else if content.starts_with(b" ") || content.starts_with(b"\t") {
                if skipping {
                    continue;
                }
            } else {
                skipping = content.len() >= 4 && content.get(..4).is_some_and(|name| name.eq_ignore_ascii_case(b"bcc:"));
                if skipping {
                    continue;
                }
            }
        }
        out.extend_from_slice(line);
    }
    out
}

/// Whether `text` could be an address: one `@` with something on each side, and nothing a protocol would
/// stumble on.
pub(super) fn looks_like_address(text: &str) -> bool {
    let plain = !text.is_empty() && text.len() <= 254 && text.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'<' | b'>' | b',' | b';' | b'"'));
    plain && text.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty() && !domain.contains('@'))
}

impl Data {
    /// Add a person the account may send as: its id. (A server that is a layer in front of an IMAP server
    /// names the identity after the address, as it names everything.)
    pub fn identity(&mut self, name: &str, email: &str) -> String {
        let id = if self.profile.strict_submission { super::facade::identity_id(email) } else { self.id("I") };
        self.identities.push(Identity { id: id.clone(), name: name.into(), email: email.into() });
        self.changed();
        id
    }

    /// Take the first fault lined up for `on` that `fits`, if there is one.
    pub(super) fn take_fault(&mut self, on: &str, fits: impl Fn(&Effect) -> bool) -> Option<Effect> {
        let at = self.faults.iter().position(|fault| fault.on == on && fits(&fault.effect))?;
        self.faults.remove(at).map(|fault| fault.effect)
    }

    /// The first moment that no email of the stand-in has arrived at: the next arrival.
    fn arrival(&mut self) -> String {
        self.arrivals = self.arrivals.saturating_add(1);
        mailcraft_time::format_iso_utc(ARRIVALS_FROM.saturating_add(i64::from(self.arrivals)))
    }

    /// `POST /jmap/upload/{accountId}/` (RFC 8620 §6.1): the status and the body of the answer.
    pub(super) fn upload(&mut self, req: &Request, path: &str) -> (u16, Vec<u8>) {
        let refused = |status: u16, kind: &str, detail: &str| (status, serde_json::to_vec(&problem(status, kind, detail)).unwrap_or_default());
        if !self.authorized(&req.headers) {
            return refused(401, "about:blank", "the credentials were refused");
        }
        if req.method != "POST" {
            return refused(405, "about:blank", "an upload is a POST");
        }
        if let Some(Effect::RefuseWith(status)) = self.take_fault("upload", |effect| matches!(effect, Effect::RefuseWith(_))) {
            return refused(status, "about:blank", "the server can't take this upload just now");
        }
        // The effects on the answer: it is changed once the upload has been made.
        if let Some(effect) = self.take_fault("upload", |effect| matches!(effect, Effect::LoseAnswer | Effect::Status(_) | Effect::Garbage)) {
            self.after = Some(effect);
        }
        let account_in_path = path.strip_prefix(UPLOAD_PATH).and_then(|rest| rest.strip_suffix('/')).unwrap_or_default();
        if account_in_path != ACCOUNT {
            return refused(404, "about:blank", "no such account");
        }
        let Some(kind) = header(&req.headers, "content-type").map(str::trim).filter(|kind| !kind.is_empty()) else {
            return refused(400, "urn:ietf:params:jmap:error:notRequest", "an upload says what it is in its Content-Type");
        };
        if req.body.len() > self.max_upload {
            return refused(413, "urn:ietf:params:jmap:error:limit", "maxSizeUpload");
        }
        if self.profile.strict_submission {
            let kept: usize = self.uploads.values().map(|u| u.octets.len()).sum();
            if self.uploads.len() >= super::facade::MAX_UPLOADS || kept.saturating_add(req.body.len()) > super::facade::MAX_UPLOAD_BYTES {
                return refused(429, "urn:ietf:params:jmap:error:limit", "Too many messages waiting to be filed.");
            }
        }
        let blob = self.id("U");
        self.uploads.insert(blob.clone(), Upload { kind: kind.to_string(), octets: req.body.clone() });
        let body = json!({"accountId": ACCOUNT, "blobId": blob, "type": kind, "size": req.body.len()});
        (201, serde_json::to_vec(&body).unwrap_or_default())
    }

    /// `Email/import` (RFC 8621 §4.8).
    pub(super) fn email_import(&mut self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "ifInState", "emails"])?;
        account(args)?;
        let Some(Value::Object(emails)) = args.get("emails") else {
            return Err(invalid("emails is an object"));
        };
        if emails.len() > self.max_get {
            return Err(refuse("requestTooLarge", format!("{} emails, and {} are taken at a time", emails.len(), self.max_get)));
        }
        if self.profile.strict_submission && emails.len() > super::facade::MAX_IMPORTS {
            return Err(invalid("emails"));
        }
        let (mut created, mut not_created) = (Map::new(), Map::new());
        for (key, spec) in emails {
            let fault = self.take_fault("Email/import", |effect| matches!(effect, Effect::SetError(..)));
            let made = match fault {
                Some(Effect::SetError(kind, description)) => Err(SetError::new(kind, description)),
                _ => self.import_one(spec),
            };
            match made {
                Ok(made) => created.insert(key.clone(), made),
                Err(error) => not_created.insert(key.clone(), error.json()),
            };
        }
        let state = self.state();
        Ok(json!({"accountId": ACCOUNT, "oldState": state, "newState": state, "created": created, "notCreated": not_created}))
    }

    fn import_one(&mut self, spec: &Value) -> std::result::Result<Value, SetError> {
        let wrong =
            |property: &str| SetError::new("invalidProperties", format!("{property} is not as it should be")).with("properties", json!([property]));
        let Some(spec) = spec.as_object() else { return Err(wrong("emails")) };
        let blob = spec.get("blobId").and_then(Value::as_str).filter(|blob| valid_id(blob)).ok_or_else(|| wrong("blobId"))?;
        let mailboxes: BTreeSet<String> = match spec.get("mailboxIds") {
            Some(Value::Object(map)) => map.iter().filter(|(_, on)| **on == Value::Bool(true)).map(|(id, _)| id.clone()).collect(),
            _ => return Err(wrong("mailboxIds")),
        };
        if mailboxes.is_empty() || mailboxes.iter().any(|id| !self.mailboxes.iter().any(|m| &m.id == id)) {
            return Err(wrong("mailboxIds"));
        }
        if self.profile.one_mailbox && mailboxes.len() > 1 {
            return Err(SetError::new("invalidProperties", "A message goes into one folder.").with("properties", json!(["mailboxIds"])));
        }
        let mut keywords: BTreeSet<String> = match spec.get("keywords") {
            None | Some(Value::Null) => BTreeSet::new(),
            Some(Value::Object(map)) => map.iter().filter(|(_, on)| **on == Value::Bool(true)).map(|(name, _)| name.clone()).collect(),
            Some(_) => return Err(wrong("keywords")),
        };
        if self.profile.strict_submission {
            keywords.retain(|name| super::facade::KEYWORDS.contains(&name.to_ascii_lowercase().as_str()));
        }
        let received = match spec.get("receivedAt") {
            None | Some(Value::Null) => None,
            Some(Value::String(date)) if mailcraft_time::iso_to_timestamp(date, 0).is_some() => Some(date.clone()),
            Some(_) => return Err(wrong("receivedAt")),
        };
        let Some(upload) = self.uploads.remove(blob) else {
            return Err(if self.profile.strict_submission {
                SetError::new("blobNotFound", "The message to file was not found.")
            } else {
                wrong("blobId")
            });
        };
        let (id, blob) = (self.id("E"), self.id("B"));
        let size = upload.octets.len();
        let received = received.unwrap_or_else(|| self.arrival());
        self.emails.push(Email { id: id.clone(), blob: blob.clone(), mailboxes, keywords, received, raw: upload.octets });
        if self.profile.strict_submission {
            self.filed.insert(id.clone());
        }
        self.changed();
        Ok(json!({"id": id, "blobId": blob, "threadId": id, "size": size}))
    }

    /// `Identity/get` (RFC 8621 §6.1).
    pub(super) fn identity_get(&self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "ids", "properties"])?;
        account(args)?;
        let wanted = super::ids(args, self.max_get)?;
        let entry = |identity: &Identity| json!({"id": identity.id, "name": identity.name, "email": identity.email, "replyTo": null, "bcc": null, "textSignature": "", "htmlSignature": "", "mayDelete": false});
        // A server that is a layer in front of a mailbox has the one identity of the mailbox, whatever is asked.
        let (list, not_found): (Vec<Value>, Vec<String>) = match wanted {
            Some(wanted) if !self.profile.strict_submission => {
                let list = self.identities.iter().filter(|i| wanted.contains(&i.id)).map(entry).collect();
                let not_found = wanted.iter().filter(|id| !self.identities.iter().any(|i| &i.id == *id)).cloned().collect();
                (list, not_found)
            }
            _ => (self.identities.iter().map(entry).collect(), Vec::new()),
        };
        Ok(json!({"accountId": ACCOUNT, "state": self.state(), "list": list, "notFound": not_found}))
    }

    /// `EmailSubmission/set` (RFC 8621 §7.5), to create submissions.
    pub(super) fn submission_set(&mut self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "ifInState", "create", "update", "destroy", "onSuccessUpdateEmail", "onSuccessDestroyEmail"])?;
        account(args)?;
        for key in ["update", "destroy", "onSuccessUpdateEmail", "onSuccessDestroyEmail"] {
            if !matches!(args.get(key), None | Some(Value::Null)) {
                return Err(invalid(format!("this stand-in only creates submissions, and {key} is not that")));
            }
        }
        let create = match args.get("create") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(create)) => create.clone(),
            Some(_) => return Err(invalid("create is an object")),
        };
        if create.len() > self.max_get {
            return Err(refuse("requestTooLarge", format!("{} submissions, and {} are taken at a time", create.len(), self.max_get)));
        }
        if self.profile.strict_submission && create.len() > super::facade::MAX_SUBMISSIONS {
            return Err(invalid("create"));
        }
        let (mut created, mut not_created) = (Map::new(), Map::new());
        for (key, spec) in create {
            let fault = self.take_fault("EmailSubmission/set", |effect| matches!(effect, Effect::SetError(..) | Effect::SentThenSetError(..)));
            let made = match fault {
                Some(Effect::SetError(kind, description)) => Err(SetError::new(kind, description)),
                // The message goes (if the server would send it at all), and the answer is a failure all the same.
                Some(Effect::SentThenSetError(kind, description)) => self.submit_one(&spec).and(Err(SetError::new(kind, description))),
                _ => self.submit_one(&spec),
            };
            match made {
                Ok(made) => created.insert(key, made),
                Err(error) => not_created.insert(key, error.json()),
            };
        }
        let state = self.state();
        Ok(json!({"accountId": ACCOUNT, "oldState": state, "newState": state, "created": created, "notCreated": not_created}))
    }

    /// One submission: the standard's rules (RFC 8621 §7.5), or those of a server in front of a mailbox
    /// ([`super::facade::submit`]). The message that left is recorded, without any Bcc field.
    fn submit_one(&mut self, spec: &Value) -> std::result::Result<Value, SetError> {
        let wrong =
            |property: &str| SetError::new("invalidProperties", format!("{property} is not as it should be")).with("properties", json!([property]));
        let Some(spec) = spec.as_object() else { return Err(wrong("create")) };
        if self.profile.strict_submission {
            return super::facade::submit(self, spec);
        }
        let identity = spec
            .get("identityId")
            .and_then(Value::as_str)
            .and_then(|id| self.identities.iter().find(|i| i.id == id))
            .cloned()
            .ok_or_else(|| wrong("identityId"))?;
        let email_id = spec.get("emailId").and_then(Value::as_str).ok_or_else(|| wrong("emailId"))?;
        let email = self.emails.iter().find(|e| e.id == email_id).cloned().ok_or_else(|| wrong("emailId"))?;
        // The envelope: the one given, or the one the message's header makes (RFC 8621 §7).
        let (mail_from, recipients) = match spec.get("envelope").filter(|envelope| !envelope.is_null()) {
            Some(envelope) => {
                let mail_from = envelope.get("mailFrom").and_then(|m| m.get("email")).and_then(Value::as_str).unwrap_or_default().to_string();
                let recipients: Vec<String> = match envelope.get("rcptTo") {
                    Some(Value::Array(list)) => {
                        list.iter().map(|r| r.get("email").and_then(Value::as_str).unwrap_or_default().trim().to_string()).collect()
                    }
                    _ => Vec::new(),
                };
                (mail_from, recipients)
            }
            None => {
                let parsed = mailcraft_mime::parse(&email.raw);
                let mut recipients: Vec<String> = Vec::new();
                for address in parsed.to.iter().chain(&parsed.cc).chain(&parsed.bcc) {
                    if !recipients.iter().any(|known| known.eq_ignore_ascii_case(&address.email)) {
                        recipients.push(address.email.clone());
                    }
                }
                (parsed.from.email.clone(), recipients)
            }
        };
        if !mail_from.eq_ignore_ascii_case(&identity.email) {
            return Err(SetError::new("forbiddenMailFrom", "The envelope's sender is not this identity's."));
        }
        let from = mailcraft_mime::parse(&email.raw).from.email;
        if !from.eq_ignore_ascii_case(&identity.email) {
            return Err(SetError::new("forbiddenFrom", "The message is not from this identity."));
        }
        if recipients.is_empty() {
            return Err(SetError::new("noRecipients", "Add at least one recipient."));
        }
        if recipients.len() > MAX_RECIPIENTS {
            return Err(SetError::new("tooManyRecipients", "Too many recipients.").with("maxRecipients", json!(MAX_RECIPIENTS)));
        }
        let invalid_ones: Vec<&String> = recipients.iter().filter(|r| !looks_like_address(r)).take(MAX_LISTED).collect();
        if !invalid_ones.is_empty() {
            return Err(SetError::new("invalidRecipients", "An address is not one.").with("invalidRecipients", json!(invalid_ones)));
        }
        if email.raw.len() > MAX_SENT {
            return Err(SetError::new("tooLarge", "The message is larger than the server sends."));
        }
        Ok(self.send_out(&email, mail_from, recipients))
    }

    /// The server sends the email: what left is recorded, and the answer is the submission that was made.
    pub(super) fn send_out(&mut self, email: &Email, from: String, recipients: Vec<String>) -> Value {
        self.delivered.push(Delivery { email: email.id.clone(), from, recipients, raw: without_bcc(&email.raw) });
        self.changed();
        let id = self.id("S");
        json!({"id": id, "undoStatus": "final"})
    }
}

/// What the stand-in does with a fault that changes an answer once it has been made: the answer.
pub(super) fn answer_after(effect: Option<Effect>, status: u16, body: Vec<u8>) -> std::result::Result<(u16, Vec<u8>), Error> {
    match effect {
        Some(Effect::LoseAnswer) => Err(Error::Io("the connection was lost".into())),
        Some(Effect::Status(status)) => Ok((status, serde_json::to_vec(&problem(status, "about:blank", "the gateway gave up")).unwrap_or_default())),
        Some(Effect::Garbage) => Ok((200, b"<html><body>Bad gateway</body></html>".to_vec())),
        _ => Ok((status, body)),
    }
}

#[cfg(test)]
mod tests;
