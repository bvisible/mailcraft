// //// Neoffice — added file (no upstream equivalent): the mailboxes a hosting page hands MailCraft (maintenance#1630).

//! The mailboxes a hosting page hands MailCraft, so that it opens them without a form or a password.
//!
//! The page (Neoffice's `/neomail` in the desk, or Neodesk's window, which frame the browser build) sets, before
//! MailCraft starts:
//!
//! ```js
//! window.mailcraftJmap = { accounts: [{
//!   email: "jane@example.com",   // the mailbox, MailCraft's account key
//!   name: "Jane Doe",            // the name on sent mail
//!   session: "https://…",        // its JMAP session URL
//!   username: "jane@example.com",
//!   secret: "<token>",           // kept in this tab's memory only, never stored
//!   scheme: "bearer",            // or "basic"
//!   signature: { html: "…", text: "…",   // optional: the mailbox's signature in Neoffice
//!     logo: { data: "<base64>", name: "logo.png", alt: "…", width: 120 },  // a company's logo, by its bytes
//!     layout: "pictureLeft" }            // where it goes: "pictureLeft" or "pictureAbove"
//! }], language: "fr",           // the person's language: MailCraft draws in it for « System default »
//!   product: "NeoMail" };        // the name the person reads, shown wherever MailCraft names itself
//! ```
//!
//! When the server refuses a token (it expired, or died with the session that asked for it), MailCraft calls
//! `window.mailcraftJmapRenew()` if the page has it: a promise of `{ accounts: [...] }` with new secrets, as above; the
//! mailbox then signs in again, once. An answer that leaves a mailbox out means the page gives it no more: the mailbox
//! asks for a reload. No answer (a promise that fails, gives no `accounts`, or doesn't settle within 20 seconds), or one
//! that names the mailbox without a secret, is asked again a few seconds later, three times (renewal.rs,
//! jmap_services.rs). A page without the function has no renewal to give: the mailbox asks for a reload at once.
//!
//! A page that can fetch what a browser tab may not read itself (another site's bytes: `/neomail` asks its server,
//! Neodesk its own process) offers `window.mailcraftFetchUrl(url)`, a promise of `{ status, contentType, bytes }`
//! (`bytes` a `Uint8Array`), or one that fails with the reason in its message: the pictures a message links to, once
//! the person asked to see them, are then fetched through it. A page without it has no pictures from the web.
//!
//! The person signed in to the page may be named in the address, as for the document editors (`?author=Jeanne%20Muster`).
//!
//! It is read on this page, or on the page that frames it when that one is of the same origin (reading another
//! origin's window throws, which reads as nothing). Nothing secret travels in the address. With host accounts,
//! MailCraft keeps no mail, account or secret between visits: only the interface settings.
// Read by the start of the browser shell (web.rs), the pictures through the page too (`fetch_url`); the name of the
// person signed in (`author`) waits for the step that uses it.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;

use mailcraft_jmap::Auth;
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::credentials::{Credentials, Renewal};
use crate::renewal;

/// The most mailboxes taken from the page.
const MAX_ACCOUNTS: u32 = 20;
/// How long the page has to answer a renewal, in milliseconds: one that doesn't settle is no answer.
const RENEW_WAIT_MS: i32 = 20_000;
/// What the timer of a renewal resolves with, told from any answer of the page.
const TIMED_OUT: &str = "mailcraft:renew-timed-out";
/// The longest name of the signed-in person taken from the address.
const MAX_AUTHOR_CHARS: usize = 128;
/// The longest signature taken from the page, its HTML and its text.
const MAX_SIGNATURE_HTML: usize = 200_000;
const MAX_SIGNATURE_TEXT: usize = 10_000;

/// A mailbox the page handed over.
#[derive(Clone, Default)]
pub struct HostAccount {
    pub email: String,
    pub name: String,
    pub session: String,
    pub username: String,
    pub secret: String,
    pub bearer: bool,
    /// Its signature in Neoffice, as the page gives it: its HTML, its text, and a company's logo with its place.
    pub signature: crate::page_signature::PageSignature,
}

impl std::fmt::Debug for HostAccount {
    // Never the secret, nor the address: they go into logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostAccount").field("bearer", &self.bearer).field("signature", &self.signature).finish_non_exhaustive()
    }
}

pub(crate) fn get(target: &JsValue, key: &str) -> Option<JsValue> {
    js_sys::Reflect::get(target, &JsValue::from_str(key)).ok().filter(|value| !value.is_undefined() && !value.is_null())
}

fn text(target: &JsValue, key: &str) -> String {
    get(target, key).and_then(|v| v.as_string()).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// The page itself, then the page that frames it when it is of the same origin (Nora's way through the page is
/// looked for there too, nora.rs).
pub(crate) fn windows() -> Vec<JsValue> {
    let Some(window) = web_sys::window() else { return Vec::new() };
    let own = JsValue::from(window.clone());
    let mut all = vec![own.clone()];
    if let Ok(Some(parent)) = window.parent() {
        let parent = JsValue::from(parent);
        // A top page is its own parent; another origin's `document` can't be read.
        if parent != own && get(&parent, "document").is_some() {
            all.push(parent);
        }
    }
    all
}

fn handed() -> Option<JsValue> {
    windows().iter().find_map(|w| get(w, "mailcraftJmap").filter(JsValue::is_object))
}

/// The mailboxes handed over, the incomplete ones left out (at most 20). Empty when the page hands none: MailCraft is
/// then the program it is without a page.
pub fn accounts() -> Vec<HostAccount> {
    handed().map(|host| parse(&host, true)).unwrap_or_default()
}

/// The hosting page's answer to `mailcraftJmapRenew()`.
pub struct Renewed {
    /// The complete mailboxes it gives, with their new secrets.
    pub accounts: Vec<HostAccount>,
    /// Every address it names (lower case), complete or not: one named without a secret has none to give yet.
    pub named: Vec<String>,
}

/// The page's `window.mailcraftJmapRenew`, and the window it belongs to.
fn renewal() -> Option<(JsValue, js_sys::Function)> {
    windows().into_iter().find_map(|w| {
        let f = get(&w, "mailcraftJmapRenew").filter(JsValue::is_function)?;
        Some((w, js_sys::Function::from(f)))
    })
}

/// Whether the hosting page can renew tokens at all (`window.mailcraftJmapRenew`).
pub fn can_renew() -> bool {
    renewal().is_some()
}

/// New tokens for the mailboxes from the hosting page's `window.mailcraftJmapRenew()`, when the server refused the ones
/// it had given: the page's answer, which may leave mailboxes out, or name none. `None` when there is no answer: the
/// page offers no such function, its promise fails or doesn't settle within [`RENEW_WAIT_MS`], or it gives no `accounts`.
pub async fn renew() -> Option<Renewed> {
    let (window, renew) = renewal()?;
    let promise = renew.call0(&window).ok()?;
    // A page that never answers (a request of its own that hangs) must not hold the mailbox up for good.
    let timer = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_1(&resolve, RENEW_WAIT_MS, &JsValue::from_str(TIMED_OUT));
        }
    });
    let raced = js_sys::Promise::race(&js_sys::Array::of2(&js_sys::Promise::resolve(&promise), &timer));
    let answer = JsFuture::from(raced).await.ok()?;
    if answer.as_string().as_deref() == Some(TIMED_OUT) {
        return None;
    }
    let list = get(&answer, "accounts").filter(js_sys::Array::is_array)?;
    let named = js_sys::Array::from(&list)
        .iter()
        .take(MAX_ACCOUNTS as usize)
        .map(|item| text(&item, "email").to_lowercase())
        .filter(|e| e.contains('@'))
        .collect();
    // A renewal brings new secrets only: the signatures it hands again are not read.
    Some(Renewed { accounts: parse(&answer, false), named })
}

/// The page's `window.mailcraftFetchUrl`, and the window it belongs to.
fn url_fetcher() -> Option<(JsValue, js_sys::Function)> {
    windows().into_iter().find_map(|w| {
        let f = get(&w, "mailcraftFetchUrl").filter(JsValue::is_function)?;
        Some((w, js_sys::Function::from(f)))
    })
}

/// Whether the hosting page fetches pictures on the web for MailCraft (`window.mailcraftFetchUrl`): a browser tab
/// can't read another site's bytes by itself.
pub fn fetches_urls() -> bool {
    url_fetcher().is_some()
}

/// A picture a message links to, fetched by the hosting page's `window.mailcraftFetchUrl(url)` for a window that may
/// not reach other sites itself: its bytes when the page got it (status 2xx, at most `max` bytes), else why not (a
/// promise that fails says why in its message). `None` when the page offers no such function.
pub async fn fetch_url(url: &str, max: usize) -> Option<Result<Vec<u8>, String>> {
    let (window, fetch) = url_fetcher()?;
    Some(fetched(&window, &fetch, url, max).await)
}

async fn fetched(window: &JsValue, fetch: &js_sys::Function, url: &str, max: usize) -> Result<Vec<u8>, String> {
    let promise = fetch.call1(window, &JsValue::from_str(url)).map_err(|e| reason(&e))?;
    let answer = JsFuture::from(js_sys::Promise::resolve(&promise)).await.map_err(|e| reason(&e))?;
    match get(&answer, "status").and_then(|s| s.as_f64()).filter(|s| (0.0..1000.0).contains(s)).map(|s| s as u16) {
        Some(status) if (200..300).contains(&status) => {}
        Some(status) => return Err(format!("HTTP {status}")),
        None => return Err("the page could not fetch it".into()),
    }
    // Viewed as bytes whatever they are (a Uint8Array or an ArrayBuffer of any window: `instanceof` fails from one window
    // to another), and measured before they are copied in.
    let bytes = get(&answer, "bytes").filter(JsValue::is_object).ok_or("the page gave no picture")?;
    let bytes = js_sys::Uint8Array::new(&bytes);
    if bytes.length() == 0 {
        return Err("the page gave no picture".into());
    }
    if usize::try_from(bytes.length()).unwrap_or(usize::MAX) > max {
        return Err(format!("the picture is larger than {max} bytes"));
    }
    Ok(bytes.to_vec())
}

fn reason(error: &JsValue) -> String {
    error.as_string().or_else(|| error.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message()))).unwrap_or_else(|| format!("{error:?}"))
}

/// The name of the person signed in to the hosting page, from the address (`?author=Jeanne%20Muster`), as the document
/// editors read it: trimmed, at most 128 characters; `None` when there is none.
pub fn author() -> Option<String> {
    let name: String = query_param("author")?.trim().chars().take(MAX_AUTHOR_CHARS).collect();
    let name = name.trim_end().to_string();
    (!name.is_empty()).then_some(name)
}

/// A `?key=value` of the page's address, decoded as a form is (`+` is a space).
fn query_param(key: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(key)
}

/// The complete mailboxes of `{ accounts: [...] }` (at most 20, each address once), with their signatures when
/// `signatures` (not for a renewal, which brings new secrets only).
fn parse(host: &JsValue, signatures: bool) -> Vec<HostAccount> {
    let Some(list) = get(host, "accounts").filter(js_sys::Array::is_array) else { return Vec::new() };
    let mut out: Vec<HostAccount> = Vec::new();
    for item in js_sys::Array::from(&list).iter().take(MAX_ACCOUNTS as usize) {
        let signature = get(&item, "signature").filter(|_| signatures).map(|given| signature(&given)).unwrap_or_default();
        let account = HostAccount {
            email: text(&item, "email"),
            name: text(&item, "name"),
            session: text(&item, "session"),
            username: text(&item, "username"),
            secret: text(&item, "secret"),
            bearer: text(&item, "scheme").eq_ignore_ascii_case("bearer"),
            signature,
        };
        if complete(&account) && !out.iter().any(|a| a.email.eq_ignore_ascii_case(&account.email)) {
            out.push(account);
        }
    }
    out
}

/// The signature a page gives a mailbox: its HTML and its text within their lengths, and a company's logo by its bytes
/// with its place, as far as MailCraft takes them (page_signature.rs).
fn signature(given: &JsValue) -> crate::page_signature::PageSignature {
    use crate::page_signature::{PageSignature, layout, picture};
    let logo = get(given, "logo").and_then(|logo| {
        let width = get(&logo, "width").and_then(|v| v.as_f64());
        picture(&text(&logo, "data"), &text(&logo, "name"), &text(&logo, "alt"), width)
    });
    PageSignature {
        html: get(given, "html").and_then(|v| v.as_string()).filter(|h| h.len() <= MAX_SIGNATURE_HTML).unwrap_or_default(),
        text: text(given, "text").chars().take(MAX_SIGNATURE_TEXT).collect(),
        layout: logo.as_ref().map_or_else(String::new, |_| layout(&text(given, "layout"))),
        logo,
    }
}

/// A mailbox MailCraft can open: an address, an `https` session (or plain `http` to this computer alone, the
/// workshop's stand-in server: hosted.rs) and a secret.
fn complete(a: &HostAccount) -> bool {
    a.email.contains('@') && crate::hosted::session_allowed(&a.session) && !a.secret.is_empty()
}

/// The product's name the person reads (Neoffice's "NeoMail"), if the page gives one.
pub fn product() -> Option<String> {
    Some(text(&handed()?, "product")).filter(|name| !name.is_empty() && name.chars().count() <= 40)
}

/// The language of the person the page is for (a tag such as `fr` or `de-CH`), if it says one.
pub fn language() -> Option<String> {
    Some(text(&handed()?, "language")).filter(|tag| !tag.is_empty() && tag.len() <= 35)
}

/// What each mailbox signs in with: its address (lower case) → (secret, Bearer rather than Basic).
pub fn secrets(accounts: &[HostAccount]) -> HashMap<String, (String, bool)> {
    accounts.iter().map(|a| (a.email.to_lowercase(), (a.secret.clone(), a.bearer))).collect()
}

/// The credentials of the mailboxes the page handed over, in this tab's memory only, renewed by the page when a
/// server refuses one (`mailcraftJmapRenew`).
pub struct HostCredentials {
    /// Address (lower case) → (secret, Bearer rather than Basic).
    secrets: RefCell<HashMap<String, (String, bool)>>,
    /// Address (lower case) → the login name a password goes with.
    usernames: HashMap<String, String>,
}

impl HostCredentials {
    pub fn new(accounts: &[HostAccount]) -> HostCredentials {
        HostCredentials {
            secrets: RefCell::new(secrets(accounts)),
            usernames: accounts.iter().map(|a| (a.email.to_lowercase(), a.username.clone())).collect(),
        }
    }
}

impl Credentials for HostCredentials {
    fn auth(&self, email: &str) -> Option<Auth> {
        let email = email.to_lowercase();
        let (secret, bearer) = self.secrets.borrow().get(&email).cloned()?;
        if bearer {
            return Some(Auth::bearer(secret));
        }
        // A login name the page left empty: the address, as JMAP servers sign their users in.
        let user = self.usernames.get(&email).filter(|u| !u.is_empty()).cloned().unwrap_or_else(|| email.clone());
        Some(Auth::basic(user, secret))
    }

    async fn renew(&self, email: &str) -> Renewal {
        let email = email.to_lowercase();
        // A page that has no renewal to give: only reloading it gives the mailbox a token again.
        if !can_renew() {
            return Renewal::Reload;
        }
        // Nothing is borrowed while the page answers: a Send/Receive and a download may ask at the same time, and each
        // takes in what the page answered it.
        let answer = renew().await.map(|renewed| renewal::Answer { given: secrets(&renewed.accounts), named: renewed.named });
        if let Some(answer) = &answer {
            renewal::merge(&mut self.secrets.borrow_mut(), answer);
        }
        // The job that asks is the one the answer is for: there is no other start of it to see to (renewal::next's
        // generations are for a runner that keeps its mailboxes waiting).
        match renewal::next(answer.as_ref(), &email, 0, Some(0), 0, 1) {
            renewal::Next::SignIn => self.auth(&email).map_or(Renewal::Reload, Renewal::Given),
            renewal::Next::AskLater => Renewal::Later,
            renewal::Next::Reload | renewal::Next::Skip => Renewal::Reload,
        }
    }
}
