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

/// A hosting page's tab asks before it is left while a change of a note waits (in the browser alone).
#[cfg(target_arch = "wasm32")]
pub mod leave;

use mailcraft_engine::Session;
use mailcraft_model::sync::{GraphFetchResult, JmapMailResult, JmapPimResult};
use mailcraft_ui_egui::jmap_job::{JmapFetchJob, JmapJob};
use mailcraft_ui_egui::jmap_pim::JmapPimJob;
use mailcraft_ui_egui::{MailApp, Services};

/// A mailbox a hosting page handed over (host.rs reads it from the page).
#[derive(Clone, Default)]
pub struct HostAccount {
    pub email: String,
    pub name: String,
    pub session: String,
    /// The name of the server's account for it, when the page says one: Nora names the mailbox by it when its
    /// session's address names none (nora.rs).
    pub account: String,
    pub username: String,
    pub secret: String,
    pub bearer: bool,
    /// Its signature in Neoffice, as the page gives it: its HTML, its text, and a company's logo with its place.
    pub signature: crate::page_signature::PageSignature,
    /// Whether the page says it is shared with the person rather than theirs (`shared: true`): Nora writes into a
    /// message of it only after the person's click on a card of her pane that names it (nora.rs).
    pub shared: bool,
    /// Where the page's site sets this mailbox up, as the page says (`settings`): File › Info's Settings opens it
    /// ([`settings_page`]).
    pub settings: String,
}

impl std::fmt::Debug for HostAccount {
    // Never the secret, nor the address: they go into logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostAccount").field("bearer", &self.bearer).field("signature", &self.signature).finish_non_exhaustive()
    }
}

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

/// The longest address of the page's signatures that is taken, in bytes.
const MAX_SIGNATURES_HOME: usize = 2_048;

/// Whether File › Signatures may link to `address`, where a hosting page says its site writes the signatures of the
/// mailboxes it hands over: an address as a session may be one ([`session_allowed`]), with nothing blank or control in
/// it, [`MAX_SIGNATURES_HOME`] bytes at most.
pub fn signatures_home_allowed(address: &str) -> bool {
    !address.is_empty()
        && address.len() <= MAX_SIGNATURES_HOME
        && !address.chars().any(|c| c.is_whitespace() || c.is_control())
        && session_allowed(address)
}

/// The path of a session on Neoffice's JMAP facade, before its query (frappe_webmail's `host_mailboxes`).
const FACADE: &str = "/api/method/frappe_webmail.mailcraft_jmap.session";
/// Where a Neoffice site lists its Webmail Accounts, each one's form under it.
const WEBMAIL_ACCOUNTS: &str = "/app/webmail-account";

/// Where the page's site sets a mailbox up (its servers, its password, its name on mail): File › Info's Settings of the
/// mailbox opens it, in place of the window's own account settings, which a hosting page's tab loses when it closes.
/// The address the page hands over with it (`settings`), taken as File › Signatures takes the page's; else, for a
/// mailbox on Neoffice's JMAP facade (a page that hands over its session but not this, as Neodesk's), the form of its
/// Webmail Account on the same site. None for a mailbox shared with the person: its owner sets it up.
pub fn settings_page(account: &HostAccount) -> Option<String> {
    if account.shared {
        return None;
    }
    let handed = account.settings.trim();
    if signatures_home_allowed(handed) {
        return Some(handed.to_string());
    }
    let (site, name) = facade(&account.session)?;
    Some(format!("{site}{WEBMAIL_ACCOUNTS}/{name}")).filter(|page| signatures_home_allowed(page))
}

/// Where the page's site lists the mailboxes it hands over, to add one or change one: File › Info links there. The
/// address the page gives (`accounts_home`), else the Webmail Accounts of the site of the first mailbox on Neoffice's
/// facade.
pub fn accounts_home(handed: Option<String>, accounts: &[HostAccount]) -> Option<String> {
    if let Some(page) = handed.filter(|page| signatures_home_allowed(page.trim())) {
        return Some(page.trim().to_string());
    }
    accounts.iter().find_map(|a| facade(&a.session)).map(|(site, _)| format!("{site}{WEBMAIL_ACCOUNTS}")).filter(|page| signatures_home_allowed(page))
}

/// (its site, the name of its Webmail Account as the address writes it) of a session on Neoffice's JMAP facade:
/// `https://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session?account=jane%40example.org`. None for any
/// other address, a site with a path, and a name with anything but unreserved characters and escapes.
fn facade(session: &str) -> Option<(&str, &str)> {
    let (site, rest) = session.trim().split_once(FACADE)?;
    let query = rest.strip_prefix('?')?;
    let name = query.split('&').find_map(|pair| pair.strip_prefix("account="))?;
    let host = &site[site.find("://")? + 3..];
    let plain = |b: u8| b.is_ascii_alphanumeric() || b"%-._~".contains(&b);
    (!host.is_empty() && !host.contains('/') && !name.is_empty() && name.bytes().all(plain)).then_some((site, name))
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

    #[test]
    fn the_signatures_page_named_by_a_hosting_page_is_an_https_address_or_a_loopback_one() {
        for good in ["https://erp.example.org/app/webmail?signatures=1", "http://127.0.0.1:8771/signatures.html"] {
            assert!(signatures_home_allowed(good), "{good}");
        }
        let long = format!("https://erp.example.org/{}", "a".repeat(MAX_SIGNATURES_HOME));
        for bad in [
            "",
            "http://erp.example.org/app/webmail",
            "javascript:alert(1)",
            "data:text/html,<p>x</p>",
            "/app/webmail?signatures=1",
            "https://erp.example.org/a b",
            "https://erp.example.org/a\nb",
            long.as_str(),
        ] {
            assert!(!signatures_home_allowed(bad), "{bad:?}");
        }
    }

    const FACADE_SESSION: &str = "https://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session?account=jane%40example.org";
    const JANE_S_FORM: &str = "https://erp.example.org/app/webmail-account/jane%40example.org";

    fn mailbox(session: &str, settings: &str, shared: bool) -> HostAccount {
        HostAccount { email: "jane@example.org".into(), session: session.into(), settings: settings.into(), shared, ..HostAccount::default() }
    }

    #[test]
    fn a_mailbox_is_set_up_where_the_page_says_else_in_its_webmail_account_on_neoffice_s_facade() {
        let given = "https://erp.example.org/app/webmail-account/other";
        assert_eq!(settings_page(&mailbox(FACADE_SESSION, given, false)).as_deref(), Some(given));
        // A page that hands over the session alone (Neodesk's): the form of its Webmail Account on the same site.
        assert_eq!(settings_page(&mailbox(FACADE_SESSION, "", false)).as_deref(), Some(JANE_S_FORM));
        // An address the page may not name is not taken; the session's site is.
        assert_eq!(settings_page(&mailbox(FACADE_SESSION, "javascript:alert(1)", false)).as_deref(), Some(JANE_S_FORM));
        // A mailbox shared with the person: its owner sets it up.
        assert_eq!(settings_page(&mailbox(FACADE_SESSION, given, true)), None);
        // Another JMAP server, or an address of the facade that names no plain account on a plain site: nothing.
        for session in [
            "https://mail.example.org/jmap/session",
            "https://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session",
            "https://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session?account=",
            "https://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session?account=a/../../b",
            "https://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session?account=a\"b",
            "https://erp.example.org/sub/api/method/frappe_webmail.mailcraft_jmap.session?account=jane",
            "http://erp.example.org/api/method/frappe_webmail.mailcraft_jmap.session?account=jane",
        ] {
            assert_eq!(settings_page(&mailbox(session, "", false)), None, "{session}");
        }
    }

    #[test]
    fn the_mailboxes_are_listed_where_the_page_says_else_in_the_webmail_accounts_of_their_site() {
        let given = "https://erp.example.org/app/webmail-account";
        assert_eq!(accounts_home(Some(given.into()), &[]).as_deref(), Some(given));
        let boxes = [mailbox("https://mail.example.org/jmap/session", "", false), mailbox(FACADE_SESSION, "", true)];
        assert_eq!(accounts_home(None, &boxes).as_deref(), Some(given));
        assert_eq!(accounts_home(Some("ftp://erp.example.org/".into()), &boxes[..1]), None);
    }
}
