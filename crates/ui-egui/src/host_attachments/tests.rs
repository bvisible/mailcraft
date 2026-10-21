// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A hosting page that files a message's attachment: where the window offers it, and the one thing the page is handed.
//! Invented people at `example.org`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::jmap_mail::apply_jmap_mail;
use mailcraft_model::sync::{FolderListing, GraphFolder, GraphMessage, JmapMailResult, MessagePage, ServerMessage};
use mailcraft_model::{Address, Attachment, FolderKind, Id, Mailbox};
use serde_json::{Value, json};

use super::*;
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const ADA: &str = "ada@example.org";
const BOB: &str = "bob@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session";

fn mail(id: &str, folder: &str) -> GraphMessage {
    GraphMessage {
        server: ServerMessage { id: id.into(), folder: folder.into(), read: true, ..Default::default() },
        message_id: format!("<{id}@mail.example.org>"),
        subject: "Invoice for the saw".into(),
        from: Address::new("Grace Hopper", "grace@example.org"),
        sent: NOW - 3_600,
        received: NOW - 3_600,
        ..Default::default()
    }
}

/// A JMAP mailbox added as a hosting page adds it, with an Inbox holding `ids`.
fn mailbox(app: &mut MailApp, email: &str, ids: &[&str]) -> Id {
    let added = app.execute("account.add", json!({"email": email, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
    let account = added["id"].as_u64().unwrap();
    let folders = vec![GraphFolder { id: "Min".into(), name: "INBOX".into(), kind: FolderKind::Inbox, ..Default::default() }];
    let listing = JmapMailResult { account, email: email.into(), listing: Some(FolderListing { folders, complete: true }), ..Default::default() };
    apply_jmap_mail(&mut app.session, listing).unwrap();
    let page = MessagePage { folder: "Min".into(), messages: ids.iter().map(|id| mail(id, "Min")).collect(), afresh: true, ..Default::default() };
    apply_jmap_mail(&mut app.session, JmapMailResult { account, email: email.into(), pages: vec![page], whole: true, ..Default::default() }).unwrap();
    account
}

/// The files of every message: the picture of its text, an invoice, a letter, a plan and a drawing.
fn files() -> Vec<Attachment> {
    let mut logo = Attachment::new("image.png", "image/png", vec![1; 24]);
    logo.content_id = "logo@example.org".into();
    logo.inline = true;
    vec![
        logo,
        Attachment::new("Invoice 118.pdf", "application/pdf", vec![2; 120]),
        Attachment::new("Letter.DOCX", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", vec![3; 40]),
        Attachment::new("plan", "image/jpeg", vec![4; 30]),
        Attachment::new("drawing.svg", "image/svg+xml", vec![5; 10]),
    ]
}

/// What the page was handed, message by message.
type Handed = Rc<RefCell<Vec<Value>>>;

/// A hosting page's window: Ada's mailbox, handed over, and Bob's, which the person added by hand, each message with
/// [`files`]. With `actions`, what the page offers to do with an attachment, and what it is handed is kept.
fn page(actions: Option<Vec<Action>>) -> (MailApp, Handed) {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    app.services.jmap_host_credentials = true;
    app.session.keeps_local = false;
    let ada = mailbox(&mut app, ADA, &["A1", "A2"]);
    mailbox(&mut app, BOB, &["B1"]);
    let ids: Vec<Id> = app.session.mailbox.messages.keys().copied().collect();
    for id in ids {
        if let Some(m) = app.session.mailbox.messages.get_mut(&id) {
            Arc::make_mut(m).attachments = files();
        }
    }
    let handed: Handed = Rc::default();
    if let Some(actions) = actions {
        let kept = handed.clone();
        app.services.attachments = Some(Attachments {
            mailboxes: vec!["Ada@Example.org".into()],
            actions,
            post: Box::new(move |message| kept.borrow_mut().push(message.clone())),
        });
    }
    app.execute("folder.select", json!({"kind": "inbox", "account": ada})).unwrap();
    (app, handed)
}

const ALL: [Action; 3] = [Action::Drive, Action::Scan, Action::Edit];

/// The window's message whose id on the server is `server`.
fn message_of(app: &MailApp, server: &str) -> Id {
    app.session.mailbox.messages.values().find(|m| m.server.as_ref().is_some_and(|s| s.id == server)).map(|m| m.id).unwrap()
}

/// The entries of the menu of the `index`-th attachment of the message whose id on the server is `server`.
fn menu_of(app: &MailApp, server: &str, index: usize) -> Vec<Entry> {
    let m = app.session.mailbox.message(message_of(app, server)).unwrap().clone();
    let a = m.attachments.get(index).unwrap().clone();
    entries(app, &m, &a)
}

#[test]
fn the_page_is_handed_the_message_and_what_identifies_the_file_never_its_bytes() {
    let (mut app, handed) = page(Some(ALL.to_vec()));
    let id = message_of(&app, "A2");
    assert!(app.enabled(TO_DRIVE));
    assert_eq!(app.run(TO_DRIVE, json!({"id": id, "index": 1})).unwrap(), json!({"handed": "drive"}));
    assert_eq!(app.run(TO_SCANS, json!({"id": id, "index": 1})).unwrap(), json!({"handed": "scan"}));
    assert_eq!(app.run(IN_EDITOR, json!({"id": id, "index": 2})).unwrap(), json!({"handed": "edit"}));
    // Its rank among the message's five files, and how many the window sees
    let invoice = json!({"index": 1, "count": 5, "name": "Invoice 118.pdf", "size": 120, "mime": "application/pdf", "cid": ""});
    assert_eq!(
        handed.borrow().as_slice(),
        [
            // The address as the page handed it over, the id as its server gave it
            json!({"type": TYPE, "action": "drive", "account": "Ada@Example.org", "email": "A2", "attachment": invoice}),
            json!({"type": TYPE, "action": "scan", "account": "Ada@Example.org", "email": "A2", "attachment": invoice}),
            json!({"type": TYPE, "action": "edit", "account": "Ada@Example.org", "email": "A2", "attachment": {
                "index": 2,
                "count": 5,
                "name": "Letter.DOCX",
                "size": 40,
                "mime": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "cid": "",
            }}),
        ]
    );
    // The picture of the text: its Content-ID, by which the page's server knows it first
    app.run(TO_DRIVE, json!({"id": id, "index": 0})).unwrap();
    assert_eq!(handed.borrow().last().unwrap()["attachment"]["cid"], "logo@example.org");
    // Without `id`: the message in the reading pane
    app.execute("mail.select", json!({"id": message_of(&app, "A1")})).unwrap();
    app.run(TO_DRIVE, json!({"index": 1})).unwrap();
    assert_eq!(handed.borrow().last().unwrap()["email"], "A1");
}

#[test]
fn a_long_name_type_or_content_id_is_cut() {
    let mut a = Attachment::new("a".repeat(5_000) + ".pdf", "x".repeat(400), vec![0; 3]);
    a.content_id = "c".repeat(2_000);
    let told = message(Action::Drive, ADA, "A1", 4, 6, &a);
    assert_eq!(told["attachment"]["name"].as_str().unwrap().chars().count(), MAX_NAME);
    assert_eq!(told["attachment"]["mime"].as_str().unwrap().chars().count(), MAX_MIME);
    assert_eq!(told["attachment"]["cid"].as_str().unwrap().chars().count(), MAX_CID);
    assert_eq!(told["attachment"]["size"], 3);
    assert_eq!((told["attachment"]["index"].as_u64(), told["attachment"]["count"].as_u64()), (Some(4), Some(6)));
}

#[test]
fn the_menu_offers_what_the_page_offers_for_what_the_file_is() {
    let (app, _) = page(Some(ALL.to_vec()));
    // The picture of the text: a picture, no editor's
    assert_eq!(menu_of(&app, "A1", 0), [Entry::Drive, Entry::Scan]);
    assert_eq!(menu_of(&app, "A1", 1), [Entry::Drive, Entry::Scan, Entry::Edit(Editor::Pdf)]);
    assert_eq!(menu_of(&app, "A1", 2), [Entry::Drive, Entry::Edit(Editor::Document)]);
    // A picture named without an extension goes by its type; a drawing is no picture to scan
    assert_eq!(menu_of(&app, "A1", 3), [Entry::Drive, Entry::Scan]);
    assert_eq!(menu_of(&app, "A1", 4), [Entry::Drive]);
    // A page that offers the Drive alone
    let (app, _) = page(Some(vec![Action::Drive]));
    assert_eq!(menu_of(&app, "A1", 1), [Entry::Drive]);
    assert!(!app.enabled(TO_SCANS) && !app.enabled(IN_EDITOR) && app.enabled(TO_DRIVE));
}

#[test]
fn only_a_message_of_a_mailbox_the_page_handed_over_that_its_server_has() {
    let (mut app, handed) = page(Some(ALL.to_vec()));
    // A mailbox the person added by hand: no entry, and refused
    assert_eq!(menu_of(&app, "B1", 1), []);
    let bob = message_of(&app, "B1");
    assert_eq!(
        app.run(TO_DRIVE, json!({"id": bob, "index": 1})).unwrap_err(),
        "Only an attachment of a message of a mailbox the page handed over can be handed to it."
    );
    // A message its server no longer has
    let gone = message_of(&app, "A1");
    if let Some(server) = app.session.mailbox.messages.get_mut(&gone).and_then(|m| Arc::make_mut(m).server.as_mut()) {
        server.gone = true;
    }
    assert_eq!(menu_of(&app, "A1", 1), []);
    assert_eq!(app.run(TO_DRIVE, json!({"id": gone, "index": 1})).unwrap_err(), "This message isn't on its server yet.");
    // No such attachment, no such message, nothing said of which
    let id = message_of(&app, "A2");
    assert_eq!(app.run(TO_DRIVE, json!({"id": id, "index": 9})).unwrap_err(), "This message has no such attachment.");
    assert!(app.run(TO_DRIVE, json!({"id": 999_999, "index": 1})).is_err());
    assert!(app.run(TO_DRIVE, json!({"id": id})).is_err());
    assert!(app.run(TO_DRIVE, json!({"id": id, "index": "1"})).is_err());
    // A file of another kind than the command's
    assert!(app.run(TO_SCANS, json!({"id": id, "index": 2})).is_err());
    assert!(app.run(IN_EDITOR, json!({"id": id, "index": 3})).is_err());
    assert!(handed.borrow().is_empty(), "nothing went to the page");
}

#[test]
fn without_the_page_s_word_nothing_is_offered() {
    let (mut app, handed) = page(None);
    assert_eq!(menu_of(&app, "A1", 1), []);
    let id = message_of(&app, "A1");
    for command in [TO_DRIVE, TO_SCANS, IN_EDITOR] {
        assert!(!app.enabled(command), "{command}");
        assert!(app.run(command, json!({"id": id, "index": 1})).is_err(), "{command}");
    }
    // A page that offers only what it offers
    let (mut app, _) = page(Some(vec![Action::Scan]));
    assert_eq!(app.run(TO_DRIVE, json!({"id": message_of(&app, "A1"), "index": 1})).unwrap_err(), "The page doesn't offer this for an attachment.");
    assert!(handed.borrow().is_empty());
}

#[test]
fn what_is_scanned_and_what_opens_in_an_editor_by_the_name_s_extension_first() {
    assert!(scannable("Invoice.PDF", "application/octet-stream"));
    assert!(scannable("photo.heic", ""));
    assert!(scannable("scan", "Image/JPEG"));
    assert!(!scannable("drawing.svg", "image/svg+xml"));
    assert!(!scannable("tool.exe", "application/pdf"), "the extension says, whatever the type");
    // A name that reads otherwise than it is (a right-to-left override): its real extension
    assert!(!scannable("Invoice\u{202e}fdp.exe", "application/pdf"));
    assert_eq!(editor("Invoice\u{202e}xcod.exe"), None);
    assert_eq!(editor("Budget 2026.XLSX"), Some(Editor::Spreadsheet));
    assert_eq!(editor("Pitch.pptx"), Some(Editor::Presentation));
    assert_eq!(editor("../../Letter.docx"), Some(Editor::Document));
    assert_eq!(editor("Invoice.pdf"), Some(Editor::Pdf));
    assert_eq!(editor("Letter.doc"), None);
    assert_eq!(editor("pdf"), None);
    assert_eq!(editor(".pdf"), None);
    assert_eq!(Editor::Presentation.product(), "NeoPrés");
    // The handover's words, and no other
    assert_eq!(["drive", "scan", "edit"].map(Action::from_code), [Some(Action::Drive), Some(Action::Scan), Some(Action::Edit)]);
    assert_eq!(Action::from_code("Drive"), None);
    let words = |list: &[&str]| list.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    assert_eq!(actions(&words(&["edit", "drive", "edit", "delete", "Scan"])), [Action::Drive, Action::Edit]);
    assert_eq!(actions(&words(&[])), []);
    assert!(is_command(TO_DRIVE) && !is_command("ui.shareMessage"));
}

#[test]
fn the_menu_reads_in_french_and_the_editors_keep_their_names() {
    use mailcraft_i18n::Lang;
    assert_eq!(Lang::French.t("Save to Drive…"), "Enregistrer dans le Drive…");
    assert_eq!(Lang::French.t("Send to Scanned Documents"), "Envoyer aux scans de documents");
    for product in ["NeoDoc", "NeoGrille", "NeoPrés", "NeoPDF"] {
        assert_eq!(Lang::French.t(&format!("Open in {product}")), format!("Ouvrir dans {product}"));
    }
}

#[test]
fn an_agent_never_hands_an_attachment_to_the_page() {
    use mailcraft_engine::agent::{Access, access};
    for command in [TO_DRIVE, TO_SCANS, IN_EDITOR] {
        assert_eq!(access(command), Some(Access::Never), "{command}");
    }
}
