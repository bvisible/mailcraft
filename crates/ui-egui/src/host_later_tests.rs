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
        for id in crate::host_later::WANT_PARAMS {
            assert!(app.session.registry.get(id).is_some(), "{id} is a command");
            assert_eq!(listed.iter().any(|l| l == id), !hosted, "{id} listed (hosted: {hosted})");
        }
        // A hosting page's own commands, nowhere.
        for id in crate::host_later::HOST_PLUMBING {
            assert!(app.session.registry.get(id).is_some(), "{id} is a command");
            assert!(!listed.iter().any(|l| l == id), "{id} listed (hosted: {hosted})");
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

/// A flag's due date is not kept by a JMAP server: in a hosting page's tab, Follow Up offers the flag without a date only, a date
/// asked for is refused whoever asks, and a flag set is a flag.
#[test]
fn follow_up_offers_no_dated_flag_in_a_hosting_pages_tab() {
    use egui_kittest::kittest::Queryable;
    use mailcraft_engine::cmd::kept_here::NO_DUE_HERE;
    use mailcraft_model::FlagStatus;
    for hosted in [false, true] {
        let (mut app, _) = app(hosted);
        let message = app.session.mailbox.messages.values().find(|m| !m.draft && m.flag == FlagStatus::None).map(|m| m.id).unwrap();
        app.session.sel.messages = vec![message];
        let dated = app.session.run("mail.flag", &json!({"ids": [message], "due": "2026-10-16"}));
        assert_eq!(dated.is_ok(), !hosted, "a dated flag (hosted: {hosted})");
        if hosted {
            assert!(dated.unwrap_err().to_string().contains(NO_DUE_HERE));
            assert_eq!(app.session.mailbox.message(message).unwrap().flag, FlagStatus::None, "nothing set by a refusal");
        }
        // A flag without a date: everywhere, and it stays one.
        app.session.run("mail.flag", &json!({"ids": [message]})).unwrap();
        assert_eq!(app.session.mailbox.message(message).unwrap().flag, FlagStatus::Flagged);
        // The menu of Follow Up.
        let mut h = Harness::builder()
            .with_accessibility_check(false)
            .allow_missing_glyphs()
            .build_ui_state(|ui, app: &mut MailApp| crate::ribbon::menu_contents(ui, app, Menu::FollowUp), app);
        h.run_steps(2);
        for label in ["Today", "Tomorrow", "This Week", "Next Week"] {
            assert_eq!(h.query_by_label_contains(label).is_some(), !hosted, "{label} (hosted: {hosted})");
        }
        for label in ["No Date", "Mark Complete", "Clear Flag"] {
            assert!(h.query_by_label_contains(label).is_some(), "{label} (hosted: {hosted})");
        }
    }
}

/// The order of folders and Favorites (upstream's #1514) is kept on this computer alone: in a hosting page's tab a folder's row isn't
/// dragged up or down, its menu has no Move Up and Move Down, and neither command is in Search Commands. Nor is Shade Rows (#1657),
/// on the View tab or there; the rows of its list wear no band whatever the setting says. And Review shows no Spelling (#954) where
/// the host has no dictionary.
#[test]
fn folders_keep_their_places_and_the_list_its_look_in_a_hosting_pages_tab() {
    use egui_kittest::kittest::Queryable;
    use mailcraft_engine::cmd::kept_here::{ORDERED, SHADED};
    for hosted in [false, true] {
        let (mut app, _) = app(hosted);
        // (At the top level of the first account, in sight, Favorites folded away.)
        let account = app.session.mailbox.accounts[0].id;
        let folder =
            app.session.run("folder.new", &json!({"name": "Zz Project", "account": account, "parent": null})).unwrap()["id"].as_u64().unwrap();
        app.session.run("folder.new", &json!({"name": "Zz Quote", "account": account, "parent": null})).unwrap();
        app.execute("ui.toggleFolder", json!({"id": 0})).unwrap();
        let listed: Vec<String> = crate::dialogs::all_commands(&app).into_iter().map(|c| c.0).collect();
        for id in ORDERED.iter().chain([&SHADED]) {
            assert_eq!(listed.iter().any(|l| l == id), !hosted, "{id} listed (hosted: {hosted})");
        }
        let shade = crate::ribbon::describe_tab(&app, "View").items().any(|item| item.command.as_ref().is_some_and(|(id, _)| *id == SHADED));
        assert_eq!(shade, !hosted, "Shade Rows on the View tab (hosted: {hosted})");
        let spelling = crate::ribbon::describe_tab(&app, "Review").items().any(|item| item.menu == Some(Menu::Spelling));
        assert_eq!(spelling, !hosted, "Spelling on the Review tab (hosted: {hosted})");
        app.session.prefs.shade_rows = true;
        let mut h = window(app);
        h.run_steps(3);
        let shaded = h.state().session.mailbox.messages.keys().any(|id| h.state().hit.rect("messageShade", *id).is_some());
        assert_eq!(shaded, !hosted, "a shaded row (hosted: {hosted})");
        let at = h.state().hit.rect("folder", folder).unwrap().center();
        // A drag of the row: carried only where it can be put elsewhere.
        let press = |pressed: bool, pos: egui::Pos2| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.run_steps(45);
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(1);
        h.event(press(true, at));
        h.run_steps(1);
        for step in 1..=4 {
            h.event(egui::Event::PointerMoved(at + egui::vec2(0.0, 8.0 * step as f32)));
            h.run_steps(1);
        }
        assert_eq!(egui::DragAndDrop::has_payload_of_type::<crate::nav::FolderDrag>(&h.ctx), !hosted, "carried (hosted: {hosted})");
        let away = at + egui::vec2(500.0, 0.0);
        h.event(egui::Event::PointerMoved(away));
        h.event(press(false, away));
        h.run_steps(3);
        for pressed in [true, false] {
            h.event(egui::Event::PointerMoved(at));
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.run_steps(2);
        }
        h.run_steps(3);
        assert!(h.query_all_by_label_contains("Mark All as Read").next().is_some(), "the folder's menu is open (hosted: {hosted})");
        assert_eq!(h.query_by_label("Move Up").is_some(), !hosted, "Move Up (hosted: {hosted})");
    }
}

/// A rule's builder in a hosting page's tab offers no desktop alert, no due day for a flag and no copy of a JMAP account's mail, and
/// says that the rule runs when it is run (its JMAP runs put no new mail through the rules).
#[test]
fn the_rule_builder_offers_what_runs_in_a_hosting_pages_tab() {
    use egui_kittest::kittest::Queryable;
    for hosted in [false, true] {
        let (mut app, _) = app(hosted);
        let inbox = app.session.view.folder;
        let m = app.session.mailbox.messages_in(inbox).find(|m| !m.from.email.is_empty()).map(|m| m.id).unwrap();
        let account = app.session.mailbox.message(m).unwrap().account;
        let to = app.session.mailbox.folder_of_kind(account, mailcraft_model::FolderKind::Archive).unwrap();
        app.session.sel.messages = vec![m];
        app.execute("ui.dialog", json!({"name": "rules", "folder": to})).unwrap();
        let mut h = window(app);
        h.run_steps(4);
        assert_eq!(h.query_by_label_contains("Show a desktop alert").is_some(), !hosted, "the alert (hosted: {hosted})");
        assert_eq!(h.query_by_label_contains("not as mail arrives").is_some(), hosted, "what is said (hosted: {hosted})");
        assert!(!hosted || h.query_by_label_contains("while MailCraft is open").is_none(), "nothing else said of where it runs");
    }
}
