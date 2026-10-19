// //// Neoffice — added file (no upstream equivalent)
//! An in-process JMAP server for tests: no socket, no thread, nothing leaves the process. A
//! [`StandIn`] is an [`Http`], so a [`Client`](crate::Client) connects to it as it would to a real server
//! (`Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))`) while the test keeps a clone to
//! fill it with mailboxes and mail and to look at what it was asked.
//!
//! Where the other stand-ins of the tests answer whatever they are told and take whatever they are
//! sent, this one holds a client to the standard (RFC 8620, 8621) as a real server does, for the part
//! it serves: the session, `Mailbox/get`, `Email/query` and `Email/get`.
//!
//! - **Who.** `Authorization: Basic` with [`USER`] and [`PASSWORD`], or `Bearer` with the one token it
//!   holds ([`Data::token`], [`TOKEN`] at first; a token it no longer holds is refused like any other:
//!   401). Nothing else is let in, not even the session.
//! - **What.** A body is `application/json` (415 otherwise) and a request of method calls is JSON with
//!   `using` and `methodCalls` (400 `notJSON`, `notRequest`). Only the capabilities it knows may be
//!   named in `using` (400 `unknownCapability`), no more calls than `maxCallsInRequest` may be in a
//!   request (400 `limit`), and a method needs its capability in `using` or it is unknown.
//! - **How.** Arguments are only the ones a method has, of the right type (`invalidArguments`). The
//!   account is the one the session names (`accountNotFound`). Ids are the URL-safe alphabet,
//!   `A-Z a-z 0-9 - _`, 1 to 512 characters, and no more than `maxObjectsInGet` are asked for at a time
//!   (`requestTooLarge`); an id that names nothing is in `notFound`. Properties are among the ones the
//!   stand-in has. `Email/query` lists one mailbox (`{"inMailbox": id}`, `unsupportedFilter`) sorted by
//!   `receivedAt` (`unsupportedSort`), at most 500 at a time, and says the total when asked.
//!   `#`-arguments resolve a back-reference to an earlier call of the same request, with RFC 6901
//!   pointers and `*` over lists (`invalidResultReference`).
//!
//! It never panics, whatever it is sent: there is no `unwrap` here, in a test build or in another
//! crate's, which turns the `stand-in` feature on.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use serde_json::{Map, Value, json};

use crate::{CORE, Error, Http, MAIL, Request, Response, Result, Url, valid_id};

/// The session's address, to connect to.
pub const SESSION: &str = "https://jmap.standin.example/jmap/session";
/// The one account, and the user and password of `Basic`.
pub const ACCOUNT: &str = "A1";
pub const USER: &str = "user@standin.example";
pub const PASSWORD: &str = "standin-password";
/// The bearer token a stand-in holds at first.
pub const TOKEN: &str = "standin-token";

const ORIGIN: &str = "https://jmap.standin.example";
const SESSION_PATH: &str = "/jmap/session";
const API_PATH: &str = "/jmap/api";
/// Largest request it reads (`maxSizeRequest`), and most ids one `Email/query` answers however many are asked.
const MAX_REQUEST: usize = 10_000_000;
const MAX_QUERY: usize = 500;
/// Most requests kept for a test to look at.
const MAX_SEEN: usize = 10_000;
/// Most header fields, and longest preview, of an email.
const MAX_HEADERS: usize = 500;
const MAX_PREVIEW: usize = 256;
/// Most segments of the path of a back-reference.
const MAX_POINTER: usize = 16;

const MAILBOX_PROPERTIES: &[&str] = &["id", "name", "parentId", "role", "sortOrder", "totalEmails", "unreadEmails"];
const EMAIL_PROPERTIES: &[&str] = &["id", "blobId", "mailboxIds", "keywords", "size", "receivedAt", "hasAttachment", "preview", "headers"];
/// What `Email/get` answers when no properties are named: all but the raw headers.
const EMAIL_DEFAULT: &[&str] = &["id", "blobId", "mailboxIds", "keywords", "size", "receivedAt", "hasAttachment", "preview"];

/// A mailbox the stand-in has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mailbox {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub role: Option<String>,
}

/// An email the stand-in has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Email {
    pub id: String,
    pub blob: String,
    pub mailboxes: BTreeSet<String>,
    pub keywords: BTreeSet<String>,
    /// A UTC date as `2026-10-15T06:00:00Z`: emails are put in order by it, as text.
    pub received: String,
    /// The message, as it was delivered.
    pub raw: Vec<u8>,
}

/// One HTTP request the stand-in was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub method: String,
    /// The path, without the query.
    pub path: String,
    /// The status it was answered with.
    pub status: u16,
    /// The methods it called, in order (none for a request that was refused before they were read).
    pub calls: Vec<String>,
}

/// What the stand-in holds, for a test to set up and to look at.
pub struct Data {
    pub mailboxes: Vec<Mailbox>,
    pub emails: Vec<Email>,
    /// The requests it was sent, oldest first (the first 10,000).
    pub seen: Vec<Seen>,
    /// What the session says it takes: method calls in a request, ids in a `get`.
    pub max_calls: usize,
    pub max_get: usize,
    /// The one bearer token it lets in; `None`: no bearer token.
    pub token: Option<String>,
    next: u32,
    version: u32,
}

impl Data {
    fn new() -> Data {
        Data {
            mailboxes: Vec::new(),
            emails: Vec::new(),
            seen: Vec::new(),
            max_calls: 16,
            max_get: 500,
            token: Some(TOKEN.into()),
            next: 0,
            version: 0,
        }
    }

    fn id(&mut self, prefix: &str) -> String {
        self.next = self.next.saturating_add(1);
        format!("{prefix}{}", self.next)
    }

    fn changed(&mut self) {
        self.version = self.version.saturating_add(1);
    }

    fn state(&self) -> String {
        format!("s{}", self.version)
    }

    /// A mailbox (`role` as `inbox`, `sent`, `trash`…): its id.
    pub fn mailbox(&mut self, name: &str, parent: Option<&str>, role: Option<&str>) -> String {
        let id = self.id("M");
        self.mailboxes.push(Mailbox { id: id.clone(), name: name.into(), parent: parent.map(str::to_string), role: role.map(str::to_string) });
        self.changed();
        id
    }

    /// An email in `mailbox` (an id this stand-in gave), as a message `raw` with `keywords` (`$seen`…)
    /// received at `received` (`2026-10-15T06:00:00Z`): its id.
    pub fn deliver(&mut self, mailbox: &str, raw: &[u8], keywords: &[&str], received: &str) -> String {
        let blob = self.id("B");
        let id = self.id("E");
        self.emails.push(Email {
            id: id.clone(),
            blob,
            mailboxes: BTreeSet::from([mailbox.to_string()]),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            received: received.into(),
            raw: raw.to_vec(),
        });
        self.changed();
        id
    }

    pub fn email(&self, id: &str) -> Option<&Email> {
        self.emails.iter().find(|e| e.id == id)
    }

    /// The id of the mailbox called `name`.
    pub fn mailbox_named(&self, name: &str) -> Option<&str> {
        self.mailboxes.iter().find(|m| m.name == name).map(|m| m.id.as_str())
    }

    /// Set or clear a keyword, as another client would: whether there was such an email.
    pub fn set_keyword(&mut self, email: &str, keyword: &str, on: bool) -> bool {
        let Some(email) = self.emails.iter_mut().find(|e| e.id == email) else {
            return false;
        };
        if on {
            email.keywords.insert(keyword.to_string());
        } else {
            email.keywords.remove(keyword);
        }
        self.changed();
        true
    }

    fn authorized(&self, headers: &[(String, String)]) -> bool {
        let Some((scheme, credentials)) = header(headers, "authorization").and_then(|v| v.split_once(' ')) else {
            return false;
        };
        if scheme.eq_ignore_ascii_case("bearer") {
            return self.token.as_deref() == Some(credentials);
        }
        if scheme.eq_ignore_ascii_case("basic") {
            let decoded = base64::engine::general_purpose::STANDARD.decode(credentials).ok().and_then(|bytes| String::from_utf8(bytes).ok());
            return decoded.and_then(|text| text.split_once(':').map(|(user, password)| user == USER && password == PASSWORD)).unwrap_or(false);
        }
        false
    }

    fn handle(&mut self, req: &Request) -> Result<Response> {
        let url = Url::parse(&req.url).map_err(|_| Error::Io("the stand-in was sent no web address".into()))?;
        if url.origin() != ORIGIN {
            return Err(Error::Io("no route to that host".into()));
        }
        let path = url.path.split('?').next().unwrap_or_default().to_string();
        let (status, body, calls) = self.route(req, &path);
        if self.seen.len() < MAX_SEEN {
            self.seen.push(Seen { method: req.method.clone(), path, status, calls });
        }
        Ok(Response { status, body: serde_json::to_vec(&body).unwrap_or_default(), retry_after: None })
    }

    fn route(&mut self, req: &Request, path: &str) -> (u16, Value, Vec<String>) {
        if !self.authorized(&req.headers) {
            return (401, problem(401, "about:blank", "the credentials were refused"), Vec::new());
        }
        match (req.method.as_str(), path) {
            ("GET", SESSION_PATH) => (200, self.session(), Vec::new()),
            ("POST", API_PATH) => {
                let json = header(&req.headers, "content-type")
                    .is_some_and(|v| v.split(';').next().is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json")));
                if json { self.api(&req.body) } else { (415, problem(415, "about:blank", "the request isn't application/json"), Vec::new()) }
            }
            (_, SESSION_PATH | API_PATH) => (405, problem(405, "about:blank", "not a method of that address"), Vec::new()),
            _ => (404, problem(404, "about:blank", "no such address"), Vec::new()),
        }
    }

    fn session(&self) -> Value {
        json!({
            "capabilities": {
                CORE: {
                    "maxSizeUpload": 25_000_000,
                    "maxConcurrentUpload": 2,
                    "maxSizeRequest": MAX_REQUEST,
                    "maxConcurrentRequests": 2,
                    "maxCallsInRequest": self.max_calls,
                    "maxObjectsInGet": self.max_get,
                    "maxObjectsInSet": self.max_get,
                    "collationAlgorithms": ["i;ascii-casemap"],
                },
                MAIL: {},
            },
            "accounts": {
                ACCOUNT: {
                    "name": USER,
                    "isPersonal": true,
                    "isReadOnly": false,
                    "accountCapabilities": {MAIL: {"maxMailboxesPerEmail": null, "emailQuerySortOptions": ["receivedAt"]}},
                },
            },
            "primaryAccounts": {MAIL: ACCOUNT},
            "username": USER,
            "apiUrl": API_PATH,
            "downloadUrl": "/jmap/download/{accountId}/{blobId}/{name}?type={type}",
            "uploadUrl": "/jmap/upload/{accountId}/",
            "state": self.state(),
        })
    }

    /// A request of method calls: the status, the body, and the names of the methods it called.
    fn api(&mut self, body: &[u8]) -> (u16, Value, Vec<String>) {
        if body.len() > MAX_REQUEST {
            return limit("maxSizeRequest", "the request is too large");
        }
        let Ok(request) = serde_json::from_slice::<Value>(body) else {
            return rejected("notJSON", "the body isn't JSON");
        };
        let using = request.get("using").and_then(Value::as_array).and_then(|u| u.iter().map(Value::as_str).collect::<Option<Vec<&str>>>());
        let (Some(using), Some(calls)) = (using, request.get("methodCalls").and_then(Value::as_array)) else {
            return rejected("notRequest", "a request has `using` (a list of names) and `methodCalls`");
        };
        if using.iter().any(|capability| !matches!(*capability, CORE | MAIL)) {
            return rejected("unknownCapability", "a capability this server doesn't have is in `using`");
        }
        if calls.len() > self.max_calls {
            return limit("maxCallsInRequest", "too many method calls");
        }
        let mut parsed: Vec<(&str, &Value, &str)> = Vec::new();
        for call in calls {
            let parts = call.as_array().filter(|p| p.len() == 3);
            let name = parts.and_then(|p| p.first()).and_then(Value::as_str);
            let id = parts.and_then(|p| p.get(2)).and_then(Value::as_str);
            let (Some(name), Some(args), Some(id)) = (name, parts.and_then(|p| p.get(1)), id) else {
                return rejected("notRequest", "a method call is [name, arguments, id]");
            };
            parsed.push((name, args, id));
        }
        let (mut done, mut responses, mut names) = (Vec::new(), Vec::new(), Vec::new());
        for (name, args, id) in parsed {
            names.push(name.to_string());
            let outcome = self.method(&using, name, args, &done);
            let (response, result) = match outcome {
                Ok(result) => (json!([name, result, id]), Some(result)),
                Err(refusal) => (json!(["error", {"type": refusal.kind, "description": refusal.description}, id]), None),
            };
            responses.push(response);
            done.push(Done { id: id.to_string(), name: name.to_string(), result });
        }
        (200, json!({"methodResponses": responses, "sessionState": self.state()}), names)
    }

    fn method(&self, using: &[&str], name: &str, args: &Value, done: &[Done]) -> Outcome<Value> {
        // A method whose capability the request doesn't name is unknown to the server (RFC 8620 §3.6.2).
        if !matches!(name, "Mailbox/get" | "Email/query" | "Email/get") || !using.contains(&MAIL) {
            return Err(refuse("unknownMethod", format!("no method {}", name.chars().take(40).collect::<String>())));
        }
        let args = arguments(args, done)?;
        match name {
            "Mailbox/get" => self.mailbox_get(&args),
            "Email/query" => self.email_query(&args),
            _ => self.email_get(&args),
        }
    }

    fn mailbox_get(&self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "ids", "properties"])?;
        account(args)?;
        let wanted = ids(args, self.max_get)?;
        let properties = properties(args, MAILBOX_PROPERTIES, MAILBOX_PROPERTIES)?;
        let (mut list, mut not_found) = (Vec::new(), Vec::new());
        match wanted {
            None if self.mailboxes.len() > self.max_get => return Err(refuse("requestTooLarge", "more mailboxes than a `get` may answer")),
            None => list.extend(self.mailboxes.iter().map(|m| self.mailbox_json(m, &properties))),
            Some(wanted) => {
                for id in wanted {
                    match self.mailboxes.iter().find(|m| m.id == id) {
                        Some(m) => list.push(self.mailbox_json(m, &properties)),
                        None => not_found.push(id),
                    }
                }
            }
        }
        Ok(json!({"accountId": ACCOUNT, "state": self.state(), "list": list, "notFound": not_found}))
    }

    fn mailbox_json(&self, mailbox: &Mailbox, properties: &[String]) -> Value {
        let inside = || self.emails.iter().filter(|e| e.mailboxes.contains(&mailbox.id));
        let mut out = Map::new();
        out.insert("id".into(), json!(mailbox.id));
        for property in properties {
            let value = match property.as_str() {
                "name" => json!(mailbox.name),
                "parentId" => json!(mailbox.parent),
                "role" => json!(mailbox.role),
                "sortOrder" => json!(0),
                "totalEmails" => json!(inside().count()),
                // Neither $seen nor $draft (RFC 8621 §2).
                "unreadEmails" => json!(inside().filter(|e| !e.keywords.contains("$seen") && !e.keywords.contains("$draft")).count()),
                _ => continue,
            };
            out.insert(property.clone(), value);
        }
        Value::Object(out)
    }

    fn email_query(&self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "filter", "sort", "position", "limit", "calculateTotal"])?;
        account(args)?;
        let unsupported = || refuse("unsupportedFilter", "only one mailbox can be listed: {\"inMailbox\": id}");
        let mailbox = match args.get("filter") {
            Some(Value::Object(filter)) if filter.len() == 1 => filter.get("inMailbox").and_then(Value::as_str).ok_or_else(unsupported)?,
            _ => return Err(unsupported()),
        };
        if !valid_id(mailbox) {
            return Err(invalid("not a mailbox id"));
        }
        if !self.mailboxes.iter().any(|m| m.id == mailbox) {
            return Err(invalid("no such mailbox"));
        }
        let ascending = sort_direction(args)?;
        let position = number(args, "position")?.unwrap_or(0);
        let limit = number(args, "limit")?.map_or(MAX_QUERY, |limit| limit.min(MAX_QUERY));
        let total = match args.get("calculateTotal") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(wanted)) => *wanted,
            Some(_) => return Err(invalid("calculateTotal is a boolean")),
        };
        // Oldest first, and those received together in the order they were delivered.
        let mut found: Vec<(usize, &Email)> = self.emails.iter().enumerate().filter(|(_, e)| e.mailboxes.contains(mailbox)).collect();
        found.sort_by(|a, b| (a.1.received.as_str(), a.0).cmp(&(b.1.received.as_str(), b.0)));
        if !ascending {
            found.reverse();
        }
        let count = found.len();
        let ids: Vec<&str> = found.iter().skip(position).take(limit).map(|(_, e)| e.id.as_str()).collect();
        let mut out = json!({"accountId": ACCOUNT, "queryState": self.state(), "canCalculateChanges": false, "position": position, "ids": ids});
        if let (true, Some(map)) = (total, out.as_object_mut()) {
            map.insert("total".into(), json!(count));
        }
        Ok(out)
    }

    fn email_get(&self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "ids", "properties"])?;
        account(args)?;
        // `ids: null` asks for every email, which is more than a `get` answers.
        let Some(wanted) = ids(args, self.max_get)? else {
            return Err(refuse("requestTooLarge", "name the emails to read"));
        };
        let properties = properties(args, EMAIL_PROPERTIES, EMAIL_DEFAULT)?;
        let (mut list, mut not_found) = (Vec::new(), Vec::new());
        for id in wanted {
            match self.email(&id) {
                Some(email) => list.push(email_json(email, &properties)),
                None => not_found.push(id),
            }
        }
        Ok(json!({"accountId": ACCOUNT, "state": self.state(), "list": list, "notFound": not_found}))
    }
}

/// An earlier call of the same request and what it answered (`None`: it failed).
struct Done {
    id: String,
    name: String,
    result: Option<Value>,
}

/// A method call that failed: JMAP's `type` and `description`.
struct Refusal {
    kind: &'static str,
    description: String,
}

type Outcome<T> = std::result::Result<T, Refusal>;

fn refuse(kind: &'static str, description: impl Into<String>) -> Refusal {
    Refusal { kind, description: description.into() }
}

fn invalid(description: impl Into<String>) -> Refusal {
    refuse("invalidArguments", description)
}

/// A request refused as a whole (RFC 8620 §3.6.1): problem details with HTTP status 400.
fn rejected(kind: &str, detail: &str) -> (u16, Value, Vec<String>) {
    (400, problem(400, &format!("urn:ietf:params:jmap:error:{kind}"), detail), Vec::new())
}

/// A request over a limit of the session (RFC 8620 §3.6.1): the problem names the limit.
fn limit(name: &str, detail: &str) -> (u16, Value, Vec<String>) {
    let body = json!({"type": "urn:ietf:params:jmap:error:limit", "status": 400, "limit": name, "detail": detail});
    (400, body, Vec::new())
}

fn problem(status: u16, kind: &str, detail: &str) -> Value {
    json!({"type": kind, "status": status, "detail": detail})
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// The arguments of a method call, with every `#name` replaced by the result it refers to.
fn arguments(args: &Value, done: &[Done]) -> Outcome<Map<String, Value>> {
    let Some(map) = args.as_object() else {
        return Err(invalid("the arguments are an object"));
    };
    let mut out = Map::new();
    for (key, value) in map {
        match key.strip_prefix('#') {
            Some(name) if map.contains_key(name) => return Err(invalid("an argument is given by value and by reference")),
            Some(name) => out.insert(name.to_string(), reference(value, done)?),
            None => out.insert(key.clone(), value.clone()),
        };
    }
    Ok(out)
}

/// A result reference (RFC 8620 §3.7): `{resultOf, name, path}`.
fn reference(value: &Value, done: &[Done]) -> Outcome<Value> {
    let bad = || refuse("invalidResultReference", "the reference doesn't name a result of an earlier call");
    let text = |key: &str| value.get(key).and_then(Value::as_str);
    let (Some(of), Some(name), Some(path)) = (text("resultOf"), text("name"), text("path")) else {
        return Err(invalid("a reference has resultOf, name and path"));
    };
    let earlier = done.iter().find(|d| d.id == of && d.name == name).and_then(|d| d.result.as_ref()).ok_or_else(bad)?;
    let segments: Vec<&str> = path.split('/').collect();
    match segments.split_first() {
        Some((&"", rest)) if rest.len() <= MAX_POINTER => pointer(earlier, rest).ok_or_else(bad),
        _ => Err(bad()),
    }
}

/// The value at an RFC 6901 pointer, in which `*` stands for every item of a list (RFC 8620 §3.7:
/// the lists found are joined into one).
fn pointer(value: &Value, segments: &[&str]) -> Option<Value> {
    let Some((first, rest)) = segments.split_first() else {
        return Some(value.clone());
    };
    let key = first.replace("~1", "/").replace("~0", "~");
    match value {
        Value::Array(items) if key == "*" => {
            let mut joined = Vec::new();
            for item in items {
                match pointer(item, rest)? {
                    Value::Array(inner) => joined.extend(inner),
                    other => joined.push(other),
                }
            }
            Some(Value::Array(joined))
        }
        Value::Array(items) => pointer(items.get(key.parse::<usize>().ok()?)?, rest),
        Value::Object(map) => pointer(map.get(&key)?, rest),
        _ => None,
    }
}

fn allow_only(args: &Map<String, Value>, names: &[&str]) -> Outcome<()> {
    match args.keys().find(|key| !names.contains(&key.as_str())) {
        Some(key) => Err(invalid(format!("no argument called {}", key.chars().take(40).collect::<String>()))),
        None => Ok(()),
    }
}

fn account(args: &Map<String, Value>) -> Outcome<()> {
    match args.get("accountId").and_then(Value::as_str) {
        Some(ACCOUNT) => Ok(()),
        Some(_) => Err(refuse("accountNotFound", "no such account")),
        None => Err(invalid("accountId is needed")),
    }
}

/// The `ids` of a `get`: `None` for `null`.
fn ids(args: &Map<String, Value>, most: usize) -> Outcome<Option<Vec<String>>> {
    match args.get("ids") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.len() > most => {
            Err(refuse("requestTooLarge", format!("{} ids, and {most} are taken at a time", items.len())))
        }
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().filter(|id| valid_id(id)).map(str::to_string))
            .collect::<Option<Vec<String>>>()
            .map(Some)
            .ok_or_else(|| invalid("an id is 1 to 512 characters of A-Z a-z 0-9 - _")),
        Some(_) => Err(invalid("ids is a list, or null")),
    }
}

fn properties(args: &Map<String, Value>, allowed: &[&str], default: &[&str]) -> Outcome<Vec<String>> {
    match args.get("properties") {
        None | Some(Value::Null) => Ok(default.iter().map(|p| p.to_string()).collect()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().filter(|p| allowed.contains(p)).map(str::to_string))
            .collect::<Option<Vec<String>>>()
            .ok_or_else(|| invalid("a property this server doesn't have")),
        Some(_) => Err(invalid("properties is a list, or null")),
    }
}

fn number(args: &Map<String, Value>, key: &str) -> Outcome<Option<usize>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            value.as_u64().map(|n| Some(usize::try_from(n).unwrap_or(usize::MAX))).ok_or_else(|| invalid(format!("{key} is an unsigned integer")))
        }
    }
}

/// Whether a query is sorted oldest first: by `receivedAt`, and by nothing else.
fn sort_direction(args: &Map<String, Value>) -> Outcome<bool> {
    let unsupported = || refuse("unsupportedSort", "only receivedAt can be sorted by");
    match args.get("sort") {
        None | Some(Value::Null) => Ok(true),
        Some(Value::Array(comparators)) => {
            let mut ascending = None;
            for comparator in comparators {
                let Some(comparator) = comparator.as_object() else {
                    return Err(unsupported());
                };
                if comparator.get("property").and_then(Value::as_str) != Some("receivedAt")
                    || comparator.keys().any(|k| k != "property" && k != "isAscending")
                {
                    return Err(unsupported());
                }
                let up = match comparator.get("isAscending") {
                    None => true,
                    Some(Value::Bool(up)) => *up,
                    Some(_) => return Err(invalid("isAscending is a boolean")),
                };
                ascending.get_or_insert(up);
            }
            Ok(ascending.unwrap_or(true))
        }
        Some(_) => Err(unsupported()),
    }
}

fn email_json(email: &Email, properties: &[String]) -> Value {
    let text = String::from_utf8_lossy(&email.raw);
    let (fields, body) = split_message(&text);
    let truths = |set: &BTreeSet<String>| Value::Object(set.iter().map(|k| (k.clone(), Value::Bool(true))).collect());
    let mut out = Map::new();
    out.insert("id".into(), json!(email.id));
    for property in properties {
        let value = match property.as_str() {
            "blobId" => json!(email.blob),
            "mailboxIds" => truths(&email.mailboxes),
            "keywords" => truths(&email.keywords),
            "size" => json!(email.raw.len()),
            "receivedAt" => json!(email.received),
            "hasAttachment" => json!(text.to_ascii_lowercase().contains("content-disposition: attachment")),
            "preview" => json!(body.split_whitespace().collect::<Vec<&str>>().join(" ").chars().take(MAX_PREVIEW).collect::<String>()),
            "headers" => Value::Array(fields.iter().map(|(name, value)| json!({"name": name, "value": value})).collect()),
            _ => continue,
        };
        out.insert(property.clone(), value);
    }
    Value::Object(out)
}

/// A message as its header fields (each with its name and the raw value: all that follows the
/// colon, the folds kept, RFC 8621 §4.1.2) and its body.
fn split_message(text: &str) -> (Vec<(String, String)>, &str) {
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut offset = 0;
    let mut body_at = text.len();
    for line in text.split_inclusive('\n') {
        offset += line.len();
        let content = line.trim_end_matches(['\r', '\n']);
        if content.is_empty() {
            body_at = offset;
            break;
        }
        if content.starts_with([' ', '\t']) {
            if let Some((_, value)) = fields.last_mut() {
                value.push_str("\r\n");
                value.push_str(content);
            }
        } else if let Some((name, value)) = content.split_once(':')
            && !name.is_empty()
            && !name.ends_with(char::is_whitespace)
            && fields.len() < MAX_HEADERS
        {
            fields.push((name.to_string(), value.to_string()));
        }
    }
    (fields, text.get(body_at..).unwrap_or_default())
}

/// An in-process JMAP server. Clones share it.
#[derive(Clone)]
pub struct StandIn {
    data: Arc<Mutex<Data>>,
}

impl Default for StandIn {
    fn default() -> StandIn {
        StandIn::new()
    }
}

impl StandIn {
    /// A server with no mailbox, that takes 16 calls and 500 ids, and lets in [`USER`] with [`PASSWORD`]
    /// and the bearer token [`TOKEN`].
    pub fn new() -> StandIn {
        StandIn { data: Arc::new(Mutex::new(Data::new())) }
    }

    /// Look at what it holds, or change it, for as long as `f` runs (hold nothing across a request).
    pub fn with<R>(&self, f: impl FnOnce(&mut Data) -> R) -> R {
        let mut data = self.data.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut data)
    }

    /// The requests it was sent so far.
    pub fn seen(&self) -> Vec<Seen> {
        self.with(|d| d.seen.clone())
    }

    /// The methods it was asked to call so far, in order.
    pub fn calls(&self) -> Vec<String> {
        self.with(|d| d.seen.iter().flat_map(|s| s.calls.iter().cloned()).collect())
    }
}

impl std::fmt::Debug for StandIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.with(|d| f.debug_struct("StandIn").field("mailboxes", &d.mailboxes.len()).field("emails", &d.emails.len()).finish_non_exhaustive())
    }
}

impl Http for StandIn {
    async fn send(&self, req: Request) -> Result<Response> {
        let resp = self.with(|d| d.handle(&req))?;
        // As an HTTP client does: an answer over the size asked for is no answer.
        if resp.body.len() > req.max {
            return Err(Error::TooLarge(format!("the answer is larger than {} bytes", req.max)));
        }
        Ok(resp)
    }
}

#[cfg(test)]
mod tests;
