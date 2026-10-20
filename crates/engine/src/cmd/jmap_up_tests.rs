// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Changes of messages of a JMAP account, as the engine works them out and takes in what became of them with
//! results made by hand. (Against the in-process JMAP server itself: `apps/mailcraft-cli/tests/jmap_up.rs`.)

use mailcraft_model::sync::{GraphMessage, JmapMailResult, JmapScope, MessagePage};
use mailcraft_model::{Account, AccountKind, Folder, FolderKind, ServerFolder};
use serde_json::{Value, json};

use super::*;
use crate::cmd::jmap_mail::{apply_jmap_mail, jmap_mail_request, unpushed};

/// Friday 2027-01-15 08:00 UTC.
const NOW: i64 = 1_800_000_000;
const ME: &str = "owner@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session";

/// A session with a JMAP account whose Inbox, Archive, Deleted Items and Junk E-mail the server has, a message
/// in its Inbox the server has said things of, and a local account with a folder: the session, the account, the
/// message and the local account's folder.
fn setup() -> (Session, Id, Id, Id) {
    let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
    // A host that runs a JMAP account's Send/Receive (this fork's browser build).
    s.jmap_runs = true;
    let account =
        s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap()["id"].as_u64().unwrap();
    for (kind, id) in [(FolderKind::Inbox, "Min"), (FolderKind::Archive, "Mar"), (FolderKind::Deleted, "Mtr"), (FolderKind::Junk, "Mju")] {
        let folder = match s.mailbox.folder_of_kind(account, kind) {
            Some(folder) => folder,
            None => {
                let folder = s.mailbox.alloc_id();
                s.mailbox.folders.push(Folder { id: folder, account, name: kind.default_name().into(), kind, ..Default::default() });
                folder
            }
        };
        s.mailbox.folder_mut(folder).unwrap().server = Some(ServerFolder { id: id.into(), name: kind.default_name().into(), ..Default::default() });
    }
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let m = add(&mut s, inbox, "E1", "Invoice 42");
    let local = s.mailbox.add_account(Account { name: "On this computer".into(), ..Default::default() });
    let kept = s.mailbox.add_folder(local, None, "Kept").unwrap();
    (s, account, m, kept)
}

/// A message the server has, in `folder` (which it has), under the id `id`.
fn add(s: &mut Session, folder: Id, id: &str, subject: &str) -> Id {
    let at = s.mailbox.folder(folder).and_then(|f| f.server.as_ref()).map(|server| server.id.clone()).unwrap_or_default();
    let server = ServerMessage { id: id.into(), folder: at, ..Default::default() };
    let message_id = format!("{}@example.org", subject.replace(' ', "-").to_lowercase());
    s.mailbox.add_message(Message { folder, subject: subject.into(), message_id, server: Some(server), ..Default::default() }).unwrap()
}

fn changes(s: &Session, account: Id) -> Vec<JmapChange> {
    message_changes(s, account, 100).0
}

fn keywords(message: Id, id: &str, read: Option<bool>, flagged: Option<bool>) -> JmapChange {
    JmapChange::Keywords { message, id: id.into(), read, flagged, answered: None, forwarded: None }
}

fn result(account: Id, changes: Vec<JmapChangeDone>) -> JmapMailResult {
    JmapMailResult { account, email: ME.into(), changes, ..Default::default() }
}

fn done(change: JmapChange, outcome: JmapOutcome) -> JmapChangeDone {
    JmapChangeDone { change, outcome }
}

fn ok(id: &str) -> JmapOutcome {
    JmapOutcome::Done { id: id.into() }
}

fn refused(why: &str) -> JmapOutcome {
    JmapOutcome::Refused { why: why.into() }
}

fn folder_of(s: &Session, account: Id, server: &str) -> Id {
    s.mailbox.folder_by_server(account, server).unwrap()
}

#[test]
fn nothing_changed_here_sends_nothing() {
    let (s, account, _, _) = setup();
    assert_eq!(changes(&s, account), []);
    assert_eq!(unpushed(&s, account), 0);
    let request = jmap_mail_request(&s, account, false, &[], false).unwrap();
    assert!(request.changes.is_empty());
}

#[test]
fn the_marks_that_differ_go_as_one_change_naming_only_those() {
    let (mut s, account, m, _) = setup();
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    assert_eq!(changes(&s, account), [keywords(m, "E1", Some(true), None)]);
    s.run("mail.flag", &json!({"ids": [m], "due": "2027-01-20"})).unwrap();
    // The follow-up flag's due date is no business of a JMAP server's: the flag is.
    assert_eq!(changes(&s, account), [keywords(m, "E1", Some(true), Some(true))]);
    {
        let m = s.mailbox.message_mut(m).unwrap();
        (m.answered, m.forwarded) = (true, true);
    }
    assert_eq!(
        changes(&s, account),
        [JmapChange::Keywords { message: m, id: "E1".into(), read: Some(true), flagged: Some(true), answered: Some(true), forwarded: Some(true) }]
    );
    // Back to what the server has: nothing to say.
    s.run("mail.markUnread", &json!({"ids": [m]})).unwrap();
    s.run("mail.clearFlag", &json!({"ids": [m]})).unwrap();
    {
        let m = s.mailbox.message_mut(m).unwrap();
        (m.flag, m.answered, m.forwarded) = (FlagStatus::None, false, false);
    }
    assert_eq!(changes(&s, account), []);
    // Marked complete: a JMAP server has no such flag, so it is no flag there.
    s.run("mail.flag", &json!({"ids": [m]})).unwrap();
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().flag = FlagStatus::Flagged;
    s.run("mail.complete", &json!({"ids": [m]})).unwrap();
    assert_eq!(s.mailbox.message(m).unwrap().flag, FlagStatus::Complete);
    assert_eq!(changes(&s, account), [keywords(m, "E1", None, Some(false))]);
    // Complete here and unflagged there: the same thing, nothing to say.
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().flag = FlagStatus::None;
    assert_eq!(changes(&s, account), []);
}

#[test]
fn a_folder_that_differs_is_a_move_and_delete_is_a_move_to_deleted_items() {
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    // The marks first, then the move.
    assert_eq!(
        changes(&s, account),
        [keywords(m, "E1", Some(true), None), JmapChange::Move { message: m, id: "E1".into(), from: "Min".into(), to: "Mar".into() }]
    );
    // Moved back to where the server has it: no move.
    let inbox = folder_of(&s, account, "Min");
    s.run("mail.move", &json!({"ids": [m], "folder": inbox})).unwrap();
    assert_eq!(changes(&s, account), [keywords(m, "E1", Some(true), None)]);
    // Delete: a move into Deleted Items. Archive and Junk likewise.
    s.run("mail.delete", &json!({"ids": [m]})).unwrap();
    assert!(changes(&s, account).contains(&JmapChange::Move { message: m, id: "E1".into(), from: "Min".into(), to: "Mtr".into() }));
    s.run("mail.archive", &json!({"ids": [m]})).unwrap();
    assert!(changes(&s, account).contains(&JmapChange::Move { message: m, id: "E1".into(), from: "Min".into(), to: "Mar".into() }));
    s.run("mail.junk", &json!({"ids": [m]})).unwrap();
    assert!(changes(&s, account).contains(&JmapChange::Move { message: m, id: "E1".into(), from: "Min".into(), to: "Mju".into() }));
}

#[test]
fn delete_for_good_and_emptying_deleted_items_destroy_and_a_message_with_no_id_cannot_be() {
    let (mut s, account, m, _) = setup();
    s.run("mail.deletePermanently", &json!({"ids": [m]})).unwrap();
    assert!(s.mailbox.message(m).is_none());
    assert_eq!(changes(&s, account), [JmapChange::Destroy { id: "E1".into() }]);
    assert_eq!(unpushed(&s, account), 1);
    // Deleted Items emptied: each message the server has there; one with no id there is not named.
    let (mut s, account, _, _) = setup();
    let deleted = folder_of(&s, account, "Mtr");
    let (a, b) = (add(&mut s, deleted, "E7", "Seven"), add(&mut s, deleted, "E8", "Eight"));
    let unbound = add(&mut s, deleted, "E9", "Nine");
    s.mailbox.message_mut(unbound).unwrap().server.as_mut().unwrap().id.clear();
    s.run("folder.empty", &json!({"id": deleted})).unwrap();
    assert!([a, b, unbound].iter().all(|id| s.mailbox.message(*id).is_none()));
    assert_eq!(changes(&s, account), [JmapChange::Destroy { id: "E7".into() }, JmapChange::Destroy { id: "E8".into() }]);
    // Junk E-mail emptied likewise; any other folder goes to Deleted Items (a move).
    let junk = folder_of(&s, account, "Mju");
    add(&mut s, junk, "E5", "Five");
    let archive = folder_of(&s, account, "Mar");
    let six = add(&mut s, archive, "E6", "Six");
    s.run("folder.empty", &json!({"id": junk})).unwrap();
    s.run("folder.empty", &json!({"id": archive})).unwrap();
    let all = changes(&s, account);
    assert!(all.contains(&JmapChange::Destroy { id: "E5".into() }));
    assert!(all.contains(&JmapChange::Move { message: six, id: "E6".into(), from: "Mar".into(), to: "Mtr".into() }));
}

#[test]
fn a_folder_the_server_does_not_have_is_no_place_to_move_to_and_a_message_with_no_id_sends_nothing() {
    let (mut s, account, m, _) = setup();
    // A folder with no server mailbox: the move waits where it is.
    let local = s.mailbox.alloc_id();
    s.mailbox.folders.push(Folder { id: local, account, name: "Made here".into(), kind: FolderKind::Custom, ..Default::default() });
    s.run("mail.move", &json!({"ids": [m], "folder": local})).unwrap();
    assert_eq!(changes(&s, account), []);
    assert_eq!(unpushed(&s, account), 0);
    // A message whose id is gone, or marked gone, is not touched either.
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().id.clear();
    assert_eq!(changes(&s, account), []);
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().id = "E1".into();
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().gone = true;
    assert_eq!(changes(&s, account), []);
    // A mailbox file that says anything: ids that cannot be an id's are not sent.
    let (mut s, account, m, _) = setup();
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().id = "E 1/../".into();
    assert_eq!(changes(&s, account), []);
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().id = "E".repeat(600);
    assert_eq!(changes(&s, account), []);
}

#[test]
fn what_waits_in_the_outbox_or_belongs_to_another_account_is_not_a_change() {
    let (mut s, account, m, kept) = setup();
    let outbox = s.mailbox.folder_of_kind(account, FolderKind::Outbox).unwrap();
    s.mailbox.add_message(Message { folder: outbox, subject: "Waiting".into(), ..Default::default() }).unwrap();
    // A message the server doesn't have yet has no server state to differ from.
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    assert_eq!(changes(&s, account).len(), 1);
    let local = s.mailbox.add_message(Message { folder: kept, subject: "Here only".into(), ..Default::default() }).unwrap();
    s.run("mail.markRead", &json!({"ids": [local]})).unwrap();
    assert_eq!(changes(&s, account).len(), 1);
    // Another account's changes are its own.
    assert_eq!(message_changes(&s, s.mailbox.accounts.iter().find(|a| a.kind == AccountKind::Local).unwrap().id, 100), (Vec::new(), 0));
}

#[test]
fn the_limit_cuts_what_is_spelled_out_and_not_what_is_counted() {
    let (mut s, account, m, _) = setup();
    let inbox = folder_of(&s, account, "Min");
    let mut ids = vec![m];
    for n in 2..=6 {
        ids.push(add(&mut s, inbox, &format!("E{n}"), &format!("Note {n}")));
    }
    s.run("mail.markRead", &json!({"ids": ids})).unwrap();
    let (some, all) = message_changes(&s, account, 4);
    assert_eq!((some.len(), all), (4, 6));
    assert_eq!(unpushed(&s, account), 6);
    assert_eq!(message_changes(&s, account, 0).0.len(), 0);
}

#[test]
fn the_request_carries_the_changes_and_has_a_light_run_read_where_the_moved_go() {
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    let request = jmap_mail_request(&s, account, false, &["Mpr".into()], true).unwrap();
    assert_eq!(request.changes, [JmapChange::Move { message: m, id: "E1".into(), from: "Min".into(), to: "Mar".into() }]);
    // The mailbox on screen first, then the one the message goes to; the Inbox is the host's.
    assert_eq!(request.scope, JmapScope::Light { also: vec!["Mpr".into(), "Mar".into()] });
    // Named already: once.
    let request = jmap_mail_request(&s, account, false, &["Mar".into()], true).unwrap();
    assert_eq!(request.scope, JmapScope::Light { also: vec!["Mar".into()] });
    // A run that reads everything needs no more, and the same changes are there.
    let request = jmap_mail_request(&s, account, true, &[], false).unwrap();
    assert_eq!((request.scope.clone(), request.changes.len()), (JmapScope::All, 1));
    // The command says the same, in JSON.
    let said = s.run("sync.jmapRequest", &json!({"account": account})).unwrap();
    assert_eq!(said["changes"][0]["move"]["to"], "Mar");
    assert_eq!(s.run("sync.request", &json!({"account": account})).unwrap()["changes"].as_array().map(Vec::len), Some(1));
    // What waits counts, and what the request names has no more than the limit.
    assert_eq!(unpushed(&s, account), 1);
    // After the message was moved and has no id, the mailbox where it lies is read until it is found again.
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().id.clear();
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().folder = "Mar".into();
    let request = jmap_mail_request(&s, account, false, &[], true).unwrap();
    assert!(request.changes.is_empty());
    assert_eq!(request.scope, JmapScope::Light { also: vec!["Mar".into()] });
    // The same for one marked gone that lies in a folder here.
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().id = "E1".into();
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().gone = true;
    assert_eq!(jmap_mail_request(&s, account, false, &[], true).unwrap().scope, JmapScope::Light { also: vec!["Mar".into()] });
}

#[test]
fn what_was_made_is_the_last_synced_state_and_the_history_stays_for_marks_alone() {
    let (mut s, account, m, _) = setup();
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    s.run("mail.flag", &json!({"ids": [m]})).unwrap();
    assert!(s.can_undo());
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    let answer = apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    assert_eq!(answer["pushed"], 1);
    let here = s.mailbox.message(m).unwrap();
    let server = here.server.as_ref().unwrap();
    assert!(server.read && here.read && server.flag == FlagStatus::Flagged && here.flag == FlagStatus::Flagged);
    assert_eq!(changes(&s, account), []);
    // A read mark or a flag going up doesn't cost the history (opening a message would otherwise wipe it every few
    // seconds).
    assert!(s.can_undo());
}

#[test]
fn a_move_that_was_made_gives_the_message_the_id_the_server_named_or_none_and_ends_the_history() {
    for (named, expected) in [("E1b", "E1b"), ("", ""), ("E1", "E1"), ("not an id!", "")] {
        let (mut s, account, m, _) = setup();
        let archive = folder_of(&s, account, "Mar");
        s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
        s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
        assert!(s.can_undo());
        let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
        assert_eq!(sent.len(), 2);
        let answers = vec![done(sent[0].clone(), ok("")), done(sent[1].clone(), ok(named))];
        let answer = apply_jmap_mail(&mut s, result(account, answers)).unwrap();
        assert_eq!(answer["pushed"], 2, "{named}");
        let server = s.mailbox.message(m).unwrap().server.clone().unwrap();
        assert_eq!((server.id.as_str(), server.folder.as_str(), server.read), (expected, "Mar", true), "{named}");
        // Nothing more to send, with an id or without. A mailbox file saved here has it too.
        assert_eq!(changes(&s, account), [], "{named}");
        // The server was told to move: an undo would bring back a mailbox the server no longer has.
        assert!(!s.can_undo(), "{named}");
        let written = serde_json::to_string(&s.mailbox).unwrap();
        assert_eq!(
            serde_json::from_str::<Mailbox>(&written).unwrap().message(m).unwrap().server.as_ref().map(|server| server.id.as_str()),
            Some(expected)
        );
    }
}

#[test]
fn a_message_moved_and_given_a_new_id_is_found_again_by_its_message_id_in_the_mailbox_it_went_to() {
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    let before = s.mailbox.messages.len();
    // The window of the Archive, in the same run, lists it under the id the server gave it.
    let there = GraphMessage {
        server: ServerMessage { id: "E1-moved".into(), folder: "Mar".into(), read: true, ..Default::default() },
        message_id: "<invoice-42@example.org>".into(),
        subject: "Invoice 42".into(),
        received: NOW,
        ..Default::default()
    };
    let page = MessagePage { folder: "Mar".into(), messages: vec![there], afresh: true, ..Default::default() };
    let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!((answer["new"].as_u64(), answer["changed"].as_u64(), answer["removedMessages"].as_u64()), (Some(0), Some(1), Some(0)), "{answer}");
    assert_eq!(s.mailbox.messages.len(), before, "the same message, not a second one");
    let server = s.mailbox.message(m).unwrap().server.clone().unwrap();
    assert_eq!((server.id.as_str(), server.folder.as_str(), s.mailbox.message(m).unwrap().folder), ("E1-moved", "Mar", archive));
    // It has what the server says of it, and from now on its changes go up under the new id.
    assert!(s.mailbox.message(m).unwrap().read);
    s.mailbox.message_mut(m).unwrap().flag = FlagStatus::Flagged;
    assert_eq!(changes(&s, account), [keywords(m, "E1-moved", None, Some(true))]);
}

#[test]
fn a_moved_message_nothing_has_found_again_is_said_at_the_end_of_the_run_and_not_when_it_has_been_found() {
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    // The server made the move and did not say the new id; no window shows the message (the Archive holds more than one).
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    assert!(
        s.sync_status.starts_with("Folders up to date. 1 moved message can't be found on the server yet, so changes to it wait. — "),
        "{}",
        s.sync_status
    );
    // A message that is edited meanwhile sends nothing, and is not counted among the changes that wait.
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    assert_eq!(changes(&s, account), []);
    assert_eq!(unpushed(&s, account), 0);
    // A second, in the same case, is counted too.
    let inbox = folder_of(&s, account, "Min");
    let second = add(&mut s, inbox, "E2", "Receipt");
    s.run("mail.move", &json!({"ids": [second], "folder": archive})).unwrap();
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    assert!(s.sync_status.contains(". 2 moved messages can't be found on the server yet, so changes to them wait. — "), "{}", s.sync_status);
    // Found: the window of the Archive lists them, and nothing is said.
    let listed_there = |id: &str, message_id: &str, subject: &str| GraphMessage {
        server: ServerMessage { id: id.into(), folder: "Mar".into(), ..Default::default() },
        message_id: message_id.into(),
        subject: subject.into(),
        received: NOW,
        ..Default::default()
    };
    let page = MessagePage {
        folder: "Mar".into(),
        messages: vec![listed_there("E1-new", "<invoice-42@example.org>", "Invoice 42"), listed_there("E2-new", "<receipt@example.org>", "Receipt")],
        afresh: true,
        ..Default::default()
    };
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert!(!s.sync_status.contains("moved message"), "{}", s.sync_status);
}

#[test]
fn a_move_the_server_has_no_message_for_is_said_and_the_message_is_swept_unless_it_is_found_again() {
    let moved = |s: &mut Session, account: Id, m: Id| {
        let archive = folder_of(s, account, "Mar");
        s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
        let sent = jmap_mail_request(s, account, false, &[], false).unwrap().changes;
        let answer = apply_jmap_mail(s, result(account, vec![done(sent[0].clone(), JmapOutcome::Gone)])).unwrap();
        assert_eq!(answer["errors"], json!(["Couldn't move 'Invoice 42' to Archive: the server says it no longer exists."]));
        assert!(s.mailbox.message(m).unwrap().server.as_ref().unwrap().gone);
        assert_eq!(changes(s, account), [], "a message the server no longer has sends nothing");
    };
    // Nobody finds it: the run that sees the mailboxes through takes it away.
    let (mut s, account, m, _) = setup();
    moved(&mut s, account, m);
    let answer = apply_jmap_mail(&mut s, JmapMailResult { whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!(answer["removedMessages"], 1);
    assert!(s.mailbox.message(m).is_none());
    // Someone does: the move was made, only its answer was lost (or another client moved it there), and the window
    // of the mailbox it is in shows it under another id.
    let (mut s, account, m, _) = setup();
    moved(&mut s, account, m);
    let there = GraphMessage {
        server: ServerMessage { id: "E1-elsewhere".into(), folder: "Mar".into(), ..Default::default() },
        message_id: "invoice-42@example.org".into(),
        subject: "Invoice 42".into(),
        ..Default::default()
    };
    let page = MessagePage { folder: "Mar".into(), messages: vec![there], ..Default::default() };
    let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!((answer["new"].as_u64(), answer["removedMessages"].as_u64()), (Some(0), Some(0)), "{answer}");
    let server = s.mailbox.message(m).unwrap().server.clone().unwrap();
    assert_eq!((server.id.as_str(), server.gone), ("E1-elsewhere", false));
}

#[test]
fn a_run_that_got_no_answer_works_the_same_changes_out_again_and_the_same_answer_twice_changes_nothing() {
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    let first = jmap_mail_request(&s, account, false, &[], false).unwrap();
    // The run was cut short (the program was closed, the connection went): nothing is kept of it.
    let again = jmap_mail_request(&s, account, false, &[], false).unwrap();
    assert_eq!(first.changes, again.changes);
    let written = serde_json::to_string(&s.mailbox).unwrap();
    let reopened: Mailbox = serde_json::from_str(&written).unwrap();
    let mut s2 = Session::at(reopened, NOW, 0);
    s2.jmap_runs = true;
    assert_eq!(jmap_mail_request(&s2, account, false, &[], false).unwrap().changes, first.changes, "the difference is still there");
    assert_eq!(unpushed(&s2, account), 2);
    // Answered, and the answer taken in twice (a host that tried again): once is all it does.
    let answers: Vec<JmapChangeDone> =
        first.changes.iter().map(|c| done(c.clone(), ok(if matches!(c, JmapChange::Move { .. }) { "E1b" } else { "" }))).collect();
    let first_time = apply_jmap_mail(&mut s2, result(account, answers.clone())).unwrap();
    let after = s2.mailbox.clone();
    let second_time = apply_jmap_mail(&mut s2, result(account, answers)).unwrap();
    assert_eq!((first_time["pushed"].as_u64(), second_time["pushed"].as_u64()), (Some(2), Some(0)));
    assert_eq!(s2.mailbox, after);
    assert_eq!(unpushed(&s2, account), 0);
}

#[test]
fn another_client_changing_the_same_message_meanwhile_does_not_undo_what_is_waiting_here() {
    let (mut s, account, m, _) = setup();
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    // The server's window comes down first: it says the message is flagged there (another client did it), unread.
    let there = GraphMessage {
        server: ServerMessage { id: "E1".into(), folder: "Min".into(), flag: FlagStatus::Flagged, ..Default::default() },
        subject: "Invoice 42".into(),
        ..Default::default()
    };
    let page = MessagePage { folder: "Min".into(), messages: vec![there], ..Default::default() };
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], whole: true, ..result(account, Vec::new()) }).unwrap();
    let here = s.mailbox.message(m).unwrap();
    // The flag is the server's, the read mark is the owner's, and goes up.
    assert_eq!((here.flag, here.read), (FlagStatus::Flagged, true));
    assert_eq!(changes(&s, account), [keywords(m, "E1", Some(true), None)]);
}

#[test]
fn a_refusal_is_put_back_and_said_once_per_kind_and_reason() {
    let (mut s, account, m, _) = setup();
    let inbox = folder_of(&s, account, "Min");
    let mut ids = vec![m];
    for n in 2..5 {
        ids.push(add(&mut s, inbox, &format!("E{n}"), &format!("Note {n}")));
    }
    s.run("mail.markRead", &json!({"ids": ids})).unwrap();
    let archive = folder_of(&s, account, "Mar");
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    let answers: Vec<JmapChangeDone> = sent
        .into_iter()
        .map(|change| {
            let outcome = match &change {
                JmapChange::Move { .. } => refused("forbidden"),
                JmapChange::Keywords { id, .. } if id == "E1" => refused("invalidPatch"),
                _ => JmapOutcome::Gone,
            };
            done(change, outcome)
        })
        .collect();
    let answer = apply_jmap_mail(&mut s, result(account, answers)).unwrap();
    assert_eq!(
        answer["errors"],
        json!([
            "Couldn't mark 3 messages as read: the server says they no longer exist.",
            "Couldn't mark 'Invoice 42' as read: the server says it can't make that change.",
            "Couldn't move 'Invoice 42' to Archive: the server says this account may not change it.",
        ])
    );
    assert_eq!(answer["updated"], 5);
    assert!(s.sync_status.starts_with("Couldn't mark 3 messages as read"), "{}", s.sync_status);
    // Put back: unread, and in the Inbox. The others are gone from the server, and leave once the run has seen
    // every mailbox.
    let back = s.mailbox.message(m).unwrap();
    assert!(!back.read && back.folder == inbox);
    assert!(ids[1..].iter().all(|id| s.mailbox.message(*id).unwrap().server.as_ref().unwrap().gone));
    assert_eq!(changes(&s, account), []);
    // The next run's status doesn't say it again, and a refusal does not end the history of marks.
    apply_jmap_mail(&mut s, result(account, Vec::new())).unwrap();
    assert!(!s.sync_status.contains("Couldn't"), "{}", s.sync_status);
}

#[test]
fn what_a_change_was_refused_for_is_said_in_plain_words() {
    let (mut s, account, m, _) = setup();
    for (why, words) in [
        ("forbidden", "this account may not change it"),
        ("accountReadOnly", "this account may not change it"),
        ("overQuota", "the mailbox is full"),
        ("tooManyKeywords", "it can't make that change"),
        ("unknown", "it can't make that change"),
        ("", "it can't make that change"),
    ] {
        s.mailbox.message_mut(m).unwrap().read = true;
        let change = keywords(m, "E1", Some(true), None);
        let answer = apply_jmap_mail(&mut s, result(account, vec![done(change, refused(why))])).unwrap();
        assert_eq!(answer["errors"], json!([format!("Couldn't mark 'Invoice 42' as read: the server says {words}.")]), "{why}");
        assert!(!s.mailbox.message(m).unwrap().read);
    }
    // Marked again here before the answer came: that stays, and goes up next time.
    s.mailbox.message_mut(m).unwrap().read = true;
    let change = keywords(m, "E1", Some(false), None);
    apply_jmap_mail(&mut s, result(account, vec![done(change, refused("forbidden"))])).unwrap();
    assert!(s.mailbox.message(m).unwrap().read);
    assert_eq!(changes(&s, account), [keywords(m, "E1", Some(true), None)]);
}

#[test]
fn a_refused_delete_for_good_puts_the_message_back_with_all_that_was_kept() {
    let (mut s, account, m, _) = setup();
    let inbox = folder_of(&s, account, "Min");
    s.mailbox.message_mut(m).unwrap().body.text = "Please pay by Friday.".into();
    // Marked read, then deleted for good: neither reached the server.
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    s.run("mail.deletePermanently", &json!({"ids": [m]})).unwrap();
    assert!(s.mailbox.message(m).is_none());
    let delete = |s: &mut Session, outcome: JmapOutcome| {
        apply_jmap_mail(s, result(account, vec![done(JmapChange::Destroy { id: "E1".into() }, outcome)])).unwrap()
    };
    let answer = delete(&mut s, refused("forbidden"));
    assert_eq!(answer["errors"], json!(["Couldn't delete 'Invoice 42' for good: the server doesn't allow it."]));
    // Back as the server has it (unread, in the Inbox), with its text, and nothing to send.
    let back = s.mailbox.messages.values().find(|m| m.subject == "Invoice 42").unwrap();
    assert_eq!((back.folder, back.read, back.body.text.as_str()), (inbox, false, "Please pay by Friday."));
    assert_eq!(back.server.as_ref().map(|server| server.id.as_str()), Some("E1"));
    assert!(s.mailbox.queued.is_empty());
    assert_eq!(changes(&s, account), []);
    // It was the same message, with its own id here.
    assert_eq!(back.id, m);

    // Gone from the server already: it stays gone, and nothing is said. Done likewise.
    for outcome in [JmapOutcome::Gone, ok("")] {
        let (mut s, account, m, _) = setup();
        s.run("mail.deletePermanently", &json!({"ids": [m]})).unwrap();
        let answer = apply_jmap_mail(&mut s, result(account, vec![done(JmapChange::Destroy { id: "E1".into() }, outcome)])).unwrap();
        assert_eq!(answer["errors"], json!([]));
        assert!(s.mailbox.messages.values().all(|m| m.subject != "Invoice 42") && s.mailbox.queued.is_empty());
        // Both reached the server: no undo to a mailbox that has it.
        assert!(!s.can_undo());
    }

    // Nothing kept (a queue from before): the server's copy comes down again with its window, and is not made up.
    let (mut s, account, m, _) = setup();
    s.run("mail.deletePermanently", &json!({"ids": [m]})).unwrap();
    for q in &mut s.mailbox.queued {
        if let Queued::DeleteMessage { kept, .. } = q {
            *kept = None;
        }
    }
    let answer = apply_jmap_mail(&mut s, result(account, vec![done(JmapChange::Destroy { id: "E1".into() }, refused("forbidden"))])).unwrap();
    assert_eq!(answer["errors"], json!(["Couldn't delete a message for good: the server doesn't allow it."]));
    assert!(s.mailbox.messages.values().all(|m| m.subject != "Invoice 42") && s.mailbox.queued.is_empty());
}

#[test]
fn deletes_for_good_are_taken_in_together_each_comes_off_the_queue_and_only_a_refused_one_is_put_back() {
    let (mut s, account, _, _) = setup();
    let inbox = folder_of(&s, account, "Min");
    // Deleted Items emptied: a great many deletes at once, of which the answer is about all but the first ten.
    let ids: Vec<Id> = (0..300).map(|n| add(&mut s, inbox, &format!("D{n}"), &format!("Mail {n}"))).collect();
    s.run("mail.deletePermanently", &json!({"ids": ids})).unwrap();
    assert_eq!(s.mailbox.queued.len(), 300);
    // One of those the server will refuse has come down again meanwhile: it is not put back a second time.
    let again = add(&mut s, inbox, "D11", "Mail 11 again");
    let outcome = |n: usize| match n % 3 {
        0 => ok(""),
        1 => JmapOutcome::Gone,
        _ => refused("forbidden"),
    };
    let dones = (10..300).map(|n| done(JmapChange::Destroy { id: format!("D{n}") }, outcome(n))).collect();
    let answer = apply_jmap_mail(&mut s, result(account, dones)).unwrap();
    // 97 refused (n = 11, 14, … 299), and said once.
    assert_eq!(answer["errors"], json!(["Couldn't delete 97 messages for good: the server doesn't allow it."]));
    // What was not answered still waits, oldest first.
    let waiting: Vec<&str> = s.mailbox.queued.iter().map(|q| if let Queued::DeleteMessage { id, .. } = q { id.as_str() } else { "" }).collect();
    assert_eq!(waiting, (0..10).map(|n| format!("D{n}")).collect::<Vec<_>>());
    // The refused are back, each once; the ones the server made or no longer had are not.
    let back = |id: &str| s.mailbox.messages.values().filter(|m| m.server.as_ref().is_some_and(|server| server.id == id)).count();
    assert_eq!((back("D14"), back("D299"), back("D12"), back("D13"), back("D5")), (1, 1, 0, 0, 0));
    assert_eq!(back("D11"), 1, "the one that came down again meanwhile is the one there is");
    assert_eq!(s.mailbox.message(again).map(|m| m.subject.as_str()), Some("Mail 11 again"));
    // (96 put back, and the one that came down again.)
    assert_eq!(s.mailbox.messages.values().filter(|m| m.subject.starts_with("Mail ")).count(), 97);
}

#[test]
fn an_answer_for_a_message_that_is_not_there_or_is_not_the_one_changes_nothing() {
    let (mut s, account, m, _) = setup();
    let before = s.mailbox.clone();
    let junk = vec![
        done(keywords(9_999, "E1", Some(true), None), ok("")),
        done(keywords(m, "E2", Some(true), None), JmapOutcome::Gone),
        done(JmapChange::Move { message: m, id: "E9".into(), from: "Min".into(), to: "Mar".into() }, ok("E10")),
        done(JmapChange::Move { message: 0, id: String::new(), from: String::new(), to: String::new() }, refused("x")),
        done(JmapChange::Destroy { id: "E404".into() }, JmapOutcome::Gone),
        done(JmapChange::Destroy { id: "E".repeat(10_000) }, refused(&"y".repeat(10_000))),
    ];
    let answer = apply_jmap_mail(&mut s, result(account, junk)).unwrap();
    assert_eq!(answer["errors"], json!(["Couldn't delete a message for good: the server says it can't make that change."]));
    assert_eq!(answer["pushed"], 0);
    assert_eq!(s.mailbox, before);
    // An answer for another account's address is refused, whatever it holds.
    let stranger = JmapMailResult { email: "eve@example.org".into(), ..result(account, vec![done(keywords(m, "E1", Some(true), None), ok(""))]) };
    assert!(apply_jmap_mail(&mut s, stranger).is_err());
    assert!(!s.mailbox.message(m).unwrap().server.as_ref().unwrap().read);
    // And junk of every kind, in the shape of an answer, is read without a panic.
    for text in [
        r#"{"account": 1, "changes": [{"change": {"keywords": {"message": 1, "id": "E1"}}, "outcome": {"done": {"id": 5}}}]}"#,
        r#"{"account": 1, "changes": [{"change": {"move": {"message": 18446744073709551615, "id": "E1", "from": "", "to": "\u0000"}}, "outcome": "gone"}]}"#,
        r#"{"account": 1, "changes": [{"change": {"destroy": {"id": "E1"}}, "outcome": {"refused": {"why": null}}}]}"#,
    ] {
        let parsed = serde_json::from_str::<Value>(text).unwrap();
        let _ = s.run("sync.applyJmap", &json!({"result": parsed}));
    }
}

#[test]
fn mail_does_not_move_between_a_jmap_account_and_another_account_and_is_never_copied_to_or_from_one() {
    let (mut s, account, m, kept) = setup();
    let archive = folder_of(&s, account, "Mar");
    let local = s.mailbox.add_message(Message { folder: kept, subject: "Here only".into(), ..Default::default() }).unwrap();
    let before = s.mailbox.clone();
    for (command, ids, folder, reason) in [
        ("mail.move", m, kept, mailcraft_model::ACROSS_JMAP),
        ("mail.move", local, archive, mailcraft_model::ACROSS_JMAP),
        ("mail.copy", m, archive, mailcraft_model::COPY_JMAP),
        ("mail.copy", m, kept, mailcraft_model::COPY_JMAP),
        ("mail.copy", local, archive, mailcraft_model::COPY_JMAP),
    ] {
        let refused = s.run(command, &json!({"ids": [ids], "folder": folder})).unwrap_err();
        assert_eq!(refused.to_string(), reason, "{command}");
    }
    // Said in the guards of the model, so that no door leads round them.
    assert!(s.mailbox.clone().move_message(m, kept).is_err() && s.mailbox.clone().copy_message(m, archive).is_err());
    // Nothing happened to either message, and nothing waits to go up.
    assert_eq!(s.mailbox.messages, before.messages);
    assert_eq!((s.mailbox.queued.len(), unpushed(&s, account)), (0, 0));
    // Within the account a move is a move.
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    assert_eq!(changes(&s, account).len(), 1);
    // The accounts of other kinds move mail as they did.
    let imap = s.run("account.add", &json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    let there = s.mailbox.folder_of_kind(imap, FolderKind::Archive).unwrap();
    s.run("mail.move", &json!({"ids": [local], "folder": there})).unwrap();
    s.run("mail.copy", &json!({"ids": [local], "folder": kept})).unwrap();
}

#[test]
fn a_jmap_accounts_folders_are_its_servers_and_deleted_items_is_emptied_with_its_folders_left() {
    let (mut s, account, m, _) = setup();
    let inbox = folder_of(&s, account, "Min");
    let deleted = folder_of(&s, account, "Mtr");
    let projects = s.mailbox.add_folder(account, None, "Projects").unwrap();
    s.mailbox.folder_mut(projects).unwrap().server = Some(ServerFolder { id: "Mpr".into(), name: "Projects".into(), ..Default::default() });
    let before = s.mailbox.folders.clone();
    for (command, params) in [
        ("folder.new", json!({"name": "Made here", "parent": inbox})),
        ("folder.new", json!({"name": "Made here", "parent": null, "account": account})),
        ("folder.rename", json!({"name": "Renamed", "id": projects})),
        ("folder.move", json!({"parent": inbox, "id": projects})),
        ("folder.delete", json!({"id": projects})),
    ] {
        let refused = s.run(command, &params).unwrap_err();
        assert_eq!(refused.to_string(), JMAP_FOLDERS_REASON, "{command}");
    }
    assert_eq!(s.mailbox.folders, before, "nothing changed");
    // Another account's folders are made as they were.
    let (mut other, ..) = setup();
    let local = other.mailbox.accounts.iter().find(|a| a.kind == AccountKind::Local).unwrap().id;
    assert!(other.run("folder.new", &json!({"name": "Made here", "parent": null, "account": local})).is_ok());
    // Deleted Items emptied: its messages are destroyed, and the folder in it stays (the server would give it back).
    let nested = s.mailbox.add_folder(account, Some(deleted), "Old").unwrap();
    s.mailbox.folder_mut(nested).unwrap().server =
        Some(ServerFolder { id: "Mol".into(), name: "Old".into(), parent: "Mtr".into(), ..Default::default() });
    let in_nested = add(&mut s, nested, "E5", "Five");
    let in_deleted = add(&mut s, deleted, "E6", "Six");
    s.run("folder.empty", &json!({"id": deleted})).unwrap();
    assert!(s.mailbox.message(in_deleted).is_none() && s.mailbox.folder(nested).is_some());
    assert!(s.mailbox.message(in_nested).is_some(), "what is in a folder of Deleted Items is that folder's");
    assert_eq!(changes(&s, account), [JmapChange::Destroy { id: "E6".into() }]);
    let _ = m;
}

const JMAP_FOLDERS_REASON: &str = super::super::folder::JMAP_FOLDERS;

#[test]
fn a_move_that_has_not_reached_the_server_is_not_undone_by_a_window_that_lists_the_message_where_it_was() {
    for afresh in [false, true] {
        let (mut s, account, m, _) = setup();
        let (inbox, archive) = (folder_of(&s, account, "Min"), folder_of(&s, account, "Mar"));
        s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
        // The push failed (or has not been tried): the server lists the message in the Inbox still.
        let there = GraphMessage {
            server: ServerMessage { id: "E1".into(), folder: "Min".into(), ..Default::default() },
            message_id: "invoice-42@example.org".into(),
            subject: "Invoice 42".into(),
            ..Default::default()
        };
        let page = MessagePage { folder: "Min".into(), messages: vec![there], afresh, ..Default::default() };
        apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], whole: true, ..result(account, Vec::new()) }).unwrap();
        let here = s.mailbox.message(m).unwrap();
        assert_eq!((here.folder, here.server.as_ref().map(|server| server.folder.as_str())), (archive, Some("Min")), "afresh: {afresh}");
        assert_ne!(here.folder, inbox);
        // And it still goes up.
        assert_eq!(
            changes(&s, account),
            [JmapChange::Move { message: m, id: "E1".into(), from: "Min".into(), to: "Mar".into() }],
            "afresh: {afresh}"
        );
    }
}

/// What the server's window says of the message `E1`, under the id `id`, in the mailbox `mailbox`.
fn listed(mailbox: &str, id: &str, flag: FlagStatus) -> GraphMessage {
    GraphMessage {
        server: ServerMessage { id: id.into(), folder: mailbox.into(), flag, ..Default::default() },
        message_id: "<invoice-42@example.org>".into(),
        subject: "Invoice 42".into(),
        received: NOW,
        ..Default::default()
    }
}

fn window_of(mailbox: &str, messages: Vec<GraphMessage>, complete: bool) -> MessagePage {
    MessagePage { folder: mailbox.into(), messages, afresh: complete, ..Default::default() }
}

#[test]
fn a_move_whose_answer_was_lost_is_found_again_in_whatever_order_the_windows_come() {
    for (first, complete_source) in [("destination", true), ("destination", false), ("source", true)] {
        let (mut s, account, m, _) = setup();
        let (inbox, archive) = (folder_of(&s, account, "Min"), folder_of(&s, account, "Mar"));
        let count = s.mailbox.messages.len();
        // The owner archived it; the server made the move, and the answer was lost: nothing here knows.
        s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
        let moved = window_of("Mar", vec![listed("Mar", "E1-moved", FlagStatus::None)], false);
        let other = add_other_in_inbox(&mut s, account);
        let source = window_of("Min", vec![other], complete_source);
        let pages = if first == "destination" { vec![moved, source] } else { vec![source, moved] };
        apply_jmap_mail(&mut s, JmapMailResult { pages, whole: true, ..result(account, Vec::new()) }).unwrap();
        let here = s.mailbox.message(m).unwrap();
        assert_eq!(s.mailbox.messages.len(), count + 1, "{first}, complete: {complete_source}: no second Invoice 42");
        assert_eq!(
            (here.folder, here.server.as_ref().map(|server| (server.id.as_str(), server.folder.as_str()))),
            (archive, Some(("E1-moved", "Mar"))),
            "{first}, complete: {complete_source}"
        );
        assert_ne!(here.folder, inbox);
        assert_eq!(changes(&s, account), [], "{first}, complete: {complete_source}");
    }
}

/// Another message the server lists in the Inbox, which this mailbox has too.
fn add_other_in_inbox(s: &mut Session, account: Id) -> GraphMessage {
    let inbox = folder_of(s, account, "Min");
    add(s, inbox, "E2", "Other");
    GraphMessage {
        server: ServerMessage { id: "E2".into(), folder: "Min".into(), ..Default::default() },
        message_id: "other@example.org".into(),
        subject: "Other".into(),
        ..Default::default()
    }
}

#[test]
fn a_message_moved_again_before_its_new_id_is_known_stays_where_the_owner_put_it_and_goes_there() {
    for complete in [false, true] {
        let (mut s, account, m, _) = setup();
        let (archive, deleted) = (folder_of(&s, account, "Mar"), folder_of(&s, account, "Mtr"));
        s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
        let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
        apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
        assert_eq!(s.mailbox.message(m).unwrap().server.as_ref().map(|server| (server.id.as_str(), server.folder.as_str())), Some(("", "Mar")));
        // Moved again, to Deleted Items, before the server's id for it was found: nothing can be sent yet.
        s.run("mail.delete", &json!({"ids": [m]})).unwrap();
        assert_eq!(changes(&s, account), []);
        // The Archive's window lists it under the id the server gave it.
        let page = window_of("Mar", vec![listed("Mar", "E1-moved", FlagStatus::None)], complete);
        let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], whole: true, ..result(account, Vec::new()) }).unwrap();
        assert_eq!((answer["new"].as_u64(), answer["removedMessages"].as_u64()), (Some(0), Some(0)), "complete: {complete}: {answer}");
        let here = s.mailbox.message(m).unwrap();
        assert_eq!(here.folder, deleted, "complete: {complete}: it stays where the owner put it");
        // Found, and its second move goes up under the id it has now.
        assert_eq!(
            changes(&s, account),
            [JmapChange::Move { message: m, id: "E1-moved".into(), from: "Mar".into(), to: "Mtr".into() }],
            "complete: {complete}"
        );
    }
}

#[test]
fn a_follow_up_flag_marked_complete_here_does_not_clear_a_flag_set_elsewhere_again_and_again() {
    let (mut s, account, m, _) = setup();
    s.run("mail.flag", &json!({"ids": [m]})).unwrap();
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    s.run("mail.complete", &json!({"ids": [m]})).unwrap();
    // Marked complete here: a JMAP server has no such flag, so it goes up as no flag.
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    assert_eq!(sent, [keywords(m, "E1", None, Some(false))]);
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    assert_eq!(s.mailbox.message(m).unwrap().flag, FlagStatus::Complete);
    // A window that says the same keeps it complete.
    let window = |flag| window_of("Min", vec![listed("Min", "E1", flag)], true);
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![window(FlagStatus::None)], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!((s.mailbox.message(m).unwrap().flag, changes(&s, account)), (FlagStatus::Complete, Vec::new()));
    // Another client flags it again: that is the server's change, and nothing here takes it away.
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![window(FlagStatus::Flagged)], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!((s.mailbox.message(m).unwrap().flag, changes(&s, account)), (FlagStatus::Flagged, Vec::new()));
    // And takes the flag off: no flag here either.
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![window(FlagStatus::None)], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!((s.mailbox.message(m).unwrap().flag, changes(&s, account)), (FlagStatus::None, Vec::new()));
}

#[test]
fn undoing_a_mark_that_already_went_up_goes_up_too_and_so_does_redoing_it() {
    let (mut s, account, m, _) = setup();
    s.run("mail.markRead", &json!({"ids": [m]})).unwrap();
    s.run("mail.flag", &json!({"ids": [m]})).unwrap();
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    assert!(s.can_undo() && changes(&s, account).is_empty());
    // Undone: the flag, then the read mark. The server still has both, so the difference is what goes up.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(changes(&s, account), [keywords(m, "E1", None, Some(false))]);
    s.run("edit.undo", &json!({})).unwrap();
    let here = s.mailbox.message(m).unwrap();
    assert_eq!((here.read, here.flag), (false, FlagStatus::None));
    assert_eq!(changes(&s, account), [keywords(m, "E1", Some(false), Some(false))]);
    // A window that still says what the server has, before the run that tells it, doesn't give them back.
    let page = window_of("Min", vec![listed("Min", "E1", FlagStatus::Flagged)], true);
    let mut there = page.clone();
    there.messages[0].server.read = true;
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![there], whole: true, ..result(account, Vec::new()) }).unwrap();
    let here = s.mailbox.message(m).unwrap();
    assert_eq!((here.read, here.flag), (false, FlagStatus::None));
    assert_eq!(changes(&s, account), [keywords(m, "E1", Some(false), Some(false))]);
    // Sent, and done; redone, and the difference the other way is what is left: the read mark first, then the flag.
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    assert!(changes(&s, account).is_empty() && s.can_redo());
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(changes(&s, account), [keywords(m, "E1", Some(true), None)]);
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(changes(&s, account), [keywords(m, "E1", Some(true), Some(true))]);
}

#[test]
fn an_undo_sends_the_step_undone_and_nothing_else_the_server_or_a_window_changed_since() {
    let (mut s, account, m1, _) = setup();
    let inbox = folder_of(&s, account, "Min");
    let archive = folder_of(&s, account, "Mar");
    let others: Vec<Id> = (2..=5).map(|n| add(&mut s, inbox, &format!("E{n}"), &format!("Mail {n}"))).collect();
    let (m2, m3, m4, m5) = (others[0], others[1], others[2], others[3]);
    // The owner flags the first (a step of the history), and opens the second (which marks it read, and is no step).
    s.run("mail.flag", &json!({"ids": [m1]})).unwrap();
    s.mailbox.message_mut(m2).unwrap().read = true;
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    assert_eq!(sent.len(), 2);
    apply_jmap_mail(&mut s, result(account, sent.iter().map(|change| done(change.clone(), ok(""))).collect())).unwrap();
    assert!(changes(&s, account).is_empty());
    // Meanwhile another client reads the third, flags the fourth and archives the fifth; the windows say so.
    let there = |id: &str, mailbox: &str, read: bool, flag: FlagStatus| GraphMessage {
        server: ServerMessage { id: id.into(), folder: mailbox.into(), read, flag, ..Default::default() },
        message_id: format!("<{}@example.org>", id.to_lowercase()),
        subject: format!("Mail {}", id.trim_start_matches('E')),
        received: NOW,
        ..Default::default()
    };
    let inbox_window = window_of(
        "Min",
        vec![
            there("E1", "Min", false, FlagStatus::Flagged),
            there("E2", "Min", true, FlagStatus::None),
            there("E3", "Min", true, FlagStatus::None),
            there("E4", "Min", false, FlagStatus::Flagged),
        ],
        true,
    );
    let archive_window = window_of("Mar", vec![there("E5", "Mar", false, FlagStatus::None)], false);
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![inbox_window, archive_window], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!(
        (s.mailbox.message(m3).unwrap().read, s.mailbox.message(m4).unwrap().flag, s.mailbox.message(m5).unwrap().folder),
        (true, FlagStatus::Flagged, archive)
    );
    assert!(changes(&s, account).is_empty());
    // Undone: only the flag of the first goes back, and so goes up; what the others did stays as it is.
    assert!(s.can_undo());
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(changes(&s, account), [keywords(m1, "E1", None, Some(false))]);
    let (third, fourth, fifth, second) =
        (s.mailbox.message(m3).unwrap(), s.mailbox.message(m4).unwrap(), s.mailbox.message(m5).unwrap(), s.mailbox.message(m2).unwrap());
    assert_eq!((third.read, fourth.flag, fifth.folder, second.read), (true, FlagStatus::Flagged, archive, true));
    // Redone: the same, the other way.
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(changes(&s, account), [keywords(m1, "E1", None, Some(true))]);
}

#[test]
fn deletes_for_good_are_not_brought_down_before_the_server_has_made_them() {
    let (mut s, account, m, _) = setup();
    s.run("mail.deletePermanently", &json!({"ids": [m]})).unwrap();
    assert_eq!(changes(&s, account), [JmapChange::Destroy { id: "E1".into() }]);
    // The window still lists it (the delete was not sent, or failed): it is not mail to bring down again.
    let page = window_of("Min", vec![listed("Min", "E1", FlagStatus::None)], true);
    let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page.clone()], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!((answer["new"].as_u64(), s.mailbox.messages.values().filter(|m| m.subject == "Invoice 42").count()), (Some(0), 0));
    assert_eq!(changes(&s, account), [JmapChange::Destroy { id: "E1".into() }]);
    // Made: the delete is off the queue; a server that lists it again after that has a message that is new to us.
    apply_jmap_mail(&mut s, result(account, vec![done(JmapChange::Destroy { id: "E1".into() }, ok(""))])).unwrap();
    assert!(s.mailbox.queued.is_empty());
    let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!(answer["new"], 1);
}

#[test]
fn a_message_with_no_id_that_is_deleted_for_good_is_deleted_when_a_window_names_it() {
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    // Deleted for good before any window shows it under its new id: the delete waits, without an id, and says nothing yet.
    s.run("mail.deletePermanently", &json!({"ids": [m]})).unwrap();
    assert!(s.mailbox.message(m).is_none());
    assert!(matches!(s.mailbox.queued.as_slice(), [Queued::DeleteMessage { id, .. }] if id.is_empty()));
    assert_eq!((changes(&s, account), unpushed(&s, account)), (Vec::new(), 0));
    // A quick run reads the Archive, where the server last had it, so that a window can name it.
    let request = jmap_mail_request(&s, account, false, &[], false).unwrap();
    assert!(matches!(&request.scope, JmapScope::Light { also } if also.as_slice() == ["Mar"]), "{:?}", request.scope);
    // A message of that Message-ID in another mailbox is not the one: the server last had it in the Archive.
    let page = window_of("Min", vec![listed("Min", "E1-copy", FlagStatus::None)], false);
    let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], ..result(account, Vec::new()) }).unwrap();
    assert_eq!(answer["new"], 1);
    assert!(matches!(s.mailbox.queued.as_slice(), [Queued::DeleteMessage { id, .. }] if id.is_empty()));
    // The window of the Archive lists it under the id the server gave it: the delete is given that id, and the message
    // is not brought down.
    let before = s.mailbox.messages.len();
    let page = window_of("Mar", vec![listed("Mar", "E1-new", FlagStatus::None)], false);
    let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], ..result(account, Vec::new()) }).unwrap();
    assert_eq!((answer["new"].as_u64(), s.mailbox.messages.len()), (Some(0), before));
    assert_eq!(changes(&s, account), [JmapChange::Destroy { id: "E1-new".into() }]);
    assert_eq!(unpushed(&s, account), 1);
    // Made: nothing waits.
    apply_jmap_mail(&mut s, result(account, vec![done(JmapChange::Destroy { id: "E1-new".into() }, ok(""))])).unwrap();
    assert!(s.mailbox.queued.is_empty());
}

#[test]
fn the_deletes_that_wait_for_an_id_are_one_for_each_message_and_few() {
    let (mut s, account, _, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    let unnamed = |s: &mut Session, n: usize| -> Id {
        let m = add(s, archive, &format!("U{n}"), &format!("Unnamed {n}"));
        let server = s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap();
        server.id = String::new();
        m
    };
    // Two messages that have no id are two deletes, not one; a message of an account that is not a JMAP account's is none.
    let (a, b) = (unnamed(&mut s, 1), unnamed(&mut s, 2));
    s.run("mail.deletePermanently", &json!({"ids": [a, b]})).unwrap();
    assert_eq!(s.mailbox.queued.len(), 2);
    // No Message-ID, no delete: nothing could name it. (Nor one that is only brackets.)
    let c = unnamed(&mut s, 3);
    s.mailbox.message_mut(c).unwrap().message_id = String::new();
    s.run("mail.deletePermanently", &json!({"ids": [c]})).unwrap();
    let d = unnamed(&mut s, 3);
    s.mailbox.message_mut(d).unwrap().message_id = " <> ".into();
    s.run("mail.deletePermanently", &json!({"ids": [d]})).unwrap();
    assert_eq!(s.mailbox.queued.len(), 2);
    // Two messages of one Message-ID are two deletes, and what is kept of each is what names it and shows it again, no more.
    let (e, f) = (unnamed(&mut s, 3), unnamed(&mut s, 3));
    for m in [e, f] {
        let m = s.mailbox.message_mut(m).unwrap();
        m.message_id = "twin@example.org".into();
        m.body.text = "x".repeat(10_000);
        m.body.html = Some("<p>x</p>".into());
        m.attachments.push(mailcraft_model::Attachment::new("a.pdf", "application/pdf", vec![0; 1_000]));
        m.headers.push(("X-Spam".into(), "no".into()));
    }
    s.run("mail.deletePermanently", &json!({"ids": [e, f]})).unwrap();
    assert_eq!(s.mailbox.queued.len(), 4);
    for q in &s.mailbox.queued[2..] {
        let Queued::DeleteMessage { id, kept: Some(kept), .. } = q else { panic!("{q:?}") };
        assert!(id.is_empty() && kept.body.text.chars().count() == 300 && kept.body.html.is_none());
        assert!(kept.attachments.is_empty() && kept.headers.is_empty() && kept.unfetched && kept.attachments_on_server);
        assert_eq!((kept.message_id.as_str(), kept.server.as_ref().map(|server| server.folder.as_str())), ("twin@example.org", Some("Mar")));
    }
    s.mailbox.queued.truncate(2);
    // The oldest is let go of beyond the most.
    let more: Vec<Id> = (4..4 + mailcraft_model::MAX_UNNAMED_DELETES).map(|n| unnamed(&mut s, n)).collect();
    s.run("mail.deletePermanently", &json!({"ids": more})).unwrap();
    let names: Vec<String> = s
        .mailbox
        .queued
        .iter()
        .filter_map(|q| if let Queued::DeleteMessage { kept: Some(kept), .. } = q { Some(kept.subject.clone()) } else { None })
        .collect();
    assert_eq!(names.len(), mailcraft_model::MAX_UNNAMED_DELETES);
    assert!(!names.contains(&"Unnamed 1".to_string()) && !names.contains(&"Unnamed 2".to_string()) && names.contains(&"Unnamed 5".to_string()));
}

#[test]
fn a_message_moved_twice_before_it_is_found_is_looked_for_where_the_server_has_it() {
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    let deleted = folder_of(&s, account, "Mtr");
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    // Moved again, to Deleted Items, while no window has shown it: it waits (nothing can be sent for it), and a quick run
    // looks in the Archive, where the server has it, and not in Deleted Items, where it lies here.
    s.run("mail.move", &json!({"ids": [m], "folder": deleted})).unwrap();
    assert_eq!(changes(&s, account), []);
    let request = jmap_mail_request(&s, account, false, &[], false).unwrap();
    assert!(matches!(&request.scope, JmapScope::Light { also } if also.as_slice() == ["Mar"]), "{:?}", request.scope);
    // A copy with its Message-ID in Deleted Items is not it: that window does not adopt the message.
    let before = s.mailbox.messages.len();
    let page = window_of("Mtr", vec![listed("Mtr", "E1-copy", FlagStatus::None)], false);
    let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], ..result(account, Vec::new()) }).unwrap();
    assert_eq!((answer["new"].as_u64(), s.mailbox.messages.len()), (Some(1), before + 1));
    assert_eq!(s.mailbox.message(m).unwrap().server.as_ref().map(|server| (server.id.as_str(), server.folder.as_str())), Some(("", "Mar")));
    // The Archive's lists it: it is adopted, stays where the owner put it, and its move goes up under the new id.
    let page = window_of("Mar", vec![listed("Mar", "E1-new", FlagStatus::None)], false);
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], ..result(account, Vec::new()) }).unwrap();
    assert_eq!(s.mailbox.message(m).unwrap().folder, deleted);
    assert_eq!(changes(&s, account), [JmapChange::Move { message: m, id: "E1-new".into(), from: "Mar".into(), to: "Mtr".into() }]);
}

#[test]
fn a_message_the_window_lists_under_its_id_is_not_taken_for_the_one_a_new_id_is_for() {
    let (mut s, account, m, _) = setup();
    // Two messages of one Message-ID in the Inbox (delivered twice), E1 here and E9 new: a complete window lists both.
    let page = window_of("Min", vec![listed("Min", "E1", FlagStatus::None), listed("Min", "E9", FlagStatus::None)], true);
    let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!(answer["new"], 1, "{answer}");
    let ids: Vec<String> = s.mailbox.messages.values().filter_map(|m| m.server.as_ref().map(|server| server.id.clone())).collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert!(ids.contains(&"E1".to_string()) && ids.contains(&"E9".to_string()));
    assert_eq!(s.mailbox.message(m).unwrap().server.as_ref().map(|server| server.id.as_str()), Some("E1"));
}

#[test]
fn a_move_that_may_have_been_made_is_not_bound_to_a_message_that_was_not_received_when_it_was() {
    for (received, adopted) in [(NOW, true), (NOW + 5, false)] {
        let (mut s, account, m, _) = setup();
        let archive = folder_of(&s, account, "Mar");
        s.mailbox.message_mut(m).unwrap().received = NOW;
        s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
        // The move is asked for and nothing is heard of it; the Archive's window lists an id nobody knows, with the same Message-ID.
        let mut there = listed("Mar", "E9", FlagStatus::None);
        there.received = received;
        let page = window_of("Mar", vec![there], false);
        let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], ..result(account, Vec::new()) }).unwrap();
        let id = s.mailbox.message(m).unwrap().server.as_ref().map(|server| server.id.clone()).unwrap_or_default();
        assert_eq!((id == "E9", answer["new"].as_u64()), (adopted, Some(if adopted { 0 } else { 1 })), "received {received}");
        if !adopted {
            // Still to be moved.
            assert_eq!(changes(&s, account).len(), 1);
        }
    }
}

#[test]
fn a_delete_that_waits_for_an_id_names_the_message_that_was_received_when_it_was_and_no_other() {
    for (other_received, named) in [(NOW + 60, Some("E1-new")), (NOW, None)] {
        let (mut s, account, m, _) = setup();
        let archive = folder_of(&s, account, "Mar");
        s.mailbox.message_mut(m).unwrap().received = NOW;
        s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
        let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
        apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
        s.run("mail.deletePermanently", &json!({"ids": [m]})).unwrap();
        // Another message of that Message-ID (a copy, a delivery twice) is listed first; the one the owner deleted, after it.
        let mut copy = listed("Mar", "E9", FlagStatus::None);
        copy.received = other_received;
        let mut mine = listed("Mar", "E1-new", FlagStatus::None);
        mine.received = NOW;
        let page = window_of("Mar", vec![copy, mine], false);
        let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], ..result(account, Vec::new()) }).unwrap();
        match named {
            // Only one of them can be it: that one is destroyed, and the other is mail.
            Some(id) => {
                assert_eq!(answer["new"], 1, "{answer}");
                assert_eq!(changes(&s, account), [JmapChange::Destroy { id: id.into() }]);
                assert!(s.mailbox.messages.values().any(|m| m.server.as_ref().is_some_and(|server| server.id == "E9")));
            }
            // Two of them could be: neither is taken for it, and nothing is destroyed.
            None => {
                assert_eq!(answer["new"], 2, "{answer}");
                assert_eq!(changes(&s, account), []);
            }
        }
    }
}

#[test]
fn a_refused_delete_of_a_message_that_was_named_late_puts_it_back_where_it_is() {
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
    apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
    s.run("mail.deletePermanently", &json!({"ids": [m]})).unwrap();
    let page = window_of("Mar", vec![listed("Mar", "E1-new", FlagStatus::None)], false);
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], ..result(account, Vec::new()) }).unwrap();
    let destroy = JmapChange::Destroy { id: "E1-new".into() };
    assert_eq!(changes(&s, account), std::slice::from_ref(&destroy));
    // The server will not: the message is back, in the Archive, as the server has it, and the delete is said.
    let answer = apply_jmap_mail(&mut s, result(account, vec![done(destroy, refused("forbidden"))])).unwrap();
    assert_eq!(answer["errors"], json!(["Couldn't delete 'Invoice 42' for good: the server doesn't allow it."]));
    let back = s.mailbox.messages.values().find(|m| m.subject == "Invoice 42").unwrap();
    assert_eq!((back.folder, back.server.as_ref().map(|server| (server.id.as_str(), server.folder.as_str()))), (archive, Some(("E1-new", "Mar"))));
    assert!(s.mailbox.queued.is_empty() && changes(&s, account).is_empty());
}

#[test]
fn a_message_moved_here_that_a_complete_window_of_where_it_was_does_not_list_is_not_swept() {
    let (mut s, account, m, _) = setup();
    let archive = folder_of(&s, account, "Mar");
    // Moved here, and the server did it (the answer was lost): the Inbox, listed to its end, does not have it.
    s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
    let inbox = window_of("Min", Vec::new(), true);
    let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![inbox], whole: true, ..result(account, Vec::new()) }).unwrap();
    assert_eq!((answer["gone"].as_u64(), answer["removedMessages"].as_u64()), (Some(0), Some(0)), "{answer}");
    let here = s.mailbox.message(m).expect("not swept");
    assert_eq!(
        (here.folder, here.server.as_ref().map(|server| (server.id.as_str(), server.folder.as_str(), server.gone))),
        (archive, Some(("", "Mar", false)))
    );
    // Nothing can be sent for it, and the end of the run says so; the Archive's window, cut short, may not show it yet,
    // and the next run, whole again, still does not sweep it.
    assert_eq!(changes(&s, account), []);
    assert!(s.sync_status.contains("1 moved message can't be found on the server yet"), "{}", s.sync_status);
    let other = window_of("Mar", vec![GraphMessage { message_id: "<other@example.org>".into(), ..listed("Mar", "E7", FlagStatus::None) }], false);
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![other, window_of("Min", Vec::new(), true)], whole: true, ..result(account, Vec::new()) })
        .unwrap();
    assert!(s.mailbox.message(m).is_some());
    // The window that reaches it finds it again.
    let page = window_of("Mar", vec![listed("Mar", "E1-new", FlagStatus::None)], false);
    apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], ..result(account, Vec::new()) }).unwrap();
    let here = s.mailbox.message(m).unwrap();
    assert_eq!((here.folder, here.server.as_ref().map(|server| (server.id.as_str(), server.folder.as_str()))), (archive, Some(("E1-new", "Mar"))));
    assert!(!s.sync_status.contains("moved message"), "{}", s.sync_status);
    // One that was never moved is gone, all the same, when a complete window does not list it.
    let (mut s, account, m, _) = setup();
    let answer =
        apply_jmap_mail(&mut s, JmapMailResult { pages: vec![window_of("Min", Vec::new(), true)], whole: true, ..result(account, Vec::new()) })
            .unwrap();
    assert_eq!(answer["removedMessages"], 1, "{answer}");
    assert!(s.mailbox.message(m).is_none());
}

#[test]
fn a_new_id_is_not_taken_for_a_message_that_waits_when_another_new_one_could_be_it_too() {
    for (second_received, adopted) in [(NOW, false), (NOW + 60, true)] {
        let (mut s, account, m, _) = setup();
        let archive = folder_of(&s, account, "Mar");
        s.mailbox.message_mut(m).unwrap().received = NOW;
        s.run("mail.move", &json!({"ids": [m], "folder": archive})).unwrap();
        let sent = jmap_mail_request(&s, account, false, &[], false).unwrap().changes;
        apply_jmap_mail(&mut s, result(account, vec![done(sent[0].clone(), ok(""))])).unwrap();
        // The Archive lists two messages of the Message-ID that the one moved had, under ids nobody here has.
        let (mut first, mut second) = (listed("Mar", "E8", FlagStatus::None), listed("Mar", "E9", FlagStatus::None));
        (first.received, second.received) = (NOW, second_received);
        let page = window_of("Mar", vec![first, second], false);
        let answer = apply_jmap_mail(&mut s, JmapMailResult { pages: vec![page], ..result(account, Vec::new()) }).unwrap();
        let id = s.mailbox.message(m).and_then(|m| m.server.as_ref().map(|server| server.id.clone())).unwrap_or_default();
        // Two that could be it: neither is; one that can: that one.
        assert_eq!((id.as_str(), answer["new"].as_u64()), if adopted { ("E8", Some(1)) } else { ("", Some(2)) }, "received {second_received}");
    }
}
