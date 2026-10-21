// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a hosting page that draws MailCraft's navigation in its own
// menu is told where the window is, and drives it (host.rs, `chrome: "menu"`).

//! The messages between the window and a hosting page that draws its navigation in its own menu (host.rs documents
//! them; what they say is `mailcraft_ui_egui::host_menu`'s): `mailcraft:ready` once, `mailcraft:state` after it and at
//! each change, at most every [`EVERY_MS`] and never twice the same, and `mailcraft:command-done` for each
//! `mailcraft:command` of the page's. Everything goes to `window.parent` on this page's own origin, and a command is
//! taken only from `window.parent` on that origin ([`accepted`]); nothing when the page is not framed.
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::Value;

/// How often at most the page is told where the window is, in milliseconds.
pub const EVERY_MS: f64 = 250.0;
/// How many of the page's messages wait for the next frame at most: more are dropped unanswered.
pub const MAX_WAITING: usize = 64;

/// The version the page is told in `mailcraft:ready`: this build's, and the commit it was built from when the build
/// knew it (`GITHUB_SHA`, which the web build's workflow has), as `0.5.2+8b7972c`.
pub fn version() -> String {
    stamp(env!("CARGO_PKG_VERSION"), option_env!("GITHUB_SHA"))
}

/// `version`, with the first seven characters of `sha` when it is a commit's (hexadecimal, seven characters at least).
fn stamp(version: &str, sha: Option<&str>) -> String {
    match sha.map(str::trim).filter(|sha| sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit())).and_then(|sha| sha.get(..7)) {
        Some(short) => format!("{version}+{}", short.to_ascii_lowercase()),
        None => version.to_string(),
    }
}

/// Whether a message is the page's to take: this page is framed, the message comes from the page that frames it
/// (`event.source === window.parent`), and from this page's own origin (`event.origin === location.origin`), which is
/// a real one (an opaque origin, `"null"`, is every sandboxed page's).
pub fn accepted(framed: bool, from_parent: bool, origin: &str, own: &str) -> bool {
    framed && from_parent && !own.is_empty() && own != "null" && origin == own
}

/// What the page was last told of where the window is, and when the window last looked.
#[derive(Debug, Default)]
pub struct Teller {
    last: Option<Value>,
    looked: Option<f64>,
}

impl Teller {
    /// How long the window waits before it looks again at `now`, in milliseconds: `None` when it may look now (it
    /// never has, [`EVERY_MS`] have gone by, or the clock went back).
    pub fn wait(&self, now: f64) -> Option<f64> {
        let since = now - self.looked?;
        (0.0..EVERY_MS).contains(&since).then_some(EVERY_MS - since)
    }

    /// What to tell the page of `state`, looked at `now`: `None` when it was told that already.
    pub fn next(&mut self, now: f64, state: Value) -> Option<Value> {
        self.looked = Some(now);
        if self.last.as_ref() == Some(&state) {
            return None;
        }
        self.last = Some(state.clone());
        Some(state)
    }
}

#[cfg(target_arch = "wasm32")]
mod page {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use mailcraft_ui_egui::MailApp;
    use mailcraft_ui_egui::host_menu;
    use serde_json::Value;
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;

    use super::{MAX_WAITING, Teller, accepted, version};

    thread_local! {
        /// The page's messages (their JSON) that wait for the next frame.
        static WAITING: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
        /// Whether the page was told the window started, and what it was told since.
        static TOLD: RefCell<(bool, Teller)> = RefCell::new((false, Teller::default()));
    }

    /// The page that frames this one and this page's origin: `None` when it is not framed.
    fn parent() -> Option<(web_sys::Window, String)> {
        let window = web_sys::window()?;
        let parent = window.parent().ok()??;
        // A top page is its own parent: nobody to tell.
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        Some((parent, window.location().origin().ok()?))
    }

    /// Take the page's messages, for as long as this page lives: each one that is the page's ([`accepted`]) waits
    /// for the next frame, which is asked for.
    pub fn listen(ctx: egui::Context) {
        let Some(window) = web_sys::window() else { return };
        let heard = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let framed = parent();
            let from_parent = framed.as_ref().zip(event.source()).is_some_and(|((parent, _), source)| js_sys::Object::is(&source, parent));
            let own = framed.as_ref().map(|(_, origin)| origin.as_str()).unwrap_or_default();
            if !accepted(framed.is_some(), from_parent, &event.origin(), own) {
                return;
            }
            // Read as JSON (what can't be, a cycle, is no command), the rest in the window's own time.
            let Some(text) = js_sys::JSON::stringify(&event.data()).ok().and_then(|text| text.as_string()) else { return };
            let kept = WAITING.with(|waiting| {
                let mut waiting = waiting.borrow_mut();
                let room = waiting.len() < MAX_WAITING;
                if room {
                    waiting.push_back(text);
                }
                room
            });
            if kept {
                ctx.request_repaint();
            } else {
                log::warn!("a message of the page that frames this one was dropped: too many wait");
            }
        });
        if window.add_event_listener_with_callback("message", heard.as_ref().unchecked_ref()).is_ok() {
            // The page's for as long as it lives.
            heard.forget();
        } else {
            log::warn!("the messages of the page that frames this one can't be heard");
        }
    }

    /// Post `message` to the page that frames this one, on this page's origin.
    fn post(message: &Value) {
        let Some((parent, origin)) = parent() else { return };
        let Ok(message) = js_sys::JSON::parse(&message.to_string()) else { return };
        if parent.post_message(&message, &origin).is_err() {
            log::warn!("the page that frames this one couldn't be told where the window is");
        }
    }

    /// Once a frame: tell the page the window has started (once), answer its commands, and tell it where the window
    /// is when that changed, at most every [`super::EVERY_MS`].
    pub fn frame(app: &mut MailApp, ctx: &egui::Context) {
        if parent().is_none() {
            return;
        }
        if TOLD.with(|told| !std::mem::replace(&mut told.borrow_mut().0, true)) {
            post(&host_menu::ready(&version()));
        }
        while let Some(text) = WAITING.with(|waiting| waiting.borrow_mut().pop_front()) {
            if let Some(done) = host_menu::answer(app, &text) {
                post(&done);
            }
        }
        let now = mailcraft_ui_egui::now_ms();
        if let Some(wait) = TOLD.with(|told| told.borrow().1.wait(now)) {
            // Looked at a moment ago: once more when it may, for what changed since.
            ctx.request_repaint_after(std::time::Duration::from_millis(wait.ceil().clamp(1.0, super::EVERY_MS) as u64));
            return;
        }
        let Some(state) = host_menu::state(app) else { return };
        if let Some(state) = TOLD.with(|told| told.borrow_mut().1.next(now, state)) {
            post(&state);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use page::{frame, listen};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_version_names_the_commit_when_the_build_knows_it() {
        assert_eq!(stamp("0.5.2", Some("8b7972ce311862a8994ff4434fc37b4c7703e65f")), "0.5.2+8b7972c");
        assert_eq!(stamp("0.5.2", Some("8B7972CE31")), "0.5.2+8b7972c");
        for unknown in [None, Some(""), Some("8b797"), Some("not a commit"), Some("8b7972ce\"><script>")] {
            assert_eq!(stamp("0.5.2", unknown), "0.5.2", "{unknown:?}");
        }
        assert!(version().starts_with(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn only_the_framing_page_on_this_page_s_origin_is_heard() {
        let own = "https://erp.example.org";
        assert!(accepted(true, true, own, own));
        // Another origin, another window (a frame of the page's, a window it opened), a page that isn't framed.
        assert!(!accepted(true, true, "https://evil.example", own));
        assert!(!accepted(true, true, "https://erp.example.org:8443", own));
        assert!(!accepted(true, false, own, own));
        assert!(!accepted(false, true, own, own));
        // A sandboxed page's origin is everybody's.
        assert!(!accepted(true, true, "null", "null"));
        assert!(!accepted(true, true, "", ""));
    }

    #[test]
    fn the_page_is_told_at_most_every_quarter_second_and_never_twice_the_same() {
        let mut teller = Teller::default();
        let a = json!({"type": "mailcraft:state", "module": "mail"});
        let b = json!({"type": "mailcraft:state", "module": "calendar"});
        // The first look, right away.
        assert_eq!(teller.wait(1_000.0), None);
        assert_eq!(teller.next(1_000.0, a.clone()), Some(a.clone()));
        // A frame soon after waits for the rest of the quarter second.
        assert_eq!(teller.wait(1_100.0), Some(150.0));
        assert_eq!(teller.wait(1_249.0), Some(1.0));
        assert_eq!(teller.wait(1_250.0), None);
        // The same again: nothing, but the look counts.
        assert_eq!(teller.next(1_250.0, a.clone()), None);
        assert_eq!(teller.wait(1_300.0), Some(200.0));
        // A change: told once.
        assert_eq!(teller.next(1_500.0, b.clone()), Some(b.clone()));
        assert_eq!(teller.next(1_750.0, b.clone()), None);
        // Back as it was: told again.
        assert_eq!(teller.next(2_000.0, a.clone()), Some(a));
        // A clock that went back looks now.
        assert_eq!(teller.wait(10.0), None);
    }

    /// A hosting page's window with the mailbox it handed over (a JMAP account), two unread messages in its Inbox.
    fn hosted_window() -> mailcraft_ui_egui::MailApp {
        use mailcraft_engine::cmd::jmap_mail::apply_jmap_mail;
        use mailcraft_model::sync::{FolderListing, GraphFolder, GraphMessage, JmapMailResult, MessagePage, ServerMessage};
        use mailcraft_model::{FolderKind, Mailbox};
        use mailcraft_ui_egui::host_menu::{Handed, Menu};

        let session = mailcraft_engine::Session::at(Mailbox::with_local_account("", ""), 1_791_993_600, 0);
        let mut app = mailcraft_ui_egui::MailApp::new(session, mailcraft_ui_egui::Services::default());
        app.autosave = false;
        app.auto_sync = false;
        app.services.jmap_host_credentials = true;
        let email = "ada@example.org";
        let added =
            app.execute("account.add", json!({"email": email, "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap", "bearer": true}}));
        let account = added.unwrap()["id"].as_u64().unwrap();
        let folders = vec![GraphFolder { id: "Min".into(), name: "INBOX".into(), kind: FolderKind::Inbox, ..Default::default() }];
        let listing = FolderListing { folders, complete: true };
        apply_jmap_mail(&mut app.session, JmapMailResult { account, email: email.into(), listing: Some(listing), ..Default::default() }).unwrap();
        let mail = |id: &str| GraphMessage {
            server: ServerMessage { id: id.into(), folder: "Min".into(), ..Default::default() },
            subject: "Hello".into(),
            received: 1_791_990_000,
            ..Default::default()
        };
        let page = MessagePage { folder: "Min".into(), messages: vec![mail("E1"), mail("E2")], afresh: true, ..Default::default() };
        apply_jmap_mail(&mut app.session, JmapMailResult { account, email: email.into(), pages: vec![page], whole: true, ..Default::default() })
            .unwrap();
        app.services.host_menu = Some(Menu { mailboxes: vec![Handed { email: email.into(), name: "Ada".into() }] });
        app.execute("folder.select", json!({"kind": "inbox", "account": account})).unwrap();
        app
    }

    #[test]
    fn the_state_of_a_window_is_posted_again_only_when_it_changed() {
        use mailcraft_ui_egui::host_menu::{answer, state};

        let mut app = hosted_window();
        let mut teller = Teller::default();
        let first = state(&app).unwrap();
        assert_eq!(first["accounts"][0]["unread"], 2);
        assert!(teller.next(0.0, first).is_some(), "once after ready");
        // Frames go by and nothing changes: nothing is posted.
        for at in [250.0, 500.0, 750.0] {
            assert!(teller.next(at, state(&app).unwrap()).is_none());
        }
        // A message read here: the Inbox's count changes, and the page is told once.
        let read = app.session.mailbox.messages.keys().next().copied().unwrap();
        app.execute("mail.markRead", json!({"ids": [read]})).unwrap();
        let changed = teller.next(1_000.0, state(&app).unwrap()).unwrap();
        assert_eq!(changed["accounts"][0]["unread"], 1);
        assert!(teller.next(1_250.0, state(&app).unwrap()).is_none());
        // The page brings Calendar to the front: answered, and told once where the window is now.
        let done = answer(&mut app, &json!({"type": "mailcraft:command", "id": 1, "name": "view.calendar", "params": {}}).to_string());
        assert_eq!(done, Some(json!({"type": "mailcraft:command-done", "id": 1, "ok": true})));
        assert_eq!(teller.next(1_500.0, state(&app).unwrap()).unwrap()["module"], "calendar");
        assert!(teller.next(1_750.0, state(&app).unwrap()).is_none());
        assert!(!state(&app).unwrap().to_string().contains("Hello"), "nothing of a message");
    }
}
