// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's Send/Receive in the window: a host that answers with pieces made by hand (from
//! a channel that already holds them, or from a thread of its own), the run's request and what is
//! asked of the owner, Cancel, a round that starts by itself, a sender that is dropped and a server
//! that turns the secret down. Invented people at `example.org`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mailcraft_engine::Session;
use mailcraft_engine::cmd::sync_progress::RunState;
use mailcraft_model::sync::{
    FolderListing, GraphFolder, GraphMessage, GraphProgress, JmapChange, JmapChangeDone, JmapKnown, JmapMailRequest, JmapMailResult, JmapOutcome,
    JmapScope, MessagePage, ServerMessage,
};
use mailcraft_model::{AccountKind, Address, FolderKind, Id, Mailbox};
use serde_json::json;

use super::JmapJob;
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const ME: &str = "ada@example.org";
/// The session address carries the account's own, as a server's may.
const SESSION: &str = "https://mail.example.org/jmap/session?account=ada@example.org";
const PASSWORD_KEY: &str = "password:ada@example.org";

/// One run the host was asked for, and the way to answer it.
struct Run {
    request: JmapMailRequest,
    secret: Option<String>,
    cancel: Arc<AtomicBool>,
    /// What the job prints.
    shown: String,
    to: Sender<JmapMailResult>,
}

/// The runs the host was asked for, in order.
type Asked = Arc<Mutex<Vec<Run>>>;

/// The system's credential store, as a map.
#[derive(Clone, Default)]
struct Store(Arc<Mutex<HashMap<String, String>>>);

impl crate::SecretStore for Store {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.to_string(), secret.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}

impl Store {
    fn kept(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }
}

/// A window with a JMAP account and a host that keeps every job it is handed for the test to
/// answer. `host_holds_credentials`: the host holds the credential itself; `bearer`: the account's
/// secret is a token. The account's id.
fn window(host_holds_credentials: bool, bearer: bool) -> (MailApp, Id, Asked) {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    let asked: Asked = Default::default();
    let log = asked.clone();
    app.services.jmap = Some(Box::new(move |job: JmapJob| {
        let (tx, rx) = mpsc::channel();
        let shown = format!("{job:?} {job:#?}");
        log.lock().unwrap().push(Run { request: job.request, secret: job.secret, cancel: job.cancel, shown, to: tx });
        rx
    }));
    app.services.jmap_host_credentials = host_holds_credentials;
    let added = app.execute("account.add", json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": bearer}}));
    let id = added.unwrap()["id"].as_u64().unwrap();
    (app, id, asked)
}

fn folder(id: &str, parent: &str, name: &str, kind: FolderKind) -> GraphFolder {
    GraphFolder { id: id.into(), parent: parent.into(), name: name.into(), kind, ..Default::default() }
}

/// The server's mailboxes: the ones with a role, and one of the owner's.
fn mailboxes() -> Vec<GraphFolder> {
    vec![
        folder("Min", "", "INBOX", FolderKind::Inbox),
        folder("Mdr", "", "Drafts", FolderKind::Drafts),
        folder("Mse", "", "Sent", FolderKind::Sent),
        folder("Mtr", "", "Trash", FolderKind::Deleted),
        folder("Mju", "", "Junk", FolderKind::Junk),
        folder("Mpr", "", "Projects", FolderKind::Custom),
    ]
}

fn mail(id: &str, subject: &str, received: i64) -> GraphMessage {
    GraphMessage {
        server: ServerMessage { id: id.into(), ..Default::default() },
        message_id: format!("<{id}@mail.example.org>"),
        subject: subject.into(),
        from: Address::new("Grace", "grace@example.org"),
        sent: received,
        received,
        preview: format!("{subject}…"),
        ..Default::default()
    }
}

/// A piece of a run: `partial` while more of it comes.
fn piece(partial: bool) -> JmapMailResult {
    JmapMailResult { email: ME.into(), partial, ..Default::default() }
}

/// The server's mailboxes, in one piece that is not the last.
fn listing() -> JmapMailResult {
    let progress = GraphProgress { folder: "Min".into(), read: 0, total: 2, number: 1, folders: 2 };
    JmapMailResult { listing: Some(FolderListing { folders: mailboxes(), complete: true }), progress: Some(progress), ..piece(true) }
}

/// A mailbox's window in a piece of its own, which holds the whole of the mailbox.
fn window_of(mailbox: &str, messages: Vec<GraphMessage>, partial: bool) -> JmapMailResult {
    JmapMailResult { pages: vec![MessagePage { folder: mailbox.into(), messages, afresh: true, ..Default::default() }], ..piece(partial) }
}

/// A mailbox's window in a piece of its own, cut short: it adds and updates, and removes nothing.
fn more_of(mailbox: &str, messages: Vec<GraphMessage>, partial: bool) -> JmapMailResult {
    JmapMailResult { pages: vec![MessagePage { folder: mailbox.into(), messages, ..Default::default() }], ..piece(partial) }
}

/// The end of a run that read everything it meant to.
fn the_end() -> JmapMailResult {
    JmapMailResult { whole: true, ..piece(false) }
}

/// Hand the host's `run`-th job a piece.
fn answer(asked: &Asked, run: usize, piece: JmapMailResult) {
    asked.lock().unwrap()[run].to.send(piece).unwrap();
}

fn jobs(asked: &Asked) -> usize {
    asked.lock().unwrap().len()
}

fn count(app: &MailApp, account: Id, mailbox: &str) -> usize {
    app.session.mailbox.folder_by_server(account, mailbox).map_or(0, |f| app.session.mailbox.messages_in(f).count())
}

fn state(app: &MailApp, account: Id) -> Option<RunState> {
    app.session.sync_progress.accounts.iter().find(|r| r.account == account).map(|r| r.state)
}

/// Take in pieces until `done` (a host on a thread of its own answers when it likes).
fn until(app: &mut MailApp, what: &str, done: impl Fn(&MailApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(app) {
        assert!(Instant::now() < deadline, "gave up waiting for {what}");
        app.poll_jmap();
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_send_receive_the_owner_asks_for_fills_the_mailbox_and_says_it_is_up_to_date() {
    let (mut app, id, asked) = window(true, true);
    assert!(app.enabled("ui.sync") && !app.enabled("ui.cancelSync"));
    let said = app.execute("ui.sync", json!({"account": id})).unwrap();
    assert_eq!(said, json!({"started": true, "account": id, "jmap": true}));
    assert!(app.syncing() && !app.enabled("ui.sync") && app.enabled("ui.cancelSync"));
    assert_eq!(app.session.sync_status, "Sending and receiving…");
    assert!(app.execute("ui.sync", json!({"account": id})).is_err(), "one run at a time");
    // The host was asked once, for everything: the owner asked, and it is the account's first run.
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        let job = &asked[0];
        let request = &job.request;
        assert_eq!((request.account, request.email.as_str(), request.session.as_str(), request.bearer), (id, ME, SESSION, true));
        assert_eq!((request.scope.clone(), request.list_folders, request.window, request.background), (JmapScope::All, true, 500, false));
        // The host holds the credential: the job carries none.
        assert_eq!(job.secret, None);
        assert!(!job.cancel.load(Ordering::Relaxed));
    }
    // Nothing has come yet: it goes on waiting.
    app.poll_jmap();
    assert!(app.syncing());

    // The folder list: the account's standard folders are the server's, by role.
    answer(&asked, 0, listing());
    app.poll_jmap();
    let inbox = app.session.mailbox.folder_by_server(id, "Min").unwrap();
    assert_eq!(app.session.mailbox.folder_of_kind(id, FolderKind::Inbox), Some(inbox));
    assert!(app.session.mailbox.folder_by_server(id, "Mpr").is_some());
    assert!(app.syncing(), "more is to come");
    assert_eq!(state(&app, id), Some(RunState::Running));
    // The Inbox, then Projects; mail shows up as it is taken in.
    answer(&asked, 0, window_of("Min", vec![mail("E1", "One", 100), mail("E2", "Two", 200)], true));
    app.poll_jmap();
    assert_eq!(count(&app, id, "Min"), 2);
    assert!(app.syncing());
    answer(&asked, 0, window_of("Mpr", vec![mail("E3", "Three", 300)], true));
    answer(&asked, 0, the_end());
    app.poll_jmap();
    assert_eq!((count(&app, id, "Mpr"), count(&app, id, "Min")), (1, 2));

    // The last piece ended it.
    assert!(!app.syncing() && app.jmap_rx.is_none() && app.enabled("ui.sync") && !app.enabled("ui.cancelSync"));
    assert!(app.session.sync_status.starts_with("Up to date — "), "{}", app.session.sync_status);
    let row = app.session.sync_progress.accounts.iter().find(|r| r.account == id).unwrap();
    assert_eq!((row.state, row.new), (RunState::Done, 3));
    assert!(!app.has_sync_problems() && app.push_seen.contains_key(&id));
    // Nothing of a message is whole: they are headers and a preview, until the next steps fetch them.
    assert!(app.session.mailbox.messages.values().all(|m| m.unfetched && m.server.is_some()));

    // The next run of the owner's reads everything again, naming the messages it has by their ids.
    app.execute("ui.sync", json!({"account": id})).unwrap();
    let request = asked.lock().unwrap()[1].request.clone();
    assert_eq!((request.scope.clone(), request.list_folders), (JmapScope::All, true));
    assert!(request.known.contains(&JmapKnown { mailbox: "Min".into(), ids: vec!["E2".into(), "E1".into()] }));
    answer(&asked, 1, the_end());
    app.poll_jmap();
    assert!(!app.syncing() && app.session.sync_status.starts_with("Up to date — "), "{}", app.session.sync_status);
}

#[test]
fn at_most_four_pieces_are_taken_in_a_frame() {
    let (mut app, id, asked) = window(true, false);
    app.execute("ui.sync", json!({"account": id})).unwrap();
    answer(&asked, 0, listing());
    for n in 1..=5 {
        answer(&asked, 0, more_of("Min", vec![mail(&format!("E{n}"), "Mail", 100 * n)], true));
    }
    answer(&asked, 0, more_of("Min", vec![mail("E6", "Mail", 600)], false));
    // Seven pieces are waiting: the listing and three windows are taken in, the rest wait.
    app.poll_jmap();
    assert_eq!(count(&app, id, "Min"), 3);
    assert!(app.syncing());
    // Three more: the last of them ends the run.
    app.poll_jmap();
    assert!(!app.syncing());
    assert_eq!(count(&app, id, "Min"), 6);
}

#[test]
fn cancel_stops_the_run_after_the_piece_in_hand() {
    let (mut app, id, asked) = window(true, true);
    assert!(app.execute("ui.cancelSync", json!({})).is_err(), "nothing to cancel");
    app.execute("ui.sync", json!({"account": id})).unwrap();
    answer(&asked, 0, listing());
    app.poll_jmap();
    let cancel = asked.lock().unwrap()[0].cancel.clone();
    assert!(!cancel.load(Ordering::Relaxed));
    let said = app.execute("ui.cancelSync", json!({})).unwrap();
    assert_eq!(said, json!({"cancelling": true}));
    // The host's job is told through the flag it was handed, and the window says it is stopping.
    assert!(cancel.load(Ordering::Relaxed));
    assert_eq!(app.session.sync_status, "Stopping…");
    assert!(app.syncing(), "the run ends with its last piece");
    // The host stopped after the piece in hand and says so: what it brought stays.
    answer(&asked, 0, JmapMailResult { stopped: true, ..window_of("Min", vec![mail("E1", "One", 100)], false) });
    app.poll_jmap();
    assert!(!app.syncing());
    assert!(app.session.sync_status.starts_with("Stopped; carries on from here next time"), "{}", app.session.sync_status);
    assert_eq!((count(&app, id, "Min"), state(&app, id)), (1, Some(RunState::Stopped)));
    assert!(app.execute("ui.cancelSync", json!({})).is_err());
    // The next run starts afresh: the flag is cleared for it.
    app.execute("ui.sync", json!({"account": id})).unwrap();
    assert!(!cancel.load(Ordering::Relaxed), "the same flag, cleared");
}

#[test]
fn a_run_on_a_thread_of_the_hosts_is_taken_in_as_it_goes_and_cancel_stops_it() {
    let (mut app, id, _) = window(true, true);
    // A host as the desktop's: its own thread, a plain channel, and nothing waits for the window.
    app.services.jmap = Some(Box::new(|job: JmapJob| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(listing());
            let _ = tx.send(window_of("Min", vec![mail("E1", "One", 100), mail("E2", "Two", 200)], true));
            let started = Instant::now();
            while !job.cancel.load(Ordering::Relaxed) && started.elapsed() < Duration::from_secs(60) {
                std::thread::sleep(Duration::from_millis(1));
            }
            let _ = tx.send(JmapMailResult { stopped: job.cancel.load(Ordering::Relaxed), ..piece(false) });
        });
        rx
    }));
    app.execute("ui.sync", json!({"account": id})).unwrap();
    until(&mut app, "the Inbox", |app| count(app, id, "Min") == 2);
    assert!(app.syncing(), "the host reads on");
    app.execute("ui.cancelSync", json!({})).unwrap();
    until(&mut app, "the end of the run", |app| !app.syncing());
    assert!(app.session.sync_status.starts_with("Stopped; carries on from here next time"), "{}", app.session.sync_status);
    assert_eq!(state(&app, id), Some(RunState::Stopped));
}

#[test]
fn a_run_that_starts_by_itself_goes_ahead_when_the_host_holds_the_credential() {
    let (mut app, id, asked) = window(true, true);
    app.auto_sync = true;
    app.session.prefs.sync_minutes = 5;
    assert!(app.can_sync_quietly(id) && app.server_accounts() == [id]);
    // The app starts: a round of every account, the first of them now.
    app.auto_sync();
    assert_eq!(jobs(&asked), 0, "the round is made up");
    app.auto_sync();
    assert_eq!(jobs(&asked), 1);
    assert!(app.syncing() && app.dialog.is_none());
    {
        let asked = asked.lock().unwrap();
        // Nobody asked: it asks for the share of requests background work has, and no secret is
        // sent; it is the first run, so it reads everything and lists the mailboxes.
        let request = &asked[0].request;
        assert_eq!((request.background, request.list_folders, request.scope.clone()), (true, true, JmapScope::All));
        assert_eq!(asked[0].secret, None);
    }
    // Nothing else starts while it is under way.
    app.last_auto_sync = 0.0;
    app.auto_sync();
    app.auto_sync();
    assert_eq!(jobs(&asked), 1);
    answer(&asked, 0, listing());
    answer(&asked, 0, window_of("Min", vec![mail("E1", "One", 100)], true));
    answer(&asked, 0, the_end());
    app.poll_jmap();
    assert!(!app.syncing() && count(&app, id, "Min") == 1);
    assert_eq!(state(&app, id), Some(RunState::Done));

    // A round some minutes later reads the Inbox and the folder on screen, and no more.
    app.session.view.folder = app.session.mailbox.folder_by_server(id, "Min").unwrap();
    app.last_auto_sync = 0.0;
    app.auto_sync();
    app.auto_sync();
    assert_eq!(jobs(&asked), 2);
    let request = asked.lock().unwrap()[1].request.clone();
    assert_eq!((request.background, request.list_folders, request.scope.clone()), (true, false, JmapScope::Light { also: vec![] }));
    answer(&asked, 1, the_end());
    app.poll_jmap();
    let projects = app.session.mailbox.folder_by_server(id, "Mpr").unwrap();
    app.session.view.folder = projects;
    app.last_auto_sync = 0.0;
    app.auto_sync();
    app.auto_sync();
    assert_eq!(jobs(&asked), 3);
    let request = asked.lock().unwrap()[2].request.clone();
    assert_eq!(request.scope, JmapScope::Light { also: vec!["Mpr".into()] });
    // The owner's own Send/Receive reads everything, whatever is on screen.
    answer(&asked, 2, the_end());
    app.poll_jmap();
    app.execute("ui.sync", json!({})).unwrap();
    let request = asked.lock().unwrap()[3].request.clone();
    assert_eq!((request.background, request.scope.clone()), (false, JmapScope::All));
}

#[test]
fn without_the_host_holding_the_credential_a_password_is_asked_for_once_and_nothing_starts_by_itself_without_one() {
    let (mut app, id, asked) = window(false, false);
    let store = Store::default();
    app.services.secrets = Some(Box::new(store.clone()));
    app.auto_sync = true;
    app.session.prefs.sync_minutes = 5;
    // Nothing is known: a round that starts by itself leaves the account out and asks nobody.
    assert!(!app.can_sync_quietly(id));
    app.auto_sync();
    app.auto_sync();
    assert_eq!(jobs(&asked), 0);
    assert!(app.dialog.is_none() && app.session.sync_progress.accounts.is_empty());
    // A run nobody asked for, named by a script, asks nobody either.
    let refused = app.execute("ui.sync", json!({"account": id, "quiet": true})).unwrap_err();
    assert!(refused.starts_with("Use Send/Receive to sign in to") && refused.ends_with(ME), "{refused}");
    assert!(app.dialog.is_none() && jobs(&asked) == 0);

    // The owner's Send/Receive: the password dialog, as for an IMAP account.
    let said = app.execute("ui.sync", json!({"account": id})).unwrap();
    assert_eq!(said, json!({"pending": "password"}));
    assert_eq!(app.dialog.as_ref().map(crate::dialogs::Dialog::name), Some("password"));
    assert_eq!(jobs(&asked), 0);
    // Typed, it goes with the job: in the job alone, and the system keeps it once it has worked.
    app.dialog = None;
    let said = app.execute("ui.sync", json!({"account": id, "password": "pw-one"})).unwrap();
    assert_eq!(said["started"], true);
    {
        let asked = asked.lock().unwrap();
        assert_eq!((asked[0].secret.as_deref(), asked[0].request.bearer), (Some("pw-one"), false));
        assert!(!asked[0].shown.contains("pw-one") && !asked[0].shown.contains('@'), "{}", asked[0].shown);
    }
    assert_eq!(store.kept(PASSWORD_KEY), None, "not before it has worked");
    answer(&asked, 0, listing());
    answer(&asked, 0, the_end());
    app.poll_jmap();
    assert!(!app.syncing());
    assert_eq!(store.kept(PASSWORD_KEY).as_deref(), Some("pw-one"));
    // It is at hand now: no dialog, and a run by itself can go ahead.
    assert!(app.can_sync_quietly(id));
    app.execute("ui.sync", json!({"account": id})).unwrap();
    assert!(app.dialog.is_none());
    assert_eq!(asked.lock().unwrap()[1].secret.as_deref(), Some("pw-one"));
    // A password never reaches the mailbox, the settings or a command's answer.
    let written = format!("{}{}", serde_json::to_string(&app.prefs()).unwrap(), serde_json::to_string(&app.session.mailbox).unwrap());
    assert!(!written.contains("pw-one"));

    // The next start: the system has it, and nobody is asked.
    let (mut next, id, asked) = window(false, false);
    next.services.secrets = Some(Box::new(store.clone()));
    assert!(next.can_sync_quietly(id));
    next.execute("ui.sync", json!({"account": id})).unwrap();
    assert!(next.dialog.is_none());
    assert_eq!(asked.lock().unwrap()[0].secret.as_deref(), Some("pw-one"));
}

#[test]
fn a_bearer_token_is_in_memory_only() {
    let (mut app, id, asked) = window(false, true);
    let store = Store::default();
    store.0.lock().unwrap().insert(PASSWORD_KEY.into(), "old password".into());
    app.services.secrets = Some(Box::new(store.clone()));
    // What the system has is a password, not this account's token: it is not used for it.
    assert!(!app.can_sync_quietly(id));
    assert_eq!(app.execute("ui.sync", json!({"account": id})).unwrap(), json!({"pending": "password"}));
    app.dialog = None;
    app.execute("ui.sync", json!({"account": id, "password": "token-one"})).unwrap();
    {
        let asked = asked.lock().unwrap();
        assert_eq!((asked[0].secret.as_deref(), asked[0].request.bearer), (Some("token-one"), true));
    }
    answer(&asked, 0, the_end());
    app.poll_jmap();
    assert!(!app.syncing() && app.can_sync_quietly(id), "good for this session");
    assert_eq!(store.kept(PASSWORD_KEY).as_deref(), Some("old password"), "the token is not kept");
    assert!(!store.0.lock().unwrap().values().any(|secret| secret.contains("token-one")));
}

#[test]
fn a_sender_that_is_dropped_ends_the_run_as_stopped_unexpectedly() {
    let (mut app, id, asked) = window(true, true);
    // Nothing came, and the host's task is gone.
    app.execute("ui.sync", json!({"account": id})).unwrap();
    asked.lock().unwrap().clear();
    app.poll_jmap();
    assert!(!app.syncing() && app.jmap_rx.is_none());
    assert_eq!(app.session.sync_status, "Send/Receive stopped unexpectedly");
    assert!(app.has_sync_problems());
    // It can be started again.
    assert!(app.enabled("ui.sync"));

    // Some of it came: that stays, and the run is over without having finished.
    app.execute("ui.sync", json!({"account": id})).unwrap();
    answer(&asked, 0, listing());
    answer(&asked, 0, window_of("Min", vec![mail("E1", "One", 100)], true));
    let _ = asked.lock().unwrap().pop();
    app.poll_jmap();
    assert!(!app.syncing());
    assert_eq!(app.session.sync_status, "Send/Receive stopped unexpectedly");
    assert_eq!(count(&app, id, "Min"), 1);
    assert_eq!(state(&app, id), Some(RunState::Stopped));
    // And the next run goes through.
    app.execute("ui.sync", json!({"account": id})).unwrap();
    answer(&asked, 0, the_end());
    app.poll_jmap();
    assert!(app.session.sync_status.starts_with("Up to date — "), "{}", app.session.sync_status);
    assert!(!app.has_sync_problems(), "a run that goes through clears what the one before said");
}

#[test]
fn a_server_that_turns_the_secret_down_ends_the_run_with_a_plain_line_and_no_run_starts_by_itself() {
    let (mut app, id, asked) = window(true, true);
    app.auto_sync = true;
    app.session.prefs.sync_minutes = 5;
    app.execute("ui.sync", json!({"account": id})).unwrap();
    answer(&asked, 0, listing());
    answer(&asked, 0, JmapMailResult { auth_refused: true, ..piece(false) });
    app.poll_jmap();
    assert!(!app.syncing());
    assert_eq!(app.session.sync_status, "The server refused the sign-in. Sign in again, then use Send/Receive.");
    assert!(app.has_sync_problems() && app.jmap_refused.contains(&id));
    assert_eq!(state(&app, id), Some(RunState::Done));
    // Asking again would only be refused again: rounds that start by themselves leave it alone.
    assert!(!app.can_sync_quietly(id));
    for _ in 0..4 {
        app.last_auto_sync = 0.0;
        app.auto_sync();
    }
    assert_eq!(jobs(&asked), 1);
    assert!(app.dialog.is_none());
    // The owner's Send/Receive tries again; a run that goes through lets the others start.
    app.execute("ui.sync", json!({"account": id})).unwrap();
    assert_eq!(jobs(&asked), 2);
    assert!(app.jmap_refused.is_empty());
    answer(&asked, 1, the_end());
    app.poll_jmap();
    assert!(app.can_sync_quietly(id) && app.session.sync_status.starts_with("Up to date — "), "{}", app.session.sync_status);
    app.last_auto_sync = 0.0;
    app.auto_sync();
    app.auto_sync();
    assert_eq!(jobs(&asked), 3);

    // Said in a piece that isn't the last, the refusal still counts.
    answer(&asked, 2, JmapMailResult { auth_refused: true, ..piece(true) });
    answer(&asked, 2, the_end());
    app.poll_jmap();
    assert!(!app.syncing() && !app.can_sync_quietly(id));
    assert!(app.session.sync_status.starts_with("The server refused the sign-in."), "{}", app.session.sync_status);
}

#[test]
fn a_password_the_server_refuses_is_forgotten_so_that_the_next_try_asks_again() {
    let (mut app, id, asked) = window(false, false);
    let store = Store::default();
    app.services.secrets = Some(Box::new(store.clone()));
    app.execute("ui.sync", json!({"account": id, "password": "pw-one"})).unwrap();
    answer(&asked, 0, the_end());
    app.poll_jmap();
    assert_eq!(store.kept(PASSWORD_KEY).as_deref(), Some("pw-one"));
    // The next time the server turns it down (it was changed there).
    app.execute("ui.sync", json!({"account": id})).unwrap();
    assert_eq!(asked.lock().unwrap()[1].secret.as_deref(), Some("pw-one"));
    answer(&asked, 1, JmapMailResult { auth_refused: true, ..piece(false) });
    app.poll_jmap();
    assert_eq!(app.session.sync_status, "The server refused the sign-in. Use Send/Receive to enter it again.");
    assert!(app.passwords.is_empty() && store.kept(PASSWORD_KEY).is_none());
    assert!(!app.can_sync_quietly(id));
    // The owner is asked again.
    assert_eq!(app.execute("ui.sync", json!({"account": id})).unwrap(), json!({"pending": "password"}));
    assert_eq!(jobs(&asked), 2);
}

#[test]
fn what_a_host_sends_wrongly_is_said_and_never_breaks_the_run() {
    let (mut app, id, asked) = window(true, true);
    app.execute("ui.sync", json!({"account": id})).unwrap();
    // Pieces that name another account, or none that exists, are for the run's account all the
    // same; a piece for another address is refused, and said.
    answer(&asked, 0, JmapMailResult { account: 9_999, ..listing() });
    answer(&asked, 0, JmapMailResult { account: 4, email: "eve@example.org".into(), ..window_of("Min", vec![mail("E1", "Stranger", 100)], true) });
    // A window for a mailbox nobody has, and error lines without end: what is kept is bounded.
    let noise = |n: usize| JmapMailResult {
        errors: (0..n).map(|k| format!("problem {k}")).collect(),
        ..window_of("Mnope", vec![mail("E2", "Lost", 200)], true)
    };
    answer(&asked, 0, noise(500));
    answer(&asked, 0, noise(500));
    app.poll_jmap();
    assert!(app.syncing());
    assert!(app.session.mailbox.folder_by_server(id, "Min").is_some(), "the first piece is the run's");
    assert_eq!(count(&app, id, "Min"), 0, "the stranger's is refused");
    assert!(app.jmap_problems.iter().any(|p| p.contains("another account")), "{:?}", app.jmap_problems);
    for _ in 0..5 {
        answer(&asked, 0, noise(500));
    }
    app.poll_jmap();
    app.poll_jmap();
    assert!(app.jmap_problems.len() <= super::MOST_PROBLEMS, "{}", app.jmap_problems.len());
    // The run ends all the same, and says its first problem.
    answer(&asked, 0, the_end());
    app.poll_jmap();
    assert!(!app.syncing());
    assert!(app.session.sync_status.contains("another account"), "{}", app.session.sync_status);
    assert!(app.sync_problem_lines().len() <= 61, "{}", app.sync_problem_lines().len());
    // Pieces that come after the last are never read.
    let pieces = asked.lock().unwrap()[0].to.send(listing());
    assert!(pieces.is_err(), "nobody listens any more");
}

#[test]
fn what_the_job_prints_is_counts_only() {
    let request = JmapMailRequest { account: 3, email: ME.into(), session: SESSION.into(), bearer: true, window: 300, ..Default::default() };
    let job = JmapJob { request, secret: Some("s3cret-token".into()), cancel: Arc::new(AtomicBool::new(false)) };
    for shown in [format!("{job:?}"), format!("{job:#?}")] {
        for hidden in ["s3cret-token", "@", "example.org", "jmap/session", ME] {
            assert!(!shown.contains(hidden), "{hidden} in {shown}");
        }
        assert!(shown.contains("secret: true") && shown.contains("cancel: false"), "{shown}");
    }
    let without = JmapJob { secret: None, ..job };
    assert!(format!("{without:?}").contains("secret: false"));
}

#[test]
fn compose_send_queues_the_message_of_a_jmap_account_like_any_other_and_nothing_says_it_cannot() {
    let (mut app, id, _) = window(true, true);
    app.execute("mail.new", json!({"to": "bob@example.org", "subject": "Hello", "body": "Hi", "account": id})).unwrap();
    let outbox = app.session.mailbox.folder_of_kind(id, FolderKind::Outbox).unwrap();
    let sent = app.session.mailbox.folder_of_kind(id, FolderKind::Sent).unwrap();
    let said = app.run("compose.send", json!({})).unwrap();
    assert_eq!(said["queued"], true);
    // The message waits in the Outbox for the next Send/Receive; the interim refusal is gone.
    assert!(app.session.compose.is_none());
    assert_eq!((app.session.mailbox.messages_in(outbox).count(), app.session.mailbox.messages_in(sent).count()), (1, 0));
    let shown = app.status_msg.as_ref().map(|(text, _)| text.clone()).unwrap_or_default();
    assert_ne!(shown, "Sending isn't available for this account yet");
    // The account has something to send, so a run starts for it.
    assert_eq!(app.waiting_to_go(id), 1);
    // Save Draft works as it did: the draft is kept here, and waits to be filed on the server.
    app.execute("mail.new", json!({"to": "bob@example.org", "subject": "Later", "account": id})).unwrap();
    assert!(app.run("compose.saveDraft", json!({})).is_ok());
    assert_eq!(app.waiting_to_go(id), 2);
}

#[test]
fn a_build_decides_which_accounts_it_can_run_by_what_its_host_gives() {
    // A host that has a JMAP runner and no mail connections (the browser): a JMAP account runs.
    let (mut app, jmap, asked) = window(true, true);
    let imap =
        app.execute("account.add", json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    assert!(app.services.can_sync() && app.services.sync.is_none());
    assert_eq!(app.server_accounts(), [jmap], "an IMAP account is not one it can run");
    assert!(app.enabled("ui.sync"));
    // The Send/Receive tab doesn't say this build can't connect.
    app.ui.tab = "Send / Receive".into();
    assert_eq!(crate::ribbon::describe(&app).note, "");
    // The IMAP account is refused in plain words; the JMAP account is not.
    let e = app.execute("ui.sync", json!({"account": imap})).unwrap_err();
    assert!(e.starts_with("This build can't connect to mail servers"), "{e}");
    assert_eq!(jobs(&asked), 0);
    // A round of every account runs the one it can.
    assert_eq!(app.execute("ui.sync", json!({})).unwrap()["jmap"], true);
    assert_eq!(jobs(&asked), 1);
    answer(&asked, 0, the_end());
    app.poll_jmap();

    // A host with mail connections and no JMAP runner (the desktop): the other way about.
    app.services.jmap = None;
    app.services.sync = Some(Box::new(|req: mailcraft_model::sync::SyncRequest| {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(mailcraft_model::sync::SyncResult { account: req.account.id, ..Default::default() });
        rx
    }));
    assert!(app.services.can_sync());
    assert_eq!(app.server_accounts(), [imap]);
    let e = app.execute("ui.sync", json!({"account": jmap})).unwrap_err();
    assert_eq!(e, "This build can't connect to JMAP servers");
    app.passwords.insert(imap, "pw".into());
    assert_eq!(app.execute("ui.sync", json!({})).unwrap()["account"], imap);

    // A host with neither: as before.
    let (mut none, id, _) = window(false, false);
    none.services.jmap = None;
    assert!(!none.services.can_sync() && !none.enabled("ui.sync") && none.server_accounts().is_empty());
    none.ui.tab = "Send / Receive".into();
    assert!(crate::ribbon::describe(&none).note.starts_with("This build can't connect to mail servers"));
    let e = none.execute("ui.sync", json!({"account": id})).unwrap_err();
    assert!(e.starts_with("This build can't connect to mail servers"), "{e}");
    assert!(none.execute("ui.sync", json!({})).unwrap_err().starts_with("This build can't connect to mail servers"));
}

#[test]
fn a_jmap_account_with_no_usable_server_address_says_so_and_asks_nobody() {
    let (mut app, id, asked) = window(true, true);
    app.session.mailbox.account_mut(id).unwrap().jmap = None;
    let e = app.execute("ui.sync", json!({"account": id})).unwrap_err();
    assert!(e.contains("has no JMAP server"), "{e}");
    assert!(!app.syncing() && jobs(&asked) == 0 && app.dialog.is_none());
    // An account of another kind is not one for it.
    let local = app.execute("account.add", json!({"email": "cy@example.org"})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(app.session.mailbox.account(local).map(|a| a.kind), Some(AccountKind::Local));
    assert!(app.start_jmap_mail(local, false).is_err());
    assert!(app.start_jmap_mail(9_999, false).is_err());
}

/// A window whose account has had its first run: the server's mailboxes are here, with mail in the Inbox and in
/// Projects, and the rounds that start by themselves are off, so that only what waits to go up starts a run.
fn settled() -> (MailApp, Id, Asked) {
    let (mut app, id, asked) = window(true, true);
    app.auto_sync = true;
    app.session.prefs.sync_minutes = 0;
    app.execute("ui.sync", json!({"account": id})).unwrap();
    answer(&asked, 0, listing());
    answer(&asked, 0, window_of("Min", vec![mail("E1", "One", 100), mail("E2", "Two", 200)], true));
    answer(&asked, 0, window_of("Mpr", vec![mail("E3", "Three", 300)], true));
    answer(&asked, 0, the_end());
    app.poll_jmap();
    assert!(!app.syncing() && count(&app, id, "Min") == 2);
    (app, id, asked)
}

/// The message of `account` the server calls `server`.
fn local(app: &MailApp, account: Id, server: &str) -> Id {
    app.session.mailbox.messages.values().find(|m| m.account == account && m.server.as_ref().is_some_and(|s| s.id == server)).unwrap().id
}

/// What the window does every moment: look at what waits to go up (at once, not once in a second and a half), and
/// start a run for it.
fn notice(app: &mut MailApp) {
    app.push_checked = 0.0;
    app.auto_sync();
    app.auto_sync();
}

fn marks_read(id: &str, message: Id) -> JmapChange {
    JmapChange::Keywords { message, id: id.into(), read: Some(true), flagged: None, answered: None, forwarded: None }
}

#[test]
fn a_change_made_here_starts_a_quick_run_that_carries_it_and_says_what_it_is_for() {
    let (mut app, id, asked) = settled();
    assert_eq!((jobs(&asked), app.waiting_to_go(id)), (1, 0));
    // Nothing waits: nothing starts.
    notice(&mut app);
    assert_eq!(jobs(&asked), 1);

    let one = local(&app, id, "E1");
    app.execute("mail.markRead", json!({"ids": [one]})).unwrap();
    assert_eq!(app.waiting_to_go(id), 1);
    notice(&mut app);
    assert_eq!(jobs(&asked), 2);
    assert!(app.syncing() && app.push_only_next.is_empty());
    assert_eq!(app.session.sync_status, "Sending your changes…");
    {
        let asked = asked.lock().unwrap();
        let request = &asked[1].request;
        // Nobody waits for it: the Inbox and the folder on screen, with what the owner changed.
        assert_eq!((request.background, request.list_folders, request.scope.clone()), (true, false, JmapScope::Light { also: vec![] }));
        assert_eq!(request.changes, [marks_read("E1", one)]);
        assert_eq!(asked[1].secret, None);
        assert!(!asked[1].shown.contains("E1") && !asked[1].shown.contains('@'), "{}", asked[1].shown);
    }
    // The host says what became of it first, and then what it read.
    answer(
        &asked,
        1,
        JmapMailResult {
            changes: vec![JmapChangeDone { change: marks_read("E1", one), outcome: JmapOutcome::Done { id: String::new() } }],
            ..piece(true)
        },
    );
    app.poll_jmap();
    assert!(app.syncing() && app.session.mailbox.message(one).is_some_and(|m| m.server.as_ref().is_some_and(|s| s.read)));
    assert_eq!(app.waiting_to_go(id), 0);
    answer(&asked, 1, the_end());
    app.poll_jmap();
    assert!(!app.syncing() && app.session.sync_status.starts_with("Up to date — "), "{}", app.session.sync_status);
    assert_eq!(app.push_seen.get(&id).map(|(waiting, _)| *waiting), Some(0));
    assert!(!app.has_sync_problems());
    // Nothing waits any more: nothing starts again.
    notice(&mut app);
    assert_eq!(jobs(&asked), 2);
}

#[test]
fn a_run_for_the_changes_reads_the_folders_they_move_messages_into() {
    let (mut app, id, asked) = settled();
    let two = local(&app, id, "E2");
    // Delete is a move into Deleted Items, which the server has.
    app.execute("mail.delete", json!({"ids": [two]})).unwrap();
    notice(&mut app);
    assert_eq!(jobs(&asked), 2);
    let request = asked.lock().unwrap()[1].request.clone();
    assert!(request.changes.iter().any(|c| matches!(c, JmapChange::Move { to, .. } if to == "Mtr")));
    // The message is found again there, in the same run; the Inbox is the host's own.
    assert_eq!(request.scope, JmapScope::Light { also: vec!["Mtr".into()] });
    // The host got nowhere with it (the server did not answer): the move still waits, and another change brings the next
    // run, which reads the folder on screen first and the folder the message goes to after.
    answer(&asked, 1, JmapMailResult { errors: vec!["The server did not answer".into()], ..piece(false) });
    app.poll_jmap();
    let projects = app.session.mailbox.folder_by_server(id, "Mpr").unwrap();
    app.session.view.folder = projects;
    app.execute("mail.markRead", json!({"ids": [local(&app, id, "E3")]})).unwrap();
    notice(&mut app);
    assert_eq!(jobs(&asked), 3);
    let request = asked.lock().unwrap()[2].request.clone();
    assert_eq!(request.scope, JmapScope::Light { also: vec!["Mpr".into(), "Mtr".into()] });
    assert_eq!(request.changes.len(), 2);
}

#[test]
fn a_change_the_server_refused_is_put_back_said_once_and_does_not_start_another_run() {
    let (mut app, id, asked) = settled();
    let one = local(&app, id, "E1");
    app.execute("mail.markRead", json!({"ids": [one]})).unwrap();
    notice(&mut app);
    let change = marks_read("E1", one);
    answer(
        &asked,
        1,
        JmapMailResult { changes: vec![JmapChangeDone { change, outcome: JmapOutcome::Refused { why: "forbidden".into() } }], ..piece(true) },
    );
    answer(&asked, 1, the_end());
    app.poll_jmap();
    let line = "Couldn't mark 'One' as read: the server says this account may not change it.";
    assert!(!app.syncing() && app.session.sync_status.starts_with(line), "{}", app.session.sync_status);
    assert!(app.has_sync_problems() && app.sync_problem_lines().iter().any(|l| l.contains(line)), "{:?}", app.sync_problem_lines());
    // Put back, and no longer waiting: no run starts for it, now or later.
    assert!(!app.session.mailbox.message(one).unwrap().read);
    assert_eq!(app.waiting_to_go(id), 0);
    notice(&mut app);
    app.push_seen.insert(id, (0, 0.0));
    notice(&mut app);
    assert_eq!(jobs(&asked), 2);
    // The next run does not say it again.
    app.execute("ui.sync", json!({"account": id})).unwrap();
    answer(&asked, 2, the_end());
    app.poll_jmap();
    assert!(app.session.sync_status.starts_with("Up to date — "), "{}", app.session.sync_status);
}

#[test]
fn a_change_the_server_did_not_take_waits_and_is_tried_again_when_there_is_more_or_in_a_while() {
    let (mut app, id, asked) = settled();
    let (one, two) = (local(&app, id, "E1"), local(&app, id, "E2"));
    app.execute("mail.markRead", json!({"ids": [one]})).unwrap();
    notice(&mut app);
    assert_eq!(jobs(&asked), 2);
    // The server did not answer: nothing was answered, the run ended and says why.
    answer(&asked, 1, JmapMailResult { errors: vec!["Couldn't send 1 change to the server: the server did not answer".into()], ..piece(false) });
    app.poll_jmap();
    assert!(!app.syncing());
    assert!(app.session.sync_status.starts_with("Couldn't send 1 change to the server"), "{}", app.session.sync_status);
    assert_eq!(app.waiting_to_go(id), 1, "it is still to go");
    // Not at once again: it would fail again.
    notice(&mut app);
    assert_eq!(jobs(&asked), 2);
    // One more change: a run, with both.
    app.execute("mail.markRead", json!({"ids": [two]})).unwrap();
    notice(&mut app);
    assert_eq!(jobs(&asked), 3);
    assert_eq!(asked.lock().unwrap()[2].request.changes.len(), 2);
    answer(&asked, 2, JmapMailResult { errors: vec!["Couldn't send 2 changes to the server: the server did not answer".into()], ..piece(false) });
    app.poll_jmap();
    // A few minutes later it is tried again for the same two.
    notice(&mut app);
    assert_eq!(jobs(&asked), 3);
    let (waiting, _) = app.push_seen.get(&id).copied().unwrap();
    app.push_seen.insert(id, (waiting, crate::now_ms() - 301_000.0));
    notice(&mut app);
    assert_eq!(jobs(&asked), 4);
    assert_eq!(asked.lock().unwrap()[3].request.changes.len(), 2);
    answer(
        &asked,
        3,
        JmapMailResult {
            changes: [(&"E1", one), (&"E2", two)]
                .iter()
                .map(|(server, message)| JmapChangeDone { change: marks_read(server, *message), outcome: JmapOutcome::Done { id: String::new() } })
                .collect(),
            ..piece(true)
        },
    );
    answer(&asked, 3, the_end());
    app.poll_jmap();
    assert!(app.session.sync_status.starts_with("Up to date — ") && app.waiting_to_go(id) == 0, "{}", app.session.sync_status);
}

#[test]
fn the_owners_own_send_receive_carries_the_changes_too_and_says_it_is_sending_and_receiving() {
    let (mut app, id, asked) = settled();
    let one = local(&app, id, "E1");
    app.execute("mail.markRead", json!({"ids": [one]})).unwrap();
    app.execute("ui.sync", json!({"account": id})).unwrap();
    assert_eq!(app.session.sync_status, "Sending and receiving…");
    {
        let asked = asked.lock().unwrap();
        let request = &asked[1].request;
        assert_eq!((request.scope.clone(), request.list_folders, request.background), (JmapScope::All, true, false));
        assert_eq!(request.changes, [marks_read("E1", one)]);
    }
    answer(&asked, 1, the_end());
    app.poll_jmap();
    assert!(app.session.sync_status.starts_with("Up to date — "), "{}", app.session.sync_status);
}

#[test]
fn a_run_by_itself_that_is_not_for_changes_does_not_say_it_is_sending_them() {
    let (mut app, id, asked) = settled();
    app.session.prefs.sync_minutes = 5;
    app.last_auto_sync = 0.0;
    app.auto_sync();
    app.auto_sync();
    assert_eq!(jobs(&asked), 2);
    assert_eq!(app.session.sync_status, "Sending and receiving…");
    assert!(asked.lock().unwrap()[1].request.changes.is_empty());
    answer(&asked, 1, the_end());
    app.poll_jmap();
    assert!(app.session.sync_status.starts_with("Up to date — ") && app.waiting_to_go(id) == 0);
}

#[test]
fn a_change_made_while_a_run_is_under_way_is_sent_as_soon_as_it_ends() {
    let (mut app, id, asked) = settled();
    let (one, two) = (local(&app, id, "E1"), local(&app, id, "E2"));
    app.execute("mail.markRead", json!({"ids": [one]})).unwrap();
    notice(&mut app);
    assert_eq!(jobs(&asked), 2);
    // While it is under way the owner reads another message, which the run was not told of.
    app.execute("mail.markRead", json!({"ids": [two]})).unwrap();
    notice(&mut app);
    assert_eq!(jobs(&asked), 2, "one run at a time");
    // The server did not take the first change. The second starts the next run, which carries both.
    answer(&asked, 1, JmapMailResult { errors: vec!["Couldn't send 1 change to the server: the server did not take them".into()], ..piece(false) });
    app.poll_jmap();
    assert!(!app.syncing() && app.waiting_to_go(id) == 2);
    notice(&mut app);
    assert_eq!(jobs(&asked), 3, "not five minutes later");
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked[2].request.changes, [marks_read("E1", one), marks_read("E2", two)]);
    }
    // Told of both and failed again: nothing new, so it waits as a change the server did not take does.
    answer(&asked, 2, JmapMailResult { errors: vec!["Couldn't send 2 changes to the server: the server did not answer".into()], ..piece(false) });
    app.poll_jmap();
    notice(&mut app);
    assert_eq!(jobs(&asked), 3);
}

#[test]
fn a_server_that_asked_to_be_left_alone_is_not_asked_for_what_is_made_meanwhile_until_it_said() {
    let (mut app, id, asked) = settled();
    let (one, two) = (local(&app, id, "E1"), local(&app, id, "E2"));
    app.execute("mail.markRead", json!({"ids": [one]})).unwrap();
    notice(&mut app);
    assert_eq!(jobs(&asked), 2);
    answer(
        &asked,
        1,
        JmapMailResult { errors: vec!["Couldn't send 1 change to the server: the server is busy".into()], busy_for: Some(30), ..piece(false) },
    );
    app.poll_jmap();
    assert!(!app.syncing());
    // More is made, and the server asked for half a minute: nothing starts.
    app.execute("mail.markRead", json!({"ids": [two]})).unwrap();
    assert_eq!(app.waiting_to_go(id), 2);
    notice(&mut app);
    assert_eq!(jobs(&asked), 2);
    // Half a minute later it does, and carries both.
    app.jmap_calm.insert(id, 0.0);
    notice(&mut app);
    assert_eq!(jobs(&asked), 3);
    assert_eq!(asked.lock().unwrap()[2].request.changes, [marks_read("E1", one), marks_read("E2", two)]);
    // A run that failed in another way says nothing of the server being well again; one that went through does, and
    // a server's wish is never kept longer than a quarter of an hour.
    app.jmap_calm.insert(id, crate::now_ms() + 20_000.0);
    answer(&asked, 2, JmapMailResult { errors: vec!["The server did not answer".into()], ..piece(false) });
    app.poll_jmap();
    assert!(app.jmap_calm.contains_key(&id));
    app.jmap_calm.insert(id, 0.0);
    app.execute("ui.sync", json!({"account": id})).unwrap();
    assert_eq!(jobs(&asked), 4);
    answer(&asked, 3, the_end());
    app.poll_jmap();
    assert!(!app.jmap_calm.contains_key(&id));
    app.execute("mail.markUnread", json!({"ids": [one]})).unwrap();
    notice(&mut app);
    assert_eq!(jobs(&asked), 5);
    answer(&asked, 4, JmapMailResult { errors: vec!["The server is busy".into()], busy_for: Some(4_000_000), ..piece(false) });
    app.poll_jmap();
    let waits = app.jmap_calm.get(&id).copied().unwrap_or(0.0) - crate::now_ms();
    assert!(waits > 800_000.0 && waits <= 900_000.0, "{waits}");
    // The round every few minutes leaves such a server out too, until it is over.
    app.session.prefs.sync_minutes = 5;
    app.last_auto_sync = 0.0;
    app.auto_sync();
    app.auto_sync();
    assert_eq!(jobs(&asked), 5, "{:?}", app.sync_queue);
    app.jmap_calm.insert(id, 0.0);
    app.last_auto_sync = 0.0;
    app.auto_sync();
    app.auto_sync();
    assert_eq!(jobs(&asked), 6);
}

/// Deleting a JMAP account's message for good destroys it on the server too, where the window runs the account's
/// Send/Receive: the question says so. A window that can't reach a JMAP server says nothing of the server.
#[test]
fn deleting_a_jmap_message_for_good_says_it_goes_from_the_server_too_where_the_window_runs_it() {
    let (mut app, id, _) = window(true, true);
    let deleted = app.session.mailbox.folder_of_kind(id, FolderKind::Deleted).unwrap();
    let old = mailcraft_model::Message { folder: deleted, account: id, subject: "Old plans".into(), ..Default::default() };
    let message = app.session.mailbox.add_message(old).unwrap();
    let asked = |app: &MailApp| crate::dialogs::confirmation(app, "mail.deletePermanently", &json!({"id": message})).map(|ask| ask.text);
    assert_eq!(
        asked(&app).as_deref(),
        Some("This message will be deleted for good. It is removed from the mail server too, and that can't be undone.")
    );
    app.services.jmap = None;
    assert_eq!(asked(&app).as_deref(), Some("This message will be deleted for good."));
}

/// What the engine says of a JMAP account's mail follows the window's runners from the start, before any frame or command.
#[test]
fn a_window_with_a_jmap_runner_tells_the_engine_so_from_the_start() {
    let services = Services { jmap: Some(Box::new(|_: JmapJob| mpsc::channel().1)), ..Default::default() };
    let app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), services);
    assert!(app.session.jmap_runs);
    let app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    assert!(!app.session.jmap_runs);
}
