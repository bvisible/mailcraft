// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what the tests of Nora share.
//! What the tests of Nora share: a host that keeps every call for the test to answer, the sample
//! mailbox in a window that reaches her, and a hosting page's window with the mailbox it handed over
//! (invented people at `example.org`).

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

use mailcraft_engine::Session;
use mailcraft_engine::cmd::jmap_mail::apply_jmap_mail;
use mailcraft_model::sync::{FolderListing, GraphFolder, GraphMessage, JmapMailResult, MessagePage, ServerMessage};
use mailcraft_model::{Address, FolderKind, Id, Mailbox};
use serde_json::{Value, json};

use super::{About, Host, METHODS, NoraAnswer, NoraCall, PageMailbox};
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
pub(crate) const NOW: i64 = 1_791_993_600;
/// The mailbox a hosting page hands over, and its JMAP session.
pub(crate) const ME: &str = "ada@example.org";
pub(crate) const SESSION: &str = "https://mail.example.org/jmap/session";
/// A plain-text message of the sample mailbox (the sample's account has a signature for replies).
pub(crate) const PLAIN: &str = "Q4 roadmap review moved to Thursday";
pub(crate) const PRIYA: &str = "From: Priya Nair <priya@lumen.example>";
pub(crate) const REFUSED: &str = "Nora refused to process this message: it contains instructions aimed at the assistant, or could not be verified.";

/// The calls a host was asked, each with the way to answer it.
#[derive(Clone, Default)]
pub(crate) struct Calls(pub(crate) Rc<RefCell<Vec<(NoraCall, Option<Sender<NoraAnswer>>)>>>);

impl Calls {
    pub(crate) fn count(&self) -> usize {
        self.0.borrow().len()
    }

    pub(crate) fn last(&self) -> NoraCall {
        self.0.borrow().last().map(|(call, _)| call.clone()).unwrap()
    }

    /// Answer the last call.
    pub(crate) fn answer(&self, status: u16, body: Value) {
        let calls = self.0.borrow();
        let (call, to) = calls.last().unwrap();
        to.as_ref().unwrap().send(NoraAnswer { id: call.id, status, body: body.to_string() }).unwrap();
    }

    /// Answer the last call, which the window may have stopped waiting for: whether it was still waited for.
    pub(crate) fn answer_late(&self, status: u16, body: Value) -> bool {
        let calls = self.0.borrow();
        let (call, to) = calls.last().unwrap();
        to.as_ref().unwrap().send(NoraAnswer { id: call.id, status, body: body.to_string() }).is_ok()
    }

    /// The host lets the last call go without an answer.
    pub(crate) fn drop_last(&self) {
        if let Some((_, to)) = self.0.borrow_mut().last_mut() {
            *to = None;
        }
    }

    /// Every call so far, each one of the six methods with a plain object of arguments, never a
    /// `cmd` (Neodesk's shell refuses it), and small.
    pub(crate) fn all_fit(&self) {
        for (call, _) in self.0.borrow().iter() {
            assert!(METHODS.contains(&call.method), "{}", call.method);
            let args = call.args.as_object().unwrap();
            assert!(!args.contains_key("cmd"));
            assert!(args.values().all(|v| v.is_string()), "{}", call.args);
            assert!(call.args.to_string().len() < 64 * 1024);
        }
    }
}

/// The host of a window that reaches Nora, for the mailboxes the page handed over: each one's address
/// and the name of the server's account for it (empty: the page didn't say).
pub(crate) fn host(app: &mut MailApp, mailboxes: &[(&str, &str)]) -> Calls {
    let calls = Calls::default();
    let kept = calls.0.clone();
    app.services.nora = Some(Host {
        call: Box::new(move |call: NoraCall| {
            let (tx, rx) = mpsc::channel();
            kept.borrow_mut().push((call, Some(tx)));
            rx
        }),
        mailboxes: mailboxes.iter().map(|(address, name)| PageMailbox { address: (*address).to_string(), name: (*name).to_string() }).collect(),
    });
    calls
}

pub(crate) fn quiet(mut app: MailApp) -> MailApp {
    app.autosave = false;
    app.auto_sync = false;
    app
}

/// The sample mailbox in a window that reaches Nora, `subject`'s message selected.
pub(crate) fn sample(subject: &str) -> (MailApp, Calls, Id) {
    let mut app = quiet(MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default()));
    let calls = host(&mut app, &[]);
    let id = app.session.mailbox.messages.values().find(|m| m.subject == subject).map(|m| m.id).unwrap();
    app.execute("mail.select", json!({"id": id})).unwrap();
    (app, calls, id)
}

pub(crate) fn server_mail(id: &str) -> GraphMessage {
    GraphMessage {
        server: ServerMessage { id: id.into(), size: 2_000, ..Default::default() },
        message_id: format!("<{id}@mail.example.org>"),
        subject: format!("The proof, {id}"),
        from: Address::new("Grace Hopper", "grace@example.org"),
        sent: 1_790_000_000,
        received: 1_790_000_000,
        preview: "Is the proof ready?".into(),
        ..Default::default()
    }
}

/// A hosting page's window: the mailbox it handed over (a JMAP account) with `E1` to `E3` in its
/// Inbox, downloaded whole when `whole`, `E1` selected; a host that reaches Nora for it. The page
/// doesn't say the name of the server's account for it.
pub(crate) fn page(whole: bool) -> (MailApp, Calls, Id) {
    page_named(whole, "")
}

/// [`page`], the page saying `name` is the name of the server's account for its mailbox.
pub(crate) fn page_named(whole: bool, name: &str) -> (MailApp, Calls, Id) {
    let mut app = quiet(MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default()));
    app.services.jmap_host_credentials = true;
    let added = app.execute("account.add", json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
    let account = added["id"].as_u64().unwrap();
    let folders = vec![GraphFolder { id: "Min".into(), name: "INBOX".into(), kind: FolderKind::Inbox, ..Default::default() }];
    let listing = JmapMailResult { account, email: ME.into(), listing: Some(FolderListing { folders, complete: true }), ..Default::default() };
    apply_jmap_mail(&mut app.session, listing).unwrap();
    let page = MessagePage { folder: "Min".into(), messages: ["E1", "E2", "E3"].map(server_mail).to_vec(), afresh: true, ..Default::default() };
    apply_jmap_mail(&mut app.session, JmapMailResult { account, email: ME.into(), pages: vec![page], whole: true, ..Default::default() }).unwrap();
    if whole {
        for id in ["E1", "E2", "E3"] {
            let m = app.session.mailbox.message_mut(id_of(&app, id)).unwrap();
            m.unfetched = false;
            m.body.text = "Hello Ada,\n\nIs the proof ready?\n\nGrace".into();
        }
    }
    // The background's downloads are held back, as after Stop: only what a test makes happen happens.
    app.session.sync_progress.full_paused = true;
    // The page names its mailbox in its own case.
    let calls = host(&mut app, &[("Ada@Example.org", name)]);
    let first = id_of(&app, "E1");
    app.execute("mail.select", json!({"id": first})).unwrap();
    (app, calls, account)
}

pub(crate) fn id_of(app: &MailApp, server_id: &str) -> Id {
    app.session.mailbox.messages.values().find(|m| m.server.as_ref().is_some_and(|s| s.id == server_id)).map(|m| m.id).unwrap()
}

pub(crate) fn body(app: &MailApp) -> String {
    app.session.compose.as_ref().map(|c| c.body.plain()).unwrap_or_default()
}

pub(crate) fn signature(app: &MailApp) -> Option<String> {
    app.session.compose.as_ref().and_then(|c| c.body.text().signature()).map(|s| s.text())
}

/// What the person was last told of what Nora did, where, and whether it went wrong.
pub(crate) fn said(app: &MailApp) -> Option<(String, bool, About)> {
    app.nora.said.as_ref().map(|s| (s.text.clone(), s.error, s.about))
}

pub(crate) fn status(app: &MailApp) -> String {
    app.status_msg.as_ref().map(|(s, _)| s.clone()).unwrap_or_default()
}

pub(crate) fn typed(app: &mut MailApp, text: &str) {
    app.execute("compose.select", json!({"from": 0})).unwrap();
    app.execute("compose.type", json!({"text": text})).unwrap();
}

pub(crate) fn refusal(said: &str) -> Value {
    let shown = json!({"message": said}).to_string();
    json!({"exc_type": "ValidationError", "_server_messages": json!([shown]).to_string()})
}

pub(crate) fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default()
}
