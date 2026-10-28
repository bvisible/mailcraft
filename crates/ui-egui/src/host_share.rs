// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a hosting page that takes a message to share in its team chat.

//! A hosting page that takes a message to share in its team chat (Neoffice's desk at `/app/neomail`, which hands the
//! browser build `share: true` and shares it in Synk, the instance's chat: apps/mailcraft-web/src/host.rs). The window
//! offers, in the Respond group of Mail's Home tab ([`ribbon`]) and in a message's context menu:
//! - Share in Synk ([`COMMAND`]): the page asks the person where to share the message ([`message`]);
//! - Chat in Synk, with the sender ([`CHAT_SENDER`]) or with everyone of the message ([`CHAT_ALL`]): the page opens the
//!   conversation with those of them who use Synk ([`chat_message`]).
//!
//! A click hands the page the message's mailbox and the message's id on its server, and nothing else: its server
//! reads the message itself (its text, or its sender and its recipients), under its own rules of who may. Only a
//! message of a mailbox the page handed over, that its server has ([`target`]): a mailbox added by hand in the window,
//! or a message only the window has so far, is never named to the page.

use mailcraft_engine::cmd::jmap_down::plausible_id;
use mailcraft_model::AccountKind;
use serde_json::{Value, json};

use crate::MailApp;
use crate::ribbon::{Item, Menu, Size, Tab};

/// What the messages to the page are called (`type`): a message to share, a conversation to open.
pub const SHARE_TYPE: &str = "mailcraft:share";
pub const CHAT_TYPE: &str = "mailcraft:chat";
/// The window's commands, which the ribbon's buttons and the context menu run: Share in Synk, and Chat in Synk with
/// the sender or with everyone of the message.
pub const COMMAND: &str = "ui.shareMessage";
pub const CHAT_SENDER: &str = "ui.chatWithSender";
pub const CHAT_ALL: &str = "ui.chatWithEveryone";
/// The group of Mail's Home tab the buttons are in.
const RESPOND: &str = "Respond";

/// What a hosting page that takes messages to share handed over.
pub struct Share {
    /// The addresses of the mailboxes it handed over: only their messages are offered.
    pub mailboxes: Vec<String>,
    /// Hand the page a message of the window's ([`message`], [`chat_message`]).
    pub post: Box<dyn Fn(&Value)>,
}

/// The message to the page for the message of the mailbox `account` (its address) whose id on the server is `email`.
pub fn message(account: &str, email: &str) -> Value {
    json!({"type": SHARE_TYPE, "account": account, "email": email})
}

/// The message to the page that opens a conversation about the message of the mailbox `account` whose id on the
/// server is `email`, with its sender (`who`: "sender") or with everyone of it ("all").
pub fn chat_message(account: &str, email: &str, who: &str) -> Value {
    json!({"type": CHAT_TYPE, "account": account, "email": email, "who": who})
}

/// The message in the reading pane as the page is told it (its mailbox's address, as the page handed it over, and its
/// id on the server), or why it can't be handed to the page, in the interface's language.
pub(crate) fn target(app: &MailApp) -> Result<(String, String), String> {
    let share = app.services.share.as_ref().ok_or_else(|| tl!("Nothing here takes a message to share.").to_string())?;
    let mb = &app.session.mailbox;
    let m = app.session.sel.focus().and_then(|id| mb.message(id)).ok_or_else(|| tl!("Select a message first").to_string())?;
    let handed = mb
        .account(m.account)
        .filter(|a| a.kind == AccountKind::Jmap && !a.is_shared())
        .and_then(|a| share.mailboxes.iter().find(|address| address.trim().eq_ignore_ascii_case(a.email.trim())));
    let Some(address) = handed else {
        return Err(tl!("Only a message of a mailbox the page handed over can be shared.").to_string());
    };
    let id = m
        .server
        .as_ref()
        .filter(|server| !server.gone && plausible_id(&server.id))
        .map(|server| server.id.clone())
        .ok_or_else(|| tl!("This message isn't on its server yet.").to_string())?;
    Ok((address.trim().to_string(), id))
}

/// Whether the commands can run now: a page takes messages to share, and the message in the reading pane is one.
pub(crate) fn enabled(app: &MailApp) -> bool {
    target(app).is_ok()
}

/// Hand the page the message in the reading pane to share: `{"shared": true}`, or why not.
pub(crate) fn run(app: &MailApp) -> Result<Value, String> {
    let (account, email) = target(app)?;
    if let Some(share) = app.services.share.as_ref() {
        (share.post)(&message(&account, &email));
    }
    Ok(json!({"shared": true}))
}

/// Have the page open a conversation about the message in the reading pane, with its sender (`all` false) or with
/// everyone of it: `{"chat": "sender"|"all"}`, or why not.
pub(crate) fn chat(app: &MailApp, all: bool) -> Result<Value, String> {
    let (account, email) = target(app)?;
    let who = if all { "all" } else { "sender" };
    if let Some(share) = app.services.share.as_ref() {
        (share.post)(&chat_message(&account, &email, who));
    }
    Ok(json!({"chat": who}))
}

/// The buttons in the Respond group of Mail's Home tab, where a page takes messages to share: Share in Synk, and Chat
/// in Synk, whose menu says with whom.
pub(crate) fn ribbon(app: &MailApp, tab: &mut Tab) {
    if app.services.share.is_none() || app.ui.tab == "Message" || app.session.view.module != mailcraft_engine::Module::Mail {
        return;
    }
    let Some(respond) = tab.groups.iter_mut().find(|g| g.title == RESPOND) else { return };
    respond.items.push(Item {
        key: COMMAND,
        icon: "shareChat",
        label: "Share\nin Synk",
        tip: "Share this message in a Synk conversation",
        command: Some((COMMAND, json!({}))),
        menu: None,
        size: Size::Big,
        checked: false,
        section: 2,
        rank: 3,
        plain: false,
        under: None,
        pending: false,
    });
    respond.items.push(Item {
        key: "hostChat",
        icon: "chatSynk",
        label: "Chat\nin Synk",
        tip: "Chat in Synk with the people of this message",
        command: None,
        menu: Some(Menu::HostChat),
        size: Size::Big,
        checked: false,
        section: 2,
        rank: 3,
        plain: false,
        under: None,
        pending: false,
    });
}

/// What Chat in Synk's menu holds: with the sender, with everyone of the message.
pub(crate) fn menu(ui: &mut egui::Ui, app: &mut MailApp) {
    crate::widgets::mi(ui, app, "With the Sender", CHAT_SENDER, json!({}));
    crate::widgets::mi(ui, app, "With Everyone", CHAT_ALL, json!({}));
}

#[cfg(test)]
mod tests;
