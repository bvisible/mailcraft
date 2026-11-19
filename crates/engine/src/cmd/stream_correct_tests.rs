// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! « Classer dans » in a hosting page's tab: the engine puts the messages under the tab chosen, their sender's others with them,
//! and asks the server, with answers made by hand (the server itself: frappe_webmail's `tests/test_mailcraft_stream.py`).

use mailcraft_model::sync::ServerMessage;
use mailcraft_model::{Address, FolderKind, Id, Mailbox, Message, ServerFolder, Stream};
use serde_json::json;

use super::*;
use crate::list;

/// Thursday 2026-11-19 12:00 UTC.
const NOW: i64 = 1_795_089_600;
const ME: &str = "owner@example.org";

/// A hosting page's tab with a JMAP account whose server takes a choice of tab, and its Inbox: the session, the account and
/// the Inbox.
fn setup() -> (Session, Id, Id) {
    let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
    s.jmap_runs = true;
    s.keeps_local = false;
    let account = s
        .run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap/session", "bearer": true}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    s.mailbox.folder_mut(inbox).unwrap().server = Some(ServerFolder { id: "Min".into(), name: "INBOX".into(), ..Default::default() });
    s.run("mail.offerStreamCorrection", &json!({"account": ME, "offered": true})).unwrap();
    (s, account, inbox)
}

fn add(s: &mut Session, folder: Id, id: &str, from: &str, subject: &str) -> Id {
    let at = s.mailbox.folder(folder).and_then(|f| f.server.as_ref()).map(|server| server.id.clone()).unwrap_or_default();
    let server = ServerMessage { id: id.into(), folder: at, read: true, ..Default::default() };
    let message = Message { folder, from: Address::new("", from), subject: subject.into(), read: true, server: Some(server), ..Default::default() };
    s.mailbox.add_message(message).unwrap()
}

fn stream(s: &Session, id: Id) -> Option<Stream> {
    s.mailbox.message(id).unwrap().stream
}

#[test]
fn a_message_goes_under_the_tab_chosen_with_its_senders_others_and_the_server_is_asked() {
    let (mut s, _, inbox) = setup();
    let shop = add(&mut s, inbox, "E1", "News@Shop.example", "Offers of the week");
    let shop_again = add(&mut s, inbox, "E2", "news@shop.example", "Offers of the month");
    let friend = add(&mut s, inbox, "E3", "ada@example.net", "Lunch?");
    s.run(
        "mail.setStreams",
        &json!({"streams": [{"id": shop, "stream": "person"}, {"id": shop_again, "stream": "person"}, {"id": friend, "stream": "person"}]}),
    )
    .unwrap();
    let steps = s.undo_steps_moved();
    let answer = s.run("mail.correctStream", &json!({"ids": [shop], "stream": "newsletter"})).unwrap();
    assert_eq!((answer["changed"].clone(), answer["asked"].clone()), (json!(2), json!(1)));
    assert_eq!((stream(&s, shop), stream(&s, shop_again)), (Some(Stream::Newsletter), Some(Stream::Newsletter)), "the sender's other follows");
    assert_eq!(stream(&s, friend), Some(Stream::Person), "another sender stays");
    assert_eq!(s.undo_steps_moved(), steps, "no step: the server keeps the choice");
    let asked = s.hosted_stream.take();
    assert_eq!(asked.len(), 1);
    assert_eq!((asked[0].message, asked[0].email_id.as_str(), asked[0].stream), (shop, "E1", Stream::Newsletter));
    assert_eq!(asked[0].was, [(shop, Some(Stream::Person)), (shop_again, Some(Stream::Person))]);
    assert!(s.hosted_stream.take().is_empty(), "taken once");
    // The server took it: nothing comes back, nothing is said.
    let told = s.run("mail.streamCorrected", &json!({"message": shop, "ok": true})).unwrap();
    assert_eq!(told["back"], 0);
    assert!(!s.hosted_stream.asking());
    assert!(s.hosted_stream.said.is_empty());
    assert_eq!(stream(&s, shop_again), Some(Stream::Newsletter));
}

#[test]
fn a_refusal_puts_back_what_went_and_the_person_is_told() {
    let (mut s, _, inbox) = setup();
    let alert = add(&mut s, inbox, "E1", "noreply@bank.example", "Your statement");
    let other = add(&mut s, inbox, "E2", "noreply@bank.example", "A login");
    s.run("mail.setStreams", &json!({"streams": [{"id": alert, "stream": "notification"}]})).unwrap();
    s.run("mail.setStreams", &json!({"streams": [{"id": other, "stream": "newsletter"}]})).unwrap();
    s.run("mail.correctStream", &json!({"ids": [alert], "stream": "person"})).unwrap();
    assert_eq!((stream(&s, alert), stream(&s, other)), (Some(Stream::Person), Some(Stream::Person)));
    s.hosted_stream.take();
    let told = s.run("mail.streamCorrected", &json!({"message": alert, "ok": false})).unwrap();
    assert_eq!(told["back"], 2);
    assert_eq!((stream(&s, alert), stream(&s, other)), (Some(Stream::Notification), Some(Stream::Newsletter)), "each as it was");
    assert_eq!(s.hosted_stream.said.len(), 1);
    assert!(s.hosted_stream.said[0].contains("Your statement"));
    // An answer nobody awaits changes nothing.
    assert_eq!(s.run("mail.streamCorrected", &json!({"message": alert, "ok": false})).unwrap()["known"], false);
}

#[test]
fn only_a_message_of_an_inbox_whose_server_keeps_the_choice_is_offered() {
    let (mut s, account, inbox) = setup();
    let m = add(&mut s, inbox, "E1", "news@shop.example", "Offers");
    assert!(offered(&s, &[m]));
    assert!(available(&s));
    // Another folder.
    let archive = s.mailbox.folder_of_kind(account, FolderKind::Archive).unwrap();
    let archived = add(&mut s, archive, "E2", "news@shop.example", "Old offers");
    assert!(!offered(&s, &[archived]));
    assert!(!offered(&s, &[m, archived]), "all of the selection, or nothing");
    assert!(s.run("mail.correctStream", &json!({"ids": [archived], "stream": "newsletter"})).is_err());
    // Not on its server yet.
    let unsent =
        s.mailbox.add_message(Message { folder: inbox, from: Address::new("", "a@example.net"), subject: "x".into(), ..Default::default() }).unwrap();
    assert!(!offered(&s, &[unsent]));
    // Nothing selected, and a word that is no tab.
    assert!(!offered(&s, &[]));
    assert!(s.run("mail.correctStream", &json!({"ids": [m], "stream": "promotion"})).is_err());
    // The server does not take it (any more): not offered, and nothing waits to be told it.
    s.run("mail.correctStream", &json!({"ids": [m], "stream": "newsletter"})).unwrap();
    s.run("mail.offerStreamCorrection", &json!({"account": ME, "offered": false})).unwrap();
    assert!(!offered(&s, &[m]));
    assert!(!available(&s));
    assert!(!s.hosted_stream.asking());
    assert!(s.run("mail.correctStream", &json!({"ids": [m], "stream": "person"})).is_err());
}

#[test]
fn a_tab_that_keeps_its_mail_on_this_computer_never_offers_it() {
    let (mut s, _, inbox) = setup();
    let m = add(&mut s, inbox, "E1", "news@shop.example", "Offers");
    s.keeps_local = true;
    assert!(!offered(&s, &[m]));
    assert!(!available(&s));
}

#[test]
fn the_message_leaves_the_tab_in_view_and_the_selection_goes_on() {
    let (mut s, _, inbox) = setup();
    let first = add(&mut s, inbox, "E1", "ada@example.net", "Lunch?");
    let second = add(&mut s, inbox, "E2", "news@shop.example", "Offers");
    s.run("mail.setStreams", &json!({"streams": [{"id": first, "stream": "person"}, {"id": second, "stream": "person"}]})).unwrap();
    s.run("view.filter", &json!({"value": "main"})).unwrap();
    s.run("mail.select", &json!({"id": second})).unwrap();
    s.run("mail.correctStream", &json!({"stream": "newsletter"})).unwrap();
    // Principal lists it no more as a message of its own: in the Newsletters' group row, which opens their tab.
    let rows = list::rows(&s);
    assert!(rows.iter().all(|r| r.bundle.is_some() || r.id != second), "not a row of its own in Principal");
    assert!(rows.iter().any(|r| r.bundle == Some(Stream::Newsletter) && r.members.contains(&second)), "in the Newsletters' group");
    assert!(!s.sel.messages.contains(&second), "the selection went on");
    // The tab it is under now lists it.
    s.run("view.filter", &json!({"value": "newsletters"})).unwrap();
    assert!(list::rows(&s).iter().any(|r| r.id == second));
}

#[test]
fn the_tab_all_messages_share_is_the_one_marked() {
    let (mut s, _, inbox) = setup();
    let a = add(&mut s, inbox, "E1", "a@example.net", "a");
    let b = add(&mut s, inbox, "E2", "b@example.net", "b");
    assert_eq!(current(&s, &[a, b]), Some(Stream::Person), "nobody said anything of them: the main tab");
    s.run("mail.setStreams", &json!({"streams": [{"id": b, "stream": "notification"}]})).unwrap();
    assert_eq!(current(&s, &[a, b]), None);
    assert_eq!(current(&s, &[b]), Some(Stream::Notification));
}

#[test]
fn an_agent_never_chooses_a_tab() {
    let (s, _, _) = setup();
    for command in ["mail.correctStream", "mail.offerStreamCorrection", "mail.streamCorrected"] {
        assert!(s.registry.get(command).is_some(), "{command} is a command");
        assert_eq!(crate::agent::access(command), Some(crate::agent::Access::Never), "{command} is the owner's click or the host's word");
    }
}

#[test]
fn choosing_the_tab_ticked_again_asks_nothing() {
    let (mut s, _, inbox) = setup();
    let news = add(&mut s, inbox, "E1", "news@shop.example", "Offers");
    let again = add(&mut s, inbox, "E2", "news@shop.example", "More offers");
    let unsaid = add(&mut s, inbox, "E3", "ada@example.net", "Lunch?");
    s.run("mail.setStreams", &json!({"streams": [{"id": news, "stream": "newsletter"}, {"id": again, "stream": "newsletter"}]})).unwrap();
    let answer = s.run("mail.correctStream", &json!({"ids": [news], "stream": "newsletter"})).unwrap();
    assert_eq!((answer["changed"].clone(), answer["asked"].clone()), (json!(0), json!(0)));
    // A message nobody said anything of is under the main one: choosing it there asks nothing either.
    let answer = s.run("mail.correctStream", &json!({"ids": [unsaid], "stream": "person"})).unwrap();
    assert_eq!(answer["asked"], 0);
    assert!(!s.hosted_stream.asking(), "nothing to tell the server");
    // But the main one for the newsletter's sender moves both, and asks.
    assert_eq!(s.run("mail.correctStream", &json!({"ids": [news], "stream": "person"})).unwrap()["asked"], 1);
}
