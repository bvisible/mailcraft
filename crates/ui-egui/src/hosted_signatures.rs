// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): File › Signatures in a tab of a hosting page.

//! File › Signatures in a tab of a hosting page ([`mailcraft_engine::Session::keeps_local`] false): the mailboxes it
//! handed over came with the signatures of its site, and the tab keeps nothing between visits, so a change made here
//! is lost when the tab closes. In place of the line that says the signatures are kept on this computer, the page says
//! so, and links to where its site writes them ([`crate::Services::signatures_home`]), in a new tab: the window's own
//! tab stays where it is.

use egui::Ui;

use crate::MailApp;
use crate::theme::{Tokens, regular};

pub(crate) fn note(app: &MailApp, ui: &mut Ui, t: &Tokens) {
    ui.label(
        egui::RichText::new(tl!("These mailboxes' signatures come from the site that opened them: a change made here is lost when this tab closes."))
            .font(regular(11.5))
            .color(t.text_dim),
    );
    if let Some(home) = &app.services.signatures_home {
        ui.add(egui::Hyperlink::from_label_and_url(tl!("Change them on the site"), home).open_in_new_tab(true));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use egui_kittest::kittest::Queryable as _;
    use mailcraft_engine::Session;
    use serde_json::json;

    use crate::{MailApp, Services};

    /// Wednesday 2026-10-14 16:00 UTC.
    const NOW: i64 = 1_791_993_600;
    const HOME: &str = "https://erp.example.org/app/webmail?signatures=1";
    const LINK: &str = "Change them on the site";
    const HOSTED: &str = "lost when this tab closes";
    const KEPT: &str = "Signatures are kept on this computer";

    /// File › Signatures of a window whose session keeps its mailbox or not, given where the page's site writes them.
    fn page(keeps_local: bool, home: Option<&str>) -> egui_kittest::Harness<'static, MailApp> {
        let mut session = Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0);
        session.keeps_local = keeps_local;
        let services = Services { signatures_home: home.map(str::to_string), ..Services::default() };
        let mut app = MailApp::new(session, services);
        app.autosave = false;
        let mut harness = egui_kittest::Harness::builder()
            .with_accessibility_check(false)
            .allow_missing_glyphs()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_ui_state(
                |ui, app: &mut MailApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                app,
            );
        harness.state_mut().execute("ui.dismissReminder", json!({"all": true})).unwrap();
        harness.state_mut().execute("ui.backstage", json!({"value": true, "page": "signatures"})).unwrap();
        harness.run_steps(3);
        harness
    }

    #[test]
    fn a_hosting_pages_tab_says_its_signatures_are_its_sites_and_opens_them_there_in_a_new_tab() {
        let mut harness = page(false, Some(HOME));
        assert!(harness.query_by_label_contains(HOSTED).is_some());
        assert!(harness.query_by_label_contains(KEPT).is_none(), "nothing is kept on this computer");
        harness.get_by_label(LINK).click();
        let mut asked = Vec::new();
        for _ in 0..3 {
            harness.step();
            asked.extend(harness.output().platform_output.commands.iter().cloned());
        }
        assert!(matches!(asked.as_slice(), [egui::OutputCommand::OpenUrl(open)] if open.url == HOME && open.new_tab), "{asked:?}");
    }

    #[test]
    fn without_an_address_the_tab_still_says_so_and_offers_no_link() {
        let harness = page(false, None);
        assert!(harness.query_by_label_contains(HOSTED).is_some());
        assert!(harness.query_by_label(LINK).is_none());
    }

    #[test]
    fn a_window_that_keeps_its_mailbox_keeps_its_own_words_and_no_link() {
        let harness = page(true, Some(HOME));
        assert!(harness.query_by_label_contains(KEPT).is_some());
        assert!(harness.query_by_label_contains(HOSTED).is_none());
        assert!(harness.query_by_label(LINK).is_none());
    }
}
