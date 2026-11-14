// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of hosted/cache.rs.
//! The copy of a hosting page's mailboxes kept in the browser: whose it is (a key for each person and set of mailboxes,
//! every other copy deleted), what it holds (the newest bodies, the rest headers and preview, drafts whole, within its
//! bound), that a copy which can't be read, or holds other mailboxes, is ignored, and that the window opens on it.
//! And (`--ignored`) what reading it costs for a mailbox of a few hundred messages.

use mailcraft_model::{Body, FolderKind, Mailbox, Message};

/// The window of a run: each folder's newest messages it reads again.
const WINDOW: usize = 500;

use super::{EVERY_MS, Writer, decode, encode, encode_within, holds, key, snapshot, stale};
use crate::hosted::{HostAccount, account_of, hosted_session};

fn jane() -> HostAccount {
    HostAccount {
        email: "Jane@Example.org".into(),
        name: "Jane".into(),
        session: "https://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session?account=jane%40example.org".into(),
        username: "jane@example.org".into(),
        ..HostAccount::default()
    }
}

fn team() -> HostAccount {
    HostAccount {
        email: "team@example.org".into(),
        session: "https://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session?account=team%40example.org".into(),
        ..HostAccount::default()
    }
}

/// The page's mailboxes set up, with `count` messages in the first one's Inbox, `bytes` of text each, a second apart.
fn mailbox(accounts: &[HostAccount], count: usize, bytes: usize) -> Mailbox {
    let mut session = hosted_session(accounts, None);
    let account = account_of(&session, &accounts[0].email).expect("set up");
    let inbox = session.mailbox.folder_of_kind(account, FolderKind::Inbox).expect("an Inbox");
    for i in 0..count {
        let m = Message {
            folder: inbox,
            subject: format!("Message {i}"),
            received: 1_791_000_000 + i as i64,
            date: 1_791_000_000 + i as i64,
            body: Body { text: "x".repeat(bytes), html: None },
            ..Message::default()
        };
        session.mailbox.add_message(m).expect("added");
    }
    session.mailbox
}

#[test]
fn a_copy_is_the_persons_and_these_mailboxes_and_every_other_is_deleted() {
    let (one, two) = (key(Some("jane@example.org"), &[jane(), team()]), key(Some("JANE@example.org "), &[team(), jane()]));
    assert_eq!(one, two, "the same whatever the order and the case");
    assert_ne!(one, key(Some("john@example.org"), &[jane(), team()]), "another person: another copy");
    assert_ne!(one, key(Some("jane@example.org"), &[jane()]), "another set of mailboxes: another copy");
    assert_ne!(one, key(None, &[jane(), team()]));
    assert!(one.starts_with("v1:") && !one.contains("example"), "no address in clear: {one}");
    // At each start, every other copy of this browser goes.
    let kept = vec![one.clone(), key(Some("john@example.org"), &[jane()]), "v0:old".to_string()];
    assert_eq!(stale(&kept, &one), kept[1..].to_vec());
    assert!(stale(std::slice::from_ref(&one), &one).is_empty());
}

#[test]
fn a_copy_is_read_only_when_it_holds_exactly_the_mailboxes_handed_over() {
    let both = mailbox(&[jane(), team()], 3, 10);
    assert!(holds(&both, &[team(), jane()]));
    assert!(!holds(&both, &[jane()]), "one more than handed over");
    assert!(!holds(&mailbox(&[jane()], 1, 10), &[jane(), team()]), "one less");
    let mut moved = team();
    moved.session = "https://other.example.org/session".into();
    assert!(!holds(&both, &[jane(), moved]), "a mailbox at another server");
    assert!(!holds(&both, &[]));
    // The window opens on it, its messages there; on a fresh set-up otherwise.
    let session = hosted_session(&[jane(), team()], Some(both.clone()));
    assert_eq!(session.mailbox.messages.len(), 3);
    assert!(account_of(&session, "jane@example.org").is_some() && account_of(&session, "TEAM@example.org").is_some());
    let fresh = hosted_session(&[jane()], Some(both));
    assert_eq!(fresh.mailbox.messages.len(), 0);
    assert!(account_of(&fresh, "jane@example.org").is_some());
}

#[test]
fn the_newest_bodies_are_kept_the_rest_fetched_again_and_nothing_that_would_be_done_twice() {
    let mut mb = mailbox(&[jane()], 10, 1_000);
    let account = mb.accounts.iter().find(|a| a.jmap.is_some()).map(|a| a.id).unwrap_or_default();
    let (drafts, outbox) = (mb.folder_of_kind(account, FolderKind::Drafts).expect("Drafts"), mb.folder_of_kind(account, FolderKind::Outbox));
    let draft = mb.add_message(Message { folder: drafts, draft: true, received: 1, ..Message::default() }).expect("draft");
    let waiting = outbox.map(|outbox| mb.add_message(Message { folder: outbox, received: 2, ..Message::default() }).expect("to send"));
    let kept = snapshot(&mb, 3_500, WINDOW);
    let whole: Vec<String> = kept.messages.values().filter(|m| !m.unfetched).map(|m| m.subject.clone()).collect();
    assert_eq!(whole.len(), 3, "the newest three: {whole:?}");
    assert!(whole.iter().all(|s| ["Message 7", "Message 8", "Message 9"].contains(&s.as_str())));
    assert!(kept.messages.values().filter(|m| m.unfetched).all(|m| m.body.text.len() < 1_000), "a preview, not the body");
    // No draft, nothing of the Outbox: a message sent since would go again.
    assert!(!kept.messages.contains_key(&draft));
    assert!(waiting.is_none_or(|id| !kept.messages.contains_key(&id)));
    assert_eq!(kept.messages.len(), 10);
    // The mailbox in use is left as it is.
    assert!(mb.messages.values().all(|m| !m.unfetched) && mb.messages.contains_key(&draft));
}

#[test]
fn of_each_folder_the_copy_keeps_the_window_a_run_reads_again() {
    let mb = mailbox(&[jane()], 30, 10);
    let kept = snapshot(&mb, u64::MAX, 12);
    let subjects: Vec<String> = kept.messages.values().map(|m| m.subject.clone()).collect();
    assert_eq!(subjects.len(), 12);
    assert!(subjects.iter().all(|s| s["Message ".len()..].parse::<usize>().is_ok_and(|n| n >= 18)), "the newest twelve: {subjects:?}");
}

#[test]
fn the_page_says_who_each_mailbox_is_over_what_the_copy_says() {
    let mut kept = mailbox(&[jane()], 1, 10);
    for a in kept.accounts.iter_mut().filter(|a| a.jmap.is_some()) {
        a.display_name = "Old name".into();
        a.username = "old".into();
    }
    let session = hosted_session(&[jane()], Some(kept));
    let jane_account = session.mailbox.accounts.iter().find(|a| a.jmap.is_some()).expect("jane");
    assert_eq!((jane_account.display_name.as_str(), jane_account.username.as_str()), ("Jane", "jane@example.org"));
    // A copy that holds a mailbox of another kind (IMAP, EWS…) is not this page's.
    let mut other = mailbox(&[jane()], 1, 10);
    other.accounts.push(mailcraft_model::Account { id: 999, email: "imap@example.org".into(), ..mailcraft_model::Account::default() });
    assert!(!holds(&other, &[jane()]));
}

#[test]
fn a_copy_stays_within_its_bound() {
    let mb = mailbox(&[jane()], 50, 20_000);
    let whole = encode_within(&mb, u64::MAX, WINDOW, usize::MAX).expect("written");
    // Too heavy with its bodies: written with none.
    let bound = whole.len() / 4;
    let light = encode_within(&mb, u64::MAX, WINDOW, bound).expect("written without bodies");
    assert!(light.len() <= bound);
    assert!(decode(&light, &[jane()]).is_some_and(|m| m.messages.values().all(|m| m.unfetched)));
    // Still too heavy: not written at all.
    assert!(encode_within(&mb, u64::MAX, WINDOW, 100).is_none());
    // The default bound holds a few hundred messages with their bodies.
    assert!(encode(&mailbox(&[jane()], 300, 2_000), WINDOW).is_some());
}

#[test]
fn a_copy_that_cant_be_read_is_ignored() {
    for junk in [&b""[..], b"{", b"not json at all", b"{\"version\": 99}", b"\x00\x01\x02"] {
        assert!(decode(junk, &[jane()]).is_none());
    }
    let good = encode(&mailbox(&[jane()], 5, 100), WINDOW).expect("written");
    let mut cut = good.clone();
    cut.truncate(good.len() / 2);
    assert!(decode(&cut, &[jane()]).is_none(), "a copy cut short");
    assert!(decode(&good, &[jane()]).is_some_and(|m| m.messages.len() == 5));
    assert!(decode(&good, &[team()]).is_none(), "another person's mailboxes");
}

#[test]
fn a_copy_is_written_when_the_mailbox_changed_and_not_more_often() {
    let mut w = Writer::new("v1:x".into(), 0.0, 0);
    assert!(!w.due(0, EVERY_MS * 5.0), "nothing changed since the window started: nothing written");
    assert!(!w.due(1, EVERY_MS - 1.0), "not at once");
    assert!(w.due(1, EVERY_MS));
    w.started(1, EVERY_MS);
    assert!(!w.due(2, EVERY_MS * 3.0), "one write at a time");
    w.writing = false;
    assert!(!w.due(1, EVERY_MS * 3.0), "unchanged");
    assert!(!w.due(2, EVERY_MS * 1.5), "too soon");
    assert!(w.due(2, EVERY_MS * 2.0));
}

/// What a copy of a few hundred messages costs: its size, writing it, and reading it back into the window's session.
#[test]
#[ignore = "a measure: cargo test -p mailcraft-web -- --ignored --nocapture what_a_copy_costs"]
fn what_a_copy_costs() {
    for (count, bytes) in [(300, 3_000), (600, 6_000)] {
        let mb = mailbox(&[jane(), team()], count, bytes);
        let at = std::time::Instant::now();
        let written = encode(&mb, WINDOW).expect("written");
        let wrote = at.elapsed();
        let at = std::time::Instant::now();
        let read = decode(&written, &[jane(), team()]).expect("read");
        let session = hosted_session(&[jane(), team()], Some(read));
        let opened = at.elapsed();
        println!(
            "{count} messages of {bytes} bytes: copy of {} KB, written in {wrote:?}, read and opened in {opened:?} ({} messages)",
            written.len() / 1024,
            session.mailbox.messages.len()
        );
    }
}
