// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): File › Info in a tab of a hosting page.

//! File › Info in a tab of a hosting page ([`mailcraft_engine::Session::keeps_local`] false): the mailboxes it handed
//! over are set up on its site (their servers, their password, their name on mail), and the tab keeps nothing between
//! visits, so what the window's own account settings change there is lost when it closes. A mailbox's Settings opens
//! its page on the site instead ([`crate::Services::account_settings`]), and File › Info says where they are all listed
//! ([`crate::Services::accounts_home`]) in place of Add Account, whose account would be lost too. Both in a new tab:
//! the window's own tab stays where it is (in Neodesk's window, the person's browser opens them).

use egui::Ui;

use crate::MailApp;
use crate::theme::{Tokens, regular};

/// The page on the hosting page's site where `account` is set up, when the page says one; None in a window that keeps
/// its mailboxes, where the account's settings are the window's own.
pub(crate) fn settings_page(app: &MailApp, account: &mailcraft_model::Account) -> Option<String> {
    if app.session.keeps_local || account.email.is_empty() {
        return None;
    }
    app.services.account_settings.get(&account.email.to_lowercase()).cloned()
}

/// Opens a page of the site in a new tab.
pub(crate) fn open(ui: &Ui, page: &str) {
    ui.ctx().open_url(egui::OpenUrl::new_tab(page));
}

/// In place of Add Account, in a tab of a hosting page whose site lists its mailboxes: that they come from there, and
/// the link. `false` everywhere else, where File › Info keeps its Add Account.
pub(crate) fn note(app: &MailApp, ui: &mut Ui, t: &Tokens) -> bool {
    let Some(home) = app.services.accounts_home.as_ref().filter(|_| !app.session.keeps_local) else {
        return false;
    };
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!("These mailboxes come from the site that opened them: add one, or change its servers and password, there."))
            .font(regular(11.5))
            .color(t.text_dim),
    );
    ui.add(egui::Hyperlink::from_label_and_url(tl!("Manage mailboxes on the site"), home).open_in_new_tab(true));
    true
}

#[cfg(test)]
mod tests;
