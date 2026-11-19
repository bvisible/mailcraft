// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Classer dans », where the person puts a message of the Inbox under
// //// Neoffice — another tab (Principal, Notifications, Newsletters) in a hosting page's tab whose server keeps the choice.

//! « Classer dans » (the engine's `cmd/stream_correct.rs`): the message goes under the tab chosen at once, and its sender's
//! other messages in the Inbox with it; the mailbox's server keeps the choice for that sender, so that the next ones come
//! under it too, here and on the phone, and what the person chose teaches the Neoffice server's model
//! (neoffice-maintenance#3142).
//!
//! Where it is reached, in a hosting page's tab whose mailbox's server takes the choice (its capability says so):
//! - « Classer dans » in the message's menu (a right click on a row of the Inbox, `mail_view.rs`);
//! - « Classer dans » in the Home tab's Tags group (`ribbon.rs`), for the messages selected.
//!
//! The tab the selection is under is ticked. A server that refused is told the person ([`frame`]): the messages are put
//! back where they were.

use egui::Ui;
use mailcraft_engine::cmd::stream_correct;
use mailcraft_model::{Id, Stream};
use serde_json::json;

use crate::MailApp;

/// The tabs, in the order the list's header shows them, with their names (English; shown through `tl!`, as the header does).
pub(crate) const TABS: [(Stream, &str); 3] = [(Stream::Person, "Main"), (Stream::Notification, "Notifications"), (Stream::Newsletter, "Newsletters")];

/// Whether « Classer dans » is offered for `ids` now.
pub(crate) fn offered(app: &MailApp, ids: &[Id]) -> bool {
    stream_correct::offered(&app.session, ids)
}

/// The tabs as menu entries for `ids`, the one they are under ticked.
pub(crate) fn menu(ui: &mut Ui, app: &mut MailApp, ids: &[Id]) {
    let current = stream_correct::current(&app.session, ids);
    for (stream, name) in TABS {
        crate::widgets::mi_check(ui, app, name, current == Some(stream), "mail.correctStream", json!({"ids": ids, "stream": stream.code()}));
    }
}

/// « Classer dans » in the message's menu, when it is offered for the messages selected.
pub(crate) fn context_menu(app: &mut MailApp, ui: &mut Ui) {
    let ids = app.session.sel.messages.clone();
    if !offered(app, &ids) {
        return;
    }
    ui.menu_button(tlc!("stream", "Move to Tab"), |ui| menu(ui, app, &ids));
}

/// « Classer dans » in the Home tab's Tags group, where some mailbox of this tab takes the choice.
pub(crate) fn ribbon(app: &MailApp, tab: &mut crate::ribbon::Tab) {
    if app.ui.tab == "Message" || app.session.view.module != mailcraft_engine::Module::Mail || !stream_correct::available(&app.session) {
        return;
    }
    let Some(tags) = tab.groups.iter_mut().find(|g| g.title == "Tags") else { return };
    // After « Plus tard » when it is there, else after Follow Up.
    let at = tags
        .items
        .iter()
        .position(|i| i.key == "snooze")
        .or_else(|| tags.items.iter().position(|i| i.key == "followUp"))
        .map_or(tags.items.len(), |n| n + 1);
    tags.items.insert(
        at,
        crate::ribbon::Item {
            key: "moveToTab",
            icon: "inbox",
            label: "Move to\nTab",
            tip: "Put the messages under another tab of the Inbox: their sender's next ones will come under it too",
            command: None,
            menu: Some(crate::ribbon::Menu::StreamTab),
            size: crate::ribbon::Size::Big,
            checked: false,
            section: 3,
            rank: 3,
            plain: true,
            under: None,
            pending: false,
        },
    );
}

/// Why the ribbon's « Classer dans » is greyed for the messages selected, or none.
pub(crate) fn refusal(app: &MailApp) -> Option<&'static str> {
    stream_correct::refusal(&app.session, &app.session.sel.messages)
}

/// What the person is to be told of a server's refusal: every frame of the window.
pub(crate) fn frame(app: &mut MailApp) {
    let said = std::mem::take(&mut app.session.hosted_stream.said);
    if !said.is_empty() && app.refused.is_none() {
        app.refused = Some(said.join("\n"));
    }
}

#[cfg(test)]
#[path = "stream_correct_tests.rs"]
mod tests;
