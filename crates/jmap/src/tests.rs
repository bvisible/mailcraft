// //// Neoffice — added file (no upstream equivalent)
//! The client over scripted answers: `connect` and `call`, and what they make of a hostile server.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use proptest::prelude::*;
use serde_json::json;

use super::*;

const SESSION: &str = "https://jmap.example.org/jmap/session?account=ada%40example.org";
const TOKEN: &str = "tk-3f9a7c";

/// An `Http` that answers what it is told to, in order, and keeps what it was sent.
#[derive(Clone, Default)]
struct Script {
    replies: Rc<RefCell<VecDeque<Result<Response>>>>,
    sent: Rc<RefCell<Vec<Request>>>,
}

impl Script {
    fn new(replies: Vec<Result<Response>>) -> Script {
        Script { replies: Rc::new(RefCell::new(replies.into())), sent: Rc::default() }
    }

    fn sent(&self) -> Vec<Request> {
        self.sent.borrow().clone()
    }
}

impl Http for Script {
    async fn send(&self, req: Request) -> Result<Response> {
        self.sent.borrow_mut().push(req);
        self.replies.borrow_mut().pop_front().unwrap_or_else(|| Err(Error::Io("nothing was scripted".into())))
    }
}

fn reply(status: u16, body: &Value) -> Result<Response> {
    Ok(Response { status, body: serde_json::to_vec(body).unwrap(), retry_after: None })
}

fn status(status: u16, retry_after: Option<u32>) -> Result<Response> {
    Ok(Response { status, body: Vec::new(), retry_after })
}

/// A session as a server of the standard kind has it, with every kind of address.
fn session_doc() -> Value {
    json!({
        "capabilities": {CORE: {"maxCallsInRequest": 16, "maxObjectsInGet": 500}, MAIL: {}, "urn:ietf:params:jmap:submission": {}},
        "accounts": {"A1": {"name": "ada@example.org"}},
        "primaryAccounts": {MAIL: "A1"},
        "apiUrl": "https://jmap.example.org/jmap/api",
        "downloadUrl": "/jmap/download/{accountId}/{blobId}/{name}?type={type}",
        "uploadUrl": "//jmap.example.org/jmap/upload/{accountId}/",
        "state": "1",
    })
}

fn connect_with(address: &str, doc: &Value) -> (Result<Client<Script>>, Script) {
    let script = Script::new(vec![reply(200, doc)]);
    (block_on(Client::connect(script.clone(), address, &Auth::bearer(TOKEN))), script)
}

fn connect_to(doc: &Value) -> Result<Client<Script>> {
    connect_with(SESSION, doc).0
}

/// A connected client, and the script it speaks over; `replies` follow the session's.
fn client(replies: Vec<Result<Response>>) -> (Client<Script>, Script) {
    let mut all = vec![reply(200, &session_doc())];
    all.extend(replies);
    let script = Script::new(all);
    (block_on(Client::connect(script.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap(), script)
}

fn answers_to(responses: Value) -> Result<Response> {
    reply(200, &json!({"methodResponses": responses, "sessionState": "1"}))
}

fn mutated(change: impl FnOnce(&mut Value)) -> Value {
    let mut doc = session_doc();
    change(&mut doc);
    doc
}

#[test]
fn a_bare_host_gets_the_well_known_path_and_nothing_else_does() {
    for (typed, session) in [
        ("jmap.example.com", "https://jmap.example.com/.well-known/jmap"),
        (" https://api.example.com/ ", "https://api.example.com/.well-known/jmap"),
        ("https://api.example.com/jmap/session", "https://api.example.com/jmap/session"),
        ("https://api.example.com/jmap/", "https://api.example.com/jmap/"),
        ("https://api.example.com?account=a", "https://api.example.com/.well-known/jmap?account=a"),
        ("https://api.example.com/?account=a#f", "https://api.example.com/.well-known/jmap?account=a#f"),
        ("https://api.example.com/s?account=a", "https://api.example.com/s?account=a"),
        ("http://127.0.0.1:8080", "http://127.0.0.1:8080/.well-known/jmap"),
        ("[::1]:8080", "https://[::1]:8080/.well-known/jmap"),
    ] {
        assert_eq!(session_url(typed), session, "{typed}");
    }
}

#[test]
fn ids_are_the_url_safe_alphabet_up_to_512_characters() {
    for good in ["a", "E1", "A-z_9", &"x".repeat(MAX_ID)] {
        assert!(valid_id(good), "{good}");
    }
    for bad in ["", "a b", "a/b", "a=", "é", "../x", "a\n", "a%41", "a.b", &"x".repeat(MAX_ID + 1)] {
        assert!(!valid_id(bad), "{bad}");
    }
}

#[test]
fn credentials_are_checked_and_never_shown() {
    assert_eq!(Auth::basic("me", "pw").header().unwrap(), "Basic bWU6cHc=");
    assert_eq!(Auth::basic("me", "p:w").header().unwrap(), "Basic bWU6cDp3", "a password may hold a colon");
    assert_eq!(Auth::basic("jörg", "pässwörd").header().unwrap(), "Basic asO2cmc6cMOkc3N3w7ZyZA==", "sent as UTF-8");
    assert_eq!(Auth::bearer("abc.DEF-1_2~3+/=").header().unwrap(), "Bearer abc.DEF-1_2~3+/=");
    for bad in [
        Auth::basic("a:b", "pw"),
        Auth::basic("me", "p\r\nX: y"),
        Auth::basic("m\0e", "pw"),
        Auth::basic("me", "p".repeat(MAX_CREDENTIALS)),
        Auth::bearer(""),
        Auth::bearer("a b"),
        Auth::bearer("tok\r\nX: y"),
        Auth::bearer("tökén"),
        Auth::bearer("a".repeat(MAX_CREDENTIALS + 1)),
    ] {
        assert!(matches!(bad.header(), Err(Error::Auth(_))), "{bad:?}");
    }
    let shown = format!("{:?} {:?}", Auth::basic("ada@example.org", "hunter2"), Auth::bearer("s3cr3t-token"));
    for secret in ["ada", "example", "hunter2", "s3cr3t"] {
        assert!(!shown.contains(secret), "{shown}");
    }
    // Nothing is sent with credentials that can't be.
    let script = Script::new(vec![reply(200, &session_doc())]);
    let refused = block_on(Client::connect(script.clone(), SESSION, &Auth::bearer("a b")));
    assert!(matches!(refused, Err(Error::Auth(_))) && script.sent().is_empty());
}

#[test]
fn connect_signs_in_and_resolves_the_session_against_its_own_address() {
    let (client, script) = client(vec![]);
    let sent = script.sent();
    let first = sent.first().unwrap();
    assert_eq!((first.method.as_str(), first.url.as_str(), first.max), ("GET", SESSION, MAX_SESSION));
    assert!(first.headers.contains(&("Authorization".into(), "Bearer tk-3f9a7c".into())));
    assert!(first.headers.contains(&("Accept".into(), "application/json".into())));
    assert_eq!(client.account_id(), "A1");
    assert!(client.has_capability(MAIL) && client.has_capability("urn:ietf:params:jmap:submission") && !client.has_capability("urn:example:x"));
    assert_eq!((client.max_objects_in_get(), client.max_calls_in_request()), (500, 16));
    // A template of the session is filled in with the account, and whatever is put in it is encoded.
    assert_eq!(
        client.download_url("B1", "message/rfc822", "message.eml").unwrap(),
        "https://jmap.example.org/jmap/download/A1/B1/message.eml?type=message%2Frfc822"
    );
    assert_eq!(
        client.download_url("B-1_x", "a b", "n/a?x#y&z={name}").unwrap(),
        "https://jmap.example.org/jmap/download/A1/B-1_x/n%2Fa%3Fx%23y%26z%3D%7Bname%7D?type=a%20b"
    );
    assert_eq!(client.upload_url().unwrap(), "https://jmap.example.org/jmap/upload/A1/");
    for bad in ["", "../x", "a/b", "B1?x", &"b".repeat(MAX_ID + 1)] {
        assert!(matches!(client.download_url(bad, "t", "n"), Err(Error::Protocol(_))), "{bad}");
    }
}

#[test]
fn a_bare_host_is_asked_for_the_well_known_session() {
    let script = Script::new(vec![reply(200, &session_doc())]);
    block_on(Client::connect(script.clone(), "jmap.example.org", &Auth::bearer(TOKEN))).unwrap();
    assert_eq!(script.sent().first().map(|r| r.url.as_str()), Some("https://jmap.example.org/.well-known/jmap"));
}

#[test]
fn connect_refuses_a_session_that_cannot_be_used() {
    let cases = [
        ("no mail", mutated(|d| drop(d["capabilities"].as_object_mut().unwrap().remove(MAIL)))),
        ("no capabilities", mutated(|d| drop(d.as_object_mut().unwrap().remove("capabilities")))),
        ("capabilities a list", mutated(|d| d["capabilities"] = json!([MAIL]))),
        ("no primary accounts", mutated(|d| drop(d.as_object_mut().unwrap().remove("primaryAccounts")))),
        ("no mail account", mutated(|d| d["primaryAccounts"] = json!({"urn:example:x": "A1"}))),
        ("an account that is no id", mutated(|d| d["primaryAccounts"][MAIL] = json!("A 1"))),
        ("an empty account id", mutated(|d| d["primaryAccounts"][MAIL] = json!(""))),
        ("a long account id", mutated(|d| d["primaryAccounts"][MAIL] = json!("a".repeat(MAX_ID + 1)))),
        ("a number for an account", mutated(|d| d["primaryAccounts"][MAIL] = json!(7))),
        ("no apiUrl", mutated(|d| drop(d.as_object_mut().unwrap().remove("apiUrl")))),
        ("no downloadUrl", mutated(|d| drop(d.as_object_mut().unwrap().remove("downloadUrl")))),
        ("no uploadUrl", mutated(|d| drop(d.as_object_mut().unwrap().remove("uploadUrl")))),
        ("a number for an address", mutated(|d| d["apiUrl"] = json!(7))),
        ("a template for the API", mutated(|d| d["apiUrl"] = json!("/api/{accountId}"))),
        ("another scheme", mutated(|d| d["apiUrl"] = json!("ftp://jmap.example.org/api"))),
        ("a user name in an address", mutated(|d| d["apiUrl"] = json!("https://u:p@jmap.example.org/api"))),
        ("a space in an address", mutated(|d| d["uploadUrl"] = json!("/up load/{accountId}"))),
        ("a line break in an address", mutated(|d| d["downloadUrl"] = json!("/d/{blobId}\r\nX-Evil: 1"))),
        ("an address that is too long", mutated(|d| d["downloadUrl"] = json!(format!("/{}", "d".repeat(MAX_TEMPLATE))))),
        ("a list", json!([])),
        ("a string", json!("session")),
        ("null", Value::Null),
    ];
    for (what, doc) in cases {
        let refused = connect_to(&doc);
        assert!(matches!(refused, Err(Error::Protocol(_))), "{what}: {refused:?}");
    }
    // Not even JSON.
    let script = Script::new(vec![Ok(Response { status: 200, body: b"<html>".to_vec(), retry_after: None })]);
    assert!(matches!(block_on(Client::connect(script, SESSION, &Auth::bearer(TOKEN))), Err(Error::Protocol(_))));
}

#[test]
fn connect_never_goes_from_https_to_http() {
    for key in ["apiUrl", "downloadUrl", "uploadUrl"] {
        for bad in ["http://jmap.example.org/x", "http://127.0.0.1/x", "//jmap.example.org/x"] {
            let doc = mutated(|d| d[key] = json!(bad));
            let result = connect_to(&doc);
            // (A scheme-relative address takes the session's scheme: it is fine.)
            assert_eq!(matches!(result, Err(Error::Tls(_))), bad.starts_with("http:"), "{key} {bad}: {result:?}");
        }
    }
    // Nor does a plain http session reach anywhere but this computer, and nothing is sent to find out.
    let script = Script::new(vec![reply(200, &session_doc())]);
    for bad in ["http://jmap.example.org/jmap/session", "http://10.0.0.1/s", "http://127.0.0.1.example.org/s"] {
        let result = block_on(Client::connect(script.clone(), bad, &Auth::bearer(TOKEN)));
        assert!(matches!(result, Err(Error::Tls(_))), "{bad}");
    }
    assert!(script.sent().is_empty());
}

#[test]
fn a_plain_http_session_is_for_this_computer_alone() {
    let doc = mutated(|d| {
        d["apiUrl"] = json!("/jmap/api");
        d["downloadUrl"] = json!("/jmap/d/{blobId}");
        d["uploadUrl"] = json!("https://elsewhere.example/up/{accountId}");
    });
    let script = Script::new(vec![reply(200, &doc), answers_to(json!([["Mailbox/get", {}, "c0"]]))]);
    let client = block_on(Client::connect(script.clone(), "http://127.0.0.1:8080/jmap/session", &Auth::bearer(TOKEN))).unwrap();
    block_on(client.mail(vec![("Mailbox/get", json!({}))])).unwrap();
    assert_eq!(script.sent().get(1).map(|r| r.url.as_str()), Some("http://127.0.0.1:8080/jmap/api"));
    assert_eq!(client.upload_url().unwrap(), "https://elsewhere.example/up/A1");
    // It may not hand the credentials on to another plain http address.
    for bad in ["http://jmap.example.org/api", "http://10.0.0.1/api", "http://127.0.0.1.example.org/api"] {
        let doc = mutated(|d| d["apiUrl"] = json!(bad));
        let script = Script::new(vec![reply(200, &doc)]);
        let result = block_on(Client::connect(script, "http://localhost:8080/jmap/session", &Auth::bearer(TOKEN)));
        assert!(matches!(result, Err(Error::Tls(_))), "{bad}");
    }
}

#[test]
fn connect_clamps_what_the_server_says_it_takes() {
    let limits = |core: Value| connect_to(&mutated(|d| d["capabilities"][CORE] = core)).unwrap();
    for (said, objects, calls) in [
        (json!({"maxObjectsInGet": 100, "maxCallsInRequest": 8}), 100, 8),
        (json!({"maxObjectsInGet": 1000, "maxCallsInRequest": 64}), 1000, 64),
        (json!({"maxObjectsInGet": 10_000_000, "maxCallsInRequest": 100_000}), 1000, 64),
        (json!({"maxObjectsInGet": u64::MAX, "maxCallsInRequest": u64::MAX}), 1000, 64),
        (json!({"maxObjectsInGet": 0, "maxCallsInRequest": 0}), 1, 1),
        (json!({}), 500, 16),
        (json!({"maxObjectsInGet": "many", "maxCallsInRequest": -3}), 500, 16),
        (json!({"maxObjectsInGet": null, "maxCallsInRequest": 2.5}), 500, 16),
        (json!({"maxObjectsInGet": 1e30, "maxCallsInRequest": [1]}), 500, 16),
        (json!("core"), 500, 16),
    ] {
        let client = limits(said.clone());
        assert_eq!((client.max_objects_in_get(), client.max_calls_in_request()), (objects, calls), "{said}");
    }
    // A session without the core capability at all is held to what the standard says every server takes.
    let client = connect_to(&mutated(|d| drop(d["capabilities"].as_object_mut().unwrap().remove(CORE)))).unwrap();
    assert_eq!((client.max_objects_in_get(), client.max_calls_in_request()), (500, 16));
}

#[test]
fn connect_remembers_every_capability_the_session_names() {
    let mut capabilities: serde_json::Map<String, Value> = (0..1000).map(|n| (format!("urn:example:cap{n}"), json!({}))).collect();
    capabilities.insert(MAIL.into(), json!({}));
    let client = connect_to(&mutated(|d| d["capabilities"] = Value::Object(capabilities))).unwrap();
    assert!(client.has_capability(MAIL) && client.has_capability("urn:example:cap999") && !client.has_capability(CORE));
}

#[test]
fn http_failures_become_the_error_the_caller_acts_on() {
    let problem = |status: u16, body: &Value| reply(status, body);
    let cases: Vec<(Result<Response>, Error)> = vec![
        (status(401, None), Error::Auth("the server refused the credentials".into())),
        (status(403, None), Error::Auth("the server refused the credentials".into())),
        (status(429, Some(120)), Error::Busy { status: 429, retry_after: Some(120) }),
        (status(503, None), Error::Busy { status: 503, retry_after: None }),
        (status(503, Some(0)), Error::Busy { status: 503, retry_after: Some(1) }),
        (status(429, Some(u32::MAX)), Error::Busy { status: 429, retry_after: Some(3600) }),
        (status(500, None), Error::Bad("HTTP 500".into())),
        (status(302, None), Error::Bad("HTTP 302".into())),
        (status(0, None), Error::Bad("HTTP 0".into())),
        (problem(404, &json!({"type": "about:blank", "detail": "no such\n\tsession"})), Error::Bad("HTTP 404 no such session".into())),
        (
            problem(400, &json!({"type": "urn:ietf:params:jmap:error:notRequest"})),
            Error::Bad("HTTP 400 urn:ietf:params:jmap:error:notRequest".into()),
        ),
        (problem(400, &json!([1, 2])), Error::Bad("HTTP 400".into())),
        (Err(Error::Io("offline".into())), Error::Io("offline".into())),
    ];
    for (reply, expected) in cases {
        // At the session...
        let script = Script::new(vec![reply.clone()]);
        assert_eq!(block_on(Client::connect(script, SESSION, &Auth::bearer(TOKEN))).unwrap_err(), expected);
        // ... and at a request.
        let (client, _) = client(vec![reply]);
        assert_eq!(block_on(client.mail(vec![("Mailbox/get", json!({}))])).unwrap_err(), expected);
    }
    // What a server says in an error is cut to a line of reasonable size.
    let long = reply(400, &json!({"detail": format!("line\n{}", "x".repeat(10_000))})).unwrap();
    let Err(Error::Bad(text)) = check(&long) else { panic!("not an error") };
    assert!(text.chars().count() < 400 && !text.contains('\n'), "{text}");
    // A body that is no problem details at all, or too big to be, is not read for a reason.
    let big = Response { status: 400, body: vec![b'{'; MAX_PROBLEM + 1], retry_after: None };
    assert_eq!(check(&big), Err(Error::Bad("HTTP 400".into())));
}

#[test]
fn an_answer_over_the_size_asked_for_is_refused() {
    let big = |max: usize| Ok(Response { status: 200, body: vec![b' '; max + 1], retry_after: None });
    let script = Script::new(vec![big(MAX_SESSION)]);
    let refused = block_on(Client::connect(script, SESSION, &Auth::bearer(TOKEN)));
    assert!(matches!(refused, Err(Error::TooLarge(_))), "{refused:?}");
    let (client, _) = client(vec![big(MAX_JSON)]);
    let refused = block_on(client.mail(vec![("Mailbox/get", json!({}))]));
    assert!(matches!(refused, Err(Error::TooLarge(_))), "{refused:?}");
    // The size the request itself named is the one held to.
    let script = Script::new(vec![Ok(Response { status: 200, body: vec![b'x'; 11], retry_after: None })]);
    let refused = block_on(send(&script, Request::get("https://jmap.example.org/", 10)));
    assert!(matches!(refused, Err(Error::TooLarge(_))));
    let script = Script::new(vec![Ok(Response { status: 200, body: vec![b'x'; 10], retry_after: None })]);
    assert!(block_on(send(&script, Request::get("https://jmap.example.org/", 10))).is_ok());
}

#[test]
fn call_sends_one_batched_post_and_answers_in_order() {
    let responses = json!([["Email/query", {"ids": ["E2", "E1"]}, "c0"], ["Email/get", {"list": [{"id": "E2"}]}, "c1"]]);
    let (client, script) = client(vec![answers_to(responses)]);
    let calls = vec![
        ("Email/query", json!({"accountId": "A1", "filter": {"inMailbox": "M1"}, "limit": 2})),
        ("Email/get", json!({"accountId": "A1", "#ids": result_of(0, "Email/query", "/ids"), "properties": ["id"]})),
    ];
    let answers = block_on(client.mail(calls)).unwrap();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers.get(0).unwrap(), &json!({"ids": ["E2", "E1"]}));
    assert_eq!(answers.get(1).unwrap(), &json!({"list": [{"id": "E2"}]}));
    assert_eq!(script.sent().len(), 2, "the session, then one request for both calls");
    let post = script.sent().get(1).cloned().unwrap();
    assert_eq!((post.method.as_str(), post.url.as_str(), post.max), ("POST", "https://jmap.example.org/jmap/api", MAX_JSON));
    assert!(post.headers.contains(&("Content-Type".into(), "application/json".into())));
    assert!(post.headers.contains(&("Authorization".into(), "Bearer tk-3f9a7c".into())));
    let body: Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(body["using"], json!([CORE, MAIL]));
    assert_eq!(body["methodCalls"][0][2], "c0");
    assert_eq!(body["methodCalls"][1][0], "Email/get");
    assert_eq!(body["methodCalls"][1][1]["#ids"], json!({"resultOf": "c0", "name": "Email/query", "path": "/ids"}));
    assert_eq!(body["methodCalls"][1][2], "c1");
    assert_eq!(format!("{answers:?}"), "Answers(2 of which 0 failed)");
}

#[test]
fn a_method_error_fails_that_call_alone() {
    let responses = json!([
        ["Mailbox/get", {"list": [{"id": "M1"}]}, "c0"],
        ["error", {"type": "invalidArguments", "description": "bad\nids\u{7}"}, "c1"],
        ["Email/get", {"list": []}, "c2"],
        ["error", {"type": 7}, "c3"]
    ]);
    let (client, _) = client(vec![answers_to(responses)]);
    let calls = ["Mailbox/get", "Email/query", "Email/get", "Email/get"].map(|name| (name, json!({}))).to_vec();
    let answers = block_on(client.mail(calls)).unwrap();
    assert_eq!(answers.get(0).unwrap(), &json!({"list": [{"id": "M1"}]}));
    let refused = Error::Refused { kind: "invalidArguments".into(), description: "bad ids".into() };
    assert_eq!(answers.get(1).unwrap_err(), refused);
    assert_eq!(refused.to_string(), "the server refused: invalidArguments bad ids");
    assert_eq!(answers.get(2).unwrap(), &json!({"list": []}));
    assert_eq!(answers.get(3).unwrap_err(), Error::Refused { kind: String::new(), description: String::new() });
    assert_eq!(format!("{answers:?}"), "Answers(4 of which 2 failed)");
}

#[test]
fn a_missing_or_misnamed_answer_fails_that_call_alone() {
    let responses = json!([
        ["Mailbox/get", {"first": true}, "c0"],
        ["Mailbox/get", {"again": true}, "c0"],
        ["Email/get", {"for": "another call"}, "c1"],
        "not an answer",
        ["Email/get"],
        [null, {}, "c3"]
    ]);
    let (client, _) = client(vec![answers_to(responses)]);
    let calls = ["Mailbox/get", "Email/query", "Email/get", "Email/get"].map(|name| (name, json!({}))).to_vec();
    let answers = block_on(client.mail(calls)).unwrap();
    assert_eq!(answers.get(0).unwrap(), &json!({"first": true}), "the first answer to a call is its answer");
    for (call, name) in [(1, "Email/query"), (2, "Email/get"), (3, "Email/get")] {
        assert_eq!(answers.get(call).unwrap_err(), Error::Protocol(format!("JMAP: no answer to {name}")), "call {call}");
    }
    assert!(matches!(answers.get(4), Err(Error::Protocol(_))), "there is no fifth call");
}

#[test]
fn a_reply_that_is_no_list_of_answers_is_an_error() {
    for body in [json!([]), json!("x"), json!(null), json!({}), json!({"methodResponses": "x"}), json!({"methodResponses": {}}), json!({"type": "x"})]
    {
        let (client, _) = client(vec![reply(200, &body)]);
        let refused = block_on(client.mail(vec![("Mailbox/get", json!({}))]));
        assert!(matches!(refused, Err(Error::Protocol(_))), "{body}: {refused:?}");
    }
    let (client, _) = client(vec![Ok(Response { status: 200, body: b"not json".to_vec(), retry_after: None })]);
    assert!(matches!(block_on(client.mail(vec![("Mailbox/get", json!({}))])), Err(Error::Protocol(_))));
}

#[test]
fn a_request_over_the_calls_the_server_takes_is_not_sent() {
    let doc = mutated(|d| d["capabilities"][CORE]["maxCallsInRequest"] = json!(2));
    let script = Script::new(vec![reply(200, &doc)]);
    let client = block_on(Client::connect(script.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let three = vec![("Mailbox/get", json!({})), ("Mailbox/get", json!({})), ("Mailbox/get", json!({}))];
    assert!(matches!(block_on(client.mail(three)), Err(Error::TooLarge(_))));
    assert_eq!(script.sent().len(), 1, "only the session was asked for");
    // None is no request either.
    assert!(block_on(client.mail(Vec::new())).unwrap().is_empty());
    assert_eq!(script.sent().len(), 1);
}

#[test]
fn what_a_server_echoes_of_the_credentials_never_reaches_an_error() {
    let auth = Auth::basic("ada", "hunter2");
    let header = auth.header().unwrap();
    let encoded = header.strip_prefix("Basic ").unwrap().to_string();
    let echo = |what: &str| format!("{what} {header} {encoded} hunter2");
    let clean = |text: &str| !text.contains("hunter2") && !text.contains(&encoded);

    let script = Script::new(vec![reply(400, &json!({"detail": echo("session"), "type": "hunter2"}))]);
    let shown = block_on(Client::connect(script, SESSION, &auth)).unwrap_err().to_string();
    assert!(shown.contains("***") && clean(&shown), "{shown}");

    let method_error = json!([["error", {"type": "hunter2", "description": echo("method")}, "c0"]]);
    let script = Script::new(vec![
        reply(200, &session_doc()),
        reply(400, &json!({"detail": echo("request")})),
        answers_to(method_error),
        Err(Error::Io(format!("lost {header}"))),
    ]);
    let client = block_on(Client::connect(script, SESSION, &auth)).unwrap();
    let calls = || vec![("Mailbox/get", json!({}))];
    let refused = block_on(client.mail(calls())).unwrap_err().to_string();
    assert!(refused.contains("***") && clean(&refused), "{refused}");
    let answers = block_on(client.mail(calls())).unwrap();
    let failed = answers.get(0).unwrap_err().to_string();
    assert!(failed.contains("***") && clean(&failed), "{failed}");
    let lost = block_on(client.mail(calls())).unwrap_err().to_string();
    assert!(lost.contains("lost ***") && clean(&lost), "{lost}");
}

#[test]
fn debug_output_shows_no_credential_address_or_mail() {
    let request = Request::post("https://jmap.example.org/api?account=ada%40example.org", "application/json", b"Subject: private".to_vec(), 10)
        .header("Authorization", "Bearer s3cr3t");
    let response = Response { status: 200, body: b"Subject: private".to_vec(), retry_after: None };
    let (client, _) = client(vec![]);
    let shown = format!("{request:?} {response:?} {client:?}");
    for secret in ["s3cr3t", "ada", "private", "account=", "/api", "3f9a7c"] {
        assert!(!shown.contains(secret), "{secret}: {shown}");
    }
    assert!(shown.contains("https://jmap.example.org") && shown.contains("Authorization"), "{shown}");
    assert!(format!("{:?}", Request::default()).contains("not a web address"));
}

#[test]
fn a_reference_names_the_call_the_path_and_the_method() {
    assert_eq!(result_of(3, "Email/query", "/ids"), json!({"resultOf": "c3", "name": "Email/query", "path": "/ids"}));
}

#[test]
fn block_on_runs_a_future_that_waits() {
    struct Once(bool);
    impl std::future::Future for Once {
        type Output = u8;
        fn poll(mut self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<u8> {
            if std::mem::replace(&mut self.0, true) { std::task::Poll::Ready(7) } else { std::task::Poll::Pending }
        }
    }
    assert_eq!(block_on(Once(false)), 7);
    assert_eq!(block_on(async { 1 + 1 }), 2);
}

/// Any JSON at all, in the names and values a session and an answer deal in.
pub(crate) fn any_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>().prop_map(|n| serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)),
        "[ -~]{0,12}".prop_map(Value::from),
        proptest::sample::select(vec![
            MAIL,
            CORE,
            "A1",
            "error",
            "c0",
            "c1",
            "Mailbox/get",
            "Email/query",
            "/jmap/api",
            "https://jmap.example.org/x/{accountId}",
            "http://jmap.example.org/x",
            "//other.example/x",
            "https://user:pw@jmap.example.org/",
            "line\r\nbreak",
        ])
        .prop_map(Value::from),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        let name = proptest::sample::select(vec![
            "capabilities",
            "primaryAccounts",
            "apiUrl",
            "maxObjectsInGet",
            "maxCallsInRequest",
            "type",
            "description",
            MAIL,
            CORE,
            "x",
        ]);
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            proptest::collection::vec((name, inner), 0..6)
                .prop_map(|members| Value::Object(members.into_iter().map(|(name, value)| (name.to_string(), value)).collect())),
        ]
    })
}

/// A session with what it takes to be used in some places, and anything at all in the others.
fn session_like() -> impl Strategy<Value = Value> {
    let part = |valid: Value| prop_oneof![3 => Just(valid), 1 => any_json()];
    let capabilities = part(json!({CORE: {"maxCallsInRequest": 16, "maxObjectsInGet": 500}, MAIL: {}}));
    (capabilities, part(json!({MAIL: "A1"})), part(json!("/jmap/api")), part(json!("/d/{blobId}")), part(json!("/u/{accountId}"))).prop_map(
        |(capabilities, accounts, api, download, upload)| {
            json!({"capabilities": capabilities, "primaryAccounts": accounts, "apiUrl": api, "downloadUrl": download, "uploadUrl": upload})
        },
    )
}

/// Answers with what it takes to be read in some places, and anything at all in the others.
fn answers_like() -> impl Strategy<Value = Value> {
    let name = proptest::sample::select(vec!["Mailbox/get", "Email/query", "Email/get", "error", "other"]);
    let id = proptest::sample::select(vec!["c0", "c1", "c2", "c9", "", "x"]);
    proptest::collection::vec((name, any_json(), id), 0..6)
        .prop_map(|answers| json!({"methodResponses": answers.into_iter().map(|(name, args, id)| json!([name, args, id])).collect::<Vec<_>>()}))
}

proptest! {
    #[test]
    fn any_session_is_used_or_refused_and_never_panics(doc in prop_oneof![session_like(), any_json()], noise in proptest::collection::vec(any::<u8>(), 0..64)) {
        for body in [serde_json::to_vec(&doc).unwrap(), noise] {
            let script = Script::new(vec![Ok(Response { status: 200, body, retry_after: None })]);
            if let Ok(client) = block_on(Client::connect(script, SESSION, &Auth::bearer(TOKEN))) {
                prop_assert!((1..=1000).contains(&client.max_objects_in_get()) && (1..=64).contains(&client.max_calls_in_request()));
                prop_assert!(valid_id(client.account_id()));
                for address in [client.upload_url().ok(), client.download_url("B1", "message/rfc822", "m.eml").ok()].into_iter().flatten() {
                    prop_assert!(address.starts_with("https://") && !address.contains(char::is_whitespace) && !address.contains(char::is_control));
                }
            }
        }
    }

    #[test]
    fn any_answer_is_read_or_refused_and_never_panics(answers in prop_oneof![answers_like(), any_json()], noise in proptest::collection::vec(any::<u8>(), 0..64), status in prop_oneof![Just(200u16), any::<u16>()]) {
        for body in [serde_json::to_vec(&answers).unwrap(), noise] {
            let (client, _) = client(vec![Ok(Response { status, body, retry_after: None })]);
            let calls = ["Mailbox/get", "Email/query", "Email/get"].map(|name| (name, json!({}))).to_vec();
            if let Ok(found) = block_on(client.mail(calls)) {
                prop_assert_eq!(found.len(), 3);
                for call in 0..4 {
                    match found.get(call) {
                        Ok(_) => {}
                        Err(Error::Refused { kind, description }) => prop_assert!(kind.chars().count() <= 65 && description.chars().count() <= MAX_ERROR_TEXT + 1),
                        Err(_) => {}
                    }
                }
            }
        }
    }

    #[test]
    fn any_text_is_made_into_an_address_or_refused_and_never_panics(text in ".{0,200}") {
        let address = session_url(&text);
        prop_assert!(address.contains("://"));
        let _ = Url::parse(&address);
        let _ = expand(&text, &[("a", &text), ("accountId", "A1")]);
        let _ = tidy(&text, 20);
        let mut secrets = Secrets::default();
        secrets.add(&text);
        let hidden = secrets.hide(&["a", &text, "b"].join(" "));
        prop_assert!(text.is_empty() || text.contains('*') || !hidden.contains(&text));
        let _ = Auth::basic(text.clone(), text.clone()).header();
        let _ = Auth::bearer(text).header();
    }
}
