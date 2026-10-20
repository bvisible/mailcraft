// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What the owner changes on a JMAP account's messages goes up to its server, from end to end: the engine's
//! `jmap_mail_request` says what differs from the server's last word (read marks, flags, moves, deletes, Deleted
//! Items emptied), `mailcraft_jmap::sync_mail` makes the changes on the in-process JMAP server and reads back what it
//! did, and `apply_jmap_mail` takes each piece in as it comes. Both kinds of server: one that keeps the id of a
//! message when it moves, and one that names a message by where it is, so that a move gives it another id and does not
//! say which. Nothing here leaves this machine; every name, address and id is invented.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicBool;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::account::unpushed;
use mailcraft_engine::cmd::jmap_mail::{apply_jmap_mail, jmap_mail_request};
use mailcraft_jmap::standin::{Profile, StandIn, TOKEN};
use mailcraft_jmap::{Auth, Client, block_on, sync_mail};
use mailcraft_model::sync::{JmapChange, JmapChangeDone, JmapMailResult, JmapOutcome};
use mailcraft_model::{FlagStatus, FolderKind, Id, Mailbox, Message};
use serde_json::{Value, json};

const ADDRESS: &str = "ada@example.org";
const SESSION: &str = "https://jmap.standin.example/jmap/session?account=ada%40example.org";

/// Every line of trouble the pieces of a run told of, in the order they came.
fn errors_of(answers: &[Value]) -> Vec<String> {
    answers.iter().flat_map(|a| a["errors"].as_array().cloned().unwrap_or_default()).filter_map(|e| e.as_str().map(str::to_string)).collect()
}

/// The two kinds of server, by name.
fn kinds() -> [(&'static str, Profile); 2] {
    [("a server that keeps ids", Profile::standard()), ("a server that names mail by place", Profile::by_place())]
}

struct Rig {
    server: StandIn,
    s: Session,
    account: Id,
    inbox: String,
    archive: String,
}

impl Rig {
    /// A session with a JMAP account, and a server of `profile` with an Inbox, Sent, Trash, Archive and Junk by role.
    fn new(profile: Profile) -> Rig {
        let mut s = Session::at(Mailbox::with_local_account("", ""), 1_800_000_000, 0);
        let added = s.run("account.add", &json!({"email": ADDRESS, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
        let server = StandIn::new();
        let (inbox, archive) = server.with(|d| {
            d.profile = profile;
            let inbox = d.mailbox("Inbox", None, Some("inbox"));
            d.mailbox("Sent", None, Some("sent"));
            d.mailbox("Trash", None, Some("trash"));
            d.mailbox("Junk", None, Some("junk"));
            (inbox, d.mailbox("Archive", None, Some("archive")))
        });
        Rig { server, account: added["id"].as_u64().unwrap(), s, inbox, archive }
    }

    /// A message in the Inbox, as a person's mail program wrote it: its id on the server.
    fn deliver(&self, subject: &str, keywords: &[&str], hour: usize) -> String {
        self.deliver_in(&self.inbox, subject, keywords, &format!("2026-10-15T{:02}:00:00Z", hour))
    }

    /// A message in the mailbox `mailbox`, received at `received`.
    fn deliver_in(&self, mailbox: &str, subject: &str, keywords: &[&str], received: &str) -> String {
        let raw = format!(
            "From: Zoe Park <zoe@example.org>\r\nTo: Ada <{ADDRESS}>\r\nSubject: {subject}\r\nMessage-ID: <{subject}@example.org>\r\nDate: Thu, 15 Oct 2026 09:30:00 +0200\r\n\r\nHello about {subject}.\r\n"
        );
        self.server.with(|d| d.deliver(mailbox, raw.as_bytes(), keywords, received))
    }

    /// A run as a host does it (`full`: every mailbox; else the Inbox and `also`), each piece taken in as it arrives: what
    /// taking each in answered.
    fn run_as(&mut self, full: bool, also: &[String], background: bool) -> Vec<Value> {
        let request = jmap_mail_request(&self.s, self.account, full, also, background).unwrap();
        let client = block_on(Client::connect(self.server.clone(), &request.session, &Auth::bearer(TOKEN))).unwrap();
        let mut answers = Vec::new();
        let (s, cancel) = (&mut self.s, AtomicBool::new(false));
        block_on(sync_mail(&client, &request, &cancel, &mut |piece| answers.push(apply_jmap_mail(s, piece).unwrap())));
        answers
    }

    /// The owner's own Send/Receive.
    fn full(&mut self) -> Vec<Value> {
        self.run_as(true, &[], false)
    }

    /// The run the window starts by itself when something waits to go up: the Inbox and the folder on screen.
    fn quick(&mut self) -> Vec<Value> {
        let on_screen: Vec<String> = self
            .s
            .mailbox
            .folder(self.s.view.folder)
            .filter(|f| f.account == self.account && f.kind != FolderKind::Inbox)
            .and_then(|f| f.server.as_ref())
            .map(|server| server.id.clone())
            .into_iter()
            .collect();
        self.run_as(false, &on_screen, true)
    }

    fn folder(&self, name: &str) -> Id {
        self.s.mailbox.folders.iter().find(|f| f.account == self.account && f.name == name).unwrap_or_else(|| panic!("no folder {name} here")).id
    }

    /// The local message with this subject.
    fn message(&self, subject: &str) -> &Message {
        let mut found = self.s.mailbox.messages.values().filter(|m| m.account == self.account && m.subject == subject);
        let one = found.next().unwrap_or_else(|| panic!("no message {subject} here"));
        assert!(found.next().is_none(), "two messages called {subject}");
        one
    }

    fn id(&self, subject: &str) -> Id {
        self.message(subject).id
    }

    fn has(&self, subject: &str) -> bool {
        self.s.mailbox.messages.values().any(|m| m.account == self.account && m.subject == subject)
    }

    /// The subjects in the local folder `name`, newest first.
    fn mail(&self, name: &str) -> Vec<String> {
        let folder = self.folder(name);
        let mut found: Vec<&Message> = self.s.mailbox.messages.values().map(|m| m.as_ref()).filter(|m| m.folder == folder).collect();
        found.sort_by_key(|m| std::cmp::Reverse(m.received));
        found.iter().map(|m| m.subject.clone()).collect()
    }

    /// What the server has of the message with this subject, wherever it lies and whatever its id is: its keywords and the
    /// name of its mailbox. `None` when it has none.
    fn there(&self, subject: &str) -> Option<(Vec<String>, String)> {
        let id = format!("<{subject}@example.org>");
        self.server.with(|d| {
            let email = d.emails.iter().find(|e| String::from_utf8_lossy(&e.raw).contains(&format!("Message-ID: {id}")))?;
            let mailbox = email.mailboxes.iter().filter_map(|id| d.mailboxes.iter().find(|m| m.id == *id)).map(|m| m.name.clone()).next()?;
            Some((email.keywords.iter().cloned().collect(), mailbox))
        })
    }

    /// The id the server has for the message with this subject.
    fn server_id(&self, subject: &str) -> Option<String> {
        let id = format!("<{subject}@example.org>");
        self.server.with(|d| d.emails.iter().find(|e| String::from_utf8_lossy(&e.raw).contains(&format!("Message-ID: {id}"))).map(|e| e.id.clone()))
    }

    /// The id this mailbox has for the message.
    fn local_server_id(&self, subject: &str) -> String {
        self.message(subject).server.as_ref().map(|server| server.id.clone()).unwrap_or_default()
    }

    fn waiting(&self) -> usize {
        unpushed(&self.s, self.account)
    }

    fn sets(&self) -> Vec<(usize, usize)> {
        self.server.asked().iter().filter(|a| a.method == "Email/set").map(|a| (a.updates.unwrap_or(0), a.destroys.unwrap_or(0))).collect()
    }

    /// Everything a run left: this mailbox and the server's agree on where each message is and what is marked.
    fn agrees(&self) {
        let account = self.account;
        let local: Vec<(String, bool, bool, String)> = {
            let mut all: Vec<_> = self
                .s
                .mailbox
                .messages
                .values()
                .filter(|m| m.account == account)
                .map(|m| {
                    (
                        m.subject.clone(),
                        m.read,
                        m.flag == FlagStatus::Flagged,
                        self.s.mailbox.folder(m.folder).map(|f| f.name.clone()).unwrap_or_default(),
                    )
                })
                .collect();
            all.sort();
            all
        };
        let server: Vec<(String, bool, bool, String)> = {
            let mut all: Vec<_> = self.server.with(|d| {
                d.emails
                    .iter()
                    .map(|e| {
                        let text = String::from_utf8_lossy(&e.raw).to_string();
                        let subject = text.split("Subject: ").nth(1).and_then(|rest| rest.split("\r\n").next()).unwrap_or_default().to_string();
                        let mailbox = e
                            .mailboxes
                            .iter()
                            .filter_map(|id| d.mailboxes.iter().find(|m| m.id == *id))
                            .map(|m| m.name.clone())
                            .next()
                            .unwrap_or_default();
                        (subject, e.keywords.contains("$seen"), e.keywords.contains("$flagged"), mailbox)
                    })
                    .collect()
            });
            all.sort();
            all
        };
        assert_eq!(local, server);
    }
}

#[test]
fn read_marks_and_flags_go_up_and_the_server_after_them_is_what_comes_back() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        rig.deliver("first", &["$seen"], 1);
        rig.deliver("second", &[], 2);
        rig.deliver("third", &["$flagged", "$answered"], 3);
        rig.full();
        rig.agrees();
        assert_eq!(rig.waiting(), 0, "{kind}");

        // Opened, marked unread and flagged, and a flag taken off.
        rig.s.run("mail.markRead", &json!({"ids": [rig.id("second")]})).unwrap();
        rig.s.run("mail.markUnread", &json!({"ids": [rig.id("first")]})).unwrap();
        rig.s.run("mail.flag", &json!({"ids": [rig.id("first")]})).unwrap();
        rig.s.run("mail.clearFlag", &json!({"ids": [rig.id("third")]})).unwrap();
        assert_eq!(rig.waiting(), 3, "{kind}");
        rig.server.with(|d| d.seen.clear());
        let answers = rig.quick();

        // One call told the server of the three, before it read anything.
        assert_eq!(rig.sets(), [(3, 0)], "{kind}");
        assert_eq!(rig.server.calls()[0], "Email/set", "{kind}");
        assert_eq!(answers[0]["pushed"], 3, "{kind}");
        assert!(answers.iter().all(|a| a["errors"] == json!([])), "{kind}: {answers:?}");
        assert_eq!(rig.there("first").unwrap().0, ["$flagged"], "{kind}");
        assert_eq!(rig.there("second").unwrap().0, ["$seen"], "{kind}");
        assert_eq!(rig.there("third").unwrap().0, ["$answered"], "{kind}: what was not changed is not touched");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.agrees();
        assert!(rig.s.sync_status.starts_with("Up to date — "), "{kind}: {}", rig.s.sync_status);
        // Marks going up don't cost the history.
        assert!(rig.s.can_undo(), "{kind}");

        // Nothing changed: nothing is asked, and the next run is as quiet.
        rig.server.with(|d| d.seen.clear());
        rig.quick();
        assert!(rig.sets().is_empty() && !rig.server.calls().iter().any(|call| call == "Email/set"), "{kind}");
        rig.agrees();
    }
}

#[test]
fn a_message_moved_here_is_moved_on_the_server_and_is_the_same_message_after_the_run() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        rig.deliver("first", &[], 1);
        rig.deliver("second", &[], 2);
        rig.deliver("third", &[], 3);
        rig.full();
        let (before_id, count, local) = (rig.local_server_id("second"), rig.s.mailbox.messages.len(), rig.id("second"));
        let body = rig.message("second").body.text.clone();

        rig.s.run("mail.archive", &json!({"ids": [local]})).unwrap();
        assert_eq!(rig.waiting(), 1, "{kind}");
        let answers = rig.quick();

        assert_eq!(rig.there("second").unwrap().1, "Archive", "{kind}");
        assert_eq!(answers[0]["pushed"], 1, "{kind}");
        // The same message, not a second one and not a new one: it keeps everything it had.
        assert_eq!(rig.s.mailbox.messages.len(), count, "{kind}");
        assert_eq!(rig.id("second"), local, "{kind}");
        assert_eq!((rig.mail("Inbox"), rig.mail("Archive")), (vec!["third".to_string(), "first".to_string()], vec!["second".to_string()]), "{kind}");
        assert_eq!(rig.message("second").body.text, body, "{kind}");
        // Its id on the server is the one the server has now: the same where ids stay, another where they name the place.
        let now = rig.server_id("second").unwrap();
        assert_eq!(rig.local_server_id("second"), now, "{kind}");
        assert_eq!(now == before_id, kind.contains("keeps"), "{kind}");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.agrees();
        // The Archive was read in the same run, though the owner was looking at the Inbox.
        assert!(rig.server.asked().iter().any(|a| a.method == "Email/query" && a.mailbox.as_deref() == Some(rig.archive.as_str())), "{kind}");
        // The server was told to move: nothing before it can be undone.
        assert!(!rig.s.can_undo(), "{kind}");

        // And its next change goes up under the id it has now.
        rig.s.run("mail.markRead", &json!({"ids": [local]})).unwrap();
        rig.quick();
        assert_eq!(rig.there("second").unwrap(), (vec!["$seen".to_string()], "Archive".to_string()), "{kind}");
        rig.agrees();
    }
}

#[test]
fn a_moved_message_is_found_again_even_when_the_window_of_the_mailbox_it_went_to_is_cut_short() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        rig.deliver("first", &[], 1);
        rig.deliver("second", &[], 2);
        // The Archive already holds a number of older messages, which this mailbox has.
        let archive = rig.archive.clone();
        for n in 1..=6 {
            rig.deliver_in(&archive, &format!("old{n}"), &[], &format!("2026-10-0{n}T00:00:00Z"));
        }
        rig.full();
        assert_eq!(rig.mail("Archive").len(), 6, "{kind}");
        let (second, count) = (rig.id("second"), rig.s.mailbox.messages.len());
        // From now on a window holds three messages: the moved one is among the newest three of the Archive, which holds seven,
        // so that the window is a part of it and says nothing of the rest.
        rig.s.prefs.sync_limit = 3;
        rig.s.run("mail.archive", &json!({"ids": [second]})).unwrap();
        rig.quick();
        assert_eq!(rig.there("second").unwrap().1, "Archive", "{kind}");
        // Found again: the same message, not a new one beside it, and it has the id the server has.
        assert_eq!(rig.s.mailbox.messages.len(), count, "{kind}");
        assert_eq!(rig.id("second"), second, "{kind}");
        assert_eq!(rig.local_server_id("second"), rig.server_id("second").unwrap(), "{kind}");
        assert_eq!(rig.waiting(), 0, "{kind}");
        assert_eq!(rig.mail("Archive").len(), 7, "{kind}");
    }
}

#[test]
fn a_move_whose_answer_was_lost_does_not_make_the_message_vanish_when_no_window_reaches_it_for_a_while() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        let archive = rig.archive.clone();
        for n in 1..=6 {
            rig.deliver_in(&archive, &format!("old{n}"), &[], &format!("2026-10-0{n}T00:00:00Z"));
        }
        // The oldest message of the Inbox is older than anything the Archive holds, so a short window of the Archive does
        // not reach it; the Inbox is short, so its window holds all of it.
        let inbox = rig.inbox.clone();
        rig.deliver_in(&inbox, "mine", &[], "2026-09-01T00:00:00Z");
        rig.deliver("first", &[], 1);
        rig.full();
        rig.s.prefs.sync_limit = 3;
        let mine = rig.id("mine");
        rig.s.run("mail.archive", &json!({"ids": [mine]})).unwrap();
        rig.server.with(|d| d.lost_answers = 1);
        let answers = rig.quick();
        assert!(errors_of(&answers).iter().any(|line| line.starts_with("Couldn't send 1 change to the server: ")), "{kind}: {answers:?}");
        // The server made the move. The Inbox says it has no such message now, and the Archive's window does not reach it:
        // it is not swept, in this run or the next (which is whole), and it is said that nobody has found it.
        for _ in 0..2 {
            assert!(rig.has("mine"), "{kind}");
            assert_eq!(rig.message("mine").folder, rig.folder("Archive"), "{kind}");
            rig.quick();
        }
        assert_eq!(rig.there("mine").unwrap().1, "Archive", "{kind}");
        assert!(rig.s.sync_status.contains("1 moved message can't be found on the server yet"), "{kind}: {}", rig.s.sync_status);
        assert_eq!(rig.waiting(), 0, "{kind}");
        // A window that reaches it finds it, under the id the server has.
        rig.s.prefs.sync_limit = 0;
        rig.quick();
        assert_eq!(rig.local_server_id("mine"), rig.server_id("mine").unwrap(), "{kind}");
        assert!(!rig.s.sync_status.contains("moved message"), "{kind}: {}", rig.s.sync_status);
        rig.full();
        rig.agrees();
    }
}

#[test]
fn a_message_deleted_for_good_before_a_window_has_found_it_again_is_destroyed_when_one_does() {
    // Only a server that names mail by place and does not say the new id leaves a moved message without one.
    let mut rig = Rig::new(Profile::by_place());
    let archive = rig.archive.clone();
    for n in 1..=6 {
        rig.deliver_in(&archive, &format!("old{n}"), &[], &format!("2026-10-0{n}T00:00:00Z"));
    }
    // The oldest message of the Inbox is older than anything the Archive holds, so a short window of the Archive does not reach it.
    let inbox = rig.inbox.clone();
    rig.deliver_in(&inbox, "mine", &[], "2026-09-01T00:00:00Z");
    rig.full();
    rig.s.prefs.sync_limit = 3;
    let mine = rig.id("mine");
    rig.s.run("mail.archive", &json!({"ids": [mine]})).unwrap();
    rig.quick();
    assert_eq!(rig.there("mine").unwrap().1, "Archive");
    assert_eq!(rig.local_server_id("mine"), "", "no window has shown it under its new id");
    assert!(rig.s.sync_status.contains("1 moved message can't be found on the server yet, so changes to it wait."), "{}", rig.s.sync_status);
    assert_eq!(rig.waiting(), 0);

    // Deleted for good now: nothing can name it to the server yet, and the delete waits.
    rig.s.run("mail.deletePermanently", &json!({"ids": [mine]})).unwrap();
    assert!(!rig.has("mine") && rig.waiting() == 0);
    rig.quick();
    assert!(rig.there("mine").is_some() && !rig.has("mine"), "the window did not reach it: it is still there, and not brought down");

    // A window that reaches it gives the delete its id, and the next run makes it.
    rig.s.prefs.sync_limit = 0;
    rig.quick();
    assert!(!rig.has("mine"), "it is not brought down as mail");
    assert_eq!(rig.waiting(), 1);
    rig.server.with(|d| d.seen.clear());
    rig.quick();
    assert_eq!(rig.sets(), [(0, 1)]);
    assert!(rig.there("mine").is_none() && !rig.has("mine"));
    assert_eq!(rig.waiting(), 0);
    rig.full();
    rig.agrees();
}

#[test]
fn delete_moves_to_deleted_items_and_a_second_delete_there_destroys() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        for (n, subject) in ["one", "two", "three", "four"].into_iter().enumerate() {
            rig.deliver(subject, &[], n + 1);
        }
        rig.full();
        rig.s.run("mail.delete", &json!({"ids": [rig.id("one")]})).unwrap();
        rig.quick();
        assert_eq!(rig.there("one").unwrap().1, "Trash", "{kind}");
        assert_eq!(rig.mail("Trash"), ["one"], "{kind}");
        rig.agrees();

        // Deleted again, where it is: for good, and the server destroys it. The run needs no id the owner could not have.
        rig.s.run("mail.delete", &json!({"ids": [rig.id("one")]})).unwrap();
        assert!(!rig.has("one"), "{kind}");
        assert_eq!(rig.waiting(), 1, "{kind}");
        rig.server.with(|d| d.seen.clear());
        rig.quick();
        assert_eq!(rig.sets(), [(0, 1)], "{kind}");
        assert!(rig.there("one").is_none(), "{kind}");
        rig.agrees();
        // It does not come back with the next full run.
        rig.full();
        assert!(!rig.has("one"), "{kind}");

        // Shift+Delete, straight from the Inbox.
        rig.s.run("mail.deletePermanently", &json!({"ids": [rig.id("two")]})).unwrap();
        rig.quick();
        assert!(rig.there("two").is_none() && !rig.has("two"), "{kind}");
        rig.agrees();
    }
}

#[test]
fn emptying_deleted_items_destroys_what_it_holds_and_nothing_comes_back() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        for (n, subject) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
            rig.deliver(subject, &[], n + 1);
        }
        rig.full();
        // Three deleted, and sent up; two more, and the folder emptied at once, before they could be.
        for subject in ["a", "b", "c"] {
            rig.s.run("mail.delete", &json!({"ids": [rig.id(subject)]})).unwrap();
        }
        rig.quick();
        rig.agrees();
        for subject in ["d", "e"] {
            rig.s.run("mail.delete", &json!({"ids": [rig.id(subject)]})).unwrap();
        }
        let deleted = rig.folder("Trash");
        rig.s.run("folder.select", &json!({"id": deleted})).unwrap();
        rig.s.run("folder.empty", &json!({})).unwrap();
        assert!(rig.mail("Trash").is_empty(), "{kind}");
        // Five to destroy: three by the ids the server gave them in the Trash, two by the ids they had in the Inbox.
        assert_eq!(rig.waiting(), 5, "{kind}");
        rig.server.with(|d| d.seen.clear());
        let answers = rig.quick();
        assert_eq!(answers[0]["pushed"], 5, "{kind}");
        assert_eq!(rig.sets(), [(0, 5)], "{kind}");
        assert!(rig.server.with(|d| d.emails.is_empty()), "{kind}");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.full();
        assert!(rig.s.mailbox.messages.values().all(|m| m.account != rig.account), "{kind}: nothing came back");
        rig.agrees();
        assert!(!rig.s.can_undo(), "{kind}");
    }
}

#[test]
fn another_client_changing_the_server_meanwhile_is_taken_in_and_what_cannot_be_done_is_said() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        for (n, subject) in ["m1", "m2", "m3", "m4"].into_iter().enumerate() {
            rig.deliver(subject, &[], n + 1);
        }
        rig.full();
        // The owner reads m1, flags m2 and archives m3...
        rig.s.run("mail.markRead", &json!({"ids": [rig.id("m1")]})).unwrap();
        rig.s.run("mail.flag", &json!({"ids": [rig.id("m2")]})).unwrap();
        rig.s.run("mail.archive", &json!({"ids": [rig.id("m3")]})).unwrap();
        // ...while another client deletes m1, reads m4 and delivers a fifth.
        let m1 = rig.server_id("m1").unwrap();
        let m4 = rig.server_id("m4").unwrap();
        rig.server.with(|d| {
            assert!(d.remove(&m1));
            assert!(d.set_keyword(&m4, "$seen", true));
        });
        rig.deliver("m5", &[], 5);

        let answers = rig.quick();
        // What could be done was. The message the server no longer has is said once, in plain words, by a server that
        // looks; one that does not look answers that the mark was made, and the window of the Inbox shows it gone.
        assert_eq!(rig.there("m2").unwrap().0, ["$flagged"], "{kind}");
        assert_eq!(rig.there("m3").unwrap().1, "Archive", "{kind}");
        if kind.contains("keeps") {
            assert_eq!(errors_of(&answers), ["Couldn't mark 'm1' as read: the server says it no longer exists."], "{kind}");
            assert!(
                rig.s.sync_status.starts_with("Couldn't mark 'm1' as read: the server says it no longer exists."),
                "{kind}: {}",
                rig.s.sync_status
            );
        } else {
            assert!(errors_of(&answers).is_empty(), "{kind}: {answers:?}");
            assert!(!rig.s.sync_status.contains("Couldn't"), "{kind}: {}", rig.s.sync_status);
        }
        // And the rest was taken in: m4 is read (it was read elsewhere), m5 is here, m1 is gone.
        assert!(rig.message("m4").read && rig.has("m5") && !rig.has("m1"), "{kind}");
        assert_eq!(rig.mail("Archive"), ["m3"], "{kind}");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.agrees();
        // The refusal is not said again by the next run.
        rig.quick();
        assert!(!rig.s.sync_status.contains("Couldn't"), "{kind}: {}", rig.s.sync_status);
    }
}

#[test]
fn another_client_moving_a_message_the_owner_marked_does_not_lose_it() {
    // A server that keeps ids takes the mark, and the message comes back where the other client put it.
    let mut rig = Rig::new(Profile::standard());
    rig.deliver("m1", &[], 1);
    rig.deliver("m2", &[], 2);
    rig.full();
    rig.s.run("mail.markRead", &json!({"ids": [rig.id("m1")]})).unwrap();
    let id = rig.server_id("m1").unwrap();
    let archive = rig.archive.clone();
    rig.server.with(|d| assert!(d.move_to(&id, &archive).is_some()));
    rig.quick();
    assert_eq!(rig.there("m1").unwrap(), (vec!["$seen".to_string()], "Archive".to_string()));
    // (The Inbox is the whole of the mailbox, and no longer lists it: it left here at the end of the run.)
    assert!(!rig.has("m1"));
    rig.full();
    assert_eq!(rig.mail("Archive"), ["m1"]);
    assert!(rig.message("m1").read);
    rig.agrees();
}

#[test]
fn an_answer_that_was_lost_is_healed_by_the_run_that_follows_and_nothing_is_done_twice() {
    for (kind, profile) in kinds() {
        // Marks: the server makes them, and the connection drops before it can say so. The windows read all the same say
        // what the server has, which is what was asked: nothing is left to send.
        let mut rig = Rig::new(profile);
        for (n, subject) in ["m1", "m2", "m3"].into_iter().enumerate() {
            rig.deliver(subject, &[], n + 1);
        }
        rig.full();
        rig.s.run("mail.markRead", &json!({"ids": [rig.id("m1")]})).unwrap();
        rig.s.run("mail.flag", &json!({"ids": [rig.id("m3")]})).unwrap();
        rig.server.with(|d| d.lost_answers = 1);
        let answers = rig.quick();
        let last = answers.last().unwrap();
        assert!(last["errors"][0].as_str().unwrap().starts_with("Couldn't send 2 changes to the server: "), "{kind}: {last}");
        assert!(!rig.s.sync_status.contains("Up to date"), "{kind}: {}", rig.s.sync_status);
        assert_eq!((rig.there("m1").unwrap().0, rig.there("m3").unwrap().0), (vec!["$seen".to_string()], vec!["$flagged".to_string()]), "{kind}");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.server.with(|d| d.seen.clear());
        rig.quick();
        assert!(rig.sets().is_empty(), "{kind}");
        rig.full();
        rig.agrees();
    }
}

#[test]
fn a_move_whose_answer_was_lost_is_found_again_in_the_run_that_made_it_and_is_not_made_twice() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        for (n, subject) in ["m1", "m2", "m3"].into_iter().enumerate() {
            rig.deliver(subject, &[], n + 1);
        }
        rig.full();
        let (m2, count) = (rig.id("m2"), rig.s.mailbox.messages.len());
        rig.s.run("mail.archive", &json!({"ids": [m2]})).unwrap();
        // The server makes the move, and the connection drops before it can say so.
        rig.server.with(|d| d.lost_answers = 1);
        let answers = rig.quick();
        let last = answers.last().unwrap();
        assert!(last["errors"][0].as_str().unwrap().starts_with("Couldn't send 1 change to the server: "), "{kind}: {last}");
        assert!(!rig.s.sync_status.contains("Up to date"), "{kind}: {}", rig.s.sync_status);
        assert_eq!(rig.there("m2").unwrap().1, "Archive", "{kind}");
        // The windows read all the same: m2 was found again in the Archive, in this very run, as the same message.
        assert_eq!((rig.s.mailbox.messages.len(), rig.id("m2")), (count, m2), "{kind}");
        assert_eq!(rig.mail("Archive"), ["m2"], "{kind}");
        assert_eq!(rig.local_server_id("m2"), rig.server_id("m2").unwrap(), "{kind}");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.server.with(|d| d.seen.clear());
        rig.quick();
        assert!(rig.sets().is_empty(), "{kind}");
        rig.full();
        rig.agrees();
    }
}

#[test]
fn a_move_into_the_inbox_whose_answer_was_lost_is_found_again_though_the_window_it_left_is_cut_short() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        // The Archive holds a number of messages, the newest of them the one the owner takes out of it.
        let archive = rig.archive.clone();
        for n in 1..=6 {
            rig.deliver_in(&archive, &format!("old{n}"), &[], &format!("2026-10-0{n}T00:00:00Z"));
        }
        rig.deliver_in(&archive, "mine", &[], "2026-10-09T00:00:00Z");
        rig.deliver("first", &[], 1);
        rig.full();
        let (mine, count) = (rig.id("mine"), rig.s.mailbox.messages.len());
        let inbox = rig.folder("Inbox");
        // From now on a window holds three messages: the Archive, of which the owner looks at the newest, is cut short.
        rig.s.prefs.sync_limit = 3;
        let on_screen = rig.folder("Archive");
        rig.s.run("folder.select", &json!({"id": on_screen})).unwrap();
        rig.s.run("mail.move", &json!({"ids": [mine], "folder": inbox})).unwrap();
        rig.server.with(|d| d.lost_answers = 1);
        let answers = rig.quick();
        assert!(errors_of(&answers).iter().any(|line| line.starts_with("Couldn't send 1 change to the server: ")), "{kind}: {answers:?}");
        // The server made the move; here the message is the same one, in the Inbox, under the id the server has.
        assert_eq!(rig.there("mine").unwrap().1, "Inbox", "{kind}");
        assert_eq!((rig.s.mailbox.messages.len(), rig.id("mine")), (count, mine), "{kind}");
        assert_eq!(rig.message("mine").folder, inbox, "{kind}");
        assert_eq!(rig.local_server_id("mine"), rig.server_id("mine").unwrap(), "{kind}");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.server.with(|d| d.seen.clear());
        rig.quick();
        assert!(rig.sets().is_empty(), "{kind}");
        rig.full();
        rig.agrees();
    }
}

#[test]
fn a_change_the_server_refuses_is_put_back_said_once_and_not_sent_again() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        rig.deliver("m1", &[], 1);
        rig.deliver("m2", &[], 2);
        rig.full();
        let id = rig.server_id("m1").unwrap();
        rig.server.with(|d| d.refused_sets.insert(id.clone(), "forbidden"));
        rig.s.run("mail.markRead", &json!({"ids": [rig.id("m1")]})).unwrap();
        rig.s.run("mail.markRead", &json!({"ids": [rig.id("m2")]})).unwrap();
        let answers = rig.quick();
        assert_eq!(errors_of(&answers), ["Couldn't mark 'm1' as read: the server says this account may not change it."], "{kind}");
        assert!(!rig.message("m1").read && rig.message("m2").read, "{kind}: put back for the one, and the other went through");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.server.with(|d| d.seen.clear());
        rig.quick();
        assert!(rig.sets().is_empty() && !rig.s.sync_status.contains("Couldn't"), "{kind}");
        rig.agrees();
    }
}

#[test]
fn a_busy_server_keeps_the_changes_waiting_and_the_next_run_sends_them() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        rig.deliver("m1", &[], 1);
        rig.full();
        rig.s.run("mail.markRead", &json!({"ids": [rig.id("m1")]})).unwrap();
        rig.server.with(|d| d.refuse_next = [Some((503, Some(20)))].into());
        let answers = rig.quick();
        assert_eq!(answers.last().unwrap()["busyFor"], 20, "{kind}");
        assert_eq!(rig.waiting(), 1, "{kind}: it is still to be sent");
        assert!(rig.there("m1").unwrap().0.is_empty(), "{kind}");
        rig.quick();
        assert_eq!(rig.there("m1").unwrap().0, ["$seen"], "{kind}");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.agrees();
    }
}

#[test]
fn a_great_many_changes_go_up_in_calls_the_server_takes_and_all_of_them_arrive() {
    for (kind, profile) in kinds() {
        let mut rig = Rig::new(profile);
        let subjects: Vec<String> = (1..=40).map(|n| format!("n{n:02}")).collect();
        for (n, subject) in subjects.iter().enumerate() {
            rig.deliver(subject, &[], n % 20 + 1);
        }
        rig.server.with(|d| d.max_set = 7);
        rig.full();
        // 40 read, the first 12 of them moved, and the last 6 deleted for good.
        let ids: Vec<Id> = subjects.iter().map(|s| rig.id(s)).collect();
        rig.s.run("mail.markRead", &json!({"ids": ids})).unwrap();
        rig.s.run("mail.archive", &json!({"ids": ids[..12]})).unwrap();
        rig.s.run("mail.deletePermanently", &json!({"ids": ids[34..]})).unwrap();
        // 34 messages marked and 12 of them moved are one change each, and 6 destroyed.
        assert_eq!(rig.waiting(), 34 + 12 + 6, "{kind}");
        rig.server.with(|d| d.seen.clear());
        let answers = rig.quick();
        assert!(answers.iter().all(|a| a["errors"] == json!([])), "{kind}: {answers:?}");
        // The marks of the 34 go first, then the 12 moves and the 6 destroys: 18 objects, all in calls of 7.
        assert_eq!(rig.sets(), [(7, 0), (7, 0), (7, 0), (7, 0), (6, 0), (7, 0), (5, 2), (0, 4)], "{kind}");
        assert_eq!(rig.waiting(), 0, "{kind}");
        rig.full();
        rig.agrees();
        assert_eq!(rig.mail("Archive").len(), 12, "{kind}");
        assert_eq!(rig.s.mailbox.messages.values().filter(|m| m.account == rig.account).count(), 34, "{kind}");
    }
}

#[test]
fn what_is_printed_of_a_request_with_changes_has_counts_only() {
    let mut rig = Rig::new(Profile::standard());
    rig.deliver("secret subject", &[], 1);
    rig.full();
    rig.s.run("mail.markRead", &json!({"ids": [rig.id("secret subject")]})).unwrap();
    rig.s.run("mail.archive", &json!({"ids": [rig.id("secret subject")]})).unwrap();
    let request = jmap_mail_request(&rig.s, rig.account, false, &[], false).unwrap();
    assert_eq!(request.changes.len(), 2);
    // What a log may show of it, and of the answer: how many, and nothing of any message (not its id, its mailbox or its subject).
    let id = rig.local_server_id("secret subject");
    let done = |change: &JmapChange| JmapChangeDone { change: change.clone(), outcome: JmapOutcome::Done { id: id.clone() } };
    let result = JmapMailResult { account: rig.account, changes: request.changes.iter().map(done).collect(), ..Default::default() };
    let shown = format!("{request:?} {result:?}");
    assert!(shown.contains("changes: 2"), "{shown}");
    assert!(!shown.contains(&id) && !shown.contains(&rig.inbox) && !shown.contains(&rig.archive), "{shown}");
    assert!(!shown.contains('@') && !shown.contains("secret") && !shown.contains("example.org"), "{shown}");
}
