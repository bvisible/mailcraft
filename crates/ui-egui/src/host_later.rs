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

use crate::MailApp;
use crate::ribbon::{Menu, Tab};
use mailcraft_engine::cmd::kept_here::{LATER, SUBSCRIBED};

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

/// Whether Search Commands leaves `id` out: one of their commands, in a tab that doesn't offer them.
pub(crate) fn hidden(app: &MailApp, id: &str) -> bool {
    !offered(app) && (LATER.contains(&id) || SUBSCRIBED.contains(&id) || SUBSCRIPTION_LEFTOVERS.contains(&id))
}

/// Whether the window `name` (`ui.dialog`) doesn't open: one of theirs, in a tab that doesn't offer them.
pub(crate) fn dialog_hidden(app: &MailApp, name: &str) -> bool {
    !offered(app) && DIALOGS.contains(&name)
}

/// Delay Delivery and the Follow Up of the message being written taken out of `tab` where the window doesn't offer them (the Follow
/// Up of a message read stays: its flags are the server's; its Add Reminder is left out, `ribbon::follow_up_items`).
pub(crate) fn ribbon(app: &MailApp, tab: &mut Tab) {
    if offered(app) {
        return;
    }
    for group in &mut tab.groups {
        group.items.retain(|item| item.command.as_ref().is_none_or(|(id, _)| !LATER.contains(id)) && item.menu != Some(Menu::ComposeFollowUp));
    }
}

#[cfg(test)]
#[path = "host_later_tests.rs"]
mod tests;
