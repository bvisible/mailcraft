// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the Neoffice design system's look of the whole window, in a hosting
// page's tab.

//! NeoMail in a hosting page's tab (`Session::keeps_local` false: Neoffice's `/neomail`, Neodesk) looks like the rest of
//! Neoffice, in light and in dark: the tokens of the Neoffice design system (`neoffice-design-system`,
//! `tokens/colors.css`, `:root` and `.theme-dark`), which Nora's pane already takes (craft-nora-ui `look.rs`). The window
//! on the warm paper (dark: the warm espresso), its sheets white (dark: `--surface-card`), its neutrals the sand ramp,
//! every action ink (dark: the bright paper), and the signal (`--signal-*`) as a field and a gesture only: the wash of
//! what is chosen, a count, today, the edge of a selected row, the focus ring, the selected text; never under light
//! text, never a button. Blue is no colour of it: the muted slate of « info » alone. The upstream looks (its accents)
//! stay those of the program that keeps its mail on the computer.
//!
//! Where the design system gives a translucent value on dark (`--border-*`, `--color-primary-soft`,
//! `--color-accent-soft`), it is laid here over `--surface-card`, the sheet it sits on most: an opaque colour, as every
//! token of the window is (one is laid over another, mixed, measured for contrast).

use egui::{Color32, Stroke};

use super::Tokens;

fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// `--signal-400`, the signal.
const SIGNAL: u32 = 0xF55D1A;

impl Tokens {
    /// The Neoffice design system's tokens, for a window that is `dark` or not.
    pub fn neoffice(dark: bool) -> Tokens {
        if dark { neoffice_dark() } else { neoffice_light() }
    }
}

/// `:root`: the paper.
fn neoffice_light() -> Tokens {
    // --color-bg (--paper-cream), --surface-subtle (--sand-25), --surface-card.
    let paper = hex(0xFAF3EA);
    let panel = hex(0xFBFAF7);
    let surface = Color32::WHITE;
    // --text-primary (--sand-950), --text-secondary (--sand-600), --text-tertiary (--sand-400).
    let text = hex(0x141414);
    let text_2 = hex(0x524B41);
    let text_3 = hex(0x968C7C);
    // A hover (--sand-100, the top bar's icon buttons), pressed (--sand-200).
    let hover = hex(0xECE7DE);
    // --signal-50, the tint: the wash of what is chosen.
    let tint = hex(0xFFEAE2);
    let signal = hex(SIGNAL);
    Tokens {
        dark: false,
        neoffice: true,
        high_contrast: false,
        sender_dark: false,
        title_bar: paper,
        ribbon: surface,
        tab_strip: paper,
        nav: paper,
        canvas: paper,
        panel,
        surface,
        // --border-default (--sand-200), --border-strong (--sand-300).
        border: hex(0xDCD4C7),
        border_strong: hex(0xBFB5A4),
        text,
        text_dim: text_2,
        text_disabled: text_3,
        // --sand-700: the icons' ink, a step under the text's.
        icon: hex(0x3A342C),
        hover,
        pressed: hex(0xDCD4C7),
        checked: tint,
        // --color-primary: every action ink, --color-primary-fg on it.
        accent: text,
        // --text-brand (--signal-600): the signal as text on paper.
        accent_text: hex(0xAA3700),
        on_accent: hex(0xFFFDF8),
        input: surface,
        // --sand-400: a field's edge, 3:1 on the paper.
        input_border: text_3,
        group_label: text_2,
        status_bar: paper,
        selection: tint,
        // --amber-100: a search hit.
        find_highlight: Color32::from_rgba_unmultiplied(0xF8, 0xE2, 0xB4, 0xD0),
        // --red-50, --red-700.
        flagged_row: hex(0xFBEBE7),
        overdue: hex(0x9C2920),
        // --surface-sunken (--sand-50).
        off_hours: hex(0xF6F3ED),
        today: tint,
        menu: surface,
        // --blue-600, the muted slate of « info »; --red-600, --green-600, --amber-700.
        blue: hex(0x3A5572),
        red: hex(0xC1372C),
        green: hex(0x178347),
        orange: hex(0x92610C),
        icon_red: hex(0xC1372C),
        icon_yellow: hex(0x92610C),
        icon_green: hex(0x178347),
        // A category's violet: the design system has none, a mailbox colour keeps its own.
        icon_purple: hex(0x7B4FB8),
        // --surface-overlay.
        scrim: Color32::from_rgba_unmultiplied(24, 20, 16, 140),
        shadow_popup: Color32::from_black_alpha(40),
        shadow_window: Color32::from_black_alpha(50),
        // `::selection`: the signal.
        text_selection: signal.linear_multiply(0.30),
        selection_unfocused: hex(0xF6F3ED),
        group_header: Color32::TRANSPARENT,
        selection_edge: signal,
        selection_edge_unfocused: hex(0xBFB5A4),
        // --color-accent-soft and its ink (--color-accent-soft-fg, --signal-700): a count, a chosen folder.
        accent_soft: tint,
        accent_soft_text: hex(0x81300F),
        pill_quiet: hover,
        // --border-subtle (--sand-100).
        border_faint: hover,
        selection_pill: tint,
        hover_subtle: hover.linear_multiply(0.6),
        hover_nav: panel.lerp_to_gamma(hover, 0.7),
        surface_sunken: surface.linear_multiply(0.55),
        // --focus-ring: the signal at 38 %.
        focus_ring: Color32::from_rgba_unmultiplied(245, 93, 26, 97),
        rule: hover,
        track: hex(0xDCD4C7),
        lane: paper,
        ribbon_edge: Stroke::new(1.0, hover),
        pane_rule: true,
        field_edge: text_3,
        backstage_column: panel,
        backstage_ink: text,
        backstage_mark: signal,
        data_tint: 0.22,
        data_soft: 0.20,
        data_mix: 0.3,
        band_chosen: signal.linear_multiply(0.16),
        band_meeting: signal.linear_multiply(0.16),
        notice_wash_alpha: 30,
        notice_edge_alpha: 90,
        on_avatar: Color32::WHITE,
        avatar_backdrop: Color32::WHITE,
        on_swatch: Color32::WHITE,
        ink_on_dark: surface,
        ink_on_light: text,
        avatar_shade: 1.0,
    }
}

/// `.theme-dark`: the warm espresso.
fn neoffice_dark() -> Tokens {
    // --color-bg (--sand-950), --surface-subtle (Nora's pane), --surface-card, --surface-raised.
    let window = hex(0x141414);
    let panel = hex(0x161413);
    let surface = hex(0x1B1611);
    let raised = hex(0x221C16);
    // --text-primary (--paper-bright), --text-secondary (--sand-300), --text-tertiary (--sand-400).
    let text = hex(0xFFFDF8);
    let text_2 = hex(0xBFB5A4);
    let text_3 = hex(0x968C7C);
    // --color-primary-soft (10 %) as a hover, 16 % pressed; --border-subtle (8 %), --border-default (14 %),
    // --border-strong (24 %): the paper over --surface-card.
    let hover = hex(0x322D28);
    let faint = hex(0x2D2823);
    // --color-accent-soft: the signal at 16 % over --surface-card.
    let tint = hex(0x3E2112);
    let signal = hex(SIGNAL);
    Tokens {
        dark: true,
        neoffice: true,
        high_contrast: false,
        sender_dark: false,
        title_bar: window,
        ribbon: raised,
        tab_strip: window,
        nav: window,
        canvas: window,
        panel,
        surface,
        border: hex(0x3B3631),
        border_strong: hex(0x524D48),
        text,
        text_dim: text_2,
        text_disabled: text_3,
        // --sand-100.
        icon: hex(0xECE7DE),
        hover,
        pressed: hex(0x3F3B35),
        checked: tint,
        // On dark the primary action is the bright paper, ink on it.
        accent: text,
        // --text-brand (--signal-300).
        accent_text: hex(0xFF8D64),
        on_accent: window,
        input: surface,
        input_border: text_3,
        group_label: text_2,
        status_bar: window,
        selection: tint,
        // --amber-500, translucent.
        find_highlight: Color32::from_rgba_unmultiplied(0xE0, 0x98, 0x0F, 0x70),
        // --red-500 at 16 % over --surface-card, --red-100.
        flagged_row: hex(0x3A1E18),
        overdue: hex(0xF6D2C9),
        off_hours: panel,
        today: tint,
        menu: raised,
        // --blue-100 (« info »), --red-100, --green-500, --amber-500.
        blue: hex(0xDCE4EC),
        red: hex(0xF6D2C9),
        green: hex(0x1F9D57),
        orange: hex(0xE0980F),
        // --red-500, --amber-500, --green-500.
        icon_red: hex(0xDC4A3D),
        icon_yellow: hex(0xE0980F),
        icon_green: hex(0x1F9D57),
        icon_purple: hex(0xB69BEE),
        // --surface-overlay.
        scrim: Color32::from_black_alpha(168),
        shadow_popup: Color32::from_black_alpha(120),
        shadow_window: Color32::from_black_alpha(140),
        text_selection: signal.linear_multiply(0.45),
        selection_unfocused: faint,
        group_header: Color32::TRANSPARENT,
        selection_edge: signal,
        // --sand-500.
        selection_edge_unfocused: hex(0x6F665A),
        accent_soft: tint,
        // --color-accent-soft-fg (--signal-200).
        accent_soft_text: hex(0xFFB599),
        pill_quiet: hover,
        border_faint: faint,
        selection_pill: tint,
        hover_subtle: hover.linear_multiply(0.6),
        hover_nav: panel.lerp_to_gamma(hover, 0.7),
        surface_sunken: surface.linear_multiply(0.55),
        // --focus-ring: the signal at 42 %.
        focus_ring: Color32::from_rgba_unmultiplied(245, 93, 26, 107),
        rule: faint,
        track: hex(0x3F3B35),
        lane: panel,
        ribbon_edge: Stroke::new(1.0, faint),
        pane_rule: true,
        field_edge: text_3,
        backstage_column: panel,
        backstage_ink: text,
        backstage_mark: signal,
        data_tint: 0.45,
        data_soft: 0.36,
        data_mix: 0.5,
        band_chosen: signal.linear_multiply(0.35),
        band_meeting: signal.linear_multiply(0.28),
        notice_wash_alpha: 46,
        notice_edge_alpha: 110,
        on_avatar: Color32::WHITE,
        avatar_backdrop: Color32::WHITE,
        on_swatch: Color32::WHITE,
        ink_on_dark: text,
        ink_on_light: surface,
        avatar_shade: 0.9,
    }
}
