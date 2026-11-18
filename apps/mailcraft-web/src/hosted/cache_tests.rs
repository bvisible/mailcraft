// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of hosted/cache.rs.
//! The copy of a hosting page's mailboxes kept in the browser: whose it is (a key for each person and set of mailboxes,
//! every other copy deleted), what it holds (the newest bodies, the rest headers and preview, drafts whole, within its
//! bound), that a copy which can't be read, or holds other mailboxes, is ignored, that the window opens on it, and asks
//! what changed since when the copy has the state of every mailbox.
//! And (`--ignored`) what reading it costs for a mailbox of a few hundred messages.

use mailcraft_model::{Attachment, Body, FolderKind, Mailbox, Message};

/// The window of a run: each folder's newest messages it reads again.
const WINDOW: usize = 500;

use mailcraft_model::sync::{JmapMailState, ServerDraft, ServerMessage};

use super::{EVERY_MS, Writer, decode, encode, encode_within, holds, key, note_opened, resumes, snapshot, snapshot_within, stale};
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
    // A draft the server has stays (what changed since does not bring it again); one saved here since, that waits to go up, doesn't.
    let on_server = |id: &str| Some(ServerMessage { id: id.into(), ..ServerMessage::default() });
    let theirs = mb.add_message(Message { folder: drafts, draft: true, received: 3, server: on_server("Ed1"), ..Message::default() }).expect("draft");
    let saved = Some(ServerDraft { pending: true, ..ServerDraft::default() });
    let edited = mb
        .add_message(Message { folder: drafts, draft: true, received: 4, server: on_server("Ed2"), server_draft: saved, ..Message::default() })
        .expect("draft");
    let kept = snapshot_within(&mb, 3_500, super::BODIES, WINDOW);
    let whole: Vec<String> = kept.messages.values().filter(|m| !m.unfetched && m.folder != drafts).map(|m| m.subject.clone()).collect();
    assert_eq!(whole.len(), 3, "the newest three: {whole:?}");
    assert!(whole.iter().all(|s| ["Message 7", "Message 8", "Message 9"].contains(&s.as_str())));
    assert!(kept.messages.values().filter(|m| m.unfetched).all(|m| m.body.text.len() < 1_000), "a preview, not the body");
    // No draft that never went or waits to go up, nothing of the Outbox: they would go again.
    assert!(!kept.messages.contains_key(&draft) && !kept.messages.contains_key(&edited));
    assert!(waiting.is_none_or(|id| !kept.messages.contains_key(&id)));
    assert!(kept.messages.contains_key(&theirs));
    assert_eq!(kept.messages.len(), 11);
    // The mailbox in use is left as it is.
    assert!(mb.messages.values().all(|m| !m.unfetched) && mb.messages.contains_key(&draft));
}

#[test]
fn a_copy_with_the_state_of_every_mailbox_asks_what_changed_and_one_without_reads_them_all() {
    let mut mb = mailbox(&[jane(), team()], 1, 10);
    assert!(!resumes(&mb), "no state");
    let served: Vec<(u64, String)> = mb.accounts.iter().filter_map(|a| a.jmap.as_ref().map(|server| (a.id, server.session.clone()))).collect();
    let (first, second) = (served[0].clone(), served[1].clone());
    mb.jmap_states.push(JmapMailState { account: first.0, session: first.1.clone(), state: "7-12".into() });
    assert!(!resumes(&mb), "one mailbox without");
    mb.jmap_states.push(JmapMailState { account: second.0, session: "https://other.example.org/session".into(), state: "7-3".into() });
    assert!(!resumes(&mb), "a state of another server");
    mb.jmap_states.push(JmapMailState { account: second.0, session: second.1.clone(), state: "-".into() });
    assert!(!resumes(&mb), "a state that names none");
    mb.jmap_states.push(JmapMailState { account: second.0, session: second.1, state: "7-3".into() });
    assert!(resumes(&mb));
    // The state goes with the copy, and comes back with it.
    let kept = decode(&encode(&mb, WINDOW).expect("written"), &[jane(), team()]).expect("read");
    assert!(resumes(&kept) && kept.jmap_states == mb.jmap_states);
    assert!(!resumes(&Mailbox::default()), "no mailbox of a server");
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
    // Too heavy with every text: the texts of the oldest go (their headers and preview stay), the newest' stay.
    let bound = whole.len() / 4;
    let light = encode_within(&mb, u64::MAX, WINDOW, bound).expect("written with fewer texts");
    assert!(light.len() <= bound);
    let kept = decode(&light, &[jane()]).expect("read back");
    let whole_ones: Vec<i64> = kept.messages.values().filter(|m| !m.unfetched).map(|m| m.received).collect();
    let previews: Vec<i64> = kept.messages.values().filter(|m| m.unfetched).map(|m| m.received).collect();
    assert!(!whole_ones.is_empty() && !previews.is_empty(), "{} whole, {} previews", whole_ones.len(), previews.len());
    assert!(whole_ones.iter().min() > previews.iter().max(), "the newest keep their text");
    assert_eq!(kept.messages.len(), 50, "and every message is there");
    // Still too heavy: not written at all.
    assert!(encode_within(&mb, u64::MAX, WINDOW, 100).is_none());
    // The default bound holds a few hundred messages with their bodies.
    assert!(encode(&mailbox(&[jane()], 300, 2_000), WINDOW).is_some());
}

#[test]
fn the_files_kept_are_those_of_the_most_recently_opened_then_the_newest() {
    let mut mb = mailbox(&[jane()], 5, 1_000);
    let inbox = inbox_of(&mb);
    let ids: Vec<mailcraft_model::Id> =
        (0..4).map(|n| mb.add_message(heavy(inbox, n, 1_000, 1 << 20, 1_792_000_000 + n as i64)).expect("added")).collect();
    // Room for the files of two. By arrival, the two newest (3 and 2); but the oldest was opened last, and the second
    // oldest before it.
    mb.message_mut(ids[0]).expect("there").opened_at = 1_792_100_200;
    mb.message_mut(ids[1]).expect("there").opened_at = 1_792_100_100;
    let kept = snapshot(&mb, 2 << 20, WINDOW);
    let files = |n: usize| !kept.messages.get(&ids[n]).expect("kept").attachments.is_empty();
    assert_eq!((0..4).map(files).collect::<Vec<_>>(), [true, true, false, false]);
    for (n, id) in ids.iter().enumerate().skip(2) {
        let m = kept.messages.get(id).expect("kept");
        assert!(m.unfetched && m.files_on_server && text_whole(m, 1_000), "Heavy {n}: its text, its files to come");
    }
    // None opened: the newest.
    let mut fresh = mailbox(&[jane()], 5, 1_000);
    let inbox = inbox_of(&fresh);
    let ids: Vec<mailcraft_model::Id> =
        (0..4).map(|n| fresh.add_message(heavy(inbox, n, 1_000, 1 << 20, 1_792_000_000 + n as i64)).expect("added")).collect();
    let kept = snapshot(&fresh, 2 << 20, WINDOW);
    assert_eq!(ids.iter().map(|id| !kept.messages.get(id).expect("kept").attachments.is_empty()).collect::<Vec<_>>(), [false, false, true, true]);
}

#[test]
fn the_message_shown_is_marked_opened_as_it_comes_into_view_and_not_again_while_it_stays() {
    let mut mb = mailbox(&[jane()], 3, 10);
    let ids: Vec<mailcraft_model::Id> = mb.messages.values().map(|m| m.id).collect();
    let mut noted = None;
    assert!(!note_opened(&mut mb, &[], &mut noted, 1_792_000_000), "nothing shown");
    assert!(!note_opened(&mut mb, &ids[..2], &mut noted, 1_792_000_000), "two selected: none shown");
    assert!(note_opened(&mut mb, &ids[..1], &mut noted, 1_792_000_000));
    assert_eq!(mb.message(ids[0]).expect("there").opened_at, 1_792_000_000);
    // Left open: never marked again, however long (the copy isn't written again for it).
    assert!(!note_opened(&mut mb, &ids[..1], &mut noted, 1_792_003_600));
    // Another, then back to the first: marked again, a minute after the last time at the earliest.
    assert!(note_opened(&mut mb, &ids[1..2], &mut noted, 1_792_000_010));
    assert!(!note_opened(&mut mb, &ids[..1], &mut noted, 1_792_000_030), "not again so soon");
    assert!(!note_opened(&mut mb, &[], &mut noted, 1_792_000_040));
    assert!(note_opened(&mut mb, &ids[..1], &mut noted, 1_792_000_060));
    // Kept with the copy, for the next visit.
    let back = decode(&encode(&mb, WINDOW).expect("written"), &[jane()]).expect("read");
    assert_eq!(back.message(ids[0]).expect("there").opened_at, 1_792_000_060);
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

/// A message of `text` bytes of text and as much HTML, with one file of `file` bytes, received at `at`, in `folder`.
fn heavy(folder: mailcraft_model::Id, n: usize, text: usize, file: usize, at: i64) -> Message {
    Message {
        folder,
        subject: format!("Heavy {n}"),
        message_id: format!("heavy.{n}@example.org"),
        received: at,
        date: at,
        body: Body { text: "t".repeat(text), html: Some(format!("<p>{}</p>", "h".repeat(text))) },
        attachments: vec![Attachment::new(format!("plan-{n}.pdf"), "application/pdf", vec![7; file])],
        ..Message::default()
    }
}

/// The page's first mailbox's Inbox in `mb`.
fn inbox_of(mb: &Mailbox) -> mailcraft_model::Id {
    let account = mb.accounts.iter().find(|a| a.jmap.is_some()).map(|a| a.id).unwrap_or_default();
    mb.folder_of_kind(account, FolderKind::Inbox).expect("an Inbox")
}

/// Whether `m` has its whole text and HTML here (`text` bytes of each, as [`heavy`] makes them): opened, it shows at once.
fn text_whole(m: &Message, text: usize) -> bool {
    m.body.text.len() == text && m.body.html.as_ref().is_some_and(|h| h.len() == text + "<p></p>".len())
}

#[test]
fn a_heavy_message_keeps_its_text_and_only_its_files_are_left_out() {
    let mut mb = mailbox(&[jane()], 20, 2_000);
    let inbox = inbox_of(&mb);
    // The newest of the Inbox, with a file heavier than all the files a copy keeps.
    let big = mb.add_message(heavy(inbox, 1, 40_000, usize::try_from(super::BODIES).unwrap() + 1, 1_792_000_000)).expect("added");
    let kept = snapshot(&mb, super::BODIES, WINDOW);
    let m = kept.messages.get(&big).expect("kept");
    assert!(text_whole(m, 40_000), "its text and HTML whole: {} bytes of text", m.body.text.len());
    assert!(m.attachments.is_empty() && m.has_attachments(), "its file left out, its paperclip still");
    assert!(m.unfetched && m.files_on_server, "fetched again when opened, for its file");
    assert!(kept.messages.values().filter(|m| m.id != big).all(|m| !m.unfetched && m.body.text.len() == 2_000), "the others whole");
    // As written and read back at the next visit.
    let back = decode(&encode(&mb, WINDOW).expect("written"), &[jane()]).expect("read");
    let m = back.messages.values().find(|m| m.subject == "Heavy 1").expect("there");
    assert!(text_whole(m, 40_000) && m.attachments.is_empty() && m.unfetched);
}

#[test]
fn a_copy_too_heavy_leaves_out_files_the_least_recent_first_and_keeps_every_text() {
    let mut mb = mailbox(&[jane()], 50, 3_000);
    let inbox = inbox_of(&mb);
    for n in 0..10 {
        mb.add_message(heavy(inbox, n, 3_000, 1 << 20, 1_792_000_000 + n as i64)).expect("added");
    }
    let whole = encode_within(&mb, super::BODIES, WINDOW, usize::MAX).expect("written");
    // Room for all but about two of the files.
    let bound = whole.len() - (3 << 20);
    let light = encode_within(&mb, super::BODIES, WINDOW, bound).expect("written within its bound");
    assert!(light.len() <= bound);
    let kept = decode(&light, &[jane()]).expect("read back");
    let lost: Vec<&str> = kept.messages.values().filter(|m| m.body.text.len() != 3_000).map(|m| m.subject.as_str()).collect();
    assert!(lost.is_empty(), "every text kept, none lost: {lost:?}");
    let mut with_files: Vec<String> = kept.messages.values().filter(|m| !m.attachments.is_empty()).map(|m| m.subject.clone()).collect();
    with_files.sort();
    assert_eq!(with_files, (3..10).map(|n| format!("Heavy {n}")).collect::<Vec<_>>(), "the files of the newest");
}

/// What a copy of a few hundred messages costs: its size, writing it, and reading it back into the window's session; and
/// with heavy messages, how many open with their text at once and how many have their files.
#[test]
#[ignore = "a measure: cargo test -p mailcraft-web -- --ignored --nocapture what_a_copy_costs"]
fn what_a_copy_costs() {
    // (count, bytes, heavy messages with a text of 30 KB and a file of 4 MB, the newest)
    for (count, bytes, heavies) in [(300, 3_000, 0), (600, 6_000, 0), (300, 3_000, 12), (600, 6_000, 12)] {
        let mut mb = mailbox(&[jane(), team()], count, bytes);
        let inbox = inbox_of(&mb);
        for n in 0..heavies {
            mb.add_message(heavy(inbox, n, 30_000, 4 << 20, 1_792_000_000 + n as i64)).expect("added");
        }
        let at = std::time::Instant::now();
        let written = encode(&mb, WINDOW).expect("written");
        let wrote = at.elapsed();
        let at = std::time::Instant::now();
        let read = decode(&written, &[jane(), team()]).expect("read");
        let session = hosted_session(&[jane(), team()], Some(read));
        let opened = at.elapsed();
        let all = session.mailbox.messages.values();
        let texts = all.clone().filter(|m| m.body.text.len() >= bytes.min(30_000)).count();
        let heavy: Vec<&Message> = all.filter(|m| m.subject.starts_with("Heavy")).map(|m| m.as_ref()).collect();
        let at_once = heavy.iter().filter(|m| text_whole(m, 30_000)).count();
        let files = heavy.iter().filter(|m| !m.attachments.is_empty()).count();
        println!(
            "{count} messages of {bytes} bytes, {heavies} heavy: copy of {} KB, written in {wrote:?}, read and opened in {opened:?} ({} messages, {texts} with their text; heavy: {at_once} with their text, {files} with their file)",
            written.len() / 1024,
            session.mailbox.messages.len()
        );
    }
}
