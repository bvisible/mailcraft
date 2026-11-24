// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of `hosted_calendar` (the flows with a server are the browser build's).

use serde_json::json;

use crate::Session;

#[test]
fn a_hosting_pages_tab_neither_makes_nor_empties_a_calendar_of_its_own() {
    for hosted in [false, true] {
        let mut s = Session::at(crate::sample::mailbox(1_791_993_600, 0), 1_791_993_600, 0);
        s.keeps_local = !hosted;
        let made = s.run("calendar.addCalendar", &json!({"name": "Mine"}));
        assert_eq!(made.is_err(), hosted);
        if hosted {
            let why = s.run("calendar.moveToAccount", &json!({})).unwrap_err().to_string();
            assert!(why.contains(crate::cmd::kept_here::NO_OWN_CALENDAR), "{why}");
        }
    }
}
