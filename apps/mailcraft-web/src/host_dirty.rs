// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A hosting page is told whether the window holds work that closing it would lose
//! ([`mailcraft_ui_egui::MailApp::unsent_work`]), and when that clears: the contract the Craft editors speak with the page
//! that frames them (`<editor>:dirty`), so that one host asks before it closes any of them.
//!
//! The page that frames the browser build on the same origin (Neodesk's window) gets a `postMessage`:
//!
//! ```js
//! { type: "mailcraft:dirty", dirty: true }   // something would be lost now
//! { type: "mailcraft:dirty", dirty: false }  // nothing would any more
//! ```
//!
//! sent to `window.parent` with the page's own origin as the target origin (another origin never hears it), once when
//! the tab of a hosting page starts (`false`, as editors say `ready`) and then each time the answer changes, never twice
//! the same. A page that isn't framed (a tab of its own) and a window that keeps its mailbox (not a hosting page's) send
//! nothing. The host listens on its own window, takes the message only from its frame (`event.source`) and its own
//! origin (`event.origin`), and asks before closing while the last one said `dirty: true`.
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

/// What a message to the page is called.
pub const DIRTY_MESSAGE: &str = "mailcraft:dirty";

/// What was last told to the page, so that it is told each change once.
#[derive(Debug, Default)]
pub struct Told {
    last: Option<bool>,
}

impl Told {
    /// What to tell the page now that the window says `dirty`: `None` when it was told that already.
    pub fn next(&mut self, dirty: bool) -> Option<bool> {
        (self.last != Some(dirty)).then(|| {
            self.last = Some(dirty);
            dirty
        })
    }
}

#[cfg(target_arch = "wasm32")]
mod page {
    use std::cell::RefCell;

    use wasm_bindgen::JsValue;

    use super::{DIRTY_MESSAGE, Told};

    thread_local! {
        static TOLD: RefCell<Told> = RefCell::new(Told::default());
    }

    /// Tell the page that frames this one, on its origin, whether the window holds work that closing it would lose,
    /// when that changed. Nothing for a page that isn't framed.
    pub fn tell(dirty: bool) {
        let Some(now) = TOLD.with(|told| told.borrow_mut().next(dirty)) else { return };
        let Some(window) = web_sys::window() else { return };
        let Ok(Some(parent)) = window.parent() else { return };
        // A top page is its own parent: nobody to tell.
        if js_sys::Object::is(&parent, &window) {
            return;
        }
        let Ok(origin) = window.location().origin() else { return };
        let message = js_sys::Object::new();
        let set = |key: &str, value: JsValue| js_sys::Reflect::set(&message, &JsValue::from_str(key), &value).is_ok();
        if set("type", JsValue::from_str(DIRTY_MESSAGE)) && set("dirty", JsValue::from_bool(now)) && parent.post_message(&message, &origin).is_err() {
            log::warn!("the page that frames this one couldn't be told whether work would be lost");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use page::tell;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_is_told_each_change_once_and_first_that_nothing_would_be_lost() {
        let mut told = Told::default();
        // The first look, whatever it is.
        assert_eq!(told.next(false), Some(false));
        assert_eq!(told.next(false), None, "not twice the same");
        // A message is being written: dirty, once.
        assert_eq!(told.next(true), Some(true));
        assert_eq!(told.next(true), None);
        // Sent or thrown away: clean again, once.
        assert_eq!(told.next(false), Some(false));
        assert_eq!(told.next(false), None);
        // A window that is dirty from its first look says so first.
        assert_eq!(Told::default().next(true), Some(true));
        assert_eq!(DIRTY_MESSAGE, "mailcraft:dirty");
    }
}
