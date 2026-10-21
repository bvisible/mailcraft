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
    // In French, as the window reads it back: its frame and its reason.
    let line = problems.iter().find(|line| line.starts_with("Couldn't bring the calendar down from the server: ")).expect("the job's line");
    let french = in_french(&flow, line);
    assert!(french.starts_with("Impossible de télécharger le calendrier depuis le serveur : ") && !french.contains("Couldn't"), "{french}");
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
    assert_eq!(books, [("Neoffice", false), ("Partners", true)], "as the account's rights there say");
    // The To field completes them.
    let found = mailcraft_engine::cmd::compose::suggest(&flow.app.session, "gra", 6);
    assert_eq!(found.first().map(|s| (s.email.as_str(), s.contact)), Some(("grace@example.org", true)));
    // Grace's book Ada may change; Carol's she may only read, and Edit and Delete say so.
    let grace = flow.app.session.mailbox.contacts.values().find(|c| c.first == "Grace").map(|c| c.id).unwrap();
    flow.app.execute("people.select", json!({"id": grace})).unwrap();
    assert!(flow.app.enabled("people.edit") && flow.app.enabled("people.delete"));
    let carol = flow.app.session.mailbox.contacts.values().find(|c| c.first == "Carol").map(|c| c.id).unwrap();
    flow.app.execute("people.select", json!({"id": carol})).unwrap();
    assert!(!flow.app.enabled("people.edit") && !flow.app.enabled("people.delete"));
    // A tab that keeps nothing makes a new contact in the book Ada may change, never on this computer.
    flow.app.session.keeps_local = false;
    assert!(flow.app.enabled("people.new"));
    let refused = flow.app.execute("people.new", json!({"first": "Gus", "email": "gus@example.org", "book": 0})).unwrap_err();
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

// What is made, changed and deleted here of the contacts goes to the server, and what it has is what comes back.

impl Flow {
    /// The team's book (Ada may change and delete its cards; the default) with Grace and Bob, and the partners' (read
    /// only) with Carol, in a tab that keeps nothing of its own; People in front, and read.
    fn with_books(&mut self) {
        self.server.with(|d| {
            d.pim.add_book(Book { default: true, ..Book::new("neoffice", "Neoffice", true) });
            d.pim.add_book(Book::new("partners", "Partners", false));
            d.pim.add_card(Card::person("Grace Okafor", "neoffice", "Grace", "Okafor", "grace@example.org"));
            d.pim.add_card(Card::person("Bob Lindqvist", "neoffice", "Bob", "Lindqvist", "bob@example.org").with_phone("+41 79 555 01 02", true));
            d.pim.add_card(Card::person("Carol Duval", "partners", "Carol", "Duval", "carol@example.org"));
        });
        self.app.session.keeps_local = false;
        self.app.execute("view.people", json!({})).unwrap();
        assert!(self.look(SECOND), "People in front: the contacts at once");
    }

    /// The contact here whose first name is `first`.
    fn contact(&self, first: &str) -> Option<mailcraft_model::Contact> {
        self.app.session.mailbox.contacts.values().find(|c| c.first == first).map(|c| c.as_ref().clone())
    }

    /// The server's card called `id`.
    fn card(&self, id: &str) -> Option<Card> {
        self.server.with(|d| d.pim.cards.iter().find(|card| card.id == id).cloned())
    }
}

#[test]
fn a_contact_is_made_edited_and_deleted_on_the_server() {
    let mut flow = Flow::start();
    flow.with_books();
    // Made: in the team's book, at once.
    flow.app.execute("people.new", json!({"first": "Gus", "last": "Hale", "email": "gus@example.org", "phone": "+41 79 555 02 02"})).unwrap();
    assert!(flow.look(SECOND), "a change goes at once");
    let made = flow.card("Gus Hale").expect("on the server");
    assert_eq!((made.books.as_slice(), made.emails.as_slice()), (["neoffice".to_string()].as_slice(), ["gus@example.org".to_string()].as_slice()));
    assert_eq!(made.phones, [("+41 79 555 02 02".to_string(), true)], "its mobile number, as one");
    let here = flow.contact("Gus").unwrap();
    assert_eq!(mailcraft_jmap::pim::server_card_id(&here.origin.id).as_deref(), Some("Gus Hale"), "the server's name for it, and no second one");
    assert_eq!(flow.app.session.mailbox.contacts.values().filter(|c| c.first == "Gus").count(), 1);
    assert!(flow.app.session.mailbox.contact_changes.is_empty());
    // Edited.
    flow.app.execute("people.edit", json!({"id": here.id, "title": "Joiner", "company": "Hale & Sons"})).unwrap();
    assert!(flow.look(SECOND));
    let edited = flow.card("Gus Hale").unwrap();
    assert_eq!((edited.title.as_str(), edited.company.as_str()), ("Joiner", "Hale & Sons"));
    assert_eq!(flow.contact("Gus").map(|c| c.title), Some("Joiner".to_string()), "and what came down after it has it");
    // What the server doesn't keep is refused here, with the reason, and nothing goes.
    let refused = flow.app.execute("people.edit", json!({"id": here.id, "notes": "Met at the fair"})).unwrap_err();
    assert!(refused.contains("keeps only"), "{refused}");
    // Deleted.
    flow.app.execute("people.delete", json!({"id": here.id})).unwrap();
    assert!(flow.look(SECOND));
    assert!(flow.card("Gus Hale").is_none() && flow.contact("Gus").is_none());
    assert!(flow.app.session.mailbox.contact_changes.is_empty());
    let sent: Vec<String> = flow.server.with(|d| d.pim.card_writes.iter().map(|(id, _)| id.clone()).collect());
    assert_eq!(sent, ["Gus Hale"; 3], "each change once");
}

#[test]
fn a_contact_deleted_by_somebody_else_goes_at_the_next_whole_read() {
    let mut flow = Flow::start();
    flow.with_books();
    flow.server.with(|d| d.pim.cards.retain(|card| card.id != "Bob Lindqvist"));
    flow.app.execute("view.mail", json!({})).unwrap();
    flow.look(SECOND);
    flow.app.execute("view.people", json!({})).unwrap();
    assert!(flow.look(SECOND));
    assert!(flow.contact("Bob").is_none());
    assert!(flow.contact("Grace").is_some());
}

#[test]
fn a_contact_in_a_read_only_book_cannot_be_changed_and_the_engine_says_why() {
    let mut flow = Flow::start();
    flow.with_books();
    let carol = flow.contact("Carol").unwrap();
    for (command, params, why) in [
        ("people.edit", json!({"id": carol.id, "title": "Partner"}), "Your rights do not let you change this contact"),
        // (Deleting says it is deleting that is not allowed.)
        ("people.delete", json!({"id": carol.id}), "Your rights do not let you delete this contact"),
    ] {
        let refused = flow.app.execute(command, params).unwrap_err();
        assert!(refused.contains(why), "{command}: {refused}");
    }
    assert!(flow.app.session.mailbox.contact_changes.is_empty() && flow.contact("Carol").is_some_and(|c| c.title.is_empty()));
    // And a book whose rights were narrowed on the server while a read was on its way: the change made here meanwhile
    // is undone, with a word, and never sent.
    let grace = flow.contact("Grace").unwrap();
    flow.app.execute("view.mail", json!({})).unwrap();
    flow.look(SECOND);
    flow.hold();
    flow.app.execute("view.people", json!({})).unwrap();
    assert!(flow.look(SECOND), "People in front again: its books and contacts at once");
    flow.app.execute("people.edit", json!({"id": grace.id, "title": "Engineer"})).unwrap();
    flow.server.with(|d| d.pim.books.iter_mut().for_each(|book| book.may_write = false));
    flow.release();
    flow.look(SECOND);
    assert_eq!(flow.contact("Grace").map(|c| c.title), Some(String::new()));
    let said = flow.app.sync_problems_of(flow.account);
    assert!(
        said.iter().any(|line| line == "\"Grace Okafor\": its address book can't be changed any more, so the change made here was undone."),
        "{said:?}"
    );
    assert_eq!(
        in_french(&flow, "\"Grace Okafor\": its address book can't be changed any more, so the change made here was undone."),
        "\"Grace Okafor\" : son carnet d’adresses ne peut plus être modifié, la modification faite ici a donc été annulée."
    );
    assert!(flow.card("Grace Okafor").is_some_and(|card| card.title.is_empty()), "nothing went");
    assert!(flow.app.session.mailbox.contact_changes.is_empty());
}

#[test]
fn a_refused_deletion_comes_back_with_the_reason() {
    let mut flow = Flow::start();
    flow.with_books();
    flow.server.with(|d| d.pim.linked_cards.insert("Grace Okafor".into(), "Issue ISS-2026-00012".into()));
    let grace = flow.contact("Grace").unwrap();
    flow.app.execute("people.delete", json!({"id": grace.id})).unwrap();
    assert!(flow.contact("Grace").is_none(), "gone here at once");
    assert!(flow.look(SECOND));
    let back = flow.contact("Grace").expect("back, as the server has her");
    assert_eq!(back.id, grace.id);
    let said = flow.app.sync_problems_of(flow.account);
    assert!(
        said.iter().any(|line| line.contains("wouldn't delete \"Grace Okafor\"") && line.contains("is linked with Issue ISS-2026-00012")),
        "{said:?}"
    );
    // In French: the frame and the sentence after it, the server's own words as they came.
    let line = said.iter().find(|line| line.contains("wouldn't delete")).expect("said");
    let french = in_french(&flow, line);
    assert!(
        french.starts_with("Le serveur a refusé de supprimer \"Grace Okafor\" (") && french.contains("is linked with Issue ISS-2026-00012"),
        "{french}"
    );
    if line.ends_with(" It is back in the contacts.") {
        assert!(french.ends_with(" Il est de retour dans les contacts."), "{french}");
    }
    assert!(flow.card("Grace Okafor").is_some() && flow.app.session.mailbox.contact_changes.is_empty());
}

#[test]
fn a_token_that_dies_mid_job_replays_only_what_was_not_answered() {
    let mut flow = Flow::start();
    flow.with_books();
    let (grace, bob) = (flow.contact("Grace").unwrap(), flow.contact("Bob").unwrap());
    flow.app.execute("people.edit", json!({"id": grace.id, "title": "Engineer"})).unwrap();
    flow.app.execute("people.edit", json!({"id": bob.id, "title": "Joiner"})).unwrap();
    // One card a call; the session ends once the first went and before the second could: the page hands a new token.
    flow.server.with(|d| {
        d.max_set = 1;
        d.faults.push_back(Fault::later("ContactCard/set", 1, mailcraft_jmap::standin::Effect::RefuseWith(401)));
    });
    assert!(flow.look(SECOND));
    assert_eq!(*flow.page.renewals.borrow(), 1);
    let sent: Vec<String> = flow.server.with(|d| d.pim.card_writes.iter().map(|(id, _)| id.clone()).collect());
    assert_eq!(sent.len(), 2, "each once: the one answered before the token died is not sent again: {sent:?}");
    assert!(sent.contains(&"Grace Okafor".to_string()) && sent.contains(&"Bob Lindqvist".to_string()));
    assert!(flow.app.session.mailbox.contact_changes.is_empty(), "and both answers were kept");
    assert_eq!((flow.contact("Grace").unwrap().title, flow.contact("Bob").unwrap().title), ("Engineer".to_string(), "Joiner".to_string()));
    assert!(!flow.app.jmap_refused_for(flow.account));
}

#[test]
fn a_lost_answer_to_a_create_heals_at_the_following_read() {
    let mut flow = Flow::start();
    flow.with_books();
    flow.server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", mailcraft_jmap::standin::Effect::LoseAnswer)));
    flow.app.execute("people.new", json!({"first": "Gus", "last": "Hale", "email": "gus@example.org"})).unwrap();
    let gus = flow.contact("Gus").map(|c| c.id).unwrap();
    assert!(flow.look(SECOND));
    // The server made it; the answer was lost: it waits to go again, and the read brought the card meanwhile, as a
    // contact of its own (the server keeps no mark of a card: nothing tells them apart yet).
    assert!(flow.card("Gus Hale").is_some());
    assert_eq!(flow.app.session.mailbox.contact_changes.len(), 1);
    let copy = flow.app.session.mailbox.contacts.values().find(|c| c.first == "Gus" && c.id != gus).map(|c| c.id).expect("the copy");
    // A number added to the copy, and a job title to Gus himself, before the next try.
    flow.app.execute("people.edit", json!({"id": copy, "phones": [{"label": "mobile", "value": "+41 79 555 02 02"}]})).unwrap();
    flow.app.execute("people.edit", json!({"id": gus, "title": "Joiner"})).unwrap();
    // A minute on (as the server asked), it goes again with its mark: the same card. The copy's change waits for its
    // answer (it is made to the card as it is once the server said which card it made), and goes at the next look:
    // the card has both, and one Gus is left here.
    flow.app.session.clock.now = flow.app.session.clock.now.map(|now| now + 120);
    assert!(flow.look(MINUTE));
    let waiting: Vec<(mailcraft_model::book::ContactChangeKind, u64)> =
        flow.app.session.mailbox.contact_changes.iter().map(|change| (change.kind, change.contact)).collect();
    assert_eq!(waiting, [(mailcraft_model::book::ContactChangeKind::Update, copy)], "the copy's change, still to go");
    assert!(flow.look(SECOND));
    let card = flow.card("Gus Hale").unwrap();
    assert_eq!((card.title.as_str(), card.phones.len()), ("Joiner", 1), "nothing made here was lost: {card:?}");
    assert_eq!(flow.server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
    assert!(flow.app.session.mailbox.contact_changes.is_empty(), "{:?}", flow.app.session.mailbox.contact_changes);
    let here: Vec<mailcraft_model::Contact> =
        flow.app.session.mailbox.contacts.values().filter(|c| c.first == "Gus").map(|c| c.as_ref().clone()).collect();
    assert_eq!(here.len(), 1);
    assert_eq!((here[0].id, here[0].title.as_str(), here[0].phones.len()), (gus, "Joiner", 1));
}

#[test]
fn what_the_desk_added_while_a_change_waited_stays() {
    let mut flow = Flow::start();
    flow.with_books();
    let bob = flow.contact("Bob").unwrap();
    // An address and a number added here; the server is slow to hear it (the job is held), and meanwhile the desk adds
    // an address of its own.
    flow.hold();
    let edit = json!({"id": bob.id,
        "emails": [{"label": "", "value": "bob@example.org"}, {"label": "work", "value": "bob@joinery.example.org"}],
        "phones": [{"label": "mobile", "value": "+41 79 555 01 02"}, {"label": "", "value": "+41 44 555 01 03"}]});
    flow.app.execute("people.edit", edit).unwrap();
    assert!(flow.look(SECOND));
    flow.server.with(|d| {
        if let Some(card) = d.pim.cards.iter_mut().find(|card| card.id == "Bob Lindqvist") {
            card.emails.push("robert@example.org".into());
        }
    });
    flow.release();
    let card = flow.card("Bob Lindqvist").unwrap();
    assert_eq!(card.emails, ["bob@example.org", "robert@example.org", "bob@joinery.example.org"], "the desk's address stays");
    assert_eq!(card.phones.len(), 2, "and the number went");
    // What comes down after it has all of it.
    assert!(flow.contact("Bob").is_some_and(|b| b.emails.len() == 3 && b.phones.len() == 2));
}

#[test]
fn a_change_of_a_contact_whose_name_another_has_now_is_not_written() {
    let mut flow = Flow::start();
    flow.with_books();
    let grace = flow.contact("Grace").unwrap();
    // Grace is deleted in the desk and a new Grace Okafor takes her name, while a change of hers waits.
    flow.hold();
    flow.app.execute("people.edit", json!({"id": grace.id, "title": "Engineer"})).unwrap();
    assert!(flow.look(SECOND));
    flow.server.with(|d| {
        d.pim.cards.retain(|card| card.id != "Grace Okafor");
        d.pim.add_card(Card::person("Grace Okafor", "neoffice", "Grace", "Okafor", "g.okafor@example.org"));
    });
    flow.release();
    assert!(flow.card("Grace Okafor").is_some_and(|card| card.title.is_empty() && card.emails == ["g.okafor@example.org"]), "nothing written to her");
    assert!(flow.server.with(|d| d.pim.card_writes.is_empty()));
    assert!(flow.app.session.mailbox.contact_changes.is_empty());
    let said = flow.app.sync_problems_of(flow.account);
    assert!(said.iter().any(|line| line.contains("is no longer on the server")), "{said:?}");
    // The read after it brings the newcomer, as the server has her.
    assert!(flow.contact("Grace").is_some_and(|c| c.emails.first().map(|e| e.value.as_str()) == Some("g.okafor@example.org")));
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

// One day of a series, the series, and answers.

impl Flow {
    /// Ada's weekly site meeting on the server (Mondays at nine in Zurich, four of them from 2026-10-12), and Grace's
    /// planning in Ada's calendar (Tuesdays at ten, three of them from 2026-10-13, Ada invited), read here.
    fn with_series(&mut self) {
        self.server.with(|d| {
            let zurich = Some("Europe/Zurich");
            d.pim.add_event(
                Event::timed("site", "ada", "Site meeting", "2026-10-12T09:00:00", zurich, "PT1H")
                    .repeating(json!({"@type": "RecurrenceRule", "frequency": "weekly", "count": 4})),
            );
            d.pim.add_event(
                Event::timed("planning", "ada", "Planning", "2026-10-13T10:00:00", zurich, "PT1H")
                    .repeating(json!({"@type": "RecurrenceRule", "frequency": "weekly", "count": 3}))
                    .with("organizerCalendarAddress", json!("mailto:grace@example.org"))
                    .with(
                        "participants",
                        json!({
                            "grace": {"@type": "Participant", "name": "Grace", "calendarAddress": "mailto:grace@example.org", "roles": {"owner": true}, "participationStatus": "accepted"},
                            "ada": {"@type": "Participant", "name": "Ada", "calendarAddress": format!("mailto:{ME}"), "roles": {"attendee": true}, "participationStatus": "needs-action"},
                        }),
                    ),
            );
        });
        // The wide stretch, with the list: the series come whole.
        self.look(10.0 * MINUTE + SECOND);
        assert!(self.event("Site meeting").is_some_and(|e| e.recurrence.is_some()) && self.event("Planning").is_some_and(|e| e.recurrence.is_some()));
    }

    /// The server's site meeting, as its store has it.
    fn site(&self) -> Event {
        self.server.with(|d| d.pim.events.iter().find(|e| e.raw == "site").cloned()).unwrap()
    }
}

#[test]
fn one_day_of_a_series_is_moved_and_the_others_stay() {
    let mut flow = Flow::start();
    flow.with_series();
    let site = flow.event("Site meeting").unwrap();
    // (`start` is where the series would start: two hours later, and so is that Monday.)
    flow.app
        .execute("calendar.move", json!({"id": site.id, "scope": "occurrence", "occurrence": "2026-10-19", "start": "2026-10-12T11:00"}))
        .unwrap();
    assert!(flow.look(SECOND));
    assert_eq!(flow.site().overrides["2026-10-19T09:00:00"]["start"], "2026-10-19T11:00:00");
    assert!(flow.app.session.mailbox.event_changes.is_empty());
    assert_eq!(flow.when("Site meeting"), ["2026-10-12T09:00:00", "2026-10-19T11:00:00", "2026-10-26T09:00:00", "2026-11-02T09:00:00"]);
    // Still one series, and the moved day an event of its own that stands for it.
    flow.look(2.0 * MINUTE + SECOND);
    assert_eq!(flow.when("Site meeting").len(), 4);
    assert_eq!(flow.app.session.mailbox.events.values().filter(|e| e.title == "Site meeting" && e.recurrence.is_some()).count(), 1);
}

#[test]
fn one_day_is_deleted_and_the_series_goes_on() {
    let mut flow = Flow::start();
    flow.with_series();
    let site = flow.event("Site meeting").unwrap();
    flow.app.execute("calendar.delete", json!({"id": site.id, "scope": "occurrence", "occurrence": "2026-10-26"})).unwrap();
    assert!(flow.look(SECOND));
    assert_eq!(flow.site().excluded, ["2026-10-26T09:00:00"]);
    flow.look(2.0 * MINUTE + SECOND);
    assert_eq!(flow.when("Site meeting"), ["2026-10-12T09:00:00", "2026-10-19T09:00:00", "2026-11-02T09:00:00"]);
}

#[test]
fn the_whole_series_is_moved_by_days_and_keeps_its_clock() {
    let mut flow = Flow::start();
    flow.with_series();
    let site = flow.event("Site meeting").unwrap();
    // To Tuesdays, from the first one: nine o'clock either side of the 25th still.
    flow.app.execute("calendar.move", json!({"id": site.id, "scope": "series", "start": "2026-10-13T09:00"})).unwrap();
    assert!(flow.look(SECOND));
    let moved = flow.site();
    assert_eq!((moved.start.as_str(), moved.zone.as_deref()), ("2026-10-13T09:00:00", Some("Europe/Zurich")));
    flow.look(2.0 * MINUTE + SECOND);
    assert_eq!(flow.when("Site meeting"), ["2026-10-13T09:00:00", "2026-10-20T09:00:00", "2026-10-27T09:00:00", "2026-11-03T09:00:00"]);
}

#[test]
fn an_invitation_about_one_occurrence_is_answered_for_that_occurrence() {
    let mut flow = Flow::start();
    flow.with_series();
    // Grace's invitation for the 27th, in the mail.
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:planning@standin.example\r\nSUMMARY:Planning\r\n\
               RECURRENCE-ID:20261027T090000Z\r\nDTSTART:20261027T090000Z\r\nDTEND:20261027T100000Z\r\n\
               ORGANIZER;CN=Grace:mailto:grace@example.org\r\nATTENDEE;PARTSTAT=NEEDS-ACTION;CN=Ada:mailto:ada@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let inbox = flow.app.session.mailbox.folders.iter().find(|f| f.kind == mailcraft_model::FolderKind::Inbox).map(|f| f.id).unwrap();
    let message = mailcraft_model::Message {
        folder: inbox,
        message_id: "planning-27@example.org".into(),
        from: mailcraft_model::Address::new("Grace", "grace@example.org"),
        to: vec![mailcraft_model::Address::new("Ada", ME)],
        subject: "Planning".into(),
        attachments: vec![mailcraft_model::Attachment::new("invite.ics", "text/calendar", ics.as_bytes().to_vec())],
        ..Default::default()
    };
    let id = flow.app.session.mailbox.add_message(message).unwrap();
    let said = flow.app.execute("calendar.invitation", json!({"message": id})).unwrap();
    assert_eq!(said["occurrence"], "2026-10-27", "{said}");
    // Declined, kept on her copy (not sent): that Tuesday alone.
    flow.app.execute("calendar.respond", json!({"message": id, "response": "decline"})).unwrap();
    assert!(flow.look(SECOND));
    let planning = flow.server.with(|d| d.pim.events.iter().find(|e| e.raw == "planning").cloned()).unwrap();
    assert_eq!(planning.overrides["2026-10-27T10:00:00"]["participants"]["ada"]["participationStatus"], "declined");
    assert_eq!(planning.props["participants"]["ada"]["participationStatus"], "needs-action", "the others as they were");
    assert!(flow.app.session.mailbox.event_changes.is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn an_answer_to_an_invitation_reaches_the_organizer_when_the_server_does_it() {
    let mut flow = Flow::start();
    flow.with_series();
    let planning = flow.event("Planning").unwrap();
    flow.app.execute("calendar.respond", json!({"id": planning.id, "response": "accept", "send": true})).unwrap();
    assert!(flow.look(SECOND));
    let told = flow.server.with(|d| d.pim.scheduled.clone());
    assert_eq!(told.iter().map(|t| (t.what, t.to.clone())).collect::<Vec<_>>(), [("reply", vec!["grace@example.org".to_string()])]);
    // A server that keeps the answer on her copy only: kept, and nobody told (as before such a server did).
    flow.server.with(|d| {
        d.pim.tells_the_organizer = false;
        d.pim.scheduled.clear();
    });
    flow.app.execute("calendar.respond", json!({"id": planning.id, "response": "tentative", "send": true})).unwrap();
    assert!(flow.look(SECOND));
    assert!(flow.server.with(|d| d.pim.scheduled.is_empty()));
}

#[test]
fn an_answer_in_a_read_only_calendar_is_left_as_the_server_has_it() {
    let mut flow = Flow::start();
    flow.server.with(|d| {
        d.pim.add_event(
            Event::timed("review", "grace", "Design review", "2026-10-16T15:00:00", Some("Europe/Zurich"), "PT1H")
                .with("organizerCalendarAddress", json!("mailto:grace@example.org"))
                .with(
                    "participants",
                    json!({"ada": {"calendarAddress": format!("mailto:{ME}"), "roles": {"attendee": true}, "participationStatus": "needs-action"}}),
                ),
        );
    });
    let grace = flow.calendar("Grace").map(|c| c.id).unwrap();
    flow.app.execute("calendar.showCalendar", json!({"id": grace, "value": true})).unwrap();
    assert!(flow.look(SECOND));
    let review = flow.event("Design review").unwrap();
    let refused = flow.app.execute("calendar.respond", json!({"id": review.id, "response": "accept", "send": true})).unwrap_err();
    assert!(refused.contains("can only be read"), "{refused}");
    assert!(flow.app.session.mailbox.event_changes.is_empty());
}

#[test]
fn a_meeting_the_person_organizes_tells_the_guests_on_a_change_and_not_on_a_silent_one() {
    let mut flow = Flow::start();
    flow.server.with(|d| {
        d.pim.add_event(
            Event::timed("review", "ada", "Review", "2026-10-16T15:00:00", Some("Europe/Zurich"), "PT1H")
                .with("organizerCalendarAddress", json!(format!("mailto:{ME}")))
                .with(
                    "participants",
                    json!({
                        "ada": {"calendarAddress": format!("mailto:{ME}"), "roles": {"owner": true, "attendee": true}, "participationStatus": "accepted"},
                        "bob": {"calendarAddress": "mailto:bob@example.org", "roles": {"attendee": true}, "participationStatus": "accepted"},
                    }),
                ),
        );
    });
    assert!(flow.look(2.0 * MINUTE + SECOND));
    let review = flow.event("Review").unwrap();
    // A reminder of her own: nobody told.
    flow.app.execute("calendar.reminder", json!({"id": review.id, "minutes": 30})).unwrap();
    assert!(flow.look(SECOND));
    assert!(flow.server.with(|d| d.pim.scheduled.is_empty()));
    // Moved, which Bob sees: he is sent the update.
    flow.app.execute("calendar.move", json!({"id": review.id, "start": "2026-10-16T16:00", "send": true})).unwrap();
    assert!(flow.look(SECOND));
    let told = flow.server.with(|d| d.pim.scheduled.clone());
    assert_eq!(told.iter().map(|t| (t.what, t.to.clone())).collect::<Vec<_>>(), [("update", vec!["bob@example.org".to_string()])]);
}

#[test]
fn a_note_with_an_answer_is_not_sent_and_the_answer_still_is() {
    let mut flow = Flow::start();
    flow.with_series();
    let planning = flow.event("Planning").unwrap();
    flow.app.execute("calendar.respond", json!({"id": planning.id, "response": "accept", "send": true, "comment": "See you there"})).unwrap();
    assert!(flow.look(SECOND));
    // Such a server takes the answer and no note with it: nothing of the note was written.
    let written = flow.server.with(|d| d.pim.written.clone());
    assert!(!written.iter().any(|(_, patch)| patch.to_string().contains("See you there")));
    assert_eq!(flow.server.with(|d| d.pim.scheduled.len()), 1);
}

#[test]
fn a_meeting_declined_here_stays_out_of_the_calendar_when_the_server_keeps_it_with_the_answer() {
    let mut flow = Flow::start();
    flow.server.with(|d| {
        d.pim.add_event(
            Event::timed("review", "ada", "Design review", "2026-10-16T15:00:00", Some("Europe/Zurich"), "PT1H")
                .with("organizerCalendarAddress", json!("mailto:grace@example.org"))
                .with(
                    "participants",
                    json!({
                        "grace": {"calendarAddress": "mailto:grace@example.org", "roles": {"owner": true}, "participationStatus": "accepted"},
                        "ada": {"calendarAddress": format!("mailto:{ME}"), "roles": {"attendee": true}, "participationStatus": "needs-action"},
                    }),
                ),
        );
    });
    assert!(flow.look(2.0 * MINUTE + SECOND));
    let review = flow.event("Design review").unwrap();
    flow.app.execute("calendar.respond", json!({"id": review.id, "response": "decline", "send": true})).unwrap();
    assert!(flow.event("Design review").is_none(), "out of the calendar at once");
    assert!(flow.look(SECOND));
    // The server keeps the meeting, with her answer, and Grace was told.
    let kept = flow.on_server("Design review").unwrap();
    assert_eq!(kept.props["participants"]["ada"]["participationStatus"], "declined");
    let told = flow.server.with(|d| d.pim.scheduled.clone());
    assert_eq!(told.iter().map(|t| (t.what, t.to.clone())).collect::<Vec<_>>(), [("reply", vec!["grace@example.org".to_string()])]);
    // Read again, and again: it stays out, and nothing waits.
    for _ in 0..3 {
        flow.look(2.0 * MINUTE + SECOND);
        assert!(flow.event("Design review").is_none());
    }
    assert!(flow.app.session.mailbox.event_changes.is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn a_series_declined_elsewhere_leaves_the_window_whole_with_a_day_accepted_alone() {
    let mut flow = Flow::start();
    flow.with_series();
    assert_eq!(flow.when("Planning").len(), 3);
    // On her phone: Grace's planning declined, then the 20th accepted alone.
    flow.server.with(|d| {
        let planning = d.pim.events.iter_mut().find(|e| e.raw == "planning").unwrap();
        planning.props["participants"]["ada"]["participationStatus"] = json!("declined");
        let mut accepted = planning.props["participants"].clone();
        accepted["ada"]["participationStatus"] = json!("accepted");
        planning.overrides.insert("2026-10-20T10:00:00".into(), json!({"participants": accepted}).as_object().unwrap().clone());
    });
    flow.look(2.0 * MINUTE + SECOND);
    assert!(flow.when("Planning").is_empty(), "{:?}", flow.when("Planning"));
    assert!(flow.event("Planning").is_none(), "the series is gone whole");
}

#[test]
fn a_day_declined_alone_elsewhere_leaves_the_window_and_the_series_stays() {
    let mut flow = Flow::start();
    flow.with_series();
    // On her phone: Grace's planning accepted, and the 20th declined alone.
    flow.server.with(|d| {
        let planning = d.pim.events.iter_mut().find(|e| e.raw == "planning").unwrap();
        planning.props["participants"]["ada"]["participationStatus"] = json!("accepted");
        let mut declined = planning.props["participants"].clone();
        declined["ada"]["participationStatus"] = json!("declined");
        planning.overrides.insert("2026-10-20T10:00:00".into(), json!({"participants": declined}).as_object().unwrap().clone());
    });
    flow.look(2.0 * MINUTE + SECOND);
    assert_eq!(flow.when("Planning"), ["2026-10-13T10:00:00", "2026-10-27T10:00:00"]);
    assert!(flow.event("Planning").is_some_and(|e| e.recurrence.is_some()));
}

#[test]
fn a_series_declined_elsewhere_stays_while_a_change_of_one_of_its_days_waits_and_goes_after_it() {
    let mut flow = Flow::start();
    flow.with_series();
    let planning = flow.event("Planning").unwrap();
    // A reminder of her own on the 20th, here; the server can't take it just now.
    let twentieth = Pim::occurrence_id("planning", "2026-10-20T10:00:00");
    flow.server.with(|d| {
        d.pim.refused_events.insert(twentieth.clone(), "serverFail");
    });
    flow.app.execute("calendar.edit", json!({"id": planning.id, "scope": "occurrence", "occurrence": "2026-10-20", "reminder": 30})).unwrap();
    // Meanwhile, on her phone, Grace's planning is declined.
    flow.server.with(|d| {
        let planning = d.pim.events.iter_mut().find(|e| e.raw == "planning").unwrap();
        planning.props["participants"]["ada"]["participationStatus"] = json!("declined");
    });
    assert!(flow.look(SECOND));
    // The change waits, and so does the series: neither is thrown away under it.
    assert_eq!(flow.app.session.mailbox.event_changes.len(), 1, "{:?}", flow.app.sync_problems_of(flow.account));
    assert!(flow.event("Planning").is_some_and(|e| e.recurrence.is_some()), "the series stays while its day's change waits");
    // The server takes it the next time: then the series goes, whole.
    flow.server.with(|d| d.pim.refused_events.clear());
    flow.app.session.clock.now = Some(NOW + 10 * 60);
    flow.look(2.0 * MINUTE + SECOND);
    assert!(
        flow.server.with(|d| d.pim.events.iter().find(|e| e.raw == "planning").unwrap().overrides.contains_key("2026-10-20T10:00:00")),
        "the change went"
    );
    assert!(flow.app.session.mailbox.event_changes.is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
    flow.look(2.0 * MINUTE + SECOND);
    assert!(flow.event("Planning").is_none() && flow.when("Planning").is_empty(), "{:?}", flow.when("Planning"));
}

#[test]
fn a_series_declined_elsewhere_stays_while_a_change_of_one_of_its_events_of_their_own_waits() {
    let mut flow = Flow::start();
    flow.with_series();
    let planning = flow.event("Planning").unwrap();
    // A reminder of her own on the 20th, sent: the day is an event of its own on the server, under a name of its own.
    flow.app.execute("calendar.edit", json!({"id": planning.id, "scope": "occurrence", "occurrence": "2026-10-20", "reminder": 30})).unwrap();
    assert!(flow.look(SECOND));
    let twentieth = Pim::occurrence_id("planning", "2026-10-20T10:00:00");
    let day = flow.app.session.mailbox.events.values().find(|e| e.origin.id == twentieth).map(|e| e.id).unwrap();
    assert!(flow.app.session.mailbox.event_changes.is_empty());
    // Changed again, by that name; the server can't take it just now; and on her phone the series is declined.
    flow.server.with(|d| {
        d.pim.refused_events.insert(twentieth.clone(), "serverFail");
    });
    flow.app.execute("calendar.edit", json!({"id": day, "reminder": 45})).unwrap();
    flow.server.with(|d| {
        let planning = d.pim.events.iter_mut().find(|e| e.raw == "planning").unwrap();
        planning.props["participants"]["ada"]["participationStatus"] = json!("declined");
    });
    assert!(flow.look(SECOND));
    assert_eq!(flow.app.session.mailbox.event_changes.len(), 1, "{:?}", flow.app.sync_problems_of(flow.account));
    assert!(flow.event("Planning").is_some_and(|e| e.recurrence.is_some()), "the series stays while the change waits");
    // Once it went, the series goes, whole, without a word.
    flow.server.with(|d| d.pim.refused_events.clear());
    flow.app.session.clock.now = Some(NOW + 10 * 60);
    flow.look(2.0 * MINUTE + SECOND);
    assert!(flow.app.session.mailbox.event_changes.is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
    flow.look(2.0 * MINUTE + SECOND);
    assert!(flow.when("Planning").is_empty(), "{:?}", flow.when("Planning"));
    let problems = flow.app.sync_problems_of(flow.account);
    assert!(!problems.iter().any(|line| line.contains("took the change") || line.contains("taken out")), "{problems:?}");
}

#[test]
fn a_series_declined_elsewhere_goes_at_once_after_a_day_of_it_was_moved_and_sent_and_nothing_is_said() {
    let mut flow = Flow::start();
    flow.with_series();
    let planning = flow.event("Planning").unwrap();
    // The 20th an hour later, here; then, before the next read, the series declined on her phone.
    flow.app
        .execute("calendar.move", json!({"id": planning.id, "scope": "occurrence", "occurrence": "2026-10-20", "start": "2026-10-13T11:00"}))
        .unwrap();
    flow.server.with(|d| {
        let planning = d.pim.events.iter_mut().find(|e| e.raw == "planning").unwrap();
        planning.props["participants"]["ada"]["participationStatus"] = json!("declined");
    });
    // The move goes, and the read after it says the series is gone: it goes at once, the move's check with it.
    assert!(flow.look(SECOND));
    assert!(
        flow.server.with(|d| d.pim.events.iter().find(|e| e.raw == "planning").unwrap().overrides.contains_key("2026-10-20T10:00:00")),
        "the move went"
    );
    assert!(flow.app.session.mailbox.event_changes.is_empty());
    assert!(flow.when("Planning").is_empty(), "{:?}", flow.when("Planning"));
    for _ in 0..4 {
        flow.look(2.0 * MINUTE + SECOND);
    }
    assert!(flow.when("Planning").is_empty());
    assert!(flow.app.sync_problems_of(flow.account).is_empty(), "nothing to say: {:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn an_appointment_the_server_organizes_under_her_identity_is_hers_and_is_simply_deleted() {
    let mut flow = Flow::start();
    // The server organizes what she makes under its default identity of hers, an address no mailbox of the page has.
    flow.server.with(|d| d.pim.own = vec!["hello@example.org".into(), ME.into()]);
    flow.app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00"})).unwrap();
    assert!(flow.look(SECOND));
    let made = flow.on_server("Dentist").unwrap();
    assert_eq!(made.props["organizerCalendarAddress"], "mailto:hello@example.org");
    assert_eq!(flow.app.pim_identities(flow.account), ["hello@example.org".to_string(), ME.to_string()]);
    flow.look(2.0 * MINUTE + SECOND);
    let dentist = flow.event("Dentist").unwrap();
    assert!(!dentist.origin.guest, "hers, not somebody else's meeting");
    // Deleting it asks nothing of a copy of somebody's meeting, and is a plain delete.
    let asked = mailcraft_ui_egui::dialogs::confirmation(&flow.app, "calendar.delete", &json!({"id": dentist.id}));
    assert!(asked.as_ref().is_none_or(|question| !question.text.contains("invited")), "{:?}", asked.map(|question| question.text));
    flow.app.execute("calendar.delete", json!({"id": dentist.id})).unwrap();
    assert!(flow.look(SECOND));
    assert!(flow.on_server("Dentist").is_none() && flow.event("Dentist").is_none());
    assert!(flow.app.sync_problems_of(flow.account).is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn a_server_that_knows_no_participant_identities_is_read_and_written_as_before() {
    let mut flow = Flow::start();
    flow.server.with(|d| d.pim.says_identities = false);
    flow.app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00"})).unwrap();
    assert!(flow.look(SECOND));
    flow.look(2.0 * MINUTE + SECOND);
    assert!(flow.event("Dentist").is_some_and(|e| !e.origin.guest));
    assert!(flow.app.sync_problems_of(flow.account).is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn what_is_taken_out_of_a_new_contact_before_it_goes_again_is_taken_out_there() {
    let mut flow = Flow::start();
    flow.with_books();
    flow.server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", mailcraft_jmap::standin::Effect::LoseAnswer)));
    let made = json!({"first": "Gus", "last": "Hale", "email": "gus@example.org", "phone": "+41 79 555 02 02", "company": "Hale & Sons"});
    flow.app.execute("people.new", made).unwrap();
    let gus = flow.contact("Gus").map(|c| c.id).unwrap();
    assert!(flow.look(SECOND));
    assert!(flow.card("Gus Hale").is_some_and(|card| card.phones.len() == 1 && card.company == "Hale & Sons"), "the server made him");
    // His number and his company taken out here before the next try: they go from the card made the first time.
    flow.app.execute("people.edit", json!({"id": gus, "phones": [], "company": ""})).unwrap();
    flow.app.session.clock.now = flow.app.session.clock.now.map(|now| now + 120);
    assert!(flow.look(MINUTE));
    let card = flow.card("Gus Hale").unwrap();
    assert!(card.phones.is_empty() && card.company.is_empty(), "{card:?}");
    assert!(flow.app.session.mailbox.contact_changes.is_empty());
    let here: Vec<mailcraft_model::Contact> =
        flow.app.session.mailbox.contacts.values().filter(|c| c.first == "Gus").map(|c| c.as_ref().clone()).collect();
    assert_eq!(here.len(), 1, "one Gus");
    assert!(here[0].phones.is_empty() && here[0].company.is_empty(), "and the read after it doesn't bring them back");
}

#[test]
fn a_copy_deleted_while_a_new_contact_waited_to_go_again_deletes_nothing_there() {
    let mut flow = Flow::start();
    flow.with_books();
    flow.server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", mailcraft_jmap::standin::Effect::LoseAnswer)));
    flow.app.execute("people.new", json!({"first": "Gus", "last": "Hale", "email": "gus@example.org"})).unwrap();
    let gus = flow.contact("Gus").map(|c| c.id).unwrap();
    assert!(flow.look(SECOND));
    let made = flow.card("Gus Hale").expect("the server made him");
    // The read brought his card as a contact of its own, which is deleted here before Gus went again.
    let copy = flow.app.session.mailbox.contacts.values().find(|c| c.first == "Gus" && c.id != gus).map(|c| c.id).expect("the copy");
    flow.app.execute("people.delete", json!({"id": copy})).unwrap();
    // Gus is not due again yet (a minute on the page's clock, not on the engine's): the deletion waits for him, for it
    // may be of his very card.
    flow.look(MINUTE);
    assert!(flow.card("Gus Hale").is_some());
    // When he is due, both go, he first: the card he is answered with is the copy's, whose deletion is not sent.
    flow.app.session.clock.now = flow.app.session.clock.now.map(|now| now + 120);
    assert!(flow.look(MINUTE));
    assert!(flow.server.with(|d| d.pim.card_writes.iter().all(|(_, sent)| sent != "destroy")), "his card was never deleted");
    assert_eq!(flow.card("Gus Hale").map(|card| card.created), Some(made.created), "the card made the first time");
    assert_eq!(flow.contact("Gus").map(|c| (c.id, mailcraft_jmap::pim::server_card_id(&c.origin.id))), Some((gus, Some("Gus Hale".to_string()))));
    assert!(flow.app.session.mailbox.contact_changes.is_empty(), "{:?}", flow.app.session.mailbox.contact_changes);
}

#[test]
fn a_new_contact_sent_again_is_found_on_the_server_and_what_the_desk_added_there_stays() {
    let mut flow = Flow::start();
    flow.with_books();
    // A server that finds a card by the uid it was made with (as Neoffice's does).
    flow.server.with(|d| {
        d.pim.finds_by_uid = true;
        d.faults.push_back(Fault::on("ContactCard/set", mailcraft_jmap::standin::Effect::LoseAnswer));
    });
    flow.app.execute("people.new", json!({"first": "Gus", "last": "Hale", "email": "gus@example.org"})).unwrap();
    let gus = flow.contact("Gus").map(|c| c.id).unwrap();
    assert!(flow.look(SECOND));
    // The desk gives his card a number; here, he gets a job title.
    flow.server.with(|d| {
        if let Some(card) = d.pim.cards.iter_mut().find(|card| card.id == "Gus Hale") {
            card.phones.push(("+41 44 555 02 09".into(), false));
        }
    });
    flow.app.execute("people.edit", json!({"id": gus, "title": "Joiner"})).unwrap();
    flow.app.session.clock.now = flow.app.session.clock.now.map(|now| now + 120);
    assert!(flow.look(MINUTE));
    let card = flow.card("Gus Hale").unwrap();
    assert_eq!((card.phones.len(), card.title.as_str()), (1, "Joiner"), "the desk's number stays: {card:?}");
    let (_, last) = flow.server.with(|d| d.pim.card_writes.last().cloned()).unwrap();
    assert_eq!(last.as_object().map(|map| map.keys().cloned().collect::<Vec<String>>()), Some(vec!["titles".to_string()]), "only what changed here");
    assert_eq!(flow.server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
    assert!(flow.app.session.mailbox.contact_changes.is_empty());
    let here: Vec<mailcraft_model::Contact> =
        flow.app.session.mailbox.contacts.values().filter(|c| c.first == "Gus").map(|c| c.as_ref().clone()).collect();
    assert_eq!(here.len(), 1, "one Gus, the one made here");
    assert_eq!((here[0].id, here[0].phones.len(), here[0].title.as_str()), (gus, 1, "Joiner"));
}

#[test]
fn a_new_contact_whose_answer_says_the_server_ran_part_of_it_is_looked_for_before_it_goes_again() {
    let mut flow = Flow::start();
    flow.with_books();
    flow.server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", mailcraft_jmap::standin::Effect::MethodError("serverPartialFail"))));
    flow.app.execute("people.new", json!({"first": "Gus", "last": "Hale", "email": "gus@example.org"})).unwrap();
    assert!(flow.look(SECOND));
    assert_eq!(flow.app.session.mailbox.contact_changes.len(), 1);
    // The next try looks for the card first (the server may have made it), then sends it.
    let before = flow.server.calls().len();
    flow.app.session.clock.now = flow.app.session.clock.now.map(|now| now + 120);
    assert!(flow.look(MINUTE));
    let asked: Vec<String> = flow.server.calls().into_iter().skip(before).filter(|c| c.starts_with("ContactCard/")).collect();
    assert_eq!(asked.first().map(String::as_str), Some("ContactCard/query"), "{asked:?}");
    assert!(asked.iter().any(|call| call == "ContactCard/set"), "{asked:?}");
    assert_eq!(flow.server.with(|d| d.pim.cards.iter().filter(|card| card.first == "Gus").count()), 1);
    assert!(flow.app.session.mailbox.contact_changes.is_empty());
}

#[test]
fn a_new_contact_the_server_has_no_room_for_waits_and_goes_once_there_is_room() {
    let mut flow = Flow::start();
    flow.with_books();
    // A tab that keeps nothing of its own (as `with_books` makes it), and an account over its quota.
    flow.server.with(|d| d.pim.full = true);
    flow.app.execute("people.new", json!({"first": "Gus", "last": "Hale", "email": "gus@example.org"})).unwrap();
    let gus = flow.contact("Gus").map(|c| c.id).unwrap();
    assert!(flow.look(SECOND));
    let said = flow.app.sync_problems_of(flow.account);
    let line = "The server's mailbox is full: \"Gus Hale\" waits here and goes as soon as there is room.";
    assert!(said.iter().any(|said| said == line), "{said:?}");
    // In French, as the window reads it back.
    let french = mailcraft_ui_egui::i18n::Lang::from_tag("fr").unwrap();
    let read = mailcraft_ui_egui::i18n::run_words::explain(french, mailcraft_ui_egui::i18n::run_words::Family::Line, line, &flow.app.session.mailbox);
    assert_eq!(
        read.map(|reading| reading.text).as_deref(),
        Some("L’espace du serveur est plein : \"Gus Hale\" attend ici et partira dès qu’il y aura de la place.")
    );
    // Refused again and again: he stays, in his book, and waits.
    for _ in 0..4 {
        flow.app.session.clock.now = flow.app.session.clock.now.map(|now| now + 1_000);
        flow.look(MINUTE);
        assert!(flow.app.session.mailbox.contact(gus).is_some_and(|c| c.book != 0 && c.origin.id.is_empty()), "never taken away");
        assert_eq!(flow.app.session.mailbox.contact_changes.len(), 1);
    }
    assert!(flow.card("Gus Hale").is_none());
    // Room again: he goes.
    flow.server.with(|d| d.pim.full = false);
    flow.app.session.clock.now = flow.app.session.clock.now.map(|now| now + 1_000);
    assert!(flow.look(MINUTE));
    assert!(flow.card("Gus Hale").is_some());
    assert_eq!(flow.contact("Gus").map(|c| (c.id, mailcraft_jmap::pim::server_card_id(&c.origin.id))), Some((gus, Some("Gus Hale".to_string()))));
    assert!(flow.app.session.mailbox.contact_changes.is_empty());
}

#[test]
fn an_address_taken_out_after_a_new_contact_was_sent_whole_again_goes_from_its_card() {
    let mut flow = Flow::start();
    flow.with_books();
    // Sent with a: the request fails before the server ran it (nothing is made).
    flow.server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", mailcraft_jmap::standin::Effect::RefuseWith(500))));
    flow.app.execute("people.new", json!({"first": "Gus", "last": "Hale", "email": "a@example.org"})).unwrap();
    let gus = flow.contact("Gus").map(|c| c.id).unwrap();
    assert!(flow.look(SECOND));
    assert!(flow.card("Gus Hale").is_none());
    // b added; the next try finds no card of his and sends him whole, and a gateway's 502 comes back after the server
    // made him.
    let both = json!({"id": gus, "emails": [{"label": "", "value": "a@example.org"}, {"label": "", "value": "b@example.org"}]});
    flow.app.execute("people.edit", both).unwrap();
    flow.server.with(|d| d.faults.push_back(Fault::on("ContactCard/set", mailcraft_jmap::standin::Effect::Status(502))));
    flow.app.session.clock.now = flow.app.session.clock.now.map(|now| now + 1_000);
    assert!(flow.look(5.0 * MINUTE));
    assert_eq!(flow.card("Gus Hale").map(|card| card.emails), Some(vec!["a@example.org".to_string(), "b@example.org".to_string()]));
    // b taken out here: the next try finds his card, and b goes from it.
    let one = json!({"id": gus, "emails": [{"label": "", "value": "a@example.org"}]});
    flow.app.execute("people.edit", one).unwrap();
    flow.app.session.clock.now = flow.app.session.clock.now.map(|now| now + 1_000);
    assert!(flow.look(10.0 * MINUTE));
    assert_eq!(flow.card("Gus Hale").map(|card| card.emails), Some(vec!["a@example.org".to_string()]), "nothing comes back");
    assert!(flow.app.session.mailbox.contact_changes.is_empty(), "{:?}", flow.app.session.mailbox.contact_changes);
    assert_eq!(flow.contact("Gus").map(|c| c.emails.len()), Some(1));
}

// A meeting held online, as the server says it.

/// A link to a meeting room of the stand-in's.
fn room(name: &str) -> String {
    format!("{}{name}", mailcraft_jmap::standin::MEET_ROOMS)
}

#[test]
fn a_meet_room_added_in_the_desk_gives_join_its_link_wherever_join_is_offered() {
    let mut flow = Flow::start();
    // A room made for the budget review in the desk: the store keeps it among the event's links.
    flow.server.with(|d| {
        let budget = d.pim.events.iter_mut().find(|e| e.raw == "budget").unwrap();
        budget.props.insert("links".into(), json!({"meet": {"@type": "Link", "href": room("abcd-efgh-ijkl")}}));
    });
    flow.look(2.0 * MINUTE + SECOND);
    let budget = flow.event("Budget review").unwrap();
    // The card, the menu, the ribbon and the reminder all offer Join by this link (`Event::join_link`), and Join opens
    // what `calendar.join` gives.
    assert_eq!(budget.join_link(), Some(room("abcd-efgh-ijkl").as_str()));
    assert_eq!(budget.online.as_ref().map(|online| online.provider), Some(mailcraft_model::OnlineProvider::Other));
    let joined = flow.app.execute("calendar.join", json!({"id": budget.id})).unwrap();
    assert_eq!(joined["url"], room("abcd-efgh-ijkl"));
}

#[test]
fn a_room_taken_away_in_the_desk_takes_join_away() {
    let mut flow = Flow::start();
    flow.server.with(|d| {
        let budget = d.pim.events.iter_mut().find(|e| e.raw == "budget").unwrap();
        budget.props.insert("links".into(), json!({"meet": {"@type": "Link", "href": room("abcd-efgh-ijkl")}}));
    });
    flow.look(2.0 * MINUTE + SECOND);
    assert!(flow.event("Budget review").is_some_and(|e| e.join_link().is_some()));
    flow.server.with(|d| {
        let budget = d.pim.events.iter_mut().find(|e| e.raw == "budget").unwrap();
        budget.props.remove("links");
    });
    flow.look(2.0 * MINUTE + SECOND);
    let budget = flow.event("Budget review").unwrap();
    assert!(budget.online.is_none() && budget.join_link().is_none());
    assert!(flow.app.execute("calendar.join", json!({"id": budget.id})).is_err());
}

// An online meeting made here, by a server that makes them.

impl Flow {
    /// The server makes online meetings, and the window has read its list since (the wide stretch).
    fn with_meet(&mut self) {
        self.server.with(|d| d.pim.meet = true);
        self.look(10.0 * MINUTE + SECOND);
    }

    /// How many online meeting rooms the server made.
    fn rooms(&self) -> u32 {
        self.server.with(|d| d.pim.rooms)
    }
}

/// What a line of the window reads as in French.
fn in_french(flow: &Flow, line: &str) -> String {
    let french = mailcraft_ui_egui::i18n::Lang::from_tag("fr").unwrap();
    mailcraft_ui_egui::i18n::run_words::explain(french, mailcraft_ui_egui::i18n::run_words::Family::Line, line, &flow.app.session.mailbox)
        .map(|reading| reading.text)
        .unwrap_or_else(|| panic!("{line:?} isn't read"))
}

#[test]
fn an_appointment_made_online_here_comes_back_with_its_link_and_join_opens_it() {
    let mut flow = Flow::start();
    flow.with_meet();
    let made = flow.app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "online": true}));
    let id = made.unwrap()["id"].as_u64().unwrap();
    assert!(flow.look(SECOND));
    let dentist = flow.app.session.mailbox.event(id).cloned().unwrap();
    assert_eq!(dentist.join_link(), Some(room("room-1").as_str()));
    assert_eq!(flow.app.execute("calendar.join", json!({"id": id})).unwrap()["url"], room("room-1"));
    // Read again: the same room.
    flow.look(2.0 * MINUTE + SECOND);
    assert_eq!((flow.app.session.mailbox.event(id).and_then(|e| e.join_link().map(str::to_string)), flow.rooms()), (Some(room("room-1")), 1));
    // Made by the server, it stays one: said so, as an online meeting, and in French.
    let refused = flow.app.execute("calendar.teamsMeeting", json!({"id": id, "value": false})).unwrap_err();
    assert!(refused.contains("has made this an online meeting"), "{refused}");
    assert_eq!(
        in_french(&flow, &refused),
        "Le serveur du compte en a fait une réunion en ligne, et elle le reste : ce choix ne peut pas être annulé d’ici."
    );
    assert!(flow.app.sync_problems_of(flow.account).is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn an_appointment_made_online_where_the_server_has_no_meet_is_refused_with_the_reason() {
    let mut flow = Flow::start();
    let refused = flow
        .app
        .execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "online": true}))
        .unwrap_err();
    assert!(refused.contains("This calendar's account can't make online meetings."), "{refused}");
    assert_eq!(in_french(&flow, &refused), "Le compte de ce calendrier ne peut pas créer de réunions en ligne.");
    assert!(flow.event("Dentist").is_none() && flow.app.session.mailbox.event_changes.is_empty());
    flow.look(2.0 * MINUTE + SECOND);
    assert!(flow.on_server("Dentist").is_none() && flow.rooms() == 0);
}

#[test]
fn a_change_of_title_never_makes_a_second_room() {
    let mut flow = Flow::start();
    flow.with_meet();
    let made = flow.app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "online": true}));
    let id = made.unwrap()["id"].as_u64().unwrap();
    assert!(flow.look(SECOND));
    flow.app.execute("calendar.edit", json!({"id": id, "title": "Dentist (Dr Lee)"})).unwrap();
    assert!(flow.look(SECOND));
    flow.look(2.0 * MINUTE + SECOND);
    assert_eq!(flow.on_server("Dentist (Dr Lee)").map(|e| e.props["links"].as_object().map(|links| links.len())), Some(Some(1)));
    assert_eq!((flow.rooms(), flow.app.session.mailbox.event(id).and_then(|e| e.join_link().map(str::to_string))), (1, Some(room("room-1"))));
}

#[test]
fn an_appointment_made_online_keeps_its_one_room_through_a_new_title_and_guests_invited_later() {
    let mut flow = Flow::start();
    flow.with_meet();
    // An appointment (nobody invited), saved as one with Online Meeting on: its room is made, its link comes back, and
    // nobody is written to.
    let made =
        flow.app.execute("calendar.new", json!({"title": "Site visit", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "online": true}));
    let id = made.unwrap()["id"].as_u64().unwrap();
    assert!(flow.look(SECOND));
    let visit = flow.app.session.mailbox.event(id).cloned().unwrap();
    assert_eq!((visit.attendees.len(), visit.join_link()), (0, Some(room("room-1").as_str())));
    assert_eq!(flow.app.execute("calendar.join", json!({"id": id})).unwrap()["url"], room("room-1"));
    assert!(flow.server.with(|d| d.pim.scheduled.is_empty()), "no mail to anyone");
    // A new title: the same room.
    flow.app.execute("calendar.edit", json!({"id": id, "title": "Site visit (north wing)"})).unwrap();
    assert!(flow.look(SECOND));
    assert_eq!(flow.rooms(), 1);
    // Grace invited later: she is sent the invitation, and the meeting keeps its one room.
    flow.app.execute("calendar.edit", json!({"id": id, "attendees": "grace@example.org", "send": true})).unwrap();
    assert!(flow.look(SECOND));
    let told = flow.server.with(|d| d.pim.scheduled.clone());
    assert_eq!(told.iter().map(|t| (t.what, t.to.clone())).collect::<Vec<_>>(), [("invitation", vec!["grace@example.org".to_string()])]);
    let there = flow.on_server("Site visit (north wing)").unwrap();
    let links: Vec<String> = there
        .props
        .get("links")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(_, link)| link.get("href").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert_eq!(links, [room("room-1")], "{:?}", there.props);
    assert!(there.props.get("participants").is_some_and(|people| people.to_string().contains("grace@example.org")), "{:?}", there.props);
    flow.look(2.0 * MINUTE + SECOND);
    assert_eq!((flow.rooms(), flow.app.session.mailbox.event(id).and_then(|e| e.join_link().map(str::to_string))), (1, Some(room("room-1"))));
    assert!(flow.app.session.mailbox.event_changes.is_empty(), "{:?}", flow.app.session.mailbox.event_changes);
}

#[test]
fn an_online_appointment_moved_to_another_calendar_keeps_its_room_and_no_second_is_made() {
    let mut flow = Flow::start();
    flow.with_meet();
    let made = flow.app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "online": true}));
    let id = made.unwrap()["id"].as_u64().unwrap();
    assert!(flow.look(SECOND));
    assert_eq!(flow.event("Dentist").and_then(|e| e.join_link().map(str::to_string)), Some(room("room-1")));
    // Into the team's calendar: made again there, and the first deleted once the copy is seen there.
    let team = flow.calendar("Team").unwrap().id;
    assert_eq!(flow.app.execute("calendar.edit", json!({"id": id, "calendar": team})).unwrap()["moved"], true);
    for _ in 0..3 {
        flow.look(2.0 * MINUTE + SECOND);
    }
    let there: Vec<Event> = flow.server.with(|d| d.pim.events.iter().filter(|e| e.props.get("title") == Some(&json!("Dentist"))).cloned().collect());
    assert_eq!(there.iter().map(|e| e.calendars.clone()).collect::<Vec<_>>(), [vec!["team".to_string()]], "the copy, in the team's calendar");
    assert!(there[0].props.get("links").is_some_and(|links| links.to_string().contains(&room("room-1"))), "with its room: {:?}", there[0].props);
    assert_eq!(flow.rooms(), 1, "no second room");
    // Here: the same, and Join opens the same room.
    let dentist = flow.event("Dentist").unwrap();
    assert_eq!((dentist.calendar, dentist.join_link()), (team, Some(room("room-1").as_str())));
    assert!(flow.app.session.mailbox.event_changes.is_empty(), "{:?}", flow.app.session.mailbox.event_changes);
    assert!(flow.app.sync_problems_of(flow.account).is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn an_appointment_held_online_elsewhere_moves_to_another_calendar_as_before() {
    let mut flow = Flow::start();
    flow.with_meet();
    // The budget review's link to join is another service's (an outside invitation's, another program's): the server
    // keeps it among its virtual locations, and makes no room of it.
    flow.server.with(|d| {
        let budget = d.pim.events.iter_mut().find(|e| e.raw == "budget").unwrap();
        budget
            .props
            .insert("virtualLocations".into(), json!({"elsewhere": {"@type": "VirtualLocation", "uri": "https://elsewhere.example.org/j/123"}}));
    });
    flow.look(2.0 * MINUTE + SECOND);
    let budget = flow.event("Budget review").unwrap();
    assert_eq!(budget.join_link(), Some("https://elsewhere.example.org/j/123"));
    // Moved: made again in the team's calendar without that link (the server keeps only a room of its own), and nothing
    // is said of an online meeting.
    let team = flow.calendar("Team").unwrap().id;
    assert_eq!(flow.app.execute("calendar.edit", json!({"id": budget.id, "calendar": team})).unwrap()["moved"], true);
    for _ in 0..3 {
        flow.look(2.0 * MINUTE + SECOND);
    }
    let there: Vec<Event> =
        flow.server.with(|d| d.pim.events.iter().filter(|e| e.props.get("title") == Some(&json!("Budget review"))).cloned().collect());
    assert_eq!(there.iter().map(|e| e.calendars.clone()).collect::<Vec<_>>(), [vec!["team".to_string()]], "moved");
    assert_eq!(flow.event("Budget review").map(|e| e.calendar), Some(team));
    assert_eq!(flow.rooms(), 0);
    assert!(flow.app.session.mailbox.event_changes.is_empty(), "{:?}", flow.app.session.mailbox.event_changes);
    assert!(flow.app.sync_problems_of(flow.account).is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn an_appointment_edited_before_the_answer_that_made_it_came_never_makes_a_second_room() {
    let mut flow = Flow::start();
    flow.with_meet();
    let made = flow.app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "online": true}));
    let id = made.unwrap()["id"].as_u64().unwrap();
    // The job that makes it is on its way (no link known here yet) when it is changed here.
    flow.hold();
    assert!(flow.look(SECOND));
    flow.app.execute("calendar.edit", json!({"id": id, "title": "Dentist (Dr Lee)", "location": "Main Street 4"})).unwrap();
    flow.release();
    // What was changed meanwhile goes after it, and asks for no room.
    for _ in 0..2 {
        flow.look(2.0 * MINUTE + SECOND);
    }
    assert_eq!(flow.rooms(), 1, "one room");
    let there = flow.on_server("Dentist (Dr Lee)").unwrap_or_else(|| panic!("{:?}", flow.server.with(|d| d.pim.events.clone())));
    assert_eq!(there.props.get("links").and_then(Value::as_object).map(|links| links.len()), Some(1), "{:?}", there.props);
    assert!(flow.server.with(|d| d.pim.written.iter().all(|(_, patch)| patch.get("virtualLocations").is_none())), "no room asked for again");
    let dentist = flow.app.session.mailbox.event(id).cloned().unwrap();
    assert_eq!((dentist.title.as_str(), dentist.join_link()), ("Dentist (Dr Lee)", Some(room("room-1").as_str())));
    assert!(flow.app.session.mailbox.event_changes.is_empty(), "{:?}", flow.app.session.mailbox.event_changes);
}

#[test]
fn a_room_the_server_wont_make_leaves_the_appointment_saved_and_says_so() {
    let mut flow = Flow::start();
    flow.with_meet();
    // Made where the server refuses its room: saved, without it, and said; what the refusal is, in words of ours.
    let refused = |flow: &mut Flow, title: &str, refusal: &'static str| -> String {
        flow.server.with(|d| d.pim.room_refusal = Some(refusal));
        let made = flow.app.execute("calendar.new", json!({"title": title, "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "online": true}));
        let id = made.unwrap()["id"].as_u64().unwrap();
        assert!(flow.look(SECOND));
        let event = flow.app.session.mailbox.event(id).cloned().unwrap();
        assert!(event.online.is_none() && !event.origin.id.is_empty(), "saved, without its room: {event:?}");
        assert!(flow.on_server(title).is_some() && flow.app.session.mailbox.event_changes.is_empty());
        let problems = flow.app.sync_problems_of(flow.account);
        problems.iter().find(|line| line.starts_with(&format!("\"{title}\""))).cloned().unwrap_or_else(|| panic!("{problems:?}"))
    };
    // No more rooms just now.
    let said = refused(&mut flow, "Dentist", "rateLimit");
    assert_eq!(
        said,
        "\"Dentist\" couldn't be made an online meeting: the account's server can't make more meeting rooms just now. The meeting itself is saved, without a link to join; turn it on again later."
    );
    assert_eq!(
        in_french(&flow, &said),
        "\"Dentist\" n’a pas pu devenir une réunion en ligne : le serveur du compte ne peut pas créer d’autres salles de réunion pour l’instant. La réunion elle-même est enregistrée, sans lien pour la rejoindre ; réactivez-la plus tard."
    );
    // Not for her.
    let said = refused(&mut flow, "Physio", "forbidden");
    assert_eq!(
        in_french(&flow, &said),
        "\"Physio\" n’a pas pu devenir une réunion en ligne : le serveur du compte ne vous permet pas de créer des salles de réunion. La réunion elle-même est enregistrée, sans lien pour la rejoindre."
    );
    // Any other refusal of the room: the server's own words.
    let said = refused(&mut flow, "Optician", "invalidProperties");
    assert!(said.contains("(No more rooms can be made just now: try again later.)"), "the server's words: {said}");
    assert!(
        in_french(&flow, &said).starts_with("\"Optician\" n’a pas pu devenir une réunion en ligne : le serveur du compte ne l’a pas accepté ("),
        "{said}"
    );
    assert_eq!(flow.rooms(), 0);
}

#[test]
fn a_new_appointment_made_without_its_room_whose_answer_was_lost_says_so_when_sent_again() {
    let mut flow = Flow::start();
    flow.with_meet();
    // Its room refused, it is made without, and that answer is lost on the way; the read that follows fails too.
    flow.server.with(|d| {
        d.pim.room_refusal = Some("forbidden");
        d.faults.push_back(Fault::later("CalendarEvent/set", 1, mailcraft_jmap::standin::Effect::LoseAnswer));
        d.faults.push_back(Fault::on("CalendarEvent/query", mailcraft_jmap::standin::Effect::MethodError("serverFail")));
    });
    let made = flow.app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "online": true}));
    let id = made.unwrap()["id"].as_u64().unwrap();
    assert!(flow.look(SECOND));
    assert!(
        flow.app.session.mailbox.event_changes.iter().any(|change| change.kind == mailcraft_model::remote::EventChangeKind::Create),
        "it waits to go again"
    );
    // Sent again once it is due (rooms can be made by now): the server has it already, without a room, and says no more.
    flow.server.with(|d| d.pim.room_refusal = None);
    flow.app.session.clock.now = Some(NOW + 120);
    flow.look(2.0 * MINUTE + SECOND);
    let dentist = flow.app.session.mailbox.event(id).cloned().unwrap();
    assert!(dentist.online.is_none() && !dentist.origin.id.is_empty(), "saved, without a room: {dentist:?}");
    assert_eq!((flow.rooms(), flow.server.with(|d| d.pim.events.iter().filter(|e| e.props.get("title") == Some(&json!("Dentist"))).count())), (0, 1));
    let problems = flow.app.sync_problems_of(flow.account);
    let said = problems.iter().find(|line| line.starts_with("\"Dentist\"")).unwrap_or_else(|| panic!("{problems:?}"));
    assert_eq!(
        said,
        "\"Dentist\" couldn't be made an online meeting: the account's server wouldn't have it. The meeting itself is saved, without a link to join."
    );
    assert_eq!(
        in_french(&flow, said),
        "\"Dentist\" n’a pas pu devenir une réunion en ligne : le serveur du compte ne l’a pas accepté. La réunion elle-même est enregistrée, sans lien pour la rejoindre."
    );
    assert!(flow.app.session.mailbox.event_changes.is_empty(), "{:?}", flow.app.session.mailbox.event_changes);
}

#[test]
fn an_appointment_made_online_after_its_server_stopped_making_them_is_saved_without_and_says_so() {
    let mut flow = Flow::start();
    flow.with_meet();
    // The calendar's list said it makes them; since, its server stopped (its session no longer says so).
    flow.server.with(|d| d.pim.meet = false);
    let made = flow.app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T14:00", "end": "2026-10-15T15:00", "online": true}));
    let id = made.unwrap()["id"].as_u64().unwrap();
    assert!(flow.look(SECOND));
    let dentist = flow.app.session.mailbox.event(id).cloned().unwrap();
    assert!(dentist.online.is_none() && !dentist.origin.id.is_empty(), "saved, without a room: {dentist:?}");
    let problems = flow.app.sync_problems_of(flow.account);
    let said = problems.iter().find(|line| line.starts_with("\"Dentist\"")).unwrap_or_else(|| panic!("{problems:?}"));
    assert_eq!(
        said,
        "\"Dentist\" couldn't be made an online meeting: its account's server no longer makes them. The meeting itself is saved, without a link to join."
    );
    assert_eq!(
        in_french(&flow, said),
        "\"Dentist\" n’a pas pu devenir une réunion en ligne : le serveur de son compte n’en crée plus. La réunion elle-même est enregistrée, sans lien pour la rejoindre."
    );
}

#[test]
fn one_day_of_a_series_made_online_goes_without_it_and_says_the_whole_series_is_one() {
    let mut flow = Flow::start();
    flow.server.with(|d| d.pim.meet = true);
    flow.with_series();
    let site = flow.event("Site meeting").unwrap();
    flow.app
        .execute(
            "calendar.edit",
            json!({"id": site.id, "scope": "occurrence", "occurrence": "2026-10-19", "title": "Site meeting (online)", "online": true}),
        )
        .unwrap();
    assert!(flow.look(SECOND));
    assert_eq!(flow.site().overrides["2026-10-19T09:00:00"]["title"], "Site meeting (online)", "the rest went");
    assert_eq!(flow.rooms(), 0);
    let problems = flow.app.sync_problems_of(flow.account);
    let said = problems.iter().find(|line| line.contains("whole series")).unwrap_or_else(|| panic!("{problems:?}"));
    assert_eq!(
        said,
        "\"Site meeting (online)\": an online meeting is the whole series', so it was not made for that day alone. Turn it on for the whole series."
    );
    assert_eq!(
        in_french(&flow, said),
        "\"Site meeting (online)\" : une réunion en ligne vaut pour toute la série, elle n’a donc pas été créée pour ce seul jour. Activez-la pour toute la série."
    );
    // The meeting alone, for another day: not sent at all, the day as it was, and said the same way.
    flow.app.execute("calendar.edit", json!({"id": site.id, "scope": "occurrence", "occurrence": "2026-10-26", "online": true})).unwrap();
    assert!(flow.look(SECOND));
    assert!(!flow.site().overrides.contains_key("2026-10-26T09:00:00"), "{:?}", flow.site().overrides);
    let problems = flow.app.sync_problems_of(flow.account);
    assert!(problems.iter().any(|line| line.starts_with("\"Site meeting\": an online meeting is the whole series'")), "{problems:?}");
    assert_eq!(flow.rooms(), 0);
}

// The person's notes, on the same rig.
#[path = "jmap_notes_flow_tests.rs"]
mod notes;
