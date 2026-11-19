// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of done.rs.
//! In the window, the Inbox of a hosting page's tab: a row's round marks it done (archived), a group's round the whole group,
//! « Marqués comme traités » keeps them with « Annuler » and « Tout annuler », a row's quick actions are Spark's, and nothing
//! of it shows where the window keeps its mail or outside the Inbox. In the list of variant D a row wears one mark, the unread dot,
//! which the round takes the place of under the pointer; a click selects, ⌘/Ctrl and Shift add; « Afficher les traités » lists the
//! Archive beside the Inbox, each with its round full, which puts it back.
//! The section shows what was marked done today; what a host puts back at the next visit (each message found again in its
//! account's Archive) shows there as it did, and each « Annuler » and « Tout annuler » puts it back where it was.
//! Backspace, which the round's words name, marks done what it archives in any folder of such a tab, and the section shows it in the
//! folder it came from, and in the Inbox: each « Annuler » puts it back there.
//! And (`--ignored`, with `DONE_SHOTS=<folder>`) the list drawn in French, light and dark, to PNG files.

use egui::PointerButton;
use mailcraft_model::{FolderKind, Id};
use serde_json::json;

use super::{Item, group_ids, mark, undo, undo_all};
use crate::people_view::testing;

fn has(w: &testing::Window, kind: &str, id: u64) -> bool {
    w.state().hit.rect(kind, id).is_some()
}

fn click(w: &mut testing::Window, kind: &str, id: u64) {
    let pos = w.state().hit.rect(kind, id).unwrap_or_else(|| panic!("no {kind} {id} in the window")).center();
    testing::click(w, pos, PointerButton::Primary, 1);
    w.run_steps(3);
}

/// A click at `pos` with `modifiers` held (⌘/Ctrl, Shift).
fn click_with(w: &mut testing::Window, pos: egui::Pos2, modifiers: egui::Modifiers) {
    w.run_steps(45);
    w.input_mut().events.push(egui::Event::PointerMoved(pos));
    w.step();
    w.input_mut().events.push(egui::Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        w.input_mut().events.push(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers });
        w.step();
    }
    w.input_mut().events.push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    w.step();
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
    // In the list of variant D, open: each of them with its round, joined by the rail.
    w.run_steps(3);
    assert!(ids.iter().all(|id| has(&w, "doneUndo", *id)));
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
fn a_row_wears_one_mark_the_unread_dot_which_the_round_takes_the_place_of_under_the_pointer_at_every_density() {
    for (one_line, tight) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut w = inbox(true);
        // (With the tabs by kind of sender, where the rows had a box to tick.)
        let ids: Vec<Id> = w.state_mut().session.rows().iter().map(|r| r.id).collect();
        let streams: Vec<serde_json::Value> = ids.iter().map(|id| json!({"id": id, "stream": "person"})).collect();
        w.state_mut().execute("mail.setStreams", json!({"streams": streams})).unwrap();
        w.state_mut().session.prefs.one_line = one_line;
        w.state_mut().session.prefs.tight_spacing = tight;
        w.run_steps(3);
        let listed = drawn(&mut w);
        let unread: Vec<Id> = listed.iter().copied().filter(|id| has(&w, "messageDot", *id)).collect();
        assert!(!unread.is_empty(), "an unread message in the sample");
        for id in &unread {
            let (dot, row) = (w.state().hit.rect("messageDot", *id).unwrap(), w.state().hit.rect("message", *id).unwrap());
            assert!(!has(&w, "messageTick", *id), "{one_line} {tight}: no box to tick");
            assert!(!has(&w, "messageRead", *id), "{one_line} {tight}: the lead column is the round's, not a read switch");
            assert!((dot.center().x - (row.min.x + super::D_LEAD / 2.0)).abs() < 0.5, "{one_line} {tight}: the dot {dot:?} in the lead column");
        }
        // Under the pointer the round takes the dot's place.
        let id = unread[0];
        let dot = w.state().hit.rect("messageDot", id).unwrap();
        hover(&mut w, "message", id);
        assert!(!has(&w, "messageDot", id), "{one_line} {tight}: the dot gives way");
        let round = w.state().hit.rect("doneRound", id).expect("the round");
        assert!((round.center() - dot.center()).length() < 0.5, "{one_line} {tight}: the round {round:?} where the dot {dot:?} was");
    }
}

#[test]
fn a_click_on_a_row_selects_it_and_command_and_shift_add_to_the_selection() {
    let mut w = inbox(true);
    let listed = drawn(&mut w);
    assert!(listed.len() >= 4);
    click(&mut w, "messageSender", listed[0]);
    assert_eq!(w.state().session.sel.messages, [listed[0]]);
    let at = w.state().hit.rect("messageSender", listed[2]).unwrap().center();
    click_with(&mut w, at, egui::Modifiers::COMMAND);
    w.run_steps(3);
    let mut picked = w.state().session.sel.messages.clone();
    picked.sort_unstable();
    let mut wanted = vec![listed[0], listed[2]];
    wanted.sort_unstable();
    assert_eq!(picked, wanted, "⌘/Ctrl adds one");
    let at = w.state().hit.rect("messageSender", listed[3]).unwrap().center();
    click_with(&mut w, at, egui::Modifiers::SHIFT);
    w.run_steps(3);
    assert!(w.state().session.sel.messages.len() >= 3, "Shift adds the range");
    // And everything, from the arrangement's menu or its key.
    w.state_mut().run("mail.selectAll", json!({})).unwrap();
    assert_eq!(w.state().session.sel.messages.len(), w.state_mut().session.rows().iter().filter(|r| r.copy == 0 && r.under.is_none()).count());
}

#[test]
fn show_done_lists_the_archive_beside_the_inbox_and_its_full_round_puts_one_back() {
    let mut w = inbox(true);
    let first = drawn(&mut w)[0];
    hover(&mut w, "message", first);
    click(&mut w, "doneRound", first);
    assert_eq!(folder_kind(&w, first), Some(FolderKind::Archive));
    assert!(!drawn(&mut w).contains(&first));
    // « Afficher les traités »: the header's switch.
    assert!(has(&w, "listShowDone", 0));
    click(&mut w, "listShowDone", 0);
    assert!(w.state().session.view.show_done);
    assert!(drawn(&mut w).contains(&first), "listed again, as done");
    assert!(has(&w, "doneListed", first), "its round, full");
    assert!(!has(&w, "messageDot", first));
    click(&mut w, "doneListed", first);
    assert_eq!(folder_kind(&w, first), Some(FolderKind::Inbox), "back in the Inbox");
    assert!(!has(&w, "doneListed", first));
    // Off again.
    click(&mut w, "listShowDone", 1);
    assert!(!w.state().session.view.show_done);
}

/// What a host keeps of `ids`, marked done in `w`: each message's Message-ID, the folder it was in, when, and what its row said.
fn kept_of(w: &testing::Window, ids: &[Id]) -> Vec<(String, Id, i64, String, String)> {
    let items = w.state().done_items();
    ids.iter()
        .map(|id| {
            let item = items.iter().find(|i| i.id == *id).expect("marked done");
            let m = w.state().session.mailbox.message(*id).expect("there");
            (m.message_id.clone(), item.from, item.at, item.who.clone(), item.subject.clone())
        })
        .collect()
}

/// The next visit, as a host puts back what was marked done: the window on the mailbox as the server has it (the messages
/// of `kept` in their account's Archive), each found again there by its Message-ID.
fn next_visit(kept: &[(String, Id, i64, String, String)]) -> testing::Window {
    let mut w = inbox(true);
    let archive = {
        let s = &w.state().session;
        let account = s.mailbox.folder(s.view.folder).expect("the Inbox").account;
        s.mailbox.folder_of_kind(account, FolderKind::Archive).expect("an Archive")
    };
    let mut items = Vec::new();
    for (message_id, from, at, who, subject) in kept {
        let id = w.state().session.mailbox.messages.values().find(|m| m.message_id == *message_id).map(|m| m.id).expect("found again");
        w.state_mut().session.mailbox.message_mut(id).expect("there").folder = archive;
        items.push(Item { id, message_id: message_id.clone(), from: *from, at: *at, who: who.clone(), subject: subject.clone() });
    }
    w.state_mut().session.mailbox_changed();
    w.run_steps(3);
    assert!(!has(&w, "doneSection", 0), "nothing marked done here yet");
    w.state_mut().set_done_items(items);
    w.run_steps(3);
    w
}

#[test]
fn the_section_shows_what_was_marked_done_today_and_undo_all_puts_back_that_alone() {
    let mut w = inbox(true);
    let ids: Vec<Id> = drawn(&mut w).into_iter().take(3).collect();
    assert!(mark(w.state_mut(), &ids));
    assert!(w.state().done_items().iter().all(|i| i.at == w.state().session.now()), "marked now");
    // The first was marked yesterday (a host put it back so, from the last visit).
    let mut items = w.state().done_items().to_vec();
    let yesterday = w.state().session.now() - 86_400;
    items.iter_mut().find(|i| i.id == ids[0]).expect("marked").at = yesterday;
    w.state_mut().set_done_items(items);
    w.run_steps(3);
    assert!(!has(&w, "doneUndo", ids[0]), "yesterday's is not in today's section");
    assert!(has(&w, "doneUndo", ids[1]) && has(&w, "doneUndo", ids[2]));
    click(&mut w, "doneUndoAll", 0);
    assert_eq!(folder_kind(&w, ids[0]), Some(FolderKind::Archive), "yesterday's stays done");
    assert!(ids[1..].iter().all(|id| folder_kind(&w, *id) == Some(FolderKind::Inbox)));
    assert!(!has(&w, "doneSection", 0), "nothing of today left: no section");
    // Nor in the list as upstream draws it.
    undo_all(w.state_mut());
    assert_eq!(folder_kind(&w, ids[0]), Some(FolderKind::Archive));
}

#[test]
fn what_a_host_puts_back_at_the_next_visit_shows_and_each_undo_puts_it_back_where_it_was() {
    let mut w = inbox(true);
    let ids: Vec<Id> = drawn(&mut w).into_iter().take(3).collect();
    assert!(mark(w.state_mut(), &ids));
    let kept = kept_of(&w, &ids);
    let mut next = next_visit(&kept);
    let back: Vec<Id> = next.state().done_items().iter().map(|i| i.id).collect();
    assert_eq!(back.len(), 3);
    assert!(back.iter().all(|id| has(&next, "doneUndo", *id)), "each in the section, as it was");
    assert!(has(&next, "doneSection", 0) && has(&next, "doneUndoAll", 0));
    click(&mut next, "doneUndo", back[1]);
    assert_eq!(folder_kind(&next, back[1]), Some(FolderKind::Inbox), "« Annuler »: back where it was");
    assert_eq!(next.state().done_items().len(), 2);
    click(&mut next, "doneUndoAll", 0);
    assert!(back.iter().all(|id| folder_kind(&next, *id) == Some(FolderKind::Inbox)), "« Tout annuler »: all back");
    assert!(next.state().done_items().is_empty() && !has(&next, "doneSection", 0));
}

#[test]
fn a_message_back_in_a_folder_or_gone_leaves_the_section() {
    let mut w = inbox(true);
    let ids: Vec<Id> = drawn(&mut w).into_iter().take(3).collect();
    assert!(mark(w.state_mut(), &ids));
    w.run_steps(3);
    assert!(ids.iter().all(|id| has(&w, "doneUndo", *id)));
    let inbox_folder = w.state().session.view.folder;
    // Put back from elsewhere (another window, the phone), as a run brings it; and gone from the mailbox.
    w.state_mut().session.mailbox.message_mut(ids[0]).unwrap().folder = inbox_folder;
    w.state_mut().session.mailbox.messages.remove(&ids[1]);
    w.state_mut().session.mailbox_changed();
    w.run_steps(3);
    assert!(!has(&w, "doneUndo", ids[0]) && !has(&w, "doneUndo", ids[1]), "they no longer show");
    assert!(has(&w, "doneUndo", ids[2]) && has(&w, "doneSection", 0), "the one still done does");
    click(&mut w, "doneUndoAll", 0);
    assert_eq!(folder_kind(&w, ids[2]), Some(FolderKind::Inbox));
    assert!(!has(&w, "doneSection", 0), "nothing left to show");
}

/// The list in French, light and dark, to PNG files in `DONE_SHOTS`: at rest, a row under the pointer, a group's heading
/// under the pointer, « Marqués comme traités » open, and the same at the next visit, as a host puts it back.
#[test]
#[ignore = "draws to PNG files: DONE_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored done_screens"]
fn done_screens_in_french() {
    let Some(dir) = std::env::var_os("DONE_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        for screen in ["1-liste", "2-ligne-survolee", "3-groupe-survole", "4-marques-comme-traites", "5-apres-rechargement"] {
            let mut w = if screen == "5-apres-rechargement" {
                // Marked done at the last visit, put back by the host at this one.
                let mut first = inbox(true);
                let done: Vec<Id> = drawn(&mut first).into_iter().take(3).collect();
                mark(first.state_mut(), &done);
                next_visit(&kept_of(&first, &done))
            } else {
                inbox(true)
            };
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

/// The folder « Receipts » of the sample, with the first two messages of the Inbox moved there, in view: the window and the folder.
fn receipts(w: &mut testing::Window) -> (Id, Vec<Id>) {
    let moved: Vec<Id> = drawn(w).into_iter().take(2).collect();
    let folder = w.state().session.mailbox.folders.iter().find(|f| f.name == "Receipts").map(|f| f.id).expect("the sample's Receipts");
    w.state_mut().execute("mail.move", json!({"ids": moved, "folder": folder})).unwrap();
    w.state_mut().execute("folder.select", json!({"id": folder})).unwrap();
    w.run_steps(3);
    (folder, moved)
}

/// Backspace on the selection, the key the round's words name.
fn backspace_on(w: &mut testing::Window, id: Id) {
    click(w, "messageSender", id);
    assert_eq!(w.state().session.sel.messages, [id]);
    testing::key(w, egui::Key::Backspace);
    w.run_steps(3);
}

#[test]
fn backspace_marks_done_what_it_archives_in_the_inbox_as_the_rounds_words_say() {
    let mut w = inbox(true);
    let first = drawn(&mut w)[0];
    backspace_on(&mut w, first);
    assert_eq!(folder_kind(&w, first), Some(FolderKind::Archive), "archived");
    assert_eq!(w.state().done.items.iter().map(|i| i.id).collect::<Vec<_>>(), [first], "and kept as done");
    assert!(has(&w, "doneUndo", first), "in the section, which puts it back");
    click(&mut w, "doneUndo", first);
    assert_eq!(folder_kind(&w, first), Some(FolderKind::Inbox));
}

#[test]
fn backspace_in_another_folder_marks_it_done_and_the_section_there_and_the_inboxs_put_it_back_where_it_was() {
    let mut w = inbox(true);
    let (folder, moved) = receipts(&mut w);
    let here = |w: &testing::Window, id: Id| w.state().session.mailbox.message(id).map(|m| m.folder);
    // Backspace in « Receipts »: archived, kept as done with the folder it was in, and shown here.
    backspace_on(&mut w, moved[0]);
    assert_eq!(folder_kind(&w, moved[0]), Some(FolderKind::Archive));
    let item = w.state().done.items.iter().find(|i| i.id == moved[0]).cloned().expect("kept as done");
    assert_eq!(item.from, folder, "with the folder it was in");
    assert!(has(&w, "doneSection", 0) && has(&w, "doneUndo", moved[0]), "the section shows it in the folder it left");
    click(&mut w, "doneUndo", moved[0]);
    assert_eq!(here(&w, moved[0]), Some(folder), "« Annuler »: back in « Receipts »");
    assert!(!has(&w, "doneSection", 0));
    // Done again, then put back from the Inbox's section, where everything done today shows.
    backspace_on(&mut w, moved[1]);
    assert_eq!(folder_kind(&w, moved[1]), Some(FolderKind::Archive));
    w.state_mut().execute("folder.select", json!({"kind": "inbox"})).unwrap();
    w.run_steps(3);
    assert!(has(&w, "doneUndo", moved[1]), "the Inbox's section has it too");
    click(&mut w, "doneUndoAll", 0);
    assert_eq!(here(&w, moved[1]), Some(folder), "« Tout annuler »: back in « Receipts », not in the Inbox");
}

#[test]
fn what_is_done_elsewhere_shows_in_the_folder_it_came_from_and_the_inbox_alone() {
    let mut w = inbox(true);
    let (folder, moved) = receipts(&mut w);
    backspace_on(&mut w, moved[0]);
    // Sent Items: nothing of it.
    w.state_mut().execute("folder.select", json!({"kind": "sent"})).unwrap();
    w.run_steps(3);
    assert!(!has(&w, "doneSection", 0) && !has(&w, "doneUndo", moved[0]));
    // Done in the Inbox: shown there, not in « Receipts ».
    w.state_mut().execute("folder.select", json!({"kind": "inbox"})).unwrap();
    w.run_steps(3);
    let first = drawn(&mut w)[0];
    backspace_on(&mut w, first);
    w.state_mut().execute("folder.select", json!({"id": folder})).unwrap();
    w.run_steps(3);
    assert!(has(&w, "doneUndo", moved[0]) && !has(&w, "doneUndo", first), "« Receipts » shows its own");
}

#[test]
fn where_the_window_keeps_its_mail_backspace_archives_as_upstream_has_it() {
    let mut w = inbox(false);
    let first = drawn(&mut w)[0];
    backspace_on(&mut w, first);
    assert_eq!(folder_kind(&w, first), Some(FolderKind::Archive));
    assert!(w.state().done.items.is_empty(), "nothing kept as done");
}

#[test]
fn a_message_in_plus_tard_is_archived_as_before_not_kept_as_done_and_many_at_once_are_kept_newest_first() {
    let mut w = inbox(true);
    let ids: Vec<Id> = drawn(&mut w).into_iter().take(3).collect();
    // « Plus tard » that the account's server keeps, with the first message in it (its record not read yet).
    let account = w.state().session.mailbox.message(ids[0]).unwrap().account;
    let snoozed = w.state_mut().session.mailbox.add_folder_free(account, None, "Plus tard").unwrap();
    w.state_mut().session.mailbox.folder_mut(snoozed).unwrap().server =
        Some(mailcraft_model::ServerFolder { id: "MSnz".into(), name: "Plus tard".into(), ..Default::default() });
    w.state_mut().session.mailbox.message_mut(ids[0]).unwrap().folder = snoozed;
    w.state_mut().session.mailbox_changed();
    let email = w.state().session.mailbox.account(account).unwrap().email.clone();
    w.state_mut().session.run("mail.setSnoozes", &json!({"account": email, "mailbox": "MSnz", "records": []})).unwrap();
    w.state_mut().run("mail.archive", json!({"ids": [ids[0]]})).unwrap();
    assert_eq!(folder_kind(&w, ids[0]), Some(FolderKind::Archive), "archived");
    assert!(w.state().done.items.is_empty(), "not kept as done: nothing would give it its time again");
    // Two at once: both kept, in the order they were named; archived again (already there), nothing twice.
    w.state_mut().run("mail.archive", json!({"ids": [ids[1], ids[2]]})).unwrap();
    let kept = |w: &testing::Window| w.state().done.items.iter().map(|i| i.id).collect::<Vec<_>>();
    assert_eq!(kept(&w), [ids[1], ids[2]]);
    let _ = w.state_mut().run("mail.archive", json!({"ids": [ids[1]]}));
    assert_eq!(kept(&w), [ids[1], ids[2]]);
}
