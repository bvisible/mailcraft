// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A JMAP account (M6) is kept in step with its server, and sends nothing yet: a message written
//! from it is neither queued, where it would wait in its Outbox for ever, nor filed as sent, and
//! no Send/Receive is made of it until the engine has its commands, while the accounts beside it
//! go on as before. (Its invitations: `calendar/invitation.rs`.)
// //// Neoffice — this fork sends from a JMAP account and runs its Send/Receive (`mailcraft-jmap`, `cmd/jmap_mail.rs`): a
// message written from one waits in its Outbox for its run, and a Send/Receive of one is a JMAP run. The two tests of
// upstream's guards say so instead; the accounts beside it go on as before.

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

// //// Neoffice — (see the top: it waits in the JMAP account's Outbox for its run)
#[test]
fn a_message_written_from_a_jmap_account_waits_in_its_outbox_for_its_run() {
    let (mut s, local, jmap, _) = mailbox();
    // (To the local account's address: a local account's mail is delivered there.)
    s.run("mail.new", &json!({"account": jmap, "to": "ada@example.org", "subject": "Lunch"})).unwrap();
    // //// Neoffice — it waits in its Outbox for its run: queued, neither filed as sent nor delivered.
    let sent = s.run("compose.send", &json!({})).unwrap();
    assert_eq!((sent["queued"].as_bool(), sent["delivered"].as_u64()), (Some(true), Some(0)));
    assert_eq!([FolderKind::Outbox, FolderKind::Sent].map(|kind| count(&s, jmap, kind)), [1, 0]);
    assert_eq!(count(&s, local, FolderKind::Inbox), 0);
}

#[test]
fn an_account_that_sends_its_mail_still_queues_it() {
    let (mut s, _, _, imap) = mailbox();
    s.run("mail.new", &json!({"account": imap, "to": "bo@example.org", "subject": "Plans"})).unwrap();
    let sent = s.run("compose.send", &json!({})).unwrap();
    assert_eq!((sent["queued"].as_bool(), count(&s, imap, FolderKind::Outbox), count(&s, imap, FolderKind::Sent)), (Some(true), 1, 0));
}

// //// Neoffice — (see the top: a Send/Receive of a JMAP account is a JMAP run, `sync.jmapRequest`)
#[test]
fn a_send_receive_of_a_jmap_account_is_a_jmap_run() {
    let (mut s, _, jmap, imap) = mailbox();
    // With none named, the IMAP account's, though the JMAP account comes before it.
    assert_eq!(s.run("sync.request", &json!({})).unwrap()["account"]["id"].as_u64(), Some(imap));
    // //// Neoffice — named, a JMAP run: no IMAP request is made of settings it doesn't have.
    let named = s.run("sync.request", &json!({"account": jmap})).unwrap();
    assert_eq!((named["jmap"].as_bool(), named["account"].as_u64()), (Some(true), Some(jmap)));
    assert!(crate::cmd::account::sync_request(&s, jmap).is_err());
    // //// Neoffice — as the only account with a server, it is the one: a JMAP run.
    s.run("account.remove", &json!({"id": imap})).unwrap();
    assert_eq!(s.run("sync.request", &json!({})).unwrap()["jmap"].as_bool(), Some(true));
}
