// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the list's short header as Spark has it (variant D), where the mail is a
// //// Neoffice — hosting page's.

//! The list's header in variant D (`super::spark_list`): one row, 64 high.
//!
//! - On the left, the view's name, « Toutes les boîtes » or the folder's, with ▾: its menu picks « Toutes les boîtes » or an
//!   Inbox. Under it, what the list is arranged by and whose mail it is (« Par date · hello@ et jeremy@ »): a click there opens
//!   the arrangement's menu (the sort, the filters, Select All), which upstream's header had on its right.
//! - On the right, « Afficher les traités » where « Traité » is offered (an Inbox), the search (it gives the search box the
//!   focus) and a new message.

use egui::{CornerRadius, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::{Filter, SortBy};
use mailcraft_model::{FolderKind, Id};
use serde_json::json;

use super::{done, sort_label, stream_tabs};
use crate::MailApp;
use crate::theme::{Tokens, paint_clipped, regular, semibold};

/// The header's height, its title and the line under it.
const HEIGHT: f32 = 64.0;
const TITLE: f32 = 17.0;
const SUBTITLE: f32 = 13.0;
/// A button on the right, and the switch.
const BUTTON: f32 = 34.0;
const SWITCH: egui::Vec2 = egui::vec2(42.0, 24.0);
/// From one control on the right to the next.
const GAP: f32 = 6.0;
/// A list narrower than this (beside Nora's pane in a small window) keeps its title first: the controls on the right go into the
/// name's menu, the name starts nearer the edge; narrower than `NO_SUBTITLE`, the line under it goes too.
pub(super) const COMPACT_BELOW: f32 = 300.0;
const NO_SUBTITLE: f32 = 220.0;

/// The short address a mailbox goes by in the header: « hello@ ».
pub(crate) fn short_address(email: &str) -> String {
    let email = email.trim();
    match email.split_once('@') {
        Some((local, _)) if !local.is_empty() => format!("{local}@"),
        _ => email.to_string(),
    }
}

/// The mailboxes whose mail is listed, by their short addresses: every own Inbox's in « Toutes les boîtes », else the folder's.
fn boxes(app: &MailApp) -> Vec<String> {
    let mb = &app.session.mailbox;
    let folders = app.session.all_inboxes().unwrap_or_else(|| vec![app.session.view.folder]);
    let mut said: Vec<String> = Vec::new();
    for folder in folders {
        let Some(account) = mb.folder(folder).and_then(|f| mb.account(f.account)) else { continue };
        let short = if account.email.trim().is_empty() { account.name.trim().to_string() } else { short_address(&account.email) };
        if !short.is_empty() && !said.contains(&short) {
            said.push(short);
        }
    }
    said
}

/// « hello@ », « hello@ et jeremy@ », « a@, b@ et c@ ».
pub(crate) fn listed(names: &[String]) -> String {
    match names.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => crate::i18n::fmt(tl!("{list} and {last}"), &[("list", &rest.join(", ")), ("last", last)]),
    }
}

/// The view's name: « Toutes les boîtes », or the folder's as the folder pane writes it.
pub(crate) fn title(app: &MailApp) -> String {
    if app.session.all_inboxes().is_some() {
        return tl!("All Inboxes").to_string();
    }
    app.session.mailbox.folder(app.session.view.folder).map(|f| crate::nav::folder_name(f).to_string()).unwrap_or_default()
}

/// The line under the name: what the list is arranged by, and whose mail it is.
pub(crate) fn subtitle(app: &MailApp) -> String {
    let sort = sort_label(app.session.view.sort);
    let boxes = listed(&boxes(app));
    if boxes.is_empty() { sort.to_string() } else { format!("{sort} · {boxes}") }
}

/// The unread mail of the Inboxes « Toutes les boîtes » lists, as the folder pane counts it.
pub(crate) fn all_unread(app: &mut MailApp) -> usize {
    let counts = app.session.folder_counts();
    app.session.own_inboxes().iter().map(|f| counts.unread(*f)).sum()
}

/// Whether « Toutes les boîtes » is offered: more than one Inbox of the person's own.
pub(crate) fn all_offered(app: &MailApp) -> bool {
    app.session.own_inboxes().len() > 1
}

/// The header: a row of [`HEIGHT`] across the list.
pub(super) fn show(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    let end = r.max.x - super::lane_width(app) - 8.0;
    let compact = end - r.min.x < COMPACT_BELOW;

    // On the right, right to left: a new message, the search, « Afficher les traités ». In a narrow list, in the name's menu.
    let mut left_of = end;
    if !compact {
        let pencil = Rect::from_center_size(pos2(end - BUTTON / 2.0, r.center().y), vec2(BUTTON, BUTTON));
        let search = pencil.translate(vec2(-(BUTTON + GAP), 0.0));
        left_of = search.min.x - GAP;
        if button(app, ui, &t, pencil, "listNew", tl!("New Email"), Icon::Pencil).clicked() {
            let _ = app.run("mail.new", json!({}));
        }
        if button(app, ui, &t, search, "listSearch", tl!("Search"), Icon::Search).clicked() {
            let _ = app.run("ui.focusSearch", json!({}));
        }
        if done::offered(app) {
            let at = Rect::from_center_size(pos2(left_of - SWITCH.x / 2.0, r.center().y), SWITCH);
            left_of = at.min.x - GAP;
            if switch(app, ui, &t, at).clicked() {
                let _ = app.run("view.showDone", json!({}));
            }
        }
    }

    // An open group of Principal: « ← | ✓ », its name and how many it holds (bundle.rs).
    if let Some(stream) = super::bundle::open_group(app.session.view.filter).filter(|_| stream_tabs::counts(app).is_some()) {
        group_head(app, ui, &t, r, stream, left_of, compact);
        return;
    }
    // On the left, the name and its menu, and under it the arrangement and its menu.
    let x = r.min.x + if compact { 12.0 } else { done::D_LEAD };
    let room = (left_of - 8.0 - x).max(0.0);
    let name = title(app);
    let title_y = r.min.y + 23.0;
    let chevron_room = 20.0;
    let painter = ui.painter().clone();
    let name_w = paint_clipped(&painter, pos2(x, title_y), (room - chevron_room).max(0.0), &name, semibold(TITLE), t.text);
    let name_at = Rect::from_min_max(pos2(x - 6.0, title_y - 13.0), pos2(x + name_w + chevron_room, title_y + 13.0));
    let name_resp = ui
        .interact(name_at, ui.scope_id().with("list_view_name"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tl!("Choose the mailbox"));
    name_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    app.hit.add("listViewName", 0, name_at);
    if name_resp.hovered() {
        painter.rect_filled(name_at, 6.0, t.hover);
        paint_clipped(&painter, pos2(x, title_y), (room - chevron_room).max(0.0), &name, semibold(TITLE), t.text);
    }
    let c = pos2(x + name_w + 10.0, title_y + 1.0);
    painter.line(vec![c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)], Stroke::new(1.6, t.text));
    egui::Popup::menu(&name_resp).show(|ui| view_menu(ui, app, compact));

    // (Too narrow for the line under the name: its arrangement's menu is in the name's.)
    if end - r.min.x < NO_SUBTITLE {
        return;
    }
    let said = subtitle(app);
    let sub_y = r.min.y + 44.0;
    let sub_w = painter.layout_no_wrap(said.clone(), regular(SUBTITLE), t.text_dim).size().x.min(room);
    let sub_at = Rect::from_min_max(pos2(x - 6.0, sub_y - 9.0), pos2(x + sub_w + 6.0, sub_y + 9.0));
    let sub_resp = ui
        .interact(sub_at, ui.scope_id().with("list_arrangement"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tl!("Arrange By"));
    sub_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &said));
    app.hit.add("listSort", 0, sub_at);
    if sub_resp.hovered() {
        painter.rect_filled(sub_at, 6.0, t.hover);
    }
    paint_clipped(&painter, pos2(x, sub_y), room, &said, regular(SUBTITLE), if sub_resp.hovered() { t.text } else { t.text_dim });
    egui::Popup::menu(&sub_resp).show(|ui| arrangement_menu(ui, app));
}

/// The header of an open group (`stream`): a pill with « ← » (back to Principal) and « ✓ » (the whole group done), the group's
/// name, and how many it holds in the view (« 24 · Toutes les boîtes »), up to `end`.
fn group_head(app: &mut MailApp, ui: &mut Ui, t: &Tokens, r: Rect, stream: mailcraft_model::Stream, end: f32, compact: bool) {
    let pill = Rect::from_min_size(pos2(r.min.x + 12.0, r.center().y - 18.0), vec2(4.0 + 30.0 + 9.0 + 30.0 + 4.0, 36.0));
    let p = ui.painter().clone();
    p.rect(pill, CornerRadius::same(18), t.surface_sunken, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let back = Rect::from_min_size(pos2(pill.min.x + 4.0, pill.min.y + 4.0), vec2(30.0, 28.0));
    let all = Rect::from_min_size(pos2(back.max.x + 9.0, back.min.y), vec2(30.0, 28.0));
    p.line_segment([pos2(back.max.x + 4.5, pill.center().y - 8.0), pos2(back.max.x + 4.5, pill.center().y + 8.0)], Stroke::new(1.0, t.border_strong));
    let name = super::bundle::name_of(stream);
    let back_words = tl!("Back to the Inbox");
    let all_words = crate::i18n::fmt(tl!("Mark all of {group} as done"), &[("group", name)]);
    let back_resp =
        ui.interact(back, ui.scope_id().with("group_back"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(back_words);
    back_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, back_words));
    app.hit.add("groupBack", 0, back);
    let all_resp = ui
        .interact(all, ui.scope_id().with("group_all_done"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&all_words);
    all_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &all_words));
    app.hit.add("groupAllDone", 0, all);
    for (resp, at) in [(&back_resp, back), (&all_resp, all)] {
        if resp.hovered() {
            p.rect_filled(at, 14.0, t.hover);
        }
    }
    let ink = |resp: &Response| if resp.hovered() { t.text } else { t.text_dim };
    let c = back.center();
    let line = Stroke::new(1.8, ink(&back_resp));
    p.line_segment([c + vec2(6.0, 0.0), c + vec2(-6.0, 0.0)], line);
    p.line(vec![c + vec2(-1.0, -5.0), c + vec2(-6.0, 0.0), c + vec2(-1.0, 5.0)], line);
    let c = all.center();
    p.line(vec![c + vec2(-5.0, 0.5), c + vec2(-1.5, 4.0), c + vec2(5.5, -4.0)], Stroke::new(2.0, ink(&all_resp)));
    let x = pill.max.x + 12.0;
    let name_w = paint_clipped(&p, pos2(x, r.center().y), (end - 8.0 - x).max(0.0), name, semibold(TITLE), t.text);
    // (A narrow list keeps the name and how many, not the view's name.)
    let count = super::bundle::count(app, stream);
    let said = if compact { count.to_string() } else { format!("{count} · {}", title(app)) };
    let sub_x = x + name_w + 10.0;
    let sub_w = paint_clipped(&p, pos2(sub_x, r.center().y + 1.0), (end - 8.0 - sub_x).max(0.0), &said, regular(SUBTITLE), t.text_dim);
    let words = Rect::from_min_max(pos2(x, r.center().y - 12.0), pos2(sub_x + sub_w, r.center().y + 12.0));
    let label = format!("{name} {said}");
    ui.interact(words, ui.scope_id().with("group_title"), Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
    if back_resp.clicked() {
        let _ = app.run("view.filter", json!({"value": Filter::Main}));
    } else if all_resp.clicked() {
        let rows = app.session.rows();
        let ids = stream_tabs::listed_ids(app, &rows);
        done::mark(app, &ids);
    }
}

/// The name's menu: « Toutes les boîtes », then each Inbox of the person's own, with its unread mail.
fn view_menu(ui: &mut Ui, app: &mut MailApp, compact: bool) {
    ui.set_min_width(220.0);
    // A narrow list's controls (`COMPACT_BELOW`), and its arrangement where the line under the name went too.
    if compact {
        crate::widgets::mi(ui, app, "New Email", "mail.new", json!({}));
        crate::widgets::mi(ui, app, "Search", "ui.focusSearch", json!({}));
        if done::offered(app) {
            let on = app.session.view.show_done;
            crate::widgets::mi_check(ui, app, "Show done", on, "view.showDone", json!({}));
        }
        ui.menu_button(tl!("Arrange By"), |ui| arrangement_menu(ui, app));
        ui.separator();
    }
    let unified = app.session.all_inboxes().is_some();
    let counts = app.session.folder_counts();
    if all_offered(app) {
        let n = all_unread(app);
        let label = if n > 0 { format!("{} ({n})", tl!("All Inboxes")) } else { tl!("All Inboxes").to_string() };
        crate::widgets::mi_check(ui, app, &label, unified, "view.allInboxes", json!({}));
        ui.separator();
    }
    let inboxes: Vec<(Id, String, usize)> = app
        .session
        .own_inboxes()
        .into_iter()
        .filter_map(|f| {
            let mb = &app.session.mailbox;
            let folder = mb.folder(f)?;
            let account = mb.account(folder.account)?;
            let who = if account.email.trim().is_empty() { account.name.trim().to_string() } else { short_address(&account.email) };
            Some((f, format!("{} — {who}", crate::nav::folder_name(folder)), counts.unread(f)))
        })
        .collect();
    for (folder, name, unread) in inboxes {
        let label = if unread > 0 { format!("{name} ({unread})") } else { name };
        let on = !unified && app.session.view.folder == folder;
        crate::widgets::mi_check(ui, app, &label, on, "folder.select", json!({"id": folder}));
    }
    // The folder in view when it is none of them (Sent, a folder of the person's): it stays, ticked.
    let current = app.session.mailbox.folder(app.session.view.folder).filter(|f| f.kind != FolderKind::Inbox).cloned();
    if let Some(folder) = current {
        ui.separator();
        let label = crate::nav::result_folder_label(&app.session.mailbox, &folder);
        crate::widgets::mi_check(ui, app, &label, !unified, "folder.select", json!({"id": folder.id}));
    }
}

/// The arrangement's menu, as upstream's header has it, and Select All.
fn arrangement_menu(ui: &mut Ui, app: &mut MailApp) {
    ui.set_min_width(190.0);
    let (sort, filter) = (app.session.view.sort, app.session.view.filter);
    for s in SortBy::ALL {
        crate::widgets::mi_check(ui, app, s.label(), s == sort, "view.sort", json!({"by": s}));
    }
    ui.separator();
    crate::widgets::mi(ui, app, "Reverse Sort", "view.reverseSort", json!({}));
    let conv = app.session.view.conversations;
    crate::widgets::mi_check(ui, app, "Show as Conversations", conv, "view.conversations", json!({}));
    ui.separator();
    if stream_tabs::counts(app).is_some() {
        stream_tabs::menu(ui, app);
    }
    for f in Filter::ALL {
        crate::widgets::mi_check(ui, app, f.label(), f == filter, "view.filter", json!({"value": f}));
    }
    ui.separator();
    crate::widgets::mi(ui, app, "Select All", "mail.selectAll", json!({}));
}

#[derive(Clone, Copy)]
enum Icon {
    Search,
    Pencil,
}

/// A button on the right: its icon, a fill under the pointer.
fn button(app: &mut MailApp, ui: &Ui, t: &Tokens, at: Rect, hit: &'static str, words: &str, icon: Icon) -> Response {
    let resp = ui.interact(at, ui.scope_id().with(hit), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(words);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, words));
    app.hit.add(hit, 0, at);
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(at, 9.0, t.hover);
    }
    let ink = if resp.hovered() { t.text } else { t.text_dim };
    match icon {
        Icon::Search => crate::icons::paint(p, Rect::from_center_size(at.center(), vec2(18.0, 18.0)), "search", ink, t.accent),
        Icon::Pencil => paint_pencil(p, at.center(), ink),
    }
    if resp.has_focus() {
        p.rect_stroke(at.expand(1.0), 9.0, Stroke::new(1.5, t.focus_ring), egui::StrokeKind::Outside);
    }
    resp
}

/// A pencil, 18 across: its body at 45°, its tip down on the left.
fn paint_pencil(p: &egui::Painter, c: Pos2, ink: egui::Color32) {
    let line = Stroke::new(1.6, ink);
    // Along the pencil, and across it.
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let along = vec2(half, -half);
    let across = vec2(half, half);
    let tip = c + vec2(-6.5, 6.5);
    let start = tip + along * 4.0;
    let end = tip + along * 15.5;
    let side = 2.4;
    let body = [start + across * side, end + across * side, end - across * side, start - across * side];
    p.add(egui::Shape::closed_line(body.to_vec(), line));
    p.line(vec![start + across * side, tip, start - across * side], line);
    // The ferrule.
    let band = tip + along * 12.5;
    p.line_segment([band + across * side, band - across * side], line);
}

/// « Afficher les traités »: a switch whose knob carries a tick, on the signal while it is on.
fn switch(app: &mut MailApp, ui: &Ui, t: &Tokens, at: Rect) -> Response {
    let on = app.session.view.show_done;
    let words = tl!("Show done");
    let resp = ui
        .interact(at, ui.scope_id().with("list_show_done"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if on { tl!("Hide what was marked as done") } else { tl!("Show what was marked as done") });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, words));
    app.hit.add("listShowDone", u64::from(on), at);
    let p = ui.painter();
    let shown = ui.ctx().animate_bool_with_time(ui.scope_id().with("list_show_done_knob"), on, crate::motion::fade(ui, 0.12));
    let track = if on { t.selection_edge } else { t.pill_quiet };
    let track = if resp.hovered() && !on { t.hover } else { track };
    p.rect_filled(at, CornerRadius::same(12), track);
    // On: the paper's knob on the signal. Off, the knob is the quiet one of the design system: sand on the espresso, paper with
    // its line on the paper.
    let paper = if t.dark { t.text } else { t.surface };
    let knob = pos2(at.min.x + 12.0 + shown * (at.width() - 24.0), at.center().y);
    let (fill, ink) = match (on, t.dark) {
        (true, _) => (paper, t.selection_edge),
        (false, true) => (t.text_dim, t.surface),
        (false, false) => (t.surface, t.text_dim),
    };
    p.circle_filled(knob, 10.0, fill);
    if !on && !t.dark {
        p.circle_stroke(knob, 10.0, Stroke::new(1.0, t.border_strong));
    }
    p.line(vec![knob + vec2(-3.6, 0.2), knob + vec2(-1.0, 2.8), knob + vec2(3.8, -2.8)], Stroke::new(1.8, ink));
    if resp.has_focus() {
        p.rect_stroke(at.expand(2.0), CornerRadius::same(14), Stroke::new(1.5, t.focus_ring), egui::StrokeKind::Outside);
    }
    resp
}
