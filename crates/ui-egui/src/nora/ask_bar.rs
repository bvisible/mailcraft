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

use egui::{CornerRadius, Id as EguiId, Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::Module;
use serde_json::json;

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
/// Above the line (or the bar), the list starts to fade into its paper over this; under them it stays veiled, never hidden: the
/// paper is laid at most at [`VEIL`] (the line and the bar have their own opaque grounds).
const FADE: f32 = 28.0;
const ABOVE: f32 = 10.0;
/// How much of the paper is laid over the rows: where the line starts, and at the foot.
const VEIL_TOP: f32 = 0.35;
const VEIL: f32 = 0.55;
/// The narrowest list the bar is drawn over.
const LIST_MIN: f32 = 220.0;

// //// Neoffice — whether the line above the bar is there to carry an offer, over a list `width` wide (stream_correct.rs).
/// Whether the bar, and the line above it, are drawn over a list `width` wide.
pub(crate) fn drawn(app: &MailApp, width: f32) -> bool {
    shown(app) && width >= LIST_MIN
}

/// What the line above the bar says, and what its button does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hint {
    /// The mailbox, by the name of the server's account for it.
    pub(crate) account: String,
    pub(crate) words: String,
    pub(crate) button: String,
    /// Shorter ways to say its words and its button where the list is narrow, longest first (`measure`); the whole of them under
    /// the pointer.
    pub(crate) words_shorter: Vec<String>,
    pub(crate) button_shorter: Vec<String>,
    pub(crate) intent: Intent,
    /// What its ✕ does: hides it for the mailbox until the next day, or lets the offer go; none for her questions, which go once
    /// answered.
    pub(crate) close: Close,
}

/// What a line's ✕ does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Close {
    UntilTomorrow,
    Offer,
    // //// Neoffice — the offer that follows « Classer dans »: its ✕ lets it go (stream_correct.rs).
    StreamOffer,
    None,
}

/// Whether the bar is drawn: a host reaches Nora's conversation, Mail is in front, and her pane is closed.
pub(crate) fn shown(app: &MailApp) -> bool {
    super::assist::available(app) && app.session.view.module == Module::Mail && !super::assist::shown(app)
}

/// Whether the line of `account` was hidden today.
fn hidden(app: &MailApp, account: &str) -> bool {
    app.ui.nora_hint_hidden.get(account).is_some_and(|day| *day == app.session.today().to_days())
}

/// What the line above the bar says: first the offer that follows a ⚡ (« Toujours prioritaire pour … », a few seconds), then her
/// questions (« Nora a 3 questions · Voir », as long as some wait, in any folder), then what « Trier avec Nora » has to say of the
/// Inbox in view, unless its ✕ hid it today. `None` where there is nothing.
pub(crate) fn hint(app: &MailApp) -> Option<Hint> {
    // //// Neoffice — first the offer that follows « Classer dans »: the same for the senders' next ones (stream_correct.rs).
    if let Some(offer) = crate::stream_correct::hint(app) {
        return Some(offer);
    }
    // (Not over another mailbox than the one of the bolt.)
    if let Some(offer) = sorting::sender_offer(app).filter(|o| sorting::box_in_view(app, false).is_none_or(|a| a == o.account)) {
        let button = if offer.on { tl!("Always priority for {sender}") } else { tl!("No longer priority for {sender}") };
        let words = if offer.on { tl!("Marked as priority.") } else { tl!("Taken out of Priority.") };
        // Narrower: the sender's name cut short (three letters at the least), then without it.
        let name: Vec<char> = offer.name.trim().chars().collect();
        let mut button_shorter: Vec<String> = (3..name.len())
            .rev()
            .map(|keep| fmt(button, &[("sender", &format!("{}…", name[..keep].iter().collect::<String>().trim_end()))]))
            .collect();
        button_shorter.push(if offer.on { tl!("Always priority") } else { tl!("No longer priority") }.to_string());
        return Some(Hint {
            account: offer.account.clone(),
            words: words.to_string(),
            button: fmt(button, &[("sender", &offer.name)]),
            words_shorter: Vec::new(),
            button_shorter,
            intent: Intent::PrioritySender { account: offer.account.clone(), email: offer.email.clone(), on: offer.on },
            close: Close::Offer,
        });
    }
    let asked = super::sorting_view::questions_count(app);
    if asked > 0 {
        return Some(Hint {
            account: String::new(),
            words: tln!(asked, "Nora has {n} question", "Nora has {n} questions"),
            button: tl!("See").to_string(),
            words_shorter: vec![tln!(asked, "{n} question", "{n} questions")],
            button_shorter: Vec::new(),
            intent: Intent::Questions,
            close: Close::None,
        });
    }
    proposal(app)
}

/// What « Trier avec Nora » has to say of the Inbox in view: `None` where it has nothing, or its ✕ hid it today.
fn proposal(app: &MailApp) -> Option<Hint> {
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
        return Some(Hint {
            account: account.clone(),
            words,
            button: tl!("See the journal").to_string(),
            words_shorter: Vec::new(),
            button_shorter: Vec::new(),
            intent: journal(),
            close: Close::UntilTomorrow,
        });
    }
    let known = app.nora.sorting.boxes.get(&account);
    if state.on {
        let run = known.and_then(|k| k.latest.as_ref()).filter(|r| r.filed > 0 && ran_on(&r.started_at, today))?;
        let words = run_summary(run, today);
        return Some(Hint {
            account: account.clone(),
            words,
            button: tl!("See the journal").to_string(),
            words_shorter: Vec::new(),
            button_shorter: Vec::new(),
            intent: journal(),
            close: Close::UntilTomorrow,
        });
    }
    let survey = known.and_then(|k| k.survey.as_ref())?;
    let kinds: BTreeMap<String, u64> =
        [("notifications".to_string(), survey.kind("notification")), ("newsletters".to_string(), survey.kind("newsletter"))].into();
    let what = kinds_said(&kinds);
    if what.is_empty() {
        return None;
    }
    let words = fmt(tl!("I can file {what}."), &[("what", &what)]);
    Some(Hint {
        account: account.clone(),
        words,
        button: tl!("See the plan").to_string(),
        words_shorter: Vec::new(),
        button_shorter: Vec::new(),
        intent: Intent::Open { account, journal: false },
        close: Close::UntilTomorrow,
    })
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
/// starts), in a layer of their own above it, on a veil of the list's paper through which the rows under them still read (a fade
/// above it). Only the line and the bar take the pointer: a row seen through the veil beside them is clicked as anywhere else.
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
    // (As wide as the bar at the most: inside the list at any width.)
    let line_at = hint.as_ref().map(|hint| measure(ui, hint, pos2(bar.max.x, bar.min.y - HINT_GAP), room.width() - RIGHT - 16.0));
    let top = line_at.as_ref().map_or(bar.min.y, |fitted| fitted.rect.min.y) - ABOVE;
    let block = Rect::from_min_max(pos2(room.min.x, top - FADE), pos2(rows_end.max(room.min.x), room.max.y));
    let mut intent = None;
    let mut hide = None;
    // What takes the pointer: the bar (the area is its own, from its corner), and the line (its own `nora_hint_line`), nothing
    // beside them.
    egui::Area::new(EguiId::unique("nora_ask_bar")).order(egui::Order::Middle).fixed_pos(bar.min).show(ui.ctx(), |ui| {
        let _ = ui.allocate_rect(bar, Sense::hover());
        app.hit.add("noraBackdrop", 0, block);
        // The veil: clear above, then a little of the paper, a little more down to the foot. (Drawn by the area's painter, it
        // stands where the area is not: the pointer goes through it to the rows.)
        let p = ui.painter().with_clip_rect(block);
        let veil = |alpha: f32| paper.gamma_multiply(alpha);
        let mut mesh = egui::Mesh::default();
        let bands = [(block.min.y, veil(0.0)), (top, veil(VEIL_TOP)), (block.max.y, veil(VEIL))];
        for (i, pair) in bands.windows(2).enumerate() {
            let [(y0, c0), (y1, c1)] = [pair[0], pair[1]];
            let base = (i * 4) as u32;
            for (at, color) in [(pos2(block.min.x, y0), c0), (pos2(block.max.x, y0), c0), (pos2(block.max.x, y1), c1), (pos2(block.min.x, y1), c1)] {
                mesh.colored_vertex(at, color);
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        p.add(egui::Shape::mesh(mesh));
        if let (Some(hint), Some(fitted)) = (&hint, &line_at) {
            let (clicked, closed) = line(app, ui, &ds, hint, fitted, paper);
            if clicked {
                intent = Some(hint.intent.clone());
            }
            if closed {
                hide = Some((hint.close, hint.account.clone()));
            }
        }
        ask(app, ui, &ds, bar);
    });
    match hide {
        Some((Close::UntilTomorrow, account)) => {
            let today = app.session.today().to_days();
            app.ui.nora_hint_hidden.retain(|_, day| *day >= today);
            app.ui.nora_hint_hidden.insert(account, today);
        }
        Some((Close::Offer, _)) => sorting::drop_sender_offer(app),
        // //// Neoffice — see `Close::StreamOffer`.
        Some((Close::StreamOffer, _)) => crate::stream_correct::drop_offer(app),
        _ => {}
    }
    // Her questions: her pane on them; the rest is « Trier avec Nora »'s.
    match intent {
        Some(Intent::Questions) => {
            let _ = app.run("ui.noraQuestions", json!({}));
        }
        Some(intent) => sorting::act(app, intent),
        None => {}
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

/// The line above the bar as it fits: where it is, and what its words and its button say there.
pub(crate) struct Fitted {
    pub(crate) rect: Rect,
    /// Its words, `None` where there is no room for them.
    pub(crate) words: Option<String>,
    pub(crate) button: String,
    pub(crate) button_w: f32,
}

/// The first of `ways` (longest first) at most `room` wide in `font`, and its width; where none is, the last (cut short when
/// drawn, at `room`).
fn fit(ui: &Ui, ways: &[&String], font: &egui::FontId, room: f32) -> (String, f32) {
    let wide = |way: &String| ui.painter().layout_no_wrap(way.clone(), font.clone(), egui::Color32::WHITE).size().x;
    for way in ways {
        let w = wide(way);
        if w <= room {
            return ((*way).clone(), w);
        }
    }
    let last = ways.last().map(|w| (*w).clone()).unwrap_or_default();
    (last, room.max(0.0))
}

/// Where the line above the bar is, its right end at `end` (its foot's middle), at most `widest` wide, and what it says there:
/// its button first (the shorter ways of it, then cut short), then its words in what is left (the shorter ways, then cut short,
/// or none); an offer whose words do not fit whole says only its button (they but repeat what the bolt just did).
fn measure(ui: &Ui, hint: &Hint, end: egui::Pos2, widest: f32) -> Fitted {
    let widest = widest.max(0.0);
    let close = if hint.close == Close::None { 0.0 } else { CLOSE };
    // Its padding: 12 before what it says, 4 between the button and the ✕, 6 after; 8 between the words and the button.
    let pad = 12.0 + 4.0 + close + 6.0;
    let buttons: Vec<&String> = std::iter::once(&hint.button).chain(&hint.button_shorter).collect();
    let (button, text_w) = fit(ui, &buttons, &semibold(12.0), widest - pad - 18.0);
    let button_w = text_w + 18.0;
    let room = widest - pad - button_w - 8.0;
    // //// Neoffice — (and the offer that follows « Classer dans »)
    let words = if matches!(hint.close, Close::Offer | Close::StreamOffer) {
        let w = ui.painter().layout_no_wrap(hint.words.clone(), regular(13.0), egui::Color32::WHITE).size().x;
        (w <= room).then(|| (hint.words.clone(), w))
    } else {
        let ways: Vec<&String> = std::iter::once(&hint.words).chain(&hint.words_shorter).collect();
        let (said, w) = fit(ui, &ways, &regular(13.0), room);
        (w >= 40.0).then_some((said, w))
    };
    let width = (pad + button_w + words.as_ref().map_or(0.0, |(_, w)| 8.0 + w)).min(widest);
    Fitted { rect: Rect::from_min_max(pos2(end.x - width, end.y - HINT_H), end), words: words.map(|(w, _)| w), button, button_w }
}

/// The line above the bar in `rect`: its words, its button, its ✕. Whether the button and the ✕ were clicked.
fn line(app: &mut MailApp, ui: &mut Ui, ds: &Ds, hint: &Hint, fitted: &Fitted, paper: egui::Color32) -> (bool, bool) {
    let (rect, button_w) = (fitted.rect, fitted.button_w);
    let said = ui.interact(rect, ui.scope_id().with("nora_hint_line"), Sense::hover());
    said.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &hint.words));
    // (Cut short in a narrow list: the whole of it under the pointer.)
    let _ = said.on_hover_text(&hint.words);
    app.hit.add("noraHint", 0, rect);
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(16), opaque(paper, ds.signal_soft), Stroke::new(1.0, opaque(paper, ds.signal_border)), egui::StrokeKind::Inside);
    // (No ✕ for her questions: the line goes once they are answered.)
    let closable = hint.close != Close::None;
    let close = Rect::from_center_size(pos2(rect.max.x - 6.0 - CLOSE / 2.0, rect.center().y), vec2(CLOSE, CLOSE));
    let button_end = if closable { close.min.x - 4.0 } else { rect.max.x - 6.0 };
    let button = Rect::from_center_size(pos2(button_end - button_w / 2.0, rect.center().y), vec2(button_w, 22.0));
    if let Some(words) = &fitted.words {
        let room = button.min.x - 8.0 - rect.min.x - 12.0;
        paint_clipped(p, pos2(rect.min.x + 12.0, rect.center().y), room, words, regular(13.0), ds.text);
    }
    let mut b = ui.interact(button, ui.scope_id().with("nora_hint_button"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    // (Said shorter in a narrow list: the whole of it under the pointer, as its words are over the line.)
    if fitted.button != hint.button {
        b = b.on_hover_text(&hint.button);
    }
    b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &hint.button));
    app.hit.add("noraHintButton", 0, button);
    let p = ui.painter();
    p.rect_filled(button, CornerRadius::same(11), if b.hovered() { ds.primary.gamma_multiply(0.88) } else { ds.primary });
    crate::theme::paint_clipped(
        p,
        pos2(button.min.x + 9.0, button.center().y),
        (button.width() - 18.0).max(0.0),
        &fitted.button,
        semibold(12.0),
        ds.primary_fg,
    );
    if !closable {
        return (b.clicked(), false);
    }
    // //// Neoffice — (and the offer that follows « Classer dans »)
    let words = if matches!(hint.close, Close::Offer | Close::StreamOffer) { tl!("Close") } else { tl!("Hide until tomorrow") };
    let x = ui
        .interact(close, ui.scope_id().with("nora_hint_close"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(words);
    x.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, words));
    app.hit.add("noraHintClose", 0, close);
    let p = ui.painter();
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
