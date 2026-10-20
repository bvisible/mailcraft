// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's Outbox and drafts, as the engine works out what a run sends and files and takes in what
//! became of each: the answers are made by hand, so that every outcome can be had. Invented people at
//! `example.org`.

use mailcraft_model::sync::{FolderListing, GraphFolder, GraphMessage, JmapMailRequest, JmapMailResult, JmapScope, MessagePage};
use mailcraft_model::{Address, FolderKind};
use serde_json::{Value, json};

use super::*;
use crate::cmd::jmap_mail::{apply_jmap_mail, jmap_mail_request};

const ME: &str = "ada@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session";

/// A session with one JMAP account that is tied to its server (it has been listed): its id.
fn session() -> (Session, Id) {
    let mut s = Session::empty();
    // A host that runs a JMAP account's Send/Receive (this fork's browser build).
    s.jmap_runs = true;
    let added = s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
    let account = added["id"].as_u64().unwrap();
    let folder = |id: &str, name: &str, kind: FolderKind| GraphFolder { id: id.into(), name: name.into(), kind, ..Default::default() };
    let listing = FolderListing {
        folders: vec![
            folder("Min", "INBOX", FolderKind::Inbox),
            folder("Mdr", "Drafts", FolderKind::Drafts),
            folder("Mse", "Sent", FolderKind::Sent),
            folder("Mtr", "Trash", FolderKind::Deleted),
            folder("Mpr", "Projects", FolderKind::Custom),
        ],
        complete: true,
    };
    apply_jmap_mail(&mut s, JmapMailResult { account, email: ME.into(), listing: Some(listing), ..Default::default() }).unwrap();
    (s, account)
}

fn request(s: &Session, account: Id) -> JmapMailRequest {
    jmap_mail_request(s, account, false, &[], false).unwrap()
}

fn outbox(s: &Session, account: Id) -> Id {
    s.mailbox.folder_of_kind(account, FolderKind::Outbox).unwrap()
}

/// Write a message and send it: its id in the Outbox.
fn send(s: &mut Session, account: Id, to: &str, subject: &str) -> Id {
    s.run("mail.new", &json!({"to": to, "subject": subject, "body": "Hi", "account": account})).unwrap();
    s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap()
}

/// What a run answers about messages and drafts.
fn answer(s: &mut Session, account: Id, sent: Vec<SendDone>, drafts: Vec<JmapDraftDone>) -> Value {
    apply_jmap_mail(s, JmapMailResult { account, email: ME.into(), sent, drafts, ..Default::default() }).unwrap()
}

fn outcome(message: Id, outcome: SendOutcome) -> SendDone {
    SendDone { message, outcome, draft: None }
}

fn held(lost: bool, why: &str) -> SendOutcome {
    SendOutcome::Held { lost, why: why.into() }
}

// ------------------------------------------------------------------------------------------------
// Sending

#[test]
fn what_waits_in_the_outbox_goes_with_the_run_whole_and_a_held_message_does_not() {
    let (mut s, account) = session();
    let (one, two, three) = (
        send(&mut s, account, "bob@example.org", "One"),
        send(&mut s, account, "carol@example.org", "Two"),
        send(&mut s, account, "dave@example.org", "Three"),
    );
    s.mailbox.message_mut(two).unwrap().held = "It wasn't sent: whatever.".into();
    let request = request(&s, account);
    // In the order they were written, the held one left out, each with everything it is.
    assert_eq!(request.sends.iter().map(|send| send.message).collect::<Vec<_>>(), [one, three]);
    assert_eq!(request.sends[0].mime.subject, "One");
    assert_eq!(request.sends[0].mime.to[0].email, "bob@example.org");
    assert_eq!(request.sends[0].mime.from.email, ME);
    assert!(request.sends[0].draft_copy.is_empty());
    // Sent, the Outbox is what it was; the request says it is for a JMAP account, and prints no address.
    let shown = format!("{request:?} {:?}", request.sends);
    assert!(!shown.contains('@') && !shown.contains("example.org") && shown.contains("sends: 2"), "{shown}");
    // Other accounts' mail is not this one's.
    let imap = s.run("account.add", &json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    let theirs = send(&mut s, imap, "carol@example.org", "Theirs");
    assert_eq!(sends(&s, account).iter().map(|send| send.message).collect::<Vec<_>>(), [one, three]);
    assert!(s.mailbox.message(theirs).is_some());
}

#[test]
fn a_run_sends_ten_messages_at_most() {
    let (mut s, account) = session();
    let outbox = outbox(&s, account);
    for n in 0..13 {
        let m = Message { account, folder: outbox, to: vec![Address::new("Bob", "bob@example.org")], subject: format!("m{n}"), ..Default::default() };
        s.mailbox.add_message(m).unwrap();
    }
    let sends = request(&s, account).sends;
    assert_eq!(sends.len(), MAX_JMAP_SENDS);
    assert_eq!(sends.iter().map(|send| send.mime.subject.as_str()).collect::<Vec<_>>(), (0..10).map(|n| format!("m{n}")).collect::<Vec<_>>());
}

#[test]
fn a_message_that_was_sent_leaves_the_outbox_and_what_it_answers_is_marked() {
    let (mut s, account) = session();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let theirs = |subject: &str, id: &str| Message {
        account,
        folder: inbox,
        message_id: format!("{id}@example.org"),
        from: Address::new("Bo", "bo@example.org"),
        to: vec![Address::new("Ada", ME)],
        subject: subject.into(),
        ..Default::default()
    };
    let (asked, passed_on) = (s.mailbox.add_message(theirs("Question", "q1")).unwrap(), s.mailbox.add_message(theirs("News", "n1")).unwrap());
    s.run("mail.reply", &json!({"id": asked})).unwrap();
    let reply = s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.run("mail.forward", &json!({"id": passed_on})).unwrap();
    s.run("compose.set", &json!({"to": "carol@example.org"})).unwrap();
    let forward = s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
    let plain = send(&mut s, account, "dave@example.org", "Plain");
    // Not yet: the originals are marked once the messages have gone, not when Send is pressed (#154).
    assert!(!s.mailbox.message(asked).unwrap().answered && !s.mailbox.message(passed_on).unwrap().forwarded);
    // Something to undo: it ends with a message that reached the server.
    s.run("mail.flag", &json!({"id": asked})).unwrap();
    assert!(s.undo_label().is_some());

    let said = answer(
        &mut s,
        account,
        vec![outcome(reply, SendOutcome::Sent), outcome(forward, SendOutcome::Sent), outcome(plain, SendOutcome::Sent)],
        vec![],
    );
    let outbox = outbox(&s, account);
    assert_eq!(s.mailbox.messages_in(outbox).count(), 0);
    assert!(s.mailbox.message(asked).unwrap().answered && !s.mailbox.message(asked).unwrap().forwarded);
    assert!(s.mailbox.message(passed_on).unwrap().forwarded && !s.mailbox.message(passed_on).unwrap().answered);
    assert_eq!(said["errors"], json!([]));
    assert!(said["updated"].as_u64().unwrap() >= 3, "{said}");
    // Mail that has gone can't be taken back by Undo.
    assert!(s.undo_label().is_none());
    // And the run no longer has them to send.
    assert!(request(&s, account).sends.is_empty());
    // The same answer taken in twice changes no message: the second time the message is not in the Outbox, which is
    // all that can be told of it, and a message that went is said to have, though it is gone from here.
    let before = s.mailbox.messages.len();
    let again = answer(&mut s, account, vec![outcome(reply, SendOutcome::Sent)], vec![]);
    assert_eq!(again["errors"], json!(["A message was sent, although it was taken out of the Outbox meanwhile. Look in Sent."]));
    assert_eq!(s.mailbox.messages.len(), before);
}

#[test]
fn a_message_the_server_refused_is_held_with_why_and_sent_again_only_when_the_owner_says_so() {
    let (mut s, account) = session();
    let one = send(&mut s, account, "bob@example.org", "One");
    let two = send(&mut s, account, "carol@example.org", "Two");
    let said = answer(&mut s, account, vec![outcome(one, held(false, "the server doesn't accept bob@example.org"))], vec![]);
    let m = s.mailbox.message(one).unwrap();
    assert_eq!(m.held, "It wasn't sent: the server doesn't accept bob@example.org. Change it, or send it again.");
    // The owner is told, ahead of anything else, and the status line says it.
    // (The line goes to the log as well, and leaves the address out; the message keeps it.)
    assert_eq!(said["errors"][0], "Couldn't send 'One': the server doesn't accept (address). It is held in the Outbox.");
    assert!(s.sync_status.is_empty() || s.sync_status.contains("Couldn't send 'One'"), "{}", s.sync_status);
    // It stays in the Outbox, and the run no longer carries it; the other still goes.
    assert!(s.mailbox.message(one).is_some());
    assert_eq!(request(&s, account).sends.iter().map(|send| send.message).collect::<Vec<_>>(), [two]);
    // Send Again puts it back.
    s.run("mail.sendAgain", &json!({"id": one})).unwrap();
    assert!(s.mailbox.message(one).unwrap().held.is_empty());
    assert_eq!(request(&s, account).sends.iter().map(|send| send.message).collect::<Vec<_>>(), [one, two]);
}

#[test]
fn a_message_whose_answer_was_lost_is_held_as_one_that_may_have_gone_and_never_sent_again_by_itself() {
    let (mut s, account) = session();
    let one = send(&mut s, account, "bob@example.org", "One");
    let said = answer(&mut s, account, vec![outcome(one, held(true, ""))], vec![]);
    let m = s.mailbox.message(one).unwrap();
    assert!(m.held.starts_with("It may have been sent: the server's answer was lost."), "{}", m.held);
    assert!(m.held.contains("ask a recipient"), "{}", m.held);
    assert_eq!(said["errors"][0], "'One' may have been sent: the server's answer was lost. It is held in the Outbox.");
    // Run after run, it is not asked for again.
    for _ in 0..3 {
        assert!(request(&s, account).sends.is_empty());
        answer(&mut s, account, vec![], vec![]);
    }
    // Only the owner can: Send Again.
    s.run("mail.sendAgain", &json!({"id": one})).unwrap();
    assert_eq!(request(&s, account).sends.len(), 1);
}

#[test]
fn a_message_that_waits_changes_nothing_and_goes_with_the_next_run() {
    let (mut s, account) = session();
    let one = send(&mut s, account, "bob@example.org", "One");
    let said = answer(&mut s, account, vec![outcome(one, SendOutcome::Waits)], vec![]);
    assert!(s.mailbox.message(one).unwrap().held.is_empty());
    assert_eq!((said["updated"].as_u64(), said["errors"].clone()), (Some(0), json!([])));
    assert_eq!(request(&s, account).sends.len(), 1);
}

#[test]
fn what_a_run_says_about_other_mail_than_the_outbox_changes_no_message() {
    let (mut s, account) = session();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    // Mail the server has, as received mail is.
    let on_the_server = Some(ServerMessage { id: "E1".into(), folder: "Min".into(), ..Default::default() });
    let received =
        s.mailbox.add_message(Message { account, folder: inbox, subject: "Received".into(), server: on_the_server, ..Default::default() }).unwrap();
    let draft = s
        .mailbox
        .add_message(Message { account, folder: s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap(), draft: true, ..Default::default() })
        .unwrap();
    // The Outbox of another account.
    let imap = s.run("account.add", &json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    let theirs = send(&mut s, imap, "carol@example.org", "Theirs");
    let ours = send(&mut s, account, "bob@example.org", "Ours");
    let before = s.mailbox.messages.len();
    for target in [received, draft, theirs, 9_999] {
        answer(&mut s, account, vec![outcome(target, SendOutcome::Sent), outcome(target, held(true, ""))], vec![]);
    }
    assert_eq!(s.mailbox.messages.len(), before, "nothing was forgotten");
    // None of them is held. (An answer about a message of the account that is not in its Outbox says that it went, and
    // one about another account's, or about none, says so without a name: see the test of that.)
    for (name, m) in [("received", received), ("draft", draft), ("theirs", theirs), ("ours", ours)] {
        assert!(s.mailbox.message(m).unwrap().held.is_empty(), "{name}");
    }
    // A reason of any size and any characters is shown as a line.
    let long = "word\u{7} ".repeat(200);
    answer(&mut s, account, vec![outcome(ours, held(false, &long))], vec![]);
    let text = &s.mailbox.message(ours).unwrap().held;
    assert!(text.chars().count() < 360 && !text.chars().any(char::is_control), "{text}");
    // One without a reason says so.
    let other = send(&mut s, account, "bob@example.org", "Other");
    answer(&mut s, account, vec![outcome(other, held(false, "  "))], vec![]);
    assert!(s.mailbox.message(other).unwrap().held.contains("the server wouldn't take it"));
}

#[test]
fn a_message_that_may_have_gone_says_why_it_is_not_known() {
    let (mut s, account) = session();
    let one = send(&mut s, account, "bob@example.org", "One");
    let said = answer(&mut s, account, vec![outcome(one, held(true, "the server failed while it was sending it"))], vec![]);
    let text = &s.mailbox.message(one).unwrap().held;
    assert!(text.starts_with("It may have been sent: the server failed while it was sending it. Before you send it again"), "{text}");
    assert_eq!(said["errors"][0], "'One' may have been sent: the server failed while it was sending it. It is held in the Outbox.");
}

#[test]
fn a_message_goes_with_its_bcc_for_the_envelope_and_the_request_shows_none_of_it() {
    let (mut s, account) = session();
    s.run("mail.new", &json!({"to": "bob@example.org", "bcc": "eve@example.org", "subject": "Quiet", "body": "Hi", "account": account})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let request = request(&s, account);
    // (The host writes the message without that field, and tells the server of the address in the envelope.)
    assert_eq!(request.sends[0].mime.bcc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["eve@example.org"]);
    let shown = format!("{request:?} {:?}", request.sends);
    assert!(!shown.contains("eve@") && !shown.contains('@'), "{shown}");
}

#[test]
fn undo_cannot_let_a_held_message_go_again_nor_bring_back_one_that_may_have_gone() {
    // A message the run could not be sure of is held; the state Undo goes back to has it in the Outbox, not
    // held, and the next run would send it again.
    for (what, outcome_of) in [
        ("an answer that was lost", held(true, "")),
        ("a refusal", held(false, "the server found the message invalid")),
        ("a message that went", SendOutcome::Sent),
    ] {
        let (mut s, account) = session();
        let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
        let other = s.mailbox.add_message(Message { account, folder: inbox, subject: "Other".into(), ..Default::default() }).unwrap();
        let one = send(&mut s, account, "bob@example.org", "One");
        // Something the owner can undo, done after the message was written.
        s.run("mail.flag", &json!({"id": other})).unwrap();
        assert!(s.can_undo(), "{what}");
        answer(&mut s, account, vec![outcome(one, outcome_of)], vec![]);
        assert!(!s.can_undo() && !s.undo(), "{what}: nothing is left to go back to");
        assert!(request(&s, account).sends.is_empty(), "{what}: the message does not go again");
    }
    // A copy that waits to be destroyed is no more to be undone than that: an older state has no such queue, and the
    // copy would come down as mail.
    let (mut s, account) = session();
    let one = send(&mut s, account, "bob@example.org", "One");
    s.run("mail.flag", &json!({"id": one})).unwrap();
    assert!(s.can_undo());
    apply_jmap_mail(&mut s, JmapMailResult { account, email: ME.into(), unwanted: vec!["not an id!".into()], ..Default::default() }).unwrap();
    assert!(s.can_undo(), "nothing was queued: nothing to protect");
    apply_jmap_mail(&mut s, JmapMailResult { account, email: ME.into(), unwanted: vec!["E5".into()], ..Default::default() }).unwrap();
    assert!(!s.can_undo() && !s.undo());
    // An answer that says nothing happened leaves what can be undone alone.
    let (mut s, account) = session();
    let one = send(&mut s, account, "bob@example.org", "One");
    s.run("mail.flag", &json!({"id": one})).unwrap();
    answer(&mut s, account, vec![outcome(one, SendOutcome::Waits)], vec![]);
    assert!(s.can_undo());
    // And one for a message that is no longer in the Outbox forgets it all the same: it went.
    let drafts = s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap();
    s.mailbox.move_message(one, drafts).unwrap();
    answer(&mut s, account, vec![outcome(one, SendOutcome::Sent)], vec![]);
    assert!(!s.can_undo());
}

#[test]
fn a_message_the_owner_took_out_of_the_outbox_meanwhile_is_left_alone_and_said_to_have_gone() {
    let (mut s, account) = session();
    let (one, two, three) = (
        send(&mut s, account, "bob@example.org", "One"),
        send(&mut s, account, "carol@example.org", "Two"),
        send(&mut s, account, "dave@example.org", "Three"),
    );
    // Edited again (back in Drafts), deleted for good, and moved to the trash, while the run was under way.
    let (drafts, trash) =
        (s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap(), s.mailbox.folder_of_kind(account, FolderKind::Deleted).unwrap());
    s.mailbox.move_message(one, drafts).unwrap();
    s.mailbox.move_message(three, trash).unwrap();
    s.mailbox.forget_message(two).unwrap();
    let said = answer(
        &mut s,
        account,
        vec![outcome(one, SendOutcome::Sent), outcome(two, held(true, "")), outcome(three, held(true, "the server failed"))],
        vec![],
    );
    assert_eq!(
        said["errors"],
        json!([
            "'One' was sent, although it was taken out of the Outbox meanwhile. Look in Sent.",
            "A message may have been sent, although it was taken out of the Outbox meanwhile. Look in Sent.",
            "'Three' may have been sent, although it was taken out of the Outbox meanwhile. Look in Sent.",
        ])
    );
    // A message the owner has put elsewhere is not moved, removed or held: what the owner does with it next is theirs to
    // decide (putting it back in the Outbox is saying that it should go), and the line says what happened.
    assert_eq!(s.mailbox.message(one).unwrap().folder, drafts);
    assert_eq!(s.mailbox.message(three).unwrap().folder, trash);
    assert!(s.mailbox.message(one).unwrap().held.is_empty() && s.mailbox.message(three).unwrap().held.is_empty());
    // A refusal, or a wait, of a message that is not in the Outbox is nothing to say.
    let said =
        answer(&mut s, account, vec![outcome(one, held(false, "no")), outcome(three, SendOutcome::Waits), outcome(two, held(false, "no"))], vec![]);
    assert_eq!(said["errors"], json!([]));
}

#[test]
fn the_lines_a_run_says_leave_the_addresses_out_and_the_message_keeps_them() {
    let (mut s, account) = session();
    let one = send(&mut s, account, "bob@example.org", "One");
    let said = answer(
        &mut s,
        account,
        vec![outcome(one, held(false, "the server doesn't accept bob@example.org, \"carol@example.org\" or <dave@example.org>"))],
        vec![],
    );
    assert_eq!(
        said["errors"][0],
        "Couldn't send 'One': the server doesn't accept (address), \"(address)\" or <(address)>. It is held in the Outbox."
    );
    // What the owner reads with the message is whole.
    let text = &s.mailbox.message(one).unwrap().held;
    assert!(text.contains("bob@example.org") && text.contains("\"carol@example.org\"") && text.contains("<dave@example.org>"), "{text}");
    // The same for a draft the server would not have, and for a doubt.
    let draft = save_draft(&mut s, account, "Plans");
    let refused = JmapDraftDone { message: draft, saves: 1, refused: Some("eve@example.org is over its quota".into()), ..Default::default() };
    let said = answer(&mut s, account, vec![], vec![refused]);
    assert_eq!(
        said["errors"][0],
        "Couldn't save the draft 'Plans' on the server: (address) is over its quota. It stays here, and goes up the next time it is saved."
    );
    let two = send(&mut s, account, "bob@example.org", "Two");
    let said = answer(&mut s, account, vec![outcome(two, held(true, "the server said: mail ada@example.org"))], vec![]);
    assert!(!said["errors"][0].as_str().unwrap().contains('@'), "{said}");
    // The subject is the owner's, and may have an address in it: the line leaves that out too.
    let three = send(&mut s, account, "bob@example.org", "Fwd: access request from carol@example.org");
    let said = answer(&mut s, account, vec![outcome(three, held(false, "no"))], vec![]);
    assert_eq!(said["errors"][0], "Couldn't send 'Fwd: access request from (address)': no. It is held in the Outbox.");
    assert!(s.mailbox.message(three).unwrap().subject.contains("carol@example.org"), "the message is what it was");
    // A message that went although it was taken out of the Outbox: the same.
    let four = send(&mut s, account, "bob@example.org", "Re: bob@example.org");
    let drafts = s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap();
    s.mailbox.move_message(four, drafts).unwrap();
    let said = answer(&mut s, account, vec![outcome(four, SendOutcome::Sent)], vec![]);
    assert_eq!(said["errors"][0], "'Re: (address)' was sent, although it was taken out of the Outbox meanwhile. Look in Sent.");
}

#[test]
fn an_address_in_a_subject_is_left_out_before_the_subject_is_cut_and_not_after() {
    let (mut s, account) = session();
    // The address straddles the place where a subject is cut for a line: it must not leave half of itself behind.
    let subject = format!("{} carol@example.org and more", "x".repeat(52));
    let one = send(&mut s, account, "bob@example.org", &subject);
    let said = answer(&mut s, account, vec![outcome(one, held(false, "no"))], vec![]);
    let line = said["errors"][0].as_str().unwrap();
    // (What is left of it is the mask, cut with the rest.)
    assert!(!line.contains("carol") && !line.contains('@') && line.contains("(addr"), "{line}");
}

#[test]
fn undo_cannot_bring_back_a_copy_filed_for_a_draft_that_was_discarded_meanwhile() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Plans");
    let asked = request(&s, account).drafts;
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    s.run("compose.discard", &json!({})).unwrap();
    assert!(s.can_undo(), "the discard can be undone");
    // The run, which was under way, filed it all the same: the copy waits to be destroyed, and an older state of the
    // mailbox has no such queue.
    answer(&mut s, account, vec![], vec![filed(draft, asked[0].saves, "E5")]);
    assert_eq!(s.mailbox.queued.len(), 1);
    assert!(!s.can_undo() && !s.undo());
}

#[test]
fn a_reason_that_ends_with_a_full_stop_does_not_double_it() {
    let (mut s, account) = session();
    let (one, two) = (send(&mut s, account, "bob@example.org", "One"), send(&mut s, account, "bob@example.org", "Two"));
    let said =
        answer(&mut s, account, vec![outcome(one, held(false, "No. Not today.")), outcome(two, held(true, "the server failed. Really.  "))], vec![]);
    assert_eq!(said["errors"][0], "Couldn't send 'One': No. Not today. It is held in the Outbox.");
    assert_eq!(said["errors"][1], "'Two' may have been sent: the server failed. Really. It is held in the Outbox.");
    assert!(s.mailbox.message(one).unwrap().held.starts_with("It wasn't sent: No. Not today. Change it"));
    let draft = save_draft(&mut s, account, "Plans");
    let refused = JmapDraftDone { message: draft, saves: 1, refused: Some("It is too big.".into()), ..Default::default() };
    let said = answer(&mut s, account, vec![], vec![refused]);
    assert_eq!(
        said["errors"][0],
        "Couldn't save the draft 'Plans' on the server: It is too big. It stays here, and goes up the next time it is saved."
    );
}

// ------------------------------------------------------------------------------------------------
// Where the copies go is read

#[test]
fn the_mailboxes_a_run_files_into_are_read_besides_so_that_the_copies_show() {
    let (mut s, account) = session();
    let light = |s: &Session, also: &[String]| jmap_mail_request(s, account, false, also, true).unwrap();
    // Nothing to send and nothing to file: the Inbox, and what is on screen.
    assert_eq!(light(&s, &["Mpr".into()]).scope, JmapScope::Light { also: vec!["Mpr".into()] });
    send(&mut s, account, "bob@example.org", "One");
    assert_eq!(light(&s, &["Mpr".into()]).scope, JmapScope::Light { also: vec!["Mse".into(), "Mpr".into()] });
    // A draft: Drafts too. What is on screen is not read twice.
    s.run("mail.new", &json!({"to": "bob@example.org", "subject": "Later", "account": account})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert_eq!(light(&s, &["Mse".into(), "Mpr".into()]).scope, JmapScope::Light { also: vec!["Mse".into(), "Mdr".into(), "Mpr".into()] });
    // A run that reads everything needs none of this.
    assert_eq!(jmap_mail_request(&s, account, true, &[], false).unwrap().scope, JmapScope::All);
}

// ------------------------------------------------------------------------------------------------
// Drafts

fn save_draft(s: &mut Session, account: Id, subject: &str) -> Id {
    s.run("mail.new", &json!({"to": "bob@example.org", "bcc": "eve@example.org", "subject": subject, "body": "Hi", "account": account})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap()["draft"].as_u64().unwrap()
}

fn filed(message: Id, saves: u32, id: &str) -> JmapDraftDone {
    JmapDraftDone { message, saves, id: id.into(), folder: "Mdr".into(), refused: None }
}

#[test]
fn a_draft_saved_here_goes_up_whole_and_waits_no_more_once_the_server_has_it() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Plans");
    let up = request(&s, account).drafts;
    assert_eq!(up.len(), 1);
    assert_eq!((up[0].message, up[0].saves, up[0].replaces.as_str()), (draft, 1, ""));
    // Whole, with its Bcc: a draft is for its owner.
    assert_eq!((up[0].mime.subject.as_str(), up[0].mime.bcc[0].email.as_str()), ("Plans", "eve@example.org"));
    assert_eq!(pending_drafts(&s.mailbox, account), 1);
    assert_eq!(crate::cmd::account::unpushed(&s, account), 1, "the window starts a run for it");

    // The server filed it: it knows it by that id, in Drafts, and nothing waits.
    let said = answer(&mut s, account, vec![], vec![filed(draft, 1, "E5")]);
    let m = s.mailbox.message(draft).unwrap();
    let (server, sd) = (m.server.as_ref().unwrap(), m.server_draft.as_ref().unwrap());
    assert_eq!((server.id.as_str(), server.folder.as_str(), server.read, server.gone), ("E5", "Mdr", true, false));
    assert!(!sd.pending);
    assert_eq!(said["updated"], 1);
    assert!(request(&s, account).drafts.is_empty() && crate::cmd::account::unpushed(&s, account) == 0);

    // Saved again, it replaces the copy the server has.
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    s.run("compose.set", &json!({"subject": "Plans, again"})).unwrap();
    assert_eq!(s.run("compose.saveDraft", &json!({})).unwrap()["draft"].as_u64(), Some(draft), "the same draft");
    let up = request(&s, account).drafts;
    assert_eq!((up[0].saves, up[0].replaces.as_str(), up[0].mime.subject.as_str()), (2, "E5", "Plans, again"));
    // The Message-ID is kept across saves, so that it is one message.
    assert_eq!(s.mailbox.message(draft).unwrap().message_id, up[0].mime.message_id);
}

#[test]
fn a_draft_saved_again_while_a_run_is_under_way_waits_for_the_next_run_which_replaces_the_copy_just_made() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Plans");
    let asked = request(&s, account).drafts;
    // Meanwhile, the owner changes it and saves again.
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    s.run("compose.set", &json!({"subject": "Plans 2"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    // The run's answer is for the first save: the copy it filed is the server's, and the draft still waits.
    answer(&mut s, account, vec![], vec![filed(draft, asked[0].saves, "E5")]);
    let m = s.mailbox.message(draft).unwrap();
    assert_eq!((m.server.as_ref().map(|s| s.id.as_str()), m.server_draft.as_ref().map(|d| (d.pending, d.saves))), (Some("E5"), Some((true, 2))));
    let next = request(&s, account).drafts;
    assert_eq!((next[0].replaces.as_str(), next[0].saves, next[0].mime.subject.as_str()), ("E5", 2, "Plans 2"));
}

#[test]
fn a_draft_the_server_would_not_take_waits_until_it_is_saved_again_and_the_owner_is_told_once() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Plans");
    let refused = JmapDraftDone { message: draft, saves: 1, refused: Some("the mailbox on the server is full".into()), ..Default::default() };
    let said = answer(&mut s, account, vec![], vec![refused]);
    assert_eq!(
        said["errors"][0],
        "Couldn't save the draft 'Plans' on the server: the mailbox on the server is full. It stays here, and goes up the next time it is saved."
    );
    assert!(request(&s, account).drafts.is_empty(), "not asked for again by itself");
    assert!(s.mailbox.message(draft).unwrap().server.is_none());
    // Saved again, it is.
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert_eq!(request(&s, account).drafts.len(), 1);
}

#[test]
fn a_draft_saved_again_while_the_server_refused_the_first_save_still_goes_up() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Too much");
    let asked = request(&s, account).drafts;
    // The owner fixes it meanwhile, and saves.
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    s.run("compose.set", &json!({"subject": "Less"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    // The refusal is of the version that was asked for, not of this one: nothing is said of it (the draft that goes up
    // next is another, and the next run will say if the server refuses that too), and the draft still waits.
    let refused =
        JmapDraftDone { message: draft, saves: asked[0].saves, refused: Some("it is too large for the server".into()), ..Default::default() };
    let said = answer(&mut s, account, vec![], vec![refused]);
    assert_eq!((said["errors"].clone(), said["updated"].clone()), (json!([]), json!(0)));
    let next = request(&s, account).drafts;
    assert_eq!((next.len(), next[0].saves, next[0].mime.subject.as_str()), (1, 2, "Less"));
    // The one that was refused is the one now waiting: it is said, and it waits no more.
    let refused = JmapDraftDone { message: draft, saves: 2, refused: Some("it is too large for the server".into()), ..Default::default() };
    let said = answer(&mut s, account, vec![], vec![refused]);
    assert_eq!(
        said["errors"][0],
        "Couldn't save the draft 'Less' on the server: it is too large for the server. It stays here, and goes up the next time it is saved."
    );
    assert!(request(&s, account).drafts.is_empty());
}

#[test]
fn the_mailbox_a_draft_was_filed_in_is_what_the_host_said_or_the_accounts_drafts() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Plans");
    for (said, kept) in [("Mdr", "Mdr"), ("Mpr", "Mpr"), ("", "Mdr"), ("not an id!", "Mdr")] {
        let mut done = filed(draft, 1, "E5");
        done.folder = said.into();
        answer(&mut s, account, vec![], vec![done]);
        let folder = s.mailbox.message(draft).and_then(|m| m.server.as_ref()).map(|server| server.folder.clone());
        assert_eq!(folder.as_deref(), Some(kept), "{said:?}");
    }
}

#[test]
fn a_draft_that_is_not_in_drafts_any_more_is_not_filed_there_again() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Plans");
    let trash = s.mailbox.folder_of_kind(account, FolderKind::Deleted).unwrap();
    s.mailbox.move_message(draft, trash).unwrap();
    assert!(request(&s, account).drafts.is_empty());
    assert_eq!(pending_drafts(&s.mailbox, account), 0);
    // A draft of another kind of account is not this one's.
    let imap = s.run("account.add", &json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    let theirs = save_draft(&mut s, imap, "Theirs");
    assert!(s.mailbox.message(theirs).unwrap().server_draft.is_none());
    assert!(request(&s, account).drafts.is_empty());
}

#[test]
fn a_draft_discarded_has_its_copy_on_the_server_destroyed_and_one_that_never_went_up_has_none() {
    let (mut s, account) = session();
    let queued = |s: &Session| s.mailbox.queued.clone();
    // One the server has.
    let draft = save_draft(&mut s, account, "Plans");
    answer(&mut s, account, vec![], vec![filed(draft, 1, "E5")]);
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    assert_eq!(s.run("compose.discard", &json!({})).unwrap()["draftDeleted"], true);
    assert!(s.mailbox.message(draft).is_none());
    assert_eq!(queued(&s), [Queued::DeleteMessage { account, id: "E5".into(), kept: None }]);
    // Discarded twice (a second call, an answer that comes again): once.
    discard(&mut s.mailbox, draft);
    assert_eq!(queued(&s).len(), 1);

    // One that was never filed has nothing to destroy.
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Never filed");
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    s.run("compose.discard", &json!({})).unwrap();
    assert!(s.mailbox.message(draft).is_none() && s.mailbox.queued.is_empty());
    // And a mailbox of another kind of account does not queue anything for the server.
    let imap = s.run("account.add", &json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    let theirs = save_draft(&mut s, imap, "Theirs");
    s.run("mail.editDraft", &json!({"id": theirs})).unwrap();
    s.run("compose.discard", &json!({})).unwrap();
    assert!(s.mailbox.queued.is_empty());
}

#[test]
fn a_copy_filed_for_a_draft_that_was_discarded_meanwhile_is_destroyed_too() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Plans");
    let asked = request(&s, account).drafts;
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    s.run("compose.discard", &json!({})).unwrap();
    // The run, which was under way, filed it all the same.
    answer(&mut s, account, vec![], vec![filed(draft, asked[0].saves, "E5")]);
    assert_eq!(s.mailbox.queued, [Queued::DeleteMessage { account, id: "E5".into(), kept: None }]);
    // The same answer again queues nothing more, and an answer that names no copy queues nothing.
    answer(&mut s, account, vec![], vec![filed(draft, 1, "E5"), filed(draft, 1, ""), filed(draft, 1, "not an id!")]);
    assert_eq!(s.mailbox.queued.len(), 1);
}

#[test]
fn a_draft_that_is_sent_gives_its_server_copy_to_the_message_which_the_host_destroys_when_it_has_gone() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Plans");
    answer(&mut s, account, vec![], vec![filed(draft, 1, "E5")]);
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    let message = s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
    // The draft is no more here, and nothing is queued for it: it goes when the message has.
    assert!(s.mailbox.message(draft).is_none());
    assert!(s.mailbox.queued.is_empty());
    let sd = s.mailbox.message(message).unwrap().server_draft.clone().unwrap();
    assert_eq!((sd.sent_from.as_str(), sd.pending), ("E5", false));
    let request = request(&s, account);
    assert_eq!((request.sends[0].message, request.sends[0].draft_copy.as_str()), (message, "E5"));
    assert!(request.drafts.is_empty(), "the message is not a draft to file");
    // A message that is held keeps it, for Send Again.
    answer(&mut s, account, vec![outcome(message, held(false, "no"))], vec![]);
    assert_eq!(s.mailbox.message(message).unwrap().server_draft.as_ref().map(|d| d.sent_from.as_str()), Some("E5"));
    s.run("mail.sendAgain", &json!({"id": message})).unwrap();
    assert_eq!(sends(&s, account)[0].draft_copy, "E5");

    // A draft the server never had is just removed.
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Never filed");
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    let message = s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
    assert!(s.mailbox.message(draft).is_none() && s.mailbox.queued.is_empty());
    assert!(s.mailbox.message(message).unwrap().server_draft.is_none());
    assert_eq!(sends(&s, account)[0].draft_copy, "");
}

fn copy(id: &str) -> GraphMessage {
    GraphMessage {
        server: ServerMessage { id: id.into(), ..Default::default() },
        message_id: format!("{id}@example.org"),
        subject: format!("Copy {id}"),
        ..Default::default()
    }
}

#[test]
fn copies_nobody_wants_wait_in_the_queue_and_do_not_come_down_as_mail() {
    let (mut s, account) = session();
    let drafts = s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap();
    let leftovers =
        |ids: &[&str]| JmapMailResult { account, email: ME.into(), unwanted: ids.iter().map(|id| (*id).to_string()).collect(), ..Default::default() };
    let said = apply_jmap_mail(&mut s, leftovers(&["E5", "E5", "not an id!", "", "E6"])).unwrap();
    assert_eq!(
        s.mailbox.queued,
        [Queued::DeleteMessage { account, id: "E5".into(), kept: None }, Queued::DeleteMessage { account, id: "E6".into(), kept: None }]
    );
    assert_eq!(said["updated"], 1);
    // The same again queues nothing more.
    assert_eq!(apply_jmap_mail(&mut s, leftovers(&["E5", "E6"])).unwrap()["updated"], 0);
    // A window that lists them brings the others down and not those.
    let page = MessagePage { folder: "Mdr".into(), messages: vec![copy("E5"), copy("E7"), copy("E6")], afresh: true, ..Default::default() };
    apply_jmap_mail(&mut s, JmapMailResult { account, email: ME.into(), pages: vec![page], ..Default::default() }).unwrap();
    let here: Vec<&str> = s.mailbox.messages_in(drafts).filter_map(|m| m.server.as_ref()).map(|server| server.id.as_str()).collect();
    assert_eq!(here, ["E7"]);
    // What a run could not destroy does not grow the queue without end: the last place goes to one more, and no more
    // after it. What another account has waiting there takes no place of this one's.
    let others = |n: usize| (0..n).map(|n| Queued::DeleteMessage { account: account + 1, id: format!("O{n}"), kept: None });
    s.mailbox.queued.extend(others(MAX_QUEUED));
    let room = MAX_QUEUED - 2 - 1;
    s.mailbox.queued.extend((0..room).map(|n| Queued::DeleteMessage { account, id: format!("X{n}"), kept: None }));
    assert!(queue_unwanted(&mut s.mailbox, account, "Y1"));
    assert!(!queue_unwanted(&mut s.mailbox, account, "Y2"));
    assert!(!unwanted(&mut s.mailbox, account, &["Y3".to_string()]));
    // The owner discarding a draft is not held to it.
    let draft = save_draft(&mut s, account, "Plans");
    answer(&mut s, account, vec![], vec![filed(draft, 1, "E9")]);
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    s.run("compose.discard", &json!({})).unwrap();
    assert!(s.mailbox.queued.iter().any(|q| matches!(q, Queued::DeleteMessage { id, .. } if id == "E9")));
}

#[test]
fn the_draft_a_message_was_written_as_does_not_come_down_while_the_message_waits_to_go() {
    let (mut s, account) = session();
    let drafts = s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap();
    let draft = save_draft(&mut s, account, "Plans");
    answer(&mut s, account, vec![], vec![filed(draft, 1, "E5")]);
    s.run("mail.editDraft", &json!({"id": draft})).unwrap();
    let message = s.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
    // The run met trouble before it sent the message, and went on to read Drafts, which still lists the draft.
    let page = MessagePage { folder: "Mdr".into(), messages: vec![copy("E5")], afresh: true, ..Default::default() };
    apply_jmap_mail(&mut s, JmapMailResult { account, email: ME.into(), pages: vec![page], ..Default::default() }).unwrap();
    assert_eq!(s.mailbox.messages_in(drafts).count(), 0, "not shown a second time as a draft");
    assert!(s.mailbox.message(message).is_some());
    // The message has gone: the host destroyed the draft, and the window no longer lists it.
    answer(&mut s, account, vec![outcome(message, SendOutcome::Sent)], vec![]);
    assert!(doomed(&s.mailbox, account).is_empty());
}

#[test]
fn a_draft_that_came_down_from_the_server_is_edited_once_it_is_whole() {
    let (mut s, account) = session();
    let drafts = s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap();
    // Only its headers and a preview are here: written over, the server's draft would be lost.
    let down = Message {
        account,
        folder: drafts,
        draft: true,
        unfetched: true,
        subject: "From elsewhere".into(),
        server: Some(ServerMessage { id: "E7".into(), folder: "Mdr".into(), read: true, ..Default::default() }),
        ..Default::default()
    };
    let down = s.mailbox.add_message(down).unwrap();
    let refused = s.run("mail.editDraft", &json!({"id": down})).unwrap_err();
    assert!(refused.to_string().contains("isn't downloaded yet"), "{refused}");
    // Fetched whole, it opens; saved here, it replaces the copy the server has.
    s.mailbox.message_mut(down).unwrap().unfetched = false;
    s.run("mail.editDraft", &json!({"id": down})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    let up = request(&s, account).drafts;
    assert_eq!((up[0].message, up[0].replaces.as_str(), up[0].mime.subject.as_str()), (down, "E7", "From elsewhere"));
}

#[test]
fn a_draft_the_server_no_longer_has_is_made_there_again() {
    let (mut s, account) = session();
    let draft = save_draft(&mut s, account, "Plans");
    answer(&mut s, account, vec![], vec![filed(draft, 1, "E5")]);
    // The server lost it (the window says so, and the draft waits to go up again): nothing is destroyed.
    s.mailbox.message_mut(draft).unwrap().server.as_mut().unwrap().gone = true;
    s.mailbox.message_mut(draft).unwrap().server_draft.as_mut().unwrap().pending = true;
    assert_eq!(request(&s, account).drafts[0].replaces, "");
}

#[test]
fn what_a_run_answers_about_a_draft_that_is_none_is_ignored() {
    let (mut s, account) = session();
    let received = s
        .mailbox
        .add_message(Message { account, folder: s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap(), ..Default::default() })
        .unwrap();
    let before = s.mailbox.messages.get(&received).cloned();
    answer(&mut s, account, vec![], vec![filed(received, 1, "E5"), filed(9_999, 1, "E6")]);
    assert_eq!(s.mailbox.messages.get(&received).cloned(), before, "a message that is no draft is not made the server's copy");
    // (The copy filed for the message that isn't there is nobody's, and its destruction is queued.)
    assert_eq!(s.mailbox.queued, [Queued::DeleteMessage { account, id: "E6".into(), kept: None }]);
}
