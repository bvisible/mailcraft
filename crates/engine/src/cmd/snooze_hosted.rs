// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Plus tard » (snooze) in a hosting page's tab, kept by its server.

//! « Plus tard » in a tab of a hosting page ([`Session::keeps_local`] false): the mailbox's server puts a message away and
//! brings it back at its time, even with every window closed (the Neoffice server's `NeomailSnooze`, frappe_webmail's
//! `neomail_snooze`). The desktop keeps snoozes itself (`snooze.rs`, the model's `mailbox_snooze.rs`), and refuses a JMAP
//! server's mail: that stays as it is there.
//!
//! - `mail.snooze` of a message of an account whose server keeps snoozes ([`Hosted::servers`]) asks that server
//!   ([`Ask`], taken by the host): the message lies at once in the account's Snoozed folder here when it is known, marked
//!   [`Snooze::placing`] so that no move of it goes up (the server makes it), and the server's answer
//!   (`mail.snoozeAnswered`) says where it is now, or puts it back where it was when the server refused.
//! - `mail.unsnooze` of a message the server keeps a record of brings it back the same way, at once, unread.
//! - `mail.setSnoozes` puts what the server keeps (the host reads it now and then): which folder is the Snoozed one, and
//!   the time each message in it comes back (or that it could not), on the messages there.
//!
//! Neither is a step of the history ([`Session::unrecorded`]): the server keeps what was done, and an undo here would put
//! the message back while the server keeps it away. Nothing here does I/O.

use std::collections::{HashMap, HashSet};

use mailcraft_model::time::Timestamp;
use mailcraft_model::{FolderKind, Id, Message, Snooze};
use serde_json::{Value, json};

use super::jmap_down::plausible_id;
use super::{bad, fail, select_after_removal};
use crate::list::Row;
use crate::{CmdResult, CommandSpec, Session, p};

/// The most records one `mail.setSnoozes` takes: what the server answers at most.
pub const MAX_RECORDS: usize = 500;

/// What a hosting page's tab knows of its servers' « Plus tard », and what it is to ask them. Never saved.
#[derive(Clone, Debug, Default)]
pub struct Hosted {
    /// The accounts whose server keeps snoozes, as the host says them (`mail.setSnoozes`).
    pub servers: HashSet<Id>,
    /// The record the server keeps of each message waiting here, by message.
    pub records: HashMap<Id, Record>,
    /// What is to be asked of the servers, and what was asked and has no answer yet, oldest first.
    pub asks: Vec<Ask>,
    /// What the person is to be told of answers that refused, for the host to say.
    pub said: Vec<String>,
    /// The accounts to read again at once: a message went to a folder this tab has not listed yet.
    pub to_read: HashSet<Id>,
    /// The « Plus tard » folder each account's server named in an answer while this tab didn't list it yet (it made it then,
    /// at the first message put away): its server id, taken as the account's Snoozed folder as soon as a run lists it
    /// ([`adopt`]), not at the next reading of the records.
    pub named: HashMap<Id, String>,
}

/// The server's record of a message put away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: String,
    /// Its server could not bring it back: what the person reads of why.
    pub failed: Option<String>,
}

/// One ask of a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub account: Id,
    pub message: Id,
    pub what: Asked,
    /// Taken by the host: its answer is awaited.
    pub sent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// Put the message away until `until` (seconds, UTC): it is `email_id` on the server, with this `Message-ID`. `from` is
    /// the folder it lay in here, `placed` whether it was put in the Snoozed folder here meanwhile, and `was` the snooze it
    /// had (one put away again from the folder), which a refusal puts back.
    Put { email_id: String, message_id: String, until: Timestamp, from: Id, placed: bool, was: Option<Snooze> },
    /// Bring it back now: its record there. It was put in `to` here meanwhile, from the Snoozed folder, where it waited
    /// until `until`, having come from `from`.
    Return { record: String, to: Id, until: Timestamp, from: Id },
}

impl Hosted {
    /// The asks not taken yet, now taken (the host sends them).
    pub fn take(&mut self) -> Vec<Ask> {
        let mut out = Vec::new();
        for ask in self.asks.iter_mut().filter(|a| !a.sent) {
            ask.sent = true;
            out.push(ask.clone());
        }
        out
    }

    /// Whether anything is asked of a server or waits to be.
    pub fn asking(&self) -> bool {
        !self.asks.is_empty()
    }

    fn waits(&self, message: Id) -> bool {
        self.asks.iter().any(|a| a.message == message)
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("mail.setSnoozes", "Set Snoozes", "Folder › Actions", set_snoozes)
            .params(
                r#"{"account": string (the mailbox's address), "offered"?: false (its server does not keep snoozed mail: what this tab shows of it goes), "mailbox": string | null (the Snoozed folder's id on the server), "records": [{"id": string, "emailId"?: string, "messageId"?: string, "fromMailboxId"?: string, "until": number (seconds since 1970, UTC), "failed"?: string (why it did not come back)}]} — what the mailbox's server keeps of its snoozed mail, as the hosting page reads it: the messages waiting in its Snoozed folder, and when each comes back. Answers `changed`, `unmatched` (records of messages this window doesn't show yet) and `folder` (whether it shows the Snoozed folder)"#,
            )
            .pure(),
        CommandSpec::new("mail.snoozeAnswered", "Snooze Answered", "Folder › Actions", answered)
            .params(
                r#"{"message": number, "ok": boolean, "record"?: string, "emailId"?: string, "mailboxId"?: string, "error"?: string} — what a mailbox's server answered when it was asked to put a message away or bring it back (the hosting page's)"#,
            )
            .pure(),
    ]
}

/// A `Message-ID` as it is compared: without blanks and angle brackets, in lower case.
pub fn norm(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').trim().to_ascii_lowercase()
}

/// Whether the snooze of `ids` is the server's in this tab: a hosting page's tab, and a message of an account whose server
/// keeps snoozes among them.
fn hosted(s: &Session, ids: &[Id]) -> bool {
    !s.keeps_local && ids.iter().any(|id| s.mailbox.message(*id).is_some_and(|m| s.hosted_snooze.servers.contains(&m.account)))
}

/// Why a message can't be put away by its server, or none.
fn refused(s: &Session, m: &Message) -> Option<&'static str> {
    if !s.hosted_snooze.servers.contains(&m.account) {
        return Some("this mailbox's server doesn't keep snoozed mail");
    }
    if m.draft || s.mailbox.is_send_queue(m.folder) {
        return Some("a message being written or waiting to be sent can't be snoozed");
    }
    if !m.server.as_ref().is_some_and(|server| !server.gone && plausible_id(&server.id)) || norm(&m.message_id).is_empty() {
        return Some("this message isn't on its server yet: try again in a moment");
    }
    if s.hosted_snooze.waits(m.id) {
        return Some("this message is being put away or brought back: try again in a moment");
    }
    // A move made here that has not reached the server yet: the server would take it from where it was, and bring it back
    // there.
    let here = s.mailbox.folder(m.folder).and_then(|f| f.server.as_ref()).map(|server| server.id.as_str());
    let there = m.server.as_ref().map(|server| server.folder.as_str());
    if here != there {
        return Some("this message is being moved: try again in a moment");
    }
    None
}

/// Why `mail.snooze` can't run on `ids` in a hosting page's tab (`Some(None)`: it can), or `None` when the desktop's own
/// snooze is the one to ask.
pub fn refusal(s: &Session, ids: &[Id]) -> Option<Option<&'static str>> {
    if !hosted(s, ids) {
        return None;
    }
    Some(ids.iter().find_map(|id| s.mailbox.message(*id).and_then(|m| refused(s, m))))
}

/// Whether « Plus tard » is offered for `ids` here: a hosting page's tab whose server keeps them, each message one it may
/// put away.
pub fn offered(s: &Session, ids: &[Id]) -> bool {
    !ids.is_empty() && refusal(s, ids) == Some(None)
}

/// Whether « Plus tard » is the server's in this tab for some mailbox: the window offers it.
pub fn available(s: &Session) -> bool {
    !s.keeps_local && !s.hosted_snooze.servers.is_empty()
}

/// The server's record of a message waiting here, when it keeps one.
pub fn record(s: &Session, message: Id) -> Option<&Record> {
    s.hosted_snooze.records.get(&message)
}

/// `mail.snooze` in a hosting page's tab: the server is asked, and the messages lie in the Snoozed folder here at once
/// where it is known. `None` when the desktop's own snooze is the one to run.
pub(crate) fn snooze(s: &mut Session, until: Timestamp, ids: &[Id], before: &[Row]) -> Option<CmdResult> {
    if !hosted(s, ids) {
        return None;
    }
    let mut seen = HashSet::new();
    let ids: Vec<Id> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
    // Every message is looked at before anything is changed.
    for id in &ids {
        let m = s.mailbox.message(*id)?;
        if let Some(why) = refused(s, m) {
            return Some(Err(fail(why)));
        }
    }
    let mut asks = Vec::with_capacity(ids.len());
    let mut moved = Vec::new();
    for id in &ids {
        let Some(m) = s.mailbox.message(*id) else { continue };
        let (account, was_in, kept) = (m.account, m.folder, m.snooze);
        let email_id = m.server.as_ref().map(|server| server.id.clone()).unwrap_or_default();
        let message_id = m.message_id.clone();
        let snoozed = s.mailbox.snoozed_folder(account);
        let placed = snoozed.is_some_and(|f| f != was_in);
        // Already waiting there: only its time changes, and where it came from stays.
        let from = match kept {
            Some(k) if Some(was_in) == snoozed && k.waiting() => k.from,
            _ => was_in,
        };
        if let Ok(m) = s.mailbox.message_mut(*id)
            && let Some(folder) = snoozed
        {
            m.folder = folder;
            m.snooze = Some(Snooze { until, from, placing: placed, after_id: Id::MAX, ..Default::default() });
        }
        if placed {
            moved.push(*id);
        }
        asks.push(Ask { account, message: *id, what: Asked::Put { email_id, message_id, until, from, placed, was: kept }, sent: false });
    }
    select_after_removal(s, before, &moved);
    let count = asks.len();
    s.hosted_snooze.asks.extend(asks);
    s.unrecorded = true;
    Some(Ok(json!({"snoozed": count, "until": until, "server": true})))
}

/// `mail.unsnooze` in a hosting page's tab: the messages the server keeps a record of come back at once here, unread, to
/// the folder they came from (the Inbox when it is gone), and the server is asked to bring them back. `None` when none
/// is such a message: the desktop's own snooze answers.
pub(crate) fn unsnooze(s: &mut Session, ids: &[Id], before: &[Row]) -> Option<CmdResult> {
    if s.keeps_local {
        return None;
    }
    let now = s.now();
    let mut back = Vec::new();
    let mut asks = Vec::new();
    for id in ids {
        if back.contains(id) || s.hosted_snooze.waits(*id) {
            continue;
        }
        let Some(record) = s.hosted_snooze.records.get(id).map(|r| r.id.clone()) else { continue };
        let Some(m) = s.mailbox.message(*id) else { continue };
        let account = m.account;
        let Some(snooze) = m.snooze.filter(|z| z.waiting() && s.mailbox.snoozed_folder(account) == Some(m.folder)) else { continue };
        let to = s
            .mailbox
            .folder(snooze.from)
            .filter(|f| f.account == account && f.kind != FolderKind::Deleted && Some(f.id) != s.mailbox.snoozed_folder(account))
            .map(|f| f.id)
            .or_else(|| s.mailbox.folder_of_kind(account, FolderKind::Inbox));
        let Some(to) = to else { continue };
        if let Ok(m) = s.mailbox.message_mut(*id) {
            m.folder = to;
            m.read = false;
            // As the server makes it: unread there too, so no change of its marks goes up meanwhile.
            if let Some(server) = m.server.as_mut() {
                server.read = false;
            }
            m.snooze = Some(Snooze { returned: Some(now), placing: true, ..snooze });
        }
        back.push(*id);
        asks.push(Ask { account, message: *id, what: Asked::Return { record, to, until: snooze.until, from: snooze.from }, sent: false });
    }
    if back.is_empty() {
        return None;
    }
    select_after_removal(s, before, &back);
    s.hosted_snooze.asks.extend(asks);
    s.unrecorded = true;
    Some(Ok(json!({"returned": back, "server": true})))
}

/// The id a server named for a message, when it can be its id here: a JMAP id no other message of the account has.
fn new_id(s: &Session, account: Id, message: Id, named: Option<&str>) -> String {
    let Some(named) = named.filter(|id| plausible_id(id)) else { return String::new() };
    let taken = s.mailbox.messages.values().any(|m| m.id != message && m.account == account && m.server.as_ref().is_some_and(|x| x.id == named));
    if taken { String::new() } else { named.to_string() }
}

fn answered(s: &mut Session, v: &Value) -> CmdResult {
    p::only(v, &["message", "ok", "record", "emailId", "mailboxId", "error"])?;
    let message = p::req_id(v, "message")?;
    let ok = v.get("ok").and_then(Value::as_bool).ok_or_else(|| bad("`ok` is true or false"))?;
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let Some(at) = s.hosted_snooze.asks.iter().position(|a| a.message == message && a.sent) else {
        return Ok(json!({"changed": false}));
    };
    let ask = s.hosted_snooze.asks.remove(at);
    let account = ask.account;
    let snoozed = s.mailbox.snoozed_folder(account);
    let mailbox = text("mailboxId").filter(|id| plausible_id(id));
    let named = new_id(s, account, message, text("emailId").as_deref());
    let subject = s.mailbox.message(message).map(|m| m.subject.clone()).unwrap_or_default();
    match ask.what {
        Asked::Put { from, until, placed, was, .. } => {
            let Some(m) = s.mailbox.message(message) else { return Ok(json!({"changed": false})) };
            let server_had = m.server.as_ref().map(|server| server.folder.clone()).unwrap_or_default();
            if ok {
                // Where the server put it, as this window has that folder.
                let there = mailbox
                    .as_ref()
                    .and_then(|id| s.mailbox.folder_by_server(account, id))
                    .filter(|f| s.mailbox.folder(*f).is_some_and(|f| f.kind == FolderKind::Custom));
                match (there, mailbox.as_ref()) {
                    (Some(folder), Some(id)) => {
                        if snoozed != Some(folder)
                            && let Ok(a) = s.mailbox.account_mut(account)
                        {
                            a.snoozed_folder = Some(folder);
                        }
                        if let Ok(m) = s.mailbox.message_mut(message) {
                            m.folder = folder;
                            // (A run that read the folder meanwhile found it under its new id already: that one stays.)
                            if let Some(server) = m.server.as_mut()
                                && !(named.is_empty() && server.folder == *id && plausible_id(&server.id))
                            {
                                server.folder = id.clone();
                                server.id = named;
                            }
                            m.snooze = Some(Snooze { until, from, after_id: Id::MAX, ..Default::default() });
                        }
                    }
                    _ => {
                        // A folder this window doesn't list yet (the server made it now): where the server had it until the
                        // next run, which lists it and brings the message there; the folder is « Plus tard » as soon as it is
                        // listed ([`adopt`]).
                        if let Some(id) = mailbox.clone() {
                            s.hosted_snooze.named.insert(account, id);
                        }
                        let to = s.mailbox.folder_by_server(account, &server_had).unwrap_or(from);
                        if let Ok(m) = s.mailbox.message_mut(message) {
                            m.folder = to;
                            m.snooze = None;
                        }
                        s.hosted_snooze.to_read.insert(account);
                    }
                }
                if let Some(record) = text("record") {
                    s.hosted_snooze.records.insert(message, Record { id: record, failed: None });
                }
            } else {
                let back_to = s.mailbox.folder_by_server(account, &server_had).unwrap_or(from);
                if let Ok(m) = s.mailbox.message_mut(message) {
                    if placed && m.snooze.is_some_and(|z| z.placing) {
                        // Back where its server has it.
                        m.folder = back_to;
                        m.snooze = None;
                    } else if !placed {
                        // Put away again from the folder: the time it had.
                        m.snooze = was;
                    }
                }
                s.hosted_snooze.said.push(refusal_said(s, &subject, text("error").as_deref(), false));
            }
        }
        Asked::Return { to, until, from, .. } => {
            if ok {
                let server_folder = mailbox.clone().or_else(|| s.mailbox.folder(to).and_then(|f| f.server.as_ref()).map(|server| server.id.clone()));
                // Where the server put it, as this window has that folder (the Inbox when the one it came from is gone).
                let there = mailbox.as_ref().and_then(|id| s.mailbox.folder_by_server(account, id));
                if let Ok(m) = s.mailbox.message_mut(message) {
                    if let Some(folder) = there {
                        m.folder = folder;
                    }
                    if let (Some(server), Some(folder)) = (m.server.as_mut(), server_folder) {
                        server.folder = folder;
                        server.id = named;
                    }
                    m.snooze = None;
                }
                s.hosted_snooze.records.remove(&message);
            } else {
                // Back in the Snoozed folder only while it is being brought back still, and its server was last known to have
                // it there: a run may have shown it elsewhere meanwhile (brought back after all, or moved by another client).
                let snoozed_id = snoozed.and_then(|f| s.mailbox.folder(f)).and_then(|f| f.server.as_ref()).map(|server| server.id.clone());
                if let (Some(folder), Ok(m)) = (snoozed, s.mailbox.message_mut(message)) {
                    let still = m.snooze.is_some_and(|z| z.placing) && m.server.as_ref().map(|server| &server.folder) == snoozed_id.as_ref();
                    if still {
                        m.folder = folder;
                        m.snooze = Some(Snooze { until, from, after_id: Id::MAX, ..Default::default() });
                    } else if m.snooze.is_some_and(|z| z.placing) {
                        m.snooze = None;
                    }
                }
                s.hosted_snooze.said.push(refusal_said(s, &subject, text("error").as_deref(), true));
            }
        }
    }
    Ok(json!({"changed": true}))
}

/// What the person reads of a refusal: the message, and the server's words when it gave some.
fn refusal_said(s: &Session, subject: &str, error: Option<&str>, returning: bool) -> String {
    let lang = s.interface_language;
    let subject = if subject.trim().is_empty() { lang.t("(no subject)").to_string() } else { subject.trim().to_string() };
    let said = if returning {
        mailcraft_i18n::fmt(lang.t("'{subject}' could not be brought back now: it stays in the Snoozed folder."), &[("subject", &subject)])
    } else {
        mailcraft_i18n::fmt(lang.t("'{subject}' could not be put away: it stays where it was."), &[("subject", &subject)])
    };
    match error.map(str::trim).filter(|e| !e.is_empty()) {
        Some(error) => format!("{said} {}", error.chars().take(300).collect::<String>()),
        None => said,
    }
}

/// The « Plus tard » folder each server named in an answer ([`Hosted::named`]), taken as its account's Snoozed folder once a run
/// lists it (a folder of the person's own, as `mail.setSnoozes` takes it). Nothing to look for, nothing done: called at every
/// frame of a hosting page's window. Whether one was taken.
pub fn adopt(s: &mut Session) -> bool {
    if s.hosted_snooze.named.is_empty() {
        return false;
    }
    let named: Vec<(Id, String)> = s.hosted_snooze.named.iter().map(|(account, id)| (*account, id.clone())).collect();
    let mut taken = false;
    for (account, id) in named {
        let Some(folder) = s.mailbox.folder_by_server(account, &id).filter(|f| s.mailbox.folder(*f).is_some_and(|f| f.kind == FolderKind::Custom))
        else {
            continue;
        };
        if let Ok(a) = s.mailbox.account_mut(account)
            && a.snoozed_folder != Some(folder)
        {
            a.snoozed_folder = Some(folder);
            taken = true;
        }
        s.hosted_snooze.named.remove(&account);
    }
    taken
}

fn set_snoozes(s: &mut Session, v: &Value) -> CmdResult {
    p::only(v, &["account", "mailbox", "records", "offered"])?;
    let email = p::req_str(v, "account")?.trim().to_lowercase();
    let Some(account) = s.mailbox.accounts.iter().find(|a| a.email.trim().eq_ignore_ascii_case(&email)).map(|a| a.id) else {
        return Ok(json!({"changed": 0}));
    };
    if v.get("offered").and_then(Value::as_bool) == Some(false) {
        // Its server does not keep « Plus tard » (any more): nothing of it is shown, and what was being put away or brought
        // back here lies where its server has it.
        s.hosted_snooze.servers.remove(&account);
        s.hosted_snooze.named.remove(&account);
        s.hosted_snooze.asks.retain(|a| a.account != account);
        let mut fix = Vec::new();
        for m in s.mailbox.messages.values().filter(|m| m.account == account && m.snooze.is_some()) {
            let server_folder = m.server.as_ref().map(|server| server.folder.as_str()).unwrap_or_default();
            let to = m.snooze.filter(|z| z.placing).and_then(|_| s.mailbox.folder_by_server(account, server_folder));
            fix.push((m.id, to));
        }
        let changed = fix.len();
        for (id, to) in fix {
            if let Ok(m) = s.mailbox.message_mut(id) {
                if let Some(to) = to {
                    m.folder = to;
                }
                m.snooze = None;
            }
            s.hosted_snooze.records.remove(&id);
        }
        return Ok(json!({"changed": changed}));
    }
    let mut records: Vec<(String, String, String, String, Timestamp, Option<String>)> = Vec::new();
    for item in v.get("records").and_then(Value::as_array).into_iter().flatten().take(MAX_RECORDS) {
        let field = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(until) = item.get("until").and_then(Value::as_i64) else { continue };
        let failed = item.get("failed").and_then(Value::as_str).map(str::to_string);
        records.push((field("id"), field("emailId"), norm(&field("messageId")), field("fromMailboxId"), until, failed));
    }
    s.hosted_snooze.servers.insert(account);
    // What the server says of its folder is the last word: one an earlier answer named waits no more ([`adopt`]).
    if v.get("mailbox").and_then(Value::as_str).is_some_and(plausible_id) {
        s.hosted_snooze.named.remove(&account);
    }
    // The Snoozed folder, by its id on the server.
    if let Some(folder) =
        v.get("mailbox").and_then(Value::as_str).filter(|id| plausible_id(id)).and_then(|id| s.mailbox.folder_by_server(account, id))
        && s.mailbox.account(account).is_some_and(|a| a.snoozed_folder != Some(folder))
        && s.mailbox.folder(folder).is_some_and(|f| f.kind == FolderKind::Custom)
        && let Ok(a) = s.mailbox.account_mut(account)
    {
        a.snoozed_folder = Some(folder);
    }
    let snoozed = s.mailbox.snoozed_folder(account);
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox);
    let mut fix: Vec<(Id, Option<Id>, Option<Snooze>, Option<Record>)> = Vec::new();
    for m in s.mailbox.messages.values().filter(|m| m.account == account) {
        let placing = m.snooze.is_some_and(|z| z.placing);
        if placing {
            if !s.hosted_snooze.waits(m.id) {
                // Asked in a visit that ended before the answer: where its server has it, until the server says more.
                let server_folder = m.server.as_ref().map(|server| server.folder.as_str()).unwrap_or_default();
                let to = s.mailbox.folder_by_server(account, server_folder).filter(|f| *f != m.folder);
                fix.push((m.id, to, None, None));
            }
            continue;
        }
        if snoozed.is_none_or(|f| f != m.folder) {
            if m.snooze.is_some() || s.hosted_snooze.records.contains_key(&m.id) {
                fix.push((m.id, None, None, None));
            }
            continue;
        }
        let server_id = m.server.as_ref().map(|server| server.id.as_str()).unwrap_or_default();
        let own = norm(&m.message_id);
        let found =
            records.iter().find(|r| !server_id.is_empty() && r.1 == server_id).or_else(|| records.iter().find(|r| !own.is_empty() && r.2 == own));
        match found {
            Some((id, _, _, from, until, failed)) => {
                let from = s.mailbox.folder_by_server(account, from).or(inbox).unwrap_or(m.folder);
                let snooze = Snooze { until: *until, from, failed: failed.is_some(), after_id: Id::MAX, ..Default::default() };
                let record = Record { id: id.clone(), failed: failed.clone() };
                if m.snooze != Some(snooze) || s.hosted_snooze.records.get(&m.id) != Some(&record) {
                    fix.push((m.id, None, Some(snooze), Some(record)));
                }
            }
            None if m.snooze.is_some() || s.hosted_snooze.records.contains_key(&m.id) => fix.push((m.id, None, None, None)),
            None => {}
        }
    }
    let changed = fix.len();
    for (id, to, snooze, record) in fix {
        if let Ok(m) = s.mailbox.message_mut(id) {
            if let Some(to) = to {
                m.folder = to;
            }
            m.snooze = snooze;
        }
        match record {
            Some(record) => s.hosted_snooze.records.insert(id, record),
            None => s.hosted_snooze.records.remove(&id),
        };
    }
    // What the window does not show yet (its folder, its messages, not listed by a run yet): the host reads again soon.
    let matched: HashSet<&str> = s.hosted_snooze.records.values().map(|r| r.id.as_str()).collect();
    let unmatched = records.iter().filter(|r| !matched.contains(r.0.as_str())).count();
    Ok(json!({"changed": changed, "unmatched": unmatched, "folder": snoozed.is_some()}))
}

#[cfg(test)]
#[path = "snooze_hosted_tests.rs"]
mod tests;
