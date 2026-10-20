// //// Neoffice — added file (no upstream equivalent)
//! The stand-in's calendars and address books, held to what a server that keeps only the basics serves, and a
//! client held to them.

use serde_json::{Value, json};

use super::super::{ACCOUNT, SESSION, StandIn, TOKEN};
use super::*;
use crate::{Auth, CALENDARS, CONTACTS, CORE, Client, Http, MAIL, Request, block_on};

fn connect(stand_in: &StandIn) -> Client<StandIn> {
    block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap()
}

/// Method calls as the API has them, and the HTTP status and body of the answer.
fn raw(stand_in: &StandIn, using: Value, calls: Value) -> (u16, Value) {
    let body = serde_json::to_vec(&json!({"using": using, "methodCalls": calls})).unwrap();
    let req =
        Request::post("https://jmap.standin.example/jmap/api", "application/json", body, 1 << 20).header("Authorization", &format!("Bearer {TOKEN}"));
    let resp = block_on(stand_in.send(req)).unwrap();
    (resp.status, serde_json::from_slice(&resp.body).unwrap_or(Value::Null))
}

#[test]
fn the_stand_in_offers_calendars_and_address_books_only_when_asked() {
    let stand_in = StandIn::new();
    let client = connect(&stand_in);
    assert!(!client.offers_calendars() && !client.offers_contacts(), "a stand-in for mail stays one");
    // A request may not name what the session doesn't offer.
    let (status, body) = raw(&stand_in, json!([CORE, MAIL, CALENDARS]), json!([["Mailbox/get", {"accountId": ACCOUNT}, "c0"]]));
    assert_eq!((status, body["type"].as_str()), (400, Some("urn:ietf:params:jmap:error:unknownCapability")));
    stand_in.with(|d| d.pim = Pim::offered());
    let client = connect(&stand_in);
    assert!(client.offers_calendars() && client.offers_contacts());
    assert_eq!((client.calendars_account(), client.contacts_account()), (Some(ACCOUNT), Some(ACCOUNT)));
    assert_eq!(client.calendars_span(), 500 * 86_400);
    assert_eq!(client.calendars_writable().map(<[String]>::len), Some(WRITABLE_EVENT.len()));
    assert!(client.calendars_writable().is_some_and(|written| !written.iter().any(|p| p == "keywords")), "no categories are kept");
    assert_eq!(client.contacts_writable().map(<[String]>::len), Some(WRITABLE_CARD.len()));
    let (status, _) = raw(&stand_in, json!([CORE, MAIL, CALENDARS, CONTACTS]), json!([["Mailbox/get", {"accountId": ACCOUNT}, "c0"]]));
    assert_eq!(status, 200);
    // A session that doesn't say what it writes, and a stretch it says otherwise.
    stand_in.with(|d| {
        d.pim.says_writable = false;
        d.pim.span = "P90D".into();
    });
    let client = connect(&stand_in);
    assert_eq!((client.calendars_writable(), client.contacts_writable(), client.calendars_span()), (None, None, 90 * 86_400));
}
