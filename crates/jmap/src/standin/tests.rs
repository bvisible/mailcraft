// //// Neoffice — added file (no upstream equivalent)
//! The stand-in held to the standard, and a client held to the stand-in.

use proptest::prelude::*;
use serde_json::json;

use super::*;
use crate::{Auth, Client, block_on, result_of};

fn mail(subject: &str) -> Vec<u8> {
    format!("From: Zoe Park <zoe@example.org>\r\nTo: {USER}\r\nSubject: {subject}\r\nMessage-ID: <{subject}@example.org>\r\n\r\nHello about {subject}.\r\n")
        .into_bytes()
}

/// A stand-in with an Inbox of three emails (two of them unread, the third flagged) and an empty Sent.
fn filled() -> (StandIn, Ids) {
    let stand_in = StandIn::new();
    let ids = stand_in.with(|d| {
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        let sent = d.mailbox("Sent", None, Some("sent"));
        let first = d.deliver(&inbox, &mail("first"), &["$seen"], "2026-10-14T09:00:00Z");
        let second = d.deliver(&inbox, &mail("second"), &[], "2026-10-15T06:00:00Z");
        let third = d.deliver(&inbox, &mail("third"), &["$flagged"], "2026-10-15T07:30:00Z");
        Ids { inbox, sent, first, second, third }
    });
    (stand_in, ids)
}

struct Ids {
    inbox: String,
    sent: String,
    first: String,
    second: String,
    third: String,
}

fn connect(stand_in: &StandIn) -> Client<StandIn> {
    block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap()
}

fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

/// A request to the API, signed in with the token, and what it was answered.
fn post(stand_in: &StandIn, headers: &[(&str, &str)], body: &[u8]) -> Response {
    let mut req = Request::post("https://jmap.standin.example/jmap/api", "application/json", body.to_vec(), 1 << 20);
    for (name, value) in headers {
        req = req.header(name, value);
    }
    block_on(stand_in.send(req)).unwrap()
}

/// Method calls as the API has them, and the answer as JSON.
fn raw(stand_in: &StandIn, using: Value, calls: Value) -> (u16, Value) {
    let body = serde_json::to_vec(&json!({"using": using, "methodCalls": calls})).unwrap();
    let resp = post(stand_in, &[("Authorization", &bearer())], &body);
    (resp.status, serde_json::from_slice(&resp.body).unwrap_or(Value::Null))
}

/// What a method call came to: its result, or the `type` of its error.
type Outcome = std::result::Result<Value, String>;

/// The answer to the one call of a request.
fn one(stand_in: &StandIn, call: Value) -> Outcome {
    let (status, body) = raw(stand_in, json!([CORE, MAIL]), json!([call]));
    assert_eq!(status, 200, "{body}");
    let answer = body["methodResponses"][0].clone();
    if answer[0] == "error" { Err(answer[1]["type"].as_str().unwrap().to_string()) } else { Ok(answer[1].clone()) }
}

#[test]
fn a_client_connects_with_either_kind_of_sign_in() {
    let (stand_in, _) = filled();
    for auth in [Auth::bearer(TOKEN), Auth::basic(USER, PASSWORD)] {
        let client = block_on(Client::connect(stand_in.clone(), SESSION, &auth)).unwrap();
        assert_eq!(client.account_id(), ACCOUNT);
        assert_eq!((client.max_objects_in_get(), client.max_calls_in_request()), (500, 16));
        assert!(client.has_capability(MAIL) && client.has_capability(CORE) && !client.has_capability("urn:ietf:params:jmap:submission"));
        assert_eq!(
            client.download_url("B1", "message/rfc822", "m.eml").unwrap(),
            "https://jmap.standin.example/jmap/download/A1/B1/m.eml?type=message%2Frfc822"
        );
        assert_eq!(client.upload_url().unwrap(), "https://jmap.standin.example/jmap/upload/A1/");
    }
    let seen: Vec<(String, String, u16)> = stand_in.seen().into_iter().map(|s| (s.method, s.path, s.status)).collect();
    assert_eq!(seen, vec![("GET".to_string(), "/jmap/session".to_string(), 200); 2]);
    // The limits are the ones it was told to state.
    stand_in.with(|d| (d.max_calls, d.max_get) = (3, 100));
    let client = connect(&stand_in);
    assert_eq!((client.max_objects_in_get(), client.max_calls_in_request()), (100, 3));
    assert!(format!("{stand_in:?}").contains("emails: 3"));
}

#[test]
fn mailboxes_come_with_their_counts() {
    let (stand_in, ids) = filled();
    stand_in.with(|d| d.deliver(&ids.sent, &mail("draft"), &["$draft"], "2026-10-16T00:00:00Z"));
    let client = connect(&stand_in);
    let get = |args: Value| {
        let answers = block_on(client.mail(vec![("Mailbox/get", args)])).unwrap();
        answers.get(0).cloned()
    };
    let all = get(json!({"accountId": ACCOUNT, "ids": null})).unwrap();
    assert_eq!(
        all["list"],
        json!([
            {"id": ids.inbox, "name": "Inbox", "parentId": null, "role": "inbox", "sortOrder": 0, "totalEmails": 3, "unreadEmails": 2},
            {"id": ids.sent, "name": "Sent", "parentId": null, "role": "sent", "sortOrder": 0, "totalEmails": 1, "unreadEmails": 0},
        ])
    );
    assert_eq!(all["notFound"], json!([]));
    let some = get(json!({"accountId": ACCOUNT, "ids": [ids.sent, "M999", "nope"], "properties": ["totalEmails"]})).unwrap();
    assert_eq!(some["list"], json!([{"id": ids.sent, "totalEmails": 1}]));
    assert_eq!(some["notFound"], json!(["M999", "nope"]));
    // Another client reads a message: the count and the state follow.
    let before = all["state"].clone();
    assert!(stand_in.with(|d| d.set_keyword(&ids.second, "$seen", true)));
    assert!(!stand_in.with(|d| d.set_keyword("E999", "$seen", true)));
    let after = get(json!({"accountId": ACCOUNT, "ids": [ids.inbox], "properties": ["unreadEmails"]})).unwrap();
    assert_eq!(after["list"], json!([{"id": ids.inbox, "unreadEmails": 1}]));
    assert_ne!(after["state"], before);
    assert_eq!(stand_in.with(|d| d.mailbox_named("Sent").map(str::to_string)), Some(ids.sent));
}

#[test]
fn the_newest_are_listed_and_read_in_one_request_by_a_reference() {
    let (stand_in, ids) = filled();
    let client = connect(&stand_in);
    let calls = vec![
        (
            "Email/query",
            json!({"accountId": ACCOUNT, "filter": {"inMailbox": ids.inbox}, "sort": [{"property": "receivedAt", "isAscending": false}], "limit": 2, "calculateTotal": true}),
        ),
        (
            "Email/get",
            json!({"accountId": ACCOUNT, "#ids": result_of(0, "Email/query", "/ids"), "properties": ["id", "keywords", "receivedAt", "size", "preview", "headers"]}),
        ),
    ];
    let answers = block_on(client.mail(calls)).unwrap();
    let query = answers.get(0).unwrap();
    assert_eq!(query["ids"], json!([ids.third, ids.second]));
    assert_eq!((&query["total"], &query["position"], &query["canCalculateChanges"]), (&json!(3), &json!(0), &json!(false)));
    let list = &answers.get(1).unwrap()["list"];
    assert_eq!(list[0]["id"], json!(ids.third));
    assert_eq!(list[0]["keywords"], json!({"$flagged": true}));
    assert_eq!(list[0]["receivedAt"], "2026-10-15T07:30:00Z");
    assert_eq!(list[0]["size"], mail("third").len());
    assert_eq!(list[0]["preview"], "Hello about third.");
    assert_eq!(list[0]["headers"][0], json!({"name": "From", "value": " Zoe Park <zoe@example.org>"}));
    let names: Vec<&str> = list[0]["headers"].as_array().unwrap().iter().map(|h| h["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["From", "To", "Subject", "Message-ID"]);
    assert_eq!(list[0].as_object().unwrap().len(), 6, "only the properties asked for, and the id");
    assert_eq!(list[1]["keywords"], json!({}));
    // That was one request for both calls, after the one for the session.
    let requests = stand_in.seen();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests.last().map(|r| (r.status, r.calls.clone())), Some((200, vec!["Email/query".to_string(), "Email/get".to_string()])));
    assert_eq!(stand_in.calls(), ["Email/query", "Email/get"]);
}

#[test]
fn a_query_pages_sorts_and_counts_as_asked() {
    let (stand_in, ids) = filled();
    // Two received together: in the order they were delivered.
    let fourth = stand_in.with(|d| d.deliver(&ids.inbox, &mail("fourth"), &[], "2026-10-15T07:30:00Z"));
    let query = |extra: Value| {
        let mut args = json!({"accountId": ACCOUNT, "filter": {"inMailbox": ids.inbox}});
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        one(&stand_in, json!(["Email/query", args, "c0"]))
    };
    let ids_of = |result: Outcome| result.unwrap()["ids"].clone();
    assert_eq!(ids_of(query(json!({}))), json!([ids.first, ids.second, ids.third, fourth]), "oldest first unless told");
    assert_eq!(ids_of(query(json!({"sort": [{"property": "receivedAt", "isAscending": true}]}))), json!([ids.first, ids.second, ids.third, fourth]));
    assert_eq!(ids_of(query(json!({"sort": [{"property": "receivedAt", "isAscending": false}]}))), json!([fourth, ids.third, ids.second, ids.first]));
    assert_eq!(ids_of(query(json!({"position": 1, "limit": 2}))), json!([ids.second, ids.third]));
    assert_eq!(ids_of(query(json!({"position": 9}))), json!([]));
    assert_eq!(ids_of(query(json!({"limit": 0}))), json!([]));
    assert!(query(json!({})).unwrap().get("total").is_none(), "the total only when asked");
    assert_eq!(query(json!({"calculateTotal": true, "limit": 1})).unwrap()["total"], 4);
    // No more than 500 at a time.
    stand_in.with(|d| {
        for n in 0..600 {
            d.deliver(&ids.sent, b"Subject: x\r\n\r\n", &[], &format!("2026-01-01T00:{:02}:{:02}Z", n / 60, n % 60));
        }
    });
    let args = json!({"accountId": ACCOUNT, "filter": {"inMailbox": ids.sent}, "limit": 100_000, "calculateTotal": true});
    let found = one(&stand_in, json!(["Email/query", args, "c0"])).unwrap();
    assert_eq!((found["ids"].as_array().map(Vec::len), &found["total"]), (Some(500), &json!(600)));
}

#[test]
fn messages_are_shown_as_a_server_shows_them() {
    let stand_in = StandIn::new();
    let (inbox, folded, lf, bare, attached, wide) = stand_in.with(|d| {
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        let folded = d.deliver(
            &inbox,
            b"Subject: one\r\n two\r\n\tthree\r\nX-Empty:\r\nnot a header\r\n: nameless\r\nSpaced : value\r\n\r\nBody\r\n  with   gaps\r\n",
            &[],
            "2026-01-01T00:00:01Z",
        );
        let lf = d.deliver(&inbox, b"Subject: lf\n\nBody\n", &[], "2026-01-01T00:00:02Z");
        let bare = d.deliver(&inbox, b"Subject: no body", &[], "2026-01-01T00:00:03Z");
        let attached = d.deliver(
            &inbox,
            b"Content-Type: multipart/mixed\r\n\r\nContent-DISPOSITION: Attachment; filename=a.txt\r\n",
            &[],
            "2026-01-01T00:00:04Z",
        );
        let mut long = b"Subject: long\r\n\r\n".to_vec();
        long.extend("é".repeat(1000).bytes());
        long.extend([0xff, 0xfe]);
        let wide = d.deliver(&inbox, &long, &[], "2026-01-01T00:00:05Z");
        (inbox, folded, lf, bare, attached, wide)
    });
    let _ = inbox;
    let get = |id: &str| {
        let args = json!({"accountId": ACCOUNT, "ids": [id], "properties": ["headers", "preview", "hasAttachment", "blobId", "mailboxIds"]});
        one(&stand_in, json!(["Email/get", args, "c0"])).unwrap()["list"][0].clone()
    };
    let email = get(&folded);
    assert_eq!(
        email["headers"],
        json!([{"name": "Subject", "value": " one\r\n two\r\n\tthree"}, {"name": "X-Empty", "value": ""}]),
        "folds are kept; a line with no colon, a field with no name and one with a blank before its colon are none"
    );
    assert_eq!(email["preview"], "Body with gaps");
    assert_eq!(email["hasAttachment"], false);
    assert!(email["blobId"].as_str().is_some_and(valid_id));
    assert_eq!(email["mailboxIds"].as_object().map(|m| m.len()), Some(1));
    assert_eq!((get(&lf)["headers"][0]["value"].clone(), get(&lf)["preview"].clone()), (json!(" lf"), json!("Body")));
    assert_eq!((get(&bare)["preview"].clone(), get(&bare)["headers"].as_array().map(Vec::len)), (json!(""), Some(1)));
    assert_eq!(get(&attached)["hasAttachment"], true);
    assert_eq!(get(&wide)["preview"].as_str().map(|p| p.chars().count()), Some(MAX_PREVIEW));
    // A message with too many header fields gives the first 500.
    let many = stand_in.with(|d| {
        let mailbox = d.mailbox_named("Inbox").map(str::to_string).unwrap();
        let raw: String = (0..MAX_HEADERS + 50).map(|n| format!("X-{n}: v\r\n")).collect();
        d.deliver(&mailbox, raw.as_bytes(), &[], "2026-01-01T00:00:06Z")
    });
    assert_eq!(get(&many)["headers"].as_array().map(Vec::len), Some(MAX_HEADERS));
}

#[test]
fn the_default_properties_leave_out_the_raw_headers() {
    let (stand_in, ids) = filled();
    let email = one(&stand_in, json!(["Email/get", {"accountId": ACCOUNT, "ids": [ids.first]}, "c0"])).unwrap()["list"][0].clone();
    let mut names: Vec<&str> = email.as_object().unwrap().keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["blobId", "hasAttachment", "id", "keywords", "mailboxIds", "preview", "receivedAt", "size"]);
    assert_eq!(email["mailboxIds"], json!({ids.inbox.clone(): true}));
}

#[test]
fn a_call_that_is_wrong_is_refused_for_the_reason_a_server_gives() {
    let (stand_in, ids) = filled();
    let inbox = &ids.inbox;
    let many: Vec<String> = (0..501).map(|n| format!("E{n}")).collect();
    let account = json!(ACCOUNT);
    let cases = vec![
        ("a method it has not", json!(["Foo/bar", {}, "c0"]), "unknownMethod"),
        ("a method of another kind", json!(["Email/set", {"accountId": account}, "c0"]), "unknownMethod"),
        ("no account", json!(["Mailbox/get", {}, "c0"]), "invalidArguments"),
        ("another account", json!(["Mailbox/get", {"accountId": "A2"}, "c0"]), "accountNotFound"),
        ("an account that is no text", json!(["Mailbox/get", {"accountId": 1}, "c0"]), "invalidArguments"),
        ("arguments that are no object", json!(["Mailbox/get", [], "c0"]), "invalidArguments"),
        ("an argument it has not", json!(["Mailbox/get", {"accountId": account, "idz": []}, "c0"]), "invalidArguments"),
        ("ids that are no list", json!(["Mailbox/get", {"accountId": account, "ids": "M1"}, "c0"]), "invalidArguments"),
        ("an id with a space", json!(["Mailbox/get", {"accountId": account, "ids": ["M 1"]}, "c0"]), "invalidArguments"),
        ("an id with a slash", json!(["Mailbox/get", {"accountId": account, "ids": ["a/b"]}, "c0"]), "invalidArguments"),
        ("an id with an accent", json!(["Mailbox/get", {"accountId": account, "ids": ["é"]}, "c0"]), "invalidArguments"),
        ("an empty id", json!(["Mailbox/get", {"accountId": account, "ids": [""]}, "c0"]), "invalidArguments"),
        ("a long id", json!(["Mailbox/get", {"accountId": account, "ids": ["a".repeat(513)]}, "c0"]), "invalidArguments"),
        ("a number for an id", json!(["Mailbox/get", {"accountId": account, "ids": [1]}, "c0"]), "invalidArguments"),
        ("too many ids", json!(["Email/get", {"accountId": account, "ids": many}, "c0"]), "requestTooLarge"),
        ("every email", json!(["Email/get", {"accountId": account, "ids": null}, "c0"]), "requestTooLarge"),
        ("no ids at all", json!(["Email/get", {"accountId": account}, "c0"]), "requestTooLarge"),
        ("a property it has not", json!(["Email/get", {"accountId": account, "ids": [], "properties": ["subject"]}, "c0"]), "invalidArguments"),
        ("properties that are no list", json!(["Email/get", {"accountId": account, "ids": [], "properties": "id"}, "c0"]), "invalidArguments"),
        ("a body to fetch", json!(["Email/get", {"accountId": account, "ids": [], "fetchTextBodyValues": true}, "c0"]), "invalidArguments"),
        ("no filter", json!(["Email/query", {"accountId": account}, "c0"]), "unsupportedFilter"),
        ("a null filter", json!(["Email/query", {"accountId": account, "filter": null}, "c0"]), "unsupportedFilter"),
        ("a filter on text", json!(["Email/query", {"accountId": account, "filter": {"text": "x"}}, "c0"]), "unsupportedFilter"),
        (
            "a filter operator",
            json!(["Email/query", {"accountId": account, "filter": {"operator": "AND", "conditions": []}}, "c0"]),
            "unsupportedFilter",
        ),
        (
            "a filter on more",
            json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox, "hasKeyword": "$seen"}}, "c0"]),
            "unsupportedFilter",
        ),
        ("a filter that is no object", json!(["Email/query", {"accountId": account, "filter": [inbox]}, "c0"]), "unsupportedFilter"),
        ("a mailbox that is none", json!(["Email/query", {"accountId": account, "filter": {"inMailbox": "M999"}}, "c0"]), "invalidArguments"),
        ("a mailbox that is no id", json!(["Email/query", {"accountId": account, "filter": {"inMailbox": "M 1"}}, "c0"]), "invalidArguments"),
        (
            "a sort by size",
            json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "sort": [{"property": "size"}]}, "c0"]),
            "unsupportedSort",
        ),
        (
            "a sort with a collation",
            json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "sort": [{"property": "receivedAt", "collation": "i;octet"}]}, "c0"]),
            "unsupportedSort",
        ),
        (
            "a sort that is no list",
            json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "sort": "receivedAt"}, "c0"]),
            "unsupportedSort",
        ),
        (
            "a comparator that is no object",
            json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "sort": ["receivedAt"]}, "c0"]),
            "unsupportedSort",
        ),
        (
            "an isAscending that is no boolean",
            json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "sort": [{"property": "receivedAt", "isAscending": 1}]}, "c0"]),
            "invalidArguments",
        ),
        (
            "a negative position",
            json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "position": -1}, "c0"]),
            "invalidArguments",
        ),
        (
            "a limit that is text",
            json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "limit": "9"}, "c0"]),
            "invalidArguments",
        ),
        (
            "a total that is text",
            json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "calculateTotal": "yes"}, "c0"]),
            "invalidArguments",
        ),
        ("an anchor", json!(["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "anchor": "E1"}, "c0"]), "invalidArguments"),
    ];
    for (what, call, kind) in cases {
        assert_eq!(one(&stand_in, call), Err(kind.to_string()), "{what}");
    }
    // More mailboxes than a `get` answers: it names them, or it refuses.
    stand_in.with(|d| d.max_get = 1);
    assert_eq!(one(&stand_in, json!(["Mailbox/get", {"accountId": account, "ids": null}, "c0"])), Err("requestTooLarge".to_string()));
    assert_eq!(one(&stand_in, json!(["Mailbox/get", {"accountId": account, "ids": [inbox, ids.sent]}, "c0"])), Err("requestTooLarge".to_string()));
    assert!(one(&stand_in, json!(["Mailbox/get", {"accountId": account, "ids": [inbox]}, "c0"])).is_ok());
}

#[test]
fn a_method_needs_its_capability_to_be_named() {
    let (stand_in, _) = filled();
    let call = json!([["Mailbox/get", {"accountId": ACCOUNT}, "c0"]]);
    let (status, body) = raw(&stand_in, json!([CORE]), call.clone());
    assert_eq!(
        (status, body["methodResponses"][0][0].clone(), body["methodResponses"][0][1]["type"].clone()),
        (200, json!("error"), json!("unknownMethod"))
    );
    assert_eq!(raw(&stand_in, json!([]), call.clone()).1["methodResponses"][0][0], "error");
    for using in [json!([MAIL]), json!([CORE, MAIL]), json!([MAIL, MAIL])] {
        assert_eq!(raw(&stand_in, using, call.clone()).1["methodResponses"][0][0], "Mailbox/get");
    }
}

#[test]
fn a_reference_is_resolved_as_the_standard_has_it() {
    let (stand_in, ids) = filled();
    let account = json!(ACCOUNT);
    let by = |path: &str, name: &str| json!({"resultOf": "c0", "name": name, "path": path});
    // `*` over a list joins what is found at the end of the path.
    let calls = json!([
        ["Mailbox/get", {"accountId": account, "ids": null, "properties": ["name"]}, "c0"],
        ["Mailbox/get", {"accountId": account, "#ids": by("/list/*/id", "Mailbox/get"), "properties": ["role"]}, "c1"],
    ]);
    let (_, body) = raw(&stand_in, json!([CORE, MAIL]), calls);
    assert_eq!(body["methodResponses"][1][1]["list"], json!([{"id": ids.inbox, "role": "inbox"}, {"id": ids.sent, "role": "sent"}]));
    // The path is an RFC 6901 pointer.
    let value = json!({"a/b": {"c~d": [1, 2]}, "list": [{"ids": ["x"]}, {"ids": ["y", "z"]}]});
    assert_eq!(pointer(&value, &["a~1b", "c~0d", "1"]), Some(json!(2)));
    assert_eq!(pointer(&value, &["list", "*", "ids"]), Some(json!(["x", "y", "z"])));
    assert_eq!(pointer(&value, &[]), Some(value.clone()));
    for no in [&["a~1b", "c~0d", "2"][..], &["a~1b", "c~0d", "x"], &["a~1b", "c~0d", "1", "1"], &["nope"], &["list", "*", "nope"]] {
        assert_eq!(pointer(&value, no), None, "{no:?}");
    }
    let broken = [
        ("a name that is not the call's", json!({"#ids": by("/ids", "Email/query")})),
        ("a call that is not there", json!({"#ids": {"resultOf": "c9", "name": "Mailbox/get", "path": "/ids"}})),
        ("a call that comes later", json!({"#ids": {"resultOf": "c1", "name": "Mailbox/get", "path": "/ids"}})),
        ("a path that is none", json!({"#ids": by("/nope", "Mailbox/get")})),
        ("a path with no slash", json!({"#ids": by("list", "Mailbox/get")})),
        ("a path of too many segments", json!({"#ids": by(&"/list".repeat(MAX_POINTER + 1), "Mailbox/get")})),
    ];
    for (what, extra) in broken {
        let mut args = json!({"accountId": account});
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let calls = json!([["Mailbox/get", {"accountId": account}, "c0"], ["Mailbox/get", args, "c1"]]);
        let (_, body) = raw(&stand_in, json!([CORE, MAIL]), calls);
        assert_eq!(body["methodResponses"][1][1]["type"], "invalidResultReference", "{what}: {body}");
    }
    // A reference to a call that failed, a reference that is incomplete, and a value given twice.
    let failed = json!([
        ["Mailbox/get", {"accountId": "A2"}, "c0"],
        ["Mailbox/get", {"accountId": account, "#ids": by("/ids", "Mailbox/get")}, "c1"],
        ["Mailbox/get", {"accountId": account, "#ids": {"resultOf": "c0"}}, "c2"],
        ["Mailbox/get", {"accountId": account, "ids": [], "#ids": by("/ids", "Mailbox/get")}, "c3"],
    ]);
    let (_, body) = raw(&stand_in, json!([CORE, MAIL]), failed);
    let kinds: Vec<&str> = (0..4).map(|n| body["methodResponses"][n][1]["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["accountNotFound", "invalidResultReference", "invalidArguments", "invalidArguments"]);
}

#[test]
fn a_request_that_is_wrong_as_a_whole_is_refused_as_a_whole() {
    let (stand_in, _) = filled();
    let auth = [("Authorization", bearer())];
    let send = |body: &[u8]| {
        let resp = post(&stand_in, &[("Authorization", &auth[0].1)], body);
        (resp.status, serde_json::from_slice::<Value>(&resp.body).unwrap())
    };
    let kind = |body: &[u8]| {
        let (status, doc) = send(body);
        assert_eq!(status, 400);
        (doc["type"].as_str().unwrap().to_string(), doc["status"].clone())
    };
    let problem = |name: &str| (format!("urn:ietf:params:jmap:error:{name}"), json!(400));
    assert_eq!(kind(b"not json"), problem("notJSON"));
    assert_eq!(kind(b""), problem("notJSON"));
    for body in [
        json!({}),
        json!([]),
        json!({"using": [CORE]}),
        json!({"methodCalls": []}),
        json!({"using": "x", "methodCalls": []}),
        json!({"using": [CORE, 1], "methodCalls": []}),
        json!({"using": [CORE], "methodCalls": {}}),
        json!({"using": [CORE], "methodCalls": [["Mailbox/get", {}]]}),
        json!({"using": [CORE], "methodCalls": [[1, {}, "c0"]]}),
        json!({"using": [CORE], "methodCalls": [["Mailbox/get", {}, 0]]}),
        json!({"using": [CORE], "methodCalls": ["Mailbox/get"]}),
    ] {
        assert_eq!(kind(&serde_json::to_vec(&body).unwrap()), problem("notRequest"), "{body}");
    }
    assert_eq!(kind(&serde_json::to_vec(&json!({"using": [CORE, "urn:example:x"], "methodCalls": []})).unwrap()), problem("unknownCapability"));
    assert_eq!(send(&serde_json::to_vec(&json!({"using": [], "methodCalls": []})).unwrap()).1["methodResponses"], json!([]));
    // A request over `maxSizeRequest` is refused unread.
    let mut big = br#"{"using":[],"methodCalls":[],"pad":""#.to_vec();
    big.extend(vec![b'x'; MAX_REQUEST]);
    big.extend(br#""}"#);
    let (status, doc) = send(&big);
    assert_eq!((status, doc["type"].clone(), doc["limit"].clone()), (400, json!("urn:ietf:params:jmap:error:limit"), json!("maxSizeRequest")));
    // 16 calls are taken, a 17th is not; and what is not taken is not run.
    let calls = |n: usize| json!((0..n).map(|i| json!(["Mailbox/get", {"accountId": ACCOUNT}, format!("c{i}")])).collect::<Vec<_>>());
    let (status, doc) = raw(&stand_in, json!([CORE, MAIL]), calls(16));
    assert_eq!((status, doc["methodResponses"].as_array().map(Vec::len)), (200, Some(16)));
    let before = stand_in.calls().len();
    let (status, doc) = raw(&stand_in, json!([CORE, MAIL]), calls(17));
    assert_eq!((status, doc["limit"].clone(), doc["type"].clone()), (400, json!("maxCallsInRequest"), json!("urn:ietf:params:jmap:error:limit")));
    assert_eq!(stand_in.calls().len(), before);
    stand_in.with(|d| d.max_calls = 2);
    assert_eq!(raw(&stand_in, json!([CORE, MAIL]), calls(3)).0, 400);
    assert_eq!(raw(&stand_in, json!([CORE, MAIL]), calls(2)).0, 200);
}

#[test]
fn the_address_the_method_and_the_content_type_are_held_to() {
    let (stand_in, _) = filled();
    let body = serde_json::to_vec(&json!({"using": [CORE, MAIL], "methodCalls": []})).unwrap();
    let status_of = |method: &str, address: &str, headers: &[(&str, &str)]| {
        let mut req = Request { method: method.into(), url: address.into(), body: body.clone(), max: 1 << 20, ..Request::default() };
        req.headers =
            std::iter::once(("Authorization".to_string(), bearer())).chain(headers.iter().map(|(n, v)| (n.to_string(), v.to_string()))).collect();
        block_on(stand_in.send(req)).map(|r| r.status)
    };
    let api = "https://jmap.standin.example/jmap/api";
    let json_type = [("Content-Type", "application/json")];
    assert_eq!(status_of("POST", api, &json_type), Ok(200));
    for ok in ["application/json", "Application/JSON", "application/json; charset=utf-8", " application/json ;charset=UTF-8"] {
        assert_eq!(status_of("POST", api, &[("content-type", ok)]), Ok(200), "{ok}");
    }
    for bad in ["text/plain", "application/jsonx", "application/json-seq", "", "json"] {
        assert_eq!(status_of("POST", api, &[("Content-Type", bad)]), Ok(415), "{bad}");
    }
    assert_eq!(status_of("POST", api, &[]), Ok(415), "no content type at all");
    assert_eq!(status_of("GET", api, &json_type), Ok(405));
    assert_eq!(status_of("POST", "https://jmap.standin.example/jmap/session", &json_type), Ok(405));
    assert_eq!(status_of("GET", "https://jmap.standin.example/jmap/session?account=x", &[]), Ok(200), "the query is not part of the path");
    assert_eq!(status_of("GET", "https://jmap.standin.example/jmap/nothing", &[]), Ok(404));
    assert_eq!(status_of("GET", "https://jmap.standin.example/", &[]), Ok(404));
    // Another place, or no place, is no connection at all.
    for elsewhere in
        ["https://jmap.example.org/jmap/api", "http://jmap.standin.example/jmap/api", "https://jmap.standin.example:8443/jmap/api", "nonsense", ""]
    {
        assert!(matches!(status_of("POST", elsewhere, &json_type), Err(Error::Io(_))), "{elsewhere}");
    }
}

#[test]
fn only_the_credentials_it_knows_are_let_in() {
    let (stand_in, _) = filled();
    let basic = |user: &str, password: &str| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}")));
    let status_with = |authorization: Option<&str>| {
        let mut req = Request::get("https://jmap.standin.example/jmap/session", 1 << 20);
        if let Some(value) = authorization {
            req = req.header("authorization", value);
        }
        block_on(stand_in.send(req)).unwrap().status
    };
    for (credentials, expected) in [
        (None, 401),
        (Some(bearer()), 200),
        (Some(format!("bearer {TOKEN}")), 200),
        (Some(basic(USER, PASSWORD)), 200),
        (Some(basic(USER, &format!("{PASSWORD}:more"))), 401),
        (Some(basic("other@standin.example", PASSWORD)), 401),
        (Some(basic(USER, "wrong")), 401),
        (Some(format!("Bearer {TOKEN}x")), 401),
        (Some(format!("Bearer {}", &TOKEN[1..])), 401),
        (Some("Bearer".to_string()), 401),
        (Some(TOKEN.to_string()), 401),
        (Some("Basic !!!not-base64".to_string()), 401),
        (Some(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode([0xff, b':', 0xfe]))), 401),
        (Some(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(USER))), 401),
        (Some("Digest x".to_string()), 401),
    ] {
        assert_eq!(status_with(credentials.as_deref()), expected, "{credentials:?}");
    }
    // The API wants them too, and a request that was refused ran nothing.
    let refused =
        block_on(stand_in.send(Request::post("https://jmap.standin.example/jmap/api", "application/json", b"{}".to_vec(), 1 << 20))).unwrap();
    assert_eq!(refused.status, 401);
    assert!(stand_in.calls().is_empty());
    // No bearer token held: only Basic.
    stand_in.with(|d| d.token = None);
    assert_eq!(status_with(Some(&bearer())), 401);
    assert_eq!(status_with(Some(&basic(USER, PASSWORD))), 200);
}

#[test]
fn a_token_that_is_no_longer_held_is_refused_with_401() {
    let (stand_in, _) = filled();
    let client = connect(&stand_in);
    stand_in.with(|d| d.token = Some("rotated-token".into()));
    // A client that is connected finds out at its next request, and one that connects at once.
    let refused = block_on(client.mail(vec![("Mailbox/get", json!({"accountId": ACCOUNT}))]));
    assert!(matches!(refused, Err(Error::Auth(_))), "{refused:?}");
    let refused = block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer(TOKEN)));
    assert!(matches!(refused, Err(Error::Auth(_))), "{refused:?}");
    let renewed = block_on(Client::connect(stand_in.clone(), SESSION, &Auth::bearer("rotated-token"))).unwrap();
    assert!(block_on(renewed.mail(vec![("Mailbox/get", json!({"accountId": ACCOUNT}))])).unwrap().get(0).is_ok());
    let statuses: Vec<u16> = stand_in.seen().iter().map(|s| s.status).collect();
    assert_eq!(statuses, [200, 401, 401, 200, 200]);
}

#[test]
fn the_stand_in_is_shared_with_a_thread_that_drives_a_client() {
    fn shared<T: Send + Sync>() {}
    shared::<StandIn>();
    let (stand_in, ids) = filled();
    let (worker, inbox) = (stand_in.clone(), ids.inbox.clone());
    let listed = std::thread::spawn(move || {
        let client = block_on(Client::connect(worker, SESSION, &Auth::bearer(TOKEN))).unwrap();
        let args = json!({"accountId": ACCOUNT, "filter": {"inMailbox": inbox}});
        let answers = block_on(client.mail(vec![("Email/query", args)])).unwrap();
        answers.get(0).unwrap()["ids"].as_array().map(Vec::len)
    })
    .join()
    .unwrap();
    assert_eq!(listed, Some(3));
    assert_eq!(stand_in.calls(), ["Email/query"], "what the thread asked is what this clone saw");
}

#[test]
fn an_answer_over_the_size_asked_for_is_no_answer() {
    let (stand_in, _) = filled();
    let req = Request::get("https://jmap.standin.example/jmap/session", 10).header("Authorization", &bearer());
    assert!(matches!(block_on(stand_in.send(req)), Err(Error::TooLarge(_))));
}

#[test]
fn what_it_remembers_of_the_requests_is_bounded() {
    let (stand_in, _) = filled();
    for _ in 0..MAX_SEEN + 10 {
        let req = Request::get("https://jmap.standin.example/jmap/session", 1 << 20).header("Authorization", &bearer());
        assert_eq!(block_on(stand_in.send(req)).unwrap().status, 200);
    }
    assert_eq!(stand_in.seen().len(), MAX_SEEN);
}

/// Requests with what it takes to run in some places, and anything at all in the others.
fn request_like() -> impl Strategy<Value = Value> {
    let call = (
        proptest::sample::select(vec!["Mailbox/get", "Email/query", "Email/get", "Core/echo", "x"]),
        prop_oneof![
            crate::tests::any_json(),
            proptest::collection::vec(
                (
                    proptest::sample::select(vec!["accountId", "ids", "#ids", "filter", "sort", "position", "limit", "properties", "calculateTotal"]),
                    prop_oneof![
                        crate::tests::any_json(),
                        Just(json!(ACCOUNT)),
                        Just(json!({"inMailbox": "M1"})),
                        Just(json!([{"property": "receivedAt"}])),
                        Just(json!({"resultOf": "c0", "name": "Email/query", "path": "/ids"})),
                        Just(json!({"resultOf": "c0", "name": "Mailbox/get", "path": "/list/*/id"})),
                        Just(json!(["M1", "E1", "E2"])),
                    ],
                ),
                0..6
            )
            .prop_map(|members| Value::Object(members.into_iter().map(|(name, value)| (name.to_string(), value)).collect())),
        ],
        proptest::sample::select(vec!["c0", "c1", "c2", "x"]),
    );
    (prop_oneof![Just(json!([CORE, MAIL])), crate::tests::any_json()], proptest::collection::vec(call, 0..20)).prop_map(|(using, calls)| {
        json!({"using": using, "methodCalls": calls.into_iter().map(|(name, args, id)| json!([name, args, id])).collect::<Vec<_>>()})
    })
}

proptest! {
    #[test]
    fn whatever_it_is_sent_it_answers_and_never_panics(request in prop_oneof![request_like(), crate::tests::any_json()], noise in proptest::collection::vec(any::<u8>(), 0..64)) {
        let (stand_in, _) = filled();
        for body in [serde_json::to_vec(&request).unwrap(), noise] {
            let resp = post(&stand_in, &[("Authorization", &bearer())], &body);
            prop_assert!(matches!(resp.status, 200 | 400), "{}", resp.status);
            let doc: Value = serde_json::from_slice(&resp.body).unwrap();
            if resp.status == 200 {
                let calls = request.get("methodCalls").and_then(Value::as_array).map_or(0, Vec::len);
                prop_assert_eq!(doc["methodResponses"].as_array().map(Vec::len), Some(calls));
            } else {
                prop_assert!(doc["type"].as_str().is_some_and(|t| t.starts_with("urn:ietf:params:jmap:error:")));
            }
        }
    }

    #[test]
    fn whatever_the_headers_say_it_lets_in_the_credentials_or_not(authorization in ".{0,80}", kind in ".{0,40}") {
        let (stand_in, _) = filled();
        let req = Request::post("https://jmap.standin.example/jmap/api", "application/json", b"{}".to_vec(), 1 << 20)
            .header("Authorization", &authorization)
            .header("content-type", &kind);
        let status = block_on(stand_in.send(req)).unwrap().status;
        prop_assert!(matches!(status, 400 | 401 | 415));
    }
}
