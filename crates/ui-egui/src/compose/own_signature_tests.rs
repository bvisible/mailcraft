// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the owner's own signature shows its pictures on the web.
//! The owner's own HTML signature, as a message being written and File › Signatures show it: its pictures on the web are
//! asked of the host and drawn, with no say-so, as the owner's recipients will see them; a received message's are still
//! asked for only on the owner's word.

use std::sync::{Arc, Mutex, mpsc};

use serde_json::json;

use super::tests::{Window, app, window};
use crate::MailApp;
use crate::reading::images::{Pictures, Remote};

/// A company's card as the hosting page writes it: its logo and an icon by their addresses on the web.
const LOGO: &str = "https://instance.example/files/signature-logo.png";
const ICON: &str = "https://instance.example/files/signature-phone.png";

fn card() -> String {
    format!(
        r#"<table cellpadding="0" cellspacing="0" width="480"><tr><td><b>Avery Quinn</b><br>Founder</td>
<td><img src="{LOGO}" width="120" height="40" alt="Lumen Works"></td>
<td><img src="{ICON}" width="14" height="14" alt="Phone"> <a href="tel:+41215192568">+41 21 519 25 68</a></td></tr></table>"#
    )
}

fn png() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(8, 8, image::Rgba([30, 30, 30, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A host that answers every address with a picture at once, and notes it.
fn answering(app: &mut MailApp) -> Arc<Mutex<Vec<String>>> {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let noted = asked.clone();
    app.services.pictures = Some(Box::new(move |url: String| {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(Ok(png()));
        noted.lock().unwrap().push(url);
        rx
    }));
    asked
}

fn ready(h: &Window, url: &str) -> bool {
    matches!(Pictures::of(&h.ctx).lock().unwrap().remote_peek(url), Some(Remote::Ready(_)))
}

/// Writing a message, the owner sees their company's card with its logo and its icons, not their names in boxes.
#[test]
fn a_message_being_written_shows_the_pictures_of_its_html_signature() {
    let mut a = app();
    let asked = answering(&mut a);
    assert!(!a.ui.download_pictures, "mail's pictures are not downloaded for everybody");
    a.execute("signature.html", json!({"name": "Standard", "html": card()})).unwrap();
    a.execute("mail.new", json!({"to": "maya@lumen.example", "body": "Hello"})).unwrap();
    a.execute("compose.formatHtml", json!({})).unwrap();
    let mut h = window(a);
    h.run_steps(6);
    assert!(h.state().hit.rect("composeSignature", 0).is_some());
    let mut wanted = asked.lock().unwrap().clone();
    wanted.sort();
    assert_eq!(wanted, [LOGO, ICON], "each picture of the signature once");
    assert!(ready(&h, LOGO) && ready(&h, ICON));
}

/// Without a way to fetch anything (a build with no host for pictures), the card keeps its stand-ins and nothing breaks.
#[test]
fn without_a_host_the_signature_keeps_its_stand_ins() {
    let mut a = app();
    a.execute("signature.html", json!({"name": "Standard", "html": card()})).unwrap();
    a.execute("mail.new", json!({"to": "maya@lumen.example", "body": "Hello"})).unwrap();
    a.execute("compose.formatHtml", json!({})).unwrap();
    let mut h = window(a);
    h.run_steps(6);
    assert!(h.state().hit.rect("composeSignature", 0).is_some());
    assert!(!ready(&h, LOGO) && !ready(&h, ICON));
}

/// The signature's say-so is its own: a received message calling a picture on the web is not fetched for it, though
/// the owner's signature fetches its own while it is on screen.
#[test]
fn a_received_message_still_waits_for_the_owner_s_word() {
    let mut a = app();
    let asked = answering(&mut a);
    a.execute("signature.html", json!({"name": "Standard", "html": card()})).unwrap();
    let inbox = a.session.mailbox.inbox().unwrap();
    let tracker = "https://tracker.example/open.png";
    let received = mailcraft_model::Message {
        folder: inbox,
        from: mailcraft_model::Address::new("A Sender", "sender@elsewhere.example"),
        subject: "A newsletter".into(),
        body: mailcraft_model::Body {
            text: String::new(),
            html: Some(format!(r#"<p>Hello</p><img src="{tracker}" width="200" height="80" alt="Banner">"#)),
        },
        ..Default::default()
    };
    let id = a.session.mailbox.add_message(received).unwrap();
    assert_ne!(id, crate::reading::remote::OWN_SIGNATURE);
    a.execute("mail.select", json!({"id": id})).unwrap();
    let mut h = window(a);
    h.run_steps(6);
    assert!(!asked.lock().unwrap().iter().any(|url| url == tracker), "{:?}", asked.lock().unwrap());
    // Then a message is written: its signature's pictures come, and still not the newsletter's.
    h.state_mut().execute("mail.new", json!({"to": "maya@lumen.example", "body": "Hello"})).unwrap();
    h.state_mut().execute("compose.formatHtml", json!({})).unwrap();
    h.run_steps(6);
    let seen = asked.lock().unwrap().clone();
    assert!(seen.iter().any(|url| url == LOGO), "{seen:?}");
    assert!(!seen.iter().any(|url| url == tracker), "{seen:?}");
}
