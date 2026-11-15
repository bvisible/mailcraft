// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the groups of Principal in the list of variant D.

//! The groups of Principal (variant D, `super::spark_list`): where the Inbox has its mail by kind of sender, its notifications and
//! its newsletters are each a row of the list where the newest of them is (`list::Row::bundle`, built by the engine), in place of
//! the tabs. The row says the group's name and how many it holds, its unread dot, the senders it holds (the most recent first, each
//! with how many), and a chevron; a click opens the group as the tab did (`view.filter`), where the header has « ← » back to
//! Principal and « ✓ » to mark the whole group done (header.rs). Its round, under the pointer, marks the group done.

use egui::{Align2, Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::Filter;
use mailcraft_engine::list::Row;
use mailcraft_model::{Address, Id, Stream};
use serde_json::json;

use super::done;
use crate::MailApp;
use crate::theme::{Tokens, paint_clipped, regular, semibold};

/// The group's mark: its round, its icon in it.
const MARK: f32 = 15.0;
/// A sender's chip: its picture, and the gap after a chip.
const CHIP_PICTURE: f32 = 10.0;
const CHIP_GAP: f32 = 18.0;
/// How many senders a row names at most.
const SENDERS: usize = 6;

/// The group's name, in the interface's language.
pub(crate) fn name_of(stream: Stream) -> &'static str {
    match stream {
        Stream::Person => tl!("Main"),
        Stream::Notification => tl!("Notifications"),
        Stream::Newsletter => tl!("Newsletters"),
    }
}

/// The filter that opens the group (its tab's).
pub(crate) fn filter_of(stream: Stream) -> Filter {
    match stream {
        Stream::Person => Filter::Main,
        Stream::Notification => Filter::Notifications,
        Stream::Newsletter => Filter::Newsletters,
    }
}

/// The hit's number of a group: 1 notifications, 2 newsletters.
fn hit_of(stream: Stream) -> u64 {
    match stream {
        Stream::Person => 0,
        Stream::Notification => 1,
        Stream::Newsletter => 2,
    }
}

/// The group's colour: green for notifications, amber for newsletters (the design system's icon colours).
fn tint(t: &Tokens, stream: Stream) -> Color32 {
    match stream {
        Stream::Newsletter => t.icon_yellow,
        _ => t.icon_green,
    }
}

/// A sender of the group: who, the newest of theirs, and how many.
struct Sender {
    who: Address,
    newest: Id,
    count: usize,
}

/// The senders of `members` (newest first), the most recent first, at most [`SENDERS`].
fn senders(app: &MailApp, members: &[Id]) -> Vec<Sender> {
    let mut out: Vec<Sender> = Vec::new();
    // (The newest few thousand: a group of years of newsletters is not walked at every frame.)
    for id in members.iter().take(2_000) {
        let Some(m) = app.session.mailbox.message(*id) else { continue };
        let key = m.from.email.trim().to_lowercase();
        if let Some(sender) = out.iter_mut().find(|s| s.who.email.trim().to_lowercase() == key) {
            sender.count += 1;
        } else if out.len() < SENDERS {
            out.push(Sender { who: m.from.clone(), newest: m.id, count: 1 });
        }
    }
    out
}

/// The row of a group of Principal in `rect`: `one` the single line of a wide list.
pub(super) fn row(app: &mut MailApp, ui: &mut Ui, row: &Row, rect: Rect, one: bool) {
    let Some(stream) = row.bundle else { return };
    let t = Tokens::get(ui.ctx());
    let key = (row.id, row.copy);
    let resp = ui.interact(rect, ui.scope_id().with(("bundle", key)), Sense::click());
    let name = name_of(stream);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    app.hit.add("bundle", hit_of(stream), rect);
    let over = ui.rect_contains_pointer(rect);
    let p = ui.painter().clone();
    let card = Rect::from_min_max(pos2(rect.min.x + 2.0, rect.min.y + 1.0), pos2(rect.max.x, rect.max.y - 2.0)).shrink2(vec2(4.0, 0.0));
    if over {
        p.rect_filled(card, 6.0, t.hover_subtle);
    }
    p.line_segment([pos2(rect.min.x + 14.0, rect.max.y - 0.5), pos2(rect.max.x - 10.0, rect.max.y - 0.5)], Stroke::new(1.0, t.border_faint));
    let line1 = if one { rect.center().y } else { rect.min.y + 21.0 };
    let line2 = rect.min.y + 41.0;
    // The lead column: the unread dot, the round of « Traité » under the pointer (the whole group done).
    let round = done::round_at(rect.min.x, line1, true);
    let offered = done::offered(app);
    if row.unread > 0 && !(offered && over) {
        p.circle_filled(round.0, 4.0, t.selection_edge);
        app.hit.add("bundleDot", hit_of(stream), Rect::from_center_size(round.0, vec2(8.0, 8.0)));
    }
    // The group's mark, where a message's picture is.
    let mark = pos2(rect.min.x + done::D_LEAD + MARK, if one { rect.center().y } else { rect.min.y + 29.0 });
    let tint = tint(&t, stream);
    p.circle_filled(mark, MARK, tint.gamma_multiply(0.18));
    crate::icons::paint(
        &p,
        Rect::from_center_size(mark, vec2(16.0, 16.0)),
        if stream == Stream::Newsletter { "journal" } else { "bell" },
        tint,
        t.accent,
    );
    let left = mark.x + MARK + 14.0;
    let chevron_x = rect.max.x - 22.0;
    // The name, and how many it holds.
    let font = if row.unread > 0 { semibold(15.0) } else { regular(15.0) };
    let name_w = paint_clipped(&p, pos2(left, line1), (chevron_x - 12.0 - left).max(0.0), name, font, t.text);
    let count = row.members.len().to_string();
    let count_w = p.text(pos2(left + name_w + 7.0, line1), Align2::LEFT_CENTER, &count, regular(12.0), t.text_dim).size().x;
    // Its senders: on the same line after the name's column in a single line, on the line under it otherwise.
    let senders = senders(app, &row.members);
    let (mut x, y) =
        if one { ((left + name_w + 7.0 + count_w + 16.0).max(left + super::d_sender_width(chevron_x - left)), line1) } else { (left, line2) };
    for sender in &senders {
        let label = sender.who.display().to_string();
        let room = chevron_x - 16.0 - x;
        if room < 2.0 * CHIP_PICTURE + 40.0 {
            break;
        }
        if let Some(m) = app.session.mailbox.message(sender.newest).cloned() {
            crate::sender_images::avatar(app, ui, pos2(x + CHIP_PICTURE, y), CHIP_PICTURE, &m, &sender.who);
        }
        let mut w = 2.0 * CHIP_PICTURE + 7.0;
        w += paint_clipped(&p, pos2(x + w, y), (room - w).max(0.0), &label, regular(if one { 14.0 } else { LINE2 }), t.text);
        if sender.count > 1 && x + w + 6.0 < chevron_x - 24.0 {
            w += 6.0 + p.text(pos2(x + w + 6.0, y), Align2::LEFT_CENTER, sender.count.to_string(), regular(12.0), t.text_dim).size().x;
        }
        x += w + CHIP_GAP;
    }
    // The chevron: it opens.
    let c = pos2(chevron_x, line1);
    p.line(vec![c + vec2(-2.5, -5.0), c + vec2(2.5, 0.0), c + vec2(-2.5, 5.0)], Stroke::new(1.6, t.text_dim));
    // A click on the round marks it all done; elsewhere it opens.
    if done::row_round(app, ui, key, row, round, offered && over) {
        return;
    }
    if resp.clicked() {
        let _ = app.run("view.filter", json!({"value": filter_of(stream)}));
    }
}

/// The size of the senders on the second line of a row of two.
const LINE2: f32 = 12.5;

/// How many the group `stream` holds in the list's Inboxes, for the header of an open group.
pub(crate) fn count(app: &MailApp, stream: Stream) -> usize {
    super::stream_tabs::counts(app).map_or(0, |c| c.total(stream))
}

/// The group a filter opens, where it is one.
pub(crate) fn open_group(filter: Filter) -> Option<Stream> {
    match filter {
        Filter::Notifications => Some(Stream::Notification),
        Filter::Newsletters => Some(Stream::Newsletter),
        _ => None,
    }
}
