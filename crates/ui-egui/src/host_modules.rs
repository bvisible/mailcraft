// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): modules a hosting page draws itself, beside the window's own.

//! Modules a hosting page draws itself (Neodesk's NeoMail window, which hands the browser build `modules` and draws
//! Synk, its own chat, in the window's place: apps/mailcraft-web/src/host.rs). Each has a button in the module bar,
//! after Calendar ([`bar`]), with the badge the page gives it ([`BADGE_TYPE`]). While one is in view ([`shown`]) the
//! window keeps its title bar, its module bar and its status bar and leaves the rest empty ([`place`]); it tells the
//! page where that is, in CSS pixels of its own page, at each change ([`MODULE_TYPE`]), and that there is nothing to
//! draw while something of the window's own stands over it (a dialog, a menu, Backstage): the page's view, drawn above
//! the window, would hide it. A module of the window's own brings it back ([`leave`]).

use std::collections::HashMap;

use egui::{Align2, Order, Rect, Sense, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::theme::{Tokens, bold};
use crate::{MailApp, icons};

/// What the window tells the page of the module it draws (`type`): its id and where to draw it, or none.
pub const MODULE_TYPE: &str = "mailcraft:module";
/// What the page tells the window (`type`): a module's badge, and a module to bring into view.
pub const BADGE_TYPE: &str = "mailcraft:module-badge";
pub const SHOW_TYPE: &str = "mailcraft:module-show";
/// The most modules a page may hand over, the longest id and title it may give one, and the longest badge drawn.
pub const MAX_MODULES: usize = 4;
pub const MAX_ID: usize = 32;
pub const MAX_TITLE: usize = 40;
const MAX_BADGE: usize = 3;
/// The icon a module is drawn with when the page names none the window has.
pub const DEFAULT_ICON: &str = "chatSynk";

/// A module a hosting page draws itself: its id (what the page and the window call it), its name (the button's
/// tooltip) and the name of one of the window's icons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostModule {
    pub id: String,
    pub title: String,
    pub icon: String,
}

/// The modules a hosting page handed over, the one in view, their badges, and what the page was last told.
pub struct Modules {
    pub list: Vec<HostModule>,
    pub shown: Option<String>,
    badges: HashMap<String, String>,
    told: Option<Value>,
    /// Where the module in view was left its place this frame (`None`: it was not, or something stood over it).
    placed: Option<Rect>,
    /// Tell the page (apps/mailcraft-web/src/host_menu.rs, `tell`).
    pub post: Box<dyn Fn(&Value)>,
}

impl Modules {
    /// The modules `handed` by a page (`{id, title, icon?}`), each once, those that cannot be refused: an id of lower
    /// case letters, digits and hyphens, a title, at most [`MAX_MODULES`] of them; an icon the window does not have
    /// draws as [`DEFAULT_ICON`].
    pub fn new(handed: &[Value], post: Box<dyn Fn(&Value)>) -> Modules {
        let mut list: Vec<HostModule> = Vec::new();
        for entry in handed {
            let Some(id) = entry.get("id").and_then(Value::as_str).filter(|id| plausible(id)) else { continue };
            let Some(title) = entry.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) else {
                continue;
            };
            if list.len() >= MAX_MODULES || list.iter().any(|m| m.id == id) {
                continue;
            }
            let icon = entry.get("icon").and_then(Value::as_str).filter(|name| icons::NAMES.contains(name)).unwrap_or(DEFAULT_ICON);
            list.push(HostModule { id: id.to_string(), title: title.chars().take(MAX_TITLE).collect(), icon: icon.to_string() });
        }
        Modules { list, shown: None, badges: HashMap::new(), told: None, placed: None, post }
    }

    /// The badge drawn on `id`'s button: what the page gave, as it gave it, [`MAX_BADGE`] characters at most.
    pub fn badge(&self, id: &str) -> Option<&str> {
        self.badges.get(id).map(String::as_str)
    }
}

/// An id a page may give a module.
fn plausible(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The modules a hosting page draws, in the module bar's order: none where no page handed any.
pub fn listed(app: &MailApp) -> Vec<HostModule> {
    app.services.modules.as_ref().map(|m| m.list.clone()).unwrap_or_default()
}

/// Whether a module of the page's is in view.
pub fn shown(app: &MailApp) -> bool {
    app.services.modules.as_ref().is_some_and(|m| m.shown.is_some())
}

/// Bring the page's module `id` into view: the window leaves it its place from the next frame.
pub fn show(app: &mut MailApp, id: &str) -> bool {
    let Some(modules) = app.services.modules.as_mut() else { return false };
    if !modules.list.iter().any(|m| m.id == id) {
        return false;
    }
    modules.shown = Some(id.to_string());
    true
}

/// The window's own module back: the page is told it has nothing to draw.
pub fn leave(app: &mut MailApp) {
    let Some(modules) = app.services.modules.as_mut() else { return };
    if modules.shown.take().is_some() {
        modules.placed = None;
        tell(modules, json!({"type": MODULE_TYPE, "id": null, "rect": null}));
    }
}

/// Tell the page `message`, unless it was told that last.
fn tell(modules: &mut Modules, message: Value) {
    if modules.told.as_ref() != Some(&message) {
        (modules.post)(&message);
        modules.told = Some(message);
    }
}

/// The page's buttons in the module bar, after Calendar's: `pitch` high each, `width` wide, the one in view lit as
/// the window's own are, its badge at its corner.
pub fn bar(app: &mut MailApp, ui: &mut Ui, width: f32, pitch: f32) {
    let t = Tokens::get(ui.ctx());
    for (index, module) in listed(app).into_iter().enumerate() {
        let (r, resp) = ui.allocate_exact_size(vec2(width, pitch), Sense::click());
        app.hit.add("hostModule", index as u64, r);
        let active = app.services.modules.as_ref().and_then(|m| m.shown.as_deref()) == Some(module.id.as_str());
        if active {
            let lit = Rect::from_min_size(pos2(r.min.x + 6.0, r.center().y - 16.0), vec2(2.0, 32.0));
            ui.painter().rect_filled(lit, 1.0, t.accent);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(6.0), 4.0, if resp.is_pointer_button_down_on() { t.pressed } else { t.hover });
        }
        let ink = if active { t.accent_text } else { t.icon };
        icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(24.0, 24.0)), &module.icon, ink, ink);
        let badge = app.services.modules.as_ref().and_then(|m| m.badge(&module.id)).map(str::to_string);
        if let Some(badge) = badge {
            let at = pos2(r.center().x + 9.0, r.center().y - 9.0);
            let wide = 14.0_f32.max(8.0 + 6.0 * badge.chars().count() as f32);
            ui.painter().rect_filled(Rect::from_center_size(at, vec2(wide, 14.0)), 7.0, t.accent);
            ui.painter().text(at, Align2::CENTER_CENTER, &badge, bold(9.0), t.on_accent);
        }
        if resp.on_hover_text(&module.title).clicked() {
            show(app, &module.id);
        }
    }
}

/// The place the module in view is drawn in by the page: the window's central part, left empty. Noted for
/// [`settle`], which tells the page.
pub fn place(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| {
        let rect = ui.max_rect();
        if let Some(modules) = app.services.modules.as_mut() {
            modules.placed = Some(rect);
        }
    });
}

/// Once the window is laid out: tell the page where to draw the module in view, in CSS pixels of the window's page,
/// or that there is nothing to draw while something of the window's own stands over that place (a window, a menu,
/// Backstage, where the central part was not laid out).
pub fn settle(app: &mut MailApp, ctx: &egui::Context) {
    let over = ctx.memory(|m| m.areas().visible_layer_ids().iter().any(|layer| matches!(layer.order, Order::Middle | Order::Foreground)));
    let zoom = ctx.zoom_factor();
    let Some(modules) = app.services.modules.as_mut() else { return };
    let Some(id) = modules.shown.clone() else { return };
    let placed = modules.placed.take().filter(|_| !over);
    let rect = placed.map(|r| {
        json!({
            "x": (r.min.x * zoom).round(),
            "y": (r.min.y * zoom).round(),
            "width": (r.width() * zoom).round(),
            "height": (r.height() * zoom).round(),
        })
    });
    tell(modules, json!({"type": MODULE_TYPE, "id": id, "rect": rect}));
}

/// A message of the page's for the modules (its JSON, `text`): a badge (`{type: BADGE_TYPE, id, badge}`, a number or
/// a short text, none for null or 0) or a module to bring into view (`{type: SHOW_TYPE, id}`). Whether it was one.
pub fn hear(app: &mut MailApp, text: &str) -> bool {
    let Ok(message) = serde_json::from_str::<Value>(text) else { return false };
    let kind = message.get("type").and_then(Value::as_str);
    if kind != Some(BADGE_TYPE) && kind != Some(SHOW_TYPE) {
        return false;
    }
    let Some(id) = message.get("id").and_then(Value::as_str).map(str::to_string) else { return true };
    if kind == Some(SHOW_TYPE) {
        show(app, &id);
        return true;
    }
    let Some(modules) = app.services.modules.as_mut() else { return true };
    if !modules.list.iter().any(|m| m.id == id) {
        return true;
    }
    let badge = match message.get("badge") {
        Some(Value::Number(n)) if n.as_u64().is_some_and(|n| n > 0) => n.as_u64().map(|n| if n > 99 { "99+".to_string() } else { n.to_string() }),
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().chars().take(MAX_BADGE).collect()),
        _ => None,
    };
    match badge {
        Some(badge) => modules.badges.insert(id, badge),
        None => modules.badges.remove(&id),
    };
    true
}

#[cfg(test)]
mod tests;
