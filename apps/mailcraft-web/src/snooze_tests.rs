// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of snooze.rs.
//! What the window asks of the Neoffice server for « Plus tard », what it makes of the answers, and a whole round against a
//! stand-in for the server that answers `NeomailSnooze/get` and `/set`. Every address, name and id is invented.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mailcraft_engine::Session;
use mailcraft_jmap::{Auth, Http, Request, Response, block_on};
use mailcraft_model::sync::ServerMessage;
use mailcraft_model::{FolderKind, Id, Mailbox, Message, ServerFolder};
use serde_json::{Value, json};

use super::*;
use crate::credentials::Renewal;

const ME: &str = "ada@example.org";
const SESSION: &str = "https://mail.example.org";
/// Thursday 2026-11-19 12:00 UTC.
const NOW: i64 = 1_795_089_600;

/// A server that offers the capability (or not), and answers what it is asked with what a test set, noting each call.
struct Server {
    offered: bool,
    get: RefCell<Value>,
    set: RefCell<Value>,
    calls: RefCell<Vec<(String, Value)>>,
    sessions: Cell<u32>,
}

impl Server {
    fn new(offered: bool) -> Rc<Server> {
        Rc::new(Server {
            offered,
            get: RefCell::new(json!({"accountId": "A1", "state": "0", "list": [], "notFound": [], "mailboxId": null})),
            set: RefCell::new(json!({})),
            calls: RefCell::new(Vec::new()),
            sessions: Cell::new(0),
        })
    }

    fn session(&self) -> Value {
        let mut capabilities = json!({"urn:ietf:params:jmap:core": {"maxObjectsInGet": 500}, "urn:ietf:params:jmap:mail": {}});
        if self.offered {
            capabilities[SNOOZE] = json!({});
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

    fn api(&self, body: &[u8]) -> Value {
        let request: Value = serde_json::from_slice(body).unwrap();
        let mut responses = Vec::new();
        for call in request["methodCalls"].as_array().unwrap() {
            let (name, args, id) = (call[0].as_str().unwrap(), call[1].clone(), call[2].clone());
            self.calls.borrow_mut().push((name.to_string(), args));
            let answer = match name {
                GET => self.get.borrow().clone(),
                SET => self.set.borrow().clone(),
                _ => json!({"type": "unknownMethod"}),
            };
            responses.push(json!([name, answer, id]));
        }
        json!({"methodResponses": responses, "sessionState": "0"})
    }
}

#[derive(Clone)]
struct Shared(Rc<Server>);

impl Http for Shared {
    async fn send(&self, req: Request) -> mailcraft_jmap::Result<Response> {
        let server = &self.0;
        let body = if req.url.ends_with("/.well-known/jmap") {
            server.sessions.set(server.sessions.get() + 1);
            server.session()
        } else if req.url.ends_with("/jmap/api") {
            server.api(&req.body)
        } else {
            return Ok(Response { status: 404, ..Default::default() });
        };
        Ok(Response { status: 200, body: serde_json::to_vec(&body).unwrap(), retry_after: None })
    }
}

struct Page;

impl Credentials for Page {
    fn auth(&self, _email: &str) -> Option<Auth> {
        Some(Auth::bearer("token-of-the-page"))
    }
    async fn renew(&self, _email: &str) -> Renewal {
        Renewal::Reload
    }
}

fn call(server: &Rc<Server>, method: &'static str, arguments: impl Fn(&str) -> Value) -> Outcome<Value> {
    block_on(ask(Shared(server.clone()), &Page, ME, SESSION, None, method, arguments, |_| async {})).1
}

/// A hosting page's tab with Ada's JMAP account: its Inbox and its Snoozed folder (`MSnz`) the server has, and a message
/// in the Inbox. The session, the account, the message and the Snoozed folder.
fn tab() -> (Session, Id, Id, Id) {
    let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
    s.jmap_runs = true;
    s.keeps_local = false;
    let account =
        s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap()["id"].as_u64().unwrap();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    s.mailbox.folder_mut(inbox).unwrap().server = Some(ServerFolder { id: "Min".into(), name: "INBOX".into(), ..Default::default() });
    let snoozed = s.mailbox.add_folder_free(account, None, "Plus tard").unwrap();
    s.mailbox.folder_mut(snoozed).unwrap().server = Some(ServerFolder { id: "MSnz".into(), name: "Plus tard".into(), ..Default::default() });
    let server = ServerMessage { id: "E1".into(), folder: "Min".into(), ..Default::default() };
    let m = s
        .mailbox
        .add_message(Message {
            folder: inbox,
            subject: "Quote".into(),
            message_id: "<quote@example.org>".into(),
            server: Some(server),
            ..Default::default()
        })
        .unwrap();
    (s, account, m, snoozed)
}

#[test]
fn the_records_read_are_put_as_the_window_takes_them_and_only_those_that_wait() {
    let answer = json!({
        "list": [
            {"id": "r1", "emailId": "E5", "messageId": "plan@example.org", "fromMailboxId": "Min", "until": "2026-11-20T07:00:00Z", "state": "waiting"},
            {"id": "r2", "emailId": "E6", "messageId": "offer@example.org", "until": "2026-11-19T13:00:00Z", "state": "failed", "error": "The mail server could not be reached."},
            {"id": "r3", "until": "2026-11-19T10:00:00Z", "state": "returned"},
            {"id": "r4", "until": "not a time", "state": "waiting"},
        ],
        "mailboxId": "MSnz",
    });
    let put = records("Ada@Example.org ", &answer).unwrap();
    assert_eq!(put["account"], json!(ME));
    assert_eq!(put["mailbox"], json!("MSnz"));
    let listed = put["records"].as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["until"], json!(1_795_158_000));
    assert_eq!(listed[1]["failed"], json!("The mail server could not be reached."));
    // A failed one is not waited for: the next read comes for the waiting one alone.
    assert_eq!(next_read_in(&put, 1_795_158_000 + AFTER_DUE_S - 30), 30);
    assert!(records(ME, &json!({"nothing": true})).is_none());
}

#[test]
fn a_late_return_is_neither_read_for_again_and_again_nor_synchronised_twice() {
    let put = |until: i64| json!({"records": [{"id": "r1", "until": until}, {"id": "r2", "until": NOW + 86_400, "failed": "x"}]});
    // Its time to come: read a little after it, never sooner than a couple of seconds, never later than a minute.
    assert_eq!(next_read_in(&put(NOW + 10), NOW), EVERY_S);
    assert_eq!(next_read_in(&put(NOW + 10), NOW + 60), 10 + AFTER_DUE_S - 60, "a little after its time");
    assert_eq!(next_read_in(&put(NOW + 10), NOW + 74), SOON_S);
    assert_eq!(next_read_in(&put(NOW + 3_600), NOW), EVERY_S);
    // Past, and late: read every minute, not every second.
    assert_eq!(next_read_in(&put(NOW - 600), NOW), EVERY_S);
    let mut asked = std::collections::BTreeSet::new();
    assert_eq!(come_back(&put(NOW - 600), NOW, &asked), ["r1"]);
    asked.insert("r1".to_string());
    assert!(come_back(&put(NOW - 600), NOW, &asked).is_empty(), "its Send/Receive asked for once");
    assert!(come_back(&put(NOW + 10), NOW, &std::collections::BTreeSet::new()).is_empty(), "not yet");
}

#[test]
fn the_asks_go_as_one_set_and_its_answer_is_read_for_each() {
    let put = Ask {
        account: 1,
        message: 7,
        what: Asked::Put { email_id: "E1".into(), message_id: "<quote@example.org>".into(), until: NOW + 3600, from: 2, placed: true, was: None },
        sent: true,
    };
    let back = Ask { account: 1, message: 8, what: Asked::Return { record: "r8".into(), to: 2, until: NOW, from: 2 }, sent: true };
    let refused = Ask { message: 9, ..put.clone() };
    let args = set_arguments("A1", &[put.clone(), back.clone(), refused.clone()]);
    assert_eq!(args["create"]["m7"], json!({"emailId": "E1", "messageId": "<quote@example.org>", "until": "2026-11-19T13:00:00Z"}));
    assert_eq!(args["update"], json!({"r8": {"state": "returned"}}));
    let answer = json!({
        "created": {"m7": {"id": "r7", "emailId": "E70", "mailboxId": "MSnz"}},
        "notCreated": {"m9": {"type": "notFound", "description": "This message is no longer in its folder."}},
        "updated": {"r8": {"state": "returned", "emailId": "E80", "mailboxId": "Min"}},
    });
    assert_eq!(
        answers(&[put, back.clone(), refused], &answer),
        [
            json!({"message": 7, "ok": true, "record": "r7", "emailId": "E70", "mailboxId": "MSnz"}),
            json!({"message": 8, "ok": true, "emailId": "E80", "mailboxId": "Min"}),
            json!({"message": 9, "ok": false, "error": "This message is no longer in its folder."}),
        ]
    );
    // No answer at all (the server could not be asked): each is refused, and the window puts it back.
    let none = answers(&[Ask { message: 7, ..back }], &Value::Null);
    assert_eq!(none, [json!({"message": 7, "ok": false})]);
}

#[test]
fn a_server_without_the_capability_is_not_asked() {
    let server = Server::new(false);
    assert!(matches!(call(&server, GET, |account| json!({"accountId": account})), Outcome::Unsupported));
    assert!(server.calls.borrow().is_empty());
}

#[test]
fn a_whole_round_from_the_window_to_the_server_and_back() {
    let (mut s, account, m, snoozed) = tab();
    let server = Server::new(true);
    // The tab reads what the server keeps: nothing waits yet, the folder is « Plus tard ».
    *server.get.borrow_mut() = json!({"accountId": "A1", "state": "1", "list": [], "notFound": [], "mailboxId": "MSnz"});
    let Outcome::Answered(read) = call(&server, GET, |account| json!({"accountId": account})) else { panic!("not read") };
    s.run("mail.setSnoozes", &records(ME, &read).unwrap()).unwrap();
    assert!(mailcraft_engine::cmd::snooze_hosted::offered(&s, &[m]));
    // The person puts the message away until tomorrow morning: it is in « Plus tard » at once, and the server is asked.
    s.run("mail.snooze", &json!({"ids": [m], "until": "2026-11-20T08:00"})).unwrap();
    assert_eq!(s.mailbox.message(m).unwrap().folder, snoozed);
    let asks = s.hosted_snooze.take();
    *server.set.borrow_mut() = json!({"created": {format!("m{m}"): {"id": "r1", "emailId": "E9", "mailboxId": "MSnz", "state": "waiting"}}});
    let sent = asks.clone();
    let Outcome::Answered(answer) = call(&server, SET, move |account| set_arguments(account, &sent)) else { panic!("not asked") };
    let (name, args) = server.calls.borrow().last().cloned().unwrap();
    assert_eq!(name, SET);
    assert_eq!(args["accountId"], json!("A1"), "the server's own account id");
    assert_eq!(args["create"][format!("m{m}")]["emailId"], json!("E1"));
    for told in answers(&asks, &answer) {
        s.run("mail.snoozeAnswered", &told).unwrap();
    }
    let here = s.mailbox.message(m).unwrap();
    assert_eq!((here.folder, here.server.as_ref().unwrap().id.as_str()), (snoozed, "E9"));
    assert!(here.snooze.is_some_and(|z| !z.placing && z.waiting()));
    assert_eq!(mailcraft_engine::cmd::snooze_hosted::record(&s, m).map(|r| r.id.as_str()), Some("r1"), "its record, to bring it back early");
    assert!(!s.hosted_snooze.asking());
    let _ = account;
}

/// The page's token, which the in-process stand-in takes.
struct StandInToken;

impl Credentials for StandInToken {
    fn auth(&self, _email: &str) -> Option<Auth> {
        Some(Auth::bearer(mailcraft_jmap::standin::TOKEN))
    }
    async fn renew(&self, _email: &str) -> Renewal {
        Renewal::Reload
    }
}

#[test]
fn against_the_stand_in_a_message_is_put_away_read_back_and_brought_back() {
    use mailcraft_jmap::standin::{SESSION, StandIn};
    let server = StandIn::new();
    let email = server.with(|d| {
        d.snoozes.on = true;
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        d.deliver(&inbox, b"From: a@example.org\r\nSubject: Plan\r\nMessage-ID: <plan@example.org>\r\n\r\nHi\r\n", &["$seen"], "2026-11-19T08:00:00Z")
    });
    let get =
        |server: &StandIn| match block_on(ask(server.clone(), &StandInToken, ME, SESSION, None, GET, |a| json!({"accountId": a}), |_| async {})).1 {
            Outcome::Answered(answer) => answer,
            Outcome::Failed(e) => panic!("{e:?}"),
            Outcome::Unsupported => panic!("unsupported"),
        };
    assert_eq!(get(&server)["list"], json!([]));
    let until = mailcraft_time::format_iso_utc(mailcraft_time::now() + 3_600);
    let put = Ask {
        account: 1,
        message: 7,
        what: Asked::Put {
            email_id: email,
            message_id: "<plan@example.org>".into(),
            until: mailcraft_time::now() + 3_600,
            from: 2,
            placed: true,
            was: None,
        },
        sent: true,
    };
    let sent = vec![put.clone()];
    let Outcome::Answered(answer) =
        block_on(ask(server.clone(), &StandInToken, ME, SESSION, None, SET, move |a| set_arguments(a, &sent), |_| async {})).1
    else {
        panic!("not put away")
    };
    let told = answers(&[put], &answer);
    assert_eq!(told[0]["ok"], json!(true), "{answer}");
    let read = records(ME, &get(&server)).unwrap();
    assert_eq!(read["records"][0]["until"], json!(mailcraft_time::iso_to_timestamp(&until, 0).unwrap()));
    assert!(read["mailbox"].is_string());
}
