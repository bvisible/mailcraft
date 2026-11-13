// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The mark at the top left: Neoffice's icon in a hosting page's tab, MailCraft's tile on the desktop (host_mark.rs).

use egui_kittest::Harness;
use mailcraft_engine::Session;
use mailcraft_model::Mailbox;

use super::*;
use crate::Services;

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

#[test]
fn the_icon_is_the_design_system_s_picture_with_its_rounded_corners_clear() {
    let rgba = image::load_from_memory_with_format(PNG, image::ImageFormat::Png).unwrap().into_rgba8();
    assert_eq!((rgba.width(), rgba.height()), (64, 64));
    // The family's rounded square: the corners are clear, the paper (#FAF3EA) all along the middle of the edge.
    for (x, y) in [(0, 0), (63, 0), (0, 63), (63, 63)] {
        assert_eq!(rgba.get_pixel(x, y)[3], 0, "corner {x},{y}");
    }
    assert_eq!(rgba.get_pixel(32, 1).0, [0xFA, 0xF3, 0xEA, 0xFF]);
}

/// Whether the window drew Neoffice's icon: its texture was made.
fn drew_neoffice_s(keeps_local: bool) -> bool {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    app.session.keeps_local = keeps_local;
    let mut harness = Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(1280.0, 860.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(3);
    harness.ctx.tex_manager().read().allocated().any(|(_, meta)| meta.name == TEXTURE)
}

#[test]
fn a_hosting_page_s_tab_shows_neoffice_s_icon_and_the_desktop_mailcraft_s_tile() {
    assert!(drew_neoffice_s(false), "a hosting page's tab");
    assert!(!drew_neoffice_s(true), "the program that keeps its mail on the computer");
}
