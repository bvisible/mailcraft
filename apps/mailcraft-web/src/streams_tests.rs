// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of streams.rs.
//! What the window asks the Neoffice server about, what it makes of the answer, and the ask itself against a server that is a
//! stand-in for the one that answers `NeomailStream/get`. Every address, name and id is invented.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use mailcraft_engine::Session;
use mailcraft_jmap::{Auth, Client, Error, Http, Request, Response, block_on};
use mailcraft_model::sync::ServerMessage;
use mailcraft_model::{Address, FolderKind, Id, Mailbox, Message, Stream};
use serde_json::{Value, json};

use super::*;
use crate::credentials::Renewal;

const ME: &str = "ada@example.org";

fn mailbox() -> (Mailbox, Id) {
    let mb = Mailbox::with_local_account("Ada", ME);
    let inbox = mb.inbox().unwrap();
    (mb, inbox)
}

/// A message of `folder` that the server knows by `server`, `age` seconds old.
fn mail(folder: Id, server: &str, age: i64) -> Message {
    Message {
        folder,
        subject: format!("message {server}"),
        from: Address::new("", "someone@example.org"),
        date: 1_791_993_600 - age,
        received: 1_791_993_600 - age,
        server: (!server.is_empty()).then(|| ServerMessage { id: server.into(), ..Default::default() }),
        ..Default::default()
    }
}

fn add(mb: &mut Mailbox, message: Message) -> Id {
    mb.add_message(message).unwrap()
}

fn servers(plan: &Plan) -> Vec<&str> {
    plan.ask.iter().map(|(_, server)| server.as_str()).collect()
}

#[test]
fn it_asks_about_the_inboxs_messages_that_have_a_server_id_and_no_stream_the_newest_first() {
    let (mut mb, inbox) = mailbox();
    let archive = mb.folder_of_kind(mb.accounts[0].id, FolderKind::Archive).unwrap();
    add(&mut mb, mail(inbox, "old", 3_000));
    add(&mut mb, mail(inbox, "new", 10));
    add(&mut mb, mail(inbox, "middle", 500));
    add(&mut mb, mail(inbox, "", 1)); // no id on the server: nothing to ask
    add(&mut mb, mail(archive, "elsewhere", 1)); // the tabs are the Inbox's
    add(&mut mb, Message { draft: true, ..mail(inbox, "draft", 1) });
    let said = add(&mut mb, mail(inbox, "said", 5));
    mb.set_streams(&[(said, Stream::Notification)]);
    let plan = Book::default().plan(&mb, ME);
    assert_eq!(servers(&plan), ["new", "middle", "old"]);
    assert!(plan.restore.is_empty());
    // another mailbox's address is another mailbox
    assert!(Book::default().plan(&mb, "grace@example.org").ask.is_empty());
}

#[test]
fn an_ask_holds_at_most_what_the_server_takes() {
    let (mut mb, inbox) = mailbox();
    for n in 0..(MAX_IDS + 30) {
        add(&mut mb, mail(inbox, &format!("m{n}"), n as i64));
    }
    let plan = Book::default().plan(&mb, ME);
    assert_eq!(plan.ask.len(), MAX_IDS);
    assert_eq!(plan.ask[0].1, "m0", "the newest first");
}

#[test]
fn what_the_server_said_is_put_and_not_asked_again() {
    let (mut mb, inbox) = mailbox();
    let a = add(&mut mb, mail(inbox, "a", 30));
    let b = add(&mut mb, mail(inbox, "b", 20));
    let mut book = Book::default();
    let plan = book.plan(&mb, ME);
    let answer = Answer { says: vec![("a".into(), Stream::Notification), ("b".into(), Stream::Person)], ..Default::default() };
    let put = book.take(&plan.ask, &answer);
    assert_eq!(put.len(), 2);
    mb.set_streams(&put);
    assert_eq!(mb.message(a).unwrap().stream, Some(Stream::Notification));
    assert_eq!(mb.message(b).unwrap().stream, Some(Stream::Person));
    assert!(book.plan(&mb, ME).ask.is_empty());
    assert!(!book.waiting());
}

#[test]
fn a_synchronisation_that_took_a_stream_gets_it_back_without_a_second_question() {
    let (mut mb, inbox) = mailbox();
    let a = add(&mut mb, mail(inbox, "a", 30));
    let mut book = Book::default();
    let plan = book.plan(&mb, ME);
    let put = book.take(&plan.ask, &Answer { says: vec![("a".into(), Stream::Newsletter)], ..Default::default() });
    mb.set_streams(&put);
    // the synchronisation puts a new copy of the message: its stream is gone
    mb.message_mut(a).unwrap().stream = None;
    let plan = book.plan(&mb, ME);
    assert!(plan.ask.is_empty(), "no second question");
    assert_eq!(plan.restore, vec![(a, Stream::Newsletter)]);
}

#[test]
fn a_message_the_model_has_not_answered_is_asked_again_a_few_times_and_then_left() {
    let (mut mb, inbox) = mailbox();
    let a = add(&mut mb, mail(inbox, "a", 30));
    let mut book = Book::default();
    for round in 1..=MAX_TRIES {
        let plan = book.plan(&mb, ME);
        assert_eq!(servers(&plan), ["a"], "round {round}");
        book.take(&plan.ask, &Answer { pending: vec!["a".into()], ..Default::default() });
        assert_eq!(book.waiting(), round < MAX_TRIES);
    }
    assert!(book.plan(&mb, ME).ask.is_empty(), "left after {MAX_TRIES} tries");
    // left with no stream: the window shows it with the main tab's
    assert_eq!(mb.message(a).unwrap().stream_or_person(), Stream::Person);
}

#[test]
fn a_message_the_server_does_not_find_or_names_a_kind_this_window_does_not_know_is_left_at_once() {
    let (mut mb, inbox) = mailbox();
    add(&mut mb, mail(inbox, "gone", 30));
    add(&mut mb, mail(inbox, "odd", 20));
    let mut book = Book::default();
    let plan = book.plan(&mb, ME);
    let answer = read_answer(&json!({"list": [{"id": "odd", "stream": "promotion"}], "notFound": ["gone"]})).unwrap();
    assert!(book.take(&plan.ask, &answer).is_empty());
    assert!(book.plan(&mb, ME).ask.is_empty());
    assert!(!book.waiting());
}

#[test]
fn an_answer_for_an_id_that_was_not_asked_changes_nothing() {
    let (mut mb, inbox) = mailbox();
    let a = add(&mut mb, mail(inbox, "a", 30));
    let mut book = Book::default();
    let plan = book.plan(&mb, ME);
    let put = book.take(&plan.ask, &Answer { says: vec![("stranger".into(), Stream::Newsletter)], ..Default::default() });
    assert!(put.is_empty());
    assert_eq!(mb.message(a).unwrap().stream, None);
}

#[test]
fn what_an_answer_says_is_read_in_the_servers_words() {
    let answer = read_answer(&json!({
        "accountId": "A1", "state": "0",
        "list": [
            {"id": "n1", "stream": "notification", "source": "headers"},
            {"id": "l1", "stream": "Newsletter", "source": "model"},
            {"id": "p1", "stream": "person", "source": "default"},
            {"stream": "person"},
            {"id": "x1", "stream": 7},
        ],
        "notFound": ["gone", 5],
        "pending": ["slow"],
    }))
    .unwrap();
    assert_eq!(answer.says, [("n1".to_string(), Stream::Notification), ("l1".into(), Stream::Newsletter), ("p1".into(), Stream::Person)]);
    assert_eq!(answer.pending, ["slow"]);
    assert_eq!(answer.left, ["x1", "gone"]);
    assert_eq!(read_answer(&json!({"list": "no"})), None);
    assert_eq!(read_answer(&json!({})), None);
}

#[test]
fn the_arguments_hold_the_account_and_at_most_the_ids_a_server_takes() {
    let ids: Vec<String> = (0..MAX_IDS + 5).map(|n| format!("m{n}")).collect();
    let args = arguments("A1", &ids);
    assert_eq!(args["accountId"], "A1");
    assert_eq!(args["ids"].as_array().unwrap().len(), MAX_IDS);
}

#[test]
fn what_was_said_goes_to_the_window_which_opens_its_tabs_on_the_main_one() {
    let mut mb = Mailbox::with_local_account("Ada", ME);
    let inbox = mb.inbox().unwrap();
    let a = add(&mut mb, mail(inbox, "a", 30));
    let b = add(&mut mb, mail(inbox, "b", 20));
    let mut session = Session::new(mb);
    assert_eq!(put(&mut session, &[]), 0);
    assert_eq!(put(&mut session, &[(a, Stream::Notification), (b, Stream::Person)]), 2);
    assert!(session.mailbox.has_streams(inbox));
    assert_eq!(session.view.filter, mailcraft_engine::Filter::Main);
    // said again: nothing changes, and the owner's choice stays
    session.run("view.filter", &json!({"value": "notifications"})).unwrap();
    assert_eq!(put(&mut session, &[(a, Stream::Notification)]), 0);
    assert_eq!(session.view.filter, mailcraft_engine::Filter::Notifications);
}

// --- the ask itself, against a stand-in for the server -------------------------------------------------------------------

/// What the server does to a request before it answers well.
enum Fault {
    /// An HTTP status of its own (a gateway's 502 while the server starts again), and the `Retry-After` it gives.
    Status(u16, Option<u32>),
    /// No answer at all: the network blinked.
    Blink,
    /// A JMAP error of the method alone.
    Method(&'static str),
}

/// A server that offers the capability (or not) and answers `NeomailStream/get` with what `says` gives for each id it is asked.
/// It takes one token when it is told to, and fails the requests it is told to, in order, before it answers.
struct Server {
    offered: bool,
    says: fn(&str) -> Value,
    asked: RefCell<Vec<Vec<String>>>,
    method_error: Option<&'static str>,
    takes: Option<&'static str>,
    on_session: RefCell<VecDeque<Fault>>,
    on_api: RefCell<VecDeque<Fault>>,
    sessions: Cell<u32>,
}

impl Server {
    fn plain(offered: bool, says: fn(&str) -> Value, method_error: Option<&'static str>) -> Server {
        Server {
            offered,
            says,
            asked: RefCell::new(Vec::new()),
            method_error,
            takes: None,
            on_session: RefCell::new(VecDeque::new()),
            on_api: RefCell::new(VecDeque::new()),
            sessions: Cell::new(0),
        }
    }

    fn new(says: fn(&str) -> Value) -> Rc<Server> {
        Rc::new(Server::plain(true, says, None))
    }

    /// A server that takes `token` and no other.
    fn taking(token: &'static str) -> Rc<Server> {
        Rc::new(Server { takes: Some(token), ..Server::plain(true, says, None) })
    }

    fn session(&self) -> Value {
        let mut capabilities = json!({
            "urn:ietf:params:jmap:core": {"maxObjectsInGet": 500, "maxCallsInRequest": 16},
            "urn:ietf:params:jmap:mail": {},
        });
        if self.offered {
            capabilities[STREAM] = json!({});
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

    fn api(&self, body: &[u8], forced: Option<&'static str>) -> Value {
        let request: Value = serde_json::from_slice(body).unwrap();
        let mut responses = Vec::new();
        for call in request["methodCalls"].as_array().unwrap() {
            let (name, args, id) = (call[0].as_str().unwrap(), &call[1], call[2].clone());
            assert_eq!(name, GET);
            assert_eq!(args["accountId"], "A1");
            let ids: Vec<String> = args["ids"].as_array().unwrap().iter().map(|i| i.as_str().unwrap().to_string()).collect();
            self.asked.borrow_mut().push(ids.clone());
            if let Some(kind) = forced.or(self.method_error) {
                responses.push(json!(["error", {"type": kind}, id]));
                continue;
            }
            let mut list = Vec::new();
            let (mut pending, mut not_found) = (Vec::new(), Vec::new());
            for server in &ids {
                let said = (self.says)(server);
                match said.as_str() {
                    Some("pending") => pending.push(server.clone()),
                    Some("gone") => not_found.push(server.clone()),
                    Some(stream) => list.push(json!({"id": server, "stream": stream, "source": "headers"})),
                    None => {}
                }
            }
            responses.push(json!([GET, {"accountId": "A1", "state": "0", "list": list, "notFound": not_found, "pending": pending}, id]));
        }
        json!({"methodResponses": responses, "sessionState": "0"})
    }
}

/// The server as a transport (a type of this crate: `Rc` is not).
#[derive(Clone)]
struct Shared(Rc<Server>);

impl Http for Shared {
    async fn send(&self, req: Request) -> mailcraft_jmap::Result<Response> {
        let server = &self.0;
        let session = req.url.ends_with("/.well-known/jmap");
        if !session && !req.url.ends_with("/jmap/api") {
            return Ok(Response { status: 404, ..Default::default() });
        }
        if session {
            server.sessions.set(server.sessions.get() + 1);
        }
        let fault = if session { server.on_session.borrow_mut().pop_front() } else { server.on_api.borrow_mut().pop_front() };
        let welcome = server.takes.is_none_or(|token| {
            req.headers.iter().any(|(name, value)| name.eq_ignore_ascii_case("authorization") && *value == format!("Bearer {token}"))
        });
        let mut forced = None;
        match fault {
            Some(Fault::Status(status, retry_after)) => {
                return Ok(Response { status, body: b"{}".to_vec(), retry_after });
            }
            Some(Fault::Blink) => return Err(Error::Io("the network blinked".into())),
            _ if !welcome => return Ok(Response { status: 401, ..Default::default() }),
            Some(Fault::Method(kind)) => forced = Some(kind),
            None => {}
        }
        let body = if session { server.session() } else { server.api(&req.body, forced) };
        Ok(Response { status: 200, body: serde_json::to_vec(&body).unwrap(), retry_after: None })
    }
}

fn connect(server: &Rc<Server>) -> Result<Client<Shared>, Error> {
    block_on(Client::connect(Shared(server.clone()), "https://mail.example.org", &Auth::bearer("token-of-the-page")))
}

fn says(server: &str) -> Value {
    match server {
        "n1" | "n2" => json!("notification"),
        "l1" => json!("newsletter"),
        "slow" => json!("pending"),
        "gone" => json!("gone"),
        _ => json!("person"),
    }
}

#[test]
fn the_ask_gives_what_the_server_said_pending_and_missing() {
    let server = Server::new(says);
    let client = connect(&server).unwrap();
    assert!(client.has_capability(STREAM));
    let ids: Vec<String> = ["n1", "l1", "p1", "slow", "gone"].iter().map(|s| s.to_string()).collect();
    let answer = block_on(ask(&client, &ids)).unwrap();
    assert_eq!(answer.says, [("n1".to_string(), Stream::Notification), ("l1".into(), Stream::Newsletter), ("p1".into(), Stream::Person)]);
    assert_eq!((answer.pending, answer.left), (vec!["slow".to_string()], vec!["gone".to_string()]));
    assert_eq!(server.asked.borrow().as_slice(), [ids]);
}

#[test]
fn a_server_without_the_capability_is_seen_before_anything_is_asked() {
    let server = Rc::new(Server::plain(false, says, None));
    let client = connect(&server).unwrap();
    assert!(!client.has_capability(STREAM));
    assert!(server.asked.borrow().is_empty());
}

#[test]
fn a_server_that_no_longer_knows_the_method_is_a_refusal_the_runner_reads() {
    let server = Rc::new(Server::plain(true, says, Some("unknownMethod")));
    let client = connect(&server).unwrap();
    match block_on(ask(&client, &["n1".to_string()])) {
        Err(Error::Refused { kind, .. }) => assert!(kind.contains("unknownMethod"), "{kind}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn from_the_page_to_the_tabs_a_whole_round() {
    // The window: five messages of the Inbox; the server: two notifications, a newsletter, a person and one the model is late on.
    let (mut mb, inbox) = mailbox();
    let ids: Vec<Id> =
        [("n1", 50), ("n2", 40), ("l1", 30), ("p1", 20), ("slow", 10)].iter().map(|(s, age)| add(&mut mb, mail(inbox, s, *age))).collect();
    let mut session = Session::new(mb);
    let server = Server::new(says);
    let client = connect(&server).unwrap();
    let mut book = Book::default();

    let plan = book.plan(&session.mailbox, ME);
    let answer = block_on(ask(&client, &plan.ask.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>())).unwrap();
    let said = book.take(&plan.ask, &answer);
    assert_eq!(put(&mut session, &said), 4);
    assert!(book.waiting(), "the model is late on one");

    let counts = session.mailbox.stream_counts(inbox);
    assert_eq!((counts.total(Stream::Notification), counts.total(Stream::Newsletter), counts.total(Stream::Person)), (2, 1, 2));
    assert_eq!(session.mailbox.message(ids[4]).unwrap().stream, None, "still nobody's: with the main tab's");

    // a few seconds later only the one it was late on is asked about again
    let plan = book.plan(&session.mailbox, ME);
    assert_eq!(servers(&plan), ["slow"]);
}

// --- the ask as the window makes it: tried once more when the server was only starting again, and signed in anew --------------

const SESSION: &str = "https://mail.example.org";

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

type Ended = (Option<Rc<Client<Shared>>>, Result<Answer, Failure>);

/// One ask about `p1` and `n1`, and the waits it made before trying again (the test does not wait: it notes them).
fn ask_once(server: &Rc<Server>, page: &Page, kept: Option<Rc<Client<Shared>>>) -> (Ended, Vec<u32>) {
    let waits = RefCell::new(Vec::new());
    let ids = ["p1".to_string(), "n1".to_string()];
    let ended = block_on(ask_streams(Shared(server.clone()), page, ME, SESSION, kept, &ids, |ms| {
        waits.borrow_mut().push(ms);
        async {}
    }));
    (ended, waits.into_inner())
}

#[test]
fn an_ask_the_server_answers_is_one_exchange_and_its_client_is_kept_for_the_next() {
    let server = Server::taking("token");
    let page = Page::holding("token", None);
    let ((kept, answer), waits) = ask_once(&server, &page, None);
    let answer = answer.unwrap();
    assert_eq!(answer.says, [("p1".to_string(), Stream::Person), ("n1".into(), Stream::Notification)]);
    assert!(waits.is_empty() && kept.is_some());
    // the next ask is made through the client kept: nobody signs in again
    let ((_, answer), _) = ask_once(&server, &page, kept);
    assert!(answer.is_ok());
    assert_eq!((server.sessions.get(), server.asked.borrow().len()), (1, 2));
}

#[test]
fn a_first_ask_that_meets_a_server_starting_again_is_tried_once_more_without_anyone_waiting_half_a_minute() {
    // The gateway's 502 while the server's workers start again, at the call: the one more try signs in anew and is answered.
    let server = Server::taking("token");
    server.on_api.borrow_mut().push_back(Fault::Status(502, None));
    let ((kept, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert!(kept.is_some());
    assert_eq!(waits, [crate::exchanges::AGAIN_MS], "a moment's wait, once");
    assert_eq!(server.sessions.get(), 2, "the second try signs in anew");
    assert_eq!(server.asked.borrow().len(), 1, "the 502 never reached the method");
}

#[test]
fn a_session_the_server_cannot_give_yet_is_asked_again_once() {
    let server = Server::taking("token");
    server.on_session.borrow_mut().push_back(Fault::Status(502, None));
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!((waits, server.sessions.get()), (vec![crate::exchanges::AGAIN_MS], 2));

    let server = Server::taking("token");
    server.on_session.borrow_mut().push_back(Fault::Blink);
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(waits, [crate::exchanges::AGAIN_MS]);
}

#[test]
fn a_method_that_failed_on_the_servers_side_is_asked_again_once() {
    let server = Server::taking("token");
    server.on_api.borrow_mut().push_back(Fault::Method("serverFail"));
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!((waits, server.asked.borrow().len()), (vec![crate::exchanges::AGAIN_MS], 2));
}

#[test]
fn the_same_trouble_twice_is_told_as_it_was_and_nothing_is_tried_a_third_time() {
    let server = Server::taking("token");
    server.on_api.borrow_mut().extend([Fault::Status(502, None), Fault::Status(502, None), Fault::Status(502, None)]);
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert_eq!(answer, Err(Failure::Trouble("refused")));
    assert_eq!(waits, [crate::exchanges::AGAIN_MS]);
    assert_eq!(server.sessions.get(), 2);
    assert_eq!(server.on_api.borrow().len(), 1, "the third 502 was never asked for");

    let server = Server::taking("token");
    server.on_session.borrow_mut().extend([Fault::Blink, Fault::Blink, Fault::Blink]);
    let ((kept, answer), _) = ask_once(&server, &Page::holding("token", None), None);
    assert_eq!(answer, Err(Failure::Trouble("network")));
    assert!(kept.is_none(), "no client to keep when none could be signed in");
}

#[test]
fn a_busy_server_is_waited_for_what_it_asks_and_not_for_more_than_a_few_seconds() {
    let server = Server::taking("token");
    server.on_api.borrow_mut().push_back(Fault::Status(503, Some(2)));
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(waits, [2_000]);

    let server = Server::taking("token");
    server.on_api.borrow_mut().push_back(Fault::Status(503, Some(600)));
    let ((_, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert_eq!(answer, Err(Failure::Trouble("busy")));
    assert!(waits.is_empty(), "ten minutes is not for an ask to wait for");
}

#[test]
fn a_token_the_server_refuses_at_the_session_is_renewed_by_the_page_once_and_the_ask_goes_on() {
    // The token the page handed when it started is not the one the server takes any more: the tabs never renewed it.
    let server = Server::taking("new-token");
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
    let server = Server::taking("token");
    server.on_api.borrow_mut().push_back(Fault::Status(401, None));
    let page = Page::holding("token", Some("token"));
    let ((_, answer), waits) = ask_once(&server, &page, None);
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!((page.renewals.get(), waits.len()), (1, 0));
}

#[test]
fn when_the_page_has_no_new_token_the_sign_in_is_what_is_told_and_nothing_is_tried_again() {
    let server = Server::taking("new-token");
    let page = Page::holding("old-token", None);
    let ((kept, answer), waits) = ask_once(&server, &page, None);
    assert_eq!(answer, Err(Failure::Trouble("sign-in")));
    assert!(kept.is_none() && waits.is_empty());
    assert_eq!((page.renewals.get(), server.sessions.get()), (1, 1));
}

#[test]
fn a_server_that_does_not_offer_the_tabs_is_never_tried_again() {
    let server = Rc::new(Server::plain(false, says, None));
    let ((kept, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert_eq!(answer, Err(Failure::Unsupported));
    assert!(kept.is_none() && waits.is_empty());
    assert!(server.asked.borrow().is_empty());

    let server = Rc::new(Server::plain(true, says, Some("unknownMethod")));
    let ((kept, answer), waits) = ask_once(&server, &Page::holding("token", None), None);
    assert_eq!(answer, Err(Failure::Unsupported));
    assert!(kept.is_none() && waits.is_empty(), "a method the server no longer has is not a trouble that passes");
    assert_eq!(server.asked.borrow().len(), 1);
}
