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
    let mut by_folder: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
    for item in &app.done.items {
        if app.session.mailbox.message(item.id).is_some_and(|m| m.folder != item.from) {
            by_folder.entry(item.from).or_default().push(item.id);
        }
    }
    let mut kept = Vec::new();
    for (folder, ids) in by_folder {
        if app.run("mail.move", json!({"ids": ids, "folder": folder})).is_err() {
            kept.extend(ids);
        }
    }
    app.done.items.retain(|i| kept.contains(&i.id));
}

/// The messages of the group whose heading is on row `first`: its rows until the next heading.
pub(super) fn group_ids(rows: &[Row], first: usize) -> Vec<Id> {
    let mut ids = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(first) {
        if i > first && row.group.is_some() {
            break;
        }
        ids.extend(row.members.iter().copied());
    }
    ids
}

/// The round in `rect`: a ring at rest, full with its tick under the pointer; `done` wears the signal's tint.
fn paint_round(ui: &Ui, t: &Tokens, center: Pos2, hovered: bool, done: bool) {
    let p = ui.painter();
    let tick = |color: Color32| {
        p.add(egui::Shape::line(
            vec![pos2(center.x - 3.6, center.y + 0.2), pos2(center.x - 1.0, center.y + 2.8), pos2(center.x + 3.8, center.y - 2.8)],
            Stroke::new(1.6, color),
        ));
    };
    if done {
        p.circle_filled(center, ROUND, t.accent_soft);
        tick(t.accent_soft_text);
    } else if hovered {
        p.circle_filled(center, ROUND, t.accent);
        tick(t.on_accent);
    } else {
        p.circle_stroke(center, ROUND - 0.6, Stroke::new(1.2, t.text_dim));
    }
}

/// The tooltip of a round: what it does, and the key that archives.
fn round_tip(app: &MailApp, resp: Response, words: &str) -> Response {
    let key = crate::widgets::shortcut_text(app, "mail.archive");
    let said = if key.is_empty() { words.to_string() } else { format!("{words} ({key})") };
    resp.on_hover_text(said)
}

/// A row's round, left of the read switch, while the row is under the pointer: `true` when it was clicked (marked done).
pub(super) fn row_round(app: &mut MailApp, ui: &Ui, key: (Id, u16), row: &Row, rect: Rect, line1: f32, shown: bool) -> bool {
    let center = pos2(rect.min.x + ROUND_X, line1);
    let at = Rect::from_center_size(center, vec2(2.0 * ROUND + 6.0, 2.0 * ROUND + 6.0));
    if !shown {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(at, ui.scope_id().with(("msg_done", key)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = round_tip(app, resp, tl!("Mark as done"));
    app.hit.add("doneRound", row.id, at);
    paint_round(ui, &t, center, resp.hovered(), false);
    resp.clicked() && mark(app, &row.members)
}

/// A group's round, left of its heading `heading`: a click marks the whole group done.
pub(super) fn group_round(app: &mut MailApp, ui: &Ui, heading: Rect, rows: &[Row], first: usize) {
    let center = pos2(heading.min.x - super::ROW_INSET + ROUND_X, heading.center().y + 3.0);
    let at = Rect::from_center_size(center, vec2(2.0 * ROUND + 6.0, 2.0 * ROUND + 6.0));
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(at, ui.scope_id().with(("group_done", first)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let n = group_ids(rows, first).len();
    let resp = round_tip(app, resp, &tln!(n, "Mark this message as done", "Mark these {n} messages as done"));
    app.hit.add("doneGroup", first as u64, at);
    let over = ui.rect_contains_pointer(heading.union(at));
    if over || resp.hovered() {
        paint_round(ui, &t, center, resp.hovered(), false);
    } else {
        // At rest a faint ring: the group can be done in a click.
        ui.painter().circle_stroke(center, ROUND - 0.6, Stroke::new(1.0, t.border_strong));
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
                crate::ribbon::folder_menu(ui, app, |app, folder| {
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

#[cfg(test)]
#[path = "done_tests.rs"]
mod tests;
