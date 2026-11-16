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
fn after_a_bolt_the_line_offers_its_sender_always_priority_and_after_an_unmark_no_longer() {
    use egui_kittest::kittest::Queryable as _;
    let (mut h, calls) = hosted();
    reply(&calls, PRIORITY, 200, json!({"ids": [], "count": 0, "why": {}, "senders": [], "reading": null}));
    h.run_steps(4);
    super::sorting::act(h.state_mut(), super::sorting::Intent::Priority { account: ME.into(), email: "E1".into(), on: true });
    reply(&calls, PRIORITY_SET, 200, json!({"email_id": "E1", "priority": "yes", "sender": "grace@example.org", "sender_priority": "no"}));
    h.run_steps(4);
    // The offer, on the line above her bar: its sender as the window writes them.
    assert!(h.query_all_by_label_contains("Always priority for Grace Hopper").next().is_some());
    let button = h.state().hit.rect("noraHintButton", 0).expect("its button").center();
    crate::people_view::testing::click(&mut h, button, egui::PointerButton::Primary, 1);
    h.run_steps(3);
    assert_eq!(made(&calls, PRIORITY_SET).last().unwrap(), &json!({"account": ME, "email_id": "E1", "on": true, "sender": true}));
    reply(&calls, PRIORITY_SET, 200, json!({"email_id": "E1", "priority": "yes", "sender": "grace@example.org", "sender_priority": "yes"}));
    h.run_steps(4);
    assert_eq!(super::sorting::priority_of(h.state(), ME).unwrap().senders, ["grace@example.org"]);
    assert!(super::sorting::sender_offer(h.state()).is_none(), "the offer goes once taken");
    assert_eq!(made(&calls, PRIORITY).len(), 2, "read again at once");
    reply(
        &calls,
        PRIORITY,
        200,
        json!({"ids": ["E3", "E2", "E1"], "count": 3, "why": {"E3": "sender", "E2": "sender", "E1": "marked"}, "senders": ["grace@example.org"], "reading": null}),
    );
    h.run_steps(4);
    // Its reason under the pointer.
    assert_eq!(crate::mail_view::priority::why_said("sender"), "Priority sender");
    // Taking one of theirs out offers « no longer ».
    super::sorting::act(h.state_mut(), super::sorting::Intent::Priority { account: ME.into(), email: "E2".into(), on: false });
    reply(&calls, PRIORITY_SET, 200, json!({"email_id": "E2", "priority": "no", "sender": "grace@example.org", "sender_priority": "yes"}));
    h.run_steps(4);
    assert!(h.query_all_by_label_contains("No longer priority for Grace Hopper").next().is_some());
    // A server older than the offer says nothing of the sender: no offer.
    super::sorting::drop_sender_offer(h.state_mut());
    super::sorting::act(h.state_mut(), super::sorting::Intent::Priority { account: ME.into(), email: "E3".into(), on: true });
    reply(&calls, PRIORITY_SET, 200, json!({"email_id": "E3", "priority": "yes"}));
    h.run_steps(4);
    assert!(super::sorting::sender_offer(h.state()).is_none());
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
        // The offer after a bolt.
        super::sorting::act(h.state_mut(), super::sorting::Intent::Priority { account: ME.into(), email: "E1".into(), on: true });
        reply(&calls, PRIORITY_SET, 200, json!({"email_id": "E1", "priority": "yes", "sender": "grace@example.org", "sender_priority": "no"}));
        h.hover_at(egui::pos2(1.0, 1.0));
        h.run_steps(6);
        let image = h.render().expect("drawn");
        image.save(dir.join(format!("21-toujours-prioritaire-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
        // In a list 220 wide: « Ventes », then a long name (cut short, its whole under the pointer).
        let long = "Ventes et administration des ventes internationales";
        for (screen, list, name) in
            [("22-etroit-ventes", 220.0, "Ventes"), ("23-etroit-nom-long", 220.0, long), ("24-liste-de-320-nom-long", 320.0, long)]
        {
            let mut h = narrow(list, name, true);
            h.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
            let _ = h.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
            h.run_steps(4);
            if !screen.starts_with("22") {
                let button = h.state().hit.rect("noraHintButton", 0).unwrap();
                h.hover_at(button.center());
                h.run_steps(60);
            }
            let image = h.render().expect("drawn");
            image.save(dir.join(format!("{screen}-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
        }
    }
}

/// A hosting page's tab of 1000 × 625 whose list is `list` wide beside the reading pane, an offer for `name` above Nora's bar.
fn narrow(list: f32, name: &str, on: bool) -> Window {
    let (mut app, _calls, _) = page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let state = super::sorting::tests::state(false, "", true);
    app.nora.sorting.boxes.insert(ME.to_string(), Known::said(read_state(&state), None));
    app.ui.list_width = list;
    super::sorting::offer_sender(&mut app, ME, "E1", name, on);
    let mut h = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(vec2(1000.0, 625.0))
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

/// The bar and the line above it drawn alone over a list `width` wide: the list's room, the line's, its button's.
fn alone(width: f32, name: &str, on: bool) -> (egui::Rect, egui::Rect, egui::Rect) {
    let mut w = narrow(300.0, name, on);
    let mut app = std::mem::replace(w.state_mut(), MailApp::new(mailcraft_engine::Session::empty(), crate::Services::default()));
    app.hit = Default::default();
    // (Styled again in the new window: its fonts and its theme.)
    app.styled = false;
    app.applied_theme = None;
    let mut h =
        egui_kittest::Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(vec2(width + 40.0, 220.0)).build_ui_state(
            move |ui, app: &mut MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                let room = egui::Rect::from_min_size(ui.max_rect().min, vec2(width, 200.0));
                super::ask_bar::show(app, ui, room, room.max.x);
            },
            app,
        );
    h.run_steps(4);
    let hit = |kind: &str| h.state().hit.rect(kind, 0).unwrap_or_else(|| panic!("{width} {name}: no {kind}"));
    (hit("noraBackdrop"), hit("noraHint"), hit("noraHintButton"))
}

#[test]
fn in_a_list_220_wide_the_line_above_her_bar_never_leaves_it_whatever_the_sender_s_name() {
    // Over a list as narrow as the bar is drawn over (220), and a little wider.
    let long = "Ventes et administration des ventes internationales";
    for width in [220.0, 230.0, 260.0] {
        for (name, on) in [("Ventes", true), ("Ventes", false), (long, true), (long, false)] {
            let (list, line, button) = alone(width, name, on);
            assert!(line.min.x >= list.min.x && line.max.x <= list.max.x, "{width} {name} {on}: the line {line:?} inside {list:?}");
            assert!(button.min.x >= line.min.x && button.max.x <= line.max.x, "{width} {name} {on}: the button {button:?} inside {line:?}");
        }
    }
}

#[test]
fn in_the_narrowest_list_the_line_above_her_bar_never_leaves_it_whatever_the_sender_s_name() {
    use egui_kittest::kittest::Queryable as _;
    let long = "Ventes et administration des ventes internationales";
    for (name, on) in [("Ventes", true), ("Ventes", false), (long, true), (long, false), ("Grace Hopper", true)] {
        // (The narrowest list there is: `mail_view::LIST_MIN`.)
        let mut h = narrow(220.0, name, on);
        let list = h.state().hit.rect("noraBackdrop", 0).expect("the bar over the list");
        assert!(list.width() <= crate::mail_view::LIST_MIN, "the list is narrow: {list:?}");
        let line = h.state().hit.rect("noraHint", 0).expect("the offer");
        let button = h.state().hit.rect("noraHintButton", 0).expect("its button");
        let close = h.state().hit.rect("noraHintClose", 0).expect("its ✕");
        for (what, rect) in [("line", line), ("button", button), ("✕", close)] {
            assert!(rect.min.x >= list.min.x && rect.max.x <= list.max.x, "{name} {on}: the {what} {rect:?} inside the list {list:?}");
        }
        assert!(button.min.x >= line.min.x && button.max.x <= close.min.x, "{name} {on}: the button inside its line");
        // Its whole wording, under the pointer where it is said shorter; and as its name to a screen reader.
        let whole = format!("{} for {name}", if on { "Always priority" } else { "No longer priority" });
        assert!(h.query_all_by_label_contains(&whole).next().is_some(), "{whole}");
        if name == long {
            h.hover_at(button.center());
            h.run_steps(60);
            assert!(h.query_all_by_label_contains(&whole).count() >= 2, "{whole} under the pointer");
        }
    }
    // Her questions' line, the same.
    let mut h = narrow(220.0, "Ventes", true);
    super::sorting::drop_sender_offer(h.state_mut());
    let asks = (1..=3)
        .map(|n| json!({"name": format!("ASK-{n}"), "box": ME, "title": format!("Question {n}?"), "context": "", "choices": [{"code": "yes", "label": "Yes"}]}))
        .collect::<Vec<_>>();
    h.state_mut().nora.sorting.questions = super::sorting::Questions::said(super::sorting::read_asks(&json!({"asks": asks, "retained": 0})).0);
    h.run_steps(4);
    let list = h.state().hit.rect("noraBackdrop", 0).unwrap();
    let line = h.state().hit.rect("noraHint", 0).expect("her questions' line");
    let button = h.state().hit.rect("noraHintButton", 0).unwrap();
    assert!(line.min.x >= list.min.x && line.max.x <= list.max.x && button.min.x >= line.min.x, "{line:?} {list:?}");
}

#[test]
fn in_a_wide_tab_her_pane_comes_before_the_exchanges_which_come_back_when_it_closes() {
    let (mut app, _calls, _) = page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(vec2(1920.0, 1000.0))
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
    assert!(h.state().session.sel.focus().is_some(), "a message is read");
    assert!(h.state().exchanges.open, "the exchanges by themselves");
    h.state_mut().run("ui.noraAssistant", json!({"open": true})).unwrap();
    h.run_steps(4);
    assert!(super::assist::shown(h.state()) && !h.state().exchanges.open, "hers first");
    assert!(!h.state().ui.exchanges_declined, "not the person's choice");
    h.state_mut().run("ui.noraAssistant", json!({"open": false})).unwrap();
    h.run_steps(4);
    assert!(h.state().exchanges.open, "back by themselves");
}
