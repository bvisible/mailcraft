// //// Neoffice — added file (no upstream equivalent): a JMAP account's Send/Receive and downloads in the browser.

//! A JMAP account's Send/Receive and downloads in the browser: what stands behind
//! [`mailcraft_ui_egui::Services::jmap`] and [`mailcraft_ui_egui::Services::jmap_fetch`].
//!
//! The window hands a job ([`JmapJob`], [`JmapFetchJob`]); the runner signs in to the server
//! ([`Client::connect`]) over the browser's `fetch` and does what the job asks ([`sync_mail`],
//! [`fetch_messages`]), as a task on the window's own thread. What it brings goes to the window through a
//! plain channel and wakes it: the task never waits for the window, which takes a few pieces a frame.
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
//! A job that is stopped (its `cancel`) says so in its answer, whether it was reading or waiting.
// Outside the browser only the tests run it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_jmap::fetch::{failed_fetch, fetch_messages};
use mailcraft_jmap::{Auth, Client, Error, Http, failed_run, sync_mail};
use mailcraft_model::sync::{GraphFetchResult, JmapMailRequest, JmapMailResult};
use mailcraft_ui_egui::jmap_job::{JmapFetchJob, JmapJob};

use crate::credentials::{Credentials, Renewal};

/// How long the runner waits before it asks the host again, when the host had no new credential yet, in
/// milliseconds: one wait after the other, then the job ends.
const ASK_AGAIN_MS: [i32; 3] = [5_000, 10_000, 20_000];

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

/// The browser's transport for the page's jobs, at most [`AT_ONCE`] requests at a time. Clones share the
/// count: one gate for a page.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Default)]
pub struct Gate(std::rc::Rc<std::cell::Cell<usize>>);

/// A request on its way: its place is given back when it is over, however it ended.
#[cfg(target_arch = "wasm32")]
struct Place(std::rc::Rc<std::cell::Cell<usize>>);

#[cfg(target_arch = "wasm32")]
impl Drop for Place {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

#[cfg(target_arch = "wasm32")]
impl Http for Gate {
    async fn send(&self, req: mailcraft_jmap::Request) -> mailcraft_jmap::Result<mailcraft_jmap::Response> {
        // One thread: nothing comes between the look and the taking.
        while self.0.get() >= AT_ONCE {
            crate::fetch::sleep(25).await;
        }
        self.0.set(self.0.get().saturating_add(1));
        let _place = Place(self.0.clone());
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
    let request = &job.request;
    // The host's credential when the job carries none: the host may renew it, the owner's it can't.
    let host = if job.secret.is_some() { None } else { credentials };
    let auth = match &job.secret {
        Some(secret) => Some(own_auth(request.bearer, &request.email, secret)),
        None => host.and_then(|c| c.auth(&request.email)),
    };
    let Some(mut auth) = auth else {
        emit(failed_run(request, &no_credential()));
        return;
    };
    let mut renewed = false;
    loop {
        if job.cancel.load(Ordering::Relaxed) {
            emit(stopped(request));
            return;
        }
        let client = match Client::connect(http.clone(), &request.session, &auth).await {
            Ok(client) => client,
            Err(Error::Auth(why)) if !renewed && host.is_some() => match renew(host, &request.email, &job.cancel, &sleep).await {
                Renewed::Fresh(fresh) => {
                    (auth, renewed) = (fresh, true);
                    continue;
                }
                Renewed::Cancelled => {
                    emit(stopped(request));
                    return;
                }
                Renewed::Refused => {
                    emit(failed_run(request, &Error::Auth(why)));
                    return;
                }
            },
            Err(e) => {
                emit(failed_run(request, &e));
                return;
            }
        };
        // The last piece of a run whose credential the server turned down is held back while the host is
        // asked for another: the run is read again with it, and what it took in before is taken in again,
        // which changes nothing.
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
            Renewed::Fresh(fresh) => (auth, renewed) = (fresh, true),
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
    let Some(mut auth) = auth else { return failed_fetch(request, &no_credential()) };
    let mut renewed = false;
    loop {
        match Client::connect(http.clone(), &request.session, &auth).await {
            Ok(client) => return fetch_messages(&client, request, &job.cancel).await,
            Err(Error::Auth(why)) if !renewed && host.is_some() => match renew(host, &request.email, &job.cancel, &sleep).await {
                Renewed::Fresh(fresh) => (auth, renewed) = (fresh, true),
                Renewed::Cancelled | Renewed::Refused => return failed_fetch(request, &Error::Auth(why)),
            },
            Err(e) => return failed_fetch(request, &e),
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
                Some(ms) => sleep(*ms).await,
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

#[cfg(test)]
#[path = "jmap_services_tests.rs"]
mod tests;
