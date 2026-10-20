// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! An account's calendars over JMAP (JMAP for Calendars, the IETF's draft on RFC 8620; events as JSCalendar,
//! RFC 8984), and later its address books (RFC 9610), read for the engine: [`sync_pim`].
//!
//! A calendar job is one of its own beside a Send/Receive: the window asks for it on a clock of its own,
//! and what it reads comes back in one answer ([`JmapPimResult`](mailcraft_model::sync::JmapPimResult)), made
//! of the downloads the engine already takes in for Microsoft Graph's calendar
//! ([`CalendarsDownload`](mailcraft_model::remote::CalendarsDownload),
//! [`EventsDownload`](mailcraft_model::remote::EventsDownload)). What is read and how is in [`calendars`];
//! how an event in JSCalendar becomes the engine's is in `jscalendar.rs`.
//!
//! This module holds what the readers share: ISO 8601 durations, the UTC and local times of JSCalendar,
//! the paging of a `/query` ([`query_all`]) and the bounds of what is read.
//!
//! **Nothing is written yet.** Until events go back to the server, every calendar is handed over as one the
//! account may only read ([`WRITE_EVENTS`]): the engine's own lock and its reason then keep the owner from
//! making, here, what would go nowhere. [`WRITE_CONTACTS`] is the same door for address books.

pub mod calendars;
pub(crate) mod jscalendar;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

use mailcraft_time::{DateTime, Timestamp, format_iso_utc, parse_iso};
use serde_json::{Map, Value};

use crate::{Client, Http, Result, valid_id};

pub use calendars::{failed_pim, sync_pim};

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
/// at a page whose list is not one, and after [`MAX_PAGES`]. An id that is no id is left out, and makes it not whole.
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
        }
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
            match id.as_str().filter(|id| valid_id(id)) {
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
