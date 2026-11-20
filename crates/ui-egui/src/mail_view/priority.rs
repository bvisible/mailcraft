// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Prioritaire » at the head of the list of variant D.

//! « Prioritaire » at the head of Principal in the list of variant D (`super::spark_list`): what waits for the person's answer,
//! according to Nora (`nora.api.v2.mail_sorting.priority`, read for each mailbox of the Inboxes in view: `sorting::want_priority`).
//! Of what the server names, the list shows what is here, in the Inbox, and not answered (the server can't know `$answered`); the
//! newest three, « Afficher tout (N) » for the rest. Each message keeps its row in the list under it too: nothing is hidden. Its
//! reason (a question, a request for a quote, an order…) is said under the pointer. A row's « ⚡ » (its quick actions: done.rs)
//! puts a message in or out of it (`priority_set`), in the Inbox and in search results alike ([`bolt_of`]): a search reads what
//! is in it for the mailboxes of what it found ([`searched_boxes`]), and no section heads its results.

use std::collections::HashMap;

use egui::{CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::Module;
use mailcraft_engine::list::Row;
use mailcraft_model::{FolderKind, Id};

use crate::MailApp;
use crate::nora::sorting;
use crate::theme::{Tokens, paint_clipped, regular, semibold};

/// How many rows show folded.
const FOLDED: usize = 3;
/// The section's heading, its foot (« Afficher tout »), its margins.
const HEAD_H: f32 = 36.0;
const FOOT_H: f32 = 34.0;
const MARGIN: f32 = 8.0;

/// A message of « Prioritaire »: its id here, and why Nora put it there (the server's code).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Waiting {
    pub(crate) id: Id,
    pub(crate) why: String,
}

/// The reason said for a code, in the interface's language.
pub(crate) fn why_said(code: &str) -> &'static str {
    match code {
        "question" => tl!("A question"),
        "quote_request" => tl!("A request for a quote"),
        "order" => tl!("An order"),
        "supplier_offer" => tl!("A supplier's offer"),
        "conversation" => tl!("A conversation under way"),
        "personal" => tl!("Personal"),
        "reply" => tl!("Waits for a reply"),
        "sales_order" => tl!("A sales order"),
        "opportunity" => tl!("An opportunity"),
        "task" => tl!("A task"),
        "marked" => tl!("Marked as priority"),
        "sender" => tl!("Priority sender"),
        _ => tl!("Waits for your answer"),
    }
}

/// Whether « Prioritaire » is offered: the list of variant D, Principal of an Inbox (or « Toutes les boîtes »), no search, where
/// a host carries Nora's sorting.
pub(crate) fn offered(app: &MailApp) -> bool {
    super::spark_list(app)
        && sorting::offered(app)
        && app.session.view.search.trim().is_empty()
        && super::bundle::open_group(app.session.view.filter).is_none()
        && app.session.mailbox.folder(app.session.view.folder).is_some_and(|f| f.kind == FolderKind::Inbox)
}

/// Whether the list shows search results where a host carries Nora's sorting (variant D): their rows have the quick actions of the
/// Inbox's, the ⚡ of a message of an Inbox among them.
pub(crate) fn searched(app: &MailApp) -> bool {
    super::spark_list(app) && sorting::offered(app) && app.session.view.module == Module::Mail && !app.session.view.search.trim().is_empty()
}

/// The ⚡ of the message `m` in a row of the list: where « Prioritaire » is offered (the Inbox in view) or among search results, for
/// a message of an Inbox of a mailbox the hosting page handed over that its server has: that mailbox's name on the server, the
/// message's JMAP id, and whether it is in « Prioritaire ». `None` for any other.
pub(crate) fn bolt_of(app: &MailApp, m: &mailcraft_model::Message) -> Option<(String, String, bool)> {
    if !(offered(app) || searched(app)) || m.draft {
        return None;
    }
    app.session.mailbox.folder(m.folder).filter(|f| f.kind == FolderKind::Inbox)?;
    let account = crate::nora::page_mailbox(app, m.account)?;
    let email = m.server.as_ref().map(|s| s.id.clone()).filter(|id| mailcraft_engine::cmd::jmap_down::plausible_id(id))?;
    let on = sorting::in_priority(app, &account, &email);
    Some((account, email, on))
}

/// The Inboxes of the messages among `ids` (what a search found), each with the name of its mailbox on the server: those whose
/// « Prioritaire » the bolts of the results read.
pub(crate) fn searched_boxes(app: &MailApp, ids: impl IntoIterator<Item = Id>) -> Vec<(Id, String)> {
    let mb = &app.session.mailbox;
    let mut out: Vec<(Id, String)> = Vec::new();
    for id in ids {
        let Some(m) = mb.message(id) else { continue };
        if out.iter().any(|(inbox, _)| *inbox == m.folder) || mb.folder(m.folder).is_none_or(|f| f.kind != FolderKind::Inbox) {
            continue;
        }
        if let Some(account) = crate::nora::page_mailbox(app, m.account) {
            out.push((m.folder, account));
        }
    }
    out
}

/// The Inboxes in view, each with the name of its mailbox on the server (`flow::page_mailbox`).
pub(crate) fn boxes(app: &MailApp) -> Vec<(Id, String)> {
    let mb = &app.session.mailbox;
    app.session
        .all_inboxes()
        .unwrap_or_else(|| vec![app.session.view.folder])
        .into_iter()
        .filter_map(|inbox| {
            let folder = mb.folder(inbox).filter(|f| f.kind == FolderKind::Inbox)?;
            Some((inbox, crate::nora::page_mailbox(app, folder.account)?))
        })
        .collect()
}

/// What « Prioritaire » lists: of what the server names for each mailbox in view, what is in its Inbox here and not answered,
/// the newest first.
pub(crate) fn waiting(app: &MailApp) -> Vec<Waiting> {
    let mb = &app.session.mailbox;
    let mut out: Vec<(i64, Waiting)> = Vec::new();
    for (inbox, account) in boxes(app) {
        let Some(said) = sorting::priority_of(app, &account) else { continue };
        if said.ids.is_empty() {
            continue;
        }
        let here: HashMap<&str, Id> =
            mb.messages_in(inbox).filter(|m| !m.answered && !m.draft).filter_map(|m| m.server.as_ref().map(|s| (s.id.as_str(), m.id))).collect();
        for server in &said.ids {
            if let Some(id) = here.get(server.as_str()).copied()
                && let Some(m) = mb.message(id)
            {
                out.push((mailcraft_engine::list::when(m), Waiting { id, why: said.why.get(server).cloned().unwrap_or_default() }));
            }
        }
    }
    out.sort_by_key(|(when, w)| std::cmp::Reverse((*when, w.id)));
    out.into_iter().map(|(_, w)| w).collect()
}

/// Whether the section shows all of it.
fn all_id() -> egui::Id {
    egui::Id::unique("priority_all")
}

/// How many rows it shows of `n`.
fn shown(ui: &Ui, n: usize) -> usize {
    if ui.data(|d| d.get_temp::<bool>(all_id())).unwrap_or(false) { n } else { n.min(FOLDED) }
}

/// The section's height at the head of the list, rows of `row_h`; none without anything in it.
pub(super) fn height(ui: &Ui, waiting: &[Waiting], row_h: f32) -> f32 {
    if waiting.is_empty() {
        return 0.0;
    }
    let foot = if waiting.len() > FOLDED { FOOT_H } else { 6.0 };
    MARGIN + HEAD_H + row_h * shown(ui, waiting.len()) as f32 + foot + MARGIN * 2.0
}

/// The section in `rect` (of [`height`]) at the head of the list: its card, its heading, its rows (the list's own, under a scope of
/// their own), « Afficher tout (N) ».
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    app: &mut MailApp,
    ui: &mut Ui,
    rect: Rect,
    waiting: &[Waiting],
    row_h: f32,
    drawn: &mailcraft_engine::Drawn,
    focus: Option<(Id, u16)>,
) {
    let t = Tokens::get(ui.ctx());
    let card = Rect::from_min_max(pos2(rect.min.x + 6.0, rect.min.y + MARGIN), pos2(rect.max.x - 4.0, rect.max.y - MARGIN));
    app.hit.add("priority", 0, card);
    let tint = t.selection_edge;
    ui.painter().rect(card, CornerRadius::same(14), tint.gamma_multiply(0.06), Stroke::new(1.0, tint.gamma_multiply(0.22)), egui::StrokeKind::Inside);
    let head = Rect::from_min_size(card.min, vec2(card.width(), HEAD_H));
    let bolt = pos2(head.min.x + 22.0, head.center().y);
    paint_bolt(ui.painter(), bolt, tint);
    let title = tl!("Priority");
    let w = paint_clipped(ui.painter(), pos2(bolt.x + 14.0, head.center().y), (head.width() - 60.0).max(0.0), title, semibold(14.0), tint);
    let said = tl!("· what waits for your answer, according to Nora");
    let x = bolt.x + 14.0 + w + 6.0;
    paint_clipped(ui.painter(), pos2(x, head.center().y), (head.max.x - 12.0 - x).max(0.0), said, regular(12.0), t.text_dim);
    ui.painter().line_segment(
        [pos2(head.min.x + 10.0, head.max.y - 0.5), pos2(head.max.x - 10.0, head.max.y - 0.5)],
        Stroke::new(1.0, tint.gamma_multiply(0.22)),
    );
    let n = shown(ui, waiting.len());
    let rows_rect = Rect::from_min_size(pos2(card.min.x, head.max.y), vec2(card.width(), row_h * n as f32));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rows_rect).id_salt("priority_rows"), |ui| {
        for (i, w) in waiting.iter().take(n).enumerate() {
            let r = Rect::from_min_size(pos2(rows_rect.min.x, rows_rect.min.y + row_h * i as f32), vec2(rows_rect.width(), row_h));
            // (Only the rows in sight are drawn, as the list's are.)
            if !r.intersects(ui.clip_rect()) {
                continue;
            }
            let Some(m) = app.session.mailbox.message(w.id) else { continue };
            let row = Row {
                id: w.id,
                members: vec![w.id],
                others: Vec::new(),
                unread: usize::from(!m.read),
                group: None,
                category: false,
                copy: 0,
                under: None,
                open: false,
                bundle: None,
            };
            // (Upstream's row takes a swipe since #1580; the band of priority senders stays out of it: a swipe is the list's,
            // reconciled with its rows, `swipe::reconcile`.)
            super::message_row_content(app, ui, &row, r, drawn, focus);
            app.hit.add("priorityRow", w.id, r);
            let why = why_said(&w.why);
            let _ = ui.interact(r, ui.scope_id().with(("priority_why", w.id)), Sense::hover()).on_hover_text(why);
        }
    });
    if waiting.len() > FOLDED {
        let all = n == waiting.len();
        let words =
            if all { tl!("Show less").to_string() } else { crate::i18n::fmt(tl!("Show all ({count})"), &[("count", &waiting.len().to_string())]) };
        let at = Rect::from_min_size(pos2(card.min.x + super::done::D_LEAD - 6.0, rows_rect.max.y + 2.0), vec2(160.0, FOOT_H - 4.0));
        let resp = ui.interact(at, ui.scope_id().with("priority_all"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &words));
        app.hit.add("priorityAll", 0, at);
        paint_clipped(
            ui.painter(),
            pos2(at.min.x + 6.0, at.center().y),
            at.width(),
            &words,
            semibold(13.0),
            if resp.hovered() { t.text } else { tint },
        );
        if resp.clicked() {
            ui.data_mut(|d| d.insert_temp(all_id(), !all));
        }
    }
}

/// A bolt, 14 high, centred on `c`: its outline.
pub(crate) fn paint_bolt(p: &egui::Painter, c: egui::Pos2, color: egui::Color32) {
    p.add(egui::Shape::closed_line(bolt(c), Stroke::new(1.5, color)));
}

/// The bolt full (a message in « Prioritaire »): its two halves, each convex.
pub(crate) fn fill_bolt(p: &egui::Painter, c: egui::Pos2, color: egui::Color32) {
    let b = bolt(c);
    if let [a, b1, b2, d, e, f] = b.as_slice() {
        p.add(egui::Shape::convex_polygon(vec![*a, *b1, *b2, *f], color, Stroke::NONE));
        p.add(egui::Shape::convex_polygon(vec![*f, *b2, *d, *e], color, Stroke::NONE));
    }
}

/// The bolt's points, around `c`.
fn bolt(c: egui::Pos2) -> Vec<egui::Pos2> {
    vec![c + vec2(1.5, -7.0), c + vec2(-4.5, 1.0), c + vec2(-0.5, 1.0), c + vec2(-1.5, 7.0), c + vec2(4.5, -1.0), c + vec2(0.5, -1.0)]
}
