// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Nouveaux expéditeurs » over the list of variant D.

//! « Nouveaux expéditeurs (N) › » over the Inbox's list of variant D (`super::spark_list`), as Spark has its gatekeeper: the people
//! who wrote to the Inbox in view (every one of « Toutes les boîtes ») in the last [`DAYS`] days and that nothing says the person
//! knows: in neither list of Junk E-mail Options (Safe Senders, Blocked Senders: the account's, `Mailbox::junk_lists`), not one of
//! the contacts, never written to (no message of Sent Items to them here). A click opens their cards: each says who, how many
//! messages and the last subject, with « Accepter » (the sender goes to Safe Senders: `junk.neverBlockSender`) and « Bloquer »
//! (to Blocked Senders, their messages to Junk E-mail: `junk.blockSender`). The lists are each mailbox's, which a hosting page's
//! window keeps on its server (kept.rs of the browser build).
//!
//! Nor someone who wrote before those days or whose mail was answered: they are known already.
//!
//! What the window knows of « never written to » is what Sent Items has here: the newest of it, the window of a JMAP account
//! (`prefs.sync_limit`); someone written to long ago, before that window, is taken for new.

use std::collections::{HashMap, HashSet};

use egui::{Color32, CornerRadius, Frame, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::Session;
use mailcraft_model::{Address, FolderKind, Id};
use serde_json::json;

use crate::MailApp;
use crate::theme::{Tokens, paint_clipped, regular, semibold};

/// How far back a sender counts as new.
pub(crate) const DAYS: i64 = 14;
/// The band over the list: its height and the room around it.
const BAND_H: f32 = 38.0;
const BAND_ROOM: f32 = 8.0;

/// A sender nothing says the person knows, of one mailbox.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NewSender {
    pub(crate) who: Address,
    pub(crate) account: Id,
    /// Their messages in the Inbox, newest first.
    pub(crate) ids: Vec<Id>,
    pub(crate) subject: String,
}

/// The new senders of the Inboxes in view, the one who wrote last first.
pub(crate) fn of(s: &Session) -> Vec<NewSender> {
    let mb = &s.mailbox;
    let inboxes: Vec<Id> = s
        .all_inboxes()
        .unwrap_or_else(|| vec![s.view.folder])
        .into_iter()
        .filter(|f| mb.folder(*f).is_some_and(|f| f.kind == FolderKind::Inbox))
        .collect();
    if inboxes.is_empty() {
        return Vec::new();
    }
    let since = s.now() - DAYS * 86_400;
    let fold = |email: &str| email.trim().to_lowercase();
    let own: HashSet<String> = mb.accounts.iter().map(|a| fold(&a.email)).filter(|e| !e.is_empty()).collect();
    let contacts: HashSet<String> = mb.contacts.values().flat_map(|c| c.emails.iter().map(|e| fold(&e.value))).collect();
    let sent: HashSet<Id> = mb.folders.iter().filter(|f| f.kind == FolderKind::Sent).map(|f| f.id).collect();
    let written: HashSet<String> = mb
        .messages
        .values()
        .filter(|m| sent.contains(&m.folder))
        .flat_map(|m| m.to.iter().chain(&m.cc).chain(&m.bcc).map(|a| fold(&a.email)))
        .collect();
    // Who wrote before those days, or whose mail was answered, is known too (outside Junk E-mail and Deleted Items).
    let away: HashSet<Id> = mb.folders.iter().filter(|f| matches!(f.kind, FolderKind::Junk | FolderKind::Deleted)).map(|f| f.id).collect();
    let earlier: HashSet<String> = mb
        .messages
        .values()
        .filter(|m| !away.contains(&m.folder) && (m.answered || mailcraft_engine::list::when(m) < since))
        .map(|m| fold(&m.from.email))
        .collect();
    let mut found: HashMap<(Id, String), NewSender> = HashMap::new();
    for inbox in inboxes {
        for m in mb.messages_in(inbox) {
            let email = fold(&m.from.email);
            if m.draft || email.is_empty() || mailcraft_engine::list::when(m) < since {
                continue;
            }
            if own.contains(&email) || contacts.contains(&email) || written.contains(&email) || earlier.contains(&email) {
                continue;
            }
            if mb.junk_lists(m.account).is_some_and(|lists| lists.verdict(&email).is_some()) {
                continue;
            }
            let entry = found.entry((m.account, email)).or_insert_with(|| NewSender {
                who: m.from.clone(),
                account: m.account,
                ids: Vec::new(),
                subject: String::new(),
            });
            entry.ids.push(m.id);
        }
    }
    let mut out: Vec<NewSender> = found.into_values().collect();
    let newest = |id: &Id| mb.message(*id).map(|m| (mailcraft_engine::list::when(m), m.id));
    for sender in &mut out {
        sender.ids.sort_by_key(|id| std::cmp::Reverse(newest(id)));
        if let Some(m) = sender.ids.first().and_then(|id| mb.message(*id)) {
            sender.subject = m.subject.trim().to_string();
            // As they wrote their name last.
            sender.who = m.from.clone();
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.ids.first().and_then(newest)));
    out
}

/// The new senders, worked out once for each state of the mailbox and of the view (`Session::rev`, the folder, the view).
fn cached(app: &MailApp, ui: &Ui) -> std::sync::Arc<Vec<NewSender>> {
    let key = egui::Id::unique("new_senders_of");
    let stamp = (app.session.rev(), app.session.view.folder, app.session.all_inboxes().is_some(), app.session.today().to_days());
    if let Some((was, list)) = ui.data(|d| d.get_temp::<((u64, Id, bool, i64), std::sync::Arc<Vec<NewSender>>)>(key))
        && was == stamp
    {
        return list;
    }
    let list = std::sync::Arc::new(of(&app.session));
    ui.data_mut(|d| d.insert_temp(key, (stamp, list.clone())));
    list
}

/// Whether the band is offered: the list of variant D on an Inbox (or « Toutes les boîtes »), its main view (no open group, no
/// search).
fn offered(app: &MailApp) -> bool {
    super::spark_list(app)
        && app.session.view.search.trim().is_empty()
        && super::bundle::open_group(app.session.view.filter).is_none()
        && app.session.mailbox.folder(app.session.view.folder).is_some_and(|f| f.kind == FolderKind::Inbox)
}

/// Whether the cards are open.
fn open_id() -> egui::Id {
    egui::Id::unique("new_senders_open")
}

/// The band « Nouveaux expéditeurs (N) › » over the list, where there are some; a click opens their cards.
pub(super) fn band(app: &mut MailApp, ui: &mut Ui) {
    // (Its cards close with it: they don't open again by themselves with the next new sender.)
    if !offered(app) {
        ui.data_mut(|d| d.insert_temp(open_id(), false));
        return;
    }
    let senders = cached(app, ui);
    if senders.is_empty() {
        ui.data_mut(|d| d.insert_temp(open_id(), false));
        return;
    }
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAND_H + BAND_ROOM), Sense::hover());
    let band =
        Rect::from_min_max(pos2(r.min.x + 12.0, r.min.y + BAND_ROOM / 2.0), pos2(r.max.x - super::lane_width(app) - 8.0, r.max.y - BAND_ROOM / 2.0));
    let resp = ui.interact(band, ui.scope_id().with("new_senders_band"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let title = crate::i18n::fmt(tl!("New senders ({count})"), &[("count", &senders.len().to_string())]);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &title));
    app.hit.add("newSenders", 0, band);
    let p = ui.painter();
    p.rect(
        band,
        CornerRadius::same(10),
        if resp.hovered() { t.hover } else { t.surface_sunken },
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    let x = band.min.x + 14.0;
    let w = paint_clipped(p, pos2(x, band.center().y), (band.width() - 60.0).max(0.0), &title, semibold(14.0), t.text);
    let said = tl!("Accept or block them before they come in here");
    paint_clipped(p, pos2(x + w + 12.0, band.center().y), (band.max.x - 30.0 - x - w - 12.0).max(0.0), said, regular(13.0), t.text_dim);
    let c = pos2(band.max.x - 16.0, band.center().y);
    p.line(vec![c + vec2(-2.5, -5.0), c + vec2(2.5, 0.0), c + vec2(-2.5, 5.0)], Stroke::new(1.6, t.text_dim));
    if resp.clicked() {
        ui.data_mut(|d| d.insert_temp(open_id(), true));
    }
    cards(app, ui, &senders);
}

/// The cards, in a window over the page while they are open: each sender with « Accepter » and « Bloquer ».
fn cards(app: &mut MailApp, ui: &mut Ui, senders: &[NewSender]) {
    if !ui.data(|d| d.get_temp::<bool>(open_id())).unwrap_or(false) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let mut close = false;
    let mut act: Option<(&'static str, Vec<Id>)> = None;
    let modal = egui::Modal::new(egui::Id::unique("new_senders_cards"))
        .backdrop_color(Color32::from_black_alpha(110))
        .frame(Frame::new().fill(t.surface).corner_radius(CornerRadius::same(16)).inner_margin(Margin::same(22)))
        .show(&ctx, |ui| {
            ui.set_width(520.0_f32.min(ctx.content_rect().width() - 80.0).max(260.0));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("New senders")).font(semibold(18.0)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let x = ui.add(egui::Button::new(tl!("Close")).frame(false));
                    app.hit.add("newSendersClose", 0, x.rect);
                    if x.clicked() {
                        close = true;
                    }
                });
            });
            ui.label(
                egui::RichText::new(tl!("They wrote to you in the last two weeks: neither a contact, nor someone you wrote to."))
                    .font(regular(13.0))
                    .color(t.text_dim),
            );
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.6).show(ui, |ui| {
                for (i, sender) in senders.iter().enumerate() {
                    if let Some(chosen) = card(app, ui, &t, sender, i) {
                        act = Some(chosen);
                    }
                }
            });
        });
    if modal.should_close() {
        close = true;
    }
    if let Some((command, ids)) = act {
        let _ = app.run(command, json!({"ids": ids}));
    }
    if close {
        ui.data_mut(|d| d.insert_temp(open_id(), false));
    }
}

/// A sender's card: who, how many and the last subject, « Accepter » and « Bloquer ». What was clicked: the command and its ids.
fn card(app: &mut MailApp, ui: &mut Ui, t: &Tokens, sender: &NewSender, i: usize) -> Option<(&'static str, Vec<Id>)> {
    let mut chosen = None;
    Frame::new().fill(t.surface_sunken).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(12)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (picture, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                if let Some(m) = sender.ids.first().and_then(|id| app.session.mailbox.message(*id)).cloned() {
                    crate::sender_images::avatar(app, ui, picture.center(), 17.0, &m, &sender.who);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let name = sender.who.display().to_string();
                    ui.label(egui::RichText::new(&name).font(semibold(14.0)).color(t.text));
                    if !sender.who.name.trim().is_empty() {
                        ui.label(egui::RichText::new(sender.who.email.trim()).font(regular(12.0)).color(t.text_dim));
                    }
                    let count = tln!(sender.ids.len(), "{n} message", "{n} messages");
                    let said = if sender.subject.is_empty() { count } else { format!("{count} · {}", sender.subject) };
                    ui.add(egui::Label::new(egui::RichText::new(said).font(regular(12.0)).color(t.text_dim)).truncate());
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let accept = ui.add(egui::Button::new(egui::RichText::new(tl!("Accept")).font(semibold(13.0))).corner_radius(CornerRadius::same(14)));
                app.hit.add("newSenderAccept", i as u64, accept.rect);
                let block = ui.add(
                    egui::Button::new(egui::RichText::new(tl!("Block")).font(semibold(13.0)).color(t.red)).corner_radius(CornerRadius::same(14)),
                );
                app.hit.add("newSenderBlock", i as u64, block.rect);
                if accept.clicked() {
                    chosen = sender.ids.first().map(|id| ("junk.neverBlockSender", vec![*id]));
                }
                if block.clicked() {
                    chosen = Some(("junk.blockSender", sender.ids.clone()));
                }
            });
        },
    );
    ui.add_space(8.0);
    chosen
}
