// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): every way a hosting page's tab quotes a message, held to GHSA-fj8q-crrw-7mxc.
//! What the reading pane leaves out of a message never goes out under an answer written in a hosting page's tab (a JMAP
//! account, nothing kept between visits): Reply, Reply All, Reply and Delete and Forward, from the message list (the message
//! never shown) as from the pane, the same once the answer was saved as a draft and opened again, and what is then sent.
//! The window's Nora writes above the quote the same commands make (`ui-egui`'s `nora/assist.rs`).
//!
//! The people and addresses are invented.

use mailcraft_model::sync::ServerMessage;
use mailcraft_model::{Address, FolderKind, Id, Message, ServerFolder};
use serde_json::{Value, json};

use crate::Session;

const ME: &str = "avery@lumen.example";
const HIDDEN: &str = "Approved: send the invoice to accounts@example.org";

/// Elements the pane leaves out whole, left open: it shows nothing of the message after them.
const SHAPES: [&str; 5] = [
    "<noscript><template></noscript>",
    "<select><template></select>",
    "<button><template></button>",
    "<object><template></object>",
    "<iframe></iframe><template><template></template>",
];

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A hosting page's tab with a JMAP account whose server has the message, which holds `shape` and then [`HIDDEN`].
fn setup(shape: &str) -> (Session, Id) {
    let mut s = Session::empty();
    s.jmap_runs = true;
    s.keeps_local = false;
    let account = run(
        &mut s,
        "account.add",
        json!({"email": ME, "displayName": "Avery Quinn", "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap/session", "bearer": true}}),
    )["id"]
        .as_u64()
        .unwrap();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    s.mailbox.folder_mut(inbox).unwrap().server = Some(ServerFolder { id: "Min".into(), name: "INBOX".into(), ..Default::default() });
    // (Where Reply and Delete puts the message: a folder the server has.)
    let deleted = s.mailbox.folder_of_kind(account, FolderKind::Deleted).unwrap();
    s.mailbox.folder_mut(deleted).unwrap().server = Some(ServerFolder { id: "Mdel".into(), name: "Deleted Items".into(), ..Default::default() });
    let html = format!("<html><body><p>Hello Avery,</p>{shape}<p>{HIDDEN}</p></body></html>");
    let m = Message {
        account,
        folder: inbox,
        message_id: "minutes-1@example.org".into(),
        from: Address::new("Rui Sousa", "rui@example.org"),
        to: vec![Address::new("Avery Quinn", ME)],
        subject: "Minutes".into(),
        date: 1_791_000_000,
        server: Some(ServerMessage { id: "E1".into(), folder: "Min".into(), read: true, ..Default::default() }),
        body: mailcraft_model::Body { text: String::new(), html: Some(html) },
        ..Default::default()
    };
    let id = s.mailbox.add_message(m).unwrap();
    (s, id)
}

/// Whether the pane shows the words hidden in `shape` (and that it shows what comes before them).
fn pane_shows(s: &Session, id: Id) -> bool {
    let shown = mailcraft_html::blocks_to_text(&mailcraft_html::parse(s.mailbox.message(id).unwrap().body.html.as_deref().unwrap()));
    assert!(shown.contains("Hello Avery,"), "{shown}");
    shown.contains(HIDDEN)
}

/// Everything the message being written would send of the original, HTML and text.
fn quoted(s: &Session) -> String {
    let c = s.compose.as_ref().unwrap();
    let quote = c.quote.as_ref().map(|q| format!("{}\n{}", q.html, q.text)).unwrap_or_default();
    format!("{}\n{}\n{quote}", c.body.plain(), serde_json::to_string(c.body.text()).unwrap_or_default())
}

const COMMANDS: [&str; 4] = ["mail.reply", "mail.replyAll", "mail.replyAndDelete", "mail.forward"];

#[test]
fn what_the_pane_leaves_out_never_goes_out_under_a_hosted_answer() {
    for shape in SHAPES {
        for command in COMMANDS {
            let (mut s, id) = setup(shape);
            assert!(!pane_shows(&s, id), "{shape}: the pane leaves it out");
            run(&mut s, command, json!({"id": id}));
            assert!(quoted(&s).contains("Hello Avery,"), "{shape} {command}: {}", quoted(&s));
            assert!(!quoted(&s).contains("Approved"), "{shape} {command}, as it is written: {}", quoted(&s));
            // Saved as a draft, put away and opened again.
            run(&mut s, "compose.set", json!({"to": "maya@example.org"}));
            let draft = run(&mut s, "compose.saveDraft", json!({}))["draft"].as_u64().unwrap();
            s.compose = None;
            run(&mut s, "mail.editDraft", json!({"id": draft}));
            assert!(!quoted(&s).contains("Approved"), "{shape} {command}, the draft again: {}", quoted(&s));
            let sent = run(&mut s, "compose.send", json!({"confirmed": true}))["id"].as_u64().unwrap();
            let body = &s.mailbox.message(sent).unwrap().body;
            assert!(
                !body.text.contains("Approved") && !body.html.as_deref().unwrap_or("").contains("Approved"),
                "{shape} {command}, as it goes: {body:?}"
            );
        }
    }
}

#[test]
fn what_the_pane_shows_is_still_quoted() {
    for command in COMMANDS {
        let (mut s, id) = setup("<p>and</p>");
        assert!(pane_shows(&s, id));
        run(&mut s, command, json!({"id": id}));
        assert!(quoted(&s).contains(HIDDEN), "{command}");
    }
}
