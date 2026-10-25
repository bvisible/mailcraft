// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Which of a server's calendars are ticked, across the visits of a tab that keeps nothing of its own
//! ([`mailcraft_engine::Session::keeps_local`] false: a hosting page's).
//!
//! Such a tab starts every visit with a mailbox of its own making: the account's main calendar ticked and the others
//! not, as a calendar that arrives is (`sync.applyCalendars`), whatever the person ticked the last time, and the ids
//! here are new at each visit. So the person's ticks are kept with the interface's settings, which the browser keeps
//! ([`crate::UiState::calendar_ticks`]): by the account's address and the calendar's name on its server, which don't
//! change from one visit to the next. The ticks are noted whenever the person changes them (`calendar.showCalendar`,
//! `calendar.showOnly`, and Undo and Redo of either), every server's calendar as it is then, and put back on a calendar
//! when the list that makes it here is taken in, at the start of a visit: a later list (every ten minutes) leaves the
//! calendars here as the person has them now. A calendar the server lists for the first time is as it arrives:
//! unticked, but the main one. What was noted of a mailbox the page no longer hands over goes at the next visit. A
//! window that keeps its mailbox keeps the ticks in it, and notes none here.

use std::collections::{BTreeMap, HashSet};

use mailcraft_engine::cmd::remote::keeper;
use mailcraft_model::Id;

use crate::MailApp;

/// The most ticks kept, and the longest key.
const MAX_TICKS: usize = 2_000;
const MAX_KEY: usize = 600;

/// What a calendar is known by across visits: its account's address (in lower case) and its name on its server (`""`
/// for the account's main calendar).
pub fn key(address: &str, calendar: &str) -> String {
    format!("{}/{calendar}", address.trim().to_lowercase())
}

/// The ticks as they may be kept: no key longer than a key is, and not too many of them.
pub fn sanitize(ticks: &mut BTreeMap<String, bool>) {
    ticks.retain(|key, _| key.len() <= MAX_KEY && !key.chars().any(char::is_control));
    while ticks.len() > MAX_TICKS {
        if ticks.pop_last().is_none() {
            break;
        }
    }
}

impl MailApp {
    /// The person changed which calendars are ticked: in a tab that keeps nothing, every server's calendar is noted as it
    /// is now, for the next visit.
    pub(crate) fn remember_calendar_ticks(&mut self) {
        if self.session.keeps_local {
            return;
        }
        let mb = &self.session.mailbox;
        let ticks: Vec<(String, bool)> = mb
            .calendars
            .iter()
            .filter_map(|calendar| keeper(mb, &calendar.origin).map(|account| (key(&account.email, &calendar.origin.id), calendar.visible)))
            .filter(|(key, _)| key.len() <= MAX_KEY)
            .collect();
        for (key, visible) in ticks {
            if self.ui.calendar_ticks.len() < MAX_TICKS || self.ui.calendar_ticks.contains_key(&key) {
                self.ui.calendar_ticks.insert(key, visible);
            }
        }
    }

    /// The calendars of `account` here, by their ids: what a list taken in after this is to be compared with
    /// ([`apply_calendar_ticks`](Self::apply_calendar_ticks)).
    pub(crate) fn calendars_of(&self, account: Id) -> HashSet<Id> {
        self.session.mailbox.calendars.iter().filter(|c| c.origin.account == account).map(|c| c.id).collect()
    }

    /// The list of `account`'s calendars was taken in: in a tab that keeps nothing, the calendars it made here (none of
    /// `before`) are ticked as they were at the last visit; those that were here stay as the person has them. (Not an
    /// undo step: nothing the person did here.)
    pub(crate) fn apply_calendar_ticks(&mut self, account: Id, before: &HashSet<Id>) {
        if self.session.keeps_local || self.ui.calendar_ticks.is_empty() {
            return;
        }
        let Some(address) = self.session.mailbox.account(account).map(|a| a.email.clone()) else { return };
        let mut changed = false;
        let made = |c: &&mut mailcraft_model::Calendar| c.origin.account == account && !before.contains(&c.id);
        for calendar in self.session.mailbox.calendars.iter_mut().filter(made) {
            if let Some(ticked) = self.ui.calendar_ticks.get(&key(&address, &calendar.origin.id))
                && calendar.visible != *ticked
            {
                calendar.visible = *ticked;
                changed = true;
            }
        }
        if changed {
            self.session.mailbox_changed();
        }
    }

    /// In a tab that keeps nothing, what was noted of a mailbox the page doesn't hand over at this visit goes.
    pub(crate) fn prune_calendar_ticks(&mut self) {
        if self.session.keeps_local {
            return;
        }
        let handed: HashSet<String> = self.session.mailbox.accounts.iter().map(|a| a.email.trim().to_lowercase()).collect();
        self.ui.calendar_ticks.retain(|key, _| key.split_once('/').is_some_and(|(address, _)| handed.contains(address)));
    }
}

#[cfg(test)]
mod tests {
    use mailcraft_engine::Session;
    use mailcraft_model::remote::{CalendarsDownload, RemoteCalendar};
    use serde_json::json;

    use super::*;
    use crate::{Services, UiState};

    const ME: &str = "Ada@Example.org";
    /// Wednesday 2026-10-14 16:00 UTC.
    const NOW: i64 = 1_791_993_600;

    /// A visit of a hosting page's tab: a window that keeps nothing, with a JMAP account (after `extra` calendars of
    /// its own, so that the ids here differ from visit to visit), and the account's id.
    fn visit(extra: usize, ui: Option<UiState>) -> (MailApp, Id) {
        let mut s = Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0);
        for n in 0..extra {
            s.run("calendar.addCalendar", &json!({"name": format!("Mine {n}")})).unwrap();
        }
        let added = s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap/session"}}));
        let account = added.unwrap()["id"].as_u64().unwrap();
        s.keeps_local = false;
        let mut app = MailApp::new(s, Services::default());
        app.autosave = false;
        if let Some(ui) = ui {
            app.apply_prefs(ui);
        }
        (app, account)
    }

    /// The server's list: the main calendar and those named.
    fn list(app: &mut MailApp, account: Id, others: &[&str]) {
        let main = RemoteCalendar { id: "C1".into(), name: "Ada".into(), main: true, ..Default::default() };
        let more = others.iter().map(|id| RemoteCalendar { id: (*id).into(), name: format!("Calendar {id}"), ..Default::default() });
        let download = CalendarsDownload {
            account,
            email: ME.into(),
            complete: true,
            calendars: std::iter::once(main).chain(more).collect(),
            refused: Vec::new(),
        };
        let mut errors = Vec::new();
        app.take_in_calendar_list(account, Some(&download), None, false, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// Whether the calendar the server calls `remote` (`""`: the main one) is ticked here.
    fn ticked(app: &MailApp, account: Id, remote: &str) -> bool {
        let calendar = app.session.mailbox.calendars.iter().find(|c| c.origin.account == account && c.origin.id == remote);
        calendar.unwrap_or_else(|| panic!("no calendar {remote}")).visible
    }

    fn id_of(app: &MailApp, account: Id, remote: &str) -> Id {
        app.session.mailbox.calendars.iter().find(|c| c.origin.account == account && c.origin.id == remote).map(|c| c.id).unwrap()
    }

    #[test]
    fn the_ticks_of_calendars_are_remembered_by_server_id_across_visits() {
        let (mut app, account) = visit(0, None);
        list(&mut app, account, &["C2", "C3"]);
        assert!(ticked(&app, account, "") && !ticked(&app, account, "C2") && !ticked(&app, account, "C3"), "as they arrive");
        // The person ticks the team's calendar and clears the main one.
        app.run("calendar.showCalendar", json!({"id": id_of(&app, account, "C2"), "value": true})).unwrap();
        app.run("calendar.showCalendar", json!({"id": id_of(&app, account, ""), "value": false})).unwrap();
        // Saved by the shell with the other settings, as the browser keeps them.
        let saved = serde_json::to_string(&app.prefs()).unwrap();
        assert!(saved.contains(r#""calendarTicks":{"ada@example.org/":false,"ada@example.org/C2":true,"ada@example.org/C3":false}"#), "{saved}");
        let ui: UiState = serde_json::from_str(&saved).unwrap();
        // The next visit: other ids here, the same calendars there.
        let first = id_of(&app, account, "C2");
        let (mut next, account) = visit(3, Some(ui));
        list(&mut next, account, &["C2", "C3"]);
        assert_ne!(id_of(&next, account, "C2"), first);
        assert!(!ticked(&next, account, "") && ticked(&next, account, "C2") && !ticked(&next, account, "C3"));
        // And a later list (every ten minutes) leaves them as the person has them.
        next.run("calendar.showCalendar", json!({"id": id_of(&next, account, "C3"), "value": true})).unwrap();
        list(&mut next, account, &["C2", "C3"]);
        assert!(ticked(&next, account, "C3"));
    }

    #[test]
    fn a_calendar_new_on_the_server_starts_unticked_except_the_main_one() {
        let (mut app, account) = visit(0, None);
        list(&mut app, account, &["C2"]);
        app.run("calendar.showCalendar", json!({"id": id_of(&app, account, "C2"), "value": true})).unwrap();
        let ui = app.prefs();
        let (mut next, account) = visit(0, Some(ui));
        // The server lists one more since.
        list(&mut next, account, &["C2", "C4"]);
        assert!(ticked(&next, account, ""), "the main one, ticked as it arrives");
        assert!(ticked(&next, account, "C2"), "as the person left it");
        assert!(!ticked(&next, account, "C4"), "new: unticked");
    }

    #[test]
    fn a_later_list_leaves_the_calendars_as_the_person_has_them_now() {
        // The last visit had the team's calendar ticked.
        let (mut app, account) = visit(0, None);
        list(&mut app, account, &["C2"]);
        app.run("calendar.showCalendar", json!({"id": id_of(&app, account, "C2"), "value": true})).unwrap();
        let (mut app, account) = visit(0, Some(app.prefs()));
        list(&mut app, account, &["C2"]);
        assert!(ticked(&app, account, "C2"));
        // Show Only This Calendar, the main one: noted, and a later list (every ten minutes) doesn't tick C2 again.
        app.run("calendar.showOnly", json!({"id": id_of(&app, account, "")})).unwrap();
        assert_eq!(app.ui.calendar_ticks.get(&key(ME, "C2")), Some(&false));
        list(&mut app, account, &["C2"]);
        assert!(ticked(&app, account, "") && !ticked(&app, account, "C2"));
        // Undone: ticked again, and noted so.
        app.run("edit.undo", json!({})).unwrap();
        assert!(ticked(&app, account, "C2"));
        assert_eq!(app.ui.calendar_ticks.get(&key(ME, "C2")), Some(&true));
        // Redone: as Show Only had it.
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.ui.calendar_ticks.get(&key(ME, "C2")), Some(&false));
        // And a tick cleared by hand stays cleared past the next list too.
        app.run("calendar.showCalendar", json!({"id": id_of(&app, account, ""), "value": false})).unwrap();
        list(&mut app, account, &["C2"]);
        assert!(!ticked(&app, account, ""));
    }

    #[test]
    fn what_was_noted_of_a_mailbox_not_handed_over_goes() {
        let mut ui = UiState::default();
        ui.calendar_ticks.insert(key(ME, "C2"), true);
        ui.calendar_ticks.insert(key("bo@example.org", "C9"), true);
        let (app, _) = visit(0, Some(ui));
        assert_eq!(app.ui.calendar_ticks.keys().collect::<Vec<_>>(), [&key(ME, "C2")]);
    }

    #[test]
    fn a_window_that_keeps_its_mailbox_notes_no_ticks() {
        let (mut app, account) = visit(0, None);
        app.session.keeps_local = true;
        list(&mut app, account, &["C2"]);
        app.run("calendar.showCalendar", json!({"id": id_of(&app, account, "C2"), "value": true})).unwrap();
        assert!(app.ui.calendar_ticks.is_empty(), "the mailbox keeps them");
        // And what is kept is held to a size, whatever the settings file says.
        let mut many: BTreeMap<String, bool> = (0..MAX_TICKS + 10).map(|n| (key(ME, &format!("C{n}")), true)).collect();
        many.insert("x".repeat(MAX_KEY + 1), true);
        sanitize(&mut many);
        assert_eq!(many.len(), MAX_TICKS);
        assert!(many.keys().all(|key| key.len() <= MAX_KEY));
    }
}
