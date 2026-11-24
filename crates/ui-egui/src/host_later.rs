// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): Send Later, Delay Delivery and follow-up reminders are not offered in a hosting page's tab.

//! Upstream's Send Later and Delay Delivery (#950) keep the message in the Outbox of this computer until its time, and its
//! follow-up reminders (#948) ring here: in a tab whose mailboxes a hosting page handed over (Neoffice's desk at `/app/neomail`,
//! `/neomail`, Neodesk's NeoMail window), which keeps nothing between visits, a message would not go and a reminder would not
//! ring once the tab is closed; and a reminder "if nobody replies" would be a second way beside Nora's follow-up, which the
//! mailbox's server keeps. There they are not drawn ([`offered`]): not the arrow beside Send, not Delay Delivery nor Follow Up
//! of the message being written, not Add Reminder in Follow Up and in a message's menu, not Change Send Time over a message
//! waiting in the Outbox, not in Search Commands, and their windows don't open; and the commands refuse whoever asks
//! (`mailcraft_engine::cmd::kept_here::later_refusal`). Undo Send stays: its wait is seconds, in the tab that is open.
//!
//! The calendars subscribed to on the web (#209) go the same way: the browser build has no host to update them, and the
//! subscription would be gone at the next visit (`kept_here::subscription_refusal`): not Open Calendar › From Internet…, not the
//! Subscribed calendars of File › Info, not Search Commands, and their window doesn't open.
//!
//! The order of folders and Favorites (#1514) would be lost at the next visit (`kept_here::order_refusal`): no row of the folder
//! pane is dragged up or down, no Move Up and Move Down in its menu (`nav.rs`), neither command in Search Commands. And the list's
//! Shade Rows (#1657) is not offered, its list being the design system's (`kept_here::own_list_refusal`).
//!
//! The calendars of the mailbox's own (Add Calendar, Move to the Account's Calendar: `kept_here::own_calendar_refusal`), the
//! Favorites of a Microsoft account (`kept_here::outlook_favorites_refusal`) and what else is of Microsoft's alone
//! ([`MICROSOFT_ONLY`]) are not in Search Commands; File › Info removes no mailbox (`kept_here::account_removal_refusal`).
//!
//! Where the host has no dictionary (the browser build), Review shows no Spelling (#954), whose menu would only say so.
//!
//! A flag's due date too: a JMAP server keeps a message's flag and nothing of its date. Follow Up offers the flag without a date
//! only (not Today, Tomorrow, This Week, Next Week), and a date asked for is refused (`kept_here::due_refusal`); a flag set stays.

use crate::MailApp;
use crate::ribbon::{Menu, Tab};
use mailcraft_engine::cmd::kept_here::{LATER, ORDERED, OUTLOOK_FAVORITES, OWN_CALENDARS, SHADED, SUBSCRIBED};

/// The windows of Send Later, Delay Delivery, Change Send Time, the reminders and a subscription (`ui.dialog`'s names).
const DIALOGS: [&str; 6] = ["sendLater", "delay", "sendTime", "addReminder", "followUpDate", "subscribe"];

/// Whether the window offers Send Later, Delay Delivery and the follow-up reminders: where what it keeps stays (not a hosting
/// page's tab).
pub(crate) fn offered(app: &MailApp) -> bool {
    app.session.keeps_local
}

/// Upstream's commands about calendars subscribed to on the web that have nothing to act on where none can be subscribed to:
/// unsubscribing from one, and a host's look at them (which calendars are due, and what it brought). They are not refused (an
/// unsubscribe has no calendar to name, the others nothing to do), only left out of Search Commands.
pub(crate) const SUBSCRIPTION_LEFTOVERS: [&str; 3] = ["calendar.unsubscribe", "sync.subscriptionRequest", "sync.applySubscription"];

/// This fork's commands that only a hosting page's code runs: what the mailbox's server keeps of « Plus tard » and of the Inbox's
/// tabs, and what it answered. Nobody does them by hand, and their names read like things one does (« Mettre en Plus tard » beside
/// « Plus tard »): never in Search Commands, wherever the window is.
pub(crate) const HOST_PLUMBING: [&str; 5] =
    ["mail.setSnoozes", "mail.snoozeAnswered", "mail.setStreams", "mail.offerStreamCorrection", "mail.streamCorrected"];

/// Upstream's commands of v0.15.0 that do nothing without what nobody types into a search box (a place in the text and the words put
/// there; a picture and a width): the spelling menu and the Picture size strip run them. Left out of Search Commands there, where a
/// person who picked one would be told of a parameter.
pub(crate) const WANT_PARAMS: [&str; 2] = ["compose.replaceText", "compose.picture.set"];

/// Upstream's commands that do something for a Microsoft (or Exchange) account alone, which no page hands over: not refused (each
/// says there is none), only left out of Search Commands.
pub(crate) const MICROSOFT_ONLY: [&str; 3] = ["category.addToOutlook", "account.graphSwitch", "account.thinOffline"];

/// Whether Search Commands leaves `id` out: one of their commands, in a tab that doesn't offer them; or a hosting page's own.
pub(crate) fn hidden(app: &MailApp, id: &str) -> bool {
    let lists = [LATER, SUBSCRIBED, ORDERED, OWN_CALENDARS, OUTLOOK_FAVORITES, &SUBSCRIPTION_LEFTOVERS[..], &WANT_PARAMS[..], &MICROSOFT_ONLY[..]];
    let kept_here = lists.iter().any(|list| list.contains(&id)) || id == SHADED || id == "account.remove";
    HOST_PLUMBING.contains(&id) || !offered(app) && kept_here
}

/// Whether the window `name` (`ui.dialog`) doesn't open: one of theirs, in a tab that doesn't offer them.
pub(crate) fn dialog_hidden(app: &MailApp, name: &str) -> bool {
    !offered(app) && DIALOGS.contains(&name)
}

/// Delay Delivery, the Follow Up of the message being written and Shade Rows taken out of `tab` where the window doesn't offer them
/// (the Follow Up of a message read stays: its flags are the server's; its Add Reminder is left out, `ribbon::follow_up_items`),
/// and Spelling where the host has no dictionary.
pub(crate) fn ribbon(app: &MailApp, tab: &mut Tab) {
    if offered(app) {
        return;
    }
    for group in &mut tab.groups {
        group.items.retain(|item| {
            item.command.as_ref().is_none_or(|(id, _)| !LATER.contains(id) && *id != SHADED)
                && item.menu != Some(Menu::ComposeFollowUp)
                && (item.menu != Some(Menu::Spelling) || app.services.spelling.is_some())
        });
    }
}

#[cfg(test)]
#[path = "host_later_tests.rs"]
mod tests;
