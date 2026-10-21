// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Modules a hosting page draws itself: what the window takes of them, where their buttons are, and what the page is
//! told of the place it draws them in.

use std::cell::RefCell;
use std::rc::Rc;

use egui_kittest::Harness;
use mailcraft_engine::{Module, Session};
use mailcraft_model::Mailbox;
use serde_json::{Value, json};

use super::*;
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// What the page was told, message by message.
type Told = Rc<RefCell<Vec<Value>>>;

/// A window whose hosting page handed `handed` modules, and what the page is told.
fn window(handed: Value) -> (MailApp, Told) {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    let told: Told = Rc::default();
    let kept = told.clone();
    let list = handed.as_array().cloned().unwrap_or_default();
    app.services.modules = Some(Modules::new(&list, Box::new(move |message| kept.borrow_mut().push(message.clone()))));
    (app, told)
}

fn synk() -> Value {
    json!([{"id": "synk", "title": "Synk", "icon": "chatSynk"}])
}

fn drawn(app: MailApp) -> Harness<'static, MailApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 860.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(4);
    harness
}

fn click(h: &mut Harness<'static, MailApp>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
}

#[test]
fn the_window_takes_only_modules_it_can_draw_each_once() {
    let handed = json!([
        {"id": "synk", "title": "Synk", "icon": "no-such-icon"},
        {"id": "synk", "title": "Synk again"},
        {"id": "Upper", "title": "Refused"},
        {"id": "x".repeat(MAX_ID + 1), "title": "Too long an id"},
        {"id": "untitled"},
        {"id": "files", "title": "  Files  ", "icon": "attach"},
        {"id": "a", "title": "A"},
        {"id": "b", "title": "B"},
        {"id": "c", "title": "One too many"},
    ]);
    let modules = Modules::new(handed.as_array().unwrap(), Box::new(|_| {}));
    let taken: Vec<(&str, &str, &str)> = modules.list.iter().map(|m| (m.id.as_str(), m.title.as_str(), m.icon.as_str())).collect();
    assert_eq!(taken, vec![("synk", "Synk", DEFAULT_ICON), ("files", "Files", "attach"), ("a", "A", DEFAULT_ICON), ("b", "B", DEFAULT_ICON)]);
}

#[test]
fn its_button_is_after_calendar_and_shows_the_page_s_module_in_the_window_s_place() {
    let (app, told) = window(synk());
    let mut h = drawn(app);
    let (calendar, people, button) = {
        let app = h.state();
        (
            app.hit.rect("module", Module::Calendar as u64).unwrap(),
            app.hit.rect("module", Module::People as u64).unwrap(),
            app.hit.rect("hostModule", 0).unwrap(),
        )
    };
    assert!(calendar.max.y <= button.min.y && button.max.y <= people.min.y, "{calendar:?} {button:?} {people:?}");
    // Nothing told while the window's own module is in view
    assert!(told.borrow().is_empty());
    click(&mut h, button.center());
    assert!(shown(h.state()));
    let last = told.borrow().last().cloned().unwrap();
    assert_eq!(last["type"], MODULE_TYPE);
    assert_eq!(last["id"], "synk");
    // The window's look goes with it: the palette in use and the font
    let tokens = crate::theme::Tokens::get(&h.ctx);
    assert_eq!(last["look"], look(&tokens));
    assert_eq!(last["look"]["dark"], tokens.dark);
    assert_eq!(last["look"]["font"], LOOK_FONT);
    // The place beside the module bar, under the title bar, above the status bar, in CSS pixels
    let rect = &last["rect"];
    assert!(rect["x"].as_f64().unwrap() >= button.max.x as f64, "{rect}");
    assert!(rect["y"].as_f64().unwrap() > 0.0 && rect["width"].as_f64().unwrap() > 800.0 && rect["height"].as_f64().unwrap() > 500.0, "{rect}");
    // No module of the window's is lit; told once, not at every frame
    let sent = told.borrow().len();
    h.run_steps(5);
    assert_eq!(told.borrow().len(), sent);
    // Mail's button brings the window's own back, and the page is told it has nothing to draw
    let mail = h.state().hit.rect("module", Module::Mail as u64).unwrap();
    click(&mut h, mail.center());
    assert!(!shown(h.state()));
    assert_eq!(told.borrow().last().cloned().unwrap(), json!({"type": MODULE_TYPE, "id": null, "rect": null}));
}

#[test]
fn nothing_is_drawn_by_the_page_while_something_of_the_window_s_stands_over_its_place() {
    let (mut app, told) = window(synk());
    assert!(show(&mut app, "synk"));
    let mut h = drawn(app);
    assert!(told.borrow().last().unwrap()["rect"].is_object());
    // A dialog of the window's: the page is told to draw nothing, then where again once it is closed
    h.state_mut().run("ui.dialog", json!({"name": "about"})).unwrap();
    h.run_steps(4);
    assert_eq!(told.borrow().last().unwrap()["rect"], Value::Null);
    assert_eq!(told.borrow().last().unwrap()["id"], "synk");
    h.state_mut().dialog = None;
    h.run_steps(4);
    assert!(told.borrow().last().unwrap()["rect"].is_object());
    // Backstage takes the whole window
    h.state_mut().ui.backstage = true;
    h.run_steps(3);
    assert_eq!(told.borrow().last().unwrap()["rect"], Value::Null);
}

#[test]
fn the_page_gives_its_module_a_badge_and_may_bring_it_into_view() {
    let (mut app, _) = window(synk());
    assert!(hear(&mut app, &json!({"type": BADGE_TYPE, "id": "synk", "badge": 3}).to_string()));
    assert_eq!(app.services.modules.as_ref().unwrap().badge("synk"), Some("3"));
    hear(&mut app, &json!({"type": BADGE_TYPE, "id": "synk", "badge": 240}).to_string());
    assert_eq!(app.services.modules.as_ref().unwrap().badge("synk"), Some("99+"));
    for none in [json!(0), json!(null), json!(""), json!(-2)] {
        hear(&mut app, &json!({"type": BADGE_TYPE, "id": "synk", "badge": none}).to_string());
        assert_eq!(app.services.modules.as_ref().unwrap().badge("synk"), None, "{none}");
    }
    // A module it did not hand over, and what is not the modules' to hear
    assert!(hear(&mut app, &json!({"type": BADGE_TYPE, "id": "other", "badge": 1}).to_string()));
    assert_eq!(app.services.modules.as_ref().unwrap().badge("other"), None);
    assert!(!hear(&mut app, &json!({"type": "mailcraft:command", "id": 1, "name": "view.mail"}).to_string()));
    assert!(!hear(&mut app, "not json"));
    // Its module into view, on the page's word (a conversation it opens there)
    assert!(hear(&mut app, &json!({"type": SHOW_TYPE, "id": "synk"}).to_string()));
    assert!(shown(&app));
    assert!(hear(&mut app, &json!({"type": SHOW_TYPE, "id": "other"}).to_string()));
    // Any of the window's own views brings it back
    app.run("view.calendar", json!({})).unwrap();
    assert!(!shown(&app));
}

#[test]
fn the_look_is_the_palette_in_use_as_css_writes_it() {
    let dark = look(&crate::theme::Tokens::dark());
    assert_eq!(dark["dark"], true);
    assert_eq!(dark["colors"]["canvas"], "#0f161f");
    assert_eq!(dark["colors"]["accent"], "#58b8fe");
    let light = look(&crate::theme::Tokens::light());
    assert_eq!(light["dark"], false);
    assert_eq!(light["colors"]["accent"], "#0f6cbd");
    assert_eq!(light["colors"]["surface"], "#ffffff");
    let colors = light["colors"].as_object().unwrap();
    assert_eq!(colors.len(), 22);
    // Opaque colours in six digits, the others with their alpha
    assert!(colors.values().all(|c| c.as_str().is_some_and(|c| c.len() == 7 && c.starts_with('#'))), "{colors:?}");
    assert_eq!(hex(egui::Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0x80)).len(), 9);
}

#[test]
fn a_window_without_modules_of_a_page_s_draws_its_own_only() {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    assert!(!show(&mut app, "synk"));
    let h = drawn(app);
    assert!(h.state().hit.rect("hostModule", 0).is_none());
    assert!(h.state().hit.rect("module", Module::Calendar as u64).is_some());
}
