// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A run that asks what changed since a state (`changes.rs`), over the stand-in: in how many requests, what it brings, and
//! what it does when the server can't say, or has no such thing.

use std::sync::atomic::AtomicBool;

use mailcraft_model::FlagStatus;
use mailcraft_model::sync::{JMAP_STATE_UNKNOWN, JmapKnown};

use super::*;
use crate::standin::{Effect, Fault, Profile, SESSION, StandIn, TOKEN};
use crate::{Auth, block_on};

const ME: &str = "ada@example.org";
const ID: u64 = 3;

struct Boxes {
    inbox: String,
    drafts: String,
    sent: String,
    archive: String,
}

/// A stand-in of `profile` with an Inbox, Drafts, Sent and an archive, and mail in the Inbox and the archive.
fn world(profile: Profile) -> (StandIn, Boxes) {
    let stand_in = StandIn::new();
    let boxes = stand_in.with(|d| {
        d.profile = profile;
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        let drafts = d.mailbox("Drafts", None, Some("drafts"));
        let sent = d.mailbox("Sent", None, Some("sent"));
        let archive = d.mailbox("Archive", None, Some("archive"));
        Boxes { inbox, drafts, sent, archive }
    });
    for n in 1..=3 {
        deliver(&stand_in, &boxes.inbox, &format!("inbox-{n}"), &[], n);
    }
    for n in 4..=5 {
        deliver(&stand_in, &boxes.archive, &format!("archive-{n}"), &["$seen"], n);
    }
    (stand_in, boxes)
}

fn mail(subject: &str) -> Vec<u8> {
    format!(
        "From: Zoe Park <zoe@example.org>\r\nTo: Ada <{ME}>\r\nSubject: {subject}\r\nMessage-ID: <{subject}@example.org>\r\nDate: Thu, 15 Oct 2026 09:30:00 +0200\r\n\r\nHello about {subject}.\r\n"
    )
    .into_bytes()
}

fn deliver(stand_in: &StandIn, mailbox: &str, subject: &str, keywords: &[&str], n: usize) -> String {
    stand_in.with(|d| d.deliver(mailbox, &mail(subject), keywords, &format!("2026-01-01T00:{:02}:{:02}Z", n / 60, n % 60)))
}

/// A run that reads every mailbox, as the owner's Send/Receive does.
fn everything() -> JmapMailRequest {
    JmapMailRequest {
        account: ID,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        window: 500,
        list_folders: true,
        scope: JmapScope::All,
        ..Default::default()
    }
}

/// A run that started by itself: the Inbox, or what changed since `state` when there is one, the engine having `known`.
fn quiet(state: &str, known: &[JmapKnown]) -> JmapMailRequest {
    JmapMailRequest {
        account: ID,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        window: 500,
        scope: JmapScope::Light { also: Vec::new() },
        known: known.to_vec(),
        email_state: state.into(),
        ..Default::default()
    }
}

fn run(stand_in: &StandIn, request: &JmapMailRequest) -> Vec<JmapMailResult> {
    let client = block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let mut pieces = Vec::new();
    block_on(sync_mail(&client, request, &AtomicBool::new(false), &mut |piece| pieces.push(piece)));
    let (last, rest) = pieces.split_last().expect("a run ends with a piece");
    assert!(!last.partial && last.pages.is_empty() && last.destroyed.is_empty(), "{last:?}");
    assert!(rest.iter().all(|piece| piece.partial && piece.email_state.is_empty()), "the state comes in the last piece");
    pieces
}

fn last(pieces: &[JmapMailResult]) -> &JmapMailResult {
    pieces.last().expect("a run ends with a piece")
}

fn pages(pieces: &[JmapMailResult]) -> Vec<&MessagePage> {
    pieces.iter().flat_map(|piece| piece.pages.iter()).collect()
}

fn destroyed(pieces: &[JmapMailResult]) -> Vec<&str> {
    pieces.iter().flat_map(|piece| piece.destroyed.iter().map(String::as_str)).collect()
}

/// The messages of the pages of `mailbox`: (id, subject).
fn in_box<'a>(pieces: &'a [JmapMailResult], mailbox: &str) -> Vec<(&'a str, &'a str)> {
    pages(pieces)
        .into_iter()
        .filter(|page| page.folder == mailbox)
        .flat_map(|page| &page.messages)
        .map(|m| (m.server.id.as_str(), m.subject.as_str()))
        .collect()
}

/// The state a whole run of every mailbox leaves, and what the engine has after it.
fn read_all(stand_in: &StandIn) -> (String, Vec<JmapKnown>) {
    let pieces = run(stand_in, &everything());
    let end = last(&pieces);
    assert!(end.whole, "{end:?}");
    assert!(JmapMailState::usable(&end.email_state), "a whole run of every mailbox leaves a state: {:?}", end.email_state);
    let known = pages(&pieces)
        .into_iter()
        .map(|page| JmapKnown { mailbox: page.folder.clone(), ids: page.messages.iter().map(|m| m.server.id.clone()).collect() })
        .collect();
    (end.email_state.clone(), known)
}

/// The requests a run of `request` makes, and the methods each calls: the pieces too.
fn counted(stand_in: &StandIn, request: &JmapMailRequest) -> (Vec<Vec<String>>, Vec<JmapMailResult>) {
    let before = stand_in.seen().len();
    let pieces = run(stand_in, request);
    let calls = stand_in.seen().into_iter().skip(before).filter(|seen| seen.path == "/jmap/api").map(|seen| seen.calls).collect();
    (calls, pieces)
}

fn names(calls: &[&str]) -> Vec<String> {
    calls.iter().map(|call| call.to_string()).collect()
}

#[test]
fn a_run_of_every_mailbox_leaves_the_state_and_a_run_of_the_inbox_leaves_none() {
    let (stand_in, _) = world(Profile::standard());
    let (state, _) = read_all(&stand_in);
    // (The state of the emails as the first window read them.)
    assert!(state.starts_with('s'), "{state}");
    // A run that reads the Inbox alone read only some of the mail: it has no state to leave.
    let pieces = run(&stand_in, &quiet("", &[]));
    assert!(last(&pieces).whole && last(&pieces).email_state.is_empty());
}

#[test]
fn nothing_changed_is_one_request_that_brings_the_same_state() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    let (calls, pieces) = counted(&stand_in, &quiet(&state, &known));
    assert_eq!(calls, [names(&["Mailbox/get", "Email/changes", "Email/get", "Email/get"])]);
    let end = last(&pieces);
    assert!(end.whole && end.errors.is_empty(), "{end:?}");
    assert_eq!(end.email_state, state);
    // The listing (a mailbox may be new), and nothing else.
    assert_eq!(pieces.len(), 2);
    assert!(pieces[0].listing.as_ref().is_some_and(|listing| listing.complete && listing.folders.iter().any(|f| f.id == boxes.inbox)));
    assert!(pages(&pieces).is_empty() && destroyed(&pieces).is_empty());
}

#[test]
fn what_was_made_changed_and_destroyed_elsewhere_comes_in_the_same_one_request() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    let inbox: Vec<String> = known.iter().find(|k| k.mailbox == boxes.inbox).unwrap().ids.clone();
    let (read_there, deleted_there) = (inbox[0].clone(), inbox[1].clone());
    let new = deliver(&stand_in, &boxes.inbox, "arrived", &[], 60);
    stand_in.with(|d| {
        assert!(d.set_keyword(&read_there, "$seen", true) && d.set_keyword(&read_there, "$flagged", true));
        assert!(d.remove(&deleted_there));
    });
    let (calls, pieces) = counted(&stand_in, &quiet(&state, &known));
    assert_eq!(calls.len(), 1, "{calls:?}");
    let end = last(&pieces);
    assert!(end.whole && JmapMailState::usable(&end.email_state) && end.email_state != state, "{end:?}");
    // The new one whole, the one read elsewhere as its marks alone (the engine has it), the deleted one gone.
    let mut inbox_page = in_box(&pieces, &boxes.inbox);
    inbox_page.sort();
    let mut expected = vec![(new.as_str(), "arrived"), (read_there.as_str(), "")];
    expected.sort();
    assert_eq!(inbox_page, expected);
    let marks = pages(&pieces).into_iter().flat_map(|page| &page.messages).find(|m| m.server.id == read_there).unwrap();
    assert_eq!((marks.server.read, marks.server.flag, marks.server.size), (true, FlagStatus::Flagged, 0));
    assert_eq!(destroyed(&pieces), [deleted_there.as_str()]);
    // No page is the whole of its mailbox: nothing is taken for gone that no destroy named.
    assert!(pages(&pieces).iter().all(|page| !page.afresh && page.removed.is_empty()));
    // And then nothing more: the next run asks from the new state.
    let (calls, again) = counted(&stand_in, &quiet(&end.email_state, &known));
    assert_eq!(calls.len(), 1);
    assert!(pages(&again).is_empty() && destroyed(&again).is_empty() && last(&again).email_state == end.email_state);
}

#[test]
fn a_message_moved_elsewhere_keeps_its_id_where_ids_survive_a_move_and_is_destroyed_and_made_again_where_they_name_its_place() {
    // A server whose ids survive a move: the message the engine has comes whole, in the mailbox it went to.
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    let moved = known.iter().find(|k| k.mailbox == boxes.inbox).unwrap().ids[0].clone();
    assert_eq!(stand_in.with(|d| d.move_to(&moved, &boxes.archive)), Some(moved.clone()));
    let pieces = run(&stand_in, &quiet(&state, &known));
    let archive = in_box(&pieces, &boxes.archive);
    assert_eq!(archive.len(), 1);
    // (Whole: the mailbox it left may be gone with the run's listing, and the message with it.)
    assert_eq!(archive[0].0, moved);
    assert!(archive[0].1.starts_with("inbox-"), "{archive:?}");
    assert!(destroyed(&pieces).is_empty() && last(&pieces).whole);

    // A server that names a message by where it lies: the old id is gone, a new one is made in the archive, whole.
    let (stand_in, boxes) = world(Profile::by_place());
    let (state, known) = read_all(&stand_in);
    let moved = known.iter().find(|k| k.mailbox == boxes.inbox).unwrap().ids[0].clone();
    let now = stand_in.with(|d| d.move_to(&moved, &boxes.archive)).unwrap();
    assert_ne!(now, moved);
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert_eq!(destroyed(&pieces), [moved.as_str()]);
    let archive = in_box(&pieces, &boxes.archive);
    assert_eq!(archive.len(), 1);
    assert_eq!(archive[0].0, now);
    assert!(archive[0].1.starts_with("inbox-"), "{archive:?}");
    assert!(last(&pieces).whole);
}

#[test]
fn an_email_changed_elsewhere_that_the_engine_has_not_got_comes_whole() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, mut known) = read_all(&stand_in);
    // (The engine has only the newest of the archive: the other is older than its window.)
    let archive = known.iter_mut().find(|k| k.mailbox == boxes.archive).unwrap();
    let older = archive.ids.pop().unwrap();
    stand_in.with(|d| assert!(d.move_to(&older, &boxes.inbox).is_some()));
    let pieces = run(&stand_in, &quiet(&state, &known));
    let inbox = in_box(&pieces, &boxes.inbox);
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].0, older);
    assert!(inbox[0].1.starts_with("archive-"), "read whole: {inbox:?}");
    assert!(last(&pieces).whole);
}

#[test]
fn more_changes_than_a_round_takes_come_in_more_rounds() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    // A server that answers four ids to a `get`: four changes to a round.
    stand_in.with(|d| d.max_get = 4);
    let made: Vec<String> = (0..9).map(|n| deliver(&stand_in, &boxes.inbox, &format!("new-{n}"), &[], 100 + n)).collect();
    let (calls, pieces) = counted(&stand_in, &quiet(&state, &known));
    // The mailboxes and the first round, then two rounds more (4, 4, 1).
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert_eq!(calls[1], names(&["Email/changes", "Email/get", "Email/get"]));
    let mut got: Vec<&str> = in_box(&pieces, &boxes.inbox).into_iter().map(|(id, _)| id).collect();
    got.sort();
    let mut wanted: Vec<&str> = made.iter().map(String::as_str).collect();
    wanted.sort();
    assert_eq!(got, wanted);
    let end = last(&pieces);
    assert!(end.whole && JmapMailState::usable(&end.email_state));
    // The last round's state is all of it: nothing more comes from it.
    let (calls, again) = counted(&stand_in, &quiet(&end.email_state, &known));
    assert_eq!(calls.len(), 1);
    assert!(pages(&again).is_empty());
}

#[test]
fn a_state_the_server_cant_calculate_from_has_every_window_read_and_a_new_state_left() {
    for lost in [true, false] {
        let (stand_in, boxes) = world(Profile::standard());
        let (state, known) = read_all(&stand_in);
        deliver(&stand_in, &boxes.inbox, "arrived", &[], 60);
        // A journal that lost what came before, or a state it never gave.
        let since = if lost {
            stand_in.with(|d| d.forget_changes());
            state.clone()
        } else {
            "s999".to_string()
        };
        let (calls, pieces) = counted(&stand_in, &quiet(&since, &known));
        // The mailboxes and the changes refused, the state and the mailboxes again, then the window of every mailbox, the Inbox
        // first (and the new message whole).
        assert_eq!(calls.len(), 7, "{calls:?}");
        assert_eq!(calls[1], names(&["Email/get", "Mailbox/get"]));
        let read: Vec<&str> = pages(&pieces).into_iter().map(|page| page.folder.as_str()).collect();
        assert_eq!(read, [boxes.inbox.as_str(), boxes.drafts.as_str(), boxes.sent.as_str(), boxes.archive.as_str()]);
        assert!(pages(&pieces).iter().all(|page| page.afresh));
        let end = last(&pieces);
        assert!(end.whole && JmapMailState::usable(&end.email_state) && end.email_state != since, "{end:?}");
        // From which what changed can be asked again.
        let (calls, _) = counted(&stand_in, &quiet(&end.email_state, &known));
        assert_eq!(calls.len(), 1);
    }
}

#[test]
fn a_server_without_changes_has_the_scope_read_and_the_state_forgotten() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    stand_in.with(|d| d.changes_unknown = true);
    let (calls, pieces) = counted(&stand_in, &quiet(&state, &known));
    // The mailboxes (the calls of the changes fail with it), then the Inbox's window.
    assert_eq!(calls.len(), 2, "{calls:?}");
    let read: Vec<&str> = pages(&pieces).into_iter().map(|page| page.folder.as_str()).collect();
    assert_eq!(read, [boxes.inbox.as_str()]);
    let end = last(&pieces);
    assert!(end.whole);
    assert_eq!(end.email_state, JMAP_STATE_UNKNOWN, "the state is forgotten");
    // A state that names none is not asked about: the Inbox is read, as without one.
    let (calls, pieces) = counted(&stand_in, &quiet(JMAP_STATE_UNKNOWN, &known));
    assert_eq!(calls, [names(&["Mailbox/get"]), names(&["Email/query", "Email/get"])]);
    assert!(last(&pieces).email_state.is_empty());
}

#[test]
fn a_run_that_is_not_whole_leaves_no_state() {
    let (stand_in, boxes) = world(Profile::standard());
    stand_in.with(|d| {
        d.broken_mailboxes.insert(boxes.archive.clone(), "serverFail");
    });
    let pieces = run(&stand_in, &everything());
    assert!(!last(&pieces).whole && last(&pieces).email_state.is_empty());
    // Nor does a run of the changes that could not read what was made.
    stand_in.with(|d| d.broken_mailboxes.clear());
    let (state, known) = read_all(&stand_in);
    let new = deliver(&stand_in, &boxes.inbox, "arrived", &[], 60);
    stand_in.with(|d| d.faults.push_back(Fault::on("Email/changes", Effect::MethodError("serverFail"))));
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert!(!last(&pieces).whole && last(&pieces).email_state.is_empty(), "{:?}", last(&pieces));
    // The next run asks from the same state, and brings it.
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert_eq!(in_box(&pieces, &boxes.inbox), [(new.as_str(), "arrived")]);
    assert!(last(&pieces).whole);
}

#[test]
fn an_answer_too_large_has_the_created_emails_read_apart() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    let new = deliver(&stand_in, &boxes.inbox, "arrived", &[], 60);
    // The request with the mailboxes is answered too large: the mailboxes alone, then the round without the created emails,
    // which are read apart.
    stand_in.with(|d| d.faults.push_back(Fault::on("Email/changes", Effect::Oversize)));
    let (calls, pieces) = counted(&stand_in, &quiet(&state, &known));
    assert_eq!(
        calls,
        [
            names(&["Mailbox/get", "Email/changes", "Email/get", "Email/get"]),
            names(&["Mailbox/get"]),
            names(&["Email/changes", "Email/get"]),
            names(&["Email/get"]),
        ]
    );
    assert_eq!(in_box(&pieces, &boxes.inbox), [(new.as_str(), "arrived")]);
    assert!(last(&pieces).whole && JmapMailState::usable(&last(&pieces).email_state));
}

#[test]
fn an_email_the_server_cant_give_whole_is_said_and_passed_over() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    let fine = deliver(&stand_in, &boxes.inbox, "fine", &[], 60);
    let broken = deliver(&stand_in, &boxes.inbox, "broken", &[], 61);
    stand_in.with(|d| {
        d.broken_emails.insert(broken.clone());
    });
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert_eq!(in_box(&pieces, &boxes.inbox), [(fine.as_str(), "fine")]);
    let end = last(&pieces);
    assert!(end.errors.iter().any(|line| line.starts_with("Couldn't read 1 message in full")), "{:?}", end.errors);
    // (Said, and the state goes on: it would be asked for again at every run.)
    assert!(JmapMailState::usable(&end.email_state));
}

#[test]
fn reopened_on_a_copy_a_run_of_the_changes_makes_one_request_where_reading_again_makes_many() {
    // A mailbox as a person has one: the special folders and a dozen of their own, a few hundred messages.
    let stand_in = StandIn::new();
    let inbox = stand_in.with(|d| {
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        for (name, role) in [("Drafts", "drafts"), ("Sent", "sent"), ("Trash", "trash"), ("Archive", "archive"), ("Junk", "junk")] {
            d.mailbox(name, None, Some(role));
        }
        for n in 0..10 {
            d.mailbox(&format!("Project {n}"), None, None);
        }
        inbox
    });
    let mailboxes: Vec<String> = stand_in.with(|d| d.mailboxes.iter().map(|m| m.id.clone()).collect());
    for n in 0..400 {
        deliver(&stand_in, &mailboxes[n % mailboxes.len()], &format!("m{n}"), &["$seen"], n);
    }
    let before = stand_in.api_requests();
    let (state, known) = read_all(&stand_in);
    let reading_again = stand_in.api_requests() - before;
    let before = stand_in.api_requests();
    let pieces = run(&stand_in, &quiet(&state, &known));
    let asking = stand_in.api_requests() - before;
    assert!(last(&pieces).whole);
    eprintln!("reopened: reading every window again {reading_again} requests, asking what changed {asking}");
    assert_eq!(asking, 1);
    assert!(reading_again >= mailboxes.len(), "{reading_again}");
    // A message come since: still one request.
    deliver(&stand_in, &inbox, "since", &[], 1000);
    let before = stand_in.api_requests();
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert_eq!(stand_in.api_requests() - before, 1);
    assert_eq!(in_box(&pieces, &inbox).len(), 1);
}

#[test]
fn a_run_of_every_mailbox_reads_the_state_before_the_mailboxes() {
    let (stand_in, boxes) = world(Profile::standard());
    let (calls, pieces) = counted(&stand_in, &everything());
    assert_eq!(calls[0], names(&["Email/get", "Mailbox/get"]));
    let (state, known) = (last(&pieces).email_state.clone(), Vec::<JmapKnown>::new());
    // Mail that came while the windows were read is in what changed since: here, mail that came after the state was read.
    let came = deliver(&stand_in, &boxes.inbox, "meanwhile", &[], 70);
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert_eq!(in_box(&pieces, &boxes.inbox), [(came.as_str(), "meanwhile")]);
}

#[test]
fn an_email_moved_from_a_mailbox_gone_since_comes_whole() {
    // Moved elsewhere from a folder that was then deleted: the engine loses the folder with this run's listing, and with it
    // the message, which comes whole to where it went rather than as marks that would find nothing.
    let (stand_in, boxes) = world(Profile::standard());
    let projects = stand_in.with(|d| d.mailbox("Projects", None, None));
    let filed = deliver(&stand_in, &projects, "filed", &["$seen"], 50);
    let (state, known) = read_all(&stand_in);
    assert!(known.iter().any(|k| k.mailbox == projects && k.ids.contains(&filed)));
    stand_in.with(|d| {
        assert!(d.move_to(&filed, &boxes.archive).is_some());
        d.mailboxes.retain(|m| m.id != projects);
    });
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert_eq!(in_box(&pieces, &boxes.archive), [(filed.as_str(), "filed")]);
    // One the engine has where it is comes as its marks.
    let (state, known) = read_all(&stand_in);
    stand_in.with(|d| assert!(d.set_keyword(&filed, "$flagged", true)));
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert_eq!(in_box(&pieces, &boxes.archive), [(filed.as_str(), "")]);
}

#[test]
fn an_email_the_server_fails_alone_with_none_read_holds_the_state_back() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    let new = deliver(&stand_in, &boxes.inbox, "arrived", &[], 60);
    // The created email's call fails in the request with the mailboxes (and so does that of the updated, none), alone just
    // after, and once more: the server, not the email.
    stand_in.with(|d| {
        for _ in 0..4 {
            d.faults.push_back(Fault::on("Email/get", Effect::MethodError("serverFail")));
        }
    });
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert!(in_box(&pieces, &boxes.inbox).is_empty());
    assert!(!last(&pieces).whole && last(&pieces).email_state.is_empty(), "{:?}", last(&pieces));
    // The next run asks from the same state, and brings it.
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert_eq!(in_box(&pieces, &boxes.inbox), [(new.as_str(), "arrived")]);
    assert!(last(&pieces).whole);
}

#[test]
fn changes_that_cant_be_had_leave_the_newest_mail_to_the_windows_and_the_state_kept() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    let new = deliver(&stand_in, &boxes.inbox, "arrived", &[], 60);
    // A server whose changes fail for a while: every run reads the Inbox meanwhile, and asks from the same state.
    for _ in 0..2 {
        stand_in.with(|d| d.faults.push_back(Fault::on("Email/changes", Effect::MethodError("serverFail"))));
        let pieces = run(&stand_in, &quiet(&state, &known));
        let read: Vec<&str> = pages(&pieces).into_iter().map(|page| page.folder.as_str()).collect();
        assert_eq!(read, [boxes.inbox.as_str()]);
        assert!(in_box(&pieces, &boxes.inbox).iter().any(|(id, _)| *id == new));
        let end = last(&pieces);
        assert!(!end.whole && end.email_state.is_empty() && !end.errors.is_empty(), "{end:?}");
    }
    let pieces = run(&stand_in, &quiet(&state, &known));
    assert!(last(&pieces).whole && last(&pieces).email_state != state);
}

#[test]
fn an_email_that_failed_alone_while_the_server_was_failing_is_asked_for_again() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    let one = deliver(&stand_in, &boxes.inbox, "one", &[], 60);
    let two = deliver(&stand_in, &boxes.inbox, "two", &[], 61);
    // The server fails a while: the created emails' call with the mailboxes (and the updated's), both of them, then the first
    // alone; the second comes, and the first asked again comes too.
    stand_in.with(|d| {
        for _ in 0..4 {
            d.faults.push_back(Fault::on("Email/get", Effect::MethodError("serverFail")));
        }
    });
    let pieces = run(&stand_in, &quiet(&state, &known));
    let mut got: Vec<&str> = in_box(&pieces, &boxes.inbox).into_iter().map(|(id, _)| id).collect();
    got.sort();
    let mut wanted = vec![one.as_str(), two.as_str()];
    wanted.sort();
    assert_eq!(got, wanted);
    assert!(last(&pieces).whole && last(&pieces).errors.is_empty(), "{:?}", last(&pieces));
}

#[test]
fn a_request_of_changes_the_server_turns_down_leaves_the_newest_mail_to_the_windows() {
    let (stand_in, boxes) = world(Profile::standard());
    let (state, known) = read_all(&stand_in);
    let new = deliver(&stand_in, &boxes.inbox, "arrived", &[], 60);
    // (A gateway that turns down a request that asks for the changes: HTTP 400, every time.)
    let refusing = Refusing(stand_in.clone());
    let client = block_on(Client::connect(refusing, SESSION, &Auth::bearer(TOKEN))).unwrap();
    let mut pieces = Vec::new();
    block_on(sync_mail(&client, &quiet(&state, &known), &AtomicBool::new(false), &mut |piece| pieces.push(piece)));
    assert!(in_box(&pieces, &boxes.inbox).iter().any(|(id, _)| *id == new));
    let end = last(&pieces);
    assert!(!end.whole && end.email_state.is_empty() && !end.errors.is_empty(), "{end:?}");
}

/// The stand-in behind a gateway that turns down, with a 400, any request that calls `Email/changes`.
#[derive(Clone)]
struct Refusing(StandIn);

impl Http for Refusing {
    async fn send(&self, req: crate::Request) -> crate::Result<crate::Response> {
        if String::from_utf8_lossy(&req.body).contains("Email/changes") {
            return Ok(crate::Response { status: 400, body: br#"{"type":"about:blank","status":400}"#.to_vec(), retry_after: None });
        }
        self.0.send(req).await
    }
}
