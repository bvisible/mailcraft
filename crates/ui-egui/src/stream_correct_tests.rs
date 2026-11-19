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

fn stream_of(w: &testing::Window, id: Id) -> Option<Stream> {
    w.state().session.mailbox.message(id).unwrap().stream
}

fn click_hit(w: &mut testing::Window, kind: &str) {
    let pos = w.state().hit.rect(kind, 0).unwrap_or_else(|| panic!("no {kind} in the window")).center();
    testing::click(w, pos, PointerButton::Primary, 1);
    w.run_steps(3);
}

/// The first row's message and another message of the Inbox made one of the same sender. The window, them.
fn two_of_one_sender() -> (testing::Window, Id, Id) {
    let mut w = inbox(true, true);
    let first = first_row(&mut w);
    let (folder, from) = {
        let m = w.state().session.mailbox.message(first).unwrap();
        (m.folder, m.from.clone())
    };
    let other = w.state().session.mailbox.messages_in(folder).map(|m| m.id).find(|id| *id != first).unwrap();
    w.state_mut().session.mailbox.message_mut(other).unwrap().from = from;
    (w, first, other)
}

#[test]
fn after_a_choice_the_bar_offers_the_senders_next_ones_and_yes_takes_its_others() {
    let (mut w, first, other) = two_of_one_sender();
    w.state_mut().execute("mail.correctStream", json!({"ids": [first], "stream": "newsletter"})).unwrap();
    w.run_steps(3);
    assert_eq!(stream_of(&w, first), Some(Stream::Newsletter));
    assert_ne!(stream_of(&w, other), Some(Stream::Newsletter), "the message alone, until the person says yes");
    assert!(w.state().hit.rect("streamOfferYes", 0).is_some(), "the offer is shown");
    w.state_mut().session.hosted_stream.take();
    click_hit(&mut w, "streamOfferYes");
    assert_eq!(stream_of(&w, other), Some(Stream::Newsletter), "the sender's other follows");
    let asked = w.state_mut().session.hosted_stream.take();
    assert!(asked.iter().any(|a| a.message == first && a.sender), "the server is asked to keep the sender");
    assert!(w.state().session.hosted_stream.proposal.is_none());
    assert!(w.state().hit.rect("streamOfferYes", 0).is_none(), "and the bar is gone");
}

#[test]
fn closing_the_offer_leaves_the_senders_other_messages_where_they_are() {
    let (mut w, first, other) = two_of_one_sender();
    let before = stream_of(&w, other);
    w.state_mut().execute("mail.correctStream", json!({"ids": [first], "stream": "notification"})).unwrap();
    w.run_steps(3);
    click_hit(&mut w, "streamOfferClose");
    assert_eq!(stream_of(&w, other), before);
    assert!(w.state().session.hosted_stream.proposal.is_none());
}

#[test]
fn the_offer_goes_by_itself_after_its_time() {
    let (mut w, first, _) = two_of_one_sender();
    w.state_mut().execute("mail.correctStream", json!({"ids": [first], "stream": "newsletter"})).unwrap();
    w.run_steps(3);
    let now = w.state().session.now();
    if let Some(offer) = w.state_mut().session.hosted_stream.proposal.as_mut() {
        offer.at = now - OFFER_S - 1;
    }
    w.run_steps(3);
    assert!(w.state().session.hosted_stream.proposal.is_none());
    assert!(w.state().hit.rect("streamOfferYes", 0).is_none());
}

#[test]
fn the_offer_names_its_sender_or_counts_them_and_has_shorter_ways() {
    let one = mailcraft_engine::cmd::stream_correct::Proposal { ids: vec![1], stream: Stream::Newsletter, senders: vec!["The Shop".into()], at: 0 };
    let (words, button, shorter) = offer_text(&one);
    assert_eq!((words.as_str(), button.as_str()), ("Moved to Newsletters.", "The next ones from The Shop too"));
    assert_eq!(shorter.first().map(String::as_str), Some("The next ones from The Sho… too"));
    assert_eq!(shorter.last().map(String::as_str), Some("The next ones too"));
    let many = mailcraft_engine::cmd::stream_correct::Proposal { senders: vec!["A".into(), "B".into()], ..one };
    assert_eq!(offer_text(&many).1, "The next ones from these 2 senders too");
}

/// A hosting page with Nora's bar over its Inbox, two messages of one sender there, the first put under Newsletters alone; the
/// window drawn, and the two messages.
fn nora_page_after_a_choice() -> (egui_kittest::Harness<'static, MailApp>, [Id; 2]) {
    let (mut app, _calls, _) = crate::nora::testing::page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let s = &mut app.session;
    let inbox = s.view.folder;
    let account = s.mailbox.folder(inbox).unwrap().account;
    s.mailbox.folder_mut(inbox).unwrap().server = Some(ServerFolder { id: "Min".into(), name: "INBOX".into(), ..Default::default() });
    let from = mailcraft_model::Address::new("The Shop", "news@shop.example");
    let mut ids = [0; 2];
    for (n, id) in ids.iter_mut().enumerate() {
        let m = mailcraft_model::Message {
            folder: inbox,
            account,
            from: from.clone(),
            subject: format!("Offers {n}"),
            server: Some(ServerMessage { id: format!("S{n}"), folder: "Min".into(), ..Default::default() }),
            ..Default::default()
        };
        *id = s.mailbox.add_message(m).unwrap();
    }
    let email = s.mailbox.account(account).unwrap().email.clone();
    s.run("mail.offerStreamCorrection", &json!({"account": email, "offered": true})).unwrap();
    s.run("mail.correctStream", &json!({"ids": [ids[0]], "stream": "newsletter"})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    h.run_steps(4);
    (h, ids)
}

fn click_line(h: &mut egui_kittest::Harness<'static, MailApp>, hit: &str) {
    let at = h.state().hit.rect(hit, 0).unwrap_or_else(|| panic!("no {hit}")).center();
    crate::people_view::testing::click(h, at, PointerButton::Primary, 1);
    h.run_steps(3);
}

/// Where Nora's bar is drawn, the offer is the line above it, as the one that follows a ⚡: no bar of its own in the list.
#[test]
fn with_noras_bar_the_offer_is_its_line_and_its_button_takes_the_senders_others() {
    let (mut h, ids) = nora_page_after_a_choice();
    assert!(h.state().hit.rect("noraHintButton", 0).is_some(), "the line above Nora's bar says it");
    assert!(h.state().hit.rect("streamOfferYes", 0).is_none(), "and no bar of its own pushes the list down");
    assert!(h.query_all_by_label_contains("The next ones from The Shop too").next().is_some());
    click_line(&mut h, "noraHintButton");
    assert_eq!(h.state().session.mailbox.message(ids[1]).unwrap().stream, Some(Stream::Newsletter), "the sender's other follows");
    assert!(h.state().session.hosted_stream.proposal.is_none());
}

#[test]
fn the_lines_close_leaves_the_senders_other_message_where_it_was() {
    let (mut h, ids) = nora_page_after_a_choice();
    let before = h.state().session.mailbox.message(ids[1]).unwrap().stream;
    click_line(&mut h, "noraHintClose");
    assert_eq!(h.state().session.mailbox.message(ids[1]).unwrap().stream, before);
    assert_eq!(h.state().session.mailbox.message(ids[0]).unwrap().stream, Some(Stream::Newsletter), "the message chosen stays");
    assert!(h.state().session.hosted_stream.proposal.is_none());
    assert!(h.state().hit.rect("noraHintButton", 0).is_none(), "and the line is gone");
}

/// The line says where the message went: whole where it has room, as « → Newsletters » where the list is narrow; in the very
/// narrowest line (about 260 px) only its shortest button is left, the whole of its words under the pointer.
#[test]
fn the_line_says_where_the_message_went_at_any_width() {
    let (mut app, _calls, _) = crate::nora::testing::page(true);
    app.session.hosted_stream.proposal = Some(mailcraft_engine::cmd::stream_correct::Proposal {
        ids: vec![1],
        stream: Stream::Newsletter,
        senders: vec!["Nora".into()],
        at: app.session.now(),
    });
    let line = hint(&app).expect("the offer is a line above Nora's bar");
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::theme::font_definitions_with(&crate::theme::SystemFont::none()));
    ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
    let mut said = Vec::new();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        for widest in [640.0, 360.0, 300.0] {
            said.push(crate::nora::ask_bar::words_said(ui, &line, widest));
        }
    })
    .drop_without_applying_deltas();
    assert_eq!(said[0].as_deref(), Some("Moved to Newsletters."), "whole where there is room");
    for words in &said[1..] {
        assert!(words.as_deref().is_some_and(|w| w.contains("Newsletters")), "the tab is said in a narrow list too: {said:?}");
    }
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
        "Moved to {tab}.",
        "The next ones from {sender} too",
        "The next ones from these {count} senders too",
        "The next ones too",
        "→ {tab}",
    ];
    let missing: Vec<&&str> = plain.iter().chain(&reasons).filter(|text| french.get(text).is_none()).collect();
    assert!(missing.is_empty(), "no French for {missing:?}: add them to crates/i18n/po/neoffice/fr.po");
}
