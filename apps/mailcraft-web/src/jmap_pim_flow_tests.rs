// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a JMAP account's calendars and contacts from the window to the server and back.

//! The whole way of a JMAP account's calendars and contacts: the window's job (`jmap_pim.rs`), the browser runner's core
//! ([`super::pim`], what `jmap_pim_service` runs in the browser) and the in-process server, on the test's own thread:
//! the host answers each job at once, as the browser's task answers it when the window next looks, or holds it until
//! the test lets it go ([`Flow::hold`]), for what happens while a job is on its way. The window is a hosting page's:
//! its credential is the page's, which renews it as the page would, from what the server holds. Nothing here leaves
//! this machine; every name, address and id is invented.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

use mailcraft_engine::Session;
use mailcraft_jmap::standin::{Book, Calendar, Card, Event, Fault, Pim, SESSION, StandIn, TOKEN};
use mailcraft_jmap::{Auth, block_on};
use mailcraft_model::sync::JmapPimResult;
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
    /// Jobs are held rather than answered at once, and the one held.
    holding: Rc<Cell<bool>>,
    held: Rc<RefCell<Option<(JmapPimJob, Sender<JmapPimResult>)>>>,
}

impl Flow {
    fn start() -> Flow {
        let server = StandIn::new();
        server.with(|d| {
            d.mailbox("Inbox", None, Some("inbox"));
            d.pim = Pim::offered();
            d.pim.own = vec![ME.into()];
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
        let (holding, held) = (Rc::new(Cell::new(false)), Rc::new(RefCell::new(None)));
        let (hold, keep) = (holding.clone(), held.clone());
        app.services.jmap_pim = Some(Box::new(move |job: JmapPimJob| {
            let (tx, rx) = mpsc::channel();
            if hold.get() {
                *keep.borrow_mut() = Some((job, tx));
            } else {
                let _ = tx.send(block_on(super::pim(to.clone(), &job, Some(credentials.as_ref()), |_| std::future::ready(()))));
            }
            rx
        }));
        let added = app.execute("account.add", json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
        app.execute("view.calendar", json!({})).unwrap();
        let account = added["id"].as_u64().unwrap();
        let mut flow = Flow { app, server, page, account, at: 0.0, holding, held };
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

    /// Hold the next job instead of answering it: it is on its way until [`Flow::release`].
    fn hold(&self) {
        self.holding.set(true);
    }

    /// Let the job held go to the server now, and have the window take its answer in.
    fn release(&mut self) {
        self.holding.set(false);
        if let Some((job, tx)) = self.held.borrow_mut().take() {
            let _ = tx.send(block_on(super::pim(self.server.clone(), &job, Some(self.page.as_ref()), |_| std::future::ready(()))));
        }
        self.app.poll_jmap_pim();
    }

    /// The event called `title` here.
    fn event(&self, title: &str) -> Option<mailcraft_model::Event> {
        self.app.session.mailbox.events.values().find(|e| e.title == title).map(|e| e.as_ref().clone())
    }

    /// The server's event of the title `title`, as its store has it.
    fn on_server(&self, title: &str) -> Option<Event> {
        self.server.with(|d| d.pim.events.iter().find(|e| e.props.get("title") == Some(&json!(title))).cloned())
    }

    /// When the calendar shows what is called `title` between the 12th and the 30th, by its clock.
    fn when(&mut self, title: &str) -> Vec<String> {
        let listed: Value = self.app.execute("calendar.list", json!({"from": "2026-10-12", "to": "2026-11-03"})).unwrap();
        listed["events"].as_array().unwrap().iter().filter(|e| e["title"] == title).map(|e| e["start"].as_str().unwrap().to_string()).collect()
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
    assert!([ada, team, grace].iter().all(|c| c.plain));
    assert_eq!([ada, team, grace].map(|c| c.read_only), [false, false, true], "Grace's she may only read");
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
fn a_calendar_she_may_only_read_refuses_a_new_event_with_its_reason() {
    let mut flow = Flow::start();
    let grace = flow.calendar("Grace").map(|c| c.id).unwrap();
    let refused =
        flow.app.execute("calendar.new", json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00", "calendar": grace}));
    let why = refused.unwrap_err();
    assert!(why.contains("can only be read"), "{why}");
    assert!(flow.event("Lunch").is_none() && flow.app.session.mailbox.event_changes.is_empty(), "nothing was made");
}

#[test]
fn the_window_brings_the_servers_address_books_and_people_and_the_to_field_have_them() {
    let mut flow = Flow::start();
    flow.server.with(|d| {
        d.pim.add_book(Book { default: true, ..Book::new("neoffice", "Neoffice", true) });
        d.pim.add_book(Book::new("partners", "Partners", false));
        d.pim.add_card(Card::person("Grace Okafor", "neoffice", "Grace", "Okafor", "grace@example.org"));
        d.pim.add_card(Card::company("Lindqvist Joinery", "neoffice", "Lindqvist Joinery", "joinery@example.org"));
        d.pim.add_card(Card::person("Carol Duval", "partners", "Carol", "Duval", "carol@example.org"));
    });
    // People comes to the front: they are asked for at once.
    flow.app.execute("view.people", json!({})).unwrap();
    assert!(flow.look(SECOND));
    let listed = flow.app.execute("people.list", json!({})).unwrap();
    let names: Vec<&str> = listed["contacts"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Carol Duval", "Lindqvist Joinery", "Grace Okafor"], "as People lists them, by last name");
    let books: Vec<(&str, bool)> = flow.app.session.mailbox.address_books.iter().map(|b| (b.name.as_str(), b.read_only)).collect();
    assert_eq!(books, [("Neoffice", true), ("Partners", true)]);
    // The To field completes them.
    let found = mailcraft_engine::cmd::compose::suggest(&flow.app.session, "gra", 6);
    assert_eq!(found.first().map(|s| (s.email.as_str(), s.contact)), Some(("grace@example.org", true)));
    // Edit and Delete are greyed with the reason, and a new contact in a tab that keeps nothing too.
    let grace = flow.app.session.mailbox.contacts.values().find(|c| c.first == "Grace").map(|c| c.id).unwrap();
    flow.app.execute("people.select", json!({"id": grace})).unwrap();
    assert!(!flow.app.enabled("people.edit") && !flow.app.enabled("people.delete"));
    flow.app.session.keeps_local = false;
    assert!(!flow.app.enabled("people.new"));
    let refused = flow.app.execute("people.new", json!({"first": "Gus", "email": "gus@example.org"})).unwrap_err();
    assert!(refused.contains("Your rights do not let you add contacts"), "{refused}");
    // A card deleted on the server goes at the next read, which the changed state makes a whole one.
    flow.server.with(|d| d.pim.cards.retain(|c| c.id != "Carol Duval"));
    assert!(flow.look(10.0 * MINUTE + SECOND));
    assert!(!flow.app.session.mailbox.contacts.values().any(|c| c.first == "Carol"));
    // Nothing changed since: the next costs one question of the contacts, and changes nothing here.
    flow.app.execute("view.mail", json!({})).unwrap();
    flow.look(SECOND);
    flow.app.execute("view.people", json!({})).unwrap();
    let rev = flow.app.session.rev();
    let before = flow.server.calls().len();
    assert!(flow.look(SECOND));
    let asked: Vec<String> = flow.server.calls().into_iter().skip(before).filter(|c| c.starts_with("ContactCard")).collect();
    assert_eq!(asked, ["ContactCard/query"], "the state alone");
    assert_eq!(flow.app.session.rev(), rev);
}

// What is made, moved, changed and deleted here goes to the server, and what it has is what comes back.

#[test]
fn an_appointment_is_made_edited_and_deleted_on_the_server() {
    let mut flow = Flow::start();
    flow.app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00"})).unwrap();
    assert!(flow.look(SECOND), "a change goes at once");
    let made = flow.on_server("Dentist").unwrap();
    assert_eq!((made.start.as_str(), made.zone.as_deref(), made.duration.as_str()), ("2026-10-15T14:00:00", Some("Europe/Zurich"), "PT1H"));
    let here = flow.event("Dentist").unwrap();
    assert_eq!(here.origin.id, Pim::event_id(&made.raw), "the server's name for it, and no second one");
    assert!(flow.app.session.mailbox.event_changes.is_empty() && flow.app.session.mailbox.event_checks.is_empty(), "sent, and seen there");
    // Dragged to four, then made half an hour longer.
    flow.app.execute("calendar.move", json!({"id": here.id, "start": "2026-10-15T16:00"})).unwrap();
    assert!(flow.look(SECOND));
    assert_eq!(flow.on_server("Dentist").map(|e| e.start), Some("2026-10-15T16:00:00".to_string()));
    flow.app.execute("calendar.move", json!({"id": here.id, "start": "2026-10-15T16:00", "end": "2026-10-15T17:30"})).unwrap();
    assert!(flow.look(SECOND));
    assert_eq!(flow.on_server("Dentist").map(|e| e.duration), Some("PT1H30M".to_string()));
    // Edited.
    flow.app.execute("calendar.edit", json!({"id": here.id, "title": "Dentist (Dr Lee)", "location": "Rue du Lac 4"})).unwrap();
    assert!(flow.look(SECOND));
    let edited = flow.on_server("Dentist (Dr Lee)").unwrap();
    assert_eq!(edited.props["locations"]["l1"]["name"], "Rue du Lac 4");
    assert_eq!(flow.when("Dentist (Dr Lee)"), ["2026-10-15T16:00:00"]);
    // Deleted.
    flow.app.execute("calendar.delete", json!({"id": here.id})).unwrap();
    assert!(flow.look(SECOND));
    assert!(flow.on_server("Dentist (Dr Lee)").is_none());
    assert!(flow.event("Dentist (Dr Lee)").is_none());
    assert!(flow.app.sync_problems_of(flow.account).is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn a_whole_day_and_a_series_are_what_the_server_has() {
    let mut flow = Flow::start();
    flow.app.execute("calendar.new", json!({"title": "Holiday", "start": "2026-10-16T00:00", "end": "2026-10-17T00:00", "allDay": true})).unwrap();
    assert!(flow.look(SECOND));
    let holiday = flow.on_server("Holiday").unwrap();
    // Friday and Saturday, the last day included.
    assert_eq!(
        (holiday.all_day, holiday.start.as_str(), holiday.duration.as_str(), holiday.zone.as_deref()),
        (true, "2026-10-16T00:00:00", "P2D", None)
    );
    flow.app
        .execute(
            "calendar.new",
            json!({"title": "Yoga", "start": "2026-10-13T18:00", "end": "2026-10-13T19:00", "recurrence": {"freq": "weekly", "count": 3}}),
        )
        .unwrap();
    assert!(flow.look(SECOND));
    let yoga = flow.on_server("Yoga").unwrap();
    assert_eq!(
        (yoga.start.as_str(), yoga.zone.as_deref(), yoga.rule.as_ref().map(|r| r["count"].clone())),
        ("2026-10-13T18:00:00", Some("Europe/Zurich"), Some(json!(3)))
    );
    // Here it is one series, the server's, on its three Tuesdays.
    let series: Vec<_> = flow.app.session.mailbox.events.values().filter(|e| e.title == "Yoga").collect();
    assert_eq!((series.len(), series[0].origin.id.as_str()), (1, Pim::event_id(&yoga.raw).as_str()));
    assert_eq!(flow.when("Yoga"), ["2026-10-13T18:00:00", "2026-10-20T18:00:00", "2026-10-27T18:00:00"]);
}

#[test]
fn a_server_that_says_yes_and_keeps_the_old_time_is_found_out_and_said() {
    let mut flow = Flow::start();
    flow.server.with(|d| {
        d.pim.keeps_old_times.insert(Pim::event_id("budget"));
    });
    let budget = flow.event("Budget review").unwrap();
    flow.app.execute("calendar.move", json!({"id": budget.id, "start": "2026-10-15T11:00"})).unwrap();
    assert!(flow.look(SECOND));
    // Not seen there yet: looked at again every ten seconds, until three downloads have shown the old time.
    for _ in 0..4 {
        flow.look(11.0 * SECOND);
    }
    assert_eq!(flow.when("Budget review"), ["2026-10-15T09:00:00"], "put back to what the server has");
    let problems = flow.app.sync_problems_of(flow.account);
    assert!(problems.iter().any(|line| line.contains("Budget review")), "{problems:?}");
}

#[test]
fn a_change_made_while_a_read_is_in_flight_is_not_lost() {
    let mut flow = Flow::start();
    // A read goes, and is on its way.
    flow.hold();
    assert!(flow.look(2.0 * MINUTE + SECOND));
    let budget = flow.event("Budget review").unwrap();
    flow.app.execute("calendar.edit", json!({"id": budget.id, "title": "Budget review (2027)"})).unwrap();
    // The read comes back with the old title: the change made meanwhile stays, and goes next.
    flow.release();
    assert!(flow.event("Budget review (2027)").is_some());
    assert!(flow.look(SECOND));
    assert!(flow.on_server("Budget review (2027)").is_some());
    assert!(flow.event("Budget review (2027)").is_some() && flow.app.session.mailbox.event_changes.is_empty());
}

#[test]
fn a_deletion_the_server_answers_not_found_is_settled_by_what_comes_down() {
    let mut flow = Flow::start();
    let fair = flow.event("Book fair").unwrap();
    // Somebody deletes it in the desk; Ada deletes it here before she has seen that.
    flow.server.with(|d| d.pim.events.retain(|e| e.raw != "fair"));
    flow.app.execute("calendar.delete", json!({"id": fair.id})).unwrap();
    assert!(flow.look(SECOND));
    assert!(flow.event("Book fair").is_none() && flow.app.session.mailbox.event_changes.is_empty());
    flow.look(11.0 * SECOND);
    assert!(flow.app.session.mailbox.event_checks.is_empty(), "the download said it is gone");
    assert!(flow.event("Book fair").is_none());
}

#[test]
fn what_is_undone_while_it_is_being_made_is_deleted() {
    let mut flow = Flow::start();
    flow.app.execute("calendar.new", json!({"title": "Coffee", "start": "2026-10-15T10:00", "end": "2026-10-15T10:30"})).unwrap();
    flow.hold();
    assert!(flow.look(SECOND));
    // Taken back while it is on its way.
    let coffee = flow.event("Coffee").unwrap();
    flow.app.execute("calendar.delete", json!({"id": coffee.id})).unwrap();
    flow.release();
    // The server made it: it is deleted there with the next job.
    assert!(flow.look(SECOND));
    assert!(flow.on_server("Coffee").is_none());
    assert!(flow.event("Coffee").is_none());
}

#[test]
fn an_appointment_made_in_another_zone_is_the_same_moment_on_the_server_and_here() {
    let mut flow = Flow::start();
    flow.app
        .execute(
            "calendar.new",
            json!({"title": "Call with New York", "start": "2026-10-15T09:00", "end": "2026-10-15T10:00", "startZone": "America/New_York"}),
        )
        .unwrap();
    assert!(flow.look(SECOND));
    let call = flow.on_server("Call with New York").unwrap();
    assert_eq!((call.start.as_str(), call.zone.as_deref()), ("2026-10-15T09:00:00", Some("America/New_York")));
    // Nine in New York is three in Zurich.
    assert_eq!(flow.when("Call with New York"), ["2026-10-15T15:00:00"]);
    assert_eq!(flow.event("Call with New York").map(|e| e.start_zone), Some("America/New_York".to_string()));
}

#[test]
fn a_series_keeps_its_hour_across_a_change_of_the_clock_here_and_on_the_server() {
    let mut flow = Flow::start();
    flow.app
        .execute(
            "calendar.new",
            json!({"title": "Site meeting", "start": "2026-10-19T09:00", "end": "2026-10-19T10:00", "recurrence": {"freq": "weekly", "count": 3}}),
        )
        .unwrap();
    assert!(flow.look(SECOND));
    let site = flow.on_server("Site meeting").unwrap();
    assert_eq!((site.start.as_str(), site.zone.as_deref()), ("2026-10-19T09:00:00", Some("Europe/Zurich")));
    // Read back: nine either side of the 25th, here as on the server.
    flow.look(2.0 * MINUTE + SECOND);
    assert_eq!(flow.when("Site meeting"), ["2026-10-19T09:00:00", "2026-10-26T09:00:00", "2026-11-02T09:00:00"]);
    assert_eq!(flow.app.session.mailbox.events.values().filter(|e| e.title == "Site meeting").count(), 1);
}
