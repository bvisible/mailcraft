// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The language of the interface in the program as it runs: one for every thread. (In the crate's
//! own tests it is one per thread, so that tests that change it can't disturb each other; this is
//! the one test that sees what a running program sees.)

use mailcraft_ui_egui::i18n::{self, Lang};

#[test]
fn the_language_is_the_same_on_every_thread() {
    // Automatic follows the locale the host says the system (or the hosting page) is in.
    assert_eq!(Lang::from_pref("auto", Some("fr-CH")), Lang::French);
    assert_eq!(Lang::from_pref("en", Some("fr-CH")), Lang::English, "a language that is chosen is not the system's");
    assert_eq!(Lang::from_pref("auto", Some("tlh")), Lang::English, "a language without a catalogue is English");

    // The window draws in the language of the setting, and everything on any thread follows it.
    i18n::set_current(Lang::from_pref("auto", Some("fr-CH")));
    assert_eq!(std::thread::spawn(|| i18n::current().code()).join().unwrap(), "fr");
    i18n::set_current(Lang::Pseudo);
    assert_eq!(std::thread::spawn(|| i18n::t("Reply").to_string()).join().unwrap(), "Réply");
    std::thread::spawn(|| i18n::set_current(Lang::English)).join().unwrap();
    assert_eq!((i18n::current(), i18n::t("Reply")), (Lang::English, "Reply"));
}
