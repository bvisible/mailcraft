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
    // A card that says nothing of a name goes without one, and its server names it from its address; one that has
    // neither is refused with the server's words.
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
    assert_eq!((done[1].result, done[1].error.as_str()), (EventResult::Refused, "A contact needs a name or an address."));
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
    // A new card with neither says nothing of them.
    let made = card_json(&create(
        "t",
        ContactFields { name: name("Hal", ""), company: Some(String::new()), title: Some(String::new()), ..Default::default() },
    ));
    assert!(made.get("organizations").is_none() && made.get("titles").is_none() && made.get("emails").is_none(), "{made}");
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
    assert_eq!(bob.emails, ["bob@example.org", "bob@lindqvist.example.org"], "the server keeps its primary first");
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
    // A stricter server answers a card made again with the one it has.
    server.with(|d| d.pim.refused_cards.insert("t-gus".into(), "alreadyExists"));
    let done = outcomes(&run(&server, &request(vec![create("t-gus", gus())])));
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
    assert_eq!(server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
    // And one that knows it again by its uid answers that one (no second card).
    server.with(|d| d.pim.refused_cards.clear());
    let done = outcomes(&run(&server, &request(vec![create("t-gus", gus())])));
    assert_eq!(server_card_id(&done[0].remote).as_deref(), Some("Gus Hale"));
    assert_eq!(server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
}

#[test]
fn not_found_of_an_update_or_a_destroy_is_gone() {
    let server = server();
    let ops = vec![update("Nobody Here", ContactFields { title: Some("x".into()), ..Default::default() }), destroy("Nobody Here")];
    let done = outcomes(&run(&server, &request(ops)));
    assert_eq!(done.iter().map(|o| o.result).collect::<Vec<_>>(), [EventResult::Gone, EventResult::Gone]);
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
    let done = outcomes(&run(
        &server,
        &request(vec![update("Carol Duval", ContactFields { title: Some("x".into()), ..Default::default() }), destroy("Carol Duval")]),
    ));
    assert_eq!(done.iter().map(|o| (o.result, o.error.as_str())).collect::<Vec<_>>(), [(EventResult::Refused, "Your rights do not allow it."); 2]);
    // Markup is taken out of the server's words.
    let line = super::strip_tags("Cannot delete <strong>Contact</strong> <a href=\"/app/contact/x\">x</a>");
    assert_eq!(line, "Cannot delete Contact x");
}

#[test]
fn request_too_large_halves_the_chunk_the_next_time() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", Effect::MethodError("requestTooLarge"))));
    let ops: Vec<ContactOp> =
        (0..4).map(|n| create(&format!("t-{n}"), ContactFields { name: name("Crowd", &format!("{n:02}")), ..Default::default() })).collect();
    let before = server.calls().len();
    let done = outcomes(&run(&server, &request(ops)));
    assert!(done.iter().all(|o| o.result == EventResult::Done), "{done:?}");
    let sets = server.calls().into_iter().skip(before).filter(|c| c == "ContactCard/set").count();
    assert_eq!(sets, 3, "the whole once, then two halves");
    assert_eq!(server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Crowd").count()), 4);
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
    assert!(card(&server, "Gus Hale").is_some(), "the server made it");
    // Sent again with its mark, it is the same card, and the next read brings that one alone.
    let mut again = request(vec![create("t-gus", gus())]);
    again.contacts = true;
    let result = run(&server, &again);
    let done = outcomes(&result);
    assert_eq!((done[0].result, server_card_id(&done[0].remote).as_deref()), (EventResult::Done, Some("Gus Hale")));
    let read = result.contacts.as_ref().unwrap();
    assert_eq!(read.contacts.iter().filter(|c| c.first == "Gus").count(), 1);
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
