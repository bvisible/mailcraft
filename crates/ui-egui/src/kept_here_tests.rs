// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A hosting page's tab, which keeps nothing of its own between visits, in the window: no Tasks, greyed controls that
//! say why where nothing would be kept, and a question before what a server keeps is deleted. Invented people at
//! `example.org`.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use mailcraft_engine::cmd::{kept_here, remote};
use mailcraft_engine::{Module, Session};
use mailcraft_model::book::{BooksDownload, RemoteBook};
use mailcraft_model::remote::{CalendarsDownload, ContactsDownload, EventsDownload, RemoteCalendar, RemoteContact, RemoteEvent, RemoteEventKind};
use mailcraft_model::{Address, Attendee, Freq, Id, Labeled, Recurrence};
use mailcraft_time::{Date, DateTime, Timestamp};
use serde_json::json;

use crate::dialogs::Dialog;
use crate::{MailApp, Services};

const ME: &str = "ada@example.org";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

fn at(day: u8, hour: u8) -> Timestamp {
    DateTime::new(Date::new(2026, 10, day).unwrap_or_default(), hour, 0, 0).map_or(0, |at| at.to_timestamp(0))
}

/// The sample mailbox with a JMAP account whose server keeps its calendar and its team's address book: in the calendar,
/// Ada's appointment, a meeting she organizes with Bo invited, and her weekly series; in the book, Bob. In a tab that
/// keeps nothing of its own when `hosted`. The window, and the account's id.
pub(crate) fn window(hosted: bool) -> (MailApp, Id) {
    let mut s = Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0);
    let added = s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap/session"}}));
    let account = added.unwrap()["id"].as_u64().unwrap();
    let calendars = vec![RemoteCalendar { id: "C1".into(), name: "Ada".into(), main: true, plain: true, ..Default::default() }];
    remote::apply_calendars(&mut s, &CalendarsDownload { account, email: ME.into(), complete: true, calendars }).unwrap();
    let mine = |id: &str, title: &str, day: u8| RemoteEvent {
        id: id.into(),
        title: title.into(),
        start: at(day, 9),
        end: at(day, 10),
        is_organizer: Some(true),
        ..Default::default()
    };
    let meeting = RemoteEvent {
        organizer: Some(Address::new("Ada", ME)),
        attendees: vec![Attendee { address: Address::new("Bo", "bo@example.org"), ..Default::default() }],
        ..mine("V2", "Planning", 20)
    };
    let series = RemoteEvent {
        kind: RemoteEventKind::SeriesMaster,
        recurrence: Some(Recurrence { freq: Freq::Weekly, interval: 1, ..Default::default() }),
        ..mine("V3", "Weekly review", 13)
    };
    let events = vec![mine("V1", "Budget review", 19), meeting, series];
    // (Not all of the stretch: a whole one names each day of a series that it holds.)
    let download = EventsDownload { account, email: ME.into(), from: at(1, 0), to: at(31, 0), events, ..Default::default() };
    remote::apply_events(&mut s, &download).unwrap();
    let book = RemoteBook { id: "neoffice".into(), name: "Neoffice".into(), main: true, may_remove: true, plain: true, ..Default::default() };
    mailcraft_engine::cmd::remote_contacts::apply_books(&mut s, &BooksDownload { account, email: ME.into(), complete: true, books: vec![book] })
        .unwrap();
    let bob = RemoteContact {
        id: "c-bob".into(),
        book: "neoffice".into(),
        first: "Bob".into(),
        last: "Stone".into(),
        emails: vec![Labeled::new("", "bob@example.org")],
        ..Default::default()
    };
    remote::apply_contacts(&mut s, &ContactsDownload { account, email: ME.into(), complete: true, contacts: vec![bob] }).unwrap();
    s.forget_history();
    s.keeps_local = !hosted;
    let mut app = MailApp::new(s, Services::default());
    app.autosave = false;
    (app, account)
}

fn event(app: &MailApp, title: &str) -> Id {
    app.session.mailbox.events.values().find(|e| e.title == title).map(|e| e.id).unwrap()
}

fn drawn(app: MailApp) -> Harness<'static, MailApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 860.0)).build_ui_state(
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

fn click(h: &mut Harness<'static, MailApp>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
}

/// The question a command run by hand asks, as `(title, text, the button that goes ahead)`; `None` when it asks none
/// (and then it ran).
fn asked(app: &mut MailApp, id: &str, params: serde_json::Value) -> Option<(String, String, String)> {
    let answer = app.run(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
    if answer["pending"] != "confirm" {
        return None;
    }
    match &app.dialog {
        Some(Dialog::Confirm { ask, .. }) => Some((ask.title.clone(), ask.text.clone(), ask.action.clone())),
        _ => None,
    }
}

#[test]
fn tasks_are_not_offered_in_a_tab_that_keeps_nothing() {
    let (app, _) = window(true);
    let h = drawn(app);
    let app = h.state();
    // Not in the module bar, where the others are.
    for module in [Module::Mail, Module::Calendar, Module::People] {
        assert!(app.hit.rect("module", module as u64).is_some(), "{module:?}");
    }
    assert!(app.hit.rect("module", Module::Tasks as u64).is_none());
    // Its command and its key do nothing, New Task and Add to Tasks are greyed, each saying why.
    assert!(!app.enabled("view.tasks") && !app.enabled("tasks.new"));
    assert_eq!(crate::keys::resolve(app, "Mod+4"), None);
    let reason = |id: &str| app.session.registry.get(id).and_then(|spec| (spec.enabled)(&app.session));
    assert_eq!((reason("view.tasks"), reason("tasks.new")), (Some(kept_here::NO_TASKS_HERE), Some(kept_here::NO_TASKS_HERE)));
}

#[test]
fn an_app_that_keeps_tasks_offers_them() {
    let (app, _) = window(false);
    let h = drawn(app);
    let app = h.state();
    assert!(app.hit.rect("module", Module::Tasks as u64).is_some());
    assert!(app.enabled("view.tasks") && app.enabled("tasks.new"));
    assert_eq!(crate::keys::resolve(app, "Mod+4"), Some("view.tasks"));
}

#[test]
fn favourite_categorize_and_import_are_greyed_with_the_reason_where_nothing_is_kept() {
    let (mut app, _) = window(true);
    let bob = app.session.mailbox.contacts.values().find(|c| c.first == "Bob").map(|c| c.id).unwrap();
    app.execute("view.people", json!({})).unwrap();
    app.execute("people.select", json!({"id": bob})).unwrap();
    for id in ["people.favorite", "people.categorize", "ui.import"] {
        assert!(!app.enabled(id), "{id}");
    }
    let reason = |app: &MailApp, id: &str| app.session.registry.get(id).and_then(|spec| (spec.enabled)(&app.session));
    assert_eq!(reason(&app, "people.favorite"), Some(kept_here::NO_MARKS_HERE));
    assert_eq!(reason(&app, "file.import"), Some(kept_here::NO_IMPORT_HERE));
    // The Categorize menu's button is greyed for the same reason.
    assert_eq!(crate::ribbon::menu_refusal(&app, Some(crate::ribbon::Menu::CategorizeContact)), Some(kept_here::NO_MARKS_HERE));
    // What the server keeps is offered as ever: Edit, Delete, New Contact (into its book).
    for id in ["people.edit", "people.delete", "people.new"] {
        assert!(app.enabled(id), "{id}");
    }
    // The card's Favorite is greyed too: a click on it changes nothing.
    let mut h = drawn(app);
    let star = h.state().hit.rect("cardAction", 2).expect("the card's Favorite");
    click(&mut h, star.center());
    assert!(h.state().session.mailbox.contact(bob).is_some_and(|c| !c.favorite));
    // Import by hand says why, and opens nothing.
    let refused = h.state_mut().run("ui.import", json!({})).unwrap_err();
    assert_eq!(refused, kept_here::NO_IMPORT_HERE);
    // File › Open & Export greys its Import tile, saying why.
    h.state_mut().execute("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
    h.run_steps(3);
    assert!(h.state().hit.rect("greyedTile", 0).is_some(), "greyed");
    // A window that keeps its mailbox offers them.
    let (mut app, _) = window(false);
    app.execute("people.select", json!({"id": bob})).unwrap();
    assert!(app.enabled("people.favorite") && app.enabled("people.categorize") && app.enabled("ui.import"));
    app.execute("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
    let h = drawn(app);
    assert!(h.state().hit.rect("greyedTile", 0).is_none(), "a tile of its own");
}

#[test]
fn a_simple_event_in_a_tab_asks_before_it_is_deleted() {
    let (mut app, _) = window(true);
    let budget = event(&app, "Budget review");
    let (title, text, action) = asked(&mut app, "calendar.delete", json!({"id": budget})).expect("a question");
    assert_eq!((title.as_str(), action.as_str()), ("Delete This Event?", "Delete"));
    assert_eq!(text, "The event \"Budget review\" will be removed from the calendar \"Ada\".");
    assert!(!text.contains("restore") && !text.contains("back"), "nothing promised back: {text}");
    // Escape is Cancel, on screen: still there, nothing noted for the server.
    let mut h = drawn(app);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    let still =
        |h: &Harness<'static, MailApp>| h.state().session.mailbox.event(budget).is_some() && h.state().session.mailbox.event_changes.is_empty();
    assert!(still(&h));
    // So is its Cancel button.
    h.state_mut().run("calendar.delete", json!({"id": budget})).unwrap();
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().dialog.is_none() && still(&h));
    // Delete: gone, and noted for the server, with no second question.
    h.state_mut().run("calendar.delete", json!({"id": budget})).unwrap();
    h.run_steps(2);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert!(h.state().session.mailbox.event(budget).is_none() && h.state().session.mailbox.event_changes.len() == 1);
    // In French, by the window's catalog.
    let (mut app, _) = window(true);
    crate::i18n::set_current(crate::i18n::Lang::from_tag("fr").unwrap());
    let budget = event(&app, "Budget review");
    let (title, text, _) = asked(&mut app, "calendar.delete", json!({"id": budget})).expect("a question");
    crate::i18n::set_current(crate::i18n::Lang::English);
    assert!(
        title != "Delete This Event?" && text.contains("Budget review") && text.contains("Ada") && !text.contains("will be removed"),
        "{title}: {text}"
    );
    // A window that keeps its mailbox asks nothing of it, as upstream.
    let (mut app, _) = window(false);
    let budget = event(&app, "Budget review");
    assert_eq!(asked(&mut app, "calendar.delete", json!({"id": budget})), None);
}

#[test]
fn one_day_of_a_series_deleted_by_itself_asks_as_that_day() {
    let (mut app, _) = window(true);
    let series = event(&app, "Weekly review");
    // Delete This Occurrence, from the event's menu: the day is named, which of the series is not asked.
    let (title, text, _) = asked(&mut app, "calendar.delete", json!({"id": series, "occurrence": "2026-10-20"})).expect("a question");
    let day = mailcraft_time::Date::new(2026, 10, 20).unwrap().medium_shown();
    assert_eq!(title, "Delete This Occurrence?");
    assert_eq!(text, format!("\"Weekly review\" on {day} will be removed from the calendar \"Ada\"."));
    // Answered: that day goes, with no second question.
    let params = crate::dialogs::answered(&json!({"id": series, "occurrence": "2026-10-20"}), &serde_json::Value::Null, None);
    assert_eq!(asked(&mut app, "calendar.delete", params), None);
    assert!(app.session.mailbox.event(series).is_some_and(|e| e.exceptions.iter().any(|d| d.iso() == "2026-10-20")));
}

#[test]
fn an_event_of_a_calendar_the_account_may_only_read_asks_nothing_and_its_delete_is_greyed() {
    let (mut app, account) = window(true);
    let calendars = vec![RemoteCalendar { id: "C1".into(), name: "Ada".into(), main: true, plain: true, read_only: true, ..Default::default() }];
    remote::apply_calendars(&mut app.session, &CalendarsDownload { account, email: ME.into(), complete: true, calendars }).unwrap();
    let budget = event(&app, "Budget review");
    app.execute("calendar.select", json!({"id": budget})).unwrap();
    assert!(!app.enabled("calendar.delete"));
    let reason = app.session.registry.get("calendar.delete").and_then(|spec| (spec.enabled)(&app.session));
    assert_eq!(reason, Some(mailcraft_engine::cmd::calendar::READ_ONLY_CALENDAR));
    // Named, it is refused with the reason, and nothing is asked first.
    let refused = app.run("calendar.delete", json!({"id": budget})).unwrap_err();
    assert!(refused.contains("can only be read") && app.dialog.as_ref().is_none_or(|d| !matches!(d, Dialog::Confirm { .. })), "{refused}");
    assert!(app.session.mailbox.event(budget).is_some());
}

#[test]
fn a_repeating_item_asks_its_scope_question_only() {
    let (mut app, _) = window(true);
    let series = event(&app, "Weekly review");
    app.execute("calendar.select", json!({"id": series, "day": "2026-10-20"})).unwrap();
    let (title, ..) = asked(&mut app, "calendar.delete", json!({"id": series})).expect("which of the series");
    assert_eq!(title, "Delete Recurring Item");
    // Answered "The Entire Series": no second question.
    app.dialog = None;
    assert_eq!(asked(&mut app, "calendar.delete", json!({"id": series, "scope": "series"})), None);
    assert!(app.session.mailbox.event(series).is_none());
    // With no day to ask about (named, not one of its days selected), it asks once, as a simple one.
    let (mut app, _) = window(true);
    let series = event(&app, "Weekly review");
    app.execute("calendar.select", json!({"id": null})).unwrap();
    let (title, ..) = asked(&mut app, "calendar.delete", json!({"id": series})).expect("asked once");
    assert_eq!(title, "Delete This Event?");
}

#[test]
fn a_meeting_with_guests_asks_its_cancellation_question_only() {
    let (mut app, _) = window(true);
    let planning = event(&app, "Planning");
    let (title, ..) = asked(&mut app, "calendar.delete", json!({"id": planning})).expect("the cancellation");
    assert_eq!(title, "Cancel This Meeting?");
    app.dialog = None;
    assert_eq!(asked(&mut app, "calendar.delete", json!({"id": planning, "send": true})), None);
    assert!(app.session.mailbox.event(planning).is_none());
}

#[test]
fn a_kept_contact_asks_before_it_is_deleted_for_the_whole_team() {
    let (mut app, _) = window(true);
    let bob = app.session.mailbox.contacts.values().find(|c| c.first == "Bob").map(|c| c.id).unwrap();
    let (title, text, action) = asked(&mut app, "people.delete", json!({"id": bob})).expect("a question");
    assert_eq!((title.as_str(), action.as_str()), ("Delete Contact", "Delete"));
    assert_eq!(text, "Bob Stone's card will be removed from Neoffice's contacts for the whole team. An administrator can restore it.");
    // Escape is Cancel: still there, nothing noted.
    let mut h = drawn(app);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert!(h.state().session.mailbox.contact(bob).is_some() && h.state().session.mailbox.contact_changes.is_empty());
    // Delete: the engine runs it, and it is noted for the server.
    h.state_mut().run("people.delete", json!({"id": bob})).unwrap();
    h.run_steps(2);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert!(h.state().session.mailbox.contact(bob).is_none() && h.state().session.mailbox.contact_changes.len() == 1);
}

#[test]
fn the_categorize_menu_names_the_starting_categories_in_the_interfaces_language() {
    let (mut app, _) = window(false);
    crate::i18n::set_current(crate::i18n::Lang::from_tag("fr").unwrap());
    let bob = app.session.mailbox.contacts.values().find(|c| c.first == "Bob").map(|c| c.id).unwrap();
    app.execute("people.select", json!({"id": bob})).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(400.0, 400.0))
        .build_ui_state(|ui, app: &mut MailApp| crate::ribbon::category_menu(ui, app, "people.categorize", |_, _| false, None), app);
    h.run_steps(2);
    let french = crate::i18n::Lang::from_tag("fr").unwrap();
    let (red, english) = (h.query_by_label(crate::i18n::t_in(french, "Red category")).is_some(), h.query_by_label("Red category").is_some());
    crate::i18n::set_current(crate::i18n::Lang::English);
    assert!(red && !english, "the menu in French");
}
