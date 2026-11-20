// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of `host_later`.
//! In a hosting page's tab, Send Later, Delay Delivery and follow-up reminders are neither drawn nor run: not the arrow beside
//! Send, not in the ribbon, not in Search Commands, their windows don't open, and the commands refuse whoever asks; the
//! program that keeps its mail offers them all, and Undo Send stays everywhere.

use egui_kittest::Harness;
use mailcraft_engine::Session;
use mailcraft_engine::cmd::kept_here::{LATER, NO_LATER_HERE, NO_SUBSCRIPTIONS_HERE, SUBSCRIBED};
use serde_json::json;

use crate::ribbon::Menu;
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// The sample mailbox with an account to send from, in the program that keeps its mail or (`hosted`) in a hosting page's tab;
/// the account's id.
fn app(hosted: bool) -> (MailApp, u64) {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app.session.driver.set_auto_sync(false);
    let account = app
        .execute("account.add", json!({"email": "avery@mail.example", "incoming": {"host": "imap.mail.example"}, "outgoing": {"host": "smtp.mail.example", "port": 587, "security": "starttls"}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    app.session.keeps_local = !hosted;
    app.services.jmap_host_credentials = hosted;
    (app, account)
}

fn window(app: MailApp) -> Harness<'static, MailApp> {
    let mut h = Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(1280.0, 700.0)).build_ui_state(
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

fn write(app: &mut MailApp, account: u64) {
    app.run("mail.new", json!({"to": "maya@lumen.example", "subject": "Quarterly plan", "body": "Hi", "account": account})).unwrap();
}

#[test]
fn the_commands_refuse_whoever_asks_in_a_hosting_pages_tab() {
    let (mut app, account) = app(true);
    write(&mut app, account);
    let message = app.session.mailbox.messages.keys().next().copied().unwrap();
    for id in LATER {
        let params = json!({"at": NOW + 3600, "days": 3, "ids": [message], "id": message, "minutes": 10});
        let refused = app.session.run(id, &params).unwrap_err().to_string();
        assert!(refused.contains(NO_LATER_HERE), "{id}: {refused}");
        assert_eq!(app.session.registry.get(id).and_then(|spec| (spec.enabled)(&app.session)), Some(NO_LATER_HERE), "{id}");
    }
    assert!(app.session.mailbox.messages.values().all(|m| m.send_at.is_none() && m.follow_up.is_none()), "nothing waits, nothing rings");
    // Undo Send stays: its wait is seconds, in the open tab.
    app.session.run("sync.waitBeforeSending", &json!({"seconds": 10})).unwrap();
    assert!(app.session.registry.get("compose.undoSend").is_some_and(|spec| (spec.enabled)(&app.session) != Some(NO_LATER_HERE)));
}

#[test]
fn the_program_that_keeps_its_mail_runs_them() {
    let (mut app, account) = app(false);
    write(&mut app, account);
    app.session.run("compose.followUp", &json!({"days": 3})).unwrap();
    let id = app.session.run("compose.sendLater", &json!({"at": NOW + 3600})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(app.session.mailbox.message(id).and_then(|m| m.send_at), Some(NOW + 3600));
}

#[test]
fn search_commands_the_ribbon_and_their_windows_leave_them_out_in_a_hosting_pages_tab() {
    for hosted in [false, true] {
        let (mut app, account) = app(hosted);
        write(&mut app, account);
        let listed: Vec<String> = crate::dialogs::all_commands(&app).into_iter().map(|c| c.0).collect();
        for id in LATER.iter().filter(|id| ["compose.sendLater", "compose.delay", "compose.followUp", "mail.reminder"].contains(id)) {
            assert_eq!(listed.iter().any(|l| l == id), !hosted, "{id} listed (hosted: {hosted})");
        }
        assert!(listed.iter().any(|l| l == "compose.send"), "Send is listed everywhere");
        let tab = crate::ribbon::describe_tab(&app, "Message");
        let delay = tab.items().any(|item| item.command.as_ref().is_some_and(|(id, _)| *id == "compose.delay"));
        let follow_up = tab.items().any(|item| item.menu == Some(Menu::ComposeFollowUp));
        assert_eq!((delay, follow_up), (!hosted, !hosted), "Delay Delivery and Follow Up of the message (hosted: {hosted})");
        // (Add Reminder… is of the selected messages.)
        let message = app.session.mailbox.messages.values().find(|m| !m.draft).map(|m| m.id).unwrap();
        app.session.sel.messages = vec![message];
        for name in ["sendLater", "delay", "addReminder", "followUpDate"] {
            assert_eq!(crate::dialogs::Dialog::open(name, &app, &json!({})).is_some(), !hosted, "{name} (hosted: {hosted})");
        }
    }
}

#[test]
fn no_arrow_beside_send_in_a_hosting_pages_tab() {
    for hosted in [false, true] {
        let (mut app, account) = app(hosted);
        write(&mut app, account);
        let mut h = window(app);
        h.run_steps(3);
        assert_eq!(h.state().hit.rect("sendOptions", 0).is_some(), !hosted, "the arrow beside Send (hosted: {hosted})");
    }
}

#[test]
fn search_commands_leaves_out_what_acts_on_a_subscribed_calendar_where_none_can_be() {
    for hosted in [false, true] {
        let (app, _) = app(hosted);
        let listed: Vec<String> = crate::dialogs::all_commands(&app).into_iter().map(|c| c.0).collect();
        for id in crate::host_later::SUBSCRIPTION_LEFTOVERS {
            assert_eq!(listed.iter().any(|l| l == id), !hosted, "{id} listed (hosted: {hosted})");
        }
        // The Reminders window stays: the reminders it rings are the events', which the calendars' server keeps.
        assert!(listed.iter().any(|l| l == "view.reminders"), "hosted: {hosted}");
    }
}

#[test]
fn no_calendar_is_subscribed_to_on_the_web_in_a_hosting_pages_tab() {
    for hosted in [false, true] {
        let (mut app, _) = app(hosted);
        let subscribed = app.session.run("calendar.subscribe", &json!({"address": "https://cal.example.org/team.ics"}));
        assert_eq!(subscribed.is_ok(), !hosted, "subscribed (hosted: {hosted})");
        if hosted {
            assert!(subscribed.unwrap_err().to_string().contains(NO_SUBSCRIPTIONS_HERE));
            for id in SUBSCRIBED {
                assert_eq!(app.session.registry.get(id).and_then(|spec| (spec.enabled)(&app.session)), Some(NO_SUBSCRIPTIONS_HERE), "{id}");
            }
        }
        let listed: Vec<String> = crate::dialogs::all_commands(&app).into_iter().map(|c| c.0).collect();
        assert_eq!(listed.iter().any(|l| l == "calendar.subscribe"), !hosted, "listed (hosted: {hosted})");
        assert_eq!(crate::dialogs::Dialog::open("subscribe", &app, &json!({})).is_some(), !hosted, "its window (hosted: {hosted})");
    }
}
