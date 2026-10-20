// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's calendars, from end to end: a calendar job worked out by hand (as the window works it out),
//! `mailcraft_jmap::sync_pim` over the in-process JMAP server (driven by `block_on`, as a host on one thread drives
//! it), then the engine's `sync.applyCalendars` and `sync.applyEvents` for what came back. Nothing here leaves this
//! machine; every name, address and id is invented.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicBool;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::remote::{apply_calendars, apply_events};
use mailcraft_jmap::standin::{Calendar, Event, Pim, StandIn, TOKEN};
use mailcraft_jmap::{Auth, Client, block_on, sync_pim};
use mailcraft_model::remote::CalendarAsk;
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use mailcraft_model::{Id, Mailbox};
use serde_json::json;

const ADDRESS: &str = "ada@example.org";
const SESSION: &str = "https://jmap.standin.example/jmap/session";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const DAY: i64 = 86_400;

/// A session with one JMAP account, and a server with Ada's calendar, the team's (which she may only read) and a
/// week of events in them.
struct Rig {
    server: StandIn,
    s: Session,
    account: Id,
    /// What the server calls the main calendar, as the last list said.
    main: String,
}

impl Rig {
    fn new() -> Rig {
        Rig::with(Session::at(Mailbox::with_local_account("", ""), NOW, 0))
    }

    /// The same, with the calendar shown in Zurich.
    fn in_zurich() -> Rig {
        Rig::with(Session::in_zone(Mailbox::with_local_account("", ""), NOW, mailcraft_time::Zone::named("Europe/Zurich").unwrap()))
    }

    fn with(mut s: Session) -> Rig {
        let added = s.run("account.add", &json!({"email": ADDRESS, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
        let server = StandIn::new();
        server.with(|d| {
            d.pim = Pim::offered();
            d.pim.add_calendar(Calendar { flagged: true, ..Calendar::new("ada", "Ada", true) });
            d.pim.add_calendar(Calendar::new("team", "Team", false));
            d.pim.add_event(Event::timed("budget", "ada", "Budget review", "2026-10-15T09:00:00", Some("Europe/Zurich"), "PT1H"));
            d.pim.add_event(Event::whole_days("fair", "ada", "Book fair", "2026-10-19", 2));
            d.pim.add_event(Event::timed("standup", "team", "Team stand-up", "2026-10-16T08:30:00", Some("Europe/Zurich"), "PT15M"));
        });
        Rig { server, account: added["id"].as_u64().unwrap(), s, main: String::new() }
    }

    /// A job for the days around today, of the calendars named (`""`: the main one), with the list when `lists`.
    fn job(&self, lists: bool, asks: &[&str]) -> JmapPimRequest {
        JmapPimRequest {
            account: self.account,
            email: ADDRESS.into(),
            session: SESSION.into(),
            bearer: true,
            me: vec![ADDRESS.into()],
            zone: "Europe/Zurich".into(),
            now: NOW,
            from: NOW - 7 * DAY,
            to: NOW + 14 * DAY,
            lists,
            main: self.main.clone(),
            asks: asks.iter().map(|calendar| CalendarAsk { calendar: calendar.to_string(), series: Vec::new() }).collect(),
            ..Default::default()
        }
    }

    /// One job, as a host does it, and its answer taken in as the window takes it: the list, then the main calendar's
    /// events, then the others'.
    fn run(&mut self, request: &JmapPimRequest) -> JmapPimResult {
        let client = block_on(Client::connect(self.server.clone(), &request.session, &Auth::bearer(TOKEN))).unwrap();
        let result = block_on(sync_pim(&client, request, &AtomicBool::new(false)));
        if let Some(list) = &result.calendars {
            apply_calendars(&mut self.s, list).unwrap();
            if let Some(main) = list.calendars.iter().find(|c| c.main) {
                self.main = main.id.clone();
            }
        }
        for download in result.events.iter().chain(&result.more_events) {
            apply_events(&mut self.s, download).unwrap();
        }
        result
    }

    /// The calendar of the account that the server calls `name` here (`""`: the main one).
    fn calendar(&self, name: &str) -> &mailcraft_model::Calendar {
        self.s
            .mailbox
            .calendars
            .iter()
            .find(|c| c.origin.account == self.account && c.origin.id == name)
            .unwrap_or_else(|| panic!("no calendar {name} here"))
    }

    /// The titles of the events in the calendar `name`, in order.
    fn titles(&self, name: &str) -> Vec<String> {
        let calendar = self.calendar(name).id;
        let mut titles: Vec<String> = self.s.mailbox.events.values().filter(|e| e.calendar == calendar).map(|e| e.title.clone()).collect();
        titles.sort();
        titles
    }
}

#[test]
fn a_first_read_brings_the_calendars_and_the_events_of_the_stretch() {
    let mut rig = Rig::new();
    let result = rig.run(&rig.job(true, &[""]));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    // The main calendar under its name, Ada's to write, and the team's beside it, unticked and hers only to read; and
    // both keep only the basics of an event.
    let main = rig.calendar("");
    assert_eq!((main.name.as_str(), main.visible, main.read_only, main.plain), ("Ada", true, false, true));
    let team = rig.calendar(&Pim::calendar_id("team"));
    assert_eq!((team.name.as_str(), team.visible, team.read_only, team.plain), ("Team", false, true, true));
    assert_eq!(rig.titles(""), ["Book fair", "Budget review"]);
    // A meeting at nine in Zurich, summer time: seven o'clock UTC, the zone kept.
    let budget = rig.s.mailbox.events.values().find(|e| e.title == "Budget review").unwrap();
    assert_eq!((budget.start.hour, budget.start_zone.as_str()), (7, "Europe/Zurich"));
    assert_eq!(budget.origin.id, Pim::event_id("budget"));
    // A whole day keeps its days.
    let fair = rig.s.mailbox.events.values().find(|e| e.title == "Book fair").unwrap();
    assert!(fair.all_day);
    assert_eq!((fair.start.date.day, fair.end.date.day), (19, 21));
    assert_eq!(rig.main, Pim::calendar_id("ada"));
    // A new appointment goes to the main calendar, noted for its server; not to the team's, which says why.
    rig.s.run("calendar.new", &json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00"})).unwrap();
    assert_eq!(rig.s.mailbox.event_changes.len(), 1);
    let team = rig.calendar(&Pim::calendar_id("team")).id;
    let refused = rig.s.run("calendar.new", &json!({"title": "Stand-up", "start": "2026-10-15T08:00", "end": "2026-10-15T08:15", "calendar": team}));
    assert!(refused.unwrap_err().to_string().contains("can only be read"));
}

#[test]
fn a_second_whole_read_removes_what_the_server_no_longer_lists() {
    let mut rig = Rig::new();
    rig.run(&rig.job(true, &[""]));
    rig.server.with(|d| d.pim.events.retain(|e| e.raw != "budget"));
    let result = rig.run(&rig.job(false, &[""]));
    assert!(result.events.as_ref().unwrap().complete);
    assert_eq!(rig.titles(""), ["Book fair"]);
}

#[test]
fn a_read_cut_short_adds_and_updates_and_removes_nothing() {
    let mut rig = Rig::new();
    rig.run(&rig.job(true, &[""]));
    rig.server.with(|d| {
        d.pim.events.retain(|e| e.raw != "budget");
        d.pim.events[0].props.insert("title".into(), json!("Book fair, hall 2"));
        d.pim.add_event(Event::timed("lunch", "ada", "Lunch with Grace", "2026-10-15T12:00:00", Some("Europe/Zurich"), "PT1H"));
        // A server that expands no more than three weeks at once: the stretch of the job is cut.
        d.pim.span = "P20D".into();
    });
    let result = rig.run(&rig.job(false, &[""]));
    assert!(!result.events.as_ref().unwrap().complete);
    assert_eq!(rig.titles(""), ["Book fair, hall 2", "Budget review", "Lunch with Grace"]);
}

#[test]
fn an_event_moved_on_the_server_moves_here_and_is_not_a_second_one() {
    let mut rig = Rig::new();
    rig.run(&rig.job(true, &[""]));
    let before = rig.s.mailbox.events.values().find(|e| e.title == "Budget review").map(|e| e.id).unwrap();
    rig.server.with(|d| d.pim.events[0].start = "2026-10-15T14:00:00".into());
    rig.run(&rig.job(false, &[""]));
    let moved: Vec<_> = rig.s.mailbox.events.values().filter(|e| e.title == "Budget review").collect();
    assert_eq!(moved.len(), 1);
    assert_eq!((moved[0].id, moved[0].start.hour), (before, 12), "two in the afternoon in Zurich is noon UTC");
}

#[test]
fn a_calendar_the_server_drops_goes_with_its_events() {
    let mut rig = Rig::new();
    rig.run(&rig.job(true, &[""]));
    let team = Pim::calendar_id("team");
    rig.run(&rig.job(false, &["", &team]));
    assert_eq!(rig.titles(&team), ["Team stand-up"]);
    rig.server.with(|d| {
        d.pim.calendar_list.retain(|c| c.raw != "team");
        d.pim.events.retain(|e| e.raw != "standup");
    });
    let result = rig.run(&rig.job(true, &["", &team]));
    assert!(result.more_events.is_empty(), "no download of it: the list takes it away");
    assert!(!rig.s.mailbox.calendars.iter().any(|c| c.origin.id == team));
    assert!(!rig.s.mailbox.events.values().any(|e| e.title == "Team stand-up"));
    assert_eq!(rig.titles(""), ["Book fair", "Budget review"]);
}

// A series is one event with a rule here, its occurrences told from it.

/// Ada's weekly site meeting on the server: Mondays at nine in Zurich, six of them from 2026-10-12, across the night
/// summer time ends (2026-10-25), with `change` made to it first.
fn weekly(rig: &Rig, change: impl FnOnce(Event) -> Event) {
    let site = Event::timed("site", "ada", "Site meeting", "2026-10-12T09:00:00", Some("Europe/Zurich"), "PT1H")
        .repeating(json!({"@type": "RecurrenceRule", "frequency": "weekly", "count": 6}));
    let site = change(site);
    rig.server.with(|d| {
        d.pim.add_event(site);
    });
}

impl Rig {
    /// A job as the window works it out: the calendars named, each with the series it expects in the stretch.
    fn window_job(&self, lists: bool, asks: &[&str]) -> JmapPimRequest {
        let mut job = self.job(lists, asks);
        for ask in &mut job.asks {
            ask.series = mailcraft_engine::cmd::remote::expected_series(&self.s, self.account, &ask.calendar, job.from, job.to);
        }
        job
    }

    /// When the calendar shows something called `title` between the 12th and the 27th, by its clock.
    fn shown(&mut self, title: &str) -> Vec<String> {
        let listed = self.s.run("calendar.list", &json!({"from": "2026-10-12", "to": "2026-10-27"})).unwrap();
        listed["events"].as_array().unwrap().iter().filter(|e| e["title"] == title).map(|e| e["start"].as_str().unwrap().to_string()).collect()
    }
}

#[test]
fn a_weekly_series_reads_as_one_event_with_a_rule() {
    let mut rig = Rig::in_zurich();
    weekly(&rig, |site| site);
    let result = rig.run(&rig.window_job(true, &[""]));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let series: Vec<_> = rig.s.mailbox.events.values().filter(|e| e.title.starts_with("Site meeting")).collect();
    assert_eq!(series.len(), 1, "one event, not one per Monday");
    let rule = series[0].recurrence.as_ref().unwrap();
    assert_eq!((rule.freq, rule.count), (mailcraft_model::Freq::Weekly, Some(6)));
    assert_eq!((series[0].origin.id.as_str(), series[0].start_zone.as_str()), (Pim::event_id("site").as_str(), "Europe/Zurich"));
    assert!(series[0].exceptions.is_empty());
}

#[test]
fn a_series_keeps_its_hour_either_side_of_the_night_summer_time_ends() {
    let mut rig = Rig::in_zurich();
    weekly(&rig, |site| site);
    rig.run(&rig.window_job(true, &[""]));
    let mondays = ["2026-10-12T09:00:00", "2026-10-19T09:00:00", "2026-10-26T09:00:00"];
    assert_eq!(rig.shown("Site meeting"), mondays, "nine on both sides of the 25th");
    // Read again, as the window reads the days on screen: still one series, still at nine.
    rig.run(&rig.window_job(false, &[""]));
    assert_eq!(rig.s.mailbox.events.values().filter(|e| e.title == "Site meeting").count(), 1);
    assert_eq!(rig.shown("Site meeting"), mondays);
}

#[test]
fn a_day_the_master_excludes_has_no_event() {
    let mut rig = Rig::in_zurich();
    weekly(&rig, |site| Event { excluded: vec!["2026-10-19T09:00:00".into()], ..site });
    rig.run(&rig.window_job(true, &[""]));
    assert_eq!(rig.shown("Site meeting"), ["2026-10-12T09:00:00", "2026-10-26T09:00:00"]);
    let series = rig.s.mailbox.events.values().find(|e| e.recurrence.is_some()).unwrap();
    assert_eq!(series.exceptions, [mailcraft_time::Date::new(2026, 10, 19).unwrap()]);
}

#[test]
fn an_occurrence_moved_on_the_server_is_an_exception_here() {
    let mut rig = Rig::in_zurich();
    weekly(&rig, |site| site.changing("2026-10-19T09:00:00", json!({"start": "2026-10-19T11:00:00"})));
    rig.run(&rig.window_job(true, &[""]));
    assert_eq!(rig.shown("Site meeting"), ["2026-10-12T09:00:00", "2026-10-19T11:00:00", "2026-10-26T09:00:00"]);
    // The moved one is an event of its own that says which Monday of which series it stands for.
    let own: Vec<_> = rig.s.mailbox.events.values().filter(|e| e.recurrence.is_none() && e.title == "Site meeting").collect();
    assert_eq!(own.len(), 1);
    assert_eq!((own[0].origin.series.as_str(), own[0].origin.occurrence), (Pim::event_id("site").as_str(), mailcraft_time::Date::new(2026, 10, 19)));
    assert_eq!(own[0].origin.id, Pim::occurrence_id("site", "2026-10-19T09:00:00"));
}

#[test]
fn an_occurrence_the_server_cancelled_is_gone_here() {
    let mut rig = Rig::in_zurich();
    weekly(&rig, |site| site.changing("2026-10-26T09:00:00", json!({"title": "Site meeting (room B)"})));
    rig.run(&rig.window_job(true, &[""]));
    assert_eq!(rig.shown("Site meeting").len(), 2);
    assert_eq!(rig.shown("Site meeting (room B)"), ["2026-10-26T09:00:00"]);
    // Somebody leaves out the 19th and the changed 26th on the server: both go here at the next whole read.
    rig.server.with(|d| {
        let site = d.pim.events.iter_mut().find(|e| e.raw == "site").unwrap();
        site.overrides.clear();
        site.excluded = vec!["2026-10-19T09:00:00".into(), "2026-10-26T09:00:00".into()];
    });
    rig.run(&rig.window_job(false, &[""]));
    assert_eq!(rig.shown("Site meeting"), ["2026-10-12T09:00:00"]);
    assert!(rig.shown("Site meeting (room B)").is_empty());
    // And the series itself, when the server deletes it: the next job asks for it by name, and it goes.
    rig.server.with(|d| d.pim.events.retain(|e| e.raw != "site"));
    let result = rig.run(&rig.window_job(false, &[""]));
    assert_eq!(result.events.as_ref().unwrap().gone, [Pim::event_id("site")]);
    assert!(!rig.s.mailbox.events.values().any(|e| e.title.starts_with("Site meeting")));
}

#[test]
fn an_invitation_about_one_occurrence_finds_its_series() {
    let mut rig = Rig::in_zurich();
    weekly(&rig, |site| {
        site.with("organizerCalendarAddress", json!("mailto:bo@example.org")).with(
            "participants",
            json!({
                "bo": {"calendarAddress": "mailto:bo@example.org", "roles": {"owner": true}, "participationStatus": "accepted"},
                "ada": {"calendarAddress": "mailto:ada@example.org", "roles": {"attendee": true}, "participationStatus": "accepted"},
            }),
        )
    });
    rig.run(&rig.window_job(true, &[""]));
    let series = rig.s.mailbox.events.values().find(|e| e.recurrence.is_some()).map(|e| (e.id, e.origin.guest)).unwrap();
    assert!(series.1, "Bo's meeting");
    // Bo moves the 19th by mail: the invitation names that occurrence.
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:site@standin.example\r\nSUMMARY:Site meeting\r\n\
               RECURRENCE-ID:20261019T070000Z\r\nDTSTART:20261019T090000Z\r\nDTEND:20261019T100000Z\r\n\
               ORGANIZER;CN=Bo Marsh:mailto:bo@example.org\r\nATTENDEE;PARTSTAT=NEEDS-ACTION;CN=Ada:mailto:ada@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let inbox = rig.s.mailbox.folders.iter().find(|f| f.kind == mailcraft_model::FolderKind::Inbox).map(|f| f.id).unwrap();
    let message = mailcraft_model::Message {
        folder: inbox,
        message_id: "moved@example.org".into(),
        from: mailcraft_model::Address::new("Bo Marsh", "bo@example.org"),
        to: vec![mailcraft_model::Address::new("Ada", ADDRESS)],
        subject: "Site meeting".into(),
        attachments: vec![mailcraft_model::Attachment::new("invite.ics", "text/calendar", ics.as_bytes().to_vec())],
        ..Default::default()
    };
    let id = rig.s.mailbox.add_message(message).unwrap();
    let said = rig.s.run("calendar.invitation", &json!({"message": id})).unwrap();
    assert_eq!((said["event"].as_u64(), said["occurrence"].as_str()), (Some(series.0), Some("2026-10-19")), "{said}");
}
