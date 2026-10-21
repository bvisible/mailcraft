// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Interface text. Every string the window shows goes through [`t`] (the `tl!` macro) or, for a
//! count, [`tn`] (`tln!`), keyed by its English text and looked up in the catalogue of the
//! window's language (`mailcraft-i18n`). The window's language is set once a frame from the
//! owner's choice ([`set_current`]); the engine is told it through `Session::interface_language`.

// //// Neoffice — the engine's and a JMAP run's sentences, read back in the interface's language where they are drawn.
pub mod run_words;

pub use mailcraft_i18n::{AUTO, Lang, fmt};

// //// Neoffice — the window's language kept per thread in the tests (as before the merge of the catalogues), so that a test that
// changes it can't change the language of another running beside it; one for the process otherwise, as upstream keeps it.
#[cfg(not(test))]
mod state {
    use std::sync::atomic::{AtomicU8, Ordering};

    static CURRENT: AtomicU8 = AtomicU8::new(0);

    pub fn get() -> u8 {
        CURRENT.load(Ordering::Relaxed)
    }
    pub fn set(value: u8) {
        CURRENT.store(value, Ordering::Relaxed);
    }
}

// //// Neoffice — as above.
#[cfg(test)]
mod state {
    use std::cell::Cell;

    thread_local! {
        static CURRENT: Cell<u8> = const { Cell::new(0) };
    }

    pub fn get() -> u8 {
        CURRENT.get()
    }
    pub fn set(value: u8) {
        CURRENT.set(value);
    }
}

/// The window's language from now on.
pub fn set_current(lang: Lang) {
    let i = Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0);
    // //// Neoffice — kept in `state` (above).
    state::set(u8::try_from(i).unwrap_or(0));
}

/// The window's language.
pub fn current() -> Lang {
    // //// Neoffice — as above.
    Lang::ALL.get(usize::from(state::get())).copied().unwrap_or_default()
}

/// `s` in the interface language.
pub fn t(s: &str) -> &str {
    // //// Neoffice — under the product's name a host gives, when it gives one (`set_product_name`).
    renamed(current().t(s))
}

/// `s` in `context` (`msgctxt`), in the interface language.
pub fn tc<'a>(context: &str, s: &'a str) -> &'a str {
    // //// Neoffice — as above.
    renamed(current().tc(context, s))
}

// //// Neoffice — `t` in a language that is given, so that what reads the engine's and a JMAP run's sentences back (`run_words`) can be
// asked for any language without changing the window's.
/// `s` in `lang`, under the product's name a host gives.
pub fn t_in(lang: Lang, s: &str) -> &str {
    renamed(lang.t(s))
}

/// The form for a count of `n` of `one` / `other` (English singular and plural), in the interface
/// language, with `{n}` filled in (other placeholders are left to [`fmt`]).
pub fn tn(n: u64, one: &str, other: &str) -> String {
    // //// Neoffice — under the product's name a host gives (`set_product_name`).
    fmt(renamed(current().tn(n, one, other)), &[("n", &n.to_string())])
}

/// A number of things, for [`tn`]: any integer (a negative one counts as none).
pub trait Count {
    fn count(self) -> u64;
}

macro_rules! count {
    ($($t:ty),*) => {$(
        impl Count for $t {
            fn count(self) -> u64 {
                u64::try_from(self).unwrap_or(0)
            }
        }
    )*};
}
count!(u8, u16, u32, u64, usize, i32, i64, isize);

/// A command's location (`Home › Respond`) in the interface language, part by part.
pub fn location(loc: &str) -> String {
    mailcraft_i18n::location(current(), loc)
}

// //// Neoffice — the name a host gives the program (a page that frames the browser build names it its own way: NeoMail).
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
/// `s` with the program's name a host gave, interned so that it can be handed out like a catalogue's entry.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_by_default() {
        // (The window's language is shared by the tests running alongside: only English here.)
        assert_eq!(t("Reply"), "Reply");
        assert_eq!(tn(1, "{n} message", "{n} messages"), "1 message");
        assert_eq!(tn(3, "{n} message", "{n} messages"), "3 messages");
        assert_eq!(location("Home › Respond"), "Home › Respond");
    }

    // //// Neoffice — the name a host gives the program, in every language's text, the plurals' included.
    #[test]
    fn a_hosts_name_for_the_program_is_the_one_its_text_shows_in_every_language() {
        assert_eq!(t("About MailCraft"), "About MailCraft", "its own name until a host gives one");
        set_product_name("NeoMail");
        assert_eq!(t("About MailCraft"), "About NeoMail");
        assert_eq!(t_in(Lang::English, "About MailCraft"), "About NeoMail");
        assert_eq!(tn(2, "MailCraft has {n} window", "MailCraft has {n} windows"), "NeoMail has 2 windows");
        set_product_name("   ");
        assert_eq!(t("About MailCraft"), "About MailCraft", "an empty name puts its own back");
        set_product_name(&"x".repeat(41));
        assert_eq!(t("About MailCraft"), "About MailCraft", "and so does one too long");
    }
}
