// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of exchanges.rs.
use serde_json::json;

use crate::exchanges::{Way, key, local, merge, read_linked, read_page, read_someone, wanted};
use crate::sync::ServerMessage;
use crate::*;

const CLIENT: &str = "claire@atelier-exemple.example";
const COLLEAGUE: &str = "marc@atelier-exemple.example";

fn mailbox() -> (Mailbox, Id, Id, Id) {
    let mb = Mailbox::with_local_account("Ada Lovelace", "ada@example.org");
    let account = mb.accounts[0].id;
    let inbox = mb.inbox().unwrap();
    let sent = mb.folder_of_kind(account, FolderKind::Sent).unwrap();
    let trash = mb.folder_of_kind(account, FolderKind::Deleted).unwrap();
    (mb, inbox, sent, trash)
}

fn add(mb: &mut Mailbox, folder: Id, from: &str, to: &str, subject: &str, date: i64) -> Id {
    let m = Message {
        folder,
        subject: subject.into(),
        from: Address::new("", from),
        to: vec![Address::new("", to)],
        date,
        read: true,
        ..Default::default()
    };
    mb.add_message(m).unwrap()
}

#[test]
fn an_address_is_a_key_and_the_wanted_ones_are_kept_once_and_capped() {
    assert_eq!(key("  Claire@Atelier-Exemple.example "), CLIENT);
    assert_eq!(wanted(&[" A@b.example".into(), "a@B.example".into(), "".into(), "c@d.example".into()]), ["a@b.example", "c@d.example"]);
    let many: Vec<String> = (0..20).map(|i| format!("p{i}@example.org")).collect();
    assert_eq!(wanted(&many).len(), crate::exchanges::MAX_ADDRESSES);
}

#[test]
fn the_exchanges_of_the_mailbox_are_what_came_from_them_and_what_went_to_them_the_newest_first() {
    let (mut mb, inbox, sent, trash) = mailbox();
    let a = add(&mut mb, inbox, CLIENT, "ada@example.org", "Offer", 1_000);
    let b = add(&mut mb, sent, "ada@example.org", CLIENT, "Re: Offer", 2_000);
    add(&mut mb, inbox, "someone@elsewhere.example", "ada@example.org", "Not them", 1_500);
    add(&mut mb, trash, CLIENT, "ada@example.org", "Thrown away", 3_000);
    let drafts = mb.folder_of_kind(mb.accounts[0].id, FolderKind::Drafts).unwrap();
    let mut draft =
        Message { folder: drafts, draft: true, subject: "Not sent".into(), from: Address::new("", "ada@example.org"), ..Default::default() };
    draft.to = vec![Address::new("", CLIENT)];
    mb.add_message(draft).unwrap();
    let rows = local(&mb, &[CLIENT.into()], 10);
    assert_eq!(rows.iter().map(|r| r.message).collect::<Vec<_>>(), [Some(b), Some(a)]);
    assert_eq!(rows[0].way, Way::Out);
    assert_eq!(rows[1].way, Way::In);
    assert_eq!(rows[0].subject, "Re: Offer");
    assert_eq!(rows[0].email, CLIENT, "for what went out, the other side is who it went to");
    assert_eq!(rows[1].email, CLIENT);
    assert_eq!(rows[0].kind, Some(FolderKind::Sent));
    assert!(!rows[0].unread);
}

#[test]
fn any_address_of_someone_finds_their_mail_and_the_limit_and_the_empty_ask_are_kept() {
    let (mut mb, inbox, _sent, _trash) = mailbox();
    add(&mut mb, inbox, CLIENT, "ada@example.org", "One", 1);
    add(&mut mb, inbox, COLLEAGUE, "ada@example.org", "Two", 2);
    add(&mut mb, inbox, CLIENT.to_uppercase().as_str(), "ada@example.org", "Three", 3);
    assert_eq!(local(&mb, &[CLIENT.into()], 10).len(), 2);
    assert_eq!(local(&mb, &[CLIENT.into(), COLLEAGUE.into()], 10).len(), 3);
    assert_eq!(local(&mb, &[CLIENT.into(), COLLEAGUE.into()], 2).len(), 2);
    assert!(local(&mb, &[], 10).is_empty());
    assert!(local(&mb, &[CLIENT.into()], 0).is_empty());
}

#[test]
fn a_copy_to_them_is_an_exchange_that_went_out() {
    let (mut mb, inbox, _sent, _trash) = mailbox();
    let mut m = Message { folder: inbox, subject: "Fyi".into(), from: Address::new("", COLLEAGUE), date: 5, read: false, ..Default::default() };
    m.to = vec![Address::new("", "ada@example.org")];
    m.cc = vec![Address::new("", CLIENT)];
    mb.add_message(m).unwrap();
    let rows = local(&mb, &[CLIENT.into()], 10);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].way, Way::Out);
    assert!(rows[0].unread);
}

#[test]
fn the_window_s_own_message_wins_over_the_server_s_row_of_the_same_message() {
    let (mut mb, inbox, _sent, _trash) = mailbox();
    let id = add(&mut mb, inbox, CLIENT, "ada@example.org", "Offer", 1_000);
    let mut stored = (**mb.messages.get(&id).unwrap()).clone();
    stored.server = Some(ServerMessage { id: "E1V1ZSU5CT1g".into(), ..Default::default() });
    mb.messages.insert(id, std::sync::Arc::new(stored));
    let mine = local(&mb, &[CLIENT.into()], 10);
    assert_eq!(mine[0].server.as_deref(), Some("E1V1ZSU5CT1g"));
    let page = read_page(&json!({"list": [
        {"id": "E1V1ZSU5CT1g", "subject": "Offer", "receivedAt": "1970-01-01T00:16:40Z", "direction": "in", "from": {"name": "", "email": CLIENT}},
        {"id": "E2V9Z0FyY2hpdmU", "subject": "Older", "receivedAt": "1970-01-01T00:01:00Z", "direction": "in", "from": {"name": "", "email": CLIENT}, "folder": "Archive", "role": "archive"},
    ]}))
    .unwrap();
    let all = merge(mine, &page.rows);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].message, Some(id));
    assert_eq!(all[1].server.as_deref(), Some("E2V9Z0FyY2hpdmU"));
    assert_eq!(all[1].message, None);
    assert_eq!(all[1].kind, Some(FolderKind::Archive));
    // The same server row twice is one
    let twice = merge(Vec::new(), &[page.rows[1].clone(), page.rows[1].clone()]);
    assert_eq!(twice.len(), 1);
}

#[test]
fn a_page_is_read_as_text_with_its_total_next_and_folders() {
    let answer = json!({
        "list": [
            {
                "id": "E1V5Z", "mailboxId": "M1", "folder": "Sent", "role": "sent", "subject": "Re: Offer",
                "from": {"name": "Ada", "email": "ada@example.org"}, "to": [{"name": "Claire", "email": CLIENT}],
                "receivedAt": "2026-10-29T12:00:00Z", "unread": false, "hasAttachment": true,
                "attachments": [{"id": "0", "name": "Offer.pdf", "type": "application/pdf", "size": 84000}],
                "direction": "out"
            },
            {"subject": "no id: left out", "receivedAt": "2026-10-29T12:00:00Z"},
            {"id": "E1V6Z", "subject": "no date: left out"},
            "not an object"
        ],
        "total": 4, "next": "2026-10-29T12:00:00Z", "scanned": {"folders": 3, "of": 5}, "partial": true
    });
    let page = read_page(&answer).unwrap();
    assert_eq!(page.rows.len(), 1);
    let row = &page.rows[0];
    assert_eq!((row.way, row.kind, row.folder.as_str()), (Way::Out, Some(FolderKind::Sent), "Sent"));
    assert_eq!((row.name.as_str(), row.email.as_str()), ("Claire", CLIENT), "for what went out: who it went to");
    assert_eq!(row.files.len(), 1);
    assert_eq!((row.files[0].name.as_str(), row.files[0].size), ("Offer.pdf", 84_000));
    assert!(row.clip);
    assert_eq!(row.when, mailcraft_time::iso_to_timestamp("2026-10-29T12:00:00Z", 0).unwrap());
    assert_eq!((page.total, page.next.as_deref(), page.folders, page.partial), (Some(4), Some("2026-10-29T12:00:00Z"), (3, 5), true));
    assert!(read_page(&json!({"nothing": 1})).is_none());
    let last = read_page(&json!({"list": [], "total": null, "next": null})).unwrap();
    assert_eq!((last.rows.len(), last.total, last.next), (0, None, None));
}

#[test]
fn what_a_stranger_wrote_is_capped() {
    let long = "x".repeat(10_000);
    let page = read_page(&json!({"list": [{"id": "E1V1Z", "receivedAt": "2026-10-29T12:00:00Z", "subject": long}]})).unwrap();
    assert_eq!(page.rows[0].subject.chars().count(), 300);
}

#[test]
fn who_someone_is_gets_the_asked_address_first_and_a_party_when_there_is_one() {
    let answer = json!({
        "address": "Claire@Atelier-Exemple.example", "known": true,
        "addresses": [COLLEAGUE, CLIENT], "domains": ["atelier-exemple.example"],
        "contact": {"fullName": "Claire Martin", "company": "Atelier Exemple SA"},
        "party": {"doctype": "Customer", "name": "CUST-1", "title": "Atelier Exemple SA", "kind": "Client", "url": "https://x.example/app/customer/CUST-1", "everything": "https://x.example/app/history/Customer/CUST-1", "invoices": 3},
        "lead": null
    });
    let someone = read_someone(&answer).unwrap();
    assert_eq!(someone.address, CLIENT);
    assert_eq!(someone.addresses, [CLIENT, COLLEAGUE], "the asked address is first, once");
    assert!(someone.known);
    assert_eq!((someone.name.as_str(), someone.company.as_str()), ("Claire Martin", "Atelier Exemple SA"));
    let party = someone.party.unwrap();
    assert_eq!((party.kind.as_str(), party.invoices), ("Client", Some(3)));
    assert!(party.everything.ends_with("/app/history/Customer/CUST-1"));
    let nobody = read_someone(&json!({"address": CLIENT, "known": false, "addresses": [], "contact": null, "party": null})).unwrap();
    assert_eq!(nobody.addresses, [CLIENT]);
    assert!(nobody.party.is_none() && !nobody.known);
    assert!(read_someone(&json!({"known": true})).is_none());
}

#[test]
fn what_neoffice_links_someone_to_is_read_with_its_amount_and_tone() {
    let linked = read_linked(&json!({
        "thing": {"doctype": "Customer", "name": "CUST-1", "everything": "https://x.example/app/history/Customer/CUST-1"},
        "total": 5,
        "rows": [
            {"kind": "Facture", "doctype": "Sales Invoice", "name": "SI-2", "number": "SI-2", "title": "Septembre", "date": "2026-09-30", "amount": 1260.0, "currency": "CHF", "status": "Impayée", "tone": "warn", "url": "https://x.example/app/sales-invoice/SI-2"},
            {"kind": "Devis", "doctype": "Quotation", "name": "", "title": "no name: left out"}
        ]
    }))
    .unwrap();
    assert_eq!(linked.total, 5);
    assert_eq!(linked.docs.len(), 1);
    let doc = &linked.docs[0];
    assert_eq!((doc.kind.as_str(), doc.amount, doc.currency.as_str(), doc.tone.as_str()), ("Facture", Some(1260.0), "CHF", "warn"));
    assert_eq!(doc.reference, "SI-2 · 2026-09-30");
    assert!(linked.everything.ends_with("/app/history/Customer/CUST-1"));
    let none = read_linked(&json!({"thing": null, "total": 0, "rows": []})).unwrap();
    assert!(none.docs.is_empty() && none.everything.is_empty());
    assert!(read_linked(&json!({"total": 1})).is_none());
}
