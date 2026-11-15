// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Prioritaire » (mail_view/priority.rs, the calls in sorting.rs).
//! « Prioritaire » through real frames: read for the Inbox in view when the list shows it, never again within a minute unless a ⚡
//! asked; what the server names shows at the head of the list, what is answered or no longer in the Inbox left out; a row's ⚡ puts
//! a message in or out (`priority_set`), and the server's refusal for mail Nora hasn't read is said. And (`--ignored`, with
//! `VARIANT_D_SHOTS=<folder>`) the section in French, light and dark.

use egui::vec2;
use serde_json::{Value, json};

use super::sorting::{Known, PRIORITY, PRIORITY_SET, read_state};
use super::testing::{Calls, ME, id_of, page};
use crate::MailApp;

type Window = egui_kittest::Harness<'static, MailApp>;

fn window(app: MailApp) -> Window {
    let mut h = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(vec2(1280.0, 800.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    h.run_steps(4);
    h
}

/// A hosting page's tab on its Inbox (`E1` to `E3`), Nora filing nothing there yet.
fn hosted() -> (Window, Calls) {
    let (mut app, calls, _) = page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let state = super::sorting::tests::state(false, "", true);
    app.nora.sorting.boxes.insert(ME.to_string(), Known::said(read_state(&state), None));
    (window(app), calls)
}

fn made(calls: &Calls, method: &str) -> Vec<Value> {
    calls.0.borrow().iter().filter(|(call, _)| call.method == method).map(|(call, _)| call.args.clone()).collect()
}

fn reply(calls: &Calls, method: &str, status: u16, message: Value) {
    let kept = calls.0.borrow();
    let (call, to) = kept.iter().rev().find(|(call, to)| call.method == method && to.is_some()).unwrap_or_else(|| panic!("no call of {method}"));
    let body = json!({"message": message}).to_string();
    to.as_ref().unwrap().send(super::NoraAnswer { id: call.id, status, body }).unwrap();
}

fn has(h: &Window, hit: &str, id: u64) -> bool {
    h.state().hit.rect(hit, id).is_some()
}

#[test]
fn what_waits_for_an_answer_is_read_once_and_shown_at_the_head_without_what_is_answered() {
    let (mut h, calls) = hosted();
    assert_eq!(made(&calls, PRIORITY), vec![json!({"account": ME, "days": 30, "limit": 200})]);
    reply(
        &calls,
        PRIORITY,
        200,
        json!({"ids": ["E3", "E2", "Egone"], "count": 3, "why": {"E3": "question", "E2": "quote_request"}, "reading": null}),
    );
    h.run_steps(4);
    let (e2, e3) = (id_of(h.state(), "E2"), id_of(h.state(), "E3"));
    assert!(has(&h, "priority", 0), "the section");
    assert!(has(&h, "priorityRow", e3) && has(&h, "priorityRow", e2));
    // Not again within a minute.
    h.run_steps(30);
    assert_eq!(made(&calls, PRIORITY).len(), 1);
    // One answered goes from it; with none left, the section goes.
    h.state_mut().session.mailbox.message_mut(e3).unwrap().answered = true;
    h.run_steps(3);
    assert!(!has(&h, "priorityRow", e3) && has(&h, "priorityRow", e2));
    h.state_mut().run("mail.archive", json!({"ids": [e2]})).unwrap();
    h.run_steps(3);
    assert!(!has(&h, "priority", 0));
}

#[test]
fn a_rows_bolt_puts_a_message_in_and_the_list_reads_it_again_and_a_refusal_is_said() {
    let (mut h, calls) = hosted();
    reply(&calls, PRIORITY, 200, json!({"ids": [], "count": 0, "why": {}, "reading": null}));
    h.run_steps(4);
    let e1 = id_of(h.state(), "E1");
    let at = h.state().hit.rect("message", e1).unwrap().center();
    h.hover_at(at);
    h.run_steps(3);
    let bolt = h.state().hit.rect("doneQuickPriority", e1).expect("the bolt").center();
    crate::people_view::testing::click(&mut h, bolt, egui::PointerButton::Primary, 1);
    h.run_steps(3);
    assert_eq!(made(&calls, PRIORITY_SET), vec![json!({"account": ME, "email_id": "E1", "on": true})]);
    reply(&calls, PRIORITY_SET, 200, json!({"email_id": "E1", "priority": "yes"}));
    h.run_steps(4);
    assert!(super::sorting::in_priority(h.state(), ME, "E1"));
    assert!(has(&h, "priorityRow", e1));
    assert_eq!(made(&calls, PRIORITY).len(), 2, "read again at once after the bolt");
    // A message Nora hasn't read: the server's refusal, said.
    super::sorting::act(h.state_mut(), super::sorting::Intent::Priority { account: ME.into(), email: "E2".into(), on: true });
    {
        // As `frappe.throw` answers.
        let kept = calls.0.borrow();
        let (call, to) = kept.iter().rev().find(|(call, to)| call.method == PRIORITY_SET && to.is_some()).unwrap();
        let said = json!([json!({"message": "Nora has not read this mail yet."}).to_string()]).to_string();
        let body = json!({"exc_type": "ValidationError", "_server_messages": said}).to_string();
        to.as_ref().unwrap().send(super::NoraAnswer { id: call.id, status: 417, body }).unwrap();
    }
    h.run_steps(3);
    let said = h.state().status_msg.as_ref().map(|(said, _)| said.clone()).unwrap_or_default();
    assert!(said.contains("Nora has not read this mail yet"), "{said}");
}

#[test]
fn a_bolt_answered_while_a_read_is_in_flight_is_read_again_after_it() {
    let (mut h, calls) = hosted();
    assert_eq!(made(&calls, PRIORITY).len(), 1, "a read in flight");
    super::sorting::act(h.state_mut(), super::sorting::Intent::Priority { account: ME.into(), email: "E1".into(), on: true });
    reply(&calls, PRIORITY_SET, 200, json!({"email_id": "E1", "priority": "yes"}));
    h.run_steps(4);
    assert_eq!(made(&calls, PRIORITY).len(), 1, "not twice at once");
    // The older read lands without it: read again after it, which has it.
    reply(&calls, PRIORITY, 200, json!({"ids": [], "count": 0, "why": {}, "reading": null}));
    h.run_steps(4);
    assert_eq!(made(&calls, PRIORITY).len(), 2);
    reply(&calls, PRIORITY, 200, json!({"ids": ["E1"], "count": 1, "why": {"E1": "marked"}, "reading": null}));
    h.run_steps(4);
    assert!(super::sorting::in_priority(h.state(), ME, "E1"));
}

#[test]
#[ignore = "draws to PNG files: VARIANT_D_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored priority_screens"]
fn priority_screens_in_french() {
    let Some(dir) = std::env::var_os("VARIANT_D_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        let (mut h, calls) = hosted();
        h.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
        let _ = h.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
        reply(&calls, PRIORITY, 200, json!({"ids": ["E3", "E2"], "count": 2, "why": {"E3": "question", "E2": "order"}, "reading": null}));
        h.run_steps(6);
        let e1 = id_of(h.state(), "E1");
        let at = h.state().hit.rect("message", e1).unwrap().center();
        h.hover_at(at);
        h.run_steps(4);
        let image = h.render().expect("drawn");
        image.save(dir.join(format!("17-prioritaire-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
    }
}
