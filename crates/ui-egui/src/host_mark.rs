// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the mark at the top left of a hosting page's tab, Neoffice's icon.

//! The mark at the top left of the title bar. In a tab of a hosting page (nothing kept here:
//! [`mailcraft_engine::Session::keeps_local`] false) it is Neoffice's icon, the picture of the Neoffice design system as it
//! is, never redrawn: `assets/neoffice/neoffice-icon-64.png` is `assets/logo/favicon.png` of `bvisible/neoffice-design-system`
//! byte for byte, the 64 px render of its `neoffice-icon.svg` (the o of the wordmark in its two weights, on the paper, in the
//! family's rounded square). The window has no SVG reader and gets none for one picture: it reads PNG already. The program that
//! keeps its mail on the computer is MailCraft's and keeps MailCraft's tile ([`crate::icons::app_mark`]).

use egui::{Rect, Ui};

use crate::MailApp;

/// Neoffice's icon, 64 px with its rounded corners clear (enough for the 22-point mark at three pixels a point).
const PNG: &[u8] = include_bytes!("../../../assets/neoffice/neoffice-icon-64.png");

/// The name of the icon's texture.
const TEXTURE: &str = "neoffice-mark";

/// Neoffice's icon as a texture, made once and kept in the context. `None` when the picture can't be read (no mark is drawn).
fn texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::unique(TEXTURE);
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let rgba = image::load_from_memory_with_format(PNG, image::ImageFormat::Png).ok()?.into_rgba8();
    let size = [usize::try_from(rgba.width()).ok()?, usize::try_from(rgba.height()).ok()?];
    let texture = ctx.load_texture(TEXTURE, egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

/// The window's mark in `rect` (a square): Neoffice's icon in a hosting page's tab, MailCraft's tile elsewhere.
pub(crate) fn paint(app: &MailApp, ui: &Ui, rect: Rect) {
    if app.session.keeps_local {
        crate::icons::app_mark(ui.painter(), rect);
    } else if let Some(texture) = texture(ui.ctx()) {
        egui::Image::new(&texture).fit_to_exact_size(rect.size()).paint_at(ui, rect);
    }
}

#[cfg(test)]
#[path = "host_mark_tests.rs"]
mod tests;
