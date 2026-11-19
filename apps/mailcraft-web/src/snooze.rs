// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Plus tard » (snooze) of a hosting page's mailboxes, kept by their
// Neoffice server, which brings the message back at its time even with every window closed.

//! « Plus tard » of the mailboxes a hosting page handed over: their Neoffice server keeps it (frappe_webmail's
//! `neomail_snooze`), under a capability of its own, [`SNOOZE`]: `NeomailSnooze/set` puts a message away in the mailbox's
//! Snoozed folder (« Plus tard ») or brings it back now, `NeomailSnooze/get` says what waits there and when it comes back.
//! The server's job brings each message back at its time, unread, to the folder it came from.
//!
//! What the window asked (the engine's `cmd/snooze_hosted.rs`: `mail.snooze` and `mail.unsnooze` of such a mailbox's
//! mail) is sent here, a mailbox's at once, and the answer goes back through `mail.snoozeAnswered`: where the message is
//! now, or why the server refused (the window puts it back and says so: ui-egui's `snooze_hosted.rs`). What the server
//! keeps is read now and then (every minute, soon after an answer, and a minute after a message's time, [`next_read_in`])
//! and put with `mail.setSnoozes`; a message that came back is brought by its mailbox's next Send/Receive, asked for once
//! ([`come_back`]): a late return is not read for again and again.
//!
//! A server without the capability (a site whose `bench migrate` is still to run) is never asked again: the window does not
//! offer « Plus tard » for that mailbox. A sign-in the server refuses is renewed by the page once, and a trouble that
//! passes is tried again once (`exchanges::ask_with`); a create asked twice is one put away on the server (one waiting
//! record per message).
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::future::Future;
use std::rc::Rc;

use mailcraft_engine::cmd::snooze_hosted::{Ask, Asked};
use mailcraft_jmap::{CORE, Client, Error, Http};
use serde_json::{Map, Value, json};

use crate::credentials::Credentials;
use crate::exchanges::{Outcome, ask_with};

/// The capability: a data type of the Neoffice server (frappe_webmail's neomail_snooze), named by it.
pub const SNOOZE: &str = "https://neoffice.me/jmap/neomail-snooze";
pub const GET: &str = "NeomailSnooze/get";
pub const SET: &str = "NeomailSnooze/set";
/// The most asks one `NeomailSnooze/set` carries: the server moves each message under the mailbox's lock and refuses
/// what is past its call's time, so a large selection goes as several calls.
pub const MAX_ASKS: usize = 25;

/// A mailbox's address as a key.
fn key(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The arguments of a `NeomailSnooze/set` for these asks of one mailbox: a create for each message to put away (by the id
/// its server gave it, and its `Message-ID`), an update for each to bring back now.
pub fn set_arguments(account: &str, asks: &[Ask]) -> Value {
    let mut create = Map::new();
    let mut update = Map::new();
    for ask in asks {
        match &ask.what {
            Asked::Put { email_id, message_id, until, .. } => {
                create.insert(
                    format!("m{}", ask.message),
                    json!({"emailId": email_id, "messageId": message_id, "until": mailcraft_time::format_iso_utc(*until)}),
                );
            }
            Asked::Return { record, .. } => {
                update.insert(record.clone(), json!({"state": "returned"}));
            }
        }
    }
    let mut args = json!({"accountId": account});
    if !create.is_empty() {
        args["create"] = Value::Object(create);
    }
    if !update.is_empty() {
        args["update"] = Value::Object(update);
    }
    args
}

/// What a `NeomailSnooze/set` answer says of each ask, as `mail.snoozeAnswered` takes it.
pub fn answers(asks: &[Ask], answer: &Value) -> Vec<Value> {
    let said = |kind: &str, name: &str| answer.get(kind).and_then(|m| m.get(name)).filter(|v| !v.is_null() || kind == "updated");
    let error = |v: &Value| v.get("description").and_then(Value::as_str).filter(|d| !d.is_empty()).map(str::to_string);
    asks.iter()
        .map(|ask| {
            let (done, refused) = match &ask.what {
                Asked::Put { .. } => {
                    let name = format!("m{}", ask.message);
                    (said("created", &name), said("notCreated", &name))
                }
                Asked::Return { record, .. } => {
                    let done = answer.get("updated").and_then(|m| m.as_object()).and_then(|m| m.get(record.as_str()));
                    (done, said("notUpdated", record))
                }
            };
            match (done, refused) {
                (Some(done), _) => {
                    let mut out = json!({"message": ask.message, "ok": true});
                    for (from, to) in [("id", "record"), ("emailId", "emailId"), ("mailboxId", "mailboxId")] {
                        if let Some(text) = done.get(from).and_then(Value::as_str) {
                            out[to] = json!(text);
                        }
                    }
                    out
                }
                (None, Some(refused)) => json!({"message": ask.message, "ok": false, "error": error(refused)}),
                // Not said either way (a server that answers otherwise): refused, the message is put back.
                (None, None) => json!({"message": ask.message, "ok": false}),
            }
        })
        .collect()
}

/// The records of a `NeomailSnooze/get` answer as `mail.setSnoozes` takes them (`until` in seconds), and the Snoozed
/// folder's id; none when it is no answer of that method.
pub fn records(email: &str, answer: &Value) -> Option<Value> {
    let listed = answer.get("list")?.as_array()?;
    let mut out = Vec::new();
    for item in listed.iter().take(mailcraft_engine::cmd::snooze_hosted::MAX_RECORDS) {
        let text = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let state = text("state");
        if state != "waiting" && state != "failed" {
            continue;
        }
        let Some(until) = mailcraft_time::iso_to_timestamp(&text("until"), 0) else { continue };
        let mut record = json!({
            "id": text("id"),
            "emailId": text("emailId"),
            "messageId": text("messageId"),
            "fromMailboxId": text("fromMailboxId"),
            "until": until,
        });
        if state == "failed" {
            let error = text("error");
            record["failed"] = json!(if error.is_empty() { "-".to_string() } else { error });
        }
        out.push(record);
    }
    Some(json!({"account": key(email), "mailbox": answer.get("mailboxId").and_then(Value::as_str), "records": out}))
}

/// The records that wait (not those whose return failed): their id and when they come back.
fn waiting(records: &Value) -> impl Iterator<Item = (String, i64)> + '_ {
    records
        .get("records")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| r.get("failed").is_none())
        .filter_map(|r| Some((r.get("id")?.as_str()?.to_string(), r.get("until")?.as_i64()?)))
}

/// When to read a mailbox's records next, `wall` and the answer's times in seconds, the result in seconds from now: a few
/// seconds after the earliest time to come (the server's job brings it back within the minute), else [`EVERY_S`]. A time
/// past already is not read for again and again: its message comes with the next Send/Receive, asked for once.
pub fn next_read_in(records: &Value, wall: i64) -> i64 {
    let soonest = waiting(records).map(|(_, until)| until + AFTER_DUE_S).filter(|at| *at > wall).min();
    soonest.map_or(EVERY_S, |at| (at - wall).clamp(SOON_S, EVERY_S))
}

/// The records whose time has come, by `wall` (seconds), that no Send/Receive was asked for yet (`asked`): their ids.
pub fn come_back(records: &Value, wall: i64, asked: &std::collections::BTreeSet<String>) -> Vec<String> {
    waiting(records).filter(|(id, until)| *until + AFTER_DUE_S <= wall && !asked.contains(id)).map(|(id, _)| id).collect()
}

/// How often what a server keeps is read, how soon at the least, and how long after a message's time (the server's job
/// runs each minute), in seconds.
pub const EVERY_S: i64 = 60;
pub const SOON_S: i64 = 2;
pub const AFTER_DUE_S: i64 = 65;

/// One ask of a mailbox's server: `calls` (`NeomailSnooze/get` or `/set` with their arguments, built for the client's
/// account), through the client `kept` or a new one signed in with the page's credential. The client to keep comes back
/// with the answer of the last call.
#[allow(clippy::too_many_arguments)]
pub async fn ask<H, C, S, W, F>(
    http: H,
    credentials: &C,
    email: &str,
    session: &str,
    kept: Option<Rc<Client<H>>>,
    method: &'static str,
    arguments: F,
    sleep: S,
) -> (Option<Rc<Client<H>>>, Outcome<Value>)
where
    H: Http + Clone,
    C: Credentials,
    S: Fn(u32) -> W,
    W: Future<Output = ()>,
    F: Fn(&str) -> Value,
{
    let arguments = &arguments;
    let call = move |client: Rc<Client<H>>| async move {
        let mut answers = client.call(&[CORE, SNOOZE], vec![(method, arguments(client.account_id()))]).await?;
        answers.take(0)
    };
    ask_with(http, credentials, email, session, SNOOZE, kept, call, sleep).await
}

/// What kind of trouble it was, never the server's words (they may name the mailbox).
pub fn kind(e: &Error) -> &'static str {
    crate::streams::kind(e)
}

#[cfg(test)]
#[path = "snooze_tests.rs"]
mod tests;

#[cfg(target_arch = "wasm32")]
pub use run::{frame, start};

/// The browser's part: sending what the window asked, reading what the servers keep, and putting it.
#[cfg(target_arch = "wasm32")]
mod run {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::rc::Rc;
    use std::time::Duration;

    use mailcraft_engine::cmd::snooze_hosted::Ask;
    use mailcraft_jmap::Client;
    use mailcraft_ui_egui::MailApp;
    use serde_json::{Value, json};

    use super::{GET, SET, answers, ask, come_back, key, kind, next_read_in, records, set_arguments};
    use crate::exchanges::Outcome;
    use crate::host::{HostAccount, HostCredentials};

    /// How often what a server keeps is read, how soon after an answer, and after a failure.
    const EVERY_MS: f64 = 60_000.0;
    const SOON_MS: f64 = 1_500.0;
    const FAILED_MS: f64 = 30_000.0;
    /// While the window doesn't show the records yet: read again this soon, this many times at most.
    const QUICK_MS: f64 = 3_000.0;
    const QUICK_READS: u32 = 20;

    type Server = Client<crate::fetch::Fetch>;

    enum Ended {
        Read { email: String, outcome: Outcome<Value> },
        Asked { email: String, asks: Vec<Ask>, outcome: Outcome<Value> },
    }

    #[derive(Default)]
    struct State {
        /// The mailboxes handed over: address (lower case), session address.
        handed: Vec<(String, String)>,
        credentials: Option<Rc<HostCredentials>>,
        ctx: Option<egui::Context>,
        /// The mailboxes whose server does not keep « Plus tard ».
        unsupported: BTreeSet<String>,
        clients: BTreeMap<String, Rc<Server>>,
        /// When each mailbox is read next (ms), and whether a read is on its way.
        next_read: BTreeMap<String, f64>,
        reading: BTreeSet<String>,
        ended: Vec<Ended>,
        /// The records whose time came and whose mailbox's Send/Receive was asked for: once each.
        synced: BTreeSet<String>,
        /// How many reads in a row came back with records the window doesn't show yet, by mailbox.
        quick: BTreeMap<String, u32>,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    /// Start with the mailboxes the page handed over: each is read at once.
    pub fn start(accounts: &[HostAccount], credentials: Rc<HostCredentials>, ctx: egui::Context) {
        let handed: Vec<(String, String)> = accounts.iter().map(|a| (key(&a.email), a.session.trim().to_string())).collect();
        if handed.is_empty() {
            return;
        }
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.next_read = handed.iter().map(|(email, _)| (email.clone(), 0.0)).collect();
            s.handed = handed;
            s.credentials = Some(credentials);
            s.ctx = Some(ctx);
        });
    }

    fn account_of(app: &MailApp, email: &str) -> Option<u64> {
        app.session.mailbox.accounts.iter().find(|a| a.email.trim().eq_ignore_ascii_case(email)).map(|a| a.id)
    }

    /// Put what came, send what the window asked, and read what is due: at every frame of a hosting page's window.
    pub fn frame(app: &mut MailApp) {
        let now = mailcraft_ui_egui::now_ms();
        STATE.with(|cell| {
            let Ok(mut guard) = cell.try_borrow_mut() else { return };
            let s = &mut *guard;
            if s.handed.is_empty() {
                return;
            }
            for ended in std::mem::take(&mut s.ended) {
                settle(s, app, ended, now);
            }
            // (What the server refused, and the runs to make, the window says and makes: ui-egui's snooze_hosted.rs.)
            send(s, app, now);
            read(s, now);
            if let Some(ctx) = &s.ctx {
                // A look to come, though nobody moves: the next read, sooner while an answer is awaited.
                let next = s.next_read.values().copied().fold(f64::MAX, f64::min);
                let busy = !s.reading.is_empty() || app.session.hosted_snooze.asking();
                let wait = if busy { SOON_MS } else { (next - now).clamp(SOON_MS, EVERY_MS) };
                ctx.request_repaint_after(Duration::from_millis(wait as u64));
            }
        });
    }

    fn session_of(s: &State, email: &str) -> Option<String> {
        s.handed.iter().find(|(e, _)| e == email).map(|(_, session)| session.clone())
    }

    /// The window's asks, one call per mailbox.
    fn send(s: &mut State, app: &mut MailApp, now: f64) {
        let asks = app.session.hosted_snooze.take();
        if asks.is_empty() {
            return;
        }
        let mut by_mailbox: BTreeMap<String, Vec<Ask>> = BTreeMap::new();
        for ask in asks {
            let email = app.session.mailbox.account(ask.account).map(|a| key(&a.email)).unwrap_or_default();
            by_mailbox.entry(email).or_default().push(ask);
        }
        let calls = by_mailbox
            .into_iter()
            .flat_map(|(email, asks)| asks.chunks(super::MAX_ASKS).map(|part| (email.clone(), part.to_vec())).collect::<Vec<_>>());
        for (email, asks) in calls {
            let (Some(session), Some(credentials)) = (session_of(s, &email), s.credentials.clone()) else {
                // A mailbox the page did not hand over: nobody to ask; the window puts the messages back.
                for answer in answers(&asks, &Value::Null) {
                    let _ = app.session.run("mail.snoozeAnswered", &answer);
                }
                continue;
            };
            let client = s.clients.get(&email).cloned();
            let ctx = s.ctx.clone();
            s.next_read.insert(email.clone(), now + SOON_MS);
            wasm_bindgen_futures::spawn_local(async move {
                let sent = asks.clone();
                let arguments = move |account: &str| set_arguments(account, &sent);
                let (client, outcome) =
                    ask(crate::fetch::Fetch, &*credentials, &email, &session, client, SET, arguments, |ms| crate::fetch::sleep(ms as i32)).await;
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    if let Some(client) = client {
                        s.clients.insert(email.clone(), client);
                    }
                    s.ended.push(Ended::Asked { email, asks, outcome });
                });
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
        }
    }

    /// Read what each mailbox's server keeps, when it is time.
    fn read(s: &mut State, now: f64) {
        let due: Vec<(String, String)> = s
            .handed
            .iter()
            .filter(|(email, _)| !s.unsupported.contains(email) && !s.reading.contains(email))
            .filter(|(email, _)| s.next_read.get(email).is_none_or(|at| now >= *at))
            .cloned()
            .collect();
        for (email, session) in due {
            let Some(credentials) = s.credentials.clone() else { return };
            s.reading.insert(email.clone());
            s.next_read.insert(email.clone(), now + EVERY_MS);
            let client = s.clients.get(&email).cloned();
            let ctx = s.ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let arguments = |account: &str| json!({"accountId": account});
                let (client, outcome) =
                    ask(crate::fetch::Fetch, &*credentials, &email, &session, client, GET, arguments, |ms| crate::fetch::sleep(ms as i32)).await;
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    if let Some(client) = client {
                        s.clients.insert(email.clone(), client);
                    }
                    s.ended.push(Ended::Read { email, outcome });
                });
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
        }
    }

    fn settle(s: &mut State, app: &mut MailApp, ended: Ended, now: f64) {
        match ended {
            Ended::Read { email, outcome } => {
                s.reading.remove(&email);
                match outcome {
                    Outcome::Answered(answer) => {
                        let Some(put) = records(&email, &answer) else { return };
                        let told = app.session.run("mail.setSnoozes", &put).unwrap_or_default();
                        // A message whose time comes is brought back by the server's job within the minute: read again a
                        // little after, and the mailbox's Send/Receive brings it, asked for once for each. What the window
                        // doesn't show yet (a tab just opened, whose first run is still listing the folder) is read again in
                        // a few seconds, a few times.
                        let wall = app.session.now();
                        let behind = told.get("unmatched").and_then(Value::as_u64).unwrap_or(0) > 0
                            || (told.get("folder") == Some(&json!(false)) && put.get("mailbox").is_some_and(Value::is_string));
                        let quick = s.quick.entry(email.clone()).or_default();
                        let soon = behind && *quick < QUICK_READS;
                        *quick = if behind { quick.saturating_add(1) } else { 0 };
                        let wait = if soon { QUICK_MS } else { (next_read_in(&put, wall) as f64) * 1000.0 };
                        s.next_read.insert(email.clone(), now + wait);
                        let back = come_back(&put, wall, &s.synced);
                        if !back.is_empty()
                            && let Some(account) = account_of(app, &email)
                        {
                            s.synced.extend(back);
                            let _ = app.run("ui.sync", json!({"account": account, "quiet": true}));
                        }
                    }
                    Outcome::Unsupported => {
                        log::info!("neomail snooze: the server does not keep « Plus tard »");
                        // Nothing of it is shown for that mailbox, and what was being placed lies where its server has it.
                        let _ = app.session.run("mail.setSnoozes", &json!({"account": email, "offered": false}));
                        s.unsupported.insert(email);
                    }
                    Outcome::Failed(e) => {
                        log::warn!("neomail snooze: couldn't be read ({})", kind(&e));
                        s.clients.remove(&email);
                        s.next_read.insert(email, now + FAILED_MS);
                    }
                }
            }
            Ended::Asked { email, asks, outcome } => {
                let answer = match outcome {
                    Outcome::Answered(answer) => answer,
                    Outcome::Unsupported => {
                        let _ = app.session.run("mail.setSnoozes", &json!({"account": email, "offered": false}));
                        s.unsupported.insert(email.clone());
                        Value::Null
                    }
                    Outcome::Failed(e) => {
                        log::warn!("neomail snooze: couldn't be asked ({})", kind(&e));
                        s.clients.remove(&email);
                        Value::Null
                    }
                };
                for told in answers(&asks, &answer) {
                    let _ = app.session.run("mail.snoozeAnswered", &told);
                }
                s.next_read.insert(email, now + SOON_MS);
            }
        }
    }
}
