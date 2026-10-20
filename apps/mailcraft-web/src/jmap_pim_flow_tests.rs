// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a JMAP account's calendars from the window to the server and back.

//! The whole way of a JMAP account's calendars: the window's calendar job (`jmap_pim.rs`), the browser runner's core
//! ([`super::pim`], what `jmap_pim_service` runs in the browser) and the in-process server, on the test's own thread:
//! the host answers each job at once, as the browser's task answers it when the window next looks. The window is a
//! hosting page's: its credential is the page's, which renews it as the page would, from what the server holds.
//! Nothing here leaves this machine; every name, address and id is invented.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use mailcraft_engine::Session;
use mailcraft_jmap::standin::{Calendar, Event, Fault, Pim, SESSION, StandIn, TOKEN};
use mailcraft_jmap::{Auth, block_on};
use mailcraft_model::{Id, Mailbox};
use mailcraft_time::Zone;
use mailcraft_ui_egui::jmap_pim::JmapPimJob;
use mailcraft_ui_egui::{MailApp, Services};
use serde_json::{Value, json};

use crate::credentials::{Credentials, Renewal};

const ME: &str = "ada@example.org";
/// Wednesday 2026-10-14, 16:00 UTC: six in the evening in Zurich.
const NOW: i64 = 1_791_993_600;
const SECOND: f64 = 1_000.0;
const MINUTE: f64 = 60.0 * SECOND;

/// The page's credential: the token it handed over, and a new one (whatever the server holds now) when asked.
struct Page {
    token: RefCell<String>,
    server: StandIn,
    renewals: RefCell<u32>,
}

impl Credentials for Page {
    fn auth(&self, _email: &str) -> Option<Auth> {
        Some(Auth::bearer(self.token.borrow().clone()))
    }

    async fn renew(&self, _email: &str) -> Renewal {
        *self.renewals.borrow_mut() += 1;
        match self.server.with(|d| d.token.clone()) {
            Some(token) => {
                *self.token.borrow_mut() = token.clone();
                Renewal::Given(Auth::bearer(token))
            }
            None => Renewal::Reload,
        }
    }
}

/// The window, in Zurich, with a JMAP account and the calendar in front; the server with Ada's calendar (the
/// default), the team's and a colleague's she may only read; and the time of the last look (milliseconds).
struct Flow {
    app: MailApp,
    server: StandIn,
    page: Rc<Page>,
    account: Id,
    at: f64,
}

impl Flow {
    fn start() -> Flow {
        let server = StandIn::new();
        server.with(|d| {
            d.mailbox("Inbox", None, Some("inbox"));
            d.pim = Pim::offered();
            d.pim.add_calendar(Calendar { flagged: true, ..Calendar::new("ada", "Ada", true) });
            d.pim.add_calendar(Calendar::new("team", "Team", true));
            d.pim.add_calendar(Calendar::new("grace", "Grace", false));
            d.pim.add_event(Event::timed("budget", "ada", "Budget review", "2026-10-15T09:00:00", Some("Europe/Zurich"), "PT1H"));
            d.pim.add_event(Event::whole_days("fair", "ada", "Book fair", "2026-10-16", 1));
            d.pim.add_event(Event::timed("standup", "team", "Team stand-up", "2026-10-15T08:30:00", Some("Europe/Zurich"), "PT15M"));
        });
        let page = Rc::new(Page { token: RefCell::new(TOKEN.into()), server: server.clone(), renewals: RefCell::new(0) });
        let session = Session::in_zone(Mailbox::with_local_account("", ""), NOW, Zone::named("Europe/Zurich").unwrap());
        let mut app = MailApp::new(session, Services::default());
        app.autosave = false;
        // The mail runs are not this flow's: a runner that never answers, so that the account is one the window may ask.
        app.services.jmap = Some(Box::new(|_| mpsc::channel().1));
        app.services.jmap_host_credentials = true;
        let (to, credentials) = (server.clone(), page.clone());
        app.services.jmap_pim = Some(Box::new(move |job: JmapPimJob| {
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(block_on(super::pim(to.clone(), &job, Some(credentials.as_ref()), |_| std::future::ready(()))));
            rx
        }));
        let added = app.execute("account.add", json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
        app.execute("view.calendar", json!({})).unwrap();
        let account = added["id"].as_u64().unwrap();
        let mut flow = Flow { app, server, page, account, at: 0.0 };
        assert!(flow.look(0.0), "the first look asks at once");
        flow
    }

    /// A look at the calendar `after` milliseconds after the last: whether a job went, which is answered and taken in.
    fn look(&mut self, after: f64) -> bool {
        self.at += after;
        let started = self.app.jmap_pim_tick_at(true, self.at);
        self.app.poll_jmap_pim();
        started
    }

    /// The titles of what the calendar shows in the week of 2026-10-12 (the ticked calendars').
    fn shown(&mut self) -> Vec<String> {
        let listed: Value = self.app.execute("calendar.list", json!({"from": "2026-10-12", "to": "2026-10-18"})).unwrap();
        let mut titles: Vec<String> = listed["events"].as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap().to_string()).collect();
        titles.sort();
        titles
    }

    fn calendar(&self, name: &str) -> Option<&mailcraft_model::Calendar> {
        self.app.session.mailbox.calendars.iter().find(|c| c.origin.account == self.account && c.name == name)
    }
}

#[test]
fn the_window_brings_the_servers_calendars_and_their_events_and_the_calendar_shows_them() {
    let mut flow = Flow::start();
    let (ada, team, grace) = (flow.calendar("Ada").unwrap(), flow.calendar("Team").unwrap(), flow.calendar("Grace").unwrap());
    assert!(ada.origin.id.is_empty() && ada.visible, "the main one, ticked");
    assert!(!team.visible && !grace.visible, "the others wait to be ticked");
    assert!([ada, team, grace].iter().all(|c| c.read_only && c.plain));
    // The main calendar's week shows.
    assert_eq!(flow.shown(), ["Book fair", "Budget review"]);
    let budget = flow.app.session.mailbox.events.values().find(|e| e.title == "Budget review").unwrap();
    assert_eq!((budget.start.hour, budget.start.minute), (9, 0), "nine in Zurich, where the calendar is shown");
    // Ticked, the team's is read at once, and shows too.
    let team = flow.calendar("Team").map(|c| c.id).unwrap();
    flow.app.execute("calendar.showCalendar", json!({"id": team, "value": true})).unwrap();
    assert!(flow.look(SECOND));
    assert_eq!(flow.shown(), ["Book fair", "Budget review", "Team stand-up"]);
}

#[test]
fn an_event_the_server_drops_goes_here_at_the_next_whole_read() {
    let mut flow = Flow::start();
    flow.server.with(|d| d.pim.events.retain(|e| e.raw != "fair"));
    assert!(!flow.look(MINUTE), "not due yet");
    assert!(flow.look(MINUTE + SECOND), "the days on screen, two minutes on");
    assert_eq!(flow.shown(), ["Budget review"]);
}

#[test]
fn a_calendar_the_server_adds_arrives_unticked_and_the_main_one_is_ticked() {
    let mut flow = Flow::start();
    flow.server.with(|d| {
        d.pim.add_calendar(Calendar::new("holidays", "Holidays", false));
    });
    // The list comes with the wide stretch, every ten minutes.
    for _ in 0..4 {
        flow.look(2.0 * MINUTE + SECOND);
    }
    assert!(flow.calendar("Holidays").is_none());
    flow.look(2.0 * MINUTE + SECOND);
    let holidays = flow.calendar("Holidays").unwrap();
    assert!(!holidays.visible && holidays.read_only);
    assert!(flow.calendar("Ada").unwrap().visible);
}

#[test]
fn a_server_that_goes_down_leaves_what_is_here_and_is_asked_again_later() {
    let mut flow = Flow::start();
    // Down: its gateway answers 502 to every request for a while.
    flow.server.with(|d| {
        for _ in 0..3 {
            d.faults.push_back(Fault::on("CalendarEvent/query", mailcraft_jmap::standin::Effect::RefuseWith(502)));
        }
    });
    assert!(flow.look(2.0 * MINUTE + SECOND));
    assert_eq!(flow.shown(), ["Book fair", "Budget review"], "what was here stays");
    let problems = flow.app.sync_problems_of(flow.account);
    assert!(problems.iter().any(|line| line.contains("calendar")), "{problems:?}");
    // Asked again after half a minute, then a minute: still down; then up again.
    assert!(!flow.look(20.0 * SECOND));
    assert!(flow.look(10.0 * SECOND + SECOND));
    assert!(!flow.look(50.0 * SECOND));
    assert!(flow.look(10.0 * SECOND + SECOND));
    flow.server.with(|d| d.pim.events.retain(|e| e.raw != "fair"));
    assert!(flow.look(2.0 * MINUTE + SECOND));
    assert_eq!(flow.shown(), ["Budget review"], "and read again in full once it is back");
}

#[test]
fn a_token_that_dies_is_renewed_and_nothing_is_lost() {
    let mut flow = Flow::start();
    // The server ends the page's session: the token it handed over is no longer good, and the page has another.
    flow.server.with(|d| d.token = Some("rotated".into()));
    flow.server.with(|d| d.pim.add_event(Event::timed("lunch", "ada", "Lunch with Dan", "2026-10-15T12:00:00", Some("Europe/Zurich"), "PT1H")));
    assert!(flow.look(2.0 * MINUTE + SECOND));
    assert_eq!(*flow.page.renewals.borrow(), 1);
    assert_eq!(flow.shown(), ["Book fair", "Budget review", "Lunch with Dan"]);
    assert!(!flow.app.jmap_refused_for(flow.account), "the account is not stopped");
}

#[test]
fn every_calendar_is_read_only_and_a_new_event_is_refused_with_its_reason() {
    let mut flow = Flow::start();
    let refused = flow.app.execute("calendar.new", json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00"}));
    let why = refused.unwrap_err();
    assert!(why.contains("can only be read"), "{why}");
    assert_eq!(flow.shown(), ["Book fair", "Budget review"], "nothing was made");
}
