// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Classer dans », where the person puts a message of the Inbox under
// //// Neoffice — another tab (Principal, Notifications, Newsletters) in a hosting page's tab whose server keeps the choice, and
// //// Neoffice — the offer of the same for its sender's next ones.

//! « Classer dans » (the engine's `cmd/stream_correct.rs`): the message goes under the tab chosen at once, alone, and what
//! the person chose teaches the Neoffice server's model (neoffice-maintenance#3142). Then a bar above the list offers the same
//! for its sender's next ones ([`offer_bar`]: « Classé dans Newsletters. Faire de même pour les prochains e-mails de … ? »,
//! as Gmail asks after a move between its tabs): on « Oui », the sender's other messages in the Inbox go with it, and the
//! mailbox's server keeps the choice for that sender, so that the next ones come under it too, here and on the phone. One
//! address may write both a person's mail and automatic notices: a choice made on one message must not carry them all.
//!
//! Where it is reached, in a hosting page's tab whose mailbox's server takes the choice (its capability says so):
//! - « Classer dans » in the message's menu (a right click on a row of the Inbox, `mail_view.rs`);
//! - « Classer dans » in the Home tab's Tags group (`ribbon.rs`), for the messages selected.
//!
//! The tab the selection is under is ticked. A server that refused is told the person ([`frame`]): the messages are put
//! back where they were.

use egui::{CornerRadius, Frame, Margin, Stroke, Ui, vec2};
use mailcraft_engine::cmd::stream_correct;
use mailcraft_model::{Id, Stream};
use serde_json::json;

use crate::MailApp;
use crate::theme::{Tokens, regular, semibold};

/// How long the offer of the same for the senders' next ones stays, unanswered, in seconds.
pub(crate) const OFFER_S: i64 = 30;

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

/// The name of a tab, as the list's header shows it.
fn tab_name(stream: Stream) -> &'static str {
    TABS.iter().find(|(s, _)| *s == stream).map_or("", |(_, name)| tl!(name))
}

/// What the offer says: where the messages went, and the question for their senders' next ones.
pub(crate) fn offer_text(offer: &stream_correct::Proposal) -> String {
    let moved = crate::i18n::fmt(tl!("Moved to {tab}."), &[("tab", tab_name(offer.stream))]);
    let question = match offer.senders.as_slice() {
        [one] => crate::i18n::fmt(tl!("Do the same for the next emails from {sender}?"), &[("sender", one)]),
        many => crate::i18n::fmt(tl!("Do the same for the next emails from these {count} senders?"), &[("count", &many.len().to_string())]),
    };
    format!("{moved} {question}")
}

/// « Classé dans Newsletters. Faire de même pour les prochains e-mails de … ? [Oui] [✕] » above the list, after messages were
/// put under a tab alone, for [`OFFER_S`] seconds: « Oui » puts their senders' other messages there too and has the server
/// keep it for the next ones; ✕ (or the time) leaves the messages alone where they are.
pub(crate) fn offer_bar(app: &mut MailApp, ui: &mut Ui) {
    let Some(offer) = app.session.hosted_stream.proposal.clone() else { return };
    let left = OFFER_S - (app.session.now() - offer.at);
    if left <= 0 || app.session.view.module != mailcraft_engine::Module::Mail {
        if left <= 0 {
            app.session.hosted_stream.proposal = None;
        }
        return;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(left.max(1) as u64));
    let t = Tokens::get(ui.ctx());
    let text = offer_text(&offer);
    let (mut yes, mut close) = (false, false);
    Frame::new()
        .fill(t.surface_sunken)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 6))
        .outer_margin(Margin { left: 12, right: 10, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let shut = crate::widgets::icon_button(ui, "close", tl!("Close"), 14.0, true);
                    app.hit.add("streamOfferClose", 0, shut.rect);
                    close = shut.clicked();
                    let words = egui::RichText::new(tlc!("stream", "Yes")).font(semibold(13.0)).color(t.accent_text);
                    let answer = ui.add(egui::Button::new(words).frame(false).min_size(vec2(40.0, 24.0)));
                    app.hit.add("streamOfferYes", 0, answer.rect);
                    yes = answer.clicked();
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&text).font(regular(13.0)).color(t.text)).truncate()).on_hover_text(&text);
                    });
                });
            });
        });
    if yes {
        let _ = app.run("mail.correctStream", json!({"ids": offer.ids, "stream": offer.stream.code(), "sender": true}));
        app.session.hosted_stream.proposal = None;
    } else if close {
        app.session.hosted_stream.proposal = None;
    }
}

#[cfg(test)]
#[path = "stream_correct_tests.rs"]
mod tests;
