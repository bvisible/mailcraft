// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what a hosting page's tab says of where things are kept.

//! A hosting page's tab keeps nothing on a computer, and has no Outlook account: nothing it draws says so, on any of the
//! modules, the pages of File, the forms and the windows about calendars, people, notes, rules and categories. Everything that
//! is drawn is read (the painted text, not the accessible names alone).

#![allow(clippy::unwrap_used)]

use egui_kittest::Harness;
use mailcraft_engine::Session;
use mailcraft_engine::cmd::remote::{apply_calendars, apply_events};
use mailcraft_model::remote::{CalendarsDownload, EventsDownload, RemoteCalendar, RemoteEvent};
use mailcraft_model::{Mailbox, Rights};
use serde_json::{Value, json};

use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const ME: &str = "noor@lumen.example";
/// What is never drawn there; but for the zone of the clock, which is the computer's, and the microphone, which is in it.
const NEVER: [&str; 4] = ["computer", "Computer", "device", "Outlook"];
/// (Outlook.com is one of the free mail services whose senders keep their initials.)
const BUT: [&str; 3] = ["This computer's, wherever it goes", "No microphone found on this computer.", "Gmail, Outlook.com, Bluewin"];

/// A hosting page's tab with Noor's mailbox: her calendar and a colleague's, an appointment, a meeting she organizes, a contact
/// of her own and a rule.
fn app() -> (MailApp, u64) {
    let mut session = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
    let added = session.run(
        "account.add",
        &json!({"email": ME, "displayName": "Noor", "kind": "jmap", "jmap": {"session": "https://erp.example.org/jmap/session", "bearer": true}}),
    );
    let account = added.unwrap()["id"].as_u64().unwrap();
    session.keeps_local = false;
    let calendar = |id: &str, name: &str, main: bool, rights: Rights| RemoteCalendar {
        id: id.into(),
        name: name.into(),
        main,
        rights,
        plain: true,
        ..Default::default()
    };
    let calendars = vec![calendar("Cnoor", "Noor", true, Rights::Edit), calendar("Cada", "Ada", false, Rights::Read)];
    apply_calendars(&mut session, &CalendarsDownload { account, email: ME.into(), complete: true, calendars, refused: Vec::new() }).unwrap();
    let event = |id: &str, title: &str, hour: i64| RemoteEvent {
        id: id.into(),
        uid: format!("{id}@x"),
        title: title.into(),
        start: NOW + hour * 3_600,
        end: NOW + (hour + 1) * 3_600,
        ..Default::default()
    };
    let events = vec![event("E1", "Dentist", 1), event("E2", "Budget", 3)];
    let download = EventsDownload { account, email: ME.into(), from: NOW - 86_400, to: NOW + 86_400, complete: true, events, ..Default::default() };
    apply_events(&mut session, &download).unwrap();
    let mut app = MailApp::new(
        session,
        Services { jmap_host_credentials: true, accounts_home: Some("https://erp.example.org/app".into()), ..Services::default() },
    );
    app.autosave = false;
    app.session.driver.set_auto_sync(false);
    app.execute(
        "rules.add",
        json!({"name": "Plans", "account": account, "conditions": [{"type": "fromContains", "value": "zoe"}], "actions": [{"type": "flag"}]}),
    )
    .unwrap();
    (app, account)
}

/// (Tall: a page of File is painted whole, not its first screen.)
fn window(app: MailApp) -> Harness<'static, MailApp> {
    let mut h = Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(1400.0, 6000.0)).build_ui_state(
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

/// Every text painted by `shape`.
fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
        _ => {}
    }
}

/// The text of what the window shows now.
fn drawn(h: &Harness<'static, MailApp>) -> Vec<String> {
    let mut out = Vec::new();
    for clipped in &h.output().shapes {
        texts(&clipped.shape, &mut out);
    }
    assert!(out.len() > 5, "something is drawn");
    out
}

/// What of the window is never to be drawn there, with where it was (`at`).
fn wrong(h: &Harness<'static, MailApp>, at: &str) -> Vec<String> {
    let said = |text: &String| BUT.iter().fold(text.clone(), |text, but| text.replace(but, ""));
    drawn(h).iter().filter(|text| NEVER.iter().any(|word| said(text).contains(word))).map(|text| format!("{at}: {text}")).collect()
}

fn run(h: &mut Harness<'static, MailApp>, command: &str, params: Value) {
    let _ = h.state_mut().execute(command, params);
    h.run_steps(4);
}

#[test]
fn nothing_drawn_in_a_hosting_pages_tab_names_a_computer_or_outlook() {
    let (mut a, account) = app();
    a.execute("people.new", json!({"first": "Zoe", "email": "zoe@example.org"})).ok();
    let mut h = window(a);
    let mut found: Vec<String> = Vec::new();
    for module in ["view.mail", "view.calendar", "view.people", "view.notes"] {
        run(&mut h, module, json!({}));
        found.extend(wrong(&h, module));
    }
    for page in ["info", "open", "options", "signatures", "about"] {
        run(&mut h, "ui.backstage", json!({"value": true, "page": page}));
        found.extend(wrong(&h, page));
    }
    run(&mut h, "ui.backstage", json!({"value": false}));
    let event = h.state().session.mailbox.events.values().find(|e| e.title == "Dentist").unwrap().id;
    run(&mut h, "view.calendar", json!({}));
    h.state_mut().session.sel.event = Some(event);
    h.run_steps(3);
    found.extend(wrong(&h, "an event selected"));
    for (dialog, params) in [
        ("event", json!({})),
        ("meeting", json!({})),
        ("editEvent", json!({"id": event})),
        ("contact", json!({})),
        ("rules", json!({})),
        ("categories", json!({})),
        ("account", json!({"id": account})),
    ] {
        let mut named = params.clone();
        named["name"] = json!(dialog);
        run(&mut h, "ui.dialog", named);
        found.extend(wrong(&h, dialog));
        h.state_mut().dialog = None;
        h.run_steps(2);
    }
    assert!(found.is_empty(), "{found:#?}");
}

/// The question before a new meeting with people invited keeps nothing on a computer, and an event of the calendar that isn't
/// on its server says so.
#[test]
fn a_meeting_nobody_is_invited_to_yet_stays_in_its_calendar() {
    let (mut a, account) = app();
    let params = json!({"title": "Kickoff", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "attendees": "bob@example.org"});
    a.run("calendar.newMeeting", params.clone()).unwrap();
    let mut h = window(a);
    let asked = drawn(&h);
    assert_eq!(wrong(&h, "the question"), Vec::<String>::new());
    assert!(asked.iter().any(|text| text.contains("Send the invitation")), "{asked:?}");
    h.state_mut().dialog = None;
    let mut unsent = params;
    unsent["send"] = json!(false);
    let id = h.state_mut().execute("calendar.newMeeting", unsent).unwrap()["id"].as_u64().unwrap();
    run(&mut h, "view.calendar", json!({}));
    assert_eq!(wrong(&h, "an unsent meeting"), Vec::<String>::new());
    let event = h.state().session.mailbox.event(id).unwrap().clone();
    assert_eq!(event.origin.account, account);
    // Changed: the question is the invitation's, and may be put off again; deleted: nobody is told.
    h.state_mut().run("calendar.edit", json!({"id": id, "location": "Room 2"})).unwrap();
    h.run_steps(3);
    let asked = drawn(&h);
    assert!(asked.iter().any(|text| text.contains("Send the invitation")) && asked.iter().any(|text| text.contains("Don't Send")), "{asked:?}");
    h.state_mut().dialog = None;
    // Its form sends the invitation (nothing was sent that an update would be of), and deletes rather than cancels.
    let opened = h.state_mut().execute("ui.openWindow", json!({"event": id})).unwrap()["window"].as_u64().unwrap();
    h.run_steps(4);
    let form = drawn(&h);
    assert!(!form.iter().any(|text| text.contains("Send Update") || text.contains("Cancel Meeting")), "{form:?}");
    assert!(form.iter().any(|text| text == "Send"), "{form:?}");
    h.state_mut().execute("ui.closeWindow", json!({"window": opened})).unwrap();
    h.run_steps(2);
    // Deleted: asked about once, as any event of a server's calendar; nobody is told, as nobody was invited.
    h.state_mut().run("calendar.delete", json!({"id": id})).unwrap();
    h.run_steps(3);
    assert!(drawn(&h).iter().any(|text| text == "Delete This Event?"), "{:?}", drawn(&h));
    h.state_mut().dialog = None;
    assert!(h.state_mut().execute("calendar.delete", json!({"id": id})).is_ok(), "nobody was invited: no cancellation");
}

/// A rule is said in the interface's language, and whole: on as many lines as it takes.
#[test]
fn a_rule_is_said_whole_and_in_the_interfaces_language() {
    let _held = crate::i18n::RestoreLanguage::hold();
    let (mut a, account) = app();
    let long = "a word long enough that three of them do not fit on a line of the window";
    let rule = json!({
        "name": "Café", "account": account,
        "conditions": [{"type": "fromContains", "value": long}, {"type": "subjectContains", "value": long}],
        "exceptions": [{"type": "bodyContains", "value": long}],
        "actions": [{"type": "flag"}],
    });
    let id = a.execute("rules.add", rule).unwrap()["id"].as_u64().unwrap();
    a.execute("ui.language", json!({"language": "fr"})).unwrap();
    crate::i18n::set_current(crate::i18n::Lang::French);
    let rule = a.session.mailbox.rules.iter().find(|r| r.id == id).unwrap().clone();
    let said = crate::forms::rules::describe(&a.session.mailbox, &rule);
    for english in ["When", " from ", " and ", "subject contains", "except", "body", "flag it"] {
        assert!(!said.contains(english), "{english}: {said}");
    }
    let mut h = window(a);
    run(&mut h, "ui.dialog", json!({"name": "rules"}));
    // (Painted as one text, uncut: no ellipsis, and the words of the exception, which come last, are there.)
    let painted: Vec<String> = drawn(&h).into_iter().filter(|text| text.matches(long).count() >= 3).collect();
    assert_eq!(painted.len(), 1, "{:?}", drawn(&h).iter().filter(|text| text.contains("a word long")).collect::<Vec<_>>());
}

/// File › Info: each button's word is on one line, nothing says that the mail is kept nowhere, rules aren't said to act on mail
/// as it arrives, and no mailbox is removed (its site would hand it over again).
#[test]
fn file_info_says_what_is_true_of_a_hosting_pages_tab() {
    let _held = crate::i18n::RestoreLanguage::hold();
    let (mut a, account) = app();
    assert!(a.execute("account.remove", json!({"id": account})).is_err(), "refused whoever asks");
    crate::i18n::set_current(crate::i18n::Lang::French);
    a.execute("ui.language", json!({"language": "fr"})).unwrap();
    let mut h = window(a);
    run(&mut h, "ui.backstage", json!({"value": true, "page": "info"}));
    let said = drawn(&h);
    assert!(said.iter().any(|text| text == "Paramètres") && !said.iter().any(|text| text == "Supprimer"), "{said:?}");
    let mut lines = Vec::new();
    for clipped in &h.output().shapes {
        rows(&clipped.shape, "Paramètres", &mut lines);
    }
    assert_eq!(lines, [1], "on one line");
    assert!(!said.iter().any(|text| text.contains("nulle part") || text.contains("à son arrivée")), "{said:?}");
}

/// On how many lines `word` is painted, each time it is.
fn rows(shape: &egui::Shape, word: &str, out: &mut Vec<usize>) {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == word => out.push(text.galley.rows.len()),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| rows(shape, word, out)),
        _ => {}
    }
}

/// What is of an Outlook account, or of a calendar of the mailbox's own, is neither found by Search Commands nor run; a favorite
/// of the person's own is.
#[test]
fn nothing_of_outlooks_is_found_or_run_in_a_hosting_pages_tab() {
    let (mut a, _) = app();
    for id in ["view.favorites", "sync.applyFavorites", "calendar.addCalendar", "calendar.moveToAccount", "account.remove"] {
        assert!(crate::host_later::hidden(&a, id), "{id}");
        let refused = a.execute(id, json!({"name": "Mine", "own": true, "download": {}, "id": 1})).unwrap_err();
        assert!(refused.contains("This page"), "{id}: {refused}");
    }
    assert!(!crate::host_later::hidden(&a, "folder.favorite"));
    let inbox = a.session.view.folder;
    a.execute("folder.favorite", json!({"id": inbox})).unwrap();
    a.session.keeps_local = true;
    assert!(!crate::host_later::hidden(&a, "view.favorites"));
}
