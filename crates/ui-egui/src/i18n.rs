// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Interface text. Every string the interface shows goes through [`t`] (the `tl!` macro), keyed by
//! its English text, so language catalogs can be added without touching the widgets. `pseudo` (a
//! test language) proves that no string bypasses the lookup.
// //// Neoffice — English is no longer the only language: the others come from a catalog each (`catalog`).
//!
//! # Languages
//! English is the language of the strings as they are written. Any other language is a catalog
//! ([`catalog`]) that maps them to what that language shows, and a string it lacks shows in
//! English. The `language` setting ([`Lang::from_pref`]) is a language's code, or `auto` to follow
//! the language that the host says the system (or the browser) is in ([`set_system_language`]).

// //// Neoffice — the catalogs, and the tests that hold them to the interface.
pub mod catalog;
#[cfg(test)]
mod interface_strings;
#[cfg(test)]
mod language_setting;

/// The interface language setting that follows the system.
pub const AUTO: &str = "auto";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    English,
    /// Accented look-alikes of the English text, for finding strings that skip translation.
    Pseudo,
    // //// Neoffice — a language that has a catalog.
    /// A language that has a catalog ([`catalog::LANGUAGES`]).
    Catalog(catalog::Id),
}

// //// Neoffice — the current language and the system's, as numbers (0 English, 1 pseudo, then a catalog's row plus 2).
// One of each per process; in tests one per thread, so that tests that run side by side and change the language can't change one another's.
#[cfg(not(test))]
mod state {
    use std::sync::atomic::{AtomicU8, Ordering};

    static CURRENT: AtomicU8 = AtomicU8::new(0);
    static SYSTEM: AtomicU8 = AtomicU8::new(0);

    pub fn current() -> u8 {
        CURRENT.load(Ordering::Relaxed)
    }
    pub fn set_current(value: u8) {
        CURRENT.store(value, Ordering::Relaxed);
    }
    pub fn system() -> u8 {
        SYSTEM.load(Ordering::Relaxed)
    }
    pub fn set_system(value: u8) {
        SYSTEM.store(value, Ordering::Relaxed);
    }
}

// //// Neoffice — in tests, one language per thread: tests that run side by side and change it can't change one another's.
#[cfg(test)]
mod state {
    use std::cell::Cell;

    thread_local! {
        static CURRENT: Cell<u8> = const { Cell::new(0) };
        static SYSTEM: Cell<u8> = const { Cell::new(0) };
    }

    pub fn current() -> u8 {
        CURRENT.get()
    }
    pub fn set_current(value: u8) {
        CURRENT.set(value);
    }
    pub fn system() -> u8 {
        SYSTEM.get()
    }
    pub fn set_system(value: u8) {
        SYSTEM.set(value);
    }
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::Pseudo => "pseudo",
            // //// Neoffice — a language that has a catalog.
            Lang::Catalog(id) => id.row().map_or("en", |row| row.code),
        }
    }

    // //// Neoffice — the language's own name, for the list of languages in File › Options.
    pub fn name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::Pseudo => "Pseudo",
            Lang::Catalog(id) => id.row().map_or("English", |row| row.name),
        }
    }

    // //// Neoffice — a code is any language that has a catalog, in any case and with or without a region; `auto` is the system's.
    /// The language for a saved setting: `auto` (the system's language, see [`set_system_language`]), `pseudo`, or the code of a
    /// language, in any case and with or without a region (`fr`, `FR`, `fr-CH`); anything unknown is English.
    pub fn from_pref(pref: &str) -> Lang {
        let pref = pref.trim();
        if pref.eq_ignore_ascii_case(AUTO) {
            Lang::decode(state::system())
        } else if pref.eq_ignore_ascii_case("pseudo") {
            Lang::Pseudo
        } else {
            Lang::from_tag(pref).unwrap_or(Lang::English)
        }
    }

    // //// Neoffice — what a locale tag (`fr`, `fr-CH`, `fr_CH.UTF-8`, `en-GB`) names, when that is English or a language with a catalog.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        if let Some(id) = catalog::find(tag) {
            return Some(Lang::Catalog(id));
        }
        let primary = tag.trim().split(['-', '_', '.', '@']).next().unwrap_or_default();
        primary.eq_ignore_ascii_case("en").then_some(Lang::English)
    }

    // //// Neoffice — the languages a person can choose among: English, then those that have a catalog.
    pub fn choices() -> impl Iterator<Item = Lang> {
        std::iter::once(Lang::English).chain(catalog::Id::all().map(Lang::Catalog))
    }

    // //// Neoffice — a language as the number it is kept as.
    fn encode(self) -> u8 {
        match self {
            Lang::English => 0,
            Lang::Pseudo => 1,
            Lang::Catalog(id) => id.index().checked_add(2).unwrap_or(0),
        }
    }

    fn decode(value: u8) -> Lang {
        match value {
            0 => Lang::English,
            1 => Lang::Pseudo,
            n => n.checked_sub(2).and_then(|row| catalog::Id::new(usize::from(row))).map_or(Lang::English, Lang::Catalog),
        }
    }
}

// //// Neoffice — a value of the `language` setting in its one form: `auto`, `pseudo` or a language's code (`None`: it is none of those).
pub fn canonical_pref(pref: &str) -> Option<&'static str> {
    let pref = pref.trim();
    if pref.eq_ignore_ascii_case(AUTO) {
        Some(AUTO)
    } else if pref.eq_ignore_ascii_case("pseudo") {
        Some(Lang::Pseudo.code())
    } else {
        Lang::from_tag(pref).map(Lang::code)
    }
}

// //// Neoffice — the language of the system, as the host says it, for the setting `auto`.
/// Tell the interface which language the system (or the browser) is in, as a locale tag such as `fr`, `fr-CH` or `fr_CH.UTF-8`:
/// what the setting `auto` shows. A tag that names no language with a catalog means English, as does never calling this.
pub fn set_system_language(code: &str) {
    state::set_system(Lang::from_tag(code).unwrap_or(Lang::English).encode());
}

pub fn set_current(lang: Lang) {
    // //// Neoffice — kept as a number in `state`, which a catalog language has too.
    state::set_current(lang.encode());
    // //// Neoffice — the dates people read follow the language too (date_words).
    mailcraft_time::set_date_words(date_words());
}

// //// Neoffice — the day and month names of the interface's language, for the dates people read; a numeric date day first.
/// The words the dates of the message list and of an open message are written with: none in English.
fn date_words() -> Option<mailcraft_time::DateWords> {
    if current() == Lang::English {
        return None;
    }
    Some(mailcraft_time::DateWords {
        weekdays_short: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map(|day| t(day).to_string()),
        months_short: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].map(|month| t(month).to_string()),
        day_first: Some('.'),
    })
}

pub fn current() -> Lang {
    // //// Neoffice — as above.
    Lang::decode(state::current())
}

/// `s` in the interface language.
pub fn t(s: &str) -> &str {
    // //// Neoffice — under the product's name a host gives, when it gives one (`set_product_name`).
    renamed(match current() {
        Lang::English => s,
        Lang::Pseudo => pseudo(s),
        // //// Neoffice — the catalog's translation; the English without one.
        Lang::Catalog(id) => catalog::translate(id, "", s).unwrap_or(s),
    })
}

// //// Neoffice — `s` under a context, for English words that need two translations (the Archive folder, the Archive command).
/// `s` where `context` tells words that are the same in English apart: the catalog's entry under that context, else what [`t`] gives.
pub fn tc<'a>(context: &str, s: &'a str) -> &'a str {
    if let Lang::Catalog(id) = current()
        && let Some(translation) = catalog::translate(id, context, s)
    {
        return renamed(translation);
    }
    t(s)
}

// //// Neoffice — the name a host gives the program (a page that frames the browser build names it its own way).
/// The name the program is written with in its interface text.
pub const PRODUCT: &str = "MailCraft";

// //// Neoffice — as above.
/// Show the program under `name` wherever its interface text names it ([`PRODUCT`]): a host that frames the program and
/// gives it its own name calls this once, before the first frame. An empty name, or one longer than 40 characters, puts
/// the program's own back.
pub fn set_product_name(name: &str) {
    let name = name.trim();
    let name = (!name.is_empty() && name.chars().count() <= 40 && !name.chars().any(char::is_control)).then(|| name.to_string());
    product::set(name);
}

// //// Neoffice — as above.
/// `s` with the program's name a host gave, interned so that it can be handed out like a catalog's entry.
fn renamed(s: &str) -> &str {
    if !s.contains(PRODUCT) {
        return s;
    }
    product::renamed(s).unwrap_or(s)
}

// //// Neoffice — the product's name and the strings written with it, one per process (one per thread in tests, as `state`).
mod product {
    use std::collections::HashMap;

    #[cfg(not(test))]
    use std::sync::{Mutex, OnceLock};

    #[cfg(not(test))]
    static NAME: OnceLock<Mutex<(Option<String>, HashMap<String, &'static str>)>> = OnceLock::new();

    #[cfg(not(test))]
    fn with<R>(f: impl FnOnce(&mut (Option<String>, HashMap<String, &'static str>)) -> R) -> R {
        let mut guard = NAME.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    #[cfg(test)]
    thread_local! {
        static NAME: std::cell::RefCell<(Option<String>, HashMap<String, &'static str>)> = Default::default();
    }

    #[cfg(test)]
    fn with<R>(f: impl FnOnce(&mut (Option<String>, HashMap<String, &'static str>)) -> R) -> R {
        NAME.with_borrow_mut(f)
    }

    pub fn set(name: Option<String>) {
        with(|(current, done)| {
            if *current != name {
                *current = name;
                done.clear();
            }
        });
    }

    /// `s` written with the host's name, when it gave one. The interface's strings are few and fixed, so each form is
    /// made once (as the pseudo-language's are).
    pub fn renamed(s: &str) -> Option<&'static str> {
        with(|(current, done)| {
            let name = current.as_deref()?;
            if let Some(form) = done.get(s) {
                return Some(*form);
            }
            let form: &'static str = Box::leak(s.replace(super::PRODUCT, name).into_boxed_str());
            done.insert(s.to_string(), form);
            Some(form)
        })
    }
}

/// The pseudo-language form of `s`, interned so it can be handed out as `&'static str`.
fn pseudo(s: &str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(done) = cache.get(s) {
        return done;
    }
    // `{placeholders}` stay as they are.
    let mut out = String::with_capacity(s.len() + 2);
    let mut inside = false;
    for c in s.chars() {
        match c {
            '{' => inside = true,
            '}' => inside = false,
            _ => {}
        }
        out.push(if inside {
            c
        } else {
            match c {
                'a' => 'á',
                'e' => 'é',
                'i' => 'í',
                'o' => 'ó',
                'u' => 'ú',
                'A' => 'Á',
                'E' => 'É',
                'O' => 'Ó',
                'U' => 'Ú',
                _ => c,
            }
        });
    }
    // The catalog is small and fixed (interface strings), so leaking each form once is bounded.
    let leaked: &'static str = Box::leak(out.into_boxed_str());
    cache.insert(s.to_string(), leaked);
    leaked
}

/// Fill `{name}` placeholders in a (translated) template.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

// //// Neoffice — the separator, named so that the tests that list the parts of the locations split at the same one.
/// What the parts of a command's location are separated by.
pub const LOCATION_SEPARATOR: &str = " › ";

/// A command's location (`Home › Respond`) in the interface language, part by part.
pub fn location(loc: &str) -> String {
    // //// Neoffice — the separator has a name.
    loc.split(LOCATION_SEPARATOR).map(t).collect::<Vec<_>>().join(LOCATION_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_and_pseudo() {
        assert_eq!(fmt("{n} of {total} items", &[("n", "3"), ("total", "10")]), "3 of 10 items");
        assert_eq!(pseudo("Reply {name}"), "Réply {name}");
        assert_eq!(Lang::from_pref("PSEUDO"), Lang::Pseudo);
        assert_eq!(Lang::from_pref("klingon"), Lang::English);
        assert_eq!(location("Home › Respond"), "Home › Respond");
    }

    // //// Neoffice — the setting that picks the language, and the system's language it can follow.
    fn french() -> Lang {
        Lang::from_tag("fr").unwrap()
    }

    // //// Neoffice — the name a host gives the program.
    #[test]
    fn a_hosts_name_for_the_program_is_the_one_its_text_shows_in_every_language() {
        assert_eq!(t("About MailCraft"), "About MailCraft", "its own name until a host gives one");
        set_product_name("NeoMail");
        assert_eq!(t("About MailCraft"), "About NeoMail");
        assert_eq!(t("Reply"), "Reply", "a string that doesn't name it is as it was");
        set_current(french());
        assert_eq!(t("About MailCraft"), "À propos de NeoMail");
        assert_eq!(tc("", "About MailCraft"), "À propos de NeoMail");
        set_current(Lang::English);
        // A name that is empty, too long or not one line puts the program's own back.
        for refused in ["", "   ", &"x".repeat(41), "Neo\nMail"] {
            set_product_name("NeoMail");
            set_product_name(refused);
            assert_eq!(t("About MailCraft"), "About MailCraft", "{refused:?}");
        }
    }

    #[test]
    fn the_setting_is_a_code_in_any_case_with_or_without_a_region() {
        for pref in ["fr", "FR", "Fr", " fr ", "fr-CH", "fr_CH", "fr-ch", "fr_CA.UTF-8", "fr-Latn-BE"] {
            assert_eq!(Lang::from_pref(pref), french(), "{pref:?}");
        }
        for pref in ["en", "EN", "en-GB", "en_US.UTF-8", "", " ", "klingon", "français", "fra", "frr", "f", "-", "fr\u{0}", "C", "POSIX", "de", "ar"]
        {
            assert_eq!(Lang::from_pref(pref), Lang::English, "{pref:?}");
        }
        for pref in ["pseudo", "PSEUDO", " Pseudo "] {
            assert_eq!(Lang::from_pref(pref), Lang::Pseudo, "{pref:?}");
        }
        // A setting from a file that someone edited can be anything.
        assert_eq!(Lang::from_pref(&"x".repeat(100_000)), Lang::English);
        assert_eq!(Lang::from_pref(&format!("fr-{}", "-".repeat(100_000))), french());
        assert_eq!(Lang::from_pref("\u{feff}fr"), Lang::English);
        assert_eq!(french().code(), "fr");
        assert_eq!(french().name(), "Français");
        assert_eq!((Lang::English.code(), Lang::English.name(), Lang::Pseudo.code()), ("en", "English", "pseudo"));
    }

    #[test]
    fn auto_follows_the_language_the_host_says_the_system_is_in() {
        for pref in ["auto", "AUTO", "Auto", " auto "] {
            // The host hasn't said: English.
            assert_eq!(Lang::from_pref(pref), Lang::English, "{pref:?}");
        }
        for (said, shown) in [
            ("fr", french()),
            ("fr-CH", french()),
            ("fr_CA.UTF-8", french()),
            ("FR", french()),
            ("en-GB", Lang::English),
            ("de", Lang::English),
            ("", Lang::English),
            ("klingon", Lang::English),
            ("pseudo", Lang::English),
            ("auto", Lang::English),
        ] {
            set_system_language(said);
            for pref in ["auto", "AUTO", " auto "] {
                assert_eq!(Lang::from_pref(pref), shown, "the system says {said:?}, the setting is {pref:?}");
            }
            // A language that is chosen is not the system's.
            assert_eq!(Lang::from_pref("en"), Lang::English);
            assert_eq!(Lang::from_pref("fr"), french());
        }
        set_system_language("fr");
        set_system_language("de");
        assert_eq!(Lang::from_pref("auto"), Lang::English, "what the host says last is what counts");
    }

    #[test]
    fn a_tag_names_a_language_and_a_setting_has_one_form() {
        assert_eq!(Lang::from_tag("fr-CH"), Some(french()));
        assert_eq!(Lang::from_tag("en"), Some(Lang::English));
        assert_eq!(Lang::from_tag("EN_us.UTF-8"), Some(Lang::English));
        assert_eq!(Lang::from_tag("english"), None, "a word that merely starts like a code is not one");
        assert_eq!(Lang::from_tag("pseudo"), None);
        assert_eq!(Lang::from_tag("auto"), None);
        assert_eq!(Lang::from_tag("de"), None);
        assert_eq!(Lang::from_tag(""), None);
        for (pref, one) in [("auto", "auto"), (" AUTO", "auto"), ("fr-CH", "fr"), ("FR", "fr"), ("en-GB", "en"), ("PSEUDO", "pseudo")] {
            assert_eq!(canonical_pref(pref), Some(one), "{pref:?}");
        }
        for pref in ["", "de", "klingon", "fra", "autom", "français"] {
            assert_eq!(canonical_pref(pref), None, "{pref:?}");
        }
    }

    #[test]
    fn the_languages_to_choose_among_are_english_and_those_with_a_catalog() {
        let codes: Vec<&str> = Lang::choices().map(Lang::code).collect();
        assert_eq!(codes.first(), Some(&"en"));
        assert_eq!(codes.len(), 1 + catalog::LANGUAGES.len());
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "{codes:?}");
        for lang in Lang::choices() {
            assert_eq!(Lang::from_pref(lang.code()), lang, "a language's code is its setting");
            assert!(!lang.name().is_empty());
        }
        assert!(Lang::choices().all(|lang| lang != Lang::Pseudo));
    }

    #[test]
    fn a_language_is_kept_as_a_number_and_comes_back() {
        for lang in Lang::choices().chain([Lang::Pseudo]) {
            assert_eq!(Lang::decode(lang.encode()), lang, "{lang:?}");
            set_current(lang);
            assert_eq!(current(), lang);
        }
        // A number no language has is English.
        for value in [u8::try_from(2 + catalog::LANGUAGES.len()).unwrap_or(u8::MAX), 250, 255] {
            assert_eq!(Lang::decode(value), Lang::English, "{value}");
        }
        set_current(Lang::English);
        assert_eq!(current(), Lang::English);
    }

    #[test]
    fn a_string_a_catalog_lacks_shows_in_english_and_pseudo_shows_every_string_changed() {
        let stranger = "A string that no catalog will ever hold ☃";
        for lang in Lang::choices() {
            set_current(lang);
            assert!(std::ptr::eq(t(stranger), stranger), "{lang:?}: the English itself comes back");
            assert!(std::ptr::eq(tc("some context", stranger), stranger), "{lang:?}: and under a context too");
            assert_eq!(fmt(t("{n} of {total} items"), &[("n", "3"), ("total", "10")]).split(' ').count(), 4);
        }
        set_current(Lang::Pseudo);
        assert_eq!(t("Reply"), "Réply");
        assert_eq!(tc("context", "Reply"), "Réply");
        // (A command's location is translated part by part, at the same separator.)
        assert_eq!(location("Home › Respond"), "Hómé › Réspónd");
        set_current(french());
        assert_eq!(location("Home › Respond").split(" › ").count(), 2);
        set_current(Lang::English);
        assert_eq!(t("Reply"), "Reply");
    }

    #[test]
    fn the_language_of_one_test_is_not_the_language_of_another() {
        // In tests each thread has its own current and system language (in the program, one for all).
        set_current(french());
        set_system_language("fr");
        let (current, auto) = std::thread::spawn(|| (current(), Lang::from_pref("auto"))).join().unwrap();
        assert_eq!((current, auto), (Lang::English, Lang::English));
        assert_eq!(super::current(), french());
        set_current(Lang::English);
        set_system_language("");
    }
}
