// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the inbox's tabs by kind of sender, the boxes to tick messages and the bar to archive them.

//! The inbox's tabs (Main, Notifications, Newsletters), as Gmail and Spark have them, and the way to archive what is in them.
//!
//! The tabs replace All and Unread above the list while the inbox holds messages a server said the kind of sender of
//! (`mail.setStreams`); the other filters stay in the arrangement's menu. Each tab says how many of its messages are unread. In
//! each tab a box at the left of every row ticks that message (what a Ctrl+click does), and a bar under the header archives what is
//! ticked, or the whole tab at once: the point of the tabs is to get rid of notifications and newsletters in a click.

use egui::{Align2, Id as EguiId, Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::Filter;
use mailcraft_engine::list::Row;
use mailcraft_model::{Id, Stream, StreamCounts};
use serde_json::json;

use crate::MailApp;
use crate::theme::{Tokens, paint_clipped, regular, semibold};

/// The bar under the header: archive what is ticked, or everything in the tab.
pub(super) const BAR_H: f32 = 34.0;
/// A tab's name: a size down from All and Unread so that three of them and their counts fit a narrow list.
const TAB_TEXT: f32 = 12.5;
/// The count in a tab.
const PILL_TEXT: f32 = 10.5;
const PILL_H: f32 = 16.0;
/// From one tab to the next.
const GAP: f32 = 10.0;
/// The least of a tab's name that is worth showing.
const NAME_MIN: f32 = 20.0;
/// The box in a row: its side, and how far from the row's left edge its centre is (right of the read switch, left of the text).
const BOX: f32 = 14.0;
const BOX_X: f32 = 21.0;
/// Archiving a tab of more than this asks once more.
const ASKS_ABOVE: usize = 20;

fn filter_of(stream: Stream) -> Filter {
    match stream {
        Stream::Person => Filter::Main,
        Stream::Notification => Filter::Notifications,
        Stream::Newsletter => Filter::Newsletters,
    }
}

fn name_of(stream: Stream) -> &'static str {
    match stream {
        Stream::Person => tl!("Main"),
        Stream::Notification => tl!("Notifications"),
        Stream::Newsletter => tl!("Newsletters"),
    }
}

/// What the tabs show, or `None` where the folder in view has none.
pub(super) fn counts(app: &MailApp) -> Option<StreamCounts> {
    let folder = app.session.view.folder;
    app.session.mailbox.has_streams(folder).then(|| app.session.mailbox.stream_counts(folder))
}

/// Whether the rows of this frame wear their boxes: the list says it once, so a row need not ask the mailbox.
pub(super) fn say_shown(ui: &Ui, shown: bool) {
    ui.data_mut(|d| d.insert_temp(EguiId::unique("stream_boxes"), shown));
}

pub(super) fn shown(ui: &Ui) -> bool {
    ui.data(|d| d.get_temp::<bool>(EguiId::unique("stream_boxes"))).unwrap_or(false)
}

struct Tab {
    stream: Option<Stream>,
    name: String,
    unread: usize,
    name_w: f32,
    pill_w: f32,
    on: bool,
}

impl Tab {
    fn width(&self) -> f32 {
        self.name_w + if self.pill_w > 0.0 { 4.0 + self.pill_w } else { 0.0 }
    }
}

/// The tabs, from `left` to `right` in the header `r`. A filter that has no tab of its own (All, Unread, Flagged…) shows as a last
/// one while it is in use, as it does where there are no streams.
pub(super) fn header_tabs(app: &mut MailApp, ui: &mut Ui, t: &Tokens, r: Rect, counts: &StreamCounts, left: f32, right: f32) {
    let filter = app.session.view.filter;
    let measure = |text: &str, font: egui::FontId| ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, t.text).size().x);
    let mut tabs: Vec<Tab> = Stream::ALL
        .into_iter()
        .map(|stream| {
            let name = name_of(stream).to_string();
            let unread = counts.unread(stream);
            let pill_w = if unread > 0 { (measure(&unread.to_string(), semibold(PILL_TEXT)) + 10.0).max(PILL_H) } else { 0.0 };
            let on = filter == filter_of(stream);
            // The tab in use is painted in the heavier weight, which is the wider.
            Tab { stream: Some(stream), name_w: measure(&name, if on { semibold(TAB_TEXT) } else { regular(TAB_TEXT) }), name, unread, pill_w, on }
        })
        .collect();
    if !tabs.iter().any(|tab| tab.on) {
        let name = tl!(filter.label()).to_string();
        tabs.push(Tab { stream: None, name_w: measure(&name, semibold(TAB_TEXT)), name, unread: 0, pill_w: 0.0, on: true });
    }
    let total = |tabs: &[Tab]| tabs.iter().map(Tab::width).sum::<f32>() + GAP * tabs.len().saturating_sub(1) as f32;
    // In a list too narrow for them all, the tab in use gives its count first (the rows under it say as much), then the others
    // are cut short, then left out, then it is cut too.
    if total(&tabs) > right - left {
        for tab in tabs.iter_mut().filter(|tab| tab.on) {
            tab.pill_w = 0.0;
        }
    }
    let mut over = total(&tabs) - (right - left);
    for tab in tabs.iter_mut().rev().filter(|tab| !tab.on) {
        let cut = over.clamp(0.0, (tab.name_w - NAME_MIN).max(0.0));
        tab.name_w -= cut;
        over -= cut;
    }
    while over > 0.0 {
        let Some(i) = tabs.iter().rposition(|tab| !tab.on) else { break };
        over -= tabs.remove(i).width() + GAP;
    }
    let mut x = left;
    for (n, tab) in tabs.iter().enumerate() {
        let name_w = if tab.on { tab.name_w - over.max(0.0) } else { tab.name_w };
        if name_w < NAME_MIN / 2.0 {
            continue;
        }
        let w = name_w + if tab.pill_w > 0.0 { 4.0 + tab.pill_w } else { 0.0 };
        let rect = Rect::from_min_max(pos2(x, r.min.y + 18.0), pos2(x + w + 3.0, r.max.y));
        let resp = ui.interact(rect.expand2(vec2(5.0, 0.0)), ui.scope_id().with(("stream_tab", n)), Sense::click());
        app.hit.add("streamTab", n as u64, rect);
        let painter = ui.painter();
        let font = if tab.on { semibold(TAB_TEXT) } else { regular(TAB_TEXT) };
        paint_clipped(painter, pos2(x + 1.0, r.min.y + 31.0), name_w + 1.0, &tab.name, font, if tab.on { t.accent_text } else { t.text });
        if tab.pill_w > 0.0 {
            let pill = Rect::from_center_size(pos2(x + name_w + 4.0 + tab.pill_w / 2.0, r.min.y + 31.0), vec2(tab.pill_w, PILL_H));
            painter.rect_filled(pill, PILL_H / 2.0, if tab.on { t.accent } else { t.border_strong });
            painter.text(
                pill.center(),
                Align2::CENTER_CENTER,
                tab.unread.to_string(),
                semibold(PILL_TEXT),
                if tab.on { t.on_accent } else { t.text },
            );
        }
        let under = Rect::from_min_max(pos2(rect.min.x, r.max.y - 1.0), rect.max);
        if tab.on {
            painter.rect_filled(under, 0.0, t.accent_text);
        } else if resp.hovered() {
            painter.rect_filled(under, 0.0, t.border_strong);
        }
        if let Some(stream) = tab.stream
            && resp.clicked()
            && !tab.on
        {
            let _ = app.run("view.filter", json!({"value": filter_of(stream)}));
        }
        x = rect.max.x + GAP;
    }
}

/// Whether the bar is there: in Notifications and Newsletters whenever they list something (so that the list does not move as rows
/// are clicked), and in the other tabs while several messages are ticked.
pub(super) fn bar_shown(app: &MailApp, rows: &[Row]) -> bool {
    let filter = app.session.view.filter;
    (matches!(filter, Filter::Notifications | Filter::Newsletters) && !rows.is_empty()) || app.session.sel.messages.len() >= 2
}

/// The messages of the folder in view that the list shows, once each: what "archive all" takes.
fn listed_ids(app: &MailApp, rows: &[Row]) -> Vec<Id> {
    let folder = app.session.view.folder;
    let mut ids: Vec<Id> = rows
        .iter()
        .filter(|r| r.copy == 0)
        .flat_map(|r| r.members.iter().copied())
        .filter(|id| app.session.mailbox.message(*id).is_some_and(|m| m.folder == folder))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The bar under the header. With messages ticked: how many, Archive, and Select all / Clear. With none: Archive all (the tab's
/// messages), asking once more above [`ASKS_ABOVE`]. In a narrow list what is left of the line is what fits: the buttons first, then
/// the count (cut short), then Clear and Select all.
pub(super) fn bar(app: &mut MailApp, ui: &mut Ui, rows: &[Row]) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_H), Sense::hover());
    let chosen = app.session.sel.messages.len();
    let all = listed_ids(app, rows);
    let painter = ui.painter();
    painter.rect_filled(r, 0.0, if chosen > 0 { t.selection.linear_multiply(0.6) } else { t.panel });
    painter.line_segment([pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, t.border));
    let asking = EguiId::unique("stream_archive_asks");
    // A message ticked and gone from the tab takes the question with it.
    if chosen > 0 {
        ui.data_mut(|d| d.insert_temp(asking, false));
    }
    let asked = ui.data(|d| d.get_temp::<bool>(asking)).unwrap_or(false);
    let mid = r.center().y;
    let count = all.len().to_string();
    // What the bar says and offers, left to right.
    let (text, buttons): (Option<String>, Vec<(&'static str, String, bool)>) = if chosen > 0 {
        (Some(tln!(chosen, "{n} selected", "{n} selected")), vec![("streamArchive", tl!("Archive").to_string(), true)])
    } else if asked {
        (
            Some(crate::i18n::fmt(tl!("Archive {count} messages?"), &[("count", &count)])),
            vec![("streamArchiveYes", tl!("Archive").to_string(), true), ("streamArchiveNo", tl!("Cancel").to_string(), false)],
        )
    } else if all.is_empty() {
        (None, Vec::new())
    } else {
        (None, vec![("streamArchiveAll", crate::i18n::fmt(tl!("Archive all ({count})"), &[("count", &count)]), true)])
    };
    let width = |ui: &Ui, text: &str, font: egui::FontId| ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, t.text).size().x);
    let buttons_w: f32 = buttons.iter().map(|(_, label, _)| width(ui, label, semibold(12.5)) + 22.0 + 8.0).sum::<f32>();
    let text_w = text.as_ref().map_or(0.0, |text| width(ui, text, semibold(12.5)) + 12.0);
    let line = r.width() - 28.0;
    // The count is cut to what the buttons leave; the links take what is left after the count in full.
    let after = (line - buttons_w - text_w).max(0.0);
    let links: Vec<(&'static str, &'static str)> = if asked || chosen == 0 && all.is_empty() {
        Vec::new()
    } else {
        let mut v = Vec::new();
        if chosen < all.len() {
            v.push(("streamSelectAll", tl!("Select all")));
        }
        if chosen > 0 {
            v.push(("streamClear", tl!("Clear selection")));
        }
        v
    };
    let mut x = r.min.x + 14.0;
    if let Some(text) = &text {
        x += paint_clipped(ui.painter(), pos2(x, mid), (line - buttons_w - 12.0).max(0.0), text, semibold(12.5), t.text) + 12.0;
    }
    for (kind, label, primary) in &buttons {
        let resp = button(app, ui, &t, kind, pos2(x, mid), label, *primary);
        x = resp.rect.max.x + 8.0;
        if resp.clicked() {
            match *kind {
                "streamArchive" => {
                    let _ = app.run("mail.archive", json!({}));
                }
                "streamArchiveNo" => {
                    ui.data_mut(|d| d.insert_temp(asking, false));
                }
                "streamArchiveYes" => {
                    let _ = app.run("mail.archive", json!({"ids": all}));
                    ui.data_mut(|d| d.insert_temp(asking, false));
                }
                _ if all.len() > ASKS_ABOVE => {
                    ui.data_mut(|d| d.insert_temp(asking, true));
                }
                _ => {
                    let _ = app.run("mail.archive", json!({"ids": all}));
                }
            }
        }
    }
    // At the right, the links that fit, Select all before Clear (a list that is selected from can always be cleared with a click on
    // a row, and has Ctrl+A for all of it, but this is the way to it): Clear is the last on the line, Select all before it.
    let mut room = after;
    let mut right = r.max.x - 14.0;
    let mut placed: Vec<(&'static str, &'static str)> = Vec::new();
    for (kind, label) in links.iter() {
        let w = width(ui, label, regular(12.0)) + 14.0;
        if w <= room {
            room -= w;
            placed.push((kind, label));
        }
    }
    for (kind, label) in placed.into_iter().rev() {
        if link(app, ui, &t, kind, &mut right, mid, label) {
            let _ = match kind {
                "streamClear" => app.run("mail.select", json!({"ids": []})),
                _ => app.run("mail.selectAll", json!({})),
            };
        }
    }
}

/// A button with its text, its left edge at `at`'s x and centred on its y; the primary one in the accent colour.
fn button(app: &mut MailApp, ui: &mut Ui, t: &Tokens, kind: &'static str, at: egui::Pos2, text: &str, primary: bool) -> egui::Response {
    let font = semibold(12.5);
    let w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), t.text).size().x);
    let rect = Rect::from_min_size(pos2(at.x, at.y - 12.0), vec2(w + 22.0, 24.0));
    let resp = ui.interact(rect, ui.scope_id().with(kind), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    app.hit.add(kind, 0, rect);
    let painter = ui.painter();
    let (fill, line, ink) = match (primary, resp.hovered()) {
        (true, false) => (t.accent, t.accent, t.on_accent),
        (true, true) => (t.accent_text, t.accent_text, t.on_accent),
        (false, false) => (t.surface, t.border_strong, t.text),
        (false, true) => (t.hover, t.border_strong, t.text),
    };
    painter.rect_filled(rect, 4.0, fill);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, line), egui::StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, text, font, ink);
    resp
}

/// A plain underlined link whose right edge is at `*right` (which moves left of it), centred on `mid`. True when clicked.
fn link(app: &mut MailApp, ui: &mut Ui, t: &Tokens, kind: &'static str, right: &mut f32, mid: f32, text: &str) -> bool {
    let font = regular(12.0);
    let w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), t.text).size().x);
    let rect = Rect::from_min_size(pos2(*right - w, mid - 10.0), vec2(w, 20.0));
    let resp = ui.interact(rect.expand2(vec2(4.0, 0.0)), ui.scope_id().with(kind), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    app.hit.add(kind, 0, rect);
    let ink = if resp.hovered() { t.accent_text } else { t.text };
    let painter = ui.painter();
    painter.text(rect.left_center(), Align2::LEFT_CENTER, text, font, ink);
    painter.line_segment([pos2(rect.min.x, mid + 7.5), pos2(rect.max.x, mid + 7.5)], Stroke::new(1.0, ink));
    *right = rect.min.x - 14.0;
    resp.clicked()
}

/// The box that ticks a row, on top of the row so that a click on it leaves the message being read alone.
pub(super) fn row_box(ui: &Ui, key: (Id, u16), rect: Rect, line1: f32) -> (egui::Response, Rect) {
    let at = Rect::from_center_size(pos2(rect.min.x + BOX_X, line1), vec2(BOX + 2.0, 24.0));
    let resp = ui.interact(at, ui.scope_id().with(("msg_tick", key)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    (resp, at)
}

pub(super) fn paint_box(app: &mut MailApp, ui: &Ui, t: &Tokens, id: Id, resp: &egui::Response, at: Rect, ticked: bool) {
    app.hit.add("messageTick", id, at);
    let side = Rect::from_center_size(at.center(), vec2(BOX, BOX));
    let painter = ui.painter();
    if ticked {
        painter.rect_filled(side, 3.0, if resp.hovered() { t.accent_text } else { t.accent });
        let c = side.center();
        painter.add(egui::Shape::line(
            vec![pos2(c.x - 3.6, c.y + 0.2), pos2(c.x - 1.0, c.y + 2.8), pos2(c.x + 3.8, c.y - 2.8)],
            Stroke::new(1.7, t.on_accent),
        ));
    } else {
        painter.rect_stroke(side, 3.0, Stroke::new(1.3, if resp.hovered() { t.accent } else { t.text_disabled }), egui::StrokeKind::Inside);
    }
}
