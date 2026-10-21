// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a hosting page that draws MailCraft's navigation in its own menu.

//! A hosting page that draws MailCraft's navigation in its own menu (Neoffice's desk, which hands the browser build
//! `chrome: "menu"`: apps/mailcraft-web/src/host.rs). The window then draws no module bar and, in Mail, no folder
//! pane, not even its strip, so that the message list starts at the window's left edge
//! ([`Services::host_menu`](crate::Services::host_menu)); the ribbon with its search box, the list, the reading
//! pane, the side panes of Calendar, People and Notes and every shortcut stay. The page is told where the window is
//! ([`state`]) and its menu drives it ([`answer`]); the host carries the messages both ways.
//!
//! **What the page is told** is of the mailboxes it handed over alone: their folders as the pane lists them (the
//! name shown, the role, the parent, the unread mail this window knows of), the Favourites the pane lists that are
//! theirs, which of their folders is in view and the module in front. No message, no address but the mailboxes' own,
//! no token; a mailbox the person added by hand in the window, or a shared one, is not named.
//!
//! **What the page may ask** is seven commands and no other, each run as the window's own button or box runs it, and
//! answered: done, or refused with the reason in the interface's language. A message that is no command (another
//! kind of message, or not one the window can read) is not answered.

use mailcraft_engine::Scope;
use mailcraft_model::{AccountKind, Folder, FolderKind, Id, Mailbox};
use serde_json::{Map, Value, json};

use crate::MailApp;

/// What each message is called (`type`): the window has started, where it is, a command of the page's, its answer.
pub const READY: &str = "mailcraft:ready";
pub const STATE: &str = "mailcraft:state";
pub const COMMAND: &str = "mailcraft:command";
pub const DONE: &str = "mailcraft:command-done";

/// The commands that bring a module to the front, by the engine's names.
const VIEWS: [&str; 4] = ["view.mail", "view.calendar", "view.people", "view.notes"];
/// How a folder that only this window has so far is named to the page: `local:` and its number here. A JMAP id is
/// made of letters, digits, `-` and `_` (RFC 8620 §1.2), so a server's id is never taken for one.
const LOCAL: &str = "local:";
/// The longest message of the page's that is read, in bytes (a longer one is not answered), the longest id it is
/// answered under, the longest address or folder id a command names, and the longest text searched for (the
/// engine's own limit, `view.search`), in characters.
const MAX_MESSAGE: usize = 64 * 1024;
const MAX_ID: usize = 64;
const MAX_NAME: usize = 320;
const MAX_QUERY: usize = 2_000;

/// What a hosting page that draws the navigation in its own menu handed over.
#[derive(Clone, Default)]
pub struct Menu {
    /// The mailboxes, in the page's order.
    pub mailboxes: Vec<Handed>,
}

/// A mailbox the page handed over, as it said it.
#[derive(Clone, Default)]
pub struct Handed {
    /// Its address, which names it to the page both ways.
    pub email: String,
    /// The name the page gave it (may be empty).
    pub name: String,
}

/// The message that says the window has started and read what the page handed over.
pub fn ready(version: &str) -> Value {
    json!({"type": READY, "version": version})
}

/// The page's mailboxes in the page's order, each with the account it is here: the JMAP account of its address, once.
fn handed<'a>(menu: &'a Menu, mb: &Mailbox) -> Vec<(&'a Handed, Id)> {
    let mut out: Vec<(&Handed, Id)> = Vec::new();
    for h in &menu.mailboxes {
        let found = mb.accounts.iter().find(|a| a.kind == AccountKind::Jmap && !a.is_shared() && a.email.trim().eq_ignore_ascii_case(h.email.trim()));
        if let Some(a) = found
            && !out.iter().any(|(_, id)| *id == a.id)
        {
            out.push((h, a.id));
        }
    }
    out
}

/// A folder's id for the page: its server's, or [`LOCAL`] and its number for one only this window has so far (an
/// Outbox, a folder made here that its server is yet to hear of).
fn folder_id(f: &Folder) -> String {
    match f.server.as_ref().map(|s| s.id.as_str()).filter(|id| !id.is_empty()) {
        Some(id) => id.to_string(),
        None => format!("{LOCAL}{}", f.id),
    }
}

/// What a folder is for, in the page's words: `null` for one of the person's own.
fn role(kind: FolderKind) -> Option<&'static str> {
    match kind {
        FolderKind::Inbox => Some("inbox"),
        FolderKind::Sent => Some("sent"),
        FolderKind::Drafts => Some("drafts"),
        FolderKind::Archive => Some("archive"),
        FolderKind::Junk => Some("spam"),
        FolderKind::Deleted => Some("trash"),
        FolderKind::Outbox => Some("outbox"),
        FolderKind::Custom => None,
    }
}

/// Where the window is, for a page that draws its navigation (`mailcraft:state`): `None` where no page does. The
/// names are the interface's, in its language.
pub fn state(app: &MailApp) -> Option<Value> {
    let menu = app.services.host_menu.as_ref()?;
    let (s, mb) = (&app.session, &app.session.mailbox);
    let handed = handed(menu, mb);
    let of = |account: Id| handed.iter().find(|(_, id)| *id == account).map(|(h, _)| *h);
    let counts = crate::nav::tallies(mb);
    let unread = |folder: Id| counts.get(&folder).map_or(0, |(_, unread)| *unread);
    // The folder in view, when it is a mailbox's of the page's: none while a search shows what it found.
    let shown = mb.folder(s.view.folder).filter(|f| of(f.account).is_some());
    let searching = !s.view.search.is_empty();
    let everywhere = searching && s.view.scope == Scope::Everywhere;
    let account = shown.filter(|_| !everywhere).and_then(|f| of(f.account)).map(|h| h.email.clone());
    let folder = shown.filter(|_| !searching).map(folder_id);
    let favorites: Vec<Value> = mailcraft_engine::cmd::folder::favorites(s)
        .into_iter()
        .filter_map(|id| mb.folder(id))
        .filter_map(|f| {
            let h = of(f.account)?;
            // As the pane writes it under Favorites: two Inboxes say whose they are.
            Some(json!({"account": h.email, "folder": folder_id(f), "name": crate::nav::result_folder_label(mb, f), "unread": unread(f.id)}))
        })
        .collect();
    let accounts: Vec<Value> = handed
        .iter()
        .map(|(h, account)| {
            // Every folder in the pane's order, folded or not.
            let folders: Vec<Value> = crate::nav::visible(mb, &[], *account)
                .iter()
                .map(|(f, _, _)| {
                    json!({
                        "id": folder_id(f),
                        "name": crate::nav::folder_name(f),
                        "role": role(f.kind),
                        "parent": f.parent.and_then(|p| mb.folder(p)).map(folder_id),
                        "unread": unread(f.id),
                        // Every folder the pane lists can be shown.
                        "selectable": true,
                    })
                })
                .collect();
            let inbox = mb.folder_of_kind(*account, FolderKind::Inbox).map_or(0, unread);
            json!({"email": h.email, "name": h.name, "unread": inbox, "folders": folders})
        })
        .collect();
    Some(json!({"type": STATE, "module": s.view.module, "account": account, "folder": folder, "favorites": favorites, "accounts": accounts}))
}

/// A command the page may send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// A module to the front: `view.mail`, `view.calendar`, `view.people` or `view.notes`.
    View(&'static str),
    /// A folder of a mailbox the page handed over, shown in Mail: the mailbox's address and the folder's id as the
    /// state gave it.
    Folder { account: String, folder: String },
    /// A new message, from the mailbox shown.
    NewMail,
    /// What the search box does with `query` as it is typed (empty: nothing is searched for any more).
    Search(String),
}

/// A message from the page, read.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// Not a command: another kind of message, or not one the window can read. Not answered.
    Ignored,
    /// A command the window takes, and the id it is answered under.
    Command { id: Value, command: Command },
    /// A command the window refuses, and why, in the interface's language.
    Refused { id: Value, reason: String },
}

/// The page's message `text` (its JSON): a command of the seven, with what it names, or why it is refused.
pub fn read(text: &str) -> Incoming {
    if text.len() > MAX_MESSAGE {
        return Incoming::Ignored;
    }
    let Ok(Value::Object(message)) = serde_json::from_str::<Value>(text) else { return Incoming::Ignored };
    if message.get("type").and_then(Value::as_str) != Some(COMMAND) {
        return Incoming::Ignored;
    }
    let id = match message.get("id") {
        Some(Value::Number(n)) => Value::Number(n.clone()),
        Some(Value::String(id)) if !id.is_empty() && id.chars().count() <= MAX_ID => Value::String(id.clone()),
        _ => return Incoming::Refused { id: Value::Null, reason: tl!("The command has no id.").to_string() },
    };
    let empty = Map::new();
    let params = match message.get("params") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(params)) => params,
        Some(_) => return Incoming::Refused { id, reason: bad_params() },
    };
    match message.get("name").and_then(Value::as_str).map(|name| command(name, params)) {
        Some(Ok(command)) => Incoming::Command { id, command },
        Some(Err(reason)) => Incoming::Refused { id, reason },
        None => Incoming::Refused { id, reason: unknown() },
    }
}

fn bad_params() -> String {
    tl!("The page's command has parameters it doesn't take.").to_string()
}

fn unknown() -> String {
    tl!("MailCraft doesn't take this command from the page.").to_string()
}

/// The command `name` with `params`, when it is one of the seven and they are what it takes (nothing more).
fn command(name: &str, params: &Map<String, Value>) -> Result<Command, String> {
    let only = |keys: &[&str]| params.keys().all(|k| keys.contains(&k.as_str()));
    let text = |key: &str, max: usize| params.get(key).and_then(Value::as_str).filter(|v| v.chars().count() <= max);
    if let Some(view) = VIEWS.iter().find(|v| **v == name) {
        return if params.is_empty() { Ok(Command::View(view)) } else { Err(bad_params()) };
    }
    match name {
        "mail.new" if params.is_empty() => Ok(Command::NewMail),
        "folder.select" if only(&["account", "folder"]) => match (text("account", MAX_NAME), text("folder", MAX_NAME)) {
            (Some(account), Some(folder)) if account.contains('@') && !folder.is_empty() => {
                Ok(Command::Folder { account: account.to_string(), folder: folder.to_string() })
            }
            _ => Err(bad_params()),
        },
        "search" if only(&["query"]) => text("query", MAX_QUERY).map(|query| Command::Search(query.to_string())).ok_or_else(bad_params),
        "mail.new" | "folder.select" | "search" => Err(bad_params()),
        _ => Err(unknown()),
    }
}

/// The folder a [`Command::Folder`] names: one of a mailbox the page handed over, by the id the state gives it.
fn named_folder(app: &MailApp, account: &str, folder: &str) -> Result<Id, String> {
    let mb = &app.session.mailbox;
    let menu = app.services.host_menu.as_ref();
    let Some(account) = menu
        .map(|menu| handed(menu, mb))
        .unwrap_or_default()
        .into_iter()
        .find(|(h, _)| h.email.trim().eq_ignore_ascii_case(account.trim()))
        .map(|(_, id)| id)
    else {
        return Err(tl!("This mailbox isn't one the page handed over.").to_string());
    };
    let here = || {
        let id = folder.strip_prefix(LOCAL)?.parse::<Id>().ok()?;
        mb.folder(id).filter(|f| f.account == account && folder_id(f) == folder).map(|f| f.id)
    };
    mb.folder_by_server(account, folder).or_else(here).ok_or_else(|| tl!("That mailbox has no such folder.").to_string())
}

/// Run a command of the page's as the window's own button or box runs it: why not, in the interface's language, when
/// it was refused.
pub fn run(app: &mut MailApp, command: &Command) -> Result<(), String> {
    let ran = match command {
        Command::View(view) => app.run(view, json!({})),
        Command::NewMail => app.run("mail.new", json!({})),
        Command::Folder { account, folder } => {
            let id = named_folder(app, account, folder)?;
            app.run("folder.select", json!({"id": id}))
        }
        Command::Search(query) => {
            let ran = app.run("view.search", json!({"query": query}));
            // The box says what is searched for, even with the keyboard in it.
            app.search_text = app.session.view.search.clone();
            ran
        }
    };
    ran.map(|_| ()).map_err(|e| crate::i18n::run_words::refusal(&e, &app.session.mailbox).into_owned())
}

/// The answer to a command: done, or why not.
pub fn done(id: Value, result: Result<(), String>) -> Value {
    match result {
        Ok(()) => json!({"type": DONE, "id": id, "ok": true}),
        Err(error) => json!({"type": DONE, "id": id, "ok": false, "error": error}),
    }
}

/// The window's answer to the page's message `text` (its JSON), the command run first: `None` when it is no command,
/// or where no page draws the navigation.
pub fn answer(app: &mut MailApp, text: &str) -> Option<Value> {
    app.services.host_menu.as_ref()?;
    let (id, result) = match read(text) {
        Incoming::Ignored => return None,
        Incoming::Refused { id, reason } => (id, Err(reason)),
        Incoming::Command { id, command } => (id, run(app, &command)),
    };
    Some(done(id, result))
}

#[cfg(test)]
mod tests;
