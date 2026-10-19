// //// Neoffice — added file (no upstream equivalent)
//! Sending a message and filing a draft, step by step: against the in-process server, of either kind (a
//! store of the standard kind and a server that is a layer in front of an IMAP and an SMTP server), made to
//! fail at each step; against a scripted server for what is asked, word for word; and against a hostile one.
//! Invented people at `example.org`.

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::rc::Rc;

use mailcraft_model::{Address, Body};
use proptest::prelude::*;

use super::*;
use crate::standin::{Effect, Email, Fault, Profile, SESSION, StandIn, TOKEN};
use crate::{Auth, Response, block_on};

const ME: &str = "ada@example.org";
/// 2026-10-14 16:00 UTC: a message that has a date is not dated by the clock.
const DATE: i64 = 1_791_993_600;

/// The mailboxes of an account, on a server of this kind with one identity, Ada's.
struct Ids {
    inbox: String,
    drafts: String,
    sent: String,
}

fn world(profile: Profile) -> (StandIn, Ids) {
    let server = StandIn::new();
    let ids = server.with(|d| {
        d.profile = profile;
        let ids = Ids {
            inbox: d.mailbox("Inbox", None, Some("inbox")),
            drafts: d.mailbox("Drafts", None, Some("drafts")),
            sent: d.mailbox("Sent", None, Some("sent")),
        };
        d.identity("Ada", ME);
        ids
    });
    (server, ids)
}

/// Both kinds of server, each with a name for a failure's message.
fn both() -> [(&'static str, Profile); 2] {
    [("a store of the standard kind", Profile::standard()), ("a server in front of IMAP and SMTP", Profile::by_place())]
}

fn client(server: &StandIn) -> Client<StandIn> {
    block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap()
}

fn outgoing<'a>(client: &'a Client<StandIn>, ids: &'a Ids) -> Outgoing<'a, StandIn> {
    Outgoing::new(client, Some(&ids.sent), Some(&ids.drafts), Secrets::default())
}

fn address(email: &str) -> Address {
    Address::new("Someone", email)
}

/// A message from Ada to these people, dated, with an id.
fn message(subject: &str, to: &[&str]) -> Message {
    Message {
        id: 7,
        from: Address::new("Ada", ME),
        to: to.iter().map(|a| address(a)).collect(),
        subject: subject.into(),
        message_id: format!("{subject}@example.org"),
        date: DATE,
        body: Body { text: format!("Hello about {subject}."), html: None },
        ..Default::default()
    }
}

fn to_send(message: Message) -> JmapSend {
    JmapSend { message: message.id, draft_copy: String::new(), mime: Box::new(message) }
}

fn to_file(message: Message, replaces: &str) -> JmapDraftUp {
    JmapDraftUp { message: message.id, saves: 2, replaces: replaces.into(), mime: Box::new(message) }
}

fn emails_in(server: &StandIn, mailbox: &str) -> Vec<Email> {
    server.with(|d| d.emails.iter().filter(|e| e.mailboxes.contains(mailbox)).cloned().collect())
}

fn delivered(server: &StandIn) -> Vec<crate::standin::Delivery> {
    server.with(|d| d.delivered.clone())
}

/// How many messages were uploaded.
fn uploads(server: &StandIn) -> usize {
    server.seen().iter().filter(|s| s.path.starts_with("/jmap/upload/")).count()
}

fn fault(server: &StandIn, on: &'static str, effect: Effect) {
    server.with(|d| d.faults.push_back(Fault::on(on, effect)));
}

fn outcome(went: &Went<SendDone>) -> Option<&SendOutcome> {
    went.done.as_ref().map(|done| &done.outcome)
}

/// What a message was held for, when it was: whether the answer was lost, and why.
fn held(went: &Went<SendDone>) -> Option<(bool, &str)> {
    match outcome(went) {
        Some(SendOutcome::Held { lost, why }) => Some((*lost, why.as_str())),
        _ => None,
    }
}

// ------------------------------------------------------------------------------------------
// Who it goes to, and how it is written

#[test]
fn the_envelope_has_every_recipient_once_whoever_they_are() {
    let m = Message {
        cc: vec![address("BOB@example.org"), address("dave@example.org"), address("")],
        bcc: vec![address("erin@example.org"), address("carol@example.org")],
        ..message("Plans", &["bob@example.org", "carol@example.org"])
    };
    let envelope = Envelope::of(&m).unwrap();
    assert_eq!(envelope.from, ME);
    // The first spelling of an address, in the order of To, Cc and Bcc.
    assert_eq!(envelope.recipients, ["bob@example.org", "carol@example.org", "dave@example.org", "erin@example.org"]);
    // The sender's blanks are not part of it.
    let spaced = Message { from: Address::new("Ada", " ada@example.org "), ..m };
    assert_eq!(Envelope::of(&spaced).unwrap().from, ME);
}

#[test]
fn a_message_that_cannot_go_is_refused_in_plain_words_and_not_cut() {
    let words = |m: Message| Envelope::of(&m).unwrap_err();
    assert_eq!(words(message("x", &[])), "the message has no recipients");
    assert_eq!(words(message("x", &["", "  "])), "the message has no recipients");
    assert_eq!(words(Message { from: Address::default(), ..message("x", &["bob@example.org"]) }), "the message has no sender");
    assert!(words(Message { from: Address::new("Ada", "ada"), ..message("x", &["bob@example.org"]) }).contains("can be sent from"));
    for bad in ["bob", "bob@", "@example.org", "bob smith@example.org", "bob@example.org>\r\nRCPT TO:<eve@example.org", "bøb@example.org", "a@b@c"] {
        // Leaving someone out silently would be worse than not sending.
        let why = words(message("x", &["carol@example.org", bad]));
        assert!(why.contains("isn't an address mail can be sent to"), "{bad:?}: {why}");
        assert!(!why.contains('\r') && !why.contains('\n'), "{why:?}");
    }
    // An address of a local network, with no dot in its domain, is one.
    assert!(Envelope::of(&message("x", &["root@localhost"])).is_ok());
    // And not too many of them.
    let many: Vec<String> = (0..=MAX_RECIPIENTS).map(|n| format!("p{n}@example.org")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    assert_eq!(words(message("x", &many)), "the message has too many recipients");
}

#[test]
fn a_message_that_is_sent_has_no_bcc_and_a_draft_keeps_it() {
    let m = Message { bcc: vec![address("secret@example.org")], cc: vec![address("carol@example.org")], ..message("Plans", &["bob@example.org"]) };
    let sent = String::from_utf8(octets_of(&m, false)).unwrap();
    assert!(!sent.to_ascii_lowercase().contains("bcc") && !sent.contains("secret@"), "{sent}");
    assert!(sent.contains("Cc: ") && sent.contains("Subject: Plans"), "{sent}");
    let draft = String::from_utf8(octets_of(&m, true)).unwrap();
    assert!(draft.contains("Bcc: ") && draft.contains("secret@example.org"), "{draft}");
    // A message that was never dated is dated now, and one that was is not touched.
    let undated = Message { date: 0, ..m.clone() };
    let now = mailcraft_time::now();
    let parsed = mailcraft_mime::parse(&octets_of(&undated, false));
    assert!((parsed.date - now).abs() < 120, "{} against {now}", parsed.date);
    assert_eq!(mailcraft_mime::parse(&octets_of(&m, false)).date, DATE);
    // No line break can come out of a value the owner typed.
    let injected = Message { subject: "Hi\r\nBcc: eve@example.org".into(), ..m };
    let text = String::from_utf8(octets_of(&injected, false)).unwrap();
    assert!(!text.lines().any(|line| line.to_ascii_lowercase().starts_with("bcc:")), "{text}");
}

// ------------------------------------------------------------------------------------------
// A message goes

#[test]
fn a_message_is_uploaded_filed_in_sent_and_submitted_by_either_kind_of_server() {
    for (kind, profile) in both() {
        let (server, ids) = world(profile);
        let client = client(&server);
        let mut outgoing = outgoing(&client, &ids);
        let m = Message { cc: vec![address("carol@example.org")], bcc: vec![address("dave@example.org")], ..message("Plans", &["bob@example.org"]) };
        let went = block_on(outgoing.send(&to_send(m)));
        assert_eq!(outcome(&went), Some(&SendOutcome::Sent), "{kind}");
        assert!(went.line.is_none() && !went.stop, "{kind}");

        // What was asked of the server: who the account is, the filing, the submission; one upload.
        assert_eq!(server.calls(), ["Identity/get", "Email/import", "EmailSubmission/set"], "{kind}");
        assert_eq!(uploads(&server), 1, "{kind}");
        // The server sent it to all three, once each, from the account, and the message that left has no Bcc.
        let sent = delivered(&server);
        assert_eq!(sent.len(), 1, "{kind}");
        assert_eq!(sent[0].from, ME);
        assert_eq!(sent[0].recipients, ["bob@example.org", "carol@example.org", "dave@example.org"], "{kind}");
        let left = String::from_utf8_lossy(&sent[0].raw).to_string();
        assert!(left.contains("Subject: Plans") && left.contains("Hello about Plans."), "{kind}: {left}");
        assert!(!left.to_ascii_lowercase().contains("bcc") && !left.contains("dave@"), "{kind}: {left}");
        // The copy is in Sent, read, and nothing else is: not a draft, not in any other mailbox.
        let copies = emails_in(&server, &ids.sent);
        assert_eq!(copies.len(), 1, "{kind}");
        assert_eq!(copies[0].keywords, BTreeSet::from(["$seen".to_string()]), "{kind}");
        assert_eq!(copies[0].mailboxes, BTreeSet::from([ids.sent.clone()]));
        assert_eq!(copies[0].raw, sent[0].raw, "the copy is the message that left");
        assert!(emails_in(&server, &ids.drafts).is_empty() && emails_in(&server, &ids.inbox).is_empty(), "{kind}");
        // The upload was taken by the filing; nothing is left to be kept.
        assert!(server.with(|d| d.uploads.is_empty()), "{kind}");
    }
}

#[test]
fn who_it_is_sent_as_is_the_identity_of_its_from_and_no_other() {
    let (server, ids) = world(Profile::standard());
    let work = server.with(|d| d.identity("Ada at work", "ada.work@example.org"));
    let client = client(&server);
    let mut outgoing = outgoing(&client, &ids);
    // The identity is the one whose address is the message's From, however the letters are cased.
    let m = Message { from: Address::new("Ada", "ADA.Work@Example.org"), ..message("Work", &["bob@example.org"]) };
    let went = block_on(outgoing.send(&to_send(m)));
    assert_eq!(outcome(&went), Some(&SendOutcome::Sent));
    let sent = delivered(&server);
    // (The envelope names the sender as the server spells the identity's address.)
    assert_eq!(sent[0].from, "ada.work@example.org");
    let _ = work;

    // One the server lists no identity for is held, with nothing uploaded and nothing sent as another.
    let stranger = Message { id: 8, from: Address::new("Ada", "ada@elsewhere.example"), ..message("Not mine", &["bob@example.org"]) };
    let before = uploads(&server);
    let went = block_on(outgoing.send(&to_send(stranger)));
    assert_eq!(held(&went), Some((false, "the server lists no identity to send as ada@elsewhere.example")));
    assert_eq!((uploads(&server), delivered(&server).len()), (before, 1));
    // The identities were asked for once for the two messages.
    assert_eq!(server.calls().iter().filter(|call| *call == "Identity/get").count(), 1);
}

#[test]
fn a_message_the_server_cannot_have_waits_for_nothing_and_asks_nothing() {
    let (server, ids) = world(Profile::standard());
    let client = client(&server);
    let mut outgoing = outgoing(&client, &ids);
    // Over the size the server takes: nothing is uploaded.
    server.with(|d| d.max_upload = 200);
    let client_small = self::client(&server);
    let mut small = self::outgoing(&client_small, &ids);
    let big = Message { body: Body { text: "x".repeat(2_000), html: None }, ..message("Big", &["bob@example.org"]) };
    let went = block_on(small.send(&to_send(big)));
    assert_eq!(held(&went), Some((false, "the message is larger than the server takes (200 bytes)")));
    assert_eq!(server.api_requests() + uploads(&server), 0, "nothing was asked of the server: {:?}", server.seen());

    // A message with no one to go to, or a sender that is none: held, without a word to the server.
    let none = block_on(outgoing.send(&to_send(message("Nobody", &[]))));
    assert_eq!(held(&none), Some((false, "the message has no recipients")));
    // A server with no sending at all.
    server.with(|d| {
        d.max_upload = 25_000_000;
        d.submission = false;
    });
    let plain = self::client(&server);
    let mut none_here = self::outgoing(&plain, &ids);
    let went = block_on(none_here.send(&to_send(message("Hello", &["bob@example.org"]))));
    assert_eq!(held(&went), Some((false, "the server doesn't send mail")));
    // And an account with no Sent mailbox to keep a copy in.
    let mut homeless = Outgoing::new(&client, None, Some(&ids.drafts), Secrets::default());
    let went = block_on(homeless.send(&to_send(message("Hello", &["bob@example.org"]))));
    assert_eq!(held(&went), Some((false, "the server has no Sent folder to keep a copy of it in")));
    assert_eq!(server.api_requests(), 0, "{:?}", server.calls());
    assert!(delivered(&server).is_empty());
}

#[test]
fn the_draft_a_message_was_written_as_is_destroyed_once_it_has_gone_and_not_before() {
    for (kind, profile) in both() {
        let (server, ids) = world(profile);
        let draft = server.with(|d| d.deliver(&ids.drafts, b"Subject: draft\r\n\r\nx\r\n", &["$draft", "$seen"], "2026-10-14T09:00:00Z"));
        let client = client(&server);
        let mut outgoing = outgoing(&client, &ids);
        let from_draft = |m: Message| JmapSend { draft_copy: draft.clone(), ..to_send(m) };

        // A server that refuses the submission leaves the draft where it was.
        fault(&server, "EmailSubmission/set", Effect::SetError("forbiddenToSend", "no"));
        let went = block_on(outgoing.send(&from_draft(message("One", &["bob@example.org"]))));
        assert!(held(&went).is_some_and(|(lost, _)| !lost), "{kind}");
        assert_eq!(emails_in(&server, &ids.drafts).len(), 1, "{kind}: the draft stays");
        // So does one whose answer was lost: the message may be gone, and the draft is what the owner has.
        fault(&server, "EmailSubmission/set", Effect::LoseAnswer);
        let went = block_on(outgoing.send(&from_draft(message("Two", &["bob@example.org"]))));
        assert_eq!(held(&went).map(|(lost, _)| lost), Some(true), "{kind}");
        assert_eq!(emails_in(&server, &ids.drafts).len(), 1, "{kind}: the draft stays");
        // One that goes takes it with it.
        let went = block_on(outgoing.send(&from_draft(message("Three", &["bob@example.org"]))));
        assert_eq!(outcome(&went), Some(&SendOutcome::Sent), "{kind}");
        assert!(emails_in(&server, &ids.drafts).is_empty(), "{kind}: the draft is gone");
        // If the draft can't be destroyed, the message has gone all the same.
        let other = server.with(|d| d.deliver(&ids.drafts, b"Subject: another\r\n\r\ny\r\n", &["$draft"], "2026-10-14T10:00:00Z"));
        fault(&server, "Email/set", Effect::MethodError("serverFail"));
        let again = JmapSend { draft_copy: other, ..to_send(message("Four", &["bob@example.org"])) };
        let went = block_on(outgoing.send(&again));
        assert_eq!(outcome(&went), Some(&SendOutcome::Sent), "{kind}");
        assert_eq!(emails_in(&server, &ids.drafts).len(), 1, "{kind}");
    }
}

// ------------------------------------------------------------------------------------------
// A message is refused

#[test]
fn a_submission_the_server_refuses_takes_its_copy_out_of_sent_and_says_why_in_plain_words() {
    let refusals = [
        ("forbiddenFrom", "the server doesn't let this account send as ada@example.org"),
        ("forbiddenMailFrom", "the server doesn't let this account send as ada@example.org"),
        ("forbiddenToSend", "the server doesn't let this account send mail just now"),
        ("noRecipients", "the message has no recipients"),
        ("invalidRecipients", "the server doesn't accept an address the message is for"),
        ("tooManyRecipients", "the message has more recipients than the server allows"),
        ("tooLarge", "the message is too large for the server"),
        ("invalidEmail", "the server found the message invalid"),
        ("somethingNew", "the server refused it (somethingNew): It said so."),
    ];
    for (kind, profile) in both() {
        for (type_, said) in refusals {
            let (server, ids) = world(profile);
            let client = client(&server);
            let mut outgoing = outgoing(&client, &ids);
            fault(&server, "EmailSubmission/set", Effect::SetError(type_, "It said so."));
            let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
            assert_eq!(held(&went), Some((false, said)), "{kind}, {type_}");
            // Held: tried no further, and it can be sent again by the owner; nothing is left to look as if it went.
            assert!(!went.stop && went.line.is_none(), "{kind}, {type_}");
            assert!(emails_in(&server, &ids.sent).is_empty() && delivered(&server).is_empty(), "{kind}, {type_}");
        }
    }
}

#[test]
fn the_recipients_a_server_will_not_have_are_named_when_they_are_the_messages_own() {
    // A server in front of an SMTP server wants a dot in the domain of an address.
    let (server, ids) = world(Profile::by_place());
    let client = client(&server);
    let mut outgoing = outgoing(&client, &ids);
    let went = block_on(outgoing.send(&to_send(message("Plans", &["root@localhost", "bob@example.org"]))));
    assert_eq!(held(&went), Some((false, "the server doesn't accept root@localhost")));
    assert!(emails_in(&server, &ids.sent).is_empty() && delivered(&server).is_empty());
    // What a server names that the message does not hold is not shown as an address.
    let words = words(
        &Refusal { kind: "invalidRecipients".into(), invalid: vec!["eve@example.org".into(), "ROOT@localhost".into()], ..Default::default() },
        Envelope::of(&message("x", &["root@localhost"])).ok().as_ref(),
        &|t| t.to_string(),
    );
    assert_eq!(words, "the server doesn't accept ROOT@localhost");
}

#[test]
fn a_server_that_limits_sending_leaves_the_rest_in_the_outbox_and_says_so() {
    let (server, ids) = world(Profile::by_place());
    // The server has sent as many as it will for a while.
    server.with(|d| d.sent_lately = 30);
    let client = client(&server);
    let mut outgoing = outgoing(&client, &ids);
    let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
    assert!(went.done.is_none(), "not sent, and not held either: it waits");
    assert_eq!(went.line.as_deref(), Some(RATE_LIMITED));
    assert!(went.stop, "nothing after it is tried this run");
    assert!(emails_in(&server, &ids.sent).is_empty() && delivered(&server).is_empty());
    // A server of the standard kind says the same in the standard's word.
    let (server, ids) = world(Profile::standard());
    fault(&server, "EmailSubmission/set", Effect::SetError("rateLimit", "slow down"));
    let client = self::client(&server);
    let mut outgoing = self::outgoing(&client, &ids);
    let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
    assert!(went.done.is_none() && went.stop && went.line.as_deref() == Some(RATE_LIMITED));
    assert!(emails_in(&server, &ids.sent).is_empty());
    // And one that is unavailable for a moment leaves it waiting too.
    fault(&server, "EmailSubmission/set", Effect::SetError("serverUnavailable", "later"));
    let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
    assert!(went.done.is_none() && went.stop && went.line.as_deref() == Some(UNAVAILABLE));
    assert!(emails_in(&server, &ids.sent).is_empty() && delivered(&server).is_empty());
}

// ------------------------------------------------------------------------------------------
// The answer is lost

#[test]
fn an_answer_that_does_not_say_whether_it_went_holds_the_message_and_keeps_its_copy() {
    // Each of these may have come after the server sent the message.
    let lost: [(&str, Effect, usize); 7] = [
        ("the connection was lost", Effect::LoseAnswer, 1),
        ("a gateway gave up (502)", Effect::Status(502), 1),
        ("a gateway gave up (504)", Effect::Status(504), 1),
        ("the server failed (500)", Effect::Status(500), 1),
        ("an answer that is not JMAP", Effect::Garbage, 1),
        ("the call failed with serverFail", Effect::MethodError("serverFail"), 0),
        ("the call failed in part", Effect::MethodError("serverPartialFail"), 0),
    ];
    for (kind, profile) in both() {
        for (what, effect, went_out) in lost {
            let (server, ids) = world(profile);
            let client = client(&server);
            let mut outgoing = outgoing(&client, &ids);
            fault(&server, "EmailSubmission/set", effect);
            let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
            assert_eq!(held(&went), Some((true, "")), "{kind}, {what}");
            // Nothing more this run, and the copy stays: if the message went, it is the owner's record of it.
            assert!(went.stop, "{kind}, {what}");
            assert_eq!(emails_in(&server, &ids.sent).len(), 1, "{kind}, {what}");
            assert_eq!(delivered(&server).len(), went_out, "{kind}, {what}");
        }
    }
}

#[test]
fn a_submission_that_never_got_as_far_as_sending_waits_and_leaves_no_copy() {
    // The server refused the request as a whole, or was busy, or turned the sign-in down: nothing was sent.
    let not_taken: [(&str, Effect, Option<&str>); 4] = [
        ("busy (503)", Effect::RefuseWith(503), Some(BUSY)),
        ("busy (429)", Effect::RefuseWith(429), Some(BUSY)),
        ("signed out (401)", Effect::RefuseWith(401), None),
        ("signed out (403)", Effect::RefuseWith(403), None),
    ];
    for (kind, profile) in both() {
        for (what, effect, line) in not_taken {
            let (server, ids) = world(profile);
            let client = client(&server);
            let mut outgoing = outgoing(&client, &ids);
            fault(&server, "EmailSubmission/set", effect);
            let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
            assert!(went.done.is_none() && went.stop, "{kind}, {what}");
            assert_eq!(went.line.as_deref(), line, "{kind}, {what}");
            assert!(emails_in(&server, &ids.sent).is_empty() && delivered(&server).is_empty(), "{kind}, {what}");
        }
    }
    // The request was refused for being a request the server won't take (a 400): the message is held, not lost.
    let (server, ids) = world(Profile::standard());
    let client = client(&server);
    let mut outgoing = outgoing(&client, &ids);
    fault(&server, "EmailSubmission/set", Effect::RefuseWith(400));
    let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
    let (lost, why) = held(&went).unwrap();
    assert!(!lost && why.starts_with("the server wouldn't take it (HTTP 400"), "{why}");
    assert!(emails_in(&server, &ids.sent).is_empty() && delivered(&server).is_empty());
    // A call that failed in a way the standard says changed nothing: the server's word is taken.
    fault(&server, "EmailSubmission/set", Effect::MethodError("forbidden"));
    let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
    assert_eq!(held(&went), Some((false, "the server doesn't let this account send mail just now")));
    assert!(emails_in(&server, &ids.sent).is_empty() && delivered(&server).is_empty());
}

#[test]
fn trouble_before_the_submission_never_costs_a_message_it_can_be_tried_again() {
    // (What went wrong, the fault, and what becomes of the message.)
    #[derive(Debug)]
    enum Becomes {
        Waits(Option<&'static str>),
        Held(&'static str),
    }
    let upload_cases: [(&str, Effect, Becomes); 7] = [
        ("too large (413)", Effect::RefuseWith(413), Becomes::Held("the message is too large for the server")),
        ("too many kept (429)", Effect::RefuseWith(429), Becomes::Waits(Some(BUSY))),
        ("busy (503)", Effect::RefuseWith(503), Becomes::Waits(Some(BUSY))),
        ("signed out (401)", Effect::RefuseWith(401), Becomes::Waits(None)),
        ("a failure (500)", Effect::RefuseWith(500), Becomes::Held("the server wouldn't take it (HTTP 500")),
        ("the answer lost", Effect::LoseAnswer, Becomes::Waits(Some(UNREACHABLE))),
        ("an answer that is not JMAP", Effect::Garbage, Becomes::Held("the server's answer wasn't understood")),
    ];
    let import_cases: [(&str, Effect, Becomes); 6] = [
        ("over its quota", Effect::SetError("overQuota", ""), Becomes::Held("the mailbox on the server is full")),
        ("the upload lost", Effect::SetError("blobNotFound", ""), Becomes::Held("the server lost the message it was sent")),
        ("unavailable", Effect::MethodError("serverUnavailable"), Becomes::Waits(Some(UNAVAILABLE))),
        ("a failure", Effect::MethodError("serverFail"), Becomes::Held("the server refused it (serverFail)")),
        ("the answer lost", Effect::LoseAnswer, Becomes::Waits(Some(UNREACHABLE))),
        ("busy (503)", Effect::RefuseWith(503), Becomes::Waits(Some(BUSY))),
    ];
    for (kind, profile) in both() {
        for (step, on, cases) in [("upload", "upload", upload_cases.as_slice()), ("filing", "Email/import", import_cases.as_slice())] {
            for (what, effect, becomes) in cases {
                let (server, ids) = world(profile);
                let client = client(&server);
                let mut outgoing = outgoing(&client, &ids);
                fault(&server, on, *effect);
                let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
                match becomes {
                    Becomes::Waits(line) => {
                        assert!(went.done.is_none() && went.stop, "{kind}, {step}: {what}");
                        assert_eq!(went.line.as_deref(), *line, "{kind}, {step}: {what}");
                    }
                    Becomes::Held(why) => {
                        let (lost, said) = held(&went).unwrap_or_else(|| panic!("{kind}, {step}: {what}: {:?}", went.done));
                        assert!(!lost && said.starts_with(why), "{kind}, {step}: {what}: {said}");
                        assert!(!went.stop, "{kind}, {step}: {what}");
                    }
                }
                // Whatever happened, nothing was submitted and nothing left.
                assert!(!server.calls().iter().any(|call| call == "EmailSubmission/set"), "{kind}, {step}: {what}");
                assert!(delivered(&server).is_empty(), "{kind}, {step}: {what}");
            }
        }
    }
}

#[test]
fn a_message_is_asked_for_in_the_order_that_cannot_lose_it() {
    // The requests, in order, of a message that goes: who, the upload, the filing, the submission.
    let (server, ids) = world(Profile::standard());
    let client = client(&server);
    let mut outgoing = outgoing(&client, &ids);
    block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
    let paths: Vec<(String, String, Vec<String>)> = server.seen().into_iter().skip(1).map(|s| (s.method, s.path, s.calls)).collect();
    let api = |call: &str| ("POST".to_string(), "/jmap/api".to_string(), vec![call.to_string()]);
    assert_eq!(
        paths,
        [api("Identity/get"), ("POST".into(), "/jmap/upload/A1/".into(), vec![]), api("Email/import"), api("EmailSubmission/set")],
        "{:?}",
        server.seen()
    );
}

// ------------------------------------------------------------------------------------------
// A draft is filed

#[test]
fn a_draft_is_filed_in_drafts_with_its_bcc_and_the_copy_before_it_is_destroyed() {
    for (kind, profile) in both() {
        let (server, ids) = world(profile);
        let before = server.with(|d| d.deliver(&ids.drafts, b"Subject: first\r\n\r\nx\r\n", &["$draft", "$seen"], "2026-10-14T09:00:00Z"));
        let client = client(&server);
        let mut outgoing = outgoing(&client, &ids);
        let m = Message { bcc: vec![address("secret@example.org")], ..message("Plans", &["bob@example.org"]) };
        let went = block_on(outgoing.draft(&to_file(m, &before)));
        let done = went.done.unwrap_or_else(|| panic!("{kind}: filed"));
        assert!(!went.stop && went.line.is_none(), "{kind}");
        assert_eq!((done.message, done.saves, done.refused.as_deref(), done.folder.as_str()), (7, 2, None, ids.drafts.as_str()), "{kind}");
        // The new copy is the only one, with `$draft` and `$seen`, and keeps its Bcc: a draft is its owner's.
        let copies = emails_in(&server, &ids.drafts);
        assert_eq!(copies.len(), 1, "{kind}");
        assert_eq!(
            (copies[0].id.as_str(), copies[0].keywords.clone()),
            (done.id.as_str(), BTreeSet::from(["$draft".to_string(), "$seen".to_string()])),
            "{kind}"
        );
        let filed = String::from_utf8_lossy(&copies[0].raw).to_string();
        assert!(filed.contains("Bcc: ") && filed.contains("secret@example.org"), "{kind}: {filed}");
        assert_ne!(done.id, before);
        // The new copy was filed before the old one was destroyed, never the other way about.
        assert_eq!(server.calls(), ["Email/import", "Email/set"], "{kind}");
        // Nothing was sent anywhere, and nothing is in Sent.
        assert!(delivered(&server).is_empty() && emails_in(&server, &ids.sent).is_empty(), "{kind}");
        // A first save makes no destroy.
        let calls = server.calls().len();
        let went = block_on(outgoing.draft(&to_file(message("Another", &["bob@example.org"]), "")));
        assert!(went.done.is_some_and(|done| !done.id.is_empty()), "{kind}");
        assert_eq!(server.calls().len(), calls + 1, "{kind}: just the filing");
    }
}

#[test]
fn a_draft_that_cannot_be_filed_keeps_the_copy_it_has() {
    for (kind, profile) in both() {
        // Trouble that may pass: the draft waits for the next run, with the copy it has.
        let waits: [(&str, &str, Effect, Option<&str>); 5] = [
            ("the network", "upload", Effect::LoseAnswer, Some(UNREACHABLE)),
            ("busy", "upload", Effect::RefuseWith(503), Some(BUSY)),
            ("signed out", "upload", Effect::RefuseWith(401), None),
            ("unavailable", "Email/import", Effect::MethodError("serverUnavailable"), Some(UNAVAILABLE)),
            ("a failure", "Email/import", Effect::MethodError("serverFail"), Some(UNAVAILABLE)),
        ];
        for (what, on, effect, line) in waits {
            let (server, ids) = world(profile);
            let before = server.with(|d| d.deliver(&ids.drafts, b"Subject: first\r\n\r\nx\r\n", &["$draft"], "2026-10-14T09:00:00Z"));
            let client = client(&server);
            let mut outgoing = outgoing(&client, &ids);
            fault(&server, on, effect);
            let went = block_on(outgoing.draft(&to_file(message("Plans", &["bob@example.org"]), &before)));
            assert!(went.done.is_none() && went.stop, "{kind}, {what}");
            assert_eq!(went.line.as_deref(), line, "{kind}, {what}");
            assert!(emails_in(&server, &ids.drafts).iter().any(|e| e.id == before), "{kind}, {what}: the copy it has stays");
        }
        // A server that says it will not have it settles it: said once, not tried again by itself.
        let refused: [(&str, Effect, &str); 3] = [
            ("over its quota", Effect::SetError("overQuota", ""), "the mailbox on the server is full"),
            ("too large", Effect::SetError("tooLarge", ""), "the message is too large for the server"),
            ("not allowed", Effect::SetError("forbidden", ""), "the server doesn't let this account send mail just now"),
        ];
        for (what, effect, why) in refused {
            let (server, ids) = world(profile);
            let before = server.with(|d| d.deliver(&ids.drafts, b"Subject: first\r\n\r\nx\r\n", &["$draft"], "2026-10-14T09:00:00Z"));
            let client = client(&server);
            let mut outgoing = outgoing(&client, &ids);
            fault(&server, "Email/import", effect);
            let went = block_on(outgoing.draft(&to_file(message("Plans", &["bob@example.org"]), &before)));
            let done = went.done.unwrap_or_else(|| panic!("{kind}, {what}"));
            assert_eq!((done.id.as_str(), done.refused.as_deref()), ("", Some(why)), "{kind}, {what}");
            assert!(!went.stop, "{kind}, {what}");
            assert!(emails_in(&server, &ids.drafts).iter().any(|e| e.id == before), "{kind}, {what}: the copy it has stays");
        }
        // Too large for the server, or nowhere to file it: refused, with nothing asked of the server.
        let (server, ids) = world(profile);
        server.with(|d| d.max_upload = 200);
        let client = self::client(&server);
        let mut outgoing = self::outgoing(&client, &ids);
        let big = Message { body: Body { text: "x".repeat(2_000), html: None }, ..message("Big", &["bob@example.org"]) };
        let done = block_on(outgoing.draft(&to_file(big, ""))).done.unwrap();
        assert_eq!(done.refused.as_deref(), Some("it is larger than the server takes (200 bytes)"), "{kind}");
        let mut homeless = Outgoing::new(&client, Some(&ids.sent), None, Secrets::default());
        let done = block_on(homeless.draft(&to_file(message("Plans", &["bob@example.org"]), ""))).done.unwrap();
        assert_eq!(done.refused.as_deref(), Some("the server has no Drafts folder to keep it in"), "{kind}");
        assert_eq!(server.api_requests() + uploads(&server), 0, "{kind}");
    }
}

#[test]
fn a_copy_that_cannot_be_destroyed_is_no_loss_of_the_draft() {
    let (server, ids) = world(Profile::standard());
    let before = server.with(|d| d.deliver(&ids.drafts, b"Subject: first\r\n\r\nx\r\n", &["$draft"], "2026-10-14T09:00:00Z"));
    let client = client(&server);
    let mut outgoing = outgoing(&client, &ids);
    // The destroy fails: the new copy is filed all the same (a second draft is better than none).
    fault(&server, "Email/set", Effect::MethodError("serverFail"));
    let went = block_on(outgoing.draft(&to_file(message("Plans", &["bob@example.org"]), &before)));
    let done = went.done.unwrap();
    assert!(done.refused.is_none() && !done.id.is_empty() && !went.stop);
    assert_eq!(emails_in(&server, &ids.drafts).len(), 2);
    // The network lost on the destroy ends this run's sending, with the draft filed.
    fault(&server, "Email/set", Effect::LoseAnswer);
    let went = block_on(outgoing.draft(&to_file(message("Plans", &["bob@example.org"]), &done.id)));
    assert!(went.done.is_some_and(|done| !done.id.is_empty()) && went.stop);
    // An earlier copy that is not there any more is as good as destroyed.
    let went = block_on(outgoing.draft(&to_file(message("Plans", &["bob@example.org"]), "Egone")));
    assert!(went.done.is_some_and(|done| !done.id.is_empty()) && !went.stop);
}

// ------------------------------------------------------------------------------------------
// What is asked, word for word

/// An `Http` that answers what it is told to, in order, and keeps what it was sent.
#[derive(Clone, Default)]
struct Script {
    replies: Rc<RefCell<VecDeque<Result<Response>>>>,
    sent: Rc<RefCell<Vec<Request>>>,
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

fn answer(method: &str, result: Value) -> Result<Response> {
    reply(200, &json!({"methodResponses": [[method, result, "c0"]], "sessionState": "1"}))
}

fn session_doc() -> Value {
    json!({
        "capabilities": {CORE: {"maxCallsInRequest": 16, "maxObjectsInGet": 500, "maxSizeUpload": 1_000_000}, MAIL: {}, SUBMISSION: {}},
        "accounts": {"A1": {"name": ME}},
        "primaryAccounts": {MAIL: "A1", SUBMISSION: "A1"},
        "apiUrl": "/jmap/api",
        "downloadUrl": "/jmap/download/{accountId}/{blobId}/{name}?type={type}",
        "uploadUrl": "/jmap/upload/{accountId}/",
        "state": "1",
    })
}

fn scripted(replies: Vec<Result<Response>>) -> (Client<Script>, Script) {
    let script = Script::default();
    script.replies.borrow_mut().push_back(reply(200, &session_doc()));
    script.replies.borrow_mut().extend(replies);
    let client = block_on(Client::connect(script.clone(), "https://jmap.example.org/jmap/session", &Auth::bearer("tk-3f9a7c"))).unwrap();
    (client, script)
}

fn body_of(request: &Request) -> Value {
    serde_json::from_slice(&request.body).unwrap_or(Value::Null)
}

/// The replies of a server that sends: who, the upload, the filing, the submission.
fn happy() -> Vec<Result<Response>> {
    vec![
        answer("Identity/get", json!({"accountId": "A1", "state": "1", "list": [{"id": "I1", "name": "Ada", "email": ME}], "notFound": []})),
        reply(201, &json!({"accountId": "A1", "blobId": "U1", "type": "message/rfc822", "size": 0})),
        answer(
            "Email/import",
            json!({"accountId": "A1", "created": {"m": {"id": "E1", "blobId": "B1", "threadId": "E1", "size": 100}}, "notCreated": {}}),
        ),
        answer("EmailSubmission/set", json!({"accountId": "A1", "created": {"s": {"id": "S1"}}, "notCreated": {}})),
    ]
}

#[test]
fn what_is_asked_of_the_server_is_what_the_standard_has_word_for_word() {
    let (client, script) = scripted(happy());
    let mut outgoing = Outgoing::new(&client, Some("Msent"), Some("Mdrafts"), Secrets::default());
    let m = Message { bcc: vec![address("dave@example.org")], ..message("Plans", &["bob@example.org", "BOB@example.org"]) };
    let octets = octets_of(&m, false);
    // (The scripted upload says it kept the octets it was sent.)
    script.replies.borrow_mut()[1] = reply(201, &json!({"accountId": "A1", "blobId": "U1", "type": "message/rfc822", "size": octets.len()}));
    let went = block_on(outgoing.send(&to_send(m)));
    assert_eq!(outcome(&went), Some(&SendOutcome::Sent));

    let sent = script.sent.borrow().clone();
    assert_eq!(sent.len(), 5, "the session, who, the upload, the filing, the submission");
    let using = json!(["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:ietf:params:jmap:submission"]);
    // Who the account may send as: with the capability of sending.
    assert_eq!(body_of(&sent[1]), json!({"using": using, "methodCalls": [["Identity/get", {"accountId": "A1", "ids": null}, "c0"]]}));
    // The message, as it goes: in the body of the upload, with the credentials and its type.
    assert_eq!((sent[2].method.as_str(), sent[2].url.as_str()), ("POST", "https://jmap.example.org/jmap/upload/A1/"));
    assert_eq!(sent[2].body, octets);
    assert!(sent[2].headers.contains(&("Content-Type".to_string(), "message/rfc822".to_string())));
    assert!(sent[2].headers.contains(&("Authorization".to_string(), "Bearer tk-3f9a7c".to_string())));
    // The filing, in the Sent mailbox, read.
    assert_eq!(
        body_of(&sent[3]),
        json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            "methodCalls": [["Email/import", {"accountId": "A1", "emails": {"m": {"blobId": "U1", "mailboxIds": {"Msent": true}, "keywords": {"$seen": true}}}}, "c0"]],
        })
    );
    // The submission: the identity, the email, and every recipient once, the Bcc's included.
    assert_eq!(
        body_of(&sent[4]),
        json!({
            "using": using,
            "methodCalls": [["EmailSubmission/set", {"accountId": "A1", "create": {"s": {
                "identityId": "I1",
                "emailId": "E1",
                "envelope": {"mailFrom": {"email": ME, "parameters": null}, "rcptTo": [
                    {"email": "bob@example.org", "parameters": null},
                    {"email": "dave@example.org", "parameters": null},
                ]},
            }}}, "c0"]],
        })
    );
    // A draft: the same upload, filed in Drafts as a draft, then the copy it replaces destroyed.
    let (client, script) = scripted(vec![
        reply(201, &json!({"accountId": "A1", "blobId": "U2", "type": "message/rfc822"})),
        answer(
            "Email/import",
            json!({"accountId": "A1", "created": {"m": {"id": "E2", "blobId": "B2", "threadId": "E2", "size": 1}}, "notCreated": {}}),
        ),
        answer("Email/set", json!({"accountId": "A1", "destroyed": ["E9"], "notDestroyed": {}})),
    ]);
    let mut outgoing = Outgoing::new(&client, Some("Msent"), Some("Mdrafts"), Secrets::default());
    let went = block_on(outgoing.draft(&to_file(message("Plans", &["bob@example.org"]), "E9")));
    assert_eq!(went.done.map(|done| (done.id, done.folder)), Some(("E2".to_string(), "Mdrafts".to_string())));
    let sent = script.sent.borrow().clone();
    assert_eq!(
        body_of(&sent[2]),
        json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            "methodCalls": [["Email/import", {"accountId": "A1", "emails": {"m": {"blobId": "U2", "mailboxIds": {"Mdrafts": true}, "keywords": {"$draft": true, "$seen": true}}}}, "c0"]],
        })
    );
    assert_eq!(
        body_of(&sent[3]),
        json!({"using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"], "methodCalls": [["Email/set", {"accountId": "A1", "destroy": ["E9"]}, "c0"]]})
    );
}

// ------------------------------------------------------------------------------------------
// What a submission's answer means

fn answers_of(parts: Vec<Result<Value>>) -> Result<Answers> {
    Ok(Answers(parts))
}

fn classified(status: Option<u16>, outcome: Result<Answers>) -> &'static str {
    match classify(status, outcome) {
        Submitted::Made => "made",
        Submitted::Refused(_) => "refused",
        Submitted::NotTaken(_) => "not taken",
        Submitted::Unknown => "unknown",
    }
}

#[test]
fn an_answer_says_a_message_went_only_when_the_server_says_it_made_the_submission() {
    let made = json!({"created": {"s": {"id": "S1"}}});
    let refused = json!({"notCreated": {"s": {"type": "forbiddenFrom", "description": "No."}}});
    let table: Vec<(&str, Option<u16>, Result<Answers>, &str)> = vec![
        ("it made it", Some(200), answers_of(vec![Ok(made.clone())]), "made"),
        ("it refused", Some(200), answers_of(vec![Ok(refused.clone())]), "refused"),
        ("both at once", Some(200), answers_of(vec![Ok(json!({"created": {"s": {}}, "notCreated": {"s": {"type": "x"}}}))]), "unknown"),
        ("neither", Some(200), answers_of(vec![Ok(json!({"created": {}, "notCreated": {}}))]), "unknown"),
        ("another object's", Some(200), answers_of(vec![Ok(json!({"created": {"other": {}}}))]), "unknown"),
        ("a created that is no object", Some(200), answers_of(vec![Ok(json!({"created": {"s": true}}))]), "unknown"),
        ("a refusal that is no object", Some(200), answers_of(vec![Ok(json!({"notCreated": {"s": "no"}}))]), "unknown"),
        ("an answer that is none", Some(200), answers_of(vec![Ok(json!("made"))]), "unknown"),
        ("no answer to the call", Some(200), answers_of(vec![Err(Error::Protocol("no answer".into()))]), "unknown"),
        ("a call with no answers at all", Some(200), answers_of(vec![]), "unknown"),
        (
            "a method error that changed nothing",
            Some(200),
            answers_of(vec![Err(Error::Refused { kind: "forbidden".into(), description: String::new() })]),
            "refused",
        ),
        (
            "an unknown method",
            Some(200),
            answers_of(vec![Err(Error::Refused { kind: "unknownMethod".into(), description: String::new() })]),
            "refused",
        ),
        ("a server failure", Some(200), answers_of(vec![Err(Error::Refused { kind: "serverFail".into(), description: String::new() })]), "unknown"),
        (
            "a partial failure",
            Some(200),
            answers_of(vec![Err(Error::Refused { kind: "serverPartialFail".into(), description: String::new() })]),
            "unknown",
        ),
        ("the connection lost", None, Err(Error::Io("offline".into())), "unknown"),
        ("an answer too large to be read", None, Err(Error::TooLarge("big".into())), "unknown"),
        ("an answer too large to be read, of a call that ran", Some(200), Err(Error::TooLarge("big".into())), "unknown"),
        ("an answer that is not JMAP", Some(200), Err(Error::Protocol("html".into())), "unknown"),
        ("a gateway that gave up", Some(504), Err(Error::Bad("HTTP 504".into())), "unknown"),
        ("a server failure", Some(500), Err(Error::Bad("HTTP 500".into())), "unknown"),
        ("a request that is none", Some(400), Err(Error::Bad("HTTP 400".into())), "not taken"),
        ("a request over a limit", Some(400), Err(Error::TooLarge("limit".into())), "not taken"),
        ("a refusal of the request, not found", Some(404), Err(Error::Bad("HTTP 404".into())), "not taken"),
        ("a sign-in refused", Some(401), Err(Error::Auth("no".into())), "not taken"),
        ("a sign-in that expired", Some(403), Err(Error::Auth("no".into())), "not taken"),
        ("a busy server", Some(503), Err(Error::Busy { status: 503, retry_after: Some(5) }), "not taken"),
        ("a limit on requests", Some(429), Err(Error::Busy { status: 429, retry_after: None }), "not taken"),
        ("no secure connection", None, Err(Error::Tls("certificate".into())), "not taken"),
    ];
    for (what, status, outcome, expected) in table {
        assert_eq!(classified(status, outcome), expected, "{what}");
    }
    // The refusal says what the server said of it, and no more than it was asked.
    let Submitted::Refused(refusal) = classify(
        Some(200),
        answers_of(vec![Ok(json!({"notCreated": {"s": {
            "type": "invalidRecipients", "description": "Not these.", "invalidRecipients": ["a@example.org", 5, "b@example.org"], "maxRecipients": 50,
        }}}))]),
    ) else {
        panic!("a refusal");
    };
    assert_eq!(
        (refusal.kind.as_str(), refusal.description.as_str(), refusal.invalid.as_slice(), refusal.most),
        ("invalidRecipients", "Not these.", &["a@example.org".to_string(), "b@example.org".to_string()][..], Some(50))
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Whatever a server answers to a submission, a message is sent only if it says it made it and not that it
    /// did not.
    #[test]
    fn nothing_but_a_created_submission_is_a_message_sent(
        answer in prop_oneof![crate::tests::any_json(), Just(json!({"created": {"s": {"id": "S"}}})), Just(json!({"notCreated": {"s": {"type": "x"}}}))],
        status in prop_oneof![Just(200u16), any::<u16>()],
        error in proptest::option::of(0usize..6),
    ) {
        let outcome = match error {
            None => answers_of(vec![Ok(answer.clone())]),
            Some(0) => answers_of(vec![Err(Error::Protocol("x".into()))]),
            Some(1) => answers_of(vec![Err(Error::Refused { kind: "serverFail".into(), description: String::new() })]),
            Some(2) => Err(Error::Io("x".into())),
            Some(3) => Err(Error::Bad("HTTP 500".into())),
            Some(_) => Err(Error::Busy { status: 503, retry_after: None }),
        };
        if matches!(classify(Some(status), outcome), Submitted::Made) {
            let says = |key: &str| answer.get(key).and_then(|c| c.get("s")).is_some_and(Value::is_object);
            prop_assert!(error.is_none() && says("created") && !says("notCreated"), "{answer}");
        }
    }

    /// A hostile server cannot make the reading of what it made panic, or name an id that is none.
    #[test]
    fn any_answer_to_a_filing_or_a_list_of_identities_is_read_or_refused(answer in prop_oneof![crate::tests::any_json(), Just(json!({"created": {"m": {"id": "E1"}}}))]) {
        if let Ok(made) = created(&answer, "m") {
            prop_assert!(made.is_object());
        }
        let list = answer.get("list").and_then(Value::as_array);
        prop_assert!(list.is_none_or(|l| l.len() < 10_000));
    }

    /// Whatever the server answers at each step, a message is never sent unless the last step says it was made.
    #[test]
    fn a_hostile_server_never_makes_a_message_go_that_it_did_not_make(
        who in prop_oneof![2 => Just(happy_reply(0)), 1 => crate::tests::any_json()],
        upload in prop_oneof![2 => Just(happy_reply(1)), 1 => crate::tests::any_json()],
        filing in prop_oneof![2 => Just(happy_reply(2)), 1 => crate::tests::any_json()],
        submission in prop_oneof![2 => Just(happy_reply(3)), 1 => crate::tests::any_json()],
        status in prop_oneof![Just(200u16), Just(201u16), any::<u16>()],
    ) {
        let (client, _script) = scripted(vec![
            Ok(Response { status: 200, body: serde_json::to_vec(&who).unwrap(), retry_after: None }),
            Ok(Response { status, body: serde_json::to_vec(&upload).unwrap(), retry_after: None }),
            Ok(Response { status: 200, body: serde_json::to_vec(&filing).unwrap(), retry_after: None }),
            Ok(Response { status, body: serde_json::to_vec(&submission).unwrap(), retry_after: None }),
            Err(Error::Io("end".into())),
        ]);
        let mut outgoing = Outgoing::new(&client, Some("Msent"), Some("Mdrafts"), Secrets::default());
        let went = block_on(outgoing.send(&to_send(message("Plans", &["bob@example.org"]))));
        if matches!(outcome(&went), Some(SendOutcome::Sent)) {
            let said = submission.get("methodResponses").and_then(|r| r.get(0)).and_then(|r| r.get(1)).and_then(|r| r.get("created")).and_then(|c| c.get("s"));
            prop_assert!(said.is_some_and(Value::is_object), "{submission}");
        }
    }
}

/// The body of the reply number `n` of a server that sends (the first is who).
fn happy_reply(n: usize) -> Value {
    let reply = happy().remove(n).unwrap();
    serde_json::from_slice(&reply.body).unwrap()
}

#[test]
fn a_filing_or_an_upload_is_only_believed_when_it_names_what_it_made() {
    // A blob that is none, a size that is not the octets sent, an email with an id that cannot be one.
    let cases: [(&str, Value, bool); 6] = [
        ("a good answer", json!({"blobId": "U1", "type": "message/rfc822", "size": 5}), true),
        ("no size", json!({"blobId": "U1"}), true),
        ("another size", json!({"blobId": "U1", "size": 6}), false),
        ("no blob", json!({"size": 5}), false),
        ("a blob that is no id", json!({"blobId": "U 1", "size": 5}), false),
        ("a blob that is no string", json!({"blobId": 7, "size": 5}), false),
    ];
    for (what, body, good) in cases {
        let (client, _) = scripted(vec![reply(201, &body)]);
        assert_eq!(block_on(upload(&client, vec![0; 5])).is_ok(), good, "{what}");
    }
    let (client, _) = scripted(vec![reply(413, &json!({"type": "urn:ietf:params:jmap:error:limit"}))]);
    assert!(matches!(block_on(upload(&client, vec![0; 5])), Err(Error::TooLarge(_))));
    for (what, result) in [
        ("an id that is none", json!({"created": {"m": {"id": "E 1"}}})),
        ("no id", json!({"created": {"m": {}}})),
        ("nothing said of it", json!({"created": {}, "notCreated": {}})),
        ("a refusal", json!({"notCreated": {"m": {"type": "overQuota"}}})),
    ] {
        let (client, _) = scripted(vec![answer("Email/import", result)]);
        assert!(block_on(import(&client, "U1", "Msent", &["$seen"])).is_err(), "{what}");
    }
}

#[test]
fn what_a_server_says_in_a_refusal_it_has_no_word_for_is_cleaned_before_the_owner_reads_it() {
    let say = |text: &str| {
        let refusal = Refusal { kind: "odd\u{7}type/with junk".into(), description: text.into(), ..Default::default() };
        let secrets = Secrets::default();
        let (client, _) = scripted(vec![]);
        let outgoing = Outgoing::new(&client, None, None, secrets);
        words(&refusal, None, &|t| outgoing.said(t))
    };
    // No control character, no address, nothing longer than a line.
    let said = say("Try\r\nagain bob@example.org please \u{1b}[31m red");
    assert_eq!(said, "the server refused it (oddtypewithjunk): Try again *** please [31m red");
    let long = say(&"word ".repeat(200));
    assert!(long.chars().count() < 260 && !long.contains('\n'), "{long}");
    assert_eq!(words(&Refusal::default(), None, &|_| String::new()), "the server refused it");
}
