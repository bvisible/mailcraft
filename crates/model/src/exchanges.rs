// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what was said with someone, for the Échanges pane.

//! What was said with someone: the messages they sent and those sent to them, for a pane that follows the message being read.
//!
//! Two sources, one list. **The window's own mailbox** says what it has in memory ([`local`]: the newest messages of each folder
//! that a Send/Receive read, as a search for the address would find them). **The server** says the rest, a page at a time
//! (`NeomailHistory/get` of the Neoffice server, `apps/mailcraft-web/src/exchanges.rs`; [`read_page`]), and who the address is in
//! Neoffice (`NeomailContact/get`, [`read_someone`]) and what Neoffice links it to (`NeomailLinked/get`, [`read_linked`]). [`merge`] puts
//! the two together: a message both know is the window's own, the one that opens at once.
//!
//! Everything a server says is a stranger's words (a subject, a name) and is read as text only, each field capped.

use std::collections::BTreeSet;

use mailcraft_time::{Timestamp, iso_to_timestamp};
use serde_json::Value;

use crate::{FolderKind, Id, Mailbox, Message};

/// Most addresses one someone has (what the server gives, and what the window searches for).
pub const MAX_ADDRESSES: usize = 8;
/// Most rows the window's own mailbox gives for one someone.
pub const MAX_LOCAL: usize = 500;
/// Most rows one page of the server holds.
pub const MAX_PAGE_ROWS: usize = 50;
/// Most documents the server says Neoffice links someone to.
pub const MAX_DOCS: usize = 12;
/// Most characters of any text taken from an answer.
const MAX_TEXT: usize = 300;
/// Most files read for one message.
const MAX_FILES: usize = 8;

/// Whether a message came in, or went out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    In,
    Out,
}

/// A file attached to a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub mime: String,
    pub size: u64,
}

/// One exchange: a message sent to or by someone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The window's own message, when it has it.
    pub message: Option<Id>,
    /// Its id on the server, when the server gave one (what opens it when the window has not got it).
    pub server: Option<String>,
    pub subject: String,
    /// The other side: who wrote it, or for a message that went out, who it went to first.
    pub name: String,
    pub email: String,
    pub when: Timestamp,
    /// The folder it is in, by the name the mailbox gives it, and what kind it is when it is a special one.
    pub folder: String,
    pub kind: Option<FolderKind>,
    pub way: Way,
    pub unread: bool,
    /// It has files (the paperclip), whether or not their names are known yet.
    pub clip: bool,
    pub files: Vec<File>,
}

/// An address as a key: what the server compares by.
pub fn key(address: &str) -> String {
    address.trim().to_lowercase()
}

/// The addresses to look for: keys, no duplicates, none empty, [`MAX_ADDRESSES`] at most.
pub fn wanted(addresses: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for address in addresses {
        let k = key(address);
        if !k.is_empty() && !out.contains(&k) && out.len() < MAX_ADDRESSES {
            out.push(k);
        }
    }
    out
}

/// A folder whose messages are not exchanges: what was thrown away, what is junk, what is not sent yet.
fn is_kept(kind: FolderKind) -> bool {
    !matches!(kind, FolderKind::Deleted | FolderKind::Junk | FolderKind::Drafts | FolderKind::Outbox)
}

fn when_of(m: &Message) -> Timestamp {
    if m.date > 0 { m.date } else { m.received }
}

fn row_of(mb: &Mailbox, m: &Message, way: Way) -> Row {
    let folder = mb.folder(m.folder);
    let other = match way {
        Way::In => &m.from,
        Way::Out => m.to.first().unwrap_or(&m.from),
    };
    let files: Vec<File> = m
        .attachments
        .iter()
        .filter(|a| !a.inline)
        .take(MAX_FILES)
        .map(|a| File { name: a.name.clone(), mime: a.mime.clone(), size: u64::try_from(a.data.len()).unwrap_or(u64::MAX) })
        .collect();
    Row {
        message: Some(m.id),
        server: m.server.as_ref().map(|s| s.id.trim().to_string()).filter(|id| !id.is_empty()),
        subject: m.subject.clone(),
        name: other.name.clone(),
        email: other.email.clone(),
        when: when_of(m),
        folder: folder.map(|f| f.name.clone()).unwrap_or_default(),
        kind: folder.map(|f| f.kind),
        way,
        unread: !m.read,
        clip: m.has_attachments(),
        files,
    }
}

/// The messages of the window's own mailbox from or to any of `addresses`, the newest first, `limit` at most: in every folder but
/// Deleted Items, Junk, Drafts and the Outbox. One pass over the mailbox.
pub fn local(mb: &Mailbox, addresses: &[String], limit: usize) -> Vec<Row> {
    let addresses = wanted(addresses);
    if addresses.is_empty() || limit == 0 {
        return Vec::new();
    }
    let theirs = |email: &str| addresses.contains(&key(email));
    let mut hits: Vec<Row> = Vec::new();
    for m in mb.messages.values() {
        if m.draft || !mb.folder(m.folder).is_some_and(|f| is_kept(f.kind)) {
            continue;
        }
        // What they wrote came in, whatever else it was sent to; the rest (to them, or copying them) went out.
        if theirs(&m.from.email) {
            hits.push(row_of(mb, m, Way::In));
        } else if m.recipients().any(|a| theirs(&a.email)) {
            hits.push(row_of(mb, m, Way::Out));
        }
    }
    hits.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| b.message.cmp(&a.message)));
    hits.truncate(limit.min(MAX_LOCAL));
    hits
}

/// The window's rows and the server's, as one list, the newest first: a message both have is the window's own (it opens at once,
/// and says whether it is read).
pub fn merge(local: Vec<Row>, more: &[Row]) -> Vec<Row> {
    let mut have: BTreeSet<String> = local.iter().filter_map(|r| r.server.clone()).collect();
    let mut out = local;
    for row in more {
        if let Some(server) = &row.server
            && !have.insert(server.clone())
        {
            continue;
        }
        out.push(row.clone());
    }
    out.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| b.server.cmp(&a.server)).then_with(|| b.message.cmp(&a.message)));
    out
}

// ---------------------------------------------------------------------------------------------------------------------------
// What the server says
// ---------------------------------------------------------------------------------------------------------------------------

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).map(|s| s.trim().chars().take(MAX_TEXT).collect()).unwrap_or_default()
}

fn flag(v: Option<&Value>) -> bool {
    v.and_then(Value::as_bool).unwrap_or(false)
}

fn count(v: Option<&Value>) -> Option<u32> {
    v.and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
}

fn kind_of(role: &str) -> Option<FolderKind> {
    match role {
        "inbox" => Some(FolderKind::Inbox),
        "sent" => Some(FolderKind::Sent),
        "drafts" => Some(FolderKind::Drafts),
        "trash" => Some(FolderKind::Deleted),
        "spam" => Some(FolderKind::Junk),
        "archive" => Some(FolderKind::Archive),
        _ => None,
    }
}

fn read_row(v: &Value) -> Option<Row> {
    let server = text(v.get("id"));
    if server.is_empty() {
        return None;
    }
    let when = iso_to_timestamp(v.get("receivedAt").and_then(Value::as_str)?, 0)?;
    let way = if text(v.get("direction")) == "out" { Way::Out } else { Way::In };
    let (name, email) = match way {
        Way::In => (text(v.get("from").and_then(|f| f.get("name"))), text(v.get("from").and_then(|f| f.get("email")))),
        Way::Out => {
            let first = v.get("to").and_then(Value::as_array).and_then(|to| to.first());
            (text(first.and_then(|f| f.get("name"))), text(first.and_then(|f| f.get("email"))))
        }
    };
    let files: Vec<File> = v
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_FILES)
        .map(|f| File { name: text(f.get("name")), mime: text(f.get("type")), size: f.get("size").and_then(Value::as_u64).unwrap_or(0) })
        .collect();
    Some(Row {
        message: None,
        server: Some(server),
        subject: text(v.get("subject")),
        name,
        email,
        when,
        folder: text(v.get("folder")),
        kind: kind_of(&text(v.get("role"))),
        way,
        unread: flag(v.get("unread")),
        clip: flag(v.get("hasAttachment")) || !files.is_empty(),
        files,
    })
}

/// A page of what the server holds of the exchanges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub rows: Vec<Row>,
    /// How many there are in all, said with the first page only.
    pub total: Option<u32>,
    /// What to ask `before` for the next page; `None` at the end.
    pub next: Option<String>,
    /// How many folders were looked through, of how many.
    pub folders: (u32, u32),
    /// The time ran out before every folder was looked through.
    pub partial: bool,
}

/// What a `NeomailHistory/get` answer says; `None` for an answer that is not one.
pub fn read_page(answer: &Value) -> Option<Page> {
    let list = answer.get("list")?.as_array()?;
    let scanned = answer.get("scanned");
    Some(Page {
        rows: list.iter().take(MAX_PAGE_ROWS).filter_map(read_row).collect(),
        total: count(answer.get("total")),
        next: Some(text(answer.get("next"))).filter(|n| !n.is_empty()),
        folders: (count(scanned.and_then(|s| s.get("folders"))).unwrap_or(0), count(scanned.and_then(|s| s.get("of"))).unwrap_or(0)),
        partial: flag(answer.get("partial")),
    })
}

/// The customer or the supplier an address is linked to in Neoffice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Party {
    pub doctype: String,
    pub name: String,
    pub title: String,
    /// What it is called in the person's language (« Client », « Fournisseur »).
    pub kind: String,
    /// The document's own page, and « Tout voir ».
    pub url: String,
    pub everything: String,
    /// How many invoices it has this year, when the person may count them.
    pub invoices: Option<u32>,
}

/// Who an address is in Neoffice, and every address of that someone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Someone {
    /// The address asked about.
    pub address: String,
    /// A contact or a lead the person may read has it.
    pub known: bool,
    /// All the addresses of that someone, the one asked first.
    pub addresses: Vec<String>,
    /// The company domains of the address (never a mail provider's).
    pub domains: Vec<String>,
    pub name: String,
    pub company: String,
    pub party: Option<Party>,
    /// A lead, when that is all it is: its title, and « Tout voir ».
    pub lead: Option<(String, String)>,
}

/// What a `NeomailContact/get` answer says.
pub fn read_someone(answer: &Value) -> Option<Someone> {
    let address = key(&text(answer.get("address")));
    if address.is_empty() {
        return None;
    }
    let strings = |field: &str| -> Vec<String> {
        answer
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|v| key(&text(Some(v))))
            .filter(|s| !s.is_empty())
            .take(MAX_ADDRESSES)
            .collect()
    };
    let mut addresses = strings("addresses");
    addresses.retain(|a| a != &address);
    addresses.insert(0, address.clone());
    addresses.truncate(MAX_ADDRESSES);
    let contact = answer.get("contact").filter(|c| c.is_object());
    let party = answer.get("party").filter(|p| p.is_object()).map(|p| Party {
        doctype: text(p.get("doctype")),
        name: text(p.get("name")),
        title: text(p.get("title")),
        kind: text(p.get("kind")),
        url: text(p.get("url")),
        everything: text(p.get("everything")),
        invoices: count(p.get("invoices")),
    });
    let lead = answer.get("lead").filter(|l| l.is_object()).map(|l| (text(l.get("title")), text(l.get("everything"))));
    Some(Someone {
        address,
        known: flag(answer.get("known")),
        addresses,
        domains: strings("domains"),
        name: text(contact.and_then(|c| c.get("fullName"))),
        company: text(contact.and_then(|c| c.get("company"))),
        party,
        lead,
    })
}

/// One thing Neoffice links someone to: a quote, an invoice, a ticket, a task.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    /// What it is called (« Facture »).
    pub kind: String,
    pub doctype: String,
    pub name: String,
    pub title: String,
    /// Its number and its date, as one line.
    pub reference: String,
    pub amount: Option<f64>,
    pub currency: String,
    pub status: String,
    /// `danger`, `warn`, `success`, `info`, `muted`, or none.
    pub tone: String,
    pub url: String,
}

/// What Neoffice links someone to, the few that wait on someone first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Linked {
    pub total: u32,
    /// « Tout voir » of the thing they are linked to.
    pub everything: String,
    pub docs: Vec<Doc>,
}

/// What a `NeomailLinked/get` answer says.
pub fn read_linked(answer: &Value) -> Option<Linked> {
    let rows = answer.get("rows")?.as_array()?;
    let docs = rows
        .iter()
        .take(MAX_DOCS)
        .filter_map(|r| {
            let name = text(r.get("name"));
            if name.is_empty() {
                return None;
            }
            let number = text(r.get("number"));
            let date = text(r.get("date"));
            let reference = [number, text(r.get("ref")), date].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            Some(Doc {
                kind: text(r.get("kind")),
                doctype: text(r.get("doctype")),
                name,
                title: text(r.get("title")),
                reference,
                amount: r.get("amount").and_then(Value::as_f64).filter(|a| a.is_finite()),
                currency: text(r.get("currency")),
                status: text(r.get("status")),
                tone: text(r.get("tone")),
                url: text(r.get("url")),
            })
        })
        .collect();
    Some(Linked { total: count(answer.get("total")).unwrap_or(0), everything: text(answer.get("thing").and_then(|t| t.get("everything"))), docs })
}

#[cfg(test)]
#[path = "exchanges_tests.rs"]
mod tests;
