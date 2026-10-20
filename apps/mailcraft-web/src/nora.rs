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
//! Only the six methods the window calls go out ([`METHODS`]), each with a plain JSON object of
//! arguments without a `cmd` key, within 8 MB (what Neodesk's shell takes). An answer is read up to
//! [`MAX_ANSWER`] bytes, its text as UTF-8 with what isn't replaced; a call is given up after
//! [`GIVE_UP_MS`]. A call that got no answer comes back with status 0 and why, which the window says.

// Outside the browser only the tests use what is here.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use mailcraft_ui_egui::NoraCall;
use mailcraft_ui_egui::nora::{METHODS, PageMailbox};

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
    PageMailbox { address: handed.email.clone(), name: name.unwrap_or_else(|| handed.email.trim().to_string()) }
}

/// The body a call goes with, when it may go out at all: one of the six methods, a plain object of
/// arguments without a `cmd` key, within 8 MB.
pub fn allowed(call: &NoraCall) -> Result<String, String> {
    if !METHODS.contains(&call.method) {
        return Err("that is not one of Nora's methods".into());
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
pub use browser::service;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::sync::mpsc::Receiver;

    use mailcraft_ui_egui::{NoraAnswer, NoraCall};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    use mailcraft_ui_egui::nora::MAX_ANSWER;

    use super::{GIVE_UP_MS, allowed, param, status, text, too_late, too_long, url};

    /// What the timer of a call through the host resolves with, told from any answer of the host.
    const TIMED_OUT: &str = "mailcraft:nora-timed-out";
    /// The longest CSRF token taken from the page.
    const MAX_TOKEN: usize = 512;

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
            Ok(body) => match host_call() {
                Some((owner, function)) => call_host(&owner, &function, call.method, &body).await,
                None => fetch(origin, call.method, body).await,
            },
        };
        match reached {
            Ok((status, body)) => NoraAnswer { id: call.id, status, body },
            Err(why) => NoraAnswer { id: call.id, status: 0, body: why },
        }
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
    async fn fetch(origin: &str, method: &str, body: String) -> Result<(u16, String), String> {
        let window = web_sys::window().ok_or("no window")?;
        let page = window.location().origin().unwrap_or_default();
        let headers = web_sys::Headers::new().map_err(|e| reason(&e))?;
        headers.set("Content-Type", "application/json").map_err(|e| reason(&e))?;
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
        body: &str,
        signal: &web_sys::AbortSignal,
    ) -> Result<(u16, String), String> {
        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        init.set_headers(headers);
        init.set_body(&JsValue::from_str(body));
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
            assert_eq!(named(account, &session), PageMailbox { address: "Ada@Example.org".into(), name: name.into() }, "{account:?} {session:?}");
        }
        // However long the address or the page makes it, the name taken is not longer than a query's value is read.
        for (account, session) in [(String::new(), format!("{facade}{}", "a".repeat(10_000))), ("a".repeat(10_000), String::new())] {
            assert_eq!(named(&account, &session).name.chars().count(), MAX_PARAM);
        }
    }

    #[test]
    fn only_the_six_methods_go_out_with_a_plain_object() {
        let call = |method: &'static str, args| NoraCall { id: 1, method, args };
        for method in METHODS {
            assert_eq!(allowed(&call(method, json!({"account_name": ""}))), Ok(r#"{"account_name":""}"#.to_string()));
        }
        for (method, args) in [
            ("nora.api.nora_webmail.get_style_profile", json!({})),
            ("nora.api.chat.post_message", json!({})),
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
