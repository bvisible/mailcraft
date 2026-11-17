// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What a paste in the browser carries as HTML, for the message being written (neoffice-maintenance#1705).
//!
//! eframe's web runner reads only the `text/plain` of the browser's `paste` event, and the browser build gave the
//! interface no clipboard of its own ([`mailcraft_ui_egui::Services::clipboard`] stayed `None`): a message, a web page,
//! a Word or Excel document pasted into a message kept their text alone. The desktop app reads the system's clipboard
//! (`apps/mailcraft/src/clipboard.rs`); a page can't read it when it likes, but each paste event carries its data.
//!
//! So a listener on the window, in the capture phase (before eframe's own on the document), keeps the `text/html` of
//! each paste with the time it came, and the clipboard below hands it to the paste egui then makes from the same event
//! ([`Caught::take`]):
//! - once: a second paste is never made from it;
//! - while it is fresh ([`FRESH_MS`]): a paste into a field of one line never asks for it, and what comes after (the
//!   body's next paste, in a minute) never brings it back;
//! - without the pictures written into it (`data:` addresses, [`without_inline_pictures`]): the engine leaves a pasted
//!   HTML's pictures out (`rich::from_html`), and a document with a large picture would otherwise be over the most one
//!   paste takes and paste nothing of its formatting;
//! - no more than the engine takes ([`MAX_HTML`]); over that, the text alone, as a paste without HTML.
//!
//! What is kept of the HTML is the engine's to decide: `edit.paste` reduces it to what a message holds (bold, italics,
//! lists, links, tables; no script, no style sheet, no picture, nothing that runs or is fetched), and the server cleans
//! again what is sent. The clipboard can't be read when asked ([`RichClipboard::reads_on_demand`]): a text's menu keeps
//! its Paste greyed and a signature offers no Paste Picture, as before. A copy stays text: `put` says no, and the
//! editor copies the text (`compose/editor.rs`).
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

/// The most HTML one paste takes, in bytes: the engine's limit (`edit.paste`), as the desktop's clipboard has it.
pub const MAX_HTML: usize = 4 << 20;

/// How long the HTML of a paste waits for the paste egui makes from the same event, in milliseconds. egui takes the
/// event at its next frame; a paste that asks later is another one.
pub const FRESH_MS: f64 = 2_000.0;

/// The HTML of the last paste event, and when it came (milliseconds of the browser's clock).
#[derive(Debug, Default)]
pub struct Caught {
    last: Option<(String, f64)>,
}

impl Caught {
    /// A paste event came at `at` with `html` on it, or with none: what it carries replaces what an earlier one did.
    pub fn caught(&mut self, html: Option<&str>, at: f64) {
        self.last = html.map(without_inline_pictures).filter(|html| !html.trim().is_empty() && html.len() <= MAX_HTML).map(|html| (html, at));
    }

    /// The HTML of the last paste event, once, when it came no more than [`FRESH_MS`] before `now`.
    pub fn take(&mut self, now: f64) -> Option<String> {
        let (html, at) = self.last.take()?;
        (now >= at && now - at <= FRESH_MS).then_some(html)
    }
}

/// `html` without the pictures written into it: each quoted attribute value or CSS `url(…)` that is a `data:` address
/// is emptied (`src=""`, `url()`), the rest is left as it came. The engine keeps no picture of a pasted HTML anyway;
/// what is taken out here is only weight.
pub fn without_inline_pictures(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut out = String::with_capacity(html.len().min(MAX_HTML));
    let (mut at, mut kept) = (0usize, 0usize);
    while let Some(found) = lower[at..].find("data:") {
        let start = at + found;
        // What opens the address: a quote (`src="data:…"`), or `url(` with or without a quote.
        let before = start.checked_sub(1).map(|i| bytes[i]);
        let end = match before {
            Some(quote @ (b'"' | b'\'')) => lower[start..].find(char::from(quote)).map(|n| start + n),
            Some(b'(') => lower[start..].find(')').map(|n| start + n),
            _ => None,
        };
        match end {
            Some(end) => {
                out.push_str(&html[kept..start]);
                kept = end;
                at = end;
            }
            // Not an address of an attribute or a style, or one that is never closed: left as it is.
            None => at = start + "data:".len(),
        }
    }
    out.push_str(&html[kept..]);
    out
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::{Cell, RefCell};

    use mailcraft_ui_egui::RichClipboard;
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;

    use super::Caught;

    thread_local! {
        static CAUGHT: RefCell<Caught> = RefCell::new(Caught::default());
        static LISTENING: Cell<bool> = const { Cell::new(false) };
    }

    /// The window's paste events are listened to, once: whether they are.
    fn listen() -> bool {
        if LISTENING.with(Cell::get) {
            return true;
        }
        let Some(window) = web_sys::window() else { return false };
        let on_paste = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(|event: web_sys::ClipboardEvent| {
            let html = event.clipboard_data().and_then(|data| data.get_data("text/html").ok());
            let html = html.as_deref().filter(|html| !html.is_empty());
            CAUGHT.with(|caught| caught.borrow_mut().caught(html, js_sys::Date::now()));
        });
        // The capture phase: before eframe's own listener, which takes the text and stops the browser's paste.
        let listening = window.add_event_listener_with_callback_and_bool("paste", on_paste.as_ref().unchecked_ref(), true).is_ok();
        // The page's for as long as it lives.
        on_paste.forget();
        LISTENING.with(|l| l.set(listening));
        listening
    }

    /// The browser's clipboard as the interface asks it: what the last paste event carried as HTML.
    struct Board;

    impl RichClipboard for Board {
        fn put(&self, _html: &str, _text: &str) -> Result<(), String> {
            // The editor copies the text then, as before.
            Err("the browser build copies text".into())
        }

        fn html(&self) -> Option<String> {
            CAUGHT.with(|caught| caught.borrow_mut().take(js_sys::Date::now()))
        }

        fn reads_on_demand(&self) -> bool {
            false
        }
    }

    /// The clipboard of the browser build, once the window's paste events are listened to.
    pub fn board() -> Option<Box<dyn RichClipboard>> {
        listen().then(|| Box::new(Board) as Box<dyn RichClipboard>)
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::board;

#[cfg(test)]
#[path = "paste_tests.rs"]
mod tests;
