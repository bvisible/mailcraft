// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Traité » in the Inbox of a hosting page's tab, as Spark has it: a round
// //// Neoffice — to mark a message done, one for each group of the list, the quick actions of a row, and what was just done.

//! « Traité » as Spark has it, in the Inbox of a hosting page's tab (`Session::keeps_local` false): done is archived
//! (`mail.archive`: the account's Archive folder, Spark's Done; upstream has no other notion of it), in a click.
//! - Each row under the pointer shows its round at its left: a click marks it done; on its right, its quick actions (read or
//!   unread, pin, flag, move, delete), those the window has, in place of upstream's two (flag, delete).
//! - Each group's heading (Today, Yesterday, This week…) has its round: a click marks the whole group done.
//! - Above the list, « Marqués comme traités (N) », folded or open, keeps what was marked done in this session, each with
//!   « Annuler » (back to the folder it was in) and « Tout annuler ». Nothing of it is kept between sessions.
//!
//! The round is ink on the paper (dark: the bright paper on the espresso), a ring at rest, full under the pointer; what was
//! done wears the signal's tint (the design system's, as the window's tokens are in a hosting page's tab).

use std::collections::BTreeMap;

use egui::{Align2, Color32, Id as EguiId, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::Module;
use mailcraft_engine::list::Row;
use mailcraft_model::{FlagStatus, FolderKind, Id, Message};
use serde_json::json;

use crate::MailApp;
use crate::theme::{Tokens, paint_clipped, regular, semibold};

/// The room a row and a group's heading give their round, left of the read switch.
pub(super) const LEAD: f32 = 26.0;
/// The round: its radius, and its centre from the row's left edge.
const ROUND: f32 = 8.0;
const ROUND_X: f32 = 15.0;
/// The list as Spark has it (variant D, `super::spark_list`): a lead column this wide holds the unread dot, which becomes a round
/// of this radius under the pointer, in its middle.
pub(super) const D_LEAD: f32 = 40.0;
const D_ROUND: f32 = 11.0;
/// The quick actions: each button, and the gap between two.
const QUICK: egui::Vec2 = egui::vec2(26.0, 22.0);
const QUICK_GAP: f32 = 2.0;
/// The section of what was done: its heading, its rows, and how many rows show before it scrolls.
const HEAD_H: f32 = 30.0;
const ROW_H: f32 = 26.0;
const ROWS_SHOWN: f32 = 6.0;

/// What was marked done in this session, newest first, and whether its section is open.
#[derive(Default)]
pub(crate) struct Done {
    pub(crate) items: Vec<Item>,
    pub(crate) open: bool,
}

/// A message marked done: the folder it was in, and what its row said.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub(crate) id: Id,
    pub(crate) from: Id,
    pub(crate) who: String,
    pub(crate) subject: String,
}

/// Where a row's or a heading's round is, from the left edge `x` of the list and the middle `y` of its line: in the lead column
/// of variant D (`d`), else left of the read switch. Its centre and radius.
pub(super) fn round_at(x: f32, y: f32, d: bool) -> (Pos2, f32) {
    if d { (pos2(x + D_LEAD / 2.0, y), D_ROUND) } else { (pos2(x + ROUND_X, y), ROUND) }
}

/// Whether `m` is listed as done: in its account's Archive, which « Afficher les traités » lists beside the Inbox.
pub(crate) fn listed_done(app: &MailApp, m: &Message) -> bool {
    app.session.view.show_done
        && !app.session.keeps_local
        && app.session.mailbox.folder(m.folder).is_some_and(|f| f.kind == FolderKind::Archive)
        && app.session.mailbox.folder(app.session.view.folder).is_some_and(|f| f.kind == FolderKind::Inbox)
}

/// The server id of `account`'s Archive while « Afficher les traités » lists it beside the account's Inbox: a run of the account
/// reads it too, so that what is shown done is what its server has.
pub(crate) fn archive_shown(s: &mailcraft_engine::Session, account: Id) -> Option<String> {
    if s.keeps_local || !s.view.show_done || s.view.module != Module::Mail {
        return None;
    }
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox)?;
    let listed = s.all_inboxes().map_or(s.view.folder == inbox, |all| all.contains(&inbox));
    if !listed {
        return None;
    }
    let archive = s.mailbox.folder_of_kind(account, FolderKind::Archive)?;
    s.mailbox.folder(archive)?.server.as_ref().map(|server| server.id.clone()).filter(|id| !id.is_empty())
}

/// Whether « Traité » is offered: a hosting page's tab, its Inbox in view.
pub(crate) fn offered(app: &MailApp) -> bool {
    !app.session.keeps_local
        && app.session.view.module == Module::Mail
        && app.session.mailbox.folder(app.session.view.folder).is_some_and(|f| f.kind == FolderKind::Inbox)
}

/// Mark `ids` done: archived, and kept in the section with the folder each was in. `false` when nothing was.
pub(crate) fn mark(app: &mut MailApp, ids: &[Id]) -> bool {
    let items: Vec<Item> = ids
        .iter()
        .filter_map(|id| {
            app.session.mailbox.message(*id).map(|m| Item {
                id: *id,
                from: m.folder,
                who: m.from.display().to_string(),
                subject: m.subject.trim().to_string(),
            })
        })
        .collect();
    if items.is_empty() {
        return false;
    }
    let asked: Vec<Id> = items.iter().map(|i| i.id).collect();
    if app.run("mail.archive", json!({"ids": asked})).is_err() {
        return false;
    }
    let moved: Vec<Item> = items.into_iter().filter(|i| app.session.mailbox.message(i.id).is_some_and(|m| m.folder != i.from)).collect();
    if moved.is_empty() {
        return false;
    }
    app.done.items.retain(|d| !moved.iter().any(|m| m.id == d.id));
    for item in moved.into_iter().rev() {
        app.done.items.insert(0, item);
    }
    true
}

/// Put one back where it was, and out of the section.
pub(crate) fn undo(app: &mut MailApp, id: Id) {
    let Some(item) = app.done.items.iter().find(|i| i.id == id).cloned() else { return };
    let here = app.session.mailbox.message(id).map(|m| m.folder);
    if here.is_some_and(|folder| folder != item.from) && app.run("mail.move", json!({"ids": [id], "folder": item.from})).is_err() {
        return;
    }
    app.done.items.retain(|i| i.id != id);
}

/// Put them all back, each where it was: one move for each folder they came from.
pub(crate) fn undo_all(app: &mut MailApp) {
    let all: Vec<Id> = app.done.items.iter().map(|i| i.id).collect();
    undo_these(app, &all);
}

/// Put back those of `ids` that were marked done here (an open group's « Tout annuler »: its own), each where it was.
pub(crate) fn undo_these(app: &mut MailApp, ids: &[Id]) {
    let asked: std::collections::HashSet<Id> = ids.iter().copied().collect();
    let mut by_folder: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
    for item in app.done.items.iter().filter(|i| asked.contains(&i.id)) {
        if app.session.mailbox.message(item.id).is_some_and(|m| m.folder != item.from) {
            by_folder.entry(item.from).or_default().push(item.id);
        }
    }
    let mut kept = std::collections::HashSet::new();
    for (folder, moved) in by_folder {
        if app.run("mail.move", json!({"ids": moved, "folder": folder})).is_err() {
            kept.extend(moved);
        }
    }
    app.done.items.retain(|i| !asked.contains(&i.id) || kept.contains(&i.id));
}

/// The messages of the group whose heading is on row `first`: its rows until the next heading. Not a group of Principal's (its
/// notifications or newsletters, of every day: `Row::bundle`), which has its own round.
pub(super) fn group_ids(rows: &[Row], first: usize) -> Vec<Id> {
    let mut ids = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(first) {
        if i > first && row.group.is_some() {
            break;
        }
        if row.bundle.is_none() {
            ids.extend(row.members.iter().copied());
        }
    }
    ids
}

/// The round: a ring at rest, full with its tick under the pointer; `done` wears the signal's tint.
fn paint_round(ui: &Ui, t: &Tokens, center: Pos2, hovered: bool, done: bool) {
    paint_round_of(ui, t, center, ROUND, hovered, done);
}

/// [`paint_round`] of `radius` (variant D's is larger, its ring a little heavier).
fn paint_round_of(ui: &Ui, t: &Tokens, center: Pos2, radius: f32, hovered: bool, done: bool) {
    let p = ui.painter();
    let k = radius / ROUND;
    let tick = |color: Color32| {
        p.add(egui::Shape::line(
            vec![
                pos2(center.x - 3.6 * k, center.y + 0.2 * k),
                pos2(center.x - 1.0 * k, center.y + 2.8 * k),
                pos2(center.x + 3.8 * k, center.y - 2.8 * k),
            ],
            Stroke::new(1.6, color),
        ));
    };
    if done {
        p.circle_filled(center, radius, if hovered { t.accent } else { t.accent_soft });
        tick(if hovered { t.on_accent } else { t.accent_soft_text });
    } else if hovered {
        p.circle_filled(center, radius, t.accent);
        tick(t.on_accent);
    } else {
        let width = if radius > ROUND { 1.5 } else { 1.2 };
        p.circle_stroke(center, radius - width / 2.0, Stroke::new(width, t.text_dim));
    }
}

/// The tooltip of a round: what it does, and the key that archives.
fn round_tip(app: &MailApp, resp: Response, words: &str) -> Response {
    let key = crate::widgets::shortcut_text(app, "mail.archive");
    let said = if key.is_empty() { words.to_string() } else { format!("{words} ({key})") };
    resp.on_hover_text(said)
}

/// A row's round at `center` (of `radius`, [`round_at`]), while the row is under the pointer: `true` when it was clicked (marked
/// done).
pub(super) fn row_round(app: &mut MailApp, ui: &Ui, key: (Id, u16), row: &Row, (center, radius): (Pos2, f32), shown: bool) -> bool {
    let at = Rect::from_center_size(center, vec2(2.0 * radius + 6.0, 2.0 * radius + 6.0));
    if !shown {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(at, ui.scope_id().with(("msg_done", key)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = round_tip(app, resp, tl!("Mark as done"));
    app.hit.add("doneRound", row.id, at);
    paint_round_of(ui, &t, center, radius, resp.hovered(), false);
    resp.clicked() && mark(app, &row.members)
}

/// The round of a row « Afficher les traités » lists as done (its message is in its account's Archive), always shown, full: a
/// click puts it back in its account's Inbox. `true` when it was clicked and it went back.
pub(super) fn listed_done_round(app: &mut MailApp, ui: &Ui, key: (Id, u16), row: &Row, (center, radius): (Pos2, f32)) -> bool {
    let at = Rect::from_center_size(center, vec2(2.0 * radius + 6.0, 2.0 * radius + 6.0));
    let t = Tokens::get(ui.ctx());
    let resp = ui
        .interact(at, ui.scope_id().with(("msg_undone", key)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tl!("Put it back in the Inbox"));
    app.hit.add("doneListed", row.id, at);
    paint_round_of(ui, &t, center, radius, resp.hovered(), true);
    resp.clicked() && back_to_inbox(app, &row.members)
}

/// Put `ids` back in their accounts' Inboxes (a message listed as done), and out of the section of what was done here.
fn back_to_inbox(app: &mut MailApp, ids: &[Id]) -> bool {
    let mut by_inbox: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
    for id in ids {
        let Some(m) = app.session.mailbox.message(*id) else { continue };
        if let Some(inbox) = app.session.mailbox.folder_of_kind(m.account, FolderKind::Inbox).filter(|inbox| *inbox != m.folder) {
            by_inbox.entry(inbox).or_default().push(*id);
        }
    }
    let mut back = false;
    for (inbox, ids) in by_inbox {
        if app.run("mail.move", json!({"ids": ids, "folder": inbox})).is_ok() {
            app.done.items.retain(|i| !ids.contains(&i.id));
            back = true;
        }
    }
    back
}

/// A group's round at `center` (of `radius`, [`round_at`]), before its heading `heading`: a click marks the whole group done. At
/// rest a faint ring (`ring`), or nothing (variant D: it shows under the pointer, as a row's does).
pub(super) fn group_round(app: &mut MailApp, ui: &Ui, heading: Rect, (center, radius): (Pos2, f32), ring: bool, rows: &[Row], first: usize) {
    let at = Rect::from_center_size(center, vec2(2.0 * radius + 6.0, 2.0 * radius + 6.0));
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(at, ui.scope_id().with(("group_done", first)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let n = group_ids(rows, first).len();
    let resp = round_tip(app, resp, &tln!(n, "Mark {n} message as done", "Mark these {n} messages as done"));
    app.hit.add("doneGroup", first as u64, at);
    let over = ui.rect_contains_pointer(heading.union(at));
    if over || resp.hovered() {
        paint_round_of(ui, &t, center, radius, resp.hovered(), false);
    } else if ring {
        // At rest a faint ring: the group can be done in a click.
        ui.painter().circle_stroke(center, radius - 0.6, Stroke::new(1.0, t.border_strong));
    }
    if resp.clicked() {
        let ids = group_ids(rows, first);
        mark(app, &ids);
    }
}

/// A quick action of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Quick {
    Read,
    Pin,
    Flag,
    Move,
    Delete,
}

/// A row's quick actions, right to left from its end: delete, move, flag, pin, read.
pub(super) struct Bar {
    buttons: Vec<(Quick, Response, Rect)>,
    /// Where the leftmost starts: the marks before it keep clear of it.
    pub(super) left: f32,
}

/// The quick actions of the row under the pointer (or whose move menu is open), on the sender's line at the row's end.
pub(super) fn bar(app: &mut MailApp, ui: &Ui, key: (Id, u16), row: &Row, m: &Message, rect: Rect, line1: f32) -> Bar {
    let unread = row.unread > 0;
    let mut at = Rect::from_center_size(pos2(rect.max.x - QUICK.x / 2.0 - 4.0, line1), QUICK);
    let mut buttons = Vec::new();
    for quick in [Quick::Delete, Quick::Move, Quick::Flag, Quick::Pin, Quick::Read] {
        let tip = match quick {
            Quick::Delete => tl!("Delete"),
            Quick::Move => tl!("Move"),
            Quick::Flag if m.flag == FlagStatus::Flagged => tl!("Clear flag"),
            Quick::Flag => tl!("Flag for follow-up"),
            Quick::Pin if m.pinned => tl!("Unpin"),
            Quick::Pin => tl!("Pin"),
            Quick::Read if unread => tl!("Mark as read"),
            Quick::Read => tl!("Mark as unread"),
        };
        let resp = ui.interact(at, ui.scope_id().with(("msg_quick", quick as u8, key)), Sense::click()).on_hover_text(tip);
        let name = match quick {
            Quick::Delete => "doneQuickDelete",
            Quick::Move => "doneQuickMove",
            Quick::Flag => "doneQuickFlag",
            Quick::Pin => "doneQuickPin",
            Quick::Read => "doneQuickRead",
        };
        app.hit.add(name, row.id, at);
        buttons.push((quick, resp, at));
        at = at.translate(vec2(-(QUICK.x + QUICK_GAP), 0.0));
    }
    let left = buttons.last().map_or(rect.max.x, |(_, _, r)| r.min.x);
    Bar { buttons, left }
}

/// Whether the row's move menu is open: its quick actions stay while the pointer is in it.
pub(super) fn menu_open(ui: &Ui, key: (Id, u16)) -> bool {
    let id = ui.scope_id().with(("msg_quick", Quick::Move as u8, key));
    egui::Popup::is_id_open(ui.ctx(), id.with("popup"))
}

/// Paint the quick actions.
pub(super) fn paint_bar(ui: &Ui, t: &Tokens, bar: &Bar, row: &Row, m: &Message) {
    let p = ui.painter();
    for (quick, resp, at) in &bar.buttons {
        if resp.hovered() {
            p.rect_filled(*at, 4.0, t.pressed);
        }
        let (icon, on) = match quick {
            Quick::Delete => ("delete", false),
            Quick::Move => ("move", false),
            Quick::Flag => ("flag", m.flag == FlagStatus::Flagged),
            Quick::Pin => ("pin", m.pinned),
            Quick::Read if row.unread > 0 => ("markRead", false),
            Quick::Read => ("markUnread", false),
        };
        let color = if on || resp.hovered() { t.text } else { t.text_dim };
        crate::icons::paint(p, Rect::from_center_size(at.center(), vec2(16.0, 16.0)), icon, color, t.accent);
    }
}

/// Do what was clicked in the quick actions, and keep the move menu while it is open. `true` when the row went (deleted,
/// moved).
pub(super) fn bar_clicks(app: &mut MailApp, bar: &Bar, row: &Row) -> bool {
    let unread = row.unread > 0;
    for (quick, resp, _) in &bar.buttons {
        if *quick == Quick::Move {
            let mut moved = false;
            egui::Popup::menu(resp).show(|ui| {
                ui.set_min_width(180.0);
                crate::ribbon::folder_menu_of(ui, app, Some(row.id), |app, folder| {
                    moved = app.run("mail.move", json!({"folder": folder, "ids": row.members})).is_ok();
                });
            });
            if moved {
                return true;
            }
            continue;
        }
        if !resp.clicked() {
            continue;
        }
        let (command, args) = match quick {
            Quick::Delete => ("mail.delete", json!({"ids": row.members})),
            Quick::Flag => ("mail.toggleFlag", json!({"ids": [row.id]})),
            Quick::Pin => ("mail.pin", json!({"ids": [row.id]})),
            Quick::Read if unread => ("mail.markRead", json!({"ids": row.members})),
            Quick::Read => ("mail.markUnread", json!({"ids": [row.id]})),
            Quick::Move => continue,
        };
        let _ = app.run(command, args);
        return *quick == Quick::Delete;
    }
    false
}

/// « Marqués comme traités (N) » above the list, while something was marked done: its heading folds or opens it, « Tout
/// annuler » puts them all back, and open, each with « Annuler ».
pub(super) fn section(app: &mut MailApp, ui: &mut Ui) {
    if app.done.items.is_empty() || !offered(app) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let n = app.done.items.len();
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEAD_H), Sense::hover());
    app.hit.add("doneSection", 0, head);
    let all_words = tl!("Undo all");
    let all_w = ui.painter().layout_no_wrap(all_words.to_string(), regular(12.0), t.accent_text).size().x;
    let all_at = Rect::from_center_size(pos2(head.max.x - 14.0 - all_w / 2.0, head.center().y), vec2(all_w + 8.0, 22.0));
    let fold_at = Rect::from_min_max(head.min, pos2(all_at.min.x - 6.0, head.max.y));
    let fold = ui
        .interact(fold_at, EguiId::unique("done_section_fold"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if app.done.open { tl!("Hide what was marked as done") } else { tl!("Show what was marked as done") });
    let all = ui.interact(all_at, EguiId::unique("done_section_undo_all"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    app.hit.add("doneUndoAll", 0, all_at);
    let p = ui.painter();
    paint_round(ui, &t, pos2(head.min.x + ROUND_X, head.center().y), false, true);
    let title = crate::i18n::fmt(tl!("Marked as done ({count})"), &[("count", &n.to_string())]);
    let x = head.min.x + LEAD + 14.0;
    let w = paint_clipped(p, pos2(x, head.center().y), (all_at.min.x - x - 24.0).max(0.0), &title, semibold(12.0), t.group_label);
    let chevron = pos2(x + w + 10.0, head.center().y);
    let icon = if app.done.open { "expanded" } else { "collapsed" };
    crate::icons::paint(p, Rect::from_center_size(chevron, vec2(10.0, 10.0)), icon, t.text_dim, t.accent);
    if all.hovered() {
        p.rect_filled(all_at, 4.0, t.hover);
    }
    p.text(all_at.center(), Align2::CENTER_CENTER, all_words, regular(12.0), t.accent_text);
    let mut undo_one = None;
    if app.done.open {
        let items = app.done.items.clone();
        let height = ROW_H * (items.len() as f32).min(ROWS_SHOWN);
        egui::ScrollArea::vertical().id_salt("done_section_rows").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
            for item in &items {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                let words = tl!("Undo");
                let undo_w = ui.painter().layout_no_wrap(words.to_string(), regular(12.0), t.accent_text).size().x;
                let undo_at = Rect::from_center_size(pos2(r.max.x - 14.0 - undo_w / 2.0, r.center().y), vec2(undo_w + 8.0, 20.0));
                let resp = ui
                    .interact(undo_at, ui.scope_id().with(("done_undo", item.id)), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(tl!("Put it back where it was"));
                app.hit.add("doneUndo", item.id, undo_at);
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(undo_at, 4.0, t.hover);
                }
                paint_round(ui, &t, pos2(r.min.x + ROUND_X, r.center().y), false, true);
                let x = r.min.x + LEAD + 14.0;
                let room = (undo_at.min.x - x - 10.0).max(0.0);
                let who_w = paint_clipped(p, pos2(x, r.center().y), (room * 0.4).min(room), &item.who, semibold(12.0), t.text);
                let subject = if item.subject.is_empty() { tl!("(no subject)") } else { item.subject.as_str() };
                paint_clipped(p, pos2(x + who_w + 10.0, r.center().y), (room - who_w - 10.0).max(0.0), subject, regular(12.0), t.text_dim);
                p.text(undo_at.center(), Align2::CENTER_CENTER, words, regular(12.0), t.accent_text);
                if resp.clicked() {
                    undo_one = Some(item.id);
                }
            }
        });
    }
    ui.painter().line_segment(
        [pos2(ui.min_rect().min.x + 12.0, ui.cursor().min.y), pos2(ui.min_rect().max.x - 10.0, ui.cursor().min.y)],
        Stroke::new(1.0, t.border_faint),
    );
    if fold.clicked() {
        app.done.open = !app.done.open;
    }
    if all.clicked() {
        undo_all(app);
    } else if let Some(id) = undo_one {
        undo(app, id);
    }
}

/// The section in the list of variant D: its heading as a day's title (a small full round, « Marqués comme traités (N) », « Tout
/// annuler »), then the newest of what was done as rows of the list, each with its full round (a click puts it back), the rounds
/// joined by a rail down the lead column. In an open group of Principal, that group's alone.
const D_HEAD_H: f32 = 40.0;
const D_ROW_H: f32 = 50.0;
const D_ROWS: usize = 5;

pub(super) fn section_d(app: &mut MailApp, ui: &mut Ui) {
    if app.done.items.is_empty() || !offered(app) {
        return;
    }
    let group = super::bundle::open_group(app.session.view.filter);
    let items: Vec<Item> = app
        .done
        .items
        .iter()
        .filter(|i| group.is_none_or(|g| app.session.mailbox.message(i.id).is_some_and(|m| m.stream == Some(g))))
        .cloned()
        .collect();
    if items.is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let n = items.len();
    let shown = items.len().min(D_ROWS);
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), D_HEAD_H + D_ROW_H * shown as f32 + 6.0), Sense::hover());
    let head = Rect::from_min_size(area.min, vec2(area.width(), D_HEAD_H));
    app.hit.add("doneSection", 0, head);
    let p = ui.painter().clone();
    let lead_x = area.min.x + D_LEAD / 2.0;
    // The rail, under the rounds: from the heading's to the last row's.
    let last_y = head.max.y + D_ROW_H * (shown as f32 - 0.5);
    p.line_segment([pos2(lead_x, head.center().y), pos2(lead_x, last_y)], Stroke::new(2.0, t.selection_edge.gamma_multiply(0.35)));
    let tick = |center: Pos2, radius: f32| {
        p.circle_filled(center, radius, t.selection_edge);
        let k = radius / ROUND;
        p.add(egui::Shape::line(
            vec![center + vec2(-3.6 * k, 0.2 * k), center + vec2(-k, 2.8 * k), center + vec2(3.8 * k, -2.8 * k)],
            Stroke::new(1.8, t.surface),
        ));
    };
    tick(pos2(lead_x, head.center().y), 10.0);
    let title = crate::i18n::fmt(tl!("Marked as done ({count})"), &[("count", &n.to_string())]);
    let x = area.min.x + D_LEAD;
    let w = paint_clipped(&p, pos2(x, head.center().y), (area.width() - D_LEAD - 120.0).max(0.0), &title, semibold(14.0), t.group_label);
    let words = tl!("Undo all");
    let words_w = p.layout_no_wrap(words.to_string(), semibold(12.0), t.text).size().x;
    let all_at = Rect::from_min_size(pos2(x + w + 12.0, head.center().y - 13.0), vec2(words_w + 20.0, 26.0));
    let all = ui.interact(all_at, EguiId::unique("done_section_undo_all"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    all.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, words));
    app.hit.add("doneUndoAll", 0, all_at);
    p.rect(all_at, 13.0, if all.hovered() { t.hover } else { Color32::TRANSPARENT }, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    p.text(all_at.center(), Align2::CENTER_CENTER, words, semibold(12.0), t.text);
    let mut undo_one = None;
    for (i, item) in items.iter().take(shown).enumerate() {
        let r = Rect::from_min_size(pos2(area.min.x, head.max.y + D_ROW_H * i as f32), vec2(area.width(), D_ROW_H));
        let center = pos2(lead_x, r.center().y);
        let at = Rect::from_center_size(center, vec2(28.0, 28.0));
        let resp = ui
            .interact(at, ui.scope_id().with(("done_undo", item.id)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tl!("Put it back where it was"));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Undo")));
        app.hit.add("doneUndo", item.id, at);
        if resp.hovered() {
            p.circle_filled(center, D_ROUND + 3.0, t.hover);
        }
        tick(center, D_ROUND);
        let m = app.session.mailbox.message(item.id).cloned();
        let picture = pos2(r.min.x + D_LEAD + 16.0, r.center().y);
        if app.ui.sender_images
            && let Some(m) = &m
        {
            crate::sender_images::avatar(app, ui, picture, 15.0, m, &m.from);
        }
        let left = if app.ui.sender_images { picture.x + 15.0 + 14.0 } else { r.min.x + D_LEAD };
        let date = m.as_ref().map(|m| {
            crate::dates::list_label(mailcraft_engine::list::when(m), app.session.now(), app.session.offset_at(m.date), app.session.prefs.twelve_hour)
        });
        let end = r.max.x - 14.0;
        let date_w = date.map_or(0.0, |date| p.text(pos2(end, r.center().y), Align2::RIGHT_CENTER, date, semibold(13.0), t.text_dim).size().x);
        let room = (end - date_w - 12.0 - left).max(0.0);
        let who_w = paint_clipped(&p, pos2(left, r.center().y), super::d_sender_width(room), &item.who, semibold(15.0), t.text_dim);
        let subject = if item.subject.is_empty() { tl!("(no subject)") } else { item.subject.as_str() };
        let sx = left + who_w.max(super::d_sender_width(room)) + 16.0;
        paint_clipped(&p, pos2(sx, r.center().y), (left + room - sx).max(0.0), subject, regular(15.0), t.text_dim);
        if resp.clicked() {
            undo_one = Some(item.id);
        }
    }
    if all.clicked() {
        let these: Vec<Id> = items.iter().map(|i| i.id).collect();
        undo_these(app, &these);
    } else if let Some(id) = undo_one {
        undo(app, id);
    }
}

#[cfg(test)]
#[path = "done_tests.rs"]
mod tests;
