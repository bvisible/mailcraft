// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's Outbox and drafts from end to end: the engine writes the message and says what a run should
//! send (`jmap_mail_request`), `mailcraft_jmap::sync_mail` sends and files it over the in-process server (of
//! either kind: a store of the standard kind, and a server that is a layer in front of an IMAP and an SMTP
//! server), driven by `block_on` as a host on one thread drives it, and `apply_jmap_mail` takes in each piece
//! as it comes. Nothing here leaves this machine; every name, address and id is invented.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicBool;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::jmap_mail::{apply_jmap_mail, jmap_mail_request};
use mailcraft_jmap::standin::{Effect, Fault, Profile, StandIn, TOKEN};
use mailcraft_jmap::{Auth, Client, block_on, sync_mail};
use mailcraft_model::{FolderKind, Id, Mailbox, Message};
use serde_json::{Value, json};

const ADDRESS: &str = "ada@example.org";
const SESSION: &str = "https://jmap.standin.example/jmap/session?account=ada%40example.org";

fn both() -> [(&'static str, Profile); 2] {
    [("a store of the standard kind", Profile::standard()), ("a server in front of IMAP and SMTP", Profile::by_place())]
}

/// A session with one JMAP account, and an in-process server for it, whose folders the account has listed.
struct Rig {
    server: StandIn,
    s: Session,
    account: Id,
    inbox: String,
}

impl Rig {
    fn new(profile: Profile) -> Rig {
        let mut s = Session::at(Mailbox::with_local_account("", ""), 1_800_000_000, 0);
        let added = s.run("account.add", &json!({"email": ADDRESS, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
        let server = StandIn::new();
        let inbox = server.with(|d| {
            d.profile = profile;
            d.mailbox("Drafts", None, Some("drafts"));
            d.mailbox("Sent", None, Some("sent"));
            d.mailbox("Trash", None, Some("trash"));
            d.identity("Ada", ADDRESS);
            d.mailbox("Inbox", None, Some("inbox"))
        });
        let mut rig = Rig { server, s, account: added["id"].as_u64().unwrap(), inbox };
        // The first run lists the mailboxes, so that the account's folders are the server's.
        rig.run(true);
        rig
    }

    /// One run, as a host does it: what the engine asks for (`full`: every mailbox; else the Inbox, and what the
    /// run needs to show), done against the server, each piece taken in as it arrives. What taking each in answered.
    fn run(&mut self, full: bool) -> Vec<Value> {
        let request = jmap_mail_request(&self.s, self.account, full, &[], false).unwrap();
        let client = block_on(Client::connect(self.server.clone(), &request.session, &Auth::bearer(TOKEN))).unwrap();
        let mut answers = Vec::new();
        let (s, cancel) = (&mut self.s, AtomicBool::new(false));
        block_on(sync_mail(&client, &request, &cancel, &mut |piece| answers.push(apply_jmap_mail(s, piece).unwrap())));
        answers
    }

    /// Write a message from the account and press Send: its id in the Outbox.
    fn send(&mut self, to: &str, bcc: &str, subject: &str) -> Id {
        self.s
            .run("mail.new", &json!({"to": to, "bcc": bcc, "subject": subject, "body": format!("Hello about {subject}."), "account": self.account}))
            .unwrap();
        self.s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap()
    }

    fn folder(&self, kind: FolderKind) -> Id {
        self.s.mailbox.folder_of_kind(self.account, kind).unwrap()
    }

    /// The messages in a folder here, newest first.
    fn mail(&self, kind: FolderKind) -> Vec<&Message> {
        let folder = self.folder(kind);
        let mut found: Vec<&Message> = self.s.mailbox.messages.values().map(|m| m.as_ref()).filter(|m| m.folder == folder).collect();
        found.sort_by_key(|m| std::cmp::Reverse((m.received, m.id)));
        found
    }

    /// The server's mailbox of this kind, by role.
    fn there(&self, role: &str) -> String {
        self.server.with(|d| d.mailboxes.iter().find(|m| m.role.as_deref() == Some(role)).map(|m| m.id.clone())).unwrap()
    }

    /// The emails the server holds in a mailbox.
    fn held_in(&self, role: &str) -> Vec<mailcraft_jmap::standin::Email> {
        let mailbox = self.there(role);
        self.server.with(|d| d.emails.iter().filter(|e| e.mailboxes.contains(&mailbox)).cloned().collect())
    }

    fn delivered(&self) -> Vec<mailcraft_jmap::standin::Delivery> {
        self.server.with(|d| d.delivered.clone())
    }

    fn fault(&self, on: &'static str, effect: Effect) {
        self.server.with(|d| d.faults.push_back(Fault::on(on, effect)));
    }
}

fn subjects(messages: &[&Message]) -> Vec<String> {
    messages.iter().map(|m| m.subject.clone()).collect()
}

fn text_of(email: &mailcraft_jmap::standin::Email) -> String {
    String::from_utf8_lossy(&email.raw).to_string()
}

#[test]
fn a_message_written_here_is_sent_to_everyone_it_names_and_comes_back_as_the_servers_copy_in_sent() {
    for (kind, profile) in both() {
        let mut rig = Rig::new(profile);
        let message = rig.send("bob@example.org, Carol <carol@example.org>, bob@example.org", "eve@example.org", "Plans");
        let message_id = rig.s.mailbox.message(message).unwrap().message_id.clone();
        assert_eq!(rig.mail(FolderKind::Outbox).len(), 1, "{kind}");

        let answers = rig.run(false);
        // The pieces: what became of the message, then the windows that were read (the Inbox, and Sent for its copy).
        assert!(answers.iter().all(|a| a["errors"] == json!([])), "{kind}: {answers:?}");
        assert_eq!(answers.last().unwrap()["authRefused"], false);
        // It left the Outbox, and the owner's own copy is the server's, in Sent, read, with the message's Message-ID.
        assert!(rig.mail(FolderKind::Outbox).is_empty(), "{kind}");
        let sent = rig.mail(FolderKind::Sent);
        assert_eq!(subjects(&sent), ["Plans"], "{kind}");
        assert!(sent[0].read && sent[0].server.is_some() && sent[0].unfetched, "{kind}");
        assert_eq!(sent[0].message_id, message_id, "{kind}");
        assert!(!sent[0].bcc.iter().any(|a| a.email.contains("eve")), "{kind}: no Bcc in what the server filed");

        // The server sent it to the three, once each, from the account, and the Bcc never left.
        let delivered = rig.delivered();
        assert_eq!(delivered.len(), 1, "{kind}");
        assert_eq!(delivered[0].from, ADDRESS, "{kind}");
        assert_eq!(delivered[0].recipients, ["bob@example.org", "carol@example.org", "eve@example.org"], "{kind}");
        let left = String::from_utf8_lossy(&delivered[0].raw).to_string();
        assert!(left.contains("Subject: Plans") && left.contains("Hello about Plans."), "{kind}: {left}");
        assert!(left.contains("To: ") && !left.to_ascii_lowercase().contains("bcc") && !left.contains("eve@"), "{kind}: {left}");
        // The copy in Sent is that message, and the only one.
        assert_eq!(rig.held_in("sent").len(), 1, "{kind}");
        assert_eq!(text_of(&rig.held_in("sent")[0]), left, "{kind}");

        // The next run has nothing to send, and sends nothing: the message went once.
        let request = jmap_mail_request(&rig.s, rig.account, false, &[], false).unwrap();
        assert!(request.sends.is_empty() && request.drafts.is_empty(), "{kind}");
        rig.run(false);
        assert_eq!(rig.delivered().len(), 1, "{kind}");
        assert_eq!(rig.mail(FolderKind::Sent).len(), 1, "{kind}");
    }
}

#[test]
fn a_reply_that_went_marks_what_it_answers_and_not_before() {
    let mut rig = Rig::new(Profile::standard());
    let raw = format!(
        "From: Bo <bo@example.org>\r\nTo: Ada <{ADDRESS}>\r\nSubject: Question\r\nMessage-ID: <q1@example.org>\r\nDate: Thu, 15 Oct 2026 09:30:00 +0200\r\n\r\nWhen?\r\n"
    );
    rig.server.with(|d| d.deliver(&rig.inbox, raw.as_bytes(), &[], "2026-10-15T07:30:00Z"));
    rig.run(true);
    let question = rig.mail(FolderKind::Inbox)[0].id;
    // The reply is written whole from the original, so the original is fetched first (here it is, so it can be).
    rig.s.mailbox.message_mut(question).unwrap().unfetched = false;
    rig.s.run("mail.reply", &json!({"id": question})).unwrap();
    rig.s.run("compose.set", &json!({"body": "Tomorrow."})).unwrap();
    let reply = rig.s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
    assert!(!rig.s.mailbox.message(question).unwrap().answered, "not when Send is pressed");
    // A refused reply does not mark it either.
    rig.fault("EmailSubmission/set", Effect::SetError("forbiddenToSend", "Not today."));
    rig.run(false);
    assert!(!rig.s.mailbox.message(question).unwrap().answered && !rig.s.mailbox.message(reply).unwrap().held.is_empty());
    rig.s.run("mail.sendAgain", &json!({"id": reply})).unwrap();
    rig.run(false);
    assert!(rig.s.mailbox.message(question).unwrap().answered, "once it has gone");
    assert!(rig.mail(FolderKind::Outbox).is_empty());
    let delivered = rig.delivered();
    assert_eq!((delivered.len(), delivered[0].recipients.clone()), (1, vec!["bo@example.org".to_string()]));
    assert!(text_of(&rig.held_in("sent")[0]).contains("In-Reply-To: <q1@example.org>"));
}

#[test]
fn a_message_the_server_refuses_is_held_with_why_leaves_nothing_in_sent_and_goes_when_the_owner_sends_it_again() {
    for (kind, profile) in both() {
        let mut rig = Rig::new(profile);
        let message = rig.send("bob@example.org", "", "Plans");
        rig.fault("EmailSubmission/set", Effect::SetError("forbiddenToSend", "Not today."));
        let answers = rig.run(false);
        let held = rig.s.mailbox.message(message).unwrap().held.clone();
        assert_eq!(held, "It wasn't sent: the server doesn't let this account send mail just now. Change it, or send it again.", "{kind}");
        assert!(answers[0]["errors"][0].as_str().unwrap().starts_with("Couldn't send 'Plans'"), "{kind}: {answers:?}");
        // Nothing looks sent: not in Sent here, not on the server, not delivered.
        assert!(rig.mail(FolderKind::Sent).is_empty() && rig.held_in("sent").is_empty() && rig.delivered().is_empty(), "{kind}");
        assert_eq!(rig.mail(FolderKind::Outbox).len(), 1, "{kind}");
        // Run after run, it is left alone.
        rig.run(false);
        rig.run(true);
        assert!(rig.delivered().is_empty() && rig.mail(FolderKind::Outbox).len() == 1, "{kind}");
        // The owner's say-so sends it, once.
        rig.s.run("mail.sendAgain", &json!({"id": message})).unwrap();
        rig.run(false);
        assert_eq!(rig.delivered().len(), 1, "{kind}");
        assert!(rig.mail(FolderKind::Outbox).is_empty(), "{kind}");
        assert_eq!(subjects(&rig.mail(FolderKind::Sent)), ["Plans"], "{kind}");
    }
}

#[test]
fn an_answer_lost_after_the_message_went_holds_it_and_it_is_never_sent_again_by_a_run() {
    for (kind, profile) in both() {
        let mut rig = Rig::new(profile);
        let message = rig.send("bob@example.org", "", "Plans");
        rig.fault("EmailSubmission/set", Effect::LoseAnswer);
        rig.run(false);
        let held = rig.s.mailbox.message(message).unwrap().held.clone();
        assert!(held.starts_with("It may have been sent"), "{kind}: {held}");
        // It did go, and the server's copy is in Sent, which is how the owner sees it.
        assert_eq!(rig.delivered().len(), 1, "{kind}");
        assert_eq!(subjects(&rig.mail(FolderKind::Sent)), ["Plans"], "{kind}");
        // Runs, one after another, light or full: it goes no more, and the Outbox still has it, held.
        for full in [false, true, false, true] {
            rig.run(full);
        }
        assert_eq!(rig.delivered().len(), 1, "{kind}: never twice");
        assert_eq!(rig.mail(FolderKind::Outbox).len(), 1, "{kind}");
        assert_eq!(rig.held_in("sent").len(), 1, "{kind}");
    }
}

#[test]
fn a_server_that_sends_and_then_says_it_failed_holds_the_message_as_one_that_may_have_gone() {
    // The real server hands the message on and then fails (its goodbye to the mail server), and says `serverFail`.
    for (kind, profile) in both() {
        let mut rig = Rig::new(profile);
        let message = rig.send("bob@example.org", "", "Plans");
        rig.fault("EmailSubmission/set", Effect::SentThenSetError("serverFail", "The mail server did not take the message: goodbye failed."));
        let answers = rig.run(false);
        let held = rig.s.mailbox.message(message).unwrap().held.clone();
        assert!(
            held.starts_with(
                "It may have been sent: the server failed while it was sending it: The mail server did not take the message: goodbye failed."
            ),
            "{kind}: {held}"
        );
        // It did go, and its copy stays in Sent: the owner's record of it, which is not taken out for a message that went.
        assert_eq!(rig.delivered().len(), 1, "{kind}");
        assert_eq!(subjects(&rig.mail(FolderKind::Sent)), ["Plans"], "{kind}");
        assert!(answers.iter().any(|a| a["errors"][0].as_str().is_some_and(|l| l.starts_with("'Plans' may have been sent"))), "{kind}: {answers:?}");
        // Run after run, light or full: it goes no more.
        for full in [false, true, false] {
            rig.run(full);
        }
        assert_eq!((rig.delivered().len(), rig.held_in("sent").len(), rig.mail(FolderKind::Outbox).len()), (1, 1, 1), "{kind}: never twice");
    }
}

#[test]
fn a_copy_the_server_will_not_take_out_of_sent_waits_to_be_destroyed_and_is_not_shown_as_mail() {
    for (kind, profile) in both() {
        let mut rig = Rig::new(profile);
        let message = rig.send("bob@example.org", "", "Plans");
        rig.fault("EmailSubmission/set", Effect::SetError("forbiddenToSend", "Not today."));
        rig.fault("Email/set", Effect::MethodError("serverFail"));
        rig.run(false);
        assert!(!rig.s.mailbox.message(message).unwrap().held.is_empty(), "{kind}");
        // The message did not go. The copy filed for it is still on the server, since it could not be taken out; it
        // waits in the queue, and is not shown as a message that was sent.
        assert!(rig.delivered().is_empty() && rig.held_in("sent").len() == 1, "{kind}");
        assert_eq!(rig.s.mailbox.queued.len(), 1, "{kind}");
        // (Full runs: they read Sent, where the copy is.)
        for _ in 0..2 {
            rig.run(true);
            assert!(rig.mail(FolderKind::Sent).is_empty(), "{kind}: not mail");
        }
    }
}

#[test]
fn trouble_before_the_message_goes_costs_it_nothing_it_waits_and_goes_with_the_next_run() {
    let cases: [(&str, &str, Effect); 4] = [
        ("the network lost on the upload", "upload", Effect::LoseAnswer),
        ("a busy server", "upload", Effect::RefuseWith(503)),
        ("who the account is lost", "Identity/get", Effect::LoseAnswer),
        ("filing unavailable", "Email/import", Effect::MethodError("serverUnavailable")),
    ];
    for (kind, profile) in both() {
        for (what, on, effect) in cases {
            let mut rig = Rig::new(profile);
            let message = rig.send("bob@example.org", "", "Plans");
            rig.fault(on, effect);
            rig.run(false);
            // Nothing was said of the message: it is in the Outbox as it was, not held, and nothing went.
            assert!(rig.s.mailbox.message(message).unwrap().held.is_empty(), "{kind}, {what}");
            assert_eq!(rig.mail(FolderKind::Outbox).len(), 1, "{kind}, {what}");
            assert!(rig.delivered().is_empty(), "{kind}, {what}");
            // The next run sends it, once.
            rig.run(false);
            assert_eq!(rig.delivered().len(), 1, "{kind}, {what}");
            assert!(rig.mail(FolderKind::Outbox).is_empty() && rig.mail(FolderKind::Sent).len() == 1, "{kind}, {what}");
        }
    }
}

#[test]
fn a_server_that_limits_sending_leaves_the_rest_in_the_outbox_for_a_later_run() {
    let mut rig = Rig::new(Profile::by_place());
    rig.server.with(|d| d.sent_lately = 29);
    let (one, two, three) =
        (rig.send("bob@example.org", "", "One"), rig.send("bob@example.org", "", "Two"), rig.send("bob@example.org", "", "Three"));
    let answers = rig.run(false);
    // The first goes; the second meets the limit; the third is not tried. None is held: they wait.
    assert_eq!(rig.delivered().len(), 1);
    assert_eq!(subjects(&rig.mail(FolderKind::Sent)), ["One"]);
    assert!(rig.mail(FolderKind::Outbox).iter().all(|m| m.held.is_empty()));
    assert_eq!(rig.mail(FolderKind::Outbox).iter().map(|m| m.id).collect::<std::collections::BTreeSet<_>>(), [two, three].into_iter().collect());
    assert!(
        answers.iter().any(|a| a["errors"]
            .as_array()
            .is_some_and(|e| e.iter().any(|l| l.as_str().is_some_and(|l| l.starts_with("The server limits how many messages"))))),
        "{answers:?}"
    );
    assert!(rig.s.mailbox.message(one).is_none());
    // Later, when the server will have it, they go.
    rig.server.with(|d| d.sent_lately = 0);
    rig.run(false);
    assert_eq!(rig.delivered().len(), 3);
    assert!(rig.mail(FolderKind::Outbox).is_empty());
}

#[test]
fn a_draft_is_filed_on_the_server_replaced_by_the_next_save_and_destroyed_when_it_goes() {
    for (kind, profile) in both() {
        let mut rig = Rig::new(profile);
        rig.s
            .run(
                "mail.new",
                &json!({"to": "bob@example.org", "bcc": "eve@example.org", "subject": "Plans", "body": "First words.", "account": rig.account}),
            )
            .unwrap();
        let draft = rig.s.run("compose.saveDraft", &json!({})).unwrap()["draft"].as_u64().unwrap();
        let answers = rig.run(false);
        assert!(answers.iter().all(|a| a["errors"] == json!([])), "{kind}: {answers:?}");
        // The server holds it in Drafts, as a draft, with its Bcc (a draft is its owner's), and it is the one draft here.
        let on_server = rig.held_in("drafts");
        assert_eq!(on_server.len(), 1, "{kind}");
        assert!(on_server[0].keywords.contains("$draft") && on_server[0].keywords.contains("$seen"), "{kind}");
        assert!(text_of(&on_server[0]).contains("eve@example.org") && text_of(&on_server[0]).contains("First words."), "{kind}");
        let here = rig.mail(FolderKind::Drafts);
        assert_eq!(here.len(), 1, "{kind}: the window found the copy as the draft it is");
        assert_eq!((here[0].id, here[0].server.as_ref().map(|s| s.id.clone())), (draft, Some(on_server[0].id.clone())), "{kind}");
        assert!(!here[0].server_draft.as_ref().unwrap().pending, "{kind}");
        assert!(rig.delivered().is_empty() && rig.mail(FolderKind::Sent).is_empty(), "{kind}: a draft goes nowhere");

        // Edited and saved again: the server's copy is replaced, not added to.
        rig.s.run("mail.editDraft", &json!({"id": draft})).unwrap();
        rig.s.run("compose.set", &json!({"body": "Second words."})).unwrap();
        rig.s.run("compose.saveDraft", &json!({})).unwrap();
        rig.run(false);
        let on_server = rig.held_in("drafts");
        assert_eq!(on_server.len(), 1, "{kind}: one copy");
        assert!(text_of(&on_server[0]).contains("Second words.") && !text_of(&on_server[0]).contains("First words."), "{kind}");
        let here = rig.mail(FolderKind::Drafts);
        assert_eq!((here.len(), here[0].id, here[0].server.as_ref().map(|s| s.id.clone())), (1, draft, Some(on_server[0].id.clone())), "{kind}");
        // Another run changes nothing: it is not filed again.
        let calls = rig.server.calls().len();
        rig.run(false);
        assert!(!rig.server.calls()[calls..].iter().any(|call| call == "Email/import"), "{kind}");

        // Sent from the draft: the message goes, and the draft's copy goes with it.
        rig.s.run("mail.editDraft", &json!({"id": draft})).unwrap();
        let message = rig.s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
        assert!(rig.s.mailbox.message(draft).is_none(), "{kind}: the draft became the message");
        assert_eq!(rig.held_in("drafts").len(), 1, "{kind}: its copy stays until the message has gone");
        rig.run(false);
        assert!(rig.held_in("drafts").is_empty(), "{kind}: destroyed once it went");
        assert_eq!(rig.delivered().len(), 1, "{kind}");
        assert!(rig.mail(FolderKind::Drafts).is_empty() && rig.mail(FolderKind::Outbox).is_empty(), "{kind}");
        assert_eq!(subjects(&rig.mail(FolderKind::Sent)), ["Plans"], "{kind}");
        let _ = message;
    }
}

#[test]
fn a_draft_whose_message_is_held_stays_on_the_server_for_the_owner() {
    let mut rig = Rig::new(Profile::standard());
    rig.s.run("mail.new", &json!({"to": "bob@example.org", "subject": "Plans", "body": "Words.", "account": rig.account})).unwrap();
    let draft = rig.s.run("compose.saveDraft", &json!({})).unwrap()["draft"].as_u64().unwrap();
    rig.run(false);
    rig.s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    let message = rig.s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
    rig.fault("EmailSubmission/set", Effect::SetError("forbiddenToSend", "Not today."));
    rig.run(false);
    assert!(!rig.s.mailbox.message(message).unwrap().held.is_empty());
    assert_eq!(rig.held_in("drafts").len(), 1, "a message that did not go leaves its draft where it was");
    rig.s.run("mail.sendAgain", &json!({"id": message})).unwrap();
    rig.run(false);
    assert!(rig.held_in("drafts").is_empty() && rig.delivered().len() == 1);
}

#[test]
fn a_draft_discarded_is_destroyed_on_the_server_by_the_next_run_and_never_comes_back() {
    let mut rig = Rig::new(Profile::standard());
    rig.s.run("mail.new", &json!({"to": "bob@example.org", "subject": "Plans", "body": "Words.", "account": rig.account})).unwrap();
    let draft = rig.s.run("compose.saveDraft", &json!({})).unwrap()["draft"].as_u64().unwrap();
    rig.run(false);
    let copy = rig.held_in("drafts")[0].id.clone();
    rig.s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    rig.s.run("compose.discard", &json!({})).unwrap();
    assert!(rig.s.mailbox.message(draft).is_none());
    assert_eq!(rig.s.mailbox.queued.len(), 1);
    assert!(matches!(&rig.s.mailbox.queued[0], mailcraft_model::Queued::DeleteMessage { id, .. } if *id == copy));
    // The next run sends the owner's changes up first, the copies that wait in the queue among them: the server's copy
    // is destroyed, and run after run nothing brings it down as a draft the owner has not got.
    // (Full runs: they read Drafts, where the copy was.)
    for _ in 0..3 {
        rig.run(true);
        assert!(rig.mail(FolderKind::Drafts).is_empty(), "the discarded draft stays discarded");
    }
    assert!(rig.s.mailbox.queued.is_empty(), "the destroy went up: {:?}", rig.s.mailbox.queued);
    assert!(rig.held_in("drafts").is_empty(), "the server's copy is gone");
}

#[test]
fn a_message_with_a_file_goes_whole_and_a_hostile_server_gets_nothing_it_was_not_given() {
    let mut rig = Rig::new(Profile::by_place());
    rig.s.run("mail.new", &json!({"to": "bob@example.org", "subject": "Plan", "body": "See the file.", "account": rig.account})).unwrap();
    let data = (0..=255u8).cycle().take(5_000).collect::<Vec<u8>>();
    let encoded = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&data)
    };
    rig.s.run("compose.attach", &json!({"name": "plan.bin", "mime": "application/octet-stream", "data": encoded})).unwrap();
    rig.s.run("compose.send", &json!({})).unwrap();
    rig.run(false);
    let delivered = rig.delivered();
    assert_eq!(delivered.len(), 1);
    let left = String::from_utf8_lossy(&delivered[0].raw).to_string();
    assert!(left.contains("plan.bin") && left.contains("application/octet-stream") && left.contains(&encoded[..60]), "{left}");
    // No secret of the account is anywhere in what the server was sent.
    assert!(!left.contains(TOKEN));
}
