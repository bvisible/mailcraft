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
use mailcraft_model::{FolderKind, Id, Mailbox};
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
        // (The browser build runs a JMAP account's Send/Receive.)
        s.jmap_runs = true;
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

    /// [`Rig::run`], with `meanwhile` done to the session after the run asked for its work and before its pieces are
    /// taken in: what the owner does while a run is under way.
    fn run_meanwhile(&mut self, full: bool, meanwhile: impl FnOnce(&mut Session)) -> Shown {
        self.job(self.server.clone(), full, false, meanwhile)
    }

    /// [`Rig::run_over`], stopped by the owner before it began when `stopped`.
    fn job_over(&mut self, http: impl Http + Clone, full: bool, stopped: bool) -> Shown {
        self.job(http, full, stopped, |_| {})
    }

    fn job(&mut self, http: impl Http + Clone, full: bool, stopped: bool, meanwhile: impl FnOnce(&mut Session)) -> Shown {
        let request = jmap_mail_request(&self.s, self.account, full, &[], false).unwrap();
        let job = JmapJob { request, secret: Some(TOKEN.into()), cancel: Arc::new(AtomicBool::new(stopped)) };
        let pieces = RefCell::new(Vec::new());
        block_on(run(http, &job, None::<&NoCredentials>, |_| std::future::ready(()), &mut |piece| pieces.borrow_mut().push(piece)));
        meanwhile(&mut self.s);
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

    /// Write a draft from the account and save it: it waits to go up.
    fn save_draft(&mut self, subject: &str) {
        self.s.run("mail.new", &json!({"to": "bob@example.org", "subject": subject, "body": "A draft.", "account": self.account})).unwrap();
        self.s.run("compose.saveDraft", &json!({})).unwrap();
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
    Lang::French
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

/// A message sent to `to` (a list of addresses, as the To field takes it), with the server answering its submission with
/// `effect`, when there is one: what the window was shown.
fn sent_with(rig: &mut Rig, to: &str, effect: Option<(&'static str, Effect)>) -> Shown {
    rig.send(to, "Plans");
    if let Some((on, effect)) = effect {
        rig.fault(on, effect);
    }
    rig.run(false)
}

/// A server's refusal of a message, in every type the crate has words for and in types it hasn't: of its submission,
/// or of its filing in Sent before that (a type the crate doesn't know is a refusal there, and a doubt at the
/// submission, which may have sent it).
fn refusals() -> Vec<Scenario> {
    let kinds: [(&'static str, &'static str, &'static str, &'static [&'static str]); 18] = [
        ("EmailSubmission/set", "forbiddenFrom", "Not as that.", &[]),
        ("EmailSubmission/set", "forbiddenToSend", "Not now.", &[]),
        ("EmailSubmission/set", "noRecipients", "Nobody.", &[]),
        ("EmailSubmission/set", "invalidRecipients", "nobody here", &[]),
        ("EmailSubmission/set", "tooManyRecipients", "Too many.", &[]),
        ("EmailSubmission/set", "tooLarge", "Too big.", &[]),
        ("EmailSubmission/set", "overQuota", "Full.", &[]),
        ("EmailSubmission/set", "invalidEmail", "Bad.", &[]),
        ("EmailSubmission/set", "accountReadOnly", "Read-only.", &["Read-only"]),
        ("EmailSubmission/set", "somethingNew", "It said so.", &["It said so"]),
        ("EmailSubmission/set", "somethingNew", "", &[]),
        ("EmailSubmission/set", "", "It said so.", &["It said so"]),
        ("EmailSubmission/set", "", "", &[]),
        ("Email/import", "blobNotFound", "Lost it.", &[]),
        ("Email/import", "somethingNew", "It said so.", &["It said so"]),
        ("Email/import", "somethingNew", "", &[]),
        ("Email/import", "", "It said so.", &["It said so"]),
        ("Email/import", "", "", &[]),
    ];
    let mut out: Vec<Scenario> = kinds
        .into_iter()
        .map(|(on, kind, description, theirs)| {
            let mut rig = Rig::listed();
            let shown = sent_with(&mut rig, "bob@example.org", Some((on, Effect::SetError(kind, description))));
            Scenario::new("a refusal", &rig, shown, theirs)
        })
        .collect();
    // A call refused as a whole has no message to name an address of.
    let mut rig = Rig::listed();
    let shown = sent_with(&mut rig, "bob@example.org", Some(("Email/import", Effect::MethodError("forbiddenFrom"))));
    out.push(Scenario::new("a refusal of the whole call", &rig, shown, &[]));
    out
}

/// What the server and the message itself make a message held for.
fn held_messages() -> Vec<Scenario> {
    let mut out = Vec::new();
    // Recipients the server won't take: it names one of the message's.
    let mut rig = Rig::listed();
    let shown = sent_with(&mut rig, "bob@example.org", Some(("EmailSubmission/set", Effect::SetError("invalidRecipients", "bob@example.org"))));
    out.push(Scenario::new("recipients the server won't take", &rig, shown, &[]));
    // More than it allows.
    let mut rig = Rig::listed();
    let many: Vec<String> = (0..120).map(|n| format!("p{n}@example.org")).collect();
    let shown = sent_with(&mut rig, &many.join(", "), None);
    out.push(Scenario::new("more recipients than the server allows", &rig, shown, &[]));
    // An address mail can't be sent to, and too many recipients for the program.
    let mut rig = Rig::listed();
    let shown = sent_with(&mut rig, "zoé@exämple.org", None);
    out.push(Scenario::new("an address mail can't go to", &rig, shown, &[]));
    let mut rig = Rig::listed();
    let crowd: Vec<String> = (0..1_001).map(|n| format!("p{n}@example.org")).collect();
    let shown = sent_with(&mut rig, &crowd.join(", "), None);
    out.push(Scenario::new("too many recipients", &rig, shown, &[]));
    // A server that doesn't send, has no Sent, no identity, takes small messages only.
    let mut rig = Rig::listed();
    rig.server.with(|d| d.submission = false);
    let shown = sent_with(&mut rig, "bob@example.org", None);
    out.push(Scenario::new("a server that doesn't send", &rig, shown, &[]));
    let mut rig = Rig::listed();
    rig.server.with(|d| d.identities.clear());
    let shown = sent_with(&mut rig, "bob@example.org", None);
    out.push(Scenario::new("no identity", &rig, shown, &[]));
    let mut rig = Rig::listed();
    rig.server.with(|d| d.max_upload = 100);
    let shown = sent_with(&mut rig, "bob@example.org", None);
    out.push(Scenario::new("a small upload", &rig, shown, &[]));
    // A gateway's refusal, an answer that is no JMAP, an upload that is too large.
    for (name, on, effect) in [
        ("a refused upload", "upload", Effect::Status(400)),
        ("a filing that is no JMAP", "Email/import", Effect::Garbage),
        ("an upload too large", "upload", Effect::Status(413)),
    ] {
        let mut rig = Rig::listed();
        let shown = sent_with(&mut rig, "bob@example.org", Some((on, effect)));
        out.push(Scenario::new(name, &rig, shown, &[]));
    }
    out
}

/// Messages that may have gone: the server sends them and then says something went wrong, or its answer is lost, or a
/// gateway fails after the submission.
fn doubts() -> Vec<Scenario> {
    let mut out = Vec::new();
    for (kind, description, theirs) in [
        ("serverFail", "It went, then failed.", &["It went, then failed"][..]),
        ("serverFail", "", &[][..]),
        ("somethingNew", "", &[][..]),
        ("somethingNew", "Odd.", &["Odd"][..]),
        ("", "", &[][..]),
        ("", "Odd.", &["Odd"][..]),
    ] {
        let mut rig = Rig::listed();
        let shown = sent_with(&mut rig, "bob@example.org", Some(("EmailSubmission/set", Effect::SentThenSetError(kind, description))));
        out.push(Scenario::new("sent, then a failure", &rig, shown, theirs));
    }
    for (name, effect) in [("an answer lost", Effect::LoseAnswer), ("a gateway that fails after", Effect::Status(502))] {
        let mut rig = Rig::listed();
        let shown = sent_with(&mut rig, "bob@example.org", Some(("EmailSubmission/set", effect)));
        out.push(Scenario::new(name, &rig, shown, &[]));
    }
    // An answer that doesn't say.
    let mut rig = Rig::listed();
    rig.send("bob@example.org", "Plans");
    let http = rig.through(|req, answer| {
        if calls(req).iter().any(|c| c == "EmailSubmission/set") {
            edited(answer, |doc| {
                if let Some(answers) = doc["methodResponses"].as_array_mut() {
                    for answer in answers.iter_mut().filter(|a| a[0] == "EmailSubmission/set") {
                        answer[1] = json!({"accountId": "A1"});
                    }
                }
            })
        } else {
            answer
        }
    });
    let shown = rig.run_over(http, false);
    out.push(Scenario::new("an answer that doesn't say", &rig, shown, &[]));
    out
}

/// A message that went (or may have) after the owner took it out of the Outbox while the run was under way: moved to
/// Drafts, or deleted for good.
fn taken_out_meanwhile() -> Vec<Scenario> {
    let mut out = Vec::new();
    for (lost, deleted) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut rig = Rig::listed();
        rig.send("bob@example.org", "Plans");
        if lost {
            rig.fault("EmailSubmission/set", Effect::LoseAnswer);
        }
        let shown = rig.run_meanwhile(false, |s| {
            let account = s.mailbox.accounts.iter().find(|a| a.email == ME).map(|a| a.id).unwrap_or_default();
            let outbox = s.mailbox.folder_of_kind(account, FolderKind::Outbox);
            let ids: Vec<Id> = s.mailbox.messages.values().filter(|m| Some(m.folder) == outbox).map(|m| m.id).collect();
            for id in ids {
                if deleted {
                    let _ = s.mailbox.forget_message(id);
                } else if let Some(drafts) = s.mailbox.folder_of_kind(account, FolderKind::Drafts)
                    && let Ok(m) = s.mailbox.message_mut(id)
                {
                    m.folder = drafts;
                }
            }
        });
        out.push(Scenario::new("taken out of the Outbox meanwhile", &rig, shown, &[]));
    }
    out
}

/// Drafts the server won't file: no Drafts, too large for it, a refusal, one of a kind of its own.
fn drafts_refused() -> Vec<Scenario> {
    let mut out = Vec::new();
    let mut rig = Rig::listed();
    rig.save_draft("Notes");
    // (Its Drafts is a mailbox like any other now: the draft is still one here.)
    rig.server.with(|d| d.mailboxes.iter_mut().filter(|m| m.role.as_deref() == Some("drafts")).for_each(|m| m.role = None));
    let shown = rig.run(true);
    out.push(Scenario::new("no Drafts on the server", &rig, shown, &[]));
    let mut rig = Rig::listed();
    rig.server.with(|d| d.max_upload = 100);
    rig.save_draft("Notes");
    let shown = rig.run(false);
    out.push(Scenario::new("a draft too large", &rig, shown, &[]));
    for (name, on, effect, theirs) in [
        ("a draft the server forbids", "Email/import", Effect::SetError("forbidden", "No."), &[][..]),
        ("a draft over quota", "Email/import", Effect::SetError("overQuota", "Full."), &[][..]),
        ("a draft of a kind of its own", "Email/import", Effect::SetError("weird", "It said so."), &["It said so"][..]),
        ("a draft upload too large", "upload", Effect::Status(413), &[][..]),
        ("a draft upload refused", "upload", Effect::Status(400), &[][..]),
        ("a draft filing that is no JMAP", "Email/import", Effect::Garbage, &[][..]),
    ] {
        let mut rig = Rig::listed();
        rig.save_draft("Notes");
        rig.fault(on, effect);
        let shown = rig.run(false);
        out.push(Scenario::new(name, &rig, shown, theirs));
    }
    out
}

/// A server with no Sent to keep a copy in.
fn no_sent() -> Scenario {
    let mut rig = Rig::listed();
    rig.server.with(|d| d.mailboxes.retain(|m| m.role.as_deref() != Some("sent")));
    rig.run(true);
    let shown = sent_with(&mut rig, "bob@example.org", None);
    Scenario::new("no Sent on the server", &rig, shown, &[])
}

/// A change made here to `n` messages that the server refuses with a SetError of type `kind`: each kind of change.
fn refused_change(change: &'static str, n: usize, kind: &'static str) -> Scenario {
    let mut rig = Rig::listed();
    rig.deliver(2);
    rig.run(true);
    let ids: Vec<Id> = rig.mine().into_iter().take(n).collect();
    if change == "unread" {
        rig.s.run("mail.markRead", &json!({"ids": ids})).unwrap();
        rig.run(false);
    }
    let trash = rig.s.mailbox.folder_of_kind(rig.account, FolderKind::Deleted).unwrap_or_default();
    match change {
        "read" => {
            rig.s.run("mail.markRead", &json!({"ids": ids})).unwrap();
        }
        "unread" => {
            rig.s.run("mail.markUnread", &json!({"ids": ids})).unwrap();
        }
        "flag" => {
            rig.s.run("mail.flag", &json!({"ids": ids})).unwrap();
        }
        // (As a reply that went marks what it answers.)
        "replied" => {
            for id in &ids {
                if let Ok(m) = rig.s.mailbox.message_mut(*id) {
                    m.answered = true;
                }
            }
        }
        "several" => {
            rig.s.run("mail.markRead", &json!({"ids": ids})).unwrap();
            rig.s.run("mail.flag", &json!({"ids": ids})).unwrap();
        }
        "move" | "move away" => {
            rig.s.run("mail.move", &json!({"folder": trash, "ids": ids})).unwrap();
        }
        _ => {
            rig.s.run("mail.deletePermanently", &json!({"ids": ids, "confirmed": true})).unwrap();
        }
    }
    let on_server: Vec<String> =
        rig.s.mailbox.messages.values().filter(|m| ids.contains(&m.id)).filter_map(|m| m.server.as_ref().map(|s| s.id.clone())).collect();
    let on_server = if on_server.is_empty() { rig.server.with(|d| d.emails.iter().map(|e| e.id.clone()).collect()) } else { on_server };
    rig.server.with(|d| d.refused_sets = on_server.iter().map(|id| (id.clone(), kind)).collect());
    let shown = if change == "move away" {
        // (The folder the messages were moved to went meanwhile: the line names none.)
        rig.run_meanwhile(false, move |s| s.mailbox.folders.retain(|f| f.id != trash))
    } else {
        rig.run(false)
    };
    Scenario::new("a refused change", &rig, shown, &[])
}

/// Every kind of change, for one message and for two, for each kind of refusal.
fn refused_changes() -> Vec<Scenario> {
    let mut out = Vec::new();
    for change in ["read", "unread", "flag", "replied", "several", "move", "move away", "delete"] {
        for n in [1, 2] {
            for kind in ["notFound", "forbidden", "overQuota", "invalidPatch"] {
                out.push(refused_change(change, n, kind));
            }
        }
    }
    // A message with no subject, deleted for good.
    let mut rig = Rig::listed();
    rig.server.with(|d| {
        let inbox = d.mailboxes.iter().find(|m| m.role.as_deref() == Some("inbox")).map(|m| m.id.clone()).unwrap_or_default();
        d.deliver(&inbox, b"From: Zoe <zoe@example.org>\r\nMessage-ID: <nosubject@example.org>\r\n\r\nHi.\r\n", &[], "2026-10-14T11:00:00Z");
    });
    rig.run(true);
    let id = rig.s.mailbox.messages.values().find(|m| m.account == rig.account && m.subject.is_empty()).map(|m| m.id).unwrap_or_default();
    let server_id = rig.s.mailbox.message(id).and_then(|m| m.server.as_ref().map(|s| s.id.clone())).unwrap_or_default();
    rig.s.run("mail.deletePermanently", &json!({"ids": [id], "confirmed": true})).unwrap();
    rig.server.with(|d| d.refused_sets = [(server_id, "invalidPatch")].into_iter().collect());
    let shown = rig.run(false);
    out.push(Scenario::new("a message with no subject deleted for good", &rig, shown, &[]));
    out
}

/// The engine's refusals: a piece of a run for another account, the address of a JMAP server wrong in each way the
/// model checks, a move or a copy between a JMAP account and another place. (A JMAP account's folders are made, renamed,
/// moved and deleted as any: their changes go up, and nothing refuses them.)
fn refusals_of_the_engine() -> Vec<Scenario> {
    let mut out = Vec::new();
    let refused = |rig: &Rig, name: &'static str, said: std::result::Result<Value, mailcraft_engine::CmdError>| {
        let shown = Shown { lines: said.err().map(|e| e.to_string()).into_iter().collect(), ..Default::default() };
        Scenario::new(name, rig, shown, &[])
    };
    // (The mailbox was replaced while the run was under way: its piece names another account's address.)
    let mut rig = Rig::listed();
    let piece = mailcraft_model::sync::JmapMailResult { account: rig.account, email: "eve@example.org".into(), ..Default::default() };
    let said = apply_jmap_mail(&mut rig.s, piece);
    out.push(refused(&rig, "a piece for another account", said));
    let long = format!("https://jmap.example.org/{}", "a".repeat(2_048));
    for address in [
        "",
        long.as_str(),
        "https://jmap.example.org/a b",
        "https://jmap.example.org\\x",
        "ftp://jmap.example.org",
        "https://",
        "https://ada:pw@jmap.example.org",
        "https://jmap.example.org:99999",
        "http://jmap.example.org",
        "https://jmap_example!.org",
    ] {
        let mut rig = Rig::new();
        let account = rig.account;
        if let Some(server) = rig.s.mailbox.accounts.iter_mut().find(|a| a.id == account).and_then(|a| a.jmap.as_mut()) {
            server.session = address.to_string();
        }
        let said = jmap_mail_request(&rig.s, account, true, &[], false).map(|_| Value::Null);
        out.push(refused(&rig, "a JMAP server's address", said));
    }
    // Between a JMAP account and a folder on this computer, and a copy.
    let mut rig = Rig::listed();
    // (An account kept on this computer.)
    let mine = rig.s.mailbox.add_account(mailcraft_model::Account { name: "Mine".into(), email: "mine@example.org".into(), ..Default::default() });
    let local = rig.s.mailbox.folder_of_kind(mine, FolderKind::Inbox);
    let trash = rig.s.mailbox.folder_of_kind(rig.account, FolderKind::Deleted);
    let ids = rig.mine();
    let said = rig.s.run("mail.move", &json!({"folder": local, "ids": ids}));
    out.push(refused(&rig, "a move to this computer", said));
    let said = rig.s.run("mail.copy", &json!({"folder": trash, "ids": ids}));
    out.push(refused(&rig, "a copy", said));
    out
}

/// What Undo can't take back of a JMAP account: a message the server has, deleted for good (one, then two), and a
/// message sent.
fn undone() -> Vec<Scenario> {
    let mut out = Vec::new();
    for n in [1, 2] {
        let mut rig = Rig::listed();
        rig.deliver(1);
        rig.run(true);
        let ids: Vec<Id> = rig.mine().into_iter().take(n).collect();
        rig.s.run("mail.deletePermanently", &json!({"ids": ids})).unwrap();
        rig.s.run("edit.undo", &json!({})).unwrap();
        let shown = Shown { status: rig.s.sync_status.clone(), ..Default::default() };
        out.push(Scenario::new("a delete for good undone", &rig, shown, &[]));
    }
    let mut rig = Rig::listed();
    rig.send("bob@example.org", "Plans");
    rig.s.run("edit.undo", &json!({})).unwrap();
    let shown = Shown { status: rig.s.sync_status.clone(), ..Default::default() };
    out.push(Scenario::new("a send undone", &rig, shown, &[]));
    out
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
        refusals(),
        held_messages(),
        doubts(),
        taken_out_meanwhile(),
        drafts_refused(),
        vec![no_sent()],
        refused_changes(),
        refusals_of_the_engine(),
        undone(),
    ] {
        all.extend(more);
    }
    all
}

/// Rows no scenario here makes, and why.
const NOT_PROVOKED: &[(&str, &str)] = &[
    ("timed out", "made by the browser's `fetch` (wasm), which no native test runs: the census holds it"),
    ("the request failed", "as above"),
    ("the response body is over {max} bytes", "as above"),
    ("no window to fetch from", "as above"),
    ("the response body cannot be read", "as above"),
    ("the response body is not bytes", "as above"),
    ("the browser refused a request header", "as above"),
    ("the browser gave no response", "as above"),
    ("the browser could not build the request", "as above"),
    ("the browser cannot limit the request's time", "as above"),
    ("the session address isn't https", "defensive: the engine refuses such an address before a run (`jmap_mail_request`)"),
    ("the session address is too long", "defensive: the model caps the address first"),
    ("a JMAP address is too long", "defensive: as above"),
    (
        "the address does not start with http:// or https://",
        "defensive: the engine checks the session's address before a run, and an `apiUrl` that is none is one that can't be used",
    ),
    ("the address cannot be split", "defensive: as above"),
    ("the address holds user information", "defensive: as above"),
    ("the link leads to a scheme other than http or https", "defensive: as above"),
    ("the address is too long", "defensive: as above"),
    ("the address holds a space or a control character", "defensive: as above"),
    ("the address holds a backslash", "defensive: as above"),
    ("the IPv6 address is not closed", "defensive: as above"),
    ("unexpected characters after the host", "defensive: as above"),
    ("the address has no host", "defensive: as above"),
    ("the host is too long", "defensive: as above"),
    ("the IPv6 address holds characters it cannot", "defensive: as above"),
    ("the IPv6 address is not valid", "defensive: as above"),
    ("the host holds characters a host cannot", "defensive: as above"),
    ("the IPv4 address is not four decimal numbers", "defensive: as above"),
    ("the port is not valid", "defensive: as above"),
    ("the message has no sender", "defensive: a message written here is from its account"),
    ("\"{address}\" isn't an address mail can be sent from", "defensive: an account's address is checked when the account is added"),
    ("the server wouldn't take it", "defensive: the engine's word for a host that gives no reason, and the crate always gives one"),
    (
        "Sending {n} messages can't be undone.",
        "a step of the window sends one message: an undo never meets several (the engine has the words all the same)",
    ),
    ("no account with id {id}", "defensive: the window asks for a run, or takes one in, for an account it has"),
    ("{account} isn't kept in step with a JMAP server", "defensive: as above, an account of that kind"),
    ("{account} has no JMAP server", "defensive: as above, an account with a server"),
    ("no message with id {id} in {account}", "defensive: a download of a message the window has"),
    (
        "these messages were for another account, not for {account}",
        "defensive: a download for an account replaced meanwhile, which the window ends first",
    ),
    (
        "The server's mailbox is full: {name} waits here and goes as soon as there is room.",
        "a contacts job's, not a mail run's: the contacts flow over the same in-process server makes it and reads it in French \
         (`jmap_pim_flow_tests`, a new contact the server has no room for)",
    ),
    (
        "The server's mailbox is full: the change to {name} waits here and goes as soon as there is room.",
        "a contacts job's, as above: the engine's tests make it, and run_words' read it in French",
    ),
    ("The server's mailbox is full: deleting {name} waits here and goes as soon as there is room.", "as above"),
    // A calendar's lines, not a Send/Receive's: the calendar's flows make them and read them in French.
    ("This calendar's account can't make online meetings.", "a calendar's: `jmap_pim_flow_tests` makes it and reads it in French"),
    ("The account's server has made this an online meeting, and it stays one: that can't be taken back from here.", "a calendar's: as above"),
    (
        "{name} couldn't be made an online meeting: the account's server wouldn't have it ({why}). The meeting itself is saved, without a link to join.",
        "a calendar's: as above",
    ),
    (
        "{name} couldn't be made an online meeting: the account's server wouldn't have it. The meeting itself is saved, without a link to join.",
        "a calendar's: a new event sent again after its answer was lost, which the server had made without its room, is answered \
         with no room and no reason (`jmap_pim_flow_tests` makes it and reads it in French)",
    ),
    (
        "{name}: an online meeting is the whole series', so it was not made for that day alone. Turn it on for the whole series.",
        "a calendar's: as above",
    ),
    (
        "{name} couldn't be made an online meeting: its account's server no longer makes them. The meeting itself is saved, without a link to join.",
        "a calendar's: as above",
    ),
    (
        "{name} couldn't be made an online meeting: the account's server can't make more meeting rooms just now. The meeting itself is saved, without a link to join; turn it on again later.",
        "a calendar's: as above",
    ),
    (
        "{name} couldn't be made an online meeting: the account's server doesn't let you make meeting rooms. The meeting itself is saved, without a link to join.",
        "a calendar's: as above",
    ),
    // The notes' (`cmd/notes/words.rs`): a command's refusal or a notes job's line, not a mail run's.
    (
        "Notes aren't kept here: this page's server offers none.",
        "a notes command's refusal: the engine's tests make them, run_words' read them in French",
    ),
    ("Your rights do not let you change this note", "as above"),
    ("Only its owner deletes a note: this one was shared with you", "as above"),
    ("This note's text is changed where it was written: open it on the web", "as above"),
    ("A note holds {max} characters at most", "as above"),
    ("A note's title holds {max} characters at most", "as above"),
    ("This note has no web address", "as above"),
    ("Too many note changes are waiting to be sent. Let them get through first.", "as above"),
    ("{title} was changed elsewhere meanwhile: your version is kept as a note of its own, {copy}.", "a notes job's line, as above"),
    ("{title} was deleted elsewhere meanwhile: your version is kept as a note of its own.", "as above"),
    ("{title} is no longer shared with you: your version is kept as a note of its own.", "as above"),
    ("The server wouldn't take the change to {title}: {said}. Your version is kept as a note of its own, {copy}.", "as above"),
    ("{title} can no longer be changed here ({why}): your version is kept as a note of its own, {copy}.", "the Notes view's, as above"),
    ("{title} couldn't be saved on the server ({said}), and there is no room for a note of its own: your version stays here, unsent.", "as above"),
    ("{title} couldn't be saved on the server: {said}", "as above"),
    ("The change to {title} was put back as the server has it: {said}", "as above"),
    ("The server wouldn't delete {title}, which is back in the notes: {said}", "as above"),
    ("{title} was already deleted on the server when its deletion was undone here: it is made there anew.", "as above"),
    ("The server no longer keeps notes here: the notes that waited to go are kept on this computer.", "as above"),
    ("The server no longer keeps notes here: the notes that waited to go stay on this page, unsent, until it is left.", "as above"),
    ("The undo couldn't keep a note in step with its server: too many note changes are waiting to be sent.", "an undo's line, as above"),
    ("Couldn't send the note changes: {why}", "a notes job's trouble, as above"),
    ("Couldn't read the notes: {why}", "as above"),
    ("1 note couldn't be read: {why}", "as above"),
    ("{n} notes couldn't be read: {why}", "as above"),
    ("the notes hold more text than one reading takes, and these are not shown", "as above"),
];

/// The files whose rows are a JMAP account's calendar and address book job's, or the engine's about their items, not a
/// mail run's: the calendar and contact flows over the same in-process server make lines of them and read them in French
/// (`jmap_pim_flow_tests`), and run_words' own tests read every row and hold it to its file.
const PIM: &[&str] = &[
    "crates/jmap-fork/src/pim.rs",
    "crates/jmap-fork/src/pim/calendars.rs",
    "crates/jmap-fork/src/pim/contacts.rs",
    "crates/jmap-fork/src/pim/push.rs",
    "crates/jmap-fork/src/pim/contacts/push.rs",
    "crates/jmap-fork/src/pim/write.rs",
    "crates/engine/src/cmd/remote.rs",
    "crates/engine/src/cmd/remote_contacts.rs",
    "crates/engine/src/cmd/remote_contacts/written.rs",
    "crates/engine/src/cmd/calendar.rs",
    "crates/engine/src/cmd/kept_here.rs",
    "crates/engine/src/lib.rs",
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
    let missed: Vec<&str> = rows()
        .iter()
        .filter(|row| !PIM.contains(&row.at))
        .map(|row| row.english)
        .filter(|english| !seen.contains(english) && !excused.contains(english))
        .collect();
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

#[test]
fn in_the_pseudo_language_nothing_of_ours_is_left_in_english() {
    // (The pseudo-language changes every letter of our words, and none of a server's: a line read in it is never the
    // line as it came, and holds what the scenario said is a server's, as it came.)
    for scenario in scenarios() {
        let shown = scenario.shown.lines.iter().chain(&scenario.shown.held).chain(std::iter::once(&scenario.shown.status));
        for english in shown.filter(|text| !text.is_empty()) {
            let family = if scenario.shown.held.contains(english) { Family::Banner } else { Family::Line };
            let reading = if english == &scenario.shown.status {
                explain_status(Lang::Pseudo, english, &scenario.mb)
            } else {
                explain(Lang::Pseudo, family, english, &scenario.mb)
            };
            let reading = reading.unwrap_or_else(|| panic!("{}: {english:?} isn't read", scenario.name));
            assert_ne!(&reading.text, english, "{}", scenario.name);
            for theirs in &scenario.theirs {
                assert_eq!(reading.text.matches(theirs.as_str()).count(), english.matches(theirs.as_str()).count(), "{}: {english:?}", scenario.name);
            }
        }
    }
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

#[test]
fn a_message_held_says_why_in_french_and_the_mailbox_keeps_the_english() {
    let mut rig = Rig::listed();
    let shown = sent_with(&mut rig, "bob@example.org", Some(("EmailSubmission/set", Effect::SetError("forbiddenToSend", "Not now."))));
    assert_eq!(shown.held, ["It wasn't sent: the server doesn't let this account send mail just now. Change it, or send it again."]);
    let scenario = Scenario::new("held", &rig, shown, &[]);
    let french: Vec<String> = scenario.readings().into_iter().map(|(_, reading)| reading.text).collect();
    assert!(
        french.contains(
            &"Il n’a pas été envoyé : le serveur ne permet pas à ce compte d’envoyer du courrier pour l’instant. Modifiez-le, ou renvoyez-le."
                .to_string()
        ),
        "{french:#?}"
    );
    // (The frame of a message held in the Outbox is a sentence upstream's catalogue has too: its French is upstream's.)
    let frame = mailcraft_ui_egui::i18n::Lang::French.t("Couldn't send '{subject}': {why}. It is held in the Outbox.");
    let why = "le serveur ne permet pas à ce compte d’envoyer du courrier pour l’instant";
    let held = mailcraft_ui_egui::i18n::fmt(frame, &[("subject", "Plans"), ("why", why)]);
    assert!(french.contains(&held), "{french:#?}");
}
