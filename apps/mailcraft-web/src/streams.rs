// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): which kind of sender wrote the Inbox's messages, asked of the Neoffice server
// for the tabs (Main, Notifications, Newsletters).

//! Which kind of sender wrote the messages of a hosting page's Inbox, for the window's tabs.
//!
//! The Neoffice server says it: `NeomailStream/get` (frappe_webmail's `mailcraft_stream`), under a capability of its own,
//! [`STREAM`], which a mailbox's session offers only where the site asks for it. It is given the ids the server itself gave the
//! messages (never a word of a message: it reads the mailbox itself) and answers each with `person`, `notification` or
//! `newsletter`, or says it has not decided yet (`pending`: its small model was slow, ask again in a few seconds).
//!
//! What it says goes to the window through `mail.setStreams` (engine `cmd/stream.rs`): nothing is written to a mailbox or kept
//! in the page, and the window does not keep what it was told between two visits (the server does, in its own cache).
//!
//! **What is asked.** Every few seconds, for each mailbox handed over, the messages of its Inbox that have a server id and no
//! stream yet, the newest first, [`MAX_IDS`] at most. A message the server does not know ([`MAX_TRIES`] answers short of an
//! answer) is a person's by default, as the window shows a message nobody said anything of. A server without the capability is
//! never asked again, and its Inbox has no tabs. A message whose stream the window lost (the synchronisation put a new copy of
//! it there) gets what it was told back at once, without a second question. An ask that meets a trouble that passes by itself
//! (a server starting again, a network blink) is made once more after a moment, and a sign-in the server refuses is renewed by the
//! page once, as for the Échanges pane: `exchanges::ask_with` is the one place that settles both.
//!
//! **A person's choice of tab** (« Classer dans », neoffice-maintenance#3142). Where the capability says `correct: true`, the
//! window offers to put a message of the Inbox under another tab (the engine's `cmd/stream_correct.rs`): what it asks is sent
//! here as `NeomailStream/set {accountId, corrections: [{id, stream, sender}]}`, a mailbox's at once, and the answer goes back
//! through `mail.streamCorrected` (a message the server did not take is put back). The server keeps the choice for the message,
//! and with `sender: true` (the person's yes to its sender's next ones) for the sender, for this person and this mailbox; its
//! next answers say it. What was chosen here is remembered as said, so that a synchronisation that takes the stream puts the
//! chosen one back.
//!
//! **What is not here.** The server's model, its headers rules and its cache are the server's; the tabs, the boxes and the bar are
//! the window's (`ui-egui`); this file is only what asks and what puts.
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::rc::Rc;

use mailcraft_engine::cmd::stream_correct::Correction;
use mailcraft_jmap::{CORE, Client, Error, Http};
use mailcraft_model::{FolderKind, Id, Mailbox, Stream};
use serde_json::{Value, json};

use crate::credentials::Credentials;
use crate::exchanges::{Outcome, ask_with};

/// A mailbox's address as a key (as kept.rs has it).
fn key(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The capability: a data type of the Neoffice server (frappe_webmail's mailcraft_stream), named by it.
pub const STREAM: &str = "https://neoffice.me/jmap/neomail-stream";
pub const GET: &str = "NeomailStream/get";
// //// Neoffice — a person's choice of tab (#3142).
pub const SET: &str = "NeomailStream/set";
/// The most ids one ask holds: what the server takes (a window of the list).
pub const MAX_IDS: usize = 200;
/// How many times a message is asked about without the server giving a stream (its model was away, or it was gone): then it
/// is left, and shown with the main tab's.
pub const MAX_TRIES: u8 = 5;

/// What the server said of one ask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    /// Each message by its id on the server, with the stream it said.
    pub says: Vec<(String, Stream)>,
    /// The messages its model had not answered in the time it has: ask again a little later.
    pub pending: Vec<String>,
    /// The messages it did not find (gone from the folder, or an id it could not read) and the ones it said a kind this window
    /// does not know (a newer server): neither is asked about again.
    pub left: Vec<String>,
    /// The mailbox's server takes a person's choice of tab (its capability says `correct: true`).
    pub correct: bool,
}

/// What was asked and told, and what is not to be asked about.
#[derive(Debug, Default)]
pub struct Book {
    /// What the server said of each message of this visit, to put back what a synchronisation took.
    told: BTreeMap<Id, Stream>,
    /// The messages asked about and left without an answer, and how many times.
    tries: BTreeMap<Id, u8>,
    /// The messages not to be asked about again.
    left: BTreeSet<Id>,
}

/// What a look at a mailbox finds to do: the messages to ask about (window id and server id), and the streams to put back.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub ask: Vec<(Id, String)>,
    pub restore: Vec<(Id, Stream)>,
}

/// The Inbox folders of the accounts of the address `email` that keep their own mail (not a shared mailbox opened through another).
fn inboxes(mb: &Mailbox, email: &str) -> Vec<Id> {
    mb.accounts
        .iter()
        .filter(|a| a.shared_of.is_none() && !a.email.trim().is_empty() && key(&a.email) == email)
        .filter_map(|a| mb.folder_of_kind(a.id, FolderKind::Inbox))
        .collect()
}

impl Book {
    /// What to do for the mailbox `email` now: ask about the Inbox's messages that have a server id and no stream (the newest
    /// first, [`MAX_IDS`] at most), and put back what a synchronisation took.
    pub fn plan(&self, mb: &Mailbox, email: &str) -> Plan {
        let inboxes = inboxes(mb, email);
        let mut plan = Plan::default();
        let mut open = Vec::new();
        for m in mb.messages.values().filter(|m| inboxes.contains(&m.folder) && !m.draft && m.stream.is_none()) {
            if let Some(stream) = self.told.get(&m.id) {
                plan.restore.push((m.id, *stream));
                continue;
            }
            let Some(server) = m.server.as_ref().map(|s| s.id.trim()).filter(|id| !id.is_empty()) else { continue };
            if self.left.contains(&m.id) || self.tries.get(&m.id).is_some_and(|n| *n >= MAX_TRIES) {
                continue;
            }
            open.push((mailcraft_engine::list::when(m), m.id, server.to_string()));
        }
        open.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        plan.ask = open.into_iter().take(MAX_IDS).map(|(_, id, server)| (id, server)).collect();
        plan
    }

    /// Take in an answer to the ask `asked`: what was said is told to the window (returned as what to put), what is pending or
    /// missing is counted, what is left is left.
    pub fn take(&mut self, asked: &[(Id, String)], answer: &Answer) -> Vec<(Id, Stream)> {
        let by_server: BTreeMap<&str, Id> = asked.iter().map(|(id, server)| (server.as_str(), *id)).collect();
        let mut put = Vec::new();
        for (server, stream) in &answer.says {
            if let Some(id) = by_server.get(server.as_str()) {
                self.told.insert(*id, *stream);
                self.tries.remove(id);
                put.push((*id, *stream));
            }
        }
        for server in &answer.left {
            if let Some(id) = by_server.get(server.as_str()) {
                self.left.insert(*id);
            }
        }
        // What was asked and got no word (pending, or left out of the answer) was tried once more.
        let said: BTreeSet<Id> = put.iter().map(|(id, _)| *id).collect();
        for (id, server) in asked {
            if said.contains(id) || answer.left.contains(server) {
                continue;
            }
            *self.tries.entry(*id).or_insert(0) += 1;
        }
        put
    }

    /// Whether some of what an answer left pending is worth asking about again soon.
    pub fn waiting(&self) -> bool {
        self.tries.values().any(|n| *n < MAX_TRIES)
    }

    /// The person chose these streams (`mail.correctStream`, sent to the server): what is put back after a synchronisation.
    pub fn chosen(&mut self, said: &[(Id, Stream)]) {
        for (id, stream) in said {
            self.told.insert(*id, *stream);
        }
    }

    /// The server did not take a choice: what each message had before is what is put back.
    pub fn unchosen(&mut self, was: &[(Id, Option<Stream>)]) {
        for (id, stream) in was {
            match stream {
                Some(stream) => self.told.insert(*id, *stream),
                None => self.told.remove(id),
            };
        }
    }
}

/// What a `NeomailStream/get` answer says.
pub fn read_answer(answer: &Value) -> Option<Answer> {
    let listed = answer.get("list")?.as_array()?;
    let mut out = Answer::default();
    for item in listed.iter().take(MAX_IDS) {
        let Some(id) = item.get("id").and_then(Value::as_str) else { continue };
        match item.get("stream").and_then(Value::as_str).and_then(Stream::from_code) {
            Some(stream) => out.says.push((id.to_string(), stream)),
            None => out.left.push(id.to_string()),
        }
    }
    let names = |field: &str| -> Vec<String> {
        answer.get(field).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).take(MAX_IDS).map(str::to_string).collect()
    };
    out.pending = names("pending");
    out.left.extend(names("notFound"));
    Some(out)
}

/// The arguments of an ask.
pub fn arguments(account: &str, ids: &[String]) -> Value {
    json!({"accountId": account, "ids": ids.iter().take(MAX_IDS).collect::<Vec<_>>()})
}

/// Ask the server about `ids` (their ids on the server) through a mailbox's client.
pub async fn ask<H: Http>(client: &Client<H>, ids: &[String]) -> Result<Answer, Error> {
    let mut answers = client.call(&[CORE, STREAM], vec![(GET, arguments(client.account_id(), ids))]).await?;
    let answer = answers.take(0)?;
    let mut answer = read_answer(&answer).ok_or_else(|| Error::Protocol("no list in the stream answer".into()))?;
    // //// Neoffice — whether its server takes a person's choice of tab, as the session it signed in with says (#3142).
    answer.correct = takes_corrections(client.account_capability(STREAM));
    Ok(answer)
}

/// Whether an account's capability (its object in `accountCapabilities`) says the server takes a person's choice of tab.
pub fn takes_corrections(capability: Option<&Value>) -> bool {
    capability.and_then(|c| c.get("correct")).and_then(Value::as_bool) == Some(true)
}

/// The arguments of a `NeomailStream/set` for these choices of one mailbox, by the ids its server gave the messages.
pub fn set_arguments(account: &str, asks: &[Correction]) -> Value {
    let corrections: Vec<Value> =
        asks.iter().take(MAX_IDS).map(|a| json!({"id": a.email_id, "stream": a.stream.code(), "sender": a.sender})).collect();
    json!({"accountId": account, "corrections": corrections})
}

/// What a `NeomailStream/set` answer says of each choice: whether the server took it (its id is among `updated`). Anything
/// else (not found, an answer of another shape, no answer) is a refusal: the window puts the messages back.
pub fn corrected(asks: &[Correction], answer: &Value) -> Vec<(Id, bool)> {
    let updated: BTreeSet<&str> = answer.get("updated").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    asks.iter().map(|a| (a.message, updated.contains(a.email_id.as_str()))).collect()
}

/// Put what the server said (and what was put back) in the window, as `mail.setStreams` does. How many messages changed.
pub fn put(session: &mut mailcraft_engine::Session, said: &[(Id, Stream)]) -> usize {
    if said.is_empty() {
        return 0;
    }
    let streams: Vec<Value> = said.iter().map(|(id, stream)| json!({"id": id, "stream": stream.code()})).collect();
    session
        .run("mail.setStreams", &json!({"streams": streams}))
        .ok()
        .and_then(|v| v.get("changed").and_then(Value::as_u64))
        .map_or(0, |n| usize::try_from(n).unwrap_or(0))
}

/// How an ask ended when it did not bring an answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The server has no such method (its site turned the tabs off since the session): never asked again.
    Unsupported,
    /// Another time.
    Trouble(&'static str),
}

/// What kind of trouble it was, never the server's words (they may name the mailbox).
pub fn kind(e: &Error) -> &'static str {
    match e {
        Error::Io(_) => "network",
        Error::Tls(_) => "secure connection",
        Error::Auth(_) => "sign-in",
        Error::Busy { .. } => "busy",
        _ => "refused",
    }
}

/// One ask of the server about `ids`, through the client `kept` or a new one signed in with the page's credential.
///
/// A trouble that passes by itself (the server that starts again, a network blink) is tried again once after a moment, and a
/// credential the server refuses, at the session as at the call, is renewed by the page once: [`ask_with`] settles both, as it
/// does for the Échanges pane. The tabs showed nothing for half a minute after a first failure, and the window's sign-in was
/// never renewed here. The client to keep for the next ask comes back with the answer.
pub async fn ask_streams<H, C, S, W>(
    http: H,
    credentials: &C,
    email: &str,
    session: &str,
    kept: Option<Rc<Client<H>>>,
    ids: &[String],
    sleep: S,
) -> (Option<Rc<Client<H>>>, Result<Answer, Failure>)
where
    H: Http + Clone,
    C: Credentials,
    S: Fn(u32) -> W,
    W: Future<Output = ()>,
{
    let call = move |client: Rc<Client<H>>| async move { ask(&client, ids).await };
    let (client, outcome) = ask_with(http, credentials, email, session, STREAM, kept, call, sleep).await;
    let told = match outcome {
        Outcome::Answered(answer) => Ok(answer),
        Outcome::Unsupported => Err(Failure::Unsupported),
        Outcome::Failed(e) => Err(Failure::Trouble(kind(&e))),
    };
    (client, told)
}

#[cfg(test)]
#[path = "streams_tests.rs"]
mod tests;

#[cfg(target_arch = "wasm32")]
pub use run::{frame, start};

/// The browser's part: asking now and then, and putting what came.
#[cfg(target_arch = "wasm32")]
mod run {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::rc::Rc;
    use std::time::Duration;

    use mailcraft_engine::cmd::stream_correct::Correction;
    use mailcraft_jmap::{CORE, Client};
    use mailcraft_model::{Id, Stream};
    use mailcraft_ui_egui::MailApp;
    use serde_json::{Value, json};

    use super::{Answer, Book, Failure, MAX_IDS, SET, STREAM, ask_streams, corrected, key, kind, put, set_arguments};
    use crate::exchanges::{Outcome, ask_with};
    use crate::host::{HostAccount, HostCredentials};

    /// How long to wait before looking again: after an answer that holds something to finish, after one that did not, when there
    /// is nothing to ask, and after a failure.
    const SOON_MS: f64 = 1_500.0;
    const PENDING_MS: f64 = 4_000.0;
    const IDLE_MS: f64 = 3_000.0;
    const FAILED_MS: f64 = 30_000.0;

    type Server = Client<crate::fetch::Fetch>;

    /// How an ask ended: what was asked, and what came of it.
    struct Ended {
        email: String,
        asked: Vec<(Id, String)>,
        result: Result<Answer, Failure>,
    }

    #[derive(Default)]
    struct State {
        /// The mailboxes handed over: address (lower case), session address.
        handed: Vec<(String, String)>,
        credentials: Option<Rc<HostCredentials>>,
        ctx: Option<egui::Context>,
        /// The mailboxes whose server does not offer the tabs.
        unsupported: BTreeSet<String>,
        /// The signed-in clients, kept from one ask to the next.
        clients: BTreeMap<String, Rc<Server>>,
        book: Book,
        ended: Option<Ended>,
        asking: bool,
        next: f64,
        /// Which mailbox the next look starts at, so that one with much to ask does not starve the others.
        turn: usize,
        // //// Neoffice — a person's choice of tab (#3142): whether each mailbox's server takes it, as last told the window, and
        // //// Neoffice — the answers of the choices sent.
        offered: BTreeMap<String, bool>,
        chosen: Vec<(String, Vec<Correction>, Outcome<Value>)>,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    /// Start asking for the mailboxes the page handed over.
    pub fn start(accounts: &[HostAccount], credentials: Rc<HostCredentials>, ctx: egui::Context) {
        let handed: Vec<(String, String)> = accounts.iter().map(|a| (key(&a.email), a.session.trim().to_string())).collect();
        if handed.is_empty() {
            return;
        }
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.handed = handed;
            s.credentials = Some(credentials);
            s.ctx = Some(ctx);
        });
    }

    /// Put what an ask brought, and look for what to ask next: called at every frame of a hosting page's window.
    pub fn frame(app: &mut MailApp) {
        let now = mailcraft_ui_egui::now_ms();
        STATE.with(|cell| {
            let Ok(mut guard) = cell.try_borrow_mut() else { return };
            let s = &mut *guard;
            if s.handed.is_empty() {
                return;
            }
            if let Some(ended) = s.ended.take() {
                s.asking = false;
                s.next = now + settle(s, app, ended);
            }
            // //// Neoffice — a person's choice of tab: the answers that came, and what the window asked to send (#3142).
            for (email, asks, outcome) in std::mem::take(&mut s.chosen) {
                answered(s, app, &email, &asks, outcome);
            }
            send(s, app);
            if !s.asking && now >= s.next {
                s.next = now + look(s, app);
            }
            if let Some(ctx) = &s.ctx {
                // A look to come, though nobody moves
                ctx.request_repaint_after(Duration::from_millis(IDLE_MS as u64));
            }
        });
    }

    /// Take in how an ask ended; how long to wait before the next look.
    fn settle(s: &mut State, app: &mut MailApp, ended: Ended) -> f64 {
        match ended.result {
            Ok(answer) => {
                let said = s.book.take(&ended.asked, &answer);
                put(&mut app.session, &said);
                // //// Neoffice — whether its server takes a person's choice of tab, told the window when it changes (#3142).
                offer(s, app, &ended.email, answer.correct);
                if s.book.waiting() { PENDING_MS } else { SOON_MS }
            }
            Err(Failure::Unsupported) => {
                // Said once a mailbox (it is never asked again), and nothing of it: why its Inbox shows no tabs.
                log::info!("neomail streams: the server does not offer the Inbox tabs (Main, Notifications, Newsletters)");
                offer(s, app, &ended.email, false);
                s.unsupported.insert(ended.email);
                SOON_MS
            }
            Err(Failure::Trouble(kind)) => {
                log::warn!("neomail streams: couldn't be asked ({kind})");
                s.clients.remove(&ended.email);
                FAILED_MS
            }
        }
    }

    /// Put back what a synchronisation took, and send an ask for the first mailbox that has something to ask; how long to wait.
    fn look(s: &mut State, app: &mut MailApp) -> f64 {
        let n = s.handed.len();
        for step in 0..n {
            let (email, session) = s.handed[(s.turn + step) % n].clone();
            if s.unsupported.contains(&email) {
                continue;
            }
            let plan = s.book.plan(&app.session.mailbox, &email);
            put(&mut app.session, &plan.restore);
            if plan.ask.is_empty() {
                continue;
            }
            let Some(credentials) = s.credentials.clone() else { return IDLE_MS };
            s.turn = (s.turn + step + 1) % n;
            s.asking = true;
            let client = s.clients.get(&email).cloned();
            let ctx = s.ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let asked = plan.ask;
                let ids: Vec<String> = asked.iter().map(|(_, server)| server.clone()).collect();
                let (client, result) =
                    ask_streams(crate::fetch::Fetch, &*credentials, &email, &session, client, &ids, |ms| crate::fetch::sleep(ms as i32)).await;
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    if let Some(client) = client {
                        s.clients.insert(email.clone(), client);
                    }
                    s.ended = Some(Ended { email, asked, result });
                });
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            return SOON_MS;
        }
        IDLE_MS
    }

    // //// Neoffice — a person's choice of tab (« Classer dans », #3142).
    /// Tell the window whether the mailbox `email`'s server takes a person's choice of tab, when that changed.
    fn offer(s: &mut State, app: &mut MailApp, email: &str, offered: bool) {
        if s.offered.get(email) == Some(&offered) {
            return;
        }
        s.offered.insert(email.to_string(), offered);
        let _ = app.session.run("mail.offerStreamCorrection", &json!({"account": email, "offered": offered}));
    }

    fn session_of(s: &State, email: &str) -> Option<String> {
        s.handed.iter().find(|(e, _)| e == email).map(|(_, session)| session.clone())
    }

    /// The choices the window asked, one call per mailbox ([`MAX_IDS`] at most each).
    fn send(s: &mut State, app: &mut MailApp) {
        let asks = app.session.hosted_stream.take();
        if asks.is_empty() {
            return;
        }
        let mut by_mailbox: BTreeMap<String, Vec<Correction>> = BTreeMap::new();
        for ask in asks {
            // What is put back after a synchronisation is what the person chose, from now on.
            let chosen: Vec<(Id, Stream)> =
                std::iter::once(ask.message).chain(ask.was.iter().map(|(id, _)| *id)).map(|id| (id, ask.stream)).collect();
            s.book.chosen(&chosen);
            let email = app.session.mailbox.account(ask.account).map(|a| key(&a.email)).unwrap_or_default();
            by_mailbox.entry(email).or_default().push(ask);
        }
        for (email, asks) in by_mailbox {
            for part in asks.chunks(MAX_IDS) {
                let part = part.to_vec();
                let (Some(session), Some(credentials)) = (session_of(s, &email), s.credentials.clone()) else {
                    // A mailbox the page did not hand over: nobody to ask; the window puts the messages back.
                    s.chosen.push((email.clone(), part, Outcome::Answered(Value::Null)));
                    continue;
                };
                let client = s.clients.get(&email).cloned();
                let ctx = s.ctx.clone();
                let email = email.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let sent = part.clone();
                    let call = move |client: Rc<Client<crate::fetch::Fetch>>| {
                        let arguments = set_arguments(client.account_id(), &sent);
                        async move {
                            let mut answers = client.call(&[CORE, STREAM], vec![(SET, arguments)]).await?;
                            answers.take(0)
                        }
                    };
                    let (client, outcome) =
                        ask_with(crate::fetch::Fetch, &*credentials, &email, &session, STREAM, client, call, |ms| crate::fetch::sleep(ms as i32))
                            .await;
                    STATE.with(|s| {
                        let mut s = s.borrow_mut();
                        if let Some(client) = client {
                            s.clients.insert(email.clone(), client);
                        }
                        s.chosen.push((email, part, outcome));
                    });
                    if let Some(ctx) = ctx {
                        ctx.request_repaint();
                    }
                });
            }
        }
    }

    /// What a server answered of choices: each taken, or put back by the window (which tells the person).
    fn answered(s: &mut State, app: &mut MailApp, email: &str, asks: &[Correction], outcome: Outcome<Value>) {
        let answer = match outcome {
            Outcome::Answered(answer) => answer,
            Outcome::Unsupported => {
                offer(s, app, email, false);
                Value::Null
            }
            Outcome::Failed(e) => {
                log::warn!("neomail streams: a choice of tab couldn't be sent ({})", kind(&e));
                s.clients.remove(email);
                Value::Null
            }
        };
        for ((message, ok), ask) in corrected(asks, &answer).into_iter().zip(asks) {
            if !ok {
                s.book.unchosen(&ask.was);
            }
            let _ = app.session.run("mail.streamCorrected", &json!({"message": message, "ok": ok}));
        }
    }
}
