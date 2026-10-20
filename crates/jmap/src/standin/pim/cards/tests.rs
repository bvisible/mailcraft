// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! `ContactCard/set` of the stand-in, held to what a server that keeps only the basics of a contact takes: called as a
//! client calls it, and answered as such a server answers. Invented people at `example.org`.

use serde_json::{Value, json};

use super::super::{Book, Card, Pim};
use crate::standin::ACCOUNT;

/// The team's book (written and deleted), the partners' (read only), Grace in the first and Carol in the second.
fn pim() -> Pim {
    let mut pim = Pim::offered();
    pim.add_book(Book { default: true, ..Book::new("neoffice", "Neoffice", true) });
    pim.add_book(Book::new("partners", "Partners", false));
    pim.add_card(Card::person("Grace Okafor", "neoffice", "Grace", "Okafor", "grace@example.org").with_phone("+41 79 555 01 01", true));
    pim.add_card(Card::person("Carol Duval", "partners", "Carol", "Duval", "carol@example.org"));
    pim
}

fn set(pim: &mut Pim, args: Value) -> Value {
    let mut args = args;
    args["accountId"] = json!(ACCOUNT);
    pim.card_set(args.as_object().unwrap(), 500).unwrap_or_else(|e| panic!("{} {}", e.kind, e.description))
}

fn error_of(pim: &mut Pim, args: Value) -> String {
    let mut args = args;
    args["accountId"] = json!(ACCOUNT);
    match pim.card_set(args.as_object().unwrap(), 500) {
        Err(e) => format!("{} {}", e.kind, e.description),
        Ok(answer) => panic!("expected a method error, got {answer}"),
    }
}

fn card<'a>(pim: &'a Pim, id: &str) -> &'a Card {
    pim.cards.iter().find(|card| card.id == id).unwrap_or_else(|| panic!("no card {id}"))
}

#[test]
fn a_card_is_made_named_by_its_contact_and_known_again_by_its_uid() {
    let mut pim = pim();
    let gus = json!({
        "@type": "Card", "version": "1.0", "uid": "t-1", "kind": "individual", "addressBookIds": {"neoffice": true},
        "name": {"full": "Gus Hale", "components": [{"kind": "given", "value": "Gus"}, {"kind": "surname", "value": "Hale"}]},
        "emails": {"e1": {"address": "gus@example.org"}, "e2": {"address": "GUS@example.org"}, "e3": {"address": "not one"}},
        "phones": {"p1": {"number": "+41 79 555 02 02", "features": {"mobile": true}}, "p2": {"number": "+41 79 555 02 03", "features": {"mobile": true}}},
        "organizations": {"o1": {"name": ""}, "o2": {"name": "Hale & Sons"}},
        "titles": {"t1": {"name": "Joiner"}},
    });
    let answer = set(&mut pim, json!({"create": {"k0": gus.clone()}}));
    assert_eq!(answer["created"]["k0"], json!({"id": "Gus Hale", "uid": "Gus Hale"}));
    let made = card(&pim, "Gus Hale");
    assert_eq!(made.emails, ["gus@example.org"], "each address once, and only addresses");
    assert_eq!(made.phones, [("+41 79 555 02 02".to_string(), true), ("+41 79 555 02 03".to_string(), false)], "one primary mobile");
    assert_eq!((made.company.as_str(), made.title.as_str(), made.books.as_slice()), ("Hale & Sons", "Joiner", ["neoffice".to_string()].as_slice()));
    // Sent again with its uid (its answer was lost): the same card.
    assert_eq!(set(&mut pim, json!({"create": {"k0": gus}}))["created"]["k0"]["id"], "Gus Hale");
    assert_eq!(pim.cards.iter().filter(|card| card.first == "Gus").count(), 1);
    // Another Gus Hale, made anew: a name of its own.
    let again = json!({"name": {"components": [{"kind": "given", "value": "Gus"}, {"kind": "surname", "value": "Hale"}]}});
    assert_eq!(set(&mut pim, json!({"create": {"k1": again}}))["created"]["k1"]["id"], "Gus Hale-1");
    // A full name alone is cut at its first space; an address alone names it.
    let full = set(
        &mut pim,
        json!({"create": {"a": {"name": {"full": "Dee Van Dyke"}}, "b": {"emails": {"e1": {"address": "jeanne.muster@example.org"}}}}}),
    );
    assert_eq!((full["created"]["a"]["id"].as_str(), full["created"]["b"]["id"].as_str()), (Some("Dee Van Dyke"), Some("Jeanne Muster")));
    assert_eq!((card(&pim, "Dee Van Dyke").first.as_str(), card(&pim, "Dee Van Dyke").last.as_str()), ("Dee", "Van Dyke"));
    // A company alone: named by its company. Nothing at all: refused, with its words.
    let company =
        set(&mut pim, json!({"create": {"c": {"organizations": {"o1": {"name": "Muster AG"}}}, "d": {"titles": {"t1": {"name": "Joiner"}}}}}));
    assert_eq!(company["created"]["c"]["id"], "Muster AG");
    assert_eq!((card(&pim, "Muster AG").full_name(), card(&pim, "Muster AG").company.as_str()), ("Muster AG".to_string(), "Muster AG"));
    assert_eq!(company["notCreated"]["d"], json!({"type": "invalidProperties", "description": "A contact needs a name, an address or a company."}));
}

#[test]
fn only_whole_properties_of_what_it_writes_are_taken() {
    let mut pim = pim();
    let refused = |answer: &Value, key: &str, id: &str| answer[key][id]["type"].as_str().map(str::to_string);
    let answer = set(
        &mut pim,
        json!({
            "create": {"notes": {"name": {"full": "Hal"}, "notes": {"n1": {"note": "x"}}}, "book": {"name": {"full": "Hal"}, "addressBookIds": {"nowhere": true}}, "two": {"name": {"full": "Hal"}, "addressBookIds": {"neoffice": true, "partners": true}}},
            "update": {"Grace Okafor": {"emails/e1": {"address": "x@example.org"}}},
        }),
    );
    for key in ["notes", "book", "two"] {
        assert_eq!(refused(&answer, "notCreated", key).as_deref(), Some("invalidProperties"), "{key}");
    }
    assert_eq!(answer["notUpdated"]["Grace Okafor"]["description"], "A whole property replaces it: no patch of a part.");
    let shapes = set(
        &mut pim,
        json!({"update": {"Grace Okafor": {"name": "Grace"}}, "create": {"a": {"emails": ["x@example.org"]}, "b": {"name": {"components": "Hal"}}}}),
    );
    assert_eq!(shapes["notUpdated"]["Grace Okafor"]["description"], "name must be an object.");
    assert_eq!(shapes["notCreated"]["a"]["description"], "emails must be an object.");
    assert_eq!(shapes["notCreated"]["b"]["description"], "name.components must be a list.");
    assert_eq!(card(&pim, "Grace Okafor").emails, ["grace@example.org"], "nothing of a refused change is kept");
    // A call that is not one.
    assert!(error_of(&mut pim, json!({"create": [1]})).contains("invalidArguments"));
    assert!(error_of(&mut pim, json!({"ifInState": "0"})).contains("invalidArguments"));
    assert!(error_of(&mut pim, json!({"destroy": "Grace Okafor"})).contains("invalidArguments"));
    assert!(error_of(&mut pim, json!({"onSuccessUpdate": {}})).contains("invalidArguments"));
    let many: Vec<String> = (0..501).map(|n| format!("x{n}")).collect();
    assert!(error_of(&mut pim, json!({"destroy": many})).contains("requestTooLarge"));
}

#[test]
fn a_change_replaces_what_it_names_as_such_a_server_does() {
    let mut pim = pim();
    // The primary address stays the primary while it is there; a number it had keeps what it was.
    let answer = set(
        &mut pim,
        json!({"update": {"Grace Okafor": {
            "emails": {"e1": {"address": "grace.okafor@example.org"}, "e2": {"address": "grace@example.org"}},
            "phones": {"p1": {"number": "+41 44 555 01 00", "features": {"voice": true}}, "p2": {"number": "+41 79 555 01 01", "features": {"voice": true}}},
            "titles": {},
        }}}),
    );
    assert_eq!(answer["updated"], json!({"Grace Okafor": null}));
    let grace = card(&pim, "Grace Okafor");
    assert_eq!(grace.emails, ["grace@example.org", "grace.okafor@example.org"]);
    assert_eq!(grace.phones, [("+41 44 555 01 00".to_string(), false), ("+41 79 555 01 01".to_string(), true)]);
    // A name emptied is not written; nor the full name as the contact has it; parts are.
    set(&mut pim, json!({"update": {"Grace Okafor": {"name": {"full": "", "components": []}}}}));
    set(&mut pim, json!({"update": {"Grace Okafor": {"name": {"full": "Grace Okafor"}}}}));
    assert_eq!((card(&pim, "Grace Okafor").first.as_str(), card(&pim, "Grace Okafor").last.as_str()), ("Grace", "Okafor"));
    set(
        &mut pim,
        json!({"update": {"Grace Okafor": {"name": {"full": "x", "components": [{"kind": "given", "value": "Grace"}, {"kind": "surname", "value": "Okafor-Brown"}]}}}}),
    );
    assert_eq!(card(&pim, "Grace Okafor").last, "Okafor-Brown");
    assert_eq!(card(&pim, "Grace Okafor").id, "Grace Okafor", "a change of name keeps the card's id");
    // What it was sent, in order.
    assert_eq!(pim.card_writes.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["Grace Okafor"; 4]);
}

#[test]
fn the_books_rights_and_the_documents_that_name_a_card_are_held_to() {
    let mut pim = pim();
    pim.linked_cards.insert("Grace Okafor".into(), "Issue ISS-2026-00012".into());
    let answer = set(
        &mut pim,
        json!({
            "create": {"read-only": {"name": {"full": "Hal"}, "addressBookIds": {"partners": true}}},
            "update": {"Carol Duval": {"titles": {"t1": {"name": "Partner"}}}, "Nobody": {"titles": {}}},
            "destroy": ["Carol Duval", "Grace Okafor", "Nobody", 7],
        }),
    );
    let forbidden = json!({"type": "forbidden", "description": "Your rights do not allow it."});
    assert_eq!((answer["notCreated"]["read-only"].clone(), answer["notUpdated"]["Carol Duval"].clone()), (forbidden.clone(), forbidden.clone()));
    assert_eq!(answer["notDestroyed"]["Carol Duval"], forbidden);
    assert_eq!(answer["notUpdated"]["Nobody"], json!({"type": "notFound"}));
    assert_eq!(answer["notDestroyed"]["Nobody"], json!({"type": "notFound"}));
    assert_eq!(
        answer["notDestroyed"]["Grace Okafor"]["description"],
        "Cannot delete or cancel because Contact Grace Okafor is linked with Issue ISS-2026-00012"
    );
    assert_eq!(answer["destroyed"], json!([]));
    // One that may be changed and not deleted.
    pim.linked_cards.clear();
    if let Some(book) = pim.books.iter_mut().find(|book| book.id == "neoffice") {
        book.may_remove = false;
    }
    let answer = set(&mut pim, json!({"update": {"Grace Okafor": {"titles": {"t1": {"name": "Engineer"}}}}, "destroy": ["Grace Okafor"]}));
    assert_eq!(
        (answer["updated"]["Grace Okafor"].clone(), answer["notDestroyed"]["Grace Okafor"]["type"].clone()),
        (Value::Null, json!("forbidden"))
    );
    if let Some(book) = pim.books.iter_mut().find(|book| book.id == "neoffice") {
        book.may_remove = true;
    }
    let state = pim.cards_state();
    assert_eq!(set(&mut pim, json!({"destroy": ["Grace Okafor"]}))["destroyed"], json!(["Grace Okafor"]));
    assert!(pim.cards.iter().all(|card| card.id != "Grace Okafor") && pim.cards_state() != state);
}

#[test]
fn hostile_calls_never_panic() {
    let junk = [
        json!(null),
        json!({"create": {"a": null, "b": 5, "c": {"name": {"components": [null, 5, {"kind": 5, "value": {"x": 1}}]}}}}),
        json!({"create": {"a": {"name": {"full": "x".repeat(10_000)}, "emails": {"e": {"address": 5}}, "phones": {"p": {"number": [1]}}}}}),
        json!({"update": {"": {}, "Grace Okafor": null, "Carol Duval": [1]}, "destroy": [null, {"x": 1}, ""]}),
        json!({"create": {"a": {"name": {"full": "\u{0}\u{202e} \r\n"}, "uid": "\u{0}".repeat(1_000)}}}),
    ];
    for args in junk {
        let mut pim = pim();
        let mut call = args.as_object().cloned().unwrap_or_default();
        call.insert("accountId".into(), json!(ACCOUNT));
        let _ = pim.card_set(&call, 500);
    }
}

#[test]
fn a_card_sent_again_has_what_was_sent_this_time() {
    let mut pim = pim();
    let first = json!({"uid": "t-1", "name": {"components": [{"kind": "given", "value": "Gus"}, {"kind": "surname", "value": "Hale"}]}});
    set(&mut pim, json!({"create": {"k0": first}}));
    let made = card(&pim, "Gus Hale").clone();
    // Its answer was lost; meanwhile a number was added: the card made the first time has it.
    let again = json!({
        "uid": "t-1",
        "name": {"components": [{"kind": "given", "value": "Gus"}, {"kind": "surname", "value": "Hale"}]},
        "phones": {"p1": {"number": "+41 79 555 02 02", "features": {"mobile": true}}},
    });
    assert_eq!(set(&mut pim, json!({"create": {"k0": again}}))["created"]["k0"]["id"], "Gus Hale");
    let gus = card(&pim, "Gus Hale");
    assert_eq!(gus.phones, [("+41 79 555 02 02".to_string(), true)]);
    assert!(gus.created == made.created && gus.updated > made.updated, "the same card, changed");
    assert_eq!(pim.cards.iter().filter(|card| card.first == "Gus").count(), 1);
}

#[test]
fn the_preferred_address_is_the_primary_and_a_long_text_is_refused() {
    let mut pim = pim();
    let answer = set(
        &mut pim,
        json!({"update": {"Grace Okafor": {"emails": {"e1": {"address": "grace.okafor@example.org", "pref": 1}, "e2": {"address": "grace@example.org"}}}}}),
    );
    assert_eq!(answer["updated"], json!({"Grace Okafor": null}));
    assert_eq!(card(&pim, "Grace Okafor").emails, ["grace.okafor@example.org", "grace@example.org"]);
    // Longer than 140 characters: refused, and nothing of it written.
    let long = "x".repeat(141);
    for patch in [
        json!({"titles": {"t1": {"name": long}}}),
        json!({"name": {"components": [{"kind": "given", "value": long}]}}),
        json!({"emails": {"e1": {"address": format!("{long}@example.org")}}}),
    ] {
        let answer = set(&mut pim, json!({"update": {"Grace Okafor": patch}}));
        assert_eq!(
            answer["notUpdated"]["Grace Okafor"],
            json!({"type": "invalidProperties", "description": "A text is longer than 140 characters."})
        );
    }
    assert_eq!(card(&pim, "Grace Okafor").title, "");
    let fits = "y".repeat(140);
    assert_eq!(set(&mut pim, json!({"update": {"Grace Okafor": {"titles": {"t1": {"name": fits}}}}}))["updated"], json!({"Grace Okafor": null}));
}

#[test]
fn every_card_says_when_it_was_made_and_last_changed() {
    let mut pim = pim();
    let served = |pim: &Pim, id: &str| super::super::served_card(card(pim, id));
    let grace = served(&pim, "Grace Okafor");
    let (made, changed) = (grace["created"].clone(), grace["updated"].clone());
    assert!(made.as_str().is_some_and(|t| t.ends_with('Z')) && made == changed, "{grace}");
    set(&mut pim, json!({"update": {"Grace Okafor": {"titles": {"t1": {"name": "Engineer"}}}}}));
    let grace = served(&pim, "Grace Okafor");
    assert!(grace["created"] == made && grace["updated"] != changed);
    // A card made under the name of one that was deleted is told apart from it.
    set(&mut pim, json!({"destroy": ["Grace Okafor"]}));
    set(&mut pim, json!({"create": {"k": {"name": {"components": [{"kind": "given", "value": "Grace"}, {"kind": "surname", "value": "Okafor"}]}}}}));
    assert_ne!(served(&pim, "Grace Okafor")["created"], made);
}
