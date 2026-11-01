// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what the Échanges pane asks of the Neoffice server, in the browser.

//! What the Échanges pane asks of the Neoffice server for a hosting page's window: who the sender is, the messages with them in every
//! folder (a page at a time), and what Neoffice links them to.
//!
//! The pane (`mailcraft_ui_egui::exchanges`) says what it wants ([`Exchanges::take_wants`]); this file asks it, under a capability of
//! the server's own, [`HISTORY`] (`frappe_webmail`'s `neomail_history`: `NeomailContact/get`, `NeomailHistory/get`,
//! `NeomailLinked/get`), through the mailbox's signed-in client, and puts each answer back. It runs in the web page and in Neodesk
//! alike: both give the window the mailbox's JMAP session.
//!
//! **What is sent** is addresses and a day, never a word of a message. **What comes back** is read as text by
//! `mailcraft_model::exchanges`. A server without the capability, or without the method (its site is older), is said so to the pane,
//! which then lists what the window has and says that it cannot ask for more; any other trouble is told as such, and the pane asks
//! again a little later when the person asks it to.
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::future::Future;
use std::rc::Rc;

use mailcraft_jmap::{CORE, Client, Error, Http};
use mailcraft_ui_egui::exchanges::{Trouble, Want};
use serde_json::{Value, json};

use crate::credentials::{Credentials, Renewal};

/// The capability: a data type of the Neoffice server (frappe_webmail's `neomail_history`), named by it.
pub const HISTORY: &str = "https://neoffice.me/jmap/neomail-history";
pub const CONTACT: &str = "NeomailContact/get";
pub const MESSAGES: &str = "NeomailHistory/get";
pub const LINKED: &str = "NeomailLinked/get";
/// How many messages one page holds.
pub const PAGE_ROWS: u32 = 20;
/// How long an ask waits before its one more try, when the server did not say, in milliseconds.
pub const AGAIN_MS: u32 = 1_500;
/// The longest an ask waits in silence for a server that asked it to (`Retry-After`), in milliseconds: a server that wants more
/// than this is told to the pane, which offers to try again.
pub const MOST_WAIT_MS: u32 = 5_000;

/// What a [`Want`] is, as a method of the server and its arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub method: &'static str,
    pub arguments: Value,
}

/// The call that answers a want, for the mailbox whose JMAP account is `account`.
pub fn call_of(account: &str, want: &Want) -> Call {
    match want {
        Want::Someone { address, .. } => Call { method: CONTACT, arguments: json!({"accountId": account, "address": address}) },
        Want::Linked { address, .. } => Call { method: LINKED, arguments: json!({"accountId": account, "address": address}) },
        Want::Page { addresses, before, cc, domain, .. } => {
            let mut arguments = json!({"accountId": account, "addresses": addresses, "limit": PAGE_ROWS});
            if let Some(before) = before {
                arguments["before"] = json!(before);
            }
            if *cc {
                arguments["includeCc"] = json!(true);
            }
            if *domain {
                arguments["domains"] = json!(true);
            }
            Call { method: MESSAGES, arguments }
        }
    }
}

/// The mailbox a want is for, by its address in lower case.
pub fn mailbox_of(want: &Want) -> &str {
    match want {
        Want::Someone { mailbox, .. } | Want::Page { mailbox, .. } | Want::Linked { mailbox, .. } => mailbox,
    }
}

/// How long to wait before the one more try that follows `e`, when `e` is a trouble that passes by itself: a server that is
/// starting again (a gateway's 502 or 504, any 5xx), a network that blinked, a server that is busy (waiting what it asked
/// for, up to [`MOST_WAIT_MS`]), a method that failed on the server's side for the moment. `None` for what trying again will not change
/// (a refusal of the sign-in, a request the server does not take, an answer that cannot be read).
pub fn passes(e: &Error) -> Option<u32> {
    match e {
        Error::Io(_) => Some(AGAIN_MS),
        Error::Busy { retry_after, .. } => {
            let wait = retry_after.map_or(AGAIN_MS, |seconds| seconds.saturating_mul(1_000));
            (wait <= MOST_WAIT_MS).then_some(wait)
        }
        // (The client says a gateway's 502 and 504 as a refusal of the request: `jmap_fork::check`.)
        Error::Bad(said) if said.starts_with("HTTP 5") => Some(AGAIN_MS),
        Error::Refused { kind, .. } if kind.contains("serverFail") || kind.contains("serverUnavailable") => Some(AGAIN_MS),
        _ => None,
    }
}

/// How an ask of the server ended, before it is said in the words of the one who asked.
#[derive(Debug)]
pub enum Outcome<T> {
    /// The server answered.
    Answered(T),
    /// The server does not offer the capability, or no longer knows the method (its site is older, or turned it off).
    Unsupported,
    /// It could not be asked, even after the one more try: the last trouble, for whoever logs its kind.
    Failed(Error),
}

/// The mailbox `email`'s server, signed in; `None` when it doesn't offer `capability`.
async fn connect<H: Http, C: Credentials>(
    http: H,
    credentials: &C,
    email: &str,
    session: &str,
    capability: &str,
) -> Result<Option<Client<H>>, Error> {
    let Some(auth) = credentials.auth(email) else { return Err(Error::Auth("no credential".into())) };
    let client = Client::connect(http, session, &auth).await?;
    Ok(client.has_capability(capability).then_some(client))
}

/// One ask of the server, for any of the window's asks under a capability of the server's own: through the client `kept` from an
/// earlier ask, or a new one signed in with the page's credential, and `call` is what is asked of that client.
///
/// What is not the person's to see is settled here, in silence, once each: a credential the server refuses (at the session
/// or at the call) is renewed by the page, and a trouble that passes by itself ([`passes`]: the server that starts again, a
/// network blink) is tried again after a moment, on a new sign-in. Only the second failure of the same kind is told, as
/// [`Outcome::Failed`]. The client to keep for the next ask comes back with the outcome (none when it is not to be kept).
///
/// Every ask of the Neoffice server that is made from the page goes through this one function, so that a way of failing is
/// settled once: the Échanges pane's ask forgot to renew a credential that kept.rs renews, and gave up on the first 502.
#[allow(clippy::too_many_arguments)]
pub async fn ask_with<H, C, S, W, T, F, Fut>(
    http: H,
    credentials: &C,
    email: &str,
    session: &str,
    capability: &str,
    kept: Option<Rc<Client<H>>>,
    call: F,
    sleep: S,
) -> (Option<Rc<Client<H>>>, Outcome<T>)
where
    H: Http + Clone,
    C: Credentials,
    S: Fn(u32) -> W,
    W: Future<Output = ()>,
    F: Fn(Rc<Client<H>>) -> Fut,
    Fut: Future<Output = Result<T, Error>>,
{
    let mut client = kept;
    let (mut renewed, mut again) = (false, false);
    loop {
        let current = match client.take() {
            Some(client) => client,
            None => match connect(http.clone(), credentials, email, session, capability).await {
                Ok(Some(client)) => Rc::new(client),
                Ok(None) => return (None, Outcome::Unsupported),
                Err(e @ Error::Auth(_)) if !renewed => {
                    renewed = true;
                    match credentials.renew(email).await {
                        Renewal::Given(_) => continue,
                        Renewal::Later | Renewal::Reload => return (None, Outcome::Failed(e)),
                    }
                }
                Err(e) => match passes(&e).filter(|_| !again) {
                    Some(wait) => {
                        again = true;
                        sleep(wait).await;
                        continue;
                    }
                    None => return (None, Outcome::Failed(e)),
                },
            },
        };
        match call(current.clone()).await {
            Ok(answer) => return (Some(current), Outcome::Answered(answer)),
            Err(Error::Refused { kind, .. }) if kind.contains("unknownMethod") || kind.contains("unknownCapability") => {
                return (None, Outcome::Unsupported);
            }
            Err(e @ Error::Auth(_)) if !renewed => {
                renewed = true;
                match credentials.renew(email).await {
                    Renewal::Given(_) => continue,
                    Renewal::Later | Renewal::Reload => return (None, Outcome::Failed(e)),
                }
            }
            Err(e) => match passes(&e).filter(|_| !again) {
                Some(wait) => {
                    again = true;
                    sleep(wait).await;
                    continue;
                }
                None => return (Some(current), Outcome::Failed(e)),
            },
        }
    }
}

/// One ask of the Échanges pane, in the pane's words: see [`ask_with`].
pub async fn ask<H, C, S, W>(
    http: H,
    credentials: &C,
    email: &str,
    session: &str,
    kept: Option<Rc<Client<H>>>,
    want: &Want,
    sleep: S,
) -> (Option<Rc<Client<H>>>, Result<Value, Trouble>)
where
    H: Http + Clone,
    C: Credentials,
    S: Fn(u32) -> W,
    W: Future<Output = ()>,
{
    let call = move |client: Rc<Client<H>>| async move {
        let Call { method, arguments } = call_of(client.account_id(), want);
        let mut answers = client.call(&[CORE, HISTORY], vec![(method, arguments)]).await?;
        answers.take(0)
    };
    let (client, outcome) = ask_with(http, credentials, email, session, HISTORY, kept, call, sleep).await;
    let told = match outcome {
        Outcome::Answered(answer) => Ok(answer),
        Outcome::Unsupported => Err(Trouble::Unsupported),
        Outcome::Failed(_) => Err(Trouble::Failed),
    };
    (client, told)
}

#[cfg(test)]
#[path = "exchanges_tests.rs"]
mod tests;

#[cfg(target_arch = "wasm32")]
pub use run::{frame, start};

/// The browser's part: asking, a few at a time, and putting what came.
#[cfg(target_arch = "wasm32")]
mod run {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, VecDeque};
    use std::rc::Rc;

    use mailcraft_jmap::Client;
    use mailcraft_model::exchanges as data;
    use mailcraft_ui_egui::MailApp;
    use mailcraft_ui_egui::exchanges::{Trouble, Want};
    use serde_json::Value;

    use super::mailbox_of;
    use crate::host::{HostAccount, HostCredentials};

    /// At most this many asks on their way at once (the page's other jobs share the server's two requests).
    const AT_ONCE: usize = 2;

    type Server = Client<crate::fetch::Fetch>;

    /// What an ask came to: the want, and the answer or why not.
    struct Ended {
        want: Want,
        result: Result<Value, Trouble>,
    }

    #[derive(Default)]
    struct State {
        /// The mailboxes handed over: address (lower case), session address.
        handed: Vec<(String, String)>,
        credentials: Option<Rc<HostCredentials>>,
        ctx: Option<egui::Context>,
        /// The signed-in clients, kept from one ask to the next.
        clients: BTreeMap<String, Rc<Server>>,
        queue: VecDeque<Want>,
        running: usize,
        ended: Vec<Ended>,
        told: bool,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    /// Start asking for the mailboxes the page handed over.
    pub fn start(accounts: &[HostAccount], credentials: Rc<HostCredentials>, ctx: egui::Context) {
        let handed: Vec<(String, String)> = accounts.iter().map(|a| (data::key(&a.email), a.session.trim().to_string())).collect();
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

    /// Put what the asks brought, and send the next: called at every frame of a hosting page's window.
    pub fn frame(app: &mut MailApp) {
        STATE.with(|cell| {
            let Ok(mut guard) = cell.try_borrow_mut() else { return };
            let s = &mut *guard;
            if s.handed.is_empty() {
                return;
            }
            // The pane asks the server for more than the window has once a host that reaches it hosts the window; a server that has
            // not the capability says so by its first answer.
            if !s.told {
                s.told = true;
                app.exchanges.set_hosted(true);
            }
            for ended in std::mem::take(&mut s.ended) {
                s.running = s.running.saturating_sub(1);
                put(app, ended);
            }
            s.queue.extend(app.exchanges.take_wants());
            while s.running < AT_ONCE {
                let Some(want) = s.queue.pop_front() else { break };
                start_one(s, want);
            }
        });
    }

    /// Put an answer in the pane.
    fn put(app: &mut MailApp, ended: Ended) {
        let Ended { want, result } = ended;
        match want {
            Want::Someone { address, .. } => {
                let answer = result.and_then(|v| data::read_someone(&v).ok_or(Trouble::Failed));
                app.exchanges.put_someone(&address, answer);
            }
            Want::Linked { address, .. } => {
                let answer = result.and_then(|v| data::read_linked(&v).ok_or(Trouble::Failed));
                app.exchanges.put_linked(&address, answer);
            }
            Want::Page { key, before, .. } => {
                let answer = result.and_then(|v| data::read_page(&v).ok_or(Trouble::Failed));
                app.exchanges.put_page(&key, before.is_some(), answer);
            }
        }
    }

    /// Send one ask, as a task of the window's thread.
    fn start_one(s: &mut State, want: Want) {
        let email = mailbox_of(&want).to_string();
        let session = s.handed.iter().find(|(e, _)| *e == email).map(|(_, session)| session.clone());
        // It counts as on its way until its end is taken in, whether or not it went.
        s.running += 1;
        let (Some(session), Some(credentials)) = (session, s.credentials.clone()) else {
            s.ended.push(Ended { want, result: Err(Trouble::Failed) });
            return;
        };
        let kept = s.clients.get(&email).cloned();
        let ctx = s.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let (client, result) =
                super::ask(crate::fetch::Fetch, &*credentials, &email, &session, kept, &want, |ms| crate::fetch::sleep(ms as i32)).await;
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                match client {
                    Some(client) => {
                        s.clients.insert(email.clone(), client);
                    }
                    None => {
                        s.clients.remove(&email);
                    }
                }
                s.ended.push(Ended { want, result });
            });
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }
}
