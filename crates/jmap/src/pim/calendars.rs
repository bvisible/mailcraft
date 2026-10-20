// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! An account's calendars and the events of a stretch of them, read for the engine: [`sync_pim`].
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

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::remote::{CalendarsDownload, EventsDownload, RemoteEvent};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult, MAX_PIM_ADDRESSES, MAX_PIM_ASKS, PimOffers};
use mailcraft_time::{Timestamp, Zone};
use serde_json::{Value, json};

use super::jscalendar::{self, Listed};
use super::{ITEMS_BYTES, Pager, WRITE_EVENTS, utc_text};
use crate::{CALENDARS, CORE, Client, Error, Http, Secrets, result_of, valid_id};

/// Events asked for whole in one request: a description may be long.
const PAGE: usize = 200;
/// Most occurrences one job reads, every calendar together.
pub const MAX_EVENTS: usize = 5_000;
/// The most a list of calendars may be, in bytes.
const LIST_BYTES: usize = 1 << 20;
/// How long a busy server is left alone when it does not say, in seconds.
const BUSY_FOR: u32 = 30;
/// Longest line of `errors`, in characters, and the length from which an account id is hidden in one.
const MAX_LINE: usize = 300;
const SECRET_ID: usize = 8;
/// Most times in a job that a `get` is halved to find the event that breaks it.
const MAX_SPLITS: usize = 32;
/// What every request names in `using`.
const USING: [&str; 2] = [CORE, CALENDARS];
/// What an event is read for.
const EVENT_PROPERTIES: [&str; 23] = [
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
];

/// Read the calendars of the account `request` is for, as it says (see the module documentation), and say what was
/// read in one answer. `client` is signed in already ([`Client::connect`]); a caller whose sign-in failed answers with
/// [`failed_pim`]. `cancel` is read before every request. A session that offers no calendars is asked nothing: the
/// answer says what it offers.
pub async fn sync_pim<H: Http>(client: &Client<H>, request: &JmapPimRequest, cancel: &AtomicBool) -> JmapPimResult {
    let offers = PimOffers { calendars: client.offers_calendars(), contacts: client.offers_contacts() };
    let mut out = JmapPimResult { account: request.account, email: request.email.clone(), offers: Some(offers), ..Default::default() };
    let Some(account) = client.calendars_account() else {
        log::debug!("jmap pim: the session offers no calendars");
        return out;
    };
    let job = Job::new(client, request, account, cancel);
    let mut got = Gathered::new();
    let stop = job.run(&mut got).await.err();
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
    job.finish(got, &mut out);
    log::debug!(
        "jmap pim: {} calendar(s) listed, {} download(s), {} event(s), {} line(s); stopped: {}, busy: {}, refused: {}",
        out.calendars.as_ref().map_or(0, |list| list.calendars.len()),
        usize::from(out.events.is_some()) + out.more_events.len(),
        out.event_count(),
        out.errors.len(),
        out.stopped,
        out.busy_for.is_some(),
        out.auth_refused
    );
    out
}

/// The answer to a job that could not begin: signing in to the server (`Client::connect`) failed with `error`. What
/// [`sync_pim`] would have said of the same trouble: the sign-in refused, the server busy, or a line.
pub fn failed_pim(request: &JmapPimRequest, error: &Error) -> JmapPimResult {
    let secrets = secrets_of(request, &Secrets::default(), "");
    let mut result = JmapPimResult { account: request.account, email: request.email.clone(), ..Default::default() };
    match error {
        Error::Auth(_) => result.auth_refused = true,
        Error::Busy { retry_after, .. } => {
            result.busy_for = Some(retry_after.unwrap_or(BUSY_FOR));
            result.errors.push(format!("Couldn't reach the calendar server: {}", said(&secrets, error)));
        }
        other => result.errors.push(format!("Couldn't reach the calendar server: {}", said(&secrets, other))),
    }
    result
}

/// Why the reading cannot go on.
enum Stop {
    /// The owner stopped the job.
    Cancelled,
    /// The server refused the credentials.
    Auth,
    /// The server is busy: for this many seconds, and what it said.
    Busy(u32, String),
    /// A request failed, or the server answered the query with an error.
    Failed(Error),
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
    /// The events read, in the order they came, each with the calendars it is in.
    events: Vec<(Vec<String>, RemoteEvent)>,
    seen: HashSet<String>,
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
        match error {
            Error::Auth(_) => Stop::Auth,
            Error::Busy { retry_after, .. } => Stop::Busy(retry_after.unwrap_or(BUSY_FOR), self.said(&error)),
            other => Stop::Failed(other),
        }
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
        Ok(())
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
                Ok(read) if got.seen.insert(read.event.id.clone()) => got.events.push((read.calendars, read.event)),
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
        // A calendar the list of this job no longer has gets no download: the list takes it away with its events, and a
        // download of it would make it again.
        let listed: Option<HashSet<&str>> = got.listed.as_ref().map(|listed| listed.iter().map(|c| c.calendar.id.as_str()).collect());
        wanted.retain(|(_, id)| listed.as_ref().is_none_or(|listed| listed.contains(id.as_str())));
        let mut downloads: BTreeMap<String, EventsDownload> = BTreeMap::new();
        for (name, id) in &wanted {
            let complete = whole;
            let download = EventsDownload {
                account: self.request.account,
                email: self.request.email.clone(),
                calendar: name.clone(),
                from: self.from,
                to: self.to,
                complete,
                ..Default::default()
            };
            downloads.insert(id.clone(), download);
        }
        out.errors.extend(lines(&got));
        let mut strays = 0usize;
        for (calendars, event) in got.events {
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
        for (_, download) in downloads {
            if download.calendar.is_empty() {
                out.events = Some(download);
            } else {
                out.more_events.push(download);
            }
        }
    }
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
    let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    let mut lines = Vec::new();
    if let Some(why) = &got.listing {
        lines.push(format!("Couldn't list the calendars on the server: {why}"));
    }
    if let Some(why) = &got.halted {
        lines.push(format!("Couldn't bring the calendar down from the server: {why}"));
    }
    if let (n @ 1.., Some(why)) = (got.unread.0, &got.unread.1) {
        lines.push(format!("Couldn't read {} of the calendar: {why}", plural(n, "event", "events")));
    }
    if got.passed_over > 0 {
        lines.push(format!(
            "{} from the server couldn't be read and {} left out.",
            plural(got.passed_over, "event", "events"),
            if got.passed_over == 1 { "was" } else { "were" }
        ));
    }
    lines
}

/// What must not show in a line: what the client hides, the account's address, the address of the session (with the
/// addresses in its query), and the account's id when it is long enough to be one of those.
fn secrets_of(request: &JmapPimRequest, client: &Secrets, account_id: &str) -> Secrets {
    let mut secrets = client.clone();
    secrets.add(&request.email);
    secrets.add_address(&request.session);
    for address in &request.me {
        secrets.add(address);
    }
    if account_id.len() >= SECRET_ID {
        secrets.add(account_id);
    }
    secrets
}

/// `error` as a line of at most [`MAX_LINE`] characters, the secrets hidden before it is cut, and any word that holds
/// an `@` (or the `%40` it is written as in an address) hidden after.
fn said(secrets: &Secrets, error: &Error) -> String {
    let cut = crate::tidy(&secrets.hide(&error.to_string()), MAX_LINE);
    cut.split(' ').map(|word| if word.contains('@') || word.contains("%40") { "***" } else { word }).collect::<Vec<&str>>().join(" ")
}

#[cfg(test)]
mod tests;
