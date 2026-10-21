// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A hosting page that draws the navigation in its own menu: what the window stops drawing, what the page is told, and
//! the commands it may send. Headless windows, invented people at `example.org`.

use std::collections::BTreeSet;

use egui::{Key, Modifiers, Rect};
use egui_kittest::Harness;
use mailcraft_engine::cmd::jmap_mail::apply_jmap_mail;
use mailcraft_engine::{Module, Session};
use mailcraft_model::note::NoteServer;
use mailcraft_model::sync::{FolderListing, GraphFolder, GraphMessage, JmapMailResult, MessagePage, ServerMessage};
use mailcraft_model::{Address, FolderKind, Id, Mailbox};
use serde_json::{Value, json};

use super::*;
use crate::i18n::{Lang, set_current};
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
/// The two mailboxes the page hands over, the one the person adds by hand, and where their server is.
const ADA: &str = "ada@example.org";
const TEAM: &str = "team@example.org";
const BOB: &str = "bob@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session";
/// What a message says, which the page is never told.
const SUBJECT: &str = "Quarterly figures, do not forward";
const PREVIEW: &str = "The numbers are attached";
const SENDER: &str = "grace@example.org";

fn window(app: MailApp) -> Harness<'static, MailApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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

/// The sample mailbox in a window, drawn by a page with its own menu when `menu`.
fn sample(menu: bool) -> Harness<'static, MailApp> {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    if menu {
        app.services.host_menu = Some(Menu::default());
    }
    window(app)
}

/// Every kind of thing the window put on the screen in its last frame.
fn kinds(harness: &mut Harness<'static, MailApp>) -> BTreeSet<String> {
    let ctx = harness.ctx.clone();
    let shown = crate::control::inspect(harness.state_mut(), &ctx)["items"].clone();
    shown.as_array().unwrap().iter().map(|item| item["kind"].as_str().unwrap().to_string()).collect()
}

fn rect(harness: &Harness<'static, MailApp>, kind: &str, id: u64) -> Option<Rect> {
    harness.state().hit.rect(kind, id)
}

/// The leftmost point of the message list's rows.
fn list_left(harness: &Harness<'static, MailApp>) -> f32 {
    let app = harness.state();
    let rows: Vec<Rect> = app.session.mailbox.messages.keys().filter_map(|id| app.hit.rect("message", *id)).collect();
    assert!(!rows.is_empty(), "the list shows messages");
    rows.iter().map(|r| r.min.x).fold(f32::INFINITY, f32::min)
}

fn names(kinds: &[&str]) -> BTreeSet<String> {
    kinds.iter().map(|k| k.to_string()).collect()
}

#[test]
fn a_page_with_its_own_menu_hides_the_module_bar_and_mail_s_folder_pane_and_nothing_else() {
    let mut own = sample(false);
    let mut menu = sample(true);
    let (with, without) = (kinds(&mut menu), kinds(&mut own));
    // Exactly what the page's menu draws in their place.
    let gone: BTreeSet<String> = without.difference(&with).cloned().collect();
    assert_eq!(gone, names(&["folder", "folderHeading", "folderPaneHide", "module"]));
    assert!(with.is_subset(&without), "nothing new: {:?}", with.difference(&without).collect::<Vec<_>>());
    // Not even the strip that stays when the pane is put away: the list starts at the window's left edge.
    assert!(rect(&menu, "folderPaneShow", 0).is_none());
    // (A row's own margin only.)
    assert!(list_left(&menu) < 16.0, "{}", list_left(&menu));
    assert!(list_left(&own) > 200.0, "beside the module bar and the pane: {}", list_left(&own));
    // The person's own setting is left as it was.
    assert!(menu.state().session.view.folder_pane);

    // The shortcuts still bring each module to the front; Calendar keeps its pane, and no module bar comes back.
    for (key, module) in [(Key::Num2, Module::Calendar), (Key::Num3, Module::People), (Key::Num1, Module::Mail)] {
        menu.key_press_modifiers(Modifiers::COMMAND, key);
        menu.run_steps(3);
        assert_eq!(menu.state().session.view.module, module);
        own.key_press_modifiers(Modifiers::COMMAND, key);
        own.run_steps(3);
        assert_eq!(own.state().session.view.module, module);
        if module == Module::Calendar {
            let (with, without) = (kinds(&mut menu), kinds(&mut own));
            assert!(with.contains("calendar") && with.contains("miniDay"), "the calendars' pane: {with:?}");
            assert_eq!(without.difference(&with).cloned().collect::<BTreeSet<_>>(), names(&["module"]));
        }
        if module == Module::People {
            let (with, without) = (kinds(&mut menu), kinds(&mut own));
            assert!(with.contains("contact"), "{with:?}");
            assert_eq!(without.difference(&with).cloned().collect::<BTreeSet<_>>(), names(&["module"]));
        }
    }
}

#[test]
fn without_a_menu_the_window_draws_its_navigation_as_before() {
    let mut own = sample(false);
    assert!(own.state().services.host_menu.is_none());
    let shown = kinds(&mut own);
    assert!(["module", "folder", "folderHeading", "folderPaneHide"].iter().all(|k| shown.contains(*k)), "{shown:?}");
    assert!(rect(&own, "module", Module::Mail as u64).is_some_and(|r| r.min.x < 16.0), "{:?}", rect(&own, "module", Module::Mail as u64));
    // Nothing is told and nothing is taken.
    assert!(state(own.state()).is_none());
    let command = json!({"type": COMMAND, "id": 1, "name": "view.calendar", "params": {}}).to_string();
    assert!(answer(own.state_mut(), &command).is_none());
    assert_eq!(own.state().session.view.module, Module::Mail);
}

fn listed(id: &str, parent: &str, name: &str, kind: FolderKind) -> GraphFolder {
    GraphFolder { id: id.into(), parent: parent.into(), name: name.into(), kind, ..Default::default() }
}

fn mail(id: &str, folder: &str, read: bool) -> GraphMessage {
    GraphMessage {
        server: ServerMessage { id: id.into(), folder: folder.into(), read, ..Default::default() },
        message_id: format!("<{id}@mail.example.org>"),
        subject: SUBJECT.into(),
        from: Address::new("Grace Hopper", SENDER),
        sent: NOW - 3_600,
        received: NOW - 3_600,
        preview: PREVIEW.into(),
        ..Default::default()
    }
}

/// A JMAP mailbox added to the window as a hosting page adds it, with its server's `folders` and `mail` in them.
fn mailbox(app: &mut MailApp, email: &str, folders: Vec<GraphFolder>, mail: Vec<GraphMessage>) -> Id {
    let added = app.execute("account.add", json!({"email": email, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
    let account = added["id"].as_u64().unwrap();
    let listing = JmapMailResult { account, email: email.into(), listing: Some(FolderListing { folders, complete: true }), ..Default::default() };
    apply_jmap_mail(&mut app.session, listing).unwrap();
    let mut pages: Vec<MessagePage> = Vec::new();
    for m in mail {
        match pages.iter_mut().find(|p| p.folder == m.server.folder) {
            Some(page) => page.messages.push(m),
            None => pages.push(MessagePage { folder: m.server.folder.clone(), messages: vec![m], afresh: true, ..Default::default() }),
        }
    }
    apply_jmap_mail(&mut app.session, JmapMailResult { account, email: email.into(), pages, whole: true, ..Default::default() }).unwrap();
    account
}

/// A hosting page's window: Ada's mailbox and the team's, handed over in that order reversed (the team's first, in the
/// page's own case), and Bob's, which the person added by hand. Ada's Inbox in view.
fn page() -> (MailApp, Id, Id, Id) {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    app.services.jmap_host_credentials = true;
    app.session.keeps_local = false;
    let ada = mailbox(
        &mut app,
        ADA,
        vec![
            listed("Min", "", "INBOX", FolderKind::Inbox),
            listed("Mdr", "", "Drafts", FolderKind::Drafts),
            listed("Mse", "", "Sent", FolderKind::Sent),
            listed("Mtr", "", "Trash", FolderKind::Deleted),
            listed("Mju", "", "Junk", FolderKind::Junk),
            listed("Mar", "", "Archive", FolderKind::Archive),
            listed("Mpr", "", "Projects", FolderKind::Custom),
            listed("Map", "Mpr", "Apollo", FolderKind::Custom),
        ],
        vec![mail("A1", "Min", false), mail("A2", "Min", false), mail("A3", "Min", true), mail("A4", "Map", false)],
    );
    let team = mailbox(&mut app, TEAM, vec![listed("Tin", "", "INBOX", FolderKind::Inbox)], vec![mail("T1", "Tin", false)]);
    let bob = mailbox(&mut app, BOB, vec![listed("Bin", "", "INBOX", FolderKind::Inbox)], vec![mail("B1", "Bin", false)]);
    app.services.host_menu = Some(Menu {
        mailboxes: vec![Handed { email: "Team@Example.org".into(), name: String::new() }, Handed { email: ADA.into(), name: "Ada Marlow".into() }],
    });
    app.execute("folder.select", json!({"kind": "inbox", "account": ada})).unwrap();
    (app, ada, team, bob)
}

fn folder_of(app: &MailApp, account: Id, server: &str) -> Id {
    app.session.mailbox.folder_by_server(account, server).unwrap()
}

/// The local id of the account's folder of `kind` that its server doesn't have.
fn only_here(app: &MailApp, account: Id, kind: FolderKind) -> Id {
    let f = app.session.mailbox.folders.iter().find(|f| f.account == account && f.kind == kind).unwrap();
    assert!(f.server.is_none(), "{kind:?} is this window's alone");
    f.id
}

#[test]
fn the_page_is_told_its_mailboxes_folders_and_favourites_and_where_the_window_is() {
    let (mut app, ada, team, _) = page();
    let outbox = only_here(&app, ada, FolderKind::Outbox);
    let told = state(&app).unwrap();
    assert_eq!((told["type"].as_str(), told["module"].as_str()), (Some(STATE), Some("mail")));
    assert_eq!((told["account"].as_str(), told["folder"].as_str()), (Some(ADA), Some("Min")));

    // The page's mailboxes in the page's order, as it wrote them; not Bob's, which the person added here.
    let accounts = told["accounts"].as_array().unwrap();
    let said: Vec<(&str, &str, u64)> =
        accounts.iter().map(|a| (a["email"].as_str().unwrap(), a["name"].as_str().unwrap(), a["unread"].as_u64().unwrap())).collect();
    assert_eq!(said, [("Team@Example.org", "", 1), (ADA, "Ada Marlow", 2)]);

    // Every folder of Ada's in the pane's order: the role of each special one, the parent of a subfolder, the unread
    // mail known here; a folder only this window has (its Outbox) by its number here.
    let folders: Vec<(String, String, Value, Value, u64, bool)> = accounts[1]["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["id"].as_str().unwrap().to_string(),
                f["name"].as_str().unwrap().to_string(),
                f["role"].clone(),
                f["parent"].clone(),
                f["unread"].as_u64().unwrap(),
                f["selectable"].as_bool().unwrap(),
            )
        })
        .collect();
    let local = format!("local:{outbox}");
    let row = |id: &str, name: &str, role: Value, parent: Value, unread: u64| (id.to_string(), name.to_string(), role, parent, unread, true);
    assert_eq!(
        folders,
        [
            row("Min", "Inbox", json!("inbox"), Value::Null, 2),
            row("Mdr", "Drafts", json!("drafts"), Value::Null, 0),
            row("Mse", "Sent Items", json!("sent"), Value::Null, 0),
            row("Mtr", "Deleted Items", json!("trash"), Value::Null, 0),
            row("Mar", "Archive", json!("archive"), Value::Null, 0),
            row("Mju", "Junk Email", json!("spam"), Value::Null, 0),
            row(&local, "Outbox", json!("outbox"), Value::Null, 0),
            row("Mpr", "Projects", Value::Null, Value::Null, 0),
            row("Map", "Apollo", Value::Null, json!("Mpr"), 1),
        ]
    );
    assert_eq!(accounts[0]["folders"].as_array().unwrap().first().unwrap()["id"], "Tin");

    // The Favourites the pane lists that are theirs, in its order, named as it names them.
    let favorites = |app: &MailApp| state(app).unwrap()["favorites"].clone();
    app.execute("folder.favorite", json!({"id": folder_of(&app, ada, "Map"), "value": true})).unwrap();
    app.execute("folder.favorite", json!({"id": folder_of(&app, team, "Tin"), "value": true})).unwrap();
    let listed = favorites(&app);
    let listed = listed.as_array().unwrap();
    let order: Vec<Id> = mailcraft_engine::cmd::folder::favorites(&app.session)
        .into_iter()
        .filter(|id| app.session.mailbox.folder(*id).is_some_and(|f| f.account == ada || f.account == team))
        .collect();
    let told: Vec<String> = listed.iter().map(|f| f["folder"].as_str().unwrap().to_string()).collect();
    let pane: Vec<String> = order.iter().map(|id| super::folder_id(app.session.mailbox.folder(*id).unwrap())).collect();
    assert_eq!(told, pane, "in the pane's order");
    let apollo = listed.iter().find(|f| f["folder"] == "Map").unwrap();
    assert_eq!(apollo, &json!({"account": ADA, "folder": "Map", "name": "Apollo", "unread": 1}));
    let inbox = listed.iter().find(|f| f["folder"] == "Tin").unwrap();
    assert_eq!(inbox, &json!({"account": "Team@Example.org", "folder": "Tin", "name": "Inbox - team@example.org", "unread": 1}));

    // In the interface's language.
    set_current(Lang::from_tag("fr").unwrap());
    let french = state(&app).unwrap();
    set_current(Lang::English);
    assert_eq!(french["accounts"][1]["folders"][0]["name"], "Boîte de réception");
    assert_eq!(
        french["favorites"].as_array().unwrap().iter().find(|f| f["folder"] == "Tin").unwrap()["name"],
        "Boîte de réception - team@example.org"
    );
}

#[test]
fn the_page_is_told_which_folder_is_in_view_and_no_folder_while_a_search_shows_what_it_found() {
    let (mut app, ada, team, bob) = page();
    app.execute("folder.select", json!({"id": folder_of(&app, ada, "Map")})).unwrap();
    let told = state(&app).unwrap();
    assert_eq!((told["account"].as_str(), told["folder"].as_str()), (Some(ADA), Some("Map")));
    app.execute("folder.select", json!({"id": folder_of(&app, team, "Tin")})).unwrap();
    assert_eq!(state(&app).unwrap()["account"], "Team@Example.org");
    // A search in the folder: the mailbox still, no folder; one of every mailbox: neither.
    app.execute("view.search", json!({"query": "figures"})).unwrap();
    let told = state(&app).unwrap();
    assert_eq!((told["account"].as_str(), told["folder"].as_str()), (Some("Team@Example.org"), None));
    app.execute("view.search", json!({"query": "figures", "scope": "everywhere"})).unwrap();
    let told = state(&app).unwrap();
    assert_eq!((told["account"].as_str(), told["folder"].as_str()), (None, None));
    app.execute("view.clearSearch", json!({})).unwrap();
    // A folder of a mailbox the person added here is not named.
    app.execute("folder.select", json!({"id": folder_of(&app, bob, "Bin")})).unwrap();
    let told = state(&app).unwrap();
    assert_eq!((told["account"].as_str(), told["folder"].as_str()), (None, None));
    // The module in front, whichever.
    for (command, module) in [("view.calendar", "calendar"), ("view.people", "people"), ("view.mail", "mail")] {
        app.execute(command, json!({})).unwrap();
        assert_eq!(state(&app).unwrap()["module"], module);
    }
}

#[test]
fn nothing_of_a_message_nor_another_address_nor_a_secret_is_told() {
    let (mut app, ada, _, bob) = page();
    app.execute("folder.favorite", json!({"id": folder_of(&app, bob, "Bin"), "value": true})).unwrap();
    app.execute("folder.favorite", json!({"id": folder_of(&app, ada, "Min"), "value": true})).unwrap();
    let told = state(&app).unwrap().to_string();
    for never in [SUBJECT, PREVIEW, SENDER, "Grace", BOB, "Bin", SESSION, "mail.example.org", "@mail.example.org"] {
        assert!(!told.contains(never), "{never} in {told}");
    }
    // Only these fields, at every level.
    let state = state(&app).unwrap();
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<BTreeSet<String>>();
    assert_eq!(keys(&state), names(&["type", "module", "account", "folder", "favorites", "accounts"]));
    assert_eq!(keys(&state["accounts"][0]), names(&["email", "name", "unread", "folders"]));
    assert_eq!(keys(&state["accounts"][0]["folders"][0]), names(&["id", "name", "role", "parent", "unread", "selectable"]));
    assert_eq!(keys(&state["favorites"][0]), names(&["account", "folder", "name", "unread"]));
    assert!(state["favorites"].as_array().unwrap().iter().all(|f| f["account"] != BOB));
}

fn command(id: Value, name: &str, params: Value) -> String {
    json!({"type": COMMAND, "id": id, "name": name, "params": params}).to_string()
}

/// Send the window a command and read its answer.
fn ask(app: &mut MailApp, name: &str, params: Value) -> Value {
    answer(app, &command(json!(7), name, params)).unwrap()
}

fn ok() -> Value {
    json!({"type": DONE, "id": 7, "ok": true})
}

fn refused(app: &mut MailApp, name: &str, params: Value) -> String {
    let answer = ask(app, name, params);
    assert_eq!((answer["type"].as_str(), answer["id"].as_u64(), answer["ok"].as_bool()), (Some(DONE), Some(7), Some(false)), "{answer}");
    answer["error"].as_str().unwrap().to_string()
}

#[test]
fn each_command_the_page_may_send_runs_its_engine_command() {
    let (mut app, ada, team, _) = page();
    for (name, module) in [("view.calendar", Module::Calendar), ("view.people", Module::People), ("view.mail", Module::Mail)] {
        assert_eq!(ask(&mut app, name, json!({})), ok());
        assert_eq!(app.session.view.module, module);
    }
    // Notes where a server keeps them; refused, with the engine's reason in the interface's language, where none does.
    set_current(Lang::from_tag("fr").unwrap());
    let why = refused(&mut app, "view.notes", json!({}));
    let engine = app.session.run("view.notes", &json!({})).unwrap_err().to_string();
    let french = crate::i18n::run_words::refusal(&engine, &app.session.mailbox).into_owned();
    set_current(Lang::English);
    assert_eq!(why, french);
    assert!(
        engine.contains(mailcraft_engine::cmd::notes::NO_NOTES_HERE) && !why.contains(mailcraft_engine::cmd::notes::NO_NOTES_HERE),
        "in French: {why}"
    );
    assert_eq!(app.session.view.module, Module::Mail);
    app.session.mailbox.notes.servers.push(NoteServer { account: ada, ..Default::default() });
    assert_eq!(ask(&mut app, "view.notes", json!({})), ok());
    assert_eq!(app.session.view.module, Module::Notes);

    // A folder by the mailbox's address in any case and the id the state gave: Mail in front, that folder shown.
    assert_eq!(ask(&mut app, "folder.select", json!({"account": "ADA@example.org", "folder": "Map"})), ok());
    assert_eq!((app.session.view.module, app.session.view.folder), (Module::Mail, folder_of(&app, ada, "Map")));
    assert_eq!(ask(&mut app, "folder.select", json!({"account": TEAM, "folder": "Tin"})), ok());
    assert_eq!(app.session.view.folder, folder_of(&app, team, "Tin"));
    let outbox = only_here(&app, ada, FolderKind::Outbox);
    assert_eq!(ask(&mut app, "folder.select", json!({"account": ADA, "folder": format!("local:{outbox}")})), ok());
    assert_eq!(app.session.view.folder, outbox);

    // A new message, from the mailbox shown.
    assert_eq!(ask(&mut app, "folder.select", json!({"account": TEAM, "folder": "Tin"})), ok());
    assert_eq!(ask(&mut app, "mail.new", json!({})), ok());
    assert_eq!(app.session.compose.as_ref().map(|c| c.account), Some(team));
    app.session.compose = None;

    // The search box's search, Mail in front; empty text searches for nothing any more.
    app.execute("view.calendar", json!({})).unwrap();
    assert_eq!(ask(&mut app, "search", json!({"query": "figures"})), ok());
    assert_eq!((app.session.view.module, app.session.view.search.as_str(), app.search_text.as_str()), (Module::Mail, "figures", "figures"));
    assert!(!app.session.rows().is_empty(), "the search found the messages");
    assert_eq!(ask(&mut app, "search", json!({"query": ""})), ok());
    assert_eq!((app.session.view.search.as_str(), app.search_text.as_str()), ("", ""));
    assert_eq!(state(&app).unwrap()["folder"], "Tin", "the folder is shown again");
}

#[test]
fn an_unknown_command_or_one_with_bad_params_is_refused_with_a_reason() {
    let (mut app, ada, _, bob) = page();
    let before = (app.session.view.clone(), app.session.compose.is_some());
    let unknown = refused(&mut app, "mail.delete", json!({}));
    assert_eq!(unknown, "MailCraft doesn't take this command from the page.");
    for name in ["view.tasks", "ui.nora", "account.remove", "folder.delete", "", "VIEW.MAIL"] {
        assert_eq!(refused(&mut app, name, json!({})), unknown, "{name}");
    }
    let bad = "The page's command has parameters it doesn't take.";
    for (name, params) in [
        ("view.mail", json!({"x": 1})),
        ("view.calendar", json!([])),
        ("mail.new", json!({"to": "eve@example.org"})),
        ("folder.select", json!({})),
        ("folder.select", json!({"account": ADA})),
        ("folder.select", json!({"account": 5, "folder": "Min"})),
        ("folder.select", json!({"account": "nobody", "folder": "Min"})),
        ("folder.select", json!({"account": ADA, "folder": ""})),
        ("folder.select", json!({"account": ADA, "folder": "Min", "id": 3})),
        ("folder.select", json!({"account": ADA, "folder": "x".repeat(400)})),
        ("search", json!({})),
        ("search", json!({"query": 3})),
        ("search", json!({"query": "a".repeat(2_001)})),
        ("search", json!({"query": "a", "scope": "everywhere"})),
    ] {
        assert_eq!(refused(&mut app, name, params.clone()), bad, "{name} {params}");
    }
    // A mailbox the page didn't hand over, a folder its mailbox hasn't.
    let not_handed = "This mailbox isn't one the page handed over.";
    assert_eq!(refused(&mut app, "folder.select", json!({"account": BOB, "folder": "Bin"})), not_handed);
    assert_eq!(refused(&mut app, "folder.select", json!({"account": "eve@example.org", "folder": "Min"})), not_handed);
    let no_folder = "That mailbox has no such folder.";
    assert_eq!(refused(&mut app, "folder.select", json!({"account": ADA, "folder": "Tin"})), no_folder, "the team's");
    let bobs = folder_of(&app, bob, "Bin");
    assert_eq!(refused(&mut app, "folder.select", json!({"account": ADA, "folder": format!("local:{bobs}")})), no_folder);
    let inbox = folder_of(&app, ada, "Min");
    assert_eq!(refused(&mut app, "folder.select", json!({"account": ADA, "folder": format!("local:{inbox}")})), no_folder, "it has a server's id");
    assert_eq!(refused(&mut app, "folder.select", json!({"account": ADA, "folder": "local:x"})), no_folder);
    // Nothing was done.
    assert_eq!((app.session.view.clone(), app.session.compose.is_some()), before);

    // An id that is no number nor a short text: refused, answered without one.
    for id in [Value::Null, json!(true), json!([1]), json!(""), json!("x".repeat(65))] {
        let answer = answer(&mut app, &command(id.clone(), "view.mail", json!({}))).unwrap();
        assert_eq!(answer, json!({"type": DONE, "id": null, "ok": false, "error": "The command has no id."}), "{id}");
    }
    let without = json!({"type": COMMAND, "name": "view.mail"}).to_string();
    assert_eq!(answer(&mut app, &without).unwrap()["error"], "The command has no id.");
    // An id as text comes back as it came; no params is none.
    let named = json!({"type": COMMAND, "id": "a-7", "name": "view.people"}).to_string();
    assert_eq!(answer(&mut app, &named).unwrap(), json!({"type": DONE, "id": "a-7", "ok": true}));
    assert_eq!(app.session.view.module, Module::People);

    // In the interface's language.
    set_current(Lang::from_tag("fr").unwrap());
    let french = refused(&mut app, "mail.delete", json!({}));
    set_current(Lang::English);
    assert_ne!(french, unknown);
    assert!(!french.is_empty());
}

#[test]
fn what_is_no_command_is_not_answered() {
    let (mut app, _, _, _) = page();
    for message in [
        String::new(),
        "not json".to_string(),
        "[1, 2]".to_string(),
        "\"mailcraft:command\"".to_string(),
        json!({"type": "mailcraft:dirty", "dirty": true}).to_string(),
        json!({"type": STATE}).to_string(),
        json!({"id": 1, "name": "view.calendar"}).to_string(),
        json!({"type": COMMAND.to_uppercase(), "id": 1, "name": "view.calendar"}).to_string(),
        // Longer than any command.
        command(json!(1), "search", json!({"query": "a".repeat(70_000)})),
    ] {
        assert!(answer(&mut app, &message).is_none(), "{}", message.chars().take(80).collect::<String>());
    }
    assert_eq!(app.session.view.module, Module::Mail);
    assert_eq!(read(&command(json!(3), "view.calendar", json!({}))), Incoming::Command { id: json!(3), command: Command::View("view.calendar") });
}

#[test]
fn hostile_messages_never_panic() {
    let (mut app, _, _, _) = page();
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    let junk = [
        json!(null),
        json!(1),
        json!("x"),
        json!({}),
        json!({"a": [1, {"b": null}]}),
        json!(u64::MAX),
        json!(-1),
        json!(1.5e300),
        json!("\u{0}\n\r"),
    ];
    for name in VIEWS.iter().copied().chain(["folder.select", "mail.new", "search", "nothing"]) {
        for params in &junk {
            let _ = answer(&mut app, &command(json!(1), name, params.clone()));
            for key in ["account", "folder", "query"] {
                for value in &junk {
                    let _ = answer(&mut app, &command(json!(1), name, json!({ key: value })));
                }
            }
        }
    }
    let _ = answer(&mut app, &format!("{{\"type\": \"{COMMAND}\", \"id\": 1, \"name\": \"search\", \"params\": {{\"query\": {deep}}}}}"));
    let _ = answer(&mut app, &deep);
    assert!(state(&app).is_some());
}
