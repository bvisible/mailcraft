// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of exchanges.rs.
//! What the Échanges pane asks of the Neoffice server, as a call of its methods. Every address and id is invented.

use mailcraft_ui_egui::exchanges::Want;
use serde_json::json;

use super::*;

const ME: &str = "ada@example.org";
const CLIENT: &str = "claire@atelier-exemple.example";

#[test]
fn who_someone_is_and_what_neoffice_links_them_to_are_asked_by_their_address() {
    let who = Want::Someone { mailbox: ME.into(), address: CLIENT.into() };
    assert_eq!(call_of("A1", &who), Call { method: "NeomailContact/get", arguments: json!({"accountId": "A1", "address": CLIENT}) });
    let linked = Want::Linked { mailbox: ME.into(), address: CLIENT.into() };
    assert_eq!(call_of("A1", &linked), Call { method: "NeomailLinked/get", arguments: json!({"accountId": "A1", "address": CLIENT}) });
}

#[test]
fn a_page_of_messages_is_asked_by_the_addresses_a_page_long_and_after_a_moment() {
    let first = Want::Page { mailbox: ME.into(), key: "k".into(), addresses: vec![CLIENT.into()], before: None, cc: false, domain: false };
    let call = call_of("A1", &first);
    assert_eq!(call.method, "NeomailHistory/get");
    assert_eq!(call.arguments, json!({"accountId": "A1", "addresses": [CLIENT], "limit": 20}));
    let next = Want::Page {
        mailbox: ME.into(),
        key: "k".into(),
        addresses: vec![CLIENT.into(), "marc@atelier-exemple.example".into()],
        before: Some("2026-10-29T10:00:00Z".into()),
        cc: true,
        domain: true,
    };
    let arguments = call_of("A1", &next).arguments;
    assert_eq!(arguments["before"], "2026-10-29T10:00:00Z");
    assert_eq!((arguments["includeCc"].as_bool(), arguments["domains"].as_bool()), (Some(true), Some(true)));
    assert_eq!(arguments["addresses"].as_array().map(Vec::len), Some(2));
}

#[test]
fn a_want_is_for_the_mailbox_it_names() {
    assert_eq!(mailbox_of(&Want::Someone { mailbox: ME.into(), address: CLIENT.into() }), ME);
    assert_eq!(mailbox_of(&Want::Linked { mailbox: "x@example.org".into(), address: CLIENT.into() }), "x@example.org");
    assert_eq!(mailbox_of(&Want::Page { mailbox: ME.into(), key: String::new(), addresses: Vec::new(), before: None, cc: false, domain: false }), ME);
}
