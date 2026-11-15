// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): Nora's bar over the message list, and what « Trier avec Nora » proposes
// //// Neoffice — above it (variant D).

//! Nora at the foot of the message list, as the list of variant D has her (`mail_view::spark_list`): a bar that floats at the
//! list's bottom right, « Demander à Nora… » beside her orb. What is typed there goes to her pane with Enter: the pane opens and
//! the question is sent, as if it was typed in it.
//!
//! Above the bar, one line, only when « Trier avec Nora » has something to say of the Inbox in view
//! ([`hint`]): what she can file where she files nothing yet (« Je peux ranger 24 notifications et 23 newsletters », read in
//! what she read of the mailbox: `mail_sorting.survey`) with « Voir le plan », which opens the sorting's conversation in her pane;
//! her tidying under way; what her last run filed today, with « Voir le journal ». Its ✕ hides it for that mailbox until the next
//! day (`UiState::nora_hint_hidden`). « Trier avec Nora » stays in her pane.
//!
//! Neither shows while her pane is open (it has her conversation and the sorting), nor where no host reaches her.

use std::collections::BTreeMap;

use egui::{Align2, CornerRadius, Id as EguiId, Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::Module;

use super::sorting::{self, Intent};
use super::sorting_view::{Ds, kinds_said, ran_on, run_summary};
use crate::MailApp;
use crate::i18n::fmt;
use crate::theme::{paint_clipped, regular, semibold};

/// The bar: its widest, its height, and how far it keeps from the list's right edge (clear of the scroll bar) and foot.
const BAR_W: f32 = 380.0;
const BAR_H: f32 = 46.0;
const RIGHT: f32 = 40.0;
const FOOT: f32 = 22.0;
/// The orb in it.
const ORB: f32 = 28.0;
/// The line above it: its height and the gap under it.
const HINT_H: f32 = 32.0;
const HINT_GAP: f32 = 10.0;
/// Its ✕.
const CLOSE: f32 = 22.0;
/// Above the line (or the bar), the list fades into its paper over this, then the paper hides it down to its foot.
const FADE: f32 = 20.0;
const ABOVE: f32 = 10.0;
/// The narrowest list the bar is drawn over.
const LIST_MIN: f32 = 220.0;

/// What the line above the bar says, and what its button does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hint {
    /// The mailbox, by the name of the server's account for it.
    pub(crate) account: String,
    pub(crate) words: String,
    pub(crate) button: &'static str,
    pub(crate) intent: Intent,
}

/// Whether the bar is drawn: a host reaches Nora's conversation, Mail is in front, and her pane is closed.
pub(crate) fn shown(app: &MailApp) -> bool {
    super::assist::available(app) && app.session.view.module == Module::Mail && !super::assist::shown(app)
}

/// Whether the line of `account` was hidden today.
fn hidden(app: &MailApp, account: &str) -> bool {
    app.ui.nora_hint_hidden.get(account).is_some_and(|day| *day == app.session.today().to_days())
}

/// What the line above the bar says, for the Inbox in view: `None` where « Trier avec Nora » has nothing to say of it, or its ✕
/// hid it today.
pub(crate) fn hint(app: &MailApp) -> Option<Hint> {
    let account = sorting::box_in_view(app, true)?;
    if hidden(app, &account) {
        return None;
    }
    let state = app.nora.sorting.available(&account)?;
    let today = app.session.today();
    let journal = || Intent::Open { account: account.clone(), journal: true };
    if let Some(tidy) = state.tidy.as_ref().filter(|t| t.running()) {
        let words =
            tln!(tidy.filed, "Nora is tidying your inbox: {n} message filed so far.", "Nora is tidying your inbox: {n} messages filed so far.");
        return Some(Hint { account: account.clone(), words, button: tl!("See the journal"), intent: journal() });
    }
    let known = app.nora.sorting.boxes.get(&account);
    if state.on {
        let run = known.and_then(|k| k.latest.as_ref()).filter(|r| r.filed > 0 && ran_on(&r.started_at, today))?;
        return Some(Hint { account: account.clone(), words: run_summary(run, today), button: tl!("See the journal"), intent: journal() });
    }
    let survey = known.and_then(|k| k.survey.as_ref())?;
    let kinds: BTreeMap<String, u64> =
        [("notifications".to_string(), survey.kind("notification")), ("newsletters".to_string(), survey.kind("newsletter"))].into();
    let what = kinds_said(&kinds);
    if what.is_empty() {
        return None;
    }
    let words = fmt(tl!("I can file {what}."), &[("what", &what)]);
    Some(Hint { account: account.clone(), words, button: tl!("See the plan"), intent: Intent::Open { account, journal: false } })
}

/// How much room the list keeps at its foot for the bar, the line above it and their backdrop, so that its last row can be
/// scrolled above them: none where the bar isn't drawn.
pub(crate) fn room_below(app: &MailApp, room: Rect) -> f32 {
    if !shown(app) || room.width() < LIST_MIN {
        return 0.0;
    }
    FOOT + BAR_H + if hint(app).is_some() { HINT_GAP + HINT_H } else { 0.0 } + ABOVE
}

/// The colour `over` laid on `under`, opaque: the design system's soft signal is a veil, which the rows would show through.
fn opaque(under: egui::Color32, over: egui::Color32) -> egui::Color32 {
    let [r, g, b, a] = over.to_srgba_unmultiplied();
    under.lerp_to_gamma(egui::Color32::from_rgb(r, g, b), f32::from(a) / 255.0)
}

/// The bar and the line above it, over the list whose room is `room` (its rows ending at `rows_end`, where the scroll bar's lane
/// starts), in a layer of their own above it, on a backdrop of the list's paper that hides the rows under them (a short fade
/// above it).
pub(crate) fn show(app: &mut MailApp, ui: &mut Ui, room: Rect, rows_end: f32) {
    if !shown(app) || room.width() < LIST_MIN {
        return;
    }
    let ds = Ds::of(ui.visuals().dark_mode);
    let paper = crate::theme::Tokens::get(ui.ctx()).surface;
    let width = BAR_W.min(room.width() - RIGHT - 16.0);
    let bar = Rect::from_min_size(pos2(room.max.x - RIGHT - width, room.max.y - FOOT - BAR_H), vec2(width, BAR_H));
    // What she read of the Inbox's mailbox is asked for the proposal (sorting.rs), as long as the bar is drawn.
    if let Some(account) = sorting::box_in_view(app, true) {
        app.nora.sorting.want_proposal(&account);
    }
    let hint = hint(app);
    let line_at = hint.as_ref().map(|hint| measure(ui, hint, pos2(bar.max.x, bar.min.y - HINT_GAP), room.width() - RIGHT - 16.0));
    let top = line_at.map_or(bar.min.y, |(rect, _)| rect.min.y) - ABOVE;
    let block = Rect::from_min_max(pos2(room.min.x, top - FADE), pos2(rows_end.max(room.min.x), room.max.y));
    let mut intent = None;
    let mut hide = None;
    egui::Area::new(EguiId::unique("nora_ask_bar")).order(egui::Order::Middle).fixed_pos(block.min).show(ui.ctx(), |ui| {
        let _ = ui.allocate_exact_size(block.size(), Sense::hover());
        app.hit.add("noraBackdrop", 0, block);
        // The fade, then the paper.
        let p = ui.painter();
        let fade = Rect::from_min_max(block.min, pos2(block.max.x, top));
        let mut mesh = egui::Mesh::default();
        let clear = paper.gamma_multiply(0.0);
        for (at, color) in [(fade.left_top(), clear), (fade.right_top(), clear), (fade.right_bottom(), paper), (fade.left_bottom(), paper)] {
            mesh.colored_vertex(at, color);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(egui::Shape::mesh(mesh));
        p.rect_filled(Rect::from_min_max(pos2(block.min.x, top), block.max), 0.0, paper);
        if let (Some(hint), Some((rect, button_w))) = (&hint, line_at) {
            let (clicked, closed) = line(app, ui, &ds, hint, rect, button_w, paper);
            if clicked {
                intent = Some(hint.intent.clone());
            }
            if closed {
                hide = Some(hint.account.clone());
            }
        }
        ask(app, ui, &ds, bar);
    });
    if let Some(account) = hide {
        let today = app.session.today().to_days();
        app.ui.nora_hint_hidden.retain(|_, day| *day >= today);
        app.ui.nora_hint_hidden.insert(account, today);
    }
    if let Some(intent) = intent {
        sorting::act(app, intent);
    }
}

/// The bar: the orb, and the box where the question is typed; Enter asks.
fn ask(app: &mut MailApp, ui: &mut Ui, ds: &Ds, bar: Rect) {
    let rect = bar;
    app.hit.add("noraAsk", 0, rect);
    let p = ui.painter();
    let shade = egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 115 } else { 40 });
    p.add(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: shade }.as_shape(rect, CornerRadius::same(23)));
    p.rect(rect, CornerRadius::same(23), ds.card, Stroke::new(1.0, ds.border_strong), egui::StrokeKind::Inside);
    let orb = Rect::from_min_size(pos2(rect.min.x + 10.0, rect.center().y - ORB / 2.0), vec2(ORB, ORB));
    craft_nora_ui::orb::paint(p, orb, ds.orb, None);
    let field = Rect::from_min_max(pos2(orb.max.x + 10.0, rect.min.y + 6.0), pos2(rect.max.x - 16.0, rect.max.y - 6.0));
    let id = EguiId::unique("nora_ask_text");
    let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
    let placeholder = egui::RichText::new(tl!("Ask Nora…")).font(regular(15.0)).color(ds.weak);
    let edit = egui::TextEdit::singleline(&mut text)
        .id(id.with("edit"))
        .frame(egui::Frame::NONE)
        .font(regular(15.0))
        .text_color(ds.text)
        .hint_text(placeholder)
        .align(egui::Align2::LEFT_CENTER)
        .desired_width(field.width());
    let resp = ui.put(field, edit);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tl!("Ask Nora…")));
    app.hit.add("noraAskText", 0, field);
    let asked = resp.lost_focus() && crate::keys::enter(ui.ctx(), ui.layer_id()) && !text.trim().is_empty();
    if asked && super::assist::ask(app, &text) {
        text.clear();
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

/// Where the line above the bar is, its right end at `end` (its foot's middle), at most `widest` wide; and its button's width.
fn measure(ui: &Ui, hint: &Hint, end: egui::Pos2, widest: f32) -> (Rect, f32) {
    let button_w = ui.painter().layout_no_wrap(hint.button.to_string(), semibold(12.0), egui::Color32::WHITE).size().x + 18.0;
    let words_w = ui.painter().layout_no_wrap(hint.words.clone(), regular(13.0), egui::Color32::WHITE).size().x;
    let width = (12.0 + words_w + 8.0 + button_w + 4.0 + CLOSE + 6.0).min(widest.max(0.0));
    (Rect::from_min_max(pos2(end.x - width, end.y - HINT_H), end), button_w)
}

/// The line above the bar in `rect`: its words, its button, its ✕. Whether the button and the ✕ were clicked.
fn line(app: &mut MailApp, ui: &mut Ui, ds: &Ds, hint: &Hint, rect: Rect, button_w: f32, paper: egui::Color32) -> (bool, bool) {
    let said = ui.interact(rect, ui.scope_id().with("nora_hint_line"), Sense::hover());
    said.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &hint.words));
    // (Cut short in a narrow list: the whole of it under the pointer.)
    let _ = said.on_hover_text(&hint.words);
    app.hit.add("noraHint", 0, rect);
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(16), opaque(paper, ds.signal_soft), Stroke::new(1.0, opaque(paper, ds.signal_border)), egui::StrokeKind::Inside);
    let close = Rect::from_center_size(pos2(rect.max.x - 6.0 - CLOSE / 2.0, rect.center().y), vec2(CLOSE, CLOSE));
    let button = Rect::from_center_size(pos2(close.min.x - 4.0 - button_w / 2.0, rect.center().y), vec2(button_w, 22.0));
    paint_clipped(
        p,
        pos2(rect.min.x + 12.0, rect.center().y),
        (button.min.x - 8.0 - rect.min.x - 12.0).max(0.0),
        &hint.words,
        regular(13.0),
        ds.text,
    );
    let b = ui.interact(button, ui.scope_id().with("nora_hint_button"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, hint.button));
    app.hit.add("noraHintButton", 0, button);
    let x = ui
        .interact(close, ui.scope_id().with("nora_hint_close"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tl!("Hide until tomorrow"));
    x.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Hide until tomorrow")));
    app.hit.add("noraHintClose", 0, close);
    let p = ui.painter();
    p.rect_filled(button, CornerRadius::same(11), if b.hovered() { ds.primary.gamma_multiply(0.88) } else { ds.primary });
    p.text(button.center(), Align2::CENTER_CENTER, hint.button, semibold(12.0), ds.primary_fg);
    if x.hovered() {
        p.circle_filled(close.center(), CLOSE / 2.0, ds.signal_border);
    }
    let c = close.center();
    for (a, b) in [(vec2(-3.5, -3.5), vec2(3.5, 3.5)), (vec2(-3.5, 3.5), vec2(3.5, -3.5))] {
        p.line_segment([c + a, c + b], Stroke::new(1.4, ds.text_2));
    }
    (b.clicked(), x.clicked())
}

#[cfg(test)]
#[path = "ask_bar_tests.rs"]
mod tests;
