// //// Neoffice — added file (no upstream equivalent)
//! The stand-in's sending, held to the standard (RFC 8620 §6, RFC 8621 §4.8, §6 and §7) and, for the server that
//! is a layer in front of an IMAP and an SMTP server, to what it does: what it refuses, and for which reason.

use serde_json::json;

use super::*;
use crate::Http as _;
use crate::standin::{Profile, SESSION, StandIn, TOKEN};
use crate::{Auth, CORE, Client, MAIL, Response, SUBMISSION, block_on};
use base64::Engine as _;

const ME: &str = "ada@example.org";

fn world(profile: Profile) -> (StandIn, String, String) {
    let server = StandIn::new();
    let (sent, drafts) = server.with(|d| {
        d.profile = profile;
        let sent = d.mailbox("Sent", None, Some("sent"));
        let drafts = d.mailbox("Drafts", None, Some("drafts"));
        d.mailbox("Inbox", None, Some("inbox"));
        d.identity("Ada", ME);
        (sent, drafts)
    });
    (server, sent, drafts)
}

fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

fn upload_to(server: &StandIn, path: &str, headers: &[(&str, &str)], body: &[u8]) -> (u16, Value) {
    let mut request = Request::post(&format!("https://jmap.standin.example{path}"), "application/octet-stream", body.to_vec(), 1 << 20);
    request.headers.clear();
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response: Response = block_on(server.send(request)).unwrap();
    (response.status, serde_json::from_slice(&response.body).unwrap_or(Value::Null))
}

/// An upload with the credentials and a type.
fn upload(server: &StandIn, body: &[u8]) -> (u16, Value) {
    upload_to(server, "/jmap/upload/A1/", &[("Authorization", &bearer()), ("Content-Type", "message/rfc822")], body)
}

/// The blob of an upload that was made.
fn blob(server: &StandIn, body: &[u8]) -> String {
    let (status, answer) = upload(server, body);
    assert_eq!(status, 201, "{answer}");
    answer["blobId"].as_str().unwrap().to_string()
}

/// What one method call came to, by its result or the `type` of its error.
fn call(server: &StandIn, using: Value, name: &str, args: Value) -> std::result::Result<Value, String> {
    let body = serde_json::to_vec(&json!({"using": using, "methodCalls": [[name, args, "c0"]]})).unwrap();
    let request = Request::post("https://jmap.standin.example/jmap/api", "application/json", body, 1 << 20).header("Authorization", &bearer());
    let response = block_on(server.send(request)).unwrap();
    let doc: Value = serde_json::from_slice(&response.body).unwrap();
    let answer = doc["methodResponses"][0].clone();
    assert_eq!(response.status, 200, "{doc}");
    if answer[0] == "error" { Err(answer[1]["type"].as_str().unwrap().to_string()) } else { Ok(answer[1].clone()) }
}

fn mail(server: &StandIn, name: &str, args: Value) -> std::result::Result<Value, String> {
    call(server, json!([CORE, MAIL]), name, args)
}

fn sending(server: &StandIn, name: &str, args: Value) -> std::result::Result<Value, String> {
    call(server, json!([CORE, MAIL, SUBMISSION]), name, args)
}

/// A message from Ada, as a program writes one.
fn letter(subject: &str, to: &str, extra: &str) -> Vec<u8> {
    format!("From: Ada <{ME}>\r\nTo: {to}\r\n{extra}Subject: {subject}\r\nMessage-ID: <{subject}@example.org>\r\n\r\nHello about {subject}.\r\n")
        .into_bytes()
}

/// File `raw` in `mailbox` through upload and import: the email's id.
fn file(server: &StandIn, mailbox: &str, raw: &[u8]) -> String {
    let blob = blob(server, raw);
    let made = mail(
        server,
        "Email/import",
        json!({"accountId": ACCOUNT, "emails": {"m": {"blobId": blob, "mailboxIds": {mailbox: true}, "keywords": {"$seen": true}}}}),
    )
    .unwrap();
    made["created"]["m"]["id"].as_str().unwrap_or_else(|| panic!("{made}")).to_string()
}

fn submission(identity: &str, email: &str, from: &str, to: &[&str]) -> Value {
    let rcpt: Vec<Value> = to.iter().map(|r| json!({"email": r, "parameters": null})).collect();
    json!({"accountId": ACCOUNT, "create": {"s": {"identityId": identity, "emailId": email, "envelope": {"mailFrom": {"email": from, "parameters": null}, "rcptTo": rcpt}}}})
}

fn identity_of(server: &StandIn) -> String {
    server.with(|d| d.identities[0].id.clone())
}

/// The SetError a submission was refused with, by its type.
fn refused_with(answer: &Value) -> String {
    answer["notCreated"]["s"]["type"].as_str().map_or_else(|| format!("not refused: {answer}"), str::to_string)
}

// -----------------------------------------------------------------------------------------------------------------

#[test]
fn an_upload_is_held_to_the_standard_and_kept_until_a_filing_takes_it() {
    let (server, ..) = world(Profile::standard());
    // Who and how: the credentials, a POST, a type, the account of the address.
    let (status, _) = upload_to(&server, "/jmap/upload/A1/", &[("Content-Type", "message/rfc822")], b"x");
    assert_eq!(status, 401);
    let (status, _) = upload_to(&server, "/jmap/upload/A1/", &[("Authorization", "Bearer nope"), ("Content-Type", "message/rfc822")], b"x");
    assert_eq!(status, 401);
    let (status, _) = upload_to(&server, "/jmap/upload/A1/", &[("Authorization", &bearer())], b"x");
    assert_eq!(status, 400, "an upload says what it is");
    let (status, _) = upload_to(&server, "/jmap/upload/A9/", &[("Authorization", &bearer()), ("Content-Type", "message/rfc822")], b"x");
    assert_eq!(status, 404);
    let request = Request::get("https://jmap.standin.example/jmap/upload/A1/", 1 << 20).header("Authorization", &bearer());
    assert_eq!(block_on(server.send(request)).unwrap().status, 405);
    // As large as the session says, and no larger.
    server.with(|d| d.max_upload = 10);
    let (status, answer) = upload(&server, &[0; 11]);
    assert_eq!((status, answer["limit"].as_str(), answer["type"].as_str()), (413, None, Some("urn:ietf:params:jmap:error:limit")));
    assert_eq!(upload(&server, &[0; 10]).0, 201);
    // The answer names the blob, its type and its size (RFC 8620 §6.1), and the stand-in keeps the octets.
    let (status, answer) = upload(&server, b"abc");
    assert_eq!(status, 201);
    assert_eq!((answer["accountId"].as_str(), answer["type"].as_str(), answer["size"].as_u64()), (Some(ACCOUNT), Some("message/rfc822"), Some(3)));
    let kept = server.with(|d| d.uploads.get(answer["blobId"].as_str().unwrap()).map(|u| u.octets.clone()));
    assert_eq!(kept, Some(b"abc".to_vec()));
    // The session says what it takes, and where.
    server.with(|d| d.max_upload = 12_345);
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    assert_eq!(client.max_size_upload(), 12_345);
}

#[test]
fn a_server_in_front_of_a_mailbox_keeps_ten_uploads_and_sixty_megabytes_at_most() {
    let (server, ..) = world(Profile::by_place());
    for _ in 0..10 {
        assert_eq!(upload(&server, b"x").0, 201);
    }
    let (status, answer) = upload(&server, b"x");
    assert_eq!((status, answer["type"].as_str()), (429, Some("urn:ietf:params:jmap:error:limit")));
    // A standard store has no such limit.
    let (standard, ..) = world(Profile::standard());
    for _ in 0..12 {
        assert_eq!(upload(&standard, b"x").0, 201);
    }
}

#[test]
fn a_filing_makes_an_email_of_an_upload_and_the_upload_is_gone() {
    let (server, sent, drafts) = world(Profile::standard());
    let first = blob(&server, &letter("one", "bob@example.org", ""));
    let args = |blob: &str, mailbox: &str| json!({"accountId": ACCOUNT, "emails": {"m": {"blobId": blob, "mailboxIds": {mailbox: true}, "keywords": {"$seen": true, "$draft": true, "$x-custom": true, "$off": false}}}});
    let answer = mail(&server, "Email/import", args(&first, &drafts)).unwrap();
    let made = &answer["created"]["m"];
    let id = made["id"].as_str().unwrap().to_string();
    assert_eq!((made["blobId"].is_string(), made["threadId"].as_str(), made["size"].as_u64().is_some()), (true, Some(id.as_str()), true));
    // It lies where it was put, with the keywords that are on, and nowhere else.
    let email = server.with(|d| d.email(&id).cloned()).unwrap();
    assert_eq!(email.mailboxes, BTreeSet::from([drafts.clone()]));
    assert_eq!(email.keywords, BTreeSet::from(["$seen".to_string(), "$draft".to_string(), "$x-custom".to_string()]));
    assert!(server.with(|d| d.uploads.is_empty()), "the filing took the upload");
    // The same upload can't be filed twice.
    let again = mail(&server, "Email/import", args(&first, &sent)).unwrap();
    assert_eq!(again["notCreated"]["m"]["type"], "invalidProperties");
    assert_eq!(again["notCreated"]["m"]["properties"], json!(["blobId"]));
    // Each email arrives after the one before, and after what the tests deliver.
    let second = file(&server, &sent, &letter("two", "bob@example.org", ""));
    let third = file(&server, &sent, &letter("three", "bob@example.org", ""));
    let received = |id: &str| server.with(|d| d.email(id).map(|e| e.received.clone())).unwrap();
    assert!(received(&id) < received(&second) && received(&second) < received(&third));
    assert!(received(&id).as_str() > "2026-11-30T00:00:00Z");
    // A date the client gives is the date.
    let blob = blob(&server, &letter("dated", "bob@example.org", ""));
    let dated = mail(
        &server,
        "Email/import",
        json!({"accountId": ACCOUNT, "emails": {"m": {"blobId": blob, "mailboxIds": {sent.clone(): true}, "receivedAt": "2026-10-01T08:00:00Z"}}}),
    )
    .unwrap();
    assert_eq!(received(dated["created"]["m"]["id"].as_str().unwrap()), "2026-10-01T08:00:00Z");
}

#[test]
fn a_filing_that_is_wrong_is_refused_for_the_reason_the_standard_gives() {
    let (server, sent, _) = world(Profile::standard());
    let blob = blob(&server, &letter("one", "bob@example.org", ""));
    let refused = |emails: Value| {
        let answer = mail(&server, "Email/import", json!({"accountId": ACCOUNT, "emails": emails})).unwrap();
        (
            answer["notCreated"]["m"]["type"].as_str().map(str::to_string),
            answer["notCreated"]["m"]["properties"].clone(),
            answer["created"].as_object().map_or(0, |c| c.len()),
        )
    };
    let cases = vec![
        ("no mailbox", json!({"m": {"blobId": blob, "mailboxIds": {}}}), "mailboxIds"),
        ("a mailbox it does not have", json!({"m": {"blobId": blob, "mailboxIds": {"M999": true}}}), "mailboxIds"),
        ("mailboxes that are no map", json!({"m": {"blobId": blob, "mailboxIds": [sent.clone()]}}), "mailboxIds"),
        ("a mailbox that is off", json!({"m": {"blobId": blob, "mailboxIds": {sent.clone(): false}}}), "mailboxIds"),
        ("no blob", json!({"m": {"mailboxIds": {sent.clone(): true}}}), "blobId"),
        ("a blob that is none", json!({"m": {"blobId": "U 9", "mailboxIds": {sent.clone(): true}}}), "blobId"),
        ("a blob it has not", json!({"m": {"blobId": "U9", "mailboxIds": {sent.clone(): true}}}), "blobId"),
        ("keywords that are no map", json!({"m": {"blobId": blob, "mailboxIds": {sent.clone(): true}, "keywords": ["$seen"]}}), "keywords"),
        ("a date that is none", json!({"m": {"blobId": blob, "mailboxIds": {sent.clone(): true}, "receivedAt": "yesterday"}}), "receivedAt"),
        ("a spec that is no object", json!({"m": 5}), "emails"),
    ];
    for (what, emails, property) in cases {
        assert_eq!(refused(emails), (Some("invalidProperties".to_string()), json!([property]), 0), "{what}");
    }
    // The upload is still there after all of it: a filing that failed took nothing.
    assert_eq!(server.with(|d| d.uploads.len()), 1);
    assert_eq!(mail(&server, "Email/import", json!({"accountId": ACCOUNT, "emails": []})), Err("invalidArguments".to_string()));
    assert_eq!(mail(&server, "Email/import", json!({"accountId": ACCOUNT})), Err("invalidArguments".to_string()));
    assert_eq!(mail(&server, "Email/import", json!({"accountId": ACCOUNT, "emails": {}, "extra": 1})), Err("invalidArguments".to_string()));
    server.with(|d| d.max_get = 1);
    let two = json!({"a": {"blobId": blob, "mailboxIds": {sent.clone(): true}}, "b": {"blobId": blob, "mailboxIds": {sent.clone(): true}}});
    assert_eq!(mail(&server, "Email/import", json!({"accountId": ACCOUNT, "emails": two})), Err("requestTooLarge".to_string()));
}

#[test]
fn a_server_in_front_of_a_mailbox_files_into_one_folder_with_a_few_flags_and_five_at_a_time() {
    let (server, sent, drafts) = world(Profile::by_place());
    let blob = blob(&server, &letter("one", "bob@example.org", ""));
    // One folder to an email.
    let two = json!({"m": {"blobId": blob, "mailboxIds": {sent.clone(): true, drafts.clone(): true}}});
    let answer = mail(&server, "Email/import", json!({"accountId": ACCOUNT, "emails": two})).unwrap();
    assert_eq!(answer["notCreated"]["m"]["type"], "invalidProperties");
    // A blob it has not is a blob not found, as such a server says.
    let lost = json!({"m": {"blobId": "U9", "mailboxIds": {sent.clone(): true}}});
    let answer = mail(&server, "Email/import", json!({"accountId": ACCOUNT, "emails": lost})).unwrap();
    assert_eq!(answer["notCreated"]["m"]["type"], "blobNotFound");
    // Only the flags it knows are kept.
    let flags = json!({"m": {"blobId": blob, "mailboxIds": {drafts.clone(): true}, "keywords": {"$draft": true, "$seen": true, "$x-custom": true, "$Flagged": true}}});
    let answer = mail(&server, "Email/import", json!({"accountId": ACCOUNT, "emails": flags})).unwrap();
    let id = answer["created"]["m"]["id"].as_str().unwrap().to_string();
    let keywords = server.with(|d| d.email(&id).map(|e| e.keywords.clone())).unwrap();
    assert_eq!(keywords, BTreeSet::from(["$seen".to_string(), "$draft".to_string(), "$Flagged".to_string()]));
    // Five emails to a call.
    let many: serde_json::Map<String, Value> =
        (0..6).map(|n| (format!("k{n}"), json!({"blobId": "U9", "mailboxIds": {sent.clone(): true}}))).collect();
    assert_eq!(mail(&server, "Email/import", json!({"accountId": ACCOUNT, "emails": many})), Err("invalidArguments".to_string()));
}

#[test]
fn an_email_is_destroyed_and_one_the_server_does_not_have_is_not_found() {
    let (server, sent, _) = world(Profile::standard());
    let first = file(&server, &sent, &letter("one", "bob@example.org", ""));
    let second = file(&server, &sent, &letter("two", "bob@example.org", ""));
    let answer = mail(&server, "Email/set", json!({"accountId": ACCOUNT, "destroy": [first, "E999", first]})).unwrap();
    assert_eq!(answer["destroyed"], json!([first]));
    assert_eq!(answer["notDestroyed"], json!({"E999": {"type": "notFound"}}));
    assert_eq!(server.with(|d| d.emails.iter().map(|e| e.id.clone()).collect::<Vec<_>>()), std::slice::from_ref(&second));
    // What this stand-in does not do yet is refused.
    assert_eq!(
        mail(&server, "Email/set", json!({"accountId": ACCOUNT, "update": {second.clone(): {"keywords/$seen": true}}})),
        Err("invalidArguments".to_string())
    );
    assert_eq!(mail(&server, "Email/set", json!({"accountId": ACCOUNT, "create": {"k": {}}})), Err("invalidArguments".to_string()));
    assert_eq!(mail(&server, "Email/set", json!({"accountId": ACCOUNT, "destroy": "E1"})), Err("invalidArguments".to_string()));
    assert_eq!(mail(&server, "Email/set", json!({"accountId": ACCOUNT, "destroy": ["a b"]})), Err("invalidArguments".to_string()));
    server.with(|d| d.max_get = 1);
    assert_eq!(mail(&server, "Email/set", json!({"accountId": ACCOUNT, "destroy": ["E1", "E2"]})), Err("requestTooLarge".to_string()));
}

#[test]
fn sending_is_a_capability_of_its_own_and_a_server_may_not_have_it() {
    let (server, ..) = world(Profile::standard());
    let id = identity_of(&server);
    let list = |using: Value| call(&server, using, "Identity/get", json!({"accountId": ACCOUNT, "ids": null}));
    assert_eq!(list(json!([CORE, MAIL])), Err("unknownMethod".to_string()), "the capability is named in the request");
    let answer = list(json!([CORE, MAIL, SUBMISSION])).unwrap();
    assert_eq!(
        answer["list"],
        json!([{"id": id, "name": "Ada", "email": ME, "replyTo": null, "bcc": null, "textSignature": "", "htmlSignature": "", "mayDelete": false}])
    );
    // The ones it is asked for, and the ones it has not.
    let by = call(&server, json!([CORE, MAIL, SUBMISSION]), "Identity/get", json!({"accountId": ACCOUNT, "ids": [id.clone(), "I99"]})).unwrap();
    assert_eq!((by["list"].as_array().map(Vec::len), by["notFound"].clone()), (Some(1), json!(["I99"])));
    // The session offers it, and a server without it neither offers it nor knows it.
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    assert!(client.has_capability(SUBMISSION));
    server.with(|d| d.submission = false);
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    assert!(!client.has_capability(SUBMISSION));
    let body = serde_json::to_vec(&json!({"using": [CORE, MAIL, SUBMISSION], "methodCalls": []})).unwrap();
    let request = Request::post("https://jmap.standin.example/jmap/api", "application/json", body, 1 << 20).header("Authorization", &bearer());
    let response = block_on(server.send(request)).unwrap();
    assert_eq!(response.status, 400, "a capability it does not have");
    assert_eq!(call(&server, json!([CORE, MAIL]), "EmailSubmission/set", json!({"accountId": ACCOUNT})), Err("unknownMethod".to_string()));
}

#[test]
fn a_submission_sends_an_email_to_the_envelopes_recipients_and_no_copy_goes_anywhere() {
    let (server, sent, drafts) = world(Profile::standard());
    let id = identity_of(&server);
    // The message has a Bcc, whose address is in the envelope only for the server's purposes: it is written in
    // the stored copy, as a draft's is, and taken out of what is sent.
    let email =
        file(&server, &sent, &letter("plans", "bob@example.org", "Bcc: Eve <eve@example.org>,\r\n  frank@example.org\r\nCc: carol@example.org\r\n"));
    let answer =
        sending(&server, "EmailSubmission/set", submission(&id, &email, ME, &["bob@example.org", "carol@example.org", "eve@example.org"])).unwrap();
    assert!(answer["created"]["s"]["id"].is_string(), "{answer}");
    let delivered = server.with(|d| d.delivered.clone());
    assert_eq!(delivered.len(), 1);
    assert_eq!((delivered[0].from.as_str(), delivered[0].email.as_str()), (ME, email.as_str()));
    assert_eq!(delivered[0].recipients, ["bob@example.org", "carol@example.org", "eve@example.org"]);
    let left = String::from_utf8(delivered[0].raw.clone()).unwrap();
    assert!(left.contains("Cc: carol@example.org\r\n") && left.contains("Subject: plans") && left.contains("Hello about plans."), "{left}");
    assert!(!left.to_ascii_lowercase().contains("bcc") && !left.contains("eve@") && !left.contains("frank@"), "{left}");
    // The email stays as it was, where it was; nothing was made in Sent or Drafts.
    let stored = server.with(|d| d.email(&email).cloned()).unwrap();
    assert!(String::from_utf8_lossy(&stored.raw).contains("Bcc: Eve"));
    assert_eq!(server.with(|d| (d.emails.len(), d.email(&email).map(|e| e.mailboxes.clone()))), (1, Some(BTreeSet::from([sent.clone()]))));
    assert!(!server.with(|d| d.emails.iter().any(|e| e.mailboxes.contains(&drafts))));
}

#[test]
fn a_submission_that_is_wrong_is_refused_for_the_reason_the_standard_gives() {
    let (server, sent, _) = world(Profile::standard());
    let work = server.with(|d| d.identity("Ada at work", "ada.work@example.org"));
    let id = identity_of(&server);
    let email = file(&server, &sent, &letter("plans", "bob@example.org", ""));
    let refused = |args: Value| refused_with(&sending(&server, "EmailSubmission/set", args).unwrap());
    let cases = vec![
        ("an identity it has not", submission("I99", &email, ME, &["bob@example.org"]), "invalidProperties"),
        ("an email it has not", submission(&id, "E999", ME, &["bob@example.org"]), "invalidProperties"),
        ("a sender that is not the identity's", submission(&id, &email, "eve@example.org", &["bob@example.org"]), "forbiddenMailFrom"),
        ("an identity that is not the message's From", submission(&work, &email, "ada.work@example.org", &["bob@example.org"]), "forbiddenFrom"),
        ("no recipient", submission(&id, &email, ME, &[]), "noRecipients"),
        ("a recipient that is none", submission(&id, &email, ME, &["bob@example.org", "not an address"]), "invalidRecipients"),
    ];
    for (what, args, kind) in cases {
        assert_eq!(refused(args), kind, "{what}");
    }
    // What it says of the recipients it won't have, and of how many it takes.
    let answer = sending(&server, "EmailSubmission/set", submission(&id, &email, ME, &["bob@example.org", "not an address"])).unwrap();
    assert_eq!(answer["notCreated"]["s"]["invalidRecipients"], json!(["not an address"]));
    let many: Vec<String> = (0..101).map(|n| format!("p{n}@example.org")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    let answer = sending(&server, "EmailSubmission/set", submission(&id, &email, ME, &many)).unwrap();
    assert_eq!(
        (answer["notCreated"]["s"]["type"].as_str(), answer["notCreated"]["s"]["maxRecipients"].as_u64()),
        (Some("tooManyRecipients"), Some(100))
    );
    assert!(server.with(|d| d.delivered.is_empty()), "nothing went");
    // A message that is too large to send.
    server.with(|d| d.max_upload = 40_000_000);
    let mut big_one = letter("big", "bob@example.org", "");
    big_one.extend(std::iter::repeat_n(b'x', 26_000_000));
    let big = file(&server, &sent, &big_one);
    assert_eq!(refused(submission(&id, &big, ME, &["bob@example.org"])), "tooLarge");
    // Not what the stand-in does: updates, and removals, and what happens after a submission.
    for key in ["update", "destroy", "onSuccessUpdateEmail", "onSuccessDestroyEmail"] {
        let mut args = submission(&id, &email, ME, &["bob@example.org"]);
        args[key] = json!({});
        assert_eq!(sending(&server, "EmailSubmission/set", args), Err("invalidArguments".to_string()), "{key}");
    }
    assert!(server.with(|d| d.delivered.is_empty()));
}

#[test]
fn without_an_envelope_the_recipients_are_those_of_the_messages_header() {
    let (server, sent, _) = world(Profile::standard());
    let id = identity_of(&server);
    let email =
        file(&server, &sent, &letter("plans", "bob@example.org, Bob again <BOB@example.org>", "Cc: carol@example.org\r\nBcc: eve@example.org\r\n"));
    let args = json!({"accountId": ACCOUNT, "create": {"s": {"identityId": id, "emailId": email}}});
    assert!(sending(&server, "EmailSubmission/set", args).unwrap()["created"]["s"].is_object());
    let delivered = server.with(|d| d.delivered.clone());
    // (RFC 8621 §7: the deduplicated To, Cc and Bcc, and the sender from the From.)
    assert_eq!(delivered[0].recipients, ["bob@example.org", "carol@example.org", "eve@example.org"]);
    assert_eq!(delivered[0].from, ME);
}

#[test]
fn a_server_in_front_of_a_mailbox_sends_a_message_it_filed_once_from_the_mailbox_alone() {
    let (server, sent, _) = world(Profile::by_place());
    let id = identity_of(&server);
    assert_eq!(id, format!("I{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ME)), "named after the address");
    let email = file(&server, &sent, &letter("plans", "bob@example.org", ""));
    let go = |email: &str| sending(&server, "EmailSubmission/set", submission(&id, email, ME, &["bob@example.org"])).unwrap();
    assert!(go(&email)["created"]["s"].is_object());
    // Once: it was just filed, and now it has gone.
    assert_eq!(refused_with(&go(&email)), "invalidEmail");
    // An email it was not told to file is not one it sends.
    let delivered = server.with(|d| d.deliver(&sent, &letter("old", "bob@example.org", ""), &["$seen"], "2026-10-01T08:00:00Z"));
    assert_eq!(refused_with(&go(&delivered)), "invalidEmail");
    // Two to a call, and no more.
    let two = |ids: [&str; 3]| {
        let create: serde_json::Map<String, Value> = ids
            .iter()
            .enumerate()
            .map(|(n, email)| {
                (
                    format!("s{n}"),
                    json!({"identityId": id, "emailId": email, "envelope": {"mailFrom": {"email": ME}, "rcptTo": [{"email": "bob@example.org"}]}}),
                )
            })
            .collect();
        json!({"accountId": ACCOUNT, "create": create})
    };
    assert_eq!(sending(&server, "EmailSubmission/set", two(["E1", "E2", "E3"])), Err("invalidArguments".to_string()));
    // The identity is its own; the sender too.
    let first = file(&server, &sent, &letter("again", "bob@example.org", ""));
    let wrong = sending(&server, "EmailSubmission/set", submission("Iother", &first, ME, &["bob@example.org"])).unwrap();
    assert_eq!(refused_with(&wrong), "forbiddenFrom");
    let wrong = sending(&server, "EmailSubmission/set", submission(&id, &first, "eve@example.org", &["bob@example.org"])).unwrap();
    assert_eq!(refused_with(&wrong), "forbiddenFrom", "such a server says so of the sender as of the identity");
    // A message from anyone else, or from two people, or with a different Sender, is not sent.
    for raw in [
        "From: Eve <eve@example.org>\r\nTo: bob@example.org\r\nSubject: x\r\n\r\ny\r\n".to_string(),
        format!("From: {ME}, eve@example.org\r\nTo: bob@example.org\r\nSubject: x\r\n\r\ny\r\n"),
        format!("From: {ME}\r\nFrom: {ME}\r\nTo: bob@example.org\r\nSubject: x\r\n\r\ny\r\n"),
        format!("From: {ME}\r\nSender: eve@example.org\r\nTo: bob@example.org\r\nSubject: x\r\n\r\ny\r\n"),
    ] {
        let email = file(&server, &sent, raw.as_bytes());
        assert_eq!(refused_with(&go(&email)), "forbiddenFrom", "{raw}");
    }
    // Recipients that are addresses as such a server reads them.
    for bad in ["root@localhost", "bob@example.", "bob@.org", "a b@example.org", "bob@exa(mple).org"] {
        let email = file(&server, &sent, &letter("plans", "bob@example.org", ""));
        let answer = sending(&server, "EmailSubmission/set", submission(&id, &email, ME, &[bad])).unwrap();
        assert_eq!(refused_with(&answer), "invalidRecipients", "{bad}");
    }
    assert_eq!(server.with(|d| d.delivered.len()), 1);
}

#[test]
fn a_server_in_front_of_a_mailbox_limits_how_many_messages_it_sends() {
    let (server, sent, _) = world(Profile::by_place());
    let id = identity_of(&server);
    for n in 0..30 {
        let email = file(&server, &sent, &letter(&format!("m{n}"), "bob@example.org", ""));
        let answer = sending(&server, "EmailSubmission/set", submission(&id, &email, ME, &["bob@example.org"])).unwrap();
        assert!(answer["created"]["s"].is_object(), "{n}: {answer}");
    }
    let email = file(&server, &sent, &letter("one too many", "bob@example.org", ""));
    let answer = sending(&server, "EmailSubmission/set", submission(&id, &email, ME, &["bob@example.org"])).unwrap();
    assert_eq!(refused_with(&answer), "rateLimit");
    assert_eq!(server.with(|d| (d.delivered.len(), d.sent_lately)), (30, 30));
    // The standard server has no such limit.
    let (standard, sent, _) = world(Profile::standard());
    let id = identity_of(&standard);
    for n in 0..35 {
        let email = file(&standard, &sent, &letter(&format!("m{n}"), "bob@example.org", ""));
        assert!(sending(&standard, "EmailSubmission/set", submission(&id, &email, ME, &["bob@example.org"])).unwrap()["created"]["s"].is_object());
    }
}

#[test]
fn the_server_in_front_of_a_mailbox_has_one_identity_whatever_is_asked() {
    let (server, ..) = world(Profile::by_place());
    let id = identity_of(&server);
    let answer = sending(&server, "Identity/get", json!({"accountId": ACCOUNT, "ids": ["Isomeone-else"]})).unwrap();
    assert_eq!((answer["list"][0]["id"].as_str(), answer["notFound"].clone()), (Some(id.as_str()), json!([])));
}

// -----------------------------------------------------------------------------------------------------------------

#[test]
fn a_fault_happens_once_to_the_call_it_was_lined_up_for() {
    let (server, sent, _) = world(Profile::standard());
    let id = identity_of(&server);
    let email = file(&server, &sent, &letter("plans", "bob@example.org", ""));
    let go = || sending(&server, "EmailSubmission/set", submission(&id, &email, ME, &["bob@example.org"]));
    // A creation refused with the SetError a test names; the next goes through, since it is once.
    server.with(|d| d.faults.push_back(Fault::on("EmailSubmission/set", Effect::SetError("forbiddenToSend", "Not today."))));
    let answer = go().unwrap();
    assert_eq!(
        (answer["notCreated"]["s"]["type"].as_str(), answer["notCreated"]["s"]["description"].as_str()),
        (Some("forbiddenToSend"), Some("Not today."))
    );
    assert!(server.with(|d| d.faults.is_empty() && d.delivered.is_empty()));
    // A fault for another method is not this one's.
    server.with(|d| d.faults.push_back(Fault::on("Email/import", Effect::MethodError("serverFail"))));
    assert!(go().unwrap()["created"]["s"].is_object());
    assert_eq!(server.with(|d| d.faults.len()), 1);
    assert_eq!(mail(&server, "Email/import", json!({"accountId": ACCOUNT, "emails": {}})), Err("serverFail".to_string()));
    assert!(server.with(|d| d.faults.is_empty()));
}

#[test]
fn a_fault_can_refuse_the_request_lose_its_answer_or_change_it_after_the_work_is_done() {
    let (server, sent, _) = world(Profile::standard());
    let id = identity_of(&server);
    let send = |email: &str| {
        let body = serde_json::to_vec(&json!({"using": [CORE, MAIL, SUBMISSION], "methodCalls": [["EmailSubmission/set", submission(&id, email, ME, &["bob@example.org"]), "c0"]]})).unwrap();
        let request = Request::post("https://jmap.standin.example/jmap/api", "application/json", body, 1 << 20).header("Authorization", &bearer());
        block_on(server.send(request))
    };
    let fresh = |n: u32| file(&server, &sent, &letter(&format!("m{n}"), "bob@example.org", ""));
    // Refused as a whole: not run.
    server.with(|d| d.faults.push_back(Fault::on("EmailSubmission/set", Effect::RefuseWith(503))));
    assert_eq!(send(&fresh(1)).unwrap().status, 503);
    assert!(server.with(|d| d.delivered.is_empty()));
    // The answer lost: run, and the client hears nothing; the request is seen with the status 0.
    server.with(|d| d.faults.push_back(Fault::on("EmailSubmission/set", Effect::LoseAnswer)));
    assert!(matches!(send(&fresh(2)), Err(Error::Io(_))));
    assert_eq!(server.with(|d| d.delivered.len()), 1);
    assert_eq!(server.seen().last().map(|s| s.status), Some(0));
    // A gateway that gave up after the work, and an answer that is none.
    server.with(|d| d.faults.push_back(Fault::on("EmailSubmission/set", Effect::Status(504))));
    assert_eq!(send(&fresh(3)).unwrap().status, 504);
    server.with(|d| d.faults.push_back(Fault::on("EmailSubmission/set", Effect::Garbage)));
    let garbled = send(&fresh(4)).unwrap();
    assert_eq!(garbled.status, 200);
    assert!(serde_json::from_slice::<Value>(&garbled.body).is_err());
    assert_eq!(server.with(|d| d.delivered.len()), 3, "the work was done for the last three");
    // Uploads have their faults too.
    server.with(|d| d.faults.push_back(Fault::on("upload", Effect::RefuseWith(429))));
    assert_eq!(upload(&server, b"x").0, 429);
    server.with(|d| d.faults.push_back(Fault::on("upload", Effect::LoseAnswer)));
    let request =
        Request::post("https://jmap.standin.example/jmap/upload/A1/", "message/rfc822", b"x".to_vec(), 1 << 20).header("Authorization", &bearer());
    assert!(matches!(block_on(server.send(request)), Err(Error::Io(_))));
    assert_eq!(upload(&server, b"x").0, 201, "once");
}

#[test]
fn a_message_loses_its_bcc_field_and_nothing_else() {
    let raw = b"From: a@example.org\r\nBcc: x@example.org,\r\n\ty@example.org\r\nTo: b@example.org\r\nbCC: z@example.org\r\n Z <z2@example.org>\r\nSubject: Bcc: not a header\r\n\r\nBcc: in the body\r\nmore\r\n";
    let left = String::from_utf8(without_bcc(raw)).unwrap();
    assert_eq!(left, "From: a@example.org\r\nTo: b@example.org\r\nSubject: Bcc: not a header\r\n\r\nBcc: in the body\r\nmore\r\n");
    // A message with no Bcc is as it was, whatever it is.
    let plain = b"Subject: x\n\nbody\n\nBcc: y\n";
    assert_eq!(without_bcc(plain), plain);
    assert_eq!(without_bcc(b""), b"");
    assert_eq!(without_bcc(b"Bcc: x"), b"");
    assert_eq!(header_fields(raw, "to"), ["b@example.org"]);
    assert_eq!(header_fields(raw, "bcc"), ["x@example.org, y@example.org", "z@example.org Z <z2@example.org>"]);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(200))]

    /// Whatever bytes a message is, taking its Bcc out neither panics nor leaves a Bcc field in the header, nor
    /// touches a message that has none.
    #[test]
    fn any_message_is_taken_without_its_bcc_or_left_as_it_is(raw in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)) {
        let left = without_bcc(&raw);
        proptest::prop_assert!(left.len() <= raw.len());
        proptest::prop_assert!(header_fields(&left, "bcc").is_empty());
        if !String::from_utf8_lossy(&raw).to_ascii_lowercase().contains("bcc:") {
            proptest::prop_assert_eq!(&left, &raw);
        }
    }
}
