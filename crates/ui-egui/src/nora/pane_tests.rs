// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): Nora's pane through real frames (neoffice-maintenance#2487 and the batch with it).
//! Nora's pane through real frames, in a hosting page's tab of invented people at `example.org`: Enter sends a written answer to one
//! of her questions as its button does; the depth question of a plan by party never cuts its buttons, the pane 240 to 420 wide;
//! « Nouvelle conversation » leaves « Trier avec Nora » and her questions for a new, empty conversation; and below the width where
//! the list at its width, the pane and a readable message stand side by side, her pane takes the message's place (as Spark has it),
//! nothing overlapping at 1024, 1100, 1280 or 1440. And (`--ignored`, with `NORA_PANE_SHOTS=<folder>`) those screens in French,
//! light and dark.

use egui::{Event, Key, Modifiers, Rect, vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::{Value, json};

use super::super::sorting::{self, ANSWER, Flow, Known, Questions, Stage, read_asks, read_plan, read_state, read_survey};
use super::super::testing::{Calls, ME, id_of, page};
use crate::MailApp;

type Window = egui_kittest::Harness<'static, MailApp>;

/// The harness draws the window in a central panel, which keeps egui's margin of 8 at each side: the room of a window `w` wide
/// (what the panes at its right and the mail share, all of a hosting page's tab) is that of a harness `w + EDGE` wide.
const EDGE: f32 = 16.0;

fn window(app: MailApp, size: (f32, f32)) -> Window {
    let mut h = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(vec2(size.0, size.1))
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

/// A hosting page's tab on its Inbox (`E1` to `E3`, `E1` selected), the server having said `state` of its mailbox.
fn hosted(state: Value) -> (MailApp, Calls) {
    let (mut app, calls, _) = page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    app.nora.sorting.boxes.insert(ME.to_string(), Known::said(read_state(&state), None));
    (app, calls)
}

fn made(calls: &Calls, method: &str) -> Vec<Value> {
    calls.0.borrow().iter().filter(|(call, _)| call.method == method).map(|(call, _)| call.args.clone()).collect()
}

fn click_at(h: &mut Window, at: egui::Pos2) {
    crate::people_view::testing::click(h, at, egui::PointerButton::Primary, 1);
    h.run_steps(4);
}

fn enter(h: &mut Window) {
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(4);
}

/// One question of Nora's about the page's mailbox, as `nora.api.asks.mine` says it.
fn one_question() -> Questions {
    let asks = json!({"asks": [{"name": "ASK-1", "title": "Payment notices from your bank: shall I make a task of each?", "context": "3 this month",
        "choices": [{"code": "task", "label": "Yes, a task"}, {"code": "none", "label": "No, file them"}], "box": ME}], "retained": 0});
    Questions::said(read_asks(&asks).0)
}

// ---- Enter sends a written answer ----

/// Enter in « Répondre à Nora… » sends what is typed, as « Répondre » does: in her pane's home, and in her questions' pane.
#[test]
fn enter_in_the_field_of_an_answer_sends_it_as_its_button_does() {
    for questions_pane in [false, true] {
        let (mut app, calls) = hosted(sorting::tests::state(false, "validated", true));
        app.nora.sorting.questions = one_question();
        if questions_pane {
            app.run("ui.noraQuestions", json!({})).unwrap();
        } else {
            app.run("ui.noraAssistant", json!({"open": true})).unwrap();
        }
        let mut h = window(app, (1440.0, 900.0));
        // The field and its button never make the pane wider than the person left it (it grew frame after frame).
        h.run_steps(30);
        assert!((pane(&h).width() - super::WIDTH).abs() < 1.5, "the pane keeps its width: {:?}", pane(&h));
        let field = h.get_by_label("Answer Nora…").rect().center();
        click_at(&mut h, field);
        h.input_mut().events.push(Event::Text("They are my bank's own".into()));
        h.run_steps(3);
        assert!(made(&calls, ANSWER).is_empty(), "nothing goes before Enter");
        enter(&mut h);
        assert_eq!(
            made(&calls, ANSWER),
            vec![json!({"name": "ASK-1", "text": "They are my bank's own", "channel": "text"})],
            "Enter sends it (the questions' pane: {questions_pane})"
        );
        // Enter in an empty field sends nothing.
        let (mut app, calls) = hosted(sorting::tests::state(false, "validated", true));
        app.nora.sorting.questions = one_question();
        app.run("ui.noraAssistant", json!({"open": true})).unwrap();
        let mut h = window(app, (1440.0, 900.0));
        let field = h.get_by_label("Answer Nora…").rect().center();
        click_at(&mut h, field);
        enter(&mut h);
        assert!(made(&calls, ANSWER).is_empty(), "an empty answer isn't sent");
    }
}

// ---- the depth question's buttons in a narrow pane ----

/// Her pane `width` wide on Nora's question before a plan by party (« Un sous-dossier par client, fournisseur… »), in a window
/// wide enough for the pane to keep that width beside the list and a message.
fn asking_parties(width: f32) -> Window {
    let (mut app, _calls) = hosted(sorting::tests::with_parties(sorting::tests::state(false, "", true)));
    app.run("ui.noraAssistant", json!({"open": true})).unwrap();
    app.nora.assist.width = width;
    let said = app.nora.sorting.available(ME).cloned().unwrap();
    let mut flow = Flow::new(ME.to_string(), false, crate::now_ms());
    flow.stage = Stage::Parties;
    flow.chosen = Some(said.modes[0].clone());
    flow.survey = Some(read_survey(&sorting::tests::survey_with_parties()));
    flow.party_types = vec!["clients".into(), "suppliers".into(), "leads".into()];
    app.nora.sorting.flow = Some(flow);
    window(app, (1600.0, 1150.0))
}

/// How the words `words` were drawn this frame: whole (not cut short with an ellipsis), and where their lines stand; `None`
/// where they weren't drawn.
fn drawn(h: &Window, words: &str) -> Option<(bool, Rect)> {
    fn find(shape: &egui::Shape, words: &str) -> Option<(bool, Rect)> {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == words => {
                Some((!text.galley.elided, Rect::from_min_size(text.pos, text.galley.size())))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, words)),
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|clipped| find(&clipped.shape, words))
}

/// The pane's rect, as the window drew it.
fn pane(h: &Window) -> Rect {
    h.state().hit.rect("noraPane", 0).expect("her pane")
}

#[test]
fn the_depth_questions_buttons_wrap_and_are_never_cut_from_the_narrowest_pane_to_420() {
    for width in [240.0, 260.0, 300.0, 340.0, 380.0, 420.0] {
        let h = asking_parties(width);
        let pane = pane(&h);
        assert!((pane.width() - width).abs() < 1.5, "the pane keeps its width {width}: {pane:?}");
        for words in ["A subfolder for each client, supplier…", "Only a folder for each kind"] {
            let button = h.get_by_label(words).rect();
            // Inside the pane, with the panel's margin at its right.
            assert!(button.min.x >= pane.min.x && button.max.x <= pane.max.x - 4.0, "{width}: « {words} » {button:?} leaves the pane {pane:?}");
            // Its words whole, on as many lines as they need, each clear of its border (as one line is in a button 36 high).
            let (whole, text) = drawn(&h, words).unwrap_or_else(|| panic!("{width}: « {words} » drawn"));
            assert!(whole, "{width}: « {words} » cut short in {button:?}");
            let clear = text.min.x >= button.min.x + 6.0 && text.max.x <= button.max.x - 6.0;
            assert!(
                clear && text.min.y >= button.min.y + 5.0 && text.max.y <= button.max.y - 5.0,
                "{width}: « {words} » {text:?} against the border of {button:?}"
            );
        }
    }
}

// ---- « Nouvelle conversation » ----

/// Her pane open on « Trier avec Nora »'s flow, choosing a way to file, her conversation started (`started`) or not.
fn in_flow(started: bool) -> (Window, Calls) {
    let (mut app, calls) = hosted(sorting::tests::state(false, "proposed", true));
    if started {
        assert!(super::ask(&mut app, "What waits for my answer?"));
    } else {
        app.run("ui.noraAssistant", json!({"open": true})).unwrap();
    }
    let mut flow = Flow::new(ME.to_string(), false, crate::now_ms());
    flow.stage = Stage::Choose;
    flow.survey = Some(read_survey(&sorting::tests::survey()));
    flow.plan = Some(read_plan(&sorting::tests::plan("Proposed")));
    app.nora.sorting.flow = Some(flow);
    let h = window(app, (1440.0, 900.0));
    (h, calls)
}

/// The pane's « New conversation », at its right (in English, or in French for the screens).
fn new_conversation(h: &Window) -> egui_kittest::Node<'_> {
    ["New conversation", "Nouvelle conversation"]
        .into_iter()
        .find_map(|label| h.query_all_by_label(label).find(|n| n.rect().min.x > 1000.0))
        .expect("her pane's New conversation")
}

#[test]
fn new_conversation_leaves_the_flow_and_her_questions_for_an_empty_conversation() {
    for started in [false, true] {
        let (mut h, _calls) = in_flow(started);
        assert!(h.query_by_label("‹ Back to the conversation").is_some(), "the flow is in her pane");
        let button = new_conversation(&h);
        assert!(!button.accesskit_node().is_disabled(), "offered in the flow (her conversation started: {started})");
        let at = button.rect().center();
        click_at(&mut h, at);
        assert!(h.state().nora.sorting.flow.is_none(), "the flow is left");
        assert!(h.state().nora.assist.open, "her pane stays open");
        let session = &h.state().nora.assist.session;
        assert!(session.transcript().is_empty() && session.thread_id().is_none(), "a new conversation, empty");
        assert!(h.query_by_label("‹ Back to the conversation").is_none());
        assert!(h.query_by_label("Morning summary").is_some(), "the empty conversation's recipes");
    }
    // From her questions (in place of her conversation, over the flow): the same.
    let (mut h, _calls) = in_flow(true);
    h.state_mut().nora.sorting.questions = one_question();
    h.state_mut().run("ui.noraQuestions", json!({})).unwrap();
    h.run_steps(4);
    assert!(h.state().nora.assist.questions && h.query_by_label("Nora asks you (1)").is_some());
    let button = new_conversation(&h);
    assert!(!button.accesskit_node().is_disabled(), "offered on her questions");
    let at = button.rect().center();
    click_at(&mut h, at);
    let state = h.state();
    assert!(!state.nora.assist.questions && state.nora.sorting.flow.is_none(), "her questions and the flow are left");
    assert!(state.nora.assist.session.transcript().is_empty(), "a new conversation, empty");
    assert!(h.query_by_label("Morning summary").is_some());
}

// ---- her pane in the message's place in a narrow window ----

/// A hosting page's tab `width` wide, `E1` read in the pane at the right; the page lists the folders in its own menu (`host_menu`)
/// or not (the folder pane shown).
fn reading(width: f32, host_menu: bool) -> Window {
    let (mut app, _calls) = hosted(sorting::tests::state(false, "validated", true));
    if host_menu {
        app.services.host_menu = Some(crate::host_menu::Menu::default());
    }
    window(app, (width + EDGE, 900.0))
}

/// What is drawn of the mail and the panes at the right: the folder pane, the list, the message being read, her pane, the
/// Échanges pane (where each is).
fn panes(h: &Window) -> Vec<(&'static str, Rect)> {
    ["folderPane", "mailList", "readingPane", "noraPane", "exchangesPane"]
        .into_iter()
        .filter_map(|kind| h.state().hit.rect(kind, 0).map(|r| (kind, r)))
        .collect()
}

fn none_overlap(h: &Window, width: f32) {
    let drawn = panes(h);
    for (i, (a, ra)) in drawn.iter().enumerate() {
        assert!(ra.min.x >= EDGE / 2.0 - 0.5 && ra.max.x <= width + EDGE / 2.0 + 0.5, "{width}: {a} {ra:?} leaves the window");
        for (b, rb) in drawn.iter().skip(i + 1) {
            let both = ra.intersect(*rb);
            assert!(both.width() <= 1.0 || both.height() <= 1.0, "{width}: {a} {ra:?} and {b} {rb:?} overlap");
        }
    }
}

#[test]
fn below_the_list_the_pane_and_a_readable_message_her_pane_takes_the_messages_place() {
    // The threshold with the widths a tab starts with: the list 380, her pane 330, a message 380.
    let threshold = crate::UiState::default().list_width + super::WIDTH + crate::mail_view::READING_MIN;
    assert_eq!(threshold, 1090.0);
    for host_menu in [true, false] {
        for width in [1024.0, 1089.0, 1090.0, 1100.0, 1280.0, 1440.0] {
            let mut h = reading(width, host_menu);
            let e1 = id_of(h.state(), "E1");
            assert!(h.state().hit.rect("readingPane", 0).is_some(), "{width}: the message, her pane closed");
            none_overlap(&h, width);
            h.state_mut().run("ui.noraAssistant", json!({"open": true})).unwrap();
            h.run_steps(4);
            let pane = h.state().hit.rect("noraPane", 0).expect("her pane");
            let list = h.state().hit.rect("mailList", 0).expect("the list");
            let message = h.state().hit.rect("readingPane", 0);
            // The room the panes at the right and the mail share: the window, but the module bar where the page doesn't draw it.
            let left = panes(&h).iter().map(|(_, r)| r.min.x).fold(f32::INFINITY, f32::min);
            let room = pane.max.x - left;
            if host_menu {
                assert!((room - width).abs() <= 1.0, "{width}: the tab's whole width: {room}");
            }
            if room < threshold {
                assert!(message.is_none(), "{width} (room {room}): her pane in the message's place");
                assert!((pane.width() - super::WIDTH).abs() < 1.5, "{width}: her pane at its width: {pane:?}");
                assert!((list.max.x - pane.min.x).abs() <= 2.0, "{width}: the list up to her pane: {list:?} {pane:?}");
                assert_eq!(h.state().session.sel.focus(), Some(e1), "the message stays selected");
            } else {
                let message = message.unwrap_or_else(|| panic!("{width} (room {room}): the message beside her pane"));
                assert!(message.width() >= crate::mail_view::READING_MIN - 1.0, "{width}: a readable message: {message:?}");
                assert!(list.width() >= crate::mail_view::LIST_MIN - 1.0, "{width}: {list:?}");
            }
            none_overlap(&h, width);
            // The pane closed, the message comes back.
            h.state_mut().run("ui.noraAssistant", json!({"open": false})).unwrap();
            h.run_steps(4);
            assert!(h.state().hit.rect("readingPane", 0).is_some(), "{width}: the message back");
            none_overlap(&h, width);
        }
    }
    // A window that widens past it: the message comes back beside her pane; narrower again, it goes.
    let mut h = reading(1024.0, true);
    h.state_mut().run("ui.noraAssistant", json!({"open": true})).unwrap();
    h.run_steps(4);
    assert!(h.state().hit.rect("readingPane", 0).is_none());
    h.set_size(vec2(1280.0 + EDGE, 900.0));
    h.run_steps(4);
    assert!(h.state().hit.rect("readingPane", 0).is_some(), "widened: the message beside her pane");
    none_overlap(&h, 1280.0);
    h.set_size(vec2(1024.0 + EDGE, 900.0));
    h.run_steps(4);
    assert!(h.state().hit.rect("readingPane", 0).is_none(), "narrowed: her pane in its place again");
    none_overlap(&h, 1024.0);
}

/// A message being written stays beside her pane, whatever the width: Nora writes in it.
#[test]
fn a_message_being_written_stays_beside_her_pane_in_a_narrow_window() {
    let mut h = reading(1024.0, true);
    h.state_mut().run("ui.noraAssistant", json!({"open": true})).unwrap();
    h.state_mut().run("mail.reply", json!({})).unwrap();
    h.run_steps(6);
    assert!(h.state().session.compose.is_some());
    assert!(h.state().hit.rect("readingPane", 0).is_some(), "the message being written is drawn");
    none_overlap(&h, 1024.0);
}

/// The Échanges pane stays consistent with hers: in a narrow window it never opens by itself, and hers, opened, comes first; in a
/// wide one it opens by itself beside the message and gives way to hers.
#[test]
fn the_exchanges_pane_never_opens_by_itself_where_her_pane_would_take_the_messages_place() {
    for width in [1024.0, 1100.0, 1280.0] {
        let h = reading(width, true);
        assert!(!h.state().exchanges.open, "{width}: no room for the exchanges by themselves");
    }
    let mut h = reading(1440.0, true);
    assert!(h.state().exchanges.open, "1440: the exchanges by themselves");
    none_overlap(&h, 1440.0);
    h.state_mut().run("ui.noraAssistant", json!({"open": true})).unwrap();
    h.run_steps(4);
    assert!(!h.state().exchanges.open && h.state().hit.rect("readingPane", 0).is_some(), "hers first, the message beside it");
    none_overlap(&h, 1440.0);
}

// ---- the screens ----

/// The screens of this batch in French, light and dark, to PNG files in `NORA_PANE_SHOTS`: the depth question at 240 and 420,
/// the flow's New conversation, and her pane in the message's place at 1024 beside the message at 1280.
#[test]
#[ignore = "draws to PNG files: NORA_PANE_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored pane_screens"]
fn pane_screens_in_french() {
    let Some(dir) = std::env::var_os("NORA_PANE_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        let tone = if dark { "sombre" } else { "clair" };
        let french = |h: &mut Window| {
            h.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
            let _ = h.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
            h.run_steps(6);
        };
        for width in [240.0, 300.0, 420.0] {
            let mut h = asking_parties(width);
            french(&mut h);
            h.render().expect("drawn").save(dir.join(format!("31-profondeur-volet-{width}-{tone}.png"))).expect("saved");
        }
        let (mut h, _calls) = in_flow(true);
        french(&mut h);
        h.render().expect("drawn").save(dir.join(format!("32-flux-nouvelle-conversation-{tone}.png"))).expect("saved");
        let at = new_conversation(&h).rect().center();
        click_at(&mut h, at);
        h.render().expect("drawn").save(dir.join(format!("33-apres-nouvelle-conversation-{tone}.png"))).expect("saved");
        let (mut app, _calls) = hosted(sorting::tests::state(false, "validated", true));
        app.nora.sorting.questions = one_question();
        app.run("ui.noraAssistant", json!({"open": true})).unwrap();
        let mut h = window(app, (1440.0, 900.0));
        french(&mut h);
        let field = h.get_by_label("Répondre à Nora…").rect().center();
        click_at(&mut h, field);
        h.input_mut().events.push(Event::Text("Ce sont celles de ma banque".into()));
        h.run_steps(4);
        h.render().expect("drawn").save(dir.join(format!("34-reponse-ecrite-{tone}.png"))).expect("saved");
        for width in [1024.0, 1100.0, 1280.0, 1440.0] {
            let mut h = reading(width, true);
            french(&mut h);
            h.state_mut().run("ui.noraAssistant", json!({"open": true})).unwrap();
            h.run_steps(6);
            h.render().expect("drawn").save(dir.join(format!("35-volet-a-{width}-{tone}.png"))).expect("saved");
        }
    }
}
