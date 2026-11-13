// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! « Trier avec Nora » through real frames: the button and the banner above the Inbox's list only where Nora files the
//! mailbox, the switch of her pane's home, the flow with the ways the server knows and nothing it doesn't, the person who
//! only reads, the activation card, its words in every language of the fork; and (`--ignored`, with
//! `NORA_SORTING_SHOTS=<folder>`) each screen drawn in French, light and dark, to PNG files.

use egui::vec2;
use egui_kittest::kittest::Queryable as _;
use serde_json::json;

use super::super::sorting::{Card, Flow, Known, Noted, Questions, Stage, read_asks, read_journal, read_plan, read_state, read_survey};
use super::super::testing::{ME, page};
use super::*;
use crate::MailApp;

type Window = egui_kittest::Harness<'static, MailApp>;

const BOX: &str = ME;

/// Whether a node of the window is labelled `label` (one or more: a switch and its words share theirs).
fn has(h: &Window, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

fn contains(h: &Window, part: &str) -> bool {
    h.query_all_by_label_contains(part).next().is_some()
}

fn window(app: MailApp, size: (f32, f32)) -> Window {
    let mut harness = egui_kittest::Harness::builder()
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
    harness.run_steps(4);
    harness
}

/// The JSON of the server's answers, from the logic's tests.
mod answers {
    pub(crate) use super::super::super::sorting::tests::{first_reading, journal, plan, state, survey, unread_state};
}

/// A hosting page's tab, its Inbox in view, the server having said `state` of its mailbox (and the run of `latest`).
fn hosted(state: serde_json::Value, latest: bool) -> MailApp {
    let (mut app, _calls, _) = page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let run = latest.then(|| read_journal(&answers::journal("NFR-3")).runs.remove(0));
    app.nora.sorting.boxes.insert(BOX.to_string(), Known::said(read_state(&state), run));
    app
}

#[test]
fn the_button_and_the_banner_are_above_the_inbox_only_where_nora_files_the_mailbox() {
    let h = window(hosted(answers::state(true, "validated", true), true), (1440.0, 900.0));
    assert!(h.state().hit.rect("noraSort", 0).is_some(), "the button");
    assert!(h.state().hit.rect("noraBanner", 0).is_some(), "the banner while she files it");
    assert!(has(&h, "Sort with Nora"));
    assert!(has(&h, "See the journal"));
    // Off: the button, no banner.
    let h = window(hosted(answers::state(false, "validated", true), false), (1440.0, 900.0));
    assert!(h.state().hit.rect("noraSort", 0).is_some() && h.state().hit.rect("noraBanner", 0).is_none());
    // Unavailable (a plan without background assistants): nothing.
    let mut off = answers::state(false, "", true);
    off["available"] = json!(false);
    off["unavailable"] = json!("plan");
    let h = window(hosted(off, false), (1440.0, 900.0));
    assert!(h.state().hit.rect("noraSort", 0).is_none() && h.state().hit.rect("noraBanner", 0).is_none());
    // The program that keeps its mail on the computer: nothing.
    let mut app = hosted(answers::state(true, "validated", true), true);
    app.session.keeps_local = true;
    let h = window(app, (1440.0, 900.0));
    assert!(h.state().hit.rect("noraSort", 0).is_none());
}

#[test]
fn the_button_opens_her_pane_on_the_flow_and_its_home_has_the_switch() {
    let mut h = window(hosted(answers::state(false, "", true), false), (1440.0, 900.0));
    // Her pane's home: « Trier ma boîte » first, the switch after the recipes.
    h.state_mut().run("ui.noraAssistant", json!({"open": true})).unwrap();
    h.run_steps(4);
    assert!(has(&h, "Sort my mailbox"));
    assert!(has(&h, "Nora files my mail"), "the switch");
    assert!(has(&h, "Morning summary"), "the recipes stay");
    // The button above the list opens the flow in it.
    h.get_by_label("Sort with Nora").click();
    h.run_steps(3);
    let flow = h.state().nora.sorting.flow.as_ref().unwrap();
    assert_eq!((flow.account.as_str(), flow.journal_first), (BOX, false));
    assert!(has(&h, "‹ Back to the conversation"));
}

/// Her pane open on the flow at `stage`, with what the server said.
fn in_flow(stage: Stage, may_set: bool) -> MailApp {
    let mut app = hosted(answers::state(false, if stage == Stage::Done { "validated" } else { "proposed" }, may_set), false);
    app.run("ui.noraAssistant", json!({"open": true})).unwrap();
    let mut flow = Flow::new(BOX.to_string(), false, crate::now_ms());
    flow.stage = stage;
    flow.survey = Some(read_survey(&answers::survey()));
    flow.plan = Some(read_plan(&answers::plan(if stage == Stage::Done { "Validated" } else { "Proposed" })));
    flow.chosen = app.nora.sorting.available(BOX).and_then(|s| mode(s, "clients"));
    if stage == Stage::Done {
        flow.tidy_run = "NFR-3".into();
        flow.journal = Some(read_journal(&answers::journal("NFR-3")));
        app.nora.sorting.questions = Questions::said(read_asks(&questions(1)).0);
    }
    app.nora.sorting.flow = Some(flow);
    app
}

#[test]
fn the_flow_offers_the_servers_ways_and_nothing_it_cant_do() {
    let mut app = in_flow(Stage::Choose, true);
    app.nora.sorting.flow.as_mut().unwrap().chosen = None;
    let h = window(app, (1440.0, 900.0));
    for label in ["One folder per client", "By activity", "Notifications and newsletters only", "Let me explain"] {
        assert!(has(&h, label), "{label}");
    }
    assert!(!has(&h, "By project"), "no way the server does not know");
    assert!(contains(&h, "I read the last 30 days of ada@example.org: 91 messages."));
}

#[test]
fn the_plan_offers_to_file_now_or_change_it_and_the_reader_only_reads() {
    let h = window(in_flow(Stage::Plan, true), (1440.0, 900.0));
    assert!(has(&h, "Here is what I would do:"));
    assert!(has(&h, "Clients › Atelier Exemple"));
    assert!(has(&h, "Clients › 3 other clients"));
    assert!(has(&h, "Bulletins d'information"), "the server's own name of the folder");
    assert!(has(&h, "File now") && has(&h, "Change this plan"));
    let h = window(in_flow(Stage::Plan, false), (1440.0, 900.0));
    assert!(has(&h, "Here is what I would do:"));
    assert!(!has(&h, "File now") && !has(&h, "Change this plan"));
    assert!(contains(&h, "only the person in charge of it"));
}

#[test]
fn the_result_can_be_undone_and_asks_whether_to_carry_on() {
    let h = window(in_flow(Stage::Done, true), (1440.0, 900.0));
    assert!(contains(&h, "23 messages changed folder"));
    assert!(has(&h, "Undo this filing"));
    assert!(has(&h, "Next, shall I carry on by myself?"));
    assert!(has(&h, "Every 15 minutes") && has(&h, "No, when I ask"));
    assert!(has(&h, "Nora asks you") && has(&h, "Yes, a task"));
    assert!(has(&h, "JOURNAL"));
    assert!(has(&h, "Undo"), "the journal's « Annuler »");
}

#[test]
fn the_card_turns_nora_on_in_the_ways_the_server_knows() {
    let mut app = hosted(answers::state(false, "", true), false);
    app.nora.sorting.card = Some(Card { account: BOX.into(), mode: "clients".into(), interval: 15, prepare: false, error: None, busy: false });
    let mut h = window(app, (1440.0, 900.0));
    assert!(has(&h, "Nora files your mail"));
    for label in ["One folder per client", "Prepare the work", "Every 15 minutes", "Turn on and see the plan", "Later"] {
        assert!(has(&h, label), "{label}");
    }
    assert!(!contains(&h, "notifications and newsletters apart"), "no switch the server can't honour");
    // « Plus tard »
    h.get_by_label("Later").click();
    h.run_steps(2);
    assert!(h.state().nora.sorting.card.is_none());
}

/// `count` questions of Nora's, as `nora.api.asks.mine` says them, the first about the page's mailbox.
fn questions(count: usize) -> serde_json::Value {
    let asks: Vec<serde_json::Value> = (1..=count)
        .map(|i| {
            if i == 1 {
                json!({"name": "ASK-1", "title": "Payment notices from your bank: shall I make a task of each?", "context": "3 this month",
                       "choices": [{"code": "task", "label": "Yes, a task"}, {"code": "none", "label": "No, file them"}], "box": BOX})
            } else {
                json!({"name": format!("ASK-{i}"), "title": format!("Is Garage Exemple {i} a client or a supplier?"), "context": "",
                       "choices": [{"code": "client", "label": "A client"}, {"code": "supplier", "label": "A supplier"}], "box": "team@example.org"})
            }
        })
        .collect();
    json!({"asks": asks, "retained": 0})
}

#[test]
fn her_questions_are_first_in_her_pane_and_their_number_is_on_her_button_of_the_ribbon() {
    // None: no number, no section.
    let h = window(hosted(answers::state(false, "validated", true), false), (1440.0, 900.0));
    assert!(h.state().hit.rect("noraQuestions", 1).is_none());
    // One, then ten (« 9+ »: the read gives ten at most, as the server shows them).
    for (count, label, shown) in [(1, "1 question from Nora", "1"), (10, "10 questions from Nora", "9+")] {
        let mut app = hosted(answers::state(false, "validated", true), false);
        app.nora.sorting.questions = Questions::said(read_asks(&questions(count)).0);
        let mut h = window(app, (1440.0, 900.0));
        let badge = h.state().hit.rect("noraQuestions", 1).expect("the number on her button");
        let button = h.state().hit.rect("ribbon", crate::ribbon::hit_id("nora")).unwrap();
        assert!(button.intersects(badge), "{badge:?} on {button:?}");
        assert!(has(&h, label), "{label}");
        assert_eq!(badge_words(count), shown);
        // Her pane's home: the questions first, each with its answers, a field and « Later ».
        h.state_mut().run("ui.noraAssistant", json!({"open": true})).unwrap();
        h.run_steps(4);
        assert!(has(&h, "Nora asks you") && has(&h, "Yes, a task") && has(&h, "Later") && has(&h, "Send my answer"));
        if count > 1 {
            assert!(has(&h, "team@example.org"), "which mailbox, where the person has several");
        }
    }
}

/// The string literals of `text`, in order.
fn strings(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut literal = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => literal.extend(chars.next()),
                '"' => break,
                c => literal.push(c),
            }
        }
        found.push(literal);
    }
    found
}

/// The words of each `tl!(` of `source` whose argument is a literal, and the first form of each `tln!(` (its first literal
/// that counts, `{n}`), however rustfmt laid them out.
fn literals(source: &str, macro_open: &str) -> Vec<String> {
    let mut found = Vec::new();
    for rest in source.split(macro_open).skip(1) {
        // The call, to its end on the line, or a few lines on where rustfmt broke it.
        let call: String = rest.lines().take(4).collect::<Vec<_>>().join(" ");
        let quoted = strings(&call);
        let word = if macro_open == "tln!(" {
            quoted.into_iter().find(|w| w.contains("{n}"))
        } else if call.trim_start().starts_with('"') {
            quoted.into_iter().next()
        } else {
            // `tl!(english)`: no literal of its own.
            None
        };
        found.extend(word);
    }
    found
}

/// Its words are this fork's: each has its translation in every language of the fork's catalogues (`po/neoffice/`),
/// read in what is compiled in.
#[test]
fn every_word_of_it_is_translated_in_every_language_of_the_fork() {
    let mut words = Vec::new();
    for source in [include_str!("sorting.rs"), include_str!("sorting_view.rs")] {
        words.extend(literals(source, "tl!("));
        words.extend(literals(source, "tln!("));
    }
    words.sort();
    words.dedup();
    assert!(words.len() >= 80, "the scan finds its words: {}", words.len());
    for lang in [
        mailcraft_i18n::Lang::French,
        mailcraft_i18n::Lang::German,
        mailcraft_i18n::Lang::Spanish,
        mailcraft_i18n::Lang::Italian,
        mailcraft_i18n::Lang::Dutch,
    ] {
        let catalogue = mailcraft_i18n::catalogue(lang).expect("compiled in");
        let missing: Vec<&String> = words.iter().filter(|w| catalogue.get(w).is_none()).collect();
        assert!(missing.is_empty(), "{lang:?} has no word for {missing:?}: add them to crates/i18n/po/neoffice/");
    }
}

/// What the server says in the person's language (its ways to file, its questions), in French.
fn in_french(app: &mut MailApp) {
    let french = [("clients", "Un dossier par client"), ("activity", "Par activité"), ("streams", "Notifications et newsletters seulement")];
    if let Some(state) = app.nora.sorting.boxes.get_mut(BOX).and_then(|k| k.state.as_mut()) {
        for mode in &mut state.modes {
            if let Some((_, label)) = french.iter().find(|(code, _)| *code == mode.code) {
                mode.label = (*label).to_string();
            }
        }
    }
    if let Some(chosen) = app.nora.sorting.flow.as_mut().and_then(|flow| flow.chosen.as_mut()) {
        chosen.label = "Un dossier par client".into();
    }
    for ask in &mut app.nora.sorting.questions.list {
        if ask.name == "ASK-1" {
            ask.title = "Les avis de paiement de votre banque : j’en fais une tâche chaque fois ?".into();
            ask.context = "3 ce mois-ci".into();
            ask.choices = vec![("task".into(), "Oui, une tâche".into()), ("none".into(), "Non, rangez-les".into())];
        } else if ask.choices.first().is_some_and(|(code, _)| code == "client") {
            ask.title = format!("Garage Exemple {} : un client ou un fournisseur ?", ask.name.trim_start_matches("ASK-"));
            ask.choices = vec![("client".into(), "Un client".into()), ("supplier".into(), "Un fournisseur".into())];
        }
    }
}

/// Each screen of « Trier avec Nora » in French, light and dark, to PNG files in `NORA_SORTING_SHOTS`: the button and the
/// banner, her pane with the switch, the conversation, the result, the card.
#[test]
#[ignore = "draws to PNG files: NORA_SORTING_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored sorting_screens"]
fn sorting_screens_in_french() {
    let Some(dir) = std::env::var_os("NORA_SORTING_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        for screen in [
            "1-bouton-bandeau",
            "2-volet-interrupteur",
            "3-conversation",
            "4-resultat",
            "5-carte",
            "6-questions",
            "7-question-dossier",
            "8-premiere-lecture",
            "9-range-en-lisant",
            "10-sans-droit",
            "11-refus",
        ] {
            let mut app = match screen {
                "1-bouton-bandeau" => hosted(answers::state(true, "validated", true), true),
                "2-volet-interrupteur" => {
                    let mut app = hosted(answers::state(true, "validated", true), true);
                    app.run("ui.noraAssistant", json!({"open": true})).unwrap();
                    app
                }
                "3-conversation" => in_flow(Stage::Plan, true),
                "6-questions" => {
                    // Her questions: their number on her button of the ribbon, and the section at the head of her pane's home.
                    let mut app = hosted(answers::state(true, "validated", true), true);
                    app.nora.sorting.questions = Questions::said(read_asks(&questions(2)).0);
                    app.run("ui.noraAssistant", json!({"open": true})).unwrap();
                    app
                }
                "4-resultat" => in_flow(Stage::Done, true),
                "8-premiere-lecture" | "11-refus" => {
                    // The first time: she reads the mailbox, the way to file chosen meanwhile; and a refusal of the server's.
                    let mut app = in_flow(Stage::Choose, true);
                    let flow = app.nora.sorting.flow.as_mut().unwrap();
                    flow.chosen = None;
                    flow.plan = None;
                    flow.survey = Some(read_survey(&answers::first_reading(0, "running")));
                    app
                }
                "9-range-en-lisant" => as_she_reads(),
                "10-sans-droit" => unstarted(),
                "7-question-dossier" => {
                    // The result's « Nora vous demande »: a folder question still open, the answer to another with the
                    // correction it made and its « Annuler », and that correction in the journal.
                    let mut app = in_flow(Stage::Done, true);
                    let folder = json!({"asks": [{"name": "ASK-0007", "kind": "Family", "topic": "Folder",
                        "title": "« Avis n° 117 » : je les mets dans « Factures reçues » ou dans « Notifications » ?",
                        "context": "Ces 30 derniers jours, j’en ai rangé 4 dans « Factures reçues » et 3 dans « Notifications ».",
                        "choices": [{"code": "folder_1", "label": "Factures reçues"}, {"code": "folder_2", "label": "Notifications"},
                                    {"code": "inbox", "label": "Les laisser dans la boîte de réception"}], "box": BOX}], "retained": 0});
                    app.nora.sorting.questions = Questions::said(read_asks(&folder).0);
                    app.nora.sorting.questions.noted = Some(Noted {
                        words:
                            "C’est noté : les relevés de « Banque Exemple » vont dans « Factures reçues ». 5 messages de cette famille y sont passés."
                                .into(),
                        account: BOX.into(),
                        run: "NFR-C1".into(),
                    });
                    let mut journal = answers::journal("NFR-3");
                    journal["runs"].as_array_mut().unwrap().insert(
                        0,
                        json!({"run": "NFR-C1", "kind": "correction", "ask": "ASK-0005", "about": "Relevés de Banque Exemple", "status": "done",
                            "started_at": "2026-10-14 15:58:00", "filed": 5, "still_filed": 5,
                            "folders": [{"folder": "Factures reçues", "parent": "", "leaf": "Factures reçues", "count": 5}],
                            "kinds": {"invoices": 5}, "can_undo": true}),
                    );
                    app.nora.sorting.flow.as_mut().unwrap().journal = Some(read_journal(&journal));
                    app
                }
                _ => {
                    // From her pane's switch, as it opens.
                    let mut app = hosted(answers::state(false, "", true), false);
                    app.run("ui.noraAssistant", json!({"open": true})).unwrap();
                    app.nora.sorting.card =
                        Some(Card { account: BOX.into(), mode: "clients".into(), interval: 15, prepare: false, error: None, busy: false });
                    app
                }
            };
            in_french(&mut app);
            app.execute("ui.language", json!({"language": "fr"})).unwrap();
            if screen == "11-refus" {
                // As the window says it when the server answers so: in the person's language, the server's own sentence.
                let refused = super::super::NoraAnswer { id: 1, status: 417, body: thrown("Cette boîte n’a pas encore de secrétaire.") };
                app.nora.sorting.flow.as_mut().unwrap().error = super::super::answer::sorting(&refused).err();
            }
            let _ = app.run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
            let tall = matches!(screen, "3-conversation" | "4-resultat" | "7-question-dossier" | "9-range-en-lisant");
            let mut h = window(app, (1440.0, if tall { 1150.0 } else { 900.0 }));
            h.run_steps(8);
            let image = h.render().expect("drawn");
            image.save(dir.join(format!("{screen}-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
        }
    }
}

#[test]
fn a_guillemet_is_never_alone_at_the_start_or_the_end_of_a_line() {
    let said = tied("« Avis n° 117 » : je les mets dans « Factures reçues » ou dans « Notifications » ?");
    assert_eq!(said, "«\u{a0}Avis n° 117\u{a0}»\u{a0}: je les mets dans «\u{a0}Factures reçues\u{a0}» ou dans «\u{a0}Notifications\u{a0}»\u{a0}?");
    // Already tied, or English: as it is.
    assert_eq!(tied("«\u{a0}A\u{a0}»"), "«\u{a0}A\u{a0}»");
    assert_eq!(tied("Shall I file them? Yes: now."), "Shall I file them? Yes: now.");
    assert_eq!(kinds_said(&[("inbox".to_string(), 3)].into_iter().collect()), "3 messages back in the inbox");
}

#[test]
fn the_journal_says_a_correction_and_what_it_was_about() {
    let mut app = in_flow(Stage::Done, true);
    let correction = json!({"runs": [{"run": "NFR-C1", "kind": "correction", "ask": "ASK-1", "about": "Payment notices", "status": "done",
        "started_at": "2026-10-14 15:58:00", "filed": 4, "still_filed": 4, "folders": [{"folder": "Notifications", "parent": "", "leaf": "Notifications", "count": 4}],
        "kinds": {"notifications": 4}, "can_undo": true}], "learned": [], "may_set": true});
    app.nora.sorting.flow.as_mut().unwrap().journal = Some(read_journal(&correction));
    let h = window(app, (1440.0, 1150.0));
    assert!(contains(&h, "Correction: Payment notices · 4 messages filed"));
}

/// Frappe's refusal (`frappe.throw`, 417) as the server writes it: the person's sentence in `_server_messages`.
fn thrown(words: &str) -> String {
    let shown = json!({"message": words}).to_string();
    json!({"exc_type": "ValidationError", "_server_messages": json!([shown]).to_string()}).to_string()
}

/// The result of « Ranger maintenant » asked while Nora still reads the mailbox (her first time on it): what she filed so
/// far, `filed` of the run.
fn as_she_reads_filed(filed: u64) -> MailApp {
    let mut app = in_flow(Stage::Done, true);
    let flow = app.nora.sorting.flow.as_mut().unwrap();
    flow.filing = true;
    flow.survey = Some(read_survey(&answers::first_reading(40, "running")));
    let mut journal = answers::journal("NFR-3");
    journal["runs"][0]["filed"] = json!(filed);
    flow.journal = Some(read_journal(&journal));
    app
}

fn as_she_reads() -> MailApp {
    as_she_reads_filed(0)
}

/// The flow on a mailbox Nora has never read, opened by a person who may not start her on it.
fn unstarted() -> MailApp {
    let mut app = hosted(answers::unread_state(false), false);
    app.run("ui.noraAssistant", json!({"open": true})).unwrap();
    let mut flow = Flow::new(BOX.to_string(), false, crate::now_ms());
    flow.unstarted = true;
    flow.stage = Stage::Done;
    flow.journal = Some(read_journal(&json!({"runs": [], "learned": [], "may_set": false})));
    app.nora.sorting.flow = Some(flow);
    app
}

#[test]
fn the_first_time_she_says_she_reads_never_zero_messages_and_the_way_to_file_may_be_chosen() {
    let mut app = in_flow(Stage::Choose, true);
    app.nora.sorting.flow.as_mut().unwrap().survey = Some(read_survey(&answers::first_reading(0, "running")));
    let h = window(app, (1440.0, 900.0));
    assert!(contains(&h, "I'm reading your mailbox… 120 of 1248 messages"));
    assert!(!contains(&h, "I read the last") && !contains(&h, "I have not read any message"), "never « 0 messages »");
    assert!(has(&h, "One folder per client") && has(&h, "How do you want me to file?"), "the way to file may be chosen meanwhile");
    // Some read already: what she read, and that it will grow.
    let mut app = in_flow(Stage::Choose, true);
    let mut some = answers::first_reading(40, "running");
    some["progress"] = serde_json::Value::Null;
    app.nora.sorting.flow.as_mut().unwrap().survey = Some(read_survey(&some));
    let h = window(app, (1440.0, 900.0));
    assert!(contains(&h, "40 messages.") && contains(&h, "I'm still reading your mailbox: these numbers will grow."));
}

#[test]
fn file_now_while_she_reads_says_she_files_as_she_reads_never_that_nothing_needed_filing() {
    let h = window(as_she_reads(), (1440.0, 1150.0));
    assert!(contains(&h, "Nora files as she reads"));
    assert!(contains(&h, "I'm still reading your mailbox: I file the messages as I read them."));
    assert!(!contains(&h, "Nothing needed filing") && !contains(&h, "Nora filed your mailbox"));
    assert!(!contains(&h, "0 messages filed"), "a run that filed nothing is no line of the journal");
    assert!(contains(&h, "Next, shall I carry on by myself?"), "the rhythm is still asked");
    // What she filed already is said, and that she files the rest as she reads.
    let h = window(as_she_reads_filed(23), (1440.0, 1150.0));
    assert!(contains(&h, "It's filed.") && contains(&h, "I'm still reading your mailbox: I file the rest as I read it."));
    // A tidying that waits for her reading (`tidy.status` « waiting »), her reading done: the same.
    let mut app = as_she_reads();
    app.nora.sorting.flow.as_mut().unwrap().survey = Some(read_survey(&answers::survey()));
    let mut waits = answers::state(false, "validated", true);
    waits["tidy"]["status"] = json!("waiting");
    app.nora.sorting.boxes.insert(BOX.to_string(), Known::said(read_state(&waits), None));
    let h = window(app, (1440.0, 1150.0));
    assert!(contains(&h, "Nora files as she reads"));
    // Her reading done, nothing waiting: over.
    let mut app = as_she_reads();
    app.nora.sorting.flow.as_mut().unwrap().survey = Some(read_survey(&answers::survey()));
    let h = window(app, (1440.0, 1150.0));
    assert!(!contains(&h, "Nora files as she reads") && contains(&h, "Nothing needed filing"));
}

#[test]
fn the_person_who_may_not_start_her_is_told_so_and_a_refusal_is_the_servers_sentence() {
    let h = window(unstarted(), (1440.0, 900.0));
    assert!(contains(&h, "Only the person in charge of this mailbox can start the filing."));
    assert!(!contains(&h, "Nora is reading your mailbox") && !contains(&h, "Try again"));
    // A refusal: the server's sentence, and « Réessayer ».
    let mut app = in_flow(Stage::Choose, true);
    let refused = super::super::NoraAnswer { id: 1, status: 417, body: thrown("Cette boîte n’a pas encore de secrétaire.") };
    app.nora.sorting.flow.as_mut().unwrap().error = super::super::answer::sorting(&refused).err();
    let h = window(app, (1440.0, 900.0));
    assert!(contains(&h, "Nora couldn't do it: Cette boîte n’a pas encore de secrétaire.") && has(&h, "Try again"));
}
