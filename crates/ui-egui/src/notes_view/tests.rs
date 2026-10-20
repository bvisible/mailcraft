// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Notes in the window: where the module is offered, a note typed into and saved, a rich note and a shared one, and
//! the view that is no longer offered. Headless windows on the sample mailbox. Invented people at `example.org`.

use std::sync::Arc;

use egui::{Key, Modifiers, PointerButton, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use mailcraft_engine::cmd::notes::{NOTE_NO_LINK, NOTE_NOT_YOURS, NOTE_READ_ONLY};
use mailcraft_engine::{Module, Session};
use mailcraft_model::note::{Note, NoteRights, NoteServer};
use mailcraft_model::{Account, Id, Origin};
use serde_json::json;

use super::*;
use crate::people_view::testing::{click, key, now, text};
use crate::{MailApp, Services};

/// A window on the sample mailbox with `module` in front; a hosting page's tab (that keeps nothing of its own) when
/// `hosted`.
fn window(hosted: bool, module: &str) -> Harness<'static, MailApp> {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(now(), 0), now(), 0), Services::default());
    app.autosave = false;
    app.session.keeps_local = !hosted;
    app.execute(module, json!({})).unwrap();
    let mut window = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    window.run_steps(5);
    window
}

/// An account whose server keeps notes, added to the window's mailbox.
fn with_server(window: &mut Harness<'static, MailApp>) -> Id {
    let app = window.state_mut();
    let account = app.session.mailbox.add_account(Account { email: "ada@example.org".into(), ..Default::default() });
    app.session.mailbox.notes.servers.push(NoteServer { account, ..Default::default() });
    account
}

/// A note of that server's, as a read would have brought it.
fn server_note(window: &mut Harness<'static, MailApp>, account: Id, note: Note) -> Id {
    let app = window.state_mut();
    let id = app.session.mailbox.alloc_id();
    let note =
        Note { id, origin: Origin { account, id: format!("VN-{id}"), ..Default::default() }, version: "2026-10-14T15:00:00.123Z".into(), ..note };
    app.session.mailbox.notes.items.insert(id, Arc::new(note));
    app.session.mailbox_changed();
    id
}

fn note(window: &Harness<'static, MailApp>, id: Id) -> Note {
    window.state().session.mailbox.notes.note(id).cloned().unwrap()
}

/// Click into the field `kind` (`noteTitle`, `noteText`) of the note `id`, at its right end.
fn click_into(window: &mut Harness<'static, MailApp>, kind: &str, id: Id) {
    let rect = window.state().hit.rect(kind, id).unwrap_or_else(|| panic!("no {kind}"));
    click(window, pos2(rect.max.x - 4.0, rect.center().y), PointerButton::Primary, 1);
}

#[test]
fn the_module_bar_shows_notes_where_they_are_kept_and_not_in_a_hosting_tab_without_them() {
    let shown = |window: &Harness<'static, MailApp>| window.state().hit.rect("module", Module::Notes as u64).is_some();
    let mut desk = window(false, "view.mail");
    assert!(shown(&desk), "this computer keeps them");
    let rect = desk.state().hit.rect("module", Module::Notes as u64).unwrap();
    click(&mut desk, rect.center(), PointerButton::Primary, 1);
    assert_eq!(desk.state().session.view.module, Module::Notes);
    let mut hosted = window(true, "view.mail");
    assert!(!shown(&hosted), "a hosting page's tab without a server that keeps them");
    with_server(&mut hosted);
    hosted.run_steps(2);
    assert!(shown(&hosted), "once its server keeps them");
}

#[test]
fn a_note_typed_into_is_saved_when_left_as_one_undo_step() {
    let mut window = window(false, "view.notes");
    let id = window.state_mut().execute("notes.new", json!({"title": "Groceries", "text": "Milk"})).unwrap()["id"].as_u64().unwrap();
    window.run_steps(3);
    assert!(window.state().hit.rect("note", id).is_some(), "its card");
    click_into(&mut window, "noteTitle", id);
    text(&mut window, " list");
    assert_eq!(note(&window, id).title, "Groceries", "not saved while it is typed into");
    click_into(&mut window, "noteText", id);
    text(&mut window, " and eggs");
    key(&mut window, Key::Escape);
    window.run_steps(3);
    let saved = note(&window, id);
    assert_eq!((saved.title.as_str(), saved.text.as_str()), ("Groceries list", "Milk and eggs"));
    // Typed into twice, saved twice: two steps, then the note as it was made.
    let app = window.state_mut();
    assert!(app.session.undo());
    assert!(app.session.undo());
    assert_eq!(app.session.mailbox.notes.note(id).map(|note| (note.title.clone(), note.text.clone())), Some(("Groceries".into(), "Milk".into())));
    // Words typed and left behind go into the note when another module comes to the front.
    window.run_steps(3);
    click_into(&mut window, "noteText", id);
    text(&mut window, "!");
    window.state_mut().execute("view.mail", json!({})).unwrap();
    window.run_steps(3);
    assert_eq!(note(&window, id).text, "Milk!");
}

#[test]
fn a_note_saves_itself_two_seconds_after_the_last_key() {
    let mut window = window(false, "view.notes");
    let id = window.state_mut().execute("notes.new", json!({"title": "Plan"})).unwrap()["id"].as_u64().unwrap();
    window.run_steps(3);
    click_into(&mut window, "noteText", id);
    text(&mut window, "Steps");
    window.run_steps(60);
    assert_eq!(note(&window, id).text, "", "a second after");
    window.run_steps(90);
    assert_eq!(note(&window, id).text, "Steps");
}

#[test]
fn a_rich_note_shows_its_text_read_only_and_open_on_the_web() {
    let mut window = window(false, "view.notes");
    let account = with_server(&mut window);
    let rich = Note {
        title: "Quarterly plan".into(),
        text: "Goals".into(),
        rich: true,
        rights: NoteRights { write_text: false, ..Default::default() },
        web_url: "https://desk.example.org/app/notes?note=VN-1".into(),
        ..Default::default()
    };
    let id = server_note(&mut window, account, rich);
    window.state_mut().execute("notes.select", json!({"id": id})).unwrap();
    window.run_steps(3);
    window.get_by_label("This note has formatting, a checklist or attachments: change its text where it was written.");
    click_into(&mut window, "noteText", id);
    text(&mut window, " changed");
    key(&mut window, Key::Escape);
    window.run_steps(3);
    assert_eq!(note(&window, id).text, "Goals", "its text is changed where it was written");
    // Its title may change.
    click_into(&mut window, "noteTitle", id);
    text(&mut window, " (draft)");
    key(&mut window, Key::Escape);
    window.run_steps(3);
    assert_eq!(note(&window, id).title, "Quarterly plan (draft)");
    assert!(window.state().hit.rect("noteWeb", id).is_some());
    assert!(card_text(window.state(), &note(&window, id)).badges.contains(&"Formatted".to_string()));
    let opened = window.state_mut().execute("ui.openNote", json!({"id": id})).unwrap();
    assert_eq!(opened["url"], "https://desk.example.org/app/notes?note=VN-1");
    // A link that isn't https is never followed.
    let local = window.state_mut().execute("notes.new", json!({"title": "Mine"})).unwrap()["id"].as_u64().unwrap();
    assert!(window.state_mut().execute("ui.openNote", json!({"id": local})).is_err());
}

#[test]
fn a_shared_note_says_who_shared_it_and_greys_what_the_rights_refuse() {
    let mut window = window(false, "view.notes");
    let account = with_server(&mut window);
    let shared = Note {
        title: "Room booking codes".into(),
        text: "4711".into(),
        shared: true,
        owner_name: "Grace Okafor".into(),
        rights: NoteRights { write: false, write_text: false, delete: false },
        ..Default::default()
    };
    let id = server_note(&mut window, account, shared);
    window.state_mut().execute("notes.select", json!({"id": id})).unwrap();
    window.run_steps(3);
    let said = card_text(window.state(), &note(&window, id));
    assert!(said.badges.contains(&"Shared by Grace Okafor".to_string()), "{said:?}");
    window.get_by_label("You may read this note, not change it.");
    let refused: Vec<(&str, Option<&str>)> = actions(&note(&window, id)).into_iter().map(|(label, _, _, why)| (label, why)).collect();
    assert_eq!(
        refused,
        [("Pin", Some(NOTE_READ_ONLY)), ("Archive", Some(NOTE_READ_ONLY)), ("Open on the Web", Some(NOTE_NO_LINK)), ("Delete", Some(NOTE_NOT_YOURS))]
    );
    click_into(&mut window, "noteTitle", id);
    text(&mut window, " mine");
    key(&mut window, Key::Escape);
    window.run_steps(3);
    assert_eq!(note(&window, id).title, "Room booking codes");
    // Its Delete in the editor is greyed: a click does nothing.
    let delete = window.state().hit.rect("noteDelete", id).unwrap();
    click(&mut window, delete.center(), PointerButton::Primary, 1);
    assert!(window.state().session.mailbox.notes.note(id).is_some());
    // A note of the mailbox's own, not saved anywhere else, says nothing of saving.
    let own = window.state_mut().session.mailbox.alloc_id();
    window.state_mut().session.mailbox.notes.items.insert(own, Arc::new(Note { id: own, title: "Mine".into(), ..Default::default() }));
    assert_eq!(save_state(window.state(), &note(&window, own)), None);
    // Notes kept in two places now: each card says where.
    let said = card_text(window.state(), &note(&window, id));
    assert!(said.badges.contains(&"ada@example.org".to_string()), "{said:?}");
    assert!(card_text(window.state(), &note(&window, own)).badges.contains(&"On this computer".to_string()));
    assert_eq!(save_state(window.state(), &note(&window, id)).as_deref(), Some("Saved"));
}

#[test]
fn a_notes_view_that_is_no_longer_offered_falls_back_to_mail() {
    let mut window = window(true, "view.mail");
    with_server(&mut window);
    window.state_mut().execute("view.notes", json!({})).unwrap();
    window.run_steps(3);
    assert_eq!(window.state().session.view.module, Module::Notes);
    // The server no longer keeps notes.
    window.state_mut().session.mailbox.notes.servers.clear();
    window.run_steps(3);
    assert_eq!(window.state().session.view.module, Module::Mail);
}

#[test]
fn mod_5_opens_notes_only_where_offered() {
    let press = |window: &mut Harness<'static, MailApp>| {
        for pressed in [true, false] {
            window.input_mut().events.push(egui::Event::Key {
                key: Key::Num5,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            });
        }
        window.run_steps(3);
    };
    let mut desk = window(false, "view.mail");
    press(&mut desk);
    assert_eq!(desk.state().session.view.module, Module::Notes);
    let mut hosted = window(true, "view.mail");
    press(&mut hosted);
    assert_eq!(hosted.state().session.view.module, Module::Mail, "nothing to open");
}

#[test]
fn the_cards_say_untitled_and_the_lists_say_when_they_are_empty() {
    let mut window = window(false, "view.notes");
    window.get_by_label("No notes yet: New Note starts one.");
    let id = window.state_mut().execute("notes.new", json!({"text": "\n"})).unwrap()["id"].as_u64().unwrap();
    window.run_steps(3);
    assert_eq!(card_text(window.state(), &note(&window, id)).title, "Untitled");
    let words = window.state_mut().execute("notes.new", json!({"text": "Lake walk\nCooking class"})).unwrap()["id"].as_u64().unwrap();
    let said = card_text(window.state(), &note(&window, words));
    assert_eq!((said.title.as_str(), said.words.as_str()), ("Lake walk", "Cooking class"), "the first line is its title");
    assert!(said.changed.starts_with("Changed "));
}

#[test]
fn words_typed_into_a_note_in_a_hosting_tab_would_be_lost_until_they_are_in_it() {
    let mut window = window(true, "view.mail");
    let account = with_server(&mut window);
    let id = server_note(&mut window, account, Note { title: "Groceries".into(), text: "Milk".into(), ..Default::default() });
    window.state_mut().execute("view.notes", json!({})).unwrap();
    window.state_mut().execute("notes.select", json!({"id": id})).unwrap();
    window.run_steps(3);
    assert!(!window.state().notes_unsent());
    click_into(&mut window, "noteText", id);
    text(&mut window, " and eggs");
    assert!(window.state().notes_unsent(), "typed, not in the note yet");
    key(&mut window, Key::Escape);
    window.run_steps(3);
    assert_eq!(note(&window, id).text, "Milk and eggs");
    assert!(window.state().notes_unsent(), "in the note, and its change waits to go");
}

#[test]
fn what_is_typed_into_a_note_stops_at_what_its_server_takes() {
    let mut window = window(false, "view.notes");
    let account = with_server(&mut window);
    if let Some(server) = window.state_mut().session.mailbox.notes.servers.iter_mut().find(|server| server.account == account) {
        (server.max_title, server.max_text) = (12, 10);
    }
    let id = server_note(&mut window, account, Note { title: "Plan".into(), text: "Steps".into(), ..Default::default() });
    window.state_mut().execute("notes.select", json!({"id": id})).unwrap();
    window.run_steps(3);
    click_into(&mut window, "noteText", id);
    text(&mut window, " one two three four");
    click_into(&mut window, "noteTitle", id);
    text(&mut window, " for the whole year");
    key(&mut window, Key::Escape);
    window.run_steps(3);
    let kept = note(&window, id);
    assert_eq!((kept.text.as_str(), kept.title.as_str()), ("Steps one ", "Plan for the"), "as much as it takes, nothing refused");
}
