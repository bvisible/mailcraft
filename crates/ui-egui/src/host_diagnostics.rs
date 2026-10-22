// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): Copy Diagnostics is not offered in a hosting page's tab.

//! Copy Diagnostics (`ui.copyDiagnostics`) is what a tester pastes into a problem report on MailCraft's own repository.
//! In a tab whose mailboxes a hosting page handed over (Neoffice's desk at `/app/neomail`, Neodesk's NeoMail window),
//! the people reading their mail have no use for it, and the browser keeps no log to copy: neither the title bar's pill
//! (chrome.rs) nor the button of the Help tab ([`ribbon`]) is drawn there ([`offered`]). The command itself stays, as
//! upstream has it, and Search Commands never listed it.

use crate::MailApp;
use crate::ribbon::Tab;

/// The window's command the pill and the button run.
pub const COMMAND: &str = "ui.copyDiagnostics";

/// Whether the window offers Copy Diagnostics: everywhere but a tab whose mailboxes a hosting page handed over (the page
/// holds their credentials, [`crate::Services::jmap_host_credentials`]).
pub(crate) fn offered(app: &MailApp) -> bool {
    !app.services.jmap_host_credentials
}

/// The Help tab's Copy Diagnostics taken out of `tab` where the window doesn't offer it.
pub(crate) fn ribbon(app: &MailApp, tab: &mut Tab) {
    if offered(app) {
        return;
    }
    for group in &mut tab.groups {
        group.items.retain(|item| item.command.as_ref().is_none_or(|(id, _)| *id != COMMAND));
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable as _;
    use mailcraft_engine::Session;
    use serde_json::json;

    use super::*;
    use crate::Services;

    /// Wednesday 2026-10-14 16:00 UTC.
    const NOW: i64 = 1_791_993_600;

    /// The sample mailbox in upstream's own window, or (`hosted`) in a tab whose mailboxes a hosting page handed over,
    /// as `apps/mailcraft-web/src/hosted.rs` makes it.
    fn app(hosted: bool) -> MailApp {
        let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
        app.autosave = false;
        app.auto_sync = false;
        app.services.jmap_host_credentials = hosted;
        app
    }

    /// The whole window, laid out, on the ribbon's `tab`.
    fn window(mut app: MailApp, tab: &str) -> egui_kittest::Harness<'static, MailApp> {
        app.ui.tab = tab.to_string();
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
            |ui, app: &mut MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        harness.run_steps(3);
        harness
    }

    /// The commands of the Help tab, as the window stands.
    fn help(mut app: MailApp) -> Vec<&'static str> {
        app.ui.tab = "Help".to_string();
        crate::ribbon::describe(&app).items().filter_map(|item| item.command.as_ref().map(|(id, _)| *id)).collect()
    }

    #[test]
    fn a_hosting_pages_tab_draws_no_copy_diagnostics_pill() {
        let harness = window(app(true), "Home");
        assert_eq!(harness.query_all_by_label_contains("Diagnostics").count(), 0);
    }

    #[test]
    fn upstreams_window_keeps_its_copy_diagnostics_pill() {
        let harness = window(app(false), "Home");
        assert!(harness.query_by_label("Copy Diagnostics").is_some());
    }

    #[test]
    fn the_help_tab_offers_copy_diagnostics_only_outside_a_hosting_page() {
        assert!(help(app(false)).contains(&COMMAND));
        let hosted = help(app(true));
        assert!(!hosted.contains(&COMMAND), "{hosted:?}");
        // Search Commands and About MailCraft stay.
        assert_eq!(hosted.iter().filter(|id| **id == "ui.dialog").count(), 2, "{hosted:?}");
        // Drawn on the Help tab: nothing that names it in a hosting page's tab (the ribbon's buttons are painted
        // without a label of their own, so upstream's window shows the pill alone here; `describe` above has the button).
        assert_eq!(window(app(false), "Help").query_all_by_label_contains("Diagnostics").count(), 1);
        assert_eq!(window(app(true), "Help").query_all_by_label_contains("Diagnostics").count(), 0);
    }

    #[test]
    fn the_command_itself_stays_in_a_hosting_pages_tab() {
        let mut hosted = app(true);
        let copied = hosted.execute(COMMAND, json!({})).unwrap();
        assert!(copied["text"].as_str().unwrap().contains(env!("CARGO_PKG_VERSION")));
    }
}
