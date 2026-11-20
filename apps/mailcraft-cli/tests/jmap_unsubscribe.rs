// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Unsubscribe from a JMAP account's mail, from end to end: a Neoffice server names the mail server behind it in its session
//! (`mailcraft_jmap::MAIL_SERVER`), a run takes that in (`Account::jmap_mail_host`), and the `Authentication-Results` that server
//! wrote on top of a message are believed as an IMAP account's server's are; a pass written under them in its name is the
//! sender's, and is not. The messages come whole through the in-process server, their header in the order it was written.
//! Nothing here leaves this process; every name, address and id is invented.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicBool;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::jmap_fetch::{apply_jmap_fetch, jmap_fetch_request};
use mailcraft_engine::cmd::jmap_mail::{apply_jmap_mail, jmap_mail_request};
use mailcraft_jmap::fetch::fetch_messages;
use mailcraft_jmap::standin::{SESSION, StandIn, TOKEN, USER};
use mailcraft_jmap::{Auth, Client, block_on, sync_mail};
use mailcraft_model::unsubscribe::Way;
use mailcraft_model::{Id, Mailbox};
use serde_json::json;

/// What the mailbox's mail server writes on top of mail that passed its checks, and of mail that didn't.
const PASSED: &str = "mail.neo.example; spf=pass smtp.mailfrom=news.example; dkim=pass header.d=news.example; dmarc=pass header.from=news.example";
const FAILED: &str = "mail.neo.example; spf=fail smtp.mailfrom=news.example; dmarc=none header.from=news.example";
/// What a sender writes in the server's name, hoping to be believed.
const FORGED: &str = "mail.neo.example; dmarc=pass header.from=news.example";

/// A letter from a list, with these `Authentication-Results`, topmost first.
fn letter(subject: &str, results: &[&str]) -> Vec<u8> {
    let mut raw: String = results.iter().map(|value| format!("Authentication-Results: {value}\r\n")).collect();
    raw.push_str(&format!(
        "From: Lumen News <letter@news.example>\r\nTo: <{USER}>\r\nSubject: {subject}\r\nMessage-ID: <{subject}@news.example>\r\n\
Date: Wed, 14 Oct 2026 09:00:00 +0000\r\nList-Unsubscribe: <mailto:leave@news.example?subject=leave>, <https://news.example/u/7>\r\n\
List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\r\nThis week's news.\r\n"
    ));
    raw.into_bytes()
}

struct Rig {
    server: StandIn,
    s: Session,
    account: Id,
}

impl Rig {
    /// A JMAP account in a hosting page's tab, against a server that names its mail server `host` (or none), with the two
    /// letters in its Inbox: "honest" (the server's pass alone) and "forged" (the server's failure, then a pass in its name).
    fn new(host: Option<&str>) -> Rig {
        let server = StandIn::new();
        server.with(|d| {
            d.incoming_host = host.map(str::to_string);
            let inbox = d.mailbox("Inbox", None, Some("inbox"));
            d.mailbox("Sent", None, Some("sent"));
            d.deliver(&inbox, &letter("honest", &[PASSED]), &[], "2026-10-14T09:00:00Z");
            d.deliver(&inbox, &letter("forged", &[FAILED, FORGED]), &[], "2026-10-14T09:01:00Z");
        });
        let mut s = Session::at(Mailbox::with_local_account("", ""), 1_791_993_600, 0);
        s.jmap_runs = true;
        s.keeps_local = false;
        let account = s.run("account.add", &json!({"email": USER, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let mut rig = Rig { server, s, account };
        rig.run();
        rig
    }

    /// One whole run, as a host does it.
    fn run(&mut self) {
        let request = jmap_mail_request(&self.s, self.account, true, &[], false).unwrap();
        let client = block_on(Client::connect(self.server.clone(), &request.session, &Auth::bearer(TOKEN))).unwrap();
        let s = &mut self.s;
        block_on(sync_mail(&client, &request, &AtomicBool::new(false), &mut |piece| {
            apply_jmap_mail(s, piece).unwrap();
        }));
    }

    /// The letter whose subject is `subject`, fetched whole: its id here.
    fn opened(&mut self, subject: &str) -> Id {
        let id = self.s.mailbox.messages.values().find(|m| m.subject == subject).map(|m| m.id).unwrap();
        let request = jmap_fetch_request(&self.s, self.account, Some(id), 10).unwrap();
        let client = block_on(Client::connect(self.server.clone(), &request.session, &Auth::bearer(TOKEN))).unwrap();
        let result = block_on(fetch_messages(&client, &request, &AtomicBool::new(false)));
        apply_jmap_fetch(&mut self.s, result).unwrap();
        assert!(!self.s.mailbox.message(id).unwrap().unfetched, "{subject} came whole");
        id
    }
}

#[test]
fn the_mail_servers_own_pass_is_believed_and_one_written_under_it_is_not() {
    let mut rig = Rig::new(Some("Mail.Neo.Example."));
    // The run took in the server the session names.
    assert_eq!(rig.s.mailbox.account(rig.account).unwrap().jmap_mail_host, "mail.neo.example");
    let honest = rig.opened("honest");
    let forged = rig.opened("forged");
    // The header is as the sender's server wrote it, topmost first: the mail server's own results on top.
    let order: Vec<String> = rig.s.mailbox.message(forged).unwrap().header_values("Authentication-Results").map(str::to_string).collect();
    assert_eq!(order, [FAILED, FORGED]);
    // Its own pass: believed, and the list is asked by mail in a hosting page's tab.
    let way = rig.s.unsubscribe_way(honest).unwrap();
    assert!(matches!(&way, Way::Mail { to, .. } if to == "leave@news.example"), "{way:?}");
    // A pass under its failure, in its name: not believed.
    let refused = rig.s.unsubscribe_way(forged).unwrap_err();
    assert!(refused.contains("doesn't vouch"), "{refused}");
}

#[test]
fn without_a_mail_server_named_nothing_is_believed() {
    let mut rig = Rig::new(None);
    assert_eq!(rig.s.mailbox.account(rig.account).unwrap().jmap_mail_host, "");
    let honest = rig.opened("honest");
    assert!(rig.s.unsubscribe_way(honest).unwrap_err().contains("doesn't vouch"));
}

#[test]
fn a_server_that_stops_naming_it_is_no_longer_believed() {
    let mut rig = Rig::new(Some("mail.neo.example"));
    let honest = rig.opened("honest");
    assert!(rig.s.unsubscribe_way(honest).is_ok());
    rig.server.with(|d| d.incoming_host = None);
    rig.run();
    assert_eq!(rig.s.mailbox.account(rig.account).unwrap().jmap_mail_host, "");
    assert!(rig.s.unsubscribe_way(honest).is_err());
}
