// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What was made, changed, deleted and answered here of an account's calendars, written to its server: the first part
//! of a job ([`super::sync_pim`]), before anything is read, so that what is read shows it ([`push`]).
//!
//! 1. **The server's copy first.** Every event a change, a deletion or an answer is about is read (a `CalendarEvent/get`
//!    of their ids): a change of its places or its people is made to the server's own map, an answer to the people as
//!    the server has them, and one the server no longer has is gone. **Nothing is written to an occurrence of a series
//!    before the series itself was read**: an event the server says is one (it has a `recurrenceId`), and one
//!    occurrence the engine knows by its series and its time, wait for the series to be read in the same job; a
//!    series the server no longer has is gone, one that can't be read now waits.
//! 2. **One occurrence known by its series and its time** is looked for in the series' calendars, each occurrence
//!    apart, in the minute either side of that time and of where the series moved it, when it did (a day either side
//!    for whole days): the one of that series, never the series itself, that says the series has it at that time, to
//!    the minute ([`pick_occurrence`]). None is gone, when the answer held everything there; one cut short (a busy
//!    day) leaves the change to be tried again. An event the server says is an occurrence (a `recurrenceId`) and that
//!    names no series is not written to. The occurrence is then changed, deleted or answered by its own id, and its
//!    time is written on the clock of its series, with no zone (the server keeps the series').
//! 3. **The calls.** Each change is a create, an update of the whole properties it names, or a destroy. Those that
//!    make the server write to people go in calls of their own, with `sendSchedulingMessages`: a new meeting the
//!    person organizes, a change the people of one would see, its deletion when it is called off, and an answer the
//!    person asked to be sent, each only with the person's yes to that mail (`EventOp::send`, which the engine notes
//!    when it asks), whoever the server's copy says is invited. So one refusal never mails anybody about another
//!    change. A change that writes to people and that the server's copy says already (one sent again after its answer
//!    was lost) is done without being written: nobody is written to about it twice. At most [`MAX_PER_CALL`] events in
//!    a call, and as many as the session takes in a `set`.
//! 4. **What such a server doesn't keep is not written** ([`write`](super::write)): a change of nothing else, a
//!    forward, and another time proposed with an answer, are refused with the reason.
//! 5. **Outcomes**, one for each change tried, in the order they came: done (a new event with the server's id and
//!    uid; one occurrence with its own id, which names it from now on), gone, refused with the server's words, or
//!    failed with when to try again (a busy server, a server that failed, a connection that broke, a change not
//!    reached). A server that refuses the credentials ends the job: what it had not answered waits as it was.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;
use mailcraft_model::remote::{EventChangeKind, EventOp, EventOutcome, EventResult, EventsUploaded, MAX_EVENT_OPS};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};
use mailcraft_time::{DateTime, Timestamp, Zone};
use serde_json::{Map, Value, json};

use super::write::{self, Clock, Copy, KEEPS_NO};
use super::{ITEMS_BYTES, Stop, local_date_time, said, secrets_of, utc_text};
use crate::{CALENDARS, CORE, Client, Error, Http, Secrets, result_of, valid_id};

/// Most events in one `CalendarEvent/set`: what such a server writes in one call (the rest it answers `rateLimit`).
pub const MAX_PER_CALL: usize = 50;
/// Events read in one `get`.
const PAGE: usize = 200;
/// What every request names in `using`.
const USING: [&str; 2] = [CORE, CALENDARS];
/// What the server's copy of an event is read for.
/// (Everything a change writes is among them, so that one that writes to people is told from one the copy says already.)
const COPY_PROPERTIES: [&str; 22] = [
    "id",
    "uid",
    "calendarIds",
    "alerts",
    "title",
    "description",
    "freeBusyStatus",
    "privacy",
    "recurrenceRule",
    "start",
    "timeZone",
    "duration",
    "showWithoutTime",
    "utcStart",
    "recurrenceId",
    "recurrenceIdTimeZone",
    "baseEventId",
    "recurrenceOverrides",
    "locations",
    "participants",
    "organizerCalendarAddress",
    "isDraft",
];
/// How near the time an occurrence is asked for its own has to be, in seconds.
const NEAR: i64 = 60;
/// How long a server that failed is left alone before a change goes again, when it doesn't say, in seconds.
const RETRY: u32 = 60;
/// The longest words of the server's kept, in characters.
const MAX_WORDS: usize = 300;
/// Why a change of an event that says it is one occurrence of a series and names no series is not written.
const NO_SERIES: &str = "the server says the event is one occurrence of a series, and doesn't say of which";
/// Why a change of one occurrence that a crowded answer didn't hold waits.
const CROWDED: &str = "the server has too many events around that time to find the occurrence among them just now";
/// Why a change of who is invited to an event with more people than are read is not written.
const TOO_MANY_PEOPLE: &str = "the server has more than 1,000 people for this event, more than are rewritten from here";

/// Write what `request.changes` says to the account's calendars (in `account` of the session) and put what came of
/// each change into `out.uploaded`. Whether the job may go on to read: not after a stop, a refusal of the credentials
/// or a busy server, which `out` then says.
pub(super) async fn push<H: Http>(client: &Client<H>, request: &JmapPimRequest, account: &str, cancel: &AtomicBool, out: &mut JmapPimResult) -> bool {
    let Some(upload) = request.changes.as_ref().filter(|upload| !upload.ops.is_empty()) else { return true };
    let ops: Vec<EventOp> = upload
        .ops
        .iter()
        .take(MAX_EVENT_OPS)
        .map(|op| {
            let mut op = op.clone();
            op.tidy();
            op
        })
        .collect();
    let mut me: Vec<String> = std::iter::once(&request.email).chain(&request.me).map(|e| e.trim().to_string()).filter(|e| e.contains('@')).collect();
    me.dedup();
    let shown = Zone::named(&upload.zone).or_else(|| Zone::named(&request.zone)).unwrap_or(Zone::UTC);
    let mut writer = Writer {
        client,
        account,
        cancel,
        secrets: secrets_of(request, &client.secrets, account),
        shown,
        me,
        outcomes: vec![None; ops.len()],
        ops,
        copies: HashMap::new(),
        gone: HashSet::new(),
        unread: HashMap::new(),
    };
    let stop = writer.run().await.err();
    let token_refused = matches!(stop, Some(Stop::Auth));
    let going = matches!(stop, None | Some(Stop::Failed(_)));
    match stop {
        None | Some(Stop::Failed(_)) => {}
        Some(Stop::Cancelled) => out.stopped = true,
        Some(Stop::Auth) => out.auth_refused = true,
        Some(Stop::Busy(seconds, _)) => out.busy_for = Some(seconds),
    }
    let outcomes: Vec<EventOutcome> = writer.outcomes.into_iter().flatten().collect();
    log::debug!("jmap pim: {} of {} change(s) answered", outcomes.len(), upload.ops.len().min(MAX_EVENT_OPS));
    out.uploaded = Some(EventsUploaded { account: request.account, email: request.email.clone(), outcomes, token_refused });
    going
}

/// One change made ready for a call: what it is about on the server, and how it is written.
struct Ready {
    /// Its place among the changes.
    at: usize,
    /// What the server calls the event it writes to: empty for a new one.
    target: String,
    /// The event (a create's or an update's), or nothing (a destroy's).
    body: Option<Value>,
    /// The server is to write to people about it.
    scheduling: bool,
    /// The zone its time was written in, when one was.
    zone: String,
}

/// The writes of one job.
struct Writer<'a, H> {
    client: &'a Client<H>,
    account: &'a str,
    cancel: &'a AtomicBool,
    secrets: Secrets,
    /// The zone the calendar is shown in, where an event of no zone of its own is written.
    shown: Zone,
    /// The person's addresses.
    me: Vec<String>,
    ops: Vec<EventOp>,
    /// What became of each change, by its place.
    outcomes: Vec<Option<EventOutcome>>,
    /// The server's copies read, by their ids; the ids it said it doesn't have; and why others couldn't be read.
    copies: HashMap<String, Copy>,
    gone: HashSet<String>,
    unread: HashMap<String, String>,
}

impl<H: Http> Writer<'_, H> {
    fn check(&self) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) { Err(Stop::Cancelled) } else { Ok(()) }
    }

    /// Say what became of the change at `at`, unless something was said already.
    fn answer(&mut self, at: usize, result: EventResult, error: &str, retry_after: Option<u32>) {
        let Some(op) = self.ops.get(at) else { return };
        if let Some(slot) = self.outcomes.get_mut(at).filter(|slot| slot.is_none()) {
            let error = crate::tidy(error, MAX_WORDS);
            *slot = Some(EventOutcome { op: op.clone(), result, error, retry_after, ..Default::default() });
        }
    }

    /// Every change not answered yet: failed, to be tried again (after `retry_after`, or [`RETRY`]).
    fn fail_the_rest(&mut self, why: &str, retry_after: Option<u32>) {
        for at in 0..self.ops.len() {
            self.answer(at, EventResult::Failed, why, Some(retry_after.unwrap_or(RETRY)));
        }
    }

    /// Why a request failed as a whole: the credentials refused, and a busy server, end the job (the changes not
    /// answered wait, or fail with the server's wait); anything else fails the changes `affected`.
    fn request_failed(&mut self, error: Error, affected: &[usize]) -> Result<(), Stop> {
        let words = said(&self.secrets, &error);
        match Stop::of(error, &self.secrets) {
            Stop::Auth => Err(Stop::Auth),
            Stop::Busy(seconds, why) => {
                self.fail_the_rest(&words, Some(seconds));
                Err(Stop::Busy(seconds, why))
            }
            _ => {
                for at in affected {
                    self.answer(*at, EventResult::Failed, &words, Some(RETRY));
                }
                Ok(())
            }
        }
    }

    async fn run(&mut self) -> Result<(), Stop> {
        // A change that says nothing, a forward, and what such a server doesn't keep, are not written.
        for at in 0..self.ops.len() {
            let Some(op) = self.ops.get(at) else { continue };
            let refusal = match op.kind {
                EventChangeKind::Forward => Some(format!("{KEEPS_NO}, and forwards no meetings")),
                EventChangeKind::Respond if op.proposed.is_some() => Some(format!("{KEEPS_NO}: answer without another time")),
                EventChangeKind::Create if op.token.is_empty() || op.fields.time.is_none() => {
                    Some("the event has no mark or no time to be made with".into())
                }
                EventChangeKind::Update if op.fields.is_empty() => Some("the change says nothing that could be sent".into()),
                _ => None,
            };
            if let Some(why) = refusal {
                self.answer(at, EventResult::Refused, &why, None);
            }
        }
        // The server's copies: of what is changed, and of the series of an occurrence known by its time.
        let first: Vec<String> = (0..self.ops.len())
            .filter(|at| self.outcomes.get(*at).is_some_and(Option::is_none))
            .filter_map(|at| self.ops.get(at))
            .filter_map(|op| match op.kind {
                EventChangeKind::Create => None,
                _ if op.of_occurrence() => Some(op.series.clone()),
                _ => Some(op.remote.clone()),
            })
            .filter(|id| valid_id(id))
            .collect();
        self.read_copies(first).await?;
        // The series of the occurrences among them, read before anything is written to those.
        let series: Vec<String> = self
            .copies
            .values()
            .filter(|copy| copy.get("recurrenceId").is_some_and(|at| !at.is_null()))
            .filter_map(|copy| copy.get("baseEventId").and_then(Value::as_str).map(str::to_string))
            .collect();
        self.read_copies(series).await?;
        // Each change made ready, or answered.
        let mut ready: Vec<Ready> = Vec::new();
        for at in 0..self.ops.len() {
            if self.outcomes.get(at).is_some_and(Option::is_some) {
                continue;
            }
            self.check()?;
            if let Some(one) = self.prepare(at).await? {
                ready.push(one);
            }
        }
        // The calls: the silent ones first, then those that make the server write to people.
        let (loud, quiet): (Vec<Ready>, Vec<Ready>) = ready.into_iter().partition(|one| one.scheduling);
        for (group, scheduling) in [(quiet, false), (loud, true)] {
            for call in calls(group, MAX_PER_CALL.min(self.client.max_objects_in_set().max(1))) {
                self.check()?;
                self.set(call, scheduling).await?;
            }
        }
        Ok(())
    }

    /// Read the server's copies of `ids` (those not read yet), [`PAGE`] at a time.
    async fn read_copies(&mut self, ids: Vec<String>) -> Result<(), Stop> {
        let mut wanted: Vec<String> = Vec::new();
        for id in ids {
            if valid_id(&id) && !self.copies.contains_key(&id) && !self.gone.contains(&id) && !self.unread.contains_key(&id) && !wanted.contains(&id)
            {
                wanted.push(id);
            }
        }
        let size = PAGE.min(self.client.max_objects_in_get().max(1));
        for chunk in wanted.chunks(size) {
            self.check()?;
            let get = json!({"accountId": self.account, "ids": chunk, "properties": COPY_PROPERTIES});
            let answers = self.client.call_within(&USING, vec![("CalendarEvent/get", get)], ITEMS_BYTES).await;
            let mut answers = match answers {
                Ok(answers) => answers,
                Err(error) => {
                    let words = said(&self.secrets, &error);
                    let affected = self.waiting_on(chunk);
                    self.request_failed(error, &affected)?;
                    self.unread.extend(chunk.iter().map(|id| (id.clone(), words.clone())));
                    continue;
                }
            };
            match answers.take(0) {
                Ok(answer) => {
                    let listed = answer.get("list").and_then(Value::as_array).cloned().unwrap_or_default();
                    for copy in listed.into_iter().take(chunk.len()).filter_map(|copy| match copy {
                        Value::Object(copy) => Some(copy),
                        _ => None,
                    }) {
                        let id = copy.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                        if chunk.contains(&id) {
                            self.copies.insert(id, copy);
                        }
                    }
                    let gone: HashSet<&str> =
                        answer.get("notFound").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
                    for id in chunk {
                        if gone.contains(id.as_str()) && !self.copies.contains_key(id) {
                            self.gone.insert(id.clone());
                        } else if !self.copies.contains_key(id) {
                            self.unread.insert(id.clone(), "the server's answer did not have it".into());
                        }
                    }
                }
                Err(error) => {
                    let words = said(&self.secrets, &error);
                    self.unread.extend(chunk.iter().map(|id| (id.clone(), words.clone())));
                }
            }
        }
        Ok(())
    }

    /// The changes not answered yet whose copy is among `ids`.
    fn waiting_on(&self, ids: &[String]) -> Vec<usize> {
        (0..self.ops.len())
            .filter(|at| self.outcomes.get(*at).is_some_and(Option::is_none))
            .filter(|at| self.ops.get(*at).is_some_and(|op| ids.contains(&op.remote) || ids.contains(&op.series)))
            .collect()
    }

    /// What became of reading the copy `id`: there (`Ok`), or the outcome its change gets (gone, or failed).
    fn copy_of(&self, id: &str) -> Result<Copy, (EventResult, String)> {
        if let Some(copy) = self.copies.get(id) {
            return Ok(copy.clone());
        }
        if self.gone.contains(id) {
            return Err((EventResult::Gone, "the server no longer has the event".into()));
        }
        let why = self.unread.get(id).cloned().unwrap_or_else(|| "the server's copy of it couldn't be read".into());
        Err((EventResult::Failed, why))
    }

    /// The change at `at` made ready for a call, or answered (`None`).
    async fn prepare(&mut self, at: usize) -> Result<Option<Ready>, Stop> {
        let Some(op) = self.ops.get(at).cloned() else { return Ok(None) };
        if op.kind == EventChangeKind::Create {
            let Some((body, invites, zone)) = write::create(&op, Clock::Own(self.shown)) else {
                self.answer(at, EventResult::Refused, "the event has no time to be made with", None);
                return Ok(None);
            };
            // (Invitations only with the person's yes: `op.send`.)
            return Ok(Some(Ready { at, target: String::new(), body: Some(body), scheduling: invites && op.send, zone }));
        }
        // The event on the server, and (for an occurrence) its series.
        let (target, copy, series) = if op.of_occurrence() {
            let series = match self.copy_of(&op.series) {
                Ok(series) => series,
                Err((result, why)) => {
                    let why = if result == EventResult::Gone { "the server no longer has the series".to_string() } else { why };
                    self.answer(at, result, &why, (result == EventResult::Failed).then_some(RETRY));
                    return Ok(None);
                }
            };
            match self.occurrence(at, &op, &series).await? {
                Some((id, copy)) => (id, copy, Some(series)),
                None => {
                    self.answer(at, EventResult::Gone, "the series no longer has that occurrence", None);
                    return Ok(None);
                }
            }
        } else {
            match self.copy_of(&op.remote) {
                Ok(copy) => {
                    // An occurrence of a series: written to once its series was read. One that names no series it can
                    // be told by is not written to at all: it would be written as an event of its own.
                    let recurring = copy.get("recurrenceId").is_some_and(|at| !at.is_null());
                    let base = copy.get("baseEventId").and_then(Value::as_str);
                    if recurring && !base.is_some_and(valid_id) {
                        self.answer(at, EventResult::Refused, NO_SERIES, None);
                        return Ok(None);
                    }
                    let series = match base.filter(|_| recurring).map(|base| self.copy_of(base)) {
                        None => None,
                        Some(Ok(series)) => Some(series),
                        Some(Err((result, why))) => {
                            let why = if result == EventResult::Gone { "the server no longer has its series".to_string() } else { why };
                            self.answer(at, result, &why, (result == EventResult::Failed).then_some(RETRY));
                            return Ok(None);
                        }
                    };
                    (op.remote.clone(), copy, series)
                }
                Err((result, why)) => {
                    self.answer(at, result, &why, (result == EventResult::Failed).then_some(RETRY));
                    return Ok(None);
                }
            }
        };
        let (organizes, others, draft) = write::meeting(series.as_ref().unwrap_or(&copy), &self.me);
        // A meeting of the person's whose people are written to: only with the person's yes to that mail (`op.send`,
        // as the engine noted it), whatever the server's copy says of who is invited now.
        let meeting = organizes && others && !draft && op.send;
        // The clock its time is written on: an occurrence's is its series'.
        let clock = match &series {
            Some(series) => {
                let zone = |of: &Copy| of.get("timeZone").and_then(Value::as_str).and_then(Zone::named);
                Clock::Series(zone(&copy).or_else(|| zone(series)), self.shown)
            }
            None => Clock::Own(self.shown),
        };
        // A change of who is invited is made to the server's people whole: not to more of them than are read.
        if op.kind == EventChangeKind::Update && op.fields.attendees.is_some() && write::too_many_people(&copy) {
            self.answer(at, EventResult::Refused, TOO_MANY_PEOPLE, None);
            return Ok(None);
        }
        let ready = match op.kind {
            EventChangeKind::Delete => Ready { at, target, body: None, scheduling: op.cancel && meeting, zone: String::new() },
            EventChangeKind::Respond => match write::answer(&copy, op.response, &self.me) {
                Some(body) => Ready { at, target, body: Some(body), scheduling: op.send, zone: String::new() },
                None => {
                    self.answer(at, EventResult::Refused, "you are not among the people invited to it", None);
                    return Ok(None);
                }
            },
            _ => match write::patch(&op, Some(&copy), clock, &self.me) {
                Some((body, zone)) => Ready { at, target, body: Some(body), scheduling: meeting && write::seen_by_others(&op.fields), zone },
                None => {
                    self.answer(at, EventResult::Refused, KEEPS_NO, None);
                    return Ok(None);
                }
            },
        };
        // A change that writes to people and that the server's copy says already (its answer was lost on the way, and
        // it is sent again): done, without writing it, so that nobody is written to about it twice.
        if ready.scheduling && ready.body.as_ref().is_some_and(|body| says_already(&copy, body)) {
            self.done(at, ready.target, String::new(), ready.zone);
            return Ok(None);
        }
        Ok(Some(ready))
    }

    /// The occurrence of `series` (the server's copy of it) that the change `op` (at `at` among the changes) is about:
    /// its id and its copy, looked for around the time the series has it (see the module documentation). `None` when
    /// there is none, or when the change was answered already (it failed, and waits).
    async fn occurrence(&mut self, at: usize, op: &EventOp, series: &Copy) -> Result<Option<(String, Copy)>, Stop> {
        let shown = self.shown;
        let zone = series.get("timeZone").and_then(Value::as_str).and_then(Zone::named).unwrap_or(shown);
        let all_day = series.get("showWithoutTime").and_then(Value::as_bool) == Some(true);
        let when = op.occurrence;
        // Where the series moved it, when it did.
        let named = if all_day { DateTime::from_timestamp(when, 0) } else { zone.wall(when) };
        let moved = series
            .get("recurrenceOverrides")
            .and_then(Value::as_object)
            .and_then(|overrides| overrides.iter().find(|(key, _)| local_date_time(key) == Some(named)).map(|(_, patch)| patch))
            .and_then(|patch| patch.get("start"))
            .and_then(Value::as_str)
            .and_then(local_date_time)
            .map(|start| if all_day { start.to_timestamp(0) } else { zone.instant(start) });
        let (from, to) = (when.min(moved.unwrap_or(when)), when.max(moved.unwrap_or(when)));
        // Only what can hold it: the minute either side of those times (a whole day either side for whole days, whose
        // days are the person's), so that a busy day doesn't crowd it out of the answer.
        let near = if all_day { mailcraft_time::DAY } else { NEAR };
        let mut filter = json!({"after": utc_text(from.saturating_sub(near)), "before": utc_text(to.saturating_add(near).saturating_add(1))});
        let calendars: Vec<&String> = series
            .get("calendarIds")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(_, on)| on.as_bool() == Some(true))
            .map(|(id, _)| id)
            .collect();
        if !calendars.is_empty() {
            filter["inCalendars"] = json!(calendars);
        }
        let query = json!({"accountId": self.account, "filter": filter, "expandRecurrences": true, "limit": PAGE, "calculateTotal": true});
        let get = json!({"accountId": self.account, "#ids": result_of(0, "CalendarEvent/query", "/ids"), "properties": COPY_PROPERTIES});
        let answers = self.client.call_within(&USING, vec![("CalendarEvent/query", query), ("CalendarEvent/get", get)], ITEMS_BYTES).await;
        let mut answers = match answers {
            Ok(answers) => answers,
            // (The change is answered as failed: it waits.)
            Err(error) => {
                self.request_failed(error, &[at])?;
                return Ok(None);
            }
        };
        // Whether the query's answer is all there is around that time: when it is not, what isn't found is not gone.
        let whole = answers.take(0).is_ok_and(|found| {
            let listed = found.get("ids").and_then(Value::as_array).map_or(0, Vec::len);
            match found.get("total").and_then(Value::as_u64).and_then(|total| usize::try_from(total).ok()) {
                Some(total) => total <= listed,
                None => listed < PAGE,
            }
        });
        let listed = match answers.take(1) {
            Ok(answer) => answer.get("list").and_then(Value::as_array).cloned().unwrap_or_default(),
            Err(error) => {
                let words = said(&self.secrets, &error);
                self.answer(at, EventResult::Failed, &words, Some(RETRY));
                return Ok(None);
            }
        };
        let found = pick_occurrence(listed, &op.series, when, zone, all_day);
        if found.is_none() && !whole {
            self.answer(at, EventResult::Failed, CROWDED, Some(RETRY));
        }
        Ok(found)
    }

    /// One `CalendarEvent/set` of `call`, the server writing to people when `scheduling`, and its outcomes.
    async fn set(&mut self, call: Vec<Ready>, scheduling: bool) -> Result<(), Stop> {
        let (mut create, mut update, mut destroy) = (Map::new(), Map::new(), Vec::new());
        for one in &call {
            match (&one.body, self.ops.get(one.at).map(|op| op.kind)) {
                (Some(body), Some(EventChangeKind::Create)) => {
                    create.insert(format!("k{}", one.at), body.clone());
                }
                (Some(body), _) => {
                    update.insert(one.target.clone(), body.clone());
                }
                (None, _) => destroy.push(json!(one.target)),
            }
        }
        let mut args = json!({"accountId": self.account});
        if !create.is_empty() {
            args["create"] = Value::Object(create);
        }
        if !update.is_empty() {
            args["update"] = Value::Object(update);
        }
        if !destroy.is_empty() {
            args["destroy"] = Value::Array(destroy);
        }
        if scheduling {
            args["sendSchedulingMessages"] = json!(true);
        }
        let affected: Vec<usize> = call.iter().map(|one| one.at).collect();
        let mut answers = match self.client.call_within(&USING, vec![("CalendarEvent/set", args)], super::IDS_BYTES).await {
            Ok(answers) => answers,
            // Whether it went is not known: it goes again (a new event with its mark, which makes no second one).
            Err(error) => return self.request_failed(error, &affected),
        };
        let answer = match answers.take(0) {
            Ok(answer) => answer,
            Err(error) => {
                let (result, retry) = match &error {
                    Error::Refused { kind, .. } if failed_for_now(kind) => (EventResult::Failed, Some(RETRY)),
                    Error::Refused { .. } => (EventResult::Refused, None),
                    _ => (EventResult::Failed, Some(RETRY)),
                };
                let words = said(&self.secrets, &error);
                for at in affected {
                    self.answer(at, result, &words, retry);
                }
                return Ok(());
            }
        };
        let map = |key: &str| answer.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
        let (created, not_created, updated, not_updated, not_destroyed) =
            (map("created"), map("notCreated"), map("updated"), map("notUpdated"), map("notDestroyed"));
        let destroyed: HashSet<String> =
            answer.get("destroyed").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        for one in call {
            let Some(op) = self.ops.get(one.at).cloned() else { continue };
            let key = format!("k{}", one.at);
            let (done, error) = match op.kind {
                EventChangeKind::Create => (created.get(&key).cloned(), not_created.get(&key).cloned()),
                _ if one.body.is_none() => (destroyed.contains(&one.target).then(|| json!({})), not_destroyed.get(&one.target).cloned()),
                _ => (updated.contains_key(&one.target).then(|| json!({})), not_updated.get(&one.target).cloned()),
            };
            match (done, error) {
                (Some(done), _) => {
                    let remote = match op.kind {
                        EventChangeKind::Create => done.get("id").and_then(Value::as_str).filter(|id| valid_id(id)).unwrap_or_default().to_string(),
                        _ => one.target.clone(),
                    };
                    let uid = done.get("uid").and_then(Value::as_str).unwrap_or_default().to_string();
                    self.done(one.at, remote, uid, one.zone);
                }
                (None, Some(error)) => self.refused(one.at, &op, &error, &one.zone),
                (None, None) => self.answer(one.at, EventResult::Failed, "the server's answer said nothing of it", Some(RETRY)),
            }
        }
        Ok(())
    }

    /// The change at `at` was done: the server's name for its event (a new one's, or one occurrence's), its uid.
    fn done(&mut self, at: usize, remote: String, uid: String, zone: String) {
        let Some(op) = self.ops.get(at) else { return };
        if let Some(slot) = self.outcomes.get_mut(at).filter(|slot| slot.is_none()) {
            let mut outcome = EventOutcome { op: op.clone(), result: EventResult::Done, remote, uid, status: 200, zone, ..Default::default() };
            outcome.tidy();
            *slot = Some(outcome);
        }
    }

    /// The server didn't do the change at `at`: `error` is its SetError (RFC 8620 §5.3).
    fn refused(&mut self, at: usize, op: &EventOp, error: &Value, zone: &str) {
        let kind = error.get("type").and_then(Value::as_str).unwrap_or_default();
        let words = error.get("description").and_then(Value::as_str).map(strip_tags).unwrap_or_default();
        let words = if words.trim().is_empty() { kind.to_string() } else { self.secrets.hide(&words) };
        // An event that is there already (a server that says so of one made before): that one.
        if op.kind == EventChangeKind::Create
            && kind == "alreadyExists"
            && let Some(existing) = error.get("existingId").and_then(Value::as_str).filter(|id| valid_id(id))
        {
            self.done(at, existing.to_string(), String::new(), zone.to_string());
            return;
        }
        let retry = error.get("retryAfter").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
        match kind {
            "notFound" if op.kind != EventChangeKind::Create => self.answer(at, EventResult::Gone, &words, None),
            kind if failed_for_now(kind) => self.answer(at, EventResult::Failed, &words, Some(retry.unwrap_or(RETRY))),
            _ => self.answer(at, EventResult::Refused, &words, None),
        }
    }
}

/// Of the events a query found around the time a series has an occurrence (`listed`), that occurrence: the one of the
/// series `series` (by its id) that says when the series has it (its `recurrenceId`, on the clock of its
/// `recurrenceIdTimeZone`, else of `zone`; for whole days, `all_day`, its day) within [`NEAR`] of `at`, the nearest. Never
/// the series itself (its own id, or no `recurrenceId`), and never one whose id names another series or another time
/// than its own, for a server that names an occurrence by its series and its start ([`named_occurrence`]).
fn pick_occurrence(listed: Vec<Value>, series: &str, at: Timestamp, zone: Zone, all_day: bool) -> Option<(String, Copy)> {
    let mut best: Option<(i64, String, Copy)> = None;
    for copy in listed.into_iter().filter_map(|copy| match copy {
        Value::Object(copy) => Some(copy),
        _ => None,
    }) {
        if copy.get("baseEventId").and_then(Value::as_str) != Some(series) {
            continue;
        }
        let id = copy.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        if !valid_id(&id) || id == series {
            continue;
        }
        let Some(recurrence) = copy.get("recurrenceId").and_then(Value::as_str) else { continue };
        if let Some((of, start)) = named_occurrence(&id)
            && (event_id_of(&of) != series || start != recurrence)
        {
            continue;
        }
        // When the series has it: its recurrence id on the series' clock.
        let Some(named) = local_date_time(recurrence) else { continue };
        let named_at = if all_day {
            named.to_timestamp(0)
        } else {
            copy.get("recurrenceIdTimeZone").and_then(Value::as_str).and_then(Zone::named).unwrap_or(zone).instant(named)
        };
        let off = named_at.saturating_sub(at).saturating_abs();
        if off <= NEAR && best.as_ref().is_none_or(|(least, ..)| off < *least) {
            best = Some((off, id, copy));
        }
    }
    best.map(|(_, id, copy)| (id, copy))
}

/// The series and the start an occurrence's id names, for a server that names an occurrence by them (whose store
/// renumbers occurrences): `O`, then `["<the series' id in the store>", "<its recurrence id>"]` as JSON, in URL-safe
/// base64 without padding. `None` for any other id.
fn named_occurrence(id: &str) -> Option<(String, String)> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(id.strip_prefix('O')?).ok()?;
    match serde_json::from_slice::<Value>(&bytes).ok()? {
        Value::Array(parts) if parts.len() == 2 => Some((parts.first()?.as_str()?.to_string(), parts.get(1)?.as_str()?.to_string())),
        _ => None,
    }
}

/// The id such a server gives an event of its store's `raw` id: `V` and that id, in URL-safe base64 without padding.
fn event_id_of(raw: &str) -> String {
    format!("V{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw))
}

/// Whether the server's copy of an event says everything `body` would write of it: each property as it is there (one
/// it doesn't have is one that is null).
fn says_already(copy: &Copy, body: &Value) -> bool {
    body.as_object().is_some_and(|body| !body.is_empty() && body.iter().all(|(key, value)| copy.get(key).unwrap_or(&Value::Null) == value))
}

/// Whether a server's refusal is one that asking again later may get past.
fn failed_for_now(kind: &str) -> bool {
    matches!(kind, "rateLimit" | "serverFail" | "serverUnavailable" | "serverPartialFail" | "tooManyRequests" | "overQuota")
}

/// Text from a server with its markup taken out.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match (c, in_tag) {
            ('<', _) => in_tag = true,
            ('>', true) => in_tag = false,
            (_, true) => {}
            (c, false) => out.push(c),
        }
    }
    out
}

/// The changes made ready, in calls of at most `size`, none of which writes twice to one event.
fn calls(ready: Vec<Ready>, size: usize) -> Vec<Vec<Ready>> {
    let mut calls: Vec<Vec<Ready>> = Vec::new();
    for one in ready {
        let fits = |call: &Vec<Ready>| call.len() < size && (one.target.is_empty() || !call.iter().any(|other| other.target == one.target));
        match calls.iter_mut().find(|call| fits(call)) {
            Some(call) => call.push(one),
            None => calls.push(vec![one]),
        }
    }
    calls
}

#[cfg(test)]
mod tests;
