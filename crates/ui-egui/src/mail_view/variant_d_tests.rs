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

/// Principal of `by_kind` with its mail dated: today a person's mail newer than both groups' newest (`late`) and one
/// older (`early`), the notifications' newest and the newsletters' newest in between; the rest two days before.
fn by_kind_dated() -> (testing::Window, Id, Id) {
    let (mut w, notes, letters) = by_kind();
    let rows = w.state_mut().session.rows();
    let mb = &w.state().session.mailbox;
    let people: Vec<Id> = rows.iter().filter(|r| r.bundle.is_none() && !mb.message(r.id).unwrap().pinned).map(|r| r.id).collect();
    let now = testing::now();
    let (late, early) = (people[0], people[1]);
    let mut dates = vec![(late, now - 60), (early, now - 7_200), (notes[0], now - 600), (letters[0], now - 900)];
    dates.extend([notes[1], letters[1]].into_iter().chain(people[2..].iter().copied()).map(|id| (id, now - 2 * 86_400)));
    for (id, at) in dates {
        let m = w.state_mut().session.mailbox.message_mut(id).unwrap();
        m.date = at;
        m.received = at;
    }
    // (The list built again from them.)
    w.state_mut().session.mailbox_changed();
    w.run_steps(3);
    (w, late, early)
}

/// Where the rows of Principal are drawn, top to bottom: `message` rows by id, groups as 1 and 2 (`bundle`).
fn drawn_in_order(w: &mut testing::Window, late: Id, early: Id) -> Vec<&'static str> {
    let mut at: Vec<(f32, &'static str)> =
        [("message", late, "late"), ("message", early, "early"), ("bundle", 1, "notifications"), ("bundle", 2, "newsletters")]
            .into_iter()
            .filter_map(|(kind, id, name)| w.state().hit.rect(kind, id).map(|r| (r.min.y, name)))
            .collect();
    at.sort_by(|a, b| a.0.total_cmp(&b.0));
    at.into_iter().map(|(_, name)| name).collect()
}

#[test]
fn principals_groups_are_at_the_foot_of_their_day_below_its_mail_notifications_first() {
    let (mut w, late, early) = by_kind_dated();
    // The rows: today's mail (the newer and the older), then the notifications, then the newsletters, then the days before.
    let rows = w.state_mut().session.rows();
    let mb = &w.state().session.mailbox;
    let rows: Vec<_> = rows.iter().filter(|r| r.bundle.is_some() || !mb.message(r.id).unwrap().pinned).cloned().collect();
    let order: Vec<String> = rows
        .iter()
        .take(4)
        .map(|r| match r.bundle {
            Some(stream) => format!("{stream:?}"),
            None if r.id == late => "late".into(),
            None if r.id == early => "early".into(),
            None => format!("{}", r.id),
        })
        .collect();
    assert_eq!(order, ["late", "early", "Notification", "Newsletter"]);
    assert!(rows[0].group.is_some() && rows[1..4].iter().all(|r| r.group.is_none()), "one heading for the day, the groups under it");
    assert!(rows[4].group.is_some(), "the day before starts after them");
    // On screen, in two lines (the list beside the reading pane)…
    assert_eq!(drawn_in_order(&mut w, late, early), ["late", "early", "notifications", "newsletters"]);
    // …in one line (the list in the whole width)…
    w.state_mut().execute("view.readingPane", json!({"value": "off"})).unwrap();
    w.run_steps(4);
    assert!(w.state().hit.rect("message", late).unwrap().height() < 60.0, "one line");
    assert_eq!(drawn_in_order(&mut w, late, early), ["late", "early", "notifications", "newsletters"]);
    // …with « Afficher les traités »…
    click(&mut w, "listShowDone", 0);
    assert_eq!(drawn_in_order(&mut w, late, early), ["late", "early", "notifications", "newsletters"]);
    click(&mut w, "listShowDone", 1);
    // …and in « Toutes les boîtes ».
    click(&mut w, "allInboxes", 0);
    assert!(w.state().session.all_inboxes().is_some());
    let order = drawn_in_order(&mut w, late, early);
    assert_eq!(&order[order.len() - 2..], ["notifications", "newsletters"], "{order:?}");
    assert!(order.contains(&"late") && order.contains(&"early"));
}

#[test]
fn groups_of_different_days_are_each_at_the_foot_of_their_own_day() {
    let (mut w, notes, letters) = by_kind();
    let rows = w.state_mut().session.rows();
    let mb = &w.state().session.mailbox;
    let people: Vec<Id> = rows.iter().filter(|r| r.bundle.is_none() && !mb.message(r.id).unwrap().pinned).map(|r| r.id).collect();
    let now = testing::now();
    // Today a person's mail and the newsletters' newest; yesterday the notifications' newest, alone; the rest three days before.
    let mut dates = vec![(people[0], now - 60), (letters[0], now - 600), (notes[0], now - 86_400)];
    dates.extend([notes[1], letters[1]].into_iter().chain(people[1..].iter().copied()).map(|id| (id, now - 3 * 86_400)));
    for (id, at) in dates {
        let m = w.state_mut().session.mailbox.message_mut(id).unwrap();
        m.date = at;
        m.received = at;
    }
    w.state_mut().session.mailbox_changed();
    w.run_steps(3);
    let rows = w.state_mut().session.rows();
    let mb = &w.state().session.mailbox;
    let rows: Vec<_> = rows.iter().filter(|r| r.bundle.is_some() || !mb.message(r.id).unwrap().pinned).cloned().collect();
    let said: Vec<(String, Option<String>)> = rows
        .iter()
        .take(4)
        .map(|r| {
            (
                r.bundle.map_or_else(|| if r.id == people[0] { "person".to_string() } else { "older".to_string() }, |b| format!("{b:?}")),
                r.group.clone(),
            )
        })
        .collect();
    assert_eq!(
        said,
        [
            ("person".to_string(), Some("Today".to_string())),
            ("Newsletter".to_string(), None),
            ("Notification".to_string(), Some("Yesterday".to_string())),
            ("older".to_string(), said[3].1.clone()),
        ]
    );
    assert!(said[3].1.is_some() && said[3].1.as_deref() != Some("Yesterday"), "the day before that starts after them");
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

#[test]
fn new_senders_are_those_nothing_says_the_person_knows_and_accept_and_block_write_their_lists() {
    let (mut w, ada, team) = window(true);
    click(&mut w, "allInboxes", 0);
    let senders = super::new_senders::of(&w.state().session);
    let emails: Vec<String> = senders.iter().map(|s| s.who.email.clone()).collect();
    // The three who wrote to team@ today are new; a contact or someone written to isn't.
    for email in ["support@service.example", "atelier@nord.example", "marc@dupont.example"] {
        assert!(emails.iter().any(|e| e == email), "{email} in {emails:?}");
    }
    let known: Vec<String> = w.state().session.mailbox.contacts.values().flat_map(|c| c.emails.iter().map(|e| e.value.to_lowercase())).collect();
    assert!(emails.iter().all(|e| !known.contains(&e.to_lowercase())), "no contact");
    assert!(senders.iter().all(|s| s.account == ada || s.account == team));
    // The band, its cards.
    assert!(has(&w, "newSenders", 0));
    click(&mut w, "newSenders", 0);
    let at = |w: &testing::Window, email: &str| super::new_senders::of(&w.state().session).iter().position(|s| s.who.email == email).unwrap() as u64;
    let accept = at(&w, "support@service.example");
    click(&mut w, "newSenderAccept", accept);
    let lists = w.state_mut().execute("junk.list", json!({"account": team})).unwrap();
    assert!(lists["safe"].as_array().unwrap().iter().any(|e| e == "support@service.example"), "{lists}");
    let block = at(&w, "atelier@nord.example");
    let blocked: Vec<Id> = super::new_senders::of(&w.state().session)[block as usize].ids.clone();
    click(&mut w, "newSenderBlock", block);
    let lists = w.state_mut().execute("junk.list", json!({"account": team})).unwrap();
    assert!(lists["blocked"].as_array().unwrap().iter().any(|e| e == "atelier@nord.example"), "{lists}");
    for id in blocked {
        assert_eq!(folder_of(&w, id), (team, FolderKind::Junk));
    }
    let left: Vec<String> = super::new_senders::of(&w.state().session).iter().map(|s| s.who.email.clone()).collect();
    assert!(!left.iter().any(|e| e == "support@service.example" || e == "atelier@nord.example"), "{left:?}");
    // The last one handled, the cards close with the band, and a new sender later doesn't open them by itself.
    let marc = at(&w, "marc@dupont.example");
    click(&mut w, "newSenderAccept", marc);
    let rest: Vec<u64> = (0..super::new_senders::of(&w.state().session).len() as u64).collect();
    for _ in rest {
        click(&mut w, "newSenderAccept", 0);
    }
    assert!(!has(&w, "newSenders", 0) && !has(&w, "newSendersClose", 0));
    let inbox = w.state().session.mailbox.folder_of_kind(team, FolderKind::Inbox).unwrap();
    let when = testing::now() - 60;
    let m = Message {
        folder: inbox,
        from: Address::new("Nouvelle Cliente", "cliente@neuve.example"),
        subject: "Bonjour".into(),
        date: when,
        received: when,
        ..Default::default()
    };
    let new = w.state_mut().session.mailbox.add_message(m).unwrap();
    // (A change through a command, as mail coming in is: the list's caches follow `Session::rev`.)
    w.state_mut().run("mail.markRead", json!({"ids": [new]})).unwrap();
    w.run_steps(3);
    assert!(has(&w, "newSenders", 0) && !has(&w, "newSendersClose", 0), "the band, its cards closed");
    // Someone written to is no new sender.
    let sent = w.state().session.mailbox.folder_of_kind(team, FolderKind::Sent).unwrap();
    let to_marc =
        Message { folder: sent, to: vec![Address::new("Marc Dupont", "marc@dupont.example")], subject: "Re: jeudi".into(), ..Default::default() };
    w.state_mut().session.mailbox.add_message(to_marc).unwrap();
    assert!(!super::new_senders::of(&w.state().session).iter().any(|s| s.who.email == "marc@dupont.example"));
}

/// A group of Principal's row drawn alone, `width` wide (a narrow list beside Nora's pane), French: its count's and its chevron's
/// rectangles, and its name's.
fn narrow_group(width: f32, dark: bool, shot: Option<&std::path::Path>) -> (egui::Rect, egui::Rect, egui::Rect) {
    // (The caller holds the language's lock.)
    let (mut w, _, _) = by_kind();
    w.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
    let _ = w.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
    w.run_steps(3);
    let rows = w.state_mut().session.rows();
    let row = rows.iter().find(|r| r.bundle == Some(mailcraft_model::Stream::Notification)).unwrap().clone();
    let mut app = std::mem::replace(w.state_mut(), crate::MailApp::new(mailcraft_engine::Session::empty(), crate::Services::default()));
    app.hit = Default::default();
    // (Styled again in the new window: its fonts and its theme.)
    app.styled = false;
    app.applied_theme = None;
    let mut h =
        egui_kittest::Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(width, 60.0)).build_ui_state(
            move |ui, app: &mut crate::MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                let t = crate::theme::Tokens::get(&ctx);
                ui.painter().rect_filled(ui.max_rect(), 0.0, t.surface);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 72.0), egui::Sense::hover());
                super::bundle::row(app, ui, &row, rect, false);
            },
            app,
        );
    h.run_steps(3);
    if let Some(path) = shot {
        h.render().expect("drawn").save(path).expect("saved");
    }
    let hit = |kind: &str| h.state().hit.rect(kind, 1).unwrap_or_else(|| panic!("no {kind}"));
    let row_rect = h.state().hit.rect("bundle", 1).unwrap();
    let count = hit("bundleCount");
    let chevron = hit("bundleChevron");
    (count, chevron, row_rect)
}

#[test]
fn a_groups_count_and_chevron_keep_their_room_in_a_narrow_list() {
    let _language = crate::i18n::RestoreLanguage::hold();
    for width in [200.0, 230.0, 300.0, 600.0] {
        let (count, chevron, row) = narrow_group(width, false, None);
        assert!(count.max.x + 2.0 <= chevron.min.x, "{width}: the count {count:?} before the chevron {chevron:?}");
        assert!(chevron.max.x <= row.max.x, "{width}");
    }
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
            "18-nouveaux-expediteurs",
            "19-groupe-etroit-230",
        ] {
            if screen.starts_with("19") {
                let name = format!("{screen}-{}.png", if dark { "sombre" } else { "clair" });
                narrow_group(230.0, dark, Some(&dir.join(name)));
                continue;
            }
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
                "18-nouveaux-expediteurs" => {
                    click(&mut w, "allInboxes", 0);
                    click(&mut w, "newSenders", 0);
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

/// The window `width` × 900 on the two mailboxes, as a hosting page's tab (`hosted`) or the desktop shows it, Ada's Inbox in view and
/// its first message read.
fn reading_at(width: f32, hosted: bool) -> testing::Window {
    let (mut w, _, _) = window(hosted);
    w.set_size(egui::vec2(width, 900.0));
    w.run_steps(3);
    let first = drawn(&mut w)[0];
    click(&mut w, "message", first);
    w.run_steps(3);
    w
}

/// Whether the Échanges pane stands, and then whether the list, the message and it each keep their room: the list's rows end before
/// the message's room, which keeps what a web message is laid out in, and the pane is inside the window.
fn pane_beside(w: &mut testing::Window, width: f32) -> bool {
    let Some(pane) = w.state().hit.rect("exchangesPane", 0) else { return false };
    let first = drawn(w)[0];
    let row = w.state().hit.rect("message", first).unwrap();
    assert!(pane.max.x <= width + 0.5, "{width}: the pane {pane:?} inside the window");
    assert!(pane.min.x - row.max.x >= crate::exchanges::READING_COMFORT - 1.0, "{width}: the message keeps its room between {row:?} and {pane:?}");
    true
}

#[test]
fn in_a_wide_tab_the_exchanges_open_by_themselves_and_go_when_it_narrows_or_the_person_closes_them() {
    // Only where the list, the message at a comfortable width and the pane all have room: not at 1280 nor 1440 (with the folder
    // pane), at 1920.
    for width in [1280.0, 1440.0] {
        let mut w = reading_at(width, true);
        assert!(!w.state().exchanges.open && !pane_beside(&mut w, width), "{width}: no room for it");
        let read = w.state().session.sel.focus().unwrap();
        assert!(has(&w, "exchangesStrip", read) || has(&w, "exchangesButton", read), "{width}: « Afficher l'historique » is there");
    }
    let mut w = reading_at(1920.0, true);
    assert!(w.state().exchanges.open && pane_beside(&mut w, 1920.0), "open by itself, beside the message");
    // It follows the message read.
    let second = drawn(&mut w)[1];
    click(&mut w, "message", second);
    assert!(w.state().exchanges.open && pane_beside(&mut w, 1920.0));
    // Narrower: it goes, and comes back when the room does.
    w.set_size(egui::vec2(1280.0, 900.0));
    w.run_steps(4);
    assert!(!w.state().exchanges.open);
    w.set_size(egui::vec2(1920.0, 900.0));
    w.run_steps(4);
    assert!(w.state().exchanges.open);
    // Closed by the person: closed it stays, at any width and after any message, until they open it again.
    w.state_mut().run("ui.exchanges", json!({"open": false})).unwrap();
    w.run_steps(3);
    assert!(!w.state().exchanges.open && w.state().ui.exchanges_declined, "the choice is kept with the settings");
    let first = drawn(&mut w)[0];
    click(&mut w, "message", first);
    assert!(!w.state().exchanges.open);
    let prefs = w.state().prefs();
    assert!(prefs.exchanges_declined, "kept between visits");
    w.state_mut().run("ui.exchanges", json!({"open": true})).unwrap();
    w.run_steps(3);
    assert!(w.state().exchanges.open && !w.state().ui.exchanges_declined);
    // Nothing read: an open pane of its own goes.
    w.state_mut().run("ui.exchanges", json!({"open": false})).unwrap();
    w.state_mut().ui.exchanges_declined = false;
    w.run_steps(3);
    assert!(w.state().exchanges.open, "open by itself again");
    w.state_mut().execute("mail.select", json!({"ids": []})).unwrap();
    w.run_steps(3);
    assert!(!w.state().exchanges.open, "nothing read");
    // The desktop keeps it closed until it is asked for.
    let w = reading_at(1920.0, false);
    assert!(!w.state().exchanges.open);
}

#[test]
fn in_a_hosting_pages_tab_every_row_has_its_round_clear_of_its_one_mark() {
    for hosted in [true, false] {
        let (mut w, _, _) = window(hosted);
        let rows = drawn(&mut w);
        assert!(!rows.is_empty());
        for id in &rows {
            let round = w.state().hit.rect("messageRound", *id);
            assert_eq!(round.is_some(), hosted, "{id}: a round in a hosting page's tab only (the desktop keeps upstream's switch)");
        }
        if !hosted {
            continue;
        }
        // The unread dot, and the round of « Traité » under the pointer, stand in the lead column, clear of it.
        let unread = rows.iter().copied().find(|id| has(&w, "messageDot", *id)).expect("an unread row");
        let round = w.state().hit.rect("messageRound", unread).unwrap();
        assert!(w.state().hit.rect("messageDot", unread).unwrap().max.x < round.min.x);
        hover(&mut w, "message", unread);
        let done = w.state().hit.rect("doneRound", unread).expect("« Traité » under the pointer");
        assert!(done.max.x <= round.min.x, "{done:?} clear of {round:?}");
        assert!(w.state().hit.rect("messageSender", unread).unwrap().min.x >= round.max.x, "the sender after it");
    }
}

/// A logo for the captures: a red square with a white bar, as a PNG.
fn logo() -> Vec<u8> {
    let mut image = image::RgbaImage::from_pixel(64, 64, image::Rgba([0xD0, 0x47, 0x3A, 255]));
    for x in 14..50 {
        for y in 28..36 {
            image.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// The four changes of the batch in French, light and dark, to PNG files in `NEOMAIL_BATCH_SHOTS`: the groups at the foot of their
/// day, the rounds of a hosting page's tab (initials, a known sender's logo), the Échanges pane by itself at 1920 and not at 1440.
#[test]
#[ignore = "draws to PNG files: NEOMAIL_BATCH_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored batch_screens"]
fn batch_screens_in_french() {
    let Some(dir) = std::env::var_os("NEOMAIL_BATCH_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        for screen in ["a-groupes-en-bas-du-jour", "b-ronds-et-logo", "c-echanges-1920", "d-echanges-1440", "e-echanges-1280"] {
            let mut w = match screen {
                "a-groupes-en-bas-du-jour" => by_kind_dated().0,
                "c-echanges-1920" => reading_at(1920.0, true),
                "d-echanges-1440" => reading_at(1440.0, true),
                "e-echanges-1280" => reading_at(1280.0, true),
                _ => window(true).0,
            };
            w.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
            let _ = w.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
            if screen == "b-ronds-et-logo" {
                // A sender the person knows, whose domain's site has an icon: the server's answer, here at once.
                let contact = mailcraft_model::Contact {
                    display_name: "Service Exemple".into(),
                    emails: vec![mailcraft_model::Labeled::new("work", "support@service.example")],
                    ..Default::default()
                };
                w.state_mut().session.mailbox.add_contact(contact);
                w.state_mut().session.mailbox_changed();
                w.state_mut().services.sender_images = Some(Box::new(|domain: String, _| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let _ = tx.send(if domain == "service.example" { Ok(logo()) } else { Err("none".into()) });
                    rx
                }));
                click(&mut w, "allInboxes", 0);
            }
            w.run_steps(8);
            let image = w.render().expect("drawn");
            image.save(dir.join(format!("{screen}-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
        }
    }
}

/// What a row of the list shows at its left: its round (none in a narrow list), its sender's name, its one mark (the unread dot) and,
/// under the pointer, the round of « Traité »; and the row.
struct Left {
    row: egui::Rect,
    round: Option<egui::Rect>,
    sender: egui::Rect,
    dot: Option<egui::Rect>,
    /// The ring of its mailbox's colour round the round, or the stripe down its left edge where there is none.
    stripe: Option<egui::Rect>,
    done: Option<egui::Rect>,
}

/// The list of a hosting page's tab drawn alone, `width` wide (its rows of two lines), French: Ada's Inbox (« Toutes les boîtes »
/// where `all`), with unread mail from « Ventes » and from a long name at its top; those two rows' left (`Left`), « Traité » with the
/// pointer over each.
fn narrow_list(width: f32, dark: bool, all: bool, shot: Option<&std::path::Path>) -> Vec<Left> {
    let (mut w, _, _) = window(true);
    if all {
        click(&mut w, "allInboxes", 0);
    }
    let rows = w.state_mut().session.rows();
    let mb = &w.state().session.mailbox;
    let mut ids: Vec<Id> = rows.iter().filter(|r| r.bundle.is_none() && !mb.message(r.id).unwrap().pinned).map(|r| r.id).collect();
    ids.truncate(2);
    let now = testing::now();
    for (i, (id, (name, address))) in ids
        .iter()
        .zip([("Ventes", "ventes@shop.example"), ("Ventes et administration des ventes internationales", "admin@ventes.example")])
        .enumerate()
    {
        let m = w.state_mut().session.mailbox.message_mut(*id).unwrap();
        m.from = Address::new(name, address);
        m.read = false;
        m.subject = "Question sur la commande".into();
        m.date = now - 60 - i as i64 * 60;
        m.received = m.date;
    }
    w.state_mut().session.mailbox_changed();
    w.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
    let _ = w.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
    w.run_steps(3);
    let mut app = std::mem::replace(w.state_mut(), crate::MailApp::new(mailcraft_engine::Session::empty(), crate::Services::default()));
    app.hit = Default::default();
    // (Styled again in the new window: its fonts and its theme. With the scroll bar's lane, as Linux and Windows have it: the same
    // rows on every system.)
    app.styled = false;
    app.applied_theme = None;
    app.overlay_scroll_bars = false;
    let mut h = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(egui::vec2(width, 420.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut crate::MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.hit.clear();
                let t = crate::theme::Tokens::get(&ctx);
                ui.painter().rect_filled(ui.max_rect(), 0.0, t.surface);
                super::list(app, ui);
            },
            app,
        );
    h.run_steps(4);
    if let Some(path) = shot {
        h.render().expect("drawn").save(path).expect("saved");
    }
    // (All but « Traité » before the pointer is over a row: its dot turns into that round, and its quick actions take its right.)
    let mut left: Vec<Left> = ids
        .iter()
        .map(|id| {
            let hit = |kind: &str| h.state().hit.rect(kind, *id);
            let row = hit("message").unwrap_or_else(|| panic!("{width}: row {id}"));
            Left {
                row,
                round: hit("messageRound"),
                sender: hit("messageSender").unwrap(),
                dot: hit("messageDot"),
                stripe: hit("messageAccountColor"),
                done: None,
            }
        })
        .collect();
    for (id, l) in ids.iter().zip(&mut left) {
        h.hover_at(l.row.center());
        h.run_steps(4);
        l.done = h.state().hit.rect("doneRound", *id);
    }
    left
}

#[test]
fn in_a_narrow_list_the_round_gives_its_room_back_to_the_name_and_the_one_mark_stays() {
    let _language = crate::i18n::RestoreLanguage::hold();
    for width in [220.0, 260.0, 320.0] {
        let rows = narrow_list(width, false, false, None);
        for (i, r) in rows.iter().enumerate() {
            let lead = r.row.min.x + super::done::D_LEAD;
            // The one mark: the unread dot, and under the pointer the round of « Traité », in the lead column at every width.
            assert!(r.dot.is_some_and(|d| d.max.x <= lead + 0.5), "{width} {i}: the unread dot in the lead column: {:?}", r.dot);
            assert!(r.done.is_some_and(|d| d.max.x <= lead + 0.5), "{width} {i}: « Traité » in the lead column: {:?}", r.done);
            match r.round {
                // A list about 300 wide and more: the round, after the lead column, the name after it.
                Some(round) => {
                    assert!(width >= 300.0, "{width} {i}: no round in a list this narrow");
                    assert!(round.min.x >= lead - 0.5 && r.sender.min.x >= round.max.x, "{width} {i}: {round:?} {:?}", r.sender);
                }
                // Narrower: the name starts where the round would have.
                None => {
                    assert!(width < 300.0, "{width} {i}: the round where there is room for it");
                    assert!((r.sender.min.x - lead).abs() < 1.0, "{width} {i}: the name at once after the lead column: {:?}", r.sender);
                }
            }
        }
        // More than a first letter of « Ventes » and of the long name.
        assert!(rows[0].sender.width() >= 30.0, "{width}: « Ventes » {:?}", rows[0].sender);
        assert!(rows[1].sender.width() >= 30.0, "{width}: the long name {:?}", rows[1].sender);
        // In « Toutes les boîtes », the mailbox's colour: a ring round the round, else the stripe down the row's left edge.
        for r in narrow_list(width, false, true, None) {
            let stripe = r.stripe.expect("its mailbox's colour");
            match r.round {
                Some(round) => assert!(stripe.contains(round.center()), "{width}: the ring {stripe:?} round {round:?}"),
                None => assert!(stripe.width() <= 4.0 && (stripe.min.x - r.row.min.x).abs() < 2.0, "{width}: the stripe {stripe:?}"),
            }
        }
    }
}

/// The narrow list at 220 and at 320, French, light and dark, to PNG files in `NEOMAIL_BATCH_SHOTS`.
#[test]
#[ignore = "draws to PNG files: NEOMAIL_BATCH_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored narrow_list_screens"]
fn narrow_list_screens_in_french() {
    let Some(dir) = std::env::var_os("NEOMAIL_BATCH_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        for width in [220.0, 260.0, 320.0] {
            let name = format!("g-liste-{width}-{}.png", if dark { "sombre" } else { "clair" });
            narrow_list(width, dark, false, Some(&dir.join(name)));
        }
    }
}
