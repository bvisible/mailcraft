// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The sending part of a run, over the in-process server of either kind: what is sent and filed before a
//! window is read, in what order and in how many pieces, what a run does when the server limits sending, goes
//! away or turns the sign-in down in the middle of it, and what a request that names too much or names a message
//! twice makes of it. Invented people at `example.org`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::sync::{JmapDraftUp, JmapMailRequest, JmapMailResult, JmapScope, JmapSend, SendOutcome};
use mailcraft_model::{Address, Body, Message};

use crate::standin::{Effect, Fault, Profile, SESSION, StandIn, TOKEN};
use crate::{Auth, Client, block_on, sync_mail};

const ME: &str = "ada@example.org";
const ID: u64 = 3;
const DATE: i64 = 1_791_993_600;

struct Ids {
    inbox: String,
    drafts: String,
    sent: String,
}

fn world(profile: Profile) -> (StandIn, Ids) {
    let server = StandIn::new();
    let ids = server.with(|d| {
        d.profile = profile;
        let ids = Ids {
            inbox: d.mailbox("Inbox", None, Some("inbox")),
            drafts: d.mailbox("Drafts", None, Some("drafts")),
            sent: d.mailbox("Sent", None, Some("sent")),
        };
        d.identity("Ada", ME);
        ids
    });
    (server, ids)
}

fn both() -> [(&'static str, Profile); 2] {
    [("a store of the standard kind", Profile::standard()), ("a server in front of IMAP and SMTP", Profile::by_place())]
}

fn message(id: u64, subject: &str) -> Message {
    Message {
        id,
        from: Address::new("Ada", ME),
        to: vec![Address::new("Bob", "bob@example.org")],
        bcc: vec![Address::new("Eve", "eve@example.org")],
        subject: subject.into(),
        message_id: format!("{subject}@example.org"),
        date: DATE,
        body: Body { text: format!("Hello about {subject}."), html: None },
        ..Default::default()
    }
}

fn to_send(id: u64, subject: &str) -> JmapSend {
    JmapSend { message: id, draft_copy: String::new(), mime: Box::new(message(id, subject)) }
}

fn to_file(id: u64, subject: &str, replaces: &str) -> JmapDraftUp {
    JmapDraftUp { message: id, saves: 1, replaces: replaces.into(), mime: Box::new(message(id, subject)) }
}

/// A run that reads every mailbox, the list of them first, and sends and files these first of all.
fn request(sends: Vec<JmapSend>, drafts: Vec<JmapDraftUp>) -> JmapMailRequest {
    JmapMailRequest {
        account: ID,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        window: 500,
        list_folders: true,
        sends,
        drafts,
        ..Default::default()
    }
}

/// Every piece of a run, handed to `each` as it comes (with the cancel flag).
fn run_with(server: &StandIn, request: &JmapMailRequest, each: &mut dyn FnMut(&JmapMailResult, &AtomicBool)) -> Vec<JmapMailResult> {
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let (cancel, pieces) = (AtomicBool::new(false), RefCell::new(Vec::new()));
    block_on(sync_mail(&client, request, &cancel, &mut |piece| {
        each(&piece, &cancel);
        pieces.borrow_mut().push(piece);
    }));
    pieces.into_inner()
}

fn run(server: &StandIn, request: &JmapMailRequest) -> Vec<JmapMailResult> {
    run_with(server, request, &mut |_, _| {})
}

fn last(pieces: &[JmapMailResult]) -> &JmapMailResult {
    pieces.last().expect("a run ends with a piece")
}

/// What each piece is: `list`, a window (its mailbox's name), what became of messages (`sent`) or of drafts
/// (`drafts`), or the end.
fn shape(pieces: &[JmapMailResult], ids: &Ids) -> Vec<String> {
    let name = |mailbox: &str| {
        if mailbox == ids.inbox {
            "Inbox"
        } else if mailbox == ids.drafts {
            "Drafts"
        } else if mailbox == ids.sent {
            "Sent"
        } else {
            "?"
        }
    };
    pieces
        .iter()
        .map(|piece| {
            if piece.listing.is_some() {
                "list".to_string()
            } else if let Some(page) = piece.pages.first() {
                format!("window {}", name(&page.folder))
            } else if !piece.sent.is_empty() {
                format!("sent {}", piece.sent.iter().map(|s| format!("{}", s.message)).collect::<Vec<_>>().join(","))
            } else if !piece.drafts.is_empty() {
                format!("drafts {}", piece.drafts.iter().map(|d| format!("{}", d.message)).collect::<Vec<_>>().join(","))
            } else if !piece.errors.is_empty() && piece.partial {
                format!("line {}", piece.errors.len())
            } else if !piece.partial {
                "end".to_string()
            } else {
                "?".to_string()
            }
        })
        .collect()
}

fn outcomes(pieces: &[JmapMailResult]) -> Vec<(u64, SendOutcome)> {
    pieces.iter().flat_map(|piece| piece.sent.iter().map(|done| (done.message, done.outcome.clone()))).collect()
}

fn subjects(piece: &JmapMailResult, mailbox: &str) -> Vec<String> {
    piece.pages.iter().filter(|page| page.folder == mailbox).flat_map(|page| page.messages.iter().map(|m| m.subject.clone())).collect()
}

fn window<'a>(pieces: &'a [JmapMailResult], mailbox: &str) -> &'a mailcraft_model::sync::MessagePage {
    pieces.iter().flat_map(|piece| piece.pages.iter()).find(|page| page.folder == mailbox).expect("a window of that mailbox")
}

// ------------------------------------------------------------------------------------------------

#[test]
fn what_waits_in_the_outbox_goes_before_a_window_is_read_so_that_sent_shows_the_copy() {
    for (kind, profile) in both() {
        let (server, ids) = world(profile);
        server.with(|d| d.deliver(&ids.inbox, b"From: z@example.org\r\nSubject: hello\r\n\r\nx\r\n", &[], "2026-10-14T09:00:00Z"));
        let pieces = run(&server, &request(vec![to_send(7, "one"), to_send(8, "two")], vec![]));
        // The list, what became of each message as it was done, then the windows, then the end.
        assert_eq!(shape(&pieces, &ids), ["list", "sent 7", "sent 8", "window Inbox", "window Drafts", "window Sent", "end"], "{kind}");
        assert_eq!(outcomes(&pieces), [(7, SendOutcome::Sent), (8, SendOutcome::Sent)], "{kind}");
        // The requests: the list of mailboxes, who the account is (once), each message's filing and submission, and
        // only then the first window.
        let calls = server.calls();
        assert_eq!(
            calls[..8],
            ["Mailbox/get", "Identity/get", "Email/import", "EmailSubmission/set", "Email/import", "EmailSubmission/set", "Email/query", "Email/get"],
            "{kind}: {calls:?}"
        );
        // Both went to the recipients named, once, with no Bcc; and the window of Sent has both copies, read.
        let delivered = server.with(|d| d.delivered.clone());
        assert_eq!(delivered.len(), 2, "{kind}");
        assert!(delivered.iter().all(|d| d.recipients == ["bob@example.org", "eve@example.org"]), "{kind}");
        assert!(delivered.iter().all(|d| !String::from_utf8_lossy(&d.raw).to_ascii_lowercase().contains("bcc")), "{kind}");
        let sent = window(&pieces, &ids.sent);
        assert_eq!(sent.messages.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["two", "one"], "{kind}: the newest first");
        assert!(sent.messages.iter().all(|m| m.server.read && !m.draft), "{kind}");
        assert!(sent.afresh, "{kind}: the window holds the whole of Sent");
        let end = last(&pieces);
        assert!(end.whole && end.errors.is_empty() && !end.partial, "{kind}: {end:?}");
    }
}

#[test]
fn a_run_with_nothing_to_send_and_no_draft_asks_nothing_for_them() {
    let (server, ids) = world(Profile::standard());
    server.with(|d| d.deliver(&ids.inbox, b"From: z@example.org\r\nSubject: hello\r\n\r\nx\r\n", &[], "2026-10-14T09:00:00Z"));
    let pieces = run(&server, &request(vec![], vec![]));
    assert_eq!(shape(&pieces, &ids), ["list", "window Inbox", "window Drafts", "window Sent", "end"]);
    assert!(!server.calls().iter().any(|call| matches!(call.as_str(), "Identity/get" | "Email/import" | "EmailSubmission/set" | "Email/set")));
    assert!(server.seen().iter().all(|s| !s.path.starts_with("/jmap/upload/")));
}

#[test]
fn a_run_with_the_light_scope_sends_all_the_same() {
    let (server, ids) = world(Profile::standard());
    let mut light = request(vec![to_send(7, "one")], vec![]);
    light.scope = JmapScope::Light { also: vec![ids.sent.clone()] };
    light.list_folders = false;
    let pieces = run(&server, &light);
    // (With no list asked for, the listing is not a piece, and the sending still has the mailboxes to go by.)
    assert_eq!(shape(&pieces, &ids), ["sent 7", "window Inbox", "window Sent", "end"]);
    assert_eq!(subjects(&pieces[2], &ids.sent), ["one"]);
}

#[test]
fn drafts_are_filed_after_the_messages_and_the_window_of_drafts_has_the_new_copy_alone() {
    for (kind, profile) in both() {
        let (server, ids) = world(profile);
        let before = server
            .with(|d| d.deliver(&ids.drafts, b"From: ada@example.org\r\nSubject: first\r\n\r\nx\r\n", &["$draft", "$seen"], "2026-10-14T09:00:00Z"));
        let pieces = run(&server, &request(vec![to_send(7, "sent")], vec![to_file(9, "draft", &before)]));
        assert_eq!(shape(&pieces, &ids), ["list", "sent 7", "drafts 9", "window Inbox", "window Drafts", "window Sent", "end"], "{kind}");
        let done = &pieces[2].drafts[0];
        assert_eq!((done.message, done.saves, done.refused.as_deref(), done.folder.as_str()), (9, 1, None, ids.drafts.as_str()), "{kind}");
        // Drafts: the new copy, a draft, with its Bcc; the one it replaces is gone.
        let page = window(&pieces, &ids.drafts);
        assert_eq!(
            page.messages.iter().map(|m| (m.server.id.as_str(), m.draft, m.subject.as_str())).collect::<Vec<_>>(),
            [(done.id.as_str(), true, "draft")],
            "{kind}"
        );
        let filed = server.with(|d| d.email(&done.id).map(|e| String::from_utf8_lossy(&e.raw).to_string())).unwrap();
        assert!(filed.contains("Bcc: ") && filed.contains("eve@example.org"), "{kind}: {filed}");
        assert!(server.with(|d| d.email(&before).is_none()), "{kind}");
        // A draft goes nowhere.
        assert_eq!(server.with(|d| d.delivered.len()), 1, "{kind}: only the message was sent");
    }
}

#[test]
fn a_run_sends_ten_messages_and_files_ten_drafts_at_most_and_a_message_named_twice_goes_once() {
    let (server, _) = world(Profile::standard());
    let sends: Vec<JmapSend> = (1..=12).map(|n| to_send(n, &format!("m{n}"))).chain([to_send(3, "m3 again")]).collect();
    let drafts: Vec<JmapDraftUp> = (101..=112).map(|n| to_file(n, &format!("d{n}"), "")).chain([to_file(101, "d101 again", "")]).collect();
    let pieces = run(&server, &request(sends, drafts));
    let sent: Vec<u64> = outcomes(&pieces).iter().map(|(message, _)| *message).collect();
    assert_eq!(sent, (1..=10).collect::<Vec<u64>>(), "the first ten, once each, in the order of the request");
    let filed: Vec<u64> = pieces.iter().flat_map(|p| p.drafts.iter().map(|d| d.message)).collect();
    assert_eq!(filed, (101..=110).collect::<Vec<u64>>());
    assert_eq!(server.with(|d| d.delivered.len()), 10);
    assert_eq!(server.with(|d| d.emails.iter().filter(|e| e.keywords.contains("$draft")).count()), 10);
    // (The rest are not mentioned: they wait for the next run.)
    assert!(last(&pieces).whole);
}

#[test]
fn the_owner_stopping_the_run_between_two_messages_leaves_the_second_for_the_next_run() {
    let (server, ids) = world(Profile::standard());
    let pieces = run_with(&server, &request(vec![to_send(7, "one"), to_send(8, "two")], vec![to_file(9, "draft", "")]), &mut |piece, cancel| {
        // Cancel is pressed as the first message is reported.
        if !piece.sent.is_empty() {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert_eq!(shape(&pieces, &ids), ["list", "sent 7", "end"]);
    assert_eq!(outcomes(&pieces), [(7, SendOutcome::Sent)]);
    assert!(last(&pieces).stopped && !last(&pieces).whole);
    assert_eq!(server.with(|d| d.delivered.len()), 1, "the second was not tried, and nor was the draft");
    assert!(server.with(|d| d.emails.iter().all(|e| !e.keywords.contains("$draft"))));
}

#[test]
fn what_became_of_a_message_is_handed_over_before_the_next_one_is_started() {
    // A run cut short after one message went must not leave the engine believing it did not.
    let (server, ids) = world(Profile::standard());
    let seen_delivered = RefCell::new(Vec::new());
    run_with(&server, &request(vec![to_send(7, "one"), to_send(8, "two"), to_send(9, "three")], vec![]), &mut |piece, _| {
        if !piece.sent.is_empty() {
            seen_delivered.borrow_mut().push(server.with(|d| d.delivered.len()));
        }
    });
    assert_eq!(*seen_delivered.borrow(), [1, 2, 3]);
    let _ = ids;
}

#[test]
fn a_server_that_limits_sending_leaves_the_rest_of_the_outbox_and_the_drafts_and_the_run_reads_on() {
    let (server, ids) = world(Profile::standard());
    server.with(|d| {
        // The first goes; the second is refused for the rate, and so would be the third.
        d.faults.push_back(Fault::on("EmailSubmission/set", Effect::SetError("rateLimit", "Slow down.")));
    });
    // (The fault is for the first submission; the one that goes is the second, so line the first up to go by it.)
    let pieces = run(&server, &request(vec![to_send(7, "one"), to_send(8, "two"), to_send(9, "three")], vec![to_file(10, "draft", "")]));
    // The first is the one refused for the rate: it waits, and nothing after it is tried this run, not even the draft.
    assert_eq!(shape(&pieces, &ids), ["list", "line 1", "window Inbox", "window Drafts", "window Sent", "end"]);
    assert_eq!(pieces[1].errors, [crate::send::RATE_LIMITED.to_string()]);
    assert!(outcomes(&pieces).is_empty());
    // Nothing is left looking sent, nothing went, and the windows were read all the same.
    assert!(server.with(|d| d.delivered.is_empty() && d.emails.is_empty() && d.faults.is_empty()));
    assert!(last(&pieces).whole && last(&pieces).errors.is_empty());
}

#[test]
fn a_server_in_front_of_a_mailbox_that_has_sent_as_many_as_it_will_says_so_for_the_rest() {
    let (server, ids) = world(Profile::by_place());
    server.with(|d| d.sent_lately = 29);
    let pieces = run(&server, &request(vec![to_send(7, "one"), to_send(8, "two"), to_send(9, "three")], vec![]));
    // One more goes, and the next is refused for the rate; the third is not tried.
    assert_eq!(shape(&pieces, &ids), ["list", "sent 7", "line 1", "window Inbox", "window Drafts", "window Sent", "end"]);
    assert_eq!(outcomes(&pieces), [(7, SendOutcome::Sent)]);
    assert_eq!(server.with(|d| d.delivered.len()), 1);
    assert_eq!(subjects(&pieces[3], &ids.inbox), Vec::<String>::new());
    assert_eq!(window(&pieces, &ids.sent).messages.len(), 1, "the copy of the one that went is in Sent, and no other");
}

#[test]
fn without_the_list_of_mailboxes_nothing_is_tried_and_what_waits_waits() {
    let (server, ids) = world(Profile::standard());
    server.with(|d| d.faults.push_back(Fault::on("Mailbox/get", Effect::MethodError("serverFail"))));
    let mut asks = request(vec![to_send(7, "one")], vec![to_file(9, "draft", "")]);
    asks.scope = JmapScope::Light { also: vec![] };
    let pieces = run(&server, &asks);
    assert!(outcomes(&pieces).is_empty() && pieces.iter().all(|p| p.drafts.is_empty()));
    assert!(!server.calls().iter().any(|call| matches!(call.as_str(), "Identity/get" | "Email/import" | "EmailSubmission/set")));
    assert!(server.seen().iter().all(|s| !s.path.starts_with("/jmap/upload/")));
    assert!(!last(&pieces).errors.is_empty(), "the trouble with the list is told");
    let _ = ids;
}

#[test]
fn trouble_with_the_server_ends_the_sending_and_not_the_run() {
    let (server, ids) = world(Profile::standard());
    // The answer to who the account is never comes: the sending ends there, and the windows are read.
    server.with(|d| d.faults.push_back(Fault::on("Identity/get", Effect::LoseAnswer)));
    let pieces = run(&server, &request(vec![to_send(7, "one"), to_send(8, "two")], vec![to_file(9, "draft", "")]));
    assert_eq!(shape(&pieces, &ids), ["list", "line 1", "window Inbox", "window Drafts", "window Sent", "end"]);
    assert_eq!(pieces[1].errors, [crate::send::UNREACHABLE.to_string()]);
    assert!(server.with(|d| d.delivered.is_empty()));
    assert!(last(&pieces).whole);
}

#[test]
fn a_sign_in_the_server_turns_down_in_the_middle_ends_the_sending_and_the_run_says_so() {
    let (server, ids) = world(Profile::standard());
    // The token is no good any more from the moment the list of mailboxes is in.
    let pieces = run_with(&server, &request(vec![to_send(7, "one")], vec![]), &mut |piece, _| {
        if piece.listing.is_some() {
            server.with(|d| d.token = Some("another".into()));
        }
    });
    assert!(outcomes(&pieces).is_empty());
    assert_eq!(shape(&pieces, &ids), ["list", "end"]);
    let end = last(&pieces);
    assert!(end.auth_refused && !end.whole);
    assert!(server.with(|d| d.delivered.is_empty() && d.emails.is_empty()));
}

#[test]
fn a_message_that_cannot_be_sent_is_held_and_the_run_goes_on_with_the_next() {
    let (server, ids) = world(Profile::standard());
    let mut no_one = to_send(7, "nobody");
    no_one.mime.to.clear();
    no_one.mime.bcc.clear();
    let pieces = run(&server, &request(vec![no_one, to_send(8, "two")], vec![]));
    assert_eq!(shape(&pieces, &ids), ["list", "sent 7", "sent 8", "window Inbox", "window Drafts", "window Sent", "end"]);
    let outcomes = outcomes(&pieces);
    assert_eq!(outcomes[0], (7, SendOutcome::Held { lost: false, why: "the message has no recipients".into() }));
    assert_eq!(outcomes[1], (8, SendOutcome::Sent));
    assert_eq!(server.with(|d| d.delivered.len()), 1);
}

#[test]
fn an_answer_that_is_lost_after_a_message_went_holds_it_and_nothing_after_it_is_tried() {
    for (kind, profile) in both() {
        let (server, ids) = world(profile);
        server.with(|d| d.faults.push_back(Fault::on("EmailSubmission/set", Effect::LoseAnswer)));
        let pieces = run(&server, &request(vec![to_send(7, "one"), to_send(8, "two")], vec![to_file(9, "draft", "")]));
        assert_eq!(shape(&pieces, &ids), ["list", "sent 7", "window Inbox", "window Drafts", "window Sent", "end"], "{kind}");
        assert_eq!(outcomes(&pieces), [(7, SendOutcome::Held { lost: true, why: String::new() })], "{kind}");
        // It went, and its copy is in Sent for the owner to see; the second was not tried, and nor was the draft.
        assert_eq!(server.with(|d| d.delivered.len()), 1, "{kind}");
        assert_eq!(window(&pieces, &ids.sent).messages.len(), 1, "{kind}");
        assert!(window(&pieces, &ids.drafts).messages.is_empty(), "{kind}");
    }
}
