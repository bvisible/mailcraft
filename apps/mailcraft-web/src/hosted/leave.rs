// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A hosting page's tab keeps nothing between visits: a change of a note that waits to go to its server, or words typed
//! into a note and not in it yet, would be lost with the page. While there is one
//! ([`mailcraft_ui_egui::MailApp::notes_unsent`], looked at every frame), leaving the page asks first: the browser's
//! own question (its words are the browser's; a page can't give its own).

use std::cell::Cell;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;

thread_local! {
    /// Whether something would be lost with the page now.
    static UNSENT: Cell<bool> = const { Cell::new(false) };
}

/// Say whether something would be lost with the page now.
pub fn set(unsent: bool) {
    UNSENT.with(|flag| flag.set(unsent));
}

/// Ask before the page is left while something would be lost with it: once, for as long as the page lives.
pub fn install() {
    let Some(window) = web_sys::window() else { return };
    let ask = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(|event: web_sys::BeforeUnloadEvent| {
        if UNSENT.with(Cell::get) {
            event.prevent_default();
            // (What older browsers look for.)
            event.set_return_value("");
        }
    });
    if window.add_event_listener_with_callback("beforeunload", ask.as_ref().unchecked_ref()).is_ok() {
        // The page's for as long as it lives.
        ask.forget();
    } else {
        log::warn!("the page couldn't be made to ask before it is left");
    }
}
