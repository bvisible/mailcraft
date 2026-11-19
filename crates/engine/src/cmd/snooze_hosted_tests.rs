// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! « Plus tard » in a hosting page's tab, as the engine asks the server and takes in its answers and its records, with
//! answers made by hand (the server itself: frappe_webmail's `tests/test_neomail_snooze.py`).

use mailcraft_model::sync::JmapChange;
use mailcraft_model::sync::ServerMessage;
use mailcraft_model::time::{Zone, parse_iso};
use mailcraft_model::{FolderKind, Id, Mailbox, Message, ServerFolder};
use serde_json::json;

use super::*;
use crate::Clock;
use crate::cmd::jmap_up::message_changes;

/// Thursday 2026-11-19 12:00 UTC.
const NOW: i64 = 1_795_089_600;
const ME: &str = "owner@example.org";

/// A hosting page's tab with a JMAP account whose server keeps snoozes: its Inbox, a folder « Projets » and its Snoozed
/// folder (« Plus tard », `MSnz` there) the server has, and a message in the Inbox. The session, the account, the
/// message, the Snoozed folder and « Projets ».
fn setup() -> (Session, Id, Id, Id, Id) {
    let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
    s.jmap_runs = true;
    s.keeps_local = false;
    let account = s
        .run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap/session", "bearer": true}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    s.mailbox.folder_mut(inbox).unwrap().server = Some(ServerFolder { id: "Min".into(), name: "INBOX".into(), ..Default::default() });
    let snoozed = s.mailbox.add_folder_free(account, None, "Plus tard").unwrap();
    s.mailbox.folder_mut(snoozed).unwrap().server = Some(ServerFolder { id: "MSnz".into(), name: "Plus tard".into(), ..Default::default() });
    let projects = s.mailbox.add_folder_free(account, None, "Projets").unwrap();
    s.mailbox.folder_mut(projects).unwrap().server = Some(ServerFolder { id: "Mpr".into(), name: "Projets".into(), ..Default::default() });
    let m = add(&mut s, inbox, "E1", "Invoice 42");
    set(&mut s, Some("MSnz"), json!([]));
    (s, account, m, snoozed, projects)
}

fn add(s: &mut Session, folder: Id, id: &str, subject: &str) -> Id {
    let at = s.mailbox.folder(folder).and_then(|f| f.server.as_ref()).map(|server| server.id.clone()).unwrap_or_default();
    let server = ServerMessage { id: id.into(), folder: at, read: true, ..Default::default() };
    let message_id = format!("<{}@example.org>", subject.replace(' ', "-").to_lowercase());
    s.mailbox.add_message(Message { folder, subject: subject.into(), message_id, read: true, server: Some(server), ..Default::default() }).unwrap()
}

fn set(s: &mut Session, mailbox: Option<&str>, records: serde_json::Value) -> serde_json::Value {
    s.run("mail.setSnoozes", &json!({"account": ME, "mailbox": mailbox, "records": records})).unwrap()
}

fn moves(s: &Session, account: Id) -> Vec<JmapChange> {
    message_changes(s, account, 100).0.into_iter().filter(|c| matches!(c, JmapChange::Move { .. })).collect()
}

#[test]
fn a_hosted_message_is_put_away_at_once_and_its_server_is_asked_without_a_move_going_up() {
    let (mut s, account, m, snoozed, _) = setup();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let steps = s.undo_steps_moved();
    let answer = s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).unwrap();
    assert_eq!(answer["server"], json!(true));
    let here = s.mailbox.message(m).unwrap();
    assert_eq!(here.folder, snoozed, "out of the Inbox at once");
    assert!(here.snooze.is_some_and(|z| z.placing && z.until == NOW + 3600 && z.from == inbox));
    assert_eq!(moves(&s, account), [], "the server makes the move");
    assert_eq!(s.undo_steps_moved(), steps, "no step: the server keeps it");
    let asked = s.hosted_snooze.take();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].what,
        Asked::Put { email_id: "E1".into(), message_id: "<invoice-42@example.org>".into(), until: NOW + 3600, from: inbox, placed: true, was: None }
    );
    assert!(s.hosted_snooze.take().is_empty(), "taken once");
    // Asked again while it is being put away: refused, nothing asked twice.
    assert!(s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 7200})).is_err());
    // The server did it, and said where the message is now.
    s.run("mail.snoozeAnswered", &json!({"message": m, "ok": true, "record": "r1", "emailId": "E9", "mailboxId": "MSnz"})).unwrap();
    let here = s.mailbox.message(m).unwrap();
    let server = here.server.as_ref().unwrap();
    assert_eq!((server.id.as_str(), server.folder.as_str()), ("E9", "MSnz"));
    assert!(here.snooze.is_some_and(|z| !z.placing && z.waiting() && z.until == NOW + 3600));
    assert_eq!(record(&s, m).map(|r| r.id.as_str()), Some("r1"));
    assert_eq!(moves(&s, account), []);
    assert!(!s.hosted_snooze.asking());
}

#[test]
fn a_refusal_puts_the_message_back_and_says_so() {
    let (mut s, account, m, _, _) = setup();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).unwrap();
    s.hosted_snooze.take();
    s.run("mail.snoozeAnswered", &json!({"message": m, "ok": false, "error": "The mailbox is busy."})).unwrap();
    let here = s.mailbox.message(m).unwrap();
    assert_eq!((here.folder, here.snooze), (inbox, None));
    assert_eq!(moves(&s, account), []);
    let said = std::mem::take(&mut s.hosted_snooze.said);
    assert_eq!(said.len(), 1);
    assert!(said[0].contains("Invoice 42") && said[0].contains("The mailbox is busy."), "{said:?}");
}

#[test]
fn a_time_in_the_past_is_refused() {
    let (mut s, _, m, _, _) = setup();
    assert!(s.run("mail.snooze", &json!({"ids": [m], "until": NOW - 60})).is_err());
    assert!(s.run("mail.snooze", &json!({"ids": [m], "until": NOW})).is_err());
    assert!(!s.hosted_snooze.asking());
}

#[test]
fn the_time_the_person_chose_is_their_wall_clock_across_a_change_of_the_clocks() {
    let (mut s, _, m, _, _) = setup();
    // Saturday 24 October 2026, 18:00 in Zurich (UTC+2 until the night of the 25th).
    s.clock = Clock { now: Some(1_792_857_600), offset: None, zone: Zone::named("Europe/Zurich") };
    s.prefs.time_zone = "Europe/Zurich".into();
    s.run("mail.snooze", &json!({"ids": [m], "until": "2026-10-26T08:00"})).unwrap();
    let asked = s.hosted_snooze.take();
    let Asked::Put { until, .. } = &asked[0].what else { panic!("{asked:?}") };
    let (wall, _) = parse_iso("2026-10-26T07:00").unwrap();
    assert_eq!(*until, wall.to_timestamp(0), "08:00 in Zurich on the 26th is 07:00 UTC (winter time)");
}

#[test]
fn a_message_brought_back_now_is_back_at_once_unread_and_its_server_is_asked() {
    let (mut s, account, _, snoozed, projects) = setup();
    let m = add(&mut s, snoozed, "E5", "Plan");
    s.mailbox.message_mut(m).unwrap().server.as_mut().unwrap().read = true;
    set(&mut s, Some("MSnz"), json!([{"id": "r5", "emailId": "E5", "fromMailboxId": "Mpr", "until": NOW + 86_400}]));
    assert!(s.mailbox.message(m).unwrap().snooze.is_some_and(|z| z.until == NOW + 86_400 && z.from == projects));
    let steps = s.undo_steps_moved();
    s.run("mail.unsnooze", &json!({"ids": [m]})).unwrap();
    let here = s.mailbox.message(m).unwrap();
    assert_eq!((here.folder, here.read), (projects, false));
    assert_eq!(moves(&s, account), [], "the server makes the move");
    let marks = message_changes(&s, account, 100).0;
    assert!(marks.is_empty(), "nor its marks: the server makes it unread: {marks:?}");
    assert_eq!(s.undo_steps_moved(), steps);
    let asked = s.hosted_snooze.take();
    assert!(matches!(&asked[0].what, Asked::Return { record, to, .. } if record == "r5" && *to == projects));
    s.run("mail.snoozeAnswered", &json!({"message": m, "ok": true, "emailId": "E6", "mailboxId": "Mpr"})).unwrap();
    let here = s.mailbox.message(m).unwrap();
    let server = here.server.as_ref().unwrap();
    assert_eq!((server.id.as_str(), server.folder.as_str(), here.snooze), ("E6", "Mpr", None));
    assert!(record(&s, m).is_none());
}

#[test]
fn a_return_the_server_refused_waits_again() {
    let (mut s, _, _, snoozed, _) = setup();
    let m = add(&mut s, snoozed, "E5", "Plan");
    set(&mut s, Some("MSnz"), json!([{"id": "r5", "emailId": "E5", "fromMailboxId": "Mpr", "until": NOW + 86_400}]));
    s.run("mail.unsnooze", &json!({"ids": [m]})).unwrap();
    s.hosted_snooze.take();
    s.run("mail.snoozeAnswered", &json!({"message": m, "ok": false})).unwrap();
    let here = s.mailbox.message(m).unwrap();
    assert_eq!(here.folder, snoozed);
    assert!(here.snooze.is_some_and(|z| z.waiting() && z.until == NOW + 86_400));
    assert_eq!(s.hosted_snooze.said.len(), 1);
}

#[test]
fn the_records_are_put_on_the_messages_of_the_folder_by_id_or_message_id_and_nothing_else_keeps_one() {
    let (mut s, account, m, snoozed, projects) = setup();
    let a = add(&mut s, snoozed, "E5", "Plan");
    let b = add(&mut s, snoozed, "E7", "Offer");
    let c = add(&mut s, snoozed, "E8", "Moved by hand");
    // An Inbox message that kept a snooze from before is no longer waiting.
    s.mailbox.message_mut(m).unwrap().snooze = Some(mailcraft_model::Snooze { until: NOW + 60, ..Default::default() });
    set(
        &mut s,
        Some("MSnz"),
        json!([
            {"id": "r5", "emailId": "E5", "messageId": "plan@example.org", "fromMailboxId": "Mpr", "until": NOW + 600},
            // Numbered anew by the server: found by its Message-ID.
            {"id": "r7", "emailId": "E70", "messageId": "OFFER@example.org", "fromMailboxId": "Mgone", "until": NOW + 900, "failed": "The mail server refused the mailbox's password."},
        ]),
    );
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    assert!(s.mailbox.message(a).unwrap().snooze.is_some_and(|z| z.until == NOW + 600 && z.from == projects && !z.failed));
    assert!(s.mailbox.message(b).unwrap().snooze.is_some_and(|z| z.until == NOW + 900 && z.from == inbox && z.failed), "its folder is gone");
    assert_eq!(record(&s, b).and_then(|r| r.failed.as_deref()), Some("The mail server refused the mailbox's password."));
    assert_eq!(s.mailbox.message(c).unwrap().snooze, None, "lies in the folder without waiting: normal mail");
    assert_eq!(s.mailbox.message(m).unwrap().snooze, None);
    assert_eq!(s.mailbox.snoozed_folder(account), Some(snoozed));
    assert!(offered(&s, &[m]));
    // Said again unchanged: nothing changes.
    let again = set(
        &mut s,
        Some("MSnz"),
        json!([
            {"id": "r5", "emailId": "E5", "messageId": "plan@example.org", "fromMailboxId": "Mpr", "until": NOW + 600},
            {"id": "r7", "emailId": "E70", "messageId": "OFFER@example.org", "fromMailboxId": "Mgone", "until": NOW + 900, "failed": "The mail server refused the mailbox's password."},
        ]),
    );
    assert_eq!(again["changed"], json!(0));
    assert_eq!((again["unmatched"].clone(), again["folder"].clone()), (json!(0), json!(true)));
    // One whose message this window doesn't show yet is said so: the host reads again soon.
    let later = set(&mut s, Some("MSnz"), json!([{"id": "r9", "emailId": "E99", "messageId": "late@example.org", "until": NOW + 60}]));
    assert_eq!(later["unmatched"], json!(1));
}

#[test]
fn one_put_away_in_a_visit_that_ended_before_the_answer_goes_back_where_its_server_has_it() {
    let (mut s, account, m, snoozed, _) = setup();
    s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).unwrap();
    assert_eq!(s.mailbox.message(m).unwrap().folder, snoozed);
    // The tab was closed and opened again: what it was asking is gone with it, the mailbox kept in the browser stays.
    s.hosted_snooze = Default::default();
    set(&mut s, Some("MSnz"), json!([]));
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let here = s.mailbox.message(m).unwrap();
    assert_eq!((here.folder, here.snooze), (inbox, None));
}

#[test]
fn the_desktop_keeps_refusing_a_jmap_servers_mail_and_a_server_without_it_is_not_asked() {
    let (mut s, _, m, _, _) = setup();
    s.keeps_local = true;
    assert!(s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).is_err(), "the desktop's own snooze, as upstream has it");
    assert!(!s.hosted_snooze.asking());
    s.keeps_local = false;
    s.hosted_snooze.servers.clear();
    assert!(s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).is_err());
    assert!(!offered(&s, &[m]));
    assert!(!s.hosted_snooze.asking());
}

#[test]
fn a_message_not_on_its_server_yet_is_refused() {
    let (mut s, account, _, _, _) = setup();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let m = add(&mut s, inbox, "", "Draft-like");
    assert!(!offered(&s, &[m]));
    assert!(s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).is_err());
}

#[test]
fn the_first_one_put_away_before_the_folder_is_listed_waits_for_the_next_run() {
    let (mut s, account, m, snoozed, _) = setup();
    // No Snoozed folder here yet: the server makes it.
    s.mailbox.account_mut(account).unwrap().snoozed_folder = None;
    s.mailbox.folders.retain(|f| f.id != snoozed);
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).unwrap();
    assert_eq!(s.mailbox.message(m).unwrap().folder, inbox, "nowhere to put it here yet");
    assert!(matches!(&s.hosted_snooze.take()[0].what, Asked::Put { placed: false, .. }));
    s.run("mail.snoozeAnswered", &json!({"message": m, "ok": true, "record": "r1", "emailId": "E9", "mailboxId": "MSnz"})).unwrap();
    assert!(s.hosted_snooze.to_read.contains(&account), "the run that lists the new folder is asked for");
}

#[test]
fn a_message_moved_here_whose_move_has_not_gone_up_is_not_put_away_yet() {
    let (mut s, _, m, _, projects) = setup();
    // Filed into « Projets » here; the server still has it in the Inbox.
    s.run("mail.move", &json!({"ids": [m], "folder": projects})).unwrap();
    assert!(!offered(&s, &[m]));
    assert!(s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).is_err(), "it would come back to the Inbox");
    assert!(!s.hosted_snooze.asking());
}

#[test]
fn its_marks_wait_while_it_is_being_put_away_and_go_up_under_its_new_id() {
    let (mut s, account, m, _, _) = setup();
    s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).unwrap();
    s.hosted_snooze.take();
    s.run("mail.markUnread", &json!({"ids": [m]})).unwrap();
    assert!(message_changes(&s, account, 100).0.is_empty(), "not under the id the server is taking away");
    s.run("mail.snoozeAnswered", &json!({"message": m, "ok": true, "record": "r1", "emailId": "E9", "mailboxId": "MSnz"})).unwrap();
    let changes = message_changes(&s, account, 100).0;
    assert!(matches!(&changes[..], [JmapChange::Keywords { id, read: Some(false), .. }] if id == "E9"), "{changes:?}");
}

#[test]
fn a_server_that_no_longer_keeps_it_leaves_nothing_of_it_here() {
    let (mut s, account, m, _, _) = setup();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).unwrap();
    s.run("mail.setSnoozes", &json!({"account": ME, "offered": false})).unwrap();
    let here = s.mailbox.message(m).unwrap();
    assert_eq!((here.folder, here.snooze), (inbox, None), "where its server has it");
    assert!(!s.hosted_snooze.asking());
    assert!(!offered(&s, &[m]));
}

#[test]
fn a_run_that_lists_it_where_it_was_put_ends_its_placing() {
    use crate::cmd::jmap_down::take_window;
    use mailcraft_model::sync::{GraphMessage, MessagePage};
    let (mut s, account, m, snoozed, _) = setup();
    s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).unwrap();
    // The answer was lost; the server moved it, and a run reads « Plus tard » with the message under its new id.
    s.hosted_snooze = Default::default();
    let there = s.mailbox.message(m).unwrap();
    let listed = GraphMessage {
        message_id: there.message_id.clone(),
        subject: there.subject.clone(),
        server: ServerMessage { id: "E9".into(), folder: "MSnz".into(), read: true, ..Default::default() },
        ..Default::default()
    };
    let page = MessagePage { folder: "MSnz".into(), messages: vec![listed], ..Default::default() };
    let mut index = Default::default();
    let mut down = Default::default();
    take_window(&mut s.mailbox, &mut index, account, &page, &mut down);
    let here = s.mailbox.message(m).unwrap();
    assert_eq!(here.folder, snoozed);
    assert_eq!(here.server.as_ref().map(|x| x.id.as_str()), Some("E9"), "found again by its Message-ID");
    assert!(here.snooze.is_some_and(|z| !z.placing && z.waiting()));
}

#[test]
fn the_server_naming_another_folder_than_the_one_here_puts_the_message_there() {
    let (mut s, account, m, snoozed, _) = setup();
    // The server keeps its « Plus tard » under another id now (renamed and made anew), which this window lists too.
    let renewed = s.mailbox.add_folder_free(account, None, "Plus tard 2").unwrap();
    s.mailbox.folder_mut(renewed).unwrap().server = Some(ServerFolder { id: "MSnz2".into(), name: "Plus tard".into(), ..Default::default() });
    s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 3600})).unwrap();
    assert_eq!(s.mailbox.message(m).unwrap().folder, snoozed);
    s.hosted_snooze.take();
    s.run("mail.snoozeAnswered", &json!({"message": m, "ok": true, "record": "r1", "emailId": "E9", "mailboxId": "MSnz2"})).unwrap();
    let here = s.mailbox.message(m).unwrap();
    assert_eq!(here.folder, renewed, "where the server has it");
    assert_eq!(s.mailbox.snoozed_folder(account), Some(renewed));
    assert_eq!(moves(&s, account), [], "no move of its own goes up");
    // A folder this window doesn't list: back where the server had it, until a run lists it.
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let other = add(&mut s, inbox, "E3", "Second");
    s.run("mail.snooze", &json!({"ids": [other], "until": NOW + 3600})).unwrap();
    s.hosted_snooze.take();
    s.run("mail.snoozeAnswered", &json!({"message": other, "ok": true, "record": "r2", "emailId": "E8", "mailboxId": "Munknown"})).unwrap();
    assert_eq!(s.mailbox.message(other).unwrap().folder, inbox);
    assert_eq!(moves(&s, account), []);
    assert!(s.hosted_snooze.to_read.contains(&account));
}

#[test]
fn a_refused_return_does_not_put_back_what_a_run_showed_elsewhere() {
    let (mut s, account, _, snoozed, projects) = setup();
    let m = add(&mut s, snoozed, "E5", "Plan");
    set(&mut s, Some("MSnz"), json!([{"id": "r5", "emailId": "E5", "fromMailboxId": "Mpr", "until": NOW + 86_400}]));
    s.run("mail.unsnooze", &json!({"ids": [m]})).unwrap();
    s.hosted_snooze.take();
    // The server brought it back and the answer was lost; a run listed it in « Projets »; then the retry is refused.
    if let Some(server) = s.mailbox.message_mut(m).unwrap().server.as_mut() {
        (server.id, server.folder) = ("E6".into(), "Mpr".into());
    }
    s.run("mail.snoozeAnswered", &json!({"message": m, "ok": false, "error": "busy"})).unwrap();
    assert_eq!(s.mailbox.message(m).unwrap().folder, projects, "stays where its server has it");
    assert_eq!(moves(&s, account), []);
}

#[test]
fn put_away_again_from_the_folder_and_refused_it_keeps_the_time_it_had() {
    let (mut s, _, _, snoozed, _) = setup();
    let m = add(&mut s, snoozed, "E5", "Plan");
    set(&mut s, Some("MSnz"), json!([{"id": "r5", "emailId": "E5", "fromMailboxId": "Mpr", "until": NOW + 86_400}]));
    s.run("mail.snooze", &json!({"ids": [m], "until": NOW + 2 * 86_400})).unwrap();
    s.hosted_snooze.take();
    s.run("mail.snoozeAnswered", &json!({"message": m, "ok": false})).unwrap();
    assert!(s.mailbox.message(m).unwrap().snooze.is_some_and(|z| z.until == NOW + 86_400));
}
