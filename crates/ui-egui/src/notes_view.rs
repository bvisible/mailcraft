// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Notes: the mailbox's own and those of a server that keeps them (`mailcraft_engine::cmd::notes`), as cards.
//!
//! - **The pane** on the left: New Note, the two lists (the notes, the archived ones) with how many each holds, and a
//!   search of their titles and words.
//! - **The cards** in the middle, the pinned first under their heading: a card shows the note's title (or "Untitled"),
//!   its first words, when it was changed, and what is to know of it: pinned, shared by whom, formatted (its text is
//!   changed where it was written), not saved yet, and where it is kept when notes are kept in more than one place.
//!   A right click offers what may be done to it, greyed with the engine's reason where its rights refuse it.
//! - **The note** on the right: its title and its text, typed into in place. What is typed is saved two seconds after
//!   the last key, when the field is left, on Escape, or when another note is chosen, one undo step each; and when
//!   Notes is left with something typed (`flush`). Whether it is saved on its server is said under it. A note whose
//!   text has more than words (formatting, a checklist, attachments) shows its text as it can't be changed here, and
//!   opens on the web; one shared for reading says so, and nothing in it can be changed.
//!
//! A note's words are its writer's: they are shown as text, never followed. Its link opens only on the owner's click
//! (`ui.openNote`), and only when it is an https address a message's link would be followed to.

use egui::{Align2, Rect, RichText, Sense, Stroke, TextEdit, Ui, pos2, vec2};
use mailcraft_engine::cmd::notes::{self, NOTE_NO_LINK, NOTE_NOT_YOURS, NOTE_READ_ONLY};
use mailcraft_model::Id;
use mailcraft_model::note::Note;
use serde_json::{Value, json};

use crate::theme::{Tokens, paint_clipped, regular, semibold};
use crate::{MailApp, icons, widgets};

/// A card's size and the room between cards; the panes' widths.
const CARD: f32 = 210.0;
const GAP: f32 = 12.0;
const HEADING_H: f32 = 30.0;
const PANE_W: f32 = 220.0;
const EDITOR_W: f32 = 380.0;
/// How long after the last key what was typed is saved (seconds).
const SAVE_AFTER: f64 = 2.0;
/// The most characters of a note's words a card lays out.
const CARD_WORDS: usize = 400;
/// Where the window keeps the list shown and the search, and what is typed.
const FILTER: &str = "notes_filter";
const DRAFT: &str = "notes_draft";
const FOCUS_TITLE: &str = "notes_focus_title";

/// The list shown and the search.
#[derive(Clone, Default)]
struct Filter {
    archived: bool,
    query: String,
}

/// What is typed into a note and not saved yet.
#[derive(Clone, Debug, Default, PartialEq)]
struct Draft {
    note: Id,
    title: String,
    text: String,
    /// When the last key was typed (the window's time, seconds); `None`: nothing typed since it was saved.
    typed: Option<f64>,
}

impl Draft {
    fn of(note: &Note) -> Draft {
        Draft { note: note.id, title: note.title.clone(), text: note.text.clone(), typed: None }
    }
}

/// What a card says of a note.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CardText {
    pub title: String,
    pub words: String,
    pub changed: String,
    pub badges: Vec<String>,
}

/// A note's title as a card and a list show it: "Untitled" for one with neither title nor words.
fn shown_title(note: &Note) -> String {
    let heading = note.heading().trim();
    if heading.is_empty() { tl!("Untitled").to_string() } else { heading.to_string() }
}

/// Where a note is kept, in words.
fn kept_where(app: &MailApp, note: &Note) -> String {
    if note.origin.is_local() {
        tl!("On this computer").to_string()
    } else {
        app.session.mailbox.account(note.origin.account).map(|account| account.email.trim().to_string()).unwrap_or_default()
    }
}

/// Whether notes are kept in more than one place here (this computer, a server, another).
fn several_places(app: &MailApp) -> bool {
    let mut places = app.session.mailbox.notes.items.values().map(|note| note.origin.account);
    places.next().is_some_and(|first| places.any(|other| other != first))
}

/// What the card of `note` says (see the module).
pub(crate) fn card_text(app: &MailApp, note: &Note) -> CardText {
    let changed = mailcraft_time::list_label(note.updated, app.session.now(), app.session.offset_at(note.updated), app.session.prefs.twelve_hour);
    let mut badges = Vec::new();
    if note.pinned {
        badges.push(tl!("Pinned").to_string());
    }
    if note.shared {
        badges.push(crate::i18n::fmt(tl!("Shared by {name}"), &[("name", note.owner_name.trim())]));
    }
    if note.rich || note.cut {
        badges.push(tl!("Formatted").to_string());
    }
    if !notes::saved(&app.session, note) || !note.error.is_empty() {
        badges.push(tl!("Not saved").to_string());
    }
    if several_places(app) {
        badges.push(kept_where(app, note));
    }
    let words: String = note.text.chars().take(CARD_WORDS).collect();
    let words = if note.title.trim().is_empty() { words.lines().skip(1).collect::<Vec<_>>().join("\n") } else { words };
    // (A note a server says nothing of the time of says nothing of it here either.)
    let changed = if note.updated > 0 { crate::i18n::fmt(tl!("Changed {when}"), &[("when", &changed)]) } else { String::new() };
    CardText { title: shown_title(note), words, changed, badges }
}

/// Whether what was done to `note` is on its server: `None` for the mailbox's own, else the line under it.
pub(crate) fn save_state(app: &MailApp, note: &Note) -> Option<String> {
    if note.origin.is_local() {
        return None;
    }
    if !note.error.is_empty() {
        return Some(crate::i18n::fmt(tl!("Not saved: {why}"), &[("why", &crate::i18n::run_words::refusal(&note.error, &app.session.mailbox))]));
    }
    let change = app.session.mailbox.notes.changes.iter().find(|change| change.note == note.id);
    Some(match change {
        None => tl!("Saved").to_string(),
        Some(change) if change.failures > 0 => tl!("Not saved yet: tried again shortly.").to_string(),
        Some(_) => tl!("Saving…").to_string(),
    })
}

/// What may be done to `note`, each with the command it runs and, when its rights refuse it, why.
pub(crate) fn actions(note: &Note) -> Vec<(&'static str, &'static str, Value, Option<&'static str>)> {
    let id = note.id;
    let write = (!note.rights.write).then_some(NOTE_READ_ONLY);
    let link = mailcraft_model::OnlineMeeting::clean_link(&note.web_url).is_none().then_some(NOTE_NO_LINK);
    vec![
        (if note.pinned { "Unpin" } else { "Pin" }, "notes.pin", json!({"id": id}), write),
        (if note.archived { "Unarchive" } else { "Archive" }, "notes.archive", json!({"id": id}), write),
        ("Open on the Web", "ui.openNote", json!({"id": id}), link),
        ("Delete", "notes.delete", json!({"id": id}), (!note.rights.delete).then_some(NOTE_NOT_YOURS)),
    ]
}

pub fn show(app: &mut MailApp, ui: &mut Ui) {
    if !notes::offered(&app.session) {
        // Notes are no longer kept here (a hosting page's tab whose server stopped keeping them): Mail instead.
        flush(app, ui.ctx(), true);
        let _ = app.run("view.mail", json!({}));
        crate::mail_view::show(app, ui);
        return;
    }
    let t = Tokens::get(ui.ctx());
    let mut filter: Filter = ui.data(|d| d.get_temp(egui::Id::new(FILTER))).unwrap_or_default();
    let frame = egui::Frame::NONE.fill(t.surface).stroke(Stroke::new(1.0, t.border));
    egui::Panel::left("notes_pane").default_size(PANE_W).size_range(180.0..=320.0).resizable(true).frame(frame).show(ui, |ui| {
        pane(app, ui, &mut filter);
    });
    let selected = app.session.sel.note.and_then(|id| app.session.mailbox.notes.note(id)).cloned();
    if let Some(note) = &selected {
        let max = (ui.available_width() - CARD - 40.0).max(300.0);
        egui::Panel::right("note_editor").default_size(EDITOR_W.min(max)).size_range(300.0..=max).resizable(true).frame(frame).show(ui, |ui| {
            editor(app, ui, note);
        });
    } else {
        flush(app, ui.ctx(), true);
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| cards(app, ui, &filter));
    ui.data_mut(|d| d.insert_temp(egui::Id::new(FILTER), filter));
}

// ---- the pane ----

fn pane(app: &mut MailApp, ui: &mut Ui, filter: &mut Filter) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let why = notes::not_offered(&app.session);
        let resp = widgets::icon_text_button(ui, "notes", tl!("New Note"), why.is_none());
        let resp = match why {
            Some(why) => resp.on_hover_text(crate::i18n::run_words::refusal(why, &app.session.mailbox)),
            None => resp,
        };
        if resp.clicked() && why.is_none() && app.run("notes.new", json!({})).is_ok() {
            filter.archived = false;
            filter.query.clear();
            ui.data_mut(|d| d.insert_temp(egui::Id::new(FOCUS_TITLE), true));
        }
    });
    ui.add_space(10.0);
    let count = |archived: bool| app.session.mailbox.notes.items.values().filter(|note| note.archived == archived).count();
    for (archived, label) in [(false, tl!("Notes")), (true, tl!("Archived notes"))] {
        let n = count(archived);
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        let on = filter.archived == archived;
        if on {
            ui.painter().rect_filled(r.shrink2(vec2(6.0, 1.0)), 4.0, t.selection);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink2(vec2(6.0, 1.0)), 4.0, t.hover);
        }
        paint_clipped(
            ui.painter(),
            pos2(r.min.x + 16.0, r.center().y),
            r.width() - 60.0,
            label,
            if on { semibold(13.0) } else { regular(13.0) },
            t.text,
        );
        ui.painter().text(pos2(r.max.x - 16.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), regular(12.0), t.text_dim);
        if resp.clicked() {
            filter.archived = archived;
        }
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let search = TextEdit::singleline(&mut filter.query).hint_text(tl!("Search notes")).desired_width(ui.available_width() - 8.0);
        let resp = ui.add(search);
        app.hit.add("noteSearch", 0, resp.rect);
    });
}

// ---- the cards ----

fn cards(app: &mut MailApp, ui: &mut Ui, filter: &Filter) {
    let t = Tokens::get(ui.ctx());
    let ids = notes::sorted(&app.session, filter.archived, &filter.query);
    if ids.is_empty() {
        let said = if !filter.query.trim().is_empty() {
            tl!("No note matches.")
        } else if filter.archived {
            tl!("No archived notes.")
        } else {
            tl!("No notes yet: New Note starts one.")
        };
        ui.add_space(40.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(said).color(t.text_dim)));
        return;
    }
    let pinned: Vec<Id> = ids.iter().copied().filter(|id| app.session.mailbox.notes.note(*id).is_some_and(|note| note.pinned)).collect();
    let others: Vec<Id> = ids.iter().copied().filter(|id| !pinned.contains(id)).collect();
    let width = ui.available_width() - 2.0 * GAP;
    let columns = (((width + GAP) / (CARD + GAP)).floor() as usize).max(1);
    // The sections, each a heading (when there are pinned notes) and its rows of cards.
    let mut sections: Vec<(Option<&'static str>, Vec<Id>)> = Vec::new();
    if !pinned.is_empty() {
        sections.push((Some(tl!("Pinned notes")), pinned));
        sections.push((Some(if filter.archived { tl!("Archived notes") } else { tl!("Notes") }), others));
    } else {
        sections.push((None, others));
    }
    let rows = |n: usize| n.div_ceil(columns);
    let height: f32 = sections
        .iter()
        .filter(|(_, ids)| !ids.is_empty())
        .map(|(heading, ids)| if heading.is_some() { HEADING_H } else { 0.0 } + rows(ids.len()) as f32 * (CARD + GAP))
        .sum::<f32>()
        + GAP;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let (whole, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let mut y = whole.min.y + GAP;
        for (heading, ids) in &sections {
            if ids.is_empty() {
                continue;
            }
            if let Some(heading) = heading {
                paint_clipped(ui.painter(), pos2(whole.min.x + GAP, y + HEADING_H / 2.0), width, heading, semibold(13.0), t.text_dim);
                y += HEADING_H;
            }
            for (n, id) in ids.iter().enumerate() {
                let (row, column) = (n / columns, n % columns);
                let top = y + row as f32 * (CARD + GAP);
                let rect = Rect::from_min_size(pos2(whole.min.x + GAP + column as f32 * (CARD + GAP), top), vec2(CARD, CARD));
                // Only the cards in view are laid out.
                if rect.max.y >= whole.min.y + viewport.min.y && rect.min.y <= whole.min.y + viewport.max.y {
                    card(app, ui, *id, rect);
                }
            }
            y += rows(ids.len()) as f32 * (CARD + GAP);
        }
    });
}

fn card(app: &mut MailApp, ui: &mut Ui, id: Id, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let Some(note) = app.session.mailbox.notes.note(id).cloned() else { return };
    let resp = ui.interact(rect, ui.id().with(("note", id)), Sense::click());
    app.hit.add("note", id, rect);
    let selected = app.session.sel.note == Some(id);
    let said = card_text(app, &note);
    let p = ui.painter();
    p.rect_filled(rect, 6.0, if resp.hovered() && !selected { t.hover } else { t.surface });
    p.rect_stroke(rect, 6.0, if selected { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.border) }, egui::StrokeKind::Inside);
    let inner = rect.shrink(12.0);
    let mut right = inner.max.x;
    if note.pinned {
        icons::paint(p, Rect::from_center_size(pos2(right - 8.0, inner.min.y + 9.0), vec2(16.0, 16.0)), "pin", t.icon, t.accent);
        right -= 22.0;
    }
    paint_clipped(p, pos2(inner.min.x, inner.min.y + 9.0), (right - inner.min.x).max(0.0), &said.title, semibold(14.0), t.text);
    // The words, as many lines as the card holds, as text.
    let words_rect = Rect::from_min_max(pos2(inner.min.x, inner.min.y + 26.0), pos2(inner.max.x, inner.max.y - 40.0));
    let galley = p.layout(said.words.clone(), regular(12.0), t.text_dim, words_rect.width());
    p.with_clip_rect(words_rect.intersect(p.clip_rect())).galley(words_rect.min, galley, t.text_dim);
    let badges = said.badges.iter().filter(|badge| **badge != tl!("Pinned")).cloned().collect::<Vec<_>>().join(" · ");
    paint_clipped(p, pos2(inner.min.x, inner.max.y - 26.0), inner.width(), &badges, regular(11.0), t.accent_text);
    paint_clipped(p, pos2(inner.min.x, inner.max.y - 8.0), inner.width(), &said.changed, regular(11.0), t.text_dim);
    if resp.clicked() || (resp.secondary_clicked() && !selected) {
        let _ = app.run("notes.select", json!({"id": id}));
    }
    resp.context_menu(|ui| {
        ui.set_min_width(200.0);
        for (label, command, params, refused) in actions(&note) {
            let button = ui.add_enabled(refused.is_none(), egui::Button::new(tl!(label)).min_size(vec2(190.0, 0.0)));
            let button = match refused {
                Some(why) => button.on_disabled_hover_text(crate::i18n::run_words::refusal(why, &app.session.mailbox)),
                None => button,
            };
            if button.clicked() {
                let _ = app.run(command, params);
                ui.close();
            }
        }
    });
}

// ---- the note ----

fn editor(app: &mut MailApp, ui: &mut Ui, note: &Note) {
    let t = Tokens::get(ui.ctx());
    let now = ui.input(|i| i.time);
    let key = egui::Id::new(DRAFT);
    let mut draft: Draft = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| Draft::of(note));
    // Another note chosen: what was typed into the last one is saved first.
    if draft.note != note.id {
        save(app, &mut draft);
        draft = Draft::of(note);
    }
    // Nothing typed: the note as it is now (a server's read may have changed it).
    if draft.typed.is_none() {
        draft = Draft::of(note);
    }
    let (write, write_text) = (note.rights.write, note.rights.write && note.text_writable());
    ui.add_space(12.0);
    let mut left = false;
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let title = TextEdit::singleline(&mut draft.title)
            .hint_text(tl!("Untitled"))
            .font(semibold(17.0))
            .frame(egui::Frame::NONE)
            .desired_width(ui.available_width() - 12.0)
            .interactive(write);
        let resp = ui.add(title);
        app.hit.add("noteTitle", note.id, resp.rect);
        if ui.data_mut(|d| d.remove_temp::<bool>(egui::Id::new(FOCUS_TITLE))).unwrap_or(false) {
            resp.request_focus();
        }
        if resp.changed() {
            draft.typed = Some(now);
        }
        left |= resp.lost_focus();
    });
    ui.add_space(4.0);
    let banner = if !write {
        Some(tl!("You may read this note, not change it."))
    } else if !write_text {
        Some(tl!("This note has formatting, a checklist or attachments: change its text where it was written."))
    } else {
        None
    };
    if let Some(banner) = banner {
        egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.label(RichText::new(banner).color(t.text));
        });
    }
    if let Some(state) = save_state(app, note) {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new(state).size(11.5).color(t.text_dim));
        });
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.add_space(8.0);
        for (label, command, params, refused) in actions(note) {
            let icon = match command {
                "notes.pin" => "pin",
                "notes.archive" => "archive",
                "ui.openNote" => "globe",
                _ => "delete",
            };
            let resp = widgets::icon_text_button(ui, icon, tl!(label), refused.is_none());
            app.hit.add(hit_kind(command), note.id, resp.rect);
            let resp = match refused {
                Some(why) => resp.on_hover_text(crate::i18n::run_words::refusal(why, &app.session.mailbox)),
                None => resp,
            };
            if resp.clicked() && refused.is_none() {
                save(app, &mut draft);
                let _ = app.run(command, params);
            }
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let text = TextEdit::multiline(&mut draft.text)
                .hint_text(tl!("Write your note…"))
                .frame(egui::Frame::NONE)
                .desired_rows(16)
                .desired_width(ui.available_width() - 12.0)
                .interactive(write_text);
            let resp = ui.add(text);
            app.hit.add("noteText", note.id, resp.rect);
            if resp.changed() {
                draft.typed = Some(now);
            }
            left |= resp.lost_focus();
        });
    });
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let quiet = draft.typed.is_some_and(|typed| now - typed >= SAVE_AFTER);
    if draft.typed.is_some() && (left || escape || quiet) {
        save(app, &mut draft);
    }
    if draft.typed.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    // (The selection may have gone: a delete.)
    if app.session.sel.note == Some(note.id) || draft.typed.is_some() {
        ui.data_mut(|d| d.insert_temp(key, draft));
    } else {
        ui.data_mut(|d| d.remove_temp::<Draft>(key));
    }
}

fn hit_kind(command: &str) -> &'static str {
    match command {
        "notes.pin" => "notePin",
        "notes.archive" => "noteArchive",
        "ui.openNote" => "noteWeb",
        _ => "noteDelete",
    }
}

/// Save what was typed into `draft`'s note (one undo step), when anything: the title and the text that differ from the
/// note's.
fn save(app: &mut MailApp, draft: &mut Draft) {
    if draft.typed.take().is_none() {
        return;
    }
    let Some(note) = app.session.mailbox.notes.note(draft.note).cloned() else { return };
    let mut params = json!({"id": note.id});
    if draft.title != note.title {
        params["title"] = json!(draft.title);
    }
    if draft.text != note.text && note.text_writable() {
        params["text"] = json!(draft.text);
    }
    if params.as_object().is_some_and(|fields| fields.len() > 1) {
        let _ = app.run("notes.edit", params);
    }
}

/// What was typed into a note and not saved yet is saved when Notes isn't in front (`always`: whatever is in front).
/// Called every frame, so that leaving Notes never leaves words behind.
pub(crate) fn flush(app: &mut MailApp, ctx: &egui::Context, always: bool) {
    let in_front = app.session.view.module == mailcraft_engine::Module::Notes && !app.ui.backstage;
    if in_front && !always {
        return;
    }
    let key = egui::Id::new(DRAFT);
    let Some(mut draft) = ctx.data(|d| d.get_temp::<Draft>(key)) else { return };
    if draft.typed.is_some() {
        save(app, &mut draft);
    }
    ctx.data_mut(|d| d.remove_temp::<Draft>(key));
}

/// `ui.openNote`: open a note on the web, in the system's browser. The link came from the note's server: it is
/// followed only because the owner asked, only when it is an `https` address, and by the rules a link in a message is
/// followed by.
pub(crate) fn ui_command(app: &mut MailApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    if id != "ui.openNote" {
        return None;
    }
    let note = p.get("id").and_then(Value::as_u64).or(app.session.sel.note);
    let Some(note) = note else { return Some(Err(tl!("Select a note first").to_string())) };
    let told = match app.session.run("notes.link", &json!({"id": note})) {
        Ok(told) => told,
        Err(e) => return Some(Err(crate::i18n::run_words::refusal(&e.to_string(), &app.session.mailbox).into_owned())),
    };
    let url = told.get("url").and_then(Value::as_str).unwrap_or_default();
    // Twice over: an https address as the model has it, and one a message's link would be followed to.
    let safe = mailcraft_model::OnlineMeeting::clean_link(url).filter(|url| mailcraft_html::is_safe_url(url) && !mailcraft_html::is_embedded(url));
    let Some(url) = safe else { return Some(Err(crate::i18n::run_words::refusal(NOTE_NO_LINK, &app.session.mailbox).into_owned())) };
    let Some(ctx) = app.ctx.clone() else { return Some(Err(tl!("The window isn't up yet.").to_string())) };
    ctx.open_url(egui::OpenUrl::new_tab(url.clone()));
    Some(Ok(json!({"note": note, "url": url})))
}

#[cfg(test)]
mod tests;
