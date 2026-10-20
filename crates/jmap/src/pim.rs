// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! An account's calendars over JMAP (JMAP for Calendars, the IETF's draft on RFC 8620; events as JSCalendar,
//! RFC 8984) and its address books (JMAP for Contacts, RFC 9610; cards as JSContact, RFC 9553), read for the
//! engine: [`sync_pim`].
//!
//! A job is one of its own beside a Send/Receive: the window asks for it on a clock of its own, and what it reads
//! comes back in one answer ([`JmapPimResult`]), made of the downloads the engine already takes in for Microsoft
//! Graph's calendar and contacts ([`CalendarsDownload`](mailcraft_model::remote::CalendarsDownload),
//! [`EventsDownload`](mailcraft_model::remote::EventsDownload),
//! [`ContactsDownload`](mailcraft_model::remote::ContactsDownload)) and of the address books
//! ([`BooksDownload`](mailcraft_model::book::BooksDownload)). What is read and how is in [`calendars`] and
//! [`contacts`]; how an event in JSCalendar and a card in JSContact become the engine's is in `jscalendar.rs` and
//! `jscontact.rs`.
//!
//! This module holds what the readers share: ISO 8601 durations, the UTC and local times of JSCalendar,
//! the paging of a `/query` ([`query_all`]), the bounds of what is read, and how trouble ends a job and is said.
//!
//! **Nothing is written yet.** Until events go back to the server, every calendar is handed over as one the
//! account may only read ([`WRITE_EVENTS`]): the engine's own lock and its reason then keep the owner from
//! making, here, what would go nowhere. [`WRITE_CONTACTS`] is the same door for address books.

pub mod calendars;
pub mod contacts;
pub(crate) mod jscalendar;
pub(crate) mod jscontact;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use mailcraft_model::sync::{JmapPimRequest, JmapPimResult, PimOffers};
use mailcraft_time::{DateTime, Timestamp, format_iso_utc, parse_iso};
use serde_json::{Map, Value};

use crate::{Client, Error, Http, Result, Secrets, valid_id};

/// How long a busy server is left alone when it does not say, in seconds.
pub(crate) const BUSY_FOR: u32 = 30;
/// Longest line of `errors`, in characters, and the length from which an account id is hidden in one.
const MAX_LINE: usize = 300;
const SECRET_ID: usize = 8;

/// Read what `request` asks of the account it is for, the calendars first and then the address books, and say what
/// was read in one answer: see [`calendars`] and [`contacts`]. `client` is signed in already ([`Client::connect`]); a
/// caller whose sign-in failed answers with [`failed_pim`]. `cancel` is read before every request. What the session
/// does not offer is not asked for, and the answer says what it offers. The server refusing the credentials, saying
/// it is busy, or a stop ends the job there: the address books are not asked for after that.
pub async fn sync_pim<H: Http>(client: &Client<H>, request: &JmapPimRequest, cancel: &AtomicBool) -> JmapPimResult {
    let offers = PimOffers { calendars: client.offers_calendars(), contacts: client.offers_contacts() };
    let mut out = JmapPimResult { account: request.account, email: request.email.clone(), offers: Some(offers), ..Default::default() };
    let mut going = true;
    match client.calendars_account() {
        Some(account) => going = calendars::read(client, request, account, cancel, &mut out).await,
        None => log::debug!("jmap pim: the session offers no calendars"),
    }
    if going
        && request.contacts
        && let Some(account) = client.contacts_account()
    {
        contacts::read(client, request, account, cancel, &mut out).await;
    }
    log::debug!(
        "jmap pim: {} calendar(s) listed, {} download(s), {} event(s), {} book(s), {} contact(s) (state {}), {} line(s); stopped: {}, busy: {}, refused: {}",
        out.calendars.as_ref().map_or(0, |list| list.calendars.len()),
        usize::from(out.events.is_some()) + out.more_events.len(),
        out.event_count(),
        out.books.as_ref().map_or(0, |list| list.books.len()),
        out.contacts.as_ref().map_or(0, |download| download.contacts.len()),
        if out.contacts_state.is_some() { "known" } else { "unknown" },
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
            result.errors.push(format!("Couldn't reach the server of the calendars and contacts: {}", said(&secrets, error)));
        }
        other => result.errors.push(format!("Couldn't reach the server of the calendars and contacts: {}", said(&secrets, other))),
    }
    result
}

/// Why a part of a job cannot go on.
pub(crate) enum Stop {
    /// The owner stopped the job.
    Cancelled,
    /// The server refused the credentials.
    Auth,
    /// The server is busy: for this many seconds, and what it said.
    Busy(u32, String),
    /// A request failed, or the server answered a query with an error.
    Failed(Error),
}

impl Stop {
    /// Why a request failed as a whole: the credentials refused and a busy server end the job, anything else ends the
    /// reading of that part with a line.
    pub(crate) fn of(error: Error, secrets: &Secrets) -> Stop {
        match error {
            Error::Auth(_) => Stop::Auth,
            Error::Busy { retry_after, .. } => Stop::Busy(retry_after.unwrap_or(BUSY_FOR), said(secrets, &error)),
            other => Stop::Failed(other),
        }
    }
}

/// What must not show in a line: what the client hides, the account's address, the address of the session (with the
/// addresses in its query), and the account's id when it is long enough to be one of those.
pub(crate) fn secrets_of(request: &JmapPimRequest, client: &Secrets, account_id: &str) -> Secrets {
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
pub(crate) fn said(secrets: &Secrets, error: &Error) -> String {
    let cut = crate::tidy(&secrets.hide(&error.to_string()), MAX_LINE);
    cut.split(' ').map(|word| if word.contains('@') || word.contains("%40") { "***" } else { word }).collect::<Vec<&str>>().join(" ")
}

/// "1 contact", "3 contacts".
pub(crate) fn counted(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// The longest name of a card a server may give that is not an id of the JMAP alphabet ([`card_id`]), in bytes.
const MAX_CARD_NAME: usize = 700;
/// What starts the engine's name of a card that a server names outside the JMAP alphabet ([`card_key`]).
const CARD_KEY: char = '~';

/// Whether `id` is what a server may call a card: an id of the JMAP alphabet ([`valid_id`]), or, from a server that
/// names its cards otherwise (one that calls a card by the name of its contact, `Grace Okafor-1`), a name of one line:
/// [`MAX_CARD_NAME`] bytes at most, no control characters, nothing blank at either end. Such a name only ever goes
/// back to the server as a JSON string, never into a path or an address.
pub fn card_id(id: &str) -> bool {
    valid_id(id) || (!id.is_empty() && id.len() <= MAX_CARD_NAME && id.trim() == id && !id.chars().any(char::is_control))
}

/// What the engine calls a card that the server calls `id` ([`card_id`]): the id itself when it is one of the JMAP
/// alphabet, else `~` and the name in URL-safe base64, which names it as surely and holds nothing a line or a word ends
/// at (the engine keeps no other kind of name: `RemoteContact::tidy`).
pub fn card_key(id: &str) -> String {
    use base64::Engine as _;
    if valid_id(id) { id.to_string() } else { format!("{CARD_KEY}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id)) }
}

/// What the server calls the card the engine calls `key` ([`card_key`] the other way): what a change goes back by.
/// `None` for a key that names no card of a server.
pub fn server_card_id(key: &str) -> Option<String> {
    use base64::Engine as _;
    let id = match key.strip_prefix(CARD_KEY) {
        Some(coded) => String::from_utf8(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(coded).ok()?).ok()?,
        None => key.to_string(),
    };
    (card_id(&id) && card_key(&id) == key).then_some(id)
}

/// Whether what is changed here in a server's calendar is written back to it. Not yet: every calendar
/// read is handed over as read-only, so that nothing is made here that would never reach the server.
pub const WRITE_EVENTS: bool = false;
/// Likewise for the contacts of a server's address books.
pub const WRITE_CONTACTS: bool = false;

/// The most an answer may be, in bytes, by what is asked: a list of ids (a query's page, 5,000 ids at the
/// most), and a page of whole items (200 events, whose descriptions may be long, or 500 cards).
pub const IDS_BYTES: usize = 4 << 20;
pub const ITEMS_BYTES: usize = 8 << 20;
/// Most pages one `/query` is read in: a server that states a tiny `limit` is not followed for ever.
const MAX_PAGES: usize = 64;
/// Most digits of one number of a duration (999,999,999 weeks is more than a duration is ever worth).
const MAX_DIGITS: usize = 9;

/// A length of time as JSCalendar says it (RFC 8984 §1.4.6): days of the calendar, which end at the same time of
/// day however many hours a change of the clock makes them, and the seconds that pass after them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// Weeks are seven of them.
    pub days: i64,
    pub seconds: i64,
}

impl Span {
    /// The whole of it in seconds, a day being 24 hours (what a span is to a limit, not to a clock).
    pub fn total_seconds(self) -> i64 {
        self.days.saturating_mul(86_400).saturating_add(self.seconds)
    }
}

/// A JSCalendar `Duration` (`P1D`, `PT1H30M`, `P1W`, `P1DT2H`, `PT0.5S`): weeks and days, then hours, minutes and
/// seconds after a `T`, each at most once and in that order; a fraction of a second is dropped. `None` for anything
/// else (a sign, years or months, a `P` alone, an empty `T`, a number too long).
pub fn duration(text: &str) -> Option<Span> {
    iso_duration(text, false)
}

/// A JSCalendar `SignedDuration` (`-PT15M`, `+P1D`, `PT0S`), in seconds (a day being 24 hours): an alert's offset
/// from the start of its event. Negative is before.
pub fn signed_seconds(text: &str) -> Option<i64> {
    let text = text.trim();
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    duration(rest).map(|span| span.total_seconds().saturating_mul(sign))
}

/// What a server's `maxExpandedQueryDuration` says, in seconds: a duration as JSCalendar has them, or with years and
/// months too (`P1Y`, `P6M`), which some servers write, a year being 365 days and a month 31. `None` for anything else.
pub fn span_seconds(text: &str) -> Option<i64> {
    iso_duration(text, true).map(Span::total_seconds)
}

/// An ISO 8601 duration without a sign: `P`, then (with `calendar`) years and months, then weeks and days, then
/// after `T` hours, minutes and seconds.
fn iso_duration(text: &str, calendar: bool) -> Option<Span> {
    let rest = text.trim().strip_prefix('P')?;
    let (date, time) = match rest.split_once('T') {
        Some((date, time)) if !time.is_empty() => (date, Some(time)),
        Some(_) => return None,
        None => (rest, None),
    };
    if date.is_empty() && time.is_none() {
        return None;
    }
    let date_units: &[(char, i64)] = if calendar { &[('Y', 365), ('M', 31), ('W', 7), ('D', 1)] } else { &[('W', 7), ('D', 1)] };
    let days = parts(date, date_units, false)?;
    let seconds = match time {
        Some(time) => parts(time, &[('H', 3_600), ('M', 60), ('S', 1)], true)?,
        None => 0,
    };
    Some(Span { days, seconds })
}

/// The numbers of one half of a duration, each with its unit from `units` (in that order, each once), as a count of
/// the smallest: `1W2D` with weeks and days is 9. `fraction`: the last unit may have a fraction, which is dropped.
fn parts(text: &str, units: &[(char, i64)], fraction: bool) -> Option<i64> {
    let mut total: i64 = 0;
    let mut rest = text;
    let mut next = 0usize;
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let (number, after) = rest.split_at(digits);
        if number.is_empty() || number.len() > MAX_DIGITS {
            return None;
        }
        // A fraction, for the seconds only: `0.5S`.
        let after = match after.strip_prefix('.') {
            Some(tail) if fraction => {
                let fraction = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
                if fraction == 0 || !tail.get(fraction..)?.starts_with('S') {
                    return None;
                }
                tail.get(fraction..)?
            }
            Some(_) => return None,
            None => after,
        };
        let unit = after.chars().next()?;
        let at = units.iter().skip(next).position(|(name, _)| *name == unit)?.checked_add(next)?;
        let (_, size) = units.get(at)?;
        total = total.checked_add(number.parse::<i64>().ok()?.checked_mul(*size)?)?;
        next = at.checked_add(1)?;
        rest = after.get(unit.len_utf8()..)?;
    }
    Some(total)
}

/// A span as JSCalendar writes a `Duration`: `P2D`, `PT1H30M`, `P1DT2H`, `PT0S`.
pub fn duration_text(span: Span) -> String {
    let (days, seconds) = (span.days.max(0), span.seconds.max(0));
    let mut out = String::from("P");
    if days > 0 {
        out.push_str(&format!("{days}D"));
    }
    if seconds > 0 || days == 0 {
        out.push('T');
        let (hours, minutes, secs) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
        if hours > 0 {
            out.push_str(&format!("{hours}H"));
        }
        if minutes > 0 {
            out.push_str(&format!("{minutes}M"));
        }
        if secs > 0 || (hours == 0 && minutes == 0) {
            out.push_str(&format!("{secs}S"));
        }
    }
    out
}

/// A JSCalendar `UTCDate` (`2026-10-20T09:00:00Z`, a fraction of a second dropped) as an instant; `None` for a time
/// that is not in UTC or is no time.
pub fn utc_date(text: &str) -> Option<Timestamp> {
    let text = text.trim();
    if !text.ends_with(['Z', 'z']) {
        return None;
    }
    match parse_iso(text)? {
        (time, Some(0)) if text.contains(['T', 't']) => Some(time.civil_seconds()),
        _ => None,
    }
}

/// An instant as a JSCalendar `UTCDate`.
pub fn utc_text(instant: Timestamp) -> String {
    format_iso_utc(instant)
}

/// A JSCalendar `LocalDateTime` (`2026-10-20T09:00:00`): a time of the clock, with no zone and no offset.
pub fn local_date_time(text: &str) -> Option<DateTime> {
    let text = text.trim();
    match parse_iso(text)? {
        (time, None) if text.contains(['T', 't']) => Some(time),
        _ => None,
    }
}

/// A time of the clock as a JSCalendar `LocalDateTime`.
pub fn local_text(time: DateTime) -> String {
    time.iso()
}

/// What paging a `/query` brought ([`query_all`]): the ids it listed, each once and in the server's order, the
/// total when the server said one, and whether they are all it has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Queried {
    pub ids: Vec<String>,
    pub total: Option<usize>,
    pub whole: bool,
}

/// Where the paging of a `/query` stands (RFC 8620 §5.5): what each page brought, and when to stop.
///
/// A page is asked from the `position` the pages before it reached, for as many as the limit in force (the one the
/// server stated, when it set one lower than asked, else the one asked). The paging ends, whole, at a page that is
/// short of that limit or empty, or when the total the server said is reached; and, not whole, at `cap` ids, at a page
/// that brings nothing new (a server that ignores `position`), at a page that says another position than was asked,
/// at a page whose list is not one, and after [`MAX_PAGES`]. An id that is no id is left out, and makes it not whole:
/// one of the JMAP alphabet ([`valid_id`]), unless the pager takes what a server may call a card too ([`Pager::cards`]).
#[derive(Clone, Debug)]
pub struct Pager {
    ids: Vec<String>,
    seen: HashSet<String>,
    position: usize,
    limit: usize,
    cap: usize,
    total: Option<usize>,
    pages: usize,
    flawed: bool,
    end: Option<bool>,
    /// What an id must be.
    fits: fn(&str) -> bool,
}

impl Pager {
    /// Paging by pages of `limit` (at least 1), up to `cap` ids in all.
    pub fn new(limit: usize, cap: usize) -> Pager {
        Pager {
            ids: Vec::new(),
            seen: HashSet::new(),
            position: 0,
            limit: limit.max(1),
            cap,
            total: None,
            pages: 0,
            flawed: false,
            end: (cap == 0).then_some(false),
            fits: valid_id,
        }
    }

    /// The same, for a query of cards: what a server may call a card is an id ([`card_id`]).
    pub fn cards(self) -> Pager {
        Pager { fits: card_id, ..self }
    }

    /// Where the next page starts.
    pub fn position(&self) -> usize {
        self.position
    }

    /// How many the next page asks for: the limit in force, no more than are left of the cap.
    pub fn limit(&self) -> usize {
        self.limit.min(self.cap.saturating_sub(self.ids.len())).max(1)
    }

    /// Take in the answer to the page asked for at [`Pager::position`]: the ids it brought that are new, in order.
    pub fn page(&mut self, answer: &Value) -> Vec<String> {
        if self.end.is_some() {
            return Vec::new();
        }
        self.pages = self.pages.saturating_add(1);
        let Some(listed) = answer.get("ids").and_then(Value::as_array) else {
            self.end = Some(false);
            return Vec::new();
        };
        let asked = self.limit();
        // The limit the server says it applied, when it set one lower than asked: a page of that many is not the last,
        // and the next pages ask for no more.
        let stated = answer.get("limit").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).filter(|n| *n > 0);
        let in_force = stated.map_or(asked, |stated| stated.min(asked));
        if let Some(stated) = stated {
            self.limit = self.limit.min(stated);
        }
        if let Some(total) = answer.get("total").and_then(Value::as_u64) {
            self.total = Some(usize::try_from(total).unwrap_or(usize::MAX));
        }
        let elsewhere = answer.get("position").and_then(Value::as_u64).is_some_and(|at| usize::try_from(at).ok() != Some(self.position));
        let mut new = Vec::new();
        for id in listed {
            if self.ids.len() >= self.cap {
                break;
            }
            match id.as_str().filter(|id| (self.fits)(id)) {
                Some(id) if self.seen.insert(id.to_string()) => {
                    self.ids.push(id.to_string());
                    new.push(id.to_string());
                }
                Some(_) => {}
                None => self.flawed = true,
            }
        }
        self.position = self.position.saturating_add(listed.len());
        let all_there = |pager: &Pager| !pager.flawed && pager.total.is_none_or(|total| pager.ids.len() >= total);
        self.end = if elsewhere {
            Some(false)
        } else if listed.len() < in_force {
            Some(all_there(self))
        } else if new.is_empty() {
            Some(false)
        } else if self.total.is_some_and(|total| self.position >= total) {
            Some(all_there(self))
        } else if self.ids.len() >= self.cap {
            Some(self.total.is_some_and(|total| total <= self.ids.len()) && !self.flawed)
        } else if self.pages >= MAX_PAGES {
            Some(false)
        } else {
            None
        };
        new
    }

    /// `Some(whole)` once no more pages are to be asked for: whether what was read is all the server has.
    pub fn done(&self) -> Option<bool> {
        self.end
    }

    /// The ids read so far, each once, in order.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// What has been read, as a [`Queried`].
    pub fn queried(self) -> Queried {
        Queried { whole: self.end == Some(true), total: self.total, ids: self.ids }
    }
}

/// Read a `/query` (`method`, with `args` as its arguments but for `position`, `limit` and `calculateTotal`) page
/// after page as [`Pager`] says: the first of `first_limit` ids with the total asked for, then on from where it
/// ended, up to `cap` ids. Each answer may be [`IDS_BYTES`] at most. Every request names `using` (the core and the
/// capability of the method). A request that fails, or a method error, ends it with that error.
pub async fn query_all<H: Http>(
    client: &Client<H>,
    using: &[&str],
    method: &str,
    args: Map<String, Value>,
    first_limit: usize,
    cap: usize,
) -> Result<Queried> {
    let mut pager = Pager::new(first_limit, cap);
    while pager.done().is_none() {
        let mut call = args.clone();
        call.insert("position".into(), Value::from(pager.position()));
        call.insert("limit".into(), Value::from(pager.limit()));
        call.insert("calculateTotal".into(), Value::Bool(pager.position() == 0));
        let mut answers = client.call_within(using, vec![(method, Value::Object(call))], IDS_BYTES).await?;
        let answer = answers.take(0)?;
        pager.page(&answer);
    }
    Ok(pager.queried())
}

/// Whether `email` is one of `me` (addresses compared as their owners write them, whatever their case).
pub fn mine(me: &[String], email: &str) -> bool {
    let email = email.trim();
    !email.is_empty() && me.iter().any(|own| own.trim().eq_ignore_ascii_case(email))
}

/// An address of a participant or an organizer as JSCalendar writes one (`mailto:ada@example.org`): the address.
pub fn mailto(text: &str) -> String {
    let text = text.trim();
    let bare = text.get(..7).filter(|scheme| scheme.eq_ignore_ascii_case("mailto:")).map_or(text, |_| text.get(7..).unwrap_or_default());
    crate::pct_decode(bare.trim())
}
