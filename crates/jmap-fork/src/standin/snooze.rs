// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! « Plus tard » as the Neoffice server keeps it (frappe_webmail's `neomail_snooze`), when a test turns it on
//! ([`Snoozes::on`]): the capability [`SNOOZE`], `NeomailSnooze/get` and `NeomailSnooze/set`, a « Plus tard » mailbox made
//! the first time something is put away, and the return of what is due ([`super::Data::snoozes_due`]), which the server's
//! minute job makes and the loopback makes at each request and at `GET /__tick`.
//!
//! What it holds to, as the server does: a create moves the email into « Plus tard » and keeps a record (its new id, its
//! Message-ID, the mailbox it came from, when it comes back); a time not to come is refused (`invalidProperties`); the
//! same email put away twice is one record; `update` with `{"state": "returned"}` brings it back now, as the job does at
//! its time: to the mailbox it came from (the Inbox when that one is gone), unread; an email no longer in « Plus tard »
//! is not brought back (the record is called off). Only waiting and failed records are listed.

use serde_json::{Map, Value, json};

use super::{Data, Outcome};

/// The capability, as the Neoffice server names it.
pub const SNOOZE: &str = "https://neoffice.me/jmap/neomail-snooze";
/// The mailbox the server puts mail away in.
pub const FOLDER: &str = "Plus tard";

/// One record of the server's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: String,
    pub email: String,
    pub message_id: String,
    pub from: String,
    /// A UTC date as `2026-10-15T06:00:00Z`.
    pub until: String,
    /// `waiting`, `returned`, `calledOff` or `failed`.
    pub state: String,
}

/// What it keeps of « Plus tard ».
#[derive(Clone, Debug, Default)]
pub struct Snoozes {
    /// The session offers it.
    pub on: bool,
    pub records: Vec<Record>,
    next: u32,
}

impl Snoozes {
    pub(super) fn has(&self, name: &str) -> bool {
        self.on && matches!(name, "NeomailSnooze/get" | "NeomailSnooze/set")
    }

    pub(super) fn add_to(&self, session: &mut Value, account: &str) {
        if self.on {
            session["capabilities"][SNOOZE] = json!({});
            session["accounts"][account]["accountCapabilities"][SNOOZE] = json!({"maxAheadDays": 1830, "maxTries": 3});
            session["primaryAccounts"][SNOOZE] = json!(account);
        }
    }
}

/// The Message-ID of a message, without its angle brackets, in lower case.
fn message_id(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap_or_default();
    head.lines()
        .find_map(|line| line.split_once(':').filter(|(name, _)| name.eq_ignore_ascii_case("message-id")).map(|(_, value)| value.trim().to_string()))
        .map(|id| id.trim_start_matches('<').trim_end_matches('>').to_ascii_lowercase())
        .unwrap_or_default()
}

impl Data {
    fn record_json(&self, r: &Record) -> Value {
        let mailbox = self.mailbox_named(FOLDER).map(str::to_string);
        json!({"id": r.id, "emailId": r.email, "messageId": r.message_id, "mailboxId": mailbox, "fromMailboxId": r.from, "until": r.until, "state": r.state})
    }

    pub(super) fn snooze_method(&mut self, name: &str, args: &Map<String, Value>) -> Outcome<Value> {
        self.snoozes_due(mailcraft_time::now());
        match name {
            "NeomailSnooze/get" => {
                let list: Vec<Value> =
                    self.snoozes.records.iter().filter(|r| r.state == "waiting" || r.state == "failed").map(|r| self.record_json(r)).collect();
                Ok(json!({"accountId": super::ACCOUNT, "state": self.state(), "list": list, "notFound": [], "mailboxId": self.mailbox_named(FOLDER)}))
            }
            _ => self.snooze_set(args),
        }
    }

    fn snooze_set(&mut self, args: &Map<String, Value>) -> Outcome<Value> {
        let mut created = Map::new();
        let mut not_created = Map::new();
        for (cid, spec) in args.get("create").and_then(Value::as_object).cloned().unwrap_or_default() {
            match self.put_away(&spec) {
                Ok(done) => created.insert(cid, done),
                Err(why) => not_created.insert(cid, why),
            };
        }
        let mut updated = Map::new();
        let mut not_updated = Map::new();
        for (record, patch) in args.get("update").and_then(Value::as_object).cloned().unwrap_or_default() {
            if patch.get("state").and_then(Value::as_str) != Some("returned") {
                not_updated.insert(record, json!({"type": "invalidPatch"}));
                continue;
            }
            match self.bring_back(&record) {
                Some(done) => updated.insert(record, done),
                None => not_updated.insert(record, json!({"type": "notFound"})),
            };
        }
        let state = self.state();
        Ok(
            json!({"accountId": super::ACCOUNT, "oldState": state, "newState": state, "created": created, "notCreated": not_created, "updated": updated, "notUpdated": not_updated}),
        )
    }

    fn put_away(&mut self, spec: &Value) -> Result<Value, Value> {
        let until = spec.get("until").and_then(Value::as_str).unwrap_or_default().to_string();
        match mailcraft_time::iso_to_timestamp(&until, 0) {
            Some(at) if at > mailcraft_time::now() => {}
            _ => return Err(json!({"type": "invalidProperties", "description": "Choose a time in the future."})),
        }
        let email = spec.get("emailId").and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(found) = self.email(&email).cloned() else {
            return Err(json!({"type": "notFound", "description": "This message is no longer in its folder."}));
        };
        let folder = match self.mailbox_named(FOLDER) {
            Some(id) => id.to_string(),
            None => self.mailbox(FOLDER, None, None),
        };
        let mid = message_id(&found.raw);
        if let Some(r) =
            self.snoozes.records.iter_mut().find(|r| r.state == "waiting" && (r.email == email || (!mid.is_empty() && r.message_id == mid)))
        {
            r.until = until;
            let r = r.clone();
            return Ok(json!({"id": r.id, "emailId": r.email, "mailboxId": folder, "state": "waiting"}));
        }
        let from = found.mailboxes.iter().next().cloned().unwrap_or_default();
        let moved = self.move_to(&email, &folder).ok_or_else(|| json!({"type": "serverFail"}))?;
        self.snoozes.next += 1;
        let record = Record { id: format!("R{}", self.snoozes.next), email: moved.clone(), message_id: mid, from, until, state: "waiting".into() };
        self.snoozes.records.push(record.clone());
        Ok(json!({"id": record.id, "emailId": moved, "mailboxId": folder, "state": "waiting"}))
    }

    /// Bring the email of `record` back now: its new place, or `None` when there is no such record waiting.
    fn bring_back(&mut self, record: &str) -> Option<Value> {
        let at = self.snoozes.records.iter().position(|r| r.id == record && (r.state == "waiting" || r.state == "failed"))?;
        let r = self.snoozes.records[at].clone();
        let folder = self.mailbox_named(FOLDER)?.to_string();
        // Where it lies now: under its id, or by its Message-ID once another client moved it within the folder.
        let here = self.email(&r.email).filter(|e| e.mailboxes.contains(&folder)).map(|e| e.id.clone()).or_else(|| {
            self.emails
                .iter()
                .find(|e| e.mailboxes.contains(&folder) && !r.message_id.is_empty() && message_id(&e.raw) == r.message_id)
                .map(|e| e.id.clone())
        });
        let Some(here) = here else {
            self.snoozes.records[at].state = "calledOff".into();
            return None;
        };
        let to = if self.mailboxes.iter().any(|m| m.id == r.from) && r.from != folder {
            r.from.clone()
        } else {
            self.mailboxes.iter().find(|m| m.role.as_deref() == Some("inbox")).map(|m| m.id.clone())?
        };
        let moved = self.move_to(&here, &to)?;
        self.set_keyword(&moved, "$seen", false);
        self.snoozes.records[at].state = "returned".into();
        Some(json!({"state": "returned", "emailId": moved, "mailboxId": to}))
    }

    /// The server's minute job: every waiting record whose time is `now` or past comes back. How many came back.
    pub fn snoozes_due(&mut self, now: i64) -> usize {
        if !self.snoozes.on {
            return 0;
        }
        let due: Vec<String> = self
            .snoozes
            .records
            .iter()
            .filter(|r| r.state == "waiting" && mailcraft_time::iso_to_timestamp(&r.until, 0).is_some_and(|at| at <= now))
            .map(|r| r.id.clone())
            .collect();
        due.iter().filter(|id| self.bring_back(id).is_some()).count()
    }
}

#[cfg(test)]
#[path = "snooze_tests.rs"]
mod tests;
