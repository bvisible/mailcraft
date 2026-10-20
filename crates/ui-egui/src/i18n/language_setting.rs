// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The `language` setting in the window: its command, File › Options' choice, what a frame is
//! drawn in, and that it is kept with the other interface settings.

use egui_kittest::kittest::Queryable as _;
use mailcraft_engine::Session;
use mailcraft_engine::agent::{self, Access};
use serde_json::json;

use super::{AUTO, Lang, catalog, pseudo, set_system_language};
use crate::{MailApp, Services, UiState};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

fn app() -> MailApp {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app
}

/// The window, drawn the way the host draws it.
fn window(app: MailApp) -> egui_kittest::Harness<'static, MailApp> {
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 900.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(3);
    harness
}

/// Whether something on screen is called `label` (once or more: a choice that is open is both its own button and its list).
fn shows(harness: &egui_kittest::Harness<'static, MailApp>, label: &str) -> bool {
    harness.query_all_by_label(label).next().is_some()
}

/// The choice of languages when it shows `value` (a combo box has its text as its value, and no label).
fn choice<'a>(harness: &'a egui_kittest::Harness<'static, MailApp>, value: &str) -> Option<egui_kittest::Node<'a>> {
    harness.query_all_by_role(egui::accesskit::Role::ComboBox).find(|node| node.value().as_deref() == Some(value))
}

fn options(harness: &mut egui_kittest::Harness<'static, MailApp>) {
    harness.state_mut().execute("ui.backstage", json!({"value": true, "page": "options"})).unwrap();
    harness.run_steps(3);
}

#[test]
fn the_command_keeps_one_form_of_the_setting_and_refuses_the_rest() {
    let mut a = app();
    assert_eq!(a.ui.language, AUTO, "a new installation follows the system");
    for (said, kept) in [
        ("fr", "fr"),
        ("FR-ch", "fr"),
        ("  fr_CA.UTF-8 ", "fr"),
        ("en-GB", "en"),
        ("pseudo", "pseudo"),
        ("PSEUDO", "pseudo"),
        ("auto", "auto"),
        (" Auto ", "auto"),
    ] {
        let answer = a.execute("ui.language", json!({"value": said})).unwrap_or_else(|e| panic!("{said:?}: {e}"));
        assert_eq!(answer["value"], kept, "{said:?}");
        assert_eq!(a.ui.language, kept, "{said:?}");
    }
    a.execute("ui.language", json!({"value": "fr"})).unwrap();
    for refused in [
        json!({"value": "klingon"}),
        json!({"value": ""}),
        json!({"value": "de"}),
        json!({"value": 7}),
        json!({"value": null}),
        json!({}),
        json!([]),
        json!("fr"),
    ] {
        let error = a.execute("ui.language", refused.clone()).unwrap_err();
        let codes: Vec<&str> = Lang::choices().map(Lang::code).collect();
        assert!(error.contains("auto") && error.contains(&codes.join(", ")), "{refused}: {error}");
        assert_eq!(a.ui.language, "fr", "{refused}: a refused value changes nothing");
    }
    // It is a command like the window's others: one that is listed, that is allowed to agents (it
    // is looks, as the theme is), and that is there to run.
    assert!(crate::UI_COMMANDS.iter().any(|c| c.0 == "ui.language" && c.2 == "File › Options"));
    assert_eq!(agent::access("ui.language"), Some(Access::Safe));
    assert!(a.enabled("ui.language"));
}

#[test]
fn the_setting_is_kept_with_the_other_interface_settings() {
    let mut a = app();
    a.execute("ui.language", json!({"value": "fr"})).unwrap();
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    assert!(saved.contains("\"language\":\"fr\""), "{saved}");
    let mut b = app();
    b.apply_prefs(serde_json::from_str(&saved).unwrap());
    assert_eq!(b.ui.language, "fr");
    // A file that someone edited can say anything: the window still opens, in English.
    for odd in [
        json!({"language": "klingon"}),
        json!({"language": ""}),
        json!({"language": "x".repeat(100_000)}),
        json!({"language": 7}),
        json!({"language": null}),
    ] {
        let state: UiState = serde_json::from_value(odd.clone()).unwrap_or_default();
        let mut c = app();
        c.apply_prefs(state);
        assert!(
            matches!(Lang::from_pref(&c.ui.language), Lang::English | Lang::Catalog(_)),
            "{}",
            odd.to_string().chars().take(40).collect::<String>()
        );
        let _ = window(c);
    }
}

#[test]
fn a_frame_is_drawn_in_the_language_of_the_setting() {
    const LABEL: &str = "Mark a message as read when it is opened in the reading pane";
    let mut harness = window(app());
    options(&mut harness);
    assert!(shows(&harness, LABEL), "English to begin with");
    assert!(choice(&harness, "System default").is_some());
    harness.state_mut().execute("ui.language", json!({"value": "pseudo"})).unwrap();
    harness.run_steps(3);
    assert!(!shows(&harness, LABEL), "the English is gone");
    assert!(shows(&harness, pseudo(LABEL)), "and the pseudo-language is there");
    assert!(choice(&harness, "Pseudo").is_some(), "the choice says which language is drawn");
    // Back to the system's language, which is English until the host says otherwise.
    harness.state_mut().execute("ui.language", json!({"value": "auto"})).unwrap();
    harness.run_steps(3);
    assert!(shows(&harness, LABEL) && choice(&harness, "System default").is_some());
}

#[test]
fn auto_follows_the_host_s_language_and_a_chosen_one_does_not() {
    let mut harness = window(app());
    options(&mut harness);
    assert_eq!(super::current(), Lang::English);
    // The host says the system is in French: "System default" is still what the choice says, in
    // French now, and the window is drawn with the French catalog.
    set_system_language("fr-CH");
    harness.run_steps(3);
    assert_eq!(super::current(), Lang::from_tag("fr").unwrap());
    assert_ne!(super::t("System default"), "System default", "the French catalog has it");
    assert!(choice(&harness, super::t("System default")).is_some());
    harness.state_mut().execute("ui.language", json!({"value": "en"})).unwrap();
    harness.run_steps(3);
    assert_eq!(super::current(), Lang::English, "a chosen language is not the system's");
    assert!(choice(&harness, "English").is_some());
    set_system_language("");
}

#[test]
fn options_offers_the_system_s_language_english_and_each_language_that_has_a_catalog() {
    let mut harness = window(app());
    options(&mut harness);
    // The choice shows what is set, and opens to the list.
    choice(&harness, "System default").unwrap().click();
    harness.run_steps(3);
    let mut offered = vec!["System default".to_string(), "English".to_string()];
    offered.extend(catalog::LANGUAGES.iter().map(|row| row.name.to_string()));
    for name in &offered {
        assert!(shows(&harness, name), "the list has no {name}");
    }
    // Picking a language sets it, by its code, and the choice then shows its own name.
    for row in catalog::LANGUAGES {
        harness.get_by_label(row.name).click();
        harness.run_steps(3);
        assert_eq!(harness.state().ui.language, row.code);
        assert!(choice(&harness, row.name).is_some(), "{} is shown by its own name", row.code);
        choice(&harness, row.name).unwrap().click();
        harness.run_steps(3);
    }
    harness.get_by_label("English").click();
    harness.run_steps(3);
    assert_eq!(harness.state().ui.language, "en");
    assert!(choice(&harness, "English").is_some());
    choice(&harness, "English").unwrap().click();
    harness.run_steps(3);
    harness.get_by_label("System default").click();
    harness.run_steps(3);
    assert_eq!(harness.state().ui.language, AUTO);
    assert!(choice(&harness, "System default").is_some());
}
