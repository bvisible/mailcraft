// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading a message: its header, attachments and body, in the reading pane or in a window of
//! its own ([`show_message`] is both).
//!
//! HTML mail is shown through `mailcraft-html`'s display blocks, never executed: scripts and
//! style sheets don't exist here. What a message may decide is on a short whitelist (colours,
//! emphasis, alignment, sizes, tables; see `mailcraft-html`), and its colours are checked against
//! the window's before they are used ([`colors`]). Pictures attached to the message are decoded
//! and shown where they belong ([`images`]).
//!
//! Pictures a message refers to on the web are placeholders, so that opening a message tells its
//! sender nothing, unless the owner asks for them: for the message, for its sender or its
//! sender's domain, or for all mail ([`remote`]). Then the host downloads them (or has them
//! from the last time); nothing in this crate goes anywhere, and nothing in it keeps a file.
//!
//! The body is laid out once per message, place, width and theme ([`layout`]) and painted from
//! that every frame ([`view`]).
//!
//! The text of a message can be selected and copied, in its body and in its header ([`select`]).
//!
//! A message that carries a meeting (an invitation, an update, a cancellation, somebody's
//! answer) says so in a bar between its header and its body, with the ways to answer it
//! ([`invitation`]).

pub(crate) mod attachment_pictures;
#[cfg(test)]
mod attachment_pictures_tests;
mod backdrop;
#[cfg(test)]
mod backdrop_tests;
pub(crate) mod colors;
#[cfg(test)]
mod colors_strong_tests;
#[cfg(test)]
mod header_tests;
pub(crate) mod images;
#[cfg(test)]
mod images_tests;
mod invitation;
// Readable from `crate::fonts`' tests, which lay a message out to see what it is drawn in.
#[cfg(test)]
mod font_faces_tests;
pub(crate) mod layout;
pub(crate) mod react;
#[cfg(test)]
mod react_tests;
pub(crate) mod remote;
#[cfg(test)]
mod scroll_keys_tests;
mod select;
mod sharing;
#[cfg(test)]
mod sharing_tests;
mod signature;
#[cfg(test)]
mod signature_tests;
#[cfg(test)]
mod tests;
pub(crate) mod unsubscribe;
mod view;
mod warnings;
#[cfg(test)]
mod warnings_tests;

use std::sync::Arc;

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_html::Block;
use mailcraft_model::{FlagStatus, Importance, Message, format_list_capped};
use serde_json::json;

pub(crate) use invitation::{list_marks, options as invitation_options, question as invitation_question};
pub use remote::{Access, RemotePictures};
pub use select::Selections;
pub(crate) use select::{inspect as selection_inspect, scroll_of, select_all_params};
pub(crate) use signature::row_seal as signature_seal;
pub(crate) use view::char_at;
pub(crate) use warnings::row_mark as warning_mark;

use crate::theme::{Tokens, paint_clipped, regular, semibold};
use crate::{MailApp, icons};

/// Let go of what is kept to show messages again quickly (the window is out of sight,
/// [`crate::out_of_sight`]): the bodies laid out, the pictures' textures and bytes, the pictures
/// from the web and the faces read for messages. Each is made, asked for or read again when it
/// is next shown; what is on its way is not waited for, and not kept.
pub(crate) fn let_go(app: &mut MailApp, ctx: &egui::Context) {
    view::Store::let_go(ctx);
    app.remote_pictures.let_go(ctx);
    // A message being written keeps the faces its fonts were read for: it is laid out again with
    // every keystroke, and a face dropped under it would be read all over again (#1493).
    if app.session.composing_count() > 0 || app.session.signature_edit.is_some() {
        return;
    }
    crate::fonts::let_go(ctx);
}

/// Most recipient names the header of the reading pane lists, and most category chips it paints.
const SHOWN_NAMES: usize = 50;
const SHOWN_CATEGORIES: usize = 8;
/// Most of a message's Gmail labels shown in the reading pane (#1153).
const SHOWN_LABELS: usize = 16;

/// The widest a picture in a message's body is shown, in points. Its text fills the pane, as
/// Outlook's does, and as the body of a reply being written does above the original (#47).
pub const MAX_BODY_WIDTH: f32 = 860.0;
/// The widest a message's text is laid out, in points: wider than any screen's pane, a bound on
/// the work of laying it out, not a measure.
pub const MAX_TEXT_WIDTH: f32 = 4_000.0;

/// Where a message is shown. One that is on screen in two places is laid out for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    /// The reading pane of the main window.
    Pane,
    /// A window of its own, by the window's number.
    Window(u64),
    /// Under the owner's text in a reply or forward being written: the original it carries.
    Quote,
    /// File › Signatures: a sample reply with the signature, as a recipient sees it.
    Preview,
}

/// A message that is not in the mailbox (a message file opened without being filed, #515) in
/// the room `ui` has: as [`show_message`], through the same display blocks, but read-only and
/// asking nothing of the mailbox: its own pictures (`cid:`) are shown, pictures on the web are
/// never fetched for it, and it has no invitation or sharing bar, no sender picture from the web.
pub fn show_file_message(app: &mut MailApp, ui: &mut Ui, m: &Arc<Message>, place: Place) {
    let ctx = ui.ctx().clone();
    remote::tick(app, &ctx);
    select::begin(app, ui, m, place, true);
    let doc = view::doc_shown(&ctx, m, place, false, view::env_of(app, ui, place));
    let access = Access::Blocked;
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    header(app, ui, m, place, |app, ui, t| {
        if !doc.remote.is_empty() {
            let said = tl!("Pictures on the web aren't downloaded for a message that is not in your mailbox: move it to a folder first.");
            ui.add(egui::Label::new(egui::RichText::new(said).font(regular(11.5)).color(t.text_dim)).wrap());
        }
        unreadable_line(ui, m, doc.from_text, t, app.windows.len());
    });
    let room = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    ui.scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
        view::page(app, place, ui).show(ui, |ui| {
            egui::ScrollArea::both().auto_shrink([false, false]).id_salt(("body", place)).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 12, bottom: 24 }).show(ui, |ui| {
                    view::show(app, ui, m, place, access);
                });
                keyed_scroll(app, ui, place);
            });
        });
    });
}

/// The reading pane: the selected message, or a word on why there is none to show.
pub fn show(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let picked = app.session.sel.messages.len();
    // What showing it needs (its body, the pictures in it, a calendar part) is brought into
    // memory first, when the mailbox keeps it on disk; the message then shows whole in this
    // very frame. (One that can't be read says so in its text, once.)
    if let Some(id) = app.session.sel.focus()
        && app.session.needs_load_for_display(id)
    {
        let _ = app.session.load_for_display(id);
    }
    let Some(m) = app.session.sel.focus().and_then(|id| app.session.mailbox.messages.get(&id).cloned()) else {
        let r = ui.available_rect_before_wrap();
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(r.center().x, r.min.y + r.height() * 0.38), vec2(56.0, 56.0)),
            "mail",
            t.border_strong,
            t.border_strong,
        );
        ui.painter().text(
            pos2(r.center().x, r.min.y + r.height() * 0.38 + 50.0),
            Align2::CENTER_CENTER,
            tl!("Select an item to read"),
            regular(13.0),
            t.text_dim,
        );
        return;
    };
    if picked > 1 {
        let r = ui.available_rect_before_wrap();
        let text = tln!(picked, "{n} item selected", "{n} items selected");
        ui.painter().text(pos2(r.center().x, r.min.y + r.height() * 0.4), Align2::CENTER_CENTER, text, semibold(15.0), t.text_dim);
        return;
    }
    show_message(app, ui, &m, Place::Pane);
}

/// The bar over a draft in the reading pane: it hasn't been sent, and the way to open it in the
/// composer (the command `mail.editDraft`, as a double click runs it).
fn draft_bar(app: &mut MailApp, ui: &mut Ui, m: &Arc<Message>) {
    let t = Tokens::get(ui.ctx());
    let id = m.id;
    let r = egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin { left: 20, right: 20, top: 6, bottom: 6 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(egui::RichText::new(tl!("This message hasn't been sent.")).font(crate::theme::regular(12.0)).color(t.text));
            if ui.button(tl!("Continue editing")).clicked() {
                let _ = app.run("mail.editDraft", serde_json::json!({"id": id}));
            }
        });
    });
    app.hit.add("draftBar", 0, r.response.rect);
}

/// One message in the room `ui` has: its header, its attachments, and its body, which scrolls.
/// The reading pane shows the selected message with it, and a message window its own.
pub fn show_message(app: &mut MailApp, ui: &mut Ui, m: &Arc<Message>, place: Place) {
    let ctx = ui.ctx().clone();
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    if crate::attachment_preview::showing(app, m.id) {
        select::begin(app, ui, m, place, false);
        header(app, ui, m, place, |_, _, _| {});
        crate::attachment_preview::show(app, ui, m);
        select::settle(app, ui, place);
        return;
    }
    // A window shows its own copy of the message: what the mailbox keeps on disk is brought in
    // here, and the window takes the whole message up at its next frame.
    if place != Place::Quote && app.session.needs_load_for_display(m.id) {
        let _ = app.session.load_for_display(m.id);
    }
    remote::tick(app, &ctx);
    select::begin(app, ui, m, place, true);
    // //// Neoffice — a message Nora translated shows her translation, as plain text, for its body (nora/pane.rs); its header, files and meeting stay its own.
    let body = crate::nora::pane::shown(app, m, place);
    let doc = view::doc_shown(&ctx, &body, place, remote::available(app), view::env_of(app, ui, place));
    let access = if doc.remote.is_empty() { Access::Blocked } else { remote::access(app, &body) };
    // Asked for: what has not come yet is sent for, while the message is on screen.
    // //// Neoffice — of the body shown (above).
    let progress = if access == Access::Allowed { remote::want(app, &ctx, &body, &doc) } else { remote::Progress::default() };
    // A draft in the reading pane is not received mail: say so, and offer to go on writing it (#651).
    if place == Place::Pane && m.draft {
        draft_bar(app, ui, m);
    }
    // One waiting in the Outbox for its Send time: when it goes, and the way to change or cancel that (#950).
    if place == Place::Pane {
        crate::send_later::queued_bar(app, ui, m);
    }
    header(app, ui, m, place, |app, ui, t| {
        remote::line(app, ui, m, &doc, access, progress, place, t);
        unreadable_line(ui, m, doc.from_text, t, app.windows.len());
    });
    // A meeting that came with the message: what it is, and the ways to answer it.
    invitation::bar(app, ui, m, place);
    // //// Neoffice — Nora's strip: she is translating the message, or her translation is shown, with the way back to the original (nora/pane.rs).
    crate::nora::pane::strip(app, ui, m, place);
    // //// Neoffice — who the sender is in Neoffice, how many exchanges there are with them, and the way to open the Échanges pane (exchanges.rs).
    crate::exchanges::strip(app, ui, m, place);
    // //// Neoffice — Nora's ready replies to the message: short replies and a draft in the person's style, with fields to fill (nora/ready.rs).
    crate::nora::ready::card(app, ui, m, place);
    // A calendar that was shared with the owner, and the way to open it (#70).
    sharing::bar(app, ui, m);
    // Only its headers and a preview are here yet (R3.2): say so, and why.
    if m.unfetched {
        let t = Tokens::get(ui.ctx());
        let said = unfetched_said(app, m);
        egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin { left: 20, right: 20, top: 6, bottom: 6 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(said).font(crate::theme::regular(12.0)).color(t.text_dim));
        });
    }
    // Held in the Outbox (R3.4.2): what happened, and the owner's say-so to send it again.
    if !m.held.is_empty() {
        let t = Tokens::get(ui.ctx());
        let id = m.id;
        egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin { left: 20, right: 20, top: 6, bottom: 6 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                // The pane lays its header out without gaps; the words and the button need them.
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new(tl!("Held:")).font(crate::theme::regular(12.0)).strong().color(t.text));
                // //// Neoffice — what it says of itself in the interface's language (run_words); the mailbox keeps it in English.
                let held = crate::i18n::run_words::held(&m.held, &app.session.mailbox);
                ui.label(egui::RichText::new(held.as_ref()).font(crate::theme::regular(12.0)).color(t.text_dim));
                // A draft is sent with Send, from its own bar (#1747).
                if !m.draft && ui.button(tl!("Send Again")).clicked() {
                    let _ = app.run("mail.sendAgain", serde_json::json!({"id": id}));
                }
            });
        });
    }
    // The body gets the part of the pane that can be seen, whatever was laid out above it: a
    // header too wide for a narrow pane must not make the text run out of sight with it.
    let room = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    ui.scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
        view::page(app, place, ui).show(ui, |ui| {
            // Sideways too: a data table is never squeezed below its words, so a wide one scrolls.
            egui::ScrollArea::both().auto_shrink([false, false]).id_salt(("body", m.id)).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 12, bottom: 24 }).show(ui, |ui| {
                    // //// Neoffice — the body, or Nora's translation for it (above).
                    view::show(app, ui, &body, place, access);
                });
                keyed_scroll(app, ui, place);
            });
        });
    });
    select::settle(app, ui, place);
}

/// How far an arrow key scrolls a message's body (#1739): about two lines of its text.
const LINE_SCROLL: f32 = 40.0;

/// Scroll the body laid out in `ui` (the inside of its scroll area, after the body) as the key
/// waiting for `place` says (#1739). The scroll area keeps it within the body.
fn keyed_scroll(app: &mut MailApp, ui: &mut Ui, place: Place) {
    let Some(scroll) = app.text_selection.take_scroll(place) else { return };
    let page = (ui.clip_rect().height() - LINE_SCROLL).max(LINE_SCROLL);
    let whole = ui.min_rect().height();
    // A positive delta brings what is above or to the left into view.
    let delta = match scroll {
        select::Scroll::Line { down, across } => vec2(-f32::from(across) * LINE_SCROLL, -f32::from(down) * LINE_SCROLL),
        select::Scroll::Page(down) => vec2(0.0, -f32::from(down) * page),
        select::Scroll::Top => vec2(0.0, whole),
        select::Scroll::End => vec2(0.0, -whole),
    };
    ui.scroll_with_delta_animation(delta, egui::style::ScrollAnimation::none());
}

/// What the reading pane says of a message of which only the headers and a preview are here.
pub(crate) fn unfetched_said(app: &MailApp, m: &Message) -> &'static str {
    // //// Neoffice — a message whose text is whole here and whose files alone are on the server (a hosting page's copy of its
    // //// Neoffice — mailboxes left them out, `Message::files_on_server`): its text is shown, and this says only the files are to come.
    if m.files_on_server && !app.jmap_refused.contains(&m.account) {
        return if app.fetching.fetching(m.id) {
            tl!("Downloading the attachments…")
        } else {
            tl!("The attachments of this message are still on the server.")
        };
    }
    if app.fetching.fetching(m.id) {
        tl!("Downloading the whole message…")
    } else if app.jmap_refused.contains(&m.account) {
        // Its server turned the account's sign-in down: nothing downloads until it is given again.
        tl!("The server turned down the sign-in for this account. Use Send/Receive to sign in again.")
    } else if app.session.driver.fetch.offline && !app.session.mailbox.within_window(m, app.session.now()) {
        // Older than its mail to keep offline (#143): only its headers and preview are kept here.
        tl!("Not connected: the rest of this message is on the server")
    } else if app.session.driver.fetch.offline {
        tl!("This message isn't on this computer yet. It will download when you're online.")
    } else if app.opened_after_batch(m.id) {
        tl!("This message downloads once the messages being downloaded now are here.")
    } else {
        tl!("Only the beginning of this message is here. The rest downloads with the next Send/Receive.")
    }
}

/// Whether `m`'s subject, sender or what is shown of its body holds U+FFFD, the mark a reader
/// leaves for a character it couldn't read (bytes that aren't in the character set the message
/// names, or no name for one that isn't UTF-8 or Windows-1252). The text part counts only when it
/// is what is shown (`text_shown`, [`view::Doc::from_text`] of the document on screen): a mark in
/// the plain copy of a message read as HTML (a sender's program that lost a © there) is nothing
/// the owner sees.
pub(crate) fn has_unreadable(m: &Message, text_shown: bool) -> bool {
    const MARK: char = char::REPLACEMENT_CHARACTER;
    [m.subject.as_str(), m.from.name.as_str(), m.from.email.as_str(), m.body.html.as_deref().unwrap_or("")].iter().any(|text| text.contains(MARK))
        || (text_shown && m.body.text.contains(MARK))
}

/// Messages whose [`has_unreadable`] answer is kept ([`unreadable_line`]), at the least: more
/// with more windows open, so that every message on screen keeps its answer.
const UNREADABLE_KEPT: usize = 8;

/// The line above a message that shows a character that couldn't be read ([`has_unreadable`]):
/// why there is a � in it. One line for the whole message, so that no � needs a word of its own.
/// Looked for once per message and kept, not every frame: a body can be long.
fn unreadable_line(ui: &mut Ui, m: &Message, text_shown: bool, t: &Tokens, windows: usize) {
    // Kept for the last few messages shown (the pane and windows), under one key: not one entry
    // in egui's memory for every message ever read.
    let key = egui::Id::unique("mailcraft-unreadable");
    let this = egui::Id::unique((m.id, m.subject.len(), m.body.text.len(), m.body.html.as_ref().map_or(0, String::len), text_shown));
    let known = ui.ctx().data(|d| d.get_temp::<Vec<(egui::Id, bool)>>(key)).unwrap_or_default();
    let unreadable = match known.iter().find(|(of, _)| *of == this) {
        Some((_, kept)) => *kept,
        None => {
            let found = has_unreadable(m, text_shown);
            let mut known = known;
            if known.len() >= UNREADABLE_KEPT.max(windows.saturating_add(2)) {
                known.remove(0);
            }
            known.push((this, found));
            ui.ctx().data_mut(|d| d.insert_temp(key, known));
            found
        }
    };
    if !unreadable {
        return;
    }
    remote::notice(t.blue, t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r.shrink(1.0), "info", t.text, t.blue);
            let said = tl!("Some characters in this message couldn't be read: the sender's program didn't say properly how they were written.");
            ui.add(egui::Label::new(egui::RichText::new(said).font(regular(11.5)).color(t.text)).wrap());
        });
    });
}

/// The original a reply or forward in HTML carries below the owner's text, read-only, through the
/// same display blocks as the reading pane: nothing in it runs, its own pictures (`cid:`) are
/// shown, and pictures on the web only as the original's would be (the owner asked for them for
/// that message, its sender or its sender's domain, or for all mail; never in Junk E-mail but
/// message by message), while it is on screen.
pub(crate) fn show_quote(app: &mut MailApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let Some(m) = quote_message(app, &ctx) else { return };
    remote::tick(app, &ctx);
    select::begin(app, ui, &m, Place::Quote, true);
    let doc = view::doc_shown(&ctx, &m, Place::Quote, remote::available(app), view::env_of(app, ui, Place::Quote));
    // A quote whose original isn't known (a reopened draft, an original since deleted) has no
    // folder or sender to judge by: it never fetches anything.
    let known = app.session.compose.as_ref().and_then(|c| c.source).is_some_and(|id| app.session.mailbox.messages.contains_key(&id));
    let access = if doc.remote.is_empty() || !known { Access::Blocked } else { remote::access(app, &m) };
    if access == Access::Allowed {
        remote::want(app, &ctx, &m, &doc);
    }
    view::show(app, ui, &m, Place::Quote, access);
}

/// The carried original as a message to show: the original's sender, date and place (what its
/// pictures are allowed by), with the quote's HTML for a body and the pictures the quote takes
/// along. Kept from frame to frame while the quote stays the same, for each message being
/// written (the windows of several share the context), and let go of with it.
fn quote_message(app: &MailApp, ctx: &egui::Context) -> Option<Arc<Message>> {
    let c = app.session.compose.as_ref()?;
    let quote = c.quote.as_ref()?;
    let key = quotes_key();
    let mut kept: Vec<(u64, Arc<Message>)> = ctx.data(|d| d.get_temp(key)).unwrap_or_default();
    let source = c.source.and_then(|id| app.session.mailbox.messages.get(&id));
    // The quote's own pictures (those the original showed, for an answer; those written into it,
    // #602), and, but for an answer, the message's attachments: a forward carries the original's
    // itself, its pictures among them, and so does one opened again as a draft (a picture its
    // HTML calls by a Content-ID written otherwise, `cid:logo%40…`, stays among them).
    let answer = matches!(c.mode, mailcraft_engine::ComposeMode::Reply | mailcraft_engine::ComposeMode::ReplyAll);
    let forward = !answer || quote.pictures.is_empty();
    let attached: &[mailcraft_model::Attachment] = if forward { &c.attachments } else { &[] };
    let pictures = || quote.pictures.iter().chain(attached);
    // The same pictures: the same parts, by their ids and their very bytes, in the same order.
    let same_part = |a: &mailcraft_model::Attachment, b: &mailcraft_model::Attachment| {
        a.content_id == b.content_id && a.name == b.name && Arc::ptr_eq(&a.data, &b.data)
    };
    let same = |m: &Arc<Message>| {
        m.body.html.as_deref() == Some(quote.html.as_str())
            && m.id == source.map_or(0, |s| s.id)
            && m.attachments.len() == quote.pictures.len().saturating_add(attached.len())
            && m.attachments.iter().zip(pictures()).all(|(a, b)| same_part(a, b))
    };
    if let Some((_, m)) = kept.iter().find(|(id, m)| *id == c.id && same(m)) {
        return Some(m.clone());
    }
    let mut m = source.map(|s| (**s).clone()).unwrap_or_default();
    m.body = mailcraft_model::Body { text: quote.text.clone(), html: Some(quote.html.clone()) };
    m.attachments = pictures().cloned().collect();
    // Its body is the quote's, whole: nothing of it waits on disk.
    m.kept = None;
    let m = Arc::new(m);
    // This one's old quote goes (what is kept for a message no longer being written went at the
    // start of the frame, [`let_go_of_quotes`]).
    kept.retain(|(id, _)| *id != c.id);
    kept.push((c.id, m.clone()));
    ctx.data_mut(|d| d.insert_temp(key, kept));
    Some(m)
}

/// Where the carried originals are kept, in the context the windows share.
fn quotes_key() -> egui::Id {
    egui::Id::unique("mailcraft-compose-quote")
}

/// The carried originals of messages no longer being written go, with their pictures' bytes:
/// once a frame, so a window that closes lets go of its own even when no other is open.
fn let_go_of_quotes(app: &MailApp, ctx: &egui::Context) {
    let key = quotes_key();
    ctx.data_mut(|d| {
        let Some(kept) = d.get_temp::<Vec<(u64, Arc<Message>)>>(key) else { return };
        let left: Vec<(u64, Arc<Message>)> = kept.into_iter().filter(|(id, _)| app.session.all_composing().any(|open| open.id == *id)).collect();
        if left.is_empty() {
            d.remove::<Vec<(u64, Arc<Message>)>>(key);
        } else {
            d.insert_temp(key, left);
        }
    });
}

/// `m` shown read-only through the display blocks, as at `place` ([`Place::Preview`]): its own
/// pictures only, nothing from the web.
// //// Neoffice — kept as upstream has it, with no caller here: its two (a message being written, File › Signatures'
// //// Neoffice — sample) show the owner's own signature, pictures on the web included (show_own_signature below).
#[expect(dead_code, reason = "its callers show the owner's own signature through show_own_signature")]
pub(crate) fn show_plain_view(app: &mut MailApp, ui: &mut Ui, m: &Arc<Message>, place: Place) {
    view::show(app, ui, m, place, Access::Blocked);
}

// //// Neoffice — the owner's own signature, in a message being written (compose.rs) and in File › Signatures' sample
// //// Neoffice — (signatures.rs): shown as it goes out, its pictures on the web too. A company's signature written by the
// //// Neoffice — hosting page calls its logo and its icons by their addresses on the owner's own instance, so that every
// //// Neoffice — recipient's program fetches them; shown as stand-ins, the owner saw a card of names in the place of its
// //// Neoffice — pictures and took it for broken. The signature is the owner's, not a sender's: fetching its pictures
// //// Neoffice — tells nobody that a message was opened. Same host, checks, limits and cache as a message's pictures
// //// Neoffice — (remote.rs); `m` is one of `signatures::message_of`, under `remote::OWN_SIGNATURE`.
/// `m`, the owner's own signature, shown read-only through the display blocks, as at `place`: its own pictures, and
/// those it calls on the web, fetched while it is on screen, where this build can fetch anything.
pub(crate) fn show_own_signature(app: &mut MailApp, ui: &mut Ui, m: &Arc<Message>, place: Place) {
    let ctx = ui.ctx().clone();
    remote::tick(app, &ctx);
    let doc = view::doc_shown(&ctx, m, place, remote::available(app), view::env_of(app, ui, place));
    let own = m.id == remote::OWN_SIGNATURE && remote::available(app) && !doc.remote.is_empty();
    let access = if own { Access::Allowed } else { Access::Blocked };
    if own {
        remote::want(app, &ctx, m, &doc);
    }
    view::show(app, ui, m, place, access);
}

/// Pictures the host has brought are taken in, whatever is on screen ([`remote::tick`]), and the
/// quoted originals of messages no longer being written are let go of.
pub(crate) fn tick(app: &mut MailApp, ctx: &egui::Context) {
    let_go_of_quotes(app, ctx);
    remote::tick(app, ctx);
}

/// The message's display blocks, from its HTML when it has some, else from its text.
pub fn blocks_of(m: &Message) -> Vec<Block> {
    view::Doc::of(m).blocks
}

/// The actions on a message that is being read, as the pane's header and a message window's
/// toolbar offer them: icon, label, command. The window adds its own to these.
pub(crate) const ACTIONS: [(&str, &str, &str); 4] = [
    ("reply", "Reply", "mail.reply"),
    ("replyAll", "Reply All", "mail.replyAll"),
    ("forward", "Forward", "mail.forward"),
    ("delete", "Delete", "mail.delete"),
];

// //// Neoffice — the least room a subject is laid out in beside the message's actions (`header`): with less, its words no
// longer fit and it wrapped a letter at a time; it goes under them instead. (With more, it stays beside them, as upstream has it.)
const SUBJECT_BESIDE_MIN: f32 = 100.0;

/// Whether the date goes on a line of its own: the pane is too narrow to give the sender a
/// useful row beside it.
pub(crate) fn header_date_on_own_line(row: f32, date: f32) -> bool {
    row - date < 220.0
}

fn header(app: &mut MailApp, ui: &mut Ui, m: &Arc<Message>, place: Place, pictures: impl FnOnce(&mut MailApp, &mut Ui, &Tokens)) {
    let t = Tokens::get(ui.ctx());
    // The texts that can be selected are laid out here and painted once the pointer has had its
    // say ([`select::header`]).
    let mut fields = select::Fields::default();
    let around = egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 12, top: 12, bottom: 10 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        // What may not be what it seems about it, above everything else (#508).
        warnings::banner(app, ui, m, place, &t);
        // That it comes from a mailing list, and Unsubscribe (#1207).
        unsubscribe::bar(app, ui, m, place, &t);
        // What checking its Digital signature found (#30).
        signature::bar(app, ui, m, place, &t);
        // Subject, with the actions on the right (a window has them in its toolbar).
        let subject = if m.subject.trim().is_empty() { tl!("(no subject)") } else { m.subject.as_str() };
        // //// Neoffice — a subject left too little room beside the actions (a narrow pane, beside Nora's) goes under them,
        // whole: beside them it wrapped a letter at a time.
        let mut under = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                if place == Place::Pane {
                    for (icon, tip, id) in ACTIONS.iter().rev() {
                        let sc = crate::widgets::shortcut_text(app, id);
                        if crate::widgets::icon_button(ui, icon, &format!("{} ({sc})", tl!(tip)), 17.0, true).clicked() {
                            let _ = app.run(id, json!({}));
                        }
                        // React sits between Forward and Delete (#1544).
                        if *id == "mail.delete" {
                            ui.push_id(("react", m.id), |ui| react::button(app, ui, m));
                        }
                    }
                    // //// Neoffice — « Plus tard »'s clock, where the mailbox's server keeps it: its times (snooze_hosted.rs).
                    crate::snooze_hosted::reading_button(app, ui, m);
                    // //// Neoffice — Nora's button beside the actions, when a host reaches her: reply with her, or have her translate the message (nora/pane.rs).
                    crate::nora::pane::button(app, ui, m);
                    // //// Neoffice — and the one that opens what was said with the sender, whether or not there is a strip to open it by (exchanges.rs).
                    crate::exchanges::button(app, ui, m);
                }
                // //// Neoffice — see `under` above.
                under = ui.available_width() < SUBJECT_BESIDE_MIN;
                if !under {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        fields.add(
                            ui,
                            select::Field::Subject,
                            egui::Label::new(egui::RichText::new(subject).font(semibold(17.0)).color(t.text)).wrap(),
                        );
                    });
                }
            });
        });
        // //// Neoffice — see `under` above.
        if under {
            fields.add(ui, select::Field::Subject, egui::Label::new(egui::RichText::new(subject).font(semibold(17.0)).color(t.text)).wrap());
        }
        ui.add_space(4.0);
        // Who and when.
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
            if m.id == 0 {
                // (Not in the mailbox: nothing is asked of the web for its sender.)
                crate::widgets::avatar(ui, r.center(), 17.0, &m.from);
            } else {
                crate::sender_images::avatar(app, ui, r.center(), 17.0, m, &m.from);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                let when = crate::dates::header_label(m.date, app.session.offset_at(m.date), app.session.prefs.twelve_hour);
                // The date at the right; the name and address get what is left, cut short when
                // the pane is narrow rather than run under the date (#630). A pane too narrow
                // for both puts the date on a line of its own.
                let date_w = ui.painter().layout_no_wrap(when.clone(), regular(11.5), t.text_dim).size().x + 8.0;
                let on_own_line = header_date_on_own_line(ui.available_width(), date_w);
                let date_label = |fields: &mut select::Fields, ui: &mut Ui| {
                    fields.add(ui, select::Field::Date, egui::Label::new(egui::RichText::new(when.clone()).font(regular(11.5)).color(t.text_dim)));
                };
                let sender = |app: &mut MailApp, fields: &mut select::Fields, ui: &mut Ui| {
                    const ADDRESS_GAP: f32 = 4.0;
                    const TAG_GAP: f32 = 6.0;
                    let room = ui.available_width();
                    let external = warnings::external(app, m);
                    let tag_room = if external { warnings::external_tag_width(ui, &t) + TAG_GAP } else { 0.0 };
                    let has_address = !m.from.name.is_empty();
                    let looks_like_address = mailcraft_model::confusables::as_seen(&m.from.name, 1_000).contains('@');
                    let name = egui::RichText::new(m.from.display().to_string()).font(semibold(13.0)).color(if looks_like_address {
                        t.orange
                    } else {
                        t.text
                    });
                    let address_text = format!("<{}>", m.from.email);
                    let name_width = ui.painter().layout_no_wrap(m.from.display().to_string(), semibold(13.0), t.text).size().x;
                    let address_width = ui.painter().layout_no_wrap(address_text.clone(), regular(11.5), t.text_dim).size().x;
                    let address_on_own_line = has_address && name_width + ADDRESS_GAP + address_width + tag_room > room;

                    let name_label = |fields: &mut select::Fields, ui: &mut Ui, width: f32| {
                        ui.scope(|ui| {
                            ui.set_max_width(width.max(0.0));
                            fields.add(ui, select::Field::Sender, egui::Label::new(name).truncate());
                        });
                    };
                    let address_and_tag = |fields: &mut select::Fields, ui: &mut Ui| {
                        if has_address {
                            let width = (ui.available_width() - tag_room).max(0.0);
                            ui.scope(|ui| {
                                ui.set_max_width(width);
                                let address = egui::RichText::new(address_text).font(regular(11.5)).color(t.text_dim);
                                fields.add(ui, select::Field::Address, egui::Label::new(address).truncate());
                            });
                        }
                        if external {
                            ui.add_space(TAG_GAP);
                            let tag = warnings::external_tag(ui, &t);
                            if place == Place::Pane {
                                app.hit.add("externalTag", m.id, tag.rect);
                            }
                        }
                    };

                    if address_on_own_line {
                        name_label(fields, ui, room);
                        ui.horizontal(|ui| address_and_tag(fields, ui));
                    } else {
                        ui.horizontal(|ui| {
                            name_label(fields, ui, if has_address { name_width } else { room - tag_room });
                            if has_address {
                                ui.add_space(ADDRESS_GAP);
                            }
                            address_and_tag(fields, ui);
                        });
                    }
                };
                if on_own_line {
                    sender(app, &mut fields, ui);
                    date_label(&mut fields, ui);
                } else {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            ui.add_space(8.0);
                            date_label(&mut fields, ui);
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| sender(app, &mut fields, ui));
                        });
                    });
                }
                // (`said` is the line in the interface language, with `{names}` still to fill.)
                let mut line = |ui: &mut Ui, field: select::Field, said: &str, list: &[mailcraft_model::Address]| {
                    if !list.is_empty() {
                        // (A sender can write thousands of names; the first ones are shown.)
                        let (mut names, more) = format_list_capped(list, SHOWN_NAMES);
                        if more > 0 {
                            names.push_str(&format!("; {}", tln!(more, "and {n} more", "and {n} more")));
                        }
                        let said = crate::i18n::fmt(said, &[("names", &names)]);
                        let text = egui::RichText::new(said).font(regular(11.5)).color(t.text_dim);
                        fields.add(ui, field, egui::Label::new(text).truncate());
                    }
                };
                line(ui, select::Field::To, tl!("To: {names}"), &m.to);
                line(ui, select::Field::Cc, tl!("Cc: {names}"), &m.cc);
                line(ui, select::Field::Bcc, tl!("Bcc: {names}"), &m.bcc);
            });
        });
        notes(app, ui, m, &t, pictures);
        attachments(app, ui, m, place, &t);
    });
    select::header(app, ui, place, fields, around.response.rect);
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([pos2(r.min.x, r.min.y), pos2(r.max.x, r.min.y)], Stroke::new(1.0, t.border));
}

/// Info lines under the header: the flag, importance, what was done with the message, its
/// pictures on the web (`pictures` lays that line out), categories.
fn notes(app: &mut MailApp, ui: &mut Ui, m: &Arc<Message>, t: &Tokens, pictures: impl FnOnce(&mut MailApp, &mut Ui, &Tokens)) {
    let mut lines: Vec<(&str, String)> = Vec::new();
    match m.flag {
        FlagStatus::Flagged => {
            let said = match m.flag_due {
                Some(d) => crate::i18n::fmt(tl!("Follow up. Due {date}"), &[("date", &crate::dates::medium(app.session.local(d).date))]),
                None => tl!("Follow up.").to_string(),
            };
            lines.push(("flag", said));
        }
        FlagStatus::Complete => lines.push(("complete", tl!("Completed.").to_string())),
        FlagStatus::None => {}
    }
    match m.importance {
        Importance::High => lines.push(("highImportance", tl!("This message was sent with High importance.").to_string())),
        Importance::Low => lines.push(("lowImportance", tl!("This message was sent with Low importance.").to_string())),
        Importance::Normal => {}
    }
    // (An invitation that was answered says so, and what, in its own bar.)
    let meeting = invitation::info(app, ui.ctx(), m);
    if m.answered && !meeting.as_ref().is_some_and(|meeting| meeting.says_answer()) {
        lines.push(("reply", tl!("You replied to this message.").to_string()));
    }
    if m.forwarded {
        lines.push(("forward", tl!("You forwarded this message.").to_string()));
    }
    for (icon, text) in lines {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r.shrink(1.0), icon, t.text_dim, t.accent);
            ui.add(egui::Label::new(egui::RichText::new(text).font(regular(11.5)).color(t.text_dim)).wrap());
        });
    }
    pictures(app, ui, t);
    if !m.categories.is_empty() {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        let mut x = r.min.x;
        for c in m.categories.iter().take(SHOWN_CATEGORIES) {
            x += crate::widgets::category_chip(ui, pos2(x, r.center().y), c, crate::theme::c32(app.session.mailbox.category_color_of(m.account, c)))
                + 6.0;
        }
    }
    gmail_labels(app, ui, m);
    // The rest of the conversation, to jump to.
    // (Worked out once per change to the mailbox, not at every frame.)
    let thread = app.session.conversation(m.id);
    if thread.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            ui.spacing_mut().button_padding = vec2(10.0, 4.0);
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, t.border);
            ui.label(
                egui::RichText::new(tln!(thread.len(), "{n} message in this conversation:", "{n} messages in this conversation:"))
                    .font(regular(11.5))
                    .color(t.text_dim),
            );
            for id in thread.iter().rev().take(12) {
                let Some(other) = app.session.mailbox.message(*id) else { continue };
                let label = format!(
                    "{} · {}",
                    other.from.display(),
                    crate::dates::list_label(other.date, app.session.now(), app.session.offset_at(other.date), app.session.prefs.twelve_hour)
                );
                let button = ui.add_enabled(
                    *id != m.id,
                    egui::Button::new(egui::RichText::new(label).font(regular(11.5))).min_size(vec2(0.0, 22.0)).corner_radius(6),
                );
                app.hit.add("conversationMessage", *id, button.rect);
                if button.clicked() {
                    // The newest message of a conversation open in the list, by its own row there,
                    // not by the conversation's (#503).
                    let own = app.session.rows_in_view().is_some_and(|rows| rows.iter().any(|r| r.id == *id && r.under.is_some() && r.copy == 1));
                    let _ = app.run("mail.select", if own { json!({"id": id, "copy": 1}) } else { json!({"id": id}) });
                }
            }
        });
    }
}

/// A Gmail account's labels of the message (#1153), as tags under the categories: read only, the
/// owner's own labels but the one of the folder it is in ([`Message::shown_gmail_labels`]), each
/// by the last part of its path, the whole path in its tooltip.
fn gmail_labels(app: &mut MailApp, ui: &mut Ui, m: &Message) {
    let folder = app.session.mailbox.folder(m.folder).map(|f| f.remote.as_str()).unwrap_or_default();
    let labels: Vec<String> = m.shown_gmail_labels(folder).take(SHOWN_LABELS).map(str::to_string).collect();
    if labels.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        for label in &labels {
            let text = crate::widgets::label_tag_text(ui.painter(), mailcraft_model::gmail::label_leaf(label), 10.5, crate::widgets::LABEL_TAG_MAX);
            let (r, response) = ui.allocate_exact_size(vec2(crate::widgets::label_tag_width(&text), 18.0), Sense::hover());
            crate::widgets::paint_label_tag(ui.painter(), pos2(r.min.x, r.center().y), text, 16.0);
            app.hit.add("gmailLabel", m.id, r);
            response.on_hover_text(crate::i18n::fmt(tl!("Gmail label: {label}"), &[("label", label)]));
        }
    });
}

pub(crate) fn attachments(app: &mut MailApp, ui: &mut Ui, m: &Message, place: Place, t: &Tokens) {
    // The calendar part of a meeting is what its bar shows: it is not one of the files.
    let meeting = invitation::info(app, ui.ctx(), m);
    // The signature part of a message whose signature was checked (that very part, by its bytes)
    // is what its bar says; it is reached from the bar's Details (#30).
    let signature = app.session.digital_signature_known(m.id).and_then(|_| app.session.digital_signature_part(m.id));
    let files: Vec<(usize, &mailcraft_model::Attachment)> =
        m.listed_attachments().filter(|(index, _)| !invitation::stands_for(meeting.as_deref(), *index) && Some(*index) != signature).collect();
    if files.is_empty() {
        return;
    }
    // Which of them are shown as pictures, when the owner has them so (#834).
    let sizes = attachment_pictures::looks(app, ui.ctx(), m, place, &files);
    ui.add_space(2.0);
    // //// Neoffice — the files' rows are top-aligned (upstream: `horizontal_wrapped`, centred): egui gives every row after a
    // //// Neoffice — picture's tile the tile's height, so the rows of chips under it stood ~110 points apart, each chip
    // //// Neoffice — centred in a row as tall as the picture. Top-aligned, a row starts where the one above it ends.
    let rows = if ui.layout().prefer_right_to_left() {
        egui::Layout::right_to_left(egui::Align::Min)
    } else {
        egui::Layout::left_to_right(egui::Align::Min)
    };
    let initial = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(initial, rows.with_main_wrap(true), |ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for ((index, a), look) in files.iter().zip(&sizes) {
            match look {
                Some(size) => attachment_pictures::tile(app, ui, m, *index, a, *size, t),
                None => chip(app, ui, m, *index, a, t),
            }
        }
    });
    crate::attachment_preview::finish_click(app, ui.ctx(), m.id);
}

/// An attachment as a chip: its icon, its name and its size.
fn chip(app: &mut MailApp, ui: &mut Ui, m: &Message, index: usize, a: &mailcraft_model::Attachment, t: &Tokens) {
    // The name it is opened and saved under, not the one the sender wrote: a right-to-left
    // override can make `fdp.exe` read as `exe.pdf`.
    let shown = mailcraft_engine::opening::file_name(&a.name);
    let name_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(shown.clone(), regular(12.0), t.text).size().x).min(220.0);
    let size = vec2(name_w + 44.0 + crate::ribbon::DROP_W, 38.0);
    file_item(app, ui, m, index, &shown, size, t, |painter, main| {
        icons::paint(painter, Rect::from_center_size(pos2(main.min.x + 17.0, main.center().y), vec2(20.0, 20.0)), "attach", t.icon, t.accent);
        paint_clipped(painter, pos2(main.min.x + 34.0, main.min.y + 13.0), name_w + 4.0, &shown, regular(12.0), t.text);
        painter.text(
            pos2(main.min.x + 34.0, main.min.y + 27.0),
            Align2::LEFT_CENTER,
            mailcraft_model::size_label_in(crate::i18n::current(), a.size() as u64),
            regular(10.5),
            t.text_dim,
        );
    });
}

/// One attachment of `size` points, as a chip or as a picture (#834): `paint` draws what it shows
/// in its main part, and at its right end a `▾` opens the menu a right-click opens (#785).
/// `shown` is the name it is opened and saved under.
#[allow(clippy::too_many_arguments)]
fn file_item(
    app: &mut MailApp,
    ui: &mut Ui,
    m: &Message,
    index: usize,
    shown: &str,
    size: egui::Vec2,
    t: &Tokens,
    paint: impl FnOnce(&egui::Painter, Rect),
) {
    // //// Neoffice — « Ouvrir dans NeoGrille », the first action on a file the hosting page's editors open
    // //// Neoffice — (host_attachments.rs `opener`): its editor's name on the chip or the picture, between the file and its `▾`.
    let opener = if m.id != 0 { m.attachments.get(index).and_then(|a| crate::host_attachments::opener(app, m, a)) } else { None };
    let open_w =
        opener.map_or(0.0, |editor| ui.ctx().fonts_mut(|f| f.layout_no_wrap(editor.product().to_string(), semibold(11.5), t.text).size().x) + 20.0);
    // //// Neoffice — (and before the `▾` its editor's part, `open_w` wide.)
    let (r, _) = ui.allocate_exact_size(vec2(size.x + open_w, size.y), Sense::hover());
    let drop_rect = Rect::from_min_max(pos2(r.max.x - crate::ribbon::DROP_W, r.min.y), r.max);
    // //// Neoffice — (the editor's part, before the `▾`.)
    let open_rect = Rect::from_min_max(pos2(drop_rect.min.x - open_w, r.min.y), pos2(drop_rect.min.x, r.max.y));
    let main_rect = Rect::from_min_max(r.min, pos2(open_rect.min.x, r.max.y));
    let filed = m.id != 0;
    let resp = ui.interact(main_rect, ui.make_persistent_id(("attachment", m.id, index)), Sense::click());
    // //// Neoffice — the editor's part before the `▾`: the keyboard goes from one to the next as the eye does.
    let open = opener.map(|_| ui.interact(open_rect, ui.make_persistent_id(("attachmentEditor", m.id, index)), Sense::click()));
    let drop_sense = if filed { Sense::click() } else { Sense::hover() };
    let drop = ui.interact(drop_rect, ui.make_persistent_id(("attachmentMenu", m.id, index)), drop_sense);
    app.hit.add("attachment", attachment_key(m.id, index), main_rect);
    app.hit.add("attachmentMenu", attachment_key(m.id, index), drop_rect);
    let selected = crate::attachment_preview::target(app, app.hit.window(), m.id) == Some(index);
    let (fill, border) = if selected { (t.selection_pill, t.selection_edge) } else { (t.panel, t.border) };
    ui.painter().rect(r, 5.0, fill, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    let left = egui::CornerRadius { nw: 5, sw: 5, ne: 0, se: 0 };
    let right = egui::CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 };
    if !selected && resp.hovered() {
        ui.painter().rect_filled(main_rect.shrink(1.0), left, t.hover);
    }
    if filed && (drop.hovered() || drop.is_pointer_button_down_on()) {
        ui.painter().rect_filled(drop_rect.shrink(1.0), right, if drop.is_pointer_button_down_on() { t.pressed } else { t.hover });
    }
    let divider = ui.painter().round_to_pixel_center(drop_rect.min.x);
    ui.painter().line_segment([pos2(divider, r.min.y + 7.0), pos2(divider, r.max.y - 7.0)], Stroke::new(1.0, t.border));
    // //// Neoffice — its editor's name, a click opens the file in it (host_attachments.rs `IN_EDITOR`).
    if let (Some(editor), Some(open)) = (opener, open) {
        let key = attachment_key(m.id, index);
        app.hit.add("attachmentEditor", key, open_rect);
        let label = crate::host_attachments::open_label(editor);
        open.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        if open.hovered() || open.is_pointer_button_down_on() {
            ui.painter().rect_filled(open_rect.shrink(1.0), 0.0, if open.is_pointer_button_down_on() { t.pressed } else { t.hover });
        }
        if open.has_focus() {
            ui.painter().rect_stroke(open_rect.shrink(1.0), 0.0, Stroke::new(2.0, t.focus_ring), egui::StrokeKind::Inside);
        }
        let line = ui.painter().round_to_pixel_center(open_rect.min.x);
        ui.painter().line_segment([pos2(line, r.min.y + 7.0), pos2(line, r.max.y - 7.0)], Stroke::new(1.0, t.border));
        ui.painter().text(open_rect.center(), Align2::CENTER_CENTER, editor.product(), semibold(11.5), t.accent);
        if open.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(label).clicked() {
            let _ = app.run(crate::host_attachments::IN_EDITOR, json!({"id": m.id, "index": index}));
        }
    }
    crate::ribbon::drop_mark(ui, drop_rect.center(), if filed { t.icon } else { t.text_disabled });
    if resp.has_focus() {
        ui.painter().rect_stroke(main_rect.shrink(1.0), left, Stroke::new(2.0, t.focus_ring), egui::StrokeKind::Inside);
    }
    if drop.has_focus() {
        ui.painter().rect_stroke(drop_rect.shrink(1.0), right, Stroke::new(2.0, t.focus_ring), egui::StrokeKind::Inside);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, shown));
    paint(&ui.painter().with_clip_rect(main_rect.intersect(ui.clip_rect())), main_rect);
    // Double-click opens it, as in Outlook; its menu opens or saves it.
    // (A message that is not in the mailbox has no number to name it by: attachments are
    // opened and saved once it is filed, and its `▾` is greyed and opens nothing.)
    if !filed {
        let why = tl!("Move the message to a folder to open or save its attachments");
        resp.on_hover_text(why);
        drop.on_hover_text(why);
        return;
    }
    let hint = tl!("Click to select {name}; double-click to open it");
    let resp = resp.on_hover_text(crate::i18n::fmt(hint, &[("name", shown)]));
    if resp.double_clicked() {
        crate::attachment_preview::cancel_click(app);
        let _ = app.run("mail.openAttachment", json!({"id": m.id, "index": index}));
    } else if resp.clicked() {
        crate::attachment_preview::queue_click(app, ui.ctx(), m.id, index);
    }
    resp.context_menu(|ui| attachment_menu(ui, app, m.id, index));
    // The `▾`: a click, or Enter or Space while it has the keyboard, opens the same menu.
    let label = crate::i18n::fmt(tl!("Attachment options for {name}"), &[("name", shown)]);
    drop.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    let drop = drop.on_hover_text(tl!("Attachment options"));
    egui::Popup::menu(&drop).show(|ui| attachment_menu(ui, app, m.id, index));
}

/// The key of attachment `index` of message `id` in the hit map (`attachment` for the chip,
/// `attachmentMenu` for its `▾`).
pub(crate) fn attachment_key(id: u64, index: usize) -> u64 {
    crate::ribbon::hit_id(&format!("{id}:{index}"))
}

/// An attachment's menu, the same from a right-click on its chip and from the chip's `▾` (#785).
/// Each entry is greyed when its command can't run, and the command keeps its own refusals.
pub(super) fn attachment_menu(ui: &mut Ui, app: &mut MailApp, id: u64, index: usize) {
    // //// Neoffice — « Ouvrir dans NeoGrille » first, where the hosting page opens the file at once (host_attachments.rs).
    if let Some(m) = app.session.mailbox.message(id).cloned()
        && let Some(a) = m.attachments.get(index)
    {
        crate::host_attachments::first(ui, app, &m, index, a);
    }
    if crate::attachment_preview::available(app, id, index) && ui.button(tl!("Preview Attachment")).clicked() {
        if let Err(error) = crate::attachment_preview::open(app, ui.ctx().viewport_id(), id, index) {
            app.status(error);
        }
        ui.close();
    }
    crate::widgets::mi(ui, app, "Open", "mail.openAttachment", json!({"id": id, "index": index}));
    crate::widgets::mi(ui, app, "Save As…", "mail.saveAttachment", json!({"id": id, "index": index}));
    crate::widgets::mi(ui, app, "Save All Attachments…", "mail.saveAllAttachments", json!({"id": id}));
    // //// Neoffice — what the hosting page does with an attachment: its Drive, its scanned documents, an editor (host_attachments.rs).
    let held = app.session.mailbox.message(id).cloned();
    if let Some(m) = held.as_ref()
        && let Some(a) = m.attachments.get(index)
    {
        crate::host_attachments::menu(ui, app, m, index, a);
    }
    crate::widgets::mi(ui, app, "Remove Attachment", "mail.removeAttachment", json!({"id": id, "index": index}));
}
