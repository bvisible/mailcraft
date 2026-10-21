// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! JMAP (RFC 8620 core, RFC 8621 mail) without sockets and without browser APIs.
//!
//! A [`Client`] speaks JMAP over any [`Http`]: the browser's `fetch`, a blocking client on the
//! desktop, or the in-process server of the tests (`standin`, behind the `stand-in` feature). Nothing
//! here starts a thread or opens a socket, and the clock is read for one thing only (the date and
//! the zone of a message that is sent or saved, through the time crate, which asks the browser as well
//! as the system), so the crate builds for the browser as it is, and its futures need not be `Send`.
//!
//! - [`Client::connect`] fetches the session and checks it: mail must be on offer, and the addresses it
//!   names are resolved against its own. The credentials go to the server the session is served from and
//!   to no other: an address on another server, or on the same one without encryption, is refused (plain
//!   `http` is for this computer alone). The limits the session states are clamped.
//! - [`Client::call`] sends one batch of method calls in one request. A method that fails fails its own
//!   answer ([`Answers::get`]), never the others; a request that fails as a whole is an [`Error`].
//! - [`sync_mail`] reads an account's mailboxes and the newest messages in them for the engine, as the
//!   request it worked out says, and hands them over in pieces ([`mail`]).
//! - [`fetch`] brings a message whole: the raw octets an email's blob holds, downloaded from the
//!   session's `downloadUrl`.
//! - Sending and drafts (RFC 8621 §7 and §4.8) are part of a run ([`mail`]): a message of the Outbox is
//!   uploaded, filed in the Sent mailbox and submitted, and a draft is filed in Drafts in place of the
//!   copy an earlier save made (`send`).
//! - [`sync_pim`] reads an account's calendars (JMAP for Calendars) for the engine, when the session
//!   offers them, as a job of its own beside a Send/Receive ([`pim`]).
//!
//! Everything a server sends is hostile: answers are capped ([`MAX_JSON`]), ids are held to
//! [`valid_id`], numbers are clamped, and nothing here indexes into what came over the wire. The
//! credentials ([`Auth`]) stay out of error text, even when a server echoes them back, and the password
//! and the token out of `Debug` output (which names a Basic user); so do the addresses and the mail a
//! request or an answer carries.
//!
//! The address, HTTP and fetch helpers ([`url`], [`http`], and the browser build's transport) are written
//! fresh from RFC 3986, 6570, 7617, 6750, 8620 and 9110 and the WHATWG URL and Fetch Standards.
//!
//! The address given to [`Client::connect`] is the one the session is served from. Finding it from a
//! bare host name through a redirect to another server (RFC 8620 §2.2) is for the caller: an [`Http`]
//! does not hand the credentials on to another origin, so a session that is behind such a redirect is
//! refused for want of them.
//!
//! What a [`Client`] needs of the [`Http`] it runs over: a redirect is never followed from `https` to
//! `http`, the `Authorization` header goes to the origin it was set for and to no other, no cookie is
//! sent or kept, and an answer whose body is over [`Request::max`] is given up as soon as that is known
//! (the client checks again the size of what it is handed).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

use serde_json::Value;

pub mod fetch;
pub mod http;
pub mod mail;
pub mod pim;
mod send;
#[cfg(any(test, feature = "stand-in"))]
pub mod standin;
pub mod url;

pub use http::{Auth, Error, Http, Request, Response, Result, retry_after};
pub use mail::{failed_run, sync_mail};
pub use pim::{failed_pim, sync_pim};
pub use url::{Url, pct, pct_decode};

/// The capability every session has (RFC 8620 §2).
pub const CORE: &str = "urn:ietf:params:jmap:core";
/// The capability of mail (RFC 8621 §1.3).
pub const MAIL: &str = "urn:ietf:params:jmap:mail";
/// The capability of sending mail (RFC 8621 §1.3.2): identities and submissions.
pub const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
/// The capability of address books and contact cards (RFC 9610).
pub const CONTACTS: &str = "urn:ietf:params:jmap:contacts";
/// The capability of calendars and their events (JMAP for Calendars, the IETF's draft on RFC 8620).
pub const CALENDARS: &str = "urn:ietf:params:jmap:calendars";
/// The kind of a problem that names a limit of the request that was exceeded (RFC 8620 §3.6.1).
const LIMIT_PROBLEM: &str = "urn:ietf:params:jmap:error:limit";

/// What a bare host is given after it (RFC 8620 §2.2).
const WELL_KNOWN: &str = "/.well-known/jmap";
/// Largest JSON answer accepted to a request of method calls. What is parsed out of it takes up much
/// more memory: a list of small numbers about sixteen times as much, a list of small objects about eighty
/// times (each object is a tree node), so a host that counts its memory should count that, and a caller
/// that knows how much it can be given asks for less ([`Client::call_within`]).
pub const MAX_JSON: usize = 32 << 20;
/// Largest session document accepted.
const MAX_SESSION: usize = 1 << 20;
/// Longest session address accepted.
const MAX_SESSION_URL: usize = 2048;
/// Longest address, or address template, the session may name.
const MAX_TEMPLATE: usize = 4096;
/// Longest variable name in an address template, and longest value put in one (a file name, a type).
const MAX_VARIABLE_NAME: usize = 32;
const MAX_VARIABLE: usize = 512;
/// Longest address made out of a template.
const MAX_EXPANDED: usize = 16 * 1024;
/// Longest id accepted from a server (RFC 8620 §1.2 allows 255; a server may go past it).
pub const MAX_ID: usize = 512;
/// What a session that states no `maxObjectsInGet` is taken to allow (RFC 8620 §2: every server takes
/// 500), and the most any session is believed: it may state anything.
const DEFAULT_OBJECTS_IN_GET: usize = 500;
const MAX_OBJECTS_IN_GET: usize = 1000;
/// Likewise for `maxObjectsInSet`: the most objects, created, updated and destroyed together, that one
/// `set` call may hold.
const DEFAULT_OBJECTS_IN_SET: usize = 500;
const MAX_OBJECTS_IN_SET: usize = 1000;
/// Likewise for `maxCallsInRequest` (every server takes 16 calls in a request).
const DEFAULT_CALLS_IN_REQUEST: usize = 16;
const MAX_CALLS_IN_REQUEST: usize = 64;
/// Likewise for `maxSizeUpload`, in octets: the largest message that is sent or saved (a message is
/// held in memory several times over while it goes).
const DEFAULT_SIZE_UPLOAD: usize = 50 << 20;
const MAX_SIZE_UPLOAD: usize = 64 << 20;
/// Longest text kept of what a server says in an error, and largest body read for it.
const MAX_ERROR_TEXT: usize = 300;
const MAX_PROBLEM: usize = 64 * 1024;
/// Longest wait, in seconds, that a server can ask for.
const MAX_RETRY_AFTER: u32 = 3600;
/// The longest stretch a server is believed to expand repeating events over in one query
/// (`maxExpandedQueryDuration`), in seconds: what it says, held between a day and two years; 500 days
/// when it says nothing that can be read.
const DEFAULT_SPAN: i64 = 500 * 86_400;
const MIN_SPAN: i64 = 86_400;
const MAX_SPAN: i64 = 2 * 365 * 86_400;
/// Most properties a session is believed to say a server writes, and the longest name of one.
const MAX_WRITABLE: usize = 64;
const MAX_PROPERTY_NAME: usize = 64;

// The type is `http::Error`; hiding what it must not show is this crate's own.
impl Error {
    /// This error with the secrets taken out of any text a transport put in it.
    fn hidden(self, secrets: &Secrets) -> Error {
        match self {
            Error::Io(t) => Error::Io(secrets.hide(&t)),
            Error::Tls(t) => Error::Tls(secrets.hide(&t)),
            Error::Protocol(t) => Error::Protocol(secrets.hide(&t)),
            Error::Refused { kind, description } => Error::Refused { kind: secrets.hide(&kind), description: secrets.hide(&description) },
            Error::Bad(t) => Error::Bad(secrets.hide(&t)),
            Error::Auth(t) => Error::Auth(secrets.hide(&t)),
            Error::TooLarge(t) => Error::TooLarge(secrets.hide(&t)),
            busy @ Error::Busy { .. } => busy,
        }
    }
}

/// What must never be shown: the credentials and every form they are sent in, a user name that is an
/// address, and the address of the session with the addresses in its query.
#[derive(Clone, Default)]
struct Secrets(Vec<String>);

impl Secrets {
    fn of(auth: &Auth, header: &str, addresses: &[&str]) -> Secrets {
        let mut secrets = Secrets::default();
        secrets.add(header);
        match auth {
            Auth::Basic { user, password } => {
                secrets.add(password);
                secrets.add(header.strip_prefix("Basic ").unwrap_or_default());
                // A user name that is not an address may be any word (`me`): it would garble every text.
                if user.contains('@') {
                    secrets.add(user);
                }
            }
            Auth::Bearer(token) => secrets.add(token),
        }
        for address in addresses {
            secrets.add_address(address);
        }
        secrets
    }

    /// The address as it is, and each value of its query that is a person's address, written either way.
    fn add_address(&mut self, address: &str) {
        self.add(address);
        let query = address.split_once('?').map_or("", |(_, query)| query.split('#').next().unwrap_or(""));
        for value in query.split('&').filter_map(|pair| pair.split_once('=').map(|(_, value)| value)) {
            let plain = pct_decode(value);
            if plain.contains('@') {
                self.add(value);
                self.add(&plain);
            }
        }
    }

    /// A secret, and the form of it a `pct`-encoded address holds.
    fn add(&mut self, secret: &str) {
        for form in [secret.to_string(), pct(secret)] {
            if !form.is_empty() && !self.0.contains(&form) {
                self.0.push(form);
            }
        }
        // Longest first, so that a secret inside another one can't leave a piece behind.
        self.0.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }

    /// `text` with every secret replaced by `***`. What a server said is hidden here first and cut or
    /// folded after, never the other way: a secret that is cut in two, or whose blanks are folded, is no
    /// longer found.
    fn hide(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in &self.0 {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), "***");
            }
        }
        out
    }
}

/// Server text made fit for an error message: one line, no control characters, not too long.
fn tidy(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut space = false;
    for (n, c) in text.trim().chars().enumerate() {
        if n >= max_chars {
            out.push('…');
            break;
        }
        if c.is_control() || c.is_whitespace() {
            space = !out.is_empty();
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    out
}

/// Whether `id` is one a server may hand out: 1 to [`MAX_ID`] characters of the URL-safe base64
/// alphabet without padding (`A-Z a-z 0-9 - _`, RFC 8620 §1.2). An id that is not goes nowhere: ids
/// are put into JSON keys (`mailboxIds/<id>`) and into addresses.
pub fn valid_id(id: &str) -> bool {
    (1..=MAX_ID).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Whether `text` is the name of a scheme (`https`): a letter, then letters, digits, `+`, `-` and `.`.
fn is_scheme(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_alphabetic()) && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The session address for what the owner typed: a bare host or origin gets the well-known path
/// (RFC 8620 §2.2), an address with a path of its own is left as it is.
pub fn session_url(input: &str) -> String {
    let typed = input.trim();
    // A `://` counts only after a scheme's own characters: the query of a bare host may hold one.
    let has_scheme = typed.split_once("://").is_some_and(|(scheme, _)| is_scheme(scheme));
    let full = if has_scheme { typed.to_string() } else { format!("https://{typed}") };
    let (scheme, rest) = full.split_once("://").unwrap_or(("https", ""));
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let (path, more) = tail.split_at(tail.find(['?', '#']).unwrap_or(tail.len()));
    if path.is_empty() || path == "/" { format!("{scheme}://{authority}{WELL_KNOWN}{more}") } else { format!("{scheme}://{authority}{path}{more}") }
}

/// The identifier of the method call numbered `index` in a request.
fn call_id(index: usize) -> String {
    format!("c{index}")
}

/// The number of the call that an identifier names, if it is one [`call_id`] gives (not `c01`, not `c+1`).
fn call_index(id: &str) -> Option<usize> {
    let index = id.strip_prefix('c')?.parse::<usize>().ok()?;
    (call_id(index) == id).then_some(index)
}

/// A reference to the result of the call numbered `call` (from 0, in the same request) for the
/// value of an argument whose name starts with `#` (RFC 8620 §3.7):
/// `json!({"#ids": result_of(0, "Email/query", "/ids")})` hands the ids a query found to a `get`.
pub fn result_of(call: usize, method: &str, path: &str) -> Value {
    serde_json::json!({"resultOf": call_id(call), "name": method, "path": path})
}

/// The answers to the method calls of one request, in the order of the calls.
pub struct Answers(Vec<Result<Value>>);

impl Answers {
    /// The answer to call number `call`: its result, or the error that call alone ended in
    /// ([`Error::Refused`] when the server said so, [`Error::Protocol`] when it gave no answer).
    pub fn get(&self, call: usize) -> Result<&Value> {
        match self.0.get(call) {
            Some(Ok(value)) => Ok(value),
            Some(Err(e)) => Err(e.clone()),
            None => Err(Error::Protocol("JMAP: there is no such call".into())),
        }
    }

    /// The answer to call number `call`, taken out: the same as [`Answers::get`], without a copy. The
    /// answer is not there afterwards.
    pub fn take(&mut self, call: usize) -> Result<Value> {
        match self.0.get_mut(call) {
            Some(slot) => std::mem::replace(slot, Err(Error::Protocol("JMAP: that answer was taken".into()))),
            None => Err(Error::Protocol("JMAP: there is no such call".into())),
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What may be shown of answers: how many, never what they hold (mail).
impl fmt::Debug for Answers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Answers({} of which {} failed)", self.0.len(), self.0.iter().filter(|a| a.is_err()).count())
    }
}

/// Send a request and hold the answer to the size the request allowed.
async fn send<H: Http>(http: &H, req: Request) -> Result<Response> {
    let max = req.max;
    let resp = http.send(req).await?;
    if resp.body.len() > max {
        return Err(Error::TooLarge(format!("the answer is larger than {max} bytes")));
    }
    Ok(resp)
}

fn json_body(resp: &Response) -> Result<Value> {
    serde_json::from_slice(&resp.body).map_err(|e| Error::Protocol(format!("JMAP: {e}")))
}

/// Turn an HTTP failure into the error the caller knows how to handle: 401 and 403 are
/// [`Error::Auth`], 429 and 503 are [`Error::Busy`] (with the wait the server asked for), a request that
/// is over a limit of the server's (RFC 8620 §3.6.1) is [`Error::TooLarge`], and any other status that is
/// not a success is [`Error::Bad`] with the reason the server gave, when it gave one as RFC 7807 problem
/// details. That includes 502 and 504: a gateway that gives up on one request may take a smaller one, so a
/// run halves the call ([`mail`]) rather than waits as for a busy server.
pub fn check(resp: &Response) -> Result<()> {
    check_hiding(resp, &Secrets::default())
}

/// [`check`], with the secrets taken out of what the server says before it is cut to size.
fn check_hiding(resp: &Response, secrets: &Secrets) -> Result<()> {
    match resp.status {
        200..=299 => Ok(()),
        401 | 403 => Err(Error::Auth("the server refused the credentials".into())),
        429 | 503 => Err(Error::Busy { status: resp.status, retry_after: resp.retry_after.map(|s| s.clamp(1, MAX_RETRY_AFTER)) }),
        status => {
            let problem = if resp.body.len() <= MAX_PROBLEM { serde_json::from_slice::<Value>(&resp.body).ok() } else { None };
            let text = |key: &str| problem.as_ref().and_then(|p| p.get(key)).and_then(Value::as_str).map(|s| tidy(&secrets.hide(s), MAX_ERROR_TEXT));
            if status == 400 && text("type").as_deref() == Some(LIMIT_PROBLEM) {
                return Err(Error::TooLarge(
                    format!("the request is over the server's limit {}", text("limit").unwrap_or_default()).trim().to_string(),
                ));
            }
            let detail = text("detail").or_else(|| text("type")).unwrap_or_default();
            Err(Error::Bad(format!("HTTP {status} {detail}").trim().to_string()))
        }
    }
}

/// `{accountId}`-style template variables (RFC 6570 level 1), percent-encoded; a variable that is not
/// given is empty. The address is built in one pass and is never allowed to pass [`MAX_EXPANDED`], so that
/// a template full of variables and a long value can't make a large one.
fn expand(template: &str, vars: &[(&str, &str)]) -> Result<String> {
    let too_long = || Error::Protocol("a JMAP address is too long".into());
    if vars.iter().any(|(_, value)| value.len() > MAX_VARIABLE) {
        return Err(too_long());
    }
    let encoded: Vec<(&str, String)> = vars.iter().map(|(name, value)| (*name, pct(value))).collect();
    let mut out = String::with_capacity(template.len());
    let mut push = |piece: &str| -> Result<()> {
        if out.len().saturating_add(piece.len()) > MAX_EXPANDED {
            return Err(too_long());
        }
        out.push_str(piece);
        Ok(())
    };
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, from_brace) = rest.split_at(open);
        push(before)?;
        let Some((name, after)) = from_brace.get(1..).and_then(|inner| inner.split_once('}')) else {
            // (An open brace that is never closed: a template that `resolve` would not have let in.)
            push(from_brace)?;
            return Ok(out);
        };
        if let Some((_, value)) = encoded.iter().find(|(known, _)| *known == name) {
            push(value)?;
        }
        rest = after;
    }
    push(rest)?;
    Ok(out)
}

/// Whether the braces of an address are the variables of a template and nothing else: `{name}` with a name
/// of letters, digits and underscores, never nested, and none at all in an address that is no template.
fn well_formed(raw: &str, templated: bool) -> bool {
    let mut rest = raw;
    while let Some(at) = rest.find(['{', '}']) {
        let Some(inner) = rest.get(at..).and_then(|from| from.strip_prefix('{')).filter(|_| templated) else {
            return false;
        };
        let Some((name, after)) = inner.split_once('}') else {
            return false;
        };
        if name.is_empty() || name.len() > MAX_VARIABLE_NAME || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return false;
        }
        rest = after;
    }
    true
}

/// An address the session names, absolute or relative, resolved against the session's own; the
/// `{variables}` of a template stay as they are. It must be on the server the session is served from, and
/// reached the way the session was (encrypted, unless that was this computer): the credentials go there.
fn resolve(base: &Url, raw: &str, key: &str) -> Result<String> {
    let templated = key != "apiUrl";
    if raw.len() > MAX_TEMPLATE || !raw.bytes().all(|b| b.is_ascii_graphic()) || !well_formed(raw, templated) {
        return Err(Error::Protocol(format!("the JMAP {key} can't be used")));
    }
    let cut = raw.find('{').unwrap_or(raw.len());
    let (head, variables) = raw.split_at(cut);
    let target = base.join(head)?;
    if base.https && !target.https {
        return Err(Error::Tls(format!("the JMAP {key} isn't https")));
    }
    if target.origin() != base.origin() {
        return Err(Error::Tls(format!("the JMAP {key} isn't on the server the session is served from")));
    }
    Ok(format!("{}{variables}", target.full()))
}

/// A limit the session states, held between 1 and `cap`; `default` when it states none, or no number.
fn limit(core: Option<&Value>, key: &str, default: usize, cap: usize) -> usize {
    let stated = core.and_then(|c| c.get(key)).and_then(Value::as_u64);
    stated.map_or(default, |n| usize::try_from(n.min(cap as u64)).unwrap_or(cap)).clamp(1, cap)
}

/// A signed-in JMAP session.
pub struct Client<H> {
    http: H,
    /// The value of the `Authorization` header: a secret.
    auth: String,
    secrets: Secrets,
    account: String,
    api: String,
    download: String,
    upload: String,
    capabilities: Vec<String>,
    max_get: usize,
    max_set: usize,
    max_calls: usize,
    max_upload: usize,
    /// The accounts that hold the address books and the calendars, when the session offers them.
    contacts: Option<String>,
    calendars: Option<String>,
    /// The longest stretch the server expands repeating events over in one query, in seconds.
    span: i64,
    /// What the session says the server writes of an event and of a card, when it says.
    calendars_writable: Option<Vec<String>>,
    contacts_writable: Option<Vec<String>>,
    /// The account that holds the person's notes, when the session offers them ([`pim::notes::NOTES`]); what it says
    /// it writes of a note; the longest title and text it takes.
    notes: Option<String>,
    notes_writable: Option<Vec<String>>,
    notes_limits: (usize, usize),
}

/// What may be shown of a client: sizes, never an address or a credential.
impl<H> fmt::Debug for Client<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("capabilities", &self.capabilities.len())
            .field("max_objects_in_get", &self.max_get)
            .field("max_objects_in_set", &self.max_set)
            .field("max_calls_in_request", &self.max_calls)
            .field("contacts", &self.contacts.is_some())
            .field("calendars", &self.calendars.is_some())
            .field("notes", &self.notes.is_some())
            .finish_non_exhaustive()
    }
}

/// The account of a capability besides mail: the session's primary account for it, when the session names
/// the capability and an id that is one. (Absent, or not an id: not offered.)
fn primary_account(doc: &Value, capabilities: &serde_json::Map<String, Value>, capability: &str) -> Option<String> {
    if !capabilities.contains_key(capability) {
        return None;
    }
    doc.get("primaryAccounts").and_then(|p| p.get(capability)).and_then(Value::as_str).filter(|id| valid_id(id)).map(str::to_string)
}

/// What the session says of `account` for `capability` (its `accountCapabilities`).
fn account_capability<'a>(doc: &'a Value, account: Option<&str>, capability: &str) -> Option<&'a Value> {
    doc.get("accounts")?.get(account?)?.get("accountCapabilities")?.get(capability)
}

/// The longest stretch a server expands repeating events over, from its `maxExpandedQueryDuration` (an ISO 8601
/// duration such as `P500D`), in seconds: held between a day and two years, and [`DEFAULT_SPAN`] when it says nothing
/// that can be read.
fn expanded_span(capability: Option<&Value>) -> i64 {
    let said = capability.and_then(|c| c.get("maxExpandedQueryDuration")).and_then(Value::as_str).and_then(pim::span_seconds);
    said.map_or(DEFAULT_SPAN, |seconds| seconds.clamp(MIN_SPAN, MAX_SPAN))
}

/// The properties a session says a server writes (`writableProperties`, which a server may add to an account's
/// capability), when it says: names of one word, [`MAX_WRITABLE`] at most. `None` when it says nothing, or nothing
/// that is a list.
fn writable_properties(capability: Option<&Value>) -> Option<Vec<String>> {
    let listed = capability?.get("writableProperties")?.as_array()?;
    fn name(value: &Value) -> Option<&str> {
        value.as_str().filter(|name| (1..=MAX_PROPERTY_NAME).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_graphic()))
    }
    Some(listed.iter().take(MAX_WRITABLE).filter_map(name).map(str::to_string).collect())
}

impl<H: Http> Client<H> {
    /// Fetch the session at `session` (what the owner typed: [`session_url`] gives it the well-known
    /// path if it has none) and check it offers mail. The session's addresses must be on the server it is
    /// served from: see the crate documentation.
    pub async fn connect(http: H, session: &str, auth: &Auth) -> Result<Client<H>> {
        let header = auth.header()?;
        let address = session_url(session);
        if address.len() > MAX_SESSION_URL {
            return Err(Error::Protocol("the session address is too long".into()));
        }
        let base = Url::parse(&address)?;
        if !base.https && !base.is_loopback() {
            return Err(Error::Tls("the session address isn't https".into()));
        }
        // What is sent is the address that was parsed and checked, not what was typed.
        let sent = base.full();
        let secrets = Secrets::of(auth, &header, &[session, address.as_str(), sent.as_str()]);
        let request = Request::get(&sent, MAX_SESSION).header("Authorization", &header).header("Accept", "application/json");
        let resp = send(&http, request).await.map_err(|e| e.hidden(&secrets))?;
        check_hiding(&resp, &secrets)?;
        let doc = json_body(&resp)?;
        let client = Client::from_session(http, &base, &doc, header, secrets)?;
        log::debug!(
            "jmap: connected, {} capabilities, up to {} objects per get and {} calls per request",
            client.capabilities.len(),
            client.max_get,
            client.max_calls
        );
        Ok(client)
    }

    fn from_session(http: H, base: &Url, doc: &Value, auth: String, secrets: Secrets) -> Result<Client<H>> {
        let capabilities =
            doc.get("capabilities").and_then(Value::as_object).ok_or_else(|| Error::Protocol("the JMAP session has no capabilities".into()))?;
        if !capabilities.contains_key(MAIL) {
            return Err(Error::Protocol("this JMAP server doesn't offer mail".into()));
        }
        let account = doc
            .get("primaryAccounts")
            .and_then(|p| p.get(MAIL))
            .and_then(Value::as_str)
            .filter(|id| valid_id(id))
            .ok_or_else(|| Error::Protocol("no mail account in the JMAP session".into()))?;
        let address = |key: &str| -> Result<String> {
            let raw = doc.get(key).and_then(Value::as_str).ok_or_else(|| Error::Protocol(format!("the JMAP session has no `{key}`")))?;
            resolve(base, raw, key)
        };
        let core = capabilities.get(CORE);
        // Calendars and address books, when the session offers them: each in an account of its own name (often the
        // mail's), with what the server says of it there.
        let contacts = primary_account(doc, capabilities, CONTACTS);
        let calendars = primary_account(doc, capabilities, CALENDARS);
        let of_calendars = account_capability(doc, calendars.as_deref(), CALENDARS);
        let of_contacts = account_capability(doc, contacts.as_deref(), CONTACTS);
        let notes = primary_account(doc, capabilities, pim::notes::NOTES);
        let of_notes = account_capability(doc, notes.as_deref(), pim::notes::NOTES);
        Ok(Client {
            api: address("apiUrl")?,
            download: address("downloadUrl")?,
            upload: address("uploadUrl")?,
            account: account.to_string(),
            // (As many as the session document holds, which is bounded by its size.)
            capabilities: capabilities.keys().cloned().collect(),
            max_get: limit(core, "maxObjectsInGet", DEFAULT_OBJECTS_IN_GET, MAX_OBJECTS_IN_GET),
            max_set: limit(core, "maxObjectsInSet", DEFAULT_OBJECTS_IN_SET, MAX_OBJECTS_IN_SET),
            max_calls: limit(core, "maxCallsInRequest", DEFAULT_CALLS_IN_REQUEST, MAX_CALLS_IN_REQUEST),
            max_upload: limit(core, "maxSizeUpload", DEFAULT_SIZE_UPLOAD, MAX_SIZE_UPLOAD),
            span: expanded_span(of_calendars),
            calendars_writable: writable_properties(of_calendars),
            contacts_writable: writable_properties(of_contacts),
            contacts,
            notes_writable: writable_properties(of_notes),
            notes_limits: pim::notes::limits(of_notes),
            notes,
            calendars,
            auth,
            secrets,
            http,
        })
    }

    /// The id of the account that holds the mail: the session's primary account for mail.
    pub fn account_id(&self) -> &str {
        &self.account
    }

    /// The most ids one `get` may be given: what the server takes, never more than 1000.
    pub fn max_objects_in_get(&self) -> usize {
        self.max_get
    }

    /// The most objects one `set` call may hold, created, updated and destroyed together: what the server takes,
    /// never more than 1000.
    pub fn max_objects_in_set(&self) -> usize {
        self.max_set
    }

    /// The most method calls one request may hold: what the server takes, never more than 64.
    pub fn max_calls_in_request(&self) -> usize {
        self.max_calls
    }

    /// The most octets a message may have when it is uploaded: what the server takes, never more than
    /// 64 MB.
    pub fn max_size_upload(&self) -> usize {
        self.max_upload
    }

    /// Whether the session names a capability (`urn:ietf:params:jmap:submission`…).
    pub fn has_capability(&self, uri: &str) -> bool {
        self.capabilities.iter().any(|c| c == uri)
    }

    /// Whether the session offers address books ([`CONTACTS`]), in an account it names.
    pub fn offers_contacts(&self) -> bool {
        self.contacts.is_some()
    }

    /// Whether the session offers calendars ([`CALENDARS`]), in an account it names.
    pub fn offers_calendars(&self) -> bool {
        self.calendars.is_some()
    }

    /// The id of the account that holds the address books, when the session offers them.
    pub fn contacts_account(&self) -> Option<&str> {
        self.contacts.as_deref()
    }

    /// The id of the account that holds the calendars, when the session offers them.
    pub fn calendars_account(&self) -> Option<&str> {
        self.calendars.as_deref()
    }

    /// The longest stretch, in seconds, that the server expands repeating events over in one query: what its
    /// session says (`maxExpandedQueryDuration`), held between a day and two years; 500 days when it says nothing.
    pub fn calendars_span(&self) -> i64 {
        self.span
    }

    /// What the server says it writes of an event (`writableProperties` of the calendars' account), when it says:
    /// a client offers nothing else rather than learn it from a refusal.
    pub fn calendars_writable(&self) -> Option<&[String]> {
        self.calendars_writable.as_deref()
    }

    /// Likewise for a contact card.
    pub fn contacts_writable(&self) -> Option<&[String]> {
        self.contacts_writable.as_deref()
    }

    /// Whether the session offers the person's notes ([`pim::notes::NOTES`]), in an account it names.
    pub fn offers_notes(&self) -> bool {
        self.notes.is_some()
    }

    /// The id of the account that holds the notes, when the session offers them.
    pub fn notes_account(&self) -> Option<&str> {
        self.notes.as_deref()
    }

    /// What the server says it writes of a note, when it says.
    pub fn notes_writable(&self) -> Option<&[String]> {
        self.notes_writable.as_deref()
    }

    /// The longest title and text of a note the server takes, in characters, as its session says them
    /// ([`pim::notes::limits`]).
    pub fn notes_limits(&self) -> (usize, usize) {
        self.notes_limits
    }

    /// Whether the path of the session's API address ends with `suffix`: for a host that knows a server by where its
    /// API is. The address itself is not handed out (it is the server's, and a host has no need of it).
    pub fn api_path_ends_with(&self, suffix: &str) -> bool {
        Url::parse(&self.api).is_ok_and(|url| url.path.split('?').next().is_some_and(|path| path.ends_with(suffix)))
    }

    /// The address a blob is downloaded from: the session's template with the account, the blob (an
    /// id of the alphabet of [`valid_id`]), the type to answer with and a file name filled in (these two
    /// at most 512 bytes each, and neither of them `.` or `..`).
    pub fn download_url(&self, blob_id: &str, content_type: &str, name: &str) -> Result<String> {
        if !valid_id(blob_id) {
            return Err(Error::Protocol("not a blob id".into()));
        }
        // Encoding leaves a dot as it is (RFC 6570 §3.2.2), and a path that is read drops a `.` segment, and a
        // `..` one with the segment before it (RFC 3986 §5.2.4): put in a template's path, either would make
        // the address lead to another place on the server than the one the template names.
        if [content_type, name].iter().any(|value| matches!(*value, "." | "..")) {
            return Err(Error::Protocol("a type or a file name that can't be put in an address".into()));
        }
        expand(&self.download, &[("accountId", &self.account), ("blobId", blob_id), ("type", content_type), ("name", name)])
    }

    /// The address a blob is uploaded to.
    pub fn upload_url(&self) -> Result<String> {
        expand(&self.upload, &[("accountId", &self.account)])
    }

    /// Run a batch of method calls in one request, and give their answers in order. A method that
    /// fails (`["error", …]`, no answer at all, an answer to another method) fails its own answer
    /// only. A request that fails as a whole, or holds more calls than the server takes (which is
    /// not sent), is an error.
    pub async fn call(&self, using: &[&str], calls: Vec<(&str, Value)>) -> Result<Answers> {
        self.call_within(using, calls, MAX_JSON).await
    }

    /// [`Client::call`] with an answer of at most `max` bytes (never more than [`MAX_JSON`]): one that is
    /// larger is [`Error::TooLarge`], and is not read to its end by an [`Http`] that keeps to
    /// [`Request::max`]. A caller that knows how big an answer can be asks for no more than that, so that a
    /// server can not make it hold a great deal of memory.
    pub async fn call_within(&self, using: &[&str], calls: Vec<(&str, Value)>, max: usize) -> Result<Answers> {
        self.call_with_status(using, calls, max).await.1
    }

    /// [`Client::call_within`], with the HTTP status the server answered with when it answered at all
    /// (`None`: no answer came, or the request was never sent). A caller that must know whether the server can
    /// have run the calls (a message sent) reads it: a status of 400 to 499 or 503 is a server that refused
    /// the request as a whole, any other error status one that may have run it first.
    pub(crate) async fn call_with_status(&self, using: &[&str], calls: Vec<(&str, Value)>, max: usize) -> (Option<u16>, Result<Answers>) {
        if calls.is_empty() {
            return (None, Ok(Answers(Vec::new())));
        }
        if calls.len() > self.max_calls {
            return (None, Err(Error::TooLarge(format!("{} method calls in one request, and the server takes {}", calls.len(), self.max_calls))));
        }
        let names: Vec<&str> = calls.iter().map(|(name, _)| *name).collect();
        let method_calls: Vec<Value> =
            calls.into_iter().enumerate().map(|(i, (name, args))| Value::Array(vec![Value::from(name), args, Value::from(call_id(i))])).collect();
        let body = serde_json::json!({ "using": using, "methodCalls": method_calls });
        let bytes = match serde_json::to_vec(&body) {
            Ok(bytes) => bytes,
            Err(e) => return (None, Err(Error::Protocol(e.to_string()))),
        };
        let limit = max.min(MAX_JSON);
        let request =
            Request::post(&self.api, "application/json", bytes, limit).header("Authorization", &self.auth).header("Accept", "application/json");
        let resp = match self.http.send(request).await {
            Ok(resp) => resp,
            Err(e) => return (None, Err(e.hidden(&self.secrets))),
        };
        let status = Some(resp.status);
        if resp.body.len() > limit {
            return (status, Err(Error::TooLarge(format!("the answer is larger than {limit} bytes"))));
        }
        (status, self.answers_in(&resp, &names))
    }

    /// The answers a response holds to the calls named in `names`, in order.
    fn answers_in(&self, resp: &Response, names: &[&str]) -> Result<Answers> {
        check_hiding(resp, &self.secrets)?;
        let responses = match json_body(resp)? {
            Value::Object(mut top) => match top.remove("methodResponses") {
                Some(Value::Array(responses)) => responses,
                _ => return Err(Error::Protocol("JMAP: no methodResponses".into())),
            },
            _ => return Err(Error::Protocol("JMAP: the answer isn't an object".into())),
        };
        log::debug!("jmap: {} method call(s) in one request", names.len());
        Ok(Answers(answers_to(responses, names, &self.secrets)))
    }

    /// [`Client::call`] with the capabilities of mail named in `using`.
    pub async fn mail(&self, calls: Vec<(&str, Value)>) -> Result<Answers> {
        self.call(&[CORE, MAIL], calls).await
    }

    /// [`Client::call_within`] with the capabilities of mail named in `using`.
    pub async fn mail_within(&self, calls: Vec<(&str, Value)>, max: usize) -> Result<Answers> {
        self.call_within(&[CORE, MAIL], calls, max).await
    }
}

/// The answers to the calls of a request, whose methods are `names`, found in one pass over everything the
/// server answered: for each call, the first answer that has its call id and is named for its method, or
/// `error`.
fn answers_to(responses: Vec<Value>, names: &[&str], secrets: &Secrets) -> Vec<Result<Value>> {
    let mut found: Vec<Option<Vec<Value>>> = names.iter().map(|_| None).collect();
    for response in responses {
        let Value::Array(parts) = response else { continue };
        let index = parts.get(2).and_then(Value::as_str).and_then(call_index);
        let name = parts.first().and_then(Value::as_str);
        let (Some(index), Some(name)) = (index, name) else { continue };
        let wanted = names.get(index).is_some_and(|method| name == *method || name == "error");
        if let Some(slot) = found.get_mut(index).filter(|slot| wanted && slot.is_none()) {
            *slot = Some(parts);
        }
    }
    names.iter().zip(found).map(|(method, parts)| answer(parts, method, secrets)).collect()
}

/// The answer to one call: its result, or the error it ended in.
fn answer(parts: Option<Vec<Value>>, method: &str, secrets: &Secrets) -> Result<Value> {
    let Some(mut parts) = parts else {
        return Err(Error::Protocol(format!("JMAP: no answer to {method}")));
    };
    let failed = parts.first().and_then(Value::as_str) == Some("error");
    let args = parts.get_mut(1).map(std::mem::take).unwrap_or_default();
    if failed {
        let text = |key: &str, max| args.get(key).and_then(Value::as_str).map(|s| tidy(&secrets.hide(s), max)).unwrap_or_default();
        return Err(Error::Refused { kind: text("type", 64), description: text("description", MAX_ERROR_TEXT) });
    }
    Ok(args)
}

/// Run a future whose every wait is already over, as the desktop's blocking HTTP client's are: it
/// completes on the first poll. A future that does wait is polled again and again until it is done, which
/// is never if what it waits for needs this thread: it is no executor, and it is not for those.
#[cfg(not(target_arch = "wasm32"))]
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(value) = f.as_mut().poll(&mut cx) {
            return value;
        }
        std::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests;
