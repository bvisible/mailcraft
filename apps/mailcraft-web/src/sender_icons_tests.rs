// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of sender_icons.rs.
//! The icon of a sender's domain, as an ask of the Neoffice server and what its answer gives. Every address and domain is invented.

use std::cell::RefCell;

use mailcraft_jmap::{Auth, Error, Request, Response, block_on};

use super::*;
use crate::credentials::Renewal;

const ME: &str = "ada@example.org";
const SESSION: &str = "https://mail.example.org";
const DOMAIN: &str = "atelier-exemple.example";

#[test]
fn a_domain_is_asked_by_its_name_alone() {
    assert_eq!(arguments("A1", DOMAIN), json!({"accountId": "A1", "domain": DOMAIN}));
}

#[test]
fn an_answer_gives_the_icons_bytes_or_why_there_are_none() {
    let png = b"\x89PNG\r\n\x1a\nthe rest".to_vec();
    let data = base64::engine::general_purpose::STANDARD.encode(&png);
    assert_eq!(read(&json!({"domain": DOMAIN, "type": "image/png", "data": data}), DOMAIN), Ok(png));
    assert!(read(&json!({"domain": DOMAIN, "data": null}), DOMAIN).is_err(), "none found");
    assert!(read(&json!({"domain": "elsewhere.example", "data": data}), DOMAIN).is_err(), "another domain's");
    assert!(read(&json!({"domain": DOMAIN, "data": "not base64 !"}), DOMAIN).is_err());
    assert!(read(&json!({"domain": DOMAIN, "data": ""}), DOMAIN).is_err(), "nothing");
    let large = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_BYTES + 1]);
    assert!(read(&json!({"domain": DOMAIN, "data": large}), DOMAIN).is_err(), "more than an icon");
    for junk in [json!(null), json!([]), json!("x"), json!({"domain": 5, "data": 3})] {
        assert!(read(&junk, DOMAIN).is_err());
    }
}

/// A server that offers the icons (or not) and answers every ask of one with `data`; the asks it was made, noted.
#[derive(Clone)]
struct Server {
    offered: bool,
    data: Value,
    asked: Rc<RefCell<Vec<Value>>>,
}

impl Http for Server {
    async fn send(&self, req: Request) -> mailcraft_jmap::Result<Response> {
        let said = |body: Value| Ok(Response { status: 200, body: serde_json::to_vec(&body).unwrap(), retry_after: None });
        if req.url.ends_with("/.well-known/jmap") {
            let mut capabilities =
                json!({"urn:ietf:params:jmap:core": {"maxObjectsInGet": 500, "maxCallsInRequest": 16}, "urn:ietf:params:jmap:mail": {}});
            if self.offered {
                capabilities[ICONS] = json!({});
            }
            return said(json!({
                "capabilities": capabilities,
                "accounts": {"A1": {"name": ME, "isPersonal": true, "isReadOnly": false, "accountCapabilities": {"urn:ietf:params:jmap:mail": {}}}},
                "primaryAccounts": {"urn:ietf:params:jmap:mail": "A1"},
                "username": ME,
                "apiUrl": "/jmap/api",
                "downloadUrl": "/jmap/download/{accountId}/{blobId}/{name}?type={type}",
                "uploadUrl": "/jmap/upload/{accountId}/",
                "state": "0",
            }));
        }
        let request: Value = serde_json::from_slice(&req.body).map_err(|e| Error::Protocol(e.to_string()))?;
        let call = &request["methodCalls"][0];
        self.asked.borrow_mut().push(call.clone());
        said(json!({"methodResponses": [[call[0], {"domain": call[1]["domain"], "data": self.data}, call[2]]], "sessionState": "0"}))
    }
}

struct Page;

impl Credentials for Page {
    fn auth(&self, _email: &str) -> Option<Auth> {
        Some(Auth::bearer("token"))
    }

    async fn renew(&self, _email: &str) -> Renewal {
        Renewal::Reload
    }
}

#[test]
fn the_icon_is_asked_of_the_server_under_its_capability_and_a_server_without_it_leaves_the_initials() {
    let png = b"\x89PNG\r\n\x1a\nthe rest".to_vec();
    let server = Server { offered: true, data: json!(base64::engine::general_purpose::STANDARD.encode(&png)), asked: Rc::default() };
    let (client, told) = block_on(ask(server.clone(), &Page, ME, SESSION, None, DOMAIN, |_| async {}));
    assert_eq!(told, Ok(png));
    assert!(client.is_some(), "its client kept for the next");
    let asked = server.asked.borrow();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0][0], ICON);
    assert_eq!(asked[0][1], json!({"accountId": "A1", "domain": DOMAIN}), "the domain alone, never an address");
    let older = Server { offered: false, data: Value::Null, asked: Rc::default() };
    let (_, told) = block_on(ask(older.clone(), &Page, ME, SESSION, None, DOMAIN, |_| async {}));
    assert!(told.is_err() && older.asked.borrow().is_empty(), "not asked where it isn't offered");
}
