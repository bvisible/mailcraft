// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A job's reading of the address books against the in-process server: what is asked, in how many requests, the state
//! that spares a whole read, what comes back, and what trouble comes to. Invented people at `example.org`.

use std::sync::atomic::AtomicBool;

use mailcraft_model::remote::ContactsDownload;
use mailcraft_model::sync::{CONTACTS_STATE_FOR, JmapPimRequest, JmapPimResult, PimOffers};

use super::*;
use crate::pim::{card_key, server_card_id, sync_pim};
use crate::standin::{Book, Card, Fault, Pim, SESSION, StandIn, TOKEN};
use crate::{Auth, block_on};

const ME: &str = "ada@example.org";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// A server with the team's book (which Ada may write, and which is the default) and a partners' book she may only
/// read, and a few people in them, named as a server that calls a card by its contact's name names them.
fn server() -> StandIn {
    let server = StandIn::new();
    server.with(|d| {
        let pim = &mut d.pim;
        *pim = Pim::offered();
        pim.add_book(Book { default: true, ..Book::new("neoffice", "Neoffice", true) });
        pim.add_book(Book::new("partners", "Partners", false));
        pim.add_card(Card::person("Grace Okafor", "neoffice", "Grace", "Okafor", "grace@example.org").working("Engineer", "Okafor Engineering"));
        pim.add_card(Card::person("Bob Lindqvist", "neoffice", "Bob", "Lindqvist", "bob@example.org").with_phone("+41 79 555 01 02", true));
        pim.add_card(Card::company("Lindqvist Joinery", "neoffice", "Lindqvist Joinery", "joinery@example.org"));
        pim.add_card(Card::person("Carol Duval", "partners", "Carol", "Duval", "carol@example.org"));
    });
    server
}

/// A job that brings the contacts alone (no calendar asked for), holding no state.
fn request() -> JmapPimRequest {
    JmapPimRequest {
        account: 7,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        me: vec![ME.into()],
        now: NOW,
        contacts: true,
        ..Default::default()
    }
}

/// The same, holding `state` from a whole read `ago` seconds before.
fn holding(state: &str, ago: i64) -> JmapPimRequest {
    JmapPimRequest { contacts_state: state.into(), contacts_read: NOW - ago, ..request() }
}

fn run(server: &StandIn, request: &JmapPimRequest) -> JmapPimResult {
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    block_on(sync_pim(&client, request, &AtomicBool::new(false)))
}

fn contacts(result: &JmapPimResult) -> &ContactsDownload {
    result.contacts.as_ref().unwrap_or_else(|| panic!("no contacts: {result:?}"))
}

/// The server's names of the cards that came, in order.
fn names(result: &JmapPimResult) -> Vec<String> {
    contacts(result).contacts.iter().map(|card| server_card_id(&card.id).unwrap_or_default()).collect()
}

/// The methods asked for, in order, since `from` of them.
fn calls_since(server: &StandIn, from: usize) -> Vec<String> {
    server.calls().into_iter().skip(from).collect()
}

#[test]
fn the_book_and_its_rights_are_read() {
    let result = run(&server(), &request());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.offers, Some(PimOffers { calendars: true, contacts: true }));
    let books = result.books.as_ref().unwrap();
    assert!(books.complete && books.account == 7 && books.email == ME);
    let listed: Vec<(&str, &str, bool, bool, bool)> =
        books.books.iter().map(|b| (b.id.as_str(), b.name.as_str(), b.read_only, b.may_remove, b.main)).collect();
    // A book is read-only unless the account may write its cards; the server's default is the main one.
    assert_eq!(listed, [("neoffice", "Neoffice", false, true, true), ("partners", "Partners", true, false, false)]);
    assert!(books.books.iter().all(|b| b.plain), "the session says a card keeps only the basics");
    const { assert!(WRITE_CONTACTS) };
    // Each card in its book, and the cards all there.
    let download = contacts(&result);
    assert!(download.complete && download.account == 7);
    assert_eq!(names(&result), ["Bob Lindqvist", "Carol Duval", "Grace Okafor", "Lindqvist Joinery"], "by the server's order");
    let book_of = |name: &str| download.contacts.iter().find(|c| c.id == card_key(name)).map(|c| c.book.clone()).unwrap();
    assert_eq!((book_of("Grace Okafor"), book_of("Carol Duval")), ("neoffice".to_string(), "partners".to_string()));
    let grace = download.contacts.iter().find(|c| c.first == "Grace").unwrap();
    assert_eq!((grace.company.as_str(), grace.title.as_str()), ("Okafor Engineering", "Engineer"));
    let joinery = download.contacts.iter().find(|c| c.company == "Lindqvist Joinery").unwrap();
    assert_eq!((joinery.display_name.as_str(), joinery.first.as_str()), ("Lindqvist Joinery", ""));
    // The state the cards were read at comes with them, for the next job to ask about alone.
    assert_eq!(result.contacts_state, Some(server().with(|d| d.pim.cards_state())));
}

#[test]
fn plain_comes_from_what_the_session_says_it_writes_or_from_the_request() {
    let server = server();
    let plain = |server: &StandIn, request: &JmapPimRequest| run(server, request).books.unwrap().books.iter().all(|b| b.plain);
    assert!(plain(&server, &request()));
    server.with(|d| d.pim.says_writable = false);
    assert!(!plain(&server, &request()), "unsaid: the server is taken to keep everything");
    assert!(plain(&server, &JmapPimRequest { plain_unless_said: true, ..request() }), "unless the host knows it for one that keeps the basics");
}

#[test]
fn cards_come_in_chunks_of_the_servers_limit_and_page_after_page() {
    let server = server();
    server.with(|d| {
        for n in 0..230 {
            d.pim.add_card(Card::person(&format!("Person {n:03}"), "neoffice", "Person", &format!("{n:03}"), &format!("p{n}@example.org")));
        }
        // A server that lists 100 cards at a time and reads 50 in a `get`.
        d.pim.card_limit = 100;
        d.max_get = 50;
    });
    let result = run(&server, &request());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(contacts(&result).complete);
    assert_eq!(contacts(&result).contacts.len(), 234);
    let calls = server.calls();
    assert_eq!(calls.iter().filter(|c| *c == "ContactCard/query").count(), 3, "pages of 100, followed: {calls:?}");
    assert_eq!(calls.iter().filter(|c| *c == "ContactCard/get").count(), 5, "234 cards, 50 a get");
    // The books and the first page come in one request.
    assert_eq!(calls[..2], ["AddressBook/get", "ContactCard/query"]);
    let gets: Vec<usize> = server.asked().iter().filter(|a| a.method == "ContactCard/get").filter_map(|a| a.ids).collect();
    assert_eq!(gets, [50, 50, 50, 50, 34]);
}

#[test]
fn a_read_cut_at_the_cap_is_incomplete_and_says_no_state() {
    let server = server();
    server.with(|d| {
        for n in 0..READ_CARDS {
            d.pim.add_card(Card::person(&format!("Many {n:05}"), "neoffice", "Many", &format!("{n:05}"), &format!("m{n}@example.org")));
        }
    });
    let result = run(&server, &request());
    assert!(result.errors.is_empty(), "a limit of this build's is no trouble of the server's: {:?}", result.errors);
    let download = contacts(&result);
    assert_eq!(download.contacts.len(), READ_CARDS);
    assert!(!download.complete, "the cards after the cap are there: nothing is removed for not being read");
    assert_eq!(result.contacts_state, None, "the next job reads them again");
}

#[test]
fn a_card_that_is_not_found_between_the_query_and_the_get_is_left_out_not_an_error() {
    let server = server();
    server.with(|d| {
        d.pim.vanishing.insert("Carol Duval".into());
    });
    let result = run(&server, &request());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(names(&result), ["Bob Lindqvist", "Grace Okafor", "Lindqvist Joinery"]);
    assert!(contacts(&result).complete, "gone since is gone: the engine may remove it");
}

#[test]
fn an_unchanged_query_state_reads_one_card_and_nothing_else() {
    let server = server();
    let state = run(&server, &request()).contacts_state.unwrap();
    let before = server.calls().len();
    let result = run(&server, &holding(&state, 600));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(calls_since(&server, before), ["AddressBook/get", "ContactCard/query"], "one request, and no card read");
    let probe = server.asked().into_iter().rev().find(|a| a.method == "ContactCard/query").unwrap();
    assert_eq!(probe.limit, Some(1));
    // The books come all the same (their rights may have changed); the cards don't, and the state is said again.
    assert_eq!(result.books.as_ref().map(|b| b.books.len()), Some(2));
    assert!(result.contacts.is_none());
    assert_eq!(result.contacts_state.as_deref(), Some(state.as_str()));
}

#[test]
fn a_changed_state_reads_everything() {
    let server = server();
    let state = run(&server, &request()).contacts_state.unwrap();
    server.with(|d| d.pim.cards[0].title = "Chief engineer".into());
    let before = server.calls().len();
    let result = run(&server, &holding(&state, 600));
    assert_eq!(calls_since(&server, before), ["AddressBook/get", "ContactCard/query", "ContactCard/query", "ContactCard/get"], "the books once");
    assert!(contacts(&result).complete);
    assert!(contacts(&result).contacts.iter().any(|c| c.title == "Chief engineer"));
    let fresh = result.contacts_state.unwrap();
    assert!(fresh != state && fresh == server.with(|d| d.pim.cards_state()));
}

#[test]
fn a_state_older_than_an_hour_is_read_through() {
    let server = server();
    let state = run(&server, &request()).contacts_state.unwrap();
    let before = server.calls().len();
    let result = run(&server, &holding(&state, CONTACTS_STATE_FOR));
    assert_eq!(calls_since(&server, before), ["AddressBook/get", "ContactCard/query", "ContactCard/get"], "not asked about: read");
    assert!(contacts(&result).complete && result.contacts_state.as_deref() == Some(state.as_str()));
}

#[test]
fn a_server_that_says_no_state_is_always_read() {
    let server = server();
    server.with(|d| d.pim.says_state = false);
    let result = run(&server, &request());
    assert!(contacts(&result).complete && result.contacts_state.is_none());
    let before = server.calls().len();
    let result = run(&server, &holding("anything", 60));
    assert_eq!(calls_since(&server, before), ["AddressBook/get", "ContactCard/query", "ContactCard/query", "ContactCard/get"]);
    assert!(contacts(&result).complete && result.contacts_state.is_none());
}

#[test]
fn the_state_is_forgotten_after_an_error() {
    // A `get` that fails for good: the cards it held are not read, the download is not complete, and no state comes.
    let server = server();
    server.with(|d| {
        d.pim.broken_cards.insert("Grace Okafor".into());
    });
    let result = run(&server, &request());
    assert_eq!(names(&result), ["Bob Lindqvist", "Carol Duval", "Lindqvist Joinery"], "halved until the card that breaks it is found");
    assert!(!contacts(&result).complete && result.contacts_state.is_none());
    assert!(result.errors.iter().any(|line| line.starts_with("Couldn't read 1 contact of the address books")), "{:?}", result.errors);
    // A query that fails: nothing of the cards, a line, and no state; the books still come.
    let server = self::server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/query", crate::standin::Effect::MethodError("serverFail"))));
    let result = run(&server, &request());
    assert!(result.contacts.is_none() && result.contacts_state.is_none() && result.books.is_some());
    assert!(result.errors.iter().any(|line| line.starts_with("Couldn't bring the contacts down from the server")), "{:?}", result.errors);
    // And when the state was being asked about.
    let server = self::server();
    let state = run(&server, &request()).contacts_state.unwrap();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/query", crate::standin::Effect::MethodError("serverFail"))));
    let result = run(&server, &holding(&state, 60));
    assert!(result.contacts.is_none() && result.contacts_state.is_none() && !result.errors.is_empty());
}

#[test]
fn the_query_asks_only_what_the_facade_supports() {
    // The stand-in refuses a sort, an anchor and any filter but one book's, as such a server does: none was refused.
    let server = server();
    let result = run(&server, &request());
    assert!(result.errors.is_empty() && contacts(&result).complete, "{:?}", result.errors);
    let answers: Vec<u16> = server.seen().iter().filter(|s| s.path == "/jmap/api").map(|s| s.status).collect();
    assert!(!answers.is_empty() && answers.iter().all(|status| *status == 200));
    for asked in server.asked().iter().filter(|a| a.method == "ContactCard/query") {
        assert!(asked.limit.is_some_and(|limit| limit <= READ_CARDS));
    }
    // And the server is as strict as that.
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let account = crate::standin::ACCOUNT;
    for (args, kind) in [
        (json!({"accountId": account, "sort": [{"property": "name"}]}), "unsupportedSort"),
        (json!({"accountId": account, "anchor": "Bob Lindqvist"}), "invalidArguments"),
        (json!({"accountId": account, "filter": {"text": "Bob"}}), "unsupportedFilter"),
        (json!({"accountId": account, "filter": {"inAddressBook": "elsewhere"}}), "unsupportedFilter"),
    ] {
        let mut answers = block_on(client.call(&USING, vec![("ContactCard/query", args)])).unwrap();
        assert!(matches!(answers.take(0), Err(Error::Refused { kind: refused, .. }) if refused == kind), "{kind}");
    }
    let mut one_book =
        block_on(client.call(&USING, vec![("ContactCard/query", json!({"accountId": account, "filter": {"inAddressBook": "partners"}}))])).unwrap();
    assert_eq!(one_book.take(0).unwrap()["ids"], json!(["Carol Duval"]));
}

#[test]
fn notes_are_not_asked_for() {
    let server = server();
    run(&server, &request());
    let gets: Vec<Vec<String>> = server.asked().into_iter().filter(|a| a.method == "ContactCard/get").map(|a| a.properties).collect();
    assert!(!gets.is_empty());
    for properties in gets {
        assert!(!properties.iter().any(|p| p == "notes"), "{properties:?}");
        assert!(properties.iter().any(|p| p == "name") && properties.iter().any(|p| p == "emails"));
    }
}

#[test]
fn a_job_that_does_not_ask_for_the_contacts_reads_none() {
    let server = server();
    let result = run(&server, &JmapPimRequest { contacts: false, ..request() });
    assert!(result.books.is_none() && result.contacts.is_none());
    assert!(!server.calls().iter().any(|c| c.starts_with("AddressBook") || c.starts_with("ContactCard")));
    // Nor a session that offers none.
    server.with(|d| d.pim.contacts = false);
    let result = run(&server, &request());
    assert_eq!(result.offers, Some(PimOffers { calendars: true, contacts: false }));
    assert!(result.books.is_none() && result.contacts.is_none() && result.errors.is_empty());
}

#[test]
fn trouble_ends_the_job_as_for_the_calendars() {
    // The credentials refused while reading the cards.
    let server = server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    server.with(|d| d.drop_token_after = Some(0));
    let result = block_on(sync_pim(&client, &request(), &AtomicBool::new(false)));
    assert!(result.auth_refused && result.contacts.is_none(), "{result:?}");
    // A busy server, for the time it says.
    let server = self::server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    server.with(|d| d.refuse_next.push_back(Some((429, Some(90)))));
    let result = block_on(sync_pim(&client, &request(), &AtomicBool::new(false)));
    assert_eq!(result.busy_for, Some(90));
    assert!(result.contacts.is_none() && result.contacts_state.is_none());
    // Stopped: nothing is asked.
    let server = self::server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let result = block_on(sync_pim(&client, &request(), &AtomicBool::new(true)));
    assert!(result.stopped && result.books.is_none() && server.api_requests() == 0);
    // A calendar part that the server refuses outright ends the job before the cards.
    let server = self::server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    server.with(|d| d.refuse_next.push_back(Some((503, None))));
    let with_calendar = JmapPimRequest { lists: true, ..request() };
    let result = block_on(sync_pim(&client, &with_calendar, &AtomicBool::new(false)));
    assert_eq!(result.busy_for, Some(30));
    assert!(!server.calls().iter().any(|c| c.starts_with("ContactCard")));
}

#[test]
fn the_lines_name_no_address() {
    let server = server();
    server.with(|d| d.faults.push_back(Fault::on("ContactCard/query", crate::standin::Effect::MethodError("serverFail"))));
    let result = run(&server, &JmapPimRequest { session: format!("{SESSION}?account={ME}"), ..request() });
    assert!(!result.errors.is_empty());
    for line in &result.errors {
        assert!(!line.contains('@') && !line.contains("example.org") && !line.contains("Okafor"), "{line}");
    }
}

#[test]
fn a_server_that_takes_one_call_a_request_is_asked_in_turn() {
    let server = server();
    server.with(|d| d.max_calls = 1);
    let result = run(&server, &request());
    assert!(result.errors.is_empty() && contacts(&result).complete, "{:?}", result.errors);
    assert_eq!(server.api_requests(), 3, "the books, the query, the cards");
    assert_eq!(result.books.as_ref().map(|b| b.books.len()), Some(2));
    // And the state alone, in two.
    let state = result.contacts_state.unwrap();
    let before = server.api_requests();
    let again = run(&server, &holding(&state, 60));
    assert!(again.contacts.is_none() && again.contacts_state.as_deref() == Some(state.as_str()));
    assert_eq!(server.api_requests() - before, 2);
}
