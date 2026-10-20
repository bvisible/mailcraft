// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The language of the interface in the program as it runs: one for every thread. (In the crate's
//! own tests the current and the system's language are one per thread, so that tests that change
//! them can't disturb each other; this is the one test that sees what a running program sees.)

use mailcraft_ui_egui::i18n::{self, Lang};

#[test]
fn the_languages_are_the_same_on_every_thread() {
    // The host says what the system's language is once, at start, from whichever thread it is on.
    std::thread::spawn(|| i18n::set_system_language("fr-CH")).join().unwrap();
    assert_eq!(Lang::from_pref("auto").code(), "fr");
    assert_eq!(std::thread::spawn(|| Lang::from_pref("auto").code()).join().unwrap(), "fr");
    assert_eq!(Lang::from_pref("en").code(), "en", "a language that is chosen is not the system's");

    // The window draws in the language of the setting, and everything on any thread follows it.
    i18n::set_current(Lang::from_pref("auto"));
    assert_eq!(std::thread::spawn(|| i18n::current().code()).join().unwrap(), "fr");
    i18n::set_current(Lang::Pseudo);
    assert_eq!(std::thread::spawn(|| i18n::t("Reply").to_string()).join().unwrap(), "Réply");
    std::thread::spawn(|| i18n::set_current(Lang::English)).join().unwrap();
    assert_eq!((i18n::current(), i18n::t("Reply")), (Lang::English, "Reply"));

    // A language the host doesn't know is English, whatever it said before.
    i18n::set_system_language("tlh");
    assert_eq!(Lang::from_pref("auto"), Lang::English);
}
