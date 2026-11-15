// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the list of variant D (Spark's) in a hosting page's tab, through real frames.
//! In a hosting page's tab the list is variant D's: a short header (the view's name and its menu, under it the arrangement and
//! whose mail it is, on the right « Afficher les traités », the search and a new message), no row of « Trier avec Nora » nor
//! arrangement in words; « Toutes les boîtes » first of the Favourites, the Inboxes of every mailbox of the person's own together,
//! each row with its mailbox's colour, its gestures on each message's own mailbox. The program that keeps its mail keeps
//! upstream's list. And (`--ignored`, with `VARIANT_D_SHOTS=<folder>`) every state drawn in French, light and dark, to PNG files.

use egui::PointerButton;
use mailcraft_model::{Account, AccountKind, Address, FolderKind, Id, Mailbox, Message, Rgb};
use serde_json::json;

use crate::people_view::testing;

const ADA: &str = "ada@example.org";
const TEAM: &str = "team@example.org";

/// The sample's mailbox as a server's (ada@), beside a second one of the person's (team@) with mail of its own in its Inbox, each
/// with its colour.
fn mailbox() -> (Mailbox, Id, Id) {
    let mut mb = mailcraft_engine::sample::mailbox(testing::now(), 0);
    let ada = mb.accounts[0].id;
    let first = &mut mb.accounts[0];
    first.kind = AccountKind::Jmap;
    first.email = ADA.into();
    first.name = ADA.into();
    first.color = Some(Rgb(0xD0, 0x47, 0x3A));
    let team = mb.add_account(Account {
        kind: AccountKind::Jmap,
        email: TEAM.into(),
        name: TEAM.into(),
        color: Some(Rgb(0xE0, 0x91, 0x3A)),
        ..Default::default()
    });
    let inbox = mb.folder_of_kind(team, FolderKind::Inbox).unwrap();
    mb.add_folder(team, None, "Chantiers").unwrap();
    let now = testing::now();
    for (i, (who, email, subject, read)) in [
        ("Service Exemple", "support@service.example", "[Nouvelle réponse] #0051 — Changement d'adresse", false),
        ("Atelier Nord", "atelier@nord.example", "Devis pour la toiture", false),
        ("Marc Dupont", "marc@dupont.example", "Réunion de jeudi", true),
    ]
    .into_iter()
    .enumerate()
    {
        let when = now - 1_200 - 2_400 * i as i64;
        let body = mailcraft_model::Body { text: format!("{subject} : le détail suit."), html: None };
        let m = Message {
            folder: inbox,
            from: Address::new(who, email),
            subject: subject.into(),
            date: when,
            received: when,
            read,
            body,
            ..Default::default()
        };
        mb.add_message(m).unwrap();
    }
    (mb, ada, team)
}

/// The window on the two mailboxes, as a hosting page's tab shows it (`hosted`), Ada's Inbox in view.
fn window(hosted: bool) -> (testing::Window, Id, Id) {
    let (mb, ada, team) = mailbox();
    let mut w = testing::window_on(mb, "view.mail");
    w.state_mut().session.keeps_local = !hosted;
    w.state_mut().execute("ui.dismissReminder", json!({"all": true})).unwrap();
    w.state_mut().execute("folder.select", json!({"kind": "inbox", "account": ada})).unwrap();
    w.run_steps(3);
    (w, ada, team)
}

fn has(w: &testing::Window, kind: &str, id: u64) -> bool {
    w.state().hit.rect(kind, id).is_some()
}

fn click(w: &mut testing::Window, kind: &str, id: u64) {
    let pos = w.state().hit.rect(kind, id).unwrap_or_else(|| panic!("no {kind} {id} in the window")).center();
    testing::click(w, pos, PointerButton::Primary, 1);
    w.run_steps(3);
}

fn hover(w: &mut testing::Window, kind: &str, id: u64) {
    let pos = w.state().hit.rect(kind, id).unwrap_or_else(|| panic!("no {kind} {id} in the window")).center();
    w.hover_at(pos);
    w.run_steps(3);
}

fn shows(w: &testing::Window, label: &str) -> bool {
    use egui_kittest::kittest::Queryable as _;
    w.query_all_by_label_contains(label).next().is_some()
}

/// The rows the window drew.
fn drawn(w: &mut testing::Window) -> Vec<Id> {
    let rows = w.state_mut().session.rows();
    rows.iter().filter(|row| w.state().hit.rect("message", row.id).is_some()).map(|row| row.id).collect()
}

fn account_of(w: &testing::Window, id: Id) -> Id {
    w.state().session.mailbox.message(id).unwrap().account
}

fn folder_of(w: &testing::Window, id: Id) -> (Id, FolderKind) {
    let s = &w.state().session;
    let m = s.mailbox.message(id).unwrap();
    (m.account, s.mailbox.folder(m.folder).unwrap().kind)
}

#[test]
fn the_header_is_one_row_with_the_views_name_whose_mail_it_is_and_show_done_search_and_new() {
    let (mut w, _, _) = window(true);
    // The name, under it the arrangement and the mailbox; no tabs All and Unread, no arrangement in words with its arrow, no
    // row of « Trier avec Nora ».
    assert!(has(&w, "listViewName", 0) && has(&w, "listSort", 0));
    assert!(!has(&w, "listSort", 1) && !has(&w, "listTab", 0) && !has(&w, "noraSort", 0));
    assert!(shows(&w, "By Date · ada@"));
    for button in ["listShowDone", "listSearch", "listNew"] {
        assert!(has(&w, button, 0), "{button}");
    }
    // The search gives its box the keyboard; the pencil opens a new message.
    click(&mut w, "listSearch", 0);
    assert!(w.ctx.memory(|m| m.focused().is_some()), "the search box has the keyboard");
    click(&mut w, "listNew", 0);
    assert!(w.state().session.compose.is_some(), "a new message");
}

#[test]
fn show_done_is_offered_in_an_inbox_only_and_the_desktop_keeps_upstreams_header() {
    let (mut w, ada, _) = window(true);
    w.state_mut().execute("folder.select", json!({"kind": "sent", "account": ada})).unwrap();
    w.run_steps(3);
    assert!(!has(&w, "listShowDone", 0) && has(&w, "listSearch", 0));
    let (w, _, _) = window(false);
    assert!(has(&w, "listTab", 0) && has(&w, "listSort", 1), "upstream's tabs and arrow");
    assert!(!has(&w, "listViewName", 0) && !has(&w, "listShowDone", 0) && !has(&w, "allInboxes", 0));
}

#[test]
fn all_inboxes_is_first_of_the_favourites_and_lists_every_inbox_with_its_mailboxs_colour() {
    let (mut w, ada, team) = window(true);
    assert!(has(&w, "allInboxes", 0), "first of the Favourites");
    let alone = drawn(&mut w);
    assert!(alone.iter().all(|id| account_of(&w, *id) == ada));
    assert!(alone.iter().all(|id| !has(&w, "messageAccountColor", *id)), "one mailbox: no colour");
    click(&mut w, "allInboxes", 0);
    assert!(w.state().session.all_inboxes().is_some());
    let listed = drawn(&mut w);
    assert!(listed.iter().any(|id| account_of(&w, *id) == team) && listed.iter().any(|id| account_of(&w, *id) == ada));
    assert!(listed.iter().all(|id| has(&w, "messageAccountColor", *id)), "each row its mailbox's colour");
    assert!(shows(&w, "All Inboxes"));
    assert!(shows(&w, "By Date · ada@ and team@"));
    // The unread mail of both Inboxes, beside its name in the Favourites.
    let unread = crate::mail_view::header::all_unread(w.state_mut());
    let counts = w.state_mut().session.folder_counts();
    let s = &w.state().session;
    let both = counts.unread(s.mailbox.folder_of_kind(ada, FolderKind::Inbox).unwrap())
        + counts.unread(s.mailbox.folder_of_kind(team, FolderKind::Inbox).unwrap());
    assert!(unread > 0 && unread == both);
    assert!(shows(&w, &unread.to_string()));
    // A folder picked in the pane leaves it.
    let team_inbox = w.state().session.mailbox.folder_of_kind(team, FolderKind::Inbox).unwrap();
    click(&mut w, "folder", team_inbox);
    assert!(w.state().session.all_inboxes().is_none());
    assert!(drawn(&mut w).iter().all(|id| account_of(&w, *id) == team));
}

#[test]
fn the_views_menu_picks_all_inboxes_or_an_inbox() {
    let (mut w, _, team) = window(true);
    click(&mut w, "listViewName", 0);
    assert!(shows(&w, "All Inboxes ("), "with its unread mail");
    assert!(shows(&w, "— team@"));
    use egui_kittest::kittest::Queryable as _;
    w.get_by_label_contains("— team@").click();
    w.run_steps(3);
    let team_inbox = w.state().session.mailbox.folder_of_kind(team, FolderKind::Inbox).unwrap();
    assert_eq!(w.state().session.view.folder, team_inbox);
    click(&mut w, "listViewName", 0);
    w.get_by_label_contains("All Inboxes (").click();
    w.run_steps(3);
    assert!(w.state().session.all_inboxes().is_some());
    // The arrangement's menu, under the name: the sort, the filters, Select All.
    click(&mut w, "listSort", 0);
    for label in ["By Date", "Reverse Sort", "Unread", "Select All"] {
        assert!(shows(&w, label), "{label}");
    }
}

#[test]
fn in_all_inboxes_done_and_the_quick_actions_act_on_each_messages_own_mailbox() {
    let (mut w, ada, team) = window(true);
    click(&mut w, "allInboxes", 0);
    let listed = drawn(&mut w);
    let t = *listed.iter().find(|id| account_of(&w, **id) == team).unwrap();
    let a = *listed.iter().find(|id| account_of(&w, **id) == ada).unwrap();
    // « Traité » on a row of each: in its own mailbox's Archive.
    for id in [t, a] {
        hover(&mut w, "message", id);
        click(&mut w, "doneRound", id);
    }
    assert_eq!(folder_of(&w, t), (team, FolderKind::Archive));
    assert_eq!(folder_of(&w, a), (ada, FolderKind::Archive));
    // « Afficher les traités »: both, from both Archives, put back each in its own Inbox.
    click(&mut w, "listShowDone", 0);
    assert!(has(&w, "doneListed", t) && has(&w, "doneListed", a));
    click(&mut w, "doneListed", t);
    assert_eq!(folder_of(&w, t), (team, FolderKind::Inbox));
    click(&mut w, "listShowDone", 1);
    // Delete from the quick actions: its own mailbox's Deleted Items.
    hover(&mut w, "message", t);
    click(&mut w, "doneQuickDelete", t);
    assert_eq!(folder_of(&w, t), (team, FolderKind::Deleted));
    // A day's round: each of the day's in its own mailbox's Archive.
    let rows = w.state_mut().session.rows();
    let first = rows.iter().position(|r| r.group.is_some()).unwrap();
    let ids = super::done::group_ids(&rows, first);
    assert!(ids.iter().any(|id| account_of(&w, *id) == team) && ids.iter().any(|id| account_of(&w, *id) == ada), "a day of both");
    hover(&mut w, "doneGroup", first as u64);
    click(&mut w, "doneGroup", first as u64);
    for id in ids {
        assert_eq!(folder_of(&w, id), (account_of(&w, id), FolderKind::Archive), "{id}");
    }
}

#[test]
fn in_all_inboxes_a_rows_move_offers_its_own_mailboxs_folders() {
    let (mut w, ada, team) = window(true);
    click(&mut w, "allInboxes", 0);
    let listed = drawn(&mut w);
    let t = *listed.iter().find(|id| account_of(&w, **id) == team).unwrap();
    let a = *listed.iter().find(|id| account_of(&w, **id) == ada).unwrap();
    hover(&mut w, "message", t);
    click(&mut w, "doneQuickMove", t);
    assert!(shows(&w, "Chantiers"), "team@'s own folder");
    use egui_kittest::kittest::Queryable as _;
    w.get_by_label("Chantiers").click();
    w.run_steps(3);
    let chantiers = w.state().session.mailbox.folders.iter().find(|f| f.name == "Chantiers").unwrap().id;
    assert_eq!(w.state().session.mailbox.message(t).unwrap().folder, chantiers);
    // Ada's row: Ada's folders, not team@'s.
    w.hover_at(egui::pos2(1.0, 1.0));
    w.run_steps(3);
    hover(&mut w, "message", a);
    click(&mut w, "doneQuickMove", a);
    assert!(!shows(&w, "Chantiers") && shows(&w, "Projects"));
}

/// The window on the two mailboxes with Ada's Inbox's mail said by kind of sender: two notifications and two newsletters, the rest
/// a person's. Their ids, notifications then newsletters.
fn by_kind() -> (testing::Window, Vec<Id>, Vec<Id>) {
    let (mut w, ada, _) = window(true);
    let inbox = w.state().session.mailbox.folder_of_kind(ada, FolderKind::Inbox).unwrap();
    let mut ids: Vec<Id> = w.state().session.mailbox.messages_in(inbox).filter(|m| !m.draft).map(|m| m.id).collect();
    ids.sort_unstable();
    let (notes, letters) = (ids[..2].to_vec(), ids[2..4].to_vec());
    let said: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| {
            let stream = if notes.contains(id) {
                "notification"
            } else if letters.contains(id) {
                "newsletter"
            } else {
                "person"
            };
            json!({"id": id, "stream": stream})
        })
        .collect();
    w.state_mut().execute("mail.setStreams", json!({"streams": said})).unwrap();
    w.state_mut().execute("folder.select", json!({"id": inbox})).unwrap();
    w.run_steps(3);
    (w, notes, letters)
}

#[test]
fn principal_has_its_groups_where_their_newest_is_and_a_group_opens_as_its_tab_did() {
    let (mut w, notes, letters) = by_kind();
    assert_eq!(w.state().session.view.filter, mailcraft_engine::Filter::Main);
    // No tabs: two rows of groups, each with all of its mail, and none of it in a row of its own.
    assert!(!has(&w, "listTab", 0));
    let rows = w.state_mut().session.rows();
    for (stream, ids) in [(mailcraft_model::Stream::Notification, &notes), (mailcraft_model::Stream::Newsletter, &letters)] {
        let row = rows.iter().find(|r| r.bundle == Some(stream)).expect("the group's row");
        let mut members = row.members.clone();
        members.sort_unstable();
        assert_eq!(&members, ids);
        assert!(!row.conversation(), "a group is no conversation");
    }
    assert!(has(&w, "bundle", 1) && has(&w, "bundle", 2));
    for id in notes.iter().chain(&letters) {
        assert!(!has(&w, "message", *id), "{id} in its group's row only");
    }
    // A group opens as its tab did: its mail alone, the header « ← | ✓ » with its name.
    click(&mut w, "bundle", 1);
    assert_eq!(w.state().session.view.filter, mailcraft_engine::Filter::Notifications);
    assert!(has(&w, "groupBack", 0) && has(&w, "groupAllDone", 0) && !has(&w, "listViewName", 0));
    assert!(shows(&w, "Notifications") && shows(&w, "2 · "));
    let mut listed = drawn(&mut w);
    listed.sort_unstable();
    assert_eq!(listed, notes);
    click(&mut w, "groupBack", 0);
    assert_eq!(w.state().session.view.filter, mailcraft_engine::Filter::Main);
}

#[test]
fn a_group_done_at_once_is_in_the_section_joined_by_its_rail_and_comes_back_with_undo_all() {
    let (mut w, notes, letters) = by_kind();
    // From the group's row: its round marks all of it done.
    hover(&mut w, "bundle", 2);
    let newest = w.state_mut().session.rows().iter().find(|r| r.bundle == Some(mailcraft_model::Stream::Newsletter)).unwrap().id;
    click(&mut w, "doneRound", newest);
    for id in &letters {
        assert_eq!(folder_of(&w, *id).1, FolderKind::Archive, "{id}");
    }
    assert!(!has(&w, "bundle", 2), "the group is gone with its mail");
    // From the open group: « ✓ ».
    click(&mut w, "bundle", 1);
    click(&mut w, "groupAllDone", 0);
    for id in &notes {
        assert_eq!(folder_of(&w, *id).1, FolderKind::Archive, "{id}");
        assert!(has(&w, "doneUndo", *id), "in the section, its own group's");
    }
    for id in &letters {
        assert!(!has(&w, "doneUndo", *id), "not another group's");
    }
    assert!(has(&w, "doneSection", 0) && has(&w, "doneUndoAll", 0));
    // One back with its round, then all of them.
    click(&mut w, "doneUndo", notes[0]);
    assert_eq!(folder_of(&w, notes[0]).1, FolderKind::Inbox);
    click(&mut w, "groupBack", 0);
    click(&mut w, "doneUndoAll", 0);
    for id in notes.iter().chain(&letters) {
        assert_eq!(folder_of(&w, *id).1, FolderKind::Inbox, "{id}");
    }
    assert!(w.state().done.items.is_empty());
}

#[test]
fn the_keys_step_past_a_group_and_select_all_leaves_it_out() {
    let (mut w, notes, letters) = by_kind();
    let rows = w.state_mut().session.rows();
    let at = rows.iter().position(|r| r.bundle.is_some()).expect("a group");
    assert!(at > 0, "a message's row before the group");
    // From the row before it, Down lands past the groups; Up comes back over them.
    let before = rows[at - 1].id;
    w.state_mut().run("mail.select", json!({"id": before})).unwrap();
    let mut seen = Vec::new();
    for _ in 0..3 {
        w.state_mut().run("mail.next", json!({})).unwrap();
        seen.push(w.state().session.sel.messages.clone());
    }
    for picked in &seen {
        assert!(picked.iter().all(|id| !notes.contains(id) && !letters.contains(id)), "{picked:?}");
    }
    w.state_mut().run("mail.selectAll", json!({})).unwrap();
    let all = w.state().session.sel.messages.clone();
    assert!(!all.is_empty() && all.iter().all(|id| !notes.contains(id) && !letters.contains(id)));
    // An agent's list says which row is a group, with all it stands for.
    let listed = w.state_mut().run("mail.list", json!({})).unwrap();
    let group = listed["messages"].as_array().unwrap().iter().find(|m| m["bundle"] == json!("notification")).expect("the group");
    assert_eq!(group["members"].as_array().unwrap().len(), notes.len());
}

#[test]
fn undo_all_in_an_open_group_puts_back_that_groups_alone() {
    let (mut w, notes, letters) = by_kind();
    click(&mut w, "bundle", 2);
    click(&mut w, "groupAllDone", 0);
    click(&mut w, "groupBack", 0);
    click(&mut w, "bundle", 1);
    click(&mut w, "groupAllDone", 0);
    for id in notes.iter().chain(&letters) {
        assert_eq!(folder_of(&w, *id).1, FolderKind::Archive, "{id}");
    }
    click(&mut w, "doneUndoAll", 0);
    for id in &notes {
        assert_eq!(folder_of(&w, *id).1, FolderKind::Inbox, "{id}");
    }
    for id in &letters {
        assert_eq!(folder_of(&w, *id).1, FolderKind::Archive, "{id}: the other group's stays done");
    }
    assert_eq!(w.state().done.items.len(), letters.len());
}

#[test]
fn a_wide_list_has_a_row_of_one_line_a_narrow_one_of_two() {
    let (mut w, _, _) = window(true);
    let first = drawn(&mut w)[0];
    let narrow = w.state().hit.rect("message", first).unwrap().height();
    assert!(narrow > 60.0, "two lines and a preview beside the reading pane: {narrow}");
    w.state_mut().execute("view.readingPane", json!({"value": "off"})).unwrap();
    w.run_steps(3);
    let first = drawn(&mut w)[0];
    let wide = w.state().hit.rect("message", first).unwrap().height();
    assert!((wide - 48.0).abs() < 0.5, "one line in the whole width: {wide}");
    // Its picture, its sender, its subject and preview, its time on that one line.
    let sender = w.state().hit.rect("messageSender", first).unwrap();
    let row = w.state().hit.rect("message", first).unwrap();
    assert!((sender.center().y - row.center().y).abs() < 1.0);
}

/// Every state of the list of variant D in French, light and dark, to PNG files in `VARIANT_D_SHOTS`: the Inbox at rest, a row
/// under the pointer, a day's title under the pointer, the view's menu, « Toutes les boîtes », « Afficher les traités » (Nora's bar
/// and what she proposes: nora/ask_bar_tests.rs).
#[test]
#[ignore = "draws to PNG files: VARIANT_D_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored variant_d_screens"]
fn variant_d_screens_in_french() {
    let Some(dir) = std::env::var_os("VARIANT_D_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        for screen in [
            "1-boite",
            "2-ligne-survolee",
            "3-jour-survole",
            "4-menu-de-la-vue",
            "5-toutes-les-boites",
            "6-traites-affiches",
            "11-groupes-de-principal",
            "12-groupe-ouvert-traites",
            "13-une-ligne",
        ] {
            let (mut w, _, _) = if screen.starts_with("11") || screen.starts_with("12") {
                let (w, _, _) = by_kind();
                (w, 0, 0)
            } else {
                window(true)
            };
            w.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
            let _ = w.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
            w.state_mut().ui.sender_images = true;
            w.run_steps(4);
            match screen {
                "2-ligne-survolee" => {
                    let id = drawn(&mut w)[1];
                    hover(&mut w, "message", id);
                }
                "3-jour-survole" => {
                    let rows = w.state_mut().session.rows();
                    let first = rows.iter().position(|r| r.group.is_some()).unwrap_or(0);
                    hover(&mut w, "doneGroup", first as u64);
                }
                "4-menu-de-la-vue" => click(&mut w, "listViewName", 0),
                "5-toutes-les-boites" => click(&mut w, "allInboxes", 0),
                "6-traites-affiches" => {
                    click(&mut w, "allInboxes", 0);
                    let id = drawn(&mut w)[0];
                    hover(&mut w, "message", id);
                    click(&mut w, "doneRound", id);
                    click(&mut w, "listShowDone", 0);
                }
                "12-groupe-ouvert-traites" => {
                    click(&mut w, "bundle", 1);
                    let id = drawn(&mut w)[0];
                    hover(&mut w, "message", id);
                    click(&mut w, "doneRound", id);
                }
                "13-une-ligne" => {
                    w.state_mut().execute("view.readingPane", json!({"value": "off"})).unwrap();
                    click(&mut w, "allInboxes", 0);
                }
                _ => {}
            }
            w.run_steps(4);
            let image = w.render().expect("drawn");
            image.save(dir.join(format!("{screen}-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
        }
    }
}
