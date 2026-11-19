// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of snooze_hosted.rs.
//! In the window of a hosting page's tab whose mailbox's server keeps « Plus tard »: the times offered, the clock of a row and
//! of the message being read, the Home tab's entry, the key and its window of times, the folder's rows that say when each
//! message comes back, and nothing of it where the window keeps its mail or the server does not keep it. The window takes a time
//! typed to the minute (« 17:35 », « 17h35 »), refuses one that is no time or is past, in the person's zone; the folder « Plus
//! tard » comes right after Drafts in the pane.
//! And (`--ignored`, with `SNOOZE_SHOTS=<folder>`) the window drawn in French, light and dark, to PNG files.

use egui::{Event, Key, Modifiers, PointerButton};
use mailcraft_model::sync::ServerMessage;
use mailcraft_model::{FolderKind, Id, ServerFolder};
use mailcraft_time::{Date, DateTime};
use serde_json::json;

use super::*;
use crate::people_view::testing;

fn has(w: &testing::Window, kind: &str, id: u64) -> bool {
    w.state().hit.rect(kind, id).is_some()
}

fn click(w: &mut testing::Window, kind: &str, id: u64) {
    let pos = w.state().hit.rect(kind, id).unwrap_or_else(|| panic!("no {kind} {id} in the window")).center();
    testing::click(w, pos, PointerButton::Primary, 1);
    w.run_steps(3);
}

fn hover(w: &mut testing::Window, kind: &str, id: u64) {
    let pos = w.state().hit.rect(kind, id).unwrap_or_else(|| panic!("no {kind} {id} in the window")).center();
    w.hover_at(pos);
    w.run_steps(3);
}

fn at(y: i32, m: u8, d: u8, hour: u8, minute: u8) -> DateTime {
    DateTime { date: Date::new(y, m, d).unwrap(), hour, minute, second: 0 }
}

/// The sample's Inbox as a hosting page's tab shows it (`hosted`), its mail being on a server that keeps « Plus tard »
/// (`server`): each message with its id there, and the folder « Plus tard » there too. The window and the folder.
fn inbox(hosted: bool, server: bool) -> (testing::Window, Id) {
    let mut w = testing::window("view.mail");
    w.state_mut().session.keeps_local = !hosted;
    w.state_mut().execute("ui.dismissReminder", json!({"all": true})).unwrap();
    w.state_mut().execute("folder.select", json!({"kind": "inbox"})).unwrap();
    let s = &mut w.state_mut().session;
    let inbox = s.view.folder;
    let account = s.mailbox.folder(inbox).unwrap().account;
    s.mailbox.folder_mut(inbox).unwrap().server = Some(ServerFolder { id: "Min".into(), name: "INBOX".into(), ..Default::default() });
    let snoozed = s.mailbox.add_folder_free(account, None, "Plus tard").unwrap();
    s.mailbox.folder_mut(snoozed).unwrap().server = Some(ServerFolder { id: "MSnz".into(), name: "Plus tard".into(), ..Default::default() });
    let ids: Vec<Id> = s.mailbox.messages_in(inbox).map(|m| m.id).collect();
    for id in ids {
        let m = s.mailbox.message_mut(id).unwrap();
        m.server = Some(ServerMessage { id: format!("E{id}"), folder: "Min".into(), read: m.read, ..Default::default() });
        if m.message_id.is_empty() {
            m.message_id = format!("<m{id}@example.org>");
        }
    }
    if server {
        let email = s.mailbox.account(account).unwrap().email.clone();
        s.run("mail.setSnoozes", &json!({"account": email, "mailbox": "MSnz", "records": []})).unwrap();
    }
    w.run_steps(3);
    (w, snoozed)
}

fn drawn(w: &mut testing::Window) -> Vec<Id> {
    let rows = w.state_mut().session.rows();
    rows.iter().filter(|row| w.state().hit.rect("message", row.id).is_some()).map(|row| row.id).collect()
}

fn asked_until(w: &mut testing::Window) -> Vec<i64> {
    w.state_mut()
        .session
        .hosted_snooze
        .take()
        .into_iter()
        .filter_map(|a| match a.what {
            snooze_hosted::Asked::Put { until, .. } => Some(until),
            _ => None,
        })
        .collect()
}

#[test]
fn the_times_offered_are_later_today_tomorrow_morning_and_next_monday() {
    // Wednesday 14 October 2026, 10:12.
    let keys: Vec<(&str, DateTime)> = choices_at(at(2026, 10, 14, 10, 12)).into_iter().map(|c| (c.key, c.at)).collect();
    assert_eq!(keys, [("later", at(2026, 10, 14, 14, 0)), ("tomorrow", at(2026, 10, 15, 8, 0)), ("nextWeek", at(2026, 10, 19, 8, 0))]);
    // At 18:00 three hours on is too late for today.
    assert!(choices_at(at(2026, 10, 14, 18, 0)).iter().all(|c| c.key != "later"));
    // On a Monday, next week is the Monday after.
    let monday = choices_at(at(2026, 10, 19, 9, 0));
    assert_eq!(monday.iter().find(|c| c.key == "nextWeek").map(|c| c.at), Some(at(2026, 10, 26, 8, 0)));
    assert_eq!(until(at(2026, 10, 26, 8, 0)), "2026-10-26T08:00");
}

#[test]
fn a_rows_clock_offers_the_times_and_puts_the_message_away_at_once() {
    let (mut w, snoozed) = inbox(true, true);
    let first = drawn(&mut w)[0];
    hover(&mut w, "message", first);
    assert!(has(&w, "doneQuickSnooze", first), "the clock among the quick actions");
    click(&mut w, "doneQuickSnooze", first);
    assert!(has(&w, "snoozeChoice", 2), "tomorrow morning in its menu");
    click(&mut w, "snoozeChoice", 2);
    assert_eq!(w.state().session.mailbox.message(first).unwrap().folder, snoozed, "out of the Inbox at once");
    assert!(!drawn(&mut w).contains(&first));
    let tomorrow = w.state().session.instant_of(at(2026, 10, 15, 8, 0));
    assert_eq!(asked_until(&mut w), [tomorrow], "its server is asked, for tomorrow 8:00");
}

#[test]
fn nothing_of_it_where_the_window_keeps_its_mail_or_its_server_does_not_keep_it() {
    for (hosted, server) in [(false, true), (true, false)] {
        let (mut w, _) = inbox(hosted, server);
        let first = drawn(&mut w)[0];
        hover(&mut w, "message", first);
        assert!(!has(&w, "doneQuickSnooze", first), "hosted {hosted}, server {server}");
        let tab = crate::ribbon::describe_tab(w.state(), "Home");
        assert!(tab.groups.iter().flat_map(|g| &g.items).all(|i| i.key != "snooze"), "hosted {hosted}, server {server}");
    }
}

#[test]
fn the_home_tab_has_it_after_follow_up() {
    let (w, _) = inbox(true, true);
    let tab = crate::ribbon::describe_tab(w.state(), "Home");
    let tags = tab.groups.iter().find(|g| g.title == "Tags").unwrap();
    let keys: Vec<&str> = tags.items.iter().map(|i| i.key).collect();
    let follow = keys.iter().position(|k| *k == "followUp").unwrap();
    assert_eq!(keys[follow + 1], "snooze", "{keys:?}");
    assert_eq!(tags.items[follow + 1].menu, Some(crate::ribbon::Menu::Snooze));
}

#[test]
fn the_message_being_read_has_its_clock() {
    let (mut w, _) = inbox(true, true);
    let first = drawn(&mut w)[0];
    click(&mut w, "message", first);
    w.run_steps(5);
    assert!(has(&w, "readingSnooze", first));
    click(&mut w, "readingSnooze", first);
    assert!(has(&w, "snoozeChoice", 3), "next week in its menu");
}

#[test]
fn the_key_opens_the_window_of_times_which_refuses_a_time_past_and_puts_away_at_one_to_come() {
    let (mut w, snoozed) = inbox(true, true);
    let first = drawn(&mut w)[0];
    click(&mut w, "message", first);
    let shift_mod = Modifiers { command: true, shift: true, ..Default::default() };
    for pressed in [true, false] {
        w.input_mut().events.push(Event::Key { key: Key::L, physical_key: None, pressed, repeat: false, modifiers: shift_mod });
    }
    w.run_steps(4);
    let ctx = w.ctx.clone();
    assert!(is_open(&ctx), "Ctrl+Shift+L opens it");
    // A day of the past, chosen in the window: said, and nothing done.
    let mut window: Window = ctx.data(|d| d.get_temp(egui::Id::unique("snooze_hosted_window"))).unwrap();
    window.date = Date::new(2026, 10, 13).unwrap();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("snooze_hosted_window"), window.clone()));
    w.run_steps(3);
    click(&mut w, "snoozeWindowPut", 0);
    let shown: Window = ctx.data(|d| d.get_temp(egui::Id::unique("snooze_hosted_window"))).unwrap();
    assert!(shown.error.is_some(), "a time in the past is refused");
    let s = &w.state().session;
    let inbox = s.mailbox.folder_of_kind(s.mailbox.message(first).unwrap().account, FolderKind::Inbox);
    assert_eq!(Some(s.mailbox.message(first).unwrap().folder), inbox, "still in the Inbox");
    // Friday at 17:30: put away.
    window.date = Date::new(2026, 10, 16).unwrap();
    window.time = "17:30".into();
    window.error = None;
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("snooze_hosted_window"), window));
    w.run_steps(3);
    click(&mut w, "snoozeWindowPut", 0);
    assert!(!is_open(&ctx), "closed");
    assert_eq!(w.state().session.mailbox.message(first).unwrap().folder, snoozed);
    let friday = w.state().session.instant_of(at(2026, 10, 16, 17, 30));
    assert_eq!(asked_until(&mut w), [friday]);
}

#[test]
fn the_folders_rows_say_when_each_comes_back_and_the_clock_brings_it_back_now() {
    let (mut w, snoozed) = inbox(true, true);
    let ids = drawn(&mut w);
    let (a, b) = (ids[0], ids[1]);
    let email = {
        let s = &w.state().session;
        s.mailbox.account(s.mailbox.message(a).unwrap().account).unwrap().email.clone()
    };
    let tomorrow = w.state().session.instant_of(at(2026, 10, 15, 8, 0));
    put_away_for(&mut w, &email, &[(a, tomorrow, None), (b, tomorrow, Some("The mail server could not be reached."))]);
    let m = w.state().session.mailbox.message(a).unwrap().clone();
    assert_eq!(row_label(w.state(), &m).as_deref(), Some("Tomorrow 8:00 AM"));
    w.state_mut().session.prefs.twelve_hour = false;
    assert_eq!(row_label(w.state(), &m).as_deref(), Some("Tomorrow 08:00"), "on a 24-hour clock");
    let failed = w.state().session.mailbox.message(b).unwrap().clone();
    assert_eq!(row_label(w.state(), &failed).as_deref(), Some("Not back"));
    assert_eq!(row_mark(w.state(), &failed), Some(true));
    assert_eq!(testing::dialog(&w), None, "{:?}", w.state().refused);
    click(&mut w, "folder", snoozed);
    w.run_steps(5);
    assert_eq!(w.state().session.view.folder, snoozed);
    let listed = drawn(&mut w);
    assert_eq!(listed.len(), 2, "{listed:?}");
    let a = listed.into_iter().find(|id| *id == a || *id == b).unwrap();
    let a = if a == b { listed_other(&mut w, b) } else { a };
    hover(&mut w, "message", a);
    click(&mut w, "doneQuickSnooze", a);
    assert!(has(&w, "snoozeChoice", 5), "Bring Back Now");
    click(&mut w, "snoozeChoice", 5);
    let back = w.state().session.mailbox.message(a).unwrap();
    assert_ne!(back.folder, snoozed, "back at once");
    assert!(!back.read);
    assert!(w.state().session.hosted_snooze.asking());
}

/// The window of times opened by its key on the first message of the Inbox, its day set to `day`: the window and the message.
fn window_on(day: Date) -> (testing::Window, Id, Id) {
    let (mut w, snoozed) = inbox(true, true);
    let first = drawn(&mut w)[0];
    click(&mut w, "message", first);
    let shift_mod = Modifiers { command: true, shift: true, ..Default::default() };
    for pressed in [true, false] {
        w.input_mut().events.push(Event::Key { key: Key::L, physical_key: None, pressed, repeat: false, modifiers: shift_mod });
    }
    w.run_steps(4);
    let ctx = w.ctx.clone();
    let mut window: Window = ctx.data(|d| d.get_temp(egui::Id::unique("snooze_hosted_window"))).expect("open");
    window.date = day;
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("snooze_hosted_window"), window));
    w.run_steps(3);
    (w, first, snoozed)
}

/// `text` typed into the window's time, in place of what it holds, as the keyboard types it.
fn type_time(w: &mut testing::Window, text: &str) {
    click(w, "snoozeWindowTime", 0);
    let all = Modifiers { command: true, ..Default::default() };
    for pressed in [true, false] {
        w.input_mut().events.push(Event::Key { key: Key::A, physical_key: None, pressed, repeat: false, modifiers: all });
    }
    w.step();
    w.input_mut().events.push(Event::Text(text.into()));
    w.run_steps(3);
}

fn window_error(w: &testing::Window) -> Option<String> {
    w.ctx.data(|d| d.get_temp::<Window>(egui::Id::unique("snooze_hosted_window"))).and_then(|w| w.error)
}

#[test]
fn the_window_takes_a_time_typed_to_the_minute() {
    // Friday 16 October at 17:35, typed: put away then, in the person's zone.
    let (mut w, first, snoozed) = window_on(Date::new(2026, 10, 16).unwrap());
    type_time(&mut w, "17:35");
    click(&mut w, "snoozeWindowPut", 0);
    assert!(!is_open(&w.ctx.clone()), "closed");
    assert_eq!(w.state().session.mailbox.message(first).unwrap().folder, snoozed);
    let friday = w.state().session.instant_of(at(2026, 10, 16, 17, 35));
    assert_eq!(asked_until(&mut w), [friday]);
    // As French writes it, and Enter puts away as the button does.
    let (mut w, first, snoozed) = window_on(Date::new(2026, 10, 16).unwrap());
    type_time(&mut w, "9h05");
    testing::key(&mut w, Key::Enter);
    w.run_steps(3);
    assert_eq!(w.state().session.mailbox.message(first).unwrap().folder, snoozed, "Enter");
    assert_eq!(asked_until(&mut w), [w.state().session.instant_of(at(2026, 10, 16, 9, 5))]);
}

#[test]
fn the_time_takes_the_keyboard_as_the_window_opens_and_what_is_typed_replaces_it_and_escape_closes_it() {
    // The key, then « 17h35 » typed at once, no click: in place of the time it opened on, and Enter puts away.
    let (mut w, first, snoozed) = window_on(Date::new(2026, 10, 16).unwrap());
    w.input_mut().events.push(Event::Text("17h35".into()));
    w.run_steps(3);
    let typed_in: Window = w.ctx.data(|d| d.get_temp(egui::Id::unique("snooze_hosted_window"))).unwrap();
    assert_eq!(typed_in.time, "17h35");
    testing::key(&mut w, Key::Enter);
    w.run_steps(3);
    assert_eq!(w.state().session.mailbox.message(first).unwrap().folder, snoozed);
    assert_eq!(asked_until(&mut w), [w.state().session.instant_of(at(2026, 10, 16, 17, 35))]);
    // Escape closes it, nothing put away, the time's field having the keyboard.
    let (mut w, first, snoozed) = window_on(Date::new(2026, 10, 16).unwrap());
    testing::key(&mut w, Key::Escape);
    w.run_steps(3);
    assert!(!is_open(&w.ctx.clone()), "closed");
    assert_ne!(w.state().session.mailbox.message(first).unwrap().folder, snoozed);
    assert!(asked_until(&mut w).is_empty());
}

#[test]
fn the_window_refuses_what_is_no_time_and_a_time_typed_that_is_past() {
    for wrong in ["25:10", "tomorrow", "17:75"] {
        let (mut w, first, _) = window_on(Date::new(2026, 10, 16).unwrap());
        type_time(&mut w, wrong);
        click(&mut w, "snoozeWindowPut", 0);
        assert!(window_error(&w).is_some(), "{wrong}: said");
        assert!(is_open(&w.ctx.clone()), "{wrong}: the window stays");
        assert_ne!(w.state().session.mailbox.message(first).unwrap().folder, snooze_folder(&w), "{wrong}: nothing done");
        assert!(asked_until(&mut w).is_empty());
    }
    // Today (Wednesday 14 October, 16:00 here) at 15:55: past, refused; at 16:05: put away.
    let (mut w, first, snoozed) = window_on(Date::new(2026, 10, 14).unwrap());
    type_time(&mut w, "15:55");
    click(&mut w, "snoozeWindowPut", 0);
    assert!(window_error(&w).is_some(), "a time past is refused");
    assert!(asked_until(&mut w).is_empty());
    type_time(&mut w, "16:05");
    click(&mut w, "snoozeWindowPut", 0);
    assert_eq!(w.state().session.mailbox.message(first).unwrap().folder, snoozed);
    assert_eq!(asked_until(&mut w), [w.state().session.instant_of(at(2026, 10, 14, 16, 5))]);
}

#[test]
fn a_time_is_read_as_it_is_typed_in_any_of_its_usual_ways() {
    for (text, read) in [
        ("17:35", Some((17, 35))),
        ("17h35", Some((17, 35))),
        ("17H35", Some((17, 35))),
        ("17h", Some((17, 0))),
        ("17.35", Some((17, 35))),
        ("1735", Some((17, 35))),
        ("17 h 35", Some((17, 35))),
        ("9 h", Some((9, 0))),
        (" 9 ", Some((9, 0))),
        ("9:05", Some((9, 5))),
        ("5:35 pm", Some((17, 35))),
        ("5:35 p.m.", Some((17, 35))),
        ("8:00 a. m.", Some((8, 0))),
        ("12:10 AM", Some((0, 10))),
        ("25:10", None),
        ("17:75", None),
        ("930", None),
        ("１７３５", None),
        ("é", None),
        ("", None),
    ] {
        assert_eq!(typed(text), read, "{text:?}");
    }
    // What the window writes, it reads back, on either clock.
    for twelve in [false, true] {
        assert_eq!(typed(&crate::forms::kit::clock_text(17 * 60 + 35, twelve)), Some((17, 35)), "twelve {twelve}");
    }
}

/// The folder « Plus tard » of the account in view.
fn snooze_folder(w: &testing::Window) -> Id {
    let s = &w.state().session;
    let account = s.mailbox.folder(s.view.folder).unwrap().account;
    s.mailbox.snoozed_folder(account).unwrap()
}

#[test]
fn the_folder_comes_right_after_drafts_in_the_pane() {
    let (w, snoozed) = inbox(true, true);
    let s = &w.state().session;
    let account = s.mailbox.folder(snoozed).unwrap().account;
    let drafts = s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap();
    let (drafts_row, snoozed_row) = (w.state().hit.rect("folder", drafts).unwrap(), w.state().hit.rect("folder", snoozed).unwrap());
    assert!((snoozed_row.min.y - drafts_row.max.y).abs() < 1.0, "right under Drafts: {drafts_row:?} {snoozed_row:?}");
    // Where the window keeps its mail, as upstream lists a folder of the person's: by its name.
    let (w, snoozed) = inbox(false, true);
    let s = &w.state().session;
    let account = s.mailbox.folder(snoozed).unwrap().account;
    let drafts = s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap();
    let (drafts_row, snoozed_row) = (w.state().hit.rect("folder", drafts).unwrap(), w.state().hit.rect("folder", snoozed).unwrap());
    assert!(snoozed_row.min.y > drafts_row.max.y + 1.0, "not moved");
}

/// `put` put away as the window does (`mail.snooze`), the server saying yes (`mail.snoozeAnswered`), then its records read
/// (`mail.setSnoozes`): each message, when it comes back, and why its server could not bring it back.
fn put_away_for(w: &mut testing::Window, email: &str, put: &[(Id, i64, Option<&str>)]) {
    let s = &mut w.state_mut().session;
    let mut records = Vec::new();
    for (n, (id, until, failed)) in put.iter().enumerate() {
        s.run("mail.snooze", &json!({"ids": [id], "until": until})).unwrap();
        s.hosted_snooze.take();
        let record = format!("r{n}");
        s.run("mail.snoozeAnswered", &json!({"message": id, "ok": true, "record": record, "emailId": format!("E{id}"), "mailboxId": "MSnz"}))
            .unwrap();
        let mut said = json!({"id": record, "emailId": format!("E{id}"), "fromMailboxId": "Min", "until": until});
        if let Some(failed) = failed {
            said["failed"] = json!(failed);
        }
        records.push(said);
    }
    s.run("mail.setSnoozes", &json!({"account": email, "mailbox": "MSnz", "records": records})).unwrap();
    w.run_steps(3);
}

/// The other row drawn than `not`.
fn listed_other(w: &mut testing::Window, not: Id) -> Id {
    drawn(w).into_iter().find(|id| *id != not).unwrap()
}

#[test]
fn a_refusal_of_the_server_is_said_to_the_person() {
    let (mut w, _) = inbox(true, true);
    w.state_mut().session.hosted_snooze.said.push("'Quote' could not be put away: it stays where it was.".into());
    w.run_steps(3);
    assert_eq!(testing::dialog(&w), Some("refused"));
}

/// The window in French, light and dark, to PNG files in `SNOOZE_SHOTS`: a row's clock and its menu, the window of times, and
/// the folder « Plus tard » with the time each message comes back (one its server could not bring back).
#[test]
#[ignore = "draws to PNG files: SNOOZE_SHOTS=<folder> cargo test -p mailcraft-ui-egui --lib -- --ignored snooze_screens"]
fn snooze_screens_in_french() {
    let Some(dir) = std::env::var_os("SNOOZE_SHOTS").map(std::path::PathBuf::from) else { return };
    let _language = crate::i18n::RestoreLanguage::hold();
    for dark in [false, true] {
        for screen in ["k1-horloge-de-la-ligne", "k2-menu-des-heures", "k3-fenetre-des-heures", "k4-dossier-plus-tard"] {
            let (mut w, snoozed) = inbox(true, true);
            w.state_mut().execute("ui.language", json!({"language": "fr"})).unwrap();
            let _ = w.state_mut().run("ui.theme", json!({"value": if dark { "dark" } else { "light" }}));
            w.run_steps(4);
            let ids = drawn(&mut w);
            match screen {
                "k1-horloge-de-la-ligne" => {
                    hover(&mut w, "message", ids[1]);
                    hover(&mut w, "doneQuickSnooze", ids[1]);
                }
                "k2-menu-des-heures" => {
                    hover(&mut w, "message", ids[1]);
                    click(&mut w, "doneQuickSnooze", ids[1]);
                }
                "k3-fenetre-des-heures" => {
                    click(&mut w, "message", ids[1]);
                    let ctx = w.ctx.clone();
                    let state = w.state_mut();
                    open(state, &ctx, &[ids[1]]);
                    w.run_steps(4);
                }
                _ => {
                    let email = {
                        let s = &w.state().session;
                        s.mailbox.account(s.mailbox.message(ids[0]).unwrap().account).unwrap().email.clone()
                    };
                    let put: Vec<(Id, i64, Option<&str>)> = ids
                        .iter()
                        .take(3)
                        .enumerate()
                        .map(|(n, id)| {
                            let until = w.state().session.instant_of(at(2026, 10, 15 + n as u8 * 3, 8, 0));
                            (*id, until, (n == 2).then_some("Le serveur de messagerie n’a pas pu être joint."))
                        })
                        .collect();
                    put_away_for(&mut w, &email, &put);
                    click(&mut w, "folder", snoozed);
                    w.run_steps(4);
                }
            }
            w.run_steps(2);
            let image = w.render().expect("drawn");
            image.save(dir.join(format!("{screen}-{}.png", if dark { "sombre" } else { "clair" }))).expect("saved");
        }
    }
}
