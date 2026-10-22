// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A job's writing of the contacts made, changed and deleted here, against the in-process server held to what a server
//! that keeps only the basics of a contact takes: what is sent, what comes of it, and what trouble comes to. Invented
//! people at `example.org`.

use std::sync::atomic::AtomicBool;

use mailcraft_model::Labeled;
use mailcraft_model::book::{ContactChangeKind, ContactFields, ContactName, ContactOp, ContactOutcome, ContactsUpload};
use mailcraft_model::remote::EventResult;
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use serde_json::{Value, json};

use super::super::super::jscontact::card_json;
use super::super::super::{card_key, server_card_id, sync_pim};
use super::{MAX_PER_CALL, MAX_WORDS};
use crate::standin::{Book, Card, Effect, Fault, Pim, SESSION, StandIn, TOKEN};
use crate::{Auth, Client, block_on};

const ME: &str = "ada@example.org";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// A server with the team's book (Ada may write and delete its cards; the default) and the partners' (read only), and
/// a few people in them, each card called by its contact's name.
fn server() -> StandIn {
    let server = StandIn::new();
    server.with(|d| {
        let pim = &mut d.pim;
        *pim = Pim::offered();
        pim.add_book(Book { default: true, ..Book::new("neoffice", "Neoffice", true) });
        pim.add_book(Book::new("partners", "Partners", false));
        pim.add_card(Card::person("Grace Okafor", "neoffice", "Grace", "Okafor", "grace@example.org").working("Engineer", "Okafor Engineering"));
        pim.add_card(
            Card::person("Bob Lindqvist", "neoffice", "Bob", "Lindqvist", "bob@example.org")
                .with_email("bob@lindqvist.example.org")
                .with_phone("+41 79 555 01 02", true)
                .with_phone("+41 44 555 01 03", false),
        );
        pim.add_card(Card::person("Carol Duval", "partners", "Carol", "Duval", "carol@example.org"));
    });
    server
}

/// A job that writes `ops` and reads nothing else (no calendar, no contacts).
fn request(ops: Vec<ContactOp>) -> JmapPimRequest {
    JmapPimRequest {
        account: 7,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        me: vec![ME.into()],
        now: NOW,
        contact_changes: Some(ContactsUpload { account: 7, email: ME.into(), ops }),
        ..Default::default()
    }
}

fn run(server: &StandIn, request: &JmapPimRequest) -> JmapPimResult {
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    block_on(sync_pim(&client, request, &AtomicBool::new(false)))
}

fn outcomes(result: &JmapPimResult) -> Vec<ContactOutcome> {
    result.contacts_uploaded.as_ref().map(|uploaded| uploaded.outcomes.clone()).unwrap_or_default()
}

fn name(first: &str, last: &str) -> Option<ContactName> {
    Some(ContactName { first: first.into(), last: last.into(), display: String::new() })
}

/// A new contact, made with the mark `token`, in the team's book.
fn create(token: &str, fields: ContactFields) -> ContactOp {
    ContactOp { kind: ContactChangeKind::Create, contact: 40, token: token.into(), book: "neoffice".into(), fields, ..Default::default() }
}

/// A change of the card the server calls `id`.
fn update(id: &str, fields: ContactFields) -> ContactOp {
    ContactOp { kind: ContactChangeKind::Update, contact: 41, remote: card_key(id), book: "neoffice".into(), fields, ..Default::default() }
}

fn destroy(id: &str) -> ContactOp {
    ContactOp { kind: ContactChangeKind::Delete, contact: 42, remote: card_key(id), book: "neoffice".into(), ..Default::default() }
}

/// The card the server has under `id`.
fn card(server: &StandIn, id: &str) -> Option<Card> {
    server.with(|d| d.pim.cards.iter().find(|card| card.id == id).cloned())
}

/// What the server was sent of cards, in order.
fn writes(server: &StandIn) -> Vec<(String, Value)> {
    server.with(|d| d.pim.card_writes.clone())
}

fn gus() -> ContactFields {
    ContactFields {
        name: name("Gus", "Hale"),
        company: Some("Hale & Sons".into()),
        title: Some("Joiner".into()),
        emails: Some(vec![Labeled::new("work", "gus@example.org"), Labeled::new("home", "gus.hale@example.org")]),
        phones: Some(vec![Labeled::new("mobile", "+41 79 555 02 02"), Labeled::new("work", "+41 44 555 02 03")]),
    }
}

#[test]
fn a_new_card_is_written_whole_with_its_uid_and_book() {
    let server = server();
    let result = run(&server, &request(vec![create("4f1c2e8a-0b7d-4e2a-a1b3-9c8d7e6f5a40", gus())]));
    let done = outcomes(&result);
    assert_eq!((done.len(), done[0].result), (1, EventResult::Done), "{done:?}");
    assert_eq!(server_card_id(&done[0].remote).as_deref(), Some("Gus Hale"), "the server's name for it, as the engine keeps it");
    let made = card(&server, "Gus Hale").unwrap();
    assert_eq!((made.books.as_slice(), made.first.as_str(), made.last.as_str()), (["neoffice".to_string()].as_slice(), "Gus", "Hale"));
    assert_eq!((made.company.as_str(), made.title.as_str()), ("Hale & Sons", "Joiner"));
    assert_eq!(made.emails, ["gus@example.org", "gus.hale@example.org"]);
    // What went: the card whole, with its mark, in its book.
    let (_, sent) = writes(&server).remove(0);
    assert_eq!(sent["uid"], "4f1c2e8a-0b7d-4e2a-a1b3-9c8d7e6f5a40");
    assert_eq!(sent["addressBookIds"], json!({"neoffice": true}));
    assert_eq!((sent["@type"].as_str(), sent["version"].as_str()), (Some("Card"), Some("1.0")));
    assert_eq!(sent["name"]["full"], "Gus Hale");
    // Nothing is read that wasn't asked for.
    assert!(result.contacts.is_none() && result.errors.is_empty(), "{result:?}");
    // A card that says nothing of a name goes without one, and its server names it from its address, else by its
    // company.
    let result = run(
        &server,
        &request(vec![
            create(
                "t-2",
                ContactFields { emails: Some(vec![Labeled::new("", "jeanne.muster@example.org")]), name: name("", ""), ..Default::default() },
            ),
            create("t-3", ContactFields { company: Some("Muster AG".into()), ..Default::default() }),
        ]),
    );
    let done = outcomes(&result);
    assert_eq!(server_card_id(&done[0].remote).as_deref(), Some("Jeanne Muster"));
    assert_eq!((done[1].result, server_card_id(&done[1].remote).as_deref()), (EventResult::Done, Some("Muster AG")));
    // A text longer than such a server takes is refused with its words, not cut.
    let long = create("t-4", ContactFields { name: name("Gus", &"Hale".repeat(40)), ..Default::default() });
    let done = outcomes(&run(&server, &request(vec![long])));
    assert_eq!((done[0].result, done[0].error.as_str()), (EventResult::Refused, "A text is longer than 140 characters."));
    assert!(writes(&server).iter().all(|(_, sent)| sent.get("name").is_none_or(|name| name["full"] != "")), "no empty name is sent");
}

#[test]
fn a_change_names_whole_properties_and_the_name_with_its_components() {
    let server = server();
    let fields = ContactFields { name: name("Robert", "Lindqvist"), title: Some("Master joiner".into()), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![update("Bob Lindqvist", fields)])));
    assert_eq!((done[0].result, done[0].remote.as_str()), (EventResult::Done, card_key("Bob Lindqvist").as_str()));
    let (id, sent) = writes(&server).remove(0);
    assert_eq!(id, "Bob Lindqvist");
    let keys: Vec<&String> = sent.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["name", "titles"], "only what changed, each whole");
    assert_eq!(
        sent["name"],
        json!({"@type": "Name", "full": "Robert Lindqvist", "components": [{"@type": "NameComponent", "kind": "given", "value": "Robert"}, {"@type": "NameComponent", "kind": "surname", "value": "Lindqvist"}]})
    );
    let bob = card(&server, "Bob Lindqvist").unwrap();
    assert_eq!((bob.first.as_str(), bob.title.as_str()), ("Robert", "Master joiner"));
    assert_eq!(bob.emails.len(), 2, "what wasn't named stays as the server has it");
    // A display name is the full form a server reads when it keeps one; a server that keeps only the basics reads the
    // parts.
    let display = ContactOp {
        fields: ContactFields { name: Some(ContactName { display: "Bobby L.".into(), ..name("Bob", "Lindqvist").unwrap() }), ..Default::default() },
        ..update("x", ContactFields::default())
    };
    assert_eq!(card_json(&display)["name"]["full"], "Bobby L.");
}

#[test]
fn the_company_and_the_title_are_the_first_organization_and_title() {
    let server = server();
    let fields = ContactFields { company: Some("Okafor & Partners".into()), title: Some(String::new()), ..Default::default() };
    run(&server, &request(vec![update("Grace Okafor", fields)]));
    let (_, sent) = writes(&server).remove(0);
    assert_eq!(sent["organizations"], json!({"o1": {"@type": "Organization", "name": "Okafor & Partners"}}));
    assert_eq!(sent["titles"], json!({}), "an empty title is none");
    let grace = card(&server, "Grace Okafor").unwrap();
    assert_eq!((grace.company.as_str(), grace.title.as_str()), ("Okafor & Partners", ""));
    // A new card with neither says so, as it says of everything it names (sent again, it is the card made the first
    // time, and what was taken out here since goes there too); what it doesn't name, it says nothing of.
    let made = card_json(&create(
        "t",
        ContactFields { name: name("Hal", ""), company: Some(String::new()), title: Some(String::new()), ..Default::default() },
    ));
    assert_eq!((&made["organizations"], &made["titles"]), (&json!({}), &json!({})), "{made}");
    assert!(made.get("emails").is_none() && made.get("phones").is_none(), "{made}");
    let titled = card_json(&create("t", ContactFields { title: Some("Joiner".into()), ..Default::default() }));
    assert_eq!(titled["titles"], json!({"t1": {"@type": "Title", "name": "Joiner", "kind": "title"}}));
}

#[test]
fn emails_keep_their_primary_first_and_only_the_primary_mobile_keeps_its_kind() {
    let server = server();
    // The addresses in another order, and the mobile number moved after a new one.
    let fields = ContactFields {
        emails: Some(vec![Labeled::new("home", "bob@lindqvist.example.org"), Labeled::new("work", "bob@example.org")]),
        phones: Some(vec![
            Labeled::new("work", "+41 44 555 01 03"),
            Labeled::new("mobile", "+41 79 555 01 02"),
            Labeled::new("mobile", "+41 79 555 09 09"),
        ]),
        ..Default::default()
    };
    run(&server, &request(vec![update("Bob Lindqvist", fields)]));
    let (_, sent) = writes(&server).remove(0);
    assert_eq!(
        sent["emails"]["e1"],
        json!({"@type": "EmailAddress", "address": "bob@lindqvist.example.org", "contexts": {"private": true}, "pref": 1})
    );
    assert_eq!(sent["emails"]["e2"]["contexts"], json!({"work": true}));
    assert_eq!(sent["phones"]["p2"]["features"], json!({"mobile": true}));
    assert_eq!(sent["phones"]["p1"]["features"], json!({"voice": true}));
    let bob = card(&server, "Bob Lindqvist").unwrap();
    assert_eq!(bob.emails, ["bob@lindqvist.example.org", "bob@example.org"], "the one preferred is the primary there");
    assert_eq!(
        bob.phones,
        [("+41 44 555 01 03".to_string(), false), ("+41 79 555 01 02".to_string(), true), ("+41 79 555 09 09".to_string(), false)],
        "only the primary mobile number keeps its kind"
    );
    // A fax and a pager say what they are; what isn't an address is not sent.
    let card = card_json(&create(
        "t",
        ContactFields {
            emails: Some(vec![Labeled::new("", "not an address")]),
            phones: Some(vec![Labeled::new("fax", "+41 44 555 00 01"), Labeled::new("pager", "1234"), Labeled::new("", "  ")]),
            ..Default::default()
        },
    ));
    assert_eq!(card["emails"], json!({}));
    assert_eq!((card["phones"]["p1"]["features"].clone(), card["phones"]["p2"]["features"].clone()), (json!({"fax": true}), json!({"pager": true})));
    assert!(card["phones"].get("p3").is_none());
}

#[test]
fn a_patch_key_with_a_slash_is_never_sent() {
    let server = server();
    let every = ContactFields {
        name: name("Grace", "Okafor-Brown"),
        company: Some("Okafor Engineering".into()),
        title: Some("Engineer".into()),
        emails: Some(vec![Labeled::new("work", "grace@example.org")]),
        phones: Some(vec![Labeled::new("mobile", "+41 79 555 03 03")]),
    };
    let done = outcomes(&run(&server, &request(vec![update("Grace Okafor", every)])));
    assert_eq!(done[0].result, EventResult::Done, "{done:?}");
    for (_, sent) in writes(&server) {
        assert!(sent.as_object().unwrap().keys().all(|key| !key.contains('/')), "{sent}");
    }
    // (The server holds a client to it, and refuses a part of a property: `standin/pim/cards/tests.rs`.)
    assert_eq!(card(&server, "Grace Okafor").map(|grace| grace.last), Some("Okafor-Brown".to_string()));
}

#[test]
fn a_create_answered_already_exists_with_an_existing_id_is_that_card() {
    let server = server();
    run(&server, &request(vec![create("t-gus", gus())]));
    // A stricter server answers a card made again with the one it has: what was sent this time (a number added since)
    // is written to that one.
    server.with(|d| d.pim.refused_cards.insert("t-gus".into(), "alreadyExists"));
    let with_number = ContactFields { phones: Some(vec![Labeled::new("work", "+41 44 555 07 07")]), ..gus() };
    let done = outcomes(&run(&server, &request(vec![create("t-gus", with_number)])));
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
    assert_eq!(server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
    assert_eq!(card(&server, "Gus Hale").map(|c| c.phones), Some(vec![("+41 44 555 07 07".to_string(), false)]));
    // And one that knows it again by its uid answers that one (no second card).
    server.with(|d| d.pim.refused_cards.clear());
    let done = outcomes(&run(&server, &request(vec![create("t-gus", gus())])));
    assert_eq!(server_card_id(&done[0].remote).as_deref(), Some("Gus Hale"));
    assert_eq!(server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
}

#[test]
fn not_found_of_an_update_or_a_destroy_is_gone() {
    let server = server();
    let done = outcomes(&run(&server, &request(vec![update("Nobody Here", ContactFields { title: Some("x".into()), ..Default::default() })])));
    assert_eq!(done.iter().map(|o| o.result).collect::<Vec<_>>(), [EventResult::Gone]);
    let done = outcomes(&run(&server, &request(vec![destroy("Nobody Here")])));
    assert_eq!(done.iter().map(|o| o.result).collect::<Vec<_>>(), [EventResult::Gone]);
    // Two changes of one card in one job: the second waits for the next, with no answer (it is made to what the first
    // leaves).
    let title = |t: &str| ContactFields { title: Some(t.into()), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![update("Grace Okafor", title("Lead")), update("Grace Okafor", title("Head"))])));
    assert_eq!(done.iter().map(|o| o.result).collect::<Vec<_>>(), [EventResult::Done]);
    assert_eq!(card(&server, "Grace Okafor").map(|c| c.title), Some("Lead".to_string()));
    // A key that names no card of a server is refused before anything is sent.
    let odd = ContactOp { remote: "~!!".into(), ..destroy("x") };
    let done = outcomes(&run(&server, &request(vec![odd])));
    assert_eq!(done[0].result, EventResult::Refused);
}

#[test]
fn forbidden_with_a_documents_words_is_refused_and_said_cut_at_200_characters() {
    let server = server();
    let document = format!("Issue ISS-2026-00012 at {}", "row ".repeat(100));
    server.with(|d| d.pim.linked_cards.insert("Grace Okafor".into(), document));
    let done = outcomes(&run(&server, &request(vec![destroy("Grace Okafor")])));
    assert_eq!(done[0].result, EventResult::Refused);
    assert!(
        done[0].error.starts_with("Cannot delete or cancel because Contact Grace Okafor is linked with Issue ISS-2026-00012"),
        "{}",
        done[0].error
    );
    assert!(done[0].error.chars().count() <= MAX_WORDS + 1);
    assert!(card(&server, "Grace Okafor").is_some());
    // A card of a book the account may only read: its rights, in its words.
    for op in [update("Carol Duval", ContactFields { title: Some("x".into()), ..Default::default() }), destroy("Carol Duval")] {
        let done = outcomes(&run(&server, &request(vec![op])));
        assert_eq!(done.iter().map(|o| (o.result, o.error.as_str())).collect::<Vec<_>>(), [(EventResult::Refused, "Your rights do not allow it.")]);
    }
    // Markup is taken out of the server's words.
    let line = super::strip_tags("Cannot delete <strong>Contact</strong> <a href=\"/app/contact/x\">x</a>");
    assert_eq!(line, "Cannot delete Contact x");
}

#[test]
fn request_too_large_halves_the_chunk_the_next_time() {
    // Said by the method (`requestTooLarge`), or by the server for the request as a whole (400, `limit`): it didn't run,
    // and goes again in halves.
    for effect in [Effect::MethodError("requestTooLarge"), Effect::OverLimit] {
        let server = server();
        server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", effect)));
        let ops: Vec<ContactOp> =
            (0..4).map(|n| create(&format!("t-{n}"), ContactFields { name: name("Crowd", &format!("{n:02}")), ..Default::default() })).collect();
        let before = server.calls().len();
        let done = outcomes(&run(&server, &request(ops)));
        assert!(done.iter().all(|o| o.result == EventResult::Done), "{effect:?}: {done:?}");
        let sets = server.calls().into_iter().skip(before).filter(|c| c == "ContactCard/set").count();
        assert_eq!(sets, if effect == Effect::OverLimit { 2 } else { 3 }, "{effect:?}: the whole once (when it was looked at), then two halves");
        assert_eq!(server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Crowd").count()), 4);
    }
    // One change alone too large: given up as lasting, not split for ever.
    let server = server();
    server.with(|d| {
        for _ in 0..3 {
            d.faults.push_back(Fault::on("ContactCard/set", Effect::MethodError("requestTooLarge")));
        }
    });
    let done = outcomes(&run(&server, &request(vec![create("t-1", gus())])));
    assert_eq!((done[0].result, done[0].lasting), (EventResult::Failed, true));
}

#[test]
fn an_answer_too_large_to_take_is_not_taken_for_a_call_that_never_ran() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::Oversize)));
    let added = || ContactFields {
        emails: Some(vec![
            Labeled::new("", "bob@example.org"),
            Labeled::new("", "bob@lindqvist.example.org"),
            Labeled::new("", "robert@example.org"),
        ]),
        ..Default::default()
    };
    let base = ContactFields {
        emails: Some(vec![Labeled::new("", "bob@example.org"), Labeled::new("", "bob@lindqvist.example.org")]),
        ..Default::default()
    };
    let change = || ContactOp { base: Some(base.clone()), ..update("Bob Lindqvist", added()) };
    let before = server.calls().len();
    let done = outcomes(&run(&server, &request(vec![change(), create("t-gus", gus())])));
    assert!(done.iter().all(|o| o.result == EventResult::Failed && !o.lasting), "not known whether it went: it goes again: {done:?}");
    assert_eq!(server.calls().into_iter().skip(before).filter(|c| c == "ContactCard/set").count(), 1, "not split: it ran");
    assert!(card(&server, "Gus Hale").is_some() && card(&server, "Bob Lindqvist").is_some_and(|b| b.emails.len() == 3), "the server did it");
    // Again: the change made anew to the card as it is (no address twice), the card made again the same one.
    let done = outcomes(&run(&server, &request(vec![change(), create("t-gus", gus())])));
    assert!(done.iter().all(|o| o.result == EventResult::Done), "{done:?}");
    assert_eq!(card(&server, "Bob Lindqvist").map(|b| b.emails.len()), Some(3));
    assert_eq!(server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
}

#[test]
fn a_change_is_made_to_the_servers_card_as_it_is_now() {
    let server = server();
    // Someone added an address in the desk while a number of Bob's, changed here, waited to go.
    server.with(|d| {
        if let Some(bob) = d.pim.cards.iter_mut().find(|card| card.id == "Bob Lindqvist") {
            bob.emails.push("robert@example.org".into());
        }
    });
    let was = ContactFields {
        emails: Some(vec![Labeled::new("", "bob@example.org"), Labeled::new("", "bob@lindqvist.example.org")]),
        phones: Some(vec![Labeled::new("mobile", "+41 79 555 01 02"), Labeled::new("", "+41 44 555 01 03")]),
        ..Default::default()
    };
    let now = ContactFields {
        // The office number taken out, a new mobile one in; and an address taken out, another added.
        emails: Some(vec![Labeled::new("", "bob@example.org"), Labeled::new("work", "bob@joinery.example.org")]),
        phones: Some(vec![Labeled::new("mobile", "+41 79 555 01 02"), Labeled::new("mobile", "+41 79 555 01 09")]),
        ..Default::default()
    };
    let op = ContactOp { base: Some(was), ..update("Bob Lindqvist", now) };
    let done = outcomes(&run(&server, &request(vec![op])));
    assert_eq!(done[0].result, EventResult::Done, "{done:?}");
    let bob = card(&server, "Bob Lindqvist").unwrap();
    assert_eq!(bob.emails, ["bob@example.org", "robert@example.org", "bob@joinery.example.org"], "what the desk added stays");
    assert_eq!(bob.phones.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["+41 79 555 01 02", "+41 79 555 01 09"]);
    // It was read just before.
    let calls = server.calls();
    let at = calls.iter().rposition(|c| c == "ContactCard/set").unwrap();
    assert_eq!(calls.get(at - 1).map(String::as_str), Some("ContactCard/get"));
}

#[test]
fn a_name_another_contact_has_now_is_never_written_to() {
    let server = server();
    // Grace was deleted in the desk, and a new Grace Okafor took her name.
    let mark =
        server.with(|d| d.pim.cards.iter().find(|card| card.id == "Grace Okafor").map(|card| mailcraft_time::format_iso_utc(card.created))).unwrap();
    server.with(|d| {
        d.pim.cards.retain(|card| card.id != "Grace Okafor");
        let newcomer = Card::person("Grace Okafor", "neoffice", "Grace", "Okafor", "g.okafor@example.org");
        d.pim.add_card(newcomer);
    });
    let title = ContactFields { title: Some("Engineer".into()), ..Default::default() };
    let ops = vec![ContactOp { mark: mark.clone(), ..update("Grace Okafor", title) }];
    let done = outcomes(&run(&server, &request(ops)));
    assert_eq!((done[0].result, done[0].error.as_str()), (EventResult::Gone, "the server has another contact under its name now"));
    let done = outcomes(&run(&server, &request(vec![ContactOp { mark, ..destroy("Grace Okafor") }])));
    assert_eq!(done[0].result, EventResult::Gone);
    assert!(writes(&server).is_empty(), "nothing written to the newcomer");
    assert!(card(&server, "Grace Okafor").is_some_and(|c| c.title.is_empty()));
    // The very card it is about takes the change.
    let mark =
        server.with(|d| d.pim.cards.iter().find(|card| card.id == "Grace Okafor").map(|card| mailcraft_time::format_iso_utc(card.created))).unwrap();
    let ops = vec![ContactOp { mark, ..update("Grace Okafor", ContactFields { title: Some("Engineer".into()), ..Default::default() }) }];
    assert_eq!(outcomes(&run(&server, &request(ops)))[0].result, EventResult::Done);
}

#[test]
fn a_busy_server_answers_what_was_tried_and_leaves_the_rest_without_an_answer() {
    let server = server();
    server.with(|d| {
        d.max_set = 1;
        d.faults.push_back(Fault::later("ContactCard/set", 1, Effect::RefuseWith(503)));
    });
    let title = |t: &str| ContactFields { title: Some(t.into()), ..Default::default() };
    let ops = vec![update("Grace Okafor", title("Lead")), update("Bob Lindqvist", title("Joiner")), create("t-gus", gus())];
    let result = run(&server, &request(ops));
    let done: Vec<(EventResult, bool)> = outcomes(&result).iter().map(|o| (o.result, o.lasting)).collect();
    assert_eq!(done, [(EventResult::Done, false), (EventResult::Failed, false)], "the third was never tried: it waits, with no answer");
    assert!(result.busy_for.is_some());
}

#[test]
fn what_the_server_fails_with_for_this_change_says_it_is_likely_to_last() {
    let server = server();
    server.with(|d| {
        d.pim.refused_cards.insert("Grace Okafor".into(), "serverFail");
        d.pim.refused_cards.insert("Bob Lindqvist".into(), "rateLimit");
    });
    let title = || ContactFields { title: Some("x".into()), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![update("Grace Okafor", title()), update("Bob Lindqvist", title())])));
    assert_eq!(done.iter().map(|o| (o.result, o.lasting)).collect::<Vec<_>>(), [(EventResult::Failed, true), (EventResult::Failed, false)]);
}

#[test]
fn rate_limit_and_server_fail_wait() {
    let server = server();
    server.with(|d| {
        d.pim.refused_cards.insert("Grace Okafor".into(), "rateLimit");
        d.pim.refused_cards.insert("Bob Lindqvist".into(), "serverFail");
    });
    let title = || ContactFields { title: Some("x".into()), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![update("Grace Okafor", title()), update("Bob Lindqvist", title())])));
    assert!(done.iter().all(|o| o.result == EventResult::Failed && o.retry_after == Some(60)), "{done:?}");
    // A busy server stops the job: what wasn't answered waits, and the server is left alone as long as it says.
    server.with(|d| {
        d.pim.refused_cards.clear();
        d.faults.push_back(Fault::on("ContactCard/set", Effect::RefuseWith(503)));
    });
    let result = run(&server, &request(vec![update("Grace Okafor", title())]));
    assert!(result.busy_for.is_some() && outcomes(&result).iter().all(|o| o.result == EventResult::Failed));
}

#[test]
fn a_lost_answer_to_a_create_heals_at_the_following_read() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer)));
    let first = outcomes(&run(&server, &request(vec![create("t-gus", gus())])));
    assert_eq!(first[0].result, EventResult::Failed, "whether it went is not known: it goes again");
    assert_eq!(made(&first[0]), "perhaps", "and its card may be on the server");
    assert!(card(&server, "Gus Hale").is_some_and(|c| c.phones.len() == 2), "the server made it");
    // A number taken out here before the next try: sent again with its mark, it is the same card, which has what was
    // sent this time; and the next read brings that one alone.
    let fewer = ContactFields { phones: Some(vec![Labeled::new("mobile", "+41 79 555 02 02")]), ..gus() };
    let mut again = request(vec![create("t-gus", fewer)]);
    again.contacts = true;
    let result = run(&server, &again);
    let done = outcomes(&result);
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
    let read = result.contacts.as_ref().unwrap();
    assert_eq!(read.contacts.iter().filter(|c| c.first == "Gus").count(), 1);
    assert_eq!(card(&server, "Gus Hale").map(|c| c.phones.len()), Some(1), "what was sent this time");
    // The card read says no mark of its own (the server's uid is its name): a contact made here finds it by the answer.
    // It says when it was made, which a change of it carries.
    assert!(read.contacts.iter().all(|c| c.uid.is_empty() && c.change_key.ends_with('Z')), "{:?}", read.contacts);
}

#[test]
fn fifty_cards_a_call_and_the_rest_wait() {
    let server = server();
    let ops: Vec<ContactOp> =
        (0..60).map(|n| create(&format!("t-{n}"), ContactFields { name: name("Crowd", &format!("{n:02}")), ..Default::default() })).collect();
    let before = server.calls().len();
    let done = outcomes(&run(&server, &request(ops)));
    assert_eq!(done.len(), MAX_PER_CALL, "an upload carries fifty: the rest wait for the next");
    assert!(done.iter().all(|o| o.result == EventResult::Done));
    assert_eq!(server.calls().into_iter().skip(before).filter(|c| c == "ContactCard/set").count(), 1);
    // A server that takes fewer in a set is sent as many as it takes at a time.
    let smaller = self::server();
    smaller.with(|d| d.max_set = 20);
    let ops: Vec<ContactOp> =
        (0..45).map(|n| create(&format!("t-{n}"), ContactFields { name: name("Crowd", &format!("{n:02}")), ..Default::default() })).collect();
    let before = smaller.calls().len();
    run(&smaller, &request(ops));
    assert_eq!(smaller.calls().into_iter().skip(before).filter(|c| c == "ContactCard/set").count(), 3);
}

#[test]
fn a_refused_token_ends_the_job_and_what_was_not_answered_waits() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::RefuseWith(401))));
    let mut request = request(vec![update("Grace Okafor", ContactFields { title: Some("x".into()), ..Default::default() })]);
    request.contacts = true;
    let result = run(&server, &request);
    assert!(result.auth_refused, "{result:?}");
    let uploaded = result.contacts_uploaded.as_ref().unwrap();
    assert!(uploaded.token_refused && uploaded.outcomes.is_empty());
    assert!(result.contacts.is_none(), "nothing is read after that");
    // A stop before anything was sent leaves it all waiting.
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let stopped = block_on(sync_pim(&client, &request, &AtomicBool::new(true)));
    assert!(stopped.stopped && outcomes(&stopped).is_empty());
}

/// A new card sent before whose answer didn't come, made with the mark `token`: `fields` now, and what it was sent
/// with then (`sent`).
fn resend(token: &str, fields: ContactFields, sent: ContactFields) -> ContactOp {
    ContactOp { base: Some(sent), ..create(token, fields) }
}

#[test]
fn a_new_card_sent_again_takes_out_there_what_was_taken_out_here() {
    // A server that can't look for a card by its uid (its only filter is the book).
    let server = server();
    server.with(|d| {
        d.pim.finds_by_uid = false;
        d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer));
    });
    run(&server, &request(vec![create("t-gus", gus())]));
    assert!(card(&server, "Gus Hale").is_some_and(|c| c.phones.len() == 2 && !c.company.is_empty()), "the server made it");
    // His numbers and his company taken out here before the next try. Sent again (the server finds no card by its
    // mark: it filters on none), it names them empty, and the card made the first time has none.
    let cleared = ContactFields { phones: Some(Vec::new()), company: Some(String::new()), ..gus() };
    let done = outcomes(&run(&server, &request(vec![resend("t-gus", cleared, gus())])));
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
    let made = card(&server, "Gus Hale").unwrap();
    assert!(made.phones.is_empty() && made.company.is_empty(), "{made:?}");
    let (_, sent) = writes(&server).pop().unwrap();
    assert_eq!((&sent["phones"], &sent["organizations"]), (&json!({}), &json!({})));
    assert_eq!(server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
}

#[test]
fn a_new_card_sent_again_that_the_server_finds_by_its_mark_gets_only_what_was_changed_here_since() {
    let server = server();
    server.with(|d| {
        d.pim.finds_by_uid = true;
        d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer));
    });
    run(&server, &request(vec![create("t-gus", gus())]));
    // Meanwhile the desk gives him another address and a new job title; here, a number is added.
    server.with(|d| {
        if let Some(card) = d.pim.cards.iter_mut().find(|card| card.id == "Gus Hale") {
            card.emails.push("gus@joinery.example.org".into());
            card.title = "Master joiner".into();
        }
    });
    let mut more = gus();
    more.phones.get_or_insert_with(Vec::new).push(Labeled::new("", "+41 44 555 02 09"));
    let before = writes(&server).len();
    let done = outcomes(&run(&server, &request(vec![resend("t-gus", more, gus())])));
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
    let made = card(&server, "Gus Hale").unwrap();
    assert_eq!(made.emails, ["gus@example.org", "gus.hale@example.org", "gus@joinery.example.org"], "the desk's address stays");
    assert_eq!((made.title.as_str(), made.phones.len()), ("Master joiner", 3), "and its title; the number went");
    let sent: Vec<(String, Value)> = writes(&server).into_iter().skip(before).collect();
    let keys: Vec<&str> = sent.iter().flat_map(|(_, what)| what.as_object().into_iter().flat_map(|map| map.keys().map(String::as_str))).collect();
    assert_eq!((sent.len(), sent[0].0.as_str(), keys), (1, "Gus Hale", vec!["phones"]), "a change of that card, of what changed here alone");
    assert_eq!(server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
    // Nothing changed here since it was sent: nothing is written, and it is that card.
    let before = writes(&server).len();
    let done = outcomes(&run(&server, &request(vec![resend("t-gus", gus(), gus())])));
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
    assert_eq!(writes(&server).len(), before);
    // A server that says the card is there already: it is read first, and gets only what was changed here since too.
    let server = self::server();
    run(&server, &request(vec![create("t-gus", gus())]));
    server.with(|d| {
        if let Some(card) = d.pim.cards.iter_mut().find(|card| card.id == "Gus Hale") {
            card.emails.push("gus@joinery.example.org".into());
        }
        d.pim.refused_cards.insert("t-gus".into(), "alreadyExists");
    });
    let titled = ContactFields { title: Some("Master joiner".into()), ..gus() };
    let done = outcomes(&run(&server, &request(vec![resend("t-gus", titled, gus())])));
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
    let made = card(&server, "Gus Hale").unwrap();
    assert_eq!((made.emails.len(), made.title.as_str()), (3, "Master joiner"), "{made:?}");
    let (_, sent) = writes(&server).pop().unwrap();
    assert_eq!(sent.as_object().map(|map| map.keys().cloned().collect::<Vec<String>>()), Some(vec!["titles".to_string()]));
}

#[test]
fn in_a_job_that_sends_a_new_card_again_a_change_of_the_card_it_is_answered_with_waits() {
    for finds in [false, true] {
        let server = server();
        server.with(|d| {
            d.pim.finds_by_uid = finds;
            d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer));
        });
        run(&server, &request(vec![create("t-gus", gus())]));
        // A read brought his card as a contact of its own, which was deleted here (and another such copy changed): in
        // the job that sends Gus again, neither goes before the server said which card it made of him.
        let title = ContactFields { title: Some("Joiner".into()), ..Default::default() };
        let ops = vec![resend("t-gus", gus(), gus()), destroy("Gus Hale"), update("Gus Hale", title)];
        let done = outcomes(&run(&server, &request(ops)));
        let answered: Vec<(EventResult, ContactChangeKind)> = done.iter().map(|o| (o.result, o.op.kind)).collect();
        assert_eq!(answered, [(EventResult::Done, ContactChangeKind::Create)], "found by its mark: {finds}");
        assert_eq!(server_card_id(&done[0].remote).as_deref(), Some("Gus Hale"));
        assert!(card(&server, "Gus Hale").is_some(), "never deleted: {finds}");
        assert!(writes(&server).iter().all(|(_, sent)| sent != "destroy"));
    }
}

#[test]
fn a_card_the_server_says_nothing_of_is_not_asked_for_for_ever_and_a_413_halves_the_call() {
    // A `get` that neither lists the card nor says it is not there: asking again is unlikely to get past it.
    let server = server();
    server.with(|d| d.pim.unsaid_cards.insert("Bob Lindqvist".into()));
    let title = |t: &str| ContactFields { title: Some(t.into()), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![update("Bob Lindqvist", title("Joiner"))])));
    assert_eq!((done[0].result, done[0].lasting, done[0].error.as_str()), (EventResult::Failed, true, "the server's answer did not have it"));
    // A proxy before the server that refuses a request for its size says so with a 413, and no more: it went nowhere,
    // and goes again in halves.
    let server = self::server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::RefuseWith(413))));
    let done = outcomes(&run(&server, &request(vec![update("Grace Okafor", title("Lead")), update("Bob Lindqvist", title("Joiner"))])));
    assert!(done.iter().all(|o| o.result == EventResult::Done), "{done:?}");
    assert_eq!(writes(&server).len(), 2);
}

#[test]
fn a_cards_time_of_making_is_compared_as_an_instant() {
    let op = |mark: &str| ContactOp { mark: mark.into(), ..update("Grace Okafor", ContactFields::default()) };
    for (mark, now, another) in [
        ("2026-10-14T16:00:00Z", "2026-10-14T18:00:00+02:00", false),
        ("2026-10-14T16:00:00Z", "2026-10-14T16:00:00.250Z", false),
        ("2026-10-14T16:00:00Z", "2026-10-14T16:00:01Z", true),
        ("2026-10-14T16:00:00Z", "2026-10-14T16:00:00+01:00", true),
        ("v1", "v1", false),
        ("v1", "v2", true),
        ("", "2026-10-14T16:00:01Z", false),
        ("2026-10-14T16:00:00Z", "", false),
    ] {
        assert_eq!(super::another(&op(mark), &json!({"id": "Grace Okafor", "created": now})), another, "{mark} / {now}");
    }
}

#[test]
fn every_deletion_waits_while_a_new_card_sent_again_has_no_answer_that_names_its_card() {
    // A server that can't look for a card by its uid: Gus's first answer was lost, a read brought his card as a contact
    // of its own, and that copy was deleted here. Sent again, his answer is lost too: which card he is isn't known, and
    // the deletion (of that very card) waits, with no outcome.
    let server = server();
    server.with(|d| {
        d.pim.finds_by_uid = false;
        d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer));
    });
    run(&server, &request(vec![create("t-gus", gus())]));
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer)));
    let title = ContactFields { title: Some("Lead".into()), ..Default::default() };
    let ops = vec![resend("t-gus", gus(), gus()), destroy("Gus Hale"), update("Grace Okafor", title)];
    let done = outcomes(&run(&server, &request(ops)));
    let answered: Vec<(ContactChangeKind, EventResult)> = done.iter().map(|o| (o.op.kind, o.result)).collect();
    assert_eq!(answered, [(ContactChangeKind::Create, EventResult::Failed)], "what is about a card the server has waits");
    assert!(card(&server, "Gus Hale").is_some(), "never deleted");
    assert!(writes(&server).iter().all(|(_, sent)| sent != "destroy"));
}

#[test]
fn a_look_for_a_card_that_fails_sends_nothing_and_what_the_desk_added_stays() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer)));
    run(&server, &request(vec![create("t-gus", gus())]));
    // The desk gives him a number meanwhile; here, a job title.
    server.with(|d| {
        if let Some(card) = d.pim.cards.iter_mut().find(|card| card.id == "Gus Hale") {
            card.phones.push(("+41 44 555 02 09".into(), false));
        }
    });
    let titled = ContactFields { title: Some("Master joiner".into()), ..gus() };
    // The server fails to look for him: he is not sent whole (which would take the desk's number away), but waits. A
    // look the server fails with is likely to fail again (given up after a few); a connection lost on the way is not.
    let before = writes(&server).len();
    for (effect, lasting) in
        [(Effect::MethodError("serverFail"), true), (Effect::MethodError("invalidArguments"), true), (Effect::RefuseWith(500), false)]
    {
        server.with(|d| d.faults.push_back(Fault::on("ContactCard/query", effect)));
        let done = outcomes(&run(&server, &request(vec![resend("t-gus", titled.clone(), gus())])));
        assert_eq!((done[0].result, done[0].lasting), (EventResult::Failed, lasting), "{effect:?}: {done:?}");
        assert_eq!(made(&done[0]), "", "{effect:?}: nothing new is known of his card");
    }
    assert_eq!(writes(&server).len(), before, "nothing sent");
    // The next time it finds him, and writes only the title.
    let done = outcomes(&run(&server, &request(vec![resend("t-gus", titled, gus())])));
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
    let made = card(&server, "Gus Hale").unwrap();
    assert_eq!((made.phones.len(), made.title.as_str()), (3, "Master joiner"), "{made:?}");
    // A server that says it can't look so (`unsupportedFilter`) has him sent again as a new card at once.
    let server = self::server();
    server.with(|d| {
        d.pim.finds_by_uid = false;
        d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer));
    });
    run(&server, &request(vec![create("t-gus", gus())]));
    let done = outcomes(&run(&server, &request(vec![resend("t-gus", gus(), gus())])));
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
}

#[test]
fn a_new_card_sent_again_refused_for_good_names_the_card_it_was_made_as() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer)));
    run(&server, &request(vec![create("t-gus", gus())]));
    // Found by its mark, the change of that card is refused: the answer says which card, for the contact to be it.
    server.with(|d| d.pim.refused_cards.insert("Gus Hale".into(), "forbidden"));
    let titled = ContactFields { title: Some("Master joiner".into()), ..gus() };
    let done = outcomes(&run(&server, &request(vec![resend("t-gus", titled, gus())])));
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Refused, Some("Gus Hale")));
}

/// What an outcome says of a card made of a new contact, as a host hands it to the engine (`""`: nothing new).
fn made(outcome: &ContactOutcome) -> String {
    serde_json::to_value(outcome).ok().and_then(|value| value.get("made").and_then(Value::as_str).map(str::to_string)).unwrap_or_default()
}

#[test]
fn a_new_card_the_server_refuses_to_make_says_none_was_made_and_holds_no_deletion_back() {
    // An account over its quota: the new card is refused (it goes again when there is room: it is not given up),
    // none was made, the answer says the server has no room, and the deletion that would make room goes.
    let server = server();
    server.with(|d| d.pim.full = true);
    let done = outcomes(&run(&server, &request(vec![create("t-gus", gus()), destroy("Bob Lindqvist")])));
    let answered: Vec<(ContactChangeKind, EventResult, bool, String, bool)> =
        done.iter().map(|o| (o.op.kind, o.result, o.lasting, made(o), says(o, "full"))).collect();
    assert_eq!(
        answered,
        [
            (ContactChangeKind::Create, EventResult::Failed, false, "no".to_string(), true),
            (ContactChangeKind::Delete, EventResult::Done, false, String::new(), false)
        ]
    );
    server.with(|d| d.pim.full = false);
    assert!(card(&server, "Bob Lindqvist").is_none());
    // One sent before that its server looked for and found none of, refused again: none was made, and nothing waits
    // for it either.
    server.with(|d| d.pim.refused_cards.insert("t-hal".into(), "overQuota"));
    let hal = ContactFields { name: name("Hal", "Berg"), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![resend("t-hal", hal.clone(), hal), destroy("Grace Okafor")])));
    let answered: Vec<(EventResult, String)> = done.iter().map(|o| (o.result, made(o))).collect();
    assert_eq!(answered, [(EventResult::Failed, "no".to_string()), (EventResult::Done, String::new())]);
    // A server that can't look: one sent before may have been made the first time, refused or not now.
    let server = self::server();
    server.with(|d| {
        d.pim.finds_by_uid = false;
        d.pim.refused_cards.insert("t-hal".into(), "overQuota");
    });
    let hal = ContactFields { name: name("Hal", "Berg"), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![resend("t-hal", hal.clone(), hal)])));
    assert_eq!((done[0].result, made(&done[0])), (EventResult::Failed, String::new()));
}

#[test]
fn a_second_new_card_sent_again_that_finds_a_card_another_took_waits() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::LoseAnswer)));
    run(&server, &request(vec![create("t-gus", gus())]));
    let before = writes(&server).len();
    let twice = vec![resend("t-gus", gus(), gus()), ContactOp { contact: 43, ..resend("t-gus", gus(), gus()) }];
    let done = outcomes(&run(&server, &request(twice)));
    assert_eq!(done.iter().map(|o| (o.op.contact, o.result)).collect::<Vec<_>>(), [(40, EventResult::Done)], "the second waits, unanswered");
    assert_eq!(writes(&server).len(), before, "and is not sent whole");
}

/// Whether an outcome says `key`, as a host hands it to the engine.
fn says(outcome: &ContactOutcome, key: &str) -> bool {
    serde_json::to_value(outcome).ok().and_then(|value| value.get(key).and_then(Value::as_bool)).unwrap_or(false)
}

#[test]
fn whatever_may_have_run_says_a_new_card_may_be_on_the_server_and_only_a_refusal_before_says_none() {
    // The server may have made it: a call that ran in part, a gateway's 503 or 429 after it ran, an answer lost or
    // that is no JMAP.
    for effect in [Effect::MethodError("serverPartialFail"), Effect::Status(503), Effect::Status(429), Effect::LoseAnswer, Effect::Garbage] {
        let server = server();
        server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", effect)));
        let done = outcomes(&run(&server, &request(vec![create("t-gus", gus())])));
        assert_eq!((done[0].result, made(&done[0])), (EventResult::Failed, "perhaps".to_string()), "{effect:?}");
    }
    // The method refused as a whole: it ran nothing, and none was made of a card never sent before.
    for effect in [Effect::MethodError("invalidArguments"), Effect::MethodError("serverFail")] {
        let server = server();
        server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", effect)));
        let done = outcomes(&run(&server, &request(vec![create("t-gus", gus())])));
        assert_eq!(made(&done[0]), "no", "{effect:?}");
    }
    // The request refused as a whole before it ran: nothing new is known.
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::RefuseWith(400))));
    let done = outcomes(&run(&server, &request(vec![create("t-gus", gus())])));
    assert_eq!((done[0].result, made(&done[0])), (EventResult::Failed, String::new()));
}

#[test]
fn a_new_card_sent_again_whose_set_ran_nothing_after_its_look_found_none_holds_no_deletion_back() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::MethodError("invalidArguments"))));
    let hal = ContactFields { name: name("Hal", "Berg"), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![resend("t-hal", hal.clone(), hal), destroy("Bob Lindqvist")])));
    let answered: Vec<(ContactChangeKind, EventResult, String)> = done.iter().map(|o| (o.op.kind, o.result, made(o))).collect();
    assert_eq!(
        answered,
        [(ContactChangeKind::Create, EventResult::Refused, "no".to_string()), (ContactChangeKind::Delete, EventResult::Done, String::new())]
    );
}

#[test]
fn a_new_card_sent_again_whole_after_its_look_found_none_says_so() {
    // Its look finds none (nothing was made the first time); sent whole, a gateway's 502 comes back after the server
    // made it: the card made of it has what was sent this time alone.
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::Status(502))));
    let hal = ContactFields { name: name("Hal", "Berg"), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![resend("t-hal", hal.clone(), hal)])));
    assert_eq!((made(&done[0]), says(&done[0], "noneBefore")), ("perhaps".to_string(), true), "{done:?}");
    // One whose look was not possible says nothing of the kind.
    let server = self::server();
    server.with(|d| {
        d.pim.finds_by_uid = false;
        d.faults.push_back(Fault::on("ContactCard/set", Effect::Status(502)));
    });
    let hal = ContactFields { name: name("Hal", "Berg"), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![resend("t-hal", hal.clone(), hal)])));
    assert_eq!((made(&done[0]), says(&done[0], "noneBefore")), ("perhaps".to_string(), false));
}

#[test]
fn a_set_the_server_refuses_for_its_quota_as_a_whole_says_it_has_no_room() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::MethodError("overQuota"))));
    let title = ContactFields { title: Some("Lead".into()), ..Default::default() };
    let done = outcomes(&run(&server, &request(vec![update("Grace Okafor", title), create("t-gus", gus())])));
    assert!(done.iter().all(|o| o.result == EventResult::Failed && !o.lasting && says(o, "full")), "{done:?}");
}
