// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of stream.rs.
use crate::stream::MAX_STREAMS_PER_SAY;
use crate::*;

fn mailbox() -> (Mailbox, Id, Id) {
    let mb = Mailbox::with_local_account("Ada Lovelace", "ada@example.org");
    let inbox = mb.inbox().unwrap();
    let account = mb.accounts[0].id;
    (mb, account, inbox)
}

fn mail(folder: Id, subject: &str, read: bool) -> Message {
    Message { folder, subject: subject.into(), from: Address::new("Bob", "bob@example.org"), read, ..Default::default() }
}

#[test]
fn a_server_word_names_a_stream_in_any_case_and_an_unknown_word_names_none() {
    assert_eq!(Stream::from_code("person"), Some(Stream::Person));
    assert_eq!(Stream::from_code(" Notification "), Some(Stream::Notification));
    assert_eq!(Stream::from_code("NEWSLETTER"), Some(Stream::Newsletter));
    assert_eq!(Stream::from_code("promotion"), None);
    assert_eq!(Stream::from_code(""), None);
    assert!(Stream::ALL.iter().all(|s| Stream::from_code(s.code()) == Some(*s)));
}

#[test]
fn a_message_nobody_said_anything_of_is_a_persons() {
    let m = mail(1, "hello", false);
    assert_eq!(m.stream, None);
    assert_eq!(m.stream_or_person(), Stream::Person);
}

#[test]
fn set_streams_puts_what_the_server_said_and_counts_what_changed() {
    let (mut mb, _, inbox) = mailbox();
    let a = mb.add_message(mail(inbox, "receipt", false)).unwrap();
    let b = mb.add_message(mail(inbox, "weekly digest", false)).unwrap();
    let c = mb.add_message(mail(inbox, "lunch?", false)).unwrap();
    assert_eq!(mb.set_streams(&[(a, Stream::Notification), (b, Stream::Newsletter), (c, Stream::Person)]), 3);
    assert_eq!(mb.message(a).unwrap().stream, Some(Stream::Notification));
    assert_eq!(mb.message(b).unwrap().stream, Some(Stream::Newsletter));
    // said again, the same: nothing changes
    assert_eq!(mb.set_streams(&[(a, Stream::Notification), (b, Stream::Newsletter)]), 0);
    // said otherwise: it changes
    assert_eq!(mb.set_streams(&[(a, Stream::Person)]), 1);
}

#[test]
fn set_streams_leaves_a_message_that_is_not_here_and_a_draft() {
    let (mut mb, _, inbox) = mailbox();
    let draft = mb.add_message(Message { draft: true, ..mail(inbox, "unfinished", true) }).unwrap();
    assert_eq!(mb.set_streams(&[(9_999, Stream::Notification), (draft, Stream::Notification)]), 0);
    assert_eq!(mb.message(draft).unwrap().stream, None);
}

#[test]
fn set_streams_takes_a_bounded_number() {
    let (mut mb, _, inbox) = mailbox();
    let a = mb.add_message(mail(inbox, "a", false)).unwrap();
    let many: Vec<(Id, Stream)> = std::iter::repeat_n((a, Stream::Notification), MAX_STREAMS_PER_SAY + 10).collect();
    assert_eq!(mb.set_streams(&many), 1);
}

#[test]
fn the_tabs_are_worth_showing_only_in_an_inbox_whose_messages_have_streams() {
    let (mut mb, account, inbox) = mailbox();
    let archive = mb.folder_of_kind(account, FolderKind::Archive).unwrap();
    let in_inbox = mb.add_message(mail(inbox, "receipt", false)).unwrap();
    let in_archive = mb.add_message(mail(archive, "receipt", false)).unwrap();
    assert!(!mb.has_streams(inbox));
    mb.set_streams(&[(in_inbox, Stream::Notification), (in_archive, Stream::Notification)]);
    assert!(mb.has_streams(inbox));
    assert!(!mb.has_streams(archive), "the tabs are an inbox's");
}

#[test]
fn the_counts_say_each_streams_messages_and_unread_and_put_the_unsaid_with_the_persons() {
    let (mut mb, _, inbox) = mailbox();
    let person = mb.add_message(mail(inbox, "lunch?", false)).unwrap();
    let _unsaid = mb.add_message(mail(inbox, "hello", true)).unwrap();
    let receipt = mb.add_message(mail(inbox, "receipt", false)).unwrap();
    let alert = mb.add_message(mail(inbox, "alert", true)).unwrap();
    let digest = mb.add_message(mail(inbox, "digest", false)).unwrap();
    let _draft = mb.add_message(Message { draft: true, ..mail(inbox, "unfinished", false) }).unwrap();
    mb.set_streams(&[(person, Stream::Person), (receipt, Stream::Notification), (alert, Stream::Notification), (digest, Stream::Newsletter)]);
    let counts = mb.stream_counts(inbox);
    assert_eq!((counts.total(Stream::Person), counts.unread(Stream::Person)), (2, 1));
    assert_eq!((counts.total(Stream::Notification), counts.unread(Stream::Notification)), (2, 1));
    assert_eq!((counts.total(Stream::Newsletter), counts.unread(Stream::Newsletter)), (1, 1));
}

#[test]
fn a_stream_is_never_written_to_the_mailbox_file_and_an_old_file_still_opens() {
    let (mut mb, _, inbox) = mailbox();
    let id = mb.add_message(mail(inbox, "receipt", false)).unwrap();
    mb.set_streams(&[(id, Stream::Notification)]);
    let written = serde_json::to_string(mb.message(id).unwrap()).unwrap();
    assert!(!written.contains("stream") && !written.contains("notification"), "{written}");
    let back: Message = serde_json::from_str(&written).unwrap();
    assert_eq!(back.stream, None);
    // a message written before the field existed
    let old: Message = serde_json::from_str(r#"{"subject": "old", "read": true}"#).unwrap();
    assert_eq!((old.subject.as_str(), old.stream), ("old", None));
}
