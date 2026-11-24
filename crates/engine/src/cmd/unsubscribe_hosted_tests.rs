// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): `mail.unsubscribe` in a hosting page's tab.
//! A hosting page's tab has no host to post one click with: a list that also gives an address to write to is asked by mail,
//! a message that goes through the mailbox's own run; a list that gives none keeps its way (the window opens its page).
//!
//! The people, lists and hosts are invented.

use mailcraft_model::sync::{FolderDelta, SyncResult};
use mailcraft_model::{Address, FolderKind, Id, Message};
use serde_json::json;

use crate::Session;

const PASSED: &str = "mx.lumen.example; spf=pass smtp.mailfrom=news.example; dkim=pass header.d=news.example; dmarc=pass header.from=news.example";
const SIGNED: &str = "v=1; a=rsa-sha256; d=news.example; s=s1; h=from:subject:list-unsubscribe:list-unsubscribe-post; bh=AA; b=BB";

/// A session with Avery's account, whose Inbox has one message from the list with `headers`; `hosted`: a hosting page's tab.
fn with(hosted: bool, headers: &[(&str, &str)]) -> (Session, Id) {
    let mut s = Session::empty();
    let a = s
        .run(
            "account.add",
            &json!({"email": "avery@lumen.example", "displayName": "Avery Quinn", "incoming": {"host": "imap.lumen.example"}, "outgoing": {"host": "smtp.lumen.example"}}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let m = Message {
        uid: 1,
        from: Address::new("Lumen News", "letter@news.example"),
        subject: "This week".into(),
        message_id: "week@news.example".into(),
        date: 100,
        headers: headers.iter().map(|(h, v)| ((*h).into(), (*v).into())).collect(),
        ..Default::default()
    };
    let result = SyncResult {
        account: a,
        folders: vec![FolderDelta {
            id: None,
            remote: "INBOX".into(),
            kind: FolderKind::Inbox,
            uid_validity: 7,
            uid_seen: 5,
            new: vec![m],
            ..Default::default()
        }],
        ..Default::default()
    };
    s.run("sync.apply", &json!({"result": result})).unwrap();
    s.keeps_local = !hosted;
    let id = s.mailbox.messages.values().find(|m| m.subject == "This week").map(|m| m.id).unwrap();
    (s, id)
}

const ONE_CLICK_AND_MAIL: [(&str, &str); 4] = [
    ("Authentication-Results", PASSED),
    ("List-Unsubscribe", "<https://news.example/u/7f3a>, <mailto:leave@news.example?subject=stop>"),
    ("List-Unsubscribe-Post", "List-Unsubscribe=One-Click"),
    ("DKIM-Signature", SIGNED),
];

#[test]
fn a_hosting_pages_tab_asks_by_mail_where_the_list_gives_an_address() {
    let (mut s, id) = with(true, &ONE_CLICK_AND_MAIL);
    let said = s.run("mail.unsubscribe", &json!({"id": id})).unwrap();
    assert_eq!((said["way"].as_str(), said["to"].as_str()), (Some("mail"), Some("leave@news.example")));
    let outbox = s.mailbox.folder_of_kind(s.mailbox.message(id).unwrap().account, FolderKind::Outbox).unwrap();
    let before = s.mailbox.messages_in(outbox).count();
    assert_eq!(s.run("mail.unsubscribe", &json!({"id": id, "send": true})).unwrap()["sent"], true);
    let queued: Vec<String> = s.mailbox.messages_in(outbox).map(|m| m.subject.clone()).collect();
    assert_eq!((queued.len(), queued.last().map(String::as_str)), (before + 1, Some("stop")), "in the Outbox, for the mailbox's run");
}

#[test]
fn the_program_that_keeps_its_mail_keeps_upstreams_one_click() {
    let (mut s, id) = with(false, &ONE_CLICK_AND_MAIL);
    assert_eq!(s.run("mail.unsubscribe", &json!({"id": id})).unwrap()["way"], "oneClick");
}

#[test]
fn a_list_without_an_address_keeps_its_way_in_a_hosting_pages_tab() {
    let headers = [ONE_CLICK_AND_MAIL[0], ("List-Unsubscribe", "<https://news.example/u/7f3a>"), ONE_CLICK_AND_MAIL[2], ONE_CLICK_AND_MAIL[3]];
    let (mut s, id) = with(true, &headers);
    let said = s.run("mail.unsubscribe", &json!({"id": id})).unwrap();
    assert_eq!(said["way"], "oneClick");
    // Nothing is posted from the engine: one click is the window's, which opens the address as a page in the browser build.
    assert!(s.run("mail.unsubscribe", &json!({"id": id, "send": true})).is_err());
}

/// Several lists at once in a hosting page's tab: each sender once, by the message the list's own address asks for; nothing goes
/// from the engine in bulk, and a list that was asked (from the Outbox, then from Sent Items) is left out, and refused alone.
#[test]
fn a_hosting_pages_tab_asks_several_lists_by_mail_and_none_twice() {
    use super::unsubscribe::ASKED_ALREADY;
    let (mut s, id) = with(true, &ONE_CLICK_AND_MAIL);
    let told = s.run("mail.unsubscribeMany", &json!({"ids": [id]})).unwrap();
    assert_eq!((told["senders"][0]["way"].as_str(), told["senders"][0]["to"].as_str()), (Some("mail"), Some("leave@news.example")));
    assert!(s.run("mail.unsubscribeMany", &json!({"ids": [id], "send": true})).is_err(), "nothing goes in bulk");
    s.run("mail.unsubscribe", &json!({"id": id, "send": true})).unwrap();
    let account = s.mailbox.message(id).unwrap().account;
    let outbox = s.mailbox.folder_of_kind(account, FolderKind::Outbox).unwrap();
    let asked = |s: &Session| s.mailbox.messages.values().filter(|m| m.subject == "stop").count();
    let refused = |s: &mut Session| {
        let alone = s.run("mail.unsubscribe", &json!({"id": id, "send": true})).unwrap_err().to_string();
        assert!(alone.contains(ASKED_ALREADY), "{alone}");
        let told = s.run("mail.unsubscribeMany", &json!({"ids": [id]})).unwrap();
        assert_eq!((told["senders"].as_array().map(Vec::len), told["leftOut"].as_array().map(Vec::len)), (Some(0), Some(1)), "{told}");
    };
    // Waiting in the Outbox, then gone: its copy in Sent Items.
    refused(&mut s);
    let request = s.mailbox.messages_in(outbox).find(|m| m.subject == "stop").map(|m| m.id).unwrap();
    let sent = s.mailbox.folder_of_kind(account, FolderKind::Sent).unwrap();
    s.mailbox.message_mut(request).unwrap().folder = sent;
    refused(&mut s);
    assert_eq!(asked(&s), 1, "once");
    // Looking is still answered, and a list that let a day go by can be asked again.
    assert_eq!(s.run("mail.unsubscribe", &json!({"id": id})).unwrap()["sent"], false);
    let m = s.mailbox.message_mut(request).unwrap();
    (m.date, m.received) = (m.date - 2 * 86_400, m.received - 2 * 86_400);
    s.run("mail.unsubscribe", &json!({"id": id, "send": true})).unwrap();
}
