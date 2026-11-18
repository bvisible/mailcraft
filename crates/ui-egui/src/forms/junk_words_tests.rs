// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the words of Junk E-mail Options in the interface's language (neoffice-maintenance#2487).
//! Junk E-mail Options says every word of it in the interface's language: its title, its two pages' names (each a tab a screen
//! reader names) and what each page is for, the list's words and its buttons, in French, German, Italian and Spanish; and each
//! of them is in every catalogue of the fork.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use mailcraft_engine::Session;
use mailcraft_i18n::Lang;
use serde_json::json;

use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// Every word the dialog shows, in English: its title, its pages and what each is for, the list's words and its buttons.
const WORDS: [&str; 10] = [
    "Junk E-mail Options",
    "Safe Senders",
    "Blocked Senders",
    "Email from addresses or domain names on your Safe Senders list will never be treated as junk email. A domain (@example.org) is that domain only, not its subdomains.",
    "Email from addresses or domain names on your Blocked Senders list will go to Junk Email as it arrives. A domain (@example.org) is that domain only, not its subdomains.",
    "No entries yet",
    "Remove",
    "Add",
    "Close",
    "someone@example.org or @example.org",
];

#[test]
fn every_word_of_the_lists_box_is_in_every_catalogue_of_the_fork() {
    for lang in [Lang::French, Lang::German, Lang::Italian, Lang::Spanish] {
        let catalogue = mailcraft_i18n::catalogue(lang).expect("compiled in");
        let missing: Vec<&&str> = WORDS.iter().filter(|w| catalogue.get(w).is_none_or(|t| t.trim().is_empty())).collect();
        assert!(missing.is_empty(), "{lang:?} has no word for {missing:?}: add them to crates/i18n/po/neoffice/");
    }
    // What the dialog shows is what the scan of the interface's strings takes (`tl!("…")`), and what the catalogues translate.
    // (However rustfmt laid the call out: on one line, or the literal on a line of its own.)
    let source = include_str!("junk.rs").split_whitespace().collect::<Vec<_>>().join(" ");
    for word in &WORDS[1..6] {
        let call = [format!("tl!(\"{word}\")"), format!("tl!( \"{word}\" )")];
        assert!(call.iter().any(|c| source.contains(c)), "« {word} » is a literal of `tl!` in junk.rs");
    }
}

#[test]
fn the_lists_box_says_its_words_in_french_german_italian_and_spanish() {
    let _language = crate::i18n::RestoreLanguage::hold();
    for (code, lang) in [("fr", Lang::French), ("de", Lang::German), ("it", Lang::Italian), ("es", Lang::Spanish)] {
        let catalogue = mailcraft_i18n::catalogue(lang).expect("compiled in");
        // (As the dialog says it: a guillemet tied to its words, `sorting_view::tied`.)
        let said = |english: &str| crate::nora::sorting_view::tied(catalogue.get(english).unwrap_or(english));
        for (page, about) in [("safe", WORDS[3]), ("blocked", WORDS[4])] {
            let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
            app.autosave = false;
            app.execute("ui.language", json!({"language": code})).unwrap();
            app.execute("ui.dialog", json!({"name": "junkOptions", "list": page})).unwrap();
            let mut h =
                Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
                    |ui, app: &mut MailApp| {
                        let ctx = ui.ctx().clone();
                        app.logic(&ctx);
                        app.ui(ui);
                    },
                    app,
                );
            h.run_steps(6);
            // The two pages, by their names in the language; what the page in view is for; the list's words; the buttons.
            for english in ["Safe Senders", "Blocked Senders", about, "No entries yet", "Remove", "Add", "Close"] {
                let words = said(english);
                assert_ne!(words, english, "{code}: « {english} » has a translation");
                assert!(h.query_all_by_label(&words).next().is_some(), "{code} ({page}): « {words} » on screen");
                assert!(h.query_all_by_label(english).next().is_none(), "{code} ({page}): « {english} » in English on screen");
            }
        }
    }
}

/// The dialog on each of its pages in French, light and dark, to PNG files in `JUNK_OPTIONS_SHOTS`.
#[test]
#[ignore = "draws to PNG files: JUNK_OPTIONS_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored junk_options_screens"]
fn junk_options_screens_in_french() {
    let Some(dir) = std::env::var_os("JUNK_OPTIONS_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        for page in ["safe", "blocked"] {
            let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
            app.autosave = false;
            app.execute("ui.language", json!({"language": "fr"})).unwrap();
            let _ = app.run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
            if page == "blocked" {
                let account = app.session.mailbox.default_account().map(|a| a.id).unwrap_or_default();
                app.session.mailbox.junk_lists_mut(account).blocked.push("offres@boutique.example".into());
            }
            app.execute("ui.dialog", json!({"name": "junkOptions", "list": page})).unwrap();
            let mut h =
                Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
                    |ui, app: &mut MailApp| {
                        let ctx = ui.ctx().clone();
                        app.logic(&ctx);
                        app.ui(ui);
                    },
                    app,
                );
            h.run_steps(8);
            let image = h.render().expect("drawn");
            image.save(dir.join(format!("51-listes-{page}-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
        }
    }
}
