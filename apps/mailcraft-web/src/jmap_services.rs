// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a JMAP account's Send/Receive and downloads in the browser.

//! A JMAP account's Send/Receive, downloads and calendars in the browser: what stands behind
//! [`mailcraft_ui_egui::Services::jmap`], [`mailcraft_ui_egui::Services::jmap_fetch`] and
//! [`mailcraft_ui_egui::Services::jmap_pim`].
//!
//! The window hands a job ([`JmapJob`], [`JmapFetchJob`], [`JmapPimJob`]); the runner signs in to the server
//! ([`Client::connect`], [`signed_in`]) over the browser's `fetch` and does what the job asks ([`sync_mail`],
//! [`fetch_messages`], [`sync_pim`]), as a task on the window's own thread. What it brings goes to the window through
//! a plain channel and wakes it: the task never waits for the window, which takes a few pieces a frame.
//!
//! **The credential** is the job's own (a password the owner typed, or a bearer token), or the host's when
//! the host holds it ([`Credentials`]). A server that refuses the host's credential, at sign-in or during
//! a Send/Receive, gets a new one from the host, and the job signs in again with it, once. A host that has
//! none yet is asked again a few seconds later, three times, before the job ends with the server's refusal.
//! A job with the owner's own credential ends with the refusal at once: the window asks the owner again.
//!
//! **At most two requests at once** go to the servers, all the page's jobs together ([`AT_ONCE`]), and a
//! request that gets no answer is given up after a while (the transport's): a slow server holds a job up,
//! never the page.
//!
//! **A run that is read again** (the server turned the credential down in the middle of it, and the host
//! gave another) only reads: what the first run sent, held or filed went up already (its pieces said so), and
//! what it did not reach (a message, a draft, a change) is the next run's, so that asking the same question
//! twice never sends a message or files a draft a second time.
//!
//! **A calendar job** only reads: one whose credential the server turns down in the middle is read again, once, with
//! the host's new one. A server whose API is the Neoffice facade's (its method path, [`FACADE_API`]) is one that keeps
//! only the basics of an event, whether or not its session says what it writes.
//!
//! A job that is stopped (its `cancel`) says so in its answer, whether it was reading or waiting.
// Outside the browser only the tests run it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_jmap::fetch::{failed_fetch, fetch_messages};
use mailcraft_jmap::{Auth, Client, Error, Http, failed_pim, failed_run, sync_mail, sync_pim};
use mailcraft_model::sync::{GraphFetchResult, JmapMailRequest, JmapMailResult, JmapPimRequest, JmapPimResult};
use mailcraft_ui_egui::jmap_job::{JmapFetchJob, JmapJob};
use mailcraft_ui_egui::jmap_pim::JmapPimJob;

use crate::credentials::{Credentials, Renewal};

/// How long the runner waits before it asks the host again, when the host had no new credential yet, in
/// milliseconds: one wait after the other, then the job ends.
const ASK_AGAIN_MS: [i32; 3] = [5_000, 10_000, 20_000];
/// Those waits are made of slices this long, between which a stop is looked at.
const WAIT_SLICE_MS: i32 = 500;

/// The path the API of the Neoffice facade is served at (`frappe_webmail`'s method): a server there keeps only the
/// basics of an event, said or not.
pub const FACADE_API: &str = "/api/method/frappe_webmail.mailcraft_jmap.api";

/// The most requests on their way to the servers at once, all the page's jobs together. A server says how
/// many it takes (RFC 8620 `maxConcurrentRequests`); two is within what servers commonly take, and keeps a
/// download from waiting long behind a Send/Receive.
pub const AT_ONCE: usize = 2;

/// `Services::jmap` for the browser: each job runs as a task of the window's thread, over `fetch`, with the
/// host's credentials when it holds them, through the page's `gate`.
#[cfg(target_arch = "wasm32")]
pub fn jmap_service<C: Credentials + 'static>(
    ctx: egui::Context,
    credentials: std::rc::Rc<C>,
    gate: Gate,
) -> Box<dyn Fn(JmapJob) -> std::sync::mpsc::Receiver<JmapMailResult>> {
    Box::new(move |job: JmapJob| {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ctx, credentials, gate) = (ctx.clone(), credentials.clone(), gate.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let mut emit = |piece: JmapMailResult| {
                // A window that has gone has nobody to take it to.
                let _ = tx.send(piece);
                ctx.request_repaint();
            };
            run(gate, &job, Some(credentials.as_ref()), crate::fetch::sleep, &mut emit).await;
        });
        rx
    })
}

/// `Services::jmap_fetch` for the browser, as [`jmap_service`]: one answer for each job.
#[cfg(target_arch = "wasm32")]
pub fn jmap_fetch_service<C: Credentials + 'static>(
    ctx: egui::Context,
    credentials: std::rc::Rc<C>,
    gate: Gate,
) -> Box<dyn Fn(JmapFetchJob) -> std::sync::mpsc::Receiver<GraphFetchResult>> {
    Box::new(move |job: JmapFetchJob| {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ctx, credentials, gate) = (ctx.clone(), credentials.clone(), gate.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let answer = fetch(gate, &job, Some(credentials.as_ref()), crate::fetch::sleep).await;
            let _ = tx.send(answer);
            ctx.request_repaint();
        });
        rx
    })
}

/// `Services::jmap_pim` for the browser, as [`jmap_service`]: one answer for each job, through the same `gate` as the
/// page's mail, so that a calendar job queues with the mail and never beside it.
#[cfg(target_arch = "wasm32")]
pub fn jmap_pim_service<C: Credentials + 'static>(
    ctx: egui::Context,
    credentials: std::rc::Rc<C>,
    gate: Gate,
) -> Box<dyn Fn(JmapPimJob) -> std::sync::mpsc::Receiver<JmapPimResult>> {
    Box::new(move |job: JmapPimJob| {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ctx, credentials, gate) = (ctx.clone(), credentials.clone(), gate.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let answer = pim(gate, &job, Some(credentials.as_ref()), crate::fetch::sleep).await;
            let _ = tx.send(answer);
            ctx.request_repaint();
        });
        rx
    })
}

/// The browser's transport for the page's jobs, at most [`AT_ONCE`] requests at a time, in the order they
/// came: a job that has just had its answer queues behind the ones already waiting. Clones share the queue:
/// one gate for a page.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Default)]
pub struct Gate(std::rc::Rc<std::cell::RefCell<Queue>>);

/// The requests on their way, and the tickets of those waiting, oldest first.
#[derive(Default)]
pub struct Queue {
    going: usize,
    waiting: std::collections::VecDeque<u64>,
    next: u64,
}

impl Queue {
    /// A ticket for a request that comes now.
    fn take(&mut self) -> u64 {
        let ticket = self.next;
        self.next = self.next.wrapping_add(1);
        self.waiting.push_back(ticket);
        ticket
    }

    /// Whether the request with `ticket` may go now: its turn has come and there is room. It is then on its way.
    fn go(&mut self, ticket: u64) -> bool {
        if self.going >= AT_ONCE || self.waiting.front() != Some(&ticket) {
            return false;
        }
        self.waiting.pop_front();
        self.going = self.going.saturating_add(1);
        true
    }

    /// A request is over, or one that waited gave up: its place, or its ticket, goes.
    fn leave(&mut self, ticket: u64, went: bool) {
        if went {
            self.going = self.going.saturating_sub(1);
        } else {
            self.waiting.retain(|t| *t != ticket);
        }
    }
}

/// A request's ticket: its place in the queue, or on its way, is given back however it ends.
#[cfg(target_arch = "wasm32")]
struct Ticket {
    queue: std::rc::Rc<std::cell::RefCell<Queue>>,
    number: u64,
    went: bool,
}

#[cfg(target_arch = "wasm32")]
impl Drop for Ticket {
    fn drop(&mut self) {
        self.queue.borrow_mut().leave(self.number, self.went);
    }
}

#[cfg(target_arch = "wasm32")]
impl Http for Gate {
    async fn send(&self, req: mailcraft_jmap::Request) -> mailcraft_jmap::Result<mailcraft_jmap::Response> {
        let number = self.0.borrow_mut().take();
        let mut ticket = Ticket { queue: self.0.clone(), number, went: false };
        // One thread: nothing comes between the look and the taking, and no borrow is held while waiting.
        while !self.0.borrow_mut().go(number) {
            crate::fetch::sleep(25).await;
        }
        ticket.went = true;
        crate::fetch::Fetch.send(req).await
    }
}

/// One Send/Receive of a JMAP account: sign in, read what `job` asks, and hand every piece to `emit`, the
/// last one included (the only one that isn't `partial`). `sleep` waits so many milliseconds without holding
/// the thread.
pub async fn run<H, C, S, W>(http: H, job: &JmapJob, credentials: Option<&C>, sleep: S, emit: &mut dyn FnMut(JmapMailResult))
where
    H: Http + Clone,
    C: Credentials,
    S: Fn(i32) -> W,
    W: Future<Output = ()>,
{
    // The host's credential when the job carries none: the host may renew it, the owner's it can't.
    let host = if job.secret.is_some() { None } else { credentials };
    let auth = match &job.secret {
        Some(secret) => Some(own_auth(job.request.bearer, &job.request.email, secret)),
        None => host.and_then(|c| c.auth(&job.request.email)),
    };
    let Some(mut auth) = auth else {
        emit(failed_run(&job.request, &no_credential()));
        return;
    };
    let mut renewed = false;
    // The request of a run read again after a refusal: the reading part of the job's alone.
    let mut reread: Option<JmapMailRequest> = None;
    loop {
        let request = reread.as_ref().unwrap_or(&job.request);
        let client = match signed_in(&http, &request.session, &request.email, auth, renewed, host, &job.cancel, &sleep).await {
            SignedIn::In(client, now_renewed) => {
                renewed = now_renewed;
                client
            }
            SignedIn::Stopped => {
                emit(stopped(request));
                return;
            }
            SignedIn::Out(e) => {
                emit(failed_run(request, &e));
                return;
            }
        };
        // The last piece of a run whose credential the server turned down is held back while the host is
        // asked for another: the run is read again with it, and what it read before is read again, which
        // changes nothing. What it sent or filed is not asked again (below).
        let may_renew = !renewed && host.is_some();
        let mut refused = None;
        sync_mail(&client, request, &job.cancel, &mut |piece: JmapMailResult| {
            if may_renew && !piece.partial && piece.auth_refused {
                refused = Some(piece);
            } else {
                emit(piece);
            }
        })
        .await;
        let Some(mut last) = refused else { return };
        match renew(host, &request.email, &job.cancel, &sleep).await {
            Renewed::Fresh(fresh) => {
                (auth, renewed) = (fresh, true);
                // Read again, and only read: what the run sent, filed or changed before the refusal went up already
                // (its pieces said so), and what didn't is the next run's, never this one's a second time.
                reread = Some(JmapMailRequest { changes: Vec::new(), sends: Vec::new(), drafts: Vec::new(), ..request.clone() });
            }
            Renewed::Cancelled => {
                (last.auth_refused, last.stopped, last.whole) = (false, true, false);
                emit(last);
                return;
            }
            Renewed::Refused => {
                emit(last);
                return;
            }
        }
    }
}

/// One download of messages of a JMAP account whole: sign in and fetch what `job` names. A credential the
/// server refuses at sign-in is renewed as a Send/Receive's is; one that expires during the batch stops it,
/// and the next batch signs in again.
pub async fn fetch<H, C, S, W>(http: H, job: &JmapFetchJob, credentials: Option<&C>, sleep: S) -> GraphFetchResult
where
    H: Http + Clone,
    C: Credentials,
    S: Fn(i32) -> W,
    W: Future<Output = ()>,
{
    let request = &job.request;
    let host = if job.secret.is_some() { None } else { credentials };
    let auth = match &job.secret {
        Some(secret) => Some(own_auth(request.bearer, &request.email, secret)),
        None => host.and_then(|c| c.auth(&request.email)),
    };
    let Some(auth) = auth else { return failed_fetch(request, &no_credential()) };
    match signed_in(&http, &request.session, &request.email, auth, false, host, &job.cancel, &sleep).await {
        SignedIn::In(client, ..) => fetch_messages(&client, request, &job.cancel).await,
        // Stopped: nothing fetched, and nothing to hold against the server.
        SignedIn::Stopped => GraphFetchResult { account: request.account, email: request.email.clone(), ..Default::default() },
        SignedIn::Out(e) => failed_fetch(request, &e),
    }
}

/// One calendar job of a JMAP account: sign in, read what `job` asks ([`sync_pim`]), and give the answer. It only
/// reads, so a credential the server turns down in the middle of it is renewed by the host, once, and the job read
/// again; a server whose API is the facade's is taken for one that keeps only the basics ([`FACADE_API`]).
pub async fn pim<H, C, S, W>(http: H, job: &JmapPimJob, credentials: Option<&C>, sleep: S) -> JmapPimResult
where
    H: Http + Clone,
    C: Credentials,
    S: Fn(i32) -> W,
    W: Future<Output = ()>,
{
    let request = &job.request;
    let host = if job.secret.is_some() { None } else { credentials };
    let auth = match &job.secret {
        Some(secret) => Some(own_auth(request.bearer, &request.email, secret)),
        None => host.and_then(|c| c.auth(&request.email)),
    };
    let Some(mut auth) = auth else { return failed_pim(request, &no_credential()) };
    let mut renewed = false;
    loop {
        let client = match signed_in(&http, &request.session, &request.email, auth, renewed, host, &job.cancel, &sleep).await {
            SignedIn::In(client, now_renewed) => {
                renewed = now_renewed;
                client
            }
            SignedIn::Stopped => return stopped_pim(request),
            SignedIn::Out(e) => return failed_pim(request, &e),
        };
        let asked = JmapPimRequest { plain_unless_said: client.api_path_ends_with(FACADE_API), ..request.clone() };
        let answer = sync_pim(&client, &asked, &job.cancel).await;
        if !answer.auth_refused || renewed || host.is_none() {
            return answer;
        }
        // Turned down in the middle: a new credential from the host, and the job read again with it.
        match renew(host, &request.email, &job.cancel, &sleep).await {
            Renewed::Fresh(fresh) => (auth, renewed) = (fresh, true),
            Renewed::Cancelled => return stopped_pim(request),
            Renewed::Refused => return answer,
        }
    }
}

/// What came of signing in to a server ([`signed_in`]).
enum SignedIn<H> {
    /// Signed in, and whether with a credential the host renewed.
    In(Box<Client<H>>, bool),
    /// The job was stopped, before or while it waited for the host.
    Stopped,
    /// The server can't be had, or refused the credential for good.
    Out(Error),
}

/// Sign in to the session at `session` with `auth`. A credential the server refuses is renewed by the `host`, once
/// (none when `renewed` already), and the sign-in tried again with the new one; the owner's own credential is never
/// renewed (the job carries it, and `host` is `None`). The stop is looked at before each try.
#[allow(clippy::too_many_arguments)]
async fn signed_in<H, C, S, W>(
    http: &H,
    session: &str,
    email: &str,
    mut auth: Auth,
    mut renewed: bool,
    host: Option<&C>,
    cancel: &AtomicBool,
    sleep: &S,
) -> SignedIn<H>
where
    H: Http + Clone,
    C: Credentials,
    S: Fn(i32) -> W,
    W: Future<Output = ()>,
{
    loop {
        if cancel.load(Ordering::Relaxed) {
            return SignedIn::Stopped;
        }
        match Client::connect(http.clone(), session, &auth).await {
            Ok(client) => return SignedIn::In(Box::new(client), renewed),
            Err(Error::Auth(why)) if !renewed && host.is_some() => match renew(host, email, cancel, sleep).await {
                Renewed::Fresh(fresh) => (auth, renewed) = (fresh, true),
                Renewed::Cancelled => return SignedIn::Stopped,
                Renewed::Refused => return SignedIn::Out(Error::Auth(why)),
            },
            Err(e) => return SignedIn::Out(e),
        }
    }
}

/// What came of asking the host for a new credential.
enum Renewed {
    Fresh(Auth),
    /// The job was stopped while it waited.
    Cancelled,
    /// The host has none to give: the server's refusal stands.
    Refused,
}

/// A new credential for the account `email` from the host, asking again a little later while it has none yet.
async fn renew<C, S, W>(host: Option<&C>, email: &str, cancel: &AtomicBool, sleep: &S) -> Renewed
where
    C: Credentials,
    S: Fn(i32) -> W,
    W: Future<Output = ()>,
{
    let Some(host) = host else { return Renewed::Refused };
    let mut waits = ASK_AGAIN_MS.iter();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Renewed::Cancelled;
        }
        match host.renew(email).await {
            Renewal::Given(auth) => return Renewed::Fresh(auth),
            Renewal::Reload => return Renewed::Refused,
            Renewal::Later => match waits.next() {
                // In slices, so that a stop is not kept waiting.
                Some(ms) => {
                    let mut left = *ms;
                    while left > 0 && !cancel.load(Ordering::Relaxed) {
                        sleep(left.min(WAIT_SLICE_MS)).await;
                        left -= WAIT_SLICE_MS;
                    }
                }
                None => return Renewed::Refused,
            },
        }
    }
}

/// The job's own secret: a bearer token, or a password, sent with the account's address as its login name
/// (the request names no other; JMAP servers sign their users in by their addresses as a rule).
fn own_auth(bearer: bool, email: &str, secret: &str) -> Auth {
    if bearer { Auth::bearer(secret) } else { Auth::basic(email, secret) }
}

/// No credential for the account at all: what the server would have said of a wrong one.
fn no_credential() -> Error {
    Error::Auth("there is no credential for this account".into())
}

/// The last piece of a run that was stopped before it read anything.
fn stopped(request: &JmapMailRequest) -> JmapMailResult {
    JmapMailResult { account: request.account, email: request.email.clone(), stopped: true, ..Default::default() }
}

/// The answer to a calendar job that was stopped before it read anything.
fn stopped_pim(request: &JmapPimRequest) -> JmapPimResult {
    JmapPimResult { account: request.account, email: request.email.clone(), stopped: true, ..Default::default() }
}

#[cfg(test)]
#[path = "jmap_services_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "jmap_pim_flow_tests.rs"]
mod pim_flow_tests;
