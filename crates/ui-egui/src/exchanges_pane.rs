// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the Échanges pane at the right of the window.

//! The Échanges pane: the person whose message is being read, with what was said with them in three tabs (the exchanges, their files,
//! what Neoffice links them to). Its state and what it asks of a host are in `exchanges.rs`.
//!
//! One frame: [`prepare`] works out everything the pane shows (and asks the host for what is missing), [`draw`] paints it and says what
//! the person did, [`carry_out`] does it. Nothing a message or a server says is anything but text here.

use egui::{Align2, Color32, FontId, Frame, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_model::FolderKind;
use mailcraft_model::exchanges::{self as data, Doc, File, Linked, Row, Someone, Way};
use mailcraft_time::DateTime;

use crate::exchanges::{Asked, Fetch, Remote, Show, Tab, Target, local_rows, mark, target_of, who};
use crate::theme::{Tokens, paint_clipped, regular, semibold};
use crate::{MailApp, icons};

/// The height of a row of the lists, a month band, the header and the tabs.
const ROW_H: f32 = 52.0;
const BAND_H: f32 = 22.0;
const HEAD_H: f32 = 52.0;
const TAB_H: f32 = 34.0;

/// What the person did in the pane.
enum Act {
    Close,
    Tab(Tab),
    Show(Show),
    /// Open this exchange's message.
    Open(Row),
    More,
    Retry,
    /// Open a page of the instance (a document, « Tout voir »).
    Url(String),
    Options {
        cc: bool,
        domain: bool,
    },
}

/// One exchange as the pane shows it.
struct Line {
    row: Row,
    when: String,
    band: String,
    folder: String,
    current: bool,
}

/// What a frame of the pane shows.
struct View {
    target: Option<Target>,
    someone: Option<Someone>,
    addresses: Vec<String>,
    lines: Vec<Line>,
    /// How many exchanges there are before the search words and the choice of way are applied.
    all: usize,
    /// How many files, before the search words.
    files: Vec<(String, String, String, Row)>,
    file_count: usize,
    remote: Remote,
    hosted: bool,
    linked: Asked<Linked>,
    /// The oldest exchange there is, as a date for the foot.
    since: Option<String>,
}

fn folder_label(row: &Row) -> String {
    match row.kind {
        Some(kind) if kind != FolderKind::Custom => crate::i18n::current().folder(kind.default_name()).to_string(),
        _ => row.folder.clone(),
    }
}

fn matches(row: &Row, words: &str) -> bool {
    let words = words.trim().to_lowercase();
    words.is_empty()
        || words.split_whitespace().all(|w| {
            row.subject.to_lowercase().contains(w)
                || row.name.to_lowercase().contains(w)
                || row.email.to_lowercase().contains(w)
                || row.folder.to_lowercase().contains(w)
        })
}

/// Everything a frame of the pane shows, and the asks it needs of the host.
fn prepare(app: &mut MailApp, ctx: &egui::Context) -> View {
    let focus = app.session.sel.focus();
    let target = focus.and_then(|id| target_of(app, app.session.mailbox.message(id)?));
    let mut view = View {
        target: target.clone(),
        someone: None,
        addresses: Vec::new(),
        lines: Vec::new(),
        all: 0,
        files: Vec::new(),
        file_count: 0,
        remote: Remote::default(),
        hosted: app.exchanges.hosted(),
        linked: Asked::Unsupported,
        since: None,
    };
    let Some(target) = target else { return view };
    let now = crate::now_ms();
    let (addresses, someone) = who(app, &target);
    let local = local_rows(app, ctx, &addresses);
    let remote = app.exchanges.remote_of(now, &target.mailbox, &addresses);
    let rows = data::merge(local.to_vec(), &remote.rows);
    let (show, words) = (app.exchanges.show, app.exchanges.text.clone());
    let (now_ts, twelve) = (app.session.now(), app.session.prefs.twelve_hour);
    view.all = rows.len();
    view.since = rows.last().map(|r| crate::dates::medium(DateTime::from_timestamp(r.when, app.session.offset_at(r.when)).date));
    for row in &rows {
        let offset = app.session.offset_at(row.when);
        if show.lets(row.way) && matches(row, &words) {
            view.lines.push(Line {
                when: crate::dates::list_label(row.when, now_ts, offset, twelve),
                band: crate::dates::month_year(DateTime::from_timestamp(row.when, offset).date),
                folder: folder_label(row),
                current: row.message == Some(target.message),
                row: row.clone(),
            });
        }
        if !row.files.is_empty() && matches(row, &words) {
            view.file_count += row.files.len();
            for file in &row.files {
                let meta = format!("{} · {}", size_label(file), crate::dates::list_label(row.when, now_ts, offset, twelve));
                view.files.push((file.name.clone(), meta, row.subject.clone(), row.clone()));
            }
        }
    }
    if app.exchanges.tab == Tab::Neoffice {
        view.linked = app.exchanges.linked_of(now, &target.mailbox, &target.address);
    }
    view.target = Some(target);
    view.someone = someone;
    view.addresses = addresses;
    view.remote = remote;
    view
}

fn size_label(file: &File) -> String {
    if file.size == 0 {
        return file.mime.clone();
    }
    mailcraft_model::size_label_in(crate::i18n::current(), file.size)
}

fn initials(name: &str, address: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let pick = |w: &str| w.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    match words.as_slice() {
        [] => pick(address),
        [one] => pick(one),
        [first, .., last] => format!("{}{}", pick(first), pick(last)),
    }
}

/// `text` in at most `rows` lines of `width`, wrapped at the words, with its left edge at `at.x` and the block centred on `at.y`.
fn paint_wrapped(painter: &egui::Painter, at: egui::Pos2, width: f32, text: &str, font: FontId, colour: Color32, rows: usize) {
    if width <= 4.0 || text.is_empty() {
        return;
    }
    let mut job =
        egui::text::LayoutJob::single_section(text.to_string(), egui::text::TextFormat { font_id: font, color: colour, ..Default::default() });
    job.wrap = egui::text::TextWrapping { max_width: width, max_rows: rows, break_anywhere: false, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let top = at.y - galley.size().y / 2.0;
    painter.galley(pos2(at.x, top), galley, colour);
}

/// A few words said in the middle of a tab, wrapped: what there is nothing of, or why.
fn note(ui: &mut Ui, t: &Tokens, text: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
    paint_wrapped(ui.painter(), pos2(r.min.x + 12.0, r.center().y), r.width() - 24.0, text, regular(12.0), t.text_dim, 3);
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x)
}

/// The pane at the right of the window, while it is open.
pub(crate) fn side(app: &mut MailApp, ui: &mut Ui) {
    if !app.exchanges.open {
        return;
    }
    let t = Tokens::get(ui.ctx());
    if app.exchanges.width < 240.0 {
        app.exchanges.width = crate::exchanges::WIDTH;
    }
    let view = prepare(app, ui.ctx());
    // What the pane asked of the host is asked at the next frame, which an idle window would not have.
    if app.exchanges.has_wants() {
        ui.ctx().request_repaint();
    }
    // Never so wide that the list at its narrowest and a message beside it would have less than they keep (`mail_view::message_room`).
    let widest = (ui.available_width() - crate::mail_view::LIST_MIN - crate::mail_view::READING_MIN).max(240.0);
    let narrowed = app.exchanges.width > widest + 0.5;
    let panel = if narrowed {
        egui::Panel::right("exchanges_pane_narrowed").exact_size(widest).resizable(false)
    } else {
        egui::Panel::right("exchanges_pane").default_size(app.exchanges.width).size_range(240.0..=widest).resizable(true)
    };
    let mut words = app.exchanges.text.clone();
    let (tab, show, cc, domain) = (app.exchanges.tab, app.exchanges.show, app.exchanges.cc, app.exchanges.domain);
    let mut acts = Vec::new();
    let drawn = panel.frame(Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        draw(ui, &t, &view, &mut words, (tab, show, cc, domain), &mut acts);
    });
    if !narrowed {
        app.exchanges.width = drawn.response.rect.width();
    }
    app.exchanges.text = words;
    carry_out(app, &ui.ctx().clone(), &view, acts);
}

fn draw(ui: &mut Ui, t: &Tokens, view: &View, words: &mut String, state: (Tab, Show, bool, bool), acts: &mut Vec<Act>) {
    let (tab, show, cc, domain) = state;
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    header(ui, t, view, (cc, domain), acts);
    if let Some(someone) = &view.someone {
        neoffice_line(ui, t, someone, acts);
    }
    tabs(ui, t, view, tab, acts);
    match tab {
        Tab::Exchanges => exchanges_tab(ui, t, view, words, show, acts),
        Tab::Files => files_tab(ui, t, view, words, acts),
        Tab::Neoffice => neoffice_tab(ui, t, view, acts),
    }
}

fn header(ui: &mut Ui, t: &Tokens, view: &View, options: (bool, bool), acts: &mut Vec<Act>) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEAD_H), Sense::hover());
    let p = ui.painter();
    p.line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));
    // The person.
    let Some(target) = &view.target else {
        paint_wrapped(
            p,
            pos2(r.min.x + 12.0, r.center().y),
            r.width() - 56.0,
            tl!("Select a message to see what was said with its sender."),
            regular(12.5),
            t.text_dim,
            3,
        );
        let close = Rect::from_center_size(pos2(r.max.x - 20.0, r.center().y), vec2(26.0, 26.0));
        if close_button(ui, t, close) {
            acts.push(Act::Close);
        }
        return;
    };
    let name = view.someone.as_ref().map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| target.name.clone());
    let shown = if name.is_empty() { target.address.clone() } else { name.clone() };
    let centre = pos2(r.min.x + 28.0, r.center().y);
    p.circle_filled(centre, 16.0, t.hover);
    p.text(centre, Align2::CENTER_CENTER, initials(&name, &target.address), semibold(12.0), t.text);
    let right = r.max.x - 12.0 - 26.0 * 2.0 - 4.0;
    paint_clipped(p, pos2(r.min.x + 52.0, r.center().y - 9.0), right - r.min.x - 52.0, &shown, semibold(13.0), t.text);
    paint_clipped(p, pos2(r.min.x + 52.0, r.center().y + 9.0), right - r.min.x - 52.0, &target.address, regular(11.5), t.text_dim);
    // The options (the gear) and the way to close.
    let gear = Rect::from_center_size(pos2(r.max.x - 20.0 - 28.0, r.center().y), vec2(26.0, 26.0));
    let resp = ui.interact(gear, ui.scope_id().with("exchanges_gear"), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(gear, 4.0, t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(gear.center(), vec2(16.0, 16.0)), "options", t.icon, t.accent);
    let (mut cc, mut domain) = options;
    let has_domains = view.someone.as_ref().is_some_and(|s| !s.domains.is_empty());
    egui::Popup::menu(&resp).show(|ui| {
        ui.checkbox(&mut cc, tl!("Also what only copied them"));
        ui.add_enabled(has_domains, egui::Checkbox::new(&mut domain, tl!("The whole company (every address of its domain)")));
    });
    if (cc, domain) != options {
        acts.push(Act::Options { cc, domain });
    }
    let close = Rect::from_center_size(pos2(r.max.x - 20.0, r.center().y), vec2(26.0, 26.0));
    if close_button(ui, t, close) {
        acts.push(Act::Close);
    }
}

fn close_button(ui: &Ui, t: &Tokens, rect: Rect) -> bool {
    let resp = ui.interact(rect, ui.scope_id().with("exchanges_close"), Sense::click()).on_hover_text(tl!("Close"));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), "close", t.icon, t.accent);
    resp.clicked()
}

/// Who the person is in Neoffice: its pill, the customer or the supplier, and the way to its page.
fn neoffice_line(ui: &mut Ui, t: &Tokens, someone: &Someone, acts: &mut Vec<Act>) {
    let line = match (&someone.party, &someone.lead) {
        (Some(p), _) => format!("{} · {}", p.kind, p.title),
        (None, Some((title, _))) => format!("{} · {}", tl!("Lead"), title),
        (None, None) if !someone.company.is_empty() => someone.company.clone(),
        _ => return,
    };
    let (fill, ink) = mark(t);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 0.0, t.ribbon);
    p.line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));
    let pill_w = text_width(ui, "Neoffice", semibold(11.0)) + 16.0;
    let pill = Rect::from_min_size(pos2(r.min.x + 12.0, r.center().y - 9.0), vec2(pill_w, 18.0));
    ui.painter().rect_filled(pill, 9.0, fill);
    ui.painter().text(pill.center(), Align2::CENTER_CENTER, "Neoffice", semibold(11.0), ink);
    let page = someone.party.as_ref().map(|p| p.url.clone()).filter(|u| !u.is_empty());
    let link_w = if page.is_some() { text_width(ui, tl!("Open the record"), semibold(11.5)) + 16.0 } else { 0.0 };
    paint_clipped(ui.painter(), pos2(pill.max.x + 8.0, r.center().y), r.max.x - link_w - 12.0 - pill.max.x - 8.0, &line, regular(12.0), t.text);
    if let Some(url) = page {
        let at = Rect::from_min_max(pos2(r.max.x - link_w - 8.0, r.min.y), pos2(r.max.x - 4.0, r.max.y));
        let resp = ui.interact(at, ui.scope_id().with("exchanges_record"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.painter().text(pos2(at.max.x - 8.0, at.center().y), Align2::RIGHT_CENTER, tl!("Open the record"), semibold(11.5), t.accent_text);
        if resp.clicked() {
            acts.push(Act::Url(url));
        }
    }
}

fn tabs(ui: &mut Ui, t: &Tokens, view: &View, active: Tab, acts: &mut Vec<Act>) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), TAB_H), Sense::hover());
    ui.painter().line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));
    let which: Vec<Tab> = [Tab::Exchanges, Tab::Files, Tab::Neoffice].into_iter().filter(|tab| *tab != Tab::Neoffice || view.hosted).collect();
    let labels = |counts: bool| -> Vec<String> {
        which
            .iter()
            .map(|tab| match (tab, counts) {
                (Tab::Exchanges, true) => format!("{} {}", tl!("Exchanges"), view.all),
                (Tab::Exchanges, false) => tl!("Exchanges").to_string(),
                (Tab::Files, true) => format!("{} {}", tl!("Files"), view.file_count),
                (Tab::Files, false) => tl!("Files").to_string(),
                (Tab::Neoffice, _) => tl!("In Neoffice").to_string(),
            })
            .collect()
    };
    let widths = |labels: &[String], size: f32| -> Vec<f32> {
        labels.iter().zip(&which).map(|(label, tab)| text_width(ui, label, if *tab == active { semibold(size) } else { regular(size) })).collect()
    };
    // A narrow pane gives up, one after the other, the size of the letters and the counts before it cuts a label.
    let room = (r.width() - 16.0).max(0.0);
    let mut chosen = None;
    for (size, counts) in [(12.5, true), (11.0, true), (11.0, false)] {
        let labels = labels(counts);
        let text = widths(&labels, size);
        let fits = text.iter().sum::<f32>() + 10.0 * text.len() as f32 <= room;
        chosen = Some((size, labels, text));
        if fits {
            break;
        }
    }
    let Some((size, labels, text)) = chosen else { return };
    let around = ((room - text.iter().sum::<f32>()) / text.len().max(1) as f32).clamp(4.0, 20.0);
    let mut x = r.min.x + 8.0;
    for ((tab, label), w) in which.iter().zip(&labels).zip(text) {
        let font = if *tab == active { semibold(size) } else { regular(size) };
        let w = w + around;
        let at = Rect::from_min_size(pos2(x, r.min.y), vec2(w, TAB_H));
        let resp = ui.interact(at, ui.scope_id().with(("exchanges_tab", *tab as u8)), Sense::click());
        ui.painter().text(at.center(), Align2::CENTER_CENTER, label, font, if *tab == active { t.text } else { t.text_dim });
        if *tab == active {
            ui.painter().line_segment([pos2(at.min.x + 4.0, r.max.y - 1.0), pos2(at.max.x - 4.0, r.max.y - 1.0)], Stroke::new(2.0, t.accent));
        }
        if resp.clicked() {
            acts.push(Act::Tab(*tab));
        }
        x += w;
    }
}

fn search_box(ui: &mut Ui, t: &Tokens, words: &mut String) {
    egui::Frame::NONE.inner_margin(Margin { left: 12, right: 12, top: 8, bottom: 6 }).show(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(words)
                .hint_text(tl!("Search in these exchanges"))
                .desired_width(f32::INFINITY)
                .font(regular(12.5))
                .text_color(t.text)
                .margin(Margin::symmetric(8, 5)),
        );
    });
}

fn chips(ui: &mut Ui, t: &Tokens, show: Show, acts: &mut Vec<Act>) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let mut x = r.min.x + 12.0;
    for (which, label) in [(Show::All, tl!("All")), (Show::In, tl!("Received")), (Show::Out, tl!("Sent"))] {
        let w = text_width(ui, label, regular(12.0)) + 20.0;
        let at = Rect::from_min_size(pos2(x, r.min.y + 3.0), vec2(w, 22.0));
        let resp = ui.interact(at, ui.scope_id().with(("exchanges_chip", which as u8)), Sense::click());
        let on = which == show;
        if on {
            ui.painter().rect_filled(at, 11.0, t.checked);
        } else {
            ui.painter().rect_stroke(at, 11.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        }
        ui.painter().text(
            at.center(),
            Align2::CENTER_CENTER,
            label,
            if on { semibold(12.0) } else { regular(12.0) },
            if on { t.accent_text } else { t.text_dim },
        );
        if resp.clicked() {
            acts.push(Act::Show(which));
        }
        x += w + 6.0;
    }
}

fn exchanges_tab(ui: &mut Ui, t: &Tokens, view: &View, words: &mut String, show: Show, acts: &mut Vec<Act>) {
    search_box(ui, t, words);
    chips(ui, t, show, acts);
    let foot = 78.0;
    let room = (ui.available_height() - foot).max(60.0);
    egui::ScrollArea::vertical().id_salt("exchanges_list").max_height(room).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        if view.lines.is_empty() {
            let said = if view.target.is_none() {
                ""
            } else if view.all == 0 {
                tl!("Nothing was said with this person yet.")
            } else {
                tl!("No exchange matches.")
            };
            note(ui, t, said);
        }
        let mut band = "";
        for line in &view.lines {
            if line.band != band {
                band = &line.band;
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAND_H), Sense::hover());
                ui.painter().rect_filled(r, 0.0, t.ribbon);
                ui.painter().line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));
                paint_clipped(
                    ui.painter(),
                    pos2(r.min.x + 12.0, r.center().y),
                    r.width() - 24.0,
                    &line.band.to_uppercase(),
                    semibold(10.5),
                    t.group_label,
                );
            }
            if row_line(ui, t, line) {
                acts.push(Act::Open(line.row.clone()));
            }
        }
    });
    foot_of(ui, t, view, acts);
}

/// One exchange: its way, its subject, its date, where it is and who with. Whether it was clicked.
fn row_line(ui: &mut Ui, t: &Tokens, line: &Line) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    if line.current {
        ui.painter().rect_filled(r, 0.0, t.checked);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover);
    }
    let p = ui.painter();
    p.line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));
    let way = if line.row.way == Way::Out { "sent" } else { "mail" };
    icons::paint(p, Rect::from_center_size(pos2(r.min.x + 22.0, r.min.y + 17.0), vec2(16.0, 16.0)), way, t.text_dim, t.accent);
    let date_w = text_width(ui, &line.when, regular(11.0));
    ui.painter().text(pos2(r.max.x - 12.0, r.min.y + 17.0), Align2::RIGHT_CENTER, &line.when, regular(11.0), t.text_dim);
    let subject = if line.row.subject.trim().is_empty() {
        tl!("(no subject)").to_string()
    } else {
        line.row.subject.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let font = if line.row.unread { semibold(12.5) } else { regular(12.5) };
    paint_clipped(ui.painter(), pos2(r.min.x + 40.0, r.min.y + 17.0), r.width() - 40.0 - 12.0 - date_w - 8.0, &subject, font, t.text);
    // Where it is, who it was with, and a paperclip.
    let who = if line.row.name.is_empty() { line.row.email.clone() } else { line.row.name.clone() };
    let tag_w = text_width(ui, &line.folder, regular(10.5)) + 10.0;
    let tag = Rect::from_min_size(pos2(r.min.x + 40.0, r.min.y + 29.0), vec2(tag_w.min(r.width() * 0.5), 16.0));
    ui.painter().rect_stroke(tag, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    paint_clipped(ui.painter(), pos2(tag.min.x + 5.0, tag.center().y), tag.width() - 8.0, &line.folder, regular(10.5), t.text_dim);
    let clip = if line.row.clip { 18.0 } else { 0.0 };
    paint_clipped(
        ui.painter(),
        pos2(tag.max.x + 8.0, tag.center().y),
        (r.max.x - 12.0 - clip - tag.max.x - 8.0).max(0.0),
        &who,
        regular(11.5),
        t.text_dim,
    );
    if line.row.clip {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 20.0, tag.center().y), vec2(14.0, 14.0)), "attach", t.text_dim, t.accent);
    }
    resp.clicked()
}

/// The foot of the exchanges: how many there are, and what asks the server for more.
fn foot_of(ui: &mut Ui, t: &Tokens, view: &View, acts: &mut Vec<Act>) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.ribbon);
    ui.painter().line_segment([pos2(r.min.x, r.min.y), pos2(r.max.x, r.min.y)], Stroke::new(1.0, t.border));
    let memory = match &view.since {
        Some(since) if !view.hosted => crate::i18n::fmt(
            &tln!(view.all, "{n} exchange in memory, since {since}.", "{n} exchanges in memory, since {since}."),
            &[("since", since.as_str())],
        ),
        Some(since) => {
            crate::i18n::fmt(&tln!(view.all, "{n} exchange, back to {since}.", "{n} exchanges, back to {since}."), &[("since", since.as_str())])
        }
        None => String::new(),
    };
    paint_clipped(ui.painter(), pos2(r.min.x + 12.0, r.center().y), r.width() - 24.0, &memory, regular(11.5), t.text_dim);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.ribbon);
    ui.painter().line_segment([pos2(r.min.x, r.min.y), pos2(r.max.x, r.min.y)], Stroke::new(1.0, t.border));
    let centre = pos2(r.min.x + 12.0, r.center().y);
    if !view.hosted {
        paint_wrapped(
            ui.painter(),
            pos2(r.min.x + 12.0, r.center().y),
            r.width() - 24.0,
            tl!("This window has no way to ask the server for more."),
            regular(11.5),
            t.text_dim,
            2,
        );
        return;
    }
    let width = r.width() - 24.0;
    match view.remote.fetch {
        Fetch::Loading => {
            let spin = Rect::from_center_size(pos2(r.min.x + 20.0, r.center().y), vec2(14.0, 14.0));
            egui::Spinner::new().size(14.0).color(t.text_dim).paint_at(ui, spin);
            let said = if view.remote.loaded { tl!("Looking further on the server…") } else { tl!("Looking on the server…") };
            paint_clipped(ui.painter(), pos2(r.min.x + 36.0, r.center().y), width - 24.0, said, regular(11.5), t.text_dim);
            ui.ctx().request_repaint();
        }
        Fetch::Failed(_) => {
            let (said, button) = (tl!("The server did not answer."), tl!("Try again"));
            let bw = text_width(ui, button, semibold(12.0)) + 8.0;
            paint_clipped(ui.painter(), centre, width - bw - 8.0, said, regular(11.5), t.red);
            let at = Rect::from_min_max(pos2(r.max.x - bw - 12.0, r.min.y), pos2(r.max.x - 4.0, r.max.y));
            let resp = ui.interact(at, ui.scope_id().with("exchanges_retry"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.painter().text(at.center(), Align2::CENTER_CENTER, button, semibold(12.0), t.accent_text);
            if resp.clicked() {
                acts.push(Act::Retry);
            }
        }
        Fetch::Idle if view.remote.loaded && view.remote.next.is_none() => {
            paint_clipped(ui.painter(), centre, width, tl!("Nothing older on the server."), regular(11.5), t.text_dim);
        }
        Fetch::Idle if view.remote.loaded => {
            let said = if view.remote.partial {
                tl!("Look further on the server… (some folders were not looked through)")
            } else {
                tl!("Look further on the server…")
            };
            let resp = ui.interact(r, ui.scope_id().with("exchanges_more"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            paint_clipped(ui.painter(), centre, width, said, semibold(12.0), t.accent_text);
            if resp.clicked() {
                acts.push(Act::More);
            }
        }
        Fetch::Idle => {}
    }
}

fn files_tab(ui: &mut Ui, t: &Tokens, view: &View, words: &mut String, acts: &mut Vec<Act>) {
    search_box(ui, t, words);
    egui::ScrollArea::vertical().id_salt("exchanges_files").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        if view.files.is_empty() && view.target.is_some() {
            note(ui, t, tl!("No file came with the exchanges the window has."));
        }
        for (name, meta, subject, row) in &view.files {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, t.hover);
            }
            let p = ui.painter();
            p.line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));
            icons::paint(p, Rect::from_center_size(pos2(r.min.x + 22.0, r.center().y), vec2(18.0, 18.0)), "attach", t.text_dim, t.accent);
            paint_clipped(p, pos2(r.min.x + 40.0, r.min.y + 17.0), r.width() - 52.0, name, semibold(12.5), t.text);
            paint_clipped(p, pos2(r.min.x + 40.0, r.min.y + 33.0), r.width() - 52.0, &format!("{meta} · {subject}"), regular(11.5), t.text_dim);
            if resp.clicked() {
                acts.push(Act::Open(row.clone()));
            }
        }
    });
}

fn neoffice_tab(ui: &mut Ui, t: &Tokens, view: &View, acts: &mut Vec<Act>) {
    // No message in view: the header says to select one, and there is nobody to say anything of.
    if view.target.is_none() {
        return;
    }
    let said = |ui: &mut Ui, text: &str| note(ui, t, text);
    match &view.linked {
        Asked::Waiting => said(ui, tl!("Looking in Neoffice…")),
        Asked::Failed(_) => said(ui, tl!("Neoffice did not answer.")),
        Asked::Unsupported => said(ui, tl!("This Neoffice cannot say what is linked to this person.")),
        Asked::Ready(linked) if linked.docs.is_empty() => {
            said(
                ui,
                if linked.everything.is_empty() {
                    tl!("No customer, supplier or lead of Neoffice has this address.")
                } else {
                    tl!("Nothing is linked to this person yet.")
                },
            );
            everything(ui, t, linked, acts);
        }
        Asked::Ready(linked) => {
            egui::ScrollArea::vertical()
                .id_salt("exchanges_docs")
                .max_height((ui.available_height() - 46.0).max(60.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    for doc in &linked.docs {
                        if doc_line(ui, t, doc) && !doc.url.is_empty() {
                            acts.push(Act::Url(doc.url.clone()));
                        }
                    }
                });
            everything(ui, t, linked, acts);
        }
    }
}

fn tone(t: &Tokens, tone: &str) -> Color32 {
    match tone {
        "danger" => t.red,
        "warn" => t.orange,
        "success" => t.green,
        "info" => t.blue,
        _ => t.text_dim,
    }
}

fn doc_line(ui: &mut Ui, t: &Tokens, doc: &Doc) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover);
    }
    ui.painter().line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));
    let right = r.max.x - 12.0;
    let amount = doc.amount.map(|a| money(a, &doc.currency)).unwrap_or_default();
    let status_w = if doc.status.is_empty() { 0.0 } else { text_width(ui, &doc.status, semibold(10.5)) + 14.0 };
    let amount_w = text_width(ui, &amount, semibold(12.0));
    ui.painter().text(pos2(right, r.min.y + 17.0), Align2::RIGHT_CENTER, &amount, semibold(12.0), t.text);
    if status_w > 0.0 {
        let pill = Rect::from_min_size(pos2(right - status_w, r.min.y + 28.0), vec2(status_w, 16.0));
        ui.painter().rect_stroke(pill, 8.0, Stroke::new(1.0, tone(t, &doc.tone)), egui::StrokeKind::Inside);
        ui.painter().text(pill.center(), Align2::CENTER_CENTER, &doc.status, semibold(10.5), tone(t, &doc.tone));
    }
    let used = amount_w.max(status_w) + 20.0;
    paint_clipped(
        ui.painter(),
        pos2(r.min.x + 12.0, r.min.y + 17.0),
        r.width() - 24.0 - used,
        &format!("{} · {}", doc.kind, if doc.title.is_empty() { &doc.name } else { &doc.title }),
        semibold(12.5),
        t.text,
    );
    paint_clipped(ui.painter(), pos2(r.min.x + 12.0, r.min.y + 33.0), r.width() - 24.0 - used, &doc.reference, regular(11.5), t.text_dim);
    resp.clicked()
}

/// « Tout voir » of the thing the person is linked to, at the foot of the tab.
fn everything(ui: &mut Ui, t: &Tokens, linked: &Linked, acts: &mut Vec<Act>) {
    if linked.everything.is_empty() {
        return;
    }
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.ribbon);
    ui.painter().line_segment([pos2(r.min.x, r.min.y), pos2(r.max.x, r.min.y)], Stroke::new(1.0, t.border));
    let label = tl!("See everything");
    let w = text_width(ui, label, semibold(12.5)) + 28.0;
    let button = Rect::from_min_size(pos2(r.max.x - 12.0 - w, r.center().y - 14.0), vec2(w, 28.0));
    let resp = ui.interact(button, ui.scope_id().with("exchanges_everything"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.painter().rect_filled(button, 4.0, if resp.hovered() { t.hover } else { t.input });
    ui.painter().rect_stroke(button, 4.0, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    ui.painter().text(button.center(), Align2::CENTER_CENTER, label, semibold(12.5), t.text);
    paint_clipped(
        ui.painter(),
        pos2(r.min.x + 12.0, r.center().y),
        (button.min.x - r.min.x - 20.0).max(0.0),
        tl!("Quotes, orders, invoices, tickets, tasks, files."),
        regular(11.5),
        t.text_dim,
    );
    if resp.clicked() {
        acts.push(Act::Url(linked.everything.clone()));
    }
}

/// An amount with its currency as a Swiss page writes it: `CHF 1'260.–`.
pub(crate) fn money(amount: f64, currency: &str) -> String {
    let cents = (amount.abs() * 100.0).round() as i64;
    let (units, rest) = (cents / 100, cents % 100);
    let digits = units.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('\'');
        }
        grouped.push(c);
    }
    let tail = if rest == 0 { ".–".to_string() } else { format!(".{rest:02}") };
    let sign = if amount < 0.0 { "-" } else { "" };
    if currency.is_empty() { format!("{sign}{grouped}{tail}") } else { format!("{currency} {sign}{grouped}{tail}") }
}

/// What the person did, done.
fn carry_out(app: &mut MailApp, ctx: &egui::Context, view: &View, acts: Vec<Act>) {
    for act in acts {
        match act {
            Act::Close => {
                let _ = app.run("ui.exchanges", serde_json::json!({"open": false}));
            }
            Act::Tab(tab) => app.exchanges.tab = tab,
            Act::Show(show) => app.exchanges.show = show,
            Act::Options { cc, domain } => {
                app.exchanges.cc = cc;
                app.exchanges.domain = domain;
            }
            Act::Open(row) => open(app, view.target.as_ref(), &row),
            Act::More => {
                if let Some(target) = &view.target {
                    app.exchanges.ask_more(&target.mailbox, &view.addresses);
                }
            }
            Act::Retry => app.exchanges.retry(&view.addresses),
            // A page of the person's instance goes through the host that frames the window when it can open one (the page `/neomail`
            // in a new tab, Neodesk in its own window), as a card of Nora's pane does; any other link in a new tab.
            Act::Url(url) => match (app.services.nora.as_ref().and_then(|host| host.open.as_ref()), instance_path(&url)) {
                (Some(open), Some(path)) => open(&path),
                _ => ctx.open_url(egui::OpenUrl::new_tab(url)),
            },
        }
    }
}

/// The path (with its query) of an address of the instance: `https://x.example/app/history/Customer/C-1` is `/app/history/Customer/C-1`.
/// `None` for anything that is not an `http` or `https` address, and for one with no path.
fn instance_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let at = rest.find('/')?;
    let path = rest.get(at..)?;
    (path.len() > 1 && !path.starts_with("//") && !path.chars().any(char::is_control)).then(|| path.to_string())
}

/// An exchange opened: the window's own message at once, else the one the server has, read by its id.
fn open(app: &mut MailApp, target: Option<&Target>, row: &Row) {
    if let Some(id) = row.message {
        app.open_in_mail(id);
        return;
    }
    let (Some(target), Some(server)) = (target, row.server.as_deref()) else { return };
    if let Err(said) = crate::host_menu::open_server_message(app, &target.mailbox, server) {
        app.status_msg = Some((said, crate::now_ms()));
    }
}

#[cfg(test)]
#[path = "exchanges_pane_tests.rs"]
mod tests;
