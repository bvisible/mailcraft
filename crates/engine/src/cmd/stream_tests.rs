// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of the inbox's tabs by kind of sender.
//! The tabs of an inbox by kind of sender: `mail.setStreams` puts what a server said without an undo step, the three filters list
//! each stream's messages, and a message nobody said anything of is in the main tab.

use mailcraft_model::{Address, FolderKind, Id, Message};
use serde_json::json;

use crate::Session;
use crate::list;

fn local() -> (Session, Id) {
    let s = Session::new(mailcraft_model::Mailbox::with_local_account("Ada", "ada@example.org"));
    let inbox = s.mailbox.inbox().unwrap();
    (s, inbox)
}

fn mail(folder: Id, subject: &str, read: bool) -> Message {
    Message { folder, from: Address::new("", "someone@example.org"), subject: subject.into(), read, ..Default::default() }
}

fn subjects(s: &Session) -> Vec<String> {
    let mut found: Vec<String> = list::rows(s).iter().filter_map(|r| s.mailbox.message(r.id)).map(|m| m.subject.clone()).collect();
    found.sort();
    found
}

/// An inbox of five messages, said as a person's, two notifications and a newsletter; one is said nothing of.
fn said() -> (Session, [Id; 5]) {
    let (mut s, inbox) = local();
    let ids = [
        s.mailbox.add_message(mail(inbox, "lunch?", false)).unwrap(),
        s.mailbox.add_message(mail(inbox, "receipt", false)).unwrap(),
        s.mailbox.add_message(mail(inbox, "alert", true)).unwrap(),
        s.mailbox.add_message(mail(inbox, "digest", false)).unwrap(),
        s.mailbox.add_message(mail(inbox, "unsaid", false)).unwrap(),
    ];
    let answer = s
        .run(
            "mail.setStreams",
            &json!({"streams": [
                {"id": ids[0], "stream": "person"}, {"id": ids[1], "stream": "notification"},
                {"id": ids[2], "stream": "notification"}, {"id": ids[3], "stream": "newsletter"},
            ]}),
        )
        .unwrap();
    assert_eq!(answer["changed"], 4);
    (s, ids)
}

#[test]
fn set_streams_says_what_changed_and_whether_the_tabs_are_worth_showing() {
    let (mut s, inbox) = local();
    let id = s.mailbox.add_message(mail(inbox, "receipt", false)).unwrap();
    let answer = s.run("mail.setStreams", &json!({"streams": [{"id": id, "stream": "notification"}]})).unwrap();
    assert_eq!((answer["changed"].clone(), answer["tabs"].clone()), (json!(1), json!(true)));
    let again = s.run("mail.setStreams", &json!({"streams": [{"id": id, "stream": "notification"}]})).unwrap();
    assert_eq!(again["changed"], 0);
}

#[test]
fn set_streams_is_no_edit_of_the_owners() {
    let (mut s, inbox) = local();
    let id = s.mailbox.add_message(mail(inbox, "receipt", false)).unwrap();
    let before = s.mailbox.clone();
    s.run("mail.setStreams", &json!({"streams": [{"id": id, "stream": "newsletter"}]})).unwrap();
    // nothing waits for the server, and there is nothing to undo
    assert_eq!(s.mailbox.pending.len(), before.pending.len());
    assert!(s.run("edit.undo", &json!({})).is_err() || s.mailbox.message(id).unwrap().stream.is_some(), "an Undo must not take a stream back");
}

#[test]
fn set_streams_refuses_what_is_not_a_list_and_a_word_that_is_no_stream() {
    let (mut s, inbox) = local();
    let id = s.mailbox.add_message(mail(inbox, "receipt", false)).unwrap();
    assert!(s.run("mail.setStreams", &json!({})).is_err());
    assert!(s.run("mail.setStreams", &json!({"streams": "notification"})).is_err());
    assert!(s.run("mail.setStreams", &json!({"streams": [{"id": id, "stream": "promotion"}]})).is_err());
    assert!(s.run("mail.setStreams", &json!({"streams": [{"stream": "person"}]})).is_err());
    assert!(s.run("mail.setStreams", &json!({"streams": [], "extra": 1})).is_err());
    assert_eq!(s.mailbox.message(id).unwrap().stream, None);
    // a message that is not here is left, not an error: the server may know of one this window dropped
    assert_eq!(s.run("mail.setStreams", &json!({"streams": [{"id": 424_242, "stream": "person"}]})).unwrap()["changed"], 0);
}

#[test]
fn each_tab_lists_its_streams_and_the_main_one_has_what_nobody_said_anything_of() {
    let (mut s, _) = said();
    let filter = |s: &mut Session, name: &str| {
        s.run("view.filter", &json!({"value": name})).unwrap();
        subjects(s)
    };
    assert_eq!(filter(&mut s, "main"), ["lunch?", "unsaid"]);
    assert_eq!(filter(&mut s, "notifications"), ["alert", "receipt"]);
    assert_eq!(filter(&mut s, "newsletters"), ["digest"]);
    assert_eq!(filter(&mut s, "all").len(), 5);
}

#[test]
fn the_tabs_count_what_is_unread_in_each() {
    let (s, _) = said();
    let inbox = s.mailbox.inbox().unwrap();
    assert!(s.mailbox.has_streams(inbox));
    let counts = s.mailbox.stream_counts(inbox);
    use mailcraft_model::Stream::{Newsletter, Notification, Person};
    assert_eq!((counts.total(Person), counts.unread(Person)), (2, 2));
    assert_eq!((counts.total(Notification), counts.unread(Notification)), (2, 1));
    assert_eq!((counts.total(Newsletter), counts.unread(Newsletter)), (1, 1));
}

#[test]
fn archiving_a_whole_tab_takes_its_messages_and_only_theirs() {
    let (mut s, ids) = said();
    s.run("view.filter", &json!({"value": "notifications"})).unwrap();
    let listed: Vec<Id> = list::rows(&s).iter().map(|r| r.id).collect();
    assert_eq!(listed.len(), 2);
    s.run("mail.archive", &json!({"ids": listed})).unwrap();
    let account = s.mailbox.accounts[0].id;
    let archive = s.mailbox.folder_of_kind(account, FolderKind::Archive).unwrap();
    let inbox = s.mailbox.inbox().unwrap();
    assert_eq!((s.mailbox.message(ids[1]).unwrap().folder, s.mailbox.message(ids[2]).unwrap().folder), (archive, archive));
    assert_eq!((s.mailbox.message(ids[0]).unwrap().folder, s.mailbox.message(ids[3]).unwrap().folder), (inbox, inbox));
}

#[test]
fn archiving_chosen_messages_of_a_tab_takes_those_and_an_undo_brings_them_back() {
    let (mut s, ids) = said();
    s.run("view.filter", &json!({"value": "notifications"})).unwrap();
    s.run("mail.select", &json!({"id": ids[1]})).unwrap();
    s.run("mail.archive", &json!({})).unwrap();
    let inbox = s.mailbox.inbox().unwrap();
    assert_ne!(s.mailbox.message(ids[1]).unwrap().folder, inbox);
    assert_eq!(s.mailbox.message(ids[2]).unwrap().folder, inbox, "the other notification stays");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.mailbox.message(ids[1]).unwrap().folder, inbox);
}

#[test]
fn the_inbox_opens_on_the_main_tab_when_the_tabs_first_appear_and_the_owners_choice_stays_after() {
    let (mut s, inbox) = local();
    let id = s.mailbox.add_message(mail(inbox, "receipt", false)).unwrap();
    assert_eq!(s.view.filter, crate::Filter::All);
    s.run("mail.setStreams", &json!({"streams": [{"id": id, "stream": "notification"}]})).unwrap();
    assert_eq!(s.view.filter, crate::Filter::Main);
    // the owner picks Notifications, then the page says the streams again
    s.run("view.filter", &json!({"value": "notifications"})).unwrap();
    s.run("mail.setStreams", &json!({"streams": [{"id": id, "stream": "newsletter"}]})).unwrap();
    assert_eq!(s.view.filter, crate::Filter::Notifications);
    // and so does All, picked on purpose
    s.run("view.filter", &json!({"value": "all"})).unwrap();
    let other = s.mailbox.add_message(mail(inbox, "digest", false)).unwrap();
    s.run("mail.setStreams", &json!({"streams": [{"id": other, "stream": "newsletter"}]})).unwrap();
    assert_eq!(s.view.filter, crate::Filter::All);
}

#[test]
fn the_tabs_do_not_open_over_a_folder_that_is_not_the_inbox() {
    let (mut s, inbox) = local();
    let account = s.mailbox.accounts[0].id;
    let archive = s.mailbox.folder_of_kind(account, FolderKind::Archive).unwrap();
    let id = s.mailbox.add_message(mail(inbox, "receipt", false)).unwrap();
    s.run("folder.select", &json!({"id": archive})).unwrap();
    s.run("mail.setStreams", &json!({"streams": [{"id": id, "stream": "notification"}]})).unwrap();
    assert_eq!(s.view.filter, crate::Filter::All, "the archive is in view, not the inbox");
}

#[test]
fn another_folder_shows_all_of_its_messages_not_a_tab_of_the_inbox() {
    let (mut s, ids) = said();
    s.run("view.filter", &json!({"value": "notifications"})).unwrap();
    let account = s.mailbox.accounts[0].id;
    let archive = s.mailbox.folder_of_kind(account, FolderKind::Archive).unwrap();
    s.run("mail.archive", &json!({"ids": [ids[0]]})).unwrap();
    s.run("folder.select", &json!({"id": archive})).unwrap();
    assert_eq!(s.view.filter, crate::Filter::All);
    assert_eq!(subjects(&s), ["lunch?"]);
    // an ordinary filter set by the owner goes along, as it always did
    s.run("view.filter", &json!({"value": "unread"})).unwrap();
    let inbox = s.mailbox.inbox().unwrap();
    s.run("folder.select", &json!({"id": inbox})).unwrap();
    assert_eq!(s.view.filter, crate::Filter::Unread);
}

#[test]
fn clearing_the_selection_with_no_ids_leaves_nothing_ticked() {
    let (mut s, ids) = said();
    s.run("mail.select", &json!({"id": ids[1]})).unwrap();
    s.run("mail.select", &json!({"id": ids[2], "extend": true})).unwrap();
    assert_eq!(s.sel.messages.len(), 2);
    s.run("mail.select", &json!({"ids": []})).unwrap();
    assert!(s.sel.messages.is_empty());
}
