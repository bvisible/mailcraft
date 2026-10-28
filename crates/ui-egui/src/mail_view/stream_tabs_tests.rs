// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of stream_tabs.rs.
//! In the window: the inbox's tabs by kind of sender appear when a server said the streams, a click on a tab lists its messages,
//! a box ticks a message, and the bar archives what is ticked or the whole tab.

use egui::PointerButton;
use mailcraft_engine::Filter;
use mailcraft_model::{FolderKind, Id};
use serde_json::json;

use crate::people_view::testing;

fn drawn(w: &mut testing::Window) -> Vec<Id> {
    let rows = w.state_mut().session.rows();
    rows.iter().filter(|row| w.state().hit.rect("message", row.id).is_some()).map(|row| row.id).collect()
}

fn click(w: &mut testing::Window, kind: &str, id: u64) {
    let pos = w.state().hit.rect(kind, id).unwrap_or_else(|| panic!("no {kind} {id} in the window")).center();
    testing::click(w, pos, PointerButton::Primary, 1);
    w.run_steps(3);
}

fn has(w: &testing::Window, kind: &str, id: u64) -> bool {
    w.state().hit.rect(kind, id).is_some()
}

fn folder_kind(w: &testing::Window, id: Id) -> Option<FolderKind> {
    let s = &w.state().session;
    s.mailbox.message(id).and_then(|m| s.mailbox.folder(m.folder)).map(|f| f.kind)
}

/// The sample's Inbox, its first messages said to be: `notifications` of them notifications, `news` newsletters, one a person's.
/// Returns (notifications, newsletters).
fn streamed(w: &mut testing::Window, notifications: usize, news: usize) -> (Vec<Id>, Vec<Id>) {
    w.state_mut().execute("ui.dismissReminder", json!({"all": true})).unwrap();
    w.state_mut().execute("folder.select", json!({"kind": "inbox"})).unwrap();
    w.run_steps(3);
    let ids: Vec<Id> = w.state_mut().session.rows().iter().map(|r| r.id).collect();
    assert!(ids.len() > notifications + news + 1, "the sample's Inbox is long enough: {}", ids.len());
    let (n, rest) = ids.split_at(notifications);
    let (l, rest) = rest.split_at(news);
    let mut streams: Vec<serde_json::Value> = n.iter().map(|id| json!({"id": id, "stream": "notification"})).collect();
    streams.extend(l.iter().map(|id| json!({"id": id, "stream": "newsletter"})));
    streams.push(json!({"id": rest[0], "stream": "person"}));
    w.state_mut().execute("mail.setStreams", json!({"streams": streams})).unwrap();
    w.run_steps(3);
    (n.to_vec(), l.to_vec())
}

#[test]
fn without_streams_the_header_has_all_and_unread() {
    let mut w = testing::window("view.mail");
    w.state_mut().execute("ui.dismissReminder", json!({"all": true})).unwrap();
    w.run_steps(3);
    assert!(has(&w, "listTab", 0) && has(&w, "listTab", 1));
    let first = drawn(&mut w)[0];
    assert!(!has(&w, "streamTab", 0) && !has(&w, "messageTick", first));
    assert!(!has(&w, "streamArchiveAll", 0));
}

#[test]
fn with_streams_the_three_tabs_replace_them_and_the_inbox_opens_on_the_main_one() {
    let mut w = testing::window("view.mail");
    streamed(&mut w, 3, 2);
    assert!(!has(&w, "listTab", 0) && !has(&w, "listTab", 1));
    assert!((0..3).all(|n| has(&w, "streamTab", n)));
    assert_eq!(w.state().session.view.filter, Filter::Main);
    // Main is not a tab with a bar to archive its whole
    assert!(!has(&w, "streamArchiveAll", 0));
    // each row wears its box
    let listed = drawn(&mut w);
    assert!(!listed.is_empty() && listed.iter().all(|id| has(&w, "messageTick", *id)));
}

#[test]
fn a_click_on_a_tab_lists_what_is_in_it_and_the_bar_archives_the_whole_tab() {
    let mut w = testing::window("view.mail");
    let (notifications, news) = streamed(&mut w, 3, 2);
    click(&mut w, "streamTab", 1);
    assert_eq!(w.state().session.view.filter, Filter::Notifications);
    let mut listed = drawn(&mut w);
    listed.sort_unstable();
    let mut want = notifications.clone();
    want.sort_unstable();
    assert_eq!(listed, want);
    assert!(has(&w, "streamArchiveAll", 0), "the tab has its bar");
    click(&mut w, "streamArchiveAll", 0);
    assert!(notifications.iter().all(|id| folder_kind(&w, *id) == Some(FolderKind::Archive)));
    assert!(news.iter().all(|id| folder_kind(&w, *id) == Some(FolderKind::Inbox)), "the newsletters are left alone");
    // nothing is left in the tab: no bar
    assert!(!has(&w, "streamArchiveAll", 0));
}

#[test]
fn boxes_tick_messages_and_the_bar_archives_those_only() {
    let mut w = testing::window("view.mail");
    let (notifications, _) = streamed(&mut w, 4, 2);
    click(&mut w, "streamTab", 1);
    click(&mut w, "messageTick", notifications[0]);
    click(&mut w, "messageTick", notifications[2]);
    assert_eq!(w.state().session.sel.messages, vec![notifications[0], notifications[2]]);
    assert!(has(&w, "streamArchive", 0) && !has(&w, "streamArchiveAll", 0));
    // a second click on a box takes the tick back
    click(&mut w, "messageTick", notifications[2]);
    assert_eq!(w.state().session.sel.messages, vec![notifications[0]]);
    click(&mut w, "messageTick", notifications[3]);
    click(&mut w, "streamArchive", 0);
    let moved: Vec<bool> = notifications.iter().map(|id| folder_kind(&w, *id) == Some(FolderKind::Archive)).collect();
    assert_eq!(moved, [true, false, false, true]);
}

#[test]
fn select_all_and_clear_selection() {
    let mut w = testing::window("view.mail");
    let (notifications, _) = streamed(&mut w, 3, 2);
    click(&mut w, "streamTab", 1);
    click(&mut w, "messageTick", notifications[0]);
    assert!(has(&w, "streamSelectAll", 0), "a list with only some ticked offers all of it");
    click(&mut w, "streamSelectAll", 0);
    assert_eq!(w.state().session.sel.messages.len(), 3);
    assert!(!has(&w, "streamSelectAll", 0), "everything is ticked");
    assert!(has(&w, "streamClear", 0), "and the tick can be cleared");
    click(&mut w, "streamClear", 0);
    assert!(w.state().session.sel.messages.is_empty());
    assert!(has(&w, "streamArchiveAll", 0));
}

#[test]
fn archiving_a_long_tab_asks_once_more() {
    let mut w = testing::window("view.mail");
    w.state_mut().execute("ui.dismissReminder", json!({"all": true})).unwrap();
    w.state_mut().execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let inbox = w.state().session.mailbox.inbox().unwrap();
    let mut added = Vec::new();
    for n in 0..25 {
        let m = mailcraft_model::Message { folder: inbox, subject: format!("alert {n}"), read: true, ..Default::default() };
        added.push(w.state_mut().session.mailbox.add_message(m).unwrap());
    }
    let streams: Vec<_> = added.iter().map(|id| json!({"id": id, "stream": "notification"})).collect();
    w.state_mut().execute("mail.setStreams", json!({"streams": streams})).unwrap();
    w.state_mut().execute("view.filter", json!({"value": "notifications"})).unwrap();
    w.run_steps(3);
    click(&mut w, "streamArchiveAll", 0);
    assert!(added.iter().all(|id| folder_kind(&w, *id) == Some(FolderKind::Inbox)), "the first click only asks");
    assert!(has(&w, "streamArchiveYes", 0) && has(&w, "streamArchiveNo", 0));
    click(&mut w, "streamArchiveNo", 0);
    assert!(has(&w, "streamArchiveAll", 0) && added.iter().all(|id| folder_kind(&w, *id) == Some(FolderKind::Inbox)));
    click(&mut w, "streamArchiveAll", 0);
    click(&mut w, "streamArchiveYes", 0);
    assert!(added.iter().all(|id| folder_kind(&w, *id) == Some(FolderKind::Archive)));
}

#[test]
fn the_tabs_go_with_the_inbox() {
    let mut w = testing::window("view.mail");
    streamed(&mut w, 3, 2);
    click(&mut w, "streamTab", 2);
    assert_eq!(w.state().session.view.filter, Filter::Newsletters);
    w.state_mut().execute("folder.select", json!({"kind": "archive"})).unwrap();
    w.run_steps(3);
    assert_eq!(w.state().session.view.filter, Filter::All);
    assert!(has(&w, "listTab", 0) && !has(&w, "streamTab", 0));
}
