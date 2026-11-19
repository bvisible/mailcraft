// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of stream_correct.rs.
//! In the window of a hosting page's tab whose mailbox's server keeps a choice of tab: « Classer dans » in a message's menu
//! puts it under the tab chosen and asks the server, the Home tab has it, a refusal is said, and nothing of it where the window
//! keeps its mail or the server does not keep the choice. Every text it shows has its French.

use egui::PointerButton;
use egui_kittest::kittest::Queryable as _;
use mailcraft_model::sync::ServerMessage;
use mailcraft_model::{Id, ServerFolder, Stream};
use serde_json::json;

use super::*;
use crate::people_view::testing;

/// The sample's Inbox as a hosting page's tab shows it (`hosted`), its mail on a server (each message with its id there) that
/// keeps a choice of tab (`server`). The window.
fn inbox(hosted: bool, server: bool) -> testing::Window {
    let mut w = testing::window("view.mail");
    w.state_mut().session.keeps_local = !hosted;
    w.state_mut().execute("ui.dismissReminder", json!({"all": true})).unwrap();
    w.state_mut().execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let s = &mut w.state_mut().session;
    let inbox = s.view.folder;
    let account = s.mailbox.folder(inbox).unwrap().account;
    s.mailbox.folder_mut(inbox).unwrap().server = Some(ServerFolder { id: "Min".into(), name: "INBOX".into(), ..Default::default() });
    let ids: Vec<Id> = s.mailbox.messages_in(inbox).map(|m| m.id).collect();
    for id in ids {
        let m = s.mailbox.message_mut(id).unwrap();
        m.server = Some(ServerMessage { id: format!("E{id}"), folder: "Min".into(), read: m.read, ..Default::default() });
    }
    if server {
        let email = s.mailbox.account(account).unwrap().email.clone();
        s.run("mail.offerStreamCorrection", &json!({"account": email, "offered": true})).unwrap();
    }
    w.run_steps(3);
    w
}

fn first_row(w: &mut testing::Window) -> Id {
    let rows = w.state_mut().session.rows();
    rows.iter().map(|r| r.id).find(|id| w.state().hit.rect("message", *id).is_some()).expect("a row drawn")
}

#[test]
fn a_messages_menu_puts_it_under_the_tab_chosen_and_asks_its_server() {
    let mut w = inbox(true, true);
    let first = first_row(&mut w);
    let pos = w.state().hit.rect("message", first).unwrap().center();
    testing::click(&mut w, pos, PointerButton::Secondary, 1);
    w.get_by_label_contains(tlc!("stream", "Move to Tab")).click();
    w.run_steps(3);
    w.get_by_label_contains(tl!("Newsletters")).click();
    w.run_steps(3);
    assert_eq!(w.state().session.mailbox.message(first).unwrap().stream, Some(Stream::Newsletter));
    let asked = w.state_mut().session.hosted_stream.take();
    assert_eq!(asked.len(), 1);
    assert_eq!((asked[0].message, asked[0].stream), (first, Stream::Newsletter));
}

#[test]
fn nothing_of_it_where_the_window_keeps_its_mail_or_its_server_does_not_keep_the_choice() {
    for (hosted, server) in [(false, true), (true, false)] {
        let mut w = inbox(hosted, server);
        let first = first_row(&mut w);
        assert!(!offered(w.state(), &[first]), "hosted {hosted}, server {server}");
        let pos = w.state().hit.rect("message", first).unwrap().center();
        testing::click(&mut w, pos, PointerButton::Secondary, 1);
        assert!(w.query_by_label_contains(tlc!("stream", "Move to Tab")).is_none(), "hosted {hosted}, server {server}");
        let tab = crate::ribbon::describe_tab(w.state(), "Home");
        assert!(tab.groups.iter().flat_map(|g| &g.items).all(|i| i.key != "moveToTab"), "hosted {hosted}, server {server}");
    }
}

#[test]
fn the_home_tab_has_it_in_the_tags_group() {
    let w = inbox(true, true);
    let tab = crate::ribbon::describe_tab(w.state(), "Home");
    let tags = tab.groups.iter().find(|g| g.title == "Tags").unwrap();
    let item = tags.items.iter().find(|i| i.key == "moveToTab").expect("in the Tags group");
    assert_eq!(item.menu, Some(crate::ribbon::Menu::StreamTab));
}

#[test]
fn a_refusal_of_the_server_is_said_and_the_message_comes_back() {
    let mut w = inbox(true, true);
    let first = first_row(&mut w);
    let before = w.state().session.mailbox.message(first).unwrap().stream;
    w.state_mut().execute("mail.correctStream", json!({"ids": [first], "stream": "notification"})).unwrap();
    w.state_mut().session.hosted_stream.take();
    w.state_mut().execute("mail.streamCorrected", json!({"message": first, "ok": false})).unwrap();
    w.run_steps(3);
    assert_eq!(w.state().session.mailbox.message(first).unwrap().stream, before);
    assert_eq!(testing::dialog(&w), Some("refused"), "the person is told");
}

/// The texts are this fork's own: they live in `po/neoffice/`, not in upstream's generated catalogues.
#[test]
fn every_text_it_shows_has_a_french_translation() {
    let french = mailcraft_i18n::catalogue(mailcraft_i18n::Lang::French).expect("French is compiled in");
    let plain = ["Move to\nTab", "Put the messages under another tab of the Inbox: their sender's next ones will come under it too", "Move to Tab"];
    let reasons = [
        "this mailbox's server doesn't keep a choice of tab",
        "only a message of the Inbox goes under a tab",
        "too many messages at once: choose fewer",
        "'{subject}' could not be put under that tab: it stays where it was.",
        "Offer Stream Correction",
        "Stream Corrected",
    ];
    let missing: Vec<&&str> = plain.iter().chain(&reasons).filter(|text| french.get(text).is_none()).collect();
    assert!(missing.is_empty(), "no French for {missing:?}: add them to crates/i18n/po/neoffice/fr.po");
}
