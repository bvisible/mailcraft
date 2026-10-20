// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What the contact form offers for a contact of a server's address book, the books a new contact may go to, and the
//! reasons: in a window without a screen. Invented people at `example.org`.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use mailcraft_engine::Session;
use mailcraft_engine::cmd::remote_contacts::{self, apply_books};
use mailcraft_model::book::{BooksDownload, RemoteBook};
use mailcraft_model::remote::{ContactsDownload, RemoteContact};
use mailcraft_model::{Id, Labeled};
use serde_json::json;

use super::{PLAIN_PHONE_LABELS, REASONS, book_choices, contact_keeps, offers_books};
use crate::{MailApp, Services};

const ME: &str = "ada@example.org";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
/// What the form says under the rows of a book that keeps only the basics.
const KEEPS_ONLY: &str = "Its address book keeps only a contact's name, company, job title, e-mail addresses and phone numbers";

/// The sample mailbox with a JMAP account whose server has the team's book (Ada may change it; it keeps only the
/// basics) with Bob in it, and Grace's (read only); the account's id.
fn session() -> (Session, Id) {
    let mut s = Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0);
    let added = s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap/session"}})).unwrap();
    let account = added["id"].as_u64().unwrap();
    let book = |id: &str, name: &str, read_only: bool| RemoteBook {
        id: id.into(),
        name: name.into(),
        read_only,
        may_remove: !read_only,
        plain: true,
        ..Default::default()
    };
    let books = vec![RemoteBook { main: true, ..book("team", "Team", false) }, book("grace", "Grace's", true)];
    apply_books(&mut s, &BooksDownload { account, email: ME.into(), complete: true, books }).unwrap();
    let bob = RemoteContact {
        id: "c-bob".into(),
        book: "team".into(),
        first: "Bob".into(),
        emails: vec![Labeled::new("", "bob@example.org")],
        ..Default::default()
    };
    remote_contacts_apply(&mut s, account, vec![bob]);
    (s, account)
}

fn remote_contacts_apply(s: &mut Session, account: Id, contacts: Vec<RemoteContact>) {
    mailcraft_engine::cmd::remote::apply_contacts(s, &ContactsDownload { account, email: ME.into(), complete: true, contacts }).unwrap();
}

fn contact(s: &Session, first: &str) -> Id {
    s.mailbox.contacts.values().find(|c| c.first == first).map(|c| c.id).unwrap()
}

/// The window with the dialog `name` open, laid out a few frames.
fn dialog(s: Session, name: &str, params: serde_json::Value) -> Harness<'static, MailApp> {
    let mut app = MailApp::new(s, Services::default());
    app.autosave = false;
    let mut p = params;
    p["name"] = json!(name);
    app.execute("ui.dialog", p).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 900.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(6);
    harness
}

#[test]
fn a_plain_book_hides_or_greys_the_rows_it_does_not_keep_with_the_reason() {
    let (s, _) = session();
    let bob = contact(&s, "Bob");
    let keeps = contact_keeps(&s, Some(bob), None);
    let team = s.mailbox.address_book(keeps.book.unwrap()).unwrap();
    assert_eq!(team.name, "Team");
    assert!(!keeps.everything && !keeps.display && !keeps.labels && keeps.local);
    assert_eq!(keeps.phone_labels(&["mobile", "work", "home", "fax", "other"]), PLAIN_PHONE_LABELS);
    assert!(keeps.choices.is_empty(), "an existing contact stays in its book");
    // A contact of the mailbox's own keeps everything, as ever.
    let own = s.mailbox.contacts.values().find(|c| c.book == 0).map(|c| c.id).unwrap();
    let all = contact_keeps(&s, Some(own), None);
    assert!(all.everything && all.display && all.labels && all.book.is_none());
    assert_eq!(all.phone_labels(&["mobile", "work"]), ["mobile", "work"]);
    // On screen: the form says why the rows are not there, and not for a contact that keeps everything.
    let h = dialog(s, "editContact", json!({"id": bob}));
    assert!(h.query_by_label(KEEPS_ONLY).is_some(), "the reason is said under the rows");
    let h = dialog(session().0, "editContact", json!({"id": own}));
    assert!(h.query_by_label(KEEPS_ONLY).is_none());
}

#[test]
fn favourite_and_categories_are_greyed_in_a_tab_that_keeps_nothing() {
    let (mut s, _) = session();
    let bob = contact(&s, "Bob");
    assert!(contact_keeps(&s, Some(bob), None).local);
    s.keeps_local = false;
    let keeps = contact_keeps(&s, Some(bob), None);
    assert!(!keeps.local);
    // The form lays them out greyed, and still opens and closes.
    let mut h = dialog(s, "editContact", json!({"id": bob}));
    h.run_steps(2);
    assert!(h.query_by_label("Favorite").is_some());
    assert_eq!(h.state().dialog.as_ref().map(crate::dialogs::Dialog::name), Some("editContact"));
}

#[test]
fn a_new_contact_form_offers_the_writable_books() {
    let (mut s, account) = session();
    let team = s.mailbox.address_book_on_server(account, "team").map(|b| b.id).unwrap();
    // The team's book, which Ada may change, and this computer; Grace's she may only read is not offered.
    assert_eq!(book_choices(&s), [(team, "Team".to_string()), (0, "On this computer".to_string())]);
    let keeps = contact_keeps(&s, None, None);
    assert_eq!(keeps.book, Some(team), "where it goes by default");
    assert!(offers_books(&keeps) && !keeps.everything);
    // Put on this computer, it keeps everything.
    assert!(contact_keeps(&s, None, Some(0)).everything);
    // A tab that keeps nothing offers no place on this computer.
    s.keeps_local = false;
    assert_eq!(book_choices(&s), [(team, "Team".to_string())]);
    // Two books of one name are told apart by their account.
    s.keeps_local = true;
    let other =
        s.run("account.add", &json!({"email": "bo@example.org", "kind": "jmap", "jmap": {"session": "https://bo.example.org/jmap"}})).unwrap()["id"]
            .as_u64()
            .unwrap();
    let book = RemoteBook { id: "team".into(), name: "Team".into(), may_remove: true, ..Default::default() };
    apply_books(&mut s, &BooksDownload { account: other, email: "bo@example.org".into(), complete: true, books: vec![book] }).unwrap();
    let names: Vec<String> = book_choices(&s).into_iter().map(|(_, name)| name).collect();
    assert_eq!(names, ["Team (ada@example.org)", "Team (bo@example.org)", "On this computer"]);
    // Without a server's book, nothing to choose from.
    let plain = Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0);
    assert!(!offers_books(&contact_keeps(&plain, None, None)));
    // On screen, the new contact's form has the choice, set to where it goes by default.
    let h = dialog(s, "contact", json!({}));
    assert!(h.query_by_value("Team (ada@example.org)").is_some());
}

#[test]
fn the_engines_reasons_and_the_tables_are_the_same_strings() {
    assert_eq!(REASONS, [remote_contacts::NO_KEEPING, remote_contacts::NO_ADDING, remote_contacts::NO_CHANGING, remote_contacts::NO_DELETING]);
    assert_eq!(KEEPS_ONLY, remote_contacts::NO_KEEPING);
}
