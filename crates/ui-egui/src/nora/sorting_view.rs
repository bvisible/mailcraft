// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Trier avec Nora » on screen: the button and the banner above the
// Inbox's list, the switch and the flow in her pane, the activation card.

//! « Trier avec Nora » on screen, after the planche « NeoMail avec Nora » (13.11) and what the server can do ([`super::sorting`]):
//! - above the Inbox's list of a mailbox the hosting page handed over, « Trier avec Nora » (her pane opens on the flow) and,
//!   while she files it, a banner with what her last run filed and « Voir le journal » ([`strip`]);
//! - in her pane, under its greeting: « Trier ma boîte » above the recipes and the switch « Nora range mon courrier » with
//!   its rhythm and next reading ([`home_chip`], [`home_switch`]);
//! - the flow in her pane, drawn where the panel draws an empty conversation ([`flow`]): what she read, « Comment voulez-vous
//!   que je range ? » with the ways the server knows (`state.modes`), « Voici ce que je ferais » with its folders and counts,
//!   « Ranger maintenant » / « Modifier », the tidying as it goes, « C'est rangé » with « Annuler ce rangement », « Ensuite, je
//!   continue toute seule ? », « Nora vous demande » and the journal;
//! - the activation card, a modal window ([`card`]);
//! - Nora's questions to the person ([`questions`]), one section in her pane's home (all of them) and under the flow's result
//!   (those of its mailbox), and their number on her button of the ribbon ([`ribbon_badge`]).
//!
//! What the server can't do isn't drawn: no « Par projet » (the ways are the server's), no switch « Séparer les
//! notifications » (the Inbox's tabs are the site's: `state.streams`), no question of a family's folder. The look is the
//! Neoffice design system's, as her pane has it (craft-nora-ui): the paper, ink for every action, Nora's replies on white
//! cards, the signal as a tint and on a switch that is on only, never a button; Karla for the words, Forum for a title.

use std::collections::BTreeMap;

use egui::{
    Align, Align2, Button, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Frame, Label, Layout, Margin, Response, RichText, Role, Sense,
    Stroke, TextEdit, TextStyle, Ui, WidgetInfo, pos2, vec2,
};
use mailcraft_time::Date;

use super::sorting::{self, BoxState, Flow, Intent, Plan, Questions, Run, Stage, every};
use crate::MailApp;
use crate::i18n::fmt;
use crate::theme::{Tokens, regular, semibold};

/// The Neoffice design system's colours, as her pane has them (craft-nora-ui `look.rs`, each value the token named).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ds {
    /// `--color-bg` (paper), dark `--surface-subtle`.
    pub(crate) paper: Color32,
    /// `--surface-card`.
    pub(crate) card: Color32,
    /// `--border-subtle`, `--border-default`.
    pub(crate) border: Color32,
    pub(crate) border_strong: Color32,
    /// `--text-primary`, `--text-secondary`, a weak word (`--sand-500`, dark `--sand-400`).
    pub(crate) text: Color32,
    pub(crate) text_2: Color32,
    pub(crate) weak: Color32,
    /// The primary action, ink (dark: bright paper), and what is drawn on it.
    pub(crate) primary: Color32,
    pub(crate) primary_fg: Color32,
    /// The signal (`--signal-400`), as a field (`--color-accent-soft` and its border) and as text on paper (`--signal-600`).
    pub(crate) signal: Color32,
    pub(crate) signal_soft: Color32,
    pub(crate) signal_border: Color32,
    pub(crate) signal_text: Color32,
    /// `--amber-*`, `--red-*`.
    pub(crate) warn_fill: Color32,
    pub(crate) warn_text: Color32,
    pub(crate) error_fill: Color32,
    pub(crate) error_text: Color32,
    /// The orb: `--clay-600`, dark `--clay-400`.
    pub(crate) orb: Color32,
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Ds {
    pub(crate) fn of(dark: bool) -> Ds {
        if dark {
            Ds {
                paper: rgb(0x161413),
                card: rgb(0x221C16),
                border: Color32::from_rgba_unmultiplied(0xFF, 0xFB, 0xF5, 20),
                border_strong: Color32::from_rgba_unmultiplied(0xFF, 0xFB, 0xF5, 61),
                text: rgb(0xFFFDF8),
                text_2: rgb(0xBFB5A4),
                weak: rgb(0x968C7C),
                primary: rgb(0xFFFDF8),
                primary_fg: rgb(0x141414),
                signal: rgb(0xF55D1A),
                signal_soft: Color32::from_rgba_unmultiplied(0xF5, 0x5D, 0x1A, 41),
                signal_border: Color32::from_rgba_unmultiplied(0xF5, 0x5D, 0x1A, 77),
                signal_text: rgb(0xFFB599),
                warn_fill: Color32::from_rgba_unmultiplied(0xE0, 0x98, 0x0F, 36),
                warn_text: rgb(0xF8E2B4),
                error_fill: Color32::from_rgba_unmultiplied(0xDC, 0x4A, 0x3D, 41),
                error_text: rgb(0xF6D2C9),
                orb: rgb(0xD68A59),
            }
        } else {
            Ds {
                paper: rgb(0xFAF3EA),
                card: rgb(0xFFFFFF),
                border: rgb(0xECE7DE),
                border_strong: rgb(0xDCD4C7),
                text: rgb(0x141414),
                text_2: rgb(0x524B41),
                weak: rgb(0x6F665A),
                primary: rgb(0x141414),
                primary_fg: rgb(0xFFFDF8),
                signal: rgb(0xF55D1A),
                signal_soft: rgb(0xFFEAE2),
                signal_border: rgb(0xFFD1C1),
                signal_text: rgb(0xAA3700),
                warn_fill: rgb(0xFCF2DD),
                warn_text: rgb(0x92610C),
                error_fill: rgb(0xFBEBE7),
                error_text: rgb(0x9C2920),
                orb: rgb(0xA15A2E),
            }
        }
    }
}

/// The families her pane installs (craft-nora-ui: Karla in three weights, Forum), by their names there.
const SANS: &str = "nora-sans";
const MEDIUM: &str = "nora-medium";
const TITLE: &str = "nora-title";

/// The family `name` where her pane installed it, the window's own where it didn't (yet): egui stops on a family it
/// doesn't know.
fn family(ctx: &egui::Context, name: &'static str) -> FontFamily {
    let wanted = FontFamily::Name(name.into());
    if ctx.fonts(|fonts| fonts.definitions().families.contains_key(&wanted)) { wanted } else { FontFamily::Proportional }
}

/// A font of her pane, by its family's name, at `size`.
fn pane_font(ctx: &egui::Context, name: &'static str, size: f32) -> FontId {
    FontId::new(size, family(ctx, name))
}

/// A font of the `Ui` her panel hands over: the family it set for that kind of text (Karla, Forum), at `size`.
fn face(ui: &Ui, style: TextStyle, size: f32) -> FontId {
    FontId::new(size, ui.style().text_styles.get(&style).map_or(FontFamily::Proportional, |f| f.family.clone()))
}

/// What the flow needs of the window besides the flow itself.
pub(crate) struct FlowCx {
    pub(crate) today: Date,
    /// The frame's clock, in seconds (the sphere turns with it).
    pub(crate) now: f64,
    /// The address of the mailbox, as the server says it.
    pub(crate) address: String,
    /// A call that changes something is in flight: its buttons wait.
    pub(crate) changing: bool,
    /// The person has more than one mailbox here: each question says which it is about.
    pub(crate) show_box: bool,
}

// ---- the parts ----

/// A reply of Nora's: a white card on the paper, at the left.
fn hers<R>(ui: &mut Ui, ds: &Ds, add: impl FnOnce(&mut Ui) -> R) -> R {
    let width = (ui.available_width() * 0.94 - 28.0).max(80.0);
    Frame::new()
        .fill(ds.card)
        .stroke(Stroke::new(1.0, ds.border))
        .corner_radius(CornerRadius { nw: 14, ne: 14, sw: 4, se: 14 })
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        })
        .inner
}

/// The person's words, in ink at the right.
fn mine(ui: &mut Ui, ds: &Ds, words: &str) {
    let most = ui.available_width() * 0.82;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 0.0), Layout::top_down(Align::Max), |ui| {
        Frame::new().fill(ds.primary).corner_radius(CornerRadius { nw: 14, ne: 14, sw: 14, se: 4 }).inner_margin(Margin::symmetric(14, 10)).show(
            ui,
            |ui| {
                ui.set_max_width(most);
                ui.add(Label::new(RichText::new(words).font(face(ui, TextStyle::Body, 14.0)).color(ds.primary_fg)).wrap());
            },
        );
    });
}

/// French typography as the window draws it: a guillemet never alone at the start or the end of a line, nor « ? », « ! », « : »
/// or « ; » at the start of one. The space the server (or a translation) put inside « … » and before them becomes a no-break
/// space (U+00A0, the one egui never breaks at); a text without such spaces is left as it is.
pub(crate) fn tied(words: &str) -> String {
    let chars: Vec<char> = words.chars().collect();
    chars
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let before = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
            let after = chars.get(i + 1).copied();
            let tie = *c == ' ' && (before == Some('«') || matches!(after, Some('»' | '?' | '!' | ':' | ';')));
            if tie { '\u{a0}' } else { *c }
        })
        .collect()
}

/// Words of a reply.
fn said(ui: &mut Ui, ds: &Ds, words: &str) {
    ui.add(Label::new(RichText::new(tied(words)).font(face(ui, TextStyle::Body, 14.0)).color(ds.text)).wrap());
}

/// Weak words under a reply.
fn note(ui: &mut Ui, ds: &Ds, words: &str) {
    ui.add(Label::new(RichText::new(tied(words)).font(face(ui, TextStyle::Small, 12.0)).color(ds.weak)).wrap());
}

/// A choice offered: a white pill, its border ink when it is the one chosen.
fn chip(ui: &mut Ui, ds: &Ds, label: &str, chosen: bool, enabled: bool) -> Response {
    let text = RichText::new(label).font(face(ui, TextStyle::Button, 13.0)).color(if chosen { ds.text } else { ds.text_2 });
    let stroke = Stroke::new(1.0, if chosen { ds.text } else { ds.border_strong });
    ui.add_enabled(enabled, Button::new(text).fill(ds.card).stroke(stroke).corner_radius(CornerRadius::same(255)).min_size(vec2(0.0, 30.0)))
        .on_hover_cursor(CursorIcon::PointingHand)
}

/// The action: ink, its words on it in paper.
fn primary(ui: &mut Ui, ds: &Ds, label: &str, enabled: bool) -> Response {
    let text = RichText::new(label).font(face(ui, TextStyle::Button, 13.0)).color(ds.primary_fg);
    ui.add_enabled(enabled, Button::new(text).fill(ds.primary).corner_radius(CornerRadius::same(10)).min_size(vec2(0.0, 36.0)))
        .on_hover_cursor(CursorIcon::PointingHand)
}

/// A second action: white, an ink border.
fn secondary(ui: &mut Ui, ds: &Ds, label: &str, enabled: bool) -> Response {
    let text = RichText::new(label).font(face(ui, TextStyle::Button, 13.0)).color(ds.text);
    ui.add_enabled(
        enabled,
        Button::new(text).fill(ds.card).stroke(Stroke::new(1.0, ds.text)).corner_radius(CornerRadius::same(10)).min_size(vec2(0.0, 36.0)),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
}

/// A link of her pane, in the family the panel set for that kind of text.
fn pane_link(ui: &mut Ui, ds: &Ds, label: &str, style: TextStyle, size: f32) -> Response {
    let font = face(ui, style, size);
    link(ui, ds, label, font)
}

/// A link: the signal as text on paper.
fn link(ui: &mut Ui, ds: &Ds, label: &str, font: FontId) -> Response {
    let response = ui.add(Label::new(RichText::new(label).font(font).color(ds.signal_text)).sense(Sense::click()).selectable(false));
    response.widget_info(|| WidgetInfo::labeled(Role::Link, true, label));
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A line of a list: its words at the left, its count at the right, a hairline under it.
fn line(ui: &mut Ui, ds: &Ds, label: &str, count: Option<u64>) {
    let font = face(ui, TextStyle::Body, 13.0);
    let strong = face(ui, TextStyle::Button, 13.0);
    let width = ui.available_width();
    let figure = count.map(|n| ui.painter().layout_no_wrap(n.to_string(), strong.clone(), ds.text));
    let room = width - figure.as_ref().map_or(0.0, |g| g.size().x + 12.0);
    let words = ui.painter().layout(label.to_string(), font, ds.text, room.max(40.0));
    let height = words.size().y.max(18.0) + 10.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.galley(pos2(rect.min.x, rect.center().y - words.size().y / 2.0), words, ds.text);
        if let Some(figure) = figure {
            painter.galley(pos2(rect.max.x - figure.size().x, rect.center().y - figure.size().y / 2.0), figure, ds.text);
        }
        painter.hline(rect.x_range(), rect.max.y - 0.5, Stroke::new(1.0, ds.border));
    }
}

/// A word of the pane itself: the warning's colours, or the error's.
fn notice(ui: &mut Ui, ds: &Ds, words: &str, error: bool) {
    let (fill, ink) = if error { (ds.error_fill, ds.error_text) } else { (ds.warn_fill, ds.warn_text) };
    Frame::new().fill(fill).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(12, 9)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(Label::new(RichText::new(tied(words)).font(face(ui, TextStyle::Body, 13.0)).color(ink)).wrap());
    });
}

/// What Nora is doing: her sphere turning, in a pill of the signal's tint, and how long it has taken.
fn working(ui: &mut Ui, ds: &Ds, words: &str, seconds: Option<u64>, now: f64, turning: bool) {
    Frame::new()
        .fill(ds.signal_soft)
        .stroke(Stroke::new(1.0, ds.signal_border))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin { left: 6, right: 14, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (mark, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                craft_nora_ui::orb::paint(ui.painter(), mark, ds.orb, turning.then_some(now as f32));
                let mut words = words.to_string();
                if let Some(s) = seconds.filter(|s| *s >= 1) {
                    words = format!("{words} · {s} s");
                }
                ui.add(Label::new(RichText::new(words).font(face(ui, TextStyle::Button, 13.0)).color(ds.text)).selectable(false));
            });
        });
    if turning {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
    }
}

/// A switch: the signal with an ink knob when it is on, the stone border with a white knob when it is off.
fn switch(ui: &mut Ui, ds: &Ds, on: bool, enabled: bool, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(42.0, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::selected(Role::Switch, enabled, on, label));
    if ui.is_rect_visible(rect) {
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(255), fade(if on { ds.signal } else { ds.border_strong }));
        let x = if on { rect.max.x - 12.0 } else { rect.min.x + 12.0 };
        painter.circle_filled(pos2(x, rect.center().y), 9.0, fade(if on { ds.text } else { ds.card }));
        if response.has_focus() {
            painter.rect_stroke(rect.expand(2.0), CornerRadius::same(255), Stroke::new(1.5, ds.text), egui::StrokeKind::Outside);
        }
    }
    response.on_hover_cursor(if enabled { CursorIcon::PointingHand } else { CursorIcon::NotAllowed })
}

/// The three lines of « Trier » (Lucide's `list-filter`), in `rect`.
fn sort_mark(painter: &egui::Painter, rect: egui::Rect, ink: Color32) {
    let stroke = Stroke::new(1.75, ink);
    let p = |x: f32, y: f32| pos2(rect.min.x + x / 24.0 * rect.width(), rect.min.y + y / 24.0 * rect.height());
    for (from, to, y) in [(3.0, 21.0, 6.0), (7.0, 17.0, 12.0), (10.0, 14.0, 18.0)] {
        painter.line_segment([p(from, y), p(to, y)], stroke);
    }
}

// ---- words ----

/// « 2026-11-13 10:30:05 » as the server writes a time: its day and « 10:30 ».
fn at(said: &str) -> Option<(Date, String)> {
    let (day, time) = said.trim().split_once(' ')?;
    let mut parts = day.split('-');
    let date = Date::new(parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?)?;
    let hm: String = time.chars().take(5).collect();
    let ok = hm.len() == 5 && hm.chars().enumerate().all(|(i, c)| if i == 2 { c == ':' } else { c.is_ascii_digit() });
    ok.then_some((date, hm))
}

/// A day and month, as the interface writes them (« 13.11 »).
fn day_month(date: Date) -> String {
    fmt(tl!("{month}/{day}"), &[("month", &format!("{:02}", date.month)), ("day", &format!("{:02}", date.day))])
}

/// When a run started: « Aujourd'hui à 10:30 », « Le 12.11 à 10:30 ».
pub(crate) fn when(started: &str, today: Date) -> String {
    match at(started) {
        Some((date, time)) if date == today => fmt(tl!("Today at {time}"), &[("time", &time)]),
        Some((date, time)) => fmt(tl!("On {date} at {time}"), &[("date", &day_month(date)), ("time", &time)]),
        None => String::new(),
    }
}

/// When a run started, short: « 10:30 » today, « 12.11 10:30 » another day.
fn when_short(started: &str, today: Date) -> String {
    match at(started) {
        Some((date, time)) if date == today => time,
        Some((date, time)) => format!("{} {time}", day_month(date)),
        None => String::new(),
    }
}

/// What a run filed, by kind: « 18 notifications, 6 newsletters et 5 messages de clients ».
pub(crate) fn kinds_said(kinds: &BTreeMap<String, u64>) -> String {
    let n = |k: &str| kinds.get(k).copied().unwrap_or(0);
    let other: u64 =
        kinds.iter().filter(|(k, _)| !matches!(k.as_str(), "notifications" | "newsletters" | "clients" | "invoices" | "inbox")).map(|(_, n)| n).sum();
    let parts: Vec<String> = [
        (n("notifications"), tln!(n("notifications"), "{n} notification", "{n} notifications")),
        (n("newsletters"), tln!(n("newsletters"), "{n} newsletter", "{n} newsletters")),
        (n("clients"), tln!(n("clients"), "{n} message from clients", "{n} messages from clients")),
        (n("invoices"), tln!(n("invoices"), "{n} invoice", "{n} invoices")),
        (n("inbox"), tln!(n("inbox"), "{n} message back in the inbox", "{n} messages back in the inbox")),
        (other, tln!(other, "{n} other message", "{n} other messages")),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(_, said)| said)
    .collect();
    match parts.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => fmt(tl!("{list} and {last}"), &[("list", &rest.join(", ")), ("last", last)]),
    }
}

/// The banner's words for the last run: « Aujourd'hui à 10:30 : 18 notifications, 6 newsletters et 5 messages de clients rangés. »
pub(crate) fn run_summary(run: &Run, today: Date) -> String {
    let what = kinds_said(&run.kinds);
    let what = if what.is_empty() { tln!(run.filed, "{n} message", "{n} messages") } else { what };
    let when = when(&run.started_at, today);
    if when.is_empty() {
        fmt(tl!("{what} filed."), &[("what", &what)])
    } else {
        fmt(tl!("{when}: {what} filed."), &[("when", &when), ("what", &what)])
    }
}

/// The rhythm and the next reading under the switch: « Toutes les 15 minutes · prochain passage 10:45 ».
fn rhythm(state: &BoxState) -> String {
    if !state.on {
        return tl!("Off: Nora files nothing by herself.").to_string();
    }
    let rhythm = every(if state.interval > 0 { state.interval } else { 15 });
    if state.next_said.is_empty() { rhythm } else { fmt(tl!("{every} · next run at {time}"), &[("every", &rhythm), ("time", &state.next_said)]) }
}

// ---- above the Inbox's list ----

/// « Trier avec Nora » above the Inbox's list of a mailbox the hosting page handed over, where Nora files mail, and the
/// banner of what she filed last while she files it. Nothing anywhere else.
pub(crate) fn strip(app: &mut MailApp, ui: &mut Ui) {
    let Some(account) = sorting::box_in_view(app, true) else { return };
    let Some(state) = app.nora.sorting.available(&account) else { return };
    let today = app.session.today();
    let banner = if let Some(tidy) = state.tidy.as_ref().filter(|t| t.running()) {
        Some(tln!(tidy.filed, "Nora is tidying your inbox: {n} message filed so far.", "Nora is tidying your inbox: {n} messages filed so far."))
    } else if state.on {
        let latest = app.nora.sorting.boxes.get(&account).and_then(|k| k.latest.as_ref()).filter(|r| r.filed > 0);
        Some(latest.map_or_else(|| tl!("Nora files the new mail as it comes.").to_string(), |run| run_summary(run, today)))
    } else {
        None
    };
    let t = Tokens::get(ui.ctx());
    let ds = Ds::of(t.dark);
    let mut intent = None;
    // The button, at the right of a row of its own.
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    let label = tl!("Sort with Nora");
    let words = ui.painter().layout_no_wrap(label.to_string(), semibold(13.0), t.text);
    let width = 8.0 + 22.0 + 8.0 + words.size().x + 14.0;
    let pill = egui::Rect::from_min_size(pos2(row.max.x - 16.0 - width, row.center().y - 16.0), vec2(width, 32.0));
    let response = ui.interact(pill, ui.scope_id().with("nora_sort"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    app.hit.add("noraSort", 0, pill);
    if ui.is_rect_visible(pill) {
        let painter = ui.painter();
        let fill = if response.hovered() { t.hover } else { t.surface };
        painter.rect(pill, CornerRadius::same(255), fill, Stroke::new(1.0, t.text), egui::StrokeKind::Inside);
        let mark = egui::Rect::from_min_size(pos2(pill.min.x + 6.0, pill.center().y - 11.0), vec2(22.0, 22.0));
        craft_nora_ui::orb::paint(painter, mark, ds.orb, None);
        painter.galley(pos2(mark.max.x + 8.0, pill.center().y - words.size().y / 2.0), words, t.text);
        if response.has_focus() {
            painter.rect_stroke(pill.expand(2.0), CornerRadius::same(255), Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
        }
    }
    if response.clicked() {
        intent = Some(Intent::Open { account: account.clone(), journal: false });
    }
    if let Some(said) = banner {
        Frame::new().inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 8 }).show(ui, |ui| {
            let shown =
                Frame::new().fill(ds.signal_soft).corner_radius(CornerRadius::same(14)).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
                        let (mark, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                        craft_nora_ui::orb::paint(ui.painter(), mark, ds.orb, None);
                        ui.add(Label::new(RichText::new(tl!("Nora files your mail.")).font(semibold(13.0)).color(ds.text)).selectable(false));
                        ui.add(Label::new(RichText::new(tied(&said)).font(regular(13.0)).color(ds.text)).wrap().selectable(false));
                        if link(ui, &ds, tl!("See the journal"), semibold(13.0)).clicked() {
                            intent = Some(Intent::Open { account: account.clone(), journal: true });
                        }
                    });
                });
            app.hit.add("noraBanner", 0, shown.response.rect);
        });
    }
    if let Some(intent) = intent {
        sorting::act(app, intent);
    }
}

// ---- in her pane, while the conversation is empty ----

/// What her pane's home shows of « Trier avec Nora », for the mailbox in view: `None` where Nora doesn't file it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Home {
    pub(crate) account: String,
    pub(crate) on: bool,
    pub(crate) may_set: bool,
    pub(crate) rhythm: String,
}

pub(crate) fn home_of(app: &MailApp) -> Option<Home> {
    let account = sorting::box_in_view(app, false)?;
    let state = app.nora.sorting.available(&account)?;
    Some(Home { account: account.clone(), on: state.on, may_set: state.may_set, rhythm: rhythm(state) })
}

/// « Trier ma boîte », first of what her pane offers, above the recipes.
pub(crate) fn home_chip(ui: &mut Ui, home: &Home) -> Option<Intent> {
    let ds = Ds::of(ui.visuals().dark_mode);
    let label = tl!("Sort my mailbox");
    let font = face(ui, TextStyle::Button, 14.0);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if response.hovered() { ds.signal_soft } else { ds.card };
        painter.rect(rect, CornerRadius::same(255), fill, Stroke::new(1.0, ds.text), egui::StrokeKind::Inside);
        sort_mark(painter, egui::Rect::from_min_size(pos2(rect.min.x + 14.0, rect.center().y - 9.0), vec2(18.0, 18.0)), ds.text);
        painter.text(pos2(rect.min.x + 42.0, rect.center().y), Align2::LEFT_CENTER, label, font, ds.text);
        if response.has_focus() {
            painter.rect_stroke(rect.expand(2.0), CornerRadius::same(255), Stroke::new(1.5, ds.text), egui::StrokeKind::Outside);
        }
    }
    response.clicked().then(|| Intent::Open { account: home.account.clone(), journal: false })
}

/// « Nora range mon courrier », its rhythm and next reading, and the switch (only the person in charge of the box turns it).
pub(crate) fn home_switch(ui: &mut Ui, home: &Home) -> Option<Intent> {
    let ds = Ds::of(ui.visuals().dark_mode);
    let mut intent = None;
    Frame::new()
        .fill(ds.card)
        .stroke(Stroke::new(1.0, ds.border))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let label = tl!("Nora files my mail");
                let words = ui.available_width() - 54.0;
                ui.allocate_ui_with_layout(vec2(words.max(80.0), 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.add(Label::new(RichText::new(label).font(face(ui, TextStyle::Button, 14.0)).color(ds.text)).selectable(false));
                    note(ui, &ds, &home.rhythm);
                    if !home.may_set {
                        note(ui, &ds, tl!("Only the person in charge of this mailbox can change this."));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if switch(ui, &ds, home.on, home.may_set, label).clicked() {
                        intent = Some(Intent::Switch { account: home.account.clone(), on: !home.on });
                    }
                });
            });
        });
    intent
}

// ---- the flow ----

/// The flow of « Trier avec Nora », where her panel draws an empty conversation. What the person asked, if anything.
pub(crate) fn flow(ui: &mut Ui, flow: &mut Flow, state: Option<&BoxState>, asked: &mut Questions, cx: &FlowCx) -> Option<Intent> {
    let ds = Ds::of(ui.visuals().dark_mode);
    let mut intent = None;
    let may_set = state.is_some_and(|s| s.may_set);
    let enabled = may_set && !cx.changing;
    ui.spacing_mut().item_spacing.y = 10.0;
    if pane_link(ui, &ds, tl!("‹ Back to the conversation"), TextStyle::Small, 12.5).clicked() {
        intent = Some(Intent::Back);
    }
    if let Some(state) = state.filter(|s| !s.available) {
        let why = if state.unavailable == "plan" {
            tl!("Nora's background assistants are not part of this site's plan.")
        } else {
            tl!("Nora does not file the mail on this site.")
        };
        notice(ui, &ds, why, false);
        return intent;
    }
    if !flow.journal_first {
        mine(ui, &ds, tl!("Sort my mailbox"));
        match &flow.survey {
            Some(survey) => {
                hers(ui, &ds, |ui| {
                    said(ui, &ds, &survey_said(survey, &cx.address));
                    if matches!(flow.stage, Stage::Choose | Stage::Explain) {
                        said(ui, &ds, tl!("How do you want me to file?"));
                    }
                });
            }
            None if flow.error.is_none() => working(ui, &ds, tl!("Nora is reading your mailbox…"), None, cx.now, true),
            None => {}
        }
    }
    if !may_set && state.is_some() && !flow.journal_first {
        note(ui, &ds, tl!("You read this mailbox: only the person in charge of it changes how Nora files it."));
    }
    match flow.stage {
        Stage::Choose if may_set => {
            if let Some(modes) = state.map(|s| &s.modes) {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for mode in modes {
                        let chosen = flow.chosen.as_ref().is_some_and(|c| c.code == mode.code);
                        if chip(ui, &ds, &mode.label, chosen, enabled).clicked() {
                            intent = Some(Intent::Choose(mode.clone()));
                        }
                    }
                    if modes.iter().any(|m| m.code == "activity") && chip(ui, &ds, tl!("Let me explain"), false, enabled).clicked() {
                        intent = Some(Intent::Explain);
                    }
                });
            }
            if flow.plan.as_ref().is_some_and(|p| !p.status.is_empty()) && chip(ui, &ds, tl!("Keep my plan"), false, enabled).clicked() {
                intent = Some(Intent::KeepPlan);
            }
        }
        Stage::Explain if may_set => {
            hers(ui, &ds, |ui| {
                said(ui, &ds, tl!("Tell me how you file your mail: by activity, by kind of document, what goes where."));
                ui.add(
                    TextEdit::multiline(&mut flow.wishes)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .char_limit(600)
                        .font(face(ui, TextStyle::Body, 14.0))
                        .hint_text(tl!("By activity, by supplier, the invoices apart…")),
                );
                if primary(ui, &ds, tl!("Propose a plan"), enabled).clicked() {
                    intent = Some(Intent::ProposeWishes);
                }
            });
        }
        _ => {}
    }
    if let Some(chosen) = &flow.chosen
        && !matches!(flow.stage, Stage::Choose | Stage::Explain)
    {
        mine(ui, &ds, &chosen.label);
    }
    if flow.stage == Stage::Proposing {
        let seconds = ((crate::now_ms() - flow.since) / 1000.0).max(0.0) as u64;
        working(ui, &ds, tl!("Nora is preparing the plan…"), Some(seconds), cx.now, true);
    }
    let plan = flow.plan.clone();
    if matches!(flow.stage, Stage::Plan | Stage::Tidying | Stage::Done)
        && !flow.journal_first
        && let Some(plan) = plan.as_ref()
    {
        let act = flow.stage == Stage::Plan && enabled;
        if let Some(asked) = plan_bubble(ui, &ds, flow, plan, act) {
            intent = Some(asked);
        }
    }
    if flow.stage == Stage::Tidying {
        let seconds = ((crate::now_ms() - flow.since) / 1000.0).max(0.0) as u64;
        let filed = state.and_then(|s| s.tidy.as_ref()).map_or(0, |t| t.filed);
        let words = tln!(filed, "Nora is filing your mailbox: {n} message so far", "Nora is filing your mailbox: {n} messages so far");
        working(ui, &ds, &words, Some(seconds), cx.now, true);
        if may_set && pane_link(ui, &ds, tl!("Stop filing"), TextStyle::Button, 13.0).clicked() {
            intent = Some(Intent::StopTidy);
        }
    }
    if flow.stage == Stage::Done
        && let Some(asked) = result(ui, &ds, flow, state, cx, enabled)
    {
        intent = Some(asked);
    }
    if let Some(words) = &flow.said {
        hers(ui, &ds, |ui| said(ui, &ds, words));
    }
    if let Some(error) = &flow.error {
        notice(ui, &ds, error, true);
        if secondary(ui, &ds, tl!("Try again"), !cx.changing).clicked() {
            intent = Some(Intent::Retry);
        }
    }
    if matches!(flow.stage, Stage::Done | Stage::Plan) || flow.journal_first {
        // Her questions about this mailbox: the section of her pane's home, for this mailbox only.
        if let Some(answered) = questions(ui, asked, Some(&cx.address), cx.show_box, may_set && !cx.changing) {
            intent = Some(answered);
        }
        if let Some(asked) = journal(ui, &ds, flow, cx, enabled) {
            intent = Some(asked);
        }
    }
    intent
}

/// « J'ai lu les 30 derniers jours de … : 1'248 messages. 22 clients de Neoffice lui ont écrit, 31 messages sont des
/// notifications… »
pub(crate) fn survey_said(survey: &sorting::Survey, address: &str) -> String {
    if survey.mails == 0 {
        return fmt(
            tl!("I have not read any message of {address} yet: I read the new mail as it comes, then propose a plan."),
            &[("address", address)],
        );
    }
    let mut said = vec![fmt(
        tl!("I read the last {days} days of {address}: {mails} messages."),
        &[("days", &survey.days.to_string()), ("address", address), ("mails", &survey.mails.to_string())],
    )];
    if survey.clients > 0 {
        said.push(tln!(survey.clients, "{n} client of Neoffice wrote to it.", "{n} clients of Neoffice wrote to it."));
    }
    let notifications = survey.kind("notification");
    if notifications > 0 {
        said.push(tln!(notifications, "{n} message is a notification from a tool.", "{n} messages are notifications from tools."));
    }
    let newsletters = survey.kind("newsletter");
    if newsletters > 0 {
        said.push(tln!(newsletters, "{n} message is a newsletter.", "{n} messages are newsletters."));
    }
    let invoices = survey.kind("invoice");
    if invoices > 0 {
        said.push(tln!(invoices, "{n} message is an invoice.", "{n} messages are invoices."));
    }
    said.join(" ")
}

/// « Voici ce que je ferais : » and the plan's folders, with « Ranger maintenant » and « Modifier » while it may be acted on.
fn plan_bubble(ui: &mut Ui, ds: &Ds, flow: &mut Flow, plan: &Plan, act: bool) -> Option<Intent> {
    let mut intent = None;
    hers(ui, ds, |ui| {
        let heading = if plan.status == "validated" {
            if plan.since_said.is_empty() {
                tl!("Your plan:").to_string()
            } else {
                fmt(tl!("Your plan, validated on {date}:"), &[("date", &plan.since_said)])
            }
        } else {
            tl!("Here is what I would do:").to_string()
        };
        said(ui, ds, &heading);
        if plan.folders.is_empty() {
            note(ui, ds, tl!("I have no folder to propose yet: I need to read more of this mailbox first."));
            return;
        }
        match flow.editing.as_mut().filter(|_| act) {
            Some(rows) => {
                for (i, row) in rows.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let width = ui.available_width() - 34.0;
                        ui.add_enabled(
                            row.kept,
                            TextEdit::singleline(&mut row.folder).desired_width(width.max(60.0)).char_limit(140).font(face(
                                ui,
                                TextStyle::Body,
                                13.0,
                            )),
                        );
                        let mark = if row.kept { "×" } else { "↺" };
                        let tip = if row.kept { tl!("Leave this folder out") } else { tl!("Keep this folder") };
                        let response = ui.add(Button::new(RichText::new(mark).color(ds.text)).frame(false)).on_hover_text(tip);
                        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("{tip} {}", i + 1)));
                        if response.clicked() {
                            row.kept = !row.kept;
                        }
                    });
                }
            }
            None => {
                for folder in &plan.folders {
                    match plan.clients.as_ref().filter(|c| folder.takes == "clients" && !c.shown.is_empty()) {
                        Some(clients) => {
                            for (name, count) in &clients.shown {
                                line(ui, ds, &format!("{} › {name}", folder.folder), Some(*count));
                            }
                            if clients.more > 0 {
                                let others = tln!(clients.more, "{n} other client", "{n} other clients");
                                line(ui, ds, &format!("{} › {others}", folder.folder), Some(clients.more_count));
                            }
                        }
                        None => line(ui, ds, &folder.folder, Some(folder.count)),
                    }
                }
            }
        }
        let kept = if plan.keep_unread_hours > 0 {
            fmt(tl!("I delete nothing. Unread messages younger than {hours} h stay in the inbox."), &[("hours", &plan.keep_unread_hours.to_string())])
        } else {
            tl!("I delete nothing.").to_string()
        };
        note(ui, ds, &kept);
        if act {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if primary(ui, ds, tl!("File now"), true).clicked() {
                    intent = Some(Intent::FileNow);
                }
                if flow.editing.is_some() {
                    if secondary(ui, ds, tl!("Keep Nora's plan"), true).clicked() {
                        intent = Some(Intent::KeepPlan);
                    }
                } else if secondary(ui, ds, tl!("Change this plan"), true).clicked() {
                    intent = Some(Intent::Edit);
                }
            });
            if plan.status == "validated" && flow.editing.is_none() && pane_link(ui, ds, tl!("Make a new plan"), TextStyle::Button, 13.0).clicked() {
                intent = Some(Intent::NewPlan);
            }
        }
    });
    intent
}

/// The run of the tidying this flow follows, else the newest of the journal.
fn result_run(flow: &Flow) -> Option<&Run> {
    let runs = &flow.journal.as_ref()?.runs;
    runs.iter().find(|r| !flow.tidy_run.is_empty() && r.run == flow.tidy_run).or_else(|| if flow.tidy_run.is_empty() { None } else { runs.first() })
}

/// « C'est rangé », what changed folder, « Annuler ce rangement »; then « Ensuite, je continue toute seule ? ».
fn result(ui: &mut Ui, ds: &Ds, flow: &Flow, state: Option<&BoxState>, cx: &FlowCx, enabled: bool) -> Option<Intent> {
    let mut intent = None;
    if let Some(run) = result_run(flow) {
        let seconds = run.seconds.round() as u64;
        working(ui, ds, tl!("Nora filed your mailbox"), Some(seconds), cx.now, false);
        hers(ui, ds, |ui| {
            if run.filed == 0 {
                said(ui, ds, tl!("Nothing needed filing: your inbox stays as it was."));
                return;
            }
            let changed = tln!(run.filed, "{n} message changed folder; nothing is deleted.", "{n} messages changed folder; nothing is deleted.");
            said(ui, ds, &format!("{} {changed}", tl!("It's filed.")));
            folders(ui, ds, run);
            match run.status.as_str() {
                "undone" => note(ui, ds, tl!("This filing was undone.")),
                "partly_undone" => note(ui, ds, tl!("This filing was partly undone.")),
                _ => {}
            }
            if run.can_undo && secondary(ui, ds, tl!("Undo this filing"), enabled).clicked() {
                intent = Some(Intent::Undo(run.run.clone()));
            }
        });
    }
    let on = state.is_some_and(|s| s.on);
    if !flow.journal_first && !flow.on_intent && !flow.continued && !on && enabled && result_run(flow).is_some() {
        hers(ui, ds, |ui| said(ui, ds, tl!("Next, shall I carry on by myself?")));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            let intervals: Vec<u64> = state.map(|s| s.intervals.clone()).unwrap_or_default();
            for minutes in intervals {
                if chip(ui, ds, &every(minutes), false, enabled).clicked() {
                    intent = Some(Intent::Continue(Some(minutes)));
                }
            }
            if chip(ui, ds, tl!("No, when I ask"), false, enabled).clicked() {
                intent = Some(Intent::Continue(None));
            }
        });
    }
    intent
}

/// The folders a run filed into, the busiest first, six at most and the rest in one line.
fn folders(ui: &mut Ui, ds: &Ds, run: &Run) {
    let mut listed: Vec<_> = run.folders.iter().collect();
    listed.sort_by_key(|f| std::cmp::Reverse(f.count));
    for folder in listed.iter().take(6) {
        line(ui, ds, &folder.said(), Some(folder.count));
    }
    let rest: Vec<_> = listed.iter().skip(6).collect();
    if !rest.is_empty() {
        let count: u64 = rest.iter().map(|f| f.count).sum();
        line(ui, ds, &tln!(rest.len(), "{n} other folder", "{n} other folders"), Some(count));
    }
}

/// « Nora vous demande »: her questions, each with the answers she offers, a field to answer in one's own words and « Plus
/// tard »; what she would keep of those words, to confirm or say otherwise; what she noted of the last answer. The same
/// section in her pane's home (all the person's questions) and under the flow's result (those of its mailbox, `mailbox`).
pub(crate) fn questions(ui: &mut Ui, asked: &mut Questions, mailbox: Option<&str>, show_box: bool, enabled: bool) -> Option<Intent> {
    let ds = Ds::of(ui.visuals().dark_mode);
    let mut intent = None;
    // Which mailbox each is about, where they are about more than one or the person has several.
    let mut boxes: Vec<String> = asked.list.iter().map(|a| a.mailbox.to_lowercase()).collect();
    boxes.sort();
    boxes.dedup();
    let show_box = show_box || boxes.len() > 1;
    let shown: Vec<sorting::Ask> =
        asked.list.iter().filter(|a| mailbox.is_none_or(|m| a.mailbox.is_empty() || a.mailbox.eq_ignore_ascii_case(m))).cloned().collect();
    for ask in shown {
        let name = ask.name.clone();
        hers(ui, &ds, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(Label::new(RichText::new(tl!("Nora asks you")).font(face(ui, TextStyle::Button, 14.0)).color(ds.text)).selectable(false));
                if show_box && !ask.mailbox.is_empty() {
                    ui.add(Label::new(RichText::new(&ask.mailbox).font(face(ui, TextStyle::Small, 11.5)).color(ds.weak)).selectable(false));
                }
            });
            said(ui, &ds, &ask.title);
            if !ask.context.is_empty() {
                note(ui, &ds, &ask.context);
            }
            match asked.confirming.get_mut(&name) {
                // What Nora would keep of the person's words: theirs to change, then to confirm.
                Some(confirming) => {
                    note(ui, &ds, tl!("Nora would keep:"));
                    ui.add(TextEdit::multiline(&mut confirming.retained).desired_rows(2).desired_width(f32::INFINITY).char_limit(500).font(face(
                        ui,
                        TextStyle::Body,
                        13.0,
                    )));
                    let ready = enabled && !confirming.retained.trim().is_empty();
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if primary(ui, &ds, tl!("That's it"), ready).clicked() {
                            intent = Some(Intent::ConfirmAnswer(name.clone()));
                        }
                        if enabled && pane_link(ui, &ds, tl!("Say it otherwise"), TextStyle::Button, 13.0).clicked() {
                            intent = Some(Intent::Rephrase(name.clone()));
                        }
                    });
                }
                None => {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for (code, label) in &ask.choices {
                            if chip(ui, &ds, label, false, enabled).clicked() {
                                intent = Some(Intent::Answer(name.clone(), code.clone()));
                            }
                        }
                    });
                    let draft = asked.drafts.entry(name.clone()).or_default();
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 96.0).max(80.0);
                        let field = TextEdit::singleline(draft).hint_text(tl!("Answer Nora…")).desired_width(width).char_limit(1000);
                        ui.add_enabled(enabled, field.font(face(ui, TextStyle::Body, 13.0)));
                        let typed = !draft.trim().is_empty();
                        if secondary(ui, &ds, tl!("Send my answer"), enabled && typed).clicked() {
                            intent = Some(Intent::SayAnswer(name.clone()));
                        }
                    });
                    if enabled && pane_link(ui, &ds, tl!("Later"), TextStyle::Button, 13.0).clicked() {
                        intent = Some(Intent::Later(name.clone()));
                    }
                }
            }
        });
    }
    if let Some(noted) = &asked.noted {
        hers(ui, &ds, |ui| {
            if !noted.words.is_empty() {
                said(ui, &ds, &noted.words);
            }
            // The correction the answer made (what Nora had filed of the family, moved where the answer says): undone here.
            if !noted.run.is_empty() && secondary(ui, &ds, tl!("Undo"), enabled).clicked() {
                intent = Some(Intent::UndoNoted);
            }
        });
    }
    if let Some(error) = &asked.error {
        notice(ui, &ds, error, true);
    }
    intent
}

/// The number on her button for `count` questions: 1 to 9, then « 9+ ».
pub(crate) fn badge_words(count: usize) -> String {
    if count > 9 { "9+".to_string() } else { count.to_string() }
}

/// The number of Nora's questions to the person on her buttons of the ribbon (« Nora » of Mail's Home tab, « Ask Nora » of a
/// message being written), as they were drawn this frame: an ink disc with its number in paper (paper and ink in the dark),
/// 1 to 9, then « 9+ ». Nothing without a question.
pub(crate) fn ribbon_badge(app: &mut MailApp, ui: &Ui) {
    let count = app.nora.sorting.questions.list.len();
    if count == 0 || !sorting::offered(app) {
        return;
    }
    let ds = Ds::of(Tokens::get(ui.ctx()).dark);
    let words = badge_words(count);
    let label = tln!(count, "{n} question from Nora", "{n} questions from Nora");
    for key in ["nora", "ui.noraAssistant"] {
        let Some(button) = app.hit.rect("ribbon", crate::ribbon::hit_id(key)) else { continue };
        let galley = ui.painter().layout_no_wrap(words.clone(), semibold(10.5), ds.primary_fg);
        let width = (galley.size().x + 8.0).max(16.0);
        let disc = egui::Rect::from_center_size(pos2(button.max.x - width / 2.0 - 1.0, button.min.y + 8.0), vec2(width, 16.0));
        let response = ui.interact(disc, ui.scope_id().with(("nora_questions_badge", key)), Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(Role::Label, true, &label));
        let response = response.on_hover_text(&label);
        let painter = ui.painter();
        painter.rect(disc, CornerRadius::same(255), ds.primary, Stroke::new(1.5, ds.paper), egui::StrokeKind::Outside);
        painter.galley(pos2(disc.center().x - galley.size().x / 2.0, disc.center().y - galley.size().y / 2.0), galley, ds.primary_fg);
        app.hit.add("noraQuestions", u64::from(key == "nora"), response.rect);
    }
}

/// « Journal »: her runs, each with « Annuler » while it can be undone, and what she learnt from the mail people moved.
fn journal(ui: &mut Ui, ds: &Ds, flow: &Flow, cx: &FlowCx, enabled: bool) -> Option<Intent> {
    let journal = flow.journal.as_ref()?;
    let mut intent = None;
    ui.add_space(4.0);
    ui.add(Label::new(RichText::new(tl!("Journal").to_uppercase()).font(face(ui, TextStyle::Small, 11.0)).color(ds.weak)).selectable(false));
    Frame::new().fill(ds.card).stroke(Stroke::new(1.0, ds.border)).corner_radius(CornerRadius::same(14)).inner_margin(Margin::symmetric(14, 4)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            if journal.runs.is_empty() && journal.learned.is_empty() {
                ui.add_space(6.0);
                note(ui, ds, tl!("Nothing filed yet."));
                ui.add_space(6.0);
            }
            for run in journal.runs.iter().take(8) {
                let filed = tln!(run.filed, "{n} message filed", "{n} messages filed");
                let mut words = if run.kind == "correction" {
                    let what =
                        if run.about.is_empty() { tl!("Correction").to_string() } else { fmt(tl!("Correction: {about}"), &[("about", &run.about)]) };
                    format!("{} · {what} · {filed}", when_short(&run.started_at, cx.today))
                } else {
                    format!("{} · {filed}", when_short(&run.started_at, cx.today))
                };
                match run.status.as_str() {
                    "undone" => words = format!("{words} · {}", tl!("undone")),
                    "partly_undone" => words = format!("{words} · {}", tl!("partly undone")),
                    "running" => words = format!("{words} · {}", tl!("in progress")),
                    _ => {}
                }
                ui.horizontal(|ui| {
                    ui.set_min_height(30.0);
                    let width = ui.available_width() - if run.can_undo && enabled { 70.0 } else { 0.0 };
                    let label = Label::new(RichText::new(tied(&words)).font(face(ui, TextStyle::Body, 13.0)).color(ds.text)).wrap().selectable(false);
                    ui.scope(|ui| {
                        ui.set_max_width(width.max(80.0));
                        ui.add(label);
                    });
                    if run.can_undo && enabled {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if pane_link(ui, ds, tl!("Undo"), TextStyle::Button, 13.0).clicked() {
                                intent = Some(Intent::Undo(run.run.clone()));
                            }
                        });
                    }
                });
            }
            for learned in &journal.learned {
                ui.horizontal(|ui| {
                    ui.set_min_height(30.0);
                    let words = fmt(tl!("You moved “{folder}”: I remember this folder."), &[("folder", &learned.leaf)]);
                    ui.add(Label::new(RichText::new(tied(&words)).font(face(ui, TextStyle::Body, 13.0)).color(ds.text_2)).wrap().selectable(false));
                });
            }
        },
    );
    intent
}

// ---- the activation card ----

/// The activation card (planche 5), a modal window over the whole window, while it is up: « Ranger dans des dossiers » in the
/// ways the server knows, « Préparer le travail », the reading's rhythm; « Activer et voir le plan », « Plus tard ».
pub(crate) fn card(app: &mut MailApp, ctx: &egui::Context) {
    let Some(card) = app.nora.sorting.card.clone() else { return };
    let Some(state) = app.nora.sorting.available(&card.account).cloned() else {
        app.nora.sorting.card = None;
        return;
    };
    let ds = Ds::of(Tokens::get(ctx).dark);
    let sans = |size: f32| pane_font(ctx, SANS, size);
    let medium = |size: f32| pane_font(ctx, MEDIUM, size);
    let title = |size: f32| pane_font(ctx, TITLE, size);
    let mut intent = None;
    let mut edited = card.clone();
    let modal = egui::Modal::new(egui::Id::unique("nora_sorting_card"))
        .backdrop_color(Color32::from_rgba_unmultiplied(24, 20, 16, 140))
        .frame(Frame::new().fill(ds.paper).corner_radius(CornerRadius::same(18)).inner_margin(Margin { left: 34, right: 34, top: 30, bottom: 24 }))
        .show(ctx, |ui| {
            ui.set_width(640.0_f32.min(ctx.content_rect().width() - 120.0).max(280.0));
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let (mark, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                craft_nora_ui::orb::paint(ui.painter(), mark, ds.orb, None);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let over = format!("NeoMail · {}", state.address);
                    ui.add(Label::new(RichText::new(over.to_uppercase()).font(sans(11.0)).color(ds.weak)).selectable(false));
                    ui.add(Label::new(RichText::new(tl!("Nora files your mail")).font(title(30.0)).color(ds.text)).selectable(false));
                });
            });
            let about = tl!(
                "Nora reads the mailbox with your rights, files it by a plan you validate once, and asks you when she hesitates. She deletes nothing and sends nothing."
            );
            ui.add(Label::new(RichText::new(about).font(sans(15.0)).color(ds.text_2)).wrap());
            option(ui, &ds, |ui| {
                ui.add(Label::new(RichText::new(tl!("File into folders")).font(medium(15.0)).color(ds.text)).selectable(false));
                let detail = tl!("Nora proposes the folders from what she read: you see the plan before anything moves.");
                ui.add(Label::new(RichText::new(detail).font(sans(13.0)).color(ds.weak)).wrap());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for mode in &state.modes {
                        if seg(ui, &ds, &mode.label, edited.mode == mode.code, sans(13.0)).clicked() {
                            edited.mode = mode.code.clone();
                        }
                    }
                });
            });
            option(ui, &ds, |ui| {
                ui.horizontal(|ui| {
                    let label = tl!("Prepare the work");
                    if switch(ui, &ds, edited.prepare, !edited.busy, label).clicked() {
                        edited.prepare = !edited.prepare;
                    }
                    ui.vertical(|ui| {
                        ui.add(Label::new(RichText::new(label).font(medium(15.0)).color(ds.text)).selectable(false));
                        let detail = tl!(
                            "A draft reply in Drafts, a received invoice read and ready to validate, a task when you are asked something. Nothing is validated or sent without you."
                        );
                        ui.add(Label::new(RichText::new(detail).font(sans(13.0)).color(ds.weak)).wrap());
                    });
                });
            });
            option(ui, &ds, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    ui.add(Label::new(RichText::new(tl!("Read the mailbox")).font(medium(15.0)).color(ds.text)).selectable(false));
                    for minutes in &state.intervals {
                        if seg(ui, &ds, &every(*minutes), edited.interval == *minutes, sans(13.0)).clicked() {
                            edited.interval = *minutes;
                        }
                    }
                });
            });
            if let Some(error) = &edited.error {
                notice(ui, &ds, error, true);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                if link(ui, &ds, tl!("Later"), medium(14.0)).clicked() {
                    intent = Some(Intent::CloseCard);
                }
                ui.add(Label::new(RichText::new(tl!("Each filing can be undone from Nora's journal.")).font(sans(12.0)).color(ds.weak)).wrap());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let text = RichText::new(tl!("Turn on and see the plan")).font(medium(14.0)).color(ds.primary_fg);
                    let button = Button::new(text).fill(ds.primary).corner_radius(CornerRadius::same(10)).min_size(vec2(0.0, 40.0));
                    if ui.add_enabled(!edited.busy && !edited.mode.is_empty(), button).clicked() {
                        intent = Some(Intent::Activate);
                    }
                });
            });
        });
    if modal.should_close() && intent.is_none() && !edited.busy {
        intent = Some(Intent::CloseCard);
    }
    if edited != card
        && let Some(kept) = app.nora.sorting.card.as_mut()
    {
        *kept = edited;
    }
    if let Some(intent) = intent {
        sorting::act(app, intent);
    }
}

/// An option of the card: a white card of its own.
fn option(ui: &mut Ui, ds: &Ds, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(ds.card)
        .stroke(Stroke::new(1.0, ds.border))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            add(ui);
        });
}

/// One of a set of choices of the card: its border ink when chosen.
fn seg(ui: &mut Ui, ds: &Ds, label: &str, chosen: bool, font: FontId) -> Response {
    let text = RichText::new(label).font(font).color(if chosen { ds.text } else { ds.text_2 });
    let stroke = Stroke::new(1.0, if chosen { ds.text } else { ds.border_strong });
    let response = ui
        .add(Button::new(text).fill(ds.card).stroke(stroke).corner_radius(CornerRadius::same(255)).min_size(vec2(0.0, 30.0)))
        .on_hover_cursor(CursorIcon::PointingHand);
    response.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, chosen, label));
    response
}

/// The mode of `code` among the server's, for a test.
#[cfg(test)]
pub(crate) fn mode(state: &BoxState, code: &str) -> Option<sorting::Mode> {
    state.modes.iter().find(|m| m.code == code).cloned()
}

#[cfg(test)]
#[path = "sorting_view_tests.rs"]
mod tests;
