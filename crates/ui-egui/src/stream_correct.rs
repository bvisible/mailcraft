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

use egui::{CornerRadius, Frame, Margin, Stroke, Ui};
use mailcraft_engine::cmd::stream_correct;
use mailcraft_model::{Id, Stream};
use serde_json::json;

use crate::MailApp;
use crate::theme::{Tokens, regular, semibold};

/// How long the offer of the same for the senders' next ones stays, unanswered, in seconds: as the one that follows a ⚡
/// (Nora's `sorting::OFFER_MS`).
pub(crate) const OFFER_S: i64 = 15;

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

/// The offer while it stands ([`OFFER_S`]).
fn live(app: &MailApp) -> Option<&stream_correct::Proposal> {
    app.session.hosted_stream.proposal.as_ref().filter(|o| app.session.now() - o.at < OFFER_S)
}

/// What the offer says: where the messages went (its words), and the same for their senders' next ones (its button), with
/// shorter ways of the button where the list is narrow: the sender's name cut short (three letters at the least), then without it.
pub(crate) fn offer_text(offer: &stream_correct::Proposal) -> (String, String, Vec<String>) {
    let words = crate::i18n::fmt(tl!("Moved to {tab}."), &[("tab", tab_name(offer.stream))]);
    let fmt = |name: &str| crate::i18n::fmt(tl!("The next ones from {sender} too"), &[("sender", name)]);
    let (button, mut shorter) = match offer.senders.as_slice() {
        [one] => {
            let name: Vec<char> = one.trim().chars().collect();
            let cut = (3..name.len()).rev().map(|keep| fmt(&format!("{}…", name[..keep].iter().collect::<String>().trim_end()))).collect();
            (fmt(one.trim()), cut)
        }
        many => (crate::i18n::fmt(tl!("The next ones from these {count} senders too"), &[("count", &many.len().to_string())]), Vec::new()),
    };
    shorter.push(tl!("The next ones too").to_string());
    (words, button, shorter)
}

/// The offer as the line above Nora's bar says it (her `ask_bar::hint`, first of all), while it stands.
pub(crate) fn hint(app: &MailApp) -> Option<crate::nora::ask_bar::Hint> {
    let offer = live(app)?;
    let (words, button, button_shorter) = offer_text(offer);
    // Shorter, where the list is narrow: the tab alone, so that the line always says where the message went.
    let words_shorter = vec![crate::i18n::fmt(tl!("→ {tab}"), &[("tab", tab_name(offer.stream))])];
    Some(crate::nora::ask_bar::Hint {
        account: String::new(),
        words,
        button,
        words_shorter,
        button_shorter,
        intent: crate::nora::sorting::Intent::StreamSender,
        close: crate::nora::ask_bar::Close::StreamOffer,
    })
}

/// The person's yes: their senders' other messages there too, and the server keeps it for their next ones.
pub(crate) fn yes(app: &mut MailApp) {
    let Some(offer) = app.session.hosted_stream.proposal.take() else { return };
    let _ = app.run("mail.correctStream", json!({"ids": offer.ids, "stream": offer.stream.code(), "sender": true}));
}

/// The offer's ✕: the messages chosen stay where they were put, alone.
pub(crate) fn drop_offer(app: &mut MailApp) {
    app.session.hosted_stream.proposal = None;
}

/// The offer where the line above Nora's bar is not there to say it (her pane open, a list too narrow, a tab without her), in
/// the list's flow above its rows, its words folded on as many lines as they need; and, wherever it is said, its end: the offer
/// goes after [`OFFER_S`] seconds unanswered.
pub(crate) fn offer_bar(app: &mut MailApp, ui: &mut Ui) {
    let Some(offer) = app.session.hosted_stream.proposal.clone() else { return };
    let left = OFFER_S - (app.session.now() - offer.at);
    if left <= 0 {
        app.session.hosted_stream.proposal = None;
        return;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(left.max(1) as u64));
    if app.session.view.module != mailcraft_engine::Module::Mail
        || (crate::mail_view::spark_list(app) && crate::nora::ask_bar::drawn(app, ui.max_rect().width()))
    {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let (words, button, _) = offer_text(&offer);
    let (mut yes_clicked, mut close) = (false, false);
    Frame::new()
        .fill(t.surface_sunken)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 6))
        .outer_margin(Margin { left: 12, right: 10, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(&words).font(regular(13.0)).color(t.text));
                let text = egui::RichText::new(&button).font(semibold(13.0)).color(t.accent_text);
                let answer = ui.add(egui::Button::new(text).frame(false));
                app.hit.add("streamOfferYes", 0, answer.rect);
                yes_clicked = answer.clicked();
                let shut = crate::widgets::icon_button(ui, "close", tl!("Close"), 14.0, true);
                app.hit.add("streamOfferClose", 0, shut.rect);
                close = shut.clicked();
            });
        });
    if yes_clicked {
        yes(app);
    } else if close {
        drop_offer(app);
    }
}

#[cfg(test)]
#[path = "stream_correct_tests.rs"]
mod tests;
