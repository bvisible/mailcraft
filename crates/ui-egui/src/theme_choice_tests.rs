// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The owner's choice of accent and high contrast (#703): the commands, `ui.json`, and what the
//! window shows.

use mailcraft_engine::Session;
use serde_json::{Value, json};

use crate::theme::{Accent, Tokens};
use crate::{MailApp, Services, UiState};

const NOW: i64 = 1_791_993_600;

fn app() -> MailApp {
    let mut a = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    a.autosave = false;
    a
}

#[test]
fn the_commands_set_the_choice_and_refuse_anything_else() {
    let mut a = app();
    assert_eq!(a.execute("ui.accent", json!({"accent": "teal"})).unwrap()["accent"], "teal");
    assert_eq!(a.ui.accent, Accent::Teal);
    assert_eq!(a.execute("ui.highContrast", json!({"on": true})).unwrap()["on"], true);
    assert!(a.ui.high_contrast);
    for junk in [
        json!({}),
        json!(null),
        json!([]),
        json!("x"),
        json!({"accent": 7}),
        json!({"accent": "pink"}),
        json!({"accent": ["teal"]}),
        json!({"accent": null}),
    ] {
        assert!(a.execute("ui.accent", junk).is_err());
    }
    for junk in [json!({}), json!(null), json!([]), json!({"on": "yes"}), json!({"on": 1}), json!({"on": null}), json!({"on": [true]})] {
        assert!(a.execute("ui.highContrast", junk).is_err());
    }
    assert_eq!(a.ui.accent, Accent::Teal);
    assert!(a.ui.high_contrast);
}

#[test]
fn the_choice_is_saved_and_a_missing_or_unreadable_one_falls_back() {
    let ui = UiState { accent: Accent::Violet, high_contrast: true, ..UiState::default() };
    let saved: Value = serde_json::to_value(&ui).unwrap();
    assert_eq!(saved["accent"], "violet");
    assert_eq!(saved["highContrast"], true);
    let back: UiState = serde_json::from_value(saved).unwrap();
    assert_eq!((back.accent, back.high_contrast), (Accent::Violet, true));
    // Keys from before this existed.
    let old: UiState = serde_json::from_value(json!({"theme": "dark"})).unwrap();
    assert_eq!((old.accent, old.high_contrast, old.theme), (Accent::Blue, false, crate::theme::Appearance::Dark));
    // Values that are not what they should be read as the defaults, and the rest is kept.
    for junk in [json!("mauve"), json!(5), json!(null), json!([1]), json!({"a": 1})] {
        let bad: UiState = serde_json::from_value(json!({"theme": "dark", "accent": junk.clone(), "highContrast": junk})).unwrap();
        assert_eq!((bad.accent, bad.high_contrast, bad.theme), (Accent::Blue, false, crate::theme::Appearance::Dark));
    }
}

#[test]
fn the_window_shows_the_accent_and_high_contrast_at_once() {
    let mut a = app();
    a.execute("ui.theme", json!({"value": "light"})).unwrap();
    a.execute("ui.accent", json!({"accent": "graphite"})).unwrap();
    a.execute("ui.highContrast", json!({"on": true})).unwrap();
    let ctx = egui::Context::default();
    a.logic(&ctx);
    let t = Tokens::get(&ctx);
    assert_eq!(t.accent, Accent::Graphite.ink(false, true).accent);
    assert!(t.high_contrast);
    a.execute("ui.highContrast", json!({"on": false})).unwrap();
    a.execute("ui.accent", json!({"accent": "blue"})).unwrap();
    a.logic(&ctx);
    assert_eq!(Tokens::get(&ctx).accent, Tokens::light().accent);
}

// //// Neoffice — added: a hosting page's tab takes the Neoffice design system's look (theme/neoffice.rs), in light and
// //// Neoffice — dark, whatever accent was picked; the program that keeps its mail keeps upstream's.
#[test]
fn a_hosting_pages_tab_takes_the_neoffice_look_in_light_and_dark() {
    let mut a = app();
    a.session.keeps_local = false;
    a.execute("ui.accent", json!({"accent": "violet"})).unwrap();
    let ctx = egui::Context::default();
    for (value, dark) in [("light", false), ("dark", true)] {
        a.execute("ui.theme", json!({"value": value})).unwrap();
        a.logic(&ctx);
        let (t, neo) = (Tokens::get(&ctx), Tokens::neoffice(dark));
        assert_eq!(t.dark, dark);
        assert_eq!((t.canvas, t.panel, t.surface, t.ribbon), (neo.canvas, neo.panel, neo.surface, neo.ribbon), "{value}");
        assert_eq!((t.accent, t.selection, t.accent_soft, t.selection_edge), (neo.accent, neo.selection, neo.accent_soft, neo.selection_edge));
        // A message on a light page in the dark theme: the paper's white, not upstream's.
        assert_eq!(Tokens::light_of(&ctx).accent_text, Tokens::neoffice(false).accent_text);
    }
    // Every action ink (dark: the bright paper), the signal only as a field and a gesture; no blue.
    let (light, dark) = (Tokens::neoffice(false), Tokens::neoffice(true));
    assert_eq!((light.accent, dark.accent), (egui::Color32::from_rgb(0x14, 0x14, 0x14), egui::Color32::from_rgb(0xFF, 0xFD, 0xF8)));
    assert_eq!((light.selection_edge, light.canvas), (egui::Color32::from_rgb(0xF5, 0x5D, 0x1A), egui::Color32::from_rgb(0xFA, 0xF3, 0xEA)));
    for t in [light, dark] {
        let avatars = ["ada@example.org", "grace@example.org", "alan@example.org", "edsger@example.org", "barbara@example.org", "ken@example.org"]
            .map(|who| crate::theme::avatar_color(who, &t));
        for c in [t.accent, t.accent_text, t.selection, t.accent_soft, t.checked, t.today, t.canvas, t.surface].into_iter().chain(avatars) {
            assert!(!(c.b() > c.r().saturating_add(24) && c.b() > c.g()), "{c:?} is blue");
        }
    }
    // High contrast: upstream's, in graphite (the design system has no ramp at 7:1).
    a.execute("ui.highContrast", json!({"on": true})).unwrap();
    a.logic(&ctx);
    assert_eq!(Tokens::get(&ctx).accent, Accent::Graphite.ink(true, true).accent);
    // The program that keeps its mail on the computer: upstream's look, in the accent picked.
    a.session.keeps_local = true;
    a.execute("ui.highContrast", json!({"on": false})).unwrap();
    a.logic(&ctx);
    assert_eq!(Tokens::get(&ctx).accent, Accent::Violet.ink(true, false).accent);
}
