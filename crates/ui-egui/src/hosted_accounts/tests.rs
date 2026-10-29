// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): File › Info in a tab of a hosting page (hosted_accounts.rs).

#![allow(clippy::unwrap_used)]

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use mailcraft_engine::Session;
use mailcraft_model::Mailbox;
use serde_json::json;

use crate::{MailApp, Services};

const PAGE: &str = "https://erp.example.org/app/webmail-account/noor%40lumen.example";
const HOME: &str = "https://erp.example.org/app/webmail-account";
const LINK: &str = "Manage mailboxes on the site";
const NOTE: &str = "These mailboxes come from the site that opened them";

/// File › Info of a window with one JMAP mailbox (its address written with a capital), whose session keeps it or not,
/// given where the page's site sets it up (`page`) and lists the mailboxes (`home`).
fn info(keeps_local: bool, page: Option<&str>, home: Option<&str>) -> Harness<'static, MailApp> {
    let mut session = Session::new(Mailbox::with_local_account("", ""));
    session.keeps_local = keeps_local;
    let services = Services {
        account_settings: page.map(|p| [("noor@lumen.example".to_string(), p.to_string())].into()).unwrap_or_default(),
        accounts_home: home.map(str::to_string),
        ..Services::default()
    };
    let mut app = MailApp::new(session, services);
    app.autosave = false;
    let jmap = json!({
        "email": "Noor@lumen.example", "displayName": "Noor", "kind": "jmap",
        "jmap": {"session": "https://erp.example.org/jmap/session", "bearer": true},
    });
    app.execute("account.add", jmap).unwrap();
    let mut harness = Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.state_mut().execute("ui.dismissReminder", json!({"all": true})).unwrap();
    harness.state_mut().execute("ui.backstage", json!({"value": true, "page": "info"})).unwrap();
    harness.run_steps(3);
    harness
}

/// The addresses the window asked to open over the next steps.
fn opened(harness: &mut Harness<'static, MailApp>) -> Vec<egui::OpenUrl> {
    let mut asked = Vec::new();
    for _ in 0..3 {
        harness.step();
        for command in &harness.output().platform_output.commands {
            if let egui::OutputCommand::OpenUrl(open) = command {
                asked.push(open.clone());
            }
        }
    }
    asked
}

#[test]
fn a_hosting_page_s_mailbox_opens_its_settings_on_the_site_in_a_new_tab() {
    let mut harness = info(false, Some(PAGE), Some(HOME));
    harness.get_by_label("Settings").click();
    let asked = opened(&mut harness);
    assert!(matches!(asked.as_slice(), [open] if open.url == PAGE && open.new_tab), "{asked:?}");
    assert!(harness.state().dialog.is_none(), "the window's own account settings stay closed");
}

#[test]
fn file_info_says_where_the_site_lists_the_mailboxes_and_links_there() {
    let mut harness = info(false, Some(PAGE), Some(HOME));
    assert!(harness.query_by_label_contains(NOTE).is_some());
    harness.get_by_label(LINK).click();
    let asked = opened(&mut harness);
    assert!(matches!(asked.as_slice(), [open] if open.url == HOME && open.new_tab), "{asked:?}");
}

#[test]
fn without_a_page_for_it_settings_open_the_window_s_own() {
    let mut harness = info(false, None, None);
    assert!(harness.query_by_label_contains(NOTE).is_none() && harness.query_by_label(LINK).is_none());
    harness.get_by_label("Settings").click();
    assert!(opened(&mut harness).is_empty());
    assert!(harness.state().dialog.is_some(), "the window's own account settings");
}

#[test]
fn a_window_that_keeps_its_mailboxes_keeps_its_own_settings_and_add_account() {
    let mut harness = info(true, Some(PAGE), Some(HOME));
    assert!(harness.query_by_label_contains(NOTE).is_none() && harness.query_by_label(LINK).is_none());
    harness.get_by_label("Settings").click();
    assert!(opened(&mut harness).is_empty());
    assert!(harness.state().dialog.is_some(), "the window's own account settings");
}
