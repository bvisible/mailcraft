// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what the person does around a renewal of the page's token is not lost.

//! The whole way of what the person does with mail, from the window to the server: the window's run (`jmap_job.rs`), the
//! browser runner's core ([`super::run`]) and the in-process server, on the test's own thread, with the page's credential,
//! which the page renews when the server refuses it. A message read, one archived and a reply sent before, during and after a
//! run whose token is refused, at whatever request of it: nothing of it is lost, undone here, or done twice. Nothing here
//! leaves this machine; every name, address and id is invented.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

use mailcraft_engine::Session;
use mailcraft_jmap::standin::{SESSION, StandIn, TOKEN};
use mailcraft_jmap::{Auth, block_on};
use mailcraft_model::sync::JmapMailResult;
use mailcraft_model::{FolderKind, Id, Mailbox};
use mailcraft_ui_egui::jmap_job::JmapJob;
use mailcraft_ui_egui::{MailApp, Services};
use serde_json::json;

use crate::credentials::{Credentials, Renewal};

const ME: &str = "ada@example.org";
const NOW: i64 = 1_791_993_600;

/// The page's credential: the token it handed over, and whatever the server holds when it is asked for another.
struct Page {
    token: RefCell<String>,
    server: StandIn,
    renewals: Cell<u32>,
}

impl Credentials for Page {
    fn auth(&self, _email: &str) -> Option<Auth> {
        Some(Auth::bearer(self.token.borrow().clone()))
    }

    async fn renew(&self, _email: &str) -> Renewal {
        self.renewals.set(self.renewals.get() + 1);
        let fresh = format!("fresh-{}", self.renewals.get());
        self.server.with(|d| d.token = Some(fresh.clone()));
        *self.token.borrow_mut() = fresh.clone();
        Renewal::Given(Auth::bearer(fresh))
    }
}

struct Flow {
    app: MailApp,
    server: StandIn,
    page: Rc<Page>,
    account: Id,
    /// Runs are held rather than made at once, and the one held.
    holding: Rc<Cell<bool>>,
    held: Rc<RefCell<Option<(JmapJob, Sender<JmapMailResult>)>>>,
}

fn go(server: &StandIn, page: &Page, job: &JmapJob, to: &Sender<JmapMailResult>) {
    block_on(super::run(server.clone(), job, Some(page), |_| std::future::ready(()), &mut |piece| {
        let _ = to.send(piece);
    }));
}

impl Flow {
    fn start() -> Flow {
        let server = StandIn::new();
        server.with(|d| {
            let inbox = d.mailbox("Inbox", None, Some("inbox"));
            d.mailbox("Sent", None, Some("sent"));
            d.mailbox("Drafts", None, Some("drafts"));
            d.mailbox("Archive", None, Some("archive"));
            d.identity("Ada", ME);
            for (n, received) in [(1, "2026-10-14T09:30:00Z"), (2, "2026-10-14T10:30:00Z"), (3, "2026-10-14T11:30:00Z")] {
                let raw = format!(
                    "From: Zoe <zoe@example.org>\r\nTo: Ada <{ME}>\r\nSubject: Hello {n}\r\nMessage-ID: <m{n}@example.org>\r\n\r\nHello.\r\n"
                );
                d.deliver(&inbox, raw.as_bytes(), &[], received);
            }
        });
        let page = Rc::new(Page { token: RefCell::new(TOKEN.into()), server: server.clone(), renewals: Cell::new(0) });
        let mut session = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
        session.keeps_local = false;
        let mut app = MailApp::new(session, Services::default());
        app.autosave = false;
        app.session.driver.set_auto_sync(false);
        app.session.jmap_runs = true;
        app.services.jmap_host_credentials = true;
        let (holding, held) = (Rc::new(Cell::new(false)), Rc::new(RefCell::new(None)));
        let (to, credentials, hold, keep) = (server.clone(), page.clone(), holding.clone(), held.clone());
        app.services.jmap = Some(Box::new(move |job: JmapJob| {
            let (tx, rx) = mpsc::channel();
            if hold.get() {
                *keep.borrow_mut() = Some((job, tx));
            } else {
                go(&to, &credentials, &job, &tx);
            }
            rx
        }));
        let added =
            app.execute("account.add", json!({"email": ME, "displayName": "Ada", "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}}));
        let account = added.unwrap()["id"].as_u64().unwrap();
        let mut flow = Flow { app, server, page, account, holding, held };
        flow.sync();
        assert_eq!(flow.app.session.mailbox.messages.values().filter(|m| m.subject.starts_with("Hello")).count(), 3);
        flow
    }

    /// A Send/Receive, taken in to its end.
    fn sync(&mut self) {
        let _ = self.app.execute("ui.sync", json!({"account": self.account}));
        self.take_in();
    }

    fn take_in(&mut self) {
        for _ in 0..200 {
            self.app.poll_jmap();
        }
    }

    /// A Send/Receive that is on its way until [`Flow::release`].
    fn hold(&mut self) {
        self.holding.set(true);
        self.app.execute("ui.sync", json!({"account": self.account})).unwrap();
        self.holding.set(false);
        assert!(self.held.borrow().is_some(), "a run is on its way");
    }

    fn release(&mut self) {
        if let Some((job, tx)) = self.held.borrow_mut().take() {
            go(&self.server, &self.page, &job, &tx);
        }
        self.take_in();
    }

    fn message(&self, subject: &str) -> Id {
        self.app.session.mailbox.messages.values().find(|m| m.subject == subject).unwrap().id
    }

    /// What the person does: reads the first, archives the second, replies to the third.
    fn acts(&mut self) {
        let (first, second, third) = (self.message("Hello 1"), self.message("Hello 2"), self.message("Hello 3"));
        self.app.execute("mail.markRead", json!({"ids": [first], "value": true})).unwrap();
        self.app.execute("mail.archive", json!({"ids": [second]})).unwrap();
        self.app.execute("mail.reply", json!({"id": third})).unwrap();
        self.app.execute("compose.send", json!({"confirmed": true})).unwrap();
    }

    /// The window shows what the person did, whatever a run brought meanwhile.
    fn shown(&self, when: &str) {
        let mb = &self.app.session.mailbox;
        let of = |subject: &str| mb.messages.values().find(|m| m.subject == subject).unwrap();
        assert!(of("Hello 1").read, "{when}: read");
        let archive = mb.folder_of_kind(self.account, FolderKind::Archive).unwrap();
        assert_eq!(of("Hello 2").folder, archive, "{when}: archived");
        let replies = mb.messages.values().filter(|m| m.subject == "RE: Hello 3").count();
        assert_eq!(replies, 1, "{when}: one reply, in the Outbox or in Sent Items");
    }

    /// The server has what the person did, once.
    fn there(&self, when: &str) {
        self.server.with(|d| {
            let find = |id: &str| d.emails.iter().find(|e| String::from_utf8_lossy(&e.raw).contains(id) && !e.keywords.contains("$draft")).cloned();
            assert!(find("<m1@example.org>").is_some_and(|e| e.keywords.contains("$seen")), "{when}: read there");
            let archive = d.mailbox_named("Archive").unwrap().to_string();
            assert!(find("<m2@example.org>").is_some_and(|e| e.mailboxes.contains(&archive)), "{when}: archived there");
            let sent = d.delivered.iter().filter(|sent| String::from_utf8_lossy(&sent.raw).contains("Subject: RE: Hello 3")).count();
            assert_eq!(sent, 1, "{when}: sent once");
        });
    }
}

#[test]
fn what_is_done_while_a_run_waits_for_a_new_token_is_kept_and_goes_next() {
    let mut flow = Flow::start();
    flow.server.with(|d| d.token = Some("rotated".into()));
    flow.hold();
    flow.acts();
    flow.shown("while the run is on its way");
    flow.release();
    assert_eq!(flow.page.renewals.get(), 1, "refused, renewed");
    flow.shown("after the run that was renewed");
    flow.sync();
    flow.shown("after the next run");
    flow.there("after the next run");
}

#[test]
fn what_is_done_just_before_or_just_after_a_renewal_goes_once() {
    // The token dies at each request of the run in turn.
    let mut renewed = 0;
    for after in 0..12 {
        let mut flow = Flow::start();
        flow.acts();
        flow.server.with(|d| d.drop_token_after = Some(after));
        flow.sync();
        flow.shown(&format!("token dead after {after}"));
        for _ in 0..3 {
            flow.sync();
        }
        flow.shown(&format!("token dead after {after}, later"));
        flow.there(&format!("token dead after {after}, later"));
        renewed += flow.page.renewals.get();
    }
    assert!(renewed >= 8, "most of them met the refusal: {renewed}");
    // And what is done right after the run that was renewed.
    let mut flow = Flow::start();
    flow.server.with(|d| d.token = Some("rotated".into()));
    flow.sync();
    assert_eq!(flow.page.renewals.get(), 1);
    flow.acts();
    flow.sync();
    flow.shown("right after a renewal");
    flow.there("right after a renewal");
}
