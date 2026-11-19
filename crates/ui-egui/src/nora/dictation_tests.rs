// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): dictating to Nora in her pane, through real frames.
//! Dictating to Nora in her pane, through real frames of a hosting page's tab (invented people at `example.org`): the microphone is
//! on offer only where the host carries a recording and her conversation is what the pane shows; a recording goes out by a call of
//! its own, in the interface's language; what comes back goes into the pane's box, for the person to read and send, and nothing is
//! sent by itself; a refusal, a failure or a phantom transcript leaves the pane as usable as it was; and once the pane is closed or
//! shows something else, what was on its way is forgotten. The microphone itself is the browser's: it is tried in one (there is none
//! in these tests, `craft-nora-ui` has its own tests of the recorder's states).

use egui::vec2;
use serde_json::{Value, json};

use super::super::sorting::{Known, read_state};
use super::super::testing::{Calls, ME, page};
use super::super::{CHAT_METHODS, DICTATION, METHODS, NoraAnswer, READY_METHODS, SORTING_METHODS};
use super::*;
use crate::MailApp;
use crate::i18n::{Lang, RestoreLanguage};

type Window = egui_kittest::Harness<'static, MailApp>;

fn window(app: MailApp) -> Window {
    let mut h = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(vec2(1440.0, 900.0))
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

/// A hosting page's tab with her pane open on her conversation.
fn with_pane() -> (Window, Calls) {
    let (mut app, calls, _) = page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    toggle(&mut app, &json!({"open": true})).unwrap();
    let mut h = window(app);
    h.run_steps(2);
    (h, calls)
}

/// The person stopped a recording: what the panel hands the window.
fn dictate(h: &mut Window) {
    let app = h.state_mut();
    let outputs = on_actions(app, vec![PanelAction::Dictate { audio_base64: "QUJDRA==".into(), mime: "audio/webm".into() }]);
    perform(app, outputs);
    h.run_steps(2);
}

fn dictations(calls: &Calls) -> Vec<Value> {
    calls.0.borrow().iter().filter(|(call, _)| call.method == DICTATION).map(|(call, _)| call.args.clone()).collect()
}

/// Answer the last dictation with an HTTP status and a body, and let the window read it.
fn answer(h: &mut Window, calls: &Calls, status: u16, body: Value) {
    {
        let mut kept = calls.0.borrow_mut();
        let (call, to) = kept.iter_mut().rev().find(|(call, to)| call.method == DICTATION && to.is_some()).expect("a dictation under way");
        to.take().unwrap().send(NoraAnswer { id: call.id, status, body: body.to_string() }).unwrap();
    }
    h.run_steps(3);
}

fn words(text: &str) -> Value {
    json!({"message": {"success": true, "text": text}})
}

#[test]
fn a_dictation_has_a_call_of_its_own() {
    assert_eq!(format!("/api/method/{DICTATION}"), craft_nora::protocol::STT_TRANSCRIBE);
    for list in [&CHAT_METHODS[..], &METHODS[..], &READY_METHODS[..], &SORTING_METHODS[..]] {
        assert!(!list.contains(&DICTATION), "its arguments are a recording: no list of plain calls has it");
    }
}

#[test]
fn the_microphone_is_offered_only_where_the_host_carries_a_recording() {
    let (mut h, _calls) = with_pane();
    assert!(h.state().nora.assist.panel.dictation, "a host that carries it");
    h.state_mut().services.nora.as_mut().unwrap().dictation = false;
    h.run_steps(2);
    assert!(!h.state().nora.assist.panel.dictation, "a host that doesn't: no microphone that sends nowhere");
}

#[test]
fn no_microphone_over_nora_s_questions_or_her_sorting() {
    let (mut h, _calls) = with_pane();
    assert!(h.state().nora.assist.panel.dictation);
    h.state_mut().nora.sorting.boxes.insert(ME.to_string(), Known::said(read_state(&json!({"enabled": false})), None));
    h.state_mut().nora.assist.questions = true;
    h.run_steps(2);
    assert!(!h.state().nora.assist.panel.dictation, "her questions: nothing here takes a dictation");
    h.state_mut().nora.assist.questions = false;
    h.run_steps(2);
    assert!(h.state().nora.assist.panel.dictation, "her conversation again");
}

#[test]
fn a_recording_goes_out_by_its_own_call_in_the_interface_s_language() {
    let _language = RestoreLanguage::hold();
    for (lang, code) in [(Lang::French, "fr"), (Lang::German, "de"), (Lang::Italian, "it"), (Lang::English, "en"), (Lang::ChineseSimplified, "zh")] {
        let (mut h, calls) = with_pane();
        // As the person's choice does, and the language a hosting page hands over: the window takes it at each frame.
        h.state_mut().execute("ui.language", json!({"language": lang.code()})).unwrap();
        h.run_steps(2);
        dictate(&mut h);
        assert_eq!(dictations(&calls), vec![json!({"audio_base64": "QUJDRA==", "mime": "audio/webm", "language": code})], "{code}");
    }
}

#[test]
fn the_words_go_into_the_box_and_nothing_is_sent() {
    let (mut h, calls) = with_pane();
    h.state_mut().nora.assist.panel.set_draft("Bonjour");
    dictate(&mut h);
    answer(&mut h, &calls, 200, words(" résume ce fil "));
    assert_eq!(h.state().nora.assist.panel.draft(), "Bonjour résume ce fil");
    assert!(!calls.0.borrow().iter().any(|(call, _)| CHAT_METHODS.contains(&call.method)), "the person reads it and sends it");
    assert!(!h.state().nora.assist.session.is_busy());
}

#[test]
fn a_failed_or_empty_dictation_leaves_the_pane_as_it_was() {
    for (status, body) in [
        (200, json!({"message": {"success": false, "error": "Empty audio"}})),
        (200, words("Sous-titrage Société Radio-Canada")),
        (200, words("   ")),
        (403, json!({"exc_type": "PermissionError"})),
        (429, json!({"exc_type": "TooManyRequestsError"})),
        (500, json!({"exception": "Traceback"})),
        (0, json!("Nora could not be reached")),
    ] {
        let (mut h, calls) = with_pane();
        h.state_mut().nora.assist.panel.set_draft("Bonjour");
        dictate(&mut h);
        answer(&mut h, &calls, status, body.clone());
        let assist = &h.state().nora.assist;
        assert_eq!(assist.panel.draft(), "Bonjour", "{body}");
        assert!(!assist.session.dictating() && !assist.session.is_busy(), "{body}");
        assert!(assist.session.last_error().is_none(), "the conversation is not in error: {body}");
        // She can be asked at once.
        let app = h.state_mut();
        let outputs = on_actions(app, vec![PanelAction::Send("Bonjour".into())]);
        perform(app, outputs);
        assert!(calls.0.borrow().iter().any(|(call, _)| call.method == "nora.api.chat.start_or_resume_thread"), "{body}");
    }
}

#[test]
fn a_host_that_carries_no_recording_gets_none() {
    let (mut h, calls) = with_pane();
    h.state_mut().services.nora.as_mut().unwrap().dictation = false;
    dictate(&mut h);
    assert!(dictations(&calls).is_empty());
    assert!(h.state().nora.assist.session.dictation().is_some_and(|done| done.text.is_none() && done.error.is_some()), "answered at once, with why");
    assert!(!h.state().nora.assist.session.dictating());
}

#[test]
fn closing_the_pane_forgets_what_was_on_its_way() {
    let (mut h, calls) = with_pane();
    dictate(&mut h);
    assert!(h.state().nora.assist.session.dictating());
    toggle(h.state_mut(), &json!({"open": false})).unwrap();
    assert!(!h.state().nora.assist.session.dictating(), "closed at once");
    answer(&mut h, &calls, 200, words("à ne pas écrire"));
    toggle(h.state_mut(), &json!({"open": true})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().nora.assist.panel.draft(), "", "nothing waits in the box when it opens again");
}

#[test]
fn what_is_dictated_is_forgotten_where_the_pane_is_not_drawn() {
    // Backstage takes the whole window, and the pane with it: no frame draws her conversation.
    let (mut h, calls) = with_pane();
    dictate(&mut h);
    h.state_mut().ui.backstage = true;
    h.run_steps(3);
    assert!(!h.state().nora.assist.session.dictating(), "behind the File page");
    h.state_mut().ui.backstage = false;
    answer(&mut h, &calls, 200, words("à ne pas écrire"));
    assert_eq!(h.state().nora.assist.panel.draft(), "");
    // Her questions stand in the place of her conversation.
    let (mut h, calls) = with_pane();
    dictate(&mut h);
    h.state_mut().nora.assist.questions = true;
    h.run_steps(3);
    assert!(!h.state().nora.assist.session.dictating(), "under her questions");
    h.state_mut().nora.assist.questions = false;
    answer(&mut h, &calls, 200, words("à ne pas écrire"));
    assert_eq!(h.state().nora.assist.panel.draft(), "");
}
