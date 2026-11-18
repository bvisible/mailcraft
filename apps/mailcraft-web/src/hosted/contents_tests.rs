// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of hosted/contents.rs.
//! The heavy messages a person opened, kept in the browser: opened again, even at the next visit and past what the copy of the
//! mailboxes holds, they come from here and the server is not asked; each person's and each mailbox's are their own; the least
//! recently opened go first, within what the browser has room for.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::future::{Future, ready};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

use mailcraft_engine::cmd::jmap_mail::apply_jmap_mail;
use mailcraft_jmap::block_on;
use mailcraft_model::sync::{
    FolderListing, GraphFetchResult, GraphFetched, GraphFolder, GraphMessage, JmapFetchRequest, JmapMailResult, MessagePage, ServerMessage,
};
use mailcraft_model::{Address, FolderKind, Id, Mailbox};
use mailcraft_ui_egui::jmap_job::JmapFetchJob;
use mailcraft_ui_egui::{MailApp, Services};

use super::{
    Entry, HEAVY, Index, MAX_BYTES, Shelf, answered, foreign, index_key, left_the_mailbox, live, mailbox_tag, record_key, room, worth_asking,
};
use crate::hosted::{HostAccount, Runners, cache, hosted_session, hosted_window};

const NOW: i64 = 1_792_000_000;
const SCOPE: &str = "v1:person";
const ADA: &str = "ada@example.org";
const SESSION: &str = "https://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session?account=ada%40example.org";

/// A shelf in memory, as the browser's store behaves.
#[derive(Default)]
struct Memory {
    index: RefCell<Index>,
    records: RefCell<HashMap<String, Vec<u8>>>,
    forgotten: RefCell<Vec<String>>,
    room: Cell<Option<u64>>,
}

impl Shelf for Memory {
    fn take(&self, keys: &[String], opened: Option<i64>) -> impl Future<Output = Vec<(String, Vec<u8>)>> {
        let mut out = Vec::new();
        for key in keys {
            if let Some(bytes) = self.records.borrow().get(key) {
                out.push((key.clone(), bytes.clone()));
                if let Some(at) = opened {
                    self.index.borrow_mut().touch(key, at);
                }
            }
        }
        ready(out)
    }

    fn keep(&self, key: &str, mime: &[u8], at: i64) {
        let room = self.room.get().unwrap_or(MAX_BYTES);
        if let Some(gone) = self.index.borrow_mut().keep(key, mime.len() as u64, at, room) {
            let mut records = self.records.borrow_mut();
            for old in gone {
                records.remove(&old);
            }
            records.insert(key.to_string(), mime.to_vec());
        }
    }

    fn forget(&self, keys: &[String]) {
        for key in keys {
            self.index.borrow_mut().remove(key);
            self.records.borrow_mut().remove(key);
            self.forgotten.borrow_mut().push(key.clone());
        }
    }
}

fn job(ids: &[&str], background: bool) -> JmapFetchJob {
    let request = JmapFetchRequest {
        account: 7,
        email: ADA.into(),
        session: SESSION.into(),
        ids: ids.iter().map(|id| (*id).to_string()).collect(),
        background,
        ..Default::default()
    };
    JmapFetchJob { request, secret: None, cancel: Arc::new(AtomicBool::new(false)) }
}

/// A server that has every message asked for, each of `size` bytes, and notes what it is asked.
struct Server {
    size: usize,
    asked: RefCell<Vec<Vec<String>>>,
}

impl Server {
    fn new(size: usize) -> Server {
        Server { size, asked: RefCell::default() }
    }

    fn answer(&self, job: &JmapFetchJob) -> GraphFetchResult {
        self.asked.borrow_mut().push(job.request.ids.clone());
        let fetched = job.request.ids.iter().map(|id| GraphFetched { id: id.clone(), mime: bytes_of(id, self.size) }).collect();
        GraphFetchResult {
            account: job.request.account,
            email: job.request.email.clone(),
            session: job.request.session.clone(),
            fetched,
            ..Default::default()
        }
    }
}

fn bytes_of(id: &str, size: usize) -> Vec<u8> {
    id.bytes().cycle().take(size).collect()
}

fn fetch(shelf: &Memory, server: &Server, job_: JmapFetchJob, now: i64) -> GraphFetchResult {
    block_on(answered(shelf, SCOPE, job_, now, |job| ready(server.answer(&job))))
}

fn ids(answer: &GraphFetchResult) -> Vec<&str> {
    answer.fetched.iter().map(|m| m.id.as_str()).collect()
}

#[test]
fn a_heavy_message_opened_again_comes_from_the_browser_and_the_server_is_not_asked() {
    let (shelf, server) = (Memory::default(), Server::new(HEAVY));
    let first = fetch(&shelf, &server, job(&["E1"], false), NOW);
    assert_eq!(ids(&first), ["E1"]);
    assert_eq!(server.asked.borrow().len(), 1);
    let again = fetch(&shelf, &server, job(&["E1"], false), NOW + 60);
    assert_eq!(server.asked.borrow().len(), 1, "not asked again");
    assert_eq!(again.fetched, first.fetched, "the same bytes");
    assert_eq!((again.account, again.email.as_str(), again.session.as_str()), (7, ADA, SESSION), "an answer the window takes in");
    assert_eq!(shelf.index.borrow().0[0].at, NOW + 60, "marked opened");
}

#[test]
fn only_what_is_not_kept_is_asked_of_the_server() {
    let (shelf, server) = (Memory::default(), Server::new(HEAVY));
    fetch(&shelf, &server, job(&["E1"], false), NOW);
    let both = fetch(&shelf, &server, job(&["E1", "E2"], false), NOW + 1);
    assert_eq!(server.asked.borrow()[1], ["E2"]);
    assert_eq!(ids(&both), ["E1", "E2"]);
    assert_eq!(both.fetched[0].mime, bytes_of("E1", HEAVY));
}

#[test]
fn a_light_message_is_the_copys_and_the_background_keeps_nothing_but_reads_what_is_here() {
    let (shelf, light) = (Memory::default(), Server::new(HEAVY - 1));
    fetch(&shelf, &light, job(&["E1"], false), NOW);
    assert!(shelf.records.borrow().is_empty(), "too light to be kept here");
    let heavy = Server::new(HEAVY);
    fetch(&shelf, &heavy, job(&["E2"], true), NOW);
    assert!(shelf.records.borrow().is_empty(), "nobody opened it");
    fetch(&shelf, &heavy, job(&["E3"], false), NOW);
    let read = fetch(&shelf, &heavy, job(&["E3"], true), NOW + 500);
    assert_eq!(ids(&read), ["E3"]);
    assert_eq!(heavy.asked.borrow().len(), 2, "read from here");
    assert_eq!(shelf.index.borrow().0[0].at, NOW, "the background opens nothing");
}

#[test]
fn what_the_server_says_is_gone_is_forgotten() {
    let shelf = Memory::default();
    let answer = block_on(answered(&shelf, SCOPE, job(&["E9"], false), NOW, |job| {
        ready(GraphFetchResult { account: job.request.account, missing: vec!["E9".into()], ..Default::default() })
    }));
    assert!(answer.fetched.is_empty());
    assert_eq!(*shelf.forgotten.borrow(), [record_key(SCOPE, &mailbox_tag(ADA, SESSION), "E9")]);
}

#[test]
fn each_person_and_each_mailbox_has_its_own() {
    let (shelf, server) = (Memory::default(), Server::new(HEAVY));
    fetch(&shelf, &server, job(&["E1"], false), NOW);
    // The same message id, for another person (another copy's key): not theirs to read.
    let theirs = block_on(answered(&shelf, "v1:someone-else", job(&["E1"], false), NOW, |job| ready(server.answer(&job))));
    assert_eq!(ids(&theirs), ["E1"]);
    assert_eq!(server.asked.borrow().len(), 2, "asked of the server for them");
    // Another mailbox of the same person: an id of its own server.
    let mut other = job(&["E1"], false);
    other.request.email = "team@example.org".into();
    block_on(answered(&shelf, SCOPE, other, NOW, |job| ready(server.answer(&job))));
    assert_eq!(server.asked.borrow().len(), 3);
    assert_ne!(mailbox_tag(ADA, SESSION), mailbox_tag("team@example.org", SESSION));
    assert_eq!(mailbox_tag(" ADA@example.org ", SESSION), mailbox_tag(ADA, SESSION));
    assert!(!record_key(SCOPE, &mailbox_tag(ADA, SESSION), "E1").contains("example"), "no address in clear");
}

#[test]
fn at_each_start_what_is_not_this_persons_goes() {
    let (mine, known) = (record_key(SCOPE, "tag", "E1"), record_key(SCOPE, "tag", "E2"));
    let index = Index(vec![Entry { key: known.clone(), size: 1, at: NOW }]);
    let theirs = record_key("v1:someone-else", "tag", "E1");
    let keys = vec![index_key(SCOPE), mine.clone(), known, theirs.clone(), index_key("v1:someone-else")];
    assert_eq!(foreign(&keys, Some(SCOPE), &index), [mine, theirs, index_key("v1:someone-else")], "and what its index doesn't name");
    assert_eq!(foreign(&keys, None, &Index::default()), keys, "a page that names nobody keeps none");
}

fn entry(key: &str, size: u64, at: i64) -> Entry {
    Entry { key: key.into(), size, at }
}

#[test]
fn the_least_recently_opened_go_first_and_what_cannot_fit_takes_nothing_away() {
    let mut index = Index(vec![entry("a", 40, 300), entry("b", 40, 100), entry("c", 40, 200)]);
    assert_eq!(index.keep("d", 50, 400, 120), Some(vec!["b".to_string(), "c".to_string()]));
    let mut kept: Vec<&str> = index.0.iter().map(|e| e.key.as_str()).collect();
    kept.sort_unstable();
    assert_eq!(kept, ["a", "d"]);
    assert_eq!(index.used(), 90);
    // Opened again, it is the last to go.
    index.touch("a", 500);
    assert_eq!(index.keep("e", 60, 501, 130), Some(vec!["d".to_string()]));
    // Heavier than all the room: not kept, and nothing goes.
    let before = index.clone();
    assert_eq!(index.keep("f", 131, 502, 130), None);
    assert_eq!(index, before);
    // The same message again: one record.
    assert_eq!(index.keep("e", 70, 503, 130), Some(vec![]));
    assert_eq!((index.0.iter().filter(|e| e.key == "e").count(), index.used()), (1, 110));
}

#[test]
fn the_index_reads_back_and_junk_reads_as_nothing() {
    let index = Index(vec![entry("k\u{1f}a", 3, 9)]);
    assert_eq!(Index::parse(&index.to_text()), index);
    for junk in ["", "{", "3", "[1, [2]]"] {
        assert_eq!(Index::parse(junk), Index::default(), "{junk}");
    }
}

#[test]
fn what_is_kept_stays_within_what_the_browser_has_room_for() {
    assert_eq!(room(0, None), MAX_BYTES, "a browser that doesn't say");
    assert_eq!(room(0, Some((0, 0))), MAX_BYTES);
    assert_eq!(room(0, Some((0, u64::MAX))), MAX_BYTES, "never more than its bound");
    assert_eq!(room(10, Some((100, 300))), 110, "what it has, and half of what is left");
    assert_eq!(room(10, Some((400, 300))), 10, "nothing left: what is kept may only be replaced");
}

fn ids_set(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|id| (*id).to_string()).collect()
}

#[test]
fn a_message_that_leaves_its_mailbox_is_forgotten_and_one_merely_not_read_is_not() {
    let tag = mailbox_tag(ADA, SESSION);
    let before = vec![(tag.clone(), ids_set(&["E1", "E2"]))];
    let now = vec![(tag.clone(), ids_set(&["E1"]))];
    assert_eq!(left_the_mailbox(SCOPE, &before, &now), [record_key(SCOPE, &tag, "E2")], "it was here, and is no longer");
    // Never here in this visit (older than what a run reads of its folder): it is still on the server, and stays kept.
    assert!(left_the_mailbox(SCOPE, &now, &before).is_empty());
    // A mailbox not looked at now: nothing can be said.
    assert!(left_the_mailbox(SCOPE, &before, &[]).is_empty());
}

#[test]
fn the_browser_is_only_asked_for_what_it_may_have() {
    let (kept, other) = (record_key(SCOPE, "tag", "E1"), record_key(SCOPE, "tag", "E2"));
    let asked = [kept.clone(), other.clone()];
    assert_eq!(worth_asking(&asked, Some(&HashSet::from([kept.clone()]))), std::slice::from_ref(&kept), "a message that isn't kept costs nothing");
    assert!(worth_asking(&asked, Some(&HashSet::new())).is_empty());
    assert_eq!(worth_asking(&asked, None), asked, "before its keys are known, it is asked for all");
}

fn ada() -> HostAccount {
    HostAccount { email: ADA.into(), name: "Ada".into(), session: SESSION.into(), username: ADA.into(), ..HostAccount::default() }
}

/// A MIME message of `id` with a file that makes it heavy.
fn heavy_mime(id: &str) -> Vec<u8> {
    let file = "QUJD".repeat(HEAVY / 4 + 1);
    format!(
        "From: Grace <grace@example.org>\r\nTo: {ADA}\r\nSubject: Plans\r\nMessage-ID: <{id}@mail.example.org>\r\nDate: Wed, 14 Oct 2026 09:00:00 +0000\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe words of {id}.\r\n\
--b\r\nContent-Type: application/pdf; name=\"plan.pdf\"\r\nContent-Disposition: attachment; filename=\"plan.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{file}\r\n--b--\r\n"
    )
    .into_bytes()
}

/// The window of a hosting page over `kept` (the copy of the last visit) or a new session, its fetches going through `shelf` and
/// a server that counts what it is asked.
fn window(kept: Option<Mailbox>, shelf: &Rc<Memory>, asked: &Rc<Cell<usize>>) -> MailApp {
    let (shelf, asked) = (shelf.clone(), asked.clone());
    let runners = Runners {
        jmap: Box::new(|_| mpsc::channel().1),
        jmap_fetch: Box::new(move |job| {
            let (tx, rx) = mpsc::channel();
            let answer = block_on(answered(&*shelf, SCOPE, job, NOW, |job| {
                asked.set(asked.get() + 1);
                let fetched = job.request.ids.iter().map(|id| GraphFetched { id: id.clone(), mime: heavy_mime(id) }).collect();
                ready(GraphFetchResult {
                    account: job.request.account,
                    email: job.request.email.clone(),
                    session: job.request.session.clone(),
                    fetched,
                    ..Default::default()
                })
            }));
            tx.send(answer).expect("the window waits");
            rx
        }),
        jmap_pim: Box::new(|_| mpsc::channel().1),
    };
    let mut app = hosted_window(hosted_session(&[ada()], kept), Services::default(), runners);
    app.session.driver.set_auto_sync(false);
    app.session.sync_progress.full_paused = true;
    app
}

fn account_of(app: &MailApp) -> Id {
    app.session.mailbox.accounts.iter().find(|a| a.jmap.is_some()).map(|a| a.id).expect("the mailbox")
}

fn message_of(app: &MailApp) -> Id {
    app.session.mailbox.messages.values().find(|m| m.server.as_ref().is_some_and(|s| s.id == "E1")).map(|m| m.id).expect("E1")
}

#[test]
fn opened_at_the_next_visit_a_message_past_the_copys_bounds_is_whole_without_the_server() {
    let (shelf, asked) = (Rc::new(Memory::default()), Rc::new(Cell::new(0)));
    let mut app = window(None, &shelf, &asked);
    let account = account_of(&app);
    let folders = vec![GraphFolder { id: "Min".into(), name: "INBOX".into(), kind: FolderKind::Inbox, ..Default::default() }];
    let listing = JmapMailResult { account, email: ADA.into(), listing: Some(FolderListing { folders, complete: true }), ..Default::default() };
    apply_jmap_mail(&mut app.session, listing).expect("listed");
    let message = GraphMessage {
        server: ServerMessage { id: "E1".into(), size: 2_000_000, ..Default::default() },
        message_id: "<E1@mail.example.org>".into(),
        subject: "Plans".into(),
        from: Address::new("Grace", "grace@example.org"),
        received: NOW - 60,
        preview: "The words".into(),
        has_attachments: true,
        ..Default::default()
    };
    let page = MessagePage { folder: "Min".into(), messages: vec![message], afresh: true, ..Default::default() };
    apply_jmap_mail(&mut app.session, JmapMailResult { account, email: ADA.into(), pages: vec![page], whole: true, ..Default::default() })
        .expect("taken in");

    // Opened: downloaded, whole.
    app.session.sel.messages = vec![message_of(&app)];
    app.drive_fetch();
    app.drive_fetch();
    assert_eq!(asked.get(), 1);
    let m = app.session.mailbox.message(message_of(&app)).expect("there");
    assert!(!m.unfetched && m.attachments.len() == 1, "{:?}", (m.unfetched, m.attachments.len()));
    let file = m.attachments[0].clone();

    // The next visit, on a copy that has no room for its file: its text is there, its file left out.
    let bytes = cache::encode_within(&app.session.mailbox, 0, 500, cache::MAX_BYTES).expect("written");
    let kept = cache::decode(&bytes, &[ada()]).expect("read");
    let mut next = window(Some(kept), &shelf, &asked);
    let opened = message_of(&next);
    let m = next.session.mailbox.message(opened).expect("there");
    assert!(m.unfetched && m.files_on_server && m.attachments.is_empty(), "the copy left its file out");
    next.session.sel.messages = vec![opened];
    next.drive_fetch();
    next.drive_fetch();
    assert_eq!(asked.get(), 1, "the server isn't asked again");
    let m = next.session.mailbox.message(opened).expect("there");
    assert!(!m.unfetched && !m.files_on_server, "whole");
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(m.attachments[0], file, "its file, as it came");
    assert!(m.body.text.contains("The words of E1"));
}

#[test]
fn a_mailbox_says_the_ids_it_holds_on_its_server() {
    let mut app = window(None, &Rc::new(Memory::default()), &Rc::new(Cell::new(0)));
    let account = account_of(&app);
    let folders = vec![GraphFolder { id: "Min".into(), name: "INBOX".into(), kind: FolderKind::Inbox, ..Default::default() }];
    let listing = JmapMailResult { account, email: ADA.into(), listing: Some(FolderListing { folders, complete: true }), ..Default::default() };
    apply_jmap_mail(&mut app.session, listing).expect("listed");
    let mail = |id: &str| GraphMessage { server: ServerMessage { id: id.into(), ..Default::default() }, received: NOW, ..Default::default() };
    let page = MessagePage { folder: "Min".into(), messages: vec![mail("E1"), mail("E2"), mail("E3")], afresh: true, ..Default::default() };
    apply_jmap_mail(&mut app.session, JmapMailResult { account, email: ADA.into(), pages: vec![page], whole: true, ..Default::default() })
        .expect("taken in");
    let gone = app.session.mailbox.messages.values().find(|m| m.server.as_ref().is_some_and(|s| s.id == "E3")).map(|m| m.id).expect("E3");
    app.session.mailbox.message_mut(gone).expect("there").server.as_mut().expect("its server").gone = true;
    assert_eq!(live(&app.session.mailbox), [(mailbox_tag(ADA, SESSION), ids_set(&["E1", "E2"]))]);
}
