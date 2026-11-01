// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of exchanges.rs.
use mailcraft_engine::Session;
use mailcraft_model::exchanges::{Linked, Page, Row, Someone, Way};
use mailcraft_model::{Address, FolderKind, Mailbox, Message};
use serde_json::json;

use super::*;
use crate::Services;

const CLIENT: &str = "claire@atelier-exemple.example";

fn someone() -> Someone {
    Someone { address: CLIENT.into(), known: true, addresses: vec![CLIENT.into()], ..Default::default() }
}

fn row(server: &str, when: i64) -> Row {
    Row {
        message: None,
        server: Some(server.into()),
        subject: format!("Subject {server}"),
        name: String::new(),
        email: CLIENT.into(),
        when,
        folder: "Archive".into(),
        kind: Some(FolderKind::Archive),
        way: Way::In,
        unread: false,
        clip: false,
        files: Vec::new(),
    }
}

fn page(rows: Vec<Row>, next: Option<&str>) -> Page {
    Page { rows, total: Some(3), next: next.map(str::to_string), folders: (2, 4), partial: false }
}

#[test]
fn nothing_is_asked_of_a_server_where_no_host_reaches_it() {
    let mut ex = Exchanges::default();
    assert!(ex.someone_of(0.0, "ada@example.org", CLIENT).is_none());
    assert!(matches!(ex.linked_of(0.0, "ada@example.org", CLIENT), Asked::Unsupported));
    assert!(!ex.remote_of(0.0, "ada@example.org", &[CLIENT.into()]).loaded);
    assert!(ex.take_wants().is_empty());
}

#[test]
fn who_someone_is_is_asked_once_and_again_only_after_a_failure_has_aged() {
    let mut ex = Exchanges::default();
    ex.set_hosted(true);
    assert!(ex.someone_of(0.0, "ada@example.org", "Claire@Atelier-Exemple.example").is_none());
    assert!(ex.someone_of(10.0, "ada@example.org", CLIENT).is_none());
    assert_eq!(ex.take_wants(), vec![Want::Someone { mailbox: "ada@example.org".into(), address: "Claire@Atelier-Exemple.example".into() }]);
    ex.put_someone(CLIENT, Ok(someone()));
    assert_eq!(ex.someone_of(20.0, "ada@example.org", CLIENT).map(|s| s.address), Some(CLIENT.to_string()));
    assert!(ex.take_wants().is_empty(), "known: not asked again");
    // A failure waits, then is asked again.
    let mut ex = Exchanges::default();
    ex.set_hosted(true);
    ex.put_someone(CLIENT, Err(Trouble::Failed));
    assert!(ex.someone_of(crate::now_ms() + 1_000.0, "ada@example.org", CLIENT).is_none());
    assert!(ex.take_wants().is_empty());
    assert!(ex.someone_of(crate::now_ms() + RETRY_MS + 1_000.0, "ada@example.org", CLIENT).is_none());
    assert_eq!(ex.take_wants().len(), 1);
    // A server that has no such method is never asked again.
    ex.put_someone(CLIENT, Err(Trouble::Unsupported));
    assert!(ex.someone_of(f64::MAX / 2.0, "ada@example.org", CLIENT).is_none());
    assert!(ex.take_wants().is_empty());
}

#[test]
fn the_first_page_is_asked_once_then_the_next_after_it_and_no_more_at_the_end() {
    let mut ex = Exchanges::default();
    ex.set_hosted(true);
    let addresses = vec![CLIENT.to_string()];
    assert!(!ex.remote_of(0.0, "ada@example.org", &addresses).loaded);
    assert!(!ex.remote_of(1.0, "ada@example.org", &addresses).loaded, "on its way: not asked twice");
    let wants = ex.take_wants();
    assert_eq!(wants.len(), 1);
    let Want::Page { key, before, addresses: asked, cc, domain, .. } = &wants[0] else { panic!("{wants:?}") };
    assert_eq!((before.clone(), asked.clone(), *cc, *domain), (None, addresses.clone(), false, false));
    let key = key.clone();
    ex.put_page(&key, false, Ok(page(vec![row("E1", 300), row("E2", 200)], Some("2026-10-29T10:00:00Z"))));
    let remote = ex.remote_of(2.0, "ada@example.org", &addresses);
    assert!(remote.loaded);
    assert_eq!((remote.rows.len(), remote.total, remote.folders), (2, Some(3), (2, 4)));
    assert!(ex.take_wants().is_empty(), "loaded: not asked again");
    ex.ask_more("ada@example.org", &addresses);
    ex.ask_more("ada@example.org", &addresses);
    let wants = ex.take_wants();
    assert_eq!(wants.len(), 1, "one page at a time");
    assert!(matches!(&wants[0], Want::Page { before: Some(b), .. } if b == "2026-10-29T10:00:00Z"));
    // The page after, added to the first (one row both have counts once); the last has no next.
    ex.put_page(&key, true, Ok(page(vec![row("E2", 200), row("E3", 100)], None)));
    let remote = ex.remote_of(3.0, "ada@example.org", &addresses);
    assert_eq!(remote.rows.iter().filter_map(|r| r.server.as_deref()).collect::<Vec<_>>(), ["E1", "E2", "E3"]);
    assert!(remote.next.is_none());
    ex.ask_more("ada@example.org", &addresses);
    assert!(ex.take_wants().is_empty(), "no more to ask");
}

#[test]
fn a_page_that_failed_is_asked_again_when_told_to_and_a_server_without_the_method_ends_the_asking() {
    let mut ex = Exchanges::default();
    ex.set_hosted(true);
    let addresses = vec![CLIENT.to_string()];
    ex.remote_of(0.0, "ada@example.org", &addresses);
    let key = remote_key(&addresses, false, false);
    ex.take_wants();
    ex.put_page(&key, false, Err(Trouble::Failed));
    assert!(matches!(ex.remote_of(1.0, "ada@example.org", &addresses).fetch, Fetch::Failed(_)));
    assert!(ex.take_wants().is_empty());
    ex.retry(&addresses);
    ex.remote_of(2.0, "ada@example.org", &addresses);
    assert_eq!(ex.take_wants().len(), 1);
    ex.put_page(&key, false, Err(Trouble::Unsupported));
    assert!(!ex.hosted(), "the server has no such method: the window lists what it has");
}

#[test]
fn the_options_change_what_the_server_is_asked_and_where_it_is_kept() {
    let mut ex = Exchanges::default();
    ex.set_hosted(true);
    let addresses = vec![CLIENT.to_string()];
    ex.remote_of(0.0, "ada@example.org", &addresses);
    ex.cc = true;
    ex.remote_of(1.0, "ada@example.org", &addresses);
    let wants = ex.take_wants();
    assert_eq!(wants.len(), 2);
    assert!(matches!(&wants[1], Want::Page { cc: true, domain: false, .. }));
    assert_ne!(remote_key(&addresses, false, false), remote_key(&addresses, true, false));
}

#[test]
fn what_neoffice_links_someone_to_is_asked_once() {
    let mut ex = Exchanges::default();
    ex.set_hosted(true);
    assert!(matches!(ex.linked_of(0.0, "ada@example.org", CLIENT), Asked::Waiting));
    assert!(matches!(ex.linked_of(1.0, "ada@example.org", CLIENT), Asked::Waiting));
    assert_eq!(ex.take_wants(), vec![Want::Linked { mailbox: "ada@example.org".into(), address: CLIENT.into() }]);
    ex.put_linked(CLIENT, Ok(Linked::default()));
    assert!(matches!(ex.linked_of(2.0, "ada@example.org", CLIENT), Asked::Ready(_)));
    assert!(ex.take_wants().is_empty());
}

#[test]
fn the_pane_opens_and_closes_by_its_command_and_takes_a_junk_param_for_none() {
    let mut app = crate::MailApp::new(Session::at(Mailbox::with_local_account("Ada", "ada@example.org"), 0, 0), Services::default());
    assert!(!shown(&app));
    assert_eq!(toggle(&mut app, &json!({})).unwrap(), json!({"open": true}));
    assert!(shown(&app));
    assert_eq!(toggle(&mut app, &json!({"open": false})).unwrap(), json!({"open": false}));
    assert!(toggle(&mut app, &json!({"open": "yes"})).is_err());
    assert!(!shown(&app), "a refused command changes nothing");
    assert!(ui_command(&mut app, "ui.somethingElse", &json!({})).is_none());
    for junk in [json!(null), json!([1, 2]), json!("x"), json!({"open": null}), json!({"open": 3})] {
        let _ = ui_command(&mut app, "ui.exchanges", &junk);
    }
}

#[test]
fn the_pane_follows_who_wrote_the_message_or_who_it_went_to() {
    let mut mb = Mailbox::with_local_account("Ada", "ada@example.org");
    let account = mb.accounts[0].id;
    let inbox = mb.inbox().unwrap();
    let sent = mb.folder_of_kind(account, FolderKind::Sent).unwrap();
    let received = mb
        .add_message(Message {
            folder: inbox,
            from: Address::new("Claire Martin", "Claire@Atelier-Exemple.example"),
            to: vec![Address::new("", "ada@example.org")],
            ..Default::default()
        })
        .unwrap();
    let written = mb
        .add_message(Message {
            folder: sent,
            from: Address::new("Ada", "ada@example.org"),
            to: vec![Address::new("Claire", CLIENT)],
            ..Default::default()
        })
        .unwrap();
    let alone = mb
        .add_message(Message {
            folder: sent,
            from: Address::new("Ada", "ADA@example.org"),
            to: vec![Address::new("", "ada@example.org")],
            ..Default::default()
        })
        .unwrap();
    let app = crate::MailApp::new(Session::at(mb, 0, 0), Services::default());
    let target = |id| app.session.mailbox.message(id).and_then(|m| target_of(&app, m));
    let t = target(received).unwrap();
    assert_eq!((t.address.as_str(), t.name.as_str(), t.mailbox.as_str(), t.message), (CLIENT, "Claire Martin", "ada@example.org", received));
    let t = target(written).unwrap();
    assert_eq!((t.address.as_str(), t.name.as_str()), (CLIENT, "Claire"), "what the owner wrote is about who it went to");
    assert!(target(alone).is_none(), "a message to oneself is about nobody");
}
