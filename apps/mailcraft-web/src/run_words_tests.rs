// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the sentences of a JMAP Send/Receive, read back in French.

//! What the window shows of a JMAP Send/Receive that goes wrong, read back in the interface's language
//! (`mailcraft_ui_egui::i18n::run_words`), from the real producers: the browser's runner ([`run`]) over the in-process
//! server, made to fail in every way it and a stand-in transport can, each piece taken in by the engine
//! (`apply_jmap_mail`) as the window takes it in. Every line, status and banner that comes out must be read, in French,
//! and leave nothing in English but the words a server or the transport said; and every row of the table must be read
//! by some scenario, or say why none can make it. Nothing here leaves this machine; every name and address is invented.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::jmap_mail::{apply_jmap_mail, jmap_mail_request};
use mailcraft_jmap::standin::{Effect, Fault, SESSION, StandIn, TOKEN};
use mailcraft_jmap::{Error, Http, Request, Response, Result, block_on};
use mailcraft_model::{Id, Mailbox};
use mailcraft_ui_egui::i18n::Lang;
use mailcraft_ui_egui::i18n::run_words::{Family, Reading, explain, explain_status, rows};
use mailcraft_ui_egui::jmap_job::JmapJob;
use serde_json::{Value, json};

use super::run;
use crate::credentials::NoCredentials;

const ME: &str = "ada@example.org";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// What a transport does to a request and to the server's answer to it.
type Change = Rc<dyn Fn(&Request, Result<Response>) -> Result<Response>>;

/// The in-process server behind a transport that a test changes: the requests go to the server, and what comes back
/// is what `change` makes of its answer (an answer of its own, an error, the server's answer edited).
#[derive(Clone)]
struct Flaky {
    server: StandIn,
    change: Change,
}

impl Http for Flaky {
    async fn send(&self, req: Request) -> Result<Response> {
        let asked = req.clone();
        let answer = self.server.send(req).await;
        (self.change)(&asked, answer)
    }
}

/// Whether a request is the one for the session.
fn is_session(req: &Request) -> bool {
    req.method == "GET" && req.url.ends_with("/jmap/session")
}

/// The methods a request to the API calls.
fn calls(req: &Request) -> Vec<String> {
    let body: Value = serde_json::from_slice(&req.body).unwrap_or_default();
    body["methodCalls"].as_array().map(|calls| calls.iter().filter_map(|c| c[0].as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// An answer of the transport's own.
fn answer(status: u16, body: &[u8]) -> Result<Response> {
    Ok(Response { status, body: body.to_vec(), retry_after: None })
}

/// The server's answer, its JSON edited by `edit`.
fn edited(answer: Result<Response>, edit: impl Fn(&mut Value)) -> Result<Response> {
    let mut resp = answer?;
    let mut doc: Value = serde_json::from_slice(&resp.body).unwrap_or_default();
    edit(&mut doc);
    resp.body = serde_json::to_vec(&doc).unwrap_or_default();
    Ok(resp)
}

/// What the window is shown of a run: the lines of its problems, the engine's status when it ends, and what each
/// message held in the Outbox says of itself.
#[derive(Debug, Default)]
struct Shown {
    lines: Vec<String>,
    status: String,
    held: Vec<String>,
}

/// A session with one JMAP account, and the in-process server for it: an Inbox with a message, Drafts, Sent and Trash,
/// and who the account may send as.
struct Rig {
    server: StandIn,
    s: Session,
    account: Id,
}

impl Rig {
    fn new() -> Rig {
        let server = StandIn::new();
        server.with(|d| {
            let inbox = d.mailbox("Inbox", None, Some("inbox"));
            d.mailbox("Drafts", None, Some("drafts"));
            d.mailbox("Sent", None, Some("sent"));
            d.mailbox("Trash", None, Some("trash"));
            d.identity("Ada", ME);
            let raw = format!("From: Zoe <zoe@example.org>\r\nTo: Ada <{ME}>\r\nSubject: Hello\r\nMessage-ID: <m1@example.org>\r\n\r\nHello.\r\n");
            d.deliver(&inbox, raw.as_bytes(), &[], "2026-10-14T09:30:00Z");
        });
        let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
        let added = s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
        Rig { server, s, account: added["id"].as_u64().unwrap() }
    }

    /// A rig whose first run went well: the account's folders are the server's.
    fn listed() -> Rig {
        let mut rig = Rig::new();
        let first = rig.run(true);
        assert!(first.lines.is_empty(), "{first:?}");
        rig
    }

    /// A transport to the rig's server that changes what comes back.
    fn through(&self, change: impl Fn(&Request, Result<Response>) -> Result<Response> + 'static) -> Flaky {
        Flaky { server: self.server.clone(), change: Rc::new(change) }
    }

    /// One run over `http`, as the window has it done: what the engine asks for (`full`: every mailbox), done by the
    /// browser's runner with the account's own token, each piece taken in by the engine as it comes.
    fn run_over(&mut self, http: impl Http + Clone, full: bool) -> Shown {
        self.job_over(http, full, false)
    }

    /// [`Rig::run_over`], stopped by the owner before it began when `stopped`.
    fn job_over(&mut self, http: impl Http + Clone, full: bool, stopped: bool) -> Shown {
        let request = jmap_mail_request(&self.s, self.account, full, &[], false).unwrap();
        let job = JmapJob { request, secret: Some(TOKEN.into()), cancel: Arc::new(AtomicBool::new(stopped)) };
        let pieces = RefCell::new(Vec::new());
        block_on(run(http, &job, None::<&NoCredentials>, |_| std::future::ready(()), &mut |piece| pieces.borrow_mut().push(piece)));
        let mut shown = Shown::default();
        for mut piece in pieces.into_inner() {
            piece.account = self.account;
            match apply_jmap_mail(&mut self.s, piece) {
                Ok(said) => shown.lines.extend(said["errors"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string)),
                Err(e) => shown.lines.push(e.to_string()),
            }
        }
        shown.status = self.s.sync_status.clone();
        shown.held = self.s.mailbox.messages.values().filter(|m| !m.held.is_empty()).map(|m| m.held.clone()).collect();
        shown
    }

    fn run(&mut self, full: bool) -> Shown {
        self.run_over(self.server.clone(), full)
    }

    /// Write a message from the account and press Send: it waits in the Outbox.
    fn send(&mut self, to: &str, subject: &str) {
        self.s.run("mail.new", &json!({"to": to, "subject": subject, "body": format!("About {subject}."), "account": self.account})).unwrap();
        self.s.run("compose.send", &json!({})).unwrap();
    }

    /// Mark the messages of the account here as read: changes the server hasn't heard of.
    fn mark_read(&mut self, how_many: usize) {
        let ids: Vec<Id> = self.mine().into_iter().take(how_many).collect();
        for id in ids {
            self.s.run("mail.markRead", &json!({"ids": [id]})).unwrap();
        }
    }

    /// The account's messages here that the server has, the oldest first.
    fn mine(&self) -> Vec<Id> {
        let mut mine: Vec<(i64, Id)> = self
            .s
            .mailbox
            .messages
            .values()
            .filter(|m| m.account == self.account && m.server.is_some() && !m.draft)
            .map(|m| (m.received, m.id))
            .collect();
        mine.sort_unstable();
        mine.into_iter().map(|(_, id)| id).collect()
    }

    /// Mail for the Inbox on the server: `n` more messages.
    fn deliver(&self, n: usize) {
        self.server.with(|d| {
            let inbox = d.mailboxes.iter().find(|m| m.role.as_deref() == Some("inbox")).map(|m| m.id.clone()).unwrap_or_default();
            let at = d.emails.len();
            for k in at..at + n {
                let raw =
                    format!("From: Zoe <zoe@example.org>\r\nTo: Ada <{ME}>\r\nSubject: Note {k}\r\nMessage-ID: <n{k}@example.org>\r\n\r\nNote.\r\n");
                let received = format!("2026-10-14T10:{:02}:00Z", k % 60);
                d.deliver(&inbox, raw.as_bytes(), &[], &received);
            }
        });
    }

    fn inbox(&self) -> String {
        self.server.with(|d| d.mailboxes.iter().find(|m| m.role.as_deref() == Some("inbox")).map(|m| m.id.clone()).unwrap_or_default())
    }

    fn fault(&self, on: &'static str, effect: Effect) {
        self.server.with(|d| d.faults.push_back(Fault::on(on, effect)));
    }
}

fn french() -> Lang {
    Lang::from_tag("fr").unwrap()
}

/// A scenario: what it did, what the window was shown, and the words in it that are a server's or the transport's
/// (what may be left in English).
struct Scenario {
    name: &'static str,
    shown: Shown,
    theirs: Vec<String>,
    mb: Mailbox,
}

impl Scenario {
    fn new(name: &'static str, rig: &Rig, shown: Shown, theirs: &[&str]) -> Scenario {
        Scenario { name, shown, theirs: theirs.iter().map(|s| (*s).to_string()).collect(), mb: rig.s.mailbox.clone() }
    }

    /// Each thing shown, read in French: (what, its reading). A line no row reads fails.
    fn readings(&self) -> Vec<(String, Reading)> {
        let mut out = Vec::new();
        let mut read = |what: &str, reading: Option<Reading>| {
            let reading = reading.unwrap_or_else(|| panic!("{}: {what:?} isn't read in French", self.name));
            for (family, words) in &reading.unread {
                assert!(
                    self.theirs.iter().any(|theirs| theirs == words),
                    "{}: {what:?} leaves {words:?} ({family:?}) in English, and the scenario didn't say it is a server's words",
                    self.name
                );
            }
            out.push((what.to_string(), reading));
        };
        for line in &self.shown.lines {
            read(line, explain(french(), Family::Line, line, &self.mb));
        }
        if !self.shown.status.is_empty() {
            read(&self.shown.status, explain_status(french(), &self.shown.status, &self.mb));
        }
        for held in &self.shown.held {
            read(held, explain(french(), Family::Banner, held, &self.mb));
        }
        out
    }

    /// The French of the problem lines that the row `english` read, outermost.
    fn french_of(&self, english: &str) -> Vec<String> {
        let lines = self.shown.lines.iter().filter_map(|line| explain(french(), Family::Line, line, &self.mb));
        lines.filter(|r| r.rows.first() == Some(&english)).map(|r| r.text).collect()
    }
}

// ---------------------------------------------------------------------------------------------
// The scenarios

/// Nothing answers: every request fails as a network fails.
fn unreachable() -> Scenario {
    let mut rig = Rig::new();
    let http = rig.through(|_, _| Err(Error::Io("connection refused".into())));
    let shown = rig.run_over(http, true);
    Scenario::new("unreachable", &rig, shown, &["connection refused"])
}

/// The session is served, and then the API can't be reached: what waits to go waits.
fn unreachable_mid_run() -> Scenario {
    let mut rig = Rig::listed();
    rig.send("bob@example.org", "Plans");
    let http = rig.through(|req, answer| if is_session(req) { answer } else { Err(Error::Io("connection reset".into())) });
    let shown = rig.run_over(http, false);
    Scenario::new("unreachable mid-run", &rig, shown, &["connection reset"])
}

/// The server is busy when the run begins.
fn busy_at_sign_in() -> Scenario {
    let mut rig = Rig::new();
    let http =
        rig.through(|req, answer| if is_session(req) { Ok(Response { status: 503, body: Vec::new(), retry_after: Some(30) }) } else { answer });
    let shown = rig.run_over(http, true);
    Scenario::new("busy at sign-in", &rig, shown, &[])
}

/// The server is busy in the middle of a run (429, with a `Retry-After`), while a message waits to go.
fn busy_mid_run() -> Scenario {
    let mut rig = Rig::listed();
    rig.send("bob@example.org", "Plans");
    rig.server.with(|d| d.refuse_next.extend([Some((429, Some(5))), Some((429, Some(5))), Some((429, Some(5))), Some((429, Some(5)))]));
    let shown = rig.run(false);
    Scenario::new("busy mid-run", &rig, shown, &[])
}

/// The server limits how fast mail goes, and then can't file mail.
fn sending_waits() -> Scenario {
    let mut rig = Rig::listed();
    rig.send("bob@example.org", "First");
    rig.fault("EmailSubmission/set", Effect::SetError("rateLimit", "Slow down."));
    let mut shown = rig.run(false);
    rig.fault("Email/import", Effect::MethodError("serverUnavailable"));
    let more = rig.run(false);
    shown.lines.extend(more.lines);
    Scenario::new("sending waits", &rig, shown, &[])
}

/// A mailbox the server fails on, then three in a row it is unavailable for.
fn broken_mailboxes() -> Scenario {
    let mut rig = Rig::listed();
    let ids: Vec<String> = rig.server.with(|d| d.mailboxes.iter().map(|m| m.id.clone()).collect());
    rig.server.with(|d| {
        d.broken_mailboxes.insert(ids[0].clone(), "serverFail");
    });
    let mut shown = rig.run(true);
    rig.server.with(|d| {
        d.broken_mailboxes = ids.iter().map(|id| (id.clone(), "serverUnavailable")).collect();
    });
    let more = rig.run(true);
    shown.lines.extend(more.lines);
    Scenario::new("broken mailboxes", &rig, shown, &[])
}

/// The list of mailboxes fails.
fn listing_fails() -> Scenario {
    let mut rig = Rig::listed();
    rig.fault("Mailbox/get", Effect::MethodError("serverFail"));
    let shown = rig.run(true);
    Scenario::new("listing fails", &rig, shown, &[])
}

/// A session document that is wrong in each way the crate checks.
fn session_documents() -> Vec<Scenario> {
    let wrong: [(&'static str, fn(&mut Value)); 7] = [
        ("an API on another server", |doc| doc["apiUrl"] = json!("https://elsewhere.example/jmap/api")),
        ("an API over plain http", |doc| doc["apiUrl"] = json!("http://jmap.standin.example/jmap/api")),
        ("no API address", |doc| _ = doc.as_object_mut().map(|o| o.remove("apiUrl"))),
        ("an API address that isn't one", |doc| doc["apiUrl"] = json!("not a url at all")),
        ("no capabilities", |doc| _ = doc.as_object_mut().map(|o| o.remove("capabilities"))),
        ("no mail", |doc| _ = doc["capabilities"].as_object_mut().map(|o| o.remove("urn:ietf:params:jmap:mail"))),
        ("no mail account", |doc| _ = doc["primaryAccounts"].as_object_mut().map(|o| o.remove("urn:ietf:params:jmap:mail"))),
    ];
    wrong
        .into_iter()
        .map(|(name, edit)| {
            let mut rig = Rig::new();
            let http = rig.through(move |req, answer| if is_session(req) { edited(answer, edit) } else { answer });
            let shown = rig.run_over(http, true);
            Scenario::new(name, &rig, shown, &[])
        })
        .collect()
}

/// Answers that are no JMAP: a page of HTML with status 200, no `methodResponses`, not an object, no answer to a call.
fn answers_that_are_not_jmap() -> Vec<Scenario> {
    let wrong: [(&'static str, fn(Result<Response>) -> Result<Response>, &'static [&'static str]); 4] = [
        ("a page of HTML", |_| answer(200, b"<html><body>Sign in</body></html>"), &["expected value at line 1 column 1"]),
        ("no methodResponses", |_| answer(200, b"{}"), &[]),
        ("not an object", |_| answer(200, b"[]"), &[]),
        ("no answer to a call", |answer| edited(answer, |doc| doc["methodResponses"] = json!([])), &[]),
    ];
    wrong
        .into_iter()
        .map(|(name, change, theirs)| {
            let mut rig = Rig::new();
            let http = rig.through(move |req, answer| if is_session(req) { answer } else { change(answer) });
            let shown = rig.run_over(http, true);
            Scenario::new(name, &rig, shown, theirs)
        })
        .collect()
}

/// A token the server no longer takes.
fn sign_in_refused() -> Scenario {
    let mut rig = Rig::new();
    rig.server.with(|d| d.token = Some("rotated".into()));
    let shown = rig.run(true);
    Scenario::new("sign-in refused", &rig, shown, &[])
}

/// What waits to go when the server can't be reached or is busy at the step that sends.
fn sending_cannot_go() -> Vec<Scenario> {
    let mut out = Vec::new();
    let mut rig = Rig::listed();
    rig.send("bob@example.org", "Plans");
    let http =
        rig.through(|req, answer| if calls(req).iter().any(|c| c == "Identity/get") { Err(Error::Io("connection reset".into())) } else { answer });
    let shown = rig.run_over(http, false);
    out.push(Scenario::new("sending unreachable", &rig, shown, &[]));
    let mut rig = Rig::listed();
    rig.send("bob@example.org", "Plans");
    rig.fault("Identity/get", Effect::RefuseWith(503));
    let shown = rig.run(false);
    out.push(Scenario::new("sending busy", &rig, shown, &[]));
    out
}

/// How a run ends: mail brought down, a run stopped, messages moved that can't be found again.
fn endings() -> Vec<Scenario> {
    let mut out = Vec::new();
    // (The status is the last piece's, which takes away here what a whole run found gone from the server.)
    let mut rig = Rig::listed();
    rig.server.with(|d| d.emails.clear());
    let shown = rig.run(true);
    out.push(Scenario::new("mail gone from the server", &rig, shown, &[]));
    let mut rig = Rig::listed();
    let shown = rig.job_over(rig.server.clone(), false, true);
    out.push(Scenario::new("stopped", &rig, shown, &[]));
    // (A message moved on a server that names a message by where it is has no id here until a window shows it again: here,
    // the oldest of the Inbox, past the window of one message a run reads.)
    for n in [1, 2] {
        let mut rig = Rig::listed();
        rig.deliver(3);
        rig.run(true);
        rig.s.prefs.sync_limit = 1;
        for id in rig.mine().into_iter().take(n) {
            if let Some(server) = rig.s.mailbox.message_mut(id).ok().and_then(|m| m.server.as_mut()) {
                server.id.clear();
            }
        }
        let shown = rig.run(false);
        out.push(Scenario::new("moved messages not found again", &rig, shown, &[]));
    }
    out
}

/// Messages the server can't read whole: one, then several; its answer leaves them out, or fails.
fn details() -> Vec<Scenario> {
    let mut out = Vec::new();
    for n in [1, 3] {
        let mut rig = Rig::new();
        rig.deliver(n - 1);
        let inbox = rig.inbox();
        rig.server.with(|d| {
            d.forgetful_headers.insert(inbox);
        });
        let shown = rig.run(true);
        out.push(Scenario::new("messages left out", &rig, shown, &[]));
    }
    out
}

/// Changes the server doesn't make: it fails on the call, says it can't now, or doesn't answer them.
fn changes() -> Vec<Scenario> {
    let mut out = Vec::new();
    for n in [1, 2] {
        let mut rig = Rig::listed();
        rig.deliver(1);
        rig.run(true);
        rig.mark_read(n);
        rig.fault("Email/set", Effect::MethodError("serverFail"));
        let shown = rig.run(false);
        out.push(Scenario::new("changes the call failed for", &rig, shown, &[]));
    }
    let mut rig = Rig::listed();
    rig.mark_read(1);
    let ids: Vec<String> = rig.server.with(|d| d.emails.iter().map(|e| e.id.clone()).collect());
    rig.server.with(|d| d.refused_sets = ids.iter().map(|id| (id.clone(), "rateLimit")).collect());
    let shown = rig.run(false);
    out.push(Scenario::new("changes the server can't make now", &rig, shown, &[]));
    let mut rig = Rig::listed();
    rig.mark_read(1);
    let http = rig.through(|req, answer| {
        if calls(req).iter().any(|c| c == "Email/set") {
            edited(answer, |doc| {
                for answer in doc["methodResponses"].as_array_mut().into_iter().flatten().filter(|a| a[0] == "Email/set") {
                    if let Some(result) = answer[1].as_object_mut() {
                        result.remove("updated");
                        result.remove("notUpdated");
                    }
                }
            })
        } else {
            answer
        }
    });
    let shown = rig.run_over(http, false);
    out.push(Scenario::new("changes not answered", &rig, shown, &[]));
    let mut rig = Rig::listed();
    rig.mark_read(1);
    rig.fault("Email/set", Effect::MethodError("requestTooLarge"));
    let shown = rig.run(false);
    out.push(Scenario::new("changes in too large a call", &rig, shown, &[]));
    out
}

/// Answers that are too large, a request over the server's limit, a request the server rejects.
fn limits() -> Vec<Scenario> {
    let wrong: [(&'static str, fn(&Request, Result<Response>) -> Result<Response>); 4] = [
        ("an answer too large", |req, _| answer(200, &vec![b' '; req.max.saturating_add(1)])),
        ("a request over the limit", |_, _| {
            answer(400, br#"{"type": "urn:ietf:params:jmap:error:limit", "limit": "maxSizeRequest", "status": 400}"#)
        }),
        ("a request over a limit it doesn't name", |_, _| answer(400, br#"{"type": "urn:ietf:params:jmap:error:limit", "status": 400}"#)),
        ("a request the server rejects", |_, _| {
            answer(400, br#"{"type": "urn:ietf:params:jmap:error:notRequest", "detail": "That is no request.", "status": 400}"#)
        }),
    ];
    wrong
        .into_iter()
        .map(|(name, change)| {
            let mut rig = Rig::listed();
            let http = rig.through(move |req, answer| if is_session(req) { answer } else { change(req, answer) });
            let shown = rig.run_over(http, false);
            Scenario::new(name, &rig, shown, &[])
        })
        .collect()
}

/// Answers that lack what was asked: no list of mailboxes, none in it, no list of ids, no list of emails.
fn answers_that_lack() -> Vec<Scenario> {
    let wrong: [(&'static str, &'static str, fn(&mut Value)); 4] = [
        ("no list of mailboxes", "Mailbox/get", |result| _ = result.as_object_mut().map(|o| o.remove("list"))),
        ("no mailbox in the list", "Mailbox/get", |result| result["list"] = json!([])),
        ("no list of ids", "Email/query", |result| _ = result.as_object_mut().map(|o| o.remove("ids"))),
        ("no list of emails", "Email/get", |result| _ = result.as_object_mut().map(|o| o.remove("list"))),
    ];
    wrong
        .into_iter()
        .map(|(name, method, edit)| {
            let mut rig = Rig::new();
            let http = rig.through(move |req, answer| {
                if !calls(req).iter().any(|c| c == method) {
                    return answer;
                }
                edited(answer, |doc| {
                    for answer in doc["methodResponses"].as_array_mut().into_iter().flatten().filter(|a| a[0] == method) {
                        edit(&mut answer[1]);
                    }
                })
            });
            let shown = rig.run_over(http, true);
            Scenario::new(name, &rig, shown, &[])
        })
        .collect()
}

/// More mailboxes than a run reads: one more, then two.
fn too_many_mailboxes() -> Vec<Scenario> {
    [1, 2]
        .into_iter()
        .map(|more| {
            let mut rig = Rig::new();
            rig.server.with(|d| {
                d.max_get = 2_000;
                let have = d.mailboxes.len();
                for n in have..1_000 + more {
                    d.mailbox(&format!("Folder {n}"), None, None);
                }
            });
            let shown = rig.run(true);
            Scenario::new("too many mailboxes", &rig, shown, &[])
        })
        .collect()
}

/// Every scenario.
fn scenarios() -> Vec<Scenario> {
    let mut all = vec![
        unreachable(),
        unreachable_mid_run(),
        busy_at_sign_in(),
        busy_mid_run(),
        sending_waits(),
        broken_mailboxes(),
        listing_fails(),
        sign_in_refused(),
    ];
    for more in [
        session_documents(),
        answers_that_are_not_jmap(),
        sending_cannot_go(),
        endings(),
        details(),
        changes(),
        limits(),
        answers_that_lack(),
        too_many_mailboxes(),
    ] {
        all.extend(more);
    }
    all
}

/// Rows no scenario here makes, and why.
const NOT_PROVOKED: &[(&str, &str)] = &[
    ("{browser} (the server may not let this page reach it)", "made by the browser's `fetch` (wasm), which no native test runs: the census holds it"),
    ("the server took too long to answer", "as above"),
    ("the session address isn't https", "defensive: the engine refuses such an address before a run (`jmap_mail_request`)"),
    ("the session address is too long", "defensive: the model caps the address first"),
    ("a JMAP address is too long", "defensive: as above"),
    ("not a web address", "defensive: the engine checks the session's address before a run, and an `apiUrl` that is none is one that can't be used"),
];

// ---------------------------------------------------------------------------------------------
// The tests

#[test]
fn what_a_failed_run_shows_reads_in_french() {
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    for scenario in scenarios() {
        assert!(!scenario.readings().is_empty(), "{}: nothing was said: {:?}", scenario.name, scenario.shown);
        for (_, reading) in scenario.readings() {
            seen.extend(reading.rows);
        }
    }
    let excused: BTreeSet<&str> = NOT_PROVOKED.iter().map(|(row, _)| *row).collect();
    let missed: Vec<&str> = rows().iter().map(|row| row.english).filter(|english| !seen.contains(english) && !excused.contains(english)).collect();
    assert!(missed.is_empty(), "rows no scenario made (a row nobody makes is a row nobody checked): {missed:#?}");
    for row in &excused {
        assert!(!seen.contains(row), "{row:?} is made after all: take it off NOT_PROVOKED");
        assert!(rows().iter().any(|r| r.english == *row), "{row:?} is no row any more");
    }
}

#[test]
fn a_run_that_cannot_reach_the_server_says_so_in_french() {
    let scenario = unreachable();
    assert_eq!(scenario.french_of("Couldn't reach the server: {why}"), ["Impossible de joindre le serveur : réseau : connection refused"]);
    let scenario = busy_at_sign_in();
    assert_eq!(scenario.french_of("Couldn't reach the server: {why}"), ["Impossible de joindre le serveur : le serveur est occupé (HTTP 503)"]);
}

#[test]
fn what_waits_to_go_says_why_in_french() {
    let scenario = sending_waits();
    let french: Vec<String> = scenario.readings().into_iter().map(|(_, r)| r.text).collect();
    assert!(french.iter().any(|l| l.starts_with("Le serveur limite le nombre de messages")), "{french:#?}");
    assert!(french.iter().any(|l| l.starts_with("Le serveur ne peut pas classer le courrier pour l’instant")), "{french:#?}");
}

/// Prints what each scenario showed, in English and in French. Run it on its own, with the output shown:
/// `cargo test -p mailcraft-web print_the_scenarios -- --ignored --nocapture`.
#[test]
#[ignore = "prints the scenarios for a reader"]
fn print_the_scenarios() {
    for scenario in scenarios() {
        println!("== {}", scenario.name);
        for (english, reading) in scenario.readings() {
            println!("  EN {english}\n  FR {}", reading.text);
        }
    }
}
