// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A JMAP account (M6) is kept in step with its server, and sends nothing yet: a message written
//! from it is neither queued, where it would wait in its Outbox for ever, nor filed as sent, and
//! no Send/Receive is made of it until the engine has its commands, while the accounts beside it
//! go on as before. (Its invitations: `calendar/invitation.rs`.)
// //// Neoffice — so where the host can't reach a JMAP server (the desktop window, the command line); where it runs a JMAP
// account's Send/Receive (`Session::jmap_runs`, this fork's browser build), mail is sent from it, and its run is a JMAP
// one: the tests at the end.

use mailcraft_model::{Account, AccountKind, FolderKind, Id, JmapServer, Mailbox};
use serde_json::json;

use crate::Session;

/// A local account (`ada@example.org`), then a JMAP account, then an IMAP account; their ids.
fn mailbox() -> (Session, Id, Id, Id) {
    let mut mb = Mailbox::with_local_account("Ada Lovelace", "ada@example.org");
    let local = mb.accounts[0].id;
    let server = JmapServer { session: "https://jmap.example/.well-known/jmap".into(), bearer: true };
    let jmap = mb.add_account(Account { email: "ada@jmap.example".into(), kind: AccountKind::Jmap, jmap: Some(server), ..Default::default() });
    let mut s = Session::new(mb);
    let imap = s.run("account.add", &json!({"email": "ada@imap.example", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    (s, local, jmap, imap)
}

fn count(s: &Session, account: Id, kind: FolderKind) -> usize {
    s.mailbox.count(s.mailbox.folder_of_kind(account, kind).unwrap())
}

#[test]
fn a_message_written_from_a_jmap_account_isnt_left_in_its_outbox() {
    let (mut s, local, jmap, _) = mailbox();
    // (To the local account's address: a local account's mail is delivered there.)
    s.run("mail.new", &json!({"account": jmap, "to": "ada@example.org", "subject": "Lunch"})).unwrap();
    let why = s.run("compose.send", &json!({})).unwrap_err().to_string();
    assert_eq!(why, "MailCraft doesn't send mail from a JMAP account yet: send this message from another account, or save it as a draft");
    // Instead it stays as it is being written: nothing is queued, filed as sent or delivered.
    assert_eq!(s.compose.as_ref().map(|c| (c.account, c.to.as_str(), c.subject.as_str())), Some((jmap, "ada@example.org", "Lunch")));
    assert_eq!([FolderKind::Outbox, FolderKind::Sent].map(|kind| count(&s, jmap, kind)), [0, 0]);
    assert_eq!(count(&s, local, FolderKind::Inbox), 0);
    // Saved as a draft, it waits in the account's Drafts.
    let draft = s.run("compose.saveDraft", &json!({})).unwrap()["draft"].as_u64().unwrap();
    assert_eq!(s.mailbox.message(draft).map(|m| m.folder), s.mailbox.folder_of_kind(jmap, FolderKind::Drafts));
    // From another account it goes as any message does, and the draft with it.
    s.run("compose.set", &json!({"account": local})).unwrap();
    let sent = s.run("compose.send", &json!({})).unwrap();
    assert_eq!((sent["queued"].as_bool(), sent["delivered"].as_u64()), (Some(false), Some(1)));
    assert_eq!([FolderKind::Outbox, FolderKind::Sent, FolderKind::Drafts].map(|kind| count(&s, jmap, kind)), [0, 0, 0]);
}

#[test]
fn an_account_that_sends_its_mail_still_queues_it() {
    let (mut s, _, _, imap) = mailbox();
    s.run("mail.new", &json!({"account": imap, "to": "bo@example.org", "subject": "Plans"})).unwrap();
    let sent = s.run("compose.send", &json!({})).unwrap();
    assert_eq!((sent["queued"].as_bool(), count(&s, imap, FolderKind::Outbox), count(&s, imap, FolderKind::Sent)), (Some(true), 1, 0));
}

#[test]
// //// Neoffice — the fork's engine makes a JMAP account's run only where the host runs it (the tests at the end); elsewhere
// //// Neoffice — none is made: this test in place of upstream's `a_send_receive_of_a_jmap_account_is_its_own_request`.
fn no_send_receive_is_made_of_a_jmap_account_yet() {
    let (mut s, _, jmap, imap) = mailbox();
    // With none named, the IMAP account's, though the JMAP account comes before it.
    assert_eq!(s.run("sync.request", &json!({})).unwrap()["account"]["id"].as_u64(), Some(imap));
    // Named, it is refused with its reason: no IMAP request is made of settings it doesn't have.
    let why = s.run("sync.request", &json!({"account": jmap})).unwrap_err().to_string();
    assert_eq!(why, "ada@jmap.example is a JMAP account, and this version doesn't send and receive with one yet");
    assert!(crate::cmd::account::sync_request(&s, jmap).is_err());
    // As the only account with a server, it is refused for the same reason.
    s.run("account.remove", &json!({"id": imap})).unwrap();
    assert_eq!(s.run("sync.request", &json!({})).unwrap_err().to_string(), why);
}

// //// Neoffice — where the host runs a JMAP account's Send/Receive (`Session::jmap_runs`): mail is sent from it, and its run
// is a JMAP one.

/// The session of [`mailbox`], its host running a JMAP account's Send/Receive.
fn hosted() -> (Session, Id, Id, Id) {
    let (mut s, local, jmap, imap) = mailbox();
    s.jmap_runs = true;
    (s, local, jmap, imap)
}

#[test]
fn where_the_host_runs_it_a_message_written_from_a_jmap_account_waits_in_its_outbox_for_its_run() {
    let (mut s, local, jmap, _) = hosted();
    s.run("mail.new", &json!({"account": jmap, "to": "ada@example.org", "subject": "Lunch"})).unwrap();
    let sent = s.run("compose.send", &json!({})).unwrap();
    assert_eq!((sent["queued"].as_bool(), sent["delivered"].as_u64()), (Some(true), Some(0)));
    assert_eq!([FolderKind::Outbox, FolderKind::Sent].map(|kind| count(&s, jmap, kind)), [1, 0]);
    assert_eq!(count(&s, local, FolderKind::Inbox), 0);
}

#[test]
fn where_the_host_runs_it_a_send_receive_of_a_jmap_account_is_a_jmap_run() {
    let (mut s, _, jmap, imap) = hosted();
    // With none named, the IMAP account's, though the JMAP account comes before it.
    assert_eq!(s.run("sync.request", &json!({})).unwrap()["account"]["id"].as_u64(), Some(imap));
    // Named, a JMAP run: no IMAP request is made of settings it doesn't have.
    let named = s.run("sync.request", &json!({"account": jmap})).unwrap();
    assert_eq!((named["jmap"].as_bool(), named["account"].as_u64()), (Some(true), Some(jmap)));
    assert!(crate::cmd::account::sync_request(&s, jmap).is_err());
    // As the only account with a server, it is the one.
    s.run("account.remove", &json!({"id": imap})).unwrap();
    assert_eq!(s.run("sync.request", &json!({})).unwrap()["jmap"].as_bool(), Some(true));
}
