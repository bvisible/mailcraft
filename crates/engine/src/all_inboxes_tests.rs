// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Toutes les boîtes » (`view.allInboxes`) and « Afficher les traités »
// //// Neoffice — (`view.showDone`), which the list of a hosting page's tab offers.
//! « Toutes les boîtes » lists the Inbox of every mailbox of the person's own on a server, not a shared one's nor the one kept on
//! this computer alone; picking a folder leaves it; Mark All as Read takes every Inbox it lists; a gesture on its mail acts on
//! each message's own account. « Afficher les traités » lists an Inbox's account's Archive beside it. Each is another list of
//! the same folder: what is kept selected and the rows built are its own.

use mailcraft_model::{Account, AccountKind, FolderKind, Id, Message};
use serde_json::json;

use crate::Session;

const NOW: i64 = 1_791_993_600;

/// The sample's mailbox as a server's, beside a second one of the person's with three messages in its Inbox, and a mailbox
/// shared with them through the first, with one.
fn session() -> (Session, Id, Id, Id) {
    let mut mb = crate::sample::mailbox(NOW, 0);
    let ada = mb.accounts[0].id;
    mb.accounts[0].kind = AccountKind::Jmap;
    mb.accounts[0].email = "ada@example.org".into();
    let team = mb.add_account(Account { kind: AccountKind::Jmap, email: "team@example.org".into(), ..Default::default() });
    let shared = mb.add_account(Account { kind: AccountKind::Jmap, email: "desk@example.org".into(), shared_of: Some(ada), ..Default::default() });
    for (account, n) in [(team, 3), (shared, 1)] {
        let inbox = mb.folder_of_kind(account, FolderKind::Inbox).unwrap();
        for i in 0..n {
            let m = Message { folder: inbox, subject: format!("Note {i}"), date: NOW - 3_600 * i, received: NOW - 3_600 * i, ..Default::default() };
            mb.add_message(m).unwrap();
        }
    }
    let mut s = Session::at(mb, NOW, 0);
    s.keeps_local = false;
    s.run("folder.select", &json!({"kind": "inbox", "account": ada})).unwrap();
    (s, ada, team, shared)
}

fn inbox(s: &Session, account: Id) -> Id {
    s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap()
}

fn listed(s: &mut Session) -> Vec<Id> {
    s.rows().iter().flat_map(|r| r.members.clone()).collect()
}

fn account_of(s: &Session, id: Id) -> Id {
    s.mailbox.message(id).unwrap().account
}

#[test]
fn all_inboxes_lists_every_inbox_of_the_persons_own_and_a_folder_picked_leaves_it() {
    let (mut s, ada, team, shared) = session();
    let local = s.mailbox.accounts.iter().find(|a| a.kind == AccountKind::Local).map(|a| a.id);
    assert_eq!(s.own_inboxes(), vec![inbox(&s, ada), inbox(&s, team)], "not the shared mailbox's, nor one kept here alone ({local:?})");
    let alone = listed(&mut s).len();
    let answer = s.run("view.allInboxes", &json!({})).unwrap();
    assert_eq!(answer["folder"], json!(inbox(&s, ada)));
    assert_eq!(s.all_inboxes(), Some(vec![inbox(&s, ada), inbox(&s, team)]));
    let ids = listed(&mut s);
    assert_eq!(ids.len(), alone + 3);
    assert!(ids.iter().any(|id| account_of(&s, *id) == team) && ids.iter().all(|id| account_of(&s, *id) != shared));
    // Its unread mail is every Inbox's.
    let counts = s.folder_counts();
    assert_eq!(answer["unread"], json!(counts.unread(inbox(&s, ada)) + counts.unread(inbox(&s, team))));
    // Picking a folder leaves it, the Inbox it was opened on too.
    s.run("folder.select", &json!({"id": inbox(&s, ada)})).unwrap();
    assert!(s.all_inboxes().is_none());
    assert_eq!(listed(&mut s).len(), alone);
    // As does a folder changed by anything else: it lists the Inbox it was opened on only while that is in view.
    s.run("view.allInboxes", &json!({})).unwrap();
    s.view.folder = inbox(&s, team);
    assert!(s.all_inboxes().is_none());
}

#[test]
fn mark_all_as_read_takes_every_inbox_and_gestures_act_on_each_messages_own_account() {
    let (mut s, ada, team, _) = session();
    s.run("view.allInboxes", &json!({})).unwrap();
    s.run("mail.markAllRead", &json!({})).unwrap();
    for account in [ada, team] {
        assert!(s.mailbox.messages_in(inbox(&s, account)).all(|m| m.read), "{account}");
    }
    // One message of each mailbox, archived and deleted together: each in its own account's folder.
    let ids = listed(&mut s);
    let a = *ids.iter().find(|id| account_of(&s, **id) == ada).unwrap();
    let t = *ids.iter().find(|id| account_of(&s, **id) == team).unwrap();
    s.run("mail.archive", &json!({"ids": [a, t]})).unwrap();
    for (id, account) in [(a, ada), (t, team)] {
        assert_eq!(s.mailbox.message(id).unwrap().folder, s.mailbox.folder_of_kind(account, FolderKind::Archive).unwrap());
    }
    let ids = listed(&mut s);
    let a = *ids.iter().find(|id| account_of(&s, **id) == ada).unwrap();
    let t = *ids.iter().find(|id| account_of(&s, **id) == team).unwrap();
    s.run("mail.delete", &json!({"ids": [a, t]})).unwrap();
    for (id, account) in [(a, ada), (t, team)] {
        assert_eq!(s.mailbox.message(id).unwrap().folder, s.mailbox.folder_of_kind(account, FolderKind::Deleted).unwrap());
    }
    s.run("mail.markUnread", &json!({"ids": [ids[0]]})).unwrap();
    assert!(!s.mailbox.message(ids[0]).unwrap().read);
}

#[test]
fn show_done_lists_the_archive_beside_an_inbox_and_both_are_lists_of_their_own() {
    let (mut s, ada, team, _) = session();
    let first = listed(&mut s)[0];
    s.run("mail.archive", &json!({"ids": [first]})).unwrap();
    assert!(!listed(&mut s).contains(&first));
    assert_eq!(s.run("view.showDone", &json!({})).unwrap(), json!({"value": true}));
    assert!(listed(&mut s).contains(&first), "listed with the Inbox's");
    // Every Inbox's Archive in « Toutes les boîtes ».
    s.run("view.allInboxes", &json!({})).unwrap();
    let t = *listed(&mut s).iter().find(|id| account_of(&s, **id) == team).unwrap();
    s.run("mail.archive", &json!({"ids": [t]})).unwrap();
    assert!(listed(&mut s).contains(&t) && listed(&mut s).contains(&first));
    // Not in another folder.
    s.run("folder.select", &json!({"kind": "sent", "account": ada})).unwrap();
    assert!(!listed(&mut s).contains(&first));
    s.run("folder.select", &json!({"kind": "inbox", "account": ada})).unwrap();
    assert_eq!(s.run("view.showDone", &json!({"value": false})).unwrap(), json!({"value": false}));
    assert!(!listed(&mut s).contains(&first));
    // Each is another list of the same folder.
    let alone = s.list_of();
    s.run("view.allInboxes", &json!({})).unwrap();
    assert_ne!(s.list_of(), alone);
    s.run("view.showDone", &json!({})).unwrap();
    let both = s.list_of();
    s.run("view.showDone", &json!({})).unwrap();
    assert_ne!(s.list_of(), both);
}

#[test]
fn in_conversations_a_message_listed_as_done_is_listed_once_and_each_mailbox_has_the_rest_of_its_own() {
    let (mut s, ada, team, _) = session();
    s.view.conversations = true;
    s.run("view.allInboxes", &json!({})).unwrap();
    // A message of each mailbox done, with « Afficher les traités »: once each, in a row of its own.
    let ids = listed(&mut s);
    let a = *ids.iter().find(|id| account_of(&s, **id) == ada).unwrap();
    let t = *ids.iter().find(|id| account_of(&s, **id) == team).unwrap();
    s.run("mail.archive", &json!({"ids": [a, t]})).unwrap();
    s.run("view.showDone", &json!({"value": true})).unwrap();
    let rows = s.rows();
    for id in [a, t] {
        let times = rows.iter().map(|r| r.members.iter().chain(&r.others).filter(|m| **m == id).count()).sum::<usize>();
        assert_eq!(times, 1, "{id} listed once");
    }
    // The rest of a conversation of the second mailbox, in its Sent Items, is its row's.
    let sent = s.mailbox.folder_of_kind(team, FolderKind::Sent).unwrap();
    let first = s.mailbox.message(t).unwrap().clone();
    let reply = Message {
        folder: sent,
        subject: format!("RE: {}", first.subject),
        in_reply_to: first.message_id.clone(),
        date: NOW,
        received: NOW,
        ..Default::default()
    };
    let reply = s.mailbox.add_message(reply).unwrap();
    s.run("view.showDone", &json!({"value": false})).unwrap();
    s.run("mail.move", &json!({"ids": [t], "folder": inbox(&s, team)})).unwrap();
    let rows = s.rows();
    let row = rows.iter().find(|r| r.members.contains(&t)).unwrap();
    assert!(row.others.contains(&reply) || row.members.contains(&reply), "the reply in Sent Items is its conversation's");
}

#[test]
fn a_list_asked_of_a_named_folder_is_that_folders_alone_and_the_desktop_offers_neither_view() {
    let (mut s, ada, team, _) = session();
    s.run("view.allInboxes", &json!({})).unwrap();
    s.run("view.showDone", &json!({"value": true})).unwrap();
    // The Inbox the view was opened on, named: its own mail alone.
    let answer = s.run("mail.list", &json!({"folder": inbox(&s, ada), "limit": 500})).unwrap();
    let ids: Vec<Id> = answer["messages"].as_array().unwrap().iter().filter_map(|m| m["id"].as_u64()).collect();
    assert!(!ids.is_empty() && ids.iter().all(|id| account_of(&s, *id) == ada && s.mailbox.message(*id).unwrap().folder == inbox(&s, ada)));
    // And the window's view is as it was.
    assert!(s.all_inboxes().is_some() && s.view.show_done);
    let _ = team;
    // The program that keeps its mail: neither is offered.
    s.keeps_local = true;
    assert!(s.run("view.allInboxes", &json!({})).is_err());
    assert!(s.run("view.showDone", &json!({})).is_err());
}
