// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what makes a browser tab a hosting page's, apart from the page so
// that it is tested natively.

//! What makes a browser tab a hosting page's (host.rs, web.rs): its mailboxes are read by runners that take the page's
//! credentials, their calendars and address books by a job beside their mail ([`mailcraft_ui_egui::Services::jmap_pim`]), and
//! nothing of them is kept between visits: no mailbox in the browser's storage, no autosave, and a session that keeps
//! nothing of its own ([`mailcraft_engine::Session::keeps_local`]).
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::sync::mpsc::Receiver;

use mailcraft_engine::Session;
use mailcraft_model::sync::{GraphFetchResult, JmapMailResult, JmapPimResult};
use mailcraft_ui_egui::jmap_job::{JmapFetchJob, JmapJob};
use mailcraft_ui_egui::jmap_pim::JmapPimJob;
use mailcraft_ui_egui::{MailApp, Services};

/// The runners of a hosting page's mailboxes, made with the page's credentials and the page's one gate.
pub struct Runners {
    pub jmap: Box<dyn Fn(JmapJob) -> Receiver<JmapMailResult>>,
    pub jmap_fetch: Box<dyn Fn(JmapFetchJob) -> Receiver<GraphFetchResult>>,
    pub jmap_pim: Box<dyn Fn(JmapPimJob) -> Receiver<JmapPimResult>>,
}

/// The window of a hosting page over `session` (its mailboxes set up): `services` with the page's runners in place of
/// the tab's own and nothing kept between visits.
pub fn hosted_window(mut session: Session, mut services: Services, runners: Runners) -> MailApp {
    services.persist = None;
    services.jmap = Some(runners.jmap);
    services.jmap_fetch = Some(runners.jmap_fetch);
    services.jmap_pim = Some(runners.jmap_pim);
    services.jmap_host_credentials = true;
    session.keeps_local = false;
    let mut app = MailApp::new(session, services);
    app.autosave = false;
    app
}

/// Whether a hosting page may hand over a mailbox whose session is at `session`: an `https` address, or plain `http`
/// to this computer alone (the workshop's stand-in server on loopback), as the JMAP client itself allows.
pub fn session_allowed(session: &str) -> bool {
    let session = session.trim();
    session.starts_with("https://") || mailcraft_jmap::Url::parse(session).is_ok_and(|url| !url.https && url.is_loopback())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use mailcraft_model::Mailbox;

    use super::*;

    fn runners() -> Runners {
        Runners { jmap: Box::new(|_| mpsc::channel().1), jmap_fetch: Box::new(|_| mpsc::channel().1), jmap_pim: Box::new(|_| mpsc::channel().1) }
    }

    #[test]
    fn the_hosted_app_offers_the_pim_service_and_keeps_nothing_local() {
        let tab_own = Services { persist: Some(Box::new(|_| Ok(()))), ..Default::default() };
        let app = hosted_window(Session::new(Mailbox::with_local_account("", "")), tab_own, runners());
        assert!(app.services.jmap_pim.is_some() && app.services.jmap.is_some() && app.services.jmap_fetch.is_some());
        assert!(app.services.jmap_host_credentials);
        assert!(!app.session.keeps_local, "nothing is kept between visits");
        assert!(app.services.persist.is_none() && !app.autosave);
    }

    #[test]
    fn an_app_that_is_not_hosted_offers_none_and_keeps_local() {
        // The tab's own services (web.rs `services`) give no calendar job, and its session keeps what it makes.
        let app = MailApp::new(Session::new(Mailbox::with_local_account("", "")), Services::default());
        assert!(app.services.jmap_pim.is_none());
        assert!(app.session.keeps_local);
    }

    #[test]
    fn a_hosting_page_hands_over_https_sessions_and_loopback_ones_only() {
        for good in [
            "https://mail.example.org/jmap/session",
            "http://127.0.0.1:8790/jmap/session",
            "http://localhost:8790/jmap/session",
            "http://[::1]:8790/x",
        ] {
            assert!(session_allowed(good), "{good}");
        }
        for bad in ["http://mail.example.org/jmap/session", "http://10.0.0.1/jmap", "ftp://127.0.0.1/", "", "127.0.0.1:8790", "javascript:alert(1)"] {
            assert!(!session_allowed(bad), "{bad}");
        }
    }
}
