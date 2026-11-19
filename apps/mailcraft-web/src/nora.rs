// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): how the browser build reaches Nora, the assistant of the Neoffice server.

//! How the browser build reaches Nora ([`mailcraft_ui_egui::Services::nora`]).
//!
//! The hosting page turns her on: with `?nora` in the build's address (`/neomail` adds it when the
//! instance has Nora), or with a function `window.mailcraftNoraCall(method, argsJson)`, on the page or
//! on the page of the same origin that frames it, a promise of `{ status, body }`. Neodesk's window
//! offers the function: its shell makes the call with the person's credential, which neither its page
//! nor MailCraft ever holds. When the function is there, every call goes through it.
//!
//! Otherwise a call is a `fetch` of `POST <origin>/api/method/<method>`, its arguments as a JSON body,
//! with the page's own session (`credentials: same-origin`: the desk's cookie, to the page's own origin
//! only) and its CSRF token (`window.frappe.csrf_token`, else `window.csrf_token`, of the page or of the
//! same-origin page that frames it) as `X-Frappe-CSRF-Token`. The calls go to the page's own origin, or
//! to a loopback one that a loopback page names (the workshop's stand-in server), and the token only to
//! the page's own origin ([`crate::nora_origin`]): an address can't send them anywhere else. Nothing
//! secret travels in the address.
//!
//! Only the six methods the window calls go out ([`METHODS`]), and the seven of Nora's conversation that
//! her pane calls ([`CHAT_METHODS`], craft-nora's protocol, as the editors' panes do), and the two of her ready replies
//! ([`READY_METHODS`]), these only where they can go ([`chat`]: the page's own `fetch`, or a host that says it takes them), and
//! the nineteen of « Trier avec Nora » ([`SORTING_METHODS`], where they can go: [`sorting`]), each with a plain JSON
//! object of arguments without a `cmd` key, within 8 MB (what Neodesk's shell takes). An answer is read up to
//! [`MAX_ANSWER`] bytes, its text as UTF-8 with what isn't replaced; a call is given up after
//! [`GIVE_UP_MS`]. A call that got no answer comes back with status 0 and why, which the window says.
//!
//! A dictation in her pane ([`DICTATION`], where it can go: [`dictation`]) is the one call that is no JSON at the server: a form with
//! the recording as a file and the language ([`dictation_form`]), made here for the page's own `fetch`, with the same session and
//! token, and by the host where the host makes the calls. The recording goes only to where her other calls go.

// Outside the browser only the tests use what is here.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use base64::Engine as _;
use mailcraft_ui_egui::NoraCall;
use mailcraft_ui_egui::nora::{CHAT_METHODS, DICTATION, MAX_DICTATION, METHODS, PageMailbox, READY_METHODS, SORTING_METHODS};

use crate::hosted::HostAccount;

/// How long a call is waited for, in milliseconds: past the 180 s after which Neodesk's shell answers
/// by itself, and before the window gives up (200 s).
pub const GIVE_UP_MS: i32 = 195_000;
/// The largest arguments sent, in bytes: what Neodesk's shell takes.
const MAX_ARGS: usize = 8 << 20;
/// The longest value of `?nora` read, in characters.
const MAX_PARAM: usize = 2048;

/// The value of `nora` in the page's query (`location.search`): `""` for a bare `?nora`, `None` when the
/// query has none. Read as a form is (`+` is a space, `%XX` a byte); the first `nora` counts.
pub fn param(search: &str) -> Option<String> {
    first(search.strip_prefix('?').unwrap_or(search), "nora")
}

/// The first value of `key` in a query (what follows `?`, without it), decoded as a form is, at most
/// [`MAX_PARAM`] characters.
fn first(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|item| {
            let (k, value) = item.split_once('=').unwrap_or((item, ""));
            (decoded(k) == key).then(|| decoded(value))
        })
        .map(|value| value.chars().take(MAX_PARAM).collect())
}

/// A part of a query, decoded as a form is.
fn decoded(text: &str) -> String {
    let hex = |b: Option<&u8>| b.and_then(|b| char::from(*b).to_digit(16)).and_then(|d| u8::try_from(d).ok());
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match (b, hex(bytes.get(i + 1)), hex(bytes.get(i + 2))) {
            (b'%', Some(high), Some(low)) => {
                out.push(high * 16 + low);
                i += 3;
            }
            (b'+', ..) => {
                out.push(b' ');
                i += 1;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A mailbox the hosting page handed over, as Nora's server knows it: by the name of its account there, which
/// stays the address the account was made with when its address changes. That name is the `account` of its
/// session's address (the facade's session is `…/frappe_webmail.mailcraft_jmap.session?account=<name>`, as
/// `/neomail` and Neodesk's shell hand it over: the account the ids of its messages come from), read as the
/// server reads it; else the one the page says (`account`); a page that says neither has the address taken for it.
pub fn handed_mailbox(handed: &HostAccount) -> PageMailbox {
    let from_session = || {
        let query = handed.session.split('#').next()?.split_once('?')?.1;
        first(query, "account").map(|name| name.trim().to_string())
    };
    let said = || Some(handed.account.trim().chars().take(MAX_PARAM).collect::<String>());
    let name = from_session().filter(|n| !n.is_empty()).or_else(said).filter(|n| !n.is_empty());
    PageMailbox { address: handed.email.clone(), name: name.unwrap_or_else(|| handed.email.trim().to_string()), shared: handed.shared }
}

/// The body a call goes with, when it may go out at all: one of the six methods, of the seven of her
/// conversation (her pane's, [`CHAT_METHODS`]) or of the two of her ready replies ([`READY_METHODS`]), a plain object of
/// arguments without a `cmd` key, within 8 MB; or a dictation ([`DICTATION`]), its arguments those of a recording.
pub fn allowed(call: &NoraCall) -> Result<String, String> {
    let listed = |list: &[&str]| list.contains(&call.method);
    let dictation = call.method == DICTATION;
    if !dictation && !listed(&METHODS) && !listed(&CHAT_METHODS) && !listed(&READY_METHODS) && !listed(&SORTING_METHODS) {
        return Err("that is not one of Nora's methods".into());
    }
    if dictation {
        dictation_form(&call.args)?;
    }
    let Some(args) = call.args.as_object() else { return Err("the arguments are not a plain object".into()) };
    if args.contains_key("cmd") {
        return Err("the arguments name a command".into());
    }
    let body = call.args.to_string();
    if body.len() > MAX_ARGS {
        return Err("the arguments are too large".into());
    }
    Ok(body)
}

/// What the form of a dictation carries (the server's `nora.api.stt.transcribe` takes a form, not JSON).
#[derive(Debug, PartialEq, Eq)]
pub struct DictationForm {
    /// The recording, its type, and a file name with the extension the server reads the type by.
    pub audio: Vec<u8>,
    pub mime: String,
    pub file_name: String,
    /// The language spoken, two letters.
    pub language: String,
}

/// The kinds of recording a browser makes (`MediaRecorder`), by the extension the server reads them by.
const AUDIO_KINDS: [(&str, &str); 5] =
    [("audio/webm", "webm"), ("audio/ogg", "ogg"), ("audio/mp4", "m4a"), ("audio/mpeg", "mp3"), ("audio/wav", "wav")];

/// The form of a dictation from its arguments: `audio_base64`, `mime` and `language` (two lowercase letters), and nothing else.
pub fn dictation_form(args: &serde_json::Value) -> Result<DictationForm, String> {
    let refused = |why: &str| format!("the dictation is refused: {why}");
    let Some(args) = args.as_object() else { return Err(refused("its arguments are not a plain object")) };
    if args.len() != 3 {
        return Err(refused("its arguments are not the three of a recording"));
    }
    let text = |key: &str| args.get(key).and_then(serde_json::Value::as_str).ok_or_else(|| refused("an argument is missing"));
    let (audio, mime, language) = (text("audio_base64")?, text("mime")?, text("language")?);
    if audio.is_empty() || audio.len() > MAX_DICTATION {
        return Err(refused("the recording is empty or too long"));
    }
    let kind = mime.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let Some((mime, extension)) = AUDIO_KINDS.iter().find(|(known, _)| *known == kind) else { return Err(refused("that is no recording")) };
    if language.len() != 2 || !language.bytes().all(|b| b.is_ascii_lowercase()) {
        return Err(refused("the language is not two letters"));
    }
    let audio = base64::engine::general_purpose::STANDARD.decode(audio).map_err(|_| refused("the recording can't be read"))?;
    Ok(DictationForm { audio, mime: (*mime).to_string(), file_name: format!("dictation.{extension}"), language: language.to_string() })
}

/// Where a call to `method` goes: under `origin`, or a path of the page's own (`origin` empty).
pub fn url(origin: &str, method: &str) -> String {
    format!("{}/api/method/{method}", origin.trim_end_matches('/'))
}

/// The HTTP status a host said: 0 for anything that is no status.
pub fn status(said: Option<f64>) -> u16 {
    said.filter(|s| s.is_finite() && (0.0..1000.0).contains(s)).map_or(0, |s| s as u16)
}

/// The text of an answer's bytes: UTF-8, what isn't replaced.
pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Why there is no answer, for an answer too large to be read.
fn too_long() -> String {
    mailcraft_ui_egui::tl!("the answer is too long").to_string()
}

/// Why there is no answer, for a call that was given up.
fn too_late() -> String {
    mailcraft_ui_egui::tl!("it took too long").to_string()
}

#[cfg(target_arch = "wasm32")]
pub use browser::{chat, dictation, opener, service, sorting};

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::sync::mpsc::Receiver;

    use mailcraft_ui_egui::{NoraAnswer, NoraCall};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    use mailcraft_ui_egui::nora::MAX_ANSWER;

    use super::{DICTATION, DictationForm, GIVE_UP_MS, allowed, dictation_form, param, status, text, too_late, too_long, url};

    /// What the timer of a call through the host resolves with, told from any answer of the host.
    const TIMED_OUT: &str = "mailcraft:nora-timed-out";
    /// The longest CSRF token taken from the page.
    const MAX_TOKEN: usize = 512;

    /// What a call carries to the server.
    enum Payload {
        /// Its arguments, as JSON.
        Json(String),
        /// A dictation's form: the browser writes its type, with the boundary.
        Form(web_sys::FormData),
    }

    /// The way to Nora when the hosting page turns her on (`?nora`, or `mailcraftNoraCall`); `None`
    /// otherwise. Each call's answer comes through its channel, and the window is woken for it.
    pub fn service(ctx: egui::Context) -> Option<Box<dyn Fn(NoraCall) -> Receiver<NoraAnswer>>> {
        let window = web_sys::window()?;
        let asked = param(&window.location().search().unwrap_or_default());
        if asked.is_none() && host_call().is_none() {
            return None;
        }
        let page = window.location().origin().unwrap_or_default();
        let origin = match asked.as_deref() {
            // An origin the address names that isn't taken is said in the console, and the page's own is used.
            Some(param) => crate::nora_origin::from_param(param, &page).unwrap_or_else(|why| {
                log::warn!("{why}");
                String::new()
            }),
            None => String::new(),
        };
        Some(Box::new(move |call: NoraCall| {
            let (tx, rx) = std::sync::mpsc::channel();
            let (ctx, origin) = (ctx.clone(), origin.clone());
            spawn_local(async move {
                let answer = answer(&origin, &call).await;
                // The window may have stopped waiting: nothing to tell then.
                let _ = tx.send(answer);
                ctx.request_repaint();
            });
            rx
        }))
    }

    /// One call, answered whatever happens: no answer is status 0 and why.
    async fn answer(origin: &str, call: &NoraCall) -> NoraAnswer {
        let reached = match allowed(call) {
            Err(why) => Err(why),
            // Her conversation's calls only where they may go now (a host that came since must say it takes them).
            Ok(_) if super::CHAT_METHODS.contains(&call.method) && !chat() => Err("this host doesn't take Nora's conversation".to_string()),
            // //// Neoffice — her ready replies go out where her conversation does (the page's own `fetch`).
            Ok(_) if super::READY_METHODS.contains(&call.method) && !chat() => Err("this host doesn't take Nora's ready replies".to_string()),
            // //// Neoffice — « Trier avec Nora » where its calls may go (the page's own `fetch`, or a host that says it takes them).
            Ok(_) if super::SORTING_METHODS.contains(&call.method) && !sorting() => Err("this host doesn't take Nora's filing".to_string()),
            // //// Neoffice — a dictation where it may go (the page's own `fetch`, or a host that says it makes its form).
            Ok(_) if call.method == DICTATION && !dictation() => Err("this host doesn't carry a dictation".to_string()),
            Ok(body) => match host_call() {
                Some((owner, function)) => call_host(&owner, &function, call.method, &body).await,
                None if call.method == DICTATION => match dictation_form(&call.args).and_then(|parts| form_of(&parts)) {
                    Ok(form) => fetch(origin, call.method, Payload::Form(form)).await,
                    Err(why) => Err(why),
                },
                None => fetch(origin, call.method, Payload::Json(body)).await,
            },
        };
        match reached {
            Ok((status, body)) => NoraAnswer { id: call.id, status, body },
            Err(why) => NoraAnswer { id: call.id, status: 0, body: why },
        }
    }

    /// Whether the calls of Nora's conversation (her pane's, [`super::CHAT_METHODS`]) go out: always as the page's own
    /// `fetch` (`/neomail`, its session, its origin); through a host that makes the calls itself only when it says it
    /// takes them, with `window.mailcraftNoraChat === true` beside its `mailcraftNoraCall` (Neodesk's shell does since
    /// 1.3.0, once the instance says it has Nora).
    pub fn chat() -> bool {
        match host_call() {
            None => true,
            Some((owner, _)) => crate::host::get(&owner, "mailcraftNoraChat").and_then(|v| v.as_bool()) == Some(true),
        }
    }

    /// Whether the calls of « Trier avec Nora » ([`super::SORTING_METHODS`]) go out: always as the page's own `fetch`
    /// (`/neomail`); through a host that makes the calls itself only when it says it takes them, with
    /// `window.mailcraftNoraSorting === true` beside its `mailcraftNoraCall` (Neodesk's shell, once its list has them).
    pub fn sorting() -> bool {
        match host_call() {
            None => true,
            Some((owner, _)) => chat() && crate::host::get(&owner, "mailcraftNoraSorting").and_then(|v| v.as_bool()) == Some(true),
        }
    }

    /// Whether a dictation in her pane ([`DICTATION`]) goes out: always as the page's own `fetch` (`/neomail`, the desk's page: the
    /// form is made here); through a host that makes the calls itself only when it says it makes that form, with
    /// `window.mailcraftNoraDictation === true` beside its `mailcraftNoraCall` (an older Neodesk never says so: no microphone there).
    pub fn dictation() -> bool {
        match host_call() {
            None => true,
            Some((owner, _)) => chat() && crate::host::get(&owner, "mailcraftNoraDictation").and_then(|v| v.as_bool()) == Some(true),
        }
    }

    /// The form the server takes for a dictation: the recording as a file, and the language.
    fn form_of(parts: &DictationForm) -> Result<web_sys::FormData, String> {
        let form = web_sys::FormData::new().map_err(|e| reason(&e))?;
        let bytes = js_sys::Array::of1(&js_sys::Uint8Array::from(parts.audio.as_slice()));
        let kind = web_sys::BlobPropertyBag::new();
        kind.set_type(&parts.mime);
        let file = web_sys::Blob::new_with_u8_array_sequence_and_options(&bytes, &kind).map_err(|e| reason(&e))?;
        form.append_with_blob_and_filename("file", &file, &parts.file_name).map_err(|e| reason(&e))?;
        form.append_with_str("language", &parts.language).map_err(|e| reason(&e))?;
        Ok(form)
    }

    /// How the page opens a page of the person's instance that a card or a link of her pane leads to, on the person's
    /// click (`mailcraftNoraOpen(path)`, on the page or on the page of the same origin that frames it: `/neomail` opens
    /// it in a new tab); `None` when it offers none, and the pane then shows a link as its words. The path is one the
    /// conversation let through (craft-nora: a page of the instance), which the page checks again.
    pub fn opener() -> Option<Box<dyn Fn(&str)>> {
        let (owner, open) = crate::host::windows()
            .into_iter()
            .find_map(|w| crate::host::get(&w, "mailcraftNoraOpen").filter(JsValue::is_function).map(|f| (w, js_sys::Function::from(f))))?;
        Some(Box::new(move |path: &str| {
            if let Err(error) = open.call1(&owner, &JsValue::from_str(path)) {
                log::warn!("the page did not open a page of the instance: {}", reason(&error));
            }
        }))
    }

    /// The host's own way to Nora, when it offers one: the window that offers it, and the function.
    fn host_call() -> Option<(JsValue, js_sys::Function)> {
        crate::host::windows()
            .into_iter()
            .find_map(|w| crate::host::get(&w, "mailcraftNoraCall").filter(JsValue::is_function).map(|f| (w, js_sys::Function::from(f))))
    }

    /// What the browser said, as text: never an address (the browser's messages don't carry the request's), short.
    fn reason(e: &JsValue) -> String {
        let said =
            e.as_string().or_else(|| e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message()))).unwrap_or_else(|| "no reason given".into());
        said.chars().filter(|c| !c.is_control()).take(200).collect()
    }

    /// One call made by the host (`mailcraftNoraCall(method, argsJson)`), its answer read as a `fetch`'s.
    async fn call_host(owner: &JsValue, function: &js_sys::Function, method: &str, body: &str) -> Result<(u16, String), String> {
        let promise = function.call2(owner, &JsValue::from_str(method), &JsValue::from_str(body)).map_err(|e| reason(&e))?;
        // A host that never answers must not hold the call for good; the timer goes once it has.
        let handle = std::cell::Cell::new(None);
        let timer = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(w) = web_sys::window() {
                handle.set(w.set_timeout_with_callback_and_timeout_and_arguments_1(&resolve, GIVE_UP_MS, &JsValue::from_str(TIMED_OUT)).ok());
            }
        });
        let raced = js_sys::Promise::race(&js_sys::Array::of2(&js_sys::Promise::resolve(&promise), &timer));
        let answer = JsFuture::from(raced).await;
        if let (Some(w), Some(timer)) = (web_sys::window(), handle.get()) {
            w.clear_timeout_with_handle(timer);
        }
        let answer = answer.map_err(|e| reason(&e))?;
        if answer.as_string().as_deref() == Some(TIMED_OUT) {
            return Err(too_late());
        }
        let said = status(crate::host::get(&answer, "status").and_then(|v| v.as_f64()));
        // Measured before it is copied in.
        let body = match crate::host::get(&answer, "body").as_ref().and_then(|v| v.dyn_ref::<js_sys::JsString>()) {
            Some(written) if usize::try_from(written.length()).map_or(true, |n| n > MAX_ANSWER) => return Err(too_long()),
            Some(written) => String::from(written),
            None => String::new(),
        };
        Ok((said, body))
    }

    /// The CSRF token of a Frappe page, if the page (or the same-origin page that frames it) has one.
    fn csrf_token() -> Option<String> {
        crate::host::windows().iter().find_map(|w| {
            crate::host::get(w, "frappe")
                .and_then(|frappe| crate::host::get(&frappe, "csrf_token"))
                .or_else(|| crate::host::get(w, "csrf_token"))
                .and_then(|token| token.as_string())
                .filter(|token| !token.is_empty() && token != "None" && token.len() <= MAX_TOKEN && !token.chars().any(char::is_control))
        })
    }

    /// One call as a `fetch` to `origin`, given up after [`GIVE_UP_MS`].
    async fn fetch(origin: &str, method: &str, payload: Payload) -> Result<(u16, String), String> {
        let window = web_sys::window().ok_or("no window")?;
        let page = window.location().origin().unwrap_or_default();
        let headers = web_sys::Headers::new().map_err(|e| reason(&e))?;
        let body = match payload {
            Payload::Json(body) => {
                headers.set("Content-Type", "application/json").map_err(|e| reason(&e))?;
                JsValue::from_str(&body)
            }
            Payload::Form(form) => form.into(),
        };
        headers.set("Accept", "application/json").map_err(|e| reason(&e))?;
        // The page's token goes to the page's own origin only.
        if crate::nora_origin::is_page_origin(origin, &page)
            && let Some(token) = csrf_token()
        {
            headers.set("X-Frappe-CSRF-Token", &token).map_err(|e| reason(&e))?;
        }
        let controller = web_sys::AbortController::new().map_err(|e| reason(&e))?;
        let give_up = {
            let controller = controller.clone();
            Closure::once(move || controller.abort())
        };
        let timer =
            window.set_timeout_with_callback_and_timeout_and_arguments_0(give_up.as_ref().unchecked_ref(), GIVE_UP_MS).map_err(|e| reason(&e))?;
        let answer = exchange(&window, &url(origin, method), &headers, &body, &controller.signal()).await;
        window.clear_timeout_with_handle(timer);
        // The timer is cleared before its closure goes.
        drop(give_up);
        match answer {
            Err(_) if controller.signal().aborted() => Err(too_late()),
            answer => {
                // An answer refused before it was read whole is not read on.
                if answer.is_err() {
                    controller.abort();
                }
                answer
            }
        }
    }

    async fn exchange(
        window: &web_sys::Window,
        url: &str,
        headers: &web_sys::Headers,
        body: &JsValue,
        signal: &web_sys::AbortSignal,
    ) -> Result<(u16, String), String> {
        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        init.set_headers(headers);
        init.set_body(body);
        // The desk's own session to the page's own origin; nothing of it to another.
        init.set_credentials(web_sys::RequestCredentials::SameOrigin);
        init.set_cache(web_sys::RequestCache::NoStore);
        // Not followed elsewhere with the page's token: a redirect is an error.
        js_sys::Reflect::set(&init, &JsValue::from_str("redirect"), &JsValue::from_str("error")).map_err(|e| reason(&e))?;
        init.set_signal(Some(signal));
        let answer = JsFuture::from(window.fetch_with_str_and_init(url, &init)).await.map_err(|e| reason(&e))?;
        let response: web_sys::Response = answer.dyn_into().map_err(|_| "the answer is not a response".to_string())?;
        // Larger than taken, by what it says of itself: refused before a byte is read.
        let length = response.headers().get("Content-Length").ok().flatten().and_then(|v| v.trim().parse::<u64>().ok());
        if length.is_some_and(|n| usize::try_from(n).map_or(true, |n| n > MAX_ANSWER)) {
            return Err(too_long());
        }
        let bytes = crate::fetch::read(&response, MAX_ANSWER).await.map_err(|e| match e {
            mailcraft_jmap::Error::TooLarge(_) => too_long(),
            other => other.to_string().chars().take(200).collect(),
        })?;
        Ok((response.status(), text(&bytes)))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_address_asks_for_nora() {
        for (search, asked) in [
            ("", None),
            ("?", None),
            ("?sample", None),
            ("?nora", Some("")),
            ("?sample&nora", Some("")),
            ("nora", Some("")),
            ("?nora=", Some("")),
            ("?nora=1", Some("1")),
            ("?nora=true&sample", Some("true")),
            ("?nora=http%3A%2F%2F127.0.0.1%3A8773", Some("http://127.0.0.1:8773")),
            ("?nora=https://evil.example&nora=1", Some("https://evil.example")),
            ("?no%72a=x", Some("x")),
            ("?nora=a+b%2", Some("a b%2")),
            ("?noraX=1", None),
            ("?xnora", None),
            ("?NORA", None),
        ] {
            assert_eq!(param(search).as_deref(), asked, "{search:?}");
        }
        assert_eq!(param("?nora=%E0%A4%A"), Some("\u{fffd}%A".into()), "bytes that aren't text are replaced");
        assert_eq!(param(&format!("?nora={}", "x".repeat(10_000))).map(|v| v.chars().count()), Some(MAX_PARAM));
        // What the address asks for, with the rule of where her calls may go: the page's own origin, or a loopback one
        // from a loopback page; nothing else.
        let origin = |search: &str, page: &str| param(search).map(|p| crate::nora_origin::from_param(&p, page).unwrap_or_default());
        assert_eq!(origin("?nora", "https://desk.example").as_deref(), Some(""));
        assert_eq!(origin("?nora=1", "https://desk.example").as_deref(), Some(""));
        assert_eq!(origin("?nora=https%3A%2F%2Fevil.example", "https://desk.example").as_deref(), Some(""), "refused: the page's own");
        assert_eq!(origin("?nora=http://127.0.0.1:8773", "http://127.0.0.1:8772").as_deref(), Some("http://127.0.0.1:8773"), "the mock");
        assert_eq!(origin("?nora=http://127.0.0.1:8773", "https://desk.example").as_deref(), Some(""), "not from an instance's page");
        assert_eq!(origin("?sample", "https://desk.example"), None);
    }

    /// The server names a mailbox by its account there, made under the address it had then: its session's address says
    /// that name (the facade's `…session?account=<name>`, which `/neomail` and Neodesk's shell both hand over), or else
    /// the page does (`account`), and an older page that says neither has it taken for the address.
    #[test]
    fn a_mailbox_is_named_as_the_server_knows_it() {
        let facade = "https://desk.example/api/method/frappe_webmail.mailcraft_jmap.session?account=";
        let named = |account: &str, session: &str| {
            let handed = HostAccount { email: "Ada@Example.org".into(), account: account.into(), session: session.into(), ..Default::default() };
            handed_mailbox(&handed)
        };
        for (account, session, name) in [
            // The page's session, as `/neomail` writes it (`quote(name, safe="")`) and Neodesk's shell keeps it.
            ("", format!("{facade}ada.former%40example.org"), "ada.former@example.org"),
            ("", format!("{facade}Ada%20%28support%29%20%2B%20sales"), "Ada (support) + sales"),
            // What would end the value or the query is in it, escaped, and so is a byte of a character that isn't ASCII.
            ("", format!("{facade}Sales%20%23%201"), "Sales # 1"),
            ("", format!("{facade}R%26D"), "R&D"),
            ("", format!("{facade}a%3Db"), "a=b"),
            ("", format!("{facade}100%25"), "100%"),
            ("", format!("{facade}Zo%C3%AB"), "Zo\u{eb}"),
            // Read as the server reads a query: `+` is a space; the first `account` counts; a fragment is no part of it.
            ("", format!("{facade}Ada+Lovelace&account=other"), "Ada Lovelace"),
            ("", format!("{facade}ada.former%40example.org#account=other"), "ada.former@example.org"),
            ("", "https://desk.example/s?x=1&account=ada.former%40example.org".to_string(), "ada.former@example.org"),
            // The session's name comes first: the ids of its messages are that account's. The page's is taken without it.
            ("other@example.org", format!("{facade}ada.former%40example.org"), "ada.former@example.org"),
            ("ada.former@example.org", "https://mail.example.org/jmap/session".to_string(), "ada.former@example.org"),
            ("Support", facade.to_string(), "Support"),
            ("  Support  ", String::new(), "Support"),
            // An older page: no name anywhere, the address is taken for it.
            ("", "https://mail.example.org/jmap/session".to_string(), "Ada@Example.org"),
            ("", facade.to_string(), "Ada@Example.org"),
            ("", format!("{facade}%20"), "Ada@Example.org"),
            ("  ", "https://desk.example/s?accounts=x&xaccount=y#?account=z".to_string(), "Ada@Example.org"),
            ("", String::new(), "Ada@Example.org"),
        ] {
            let expected = PageMailbox { address: "Ada@Example.org".into(), name: name.into(), shared: false };
            assert_eq!(named(account, &session), expected, "{account:?} {session:?}");
        }
        // A mailbox shared with the person stays one for her pane, as the page said.
        let shared = HostAccount { email: "Team@Example.org".into(), shared: true, ..Default::default() };
        assert!(handed_mailbox(&shared).shared);
        // However long the address or the page makes it, the name taken is not longer than a query's value is read.
        for (account, session) in [(String::new(), format!("{facade}{}", "a".repeat(10_000))), ("a".repeat(10_000), String::new())] {
            assert_eq!(named(&account, &session).name.chars().count(), MAX_PARAM);
        }
    }

    #[test]
    fn only_noras_methods_go_out_with_a_plain_object() {
        let call = |method: &'static str, args| NoraCall { id: 1, method, args };
        for method in METHODS {
            assert_eq!(allowed(&call(method, json!({"account_name": ""}))), Ok(r#"{"account_name":""}"#.to_string()));
        }
        // Her pane's conversation: the five calls of craft-nora's protocol and the two doors of her cards, and no other of the chat's.
        for method in CHAT_METHODS {
            assert_eq!(allowed(&call(method, json!({"thread_id": "t1"}))), Ok(r#"{"thread_id":"t1"}"#.to_string()));
            assert!(allowed(&call(method, json!({"thread_id": "t1", "cmd": "frappe.client.delete"}))).is_err(), "{method}");
        }
        // Her ready replies: the two methods, and no other of the server's mail module.
        for method in READY_METHODS {
            assert_eq!(allowed(&call(method, json!({"action": "get"}))), Ok(r#"{"action":"get"}"#.to_string()));
            assert!(allowed(&call(method, json!({"cmd": "frappe.client.delete"}))).is_err(), "{method}");
        }
        // « Trier avec Nora »: the thirteen methods of the mailbox's filing, the four of her questions and the two of
        // «Prioritaire», numbers and lists among their arguments, and no other of the server's filing or questions.
        for method in SORTING_METHODS {
            let args = json!({"account": "ada@example.org", "on": 1, "folders": [{"folder": "Clients", "takes": "clients"}]});
            assert!(allowed(&call(method, args.clone())).is_ok(), "{method}");
            assert!(allowed(&call(method, json!({"account": "ada@example.org", "cmd": "frappe.client.delete"}))).is_err(), "{method}");
        }
        assert_eq!(SORTING_METHODS.len(), 19);
        assert!(SORTING_METHODS.contains(&"nora.api.v2.mail_sorting.priority") && SORTING_METHODS.contains(&"nora.api.v2.mail_sorting.priority_set"));
        assert!(SORTING_METHODS.iter().all(|m| m.starts_with("nora.api.v2.mail_sorting.") || m.starts_with("nora.api.asks.")));
        for (method, args) in [
            ("nora.api.v2.mail_sorting.reset", json!({})),
            ("nora.api.asks.retained", json!({})),
            ("nora.api.asks.reopen", json!({})),
            ("nora.api.v2.secretary_plan.validate", json!({})),
            ("nora.api.nora_webmail.get_style_profile", json!({})),
            ("nora.api.mail_ready.refresh_if_sent_more", json!({})),
            ("nora.api.chat.clear_thread", json!({})),
            ("nora.api.chat.stop", json!({})),
            ("frappe.client.get_list", json!({"doctype": "Webmail Account"})),
            ("", json!({})),
            (METHODS[0], json!(["account_name"])),
            (METHODS[0], json!("text")),
            (METHODS[0], json!(null)),
            (METHODS[0], json!({"cmd": "frappe.client.delete"})),
            (METHODS[1], json!({"reply_context": "x".repeat(9 << 20)})),
        ] {
            assert!(allowed(&call(method, args)).is_err(), "{method}");
        }
    }

    /// A dictation is the one call whose arguments are a recording: they go out only as what the panel hands over, and become the
    /// form the server takes, as Neodesk's shell makes it.
    #[test]
    fn a_dictation_goes_out_as_a_recording_and_nothing_else() {
        let call = |args| NoraCall { id: 1, method: DICTATION, args };
        assert_eq!(DICTATION, "nora.api.stt.transcribe");
        let good = json!({"audio_base64": "QUJDRA==", "mime": "audio/webm", "language": "de"});
        assert!(allowed(&call(good.clone())).is_ok());
        assert_eq!(
            dictation_form(&good),
            Ok(DictationForm { audio: b"ABCD".to_vec(), mime: "audio/webm".into(), file_name: "dictation.webm".into(), language: "de".into() })
        );
        for (mime, name) in [("audio/mp4", "dictation.m4a"), ("audio/ogg", "dictation.ogg"), ("AUDIO/WEBM;codecs=opus", "dictation.webm")] {
            let form = dictation_form(&json!({"audio_base64": "QUJDRA==", "mime": mime, "language": "fr"})).unwrap();
            assert_eq!(form.file_name, name, "{mime}");
            assert!(form.mime.starts_with("audio/") && !form.mime.contains(';') && form.mime == form.mime.to_lowercase(), "{mime}");
        }
        let at_most = "A".repeat(MAX_DICTATION);
        assert!(dictation_form(&json!({"audio_base64": at_most, "mime": "audio/webm", "language": "fr"})).is_ok());
        for bad in [
            json!({}),
            json!({"audio_base64": "QUJDRA==", "mime": "audio/webm"}),
            json!({"audio_base64": "", "mime": "audio/webm", "language": "fr"}),
            json!({"audio_base64": "not base64!", "mime": "audio/webm", "language": "fr"}),
            json!({"audio_base64": 12, "mime": "audio/webm", "language": "fr"}),
            json!({"audio_base64": "A".repeat(MAX_DICTATION + 4), "mime": "audio/webm", "language": "fr"}),
            json!({"audio_base64": "QUJDRA==", "mime": "text/html", "language": "fr"}),
            json!({"audio_base64": "QUJDRA==", "mime": "", "language": "fr"}),
            json!({"audio_base64": "QUJDRA==", "mime": "audio/webm", "language": "french"}),
            json!({"audio_base64": "QUJDRA==", "mime": "audio/webm", "language": "FR"}),
            json!({"audio_base64": "QUJDRA==", "mime": "audio/webm", "language": "fr", "cmd": "frappe.client.delete"}),
            json!({"audio_base64": "QUJDRA==", "mime": "audio/webm", "language": "fr", "user": "someone"}),
        ] {
            assert!(allowed(&call(bad.clone())).is_err(), "{bad}");
            assert!(dictation_form(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_answer_is_shaped_whatever_the_host_says() {
        assert_eq!(url("", "nora.api.nora_webmail.translate"), "/api/method/nora.api.nora_webmail.translate");
        assert_eq!(url("http://127.0.0.1:8773/", "nora.api.nora_webmail.improve"), "http://127.0.0.1:8773/api/method/nora.api.nora_webmail.improve");
        for (said, shaped) in [
            (Some(200.0), 200),
            (Some(417.9), 417),
            (Some(0.0), 0),
            (Some(-1.0), 0),
            (Some(1000.0), 0),
            (Some(1e300), 0),
            (Some(f64::NAN), 0),
            (Some(f64::INFINITY), 0),
            (None, 0),
        ] {
            assert_eq!(status(said), shaped, "{said:?}");
        }
        assert_eq!(text(b"Bonjour \xC3\xA0 vous"), "Bonjour à vous");
        assert_eq!(text(b"half \xC3"), "half \u{fffd}");
        assert_eq!(text(&[0xff, 0xfe, b'{']), "\u{fffd}\u{fffd}{");
        assert_eq!(too_long(), "the answer is too long");
        assert_eq!(too_late(), "it took too long");
    }
}
