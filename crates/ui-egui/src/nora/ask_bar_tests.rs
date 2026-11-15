// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the tests of Nora's bar over the list (ask_bar.rs).
//! Nora's bar over the list of a hosting page's tab, through real frames: what is typed there goes to her pane with Enter; the
//! line above it says what « Trier avec Nora » proposes only when it proposes something, and its ✕ hides it for the mailbox until
//! the next day; what she read of the mailbox is asked for it while the bar is drawn, once in half an hour. And (`--ignored`, with
//! `VARIANT_D_SHOTS=<folder>`) the bar and the line in French, light and dark, to PNG files.

use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::kittest::Queryable as _;
use serde_json::{Value, json};

use super::super::sorting::{Known, SURVEY, read_journal, read_state, read_survey};
use super::super::testing::{Calls, ME, page};
use crate::MailApp;

type Window = egui_kittest::Harness<'static, MailApp>;

mod answers {
    pub(crate) use super::super::super::sorting::tests::{journal, state, survey};
}

fn window(app: MailApp) -> Window {
    let mut harness = egui_kittest::Harness::builder()
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
    harness.run_steps(4);
    harness
}

/// A hosting page's tab on its Inbox, the server having said `state` of its mailbox and, with `survey`, what Nora read of it; and
/// the calls asked of the host.
fn hosted(state: Value, survey: bool) -> (MailApp, Calls) {
    let (mut app, calls, _) = page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let said = read_state(&state);
    let known = if survey { Known::surveyed(said, read_survey(&answers::survey())) } else { Known::said(said, None) };
    app.nora.sorting.boxes.insert(ME.to_string(), known);
    (app, calls)
}

fn shown(h: &Window, hit: &str) -> bool {
    h.state().hit.rect(hit, 0).is_some()
}

fn contains(h: &Window, part: &str) -> bool {
    h.query_all_by_label_contains(part).next().is_some()
}

fn click(h: &mut Window, hit: &str) {
    let at = h.state().hit.rect(hit, 0).unwrap_or_else(|| panic!("no {hit}")).center();
    crate::people_view::testing::click(h, at, egui::PointerButton::Primary, 1);
    h.run_steps(3);
}

#[test]
fn what_is_typed_in_the_bar_goes_to_her_pane_with_enter() {
    let (app, _) = hosted(answers::state(false, "validated", true), false);
    let mut h = window(app);
    assert!(shown(&h, "noraAsk") && contains(&h, "Ask Nora…"));
    click(&mut h, "noraAskText");
    h.input_mut().events.push(Event::Text("What is waiting for my answer?".into()));
    h.run_steps(2);
    assert!(!h.state().nora.assist.open, "nothing is asked before Enter");
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(4);
    assert!(h.state().nora.assist.open, "her pane opens");
    assert!(contains(&h, "What is waiting for my answer?"), "with the question in it");
    // The bar gives way to her pane, and is empty when it comes back.
    assert!(!shown(&h, "noraAsk"));
    h.state_mut().run("ui.noraAssistant", json!({"open": false})).unwrap();
    h.run_steps(4);
    assert!(shown(&h, "noraAsk") && !contains(&h, "What is waiting for my answer?"));
}

#[test]
fn what_she_proposes_shows_only_when_she_proposes_something_and_its_cross_hides_it_until_the_next_day() {
    // Nothing read of it: the bar alone.
    let (app, _) = hosted(answers::state(false, "", true), false);
    let h = window(app);
    assert!(shown(&h, "noraAsk") && !shown(&h, "noraHint"));
    // What she read has notifications and newsletters: what she can file, and the plan.
    let (app, _) = hosted(answers::state(false, "", true), true);
    let mut h = window(app);
    assert!(shown(&h, "noraHint") && contains(&h, "I can file 8 notifications and 6 newsletters.") && contains(&h, "See the plan"));
    // Its ✕: gone, for this mailbox, today.
    click(&mut h, "noraHintClose");
    assert!(!shown(&h, "noraHint"));
    let today = h.state().session.today().to_days();
    assert_eq!(h.state().ui.nora_hint_hidden.get(ME), Some(&today));
    // Back the next day (it was hidden the day before).
    h.state_mut().ui.nora_hint_hidden.insert(ME.to_string(), today - 1);
    h.run_steps(3);
    assert!(shown(&h, "noraHint"));
    // Where she files the mail, what her last run filed today; while she tidies the inbox, how far she is.
    let (mut app, _) = hosted(answers::state(true, "validated", true), false);
    let run = read_journal(&answers::journal("NFR-3")).runs.remove(0);
    let said = app.nora.sorting.boxes.get(ME).and_then(|k| k.state.clone()).unwrap();
    app.nora.sorting.boxes.insert(ME.to_string(), Known::said(said, Some(run)));
    let h = window(app);
    assert!(shown(&h, "noraHint") && contains(&h, "See the journal"));
    // Outside the Inbox: nothing.
    let (mut app, _) = hosted(answers::state(false, "", true), true);
    app.execute("folder.select", json!({"kind": "sent"})).unwrap();
    let h = window(app);
    assert!(shown(&h, "noraAsk") && !shown(&h, "noraHint"));
}

#[test]
fn what_she_read_is_asked_for_the_proposal_while_the_bar_is_drawn_and_not_again_within_half_an_hour() {
    let (mut app, calls, _) = page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let mut h = window(app);
    let surveys = |calls: &Calls| calls.0.borrow().iter().filter(|(call, _)| call.method == SURVEY).count();
    // The state first: nothing read before it says she files nothing yet.
    assert_eq!(surveys(&calls), 0);
    {
        let kept = calls.0.borrow();
        let (call, to) = kept.iter().rev().find(|(call, to)| call.method == super::super::sorting::STATE && to.is_some()).expect("the state");
        let body = json!({"message": answers::state(false, "", true)}).to_string();
        to.as_ref().unwrap().send(super::super::NoraAnswer { id: call.id, status: 200, body }).unwrap();
    }
    h.run_steps(6);
    assert_eq!(surveys(&calls), 1, "asked once the state is known");
    let args = calls.0.borrow().iter().find(|(call, _)| call.method == SURVEY).map(|(call, _)| call.args.clone()).unwrap();
    assert_eq!(args, json!({"account": ME, "days": 30}));
    h.run_steps(30);
    assert_eq!(surveys(&calls), 1, "not again within half an hour");
}

/// [`hosted`], with `more` messages more in the Inbox: a list longer than the window.
fn long(state: Value, survey: bool, more: usize) -> (MailApp, Calls) {
    let (mut app, calls) = hosted(state, survey);
    let inbox = app.session.view.folder;
    for i in 0..more {
        let when = super::super::testing::NOW - 3_600 * (i as i64 + 1);
        let m = mailcraft_model::Message {
            folder: inbox,
            from: mailcraft_model::Address::new("Commandes Exemple", "commandes@example.org"),
            subject: format!("Commande L{i} : 6 tables"),
            date: when,
            received: when,
            ..Default::default()
        };
        app.session.mailbox.add_message(m).unwrap();
    }
    (app, calls)
}

#[test]
fn the_line_is_opaque_and_the_list_scrolls_its_last_row_above_the_bar_and_the_line() {
    // The design system's soft signal is a veil; laid on the paper it is opaque.
    let veil = egui::Color32::from_rgba_unmultiplied(0xF5, 0x5D, 0x1A, 41);
    assert_eq!(super::opaque(egui::Color32::from_rgb(0x14, 0x14, 0x14), veil).a(), 255);
    let (app, _) = long(answers::state(false, "", true), true, 40);
    let mut h = window(app);
    assert!(shown(&h, "noraHint") && shown(&h, "noraBackdrop"));
    // To the end of the list.
    let first = h.state_mut().session.rows().first().unwrap().id;
    let over = h.state().hit.rect("message", first).expect("the first row").center();
    h.hover_at(over);
    for _ in 0..30 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -800.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        });
        h.step();
    }
    h.run_steps(4);
    let rows = h.state_mut().session.rows();
    let last = rows.last().unwrap().id;
    let row = h.state().hit.rect("message", last).expect("the last row, scrolled to");
    let backdrop = h.state().hit.rect("noraBackdrop", 0).unwrap();
    let line = h.state().hit.rect("noraHint", 0).unwrap();
    assert!(row.max.y <= line.min.y, "the last row {row:?} above the line {line:?}");
    assert!(backdrop.max.y >= h.state().hit.rect("noraAsk", 0).unwrap().max.y && backdrop.min.y < line.min.y, "a backdrop under the whole block");
}

/// The window `size` wide on [`long`], Nora's pane opened by a question asked in her bar.
fn with_her_pane(width: f32) -> Window {
    let (app, _) = long(answers::state(false, "", true), true, 12);
    let mut h = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(vec2(width, 768.0))
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
    assert!(super::super::assist::ask(h.state_mut(), "Qu'est-ce qui attend ma réponse ?"));
    h.run_steps(6);
    h
}

#[test]
fn beside_her_pane_in_a_small_window_the_list_keeps_its_title_and_folds_its_controls_into_its_menu() {
    let mut h = with_her_pane(1024.0);
    assert!(h.state().nora.assist.open);
    let name = h.state().hit.rect("listViewName", 0).expect("the view's name");
    assert!(name.width() >= 60.0, "the title keeps its room: {name:?}");
    for control in ["listShowDone", "listSearch", "listNew"] {
        assert!(!shown(&h, control), "{control} is in the name's menu");
    }
    click(&mut h, "listViewName");
    for item in ["New Email", "Search", "Show done", "Arrange By"] {
        assert!(contains(&h, item), "{item} in the menu");
    }
    // The desk's frame of a small window: the name keeps a cut of itself and its ▾.
    let h = with_her_pane(728.0);
    let name = h.state().hit.rect("listViewName", 0).expect("the view's name");
    assert!(name.width() >= 50.0, "the title keeps its room: {name:?}");
}

/// Nora's bar and the line above it in French, light and dark, to PNG files in `VARIANT_D_SHOTS`: the bar alone, what she
/// proposes, what her last run filed, and a question typed in the bar.
#[test]
#[ignore = "draws to PNG files: VARIANT_D_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored ask_bar_screens"]
fn ask_bar_screens_in_french() {
    let Some(dir) = std::env::var_os("VARIANT_D_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        for screen in [
            "7-barre-de-nora",
            "8-proposition",
            "9-journal-du-jour",
            "10-question-tapee",
            "14-liste-longue-sous-la-barre",
            "15-volet-ouvert-1024",
            "16-volet-ouvert-728",
        ] {
            if screen.starts_with("15") || screen.starts_with("16") {
                let mut h = with_her_pane(if screen.starts_with("15") { 1024.0 } else { 728.0 });
                h.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
                let _ = h.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
                h.run_steps(6);
                let image = h.render().expect("drawn");
                image.save(dir.join(format!("{screen}-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
                continue;
            }
            let (mut app, _) = if screen.starts_with("14") {
                long(answers::state(false, "", true), true, 40)
            } else {
                hosted(answers::state(screen == "9-journal-du-jour", "", true), screen != "7-barre-de-nora")
            };
            if screen == "9-journal-du-jour" {
                let run = read_journal(&answers::journal("NFR-3")).runs.remove(0);
                let said = app.nora.sorting.boxes.get(ME).and_then(|k| k.state.clone()).unwrap();
                app.nora.sorting.boxes.insert(ME.to_string(), Known::said(said, Some(run)));
            }
            app.execute("ui.language", json!({"language": "fr"})).unwrap();
            let _ = app.run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
            let mut h = window(app);
            if screen == "10-question-tapee" {
                click(&mut h, "noraAskText");
                h.input_mut().events.push(Event::Text("Qu'est-ce qui attend ma réponse ?".into()));
            }
            h.run_steps(4);
            let image = h.render().expect("drawn");
            image.save(dir.join(format!("{screen}-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
        }
    }
}
