// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of exchanges.rs.
//! What the Échanges pane asks of the Neoffice server, as a call of its methods. Every address and id is invented.

use mailcraft_ui_egui::exchanges::Want;
use serde_json::json;

use super::*;

const ME: &str = "ada@example.org";
const CLIENT: &str = "claire@atelier-exemple.example";

#[test]
fn who_someone_is_and_what_neoffice_links_them_to_are_asked_by_their_address() {
    let who = Want::Someone { mailbox: ME.into(), address: CLIENT.into() };
    assert_eq!(call_of("A1", &who), Call { method: "NeomailContact/get", arguments: json!({"accountId": "A1", "address": CLIENT}) });
    let linked = Want::Linked { mailbox: ME.into(), address: CLIENT.into() };
    assert_eq!(call_of("A1", &linked), Call { method: "NeomailLinked/get", arguments: json!({"accountId": "A1", "address": CLIENT}) });
}

#[test]
fn a_page_of_messages_is_asked_by_the_addresses_a_page_long_and_after_a_moment() {
    let first = Want::Page { mailbox: ME.into(), key: "k".into(), addresses: vec![CLIENT.into()], before: None, cc: false, domain: false };
    let call = call_of("A1", &first);
    assert_eq!(call.method, "NeomailHistory/get");
    assert_eq!(call.arguments, json!({"accountId": "A1", "addresses": [CLIENT], "limit": 20}));
    let next = Want::Page {
        mailbox: ME.into(),
        key: "k".into(),
        addresses: vec![CLIENT.into(), "marc@atelier-exemple.example".into()],
        before: Some("2026-10-29T10:00:00Z".into()),
        cc: true,
        domain: true,
    };
    let arguments = call_of("A1", &next).arguments;
    assert_eq!(arguments["before"], "2026-10-29T10:00:00Z");
    assert_eq!((arguments["includeCc"].as_bool(), arguments["domains"].as_bool()), (Some(true), Some(true)));
    assert_eq!(arguments["addresses"].as_array().map(Vec::len), Some(2));
}

#[test]
fn a_want_is_for_the_mailbox_it_names() {
    assert_eq!(mailbox_of(&Want::Someone { mailbox: ME.into(), address: CLIENT.into() }), ME);
    assert_eq!(mailbox_of(&Want::Linked { mailbox: "x@example.org".into(), address: CLIENT.into() }), "x@example.org");
    assert_eq!(mailbox_of(&Want::Page { mailbox: ME.into(), key: String::new(), addresses: Vec::new(), before: None, cc: false, domain: false }), ME);
}

// --- the ask itself, against a server that is scripted to fail first ---------------------------------------------------------

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use mailcraft_jmap::{Auth, Client, Error, Http, Request, Response, block_on};
use mailcraft_ui_egui::exchanges::Trouble;

use crate::credentials::{Credentials, Renewal};

const SESSION: &str = "https://mail.example.org";

/// What the server does to a request before it answers well.
enum Fault {
    /// An HTTP status of its own (a gateway's 502 while the server starts again), and the `Retry-After` it gives.
    Status(u16, Option<u32>),
    /// No answer at all: the network blinked.
    Blink,
    /// A JMAP error of the method alone.
    Method(&'static str),
}

/// A server that takes one token, offers the history (or not), and fails the requests it is told to, in order, before it answers.
struct Server {
    takes: &'static str,
    offered: bool,
    on_session: RefCell<VecDeque<Fault>>,
    on_api: RefCell<VecDeque<Fault>>,
    sessions: Cell<u32>,
    methods: RefCell<Vec<String>>,
}

impl Server {
    fn plain(takes: &'static str) -> Server {
        Server {
            takes,
            offered: true,
            on_session: RefCell::new(VecDeque::new()),
            on_api: RefCell::new(VecDeque::new()),
            sessions: Cell::new(0),
            methods: RefCell::new(Vec::new()),
        }
    }

    fn new(takes: &'static str) -> Rc<Server> {
        Rc::new(Server::plain(takes))
    }

    fn session(&self) -> Value {
        let mut capabilities = json!({
            "urn:ietf:params:jmap:core": {"maxObjectsInGet": 500, "maxCallsInRequest": 16},
            "urn:ietf:params:jmap:mail": {},
        });
        if self.offered {
            capabilities[HISTORY] = json!({});
        }
        json!({
            "capabilities": capabilities,
            "accounts": {"A1": {"name": ME, "isPersonal": true, "isReadOnly": false, "accountCapabilities": {"urn:ietf:params:jmap:mail": {}}}},
            "primaryAccounts": {"urn:ietf:params:jmap:mail": "A1"},
            "username": ME,
            "apiUrl": "/jmap/api",
            "downloadUrl": "/jmap/download/{accountId}/{blobId}/{name}?type={type}",
            "uploadUrl": "/jmap/upload/{accountId}/",
            "state": "0",
        })
    }
}

/// The server as a transport (a type of this crate: `Rc` is not).
#[derive(Clone)]
struct Facade(Rc<Server>);

fn said(status: u16, body: &Value, retry_after: Option<u32>) -> mailcraft_jmap::Result<Response> {
    Ok(Response { status, body: serde_json::to_vec(body).unwrap(), retry_after })
}

impl Http for Facade {
    async fn send(&self, req: Request) -> mailcraft_jmap::Result<Response> {
        let server = &self.0;
        let welcome =
            req.headers.iter().any(|(name, value)| name.eq_ignore_ascii_case("authorization") && *value == format!("Bearer {}", server.takes));
        let session = req.url.ends_with("/.well-known/jmap");
        if !session && !req.url.ends_with("/jmap/api") {
            return Ok(Response { status: 404, ..Default::default() });
        }
        if session {
            server.sessions.set(server.sessions.get() + 1);
        }
        let fault = if session { server.on_session.borrow_mut().pop_front() } else { server.on_api.borrow_mut().pop_front() };
        match fault {
            Some(Fault::Status(status, retry_after)) => return said(status, &json!({}), retry_after),
            Some(Fault::Blink) => return Err(Error::Io("the network blinked".into())),
            _ if !welcome => return Ok(Response { status: 401, ..Default::default() }),
            _ => {}
        }
        if session {
            return said(200, &server.session(), None);
        }
        let request: Value = serde_json::from_slice(&req.body).unwrap();
        let call = &request["methodCalls"][0];
        let (name, id) = (call[0].as_str().unwrap().to_string(), call[2].clone());
        server.methods.borrow_mut().push(name.clone());
        let answer = match fault {
            Some(Fault::Method(kind)) => json!(["error", {"type": kind}, id]),
            _ => json!([name, {"accountId": "A1", "list": [{"id": "m1"}]}, id]),
        };
        said(200, &json!({"methodResponses": [answer], "sessionState": "0"}), None)
    }
}

/// What the page holds for the mailbox: a token, and the one it gives when the server refused the first (none: it has none to give).
struct Page {
    token: RefCell<String>,
    fresh: Option<&'static str>,
    renewals: Cell<u32>,
}

impl Page {
    fn holding(token: &str, fresh: Option<&'static str>) -> Page {
        Page { token: RefCell::new(token.to_string()), fresh, renewals: Cell::new(0) }
    }
}

impl Credentials for Page {
    fn auth(&self, _email: &str) -> Option<Auth> {
        Some(Auth::bearer(self.token.borrow().clone()))
    }

    async fn renew(&self, _email: &str) -> Renewal {
        self.renewals.set(self.renewals.get() + 1);
        match self.fresh {
            Some(token) => {
                *self.token.borrow_mut() = token.to_string();
                Renewal::Given(Auth::bearer(token))
            }
            None => Renewal::Reload,
        }
    }
}

fn first_page() -> Want {
    Want::Page { mailbox: ME.into(), key: "k".into(), addresses: vec![CLIENT.into()], before: None, cc: false, domain: false }
}

type Ended = (Option<Rc<Client<Facade>>>, Result<Value, Trouble>);

/// One ask of the server, and the waits it made before trying again (the test does not wait: it notes them).
fn ask_once(server: &Rc<Server>, page: &Page, kept: Option<Rc<Client<Facade>>>) -> (Ended, Vec<u32>) {
    let waits = RefCell::new(Vec::new());
    let ended = block_on(ask(Facade(server.clone()), page, ME, SESSION, kept, &first_page(), |ms| {
        waits.borrow_mut().push(ms);
        async {}
    }));
    (ended, waits.into_inner())
}

#[test]
fn a_trouble_that_passes_is_told_from_one_that_does_not() {
    let busy = |status, retry_after| Error::Busy { status, retry_after };
    assert_eq!(passes(&Error::Io("offline".into())), Some(AGAIN_MS));
    assert_eq!(passes(&Error::Bad("HTTP 502".into())), Some(AGAIN_MS), "a gateway while the server starts again");
    assert_eq!(passes(&Error::Bad("HTTP 504 ".into())), Some(AGAIN_MS));
    assert_eq!(passes(&Error::Bad("HTTP 500 boom".into())), Some(AGAIN_MS));
    assert_eq!(passes(&busy(503, None)), Some(AGAIN_MS));
    assert_eq!(passes(&busy(429, Some(2))), Some(2_000), "what the server asked for");
    assert_eq!(passes(&busy(429, Some(60))), None, "more than a few seconds is for the pane to say");
    assert_eq!(passes(&Error::Refused { kind: "serverFail".into(), description: String::new() }), Some(AGAIN_MS));
    assert_eq!(passes(&Error::Refused { kind: "serverUnavailable".into(), description: String::new() }), Some(AGAIN_MS));
    // what trying again does not change
    assert_eq!(passes(&Error::Auth("refused".into())), None);
    assert_eq!(passes(&Error::Bad("HTTP 404".into())), None);
    assert_eq!(passes(&Error::Bad("HTTP 400 ".into())), None);
    assert_eq!(passes(&Error::Refused { kind: "unknownMethod".into(), description: String::new() }), None);
    assert_eq!(passes(&Error::Protocol("no methodResponses".into())), None);
    assert_eq!(passes(&Error::Tls("not https".into())), None);
}

#[test]
fn an_ask_the_server_answers_is_one_exchange_and_its_client_is_kept_for_the_next() {
    let server = Server::new("token");
    let page = Page::holding("token", None);
    let ((kept, answer), waits) = ask_once(&server, &page, None);
    assert_eq!(answer.unwrap()["list"][0]["id"], "m1");
    assert!(waits.is_empty() && kept.is_some());
    assert_eq!((server.sessions.get(), server.methods.borrow().as_slice()), (1, ["NeomailHistory/get".to_string()].as_slice()));
    // the next ask is made through the client kept: nobody signs in again
    let ((_, answer), _) = ask_once(&server, &page, kept);
    assert!(answer.is_ok());
    assert_eq!((server.sessions.get(), server.methods.borrow().len()), (1, 2));
}

#[test]
fn a_first_ask_that_meets_a_server_starting_again_is_tried_once_more_without_the_person_seeing_it() {
    // The gateway's 502 while the server's workers start again, at the call: the one more try signs in anew and is answered.
    let server = Server::new("token");
    server.on_api.borrow_mut().push_back(Fault::Status(502, None));
    let ((kept, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert!(kept.is_some());
    assert_eq!(waits, [AGAIN_MS], "a moment's wait, once");
    assert_eq!(server.sessions.get(), 2, "the second try signs in anew");
    assert_eq!(server.methods.borrow().len(), 1, "the 502 never reached the method");
}

#[test]
fn a_session_the_server_cannot_give_yet_is_asked_again_once() {
    let server = Server::new("token");
    server.on_session.borrow_mut().push_back(Fault::Status(502, None));
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!((waits, server.sessions.get()), (vec![AGAIN_MS], 2));

    let server = Server::new("token");
    server.on_session.borrow_mut().push_back(Fault::Blink);
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(waits, [AGAIN_MS]);
}

#[test]
fn a_method_that_failed_on_the_servers_side_is_asked_again_once() {
    let server = Server::new("token");
    server.on_api.borrow_mut().push_back(Fault::Method("serverFail"));
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!((waits, server.methods.borrow().len()), (vec![AGAIN_MS], 2));
}

#[test]
fn the_same_trouble_twice_is_the_panes_to_tell_and_nothing_is_tried_a_third_time() {
    let server = Server::new("token");
    server.on_api.borrow_mut().extend([Fault::Status(502, None), Fault::Status(502, None), Fault::Status(502, None)]);
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert_eq!(answer, Err(Trouble::Failed));
    assert_eq!(waits, [AGAIN_MS]);
    assert_eq!(server.sessions.get(), 2);
    assert_eq!(server.on_api.borrow().len(), 1, "the third 502 was never asked for");
}

#[test]
fn a_busy_server_is_waited_for_what_it_asks_and_not_for_more_than_a_few_seconds() {
    let server = Server::new("token");
    server.on_api.borrow_mut().push_back(Fault::Status(503, Some(2)));
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(waits, [2_000]);

    let server = Server::new("token");
    server.on_api.borrow_mut().push_back(Fault::Status(503, Some(600)));
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert_eq!(answer, Err(Trouble::Failed));
    assert!(waits.is_empty(), "ten minutes is not for an ask to wait for");
}

#[test]
fn a_token_the_server_refuses_at_the_session_is_renewed_by_the_page_once_and_the_ask_goes_on() {
    // The token the page handed when it started is not the one the server takes any more (its cache was emptied, say).
    let server = Server::new("new-token");
    let page = Page::holding("old-token", Some("new-token"));
    let ((kept, answer), waits) = ask_once(&server, &page, None);
    assert!(answer.is_ok(), "{answer:?}");
    assert!(kept.is_some() && waits.is_empty());
    assert_eq!((page.renewals.get(), server.sessions.get()), (1, 2), "refused, renewed, signed in again");
    // and the page now holds the new one: the next ask needs no renewal
    let ((_, answer), _) = ask_once(&server, &page, None);
    assert!(answer.is_ok());
    assert_eq!(page.renewals.get(), 1);
}

#[test]
fn a_token_the_server_refuses_at_the_call_is_renewed_too() {
    let server = Server::new("token");
    server.on_api.borrow_mut().push_back(Fault::Status(401, None));
    let page = Page::holding("token", Some("token"));
    let ((_, answer), waits) = ask_once(&server, &page, None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!((page.renewals.get(), waits.len()), (1, 0));
}

#[test]
fn a_page_with_no_new_token_ends_the_ask_at_once_and_a_token_refused_again_is_not_renewed_in_a_loop() {
    let server = Server::new("new-token");
    let page = Page::holding("old-token", None);
    let ((kept, answer), waits) = ask_once(&server, &page, None);
    assert_eq!(answer, Err(Trouble::Failed));
    assert!(kept.is_none() && waits.is_empty());
    assert_eq!((page.renewals.get(), server.sessions.get()), (1, 1));

    let page = Page::holding("old-token", Some("still-wrong"));
    let ((_, answer), _) = ask_once(&server, &page, None);
    assert_eq!(answer, Err(Trouble::Failed));
    assert_eq!(page.renewals.get(), 1, "once");
}

#[test]
fn a_server_without_the_history_or_without_its_method_is_unsupported_and_never_waited_for() {
    let server = Rc::new(Server { offered: false, ..Server::plain("token") });
    let ((kept, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert_eq!(answer, Err(Trouble::Unsupported));
    assert!(kept.is_none() && waits.is_empty() && server.methods.borrow().is_empty());

    let server = Server::new("token");
    server.on_api.borrow_mut().push_back(Fault::Method("unknownMethod"));
    let ((kept, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert_eq!(answer, Err(Trouble::Unsupported));
    assert!(kept.is_none() && waits.is_empty());
}
