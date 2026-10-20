// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): how Nora looks in the window: her clay, her sphere, her pills.

//! How Nora looks, the same in the band of a message being written, the reading pane and the ribbon:
//! her colours are the clay of Neoffice rather than the window's own accent, as the desk shows her (a
//! mark of hers, kept in this file rather than among the theme's tokens); her sphere is the Nora app's
//! orb, rings of latitude on a sphere, the near half of each strong and the far half faint; her
//! buttons are pills with rounded ends, a mark, the words, and a chevron when they open a menu. The
//! marks are drawn here, in the one-stroke, round-ended way of the window's icons.

use egui::{Color32, Painter, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::theme::{Tokens, regular};

/// Nora's colours in this theme.
pub(crate) struct Tone {
    pub(crate) orb: Color32,
    pub(crate) text: Color32,
    pub(crate) fill: Color32,
    pub(crate) hover: Color32,
    pub(crate) border: Color32,
}

pub(crate) fn tone(t: &Tokens) -> Tone {
    if t.dark {
        Tone {
            orb: Color32::from_rgb(0xD6, 0x8A, 0x59),
            text: Color32::from_rgb(0xE7, 0xB4, 0x8F),
            fill: Color32::from_rgb(0x3A, 0x2A, 0x21),
            hover: Color32::from_rgb(0x46, 0x33, 0x2A),
            border: Color32::from_rgb(0x7A, 0x4A, 0x2C),
        }
    } else {
        Tone {
            orb: Color32::from_rgb(0xA1, 0x5A, 0x2E),
            text: Color32::from_rgb(0x8A, 0x4A, 0x22),
            fill: Color32::from_rgb(0xF7, 0xEB, 0xE3),
            hover: Color32::from_rgb(0xF0, 0xDC, 0xCE),
            border: Color32::from_rgb(0xE2, 0xB5, 0x97),
        }
    }
}

/// Nora's sphere in `rect`: seven rings of latitude (the app's thirteen would merge at the size of
/// a button), the near half of each strong and the far half faint.
pub(crate) fn orb(painter: &Painter, rect: Rect, color: Color32) {
    use std::f32::consts::PI;
    const RINGS: usize = 7;
    const STEPS: usize = 18;
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.46;
    if !r.is_finite() || r <= 0.0 {
        return;
    }
    let width = (r * 0.11).max(0.9);
    for k in 0..RINGS {
        let latitude = ((k as f32 + 0.5) / RINGS as f32 - 0.5) * PI * 0.9;
        let (sin, cos) = latitude.sin_cos();
        let (y, rx) = (c.y + r * sin, r * cos);
        let ry = rx * 0.342;
        for (near, alpha) in [(true, 0.95), (false, 0.35)] {
            let points: Vec<egui::Pos2> = (0..=STEPS)
                .map(|i| {
                    let a = PI * i as f32 / STEPS as f32;
                    let a = if near { a } else { -a };
                    pos2(c.x + rx * a.cos(), y + ry * a.sin())
                })
                .collect();
            painter.add(egui::Shape::line(points, Stroke::new(width, color.gamma_multiply(alpha))));
        }
    }
}

/// What a pill of Nora's shows before its words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// Her sphere: Nora herself.
    Orb,
    /// Two ticks: the spelling checked.
    Check,
    /// A wand and its glint: made better.
    Wand,
    /// A globe: another language.
    Globe,
    /// A pencil: the person's own way of writing.
    Pen,
}

/// The height of a pill, in points.
pub(crate) const PILL_H: f32 = 24.0;

/// What a pill can do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// It can be clicked.
    Live,
    /// Not now: drawn faded, and a click does nothing.
    Greyed,
    /// What it asked for is under way: a spinner in place of its mark, and a click does nothing.
    Spinning,
}

/// A pill of Nora's: `label` (already in the interface's language) after its mark, and a chevron when
/// it opens a menu. Drawn in the `Ui`'s own layout (a wrapping row wraps it), labelled for the
/// accessibility tree.
pub(crate) fn pill(ui: &mut Ui, t: &Tokens, label: &str, mark: Mark, menu: bool, state: State) -> Response {
    let mut tone = tone(t);
    if state == State::Greyed {
        for c in [&mut tone.orb, &mut tone.text, &mut tone.border] {
            *c = c.gamma_multiply(0.45);
        }
    }
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(12.0), tone.text);
    let side = PILL_H - 10.0;
    let chevron = if menu { 14.0 } else { 0.0 };
    let w = 10.0 + side + 6.0 + galley.size().x + chevron + 10.0;
    let sense = if state == State::Live { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, PILL_H), sense);
    if ui.is_rect_visible(rect) {
        let lit = state == State::Live && (resp.hovered() || resp.is_pointer_button_down_on());
        let painter = ui.painter();
        painter.rect(rect, PILL_H / 2.0, if lit { tone.hover } else { tone.fill }, Stroke::new(1.0, tone.border), egui::StrokeKind::Inside);
        let m = Rect::from_min_size(pos2(rect.min.x + 10.0, rect.min.y + 5.0), vec2(side, side));
        if state == State::Spinning {
            egui::Spinner::new().size(side).color(tone.text).paint_at(ui, m);
        } else {
            paint_mark(painter, m, mark, &tone);
        }
        let at = pos2(m.max.x + 6.0, rect.center().y - galley.size().y / 2.0);
        painter.galley(at, galley, tone.text);
        if menu {
            let r = Rect::from_center_size(pos2(rect.max.x - 15.0, rect.center().y), vec2(10.0, 10.0));
            crate::icons::paint(painter, r, "expanded", tone.text, tone.text);
        }
    }
    let live = state == State::Live && ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, live, label));
    resp
}

/// Nora's marks as icons of the window (`icons::paint` draws them by these names, the ribbon uses
/// them): `nora`, her sphere, in her clay unless the icon is drawn in one colour (greyed); `noraWand`,
/// the wand of Improve, in the icon's ink.
pub(crate) fn icon(painter: &Painter, r: Rect, name: &str, ink: Color32, accent: Color32) {
    let side = r.width().min(r.height());
    if !side.is_finite() || side <= 0.0 {
        return;
    }
    let square = Rect::from_center_size(r.center(), vec2(side, side));
    match name {
        "nora" => {
            let clay = tone(&Tokens::get(painter.ctx())).orb;
            orb(painter, square, if ink == accent { ink } else { clay });
        }
        "noraWand" => {
            let s = Stroke::new((side / 12.0).clamp(1.2, 2.6), ink);
            let p = |x: f32, y: f32| pos2(square.min.x + x * side, square.min.y + y * side);
            painter.line_segment([p(0.15, 0.85), p(0.62, 0.38)], s);
            let glint = Stroke::new(s.width, accent);
            painter.line_segment([p(0.75, 0.08), p(0.75, 0.36)], glint);
            painter.line_segment([p(0.61, 0.22), p(0.89, 0.22)], glint);
        }
        _ => {}
    }
}

fn paint_mark(painter: &Painter, r: Rect, mark: Mark, tone: &Tone) {
    let s = Stroke::new(1.4, tone.text);
    let p = |x: f32, y: f32| pos2(r.min.x + x * r.width(), r.min.y + y * r.height());
    match mark {
        Mark::Orb => orb(painter, r, tone.orb),
        Mark::Check => {
            painter.add(egui::Shape::line(vec![p(0.05, 0.55), p(0.28, 0.78), p(0.62, 0.3)], s));
            painter.add(egui::Shape::line(vec![p(0.45, 0.72), p(0.52, 0.78), p(0.95, 0.3)], s));
        }
        Mark::Wand => {
            painter.line_segment([p(0.1, 0.9), p(0.62, 0.38)], s);
            painter.line_segment([p(0.78, 0.05), p(0.78, 0.35)], s);
            painter.line_segment([p(0.63, 0.2), p(0.93, 0.2)], s);
        }
        // The window's own globe and pencil, in her clay.
        Mark::Globe => crate::icons::paint(painter, r, "globe", tone.text, tone.text),
        Mark::Pen => crate::icons::paint(painter, r, "drafts", tone.text, tone.text),
    }
}
