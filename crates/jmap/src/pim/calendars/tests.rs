// //// Neoffice — added file (no upstream equivalent)
//! A calendar job against the in-process server: what is asked, in how many requests, what comes back, and what
//! trouble comes to. Invented people at `example.org`.

use std::sync::atomic::AtomicBool;

use mailcraft_model::remote::{CalendarAsk, EventsDownload};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult, PimOffers};
use serde_json::json;

use super::*;
use crate::standin::{Calendar, Event, Pim, SESSION, StandIn, TOKEN};
use crate::{Auth, block_on};

const ME: &str = "ada@example.org";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;
const DAY: i64 = 86_400;

/// A server with three calendars (Ada's, which she may write and which is flagged the default; the team's, which she
/// may only read; and one where she may write only her own events) and a week of events in them.
fn server() -> StandIn {
    let server = StandIn::new();
    server.with(|d| {
        let pim = &mut d.pim;
        *pim = Pim::offered();
        pim.add_calendar(Calendar { flagged: true, color: Some("#2e8b57".into()), ..Calendar::new("ada", "Ada", true) });
        pim.add_calendar(Calendar { color: Some("teal".into()), ..Calendar::new("team", "Team", false) });
        pim.add_calendar(Calendar { write_own: true, ..Calendar::new("own", "Own", false) });
        pim.add_event(Event::timed("budget", "ada", "Budget review", "2026-10-15T09:00:00", Some("Europe/Zurich"), "PT1H"));
        pim.add_event(Event::timed("standup", "team", "Team stand-up", "2026-10-16T08:30:00", Some("Europe/Zurich"), "PT15M"));
        pim.add_event(Event::whole_days("fair", "ada", "Book fair", "2026-10-19", 2));
        pim.add_event(Event::timed("later", "ada", "Far ahead", "2027-03-01T10:00:00", Some("Europe/Zurich"), "PT1H"));
    });
    server
}

fn ask(calendar: &str) -> CalendarAsk {
    CalendarAsk { calendar: calendar.into(), series: Vec::new() }
}

/// A job for the days around today (a week back, two ahead), listing the calendars, of the main one.
fn request() -> JmapPimRequest {
    JmapPimRequest {
        account: 7,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        me: vec![ME.into()],
        zone: "Europe/Zurich".into(),
        now: NOW,
        from: NOW - 7 * DAY,
        to: NOW + 14 * DAY,
        lists: true,
        main: String::new(),
        asks: vec![ask("")],
        plain_unless_said: false,
    }
}

fn run(server: &StandIn, request: &JmapPimRequest) -> JmapPimResult {
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    block_on(sync_pim(&client, request, &AtomicBool::new(false)))
}

fn titles(download: &EventsDownload) -> Vec<String> {
    let mut titles: Vec<String> = download.events.iter().map(|e| e.title.clone()).collect();
    titles.sort();
    titles
}

fn more<'a>(result: &'a JmapPimResult, calendar: &str) -> &'a EventsDownload {
    result.more_events.iter().find(|d| d.calendar == calendar).unwrap_or_else(|| panic!("no download of {calendar}: {result:?}"))
}

#[test]
fn the_calendars_and_their_rights_are_read() {
    let result = run(&server(), &request());
    assert_eq!(result.offers, Some(PimOffers { calendars: true, contacts: true }));
    let list = result.calendars.as_ref().unwrap();
    assert!(list.complete && list.account == 7 && list.email == ME);
    let names: Vec<(&str, &str)> = list.calendars.iter().map(|c| (c.id.as_str(), c.name.as_str())).collect();
    assert_eq!(
        names,
        [(Pim::calendar_id("ada").as_str(), "Ada"), (Pim::calendar_id("team").as_str(), "Team"), (Pim::calendar_id("own").as_str(), "Own")]
    );
    assert_eq!(list.calendars[0].color, Some(mailcraft_model::Rgb(0x2E, 0x8B, 0x57)));
    assert_eq!(list.calendars[1].color, Some(mailcraft_model::Rgb(0x00, 0x80, 0x80)), "a name is served as #rrggbb");
    assert!(list.calendars.iter().all(|c| c.group.is_empty() && c.plain), "a server that writes no categories keeps only the basics");
    assert!(result.errors.is_empty() && !result.auth_refused && !result.stopped, "{result:?}");
}

#[test]
fn every_calendar_is_read_only_while_writes_are_not_carried() {
    let result = run(&server(), &request());
    assert!(result.calendars.unwrap().calendars.iter().all(|c| c.read_only), "the door stays shut");
}

#[test]
fn the_main_calendar_is_the_default_one_else_the_first_writable_else_the_first() {
    let main_of =
        |server: &StandIn| run(server, &request()).calendars.unwrap().calendars.into_iter().filter(|c| c.main).map(|c| c.name).collect::<Vec<_>>();
    let server = server();
    assert_eq!(main_of(&server), ["Ada"]);
    // The server serves one default: the first it flags among those Ada may write, else the first she may write.
    server.with(|d| d.pim.calendar_list.iter_mut().for_each(|c| c.flagged = false));
    server.with(|d| d.pim.calendar_list[1].flagged = true);
    assert_eq!(main_of(&server), ["Ada"], "a flag on one she can't write does not make it the default");
    // A server that says no default: the first she may write (writing only her own is not writing it), else the first.
    server.with(|d| d.pim.calendar_list.swap(0, 2));
    let listed = run(&server, &request()).calendars.unwrap();
    assert_eq!(listed.calendars.iter().filter(|c| c.main).count(), 1);
    server.with(|d| d.pim.calendar_list.iter_mut().for_each(|c| c.write_all = false));
    let none_writable = run(&server, &request()).calendars.unwrap();
    assert_eq!(none_writable.calendars.iter().filter(|c| c.main).map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Team"], "the flagged one, then");
}

#[test]
fn the_window_is_asked_in_one_request_of_three_calls() {
    let server = server();
    let result = run(&server, &request());
    let requests: Vec<_> = server.seen().into_iter().filter(|s| s.path == "/jmap/api").collect();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].calls, ["Calendar/get", "CalendarEvent/query", "CalendarEvent/get"]);
    // The query is for every calendar (the main one has no name here yet), the get of what it found by reference.
    let asked = server.asked();
    assert_eq!((asked[1].limit, asked[2].ids), (Some(200), Some(3)));
    assert!(asked[2].properties.iter().any(|p| p == "utcStart") && !asked[2].properties.iter().any(|p| p == "keywords"));
    // What is not asked for is left out: the team's stand-up is in a calendar the job did not ask about.
    let main = result.events.as_ref().unwrap();
    assert_eq!((main.calendar.as_str(), main.complete), ("", true));
    assert_eq!(titles(main), ["Book fair", "Budget review"]);
    assert!(result.more_events.is_empty());
}

#[test]
fn a_job_with_the_main_calendars_name_asks_by_name_and_without_the_list() {
    let server = server();
    let request = JmapPimRequest { lists: false, main: Pim::calendar_id("ada"), asks: vec![ask(""), ask(&Pim::calendar_id("team"))], ..request() };
    let result = run(&server, &request);
    let requests: Vec<_> = server.seen().into_iter().filter(|s| s.path == "/jmap/api").collect();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].calls, ["CalendarEvent/query", "CalendarEvent/get"]);
    assert!(result.calendars.is_none(), "no list was asked for");
    assert_eq!(titles(result.events.as_ref().unwrap()), ["Book fair", "Budget review"]);
    assert_eq!(titles(more(&result, &Pim::calendar_id("team"))), ["Team stand-up"]);
    assert!(more(&result, &Pim::calendar_id("team")).complete);
}

#[test]
fn only_the_asked_calendars_are_queried() {
    let server = server();
    let team = Pim::calendar_id("team");
    let request = JmapPimRequest { lists: false, main: Pim::calendar_id("ada"), asks: vec![ask(&team)], ..request() };
    let result = run(&server, &request);
    assert!(result.events.is_none(), "the main calendar was not asked for");
    assert_eq!(result.more_events.len(), 1);
    assert_eq!(titles(&result.more_events[0]), ["Team stand-up"]);
    // The query named the calendar: what it found, and the get read, is its one event.
    let asked = server.asked();
    assert_eq!((asked[1].method.as_str(), asked[1].ids), ("CalendarEvent/get", Some(1)), "{asked:?}");
}

#[test]
fn an_event_in_two_asked_calendars_goes_to_the_smaller_id_only() {
    let server = server();
    server.with(|d| d.pim.events[0].calendars = vec!["ada".into(), "team".into()]);
    let (ada, team) = (Pim::calendar_id("ada"), Pim::calendar_id("team"));
    let request = JmapPimRequest { lists: false, main: String::new(), asks: vec![ask(&ada), ask(&team)], ..request() };
    let result = run(&server, &request);
    let smaller = ada.clone().min(team.clone());
    let budget_in = |calendar: &str| more(&result, calendar).events.iter().any(|e| e.title == "Budget review");
    assert!(budget_in(&smaller));
    assert!(!budget_in(if smaller == ada { &team } else { &ada }), "never in both");
}

#[test]
fn an_asked_calendar_with_no_events_still_answers_an_empty_download() {
    let own = Pim::calendar_id("own");
    let request = JmapPimRequest { asks: vec![ask(""), ask(&own)], ..request() };
    let result = run(&server(), &request);
    let empty = more(&result, &own);
    assert!(empty.events.is_empty() && empty.complete, "{empty:?}");
    assert_eq!((empty.from, empty.to), (NOW - 7 * DAY, NOW + 14 * DAY));
}

#[test]
fn a_calendar_the_server_no_longer_lists_gets_no_download_and_the_list_takes_it_away() {
    // The window knows a calendar the server has since removed: the list leaves it out, and no download of it comes (the
    // engine would make the calendar again for one).
    let server = server();
    let gone = Pim::calendar_id("gone");
    let request = JmapPimRequest { main: Pim::calendar_id("ada"), asks: vec![ask(""), ask(&gone)], ..request() };
    let result = run(&server, &request);
    assert!(result.more_events.is_empty(), "{result:?}");
    assert!(result.events.as_ref().unwrap().complete);
    assert!(!result.calendars.unwrap().calendars.iter().any(|c| c.id == gone));
    // Without a list in the job, there is nothing to say it is gone: its download is what the server has there, nothing.
    let days = JmapPimRequest { lists: false, ..request };
    let result = run(&server, &days);
    assert!(more(&result, &gone).events.is_empty() && more(&result, &gone).complete);
}

#[test]
fn a_stretch_longer_than_the_servers_span_is_cut_in_proportion_and_is_not_whole() {
    let server = server();
    server.with(|d| d.pim.span = "P30D".into());
    // The wide stretch: a month back, a year ahead.
    let request = JmapPimRequest { from: NOW - 31 * DAY, to: NOW + 366 * DAY, ..request() };
    let result = run(&server, &request);
    let main = result.events.as_ref().unwrap();
    assert_eq!(main.to - main.from, 30 * DAY);
    // As much of it before now as the whole had, in proportion: 31 of 397 days.
    let before = NOW - main.from;
    assert_eq!(before, 31 * DAY * 30 / 397);
    assert!(!main.complete, "a cut stretch is not read whole");
    assert_eq!(titles(main), ["Book fair", "Budget review"], "what is in it still comes");
}

#[test]
fn pages_follow_from_position_and_gets_come_in_chunks() {
    let server = server();
    server.with(|d| {
        for n in 0..450 {
            let start = format!("2026-10-{:02}T{:02}:{:02}:00", 12 + n / 100, 7 + (n % 100) / 10, (n % 10) * 5);
            d.pim.add_event(Event::timed(&format!("e{n}"), "ada", &format!("Slot {n}"), &start, Some("Europe/Zurich"), "PT5M"));
        }
    });
    let result = run(&server, &JmapPimRequest { lists: false, main: Pim::calendar_id("ada"), ..request() });
    let main = result.events.as_ref().unwrap();
    assert_eq!(main.events.len(), 452, "the 450, the budget review and the fair");
    assert!(main.complete);
    let asked = server.asked();
    let queries: Vec<_> = asked.iter().filter(|a| a.method == "CalendarEvent/query").map(|a| a.limit).collect();
    let gets: Vec<_> = asked.iter().filter(|a| a.method == "CalendarEvent/get").map(|a| a.ids).collect();
    assert_eq!(queries, [Some(200), Some(200), Some(200)]);
    assert_eq!(gets, [Some(200), Some(200), Some(52)], "each page's events are read with it, by reference");
}

#[test]
fn a_server_that_takes_one_call_at_a_time_is_asked_in_turn() {
    let server = server();
    server.with(|d| d.max_calls = 1);
    let result = run(&server, &request());
    let calls: Vec<Vec<String>> = server.seen().into_iter().filter(|s| s.path == "/jmap/api").map(|s| s.calls).collect();
    assert_eq!(calls, [vec!["Calendar/get".to_string()], vec!["CalendarEvent/query".to_string()], vec!["CalendarEvent/get".to_string()]]);
    assert_eq!(titles(result.events.as_ref().unwrap()), ["Book fair", "Budget review"]);
    assert!(result.events.unwrap().complete);
}

#[test]
fn a_get_that_fails_makes_the_downloads_incomplete_and_the_other_events_still_come() {
    let server = server();
    let budget = Pim::event_id("budget");
    server.with(|d| {
        d.pim.broken_events.insert(budget.clone());
    });
    let request = JmapPimRequest { asks: vec![ask(""), ask(&Pim::calendar_id("team"))], main: Pim::calendar_id("ada"), ..request() };
    let result = run(&server, &request);
    let main = result.events.as_ref().unwrap();
    assert_eq!(titles(main), ["Book fair"], "the one that breaks the call is left out");
    assert_eq!(titles(more(&result, &Pim::calendar_id("team"))), ["Team stand-up"]);
    assert!(!main.complete && !more(&result, &Pim::calendar_id("team")).complete, "nobody can say which calendar it was in");
    assert_eq!(result.errors.len(), 1, "{:?}", result.errors);
    assert!(result.errors[0].starts_with("Couldn't read 1 event of the calendar"), "{:?}", result.errors);
}

#[test]
fn a_list_the_server_fails_on_is_a_line_and_the_named_calendars_still_come() {
    let server = server();
    server.with(|d| d.faults.push_back(crate::standin::Fault::on("Calendar/get", crate::standin::Effect::MethodError("serverFail"))));
    // The main calendar's name is known from an earlier list: its events come.
    let known = JmapPimRequest { main: Pim::calendar_id("ada"), ..request() };
    let result = run(&server, &known);
    assert!(result.calendars.is_none());
    assert_eq!(titles(result.events.as_ref().unwrap()), ["Book fair", "Budget review"]);
    assert!(result.errors.iter().any(|line| line.starts_with("Couldn't list the calendars on the server")), "{:?}", result.errors);
    // Not known: nothing can be said of the main calendar.
    server.with(|d| d.faults.push_back(crate::standin::Fault::on("Calendar/get", crate::standin::Effect::MethodError("serverFail"))));
    let result = run(&server, &request());
    assert!(result.events.is_none() && result.calendars.is_none());
}

#[test]
fn a_401_is_a_refused_token_and_not_a_failure() {
    let server = server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    server.with(|d| d.drop_token_after = Some(0));
    let result = block_on(sync_pim(&client, &request(), &AtomicBool::new(false)));
    assert!(result.auth_refused && result.errors.is_empty() && result.busy_for.is_none(), "{result:?}");
    // A job that could not sign in at all says the same.
    assert!(failed_pim(&request(), &Error::Auth("no".into())).auth_refused);
}

#[test]
fn a_busy_server_is_left_alone_for_the_time_it_says() {
    let server = server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    server.with(|d| d.refuse_next.push_back(Some((429, Some(120)))));
    let result = block_on(sync_pim(&client, &request(), &AtomicBool::new(false)));
    assert_eq!(result.busy_for, Some(120));
    assert!(!result.auth_refused && result.events.is_none() && result.calendars.is_none(), "{result:?}");
    // Without saying how long: half a minute.
    server.with(|d| d.refuse_next.push_back(Some((503, None))));
    assert_eq!(block_on(sync_pim(&client, &request(), &AtomicBool::new(false))).busy_for, Some(30));
    let failed = failed_pim(&request(), &Error::Busy { status: 429, retry_after: Some(60) });
    assert_eq!(failed.busy_for, Some(60));
}

#[test]
fn a_stopped_job_says_so_and_asks_nothing() {
    let server = server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let result = block_on(sync_pim(&client, &request(), &AtomicBool::new(true)));
    assert!(result.stopped && result.events.is_none(), "{result:?}");
    assert_eq!(server.api_requests(), 0);
}

#[test]
fn a_session_without_calendars_is_asked_nothing_and_says_what_it_offers() {
    let server = server();
    server.with(|d| d.pim.calendars = false);
    let result = run(&server, &request());
    assert_eq!(result.offers, Some(PimOffers { calendars: false, contacts: true }));
    assert!(result.calendars.is_none() && result.events.is_none() && result.errors.is_empty());
    assert_eq!(server.api_requests(), 0);
    // A job that asks for nothing asks nothing.
    let nothing = JmapPimRequest { lists: false, main: Pim::calendar_id("ada"), asks: Vec::new(), ..request() };
    let fresh = self::server();
    assert_eq!(run(&fresh, &nothing).offers.map(|o| o.calendars), Some(true));
    assert_eq!(fresh.api_requests(), 0);
}

#[test]
fn plain_comes_from_what_the_session_says_it_writes_or_from_the_request() {
    let server = server();
    server.with(|d| d.pim.says_writable = false);
    let plain = |request: &JmapPimRequest| run(&server, request).calendars.unwrap().calendars.iter().all(|c| c.plain);
    assert!(!plain(&request()), "unsaid: the server is taken to keep everything");
    assert!(plain(&JmapPimRequest { plain_unless_said: true, ..request() }), "unless the host knows it for one that keeps the basics");
}

#[test]
fn every_pim_request_names_core_and_its_capability() {
    let server = server();
    run(&server, &JmapPimRequest { asks: vec![ask(""), ask(&Pim::calendar_id("team"))], ..request() });
    // The stand-in refuses a method of calendars that `using` doesn't name: none was refused.
    let answers: Vec<u16> = server.seen().iter().filter(|s| s.path == "/jmap/api").map(|s| s.status).collect();
    assert!(!answers.is_empty() && answers.iter().all(|status| *status == 200));
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let refused = block_on(client.call(&[CORE], vec![("Calendar/get", json!({"accountId": crate::standin::ACCOUNT}))])).unwrap();
    assert!(matches!(refused.get(0), Err(Error::Refused { kind, .. }) if kind == "unknownMethod"));
}

#[test]
fn the_lines_name_no_address() {
    let server = server();
    server.with(|d| d.faults.push_back(crate::standin::Fault::on("CalendarEvent/query", crate::standin::Effect::MethodError("serverFail"))));
    let result = run(&server, &JmapPimRequest { session: format!("{SESSION}?account={ME}"), ..request() });
    assert!(!result.errors.is_empty());
    for line in &result.errors {
        assert!(!line.contains('@') && !line.contains("example.org"), "{line}");
    }
    let failed = failed_pim(&request(), &Error::Io(format!("can't reach {ME} at {SESSION}")));
    assert!(failed.errors.iter().all(|line| !line.contains('@') && !line.contains("standin.example")), "{:?}", failed.errors);
}

#[test]
fn the_cut_of_a_stretch_keeps_its_proportions_around_now() {
    assert_eq!(stretch(0, 100, 50, 200), (0, 100, false), "short enough");
    assert_eq!(stretch(0, 100, 50, 50), (25, 75, true));
    assert_eq!(stretch(0, 100, 0, 50), (0, 50, true), "now at the start keeps the start");
    assert_eq!(stretch(0, 100, 500, 50), (50, 100, true), "now after it keeps the end");
    assert_eq!(stretch(100, 0, 50, 50), (100, 0, false), "an empty stretch is left as it is");
}
