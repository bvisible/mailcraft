// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A hosting page that takes a message to share in its team chat: where the window offers it, and the one thing the
//! page is handed. Invented people at `example.org`.

use std::cell::RefCell;
use std::rc::Rc;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::jmap_mail::apply_jmap_mail;
use mailcraft_model::sync::{FolderListing, GraphFolder, GraphMessage, JmapMailResult, MessagePage, ServerMessage};
use mailcraft_model::{Address, FolderKind, Id, Mailbox};
use serde_json::{Value, json};

use super::*;
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const ADA: &str = "ada@example.org";
const BOB: &str = "bob@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session";

fn mail(id: &str, folder: &str) -> GraphMessage {
    GraphMessage {
        server: ServerMessage { id: id.into(), folder: folder.into(), read: true, ..Default::default() },
        message_id: format!("<{id}@mail.example.org>"),
        subject: "Delivery of the saw".into(),
        from: Address::new("Grace Hopper", "grace@example.org"),
        sent: NOW - 3_600,
        received: NOW - 3_600,
        ..Default::default()
    }
}

/// A JMAP mailbox added as a hosting page adds it, with an Inbox holding `ids`.
fn mailbox(app: &mut MailApp, email: &str, ids: &[&str]) -> Id {
    let added = app.execute("account.add", json!({"email": email, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
    let account = added["id"].as_u64().unwrap();
    let folders = vec![GraphFolder { id: "Min".into(), name: "INBOX".into(), kind: FolderKind::Inbox, ..Default::default() }];
    let listing = JmapMailResult { account, email: email.into(), listing: Some(FolderListing { folders, complete: true }), ..Default::default() };
    apply_jmap_mail(&mut app.session, listing).unwrap();
    let page = MessagePage { folder: "Min".into(), messages: ids.iter().map(|id| mail(id, "Min")).collect(), afresh: true, ..Default::default() };
    apply_jmap_mail(&mut app.session, JmapMailResult { account, email: email.into(), pages: vec![page], whole: true, ..Default::default() }).unwrap();
    account
}

/// What the page was handed, message by message.
type Handed = Rc<RefCell<Vec<Value>>>;

/// A hosting page's window: Ada's mailbox, handed over (in the page's own case), and Bob's, which the person added by
/// hand. With `share`, the page takes messages to share, and what it is handed is kept.
fn page(share: bool) -> (MailApp, Id, Id, Handed) {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    app.services.jmap_host_credentials = true;
    app.session.keeps_local = false;
    let ada = mailbox(&mut app, ADA, &["A1", "A2"]);
    let bob = mailbox(&mut app, BOB, &["B1"]);
    let handed: Handed = Rc::default();
    if share {
        let kept = handed.clone();
        app.services.share =
            Some(Share { mailboxes: vec!["Ada@Example.org".into()], post: Box::new(move |message| kept.borrow_mut().push(message.clone())) });
    }
    app.execute("folder.select", json!({"kind": "inbox", "account": ada})).unwrap();
    (app, ada, bob, handed)
}

/// The window's message whose id on the server is `server`.
fn message(app: &MailApp, server: &str) -> Id {
    app.session.mailbox.messages.values().find(|m| m.server.as_ref().is_some_and(|s| s.id == server)).map(|m| m.id).unwrap()
}

fn select(app: &mut MailApp, server: &str) {
    let id = message(app, server);
    app.execute("mail.select", json!({"id": id})).unwrap();
}

/// The keys of the items of the Respond group of the tab in view.
fn respond(app: &MailApp) -> Vec<&'static str> {
    let tab = crate::ribbon::describe(app);
    tab.groups.iter().find(|g| g.title == "Respond").map(|g| g.items.iter().map(|i| i.key).collect()).unwrap_or_default()
}

#[test]
fn the_page_is_handed_the_message_s_mailbox_and_its_id_on_the_server_and_nothing_else() {
    let (mut app, _, _, handed) = page(true);
    select(&mut app, "A2");
    assert!(app.enabled(COMMAND));
    assert_eq!(app.run(COMMAND, json!({})).unwrap(), json!({"shared": true}));
    // The address as the page handed it over, the id as its server gave it.
    assert_eq!(handed.borrow().as_slice(), [json!({"type": SHARE_TYPE, "account": "Ada@Example.org", "email": "A2"})]);
    assert_eq!(super::message("Ada@Example.org", "A2"), json!({"type": SHARE_TYPE, "account": "Ada@Example.org", "email": "A2"}));
}

#[test]
fn only_a_message_of_a_mailbox_the_page_handed_over_that_its_server_has_is_shared() {
    let (mut app, _, _, handed) = page(true);
    // Nothing selected.
    assert!(app.session.sel.focus().is_none());
    assert!(!app.enabled(COMMAND));
    assert!(app.run(COMMAND, json!({})).is_err());
    // A mailbox the person added by hand.
    select(&mut app, "B1");
    assert!(!app.enabled(COMMAND));
    assert_eq!(target(&app).unwrap_err(), "Only a message of a mailbox the page handed over can be shared.");
    // A message its server no longer has.
    select(&mut app, "A1");
    let id = message(&app, "A1");
    if let Some(server) = app.session.mailbox.messages.get_mut(&id).and_then(|m| std::sync::Arc::make_mut(m).server.as_mut()) {
        server.gone = true;
    }
    assert_eq!(target(&app).unwrap_err(), "This message isn't on its server yet.");
    assert!(handed.borrow().is_empty(), "nothing went to the page");
}

#[test]
fn the_button_is_in_mail_s_respond_group_only_where_a_page_takes_messages_to_share() {
    let (app, _, _, _) = page(false);
    assert!(!respond(&app).contains(&COMMAND));
    assert!(!app.enabled(COMMAND));
    let (mut app, ada, _, _) = page(true);
    assert!(respond(&app).contains(&COMMAND));
    // Not in the other modules' tabs.
    app.execute("view.calendar", json!({})).unwrap();
    assert!(!respond(&app).contains(&COMMAND));
    app.execute("view.mail", json!({})).unwrap();
    app.execute("folder.select", json!({"kind": "inbox", "account": ada})).unwrap();
    assert!(respond(&app).contains(&COMMAND));
}

#[test]
fn chat_in_synk_hands_the_page_the_message_and_with_whom_and_nothing_else() {
    let (mut app, _, _, handed) = page(true);
    select(&mut app, "A1");
    assert!(app.enabled(CHAT_SENDER));
    assert!(app.enabled(CHAT_ALL));
    assert_eq!(app.run(CHAT_SENDER, json!({})).unwrap(), json!({"chat": "sender"}));
    assert_eq!(app.run(CHAT_ALL, json!({})).unwrap(), json!({"chat": "all"}));
    assert_eq!(
        handed.borrow().as_slice(),
        [
            json!({"type": CHAT_TYPE, "account": "Ada@Example.org", "email": "A1", "who": "sender"}),
            json!({"type": CHAT_TYPE, "account": "Ada@Example.org", "email": "A1", "who": "all"}),
        ]
    );
    // The same messages as Share in Synk: not one of a mailbox added by hand.
    select(&mut app, "B1");
    assert!(!app.enabled(CHAT_SENDER));
    assert!(app.run(CHAT_ALL, json!({})).is_err());
    assert_eq!(handed.borrow().len(), 2);
}

#[test]
fn chat_in_synk_is_a_menu_beside_share_in_synk_only_where_a_page_takes_messages() {
    let (app, _, _, _) = page(false);
    assert!(!respond(&app).contains(&"hostChat"));
    assert!(!app.enabled(CHAT_SENDER));
    let (app, _, _, _) = page(true);
    let tab = crate::ribbon::describe(&app);
    let respond = tab.groups.iter().find(|g| g.title == "Respond").unwrap();
    let keys: Vec<&str> = respond.items.iter().map(|i| i.key).collect();
    let share = keys.iter().position(|k| *k == COMMAND).unwrap();
    assert_eq!(keys.get(share + 1), Some(&"hostChat"));
    let chat = respond.items.iter().find(|i| i.key == "hostChat").unwrap();
    assert_eq!(chat.menu, Some(Menu::HostChat));
    assert!(chat.command.is_none());
}
