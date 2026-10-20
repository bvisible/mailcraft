// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's calendar jobs in the window: a host that keeps every job for the test to answer by hand, when
//! jobs start and what they ask, one at a time, a job given up, a sender dropped, a refused token, a busy server, and
//! the order an answer is taken in. Invented people at `example.org`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use mailcraft_engine::{Module, Session};
use mailcraft_model::remote::{CalendarsDownload, EventsDownload, RemoteCalendar, RemoteEvent};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult, PimOffers};
use mailcraft_model::{Id, Mailbox};
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
    // Every calendar of the server is read-only until writing is carried: the engine refuses a new appointment.
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
