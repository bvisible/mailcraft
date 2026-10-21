// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A transport for the browser build, over the WHATWG Fetch Standard: `window.fetch`, `Request`, `Headers`,
//! `AbortController` and the response's `ReadableStream`.
//!
//! The browser-free decisions (what the request is built from, how failures read, the body-size accounting)
//! live in `fetch_rules`, where they are tested natively.

use std::cell::Cell;
use std::rc::Rc;

use js_sys::{Function, Promise, Reflect, Uint8Array};
use mailcraft_jmap::http::{self, Error, Http, Request, Response, Result};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, Headers, ReadableStreamDefaultReader, ReadableStreamReadResult, ReferrerPolicy, RequestCache, RequestCredentials, RequestInit,
    RequestMode, RequestRedirect, Window,
};

use crate::fetch_rules;

/// How long one exchange may take before it is given up, in milliseconds (5 minutes).
pub const GIVE_UP_MS: i32 = 300_000;

/// The browser transport: sends each [`Request`] with `window.fetch`.
///
/// Never sends the page's cookies (`credentials: "omit"`; the client authenticates with its own
/// `Authorization` header), never caches, follows redirects, sends no referrer, and gives up after
/// [`GIVE_UP_MS`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Fetch;

impl Http for Fetch {
    /// Send `req` and return the status, the body (at most `req.max` bytes) and the `Retry-After` delay.
    /// Statuses are not interpreted: a 401 or a 503 comes back as a [`Response`].
    async fn send(&self, req: Request) -> Result<Response> {
        let window = web_sys::window().ok_or_else(|| Error::Io("no window to fetch from".to_owned()))?;
        let controller = AbortController::new().map_err(|_| no_time_limit())?;
        let request = build(&req, &controller)?;
        let deadline = Deadline::start(&window, controller)?;
        let outcome = exchange(&window, &request, req.max).await;
        let timed_out = deadline.finish();
        match outcome {
            // An abort shows up as a rejected fetch or body read; say why it happened.
            Err(Error::Io(_)) if timed_out => Err(fetch_rules::failure(true, None)),
            other => other,
        }
    }
}

/// Read a response's body as bytes, at most `max`: Error::TooLarge once more arrives. Reads the stream chunk by
/// chunk (never the whole body first), and stops reading as soon as the limit is passed.
pub async fn read(resp: &web_sys::Response, max: usize) -> Result<Vec<u8>> {
    let Some(stream) = resp.body() else {
        return Ok(Vec::new());
    };
    let reader = ReadableStreamDefaultReader::new(&stream).map_err(|_| Error::Protocol("the response body cannot be read".to_owned()))?;
    let mut body = Vec::new();
    loop {
        let next = JsFuture::from(reader.read()).await.map_err(|reason| fetch_rules::failure(false, message_of(&reason).as_deref()))?;
        let next: ReadableStreamReadResult = next.unchecked_into();
        if next.get_done() == Some(true) {
            return Ok(body);
        }
        let Ok(chunk) = next.get_value().dyn_into::<Uint8Array>() else {
            stop(&reader).await;
            return Err(Error::Protocol("the response body is not bytes".to_owned()));
        };
        let length = usize::try_from(chunk.length()).unwrap_or(usize::MAX);
        if let Err(too_large) = fetch_rules::add_chunk(body.len(), length, max) {
            stop(&reader).await;
            return Err(too_large);
        }
        body.extend_from_slice(&chunk.to_vec());
    }
}

/// Wait `ms` milliseconds (a setTimeout promise).
///
/// Works in a window and in a worker. Returns at once if the global scope has no `setTimeout`.
pub async fn sleep(ms: i32) {
    let promise = Promise::new(&mut |resolve: Function, _reject: Function| {
        if set_timeout(&resolve, ms).is_err() {
            // Nothing to wait with: do not hang the caller.
            let _ = resolve.call0(&JsValue::UNDEFINED);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// The fetch `Request` for `req`, with its abort signal.
fn build(req: &Request, controller: &AbortController) -> Result<web_sys::Request> {
    let plan = fetch_rules::plan(req);
    let headers = Headers::new().map_err(|_| cannot_build())?;
    for (name, value) in plan.headers {
        // The browser's message may quote the value, which may be a secret: keep a fixed reason.
        headers.append(name, value).map_err(|_| Error::Protocol("the browser refused a request header".to_owned()))?;
    }
    let init = RequestInit::new();
    init.set_method(plan.method);
    init.set_headers(&headers);
    if let Some(body) = plan.body {
        init.set_body(&Uint8Array::from(body));
    }
    init.set_credentials(RequestCredentials::Omit);
    init.set_cache(RequestCache::NoStore);
    init.set_redirect(RequestRedirect::Follow);
    init.set_referrer_policy(ReferrerPolicy::NoReferrer);
    init.set_mode(RequestMode::Cors);
    init.set_signal(Some(&controller.signal()));
    // The browser's message may repeat the address, which may hold a token: keep a fixed reason.
    web_sys::Request::new_with_str_and_init(plan.url, &init).map_err(|_| cannot_build())
}

/// Fetch `request` and read the answer.
async fn exchange(window: &Window, request: &web_sys::Request, max: usize) -> Result<Response> {
    let answer =
        JsFuture::from(window.fetch_with_request(request)).await.map_err(|reason| fetch_rules::failure(false, message_of(&reason).as_deref()))?;
    let resp: web_sys::Response = answer.dyn_into().map_err(|_| Error::Protocol("the browser gave no response".to_owned()))?;
    let now = fetch_rules::unix_seconds(js_sys::Date::now());
    let retry_after = resp.headers().get("Retry-After").ok().flatten().and_then(|value| http::retry_after(&value, now));
    let body = read(&resp, max).await?;
    Ok(Response { status: resp.status(), body, retry_after })
}

/// The time limit of one exchange: aborts it after [`GIVE_UP_MS`]. Dropped early (the caller gave up on the
/// future), it aborts the exchange at once; either way its timer is cleared.
struct Deadline {
    window: Window,
    controller: AbortController,
    timer: i32,
    fired: Rc<Cell<bool>>,
    finished: bool,
    // Kept alive until the timer is cleared, so the browser never calls a freed closure.
    _on_timeout: Closure<dyn FnMut()>,
}

impl Deadline {
    /// Start the timer that aborts `controller`.
    fn start(window: &Window, controller: AbortController) -> Result<Deadline> {
        let fired = Rc::new(Cell::new(false));
        let on_timeout = {
            let fired = Rc::clone(&fired);
            let controller = controller.clone();
            Closure::<dyn FnMut()>::new(move || {
                fired.set(true);
                controller.abort();
            })
        };
        let timer = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(on_timeout.as_ref().unchecked_ref(), GIVE_UP_MS)
            .map_err(|_| no_time_limit())?;
        Ok(Deadline { window: window.clone(), controller, timer, fired, finished: false, _on_timeout: on_timeout })
    }

    /// End of the exchange: clear the timer and tell whether it had fired.
    fn finish(mut self) -> bool {
        self.finished = true;
        self.fired.get()
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        self.window.clear_timeout_with_handle(self.timer);
        if !self.finished {
            self.controller.abort();
        }
    }
}

/// Cancel the body stream: no more of it is downloaded.
async fn stop(reader: &ReadableStreamDefaultReader) {
    // The body is abandoned either way; a failed cancel changes nothing for the caller.
    let _ = JsFuture::from(reader.cancel()).await;
}

/// The message of a rejection: a string, or the `message` of an `Error` or `DOMException`.
fn message_of(reason: &JsValue) -> Option<String> {
    if let Some(text) = reason.as_string() {
        return Some(text);
    }
    Reflect::get(reason, &JsValue::from_str("message")).ok().and_then(|message| message.as_string())
}

/// `setTimeout(handler, ms)` on whatever the global scope is (a window or a worker).
fn set_timeout(handler: &Function, ms: i32) -> std::result::Result<JsValue, JsValue> {
    let global = js_sys::global();
    let set_timeout: Function = Reflect::get(&global, &JsValue::from_str("setTimeout"))?.dyn_into()?;
    set_timeout.call2(&global, handler, &JsValue::from(ms))
}

/// The browser would not build the request (a malformed address or method).
fn cannot_build() -> Error {
    Error::Protocol("the browser could not build the request".to_owned())
}

/// The browser would not give us a timer or an abort controller.
fn no_time_limit() -> Error {
    Error::Io("the browser cannot limit the request's time".to_owned())
}
