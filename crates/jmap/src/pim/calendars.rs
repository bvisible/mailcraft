// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! An account's calendars and the events of a stretch of them, read for the engine: [`read`], the first part of a job
//! ([`super::sync_pim`]).
//!
//! **What is asked of the server**, in as few requests as it takes:
//!
//! 1. One request of three calls when the list and the events are both wanted: `Calendar/get` (every calendar),
//!    `CalendarEvent/query` (the stretch, `after` and `before` in UTC, each occurrence of a series apart, the first
//!    200, with the total) and `CalendarEvent/get` of what the query found, by a back-reference, for the properties
//!    the engine takes. The query names the calendars asked for (`inCalendars`) when all their names on the server
//!    are known; when one is not (the main calendar, before any list has said which it is), it asks for every
//!    calendar's, and what is not asked for is left out. A server that reads in the same request what its query
//!    found is not made to read it twice. A request that asks for no calendar asks for the list alone, and one that
//!    asks for neither is not made.
//! 2. Further pages, each a query from where the last ended and the `get` of what it found, in one request, until
//!    the query is read whole ([`Pager`](super::Pager)), up to [`MAX_EVENTS`] occurrences.
//! 3. The series themselves: those the occurrences read belong to (their `baseEventId`), and those the window asked
//!    about by name ([`CalendarAsk::series`](mailcraft_model::remote::CalendarAsk::series)), a `CalendarEvent/get` of
//!    their ids, 200 a request, up to [`MAX_SERIES`]. A series is what the engine keeps as one event that repeats, by
//!    its rule; its occurrences are told from it, and one it changes (moved, retitled, with other people) is an
//!    exception of its own. A series asked about by name that the server says it doesn't have is gone (the
//!    download's `gone`); one that can't be read leaves its occurrences as events of their own, and the downloads
//!    not complete.
//!
//! A server that takes fewer calls in a request is asked in more requests: the list alone, then the query and its
//! `get`, or the query alone and then the `get` of the ids it gave.
//!
//! **What comes back.** The list ([`CalendarsDownload`], complete when it was read): each calendar with its name,
//! its colour (`#rrggbb`, `#rgb` or a CSS name, else none), read-only unless the account may write all of its events
//! (`myRights.mayWriteAll`; one it may write only its own events in is read-only here, as the server will write only
//! where all may be written) and always while [`WRITE_EVENTS`](super::WRITE_EVENTS) is shut; the main one is the one
//! the server flags as the default, else the first the account may write, else the first; and each says whether its
//! server keeps only the basics of an event (`plain`): it does when the session says what an event keeps and that is
//! not its categories (`keywords`), or, when the session says nothing of it, when the request says such a server is
//! one of those ([`JmapPimRequest::plain_unless_said`]).
//!
//! Then one [`EventsDownload`] for each calendar asked for, even when it is empty, so that the engine removes what the
//! server no longer has there; the main calendar's as `events`, the others' as `more_events`. An event in several of
//! them goes to the download of the calendar with the smallest id and to no other, or two downloads would take it
//! from one another. A download is complete when the query was read whole, every event it found was read (or the
//! server said it was gone since), the stretch was not cut and the job was not stopped. A calendar asked for that the
//! list read in the same job no longer has gets no download: the list takes it away, with its events, and a download
//! of it would make it again. Categories, importance, response options, online meetings and proposed times are not
//! read (`has_categories` and `has_options` are false): they stay as they are here.
//!
//! **A meeting the person declined** (their own answer is no, and it is known to be somebody else's: its organizer is
//! named, is none of their addresses, and their own entry is not an owner's) is in no download, as it is for a server
//! that takes a meeting away on a no, where such a server keeps it with the answer: an event of its own; a series, gone
//! whole with every occurrence and exception of it (one the person accepted alone too), and named in the `gone` of its
//! calendar's download so that the engine drops its copy; and one occurrence declined alone (that day is left out of
//! a series the person accepted). Exceptions are told from the series as they came.
//!
//! **The stretch** is the request's, or, when it is longer than the server expands repeating events over in one query
//! (its `maxExpandedQueryDuration`), as much of it as fits, cut in proportion on either side of the time the window
//! asked at; a cut stretch is not read whole.
//!
//! **When something goes wrong.** The server refusing the credentials (401, 403) ends the job with `auth_refused`,
//! and one that is busy (429, 503) with `busy_for` (the seconds it asked for, or 30); a stop (`cancel`) with
//! `stopped`. A request that fails in another way, or a query the server answers with an error, ends the reading with
//! a line; what was read before it is handed over, in downloads that add and update and remove nothing. A list the
//! server answers with an error is a line, and the events of calendars whose names are known still come. A `get` that
//! fails is asked again in halves, by its ids, until the event that breaks it is found; that one is left out, and the
//! downloads are then not complete (nobody can say which calendar it was in). The lines name no event and no calendar,
//! and what a server says in them is scrubbed of the session's address, the account's, the credentials and any word
//! that holds an `@`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::Response;
use mailcraft_model::remote::{CalendarsDownload, EventsDownload, RemoteEvent, RemoteEventKind};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult, MAX_PIM_ADDRESSES, MAX_PIM_ASKS};
use mailcraft_time::{DateTime, Timestamp, Zone};
use serde_json::{Value, json};

use super::jscalendar::{self, Listed, Read};
use super::{ITEMS_BYTES, Pager, Stop, WRITE_EVENTS, counted, said, secrets_of, utc_text};
use crate::{CALENDARS, CORE, Client, Error, Http, Secrets, result_of, valid_id};

/// Events asked for whole in one request: a description may be long.
const PAGE: usize = 200;
/// Most occurrences one job reads, every calendar together, and most series.
pub const MAX_EVENTS: usize = 5_000;
pub const MAX_SERIES: usize = 500;
/// The most a list of calendars may be, in bytes.
const LIST_BYTES: usize = 1 << 20;
/// Most times in a job that a `get` is halved to find the event that breaks it.
const MAX_SPLITS: usize = 32;
/// What every request names in `using`.
const USING: [&str; 2] = [CORE, CALENDARS];
/// What an event is read for.
const EVENT_PROPERTIES: [&str; 28] = [
    "id",
    "uid",
    "calendarIds",
    "title",
    "description",
    "descriptionContentType",
    "locations",
    "start",
    "timeZone",
    "duration",
    "showWithoutTime",
    "utcStart",
    "utcEnd",
    "recurrenceId",
    "baseEventId",
    "status",
    "freeBusyStatus",
    "privacy",
    "participants",
    "organizerCalendarAddress",
    "alerts",
    "updated",
    "sequence",
    "recurrenceIdTimeZone",
    "recurrenceRule",
    "recurrenceRules",
    "excludedRecurrenceRules",
    "recurrenceOverrides",
];

/// Read the calendars of the account `request` is for, as it says (see the module documentation), into `out`; the
/// calendars are in the account `account` of the session. Whether the job may go on: not after a stop, a refusal of
/// the credentials or a busy server, which `out` then says.
pub(super) async fn read<H: Http>(client: &Client<H>, request: &JmapPimRequest, account: &str, cancel: &AtomicBool, out: &mut JmapPimResult) -> bool {
    let job = Job::new(client, request, account, cancel);
    let mut got = Gathered::new();
    let stop = job.run(&mut got).await.err();
    let going = matches!(stop, None | Some(Stop::Failed(_)));
    match stop {
        None => {}
        Some(Stop::Cancelled) => out.stopped = true,
        Some(Stop::Auth) => out.auth_refused = true,
        Some(Stop::Busy(seconds, why)) => {
            out.busy_for = Some(seconds);
            got.halted = Some(why);
        }
        Some(Stop::Failed(error)) => got.halted = Some(job.said(&error)),
    }
    job.finish(got, out);
    going
}

/// A calendar asked for: what the window calls it (empty for the main one) and its name on the server when known.
#[derive(Clone, Debug)]
struct Ask {
    name: String,
    id: Option<String>,
}

/// What the job has read so far.
struct Gathered {
    /// The calendars listed, when they were; `None` when the list was not asked for, or could not be read.
    listed: Option<Vec<Listed>>,
    /// Why the list could not be read, when it was asked for.
    listing: Option<String>,
    /// The events read, in the order they came, each with the calendars it is in; then the series.
    events: Vec<Read>,
    seen: HashSet<String>,
    /// The series the server says it doesn't have, of those asked for by name.
    gone: HashSet<String>,
    pager: Pager,
    /// Events the query found that could not be read (a `get` that failed, or an answer that left them out), and why
    /// the first could not.
    unread: (usize, Option<String>),
    /// Events of an answer that were no events (no id, no start, no end).
    passed_over: usize,
    /// The stretch was cut to what the server expands.
    cut: bool,
    /// Why the reading ended early, as a line.
    halted: Option<String>,
    /// How many times a `get` was halved.
    splits: usize,
}

impl Gathered {
    fn new() -> Gathered {
        Gathered {
            listed: None,
            listing: None,
            events: Vec::new(),
            seen: HashSet::new(),
            gone: HashSet::new(),
            pager: Pager::new(PAGE, MAX_EVENTS),
            unread: (0, None),
            passed_over: 0,
            cut: false,
            halted: None,
            splits: 0,
        }
    }
}

/// One job: the request, the client, and what is worked out once from them.
struct Job<'a, H> {
    client: &'a Client<H>,
    request: &'a JmapPimRequest,
    cancel: &'a AtomicBool,
    /// The account that holds the calendars.
    account: &'a str,
    asks: Vec<Ask>,
    /// The owner's addresses, and the zone the calendar is shown in.
    me: Vec<String>,
    zone: Option<Zone>,
    /// What an error line must not show.
    secrets: Secrets,
    /// The stretch that is read, and whether it is the request's, cut.
    from: Timestamp,
    to: Timestamp,
    cut: bool,
}

impl<'a, H: Http> Job<'a, H> {
    fn new(client: &'a Client<H>, request: &'a JmapPimRequest, account: &'a str, cancel: &'a AtomicBool) -> Job<'a, H> {
        let main = Some(request.main.clone()).filter(|id| valid_id(id));
        let mut asks: Vec<Ask> = Vec::new();
        for ask in &request.asks {
            let name = ask.calendar.trim();
            if (!name.is_empty() && !valid_id(name)) || asks.iter().any(|have| have.name == name) || asks.len() >= MAX_PIM_ASKS {
                continue;
            }
            let id = if name.is_empty() { main.clone() } else { Some(name.to_string()) };
            asks.push(Ask { name: name.to_string(), id });
        }
        let mut me: Vec<String> =
            std::iter::once(&request.email).chain(&request.me).map(|e| e.trim().to_string()).filter(|e| e.contains('@')).collect();
        me.truncate(MAX_PIM_ADDRESSES.saturating_add(1));
        let (from, to, cut) = stretch(request.from, request.to, request.now, client.calendars_span());
        Job {
            client,
            request,
            cancel,
            account,
            asks,
            me,
            zone: Zone::named(&request.zone),
            secrets: secrets_of(request, &client.secrets, account),
            from,
            to,
            cut,
        }
    }

    fn check(&self) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) { Err(Stop::Cancelled) } else { Ok(()) }
    }

    /// What `error` says, fit for a line.
    fn said(&self, error: &Error) -> String {
        said(&self.secrets, error)
    }

    /// Why a request failed as a whole: the credentials refused and a busy server end the job, anything else ends the
    /// reading with a line.
    fn call_stop(&self, error: Error) -> Stop {
        Stop::of(error, &self.secrets)
    }

    /// The whole job: the list and the first page, then the other pages.
    async fn run(&self, got: &mut Gathered) -> Result<(), Stop> {
        got.cut = self.cut;
        let list = self.request.lists || self.asks.iter().any(|ask| ask.id.is_none());
        let events = !self.asks.is_empty() && self.from < self.to;
        if !list && !events {
            return Ok(());
        }
        // The calendars asked for by name, when every one has a name; else every calendar's events are asked for.
        let named: Option<Vec<String>> = self.asks.iter().map(|ask| ask.id.clone()).collect();
        let mut first = true;
        while (first && list) || (events && got.pager.done().is_none()) {
            self.check()?;
            self.page(got, first && list, events, named.as_deref()).await?;
            first = false;
        }
        if events {
            self.masters(got).await?;
        }
        Ok(())
    }

    /// The series of the occurrences read and the ones asked about by name, read themselves by their ids (see the
    /// module documentation).
    async fn masters(&self, got: &mut Gathered) -> Result<(), Stop> {
        let mut wanted: Vec<String> = Vec::new();
        let named = self.request.asks.iter().flat_map(|ask| &ask.series).map(|series| series.trim());
        let of_occurrences = got.events.iter().filter(|read| read.event.kind == RemoteEventKind::Occurrence).map(|read| read.event.series.as_str());
        for series in of_occurrences.chain(named) {
            if wanted.len() >= MAX_SERIES {
                break;
            }
            if valid_id(series) && !got.seen.contains(series) && !wanted.iter().any(|have| have == series) {
                wanted.push(series.to_string());
            }
        }
        for chunk in wanted.chunks(PAGE) {
            self.check()?;
            let get = json!({"accountId": self.account, "ids": chunk, "properties": EVENT_PROPERTIES});
            let mut answers = self.client.call_within(&USING, vec![("CalendarEvent/get", get)], ITEMS_BYTES).await.map_err(|e| self.call_stop(e))?;
            match answers.take(0) {
                Ok(answer) => self.take_masters(got, chunk, &answer),
                Err(error) => self.unread(got, chunk.len(), self.said(&error)),
            }
        }
        Ok(())
    }

    /// The series of a `get`'s answer, of those asked for (`ids`): each once; those the server says it doesn't have,
    /// as gone; what it left out without saying it was gone (or what was no series), as unread.
    fn take_masters(&self, got: &mut Gathered, ids: &[String], answer: &Value) {
        let Some(list) = answer.get("list").and_then(Value::as_array) else {
            self.unread(got, ids.len(), "the answer has no list of events".into());
            return;
        };
        let asked: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut came: HashSet<String> = HashSet::new();
        for value in list.iter().take(ids.len()) {
            let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
            if !asked.contains(id) || came.contains(id) {
                continue;
            }
            match jscalendar::event_of(value, &self.me, self.zone) {
                Ok(read) if read.event.kind == RemoteEventKind::SeriesMaster && got.seen.insert(read.event.id.clone()) => {
                    came.insert(id.to_string());
                    got.events.push(read);
                }
                Ok(_) => log::debug!("jmap pim: what a series' id named was no series"),
                Err(missing) => {
                    log::debug!("jmap pim: a series of the server's answer was passed over (no {missing:?})");
                    got.passed_over = got.passed_over.saturating_add(1);
                }
            }
        }
        let gone: HashSet<&str> = answer.get("notFound").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        got.gone.extend(ids.iter().filter(|id| gone.contains(id.as_str()) && !came.contains(id.as_str())).cloned());
        let left_out = ids.iter().filter(|id| !came.contains(id.as_str()) && !gone.contains(id.as_str())).count();
        if left_out > 0 {
            self.unread(got, left_out, "the server's answer did not have the series".into());
        }
    }

    /// The arguments of the query for the next page.
    fn query(&self, pager: &Pager, named: Option<&[String]>) -> Value {
        let mut filter = json!({"after": utc_text(self.from), "before": utc_text(self.to)});
        if let Some(named) = named {
            filter["inCalendars"] = json!(named);
        }
        json!({
            "accountId": self.account,
            "filter": filter,
            "expandRecurrences": true,
            "position": pager.position(),
            "limit": pager.limit(),
            "calculateTotal": pager.position() == 0,
        })
    }

    /// One page: the list when `list`, then (when `events`) a query and the `get` of what it found, in as few requests
    /// as the server takes.
    async fn page(&self, got: &mut Gathered, list: bool, events: bool, named: Option<&[String]>) -> Result<(), Stop> {
        let list_call = || ("Calendar/get", json!({"accountId": self.account, "ids": null}));
        let query = self.query(&got.pager, named);
        let calls = self.client.max_calls_in_request();
        // Everything in one request, when the server takes it.
        if events && calls >= 2 && (!list || calls >= 3) {
            let mut sent: Vec<(&str, Value)> = Vec::new();
            if list {
                sent.push(list_call());
            }
            let at = sent.len();
            let get = json!({"accountId": self.account, "#ids": result_of(at, "CalendarEvent/query", "/ids"), "properties": EVENT_PROPERTIES});
            sent.push(("CalendarEvent/query", query));
            sent.push(("CalendarEvent/get", get));
            let mut answers = self.client.call_within(&USING, sent, ITEMS_BYTES).await.map_err(|e| self.call_stop(e))?;
            if list {
                self.take_list(got, answers.take(0));
            }
            let found = answers.take(at).map_err(Stop::Failed)?;
            let ids = got.pager.page(&found);
            return match answers.take(at.saturating_add(1)) {
                Ok(read) => {
                    self.take_events(got, &ids, &read);
                    Ok(())
                }
                // Something in these events breaks the call: they are asked for by their ids, in halves if need be.
                Err(_) => self.read_by_ids(got, ids).await,
            };
        }
        // A server that takes fewer calls: one thing at a time.
        if list {
            let mut answers = self.client.call_within(&USING, vec![list_call()], LIST_BYTES).await.map_err(|e| self.call_stop(e))?;
            self.take_list(got, answers.take(0));
        }
        if !events {
            return Ok(());
        }
        self.check()?;
        let mut answers =
            self.client.call_within(&USING, vec![("CalendarEvent/query", query)], super::IDS_BYTES).await.map_err(|e| self.call_stop(e))?;
        let found = answers.take(0).map_err(Stop::Failed)?;
        let ids = got.pager.page(&found);
        self.read_by_ids(got, ids).await
    }

    /// The list of calendars as it came, or why it didn't.
    fn take_list(&self, got: &mut Gathered, answer: crate::Result<Value>) {
        match answer {
            Ok(answer) => match answer.get("list").and_then(Value::as_array) {
                Some(list) => {
                    let mut listed: Vec<Listed> = Vec::new();
                    for calendar in list.iter().take(mailcraft_model::remote::MAX_REMOTE_CALENDARS).filter_map(jscalendar::calendar_of) {
                        if !listed.iter().any(|have| have.calendar.id == calendar.calendar.id) {
                            listed.push(calendar);
                        }
                    }
                    got.listed = Some(listed);
                }
                None => got.listing = Some("the answer has no list of calendars".into()),
            },
            Err(error) => got.listing = Some(self.said(&error)),
        }
    }

    /// The events of a `get`'s answer, of those asked for (`ids`): each once, and what it left out without saying it
    /// was gone counted as unread.
    fn take_events(&self, got: &mut Gathered, ids: &[String], answer: &Value) {
        let Some(list) = answer.get("list").and_then(Value::as_array) else {
            self.unread(got, ids.len(), "the answer has no list of events".into());
            return;
        };
        let asked: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let gone: HashSet<&str> = answer.get("notFound").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        let mut came: HashSet<String> = HashSet::new();
        for value in list.iter().take(ids.len()) {
            let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
            if !asked.contains(id) || came.contains(id) {
                continue;
            }
            came.insert(id.to_string());
            match jscalendar::event_of(value, &self.me, self.zone) {
                Ok(read) if got.seen.insert(read.event.id.clone()) => got.events.push(read),
                Ok(_) => {}
                Err(missing) => {
                    log::debug!("jmap pim: an event of the server's answer was passed over (no {missing:?})");
                    got.passed_over = got.passed_over.saturating_add(1);
                }
            }
        }
        let left_out = ids.iter().filter(|id| !came.contains(id.as_str()) && !gone.contains(id.as_str())).count();
        if left_out > 0 {
            self.unread(got, left_out, "the server's answer did not have them".into());
        }
    }

    fn unread(&self, got: &mut Gathered, count: usize, why: String) {
        got.unread.0 = got.unread.0.saturating_add(count);
        got.unread.1.get_or_insert(why);
    }

    /// `ids` asked for by themselves, a page at a time and in halves when a call for them fails, until the event that
    /// breaks it is found: that one is left out, the rest are read.
    async fn read_by_ids(&self, got: &mut Gathered, ids: Vec<String>) -> Result<(), Stop> {
        let mut work: Vec<Vec<String>> = ids.chunks(PAGE).rev().map(<[String]>::to_vec).collect();
        while let Some(chunk) = work.pop() {
            self.check()?;
            let get = json!({"accountId": self.account, "ids": chunk, "properties": EVENT_PROPERTIES});
            let mut answers = self.client.call_within(&USING, vec![("CalendarEvent/get", get)], ITEMS_BYTES).await.map_err(|e| self.call_stop(e))?;
            match answers.take(0) {
                Ok(answer) => self.take_events(got, &chunk, &answer),
                Err(_) if chunk.len() > 1 && got.splits < MAX_SPLITS => {
                    got.splits = got.splits.saturating_add(1);
                    let (newer, older) = chunk.split_at(chunk.len() / 2);
                    work.push(older.to_vec());
                    work.push(newer.to_vec());
                }
                Err(error) if chunk.len() == 1 => self.unread(got, 1, self.said(&error)),
                // The server keeps failing: what is left waits for the next job.
                Err(error) => {
                    let left = chunk.len().saturating_add(work.iter().map(Vec::len).sum::<usize>());
                    self.unread(got, left, self.said(&error));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// The answer, from what was read.
    fn finish(&self, got: Gathered, out: &mut JmapPimResult) {
        // The main calendar: the list's, when it was read, else the one the request names.
        let main_listed = got.listed.as_ref().and_then(|listed| main_of(listed)).map(|listed| listed.calendar.id.clone());
        let main = main_listed.or_else(|| Some(self.request.main.clone()).filter(|id| valid_id(id)));
        if let Some(listed) = &got.listed {
            let plain = match self.client.calendars_writable() {
                Some(written) => !written.iter().any(|property| property == "keywords"),
                None => self.request.plain_unless_said,
            };
            let calendars = listed
                .iter()
                .map(|listed| mailcraft_model::remote::RemoteCalendar {
                    read_only: !(listed.writable && WRITE_EVENTS),
                    main: main.as_deref() == Some(listed.calendar.id.as_str()),
                    plain,
                    ..listed.calendar.clone()
                })
                .collect();
            out.calendars = Some(CalendarsDownload { account: self.request.account, email: self.request.email.clone(), complete: true, calendars });
        }
        // The calendars asked for, each by its name on the server, the first ask of a name only.
        let mut wanted: Vec<(String, String)> = Vec::new();
        for ask in &self.asks {
            let id = if ask.name.is_empty() { main.clone() } else { ask.id.clone() };
            if let Some(id) = id.filter(|id| !wanted.iter().any(|(_, have)| have == id)) {
                wanted.push((ask.name.clone(), id));
            }
        }
        let whole = got.pager.done() == Some(true) && got.unread.0 == 0 && got.halted.is_none() && !got.cut && !self.cancel.load(Ordering::Relaxed);
        // The series the person declined (see `declined`), with the calendars they are in: gone whole, the occurrences
        // the person accepted alone with them.
        let declined_series: HashMap<String, Vec<String>> = got
            .events
            .iter()
            .filter(|read| read.event.kind == RemoteEventKind::SeriesMaster && declined(read))
            .map(|read| (read.event.id.clone(), read.calendars.clone()))
            .collect();
        // A calendar the list of this job no longer has gets no download: the list takes it away with its events, and a
        // download of it would make it again.
        let listed: Option<HashSet<&str>> = got.listed.as_ref().map(|listed| listed.iter().map(|c| c.calendar.id.as_str()).collect());
        wanted.retain(|(_, id)| listed.as_ref().is_none_or(|listed| listed.contains(id.as_str())));
        let mut downloads: BTreeMap<String, EventsDownload> = BTreeMap::new();
        for (name, id) in &wanted {
            let complete = whole;
            // The series asked about in it by name that the server says it doesn't have, or that the person declined.
            let gone: Vec<String> = self
                .request
                .asks
                .iter()
                .filter(|ask| ask.calendar.trim() == name.as_str())
                .flat_map(|ask| &ask.series)
                .filter(|series| got.gone.contains(series.trim()) || declined_series.contains_key(series.trim()))
                .map(|series| series.trim().to_string())
                .collect();
            let download = EventsDownload {
                account: self.request.account,
                email: self.request.email.clone(),
                calendar: name.clone(),
                from: self.from,
                to: self.to,
                complete,
                gone,
                ..Default::default()
            };
            downloads.insert(id.clone(), download);
        }
        // A declined series is gone from the download of its calendar too, asked about by name or not: the engine drops
        // its copy whole (it would keep it for an occurrence of it).
        for (name, calendars) in &declined_series {
            let home = calendars.iter().filter(|id| downloads.contains_key(id.as_str())).min().cloned();
            if let Some(download) = home.and_then(|home| downloads.get_mut(&home))
                && !download.gone.contains(name)
            {
                download.gone.push(name.clone());
            }
        }
        out.errors.extend(lines(&got));
        // An occurrence the series changes beyond leaving it out is an exception of its own: the series has it
        // otherwise than its rule says.
        let series: HashMap<String, (RemoteEvent, HashSet<DateTime>)> = got
            .events
            .iter()
            .filter(|read| read.event.kind == RemoteEventKind::SeriesMaster)
            .map(|read| (read.event.id.clone(), (read.event.clone(), read.changed.iter().copied().collect())))
            .collect();
        let (mut strays, mut left_out) = (0usize, 0usize);
        for read in got.events {
            // A meeting the person declined is out of the calendar: an event of its own, one occurrence, and a series
            // with every occurrence and exception of it, one the person accepted alone too.
            let of_declined = !read.event.series.is_empty() && declined_series.contains_key(&read.event.series);
            if declined(&read) || of_declined {
                left_out = left_out.saturating_add(1);
                continue;
            }
            let Read { calendars, mut event, recurrence_id, .. } = read;
            if event.kind == RemoteEventKind::Occurrence
                && let Some((master, changed)) = series.get(&event.series)
                && (recurrence_id.is_some_and(|at| changed.contains(&at)) || differs(&event, master))
            {
                event.kind = RemoteEventKind::Exception;
            }
            // The smallest id of the calendars asked for that it is in.
            let home = calendars.iter().filter(|id| downloads.contains_key(id.as_str())).min();
            match home.and_then(|id| downloads.get_mut(id.as_str())) {
                Some(download) => download.events.push(event),
                None => strays = strays.saturating_add(1),
            }
        }
        if strays > 0 {
            log::debug!("jmap pim: {strays} event(s) in no calendar that was asked for were left out");
        }
        if left_out > 0 {
            log::debug!("jmap pim: {left_out} event(s) the owner declined were left out");
        }
        for (_, download) in downloads {
            if download.calendar.is_empty() {
                out.events = Some(download);
            } else {
                out.more_events.push(download);
            }
        }
    }
}

/// Whether the person declined a meeting that is known to be somebody else's: their own answer is no, its organizer is
/// named and is none of their addresses, and their own entry is not an owner's (one they organize under an address the
/// request doesn't name). Such a meeting is out of their calendar, as it is for a server that takes it away on a no;
/// one whose organizer is not known stays.
fn declined(read: &Read) -> bool {
    read.event.response == Response::Declined && read.event.is_organizer == Some(false) && !read.owner_role
}

/// Whether an occurrence says otherwise than its series what an event is besides its time (what it is called,
/// where, what it says, how it shows, who comes and what the owner answered): it was changed on its own then,
/// whatever the series' overrides say.
fn differs(occurrence: &RemoteEvent, series: &RemoteEvent) -> bool {
    let people = |event: &RemoteEvent| {
        let mut people: Vec<(String, bool)> = event.attendees.iter().map(|who| (who.address.email.to_ascii_lowercase(), who.optional)).collect();
        people.sort();
        people
    };
    occurrence.title != series.title
        || occurrence.location != series.location
        || occurrence.description != series.description
        || occurrence.show_as != series.show_as
        || occurrence.private != series.private
        || occurrence.reminder_minutes != series.reminder_minutes
        || occurrence.response != series.response
        || people(occurrence) != people(series)
}

/// The main calendar of a list: the one the server flags as the default, else the first the account may write, else
/// the first.
fn main_of(listed: &[Listed]) -> Option<&Listed> {
    listed.iter().find(|c| c.default).or_else(|| listed.iter().find(|c| c.writable)).or_else(|| listed.first())
}

/// The stretch `from`..`to` as far as a server that expands `span` seconds at once reads it: all of it, or `span` of
/// it cut in proportion on either side of `now`. Whether it was cut.
fn stretch(from: Timestamp, to: Timestamp, now: Timestamp, span: i64) -> (Timestamp, Timestamp, bool) {
    let length = to.saturating_sub(from);
    if length <= span || span <= 0 {
        return (from, to, false);
    }
    let anchor = now.clamp(from, to);
    let before = i128::from(anchor.saturating_sub(from));
    let kept_before = i64::try_from(before.saturating_mul(i128::from(span)) / i128::from(length)).unwrap_or(0);
    let start = anchor.saturating_sub(kept_before);
    (start, start.saturating_add(span), true)
}

/// The lines of trouble worth telling the owner.
fn lines(got: &Gathered) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(why) = &got.listing {
        lines.push(format!("Couldn't list the calendars on the server: {why}"));
    }
    if let Some(why) = &got.halted {
        lines.push(format!("Couldn't bring the calendar down from the server: {why}"));
    }
    if let (n @ 1.., Some(why)) = (got.unread.0, &got.unread.1) {
        lines.push(format!("Couldn't read {} of the calendar: {why}", counted(n, "event", "events")));
    }
    if got.passed_over > 0 {
        lines.push(format!(
            "{} from the server couldn't be read and {} left out.",
            counted(got.passed_over, "event", "events"),
            if got.passed_over == 1 { "was" } else { "were" }
        ));
    }
    lines
}

#[cfg(test)]
mod tests;
