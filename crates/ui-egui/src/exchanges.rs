// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the Échanges pane's state, what a host is asked, and the strip under a sender.

//! What was said with the person whose message is being read: their exchanges (what they wrote, what was written to them), their
//! files, and what Neoffice links them to.
//!
//! The pane (`exchanges_pane.rs`) follows the message in the reading pane. It lists, as one, **what the window has in
//! memory** (`mailcraft_model::exchanges::local`: the newest messages of each folder a Send/Receive read) and **what the server holds
//! beyond it**, a page at a time. The server's part is asked of the host that hosts the window (the Neoffice page, Neodesk): this file
//! keeps what was asked and what came ([`Exchanges`]), and says what is still to be asked ([`Exchanges::take_wants`]); the host asks,
//! and puts the answers back ([`Exchanges::put_someone`], [`Exchanges::put_page`], [`Exchanges::put_linked`]). Without a host that
//! reaches the server, the pane lists what the window has, and says so.
//!
//! The pane stands where Nora's does (the right of the window), and the two never stand together: opening one closes the other.
//! A strip under the sender of a message ([`strip`]) says who the sender is in Neoffice and how many exchanges there are, and opens
//! the pane.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Margin, RichText, Sense, Stroke, Ui, vec2};
use mailcraft_model::exchanges::{self as data, Linked, Page, Row, Someone};
use mailcraft_model::{FolderKind, Id, Message};
use serde_json::{Value, json};

use crate::MailApp;
use crate::reading::Place;
use crate::theme::{Tokens, regular, semibold};

/// How long after a failure a thing is asked again (milliseconds).
const RETRY_MS: f64 = 30_000.0;
/// The pane's width when the person has not set one.
pub(crate) const WIDTH: f32 = 380.0;

/// What the pane shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    /// The messages.
    #[default]
    Exchanges,
    /// The files that came with them.
    Files,
    /// What Neoffice links the person to.
    Neoffice,
}

/// Which of the exchanges are listed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Show {
    #[default]
    All,
    /// What came in.
    In,
    /// What went out.
    Out,
}

impl Show {
    pub(crate) fn lets(self, way: data::Way) -> bool {
        matches!((self, way), (Show::All, _) | (Show::In, data::Way::In) | (Show::Out, data::Way::Out))
    }
}

/// Why something asked of the server came to nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The server does not have what is asked (an older Neoffice, a site without it): never asked again.
    Unsupported,
    /// Another time.
    Failed,
}

/// What the pane needs the host to ask of the Neoffice server, for the mailbox `mailbox` (its address, in lower case).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Want {
    /// `NeomailContact/get`: who the address is, and every address of that someone.
    Someone { mailbox: String, address: String },
    /// `NeomailHistory/get`: the messages with these addresses, the page that starts after `before` (the first when `None`). The
    /// answer is put back under `key`.
    Page { mailbox: String, key: String, addresses: Vec<String>, before: Option<String>, cc: bool, domain: bool },
    /// `NeomailLinked/get`: what Neoffice links the address to.
    Linked { mailbox: String, address: String },
}

/// What was asked of one thing, and how it went.
#[derive(Clone, Debug)]
pub(crate) enum Asked<T> {
    Waiting,
    Ready(T),
    /// When it failed (milliseconds).
    Failed(f64),
    Unsupported,
}

/// Where the server's pages of one someone's exchanges stand.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Fetch {
    /// Nothing is on its way.
    #[default]
    Idle,
    /// A page is asked for.
    Loading,
    /// It failed, at this time (milliseconds).
    Failed(f64),
}

/// What the server brought of one someone's exchanges (under the key of their addresses and the options).
#[derive(Clone, Debug, Default)]
pub(crate) struct Remote {
    pub(crate) rows: Vec<Row>,
    /// The first page came.
    pub(crate) loaded: bool,
    /// What to ask `before` for the next page; `None` once there is no more.
    pub(crate) next: Option<String>,
    pub(crate) total: Option<u32>,
    pub(crate) folders: (u32, u32),
    pub(crate) partial: bool,
    pub(crate) fetch: Fetch,
}

/// The Échanges pane's state, in the window.
#[derive(Default)]
pub struct Exchanges {
    pub(crate) open: bool,
    pub(crate) tab: Tab,
    pub(crate) width: f32,
    /// The words typed in the pane's search box.
    pub(crate) text: String,
    pub(crate) show: Show,
    /// Also the messages that only copied them, and the whole company's domain (what the server searches for).
    pub(crate) cc: bool,
    pub(crate) domain: bool,
    /// A host that reaches the server hosts the window: the pane asks it for more than the window has.
    hosted: bool,
    someone: BTreeMap<String, Asked<Someone>>,
    linked: BTreeMap<String, Asked<Linked>>,
    remote: BTreeMap<String, Remote>,
    wants: Vec<Want>,
}

/// The key under which a someone's pages are kept: their addresses and what is searched.
pub(crate) fn remote_key(addresses: &[String], cc: bool, domain: bool) -> String {
    format!("{}|{}|{}", addresses.join(","), u8::from(cc), u8::from(domain))
}

impl Exchanges {
    /// Tell the pane whether a host reaches the Neoffice server for it (a mailbox whose session offers the history). Without one the
    /// pane lists only what the window has.
    pub fn set_hosted(&mut self, hosted: bool) {
        self.hosted = hosted;
    }

    /// Whether a host reaches the server.
    pub fn hosted(&self) -> bool {
        self.hosted
    }

    /// Whether something is still to be asked of the server.
    pub fn has_wants(&self) -> bool {
        !self.wants.is_empty()
    }

    /// What is still to be asked of the server: the host asks it, and puts each answer back.
    pub fn take_wants(&mut self) -> Vec<Want> {
        std::mem::take(&mut self.wants)
    }

    /// The answer to a [`Want::Someone`].
    pub fn put_someone(&mut self, address: &str, answer: Result<Someone, Trouble>) {
        let entry = match answer {
            Ok(someone) => Asked::Ready(someone),
            Err(Trouble::Unsupported) => Asked::Unsupported,
            Err(Trouble::Failed) => Asked::Failed(crate::now_ms()),
        };
        self.someone.insert(data::key(address), entry);
    }

    /// The answer to a [`Want::Linked`].
    pub fn put_linked(&mut self, address: &str, answer: Result<Linked, Trouble>) {
        let entry = match answer {
            Ok(linked) => Asked::Ready(linked),
            Err(Trouble::Unsupported) => Asked::Unsupported,
            Err(Trouble::Failed) => Asked::Failed(crate::now_ms()),
        };
        self.linked.insert(data::key(address), entry);
    }

    /// The answer to a [`Want::Page`], under its `key`; `more`: it was the page after another's (its rows are added to what came).
    pub fn put_page(&mut self, key: &str, more: bool, answer: Result<Page, Trouble>) {
        let entry = self.remote.entry(key.to_string()).or_default();
        match answer {
            Ok(page) => {
                entry.rows = if more { data::merge(std::mem::take(&mut entry.rows), &page.rows) } else { page.rows };
                entry.loaded = true;
                entry.next = page.next;
                entry.total = page.total.or(entry.total);
                entry.folders = page.folders;
                entry.partial = page.partial;
                entry.fetch = Fetch::Idle;
            }
            Err(Trouble::Unsupported) => {
                entry.fetch = Fetch::Idle;
                self.hosted = false;
            }
            Err(Trouble::Failed) => entry.fetch = Fetch::Failed(crate::now_ms()),
        }
    }

    /// Who `address` is, asking the server when it has not been (and when a failure is old enough to try again): `None` while it
    /// is not known, and always where no host reaches the server.
    pub(crate) fn someone_of(&mut self, now: f64, mailbox: &str, address: &str) -> Option<Someone> {
        if !self.hosted {
            return None;
        }
        let key = data::key(address);
        match self.someone.get(&key) {
            Some(Asked::Ready(someone)) => return Some(someone.clone()),
            Some(Asked::Waiting | Asked::Unsupported) => return None,
            Some(Asked::Failed(at)) if now - at < RETRY_MS => return None,
            _ => {}
        }
        self.someone.insert(key, Asked::Waiting);
        self.wants.push(Want::Someone { mailbox: mailbox.to_string(), address: address.to_string() });
        None
    }

    /// What Neoffice links `address` to, asked as [`Exchanges::someone_of`] asks who it is.
    pub(crate) fn linked_of(&mut self, now: f64, mailbox: &str, address: &str) -> Asked<Linked> {
        if !self.hosted {
            return Asked::Unsupported;
        }
        let key = data::key(address);
        match self.linked.get(&key) {
            Some(Asked::Ready(linked)) => return Asked::Ready(linked.clone()),
            Some(Asked::Waiting) => return Asked::Waiting,
            Some(Asked::Unsupported) => return Asked::Unsupported,
            Some(Asked::Failed(at)) if now - at < RETRY_MS => return Asked::Failed(*at),
            _ => {}
        }
        self.linked.insert(key, Asked::Waiting);
        self.wants.push(Want::Linked { mailbox: mailbox.to_string(), address: address.to_string() });
        Asked::Waiting
    }

    /// The server's pages of the exchanges with `addresses`, the first asked for when they have not been (and again when a
    /// failure is old enough): what came so far.
    pub(crate) fn remote_of(&mut self, now: f64, mailbox: &str, addresses: &[String]) -> Remote {
        if !self.hosted {
            return Remote::default();
        }
        let key = remote_key(addresses, self.cc, self.domain);
        let (cc, domain) = (self.cc, self.domain);
        let entry = self.remote.entry(key.clone()).or_default();
        let ask = match entry.fetch {
            Fetch::Idle => !entry.loaded,
            Fetch::Loading => false,
            Fetch::Failed(at) => !entry.loaded && now - at >= RETRY_MS,
        };
        if ask {
            entry.fetch = Fetch::Loading;
            self.wants.push(Want::Page { mailbox: mailbox.to_string(), key, addresses: addresses.to_vec(), before: None, cc, domain });
        }
        self.remote.get(&remote_key(addresses, cc, domain)).cloned().unwrap_or_default()
    }

    /// The next page of the exchanges with `addresses`, when there is one and none is on its way.
    pub(crate) fn ask_more(&mut self, mailbox: &str, addresses: &[String]) {
        let key = remote_key(addresses, self.cc, self.domain);
        let (cc, domain) = (self.cc, self.domain);
        let Some(entry) = self.remote.get_mut(&key) else { return };
        if matches!(entry.fetch, Fetch::Loading) {
            return;
        }
        let Some(before) = entry.next.clone() else { return };
        entry.fetch = Fetch::Loading;
        self.wants.push(Want::Page { mailbox: mailbox.to_string(), key, addresses: addresses.to_vec(), before: Some(before), cc, domain });
    }

    /// Forget a page that failed, so that it is asked again at the next look.
    pub(crate) fn retry(&mut self, addresses: &[String]) {
        let key = remote_key(addresses, self.cc, self.domain);
        if let Some(entry) = self.remote.get_mut(&key)
            && matches!(entry.fetch, Fetch::Failed(_))
        {
            entry.fetch = Fetch::Idle;
        }
    }
}

/// Whether the pane stands at the right of the window now (the mail beside it makes room: `mail_view::message_room`).
pub(crate) fn shown(app: &MailApp) -> bool {
    app.exchanges.open
}

/// `ui.exchanges {"open"?: bool}`: open or close the Échanges pane (without `open`, the other way round). Opening it closes Nora's.
pub(crate) fn toggle(app: &mut MailApp, p: &Value) -> Result<Value, String> {
    let open = match p.get("open") {
        None | Some(Value::Null) => !app.exchanges.open,
        Some(open) => open.as_bool().ok_or_else(|| "`open` is true or false".to_string())?,
    };
    if open && !app.exchanges.open && crate::nora::assist::shown(app) {
        // The two panes share the right of the window.
        let _ = crate::nora::assist::toggle(app, &json!({"open": false}));
    }
    app.exchanges.open = open;
    Ok(json!({"open": open}))
}

/// The window commands for the pane (`UI_COMMANDS` lists them).
pub(crate) fn ui_command(app: &mut MailApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    match id {
        "ui.exchanges" => Some(toggle(app, p)),
        _ => None,
    }
}

/// The person a message is about, for the pane: who wrote it, or, for a message the mailbox's owner wrote, who it went to first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    /// The mailbox it is in, by its address in lower case: what the host names a session by.
    pub(crate) mailbox: String,
    /// The someone's address, in lower case.
    pub(crate) address: String,
    pub(crate) name: String,
    /// The message the pane follows.
    pub(crate) message: Id,
}

/// Who the message `m` is about, in the pane; `None` for one with nobody else (a message the owner sent to nobody, or to themselves).
pub(crate) fn target_of(app: &MailApp, m: &Message) -> Option<Target> {
    let mb = &app.session.mailbox;
    let account = mb.account(m.account)?;
    let mine = data::key(&account.email);
    let sent = mb.folder(m.folder).is_some_and(|f| f.kind == FolderKind::Sent);
    let other = if sent || data::key(&m.from.email) == mine { m.to.first().unwrap_or(&m.from) } else { &m.from };
    let address = data::key(&other.email);
    if address.is_empty() || address == mine {
        return None;
    }
    Some(Target { mailbox: mine, address, name: other.name.trim().to_string(), message: m.id })
}

/// The window's own exchanges with `addresses`, kept until the mailbox changes.
pub(crate) fn local_rows(app: &MailApp, ctx: &egui::Context, addresses: &[String]) -> Arc<Vec<Row>> {
    type Cached = ((u64, usize, Vec<String>), Arc<Vec<Row>>);
    let slot = egui::Id::unique("exchanges_local_rows");
    let key = (app.session.rev(), app.session.mailbox.messages.len(), addresses.to_vec());
    if let Some((cached, list)) = ctx.data(|d| d.get_temp::<Cached>(slot))
        && cached == key
    {
        return list;
    }
    let list = Arc::new(data::local(&app.session.mailbox, addresses, data::MAX_LOCAL));
    ctx.data_mut(|d| d.insert_temp::<Cached>(slot, (key, list.clone())));
    list
}

/// The addresses to look for, and who the someone is: what the server says when it has, else the address the message has.
pub(crate) fn who(app: &mut MailApp, t: &Target) -> (Vec<String>, Option<Someone>) {
    let someone = app.exchanges.someone_of(crate::now_ms(), &t.mailbox, &t.address);
    let addresses = match &someone {
        Some(s) if !s.addresses.is_empty() => s.addresses.clone(),
        _ => vec![t.address.clone()],
    };
    (addresses, someone)
}

/// The colours of Neoffice's mark in this theme (the pill that says a sender is known there): the signal for a field, the ink on it.
pub(crate) fn mark(t: &Tokens) -> (egui::Color32, egui::Color32) {
    let ink = egui::Color32::from_rgb(0x14, 0x14, 0x14);
    if t.dark { (egui::Color32::from_rgb(0xF5, 0x5D, 0x1A), ink) } else { (egui::Color32::from_rgb(0xFF, 0xEA, 0xE2), ink) }
}

/// The strip between a message's header and its body: who the sender is in Neoffice, how many exchanges the window has with them,
/// and the way to open the pane.
pub(crate) fn strip(app: &mut MailApp, ui: &mut Ui, m: &Message, place: Place) {
    if place != Place::Pane || app.session.compose.is_some() || app.exchanges.open {
        return;
    }
    let Some(target) = target_of(app, m) else { return };
    let (addresses, someone) = who(app, &target);
    if app.exchanges.has_wants() {
        ui.ctx().request_repaint();
    }
    let rows = local_rows(app, ui.ctx(), &addresses);
    let others: Vec<&Row> = rows.iter().filter(|r| r.message != Some(m.id)).collect();
    let party = someone.as_ref().and_then(|s| s.party.clone());
    let lead = someone.as_ref().and_then(|s| s.lead.clone());
    if others.is_empty() && party.is_none() && lead.is_none() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let (fill, ink) = mark(&t);
    let latest =
        others.first().map(|r| crate::dates::list_label(r.when, app.session.now(), app.session.offset_at(r.when), app.session.prefs.twelve_hour));
    let count = others.len();
    let line = match (&party, &lead) {
        (Some(p), _) => {
            let invoices =
                p.invoices.map(|n| format!(" · {}", tln!(n as usize, "{n} invoice this year", "{n} invoices this year"))).unwrap_or_default();
            format!("{} · {}{}", p.kind, p.title, invoices)
        }
        (None, Some((title, _))) => format!("{} · {}", tl!("Lead"), title),
        (None, None) => String::new(),
    };
    let on = !line.is_empty();
    let shown = egui::Frame::NONE.inner_margin(Margin { left: 20, right: 12, top: 6, bottom: 6 }).show(ui, |ui| {
        egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(Margin::symmetric(8, 5)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 2.0);
                if on {
                    egui::Frame::NONE.fill(fill).corner_radius(10.0).inner_margin(Margin::symmetric(8, 1)).show(ui, |ui| {
                        ui.label(RichText::new("Neoffice").font(semibold(11.0)).color(ink));
                    });
                    ui.add(egui::Label::new(RichText::new(&line).font(regular(12.0)).color(t.text)).truncate());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(tl!("Show the history")).font(semibold(12.0)).color(t.accent_text));
                    if count > 0 {
                        let said = match &latest {
                            Some(when) => crate::i18n::fmt(
                                &tln!(count, "{n} exchange, the latest {when}", "{n} exchanges, the latest {when}"),
                                &[("when", when.as_str())],
                            ),
                            None => tln!(count, "{n} exchange", "{n} exchanges"),
                        };
                        ui.add(egui::Label::new(RichText::new(said).font(regular(12.0)).color(t.text_dim)).truncate());
                    }
                });
            });
        });
    });
    let resp = ui.interact(shown.response.rect, ui.scope_id().with("exchanges_strip").with(m.id), Sense::click());
    app.hit.add("exchangesStrip", m.id, shown.response.rect);
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        let _ = app.run("ui.exchanges", json!({"open": true}));
    }
}

#[cfg(test)]
#[path = "exchanges_tests.rs"]
mod tests;
