// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A card and an address book as JMAP for Contacts serves them, into the engine's: names, addresses, numbers, the
//! company and the title, the book a card is in, and what is passed over. Invented people at `example.org`.

use std::collections::HashSet;

use mailcraft_model::Labeled;
use proptest::prelude::*;
use serde_json::{Value, json};

use super::*;
use crate::pim::{card_key, server_card_id};

/// A card as a server that keeps only the basics serves one (Neoffice's facade: the contact's name is its id).
fn card() -> Value {
    json!({
        "@type": "Card",
        "version": "1.0",
        "id": "Grace Okafor",
        "uid": "Grace Okafor",
        "addressBookIds": {"neoffice": true},
        "name": {"@type": "Name", "full": "Grace Okafor", "components": [
            {"@type": "NameComponent", "kind": "given", "value": "Grace"},
            {"@type": "NameComponent", "kind": "surname", "value": "Okafor"},
        ]},
        "emails": {
            "e1": {"@type": "EmailAddress", "address": "grace@example.org", "pref": 1},
            "e2": {"@type": "EmailAddress", "address": "grace.okafor@example.org"},
        },
        "phones": {
            "p1": {"@type": "Phone", "number": "+41 44 555 01 01", "features": {"voice": true}},
            "p2": {"@type": "Phone", "number": "+41 79 555 01 02", "features": {"mobile": true}},
        },
        "organizations": {"o1": {"@type": "Organization", "name": "Okafor Engineering"}},
        "titles": {"t1": {"@type": "Title", "name": "Structural engineer", "kind": "title"}},
    })
}

fn none() -> HashSet<String> {
    HashSet::new()
}

fn read(value: &Value) -> RemoteContact {
    card_of(value, &none()).unwrap_or_else(|missing| panic!("passed over for {missing:?}: {value}"))
}

#[test]
fn a_card_reads_names_emails_phones_company_and_title() {
    let grace = read(&card());
    assert_eq!(grace.id, card_key("Grace Okafor"), "a name with a space is kept under a key the engine can keep");
    assert_eq!(server_card_id(&grace.id).as_deref(), Some("Grace Okafor"), "and the key gives the server's name back");
    assert_eq!((grace.first.as_str(), grace.last.as_str(), grace.display_name.as_str()), ("Grace", "Okafor", ""));
    assert_eq!(grace.emails, [Labeled::new("", "grace@example.org"), Labeled::new("", "grace.okafor@example.org")]);
    assert_eq!(grace.phones, [Labeled::new("", "+41 44 555 01 01"), Labeled::new("mobile", "+41 79 555 01 02")]);
    assert_eq!((grace.company.as_str(), grace.title.as_str(), grace.book.as_str()), ("Okafor Engineering", "Structural engineer", "neoffice"));
    // What such a server doesn't keep is not read, and its id is an id of the alphabet when it is one.
    assert!(grace.middle.is_empty() && grace.notes.is_empty() && grace.addresses.is_empty() && grace.birthday.is_none());
    let mut plain = card();
    plain["id"] = json!("c-17");
    assert_eq!(read(&plain).id, "c-17");
}

#[test]
fn a_company_contact_reads_as_its_company_and_a_name_that_is_first_and_last_has_no_display_name() {
    let company = json!({
        "id": "Lindqvist Joinery",
        "addressBookIds": {"neoffice": true},
        "name": {"full": "Lindqvist Joinery", "components": []},
        "emails": {"e1": {"address": "joinery@example.org", "pref": 1}},
        "organizations": {"o1": {"name": "Lindqvist Joinery"}},
    });
    let read_company = read(&company);
    assert_eq!(
        (read_company.display_name.as_str(), read_company.first.as_str(), read_company.company.as_str()),
        ("Lindqvist Joinery", "", "Lindqvist Joinery")
    );
    // A middle name the components don't carry: the full name is what is shown.
    let mut middle = card();
    middle["name"]["full"] = json!("Grace Adaeze Okafor");
    assert_eq!(read(&middle).display_name, "Grace Adaeze Okafor");
    // Only a first name, or only a full name.
    let first_only = json!({"id": "Bo", "name": {"full": "Bo", "components": [{"kind": "given", "value": "Bo"}]}});
    assert_eq!((read(&first_only).first.as_str(), read(&first_only).display_name.as_str()), ("Bo", ""));
    let full_only = json!({"id": "Cy", "name": {"full": "Cy Duval"}});
    assert_eq!((read(&full_only).first.as_str(), read(&full_only).display_name.as_str()), ("", "Cy Duval"));
}

#[test]
fn the_primary_email_comes_first_and_the_primary_mobile_keeps_its_kind() {
    let mut out_of_order = card();
    out_of_order["emails"] = json!({
        "e10": {"address": "ten@example.org"},
        "e2": {"address": "two@example.org", "contexts": {"private": true}},
        "e3": {"address": "primary@example.org", "pref": 1, "contexts": {"work": true}},
        "e1": {"address": "one@example.org", "pref": 50},
    });
    let emails: Vec<(String, String)> = read(&out_of_order).emails.into_iter().map(|e| (e.value, e.label)).collect();
    let expected = [("primary@example.org", "work"), ("one@example.org", ""), ("two@example.org", "home"), ("ten@example.org", "")];
    assert_eq!(emails, expected.map(|(value, label)| (value.to_string(), label.to_string())));
    let mut phones = card();
    phones["phones"] = json!({
        "p1": {"number": "+41 79 555 01 02", "features": {"mobile": true}, "contexts": {"work": true}},
        "p2": {"number": "+41 44 555 01 01", "contexts": {"private": true}},
        "p3": {"number": "+41 44 555 01 03", "features": {"voice": true}},
    });
    let labels: Vec<String> = read(&phones).phones.into_iter().map(|p| p.label).collect();
    assert_eq!(labels, ["mobile", "home", ""]);
}

#[test]
fn a_card_with_no_name_and_no_address_is_passed_over() {
    let nothing = json!({"id": "x1", "name": {"full": " ", "components": []}, "emails": {"e1": {"address": "not an address"}}, "phones": {"p1": {"number": "+41 44 555 01 01"}}});
    assert_eq!(card_of(&nothing, &none()), Err(Missing::Person));
    let group = json!({"id": "x2", "kind": "group", "name": {"full": "The board"}, "members": {"Grace Okafor": true}});
    assert_eq!(card_of(&group, &none()), Err(Missing::Person));
    for id in [json!(null), json!(""), json!(5), json!(" padded "), json!("line\nbreak"), json!("x".repeat(701))] {
        let mut no_id = card();
        no_id["id"] = id.clone();
        assert_eq!(card_of(&no_id, &none()), Err(Missing::Id), "{id}");
    }
    // An address alone is enough: it is what the lists show.
    let address_only = json!({"id": "x3", "emails": {"e1": {"address": "someone@example.org"}}});
    assert_eq!(read(&address_only).emails.len(), 1);
}

#[test]
fn a_card_in_two_books_goes_to_the_first_the_person_may_write_else_the_first() {
    let mut two = card();
    two["addressBookIds"] = json!({"partners": true, "neoffice": true, "old": false, "bad id": true});
    assert_eq!(read(&two).book, "neoffice", "the first by its id, of those it is in");
    let writable: HashSet<String> = ["partners".to_string()].into();
    assert_eq!(card_of(&two, &writable).unwrap().book, "partners", "the first the account may write");
    let mut nowhere = card();
    nowhere["addressBookIds"] = json!({});
    assert_eq!(read(&nowhere).book, "");
}

#[test]
fn a_book_reads_with_its_rights() {
    let listed = |rights: Value| book_of(&json!({"id": "neoffice", "name": "Neoffice", "isDefault": true, "myRights": rights})).unwrap();
    let full = listed(json!({"mayRead": true, "mayWrite": true, "mayRemoveItems": true}));
    assert_eq!(
        (full.book.id.as_str(), full.book.name.as_str(), full.book.main, full.writable, full.removes),
        ("neoffice", "Neoffice", true, true, true)
    );
    assert!(listed(json!({"mayWrite": true})).removes, "a server that doesn't narrow it lets the cards be deleted");
    let no_deleting = listed(json!({"mayWrite": true, "mayRemoveItems": false}));
    assert!(no_deleting.writable && !no_deleting.removes);
    let read_only = listed(json!({"mayWrite": false, "mayRemoveItems": true}));
    assert!(!read_only.writable && !read_only.removes);
    assert!(!listed(json!("junk")).writable);
    assert!(book_of(&json!({"id": "two words"})).is_none() && book_of(&json!({"name": "No id"})).is_none());
}

#[test]
fn the_server_names_of_cards_and_their_keys() {
    for name in ["Grace Okafor", "Hélène Dupré-Acme SA", "O'Neil & Co / Zurich", "x", "~tilde first"] {
        let key = card_key(name);
        assert!(!key.chars().any(|c| c.is_whitespace() || c.is_control()), "{key}");
        assert_eq!(server_card_id(&key).as_deref(), Some(name));
    }
    assert_eq!(card_key("c-17"), "c-17");
    assert_eq!(server_card_id("c-17").as_deref(), Some("c-17"));
    // A key nobody made names no card.
    for junk in ["", "~", "~!!", "~AA", "two words", "~R3JhY2U"] {
        assert!(server_card_id(junk).is_none_or(|id| card_key(&id) == junk), "{junk}");
    }
    assert!(server_card_id("").is_none() && server_card_id("~!!").is_none());
}

proptest! {
    #[test]
    fn hostile_cards_never_panic(text in any::<String>(), n in any::<i64>(), flag in any::<bool>()) {
        let hostile = json!({
            "id": text, "kind": text, "addressBookIds": {text.clone(): flag, "neoffice": true},
            "name": {"full": text, "components": [{"kind": "given", "value": text}, {"kind": "surname", "value": n}, 5, null]},
            "emails": {text.clone(): {"address": text, "pref": n, "contexts": {text.clone(): true}}, "e1": "x"},
            "phones": {"p1": {"number": text, "features": text}},
            "organizations": [text], "titles": {"t1": {"name": n}},
        });
        let writable: HashSet<String> = [text.clone()].into();
        if let Ok(mut contact) = card_of(&hostile, &writable) {
            prop_assert!(server_card_id(&contact.id).is_some());
            contact.tidy();
            prop_assert!(!contact.id.is_empty());
        }
        let _ = book_of(&json!({"id": text, "name": n, "myRights": {"mayWrite": text, "mayRemoveItems": flag}}));
        let many: serde_json::Map<String, Value> = (0..1_000).map(|i| (format!("e{i}"), json!({"address": format!("a{i}@example.org"), "pref": i}))).collect();
        let crowded = card_of(&json!({"id": "c1", "name": {"full": text}, "emails": many}), &writable);
        prop_assert!(crowded.is_ok_and(|card| card.emails.len() <= 100));
    }
}
