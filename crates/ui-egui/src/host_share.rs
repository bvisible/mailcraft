// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a hosting page that takes a message to share in its team chat.

//! A hosting page that takes a message to share in its team chat (Neoffice's desk at `/app/neomail`, which hands the
//! browser build `share: true` and shares it in Synk, the instance's chat: apps/mailcraft-web/src/host.rs). The window
//! offers Share in Synk in the Respond group of Mail's Home tab ([`ribbon`]) and in a message's context menu; a click
//! hands the page the message's mailbox and the message's id on its server, and nothing else ([`message`]): the page
//! asks the person where to share it, and its server reads the message itself, under its own rules of who may.
//!
//! Only a message of a mailbox the page handed over, that its server has ([`target`]): a mailbox added by hand in the
//! window, or a message only the window has so far, is never named to the page.

use mailcraft_engine::cmd::jmap_down::plausible_id;
use mailcraft_model::AccountKind;
use serde_json::{Value, json};

use crate::MailApp;
use crate::ribbon::{Item, Size, Tab};

/// What the message to the page is called (`type`).
pub const SHARE_TYPE: &str = "mailcraft:share";
/// The window's command, which the ribbon's button and the context menu run.
pub const COMMAND: &str = "ui.shareMessage";
/// The group of Mail's Home tab the button is in.
const RESPOND: &str = "Respond";

/// What a hosting page that takes messages to share handed over.
pub struct Share {
    /// The addresses of the mailboxes it handed over: only their messages are offered.
    pub mailboxes: Vec<String>,
    /// Hand the page a message to share: the address of its mailbox, and the message's id on its server.
    pub send: Box<dyn Fn(&str, &str)>,
}

/// The message to the page for the message of the mailbox `account` (its address) whose id on the server is `email`.
pub fn message(account: &str, email: &str) -> Value {
    json!({"type": SHARE_TYPE, "account": account, "email": email})
}

/// The message in the reading pane as the page is told it (its mailbox's address, as the page handed it over, and its
/// id on the server), or why it can't be shared, in the interface's language.
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

/// Whether the command can run now: a page takes messages to share, and the message in the reading pane is one.
pub(crate) fn enabled(app: &MailApp) -> bool {
    target(app).is_ok()
}

/// Hand the page the message in the reading pane: `{"shared": true}`, or why not.
pub(crate) fn run(app: &MailApp) -> Result<Value, String> {
    let (account, email) = target(app)?;
    if let Some(share) = app.services.share.as_ref() {
        (share.send)(&account, &email);
    }
    Ok(json!({"shared": true}))
}

/// The button in the Respond group of Mail's Home tab, where a page takes messages to share.
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
    });
}

#[cfg(test)]
mod tests;
