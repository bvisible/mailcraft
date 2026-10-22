// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): NeoMail's icons as SVG, for a page that draws beside it.
//! The window's icons (`icons::paint`) as vector drawings, for a page that draws in the window's place and should look
//! like it (Neodesk's Synk, its chat, in NeoMail's window): each icon is painted on its 20-unit grid, ten pixels to the
//! unit, and the shapes it gives are written back on that grid as SVG elements, in JSON on the standard output.
//!
//! ```text
//! cargo run -p mailcraft-ui-egui --example export_icons -- search lock attach send > icons.json
//! ```
//!
//! `{"<name>": [{"tag": "path" | "circle" | "rect", …attributes}]}`; a colour is `ink` (the icon's own colour), `accent`
//! (its one accented part), or a colour of the light palette written out (a state mark). A name the window has no drawing
//! for is refused.

use std::process::ExitCode;

use egui::epaint::{ColorMode, Shape};
use egui::{Color32, Context, LayerId, Pos2, RawInput, Rect, Visuals, pos2, vec2};
use mailcraft_ui_egui::icons;
use serde_json::{Map, Value, json};

/// The grid of an icon, and how many pixels a unit is drawn with here: fine enough that the pen's rounding to whole
/// pixels is a tenth of a unit.
const GRID: f32 = 20.0;
const SCALE: f32 = 10.0;
/// The colours the icon is painted in, told apart afterwards: its ink and its accent.
const INK: Color32 = Color32::from_rgb(250, 10, 100);
const ACCENT: Color32 = Color32::from_rgb(10, 250, 100);

fn main() -> ExitCode {
    let names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        eprintln!("usage: export_icons <name>…");
        return ExitCode::from(2);
    }
    let mut out = Map::new();
    for name in &names {
        if !icons::NAMES.contains(&name.as_str()) {
            eprintln!("export_icons: no drawing for {name:?}");
            return ExitCode::from(2);
        }
        out.insert(name.clone(), Value::Array(drawing(name)));
    }
    match serde_json::to_string_pretty(&Value::Object(out)) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("export_icons: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The shapes `name` is drawn with, as SVG elements on its 20-unit grid.
fn drawing(name: &str) -> Vec<Value> {
    let ctx = Context::default();
    ctx.set_visuals(Visuals::light());
    let side = GRID * SCALE;
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(side * 2.0, side * 2.0))), ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| {
        let painter = ui.ctx().layer_painter(LayerId::background());
        icons::paint(&painter, Rect::from_min_size(Pos2::ZERO, vec2(side, side)), name, INK, ACCENT);
    });
    // The font atlas the frame made is nobody's to upload here
    output.textures_delta.clear();
    let mut elements = Vec::new();
    for clipped in std::mem::take(&mut output.shapes) {
        element(&clipped.shape, &mut elements);
    }
    elements
}

fn element(shape: &Shape, out: &mut Vec<Value>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| element(s, out)),
        Shape::Path(path) => {
            let mut d = path
                .points
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}{} {}", if i == 0 { "M" } else { "L" }, unit(p.x), unit(p.y)))
                .collect::<Vec<_>>()
                .join(" ");
            if path.closed {
                d.push_str(" Z");
            }
            let stroke = match &path.stroke.color {
                ColorMode::Solid(c) if path.stroke.width > 0.0 => paint(*c),
                _ => None,
            };
            out.push(with_paint(json!({"tag": "path", "d": d}), paint(path.fill), stroke, path.stroke.width));
        }
        Shape::LineSegment { points, stroke } => {
            let d = format!("M{} {} L{} {}", unit(points[0].x), unit(points[0].y), unit(points[1].x), unit(points[1].y));
            out.push(with_paint(json!({"tag": "path", "d": d}), None, paint(stroke.color), stroke.width));
        }
        Shape::Circle(circle) => {
            let base = json!({"tag": "circle", "cx": unit(circle.center.x), "cy": unit(circle.center.y), "r": unit(circle.radius)});
            out.push(with_paint(base, paint(circle.fill), paint(circle.stroke.color).filter(|_| circle.stroke.width > 0.0), circle.stroke.width));
        }
        Shape::Rect(rect) => {
            let r = &rect.rect;
            let base = json!({
                "tag": "rect",
                "x": unit(r.min.x),
                "y": unit(r.min.y),
                "width": unit(r.width()),
                "height": unit(r.height()),
                "rx": unit(f32::from(rect.corner_radius.nw)),
            });
            out.push(with_paint(base, paint(rect.fill), paint(rect.stroke.color).filter(|_| rect.stroke.width > 0.0), rect.stroke.width));
        }
        Shape::Mesh(mesh) => {
            // Filled shapes the pen fans out from a centre: each triangle, in its first corner's colour.
            for triangle in mesh.indices.as_chunks::<3>().0 {
                let corners: Vec<Pos2> = triangle.iter().filter_map(|&i| mesh.vertices.get(i as usize)).map(|v| v.pos).collect();
                let (Some(first), true) = (triangle.first().and_then(|&i| mesh.vertices.get(i as usize)), corners.len() == 3) else { continue };
                let d = format!(
                    "M{} {} L{} {} L{} {} Z",
                    unit(corners[0].x),
                    unit(corners[0].y),
                    unit(corners[1].x),
                    unit(corners[1].y),
                    unit(corners[2].x),
                    unit(corners[2].y)
                );
                out.push(with_paint(json!({"tag": "path", "d": d}), paint(first.color), None, 0.0));
            }
        }
        Shape::Noop => {}
        other => eprintln!("export_icons: a shape left out: {:?}", std::mem::discriminant(other)),
    }
}

/// A length in pixels as units of the grid, to a hundredth.
fn unit(px: f32) -> f64 {
    (f64::from(px / SCALE) * 100.0).round() / 100.0
}

/// A colour as the drawing names it: `ink`, `accent` (each with its opacity when see-through), a written colour, or
/// none for a transparent one.
fn paint(c: Color32) -> Option<Value> {
    if c.a() == 0 {
        return None;
    }
    for (role, base) in [("ink", INK), ("accent", ACCENT)] {
        let alpha = f32::from(c.a()) / 255.0;
        let near = |got: u8, want: u8| (f32::from(got) - f32::from(want) * alpha).abs() <= 2.0;
        if near(c.r(), base.r()) && near(c.g(), base.g()) && near(c.b(), base.b()) {
            return Some(if c.a() == 255 { json!(role) } else { json!({"role": role, "opacity": (f64::from(alpha) * 100.0).round() / 100.0}) });
        }
    }
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Some(json!(if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }))
}

fn with_paint(mut base: Value, fill: Option<Value>, stroke: Option<Value>, width: f32) -> Value {
    if let Some(object) = base.as_object_mut() {
        object.insert("fill".into(), fill.unwrap_or(json!("none")));
        if let Some(stroke) = stroke {
            object.insert("stroke".into(), stroke);
            object.insert("strokeWidth".into(), json!(unit(width)));
        }
    }
    base
}

#[allow(dead_code)]
fn origin() -> Pos2 {
    pos2(0.0, 0.0)
}
