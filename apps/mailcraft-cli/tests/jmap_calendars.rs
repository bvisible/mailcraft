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
        let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
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
            plain_unless_said: false,
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
    // The main calendar under its name, and the team's beside it, unticked and read-only, as every one is until writing
    // is carried; and both keep only the basics of an event.
    let main = rig.calendar("");
    assert_eq!((main.name.as_str(), main.visible, main.read_only, main.plain), ("Ada", true, true, true));
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
    // A new appointment can't go to a calendar of the server: it is read-only, and says so.
    let refused = rig.s.run("calendar.new", &json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00"}));
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
