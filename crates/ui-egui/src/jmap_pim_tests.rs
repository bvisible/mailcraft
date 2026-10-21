// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's calendar jobs in the window: a host that keeps every job for the test to answer by hand, when
//! jobs start and what they ask, one at a time, a job given up, a sender dropped, a refused token, a busy server, the
//! order an answer is taken in, and the address books and contacts that come with it. Invented people at `example.org`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use mailcraft_engine::{Module, Session};
use mailcraft_model::book::{BooksDownload, ContactChangeKind, ContactOutcome, ContactsUploaded, RemoteBook};
use mailcraft_model::remote::{
    CalendarsDownload, ContactsDownload, EventChangeKind, EventOutcome, EventResult, EventsDownload, EventsUploaded, RemoteCalendar, RemoteContact,
    RemoteEvent,
};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult, PimOffers};
use mailcraft_model::{Id, Labeled, Mailbox};
use serde_json::json;

use super::JmapPimJob;
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const ME: &str = "ada@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session";
const SECOND: f64 = 1_000.0;
const MINUTE: f64 = 60.0 * SECOND;

/// One job the host was handed, and the way to answer it.
struct Job {
    request: JmapPimRequest,
    secret: Option<String>,
    cancel: Arc<AtomicBool>,
    to: Sender<JmapPimResult>,
}

type Asked = Arc<Mutex<Vec<Job>>>;

/// A window with a JMAP account whose credential the host holds, the calendar in front, and a host that keeps every
/// calendar job it is handed. The account's id.
fn window() -> (MailApp, Id, Asked) {
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    // A mail runner too: an account is asked only by a window that can run it.
    app.services.jmap = Some(Box::new(|_| mpsc::channel().1));
    app.services.jmap_host_credentials = true;
    let asked: Asked = Default::default();
    let log = asked.clone();
    app.services.jmap_pim = Some(Box::new(move |job: JmapPimJob| {
        let (tx, rx) = mpsc::channel();
        log.lock().unwrap().push(Job { request: job.request, secret: job.secret, cancel: job.cancel, to: tx });
        rx
    }));
    let id = add_account(&mut app, ME);
    app.execute("view.calendar", json!({})).unwrap();
    (app, id, asked)
}

fn add_account(app: &mut MailApp, email: &str) -> Id {
    let added = app.execute("account.add", json!({"email": email, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}}));
    added.unwrap()["id"].as_u64().unwrap()
}

/// The jobs asked for so far.
fn jobs(asked: &Asked) -> usize {
    asked.lock().unwrap().len()
}

/// The request of the last job.
fn last(asked: &Asked) -> JmapPimRequest {
    asked.lock().unwrap().last().unwrap().request.clone()
}

/// Answer the last job with `result`, and have the window take it in.
fn answer(app: &mut MailApp, asked: &Asked, result: JmapPimResult) {
    let to = asked.lock().unwrap().last().unwrap().to.clone();
    to.send(result).unwrap();
    app.poll_jmap_pim();
}

/// What a server that keeps two calendars answers: the list (Ada's, the main one, and the team's) and an event in each.
fn answered(account: Id) -> JmapPimResult {
    let event =
        |id: &str, title: &str, start: i64| RemoteEvent { id: id.into(), title: title.into(), start, end: start + 3_600, ..Default::default() };
    JmapPimResult {
        account,
        email: ME.into(),
        offers: Some(PimOffers { calendars: true, contacts: true }),
        calendars: Some(CalendarsDownload {
            account,
            email: ME.into(),
            complete: true,
            calendars: vec![
                RemoteCalendar { id: "C-ada".into(), name: "Ada".into(), main: true, read_only: true, plain: true, ..Default::default() },
                RemoteCalendar { id: "C-team".into(), name: "Team".into(), read_only: true, plain: true, ..Default::default() },
            ],
        }),
        events: Some(EventsDownload {
            account,
            email: ME.into(),
            from: NOW - 31 * 86_400,
            to: NOW + 366 * 86_400,
            complete: true,
            events: vec![event("V1", "Budget review", NOW + 86_400)],
            ..Default::default()
        }),
        more_events: vec![EventsDownload {
            account,
            email: ME.into(),
            calendar: "C-team".into(),
            from: NOW - 31 * 86_400,
            to: NOW + 366 * 86_400,
            complete: true,
            events: vec![event("V2", "Team stand-up", NOW + 2 * 86_400)],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn titles(app: &MailApp) -> Vec<String> {
    let mut titles: Vec<String> = app.session.mailbox.events.values().map(|e| e.title.clone()).collect();
    titles.sort();
    titles
}

#[test]
fn an_account_with_a_credential_is_asked_for_its_lists_and_its_wide_stretch_at_once() {
    let (mut app, id, asked) = window();
    assert!(app.jmap_pim_tick_at(true, 0.0));
    assert_eq!(jobs(&asked), 1);
    let request = last(&asked);
    assert_eq!((request.account, request.email.as_str(), request.session.as_str(), request.bearer), (id, ME, SESSION, true));
    assert!(request.lists && request.main.is_empty(), "the first job lists the calendars");
    assert_eq!((request.from, request.to), (NOW - 31 * 86_400, NOW + 366 * 86_400), "the wide stretch");
    assert_eq!(request.asks.iter().map(|a| a.calendar.as_str()).collect::<Vec<_>>(), [""], "the main one, before any list");
    assert_eq!((request.me.clone(), request.now, request.zone.as_str()), (vec![ME.to_string()], NOW, "UTC"));
    assert!(asked.lock().unwrap()[0].secret.is_none(), "the host holds the credential");
    // Taken in: the calendars and their events, the main one's name kept for the next jobs.
    answer(&mut app, &asked, answered(id));
    assert_eq!(titles(&app), ["Budget review", "Team stand-up"]);
    assert!(app.session.mailbox.calendars.iter().any(|c| c.origin.account == id && c.origin.id.is_empty() && c.name == "Ada" && c.plain));
    assert_eq!(app.jmap_pim.mains.get(&id).map(String::as_str), Some("C-ada"));
    assert!(!app.jmap_pim_running());
}

#[test]
fn the_days_on_screen_are_asked_every_two_minutes_in_front_and_ten_behind() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, answered(id));
    // Not again at once; after two minutes, the days on screen, by name.
    assert!(!app.jmap_pim_tick_at(true, 30.0 * SECOND));
    assert!(app.jmap_pim_tick_at(true, 2.0 * MINUTE));
    let request = last(&asked);
    assert!(!request.lists, "the list comes with the wide stretch");
    assert_eq!(request.main, "C-ada");
    assert_eq!((request.from, request.to), app.calendar_window());
    answer(&mut app, &asked, JmapPimResult { offers: Some(PimOffers { calendars: true, contacts: false }), ..Default::default() });
    // Behind: every ten minutes, and the wide stretch is due then too.
    app.execute("view.mail", json!({})).unwrap();
    assert!(!app.jmap_pim_tick_at(true, 6.0 * MINUTE));
    assert!(app.jmap_pim_tick_at(true, 10.0 * MINUTE + SECOND));
}

#[test]
fn the_wide_stretch_and_the_lists_are_asked_every_ten_minutes() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, answered(id));
    for minute in [2.0, 4.0, 6.0, 8.0] {
        assert!(app.jmap_pim_tick_at(true, minute * MINUTE + SECOND));
        assert!(!last(&asked).lists, "at minute {minute}");
        answer(&mut app, &asked, answered(id));
    }
    assert!(app.jmap_pim_tick_at(true, 10.0 * MINUTE + SECOND));
    let wide = last(&asked);
    assert!(wide.lists && wide.to - wide.from == 397 * 86_400);
}

#[test]
fn the_window_coming_to_the_front_or_the_calendar_opening_asks_at_once() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, answered(id));
    // Behind the mail, the window loses the focus and gets it back: nothing, the calendar is not in front.
    app.execute("view.mail", json!({})).unwrap();
    app.jmap_pim_tick_at(false, 10.0 * SECOND);
    assert!(!app.jmap_pim_tick_at(true, 20.0 * SECOND));
    // The calendar comes in front: at once.
    app.execute("view.calendar", json!({})).unwrap();
    assert!(app.jmap_pim_tick_at(true, 30.0 * SECOND));
    answer(&mut app, &asked, answered(id));
    // So does People.
    app.execute("view.mail", json!({})).unwrap();
    app.jmap_pim_tick_at(true, 40.0 * SECOND);
    app.session.view.module = Module::People;
    assert!(app.jmap_pim_tick_at(true, 50.0 * SECOND));
    answer(&mut app, &asked, answered(id));
    // And the window coming back to the front with the calendar in it.
    app.execute("view.calendar", json!({})).unwrap();
    app.jmap_pim_tick_at(true, 55.0 * SECOND);
    let before = jobs(&asked);
    app.jmap_pim_tick_at(false, 60.0 * SECOND);
    assert!(app.jmap_pim_tick_at(true, 61.0 * SECOND));
    assert!(jobs(&asked) > before);
}

#[test]
fn the_ticked_calendars_are_asked_on_screen_and_all_of_them_on_the_wide_stretch() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, answered(id));
    // The team's calendar arrived unticked: the days on screen are the main one's alone.
    assert!(app.jmap_pim_tick_at(true, 2.0 * MINUTE + SECOND));
    assert_eq!(last(&asked).asks.iter().map(|a| a.calendar.as_str()).collect::<Vec<_>>(), [""]);
    answer(&mut app, &asked, answered(id));
    // Ticked, it is asked about too.
    let team = app.session.mailbox.calendars.iter().find(|c| c.origin.id == "C-team").map(|c| c.id).unwrap();
    app.session.mailbox.calendars.iter_mut().find(|c| c.id == team).unwrap().visible = true;
    assert!(app.jmap_pim_tick_at(true, 4.0 * MINUTE + 2.0 * SECOND));
    assert_eq!(last(&asked).asks.iter().map(|a| a.calendar.as_str()).collect::<Vec<_>>(), ["", "C-team"]);
    answer(&mut app, &asked, answered(id));
    // Unticked again, it is still read on the wide stretch.
    app.session.mailbox.calendars.iter_mut().find(|c| c.id == team).unwrap().visible = false;
    assert!(app.jmap_pim_tick_at(true, 10.0 * MINUTE + 3.0 * SECOND));
    assert_eq!(last(&asked).asks.iter().map(|a| a.calendar.as_str()).collect::<Vec<_>>(), ["", "C-team"]);
}

#[test]
fn a_calendar_ticked_here_is_asked_for_at_once() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, answered(id));
    assert!(!app.jmap_pim_tick_at(true, 10.0 * SECOND));
    let team = app.session.mailbox.calendars.iter().find(|c| c.origin.id == "C-team").map(|c| c.id).unwrap();
    app.execute("calendar.showCalendar", json!({"id": team, "value": true})).unwrap();
    assert!(app.jmap_pim_tick_at(true, 11.0 * SECOND), "not two minutes later: at once");
    assert_eq!(last(&asked).asks.iter().map(|a| a.calendar.as_str()).collect::<Vec<_>>(), ["", "C-team"]);
    answer(&mut app, &asked, answered(id));
    // Behind the calendar, a tick waits for the cadence.
    app.execute("calendar.showCalendar", json!({"id": team, "value": false})).unwrap();
    app.execute("view.mail", json!({})).unwrap();
    app.jmap_pim_tick_at(true, 12.0 * SECOND);
    app.execute("calendar.showCalendar", json!({"id": team, "value": true})).unwrap();
    assert!(!app.jmap_pim_tick_at(true, 13.0 * SECOND));
}

#[test]
fn one_job_at_a_time_and_none_beside_a_mail_run_of_the_account_or_a_graph_calendar_job() {
    let (mut app, id, asked) = window();
    assert!(app.jmap_pim_tick_at(true, 0.0));
    // In flight: no second, however due one is.
    app.jmap_pim.due = true;
    assert!(!app.jmap_pim_tick_at(true, MINUTE));
    answer(&mut app, &asked, answered(id));
    // The account's mail is being read: it waits.
    let (_mail, mail_rx) = mpsc::channel();
    app.jmap_rx = Some((id, true, mail_rx));
    assert!(!app.jmap_pim_tick_at(true, 3.0 * MINUTE));
    app.jmap_rx = None;
    // A Graph calendar job is under way: everything waits.
    let (_graph, graph_rx) = mpsc::channel();
    app.calendar.rx = Some((99, false, 0.0, graph_rx));
    assert!(!app.jmap_pim_tick_at(true, 3.0 * MINUTE));
    app.calendar.rx = None;
    assert!(app.jmap_pim_tick_at(true, 3.0 * MINUTE));
}

#[test]
fn a_job_not_answered_in_two_minutes_is_abandoned_and_its_late_answer_is_never_read() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    let cancel = asked.lock().unwrap()[0].cancel.clone();
    let late = asked.lock().unwrap()[0].to.clone();
    assert!(!app.jmap_pim_tick_at(true, 119.0 * SECOND));
    assert!(app.jmap_pim_running());
    // Two minutes: given up, and counted; the next waits half a minute from then.
    assert!(!app.jmap_pim_tick_at(true, 120.0 * SECOND));
    assert!(cancel.load(Ordering::Relaxed), "the host is told to stop");
    assert!(!app.jmap_pim_running());
    assert!(!app.jmap_pim_tick_at(true, 149.0 * SECOND));
    assert_eq!(app.jmap_pim.failures.get(&id).map(|f| f.0), Some(1));
    // Its answer comes after all: nobody listens for it any more, and nothing of it is taken in.
    assert!(late.send(answered(id)).is_err());
    app.poll_jmap_pim();
    assert!(titles(&app).is_empty());
    assert!(app.jmap_pim_tick_at(true, 150.0 * SECOND));
    assert_eq!(jobs(&asked), 2);
}

#[test]
fn a_dropped_sender_is_a_failure_with_a_back_off_from_thirty_seconds_to_ten_minutes() {
    let (mut app, id, asked) = window();
    let mut at = 0.0;
    let mut waits = Vec::new();
    assert!(app.jmap_pim_tick_at(true, at));
    for _ in 0..7 {
        // The host's task ends without an answer.
        asked.lock().unwrap().clear();
        app.poll_jmap_pim();
        assert!(!app.jmap_pim_running());
        // The next job waits: how long, from the start of the one that came to nothing?
        let mut next = at + SECOND;
        while !app.jmap_pim_tick_at(true, next) {
            next += SECOND;
        }
        waits.push(((next - at) / SECOND).round() as u64);
        at = next;
    }
    assert_eq!(waits, [30, 60, 120, 240, 480, 600, 600]);
    assert_eq!(app.jmap_pim.failures.get(&id).map(|f| f.0), Some(7));
    // A job that goes through ends the waiting.
    answer(&mut app, &asked, answered(id));
    assert!(!app.jmap_pim.failures.contains_key(&id));
}

#[test]
fn a_refused_token_stops_the_account_until_the_owner_asks_and_the_mail_run_shares_it() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, JmapPimResult { account: id, auth_refused: true, ..Default::default() });
    assert!(app.jmap_refused.contains(&id), "shared with the mail runs");
    assert!(!app.jmap_pim_tick_at(true, 20.0 * MINUTE));
    // The owner's Send/Receive is a fresh try for both.
    app.jmap_refused.remove(&id);
    app.jmap_pim_asked_for(id);
    assert!(app.jmap_pim_tick_at(true, 20.0 * MINUTE + SECOND));
}

#[test]
fn a_busy_server_is_left_alone_for_the_time_it_said() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    let started = crate::now_ms();
    answer(&mut app, &asked, JmapPimResult { account: id, busy_for: Some(120), offers: Some(PimOffers::default()), ..Default::default() });
    let calm = app.jmap_calm.get(&id).copied().unwrap();
    assert!(calm >= started + 119.0 * SECOND, "left alone two minutes");
    assert!(!app.jmap_pim_tick_at(true, calm - SECOND));
    // (Its offers said nothing is offered: asked again only at the owner's Send/Receive, below.)
    app.jmap_pim_asked_for(id);
    assert!(app.jmap_pim_tick_at(true, calm + SECOND));
}

#[test]
fn nothing_is_asked_of_an_account_that_offers_nothing_until_the_owners_send_receive() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, JmapPimResult { account: id, offers: Some(PimOffers::default()), ..Default::default() });
    assert!(!app.jmap_pim_tick_at(true, 30.0 * MINUTE), "a second mailbox of the person, which offers no calendar");
    assert!(!app.jmap_pim.failures.contains_key(&id), "offering nothing is not failing");
    // The owner's Send/Receive asks again, once its mail is read.
    app.execute("ui.sync", json!({"account": id})).unwrap();
    assert!(!app.jmap_pim_tick_at(true, 31.0 * MINUTE), "not beside the mail run");
    app.jmap_rx = None;
    assert!(app.jmap_pim_tick_at(true, 31.0 * MINUTE));
}

#[test]
fn send_receive_marks_the_calendar_due() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, answered(id));
    app.execute("view.mail", json!({})).unwrap();
    assert!(!app.jmap_pim_tick_at(true, MINUTE));
    app.execute("ui.sync", json!({"account": id})).unwrap();
    app.jmap_rx = None;
    assert!(app.jmap_pim_tick_at(true, MINUTE + SECOND), "the mail first, then the calendar, though it is behind");
    assert!(last(&asked).lists);
}

#[test]
fn the_answer_is_taken_in_the_order_lists_then_the_main_calendar_then_the_others() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    // A first answer: the list makes the calendars, which the events then go into, the team's in its own.
    answer(&mut app, &asked, answered(id));
    let calendar_of = |app: &MailApp, title: &str| {
        let event = app.session.mailbox.events.values().find(|e| e.title == title).unwrap();
        app.session.mailbox.calendar(event.calendar).unwrap().origin.id.clone()
    };
    assert_eq!((calendar_of(&app, "Budget review"), calendar_of(&app, "Team stand-up")), (String::new(), "C-team".to_string()));
    // The server says they may only be read: the engine refuses a new appointment.
    let refused = app.execute("calendar.new", json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00"}));
    assert!(refused.unwrap_err().contains("can only be read"));
}

#[test]
fn a_download_of_a_calendar_that_isnt_here_is_left_out() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    // No list came, and so no calendar of the server is here: the downloads are not taken in (the engine would make
    // a calendar for them, which nobody could tell was read-only).
    answer(&mut app, &asked, JmapPimResult { calendars: None, ..answered(id) });
    assert!(titles(&app).is_empty());
    assert!(!app.session.mailbox.calendars.iter().any(|c| c.origin.account == id));
}

#[test]
fn the_hosts_lines_go_to_the_problems_report() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    let line = "Couldn't list the calendars on the server: the server refused: serverFail".to_string();
    answer(&mut app, &asked, JmapPimResult { errors: vec![line.clone()], ..answered(id) });
    assert!(app.sync_problems.iter().any(|(account, _, _, lines)| *account == id && lines.contains(&line)));
    // And the job came to nothing, as far as waiting goes.
    assert_eq!(app.jmap_pim.failures.get(&id).map(|f| f.0), Some(1));
}

#[test]
fn an_invitation_for_a_jmap_account_asks_for_the_day_around_it() {
    let (mut app, id, asked) = window();
    let day = mailcraft_time::Date::new(2026, 10, 21).unwrap();
    app.calendar_job_around(id, day).unwrap();
    let request = last(&asked);
    assert_eq!(request.to - request.from, 3 * 86_400, "the day before, the day, the day after");
    assert_eq!(mailcraft_time::DateTime::from_timestamp(request.from, 0).date, day.add_days(-1));
    // It says nothing of when the days on screen were last asked for.
    assert!(!app.jmap_pim.checked.contains_key(&id));
    // Not while another job is under way.
    assert!(app.calendar_job_around(id, day).is_err());
}

#[test]
fn an_invitation_for_a_graph_account_is_as_before() {
    let (mut app, _, asked) = window();
    let added = app.execute("account.add", json!({"email": "bob@example.org", "kind": "microsoft"})).unwrap();
    let graph = added["id"].as_u64().unwrap();
    let said = app.calendar_job_around(graph, mailcraft_time::Date::new(2026, 10, 21).unwrap());
    assert_eq!(said.unwrap_err(), "This build can't reach the account's calendar.", "no Graph in this window");
    assert_eq!(jobs(&asked), 0);
}

#[test]
fn two_jmap_accounts_are_asked_one_after_the_other() {
    let (mut app, first, asked) = window();
    let second = add_account(&mut app, "ada.marlow@example.org");
    assert!(app.jmap_pim_tick_at(true, 0.0));
    assert_eq!(last(&asked).account, first);
    answer(&mut app, &asked, answered(first));
    assert!(app.jmap_pim_tick_at(true, SECOND));
    assert_eq!(last(&asked).account, second);
    assert_eq!(last(&asked).me.len(), 2, "both of the owner's addresses");
}

#[test]
fn a_window_with_no_pim_service_never_asks() {
    let (mut app, _, asked) = window();
    app.services.jmap_pim = None;
    assert!(!app.jmap_pim_tick_at(true, 0.0));
    // Nor one whose owner turned automatic Send/Receive off.
    let (mut app, _, _) = window();
    app.auto_sync = false;
    assert!(!app.jmap_pim_tick_at(true, 0.0));
    assert_eq!(jobs(&asked), 0);
}

#[test]
fn an_account_without_a_credential_at_hand_is_not_asked() {
    let (mut app, _, asked) = window();
    app.services.jmap_host_credentials = false;
    assert!(!app.jmap_pim_tick_at(true, 0.0), "nobody signed in this session");
    let id = app.session.mailbox.accounts.iter().find(|a| a.email == ME).map(|a| a.id).unwrap();
    app.passwords.insert(id, "secret".into());
    assert!(app.jmap_pim_tick_at(true, SECOND));
    assert_eq!(asked.lock().unwrap()[0].secret.as_deref(), Some("secret"));
    // What is shown of a job: never the secret.
    let job = JmapPimJob { request: last(&asked), secret: Some("secret".into()), cancel: Arc::new(AtomicBool::new(false)) };
    let shown = format!("{job:?}");
    assert!(!shown.contains("secret\"") && !shown.contains('@') && shown.contains("secret: true"), "{shown}");
}

/// `result` with what a server that keeps an address book answers besides: its book (read-only, as every book is until
/// writing is carried) and two people in it, the contacts at the state `state`.
fn with_contacts(mut result: JmapPimResult, account: Id, state: &str) -> JmapPimResult {
    let person = |id: &str, first: &str, last: &str, email: &str| RemoteContact {
        id: id.into(),
        book: "neoffice".into(),
        first: first.into(),
        last: last.into(),
        emails: vec![Labeled::new("", email)],
        ..Default::default()
    };
    let book = RemoteBook { id: "neoffice".into(), name: "Neoffice".into(), read_only: true, main: true, plain: true, ..Default::default() };
    result.books = Some(BooksDownload { account, email: ME.into(), complete: true, books: vec![book] });
    let contacts = vec![person("c-grace", "Grace", "Okafor", "grace@example.org"), person("c-bob", "Bob", "Lindqvist", "bob@example.org")];
    result.contacts = Some(ContactsDownload { account, email: ME.into(), complete: true, contacts });
    result.contacts_state = Some(state.into());
    result
}

/// The names of the contacts of the account's server here, in order.
fn kept_contacts(app: &MailApp, account: Id) -> Vec<String> {
    let mut names: Vec<String> = app.session.mailbox.contacts.values().filter(|c| c.origin.account == account).map(|c| c.name()).collect();
    names.sort();
    names
}

#[test]
fn the_contacts_are_asked_with_the_lists_every_ten_minutes_and_at_once_when_people_comes_to_the_front() {
    let (mut app, id, asked) = window();
    // The first job brings the contacts, holding no state.
    assert!(app.jmap_pim_tick_at(true, 0.0));
    let first = last(&asked);
    assert!(first.contacts && first.contacts_state.is_empty() && first.contacts_read == 0);
    answer(&mut app, &asked, with_contacts(answered(id), id, "s1"));
    assert_eq!(kept_contacts(&app, id), ["Bob Lindqvist", "Grace Okafor"]);
    assert_eq!(app.jmap_contacts_state(id), Some(("s1".to_string(), NOW)), "kept, with when the job was asked for");
    // The days on screen, two minutes on, don't bring them.
    assert!(app.jmap_pim_tick_at(true, 2.0 * MINUTE + SECOND));
    assert!(!last(&asked).contacts);
    answer(&mut app, &asked, answered(id));
    // People comes to the front: at once, with the state held, which the server still says.
    app.execute("view.people", json!({})).unwrap();
    assert!(app.jmap_pim_tick_at(true, 2.0 * MINUTE + 2.0 * SECOND));
    let people = last(&asked);
    assert!(people.contacts && !people.lists);
    assert_eq!((people.contacts_state.as_str(), people.contacts_read), ("s1", NOW));
    answer(&mut app, &asked, JmapPimResult { contacts: None, ..with_contacts(answered(id), id, "s1") });
    assert_eq!(app.jmap_contacts_state(id), Some(("s1".to_string(), NOW)), "unchanged: kept as it was");
    assert_eq!(kept_contacts(&app, id), ["Bob Lindqvist", "Grace Okafor"], "and the contacts stay as they are");
    // While People stays in front, the days on screen every two minutes, without the contacts.
    assert!(app.jmap_pim_tick_at(true, 4.0 * MINUTE + 3.0 * SECOND));
    assert!(!last(&asked).contacts);
    answer(&mut app, &asked, answered(id));
    // Ten minutes on: the wide stretch, and the contacts with it.
    assert!(app.jmap_pim_tick_at(true, 10.0 * MINUTE + SECOND));
    assert!(last(&asked).lists && last(&asked).contacts);
}

#[test]
fn the_state_of_the_contacts_is_forgotten_after_an_error() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, with_contacts(answered(id), id, "s1"));
    // A job whose contacts couldn't be read: no state comes, and the window forgets the one it had.
    app.execute("view.people", json!({})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    let line = "Couldn't bring the contacts down from the server: the server refused: serverFail".to_string();
    answer(&mut app, &asked, JmapPimResult { errors: vec![line], ..answered(id) });
    assert_eq!(app.jmap_contacts_state(id), None);
    assert_eq!(kept_contacts(&app, id), ["Bob Lindqvist", "Grace Okafor"], "what is here stays");
    // The next job that brings them holds none: they are all read.
    app.execute("view.mail", json!({})).unwrap();
    app.jmap_pim_tick_at(true, 10.0 * MINUTE + 2.0 * SECOND);
    assert!(last(&asked).contacts && last(&asked).contacts_state.is_empty());
    // A job that ends before the contacts (a busy server) learns nothing of them: the state is kept.
    answer(&mut app, &asked, with_contacts(answered(id), id, "s2"));
    assert_eq!(app.jmap_contacts_state(id).map(|(state, _)| state), Some("s2".to_string()));
    app.execute("view.people", json!({})).unwrap();
    assert!(app.jmap_pim_tick_at(true, 10.0 * MINUTE + 3.0 * SECOND));
    answer(
        &mut app,
        &asked,
        JmapPimResult { account: id, busy_for: Some(60), offers: Some(PimOffers { calendars: true, contacts: true }), ..Default::default() },
    );
    assert_eq!(app.jmap_contacts_state(id).map(|(state, _)| state), Some("s2".to_string()));
}

#[test]
fn an_unreadable_answer_changes_nothing() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, with_contacts(answered(id), id, "s1"));
    let before = app.session.mailbox.clone();
    // A job whose books and contacts are for another account (a mailbox replaced while it was under way).
    app.execute("view.people", json!({})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    let mut stranger =
        with_contacts(JmapPimResult { account: id, offers: Some(PimOffers { calendars: true, contacts: true }), ..Default::default() }, id, "s2");
    for download in stranger.books.iter_mut() {
        download.email = "eve@example.org".into();
        download.books[0].name = "Eve's".into();
    }
    for download in stranger.contacts.iter_mut() {
        download.email = "eve@example.org".into();
        download.contacts.clear();
    }
    answer(&mut app, &asked, stranger);
    assert!(app.session.mailbox.address_books == before.address_books && app.session.mailbox.contacts == before.contacts);
    assert!(app.sync_problems_of(id).iter().any(|line| line.contains("another account")), "{:?}", app.sync_problems_of(id));
    assert_eq!(app.jmap_contacts_state(id), None, "nothing that came was taken in: read them all next time");
}

#[test]
fn a_kept_contact_completes_the_to_field() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, with_contacts(answered(id), id, "s1"));
    // In their book, which came first.
    let grace = app.session.mailbox.contacts.values().find(|c| c.first == "Grace").cloned().unwrap();
    assert_eq!(app.session.mailbox.book_of(&grace).map(|b| (b.origin.id.as_str(), b.read_only)), Some(("neoffice", true)));
    // The To field finds her among the contacts, and People lists her.
    let found = mailcraft_engine::cmd::compose::suggest(&app.session, "gra", 6);
    assert_eq!(found.first().map(|s| (s.name.as_str(), s.email.as_str(), s.contact)), Some(("Grace Okafor", "grace@example.org", true)));
    let listed = app.execute("people.list", json!({"query": "Okafor"})).unwrap();
    assert_eq!(listed["contacts"][0]["email"], "grace@example.org");
    // Selected, Edit and Delete are greyed with their reason: the book may only be read.
    app.execute("people.select", json!({"id": grace.id})).unwrap();
    let reason = |app: &MailApp, command: &str| app.session.registry.get(command).and_then(|spec| (spec.enabled)(&app.session));
    let why = mailcraft_engine::cmd::remote_contacts::NO_CHANGING;
    // (Deleting says it is deleting that is not allowed.)
    let not_deleting = mailcraft_engine::cmd::remote_contacts::NO_DELETING;
    assert_eq!((reason(&app, "people.edit"), reason(&app, "people.delete")), (Some(why), Some(not_deleting)));
    assert!(!app.enabled("people.edit") && !app.enabled("people.delete"));
}

#[test]
fn an_account_that_offers_no_contacts_is_not_asked_for_them() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    assert!(!app.session.books_listed);
    answer(&mut app, &asked, JmapPimResult { offers: Some(PimOffers { calendars: true, contacts: false }), ..answered(id) });
    // The server said it keeps no address books: New Contact says there are none, not that they are still being read.
    assert!(app.session.books_listed);
    app.execute("view.people", json!({})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    assert!(!last(&asked).contacts, "a session with no address book");
    answer(&mut app, &asked, JmapPimResult { offers: Some(PimOffers { calendars: true, contacts: false }), ..answered(id) });
    assert!(app.jmap_pim_tick_at(true, 10.0 * MINUTE + 2.0 * SECOND));
    assert!(last(&asked).lists && !last(&asked).contacts);
    // Nor for the days around an invitation.
    let (mut app, id, asked) = window();
    app.calendar_job_around(id, mailcraft_time::Date::new(2026, 10, 21).unwrap()).unwrap();
    assert!(!last(&asked).contacts);
}

#[test]
fn a_job_that_came_to_nothing_brings_the_contacts_at_the_next() {
    let (mut app, id, asked) = window();
    app.jmap_pim_tick_at(true, 0.0);
    answer(&mut app, &asked, with_contacts(answered(id), id, "s1"));
    // People in front: a job for the contacts, whose task ends without an answer.
    app.execute("view.people", json!({})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    assert!(last(&asked).contacts);
    asked.lock().unwrap().clear();
    app.poll_jmap_pim();
    // Half a minute on: the next asks for them again, and the state is still the one held.
    assert!(app.jmap_pim_tick_at(true, 32.0 * SECOND));
    assert!(last(&asked).contacts && last(&asked).contacts_state == "s1");
}

// What is changed here goes with the next job, first.

/// The window after its first job was answered with calendars Ada may write.
fn writable_window() -> (MailApp, Id, Asked) {
    let (mut app, id, asked) = window();
    assert!(app.jmap_pim_tick_at(true, 0.0));
    let mut first = answered(id);
    for calendar in first.calendars.as_mut().map(|list| list.calendars.iter_mut()).into_iter().flatten() {
        calendar.read_only = false;
    }
    answer(&mut app, &asked, first);
    (app, id, asked)
}

/// What the host answers a job that wrote `outcomes` and read the main calendar's `events`.
fn wrote(account: Id, outcomes: Vec<EventOutcome>, events: Vec<RemoteEvent>) -> JmapPimResult {
    JmapPimResult {
        account,
        email: ME.into(),
        offers: Some(PimOffers { calendars: true, contacts: true }),
        uploaded: Some(EventsUploaded { account, email: ME.into(), outcomes, token_refused: false }),
        events: Some(EventsDownload {
            account,
            email: ME.into(),
            from: NOW - 86_400,
            to: NOW + 14 * 86_400,
            complete: true,
            events,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn a_change_made_here_goes_at_once_and_what_the_server_said_is_taken_in_before_the_events() {
    let (mut app, id, asked) = writable_window();
    app.execute("calendar.new", json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00"})).unwrap();
    // A second later, not two minutes: the change, then the days on screen.
    assert!(app.jmap_pim_tick_at(true, SECOND));
    let request = last(&asked);
    let changes = request.changes.clone().unwrap();
    assert_eq!((changes.ops.len(), changes.ops[0].kind, changes.account), (1, EventChangeKind::Create, id));
    assert!(!changes.ops[0].token.is_empty() && !request.asks.is_empty());
    // Made as V9: the download that has it is the same event, under the server's name.
    let op = changes.ops[0].clone();
    let lunch = RemoteEvent {
        id: "V9".into(),
        token: op.token.clone(),
        uid: op.token.clone(),
        title: "Lunch".into(),
        start: op.fields.time.unwrap().start,
        end: op.fields.time.unwrap().end,
        is_organizer: Some(true),
        ..Default::default()
    };
    let done = EventOutcome { op, result: EventResult::Done, remote: "V9".into(), ..Default::default() };
    answer(&mut app, &asked, wrote(id, vec![done], vec![lunch]));
    let made: Vec<_> = app.session.mailbox.events.values().filter(|e| e.title == "Lunch").collect();
    assert_eq!((made.len(), made[0].origin.id.as_str()), (1, "V9"));
    assert!(app.session.mailbox.event_changes.is_empty() && app.session.mailbox.event_checks.is_empty());
}

#[test]
fn the_same_changes_are_not_sent_again_within_half_a_minute() {
    let (mut app, id, asked) = writable_window();
    app.execute("calendar.new", json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00"})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    // The job came back without a word of the change (stopped on its way): it waits, and is not sent again at once.
    answer(
        &mut app,
        &asked,
        JmapPimResult {
            account: id,
            email: ME.into(),
            offers: Some(PimOffers { calendars: true, contacts: true }),
            stopped: true,
            ..Default::default()
        },
    );
    assert_eq!(app.session.mailbox.event_changes.len(), 1);
    assert!(!app.jmap_pim_tick_at(true, 10.0 * SECOND));
    assert!(app.jmap_pim_tick_at(true, 32.0 * SECOND));
    assert!(last(&asked).changes.is_some(), "and then again");
}

#[test]
fn a_change_just_sent_and_not_seen_yet_brings_the_days_on_screen_every_ten_seconds() {
    let (mut app, id, asked) = writable_window();
    let budget = app.session.mailbox.events.values().find(|e| e.title == "Budget review").map(|e| e.id).unwrap();
    app.execute("calendar.edit", json!({"id": budget, "title": "Budget review (2027)"})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    let op = last(&asked).changes.unwrap().ops[0].clone();
    // Done, says the server; the download that came with it still has the old title.
    let old = RemoteEvent { id: "V1".into(), title: "Budget review".into(), start: NOW + 86_400, end: NOW + 90_000, ..Default::default() };
    answer(&mut app, &asked, wrote(id, vec![EventOutcome { op, result: EventResult::Done, remote: "V1".into(), ..Default::default() }], vec![old]));
    assert_eq!(app.session.mailbox.event_checks.len(), 1, "yet to be seen there");
    assert!(!app.jmap_pim_tick_at(true, 6.0 * SECOND));
    assert!(app.jmap_pim_tick_at(true, 12.0 * SECOND), "ten seconds on, not two minutes");
    assert!(last(&asked).changes.is_none(), "nothing more to send");
}

#[test]
fn a_job_that_carried_changes_and_says_nothing_of_them_came_to_nothing() {
    let (mut app, id, asked) = writable_window();
    app.execute("calendar.new", json!({"title": "Lunch", "start": "2026-10-15T12:00", "end": "2026-10-15T13:00"})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    answer(
        &mut app,
        &asked,
        JmapPimResult { account: id, email: ME.into(), offers: Some(PimOffers { calendars: true, contacts: true }), ..Default::default() },
    );
    assert_eq!(app.jmap_pim.failures.get(&id).map(|(n, _)| *n), Some(1));
}

// What is changed here of the contacts of a server's book goes with the next job too, before the contacts are read.

/// The window after its first job brought the team's book, which Ada may change, with Grace and Bob in it.
fn contacts_window() -> (MailApp, Id, Asked) {
    let (mut app, id, asked) = window();
    assert!(app.jmap_pim_tick_at(true, 0.0));
    let mut first = with_contacts(answered(id), id, "s1");
    for book in first.books.iter_mut().flat_map(|list| list.books.iter_mut()) {
        (book.read_only, book.may_remove) = (false, true);
    }
    answer(&mut app, &asked, first);
    (app, id, asked)
}

fn grace(app: &MailApp) -> mailcraft_model::Contact {
    app.session.mailbox.contacts.values().find(|c| c.first == "Grace").map(|c| c.as_ref().clone()).unwrap()
}

/// What the host answers a job that wrote `outcomes` to the contacts and then read them, Grace with the job title
/// `title`, at the state `state`.
fn wrote_contacts(account: Id, outcomes: Vec<ContactOutcome>, title: &str, state: &str) -> JmapPimResult {
    let mut result = with_contacts(answered(account), account, state);
    for book in result.books.iter_mut().flat_map(|list| list.books.iter_mut()) {
        (book.read_only, book.may_remove) = (false, true);
    }
    for contact in result.contacts.iter_mut().flat_map(|download| download.contacts.iter_mut()).filter(|c| c.first == "Grace") {
        contact.title = title.into();
    }
    result.contacts_uploaded = Some(ContactsUploaded { account, email: ME.into(), outcomes, token_refused: false });
    result
}

#[test]
fn a_contact_changed_here_goes_at_once_and_what_the_server_said_is_taken_in_before_the_contacts() {
    let (mut app, id, asked) = contacts_window();
    let before = jobs(&asked);
    app.execute("people.edit", json!({"id": grace(&app).id, "title": "Engineer"})).unwrap();
    // A second later: the change, and the contacts after it, in one job.
    assert!(app.jmap_pim_tick_at(true, SECOND));
    assert_eq!(jobs(&asked), before + 1);
    let request = last(&asked);
    let cards = request.contact_changes.clone().unwrap();
    assert_eq!((cards.ops.len(), cards.ops[0].kind, cards.ops[0].remote.as_str(), cards.account), (1, ContactChangeKind::Update, "c-grace", id));
    assert!(request.contacts, "the contacts are read after the change went");
    // Done, and the read that came after it has the change.
    let done = ContactOutcome { op: cards.ops[0].clone(), result: EventResult::Done, remote: "c-grace".into(), ..Default::default() };
    answer(&mut app, &asked, wrote_contacts(id, vec![done], "Engineer", "s2"));
    assert_eq!(grace(&app).title, "Engineer");
    assert!(app.session.mailbox.contact_changes.is_empty());
    assert_eq!(app.jmap_contacts_state(id).map(|(state, _)| state), Some("s2".to_string()));
    assert!(!app.jmap_pim.failures.contains_key(&id));
}

#[test]
fn a_contact_change_the_server_refuses_is_put_back_and_said() {
    let (mut app, id, asked) = contacts_window();
    app.execute("people.edit", json!({"id": grace(&app).id, "title": "Engineer"})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    let op = last(&asked).contact_changes.unwrap().ops[0].clone();
    let refused = ContactOutcome { op, result: EventResult::Refused, error: "Your rights do not allow it.".into(), ..Default::default() };
    answer(&mut app, &asked, wrote_contacts(id, vec![refused], "", "s2"));
    assert_eq!(grace(&app).title, "", "as the server has it");
    let said = "The server wouldn't take the change to \"Grace Okafor\" (Your rights do not allow it.). It was put back as it was.";
    assert!(app.sync_problems_of(id).iter().any(|line| line == said), "{:?}", app.sync_problems_of(id));
    assert!(app.session.mailbox.contact_changes.is_empty());
}

#[test]
fn a_contact_change_that_couldnt_be_sent_waits_and_says_so_and_is_not_sent_again_at_once() {
    let (mut app, id, asked) = contacts_window();
    app.execute("people.delete", json!({"id": grace(&app).id})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    let op = last(&asked).contact_changes.unwrap().ops[0].clone();
    assert_eq!(op.kind, ContactChangeKind::Delete);
    let failed = ContactOutcome { op, result: EventResult::Failed, error: "the server failed".into(), retry_after: Some(60), ..Default::default() };
    // (Grace still on the server: she stays deleted here while her deletion waits.)
    answer(&mut app, &asked, wrote_contacts(id, vec![failed], "", "s2"));
    assert!(!app.session.mailbox.contacts.values().any(|c| c.first == "Grace"));
    assert_eq!(app.session.mailbox.contact_changes.len(), 1);
    assert!(app.sync_problems_of(id).iter().any(|line| line.starts_with("A contact change couldn't be sent yet and will be tried again")));
    // Held back as the server asked (a minute, by the session's clock, which stands still here): the days on screen are
    // asked for as ever, and the deletion doesn't go with them.
    assert!(app.jmap_pim_tick_at(true, 3.0 * MINUTE));
    assert!(last(&asked).contact_changes.is_none());
}

#[test]
fn a_job_that_carried_contact_changes_and_says_nothing_of_them_came_to_nothing() {
    let (mut app, id, asked) = contacts_window();
    app.execute("people.edit", json!({"id": grace(&app).id, "title": "Engineer"})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    assert!(last(&asked).contact_changes.is_some());
    answer(
        &mut app,
        &asked,
        JmapPimResult {
            account: id,
            email: ME.into(),
            offers: Some(PimOffers { calendars: true, contacts: true }),
            stopped: true,
            ..Default::default()
        },
    );
    assert_eq!(app.jmap_pim.failures.get(&id).map(|(n, _)| *n), Some(1));
    assert_eq!(app.session.mailbox.contact_changes.len(), 1, "it waits");
    // Not the very same again within half a minute.
    assert!(!app.jmap_pim_tick_at(true, 10.0 * SECOND));
    assert!(app.jmap_pim_tick_at(true, 32.0 * SECOND));
    assert!(last(&asked).contact_changes.is_some(), "and then again");
}

#[test]
fn a_new_contact_noted_as_on_its_way_is_not_once_the_job_ended_before_sending_it() {
    let (mut app, id, asked) = contacts_window();
    app.execute("people.new", json!({"first": "Gus", "email": "gus@example.org"})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    assert!(last(&asked).contact_changes.is_some_and(|cards| cards.ops.iter().any(|op| op.kind == ContactChangeKind::Create)));
    let on_its_way = |app: &MailApp| app.session.mailbox.contact_changes.iter().any(|change| change.base.is_some());
    assert!(on_its_way(&app), "noted as on its way while the job runs");
    // The calendar's part answered busy: the contacts were never written, and Gus was never sent.
    answer(
        &mut app,
        &asked,
        JmapPimResult {
            account: id,
            email: ME.into(),
            offers: Some(PimOffers { calendars: true, contacts: true }),
            busy_for: Some(30),
            ..Default::default()
        },
    );
    assert!(!on_its_way(&app), "nothing of him is on the server: nothing waits for him, and no look is made first");
    // A job that wrote the contacts and answered him keeps what the answer says.
    let (mut app, id, asked) = contacts_window();
    app.execute("people.new", json!({"first": "Gus", "email": "gus@example.org"})).unwrap();
    assert!(app.jmap_pim_tick_at(true, SECOND));
    let op = last(&asked).contact_changes.unwrap().ops[0].clone();
    let lost = ContactOutcome { op, result: EventResult::Failed, error: "the connection was lost".into(), ..Default::default() };
    let mut lost = serde_json::to_value(lost).unwrap();
    lost["made"] = json!("perhaps");
    answer(&mut app, &asked, wrote_contacts(id, vec![serde_json::from_value(lost).unwrap()], "", "s2"));
    assert!(on_its_way(&app));
}
