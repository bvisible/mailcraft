// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Plus tard » (snooze) in a hosting page's tab, where the person reaches
// //// Neoffice — it, and the times it offers.

//! « Plus tard » in a tab of a hosting page whose mailbox's server keeps it (the engine's `cmd/snooze_hosted.rs`): the message
//! leaves its folder for the mailbox's « Plus tard » folder and comes back to it at the time chosen, unread, even with every
//! window closed (the server brings it back).
//!
//! Where it is reached:
//! - a clock among the quick actions of a row (`mail_view/done.rs`), and beside the actions of the message being read
//!   (`reading.rs`): a menu of times;
//! - « Plus tard » in the message's menu (`mail_view.rs`) and in the Home tab's Tags group (`ribbon.rs`), and its key ([`KEY`]:
//!   Ctrl+Shift+L, ⌘⇧L; this tab's alone, not upstream's Snooze's), which opens the window of times ([`open`]);
//! - the folder « Plus tard » itself in the folder pane: each of its rows says when the message comes back ([`row_label`]), or
//!   that its server could not bring it back, and the same clock brings it back now.
//!
//! The times: later today (three hours on, at the next full hour, while that is before nine in the evening), tomorrow
//! morning (8:00), next week (Monday, 8:00), or a date and time chosen in the window, all in the zone the window shows its
//! times in (a change of the clocks between now and then is the engine's to weigh: it takes the wall-clock time).

use egui::{Ui, vec2};
use mailcraft_engine::cmd::snooze_hosted;
use mailcraft_model::{Id, Message};
use mailcraft_time::{Date, DateTime, Timestamp};
use serde_json::json;

use crate::MailApp;
use crate::theme::{Tokens, regular, semibold};

/// The key that opens the window of times, in a hosting page's tab whose mailbox's server keeps « Plus tard » (`keys.rs`).
pub(crate) const KEY: &str = "Mod+Shift+L";

/// The key as the window writes it beside the clock: `Ctrl+Shift+L`, `⌘⇧L` on a Mac.
fn key_text() -> String {
    crate::widgets::key_text(KEY)
}

/// Whether the key opens the window of times now: the Mail module in front, messages selected that may be put away.
pub(crate) fn key_opens(app: &MailApp) -> bool {
    app.session.view.module == mailcraft_engine::Module::Mail && offered(app, &app.session.sel.messages)
}

/// The morning hour of « Demain matin » and « La semaine prochaine ».
pub(crate) const MORNING: u8 = 8;
/// « Plus tard aujourd'hui » is three hours on, and offered while that is before this hour.
const LATER_HOURS: u8 = 3;
const LAST_HOUR_TODAY: u8 = 21;
/// The window's id in egui's memory: what it is for, while it is open.
fn window_id() -> egui::Id {
    egui::Id::unique("snooze_hosted_window")
}

/// A time offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Choice {
    /// What the hit map and the tests call it.
    pub(crate) key: &'static str,
    /// Its name (English; shown through `tlc!("snooze", …)`).
    pub(crate) label: &'static str,
    pub(crate) at: DateTime,
}

/// What the window of times holds while it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Window {
    pub(crate) ids: Vec<Id>,
    pub(crate) date: Date,
    /// Minutes after midnight.
    pub(crate) minutes: u16,
    pub(crate) error: Option<String>,
    /// The list of times is open and was scrolled to the one chosen.
    pub(crate) listed: bool,
}

fn label(choice: &Choice) -> &'static str {
    match choice.key {
        "later" => tlc!("snooze", "Later Today"),
        "tomorrow" => tlc!("snooze", "Tomorrow Morning"),
        _ => tlc!("snooze", "Next Week"),
    }
}

/// The times offered at the wall-clock time `now`.
pub(crate) fn choices_at(now: DateTime) -> Vec<Choice> {
    let mut out = Vec::new();
    let hour = u16::from(now.hour) + u16::from(LATER_HOURS) + u16::from(now.minute > 0 || now.second > 0);
    if hour < u16::from(LAST_HOUR_TODAY) {
        out.push(Choice { key: "later", label: "Later Today", at: DateTime { date: now.date, hour: hour as u8, minute: 0, second: 0 } });
    }
    let tomorrow = now.date.add_days(1);
    out.push(Choice { key: "tomorrow", label: "Tomorrow Morning", at: DateTime { date: tomorrow, hour: MORNING, minute: 0, second: 0 } });
    let monday = now.date.add_days(7 - i64::from(now.date.weekday().index()));
    out.push(Choice { key: "nextWeek", label: "Next Week", at: DateTime { date: monday, hour: MORNING, minute: 0, second: 0 } });
    out
}

/// The times offered now.
pub(crate) fn choices(app: &MailApp) -> Vec<Choice> {
    choices_at(app.session.wall_at(app.session.now()))
}

/// The wall-clock time as `mail.snooze` takes it: in the zone the window shows its times in.
pub(crate) fn until(at: DateTime) -> String {
    format!("{}T{:02}:{:02}", at.date.iso(), at.hour, at.minute)
}

/// A time as a choice shows it: « 13:00 » today, « demain 08:00 », « lun. 08:00 » within the week, « 20 nov. 08:00 » later.
pub(crate) fn when(app: &MailApp, at: DateTime) -> String {
    let today = app.session.today();
    let clock = crate::dates::clock(at, app.session.prefs.twelve_hour);
    let days = at.date.to_days() - today.to_days();
    match days {
        0 => clock,
        1 => crate::i18n::fmt(tlc!("snooze", "Tomorrow {time}"), &[("time", &clock)]),
        2..=6 => format!("{} {clock}", crate::dates::weekday_short(at.date.weekday())),
        _ => format!("{} {clock}", crate::dates::month_day(at.date)),
    }
}

/// A time as a menu of times shows it beside its name: « 13:00 » today, « ven. 08:00 » within the week, « 20 nov. 08:00 »
/// later (« Demain matin » says tomorrow already).
pub(crate) fn when_offered(app: &MailApp, at: DateTime) -> String {
    let clock = crate::dates::clock(at, app.session.prefs.twelve_hour);
    match at.date.to_days() - app.session.today().to_days() {
        0 => clock,
        1..=6 => format!("{} {clock}", crate::dates::weekday_short(at.date.weekday())),
        _ => format!("{} {clock}", crate::dates::month_day(at.date)),
    }
}

/// When a message waiting in « Plus tard » comes back, as its row says it (or that its server could not bring it back);
/// `None` for any other message.
pub(crate) fn row_label(app: &MailApp, m: &Message) -> Option<String> {
    let z = m.snooze.filter(|z| z.waiting() && !z.placing)?;
    if app.session.keeps_local || snooze_hosted::record(&app.session, m.id).is_none() {
        return None;
    }
    if z.failed {
        return Some(tlc!("snooze", "Not back").to_string());
    }
    Some(when(app, app.session.wall_at(z.until)))
}

/// Whether the row of `m` wears the clock (it waits in « Plus tard »), and whether its server could not bring it back.
pub(crate) fn row_mark(app: &MailApp, m: &Message) -> Option<bool> {
    row_label(app, m).map(|_| m.snooze.is_some_and(|z| z.failed))
}

/// Whether « Plus tard » is offered for `ids`: a hosting page's tab whose mailbox's server keeps it.
pub(crate) fn offered(app: &MailApp, ids: &[Id]) -> bool {
    snooze_hosted::offered(&app.session, ids)
}

/// Whether the folder in view is a mailbox's « Plus tard » folder that its server keeps: its rows have Spark's quick actions,
/// the clock among them, to bring a message back now or give it another time.
pub(crate) fn in_view(app: &MailApp) -> bool {
    let s = &app.session;
    !s.keeps_local
        && s.view.module == mailcraft_engine::Module::Mail
        && s.mailbox
            .folder(s.view.folder)
            .is_some_and(|f| s.hosted_snooze.servers.contains(&f.account) && s.mailbox.snoozed_folder(f.account) == Some(f.id))
}

/// Whether one of `ids` waits in « Plus tard » and can be brought back now.
pub(crate) fn returnable(app: &MailApp, ids: &[Id]) -> bool {
    !app.session.keeps_local
        && ids.iter().any(|id| {
            snooze_hosted::record(&app.session, *id).is_some()
                && app.session.mailbox.message(*id).is_some_and(|m| m.snooze.is_some_and(|z| z.waiting() && !z.placing))
        })
}

/// Put `ids` away until `at` (a wall-clock time): whether it was done. A refusal is said as any command's is.
pub(crate) fn put_away(app: &mut MailApp, ids: &[Id], at: DateTime) -> bool {
    app.run("mail.snooze", json!({"ids": ids, "until": until(at)})).is_ok()
}

/// The menu of times for `ids`: the three times, a date and time of one's own (the window), and « Revenir maintenant » for
/// one that waits. `true` when the messages left the list (put away, or brought back).
pub(crate) fn menu(ui: &mut Ui, app: &mut MailApp, ids: &[Id]) -> bool {
    ui.set_min_width(250.0);
    let mut done = false;
    if offered(app, ids) {
        for choice in choices(app) {
            let shown = when_offered(app, choice.at);
            let resp = ui.add(egui::Button::new(label(&choice)).shortcut_text(shown).min_size(vec2(250.0, 0.0)));
            app.hit.add("snoozeChoice", choice_hit(choice.key), resp.rect);
            if resp.clicked() {
                done = put_away(app, ids, choice.at);
                ui.close();
            }
        }
        let own = ui.add(egui::Button::new(tlc!("snooze", "Choose a Date and Time…")).min_size(vec2(250.0, 0.0)));
        app.hit.add("snoozeChoice", choice_hit("own"), own.rect);
        if own.clicked() {
            open(app, ui.ctx(), ids);
            ui.close();
        }
    }
    if returnable(app, ids) {
        if offered(app, ids) {
            ui.separator();
        }
        let back = ui.add(egui::Button::new(tlc!("snooze", "Bring Back Now")).min_size(vec2(250.0, 0.0)));
        app.hit.add("snoozeChoice", choice_hit("back"), back.rect);
        if back.clicked() {
            done = app.run("mail.unsnooze", json!({"ids": ids})).is_ok();
            ui.close();
        }
    }
    done
}

fn choice_hit(key: &str) -> u64 {
    match key {
        "later" => 1,
        "tomorrow" => 2,
        "nextWeek" => 3,
        "own" => 4,
        _ => 5,
    }
}

/// « Plus tard » in the message's menu, for the messages selected.
pub(crate) fn context_menu(app: &mut MailApp, ui: &mut Ui) {
    let ids = app.session.sel.messages.clone();
    if !offered(app, &ids) && !returnable(app, &ids) {
        return;
    }
    ui.menu_button(tlc!("snooze", "Later"), |ui| {
        menu(ui, app, &ids);
    });
}

/// The clock beside the actions of the message being read, and its menu.
pub(crate) fn reading_button(app: &mut MailApp, ui: &mut Ui, m: &Message) {
    let ids = [m.id];
    if !offered(app, &ids) && !returnable(app, &ids) {
        return;
    }
    let tip = format!("{} ({})", tlc!("snooze", "Later"), key_text());
    let resp = crate::widgets::icon_button(ui, "clock", &tip, 17.0, true);
    app.hit.add("readingSnooze", m.id, resp.rect);
    egui::Popup::menu(&resp).show(|ui| {
        menu(ui, app, &ids);
    });
}

/// Open the window of times for `ids` (or the messages selected): the key's, and « Choisir une date et une heure… ».
pub(crate) fn open(app: &mut MailApp, ctx: &egui::Context, ids: &[Id]) {
    let ids = if ids.is_empty() { app.session.sel.messages.clone() } else { ids.to_vec() };
    if ids.is_empty() || !offered(app, &ids) {
        return;
    }
    let first = choices(app).into_iter().find(|c| c.key == "tomorrow").map(|c| c.at).unwrap_or_else(|| app.session.wall_at(app.session.now()));
    let window = Window { ids, date: first.date, minutes: u16::from(first.hour) * 60 + u16::from(first.minute), error: None, listed: false };
    ctx.data_mut(|d| d.insert_temp(window_id(), window));
}

/// Whether the window of times is open.
#[cfg(test)]
pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Window>(window_id())).is_some()
}

/// What the person is to be told of a server's refusal, and the runs to make: every frame of the window.
pub(crate) fn frame(app: &mut MailApp, ctx: &egui::Context) {
    let said = std::mem::take(&mut app.session.hosted_snooze.said);
    if !said.is_empty() && app.refused.is_none() {
        app.refused = Some(said.join("\n"));
    }
    for account in std::mem::take(&mut app.session.hosted_snooze.to_read) {
        let _ = app.run("ui.sync", json!({"account": account, "quiet": true}));
    }
    window(app, ctx);
}

/// The window of times: the three, then a day and a time of one's own, and « Plus tard » to put the messages away.
fn window(app: &mut MailApp, ctx: &egui::Context) {
    let Some(mut w) = ctx.data(|d| d.get_temp::<Window>(window_id())) else { return };
    let t = Tokens::get(ctx);
    let mut close = false;
    let mut put: Option<DateTime> = None;
    let open = crate::dialogs::window(ctx, tlc!("snooze", "Later"), 340.0, |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        for choice in choices(app) {
            let shown = when_offered(app, choice.at);
            let resp = ui.add(egui::Button::new(label(&choice)).shortcut_text(shown).min_size(vec2(ui.available_width(), 28.0)));
            app.hit.add("snoozeWindowChoice", choice_hit(choice.key), resp.rect);
            if resp.clicked() {
                put = Some(choice.at);
            }
        }
        ui.add_space(4.0);
        ui.separator();
        ui.label(egui::RichText::new(tlc!("snooze", "Choose a Date and Time…")).font(semibold(13.0)).color(t.text));
        ui.horizontal(|ui| {
            let today = app.session.today();
            ui.label(egui::RichText::new(crate::dates::weekday_month_day(w.date)).font(regular(13.0)).color(t.text));
            let calendar = crate::date_picker::Calendar {
                field: window_id().with("date"),
                current: Some(w.date),
                today,
                week_start: app.session.prefs.week_start,
                offer_none: false,
                name: tlc!("snooze", "Choose the day"),
            };
            if let Some(crate::date_picker::Picked::Day(day)) = calendar.show(ui) {
                w.date = day;
                w.error = None;
            }
            let twelve = app.session.prefs.twelve_hour;
            let shown = crate::dates::time((w.minutes / 60) as u8, (w.minutes % 60) as u8, twelve);
            let listed = w.listed;
            let combo = egui::ComboBox::from_id_salt(window_id().with("time")).selected_text(shown).width(96.0).show_ui(ui, |ui| {
                for step in 0..48u16 {
                    let minutes = step * 30;
                    let text = crate::dates::time((minutes / 60) as u8, (minutes % 60) as u8, twelve);
                    let item = ui.selectable_label(w.minutes == minutes, text);
                    // Opened on the time chosen, not at midnight.
                    if w.minutes == minutes && !listed {
                        item.scroll_to_me(Some(egui::Align::Center));
                    }
                    if item.clicked() {
                        w.minutes = minutes;
                        w.error = None;
                    }
                }
            });
            w.listed = combo.inner.is_some();
            app.hit.add("snoozeWindowTime", 0, combo.response.rect);
        });
        if let Some(error) = &w.error {
            ui.label(egui::RichText::new(error).font(regular(12.0)).color(t.overdue));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = crate::widgets::primary_button(ui, tlc!("snooze", "Put Away"), true);
                app.hit.add("snoozeWindowPut", 0, ok.rect);
                if ok.clicked() {
                    let at = DateTime { date: w.date, hour: (w.minutes / 60) as u8, minute: (w.minutes % 60) as u8, second: 0 };
                    put = Some(at);
                }
                if ui.button(tl!("Cancel")).clicked() {
                    close = true;
                }
            });
        });
    });
    if let Some(at) = put {
        let instant: Timestamp = app.session.instant_of(at);
        if instant <= app.session.now() {
            w.error = Some(tlc!("snooze", "Choose a time in the future.").to_string());
        } else {
            // Put away, or refused (said as any command's refusal is): either way the window goes.
            put_away(app, &w.ids.clone(), at);
            close = true;
        }
    }
    if close || !open {
        ctx.data_mut(|d| d.remove::<Window>(window_id()));
    } else {
        ctx.data_mut(|d| d.insert_temp(window_id(), w));
    }
}

/// « Plus tard » in the Home tab's Tags group, after Follow Up, where the mailbox's server keeps it: its times.
pub(crate) fn ribbon(app: &MailApp, tab: &mut crate::ribbon::Tab) {
    if app.ui.tab == "Message" || app.session.view.module != mailcraft_engine::Module::Mail || !snooze_hosted::available(&app.session) {
        return;
    }
    let Some(tags) = tab.groups.iter_mut().find(|g| g.title == "Tags") else { return };
    let at = tags.items.iter().position(|i| i.key == "followUp").map_or(tags.items.len(), |n| n + 1);
    tags.items.insert(
        at,
        crate::ribbon::Item {
            key: "snooze",
            icon: "clock",
            label: "Later",
            tip: "Put away until a time of your choosing",
            command: None,
            menu: Some(crate::ribbon::Menu::Snooze),
            size: crate::ribbon::Size::Big,
            checked: false,
            section: 3,
            rank: 3,
            plain: true,
            under: None,
            pending: false,
        },
    );
}

#[cfg(test)]
#[path = "snooze_hosted_tests.rs"]
mod tests;
