// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of done.rs.
//! In the window, the Inbox of a hosting page's tab: a row's round marks it done (archived), a group's round the whole group,
//! « Marqués comme traités » keeps them with « Annuler » and « Tout annuler », a row's quick actions are Spark's, and nothing
//! of it shows where the window keeps its mail or outside the Inbox. The unread dot keeps clear of the box that ticks the row.
//! And (`--ignored`, with `DONE_SHOTS=<folder>`) the list drawn in French, light and dark, to PNG files.

use egui::PointerButton;
use mailcraft_model::{FolderKind, Id};
use serde_json::json;

use super::{group_ids, mark, undo, undo_all};
use crate::people_view::testing;

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

fn folder_kind(w: &testing::Window, id: Id) -> Option<FolderKind> {
    let s = &w.state().session;
    s.mailbox.message(id).and_then(|m| s.mailbox.folder(m.folder)).map(|f| f.kind)
}

/// The window on the sample's Inbox, as a hosting page's tab shows it (`hosted`) or as the program that keeps its mail.
fn inbox(hosted: bool) -> testing::Window {
    let mut w = testing::window("view.mail");
    w.state_mut().session.keeps_local = !hosted;
    w.state_mut().execute("ui.dismissReminder", json!({"all": true})).unwrap();
    w.state_mut().execute("folder.select", json!({"kind": "inbox"})).unwrap();
    w.run_steps(3);
    w
}

/// Whether a group's heading wears a round.
fn any_group_round(w: &mut testing::Window) -> bool {
    let rows = w.state_mut().session.rows().len();
    (0..rows as u64).any(|i| has(w, "doneGroup", i))
}

/// The rows the window drew.
fn drawn(w: &mut testing::Window) -> Vec<Id> {
    let rows = w.state_mut().session.rows();
    rows.iter().filter(|row| w.state().hit.rect("message", row.id).is_some()).map(|row| row.id).collect()
}

#[test]
fn a_rows_round_marks_it_done_archived_and_the_section_keeps_it() {
    let mut w = inbox(true);
    let first = drawn(&mut w)[0];
    assert!(!has(&w, "doneRound", first), "no round at rest");
    hover(&mut w, "message", first);
    assert!(has(&w, "doneRound", first));
    click(&mut w, "doneRound", first);
    assert_eq!(folder_kind(&w, first), Some(FolderKind::Archive));
    assert_eq!(w.state().done.items.iter().map(|i| i.id).collect::<Vec<_>>(), [first]);
    assert!(has(&w, "doneSection", 0));
    assert!(!drawn(&mut w).contains(&first));
}

#[test]
fn a_groups_round_marks_the_whole_group_done() {
    let mut w = inbox(true);
    let rows = w.state_mut().session.rows();
    let first = rows.iter().position(|r| r.group.is_some()).expect("a group");
    let ids = group_ids(&rows, first);
    assert!(!ids.is_empty());
    assert!(has(&w, "doneGroup", first as u64), "each heading wears its round");
    click(&mut w, "doneGroup", first as u64);
    assert!(ids.iter().all(|id| folder_kind(&w, *id) == Some(FolderKind::Archive)));
    assert_eq!(w.state().done.items.len(), ids.len());
    // The group's rows only: the next group stays.
    let next = rows.iter().enumerate().skip(first + 1).find(|(_, r)| r.group.is_some()).map(|(i, _)| i);
    if let Some(next) = next {
        assert!(rows[next].members.iter().all(|id| folder_kind(&w, *id) == Some(FolderKind::Inbox)));
    }
}

#[test]
fn undo_puts_one_back_undo_all_the_rest_and_the_section_empties() {
    let mut w = inbox(true);
    let ids: Vec<Id> = drawn(&mut w).into_iter().take(3).collect();
    assert!(mark(w.state_mut(), &ids));
    assert_eq!(w.state().done.items.len(), 3);
    // Folded at first: its heading opens it.
    w.run_steps(3);
    assert!(!has(&w, "doneUndo", ids[0]));
    let fold = w.state().hit.rect("doneSection", 0).unwrap().min + egui::vec2(60.0, 15.0);
    testing::click(&mut w, fold, PointerButton::Primary, 1);
    w.run_steps(3);
    assert!(w.state().done.open && ids.iter().all(|id| has(&w, "doneUndo", *id)));
    click(&mut w, "doneUndo", ids[1]);
    assert_eq!(folder_kind(&w, ids[1]), Some(FolderKind::Inbox));
    assert_eq!(w.state().done.items.len(), 2);
    click(&mut w, "doneUndoAll", 0);
    assert!(ids.iter().all(|id| folder_kind(&w, *id) == Some(FolderKind::Inbox)));
    assert!(w.state().done.items.is_empty());
    assert!(!has(&w, "doneSection", 0), "the section goes once it is empty");
    // Directly too: undo of one not in it, undo all of nothing.
    undo(w.state_mut(), ids[0]);
    undo_all(w.state_mut());
    assert!(w.state().done.items.is_empty());
}

#[test]
fn a_rows_quick_actions_are_spark_s() {
    let mut w = inbox(true);
    // One not pinned: pinned, it comes to the top, still in view.
    let listed = drawn(&mut w);
    let first = listed.into_iter().find(|id| w.state().session.mailbox.message(*id).is_some_and(|m| !m.pinned)).expect("a message not pinned");
    hover(&mut w, "message", first);
    for kind in ["doneQuickRead", "doneQuickPin", "doneQuickFlag", "doneQuickMove", "doneQuickDelete"] {
        assert!(has(&w, kind, first), "{kind}");
    }
    assert!(!has(&w, "messageDelete", first) && !has(&w, "messageFlag", first), "in place of upstream's two");
    // Pin and flag change it (the sample's first may be pinned or flagged already).
    let pinned = |w: &testing::Window| w.state().session.mailbox.message(first).is_some_and(|m| m.pinned);
    let flagged = |w: &testing::Window| w.state().session.mailbox.message(first).is_some_and(|m| m.flag == mailcraft_model::FlagStatus::Flagged);
    let (was_pinned, was_flagged) = (pinned(&w), flagged(&w));
    click(&mut w, "doneQuickPin", first);
    assert_eq!(pinned(&w), !was_pinned);
    hover(&mut w, "message", first);
    click(&mut w, "doneQuickFlag", first);
    assert_eq!(flagged(&w), !was_flagged);
    hover(&mut w, "message", first);
    click(&mut w, "doneQuickDelete", first);
    assert_eq!(folder_kind(&w, first), Some(FolderKind::Deleted));
}

#[test]
fn nothing_of_it_where_the_window_keeps_its_mail_nor_outside_the_inbox() {
    let mut w = inbox(false);
    let first = drawn(&mut w)[0];
    hover(&mut w, "message", first);
    assert!(!has(&w, "doneRound", first) && !has(&w, "doneQuickPin", first));
    assert!(has(&w, "messageDelete", first), "upstream's quick actions");
    assert!(!any_group_round(&mut w));
    // A hosting page's tab, its Sent Items in view.
    let mut w = inbox(true);
    w.state_mut().execute("folder.select", json!({"kind": "sent"})).unwrap();
    w.run_steps(3);
    if let Some(first) = drawn(&mut w).first().copied() {
        hover(&mut w, "message", first);
        assert!(!has(&w, "doneRound", first));
    }
    assert!(!any_group_round(&mut w));
}

#[test]
fn the_unread_dot_keeps_clear_of_the_box_that_ticks_the_row_at_every_density() {
    for (one_line, tight) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut w = inbox(true);
        let ids: Vec<Id> = w.state_mut().session.rows().iter().map(|r| r.id).collect();
        let streams: Vec<serde_json::Value> = ids.iter().map(|id| json!({"id": id, "stream": "person"})).collect();
        w.state_mut().execute("mail.setStreams", json!({"streams": streams})).unwrap();
        w.state_mut().session.prefs.one_line = one_line;
        w.state_mut().session.prefs.tight_spacing = tight;
        w.run_steps(3);
        let listed = drawn(&mut w);
        let unread: Vec<Id> = listed.iter().copied().filter(|id| has(&w, "messageDot", *id)).collect();
        assert!(!unread.is_empty(), "an unread message in the sample");
        for id in unread {
            let (dot, tick) = (w.state().hit.rect("messageDot", id).unwrap(), w.state().hit.rect("messageTick", id).unwrap());
            assert!(dot.max.x <= tick.min.x - 2.0, "{one_line} {tight}: the dot {dot:?} on the box {tick:?}");
        }
    }
}

/// The list in French, light and dark, to PNG files in `DONE_SHOTS`: at rest, a row under the pointer, a group's heading
/// under the pointer, and « Marqués comme traités » open.
#[test]
#[ignore = "draws to PNG files: DONE_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored done_screens"]
fn done_screens_in_french() {
    let Some(dir) = std::env::var_os("DONE_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        for screen in ["1-liste", "2-ligne-survolee", "3-groupe-survole", "4-marques-comme-traites"] {
            let mut w = inbox(true);
            w.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
            let _ = w.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
            w.run_steps(4);
            let listed = drawn(&mut w);
            match screen {
                "2-ligne-survolee" => hover(&mut w, "message", listed[1]),
                "3-groupe-survole" => {
                    let rows = w.state_mut().session.rows();
                    let first = rows.iter().position(|r| r.group.is_some()).unwrap_or(0);
                    hover(&mut w, "doneGroup", first as u64);
                }
                "4-marques-comme-traites" => {
                    let done: Vec<Id> = listed.iter().copied().take(3).collect();
                    mark(w.state_mut(), &done);
                    w.state_mut().done.open = true;
                    w.run_steps(4);
                }
                _ => {}
            }
            let image = w.render().expect("drawn");
            image.save(dir.join(format!("{screen}-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
        }
    }
}

#[test]
fn the_inbox_opened_from_favorites_has_it_too() {
    let mut w = inbox(true);
    let inbox_id = w.state().session.view.folder;
    // The Inbox under Favorites (it is the same folder): opened from there, a row's round and a group's.
    if !mailcraft_engine::cmd::folder::favorites(&w.state().session).contains(&inbox_id) {
        w.state_mut().execute("folder.favorite", json!({"id": inbox_id})).unwrap();
    }
    w.state_mut().execute("folder.select", json!({"kind": "sent"})).unwrap();
    w.run_steps(3);
    click(&mut w, "folder", inbox_id);
    assert_eq!(w.state().session.view.folder, inbox_id);
    let first = drawn(&mut w)[0];
    hover(&mut w, "message", first);
    assert!(has(&w, "doneRound", first));
    assert!(any_group_round(&mut w));
}
