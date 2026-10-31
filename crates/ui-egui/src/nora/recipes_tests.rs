// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the recipes of Nora's pane.
//! The recipes of Nora's pane: what is offered, that a click asks her what the person would have typed (about the
//! whole mailbox, never about the message that is open), and that the list goes once the conversation has begun.

use std::collections::BTreeSet;

use egui::vec2;
use egui_kittest::kittest::Queryable as _;
use serde_json::{Value, json};

use super::super::testing::*;
use super::groups;
use crate::MailApp;

type Window = egui_kittest::Harness<'static, MailApp>;

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
    // The sample has reminders due, and their window would lie over the panes.
    let _ = harness.state_mut().execute("ui.dismissReminder", json!({"all": true}));
    harness.run_steps(3);
    harness
}

#[test]
fn the_recipes_are_five_requests_about_the_whole_mailbox_that_write_nothing() {
    let groups = groups();
    assert_eq!(groups.iter().map(|g| g.recipes.len()).collect::<Vec<_>>(), [2, 3]);
    let mut titles = BTreeSet::new();
    let mut requests = BTreeSet::new();
    for group in &groups {
        assert!(!group.heading.is_empty());
        for recipe in &group.recipes {
            assert!(titles.insert(recipe.title.clone()) && requests.insert(recipe.request.clone()), "{}", recipe.title);
            // About the mailbox, not the message that is open, and short enough to be a request.
            assert!(recipe.request.contains("not the message that is open"), "{}", recipe.title);
            assert!(recipe.request.len() < 700, "{}", recipe.title);
            // None asks for anything to be sent, deleted, moved or written.
            let words = recipe.request.to_lowercase();
            for never in ["send it", "delete", "move it", "archive", "unsubscribe", "reply to", "write a"] {
                assert!(!words.contains(never), "{}: {never}", recipe.title);
            }
        }
    }
}

#[test]
fn a_click_asks_nora_the_recipe_and_the_list_goes_once_the_conversation_has_begun() {
    let (mut app, calls, _) = page(true);
    app.run("ui.noraAssistant", json!({"open": true})).unwrap();
    let mut harness = window(app, (1440.0, 900.0));
    let every: Vec<String> = groups().iter().flat_map(|g| g.recipes.iter().map(|r| r.title.clone())).collect();
    for title in &every {
        assert!(harness.query_by_label(title).is_some(), "{title} isn't offered");
    }
    assert_eq!(calls.count(), 0, "nothing asked yet");
    harness.get_by_label("Needs my reply").click();
    harness.run_steps(4);
    let wanted = groups().iter().flat_map(|g| g.recipes.iter()).find(|r| r.title == "Needs my reply").unwrap().request.clone();
    let start: Vec<Value> = calls
        .0
        .borrow()
        .iter()
        .filter(|(call, _)| call.method == "nora.api.chat.start_or_resume_thread")
        .map(|(call, _)| call.args.clone())
        .collect();
    assert_eq!(start.len(), 1);
    assert_eq!(start[0]["message"], wanted.as_str());
    // As any message of the pane: where the person is, and nothing of the mail.
    let context: Value = serde_json::from_str(start[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(context, json!({"surface": "editor", "app": "mail", "document": {"name": "NeoMail"}}));
    // The conversation has begun: the list is gone, and a second click can't ask twice.
    for title in &every {
        assert!(harness.query_by_label(title).is_none(), "{title} is still offered");
    }
}

#[test]
fn the_recipes_are_not_there_without_the_pane() {
    let (app, calls, _) = page(true);
    let harness = window(app, (1440.0, 900.0));
    for group in groups() {
        for recipe in group.recipes {
            assert!(harness.query_by_label(&recipe.title).is_none(), "{} without the pane", recipe.title);
        }
    }
    assert_eq!(calls.count(), 0);
}
