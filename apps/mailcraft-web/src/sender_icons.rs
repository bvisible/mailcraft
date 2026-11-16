// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the icons of senders' domains, asked of the Neoffice server, in the browser.

//! The round of a row of a hosting page's window shows the icon of its sender's site where the Neoffice server finds one, else
//! the sender's initials. The window (`mailcraft_ui_egui::sender_images`) decides whose icon is wanted, by its guards (a sender the
//! person knows or whose pictures they chose to download, never Junk E-mail, never a free mail service's domain), and hands over
//! the site alone (`example.com` for `news.example.com`); this file asks it of the server, one at a time through the first mailbox the page handed over, under a capability of the server's
//! own, [`ICONS`] (`frappe_webmail`'s `neomail_icons`: `NeomailSenderIcon/get {accountId, domain}`), and hands back its bytes or
//! nothing.
//!
//! **The browser asks no third party**: the server asks the domain's own site (its `apple-touch-icon.png`, then its `favicon.ico`)
//! under its own rules, and keeps what it found for a week for everyone of the instance. **What is sent** is a domain; **what comes
//! back** is a picture's bytes in base64, at most [`MAX_BYTES`], which the window decodes within its caps. A server without the
//! capability, or any trouble, leaves the initials.
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::future::Future;
use std::rc::Rc;

use base64::Engine as _;
use mailcraft_jmap::{CORE, Client, Http};
use serde_json::{Value, json};

use crate::credentials::Credentials;
use crate::exchanges::{Outcome, ask_with};

/// The capability: a data type of the Neoffice server (frappe_webmail's `neomail_icons`), named by it.
pub const ICONS: &str = "https://neoffice.me/jmap/neomail-sender-icons";
pub const ICON: &str = "NeomailSenderIcon/get";
/// What an answer says of a server without the capability (or the method): nothing more is asked of it.
pub const UNSUPPORTED: &str = "this server finds no icons";
/// The most bytes of an icon taken: the server keeps none larger.
pub const MAX_BYTES: usize = 100 * 1024;

/// The arguments of the ask for `domain`'s icon, for the mailbox whose JMAP account is `account`.
pub fn arguments(account: &str, domain: &str) -> Value {
    json!({"accountId": account, "domain": domain})
}

/// The bytes of the icon the server's answer `v` gives for `domain`, or why there are none.
pub fn read(v: &Value, domain: &str) -> Result<Vec<u8>, String> {
    if v.get("domain").and_then(Value::as_str) != Some(domain) {
        return Err("the answer is about another domain".into());
    }
    let Some(data) = v.get("data").and_then(Value::as_str) else { return Err("the server found no icon".into()) };
    // (Its base64 at most a third longer than the bytes it may stand for, and a little.)
    if data.len() > MAX_BYTES / 3 * 4 + 8 {
        return Err("the icon is too large".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|_| "the icon can't be read".to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("the icon is empty or too large".into());
    }
    Ok(bytes)
}

/// One ask of the server for `domain`'s icon, through the client `kept` from an earlier ask or a new one signed in with the page's
/// credential for the mailbox `email` (see [`ask_with`], which settles a renewed credential and a passing trouble).
pub async fn ask<H, C, S, W>(
    http: H,
    credentials: &C,
    email: &str,
    session: &str,
    kept: Option<Rc<Client<H>>>,
    domain: &str,
    sleep: S,
) -> (Option<Rc<Client<H>>>, Result<Vec<u8>, String>)
where
    H: Http + Clone,
    C: Credentials,
    S: Fn(u32) -> W,
    W: Future<Output = ()>,
{
    let call = move |client: Rc<Client<H>>| async move {
        let arguments = arguments(client.account_id(), domain);
        let mut answers = client.call(&[CORE, ICONS], vec![(ICON, arguments)]).await?;
        answers.take(0)
    };
    let (client, outcome) = ask_with(http, credentials, email, session, ICONS, kept, call, sleep).await;
    let told = match outcome {
        Outcome::Answered(answer) => read(&answer, domain),
        Outcome::Unsupported => Err(UNSUPPORTED.to_string()),
        Outcome::Failed(_) => Err("the server could not be asked".to_string()),
    };
    (client, told)
}

#[cfg(test)]
#[path = "sender_icons_tests.rs"]
mod tests;

#[cfg(target_arch = "wasm32")]
pub use run::service;

/// The browser's part: the window's look-ups, one at a time, as tasks of its thread.
#[cfg(target_arch = "wasm32")]
mod run {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::sync::Weak;
    use std::sync::mpsc::{self, Receiver, Sender};

    use mailcraft_jmap::Client;

    use crate::host::{HostAccount, HostCredentials};

    type Server = Client<crate::fetch::Fetch>;
    type Answer = Result<Vec<u8>, String>;
    type LookUp = Box<dyn Fn(String, Weak<()>) -> Receiver<Answer>>;

    /// The look-ups waiting, the one on its way, and what is kept from one to the next.
    struct State {
        email: String,
        session: String,
        credentials: Rc<HostCredentials>,
        ctx: egui::Context,
        waiting: VecDeque<(String, Weak<()>, Sender<Answer>)>,
        running: bool,
        /// The signed-in client.
        kept: Option<Rc<Server>>,
        /// The server said it finds no icons: nothing more is asked of it.
        unsupported: bool,
    }

    /// The window's look-up of a site's icon (`Services::sender_images`), through the first mailbox the page handed over (the server
    /// is the same for all), one at a time, so that a window takes one of the server's workers at most; `None` where it handed over
    /// none.
    pub fn service(accounts: &[HostAccount], credentials: Rc<HostCredentials>, ctx: egui::Context) -> Option<LookUp> {
        let first = accounts.first()?;
        let state = Rc::new(RefCell::new(State {
            email: first.email.trim().to_ascii_lowercase(),
            session: first.session.trim().to_string(),
            credentials,
            ctx,
            waiting: VecDeque::new(),
            running: false,
            kept: None,
            unsupported: false,
        }));
        Some(Box::new(move |site: String, alive: Weak<()>| {
            let (tx, rx) = mpsc::channel();
            let unsupported = state.borrow().unsupported;
            if unsupported {
                let _ = tx.send(Err(super::UNSUPPORTED.to_string()));
            } else {
                state.borrow_mut().waiting.push_back((site, alive, tx));
                next(&state);
            }
            rx
        }))
    }

    /// Send the next look-up, when none is on its way.
    fn next(state: &Rc<RefCell<State>>) {
        let mut s = state.borrow_mut();
        if s.running {
            return;
        }
        // (One the window no longer wants is let go: its pictures were let go of, or turned off.)
        let Some((site, _, tx)) = std::iter::from_fn(|| s.waiting.pop_front()).find(|(_, alive, _)| alive.upgrade().is_some()) else { return };
        s.running = true;
        let (email, session, credentials, kept) = (s.email.clone(), s.session.clone(), s.credentials.clone(), s.kept.take());
        drop(s);
        let state = state.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let (client, told) =
                super::ask(crate::fetch::Fetch, &*credentials, &email, &session, kept, &site, |ms| crate::fetch::sleep(ms as i32)).await;
            {
                let mut s = state.borrow_mut();
                s.kept = client;
                s.running = false;
                if told.as_ref().is_err_and(|why| why == super::UNSUPPORTED) {
                    s.unsupported = true;
                    for (_, _, tx) in s.waiting.drain(..) {
                        let _ = tx.send(Err(super::UNSUPPORTED.to_string()));
                    }
                }
                let _ = tx.send(told);
                s.ctx.request_repaint();
            }
            next(&state);
        });
    }
}
