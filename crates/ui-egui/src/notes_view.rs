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
use mailcraft_engine::cmd::notes::{self, NOTE_NO_LINK, NOTE_NOT_YOURS, NOTE_READ_ONLY, NOTE_TEXT_ELSEWHERE, words};
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

/// What is typed into a note and not saved yet, and the note's words it was typed over.
#[derive(Clone, Debug, Default, PartialEq)]
struct Draft {
    note: Id,
    /// The note's account (0: this computer's): where its words go when the note is gone.
    account: Id,
    title: String,
    text: String,
    /// The note's title and text when the typing began (or since the last save): what the words typed are made over.
    from_title: String,
    from_text: String,
    /// When the last key was typed (the window's time, seconds); `None`: nothing typed since it was saved.
    typed: Option<f64>,
}

impl Draft {
    fn of(note: &Note) -> Draft {
        Draft {
            note: note.id,
            account: note.origin.account,
            title: note.title.clone(),
            text: note.text.clone(),
            from_title: note.title.clone(),
            from_text: note.text.clone(),
            typed: None,
        }
    }
}

/// The drafts of the notes typed into and not saved yet, by note: one that can't be saved now (too long for its server)
/// stays while another note is shown.
type Drafts = std::collections::HashMap<Id, Draft>;

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
    let typed_here = app.ctx.as_ref().is_some_and(|ctx| {
        ctx.data_mut(|d| d.get_temp_mut_or_default::<Drafts>(egui::Id::new(DRAFT)).get(&note.id).is_some_and(|draft| draft.typed.is_some()))
    });
    if !notes::saved(&app.session, note) || !note.error.is_empty() || typed_here {
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
    let mut all = drafts(ui.ctx());
    // What was typed into other notes is saved first (another note was chosen); what can't be yet stays.
    let others: Vec<Id> = all.iter().filter(|(id, draft)| **id != note.id && draft.typed.is_some()).map(|(id, _)| *id).collect();
    for id in others {
        if let Some(mut other) = all.remove(&id)
            && !save(app, &mut other, now)
        {
            all.insert(id, other);
        }
    }
    let mut draft: Draft = all.remove(&note.id).unwrap_or_else(|| Draft::of(note));
    // Nothing typed: the note as it is now (a server's read may have changed it).
    if draft.typed.is_none() {
        draft = Draft::of(note);
    }
    let (write, write_text) = (note.rights.write, note.rights.write && note.text_writable());
    // What its server takes (or this computer keeps): more is said, never cut.
    let (max_title, max_text) = notes::limits(&app.session, if note.origin.is_remote() { note.origin.account } else { 0 });
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
    too_many(ui, draft.title.chars().count(), max_title);
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
                save(app, &mut draft, now);
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
        too_many(ui, draft.text.chars().count(), max_text);
    });
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let quiet = draft.typed.is_some_and(|typed| now - typed >= SAVE_AFTER);
    if draft.typed.is_some() && (left || escape || quiet) {
        save(app, &mut draft, now);
    }
    if draft.typed.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    // (The selection may have gone: a delete. What is typed stays whatever happens.)
    if app.session.sel.note == Some(note.id) || draft.typed.is_some() {
        all.insert(note.id, draft);
    }
    keep_drafts(ui.ctx(), all);
}

/// Under a field: how many characters are more than the note's server takes, when any (they are kept, never cut).
fn too_many(ui: &mut Ui, count: usize, most: usize) {
    if count <= most {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let n = count - most;
    let said =
        if n == 1 { tl!("1 character too many").to_string() } else { crate::i18n::fmt(tl!("{n} characters too many"), &[("n", &n.to_string())]) };
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new(said).size(11.5).color(t.red));
    });
}

fn drafts(ctx: &egui::Context) -> Drafts {
    ctx.data(|d| d.get_temp::<Drafts>(egui::Id::new(DRAFT))).unwrap_or_default()
}

fn keep_drafts(ctx: &egui::Context, all: Drafts) {
    if all.is_empty() {
        ctx.data_mut(|d| d.remove_temp::<Drafts>(egui::Id::new(DRAFT)));
    } else {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(DRAFT), all));
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

/// Save what was typed into `draft`'s note (one undo step): the parts typed into, over the words they were typed over.
/// Never over other words: when the note's words were changed underneath (a read brought another device's), when it is
/// gone, or when it can no longer be changed so (it is read only now, or its text is changed where it was written), the
/// words typed are kept as a note of their own and the person is told. `false` while they can't be saved yet (more than
/// the note's server takes, or the engine refused them): the draft stays, and is tried again.
fn save(app: &mut MailApp, draft: &mut Draft, now: f64) -> bool {
    if draft.typed.is_none() {
        return true;
    }
    let (wants_title, wants_text) = (draft.title != draft.from_title, draft.text != draft.from_text);
    let Some(note) = app.session.mailbox.notes.note(draft.note).cloned() else {
        return keep_copy(app, draft, Kept::Gone);
    };
    if !wants_title && !wants_text {
        draft.typed = None;
        return true;
    }
    // (A title the server gave a note sent without one, from its first line, is nobody's change of it.)
    let most_title = notes::limits(&app.session, if note.origin.is_remote() { note.origin.account } else { 0 }).0;
    let derived = draft.from_title.is_empty() && note.title == notes::derived_title(&note.text, most_title);
    let title_moved = note.title != draft.from_title && !derived;
    let underneath = (wants_title && title_moved) || (wants_text && note.text != draft.from_text);
    if underneath {
        return keep_copy(app, draft, Kept::Underneath);
    }
    let refused = if wants_title && !note.rights.write {
        Some(NOTE_READ_ONLY)
    } else if wants_text && !(note.rights.write && note.text_writable()) {
        Some(if note.rights.write { NOTE_TEXT_ELSEWHERE } else { NOTE_READ_ONLY })
    } else {
        None
    };
    if let Some(why) = refused {
        return keep_copy(app, draft, Kept::Refused(why));
    }
    let (max_title, max_text) = notes::limits(&app.session, if note.origin.is_remote() { note.origin.account } else { 0 });
    if draft.title.chars().count() > max_title || draft.text.chars().count() > max_text {
        return false;
    }
    let mut params = json!({"id": note.id});
    if wants_title {
        params["title"] = json!(draft.title);
    }
    if wants_text {
        params["text"] = json!(draft.text);
    }
    if app.run("notes.edit", params).is_err() {
        // (Said by the engine; tried again in a moment, the words kept.)
        draft.typed = Some(now);
        return false;
    }
    if let Some(saved) = app.session.mailbox.notes.note(draft.note) {
        (draft.from_title, draft.from_text) = (saved.title.clone(), saved.text.clone());
        (draft.title, draft.text) = (saved.title.clone(), saved.text.clone());
    }
    draft.typed = None;
    true
}

/// Why words typed are kept as a note of their own.
enum Kept {
    /// The note was changed underneath them.
    Underneath,
    /// It is gone.
    Gone,
    /// It can't be changed so any more.
    Refused(&'static str),
}

/// Keep the words of `draft` as a note of their own, in the note's account (where its server still keeps notes), and say
/// so. `false` when that can't be done now either: the draft stays.
fn keep_copy(app: &mut MailApp, draft: &mut Draft, why: Kept) -> bool {
    let account = draft.account;
    let to = if account != 0 && app.session.mailbox.notes.server(account).is_some() { account } else { 0 };
    let (max_title, max_text) = notes::limits(&app.session, to);
    if draft.text.chars().count() > max_text {
        return false;
    }
    let heading = if draft.title.trim().is_empty() {
        draft.text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_string()
    } else {
        draft.title.trim().to_string()
    };
    let (title, cut) = match why {
        Kept::Gone => (heading.chars().take(max_title).collect::<String>(), heading.chars().count() > max_title),
        _ => notes::server::copy_title(&app.session.mailbox, &heading, max_title),
    };
    // A title that had to be cut goes whole as the first line of the text (one from the text is in it already).
    let text = if cut && !draft.title.trim().is_empty() { format!("{}\n{}", draft.title.trim(), draft.text) } else { draft.text.clone() };
    if text.chars().count() > max_text {
        return false;
    }
    let mut params = json!({"title": title, "text": text});
    if to != 0 {
        params["account"] = json!(to);
    }
    if app.run("notes.new", params).is_err() {
        return false;
    }
    let named = |words: &str| if words.trim().is_empty() { "\"\u{2026}\"".to_string() } else { format!("\"{}\"", words.trim()) };
    let was = named(if draft.from_title.trim().is_empty() { &heading } else { &draft.from_title });
    let said = match why {
        Kept::Underneath => words::fill(words::KEPT_AS_COPY, &[("title", &was), ("copy", &named(&title))]),
        Kept::Gone => words::fill(words::KEPT_WHEN_GONE, &[("title", &was)]),
        Kept::Refused(reason) => words::fill(words::KEPT_NOT_WRITABLE, &[("title", &was), ("why", reason), ("copy", &named(&title))]),
    };
    app.status(said.clone());
    app.keep_sync_problems(account, std::slice::from_ref(&said));
    draft.typed = None;
    true
}

/// What was typed into notes and not saved yet is saved when Notes isn't in front (`always`: whatever is in front).
/// Called every frame, so that leaving Notes never leaves words behind; what can't be saved yet stays.
pub(crate) fn flush(app: &mut MailApp, ctx: &egui::Context, always: bool) {
    let in_front = app.session.view.module == mailcraft_engine::Module::Notes && !app.ui.backstage;
    if in_front && !always {
        return;
    }
    let all = drafts(ctx);
    if all.is_empty() {
        return;
    }
    let now = ctx.input(|i| i.time);
    let mut kept = Drafts::new();
    for (id, mut draft) in all {
        if draft.typed.is_some() && !save(app, &mut draft, now) {
            kept.insert(id, draft);
        }
    }
    keep_drafts(ctx, kept);
}

/// The window is asked to close: what was typed into notes goes into them first, and what still can't be saved (more
/// than its server takes) holds the close with a question, as a message being written does. Whether it holds it.
pub(crate) fn hold_quit(app: &mut MailApp) -> bool {
    let Some(ctx) = app.ctx.clone() else { return false };
    flush(app, &ctx, true);
    let Some(first) = drafts(&ctx).values().find(|draft| draft.typed.is_some()).map(|draft| draft.note) else { return false };
    // The note in front, for the owner to see what can't be saved; and the question.
    let _ = app.run("view.notes", json!({}));
    let _ = app.run("notes.select", json!({"id": first}));
    let ask = crate::dialogs::Question {
        title: tl!("Unsaved Note").to_string(),
        text: tl!("A note has words that can't be saved yet: they are longer than its server takes. Closing now loses them.").to_string(),
        action: tl!("Close Anyway").to_string(),
        with: Value::Null,
        more: Vec::new(),
        note: None,
    };
    app.dialog = Some(crate::dialogs::Dialog::Confirm { command: "ui.dropNoteDrafts".into(), params: json!({"then": "quit"}), ask });
    true
}

/// Whether words typed into a note are not in it yet.
pub(crate) fn typed(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Drafts>(egui::Id::new(DRAFT)).values().any(|draft| draft.typed.is_some()))
}

/// `ui.openNote`: open a note on the web, in the system's browser. The link came from the note's server: it is
/// followed only because the owner asked, only when it is an `https` address, and by the rules a link in a message is
/// followed by.
pub(crate) fn ui_command(app: &mut MailApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    if id == "ui.dropNoteDrafts" {
        // What was typed into notes and can't be saved, given up (the owner's answer to closing): then the close.
        if let Some(ctx) = app.ctx.clone() {
            keep_drafts(&ctx, Drafts::new());
        }
        if p.get("then").and_then(Value::as_str) == Some("quit") {
            app.quit_requested = true;
        }
        return Some(Ok(json!({"dropped": true})));
    }
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
