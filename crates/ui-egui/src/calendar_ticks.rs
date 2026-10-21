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
//! change from one visit to the next. A tick is noted when the person ticks a calendar or clears it
//! (`calendar.showCalendar`), and the ticks noted are put back each time the list of an account's calendars is taken in.
//! A calendar the server lists for the first time is as it arrives: unticked, but the main one. A window that keeps its
//! mailbox keeps the ticks in it, and notes none here.

use std::collections::BTreeMap;

use mailcraft_engine::cmd::remote::keeper;
use mailcraft_model::Id;
use serde_json::Value;

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
    /// What a calendar of `id` is known by across visits, when a server keeps it.
    fn tick_key(&self, id: Id) -> Option<String> {
        let calendar = self.session.mailbox.calendar(id)?;
        let account = keeper(&self.session.mailbox, &calendar.origin)?;
        Some(key(&account.email, &calendar.origin.id))
    }

    /// `calendar.showCalendar` ran with `params`: in a tab that keeps nothing, whether that calendar is ticked now is
    /// noted for the next visit.
    pub(crate) fn remember_calendar_tick(&mut self, params: &Value) {
        if self.session.keeps_local {
            return;
        }
        let Some(id) = params.get("id").and_then(Value::as_u64) else { return };
        let (Some(key), Some(visible)) = (self.tick_key(id), self.session.mailbox.calendar(id).map(|c| c.visible)) else { return };
        if key.len() > MAX_KEY || (self.ui.calendar_ticks.len() >= MAX_TICKS && !self.ui.calendar_ticks.contains_key(&key)) {
            return;
        }
        self.ui.calendar_ticks.insert(key, visible);
    }

    /// The list of `account`'s calendars was taken in: in a tab that keeps nothing, its calendars are ticked as they were
    /// at the last visit. (Not an undo step: nothing the person did here.)
    pub(crate) fn apply_calendar_ticks(&mut self, account: Id) {
        if self.session.keeps_local || self.ui.calendar_ticks.is_empty() {
            return;
        }
        let Some(address) = self.session.mailbox.account(account).map(|a| a.email.clone()) else { return };
        let mut changed = false;
        for calendar in self.session.mailbox.calendars.iter_mut().filter(|c| c.origin.account == account) {
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
        let download = CalendarsDownload { account, email: ME.into(), complete: true, calendars: std::iter::once(main).chain(more).collect() };
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
        assert!(saved.contains(r#""calendarTicks":{"ada@example.org/":false,"ada@example.org/C2":true}"#), "{saved}");
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
