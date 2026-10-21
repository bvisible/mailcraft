// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a hosting page that files a message's attachment: its Drive, its scanned documents, an editor.

//! A hosting page that does things with a message's attachment (Neoffice's desk at `/app/neomail`, which hands the
//! browser build `attachments: ["drive", "scan", "edit"]`, or those of them the person may use there:
//! apps/mailcraft-web/src/host.rs). An attachment's menu in the reading pane then offers, after Save As:
//! - Save to Drive… ([`TO_DRIVE`]): the page asks for a folder of its Drive and saves the file there;
//! - Send to Scanned Documents ([`TO_SCANS`]), for a PDF or a picture only ([`scannable`]): the page hands it to its
//!   scanned documents, which read it;
//! - Open in NeoDoc, NeoGrille, NeoPrés or NeoPDF ([`IN_EDITOR`]), for a `.docx`, `.xlsx`, `.pptx` or `.pdf` only
//!   ([`editor`]): the page saves it in its Drive and opens it there in that editor.
//!
//! A click hands the page the message's mailbox and its id on the server, and what the window knows of the file: its
//! rank among the message's attachments and how many the window sees, its name, size, type and Content-ID
//! ([`message`]). Never its bytes: the page's server reads the message itself, under its own rules of who may, and finds
//! the file by what identifies it, the rank only telling apart files that are otherwise alike, and only when the server
//! counts as many attachments as the window (it counts the pictures of a message's text among them and keeps an attached
//! message whole, as the window does; one removed here makes the window's ranks not the server's). Only for a message of
//! a mailbox the page handed over that its server has ([`target`]), as Share in Synk (host_share.rs). Without the page's
//! word, nothing is offered.

use mailcraft_engine::cmd::jmap_down::plausible_id;
use mailcraft_model::{AccountKind, Attachment, Id, Message};
use serde_json::{Value, json};

use crate::MailApp;

/// What the messages to the page are called (`type`).
pub const TYPE: &str = "mailcraft:attachment";
/// The window's commands, which the attachment's menu runs: Save to Drive…, Send to Scanned Documents, and Open in
/// the file's editor.
pub const TO_DRIVE: &str = "ui.saveAttachmentToDrive";
pub const TO_SCANS: &str = "ui.sendAttachmentToScans";
pub const IN_EDITOR: &str = "ui.openAttachmentInEditor";
/// The longest name, type and Content-ID the page is told, in characters.
const MAX_NAME: usize = 1024;
const MAX_MIME: usize = 255;
const MAX_CID: usize = 998;
/// What the scanned documents take: a PDF or a picture, by the name's extension, else by the file's type when the
/// name has none. Never a drawing (SVG), which is text a browser runs. The page's server has the same rule
/// (frappe_webmail, neomail_attachments.py).
const SCAN_EXTENSIONS: &[&str] = &["pdf", "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "heic", "heif"];
const SCAN_TYPES: &[&str] =
    &["application/pdf", "image/png", "image/jpeg", "image/gif", "image/bmp", "image/tiff", "image/webp", "image/heic", "image/heif"];

/// What the page may be asked to do with an attachment, as the handover names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Save it in a folder of its Drive.
    Drive,
    /// Send it to its scanned documents.
    Scan,
    /// Save it in its Drive and open it in its editor.
    Edit,
}

impl Action {
    /// The handover's word for it, and the message's `action`.
    pub fn code(self) -> &'static str {
        match self {
            Action::Drive => "drive",
            Action::Scan => "scan",
            Action::Edit => "edit",
        }
    }

    /// The action the handover's word names: `None` for any other word.
    pub fn from_code(code: &str) -> Option<Action> {
        [Action::Drive, Action::Scan, Action::Edit].into_iter().find(|a| a.code() == code)
    }

    /// The action of the window's command `id`.
    fn of_command(id: &str) -> Option<Action> {
        match id {
            TO_DRIVE => Some(Action::Drive),
            TO_SCANS => Some(Action::Scan),
            IN_EDITOR => Some(Action::Edit),
            _ => None,
        }
    }
}

/// What a handover's words offer (`attachments: ["drive", "scan", "edit"]`): each once, in the window's order, a word
/// the window doesn't know left out.
pub fn actions(words: &[String]) -> Vec<Action> {
    [Action::Drive, Action::Scan, Action::Edit].into_iter().filter(|action| words.iter().any(|word| word == action.code())).collect()
}

/// Whether `id` is one of the window's commands for an attachment handed to the page.
pub fn is_command(id: &str) -> bool {
    Action::of_command(id).is_some()
}

/// The page's editors, each for the format it opens and writes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Document,
    Spreadsheet,
    Presentation,
    Pdf,
}

impl Editor {
    /// Its name, a product's, which no language translates.
    pub fn product(self) -> &'static str {
        match self {
            Editor::Document => "NeoDoc",
            Editor::Spreadsheet => "NeoGrille",
            Editor::Presentation => "NeoPrés",
            Editor::Pdf => "NeoPDF",
        }
    }
}

/// What the page that does things with attachments handed over.
pub struct Attachments {
    /// The addresses of the mailboxes it handed over: only their messages' attachments are offered.
    pub mailboxes: Vec<String>,
    /// What it offers to do with an attachment.
    pub actions: Vec<Action>,
    /// Hand the page an attachment ([`message`]).
    pub post: Box<dyn Fn(&Value)>,
}

/// The extension of the name a file is saved under (`opening::file_name`), in lower case: `None` without one.
fn extension(name: &str) -> Option<String> {
    let saved = mailcraft_engine::opening::file_name(name);
    let (_, extension) = saved.rsplit_once('.')?;
    let extension = extension.trim().to_lowercase();
    (!extension.is_empty()).then_some(extension)
}

/// Whether the page's scanned documents take the file `name` of type `mime`: a PDF or a picture, by the name's
/// extension, else by the type when the name has none.
pub fn scannable(name: &str, mime: &str) -> bool {
    match extension(name) {
        Some(extension) => SCAN_EXTENSIONS.contains(&extension.as_str()),
        None => SCAN_TYPES.contains(&mime.trim().to_ascii_lowercase().as_str()),
    }
}

/// The page's editor that opens the file `name`, by its extension: `None` for any other kind of file.
pub fn editor(name: &str) -> Option<Editor> {
    match extension(name)?.as_str() {
        "docx" => Some(Editor::Document),
        "xlsx" => Some(Editor::Spreadsheet),
        "pptx" => Some(Editor::Presentation),
        "pdf" => Some(Editor::Pdf),
        _ => None,
    }
}

/// The message to the page for the attachment `a`, the `index`-th of the `count` attachments of the message of the
/// mailbox `account` (its address) whose id on the server is `email`: what to do with it, and what the window knows of
/// the file, never its bytes.
pub fn message(action: Action, account: &str, email: &str, index: usize, count: usize, a: &Attachment) -> Value {
    let cut = |text: &str, most: usize| text.chars().take(most).collect::<String>();
    json!({
        "type": TYPE,
        "action": action.code(),
        "account": account,
        "email": email,
        "attachment": {
            "index": index,
            "count": count,
            "name": cut(&a.name, MAX_NAME),
            "size": a.size(),
            "mime": cut(&a.mime, MAX_MIME),
            "cid": cut(&a.content_id, MAX_CID),
        },
    })
}

/// The message `id` as the page is told it (its mailbox's address, as the page handed it over, and its id on the
/// server), or why its attachments can't be handed to the page, in the interface's language.
pub(crate) fn target(app: &MailApp, id: Id) -> Result<(String, String), String> {
    let host = app.services.attachments.as_ref().ok_or_else(|| tl!("Nothing here takes an attachment.").to_string())?;
    let mb = &app.session.mailbox;
    let m = mb.message(id).ok_or_else(|| tl!("Select a message first").to_string())?;
    let handed = mb
        .account(m.account)
        .filter(|a| a.kind == AccountKind::Jmap && !a.is_shared())
        .and_then(|a| host.mailboxes.iter().find(|address| address.trim().eq_ignore_ascii_case(a.email.trim())));
    let Some(address) = handed else {
        return Err(tl!("Only an attachment of a message of a mailbox the page handed over can be handed to it.").to_string());
    };
    let server = m
        .server
        .as_ref()
        .filter(|server| !server.gone && plausible_id(&server.id))
        .map(|server| server.id.clone())
        .ok_or_else(|| tl!("This message isn't on its server yet.").to_string())?;
    Ok((address.trim().to_string(), server))
}

/// Whether the command `id` can run: the page offers what it does. Which attachment, and of which message, is the
/// command's params ([`run`]).
pub(crate) fn enabled(app: &MailApp, id: &str) -> bool {
    let offered = app.services.attachments.as_ref().map(|host| host.actions.as_slice()).unwrap_or_default();
    Action::of_command(id).is_some_and(|action| offered.contains(&action))
}

/// Hand the page the attachment `{"id": message, "index": n}` names (the message in the reading pane without `id`), for
/// what the command `id` does with it: `{"handed": "drive"|"scan"|"edit"}`, or why not.
pub(crate) fn run(app: &MailApp, id: &str, params: &Value) -> Result<Value, String> {
    let action = Action::of_command(id).ok_or_else(|| format!("no command \"{id}\""))?;
    let Some(host) = app.services.attachments.as_ref() else {
        return Err(tl!("Nothing here takes an attachment.").to_string());
    };
    if !host.actions.contains(&action) {
        return Err(tl!("The page doesn't offer this for an attachment.").to_string());
    }
    let message_id =
        params.get("id").and_then(Value::as_u64).or_else(|| app.session.sel.focus()).ok_or_else(|| tl!("Select a message first").to_string())?;
    let index = params
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| "`index` is the number of the attachment in the message, from 0".to_string())?;
    let (account, email) = target(app, message_id)?;
    let attachments = app.session.mailbox.message(message_id).map(|m| m.attachments.as_slice()).unwrap_or_default();
    let a = attachments.get(index).ok_or_else(|| tl!("This message has no such attachment.").to_string())?;
    match action {
        Action::Scan if !scannable(&a.name, &a.mime) => return Err(tl!("Only a PDF or a picture goes to the scanned documents.").to_string()),
        Action::Edit if editor(&a.name).is_none() => return Err(tl!("Only a .docx, .xlsx, .pptx or .pdf file opens in an editor.").to_string()),
        _ => {}
    }
    (host.post)(&message(action, &account, &email, index, attachments.len(), a));
    Ok(json!({"handed": action.code()}))
}

/// An entry of an attachment's menu for what the page does with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// Save to Drive…
    Drive,
    /// Send to Scanned Documents
    Scan,
    /// Open in the file's editor
    Edit(Editor),
}

/// The entries of the menu of the attachment `a` of `m` for what the page does with it: only what the page offers, for
/// what the file is, and only for a message the page may be handed. None without the page's word.
pub(crate) fn entries(app: &MailApp, m: &Message, a: &Attachment) -> Vec<Entry> {
    let Some(host) = app.services.attachments.as_ref() else { return Vec::new() };
    if target(app, m.id).is_err() {
        return Vec::new();
    }
    let offers = |action| host.actions.contains(&action);
    let mut out = Vec::new();
    if offers(Action::Drive) {
        out.push(Entry::Drive);
    }
    if offers(Action::Scan) && scannable(&a.name, &a.mime) {
        out.push(Entry::Scan);
    }
    if let Some(editor) = editor(&a.name).filter(|_| offers(Action::Edit)) {
        out.push(Entry::Edit(editor));
    }
    out
}

/// The [`entries`] of the menu of the attachment `a`, the `index`-th of `m`, between two separators.
pub(crate) fn menu(ui: &mut egui::Ui, app: &mut MailApp, m: &Message, index: usize, a: &Attachment) {
    let entries = entries(app, m, a);
    if entries.is_empty() {
        return;
    }
    use crate::widgets::mi;
    ui.separator();
    for entry in entries {
        let params = json!({"id": m.id, "index": index});
        // Each label whole, for the catalogue; a product's name in each of the editors', which no language translates.
        match entry {
            Entry::Drive => mi(ui, app, "Save to Drive…", TO_DRIVE, params),
            Entry::Scan => mi(ui, app, "Send to Scanned Documents", TO_SCANS, params),
            Entry::Edit(Editor::Document) => mi(ui, app, "Open in NeoDoc", IN_EDITOR, params),
            Entry::Edit(Editor::Spreadsheet) => mi(ui, app, "Open in NeoGrille", IN_EDITOR, params),
            Entry::Edit(Editor::Presentation) => mi(ui, app, "Open in NeoPrés", IN_EDITOR, params),
            Entry::Edit(Editor::Pdf) => mi(ui, app, "Open in NeoPDF", IN_EDITOR, params),
        }
    }
    ui.separator();
}

#[cfg(test)]
mod tests;
