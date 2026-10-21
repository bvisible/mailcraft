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
//! name shown, the role, the parent, the unread mail this window knows of, and the number the pane shows beside
//! each, by the pane's own rule), the Favourites the pane lists that are theirs, which of their folders is in view,
//! the module in front and the modules offered. No message, no address but the mailboxes' own,
//! no token; a mailbox the person added by hand in the window, or a shared one, is not named.
//!
//! **What the page may ask** is eight commands and no other, each run as the window's own button or box runs it, and
//! answered: done, or refused with the reason in the interface's language. A message that is no command (another
//! kind of message, or not one the window can read) is not answered. `message.open` names a message by its server's
//! id: one that no window has brought yet (a link from a notification) is read from its server first, by its id alone
//! (`JmapMailRequest::emails`, through [`Services::jmap`](crate::Services::jmap)), and the answer waits for it
//! ([`poll`]), [`OPEN_WAIT_MS`] at most.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

use mailcraft_engine::Scope;
use mailcraft_engine::cmd::jmap_down::plausible_id;
use mailcraft_engine::cmd::jmap_mail::apply_jmap_opened;
use mailcraft_model::sync::{JmapMailRequest, JmapMailResult, JmapScope, MessagePage};
use mailcraft_model::{AccountKind, Folder, FolderKind, Id, Mailbox};
use serde_json::{Map, Value, json};

use crate::MailApp;
use crate::jmap_job::JmapJob;
use crate::nav::Count;

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
/// How long a message the page opens may take to come from its server, in milliseconds, and how many may be on their
/// way at once.
pub const OPEN_WAIT_MS: f64 = 30_000.0;
const MAX_OPENING: usize = 8;

/// What a hosting page that draws the navigation in its own menu handed over, the messages it opened that are on
/// their way from their server, and the folders a Send/Receive under way must not empty of them.
#[derive(Default)]
pub struct Menu {
    /// The mailboxes, in the page's order.
    pub mailboxes: Vec<Handed>,
    opening: Vec<Opening>,
    /// The account and the server id of each folder a message read by its id went into while a Send/Receive of that
    /// account was under way: that run read the folder before the message was there, perhaps, and its window of it,
    /// whole as it may be, is not taken for the whole of the folder ([`spare`]) until the run is over ([`run_over`]).
    spared: Vec<(Id, String)>,
}

impl Menu {
    /// The menu of a page that handed over `mailboxes`, in its order.
    pub fn new(mailboxes: Vec<Handed>) -> Menu {
        Menu { mailboxes, ..Default::default() }
    }
}

/// A message the page opened that is on its way from its server: the ids of the commands that wait for it (the page
/// may ask again before it comes), the account and the email's id, the run that reads it, and until when it is waited
/// for.
struct Opening {
    ids: Vec<Value>,
    account: Id,
    email: String,
    pieces: Receiver<JmapMailResult>,
    cancel: Arc<AtomicBool>,
    until: f64,
    /// What the run said went wrong, and whether the server refused the sign-in.
    errors: Vec<String>,
    refused: bool,
    /// The run is over (its last piece came, or it went).
    over: bool,
    /// What it brought for a folder that isn't here yet (a tab whose first Send/Receive hasn't listed the folders):
    /// handed in again at each look, until it is.
    held: Vec<MessagePage>,
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
                        // What the pane shows beside it, by the pane's own rule.
                        "count": pane_count(crate::nav::shown_count(&counts, f)),
                    })
                })
                .collect();
            let inbox = mb.folder_of_kind(*account, FolderKind::Inbox).map_or(0, unread);
            json!({"email": h.email, "name": h.name, "unread": inbox, "folders": folders})
        })
        .collect();
    // The views offered here, in the module bar's order: Notes only where `view.notes` runs.
    let views: Vec<&str> = VIEWS.iter().filter(|view| s.enabled(view)).filter_map(|view| view.strip_prefix("view.")).collect();
    Some(json!({
        "type": STATE,
        "module": s.view.module,
        "account": account,
        "folder": folder,
        "favorites": favorites,
        "accounts": accounts,
        "views": views,
    }))
}

/// The number the pane shows beside a folder, as the page is told it: unread messages, or all of them (what the pane
/// puts in brackets: Drafts, Outbox, Junk Email), or `null` when the pane shows none.
fn pane_count(count: Count) -> Value {
    match count {
        Count::Unread(n) => json!({"value": n, "kind": "unread"}),
        Count::Total(n) => json!({"value": n, "kind": "total"}),
        Count::Nothing => Value::Null,
    }
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
    /// A message of a mailbox the page handed over, by the email's id on its server: Mail in front, its folder shown
    /// and the message selected, as a click in the list does.
    Message { account: String, id: String },
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

/// The page's message `text` (its JSON): a command of the eight, with what it names, or why it is refused.
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

/// The command `name` with `params`, when it is one of the eight and they are what it takes (nothing more).
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
        "message.open" if only(&["account", "id"]) => match (text("account", MAX_NAME), text("id", MAX_NAME)) {
            (Some(account), Some(id)) if account.contains('@') && plausible_id(id) => {
                Ok(Command::Message { account: account.to_string(), id: id.to_string() })
            }
            _ => Err(bad_params()),
        },
        "mail.new" | "folder.select" | "search" | "message.open" => Err(bad_params()),
        _ => Err(unknown()),
    }
}

/// The account here of the mailbox the page names by `address`: one it handed over.
fn named_account(app: &MailApp, address: &str) -> Result<Id, String> {
    let menu = app.services.host_menu.as_ref();
    menu.map(|menu| handed(menu, &app.session.mailbox))
        .unwrap_or_default()
        .into_iter()
        .find(|(h, _)| h.email.trim().eq_ignore_ascii_case(address.trim()))
        .map(|(_, id)| id)
        .ok_or_else(|| tl!("This mailbox isn't one the page handed over.").to_string())
}

/// The folder a [`Command::Folder`] names: one of a mailbox the page handed over, by the id the state gives it.
fn named_folder(app: &MailApp, account: &str, folder: &str) -> Result<Id, String> {
    let account = named_account(app, account)?;
    let mb = &app.session.mailbox;
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
        // (One this window has: one it hasn't is read from its server first, by `answer`.)
        Command::Message { account, id } => {
            let account = named_account(app, account)?;
            let message = here(app, account, id).ok_or_else(|| tl!("That mailbox has no such message.").to_string())?;
            return show(app, message);
        }
    };
    refused(app, ran)
}

/// A command's refusal, in the interface's language.
fn refused(app: &MailApp, ran: Result<Value, String>) -> Result<(), String> {
    ran.map(|_| ()).map_err(|e| crate::i18n::run_words::refusal(&e, &app.session.mailbox).into_owned())
}

/// The message of `account` whose server id is `id`, when this window has it.
fn here(app: &MailApp, account: Id, id: &str) -> Option<Id> {
    let mb = &app.session.mailbox;
    mb.messages.values().find(|m| m.account == account && m.server.as_ref().is_some_and(|server| server.id == id && !server.gone)).map(|m| m.id)
}

/// The message `message` as a click in the list shows it: Mail in front, its folder in view (no search), the message
/// selected and in the reading pane, and the list brought round to it.
fn show(app: &mut MailApp, message: Id) -> Result<(), String> {
    let folder = app.session.mailbox.message(message).map(|m| m.folder).ok_or_else(|| tl!("That mailbox has no such message.").to_string())?;
    let ran = app.run("folder.select", json!({"id": folder}));
    refused(app, ran)?;
    let ran = app.run("mail.select", json!({"id": message}));
    refused(app, ran)?;
    app.reveal_selection = true;
    Ok(())
}

/// What `message.open` gets on with when this window hasn't the message: the account, and the run that reads it.
type Fetching = (Id, Receiver<JmapMailResult>, Arc<AtomicBool>);

/// Ask the server of `account` for the email `id`, which this window hasn't got: by its id alone, in a run of its own
/// beside any Send/Receive, with the page's credentials.
fn fetch(app: &mut MailApp, account: Id, id: &str) -> Result<Fetching, String> {
    let cannot = || tl!("This window can't ask the server for the message.").to_string();
    let a = app.session.mailbox.account(account).ok_or_else(cannot)?;
    let server = a.jmap.as_ref().filter(|server| server.problem().is_none()).ok_or_else(cannot)?;
    // (A tab whose mailboxes a page handed over: the page holds the credentials, and no job carries a secret.)
    if !app.services.jmap_host_credentials {
        return Err(cannot());
    }
    // The run lists the folders too when none of the account's is known yet (a tab that has just opened): the email
    // is put in its folder.
    let tied = app.session.mailbox.folders.iter().any(|f| f.account == account && f.server.is_some());
    let request = JmapMailRequest {
        account,
        email: a.email.clone(),
        session: server.session.clone(),
        bearer: server.bearer,
        window: 1,
        list_folders: !tied,
        scope: JmapScope::Light { also: Vec::new() },
        emails: vec![id.to_string()],
        ..Default::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let runner = app.services.jmap.as_ref().ok_or_else(cannot)?;
    Ok((account, runner(JmapJob { request, secret: None, cancel: cancel.clone() }), cancel))
}

/// What `message.open` comes to at once.
enum Opened {
    /// The window has the message: shown.
    Shown,
    /// The message is on its way already, for an earlier command: this one waits with it.
    Joined,
    /// The run that brings it from its server.
    Fetching(Fetching),
}

/// `message.open`: the message shown at once when this window has it, else the run that brings it from its server
/// (one for each message: a second command for one on its way waits with the first).
fn open(app: &mut MailApp, account: &str, email: &str, id: &Value) -> Result<Opened, String> {
    let account = named_account(app, account)?;
    if let Some(message) = here(app, account, email) {
        return show(app, message).map(|()| Opened::Shown);
    }
    if let Some(menu) = app.services.host_menu.as_mut()
        && let Some(on_its_way) = menu.opening.iter_mut().find(|o| o.account == account && o.email == email)
    {
        on_its_way.ids.push(id.clone());
        return Ok(Opened::Joined);
    }
    if app.services.host_menu.as_ref().is_some_and(|menu| menu.opening.len() >= MAX_OPENING) {
        return Err(tl!("Too many messages are being opened at once.").to_string());
    }
    fetch(app, account, email).map(Opened::Fetching)
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
        // A message no window has brought yet: read from its server first, and answered once it is here ([`poll`]).
        Incoming::Command { id, command: Command::Message { account, id: email } } => match open(app, &account, &email, &id) {
            Ok(Opened::Fetching((account, pieces, cancel))) => {
                let until = crate::now_ms() + OPEN_WAIT_MS;
                let ids = vec![id];
                let opening =
                    Opening { ids, account, email, pieces, cancel, until, errors: Vec::new(), refused: false, over: false, held: Vec::new() };
                if let Some(menu) = app.services.host_menu.as_mut() {
                    menu.opening.push(opening);
                }
                return None;
            }
            Ok(Opened::Joined) => return None,
            Ok(Opened::Shown) => (id, Ok(())),
            Err(reason) => (id, Err(reason)),
        },
        Incoming::Command { id, command } => (id, run(app, &command)),
    };
    Some(done(id, result))
}

/// Whether messages the page opened are on their way from their server: the host looks again ([`poll`]) a moment
/// later even when nothing else happens, so that one that doesn't come in time is answered.
pub fn waiting(app: &MailApp) -> bool {
    app.services.host_menu.as_ref().is_some_and(|menu| !menu.opening.is_empty())
}

/// Once a frame: take in what the runs of the messages the page opened brought, and the answers of those that are
/// over: shown, not on the server, not placed in time (its folder isn't known here yet), or not come in time (that
/// run is stopped).
pub fn poll(app: &mut MailApp) -> Vec<Value> {
    let Some(menu) = app.services.host_menu.as_mut() else { return Vec::new() };
    let opening = std::mem::take(&mut menu.opening);
    let now = crate::now_ms();
    let (mut answers, mut waits) = (Vec::new(), Vec::new());
    for mut o in opening {
        while !o.over {
            match o.pieces.try_recv() {
                Ok(mut piece) => {
                    o.over = !piece.partial;
                    o.refused |= piece.auth_refused;
                    o.errors.extend(piece.errors.iter().take(4).cloned());
                    // The run's pieces are for its account, whatever they name; the engine still checks the address.
                    piece.account = o.account;
                    take_in(app, &mut o, &piece);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => o.over = true,
            }
        }
        // What waited for its folder, once a Send/Receive has listed it.
        if !o.held.is_empty() {
            let held = JmapMailResult { account: o.account, pages: std::mem::take(&mut o.held), ..Default::default() };
            take_in(app, &mut o, &held);
        }
        let answer = match here(app, o.account, &o.email) {
            Some(message) => show(app, message),
            // The server has it, in a folder this window doesn't know yet: a Send/Receive may list it in time.
            None if !o.held.is_empty() && now < o.until => {
                waits.push(o);
                continue;
            }
            None if !o.held.is_empty() => Err(tl!("The message's folder isn't known here yet: try again in a moment.").to_string()),
            None if o.over => Err(match o.errors.first() {
                Some(line) => crate::i18n::run_words::refusal(line, &app.session.mailbox).into_owned(),
                None if o.refused => crate::i18n::run_words::refusal("The server refused the sign-in", &app.session.mailbox).into_owned(),
                None => tl!("That mailbox has no such message.").to_string(),
            }),
            None if now >= o.until => {
                o.cancel.store(true, Ordering::Relaxed);
                Err(tl!("The message didn't come from the server in time.").to_string())
            }
            None => {
                waits.push(o);
                continue;
            }
        };
        answers.extend(o.ids.into_iter().map(|id| done(id, answer.clone())));
    }
    if let Some(menu) = app.services.host_menu.as_mut() {
        menu.opening.extend(waits);
    }
    answers
}

/// Take in a piece of the run of `o`: what it brought for a folder that isn't here yet is held for the next look, and
/// a folder a message went into while a Send/Receive of the account is under way is spared by that run ([`spare`]).
fn take_in(app: &mut MailApp, o: &mut Opening, piece: &JmapMailResult) {
    let waiting = match apply_jmap_opened(&mut app.session, piece) {
        Ok(waiting) => waiting,
        Err(e) => {
            o.errors.push(e.to_string());
            return;
        }
    };
    let running = app.jmap_rx.as_ref().is_some_and(|(account, _, _)| *account == o.account);
    if running && let Some(menu) = app.services.host_menu.as_mut() {
        for page in piece.pages.iter().filter(|page| !waiting.iter().any(|w| w.folder == page.folder)) {
            if !menu.spared.iter().any(|(account, folder)| *account == o.account && *folder == page.folder) {
                menu.spared.push((o.account, page.folder.clone()));
            }
        }
    }
    o.held.extend(waiting);
}

/// A piece of the Send/Receive of `account` under way, before the window takes it in: a window of a folder that a
/// message read by its id went into since the run began is not the whole of the folder (the message may have come
/// after the run read it), so that nothing of the folder is taken for gone by this run.
pub(crate) fn spare(app: &MailApp, account: Id, piece: &mut JmapMailResult) {
    let Some(menu) = app.services.host_menu.as_ref() else { return };
    for page in piece.pages.iter_mut().filter(|page| page.afresh) {
        if menu.spared.iter().any(|(spared, folder)| *spared == account && *folder == page.folder) {
            page.afresh = false;
        }
    }
}

/// The Send/Receive of `account` is over: the next one reads the folders after the messages read by their ids.
pub(crate) fn run_over(app: &mut MailApp, account: Id) {
    if let Some(menu) = app.services.host_menu.as_mut() {
        menu.spared.retain(|(spared, _)| *spared != account);
    }
}

#[cfg(test)]
mod tests;
