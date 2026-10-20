// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A believable fortnight on the stand-in, for looking at a window that reads it ([`demo`]): a mailbox with a few
//! messages, and three calendars around a day. Invented people at `example.org`, invented words.
//!
//! - **Ada's calendar** ([`DEMO_PERSON`]'s own, the default, which she may write): a meeting she organizes with three
//!   guests, a meeting she is invited to and has not answered, a whole-day event of two days, an event that names no
//!   zone (a floating one), a private one, a tentative one, one shown as free, one cancelled.
//! - **The team's calendar**, which she may write: a weekly meeting on Mondays at nine in Zurich, from the week before
//!   to five weeks after; around the end of October it crosses the night summer time ends, and stays at nine.
//! - **Grace's calendar**, a colleague's she may only read: her days off and a review.
//!
//! Everything is placed by the Monday of the week of the day given, so that a window opened on that day shows it. For
//! a look at what many events cost a window, [`crowd`] adds as many short ones as asked to her calendar.

use mailcraft_time::{Date, DateTime, format_iso_utc, format_rfc5322};
use serde_json::json;

use super::{Calendar, Event, Pim, StandIn};

/// The person the demo is for: the address a hosting page hands the window for it.
pub const DEMO_PERSON: &str = "ada@example.org";

/// A stand-in filled with the demo around `today` (see the module).
pub fn demo(today: Date) -> StandIn {
    let stand_in = StandIn::new();
    let monday = today.add_days(-i64::from(today.weekday().index()));
    let day = |offset: i64| monday.add_days(offset).iso();
    let at = |offset: i64, clock: &str| format!("{}T{clock}:00", day(offset));
    let zurich = Some("Europe/Zurich");
    let mailto = |who: &str| format!("mailto:{who}");
    let person = |name: &str, email: &str, roles: serde_json::Value, answer: &str| json!({"@type": "Participant", "name": name, "calendarAddress": mailto(email), "roles": roles, "participationStatus": answer});
    stand_in.with(|d| {
        // The mailbox: a few messages, so that a mail run has something to read too.
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        d.mailbox("Sent", None, Some("sent"));
        d.mailbox("Drafts", None, Some("drafts"));
        d.mailbox("Trash", None, Some("trash"));
        d.identity("Ada Marlow", DEMO_PERSON);
        let letters = [
            ("Grace Okafor", "grace@example.org", "Quarterly planning: the agenda", "Here is the agenda for Wednesday. See you there."),
            ("Bob Lindqvist", "bob@example.org", "Budget review on Tuesday", "I have added two lines to the budget sheet."),
            ("Carol Duval", "carol@example.org", "Book fair: the stand", "The stand is booked for Thursday and Friday."),
            ("Dan Moreau", "dan@example.org", "Lunch?", "Shall we have lunch on Tuesday?"),
        ];
        // On the Monday morning, one after the other, the first unread.
        let morning = DateTime::at_midnight(monday).to_timestamp(0) + 6 * 3_600;
        for (n, (name, email, subject, words)) in letters.iter().enumerate() {
            let sent = morning + 1_800 * i64::try_from(n).unwrap_or(0);
            let raw = format!(
                "From: {name} <{email}>\r\nTo: Ada Marlow <{DEMO_PERSON}>\r\nSubject: {subject}\r\nMessage-ID: <demo-{n}@example.org>\r\nDate: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{words}\r\n",
                format_rfc5322(sent, 120)
            );
            d.deliver(&inbox, raw.as_bytes(), if n == 0 { &[] } else { &["$seen"] }, &format_iso_utc(sent));
        }
        // The calendars.
        let pim = &mut d.pim;
        *pim = Pim::offered();
        pim.add_calendar(Calendar { flagged: true, color: Some("#2E8B57".into()), ..Calendar::new("ada", "Ada Marlow", true) });
        pim.add_calendar(Calendar { color: Some("#E07B1F".into()), ..Calendar::new("team", "Team", true) });
        pim.add_calendar(Calendar { color: Some("#7E57C2".into()), ..Calendar::new("grace", "Grace Okafor", false) });
        // A meeting Ada organizes, with three guests and a reminder.
        pim.add_event(
            Event::timed("budget", "ada", "Budget review", &at(1, "14:00"), zurich, "PT1H")
                .with("organizerCalendarAddress", json!(mailto(DEMO_PERSON)))
                .with(
                    "participants",
                    json!({
                        "ada": person("Ada Marlow", DEMO_PERSON, json!({"owner": true, "attendee": true}), "accepted"),
                        "bob": person("Bob Lindqvist", "bob@example.org", json!({"attendee": true}), "accepted"),
                        "carol": person("Carol Duval", "carol@example.org", json!({"attendee": true}), "tentative"),
                        "dan": person("Dan Moreau", "dan@example.org", json!({"attendee": true, "optional": true}), "needs-action"),
                    }),
                )
                .with("locations", json!({"l1": {"@type": "Location", "name": "Room 4"}}))
                .with("alerts", json!({"a1": {"@type": "Alert", "trigger": {"@type": "OffsetTrigger", "offset": "-PT15M"}}})),
        );
        // One she is invited to, and has not answered.
        pim.add_event(
            Event::timed("planning", "ada", "Quarterly planning", &at(2, "10:00"), zurich, "PT2H")
                .with("organizerCalendarAddress", json!(mailto("grace@example.org")))
                .with(
                    "participants",
                    json!({
                        "grace": person("Grace Okafor", "grace@example.org", json!({"owner": true}), "accepted"),
                        "ada": person("Ada Marlow", DEMO_PERSON, json!({"attendee": true}), "needs-action"),
                        "bob": person("Bob Lindqvist", "bob@example.org", json!({"attendee": true}), "accepted"),
                    }),
                )
                .with("description", json!("The agenda is in Grace's message.")),
        );
        pim.add_event(Event::whole_days("fair", "ada", "Book fair", &day(3), 2).with("freeBusyStatus", json!("free")));
        pim.add_event(Event::timed("lunch", "ada", "Lunch with Dan", &at(1, "12:30"), zurich, "PT1H"));
        // A floating one: no zone, read where the person is.
        pim.add_event(Event::timed("dentist", "ada", "Dentist", &at(4, "16:00"), None, "PT45M").with("privacy", json!("private")));
        pim.add_event(Event::timed("workshop", "ada", "Writing workshop", &at(8, "13:00"), zurich, "PT3H").with("status", json!("tentative")));
        pim.add_event(Event::timed("focus", "ada", "Focus time", &at(9, "08:00"), zurich, "PT2H").with("freeBusyStatus", json!("free")));
        pim.add_event(Event::timed("old-sync", "ada", "Old sync", &at(10, "15:00"), zurich, "PT30M").with("status", json!("cancelled")));
        pim.add_event(Event::whole_days("release", "ada", "Release day", &day(11), 1));
        // The team's weekly meeting, from the week before to five weeks after.
        pim.add_event(
            Event::timed("site", "team", "Site meeting", &at(-7, "09:00"), zurich, "PT1H30M")
                .repeating(json!({"@type": "RecurrenceRule", "frequency": "weekly", "count": 7}))
                .with("locations", json!({"l1": {"@type": "Location", "name": "Workshop B"}})),
        );
        // Grace's own, which Ada may only read.
        pim.add_event(Event::whole_days("grace-off", "grace", "Grace away", &day(7), 3));
        pim.add_event(Event::timed("grace-review", "grace", "Design review", &at(3, "15:00"), zurich, "PT1H"));
    });
    stand_in
}

/// Add `count` events of half an hour to Ada's calendar of a [`demo`] around `today`, spread over the days a window
/// reads at its first job (from thirty days back to a year ahead), between eight and six on the clock in Zurich.
pub fn crowd(stand_in: &StandIn, today: Date, count: usize) {
    let first = today.add_days(-30);
    stand_in.with(|d| {
        for n in 0..count {
            let n64 = i64::try_from(n).unwrap_or(i64::MAX);
            let day = first.add_days(n64.rem_euclid(395));
            let slot = (n64 / 395).rem_euclid(20);
            let start = format!("{}T{:02}:{:02}:00", day.iso(), 8 + slot / 2, (slot % 2) * 30);
            d.pim.add_event(Event::timed(&format!("crowd-{n}"), "ada", &format!("Slot {n}"), &start, Some("Europe/Zurich"), "PT30M"));
        }
    });
}
