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
//! it there) gets what it was told back at once, without a second question.
//!
//! **What is not here.** The server's model, its headers rules and its cache are the server's; the tabs, the boxes and the bar are
//! the window's (`ui-egui`); this file is only what asks and what puts.
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use mailcraft_jmap::{CORE, Client, Error, Http};
use mailcraft_model::{FolderKind, Id, Mailbox, Stream};
use serde_json::{Value, json};

/// A mailbox's address as a key (as kept.rs has it).
fn key(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The capability: a data type of the Neoffice server (frappe_webmail's mailcraft_stream), named by it.
pub const STREAM: &str = "https://neoffice.me/jmap/neomail-stream";
pub const GET: &str = "NeomailStream/get";
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
    read_answer(&answer).ok_or_else(|| Error::Protocol("no list in the stream answer".into()))
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

    use mailcraft_jmap::{Client, Error};
    use mailcraft_model::Id;
    use mailcraft_ui_egui::MailApp;

    use super::{Answer, Book, STREAM, ask, key, put};
    use crate::credentials::{Credentials, Renewal};
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

    enum Failure {
        /// The server has no such method (its site turned the tabs off since the session): never asked again.
        Unsupported,
        /// Another time.
        Trouble(&'static str),
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
                if s.book.waiting() { PENDING_MS } else { SOON_MS }
            }
            Err(Failure::Unsupported) => {
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
                let (client, result) = go(&credentials, &email, &session, client, &asked).await;
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

    /// One ask: through the client kept (signed in again when the server refuses its credential, once), or a new one.
    async fn go(
        credentials: &HostCredentials,
        email: &str,
        session: &str,
        kept: Option<Rc<Server>>,
        asked: &[(Id, String)],
    ) -> (Option<Rc<Server>>, Result<Answer, Failure>) {
        let ids: Vec<String> = asked.iter().map(|(_, server)| server.clone()).collect();
        let mut client = kept;
        let mut renewed = false;
        loop {
            let current = match client.take() {
                Some(client) => client,
                None => match connect(credentials, email, session).await {
                    Ok(Some(client)) => Rc::new(client),
                    Ok(None) => return (None, Err(Failure::Unsupported)),
                    Err(e) => return (None, Err(Failure::Trouble(kind(&e)))),
                },
            };
            match ask(&current, &ids).await {
                Ok(answer) => return (Some(current), Ok(answer)),
                Err(Error::Refused { kind, .. }) if kind.contains("unknownMethod") || kind.contains("unknownCapability") => {
                    return (None, Err(Failure::Unsupported));
                }
                Err(Error::Auth(_)) if !renewed => {
                    renewed = true;
                    match credentials.renew(email).await {
                        Renewal::Given(_) => continue,
                        Renewal::Later | Renewal::Reload => return (None, Err(Failure::Trouble("sign-in"))),
                    }
                }
                Err(e) => return (Some(current), Err(Failure::Trouble(kind(&e)))),
            }
        }
    }

    /// The mailbox `email`'s server, signed in; `None` when it doesn't offer the tabs.
    async fn connect(credentials: &HostCredentials, email: &str, session: &str) -> Result<Option<Server>, Error> {
        let Some(auth) = credentials.auth(email) else { return Err(Error::Auth("no credential".into())) };
        match Client::connect(crate::fetch::Fetch, session, &auth).await {
            Ok(client) => Ok(client.has_capability(STREAM).then_some(client)),
            Err(e) => Err(e),
        }
    }

    /// What kind of trouble it was, never the server's words (they may name the mailbox).
    fn kind(e: &Error) -> &'static str {
        match e {
            Error::Io(_) => "network",
            Error::Tls(_) => "secure connection",
            Error::Auth(_) => "sign-in",
            Error::Busy { .. } => "busy",
            _ => "refused",
        }
    }
}
