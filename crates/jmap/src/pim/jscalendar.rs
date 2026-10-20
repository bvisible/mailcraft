// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A calendar and an event as JMAP for Calendars serves them (JSCalendar, RFC 8984), into what the engine takes:
//! [`RemoteCalendar`] and [`RemoteEvent`].
//!
//! - **Times.** A timed event is its two instants, from the server's `utcStart` and `utcEnd` when it gives them (a
//!   server that keeps no zone database for its clients works them out), else from its `start` on the clock of its
//!   `timeZone` (or, for an event with none, the zone the calendar is shown in here) and its `duration`, the days on
//!   the clock first and the hours after them, as RFC 8984 counts them. Its zone is the name the server gives, when it
//!   is one the time crate knows; an event of another or of none is a floating one here. A whole-day event
//!   (`showWithoutTime`) is its days, from midnight UTC of the first to midnight UTC of the day after the last, as the
//!   engine keeps them.
//! - **People.** An address is, in this order, a participant's `calendarAddress`, `email`, `sendTo.imip` and
//!   `scheduleId`, without its `mailto:`. The organizer is `organizerCalendarAddress`, named as its participant is,
//!   else the participant whose role is `owner`; the organizer is never one of the people invited. The others are
//!   invited when their roles say `attendee` or `optional`, or say nothing. The owner's own entry is the first
//!   participant whose calendar address, else email, is one of the owner's addresses (as the server finds it when it is
//!   told an answer, [`own_entry`](super::own_entry)): what it answered is the event's answer. Whether the organizer is
//!   one of the owner's addresses says whether the owner organizes it (`None` when nobody does).
//! - **The rest.** Free or Busy from `freeBusyStatus`, Tentative from a `status` of `tentative`; private when its
//!   `privacy` is `private` or `secret`; cancelled from its `status`; the reminder is the first alert (in the order of
//!   their names) that shows itself, with an offset before the start ([`reminder_alert`]); the place is the first
//!   location with a name; a description the server says is HTML loses its tags.
//! - **Series.** A series itself (an event with a rule and no `recurrenceId`, read by its own id) is the engine's
//!   series: its rule as the mailbox says rules ([`rule_of`]: `recurrenceRule`, or the one entry of the newer
//!   `recurrenceRules`), the day it counts from (the date of its `start`), and the times of its first occurrence. A
//!   rule the mailbox has no way of saying (two of them, an excluding rule, a frequency of hours, days of a month
//!   other than the start's, the nth occurrence of a set, another calendar scale) leaves the series without one: the
//!   engine then passes it over, and its occurrences are events of their own. The occurrences it changes beyond
//!   leaving them out (its `recurrenceOverrides`) are said with it ([`Read::changed`]). An occurrence (it has a
//!   `recurrenceId`) names its series by its `baseEventId`, and says when the series has it: its `recurrenceId` on the
//!   clock of its `recurrenceIdTimeZone`, else of its own zone, else of the zone the calendar is shown in
//!   ([`RemoteEvent::original`], set when that is not when it happens).
//!
//! Everything is hostile: a value of the wrong type is no value, lists are looked at up to a bound, and the event is
//! cut to size afterwards ([`RemoteEvent::tidy`]).

use mailcraft_model::remote::{RemoteCalendar, RemoteEvent, RemoteEventKind};
use mailcraft_model::{Address, Attendee, Freq, Recurrence, Response, Rgb, ShowAs};
use mailcraft_time::{Date, DateTime, Timestamp, Weekday, Zone};
use serde_json::{Map, Value};

use super::{duration, local_date_time, mailto, mine, own_entry, signed_seconds, utc_date};
use crate::valid_id;

/// Most participants of one event that are looked at, and most alerts and locations.
const MAX_PARTICIPANTS: usize = 1_000;
const MAX_ENTRIES: usize = 100;
/// The longest text taken of a description, and of a title, a name or a place, in characters (what the model keeps,
/// and a little more for the tidying to cut).
const MAX_TEXT: usize = 100_000;
const MAX_LINE: usize = 1_000;
/// The longest a whole-day event is taken to last, in days.
const MAX_DAYS: i64 = 3_660;
/// Most changed occurrences of one series that are looked at, and most days of the week of one rule.
const MAX_OVERRIDES: usize = 5_000;
const MAX_RULE_DAYS: usize = 35;

/// What an event of a server's answer was passed over for lacking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// An id that is one.
    Id,
    /// A start that can be read.
    Start,
    /// An end that can be read (no `utcEnd`, and no length).
    End,
}

/// An event as it was read: the calendars it says it is in (their ids), and the event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub calendars: Vec<String>,
    pub event: RemoteEvent,
    /// For an occurrence of a series: when the series has it, on the clock of the series (its `recurrenceId`).
    pub recurrence_id: Option<DateTime>,
    /// For a series: the occurrences it changes beyond leaving them out (a time, a title, the people), by when the
    /// series has them.
    pub changed: Vec<DateTime>,
    /// The owner's own entry ([`own_entry`](super::own_entry)) has the role of an owner of the event: they organize it,
    /// whatever address the organizer is named by (one of theirs the request doesn't name).
    pub owner_role: bool,
}

/// A calendar as it was read: the engine's, and whether the account may write its events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub calendar: RemoteCalendar,
    /// The server flags it as the account's default calendar.
    pub default: bool,
    /// The account may write every event of it (`myRights.mayWriteAll`).
    pub writable: bool,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Text from outside, no longer than `max` characters (the model cuts it to its own size later).
fn bounded(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The entries of a JSCalendar map (`locations`, `participants`, `alerts`) that are objects, in the order of their names,
/// at most `most` of them.
fn entries<'a>(value: &'a Value, key: &str, most: usize) -> impl Iterator<Item = &'a Map<String, Value>> {
    value.get(key).and_then(Value::as_object).into_iter().flat_map(|map| map.values()).filter_map(Value::as_object).take(most)
}

/// A calendar of `Calendar/get`, as the engine takes it; `None` without an id that is one.
pub fn calendar_of(value: &Value) -> Option<Listed> {
    let id = text(value, "id");
    if !valid_id(id) {
        return None;
    }
    let right = |key: &str| value.get("myRights").and_then(|rights| rights.get(key)).and_then(Value::as_bool) == Some(true);
    let calendar = RemoteCalendar {
        id: id.to_string(),
        name: bounded(text(value, "name"), MAX_LINE),
        color: color(value.get("color").and_then(Value::as_str).unwrap_or_default()),
        ..Default::default()
    };
    Some(Listed { calendar, default: value.get("isDefault").and_then(Value::as_bool) == Some(true), writable: right("mayWriteAll") })
}

/// A colour as a calendar may have it: `#rrggbb`, `#rgb`, or one of a few CSS names; `None` for anything else.
pub fn color(text: &str) -> Option<Rgb> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        return match hex.len() {
            6 => Rgb::parse(hex),
            3 if hex.is_ascii() => {
                let doubled: String = hex.chars().flat_map(|c| [c, c]).collect();
                Rgb::parse(&doubled)
            }
            _ => None,
        };
    }
    const NAMED: [(&str, Rgb); 18] = [
        ("black", Rgb(0x00, 0x00, 0x00)),
        ("white", Rgb(0xFF, 0xFF, 0xFF)),
        ("red", Rgb(0xFF, 0x00, 0x00)),
        ("green", Rgb(0x00, 0x80, 0x00)),
        ("blue", Rgb(0x00, 0x00, 0xFF)),
        ("yellow", Rgb(0xFF, 0xFF, 0x00)),
        ("orange", Rgb(0xFF, 0xA5, 0x00)),
        ("purple", Rgb(0x80, 0x00, 0x80)),
        ("gray", Rgb(0x80, 0x80, 0x80)),
        ("grey", Rgb(0x80, 0x80, 0x80)),
        ("pink", Rgb(0xFF, 0xC0, 0xCB)),
        ("brown", Rgb(0xA5, 0x2A, 0x2A)),
        ("teal", Rgb(0x00, 0x80, 0x80)),
        ("navy", Rgb(0x00, 0x00, 0x80)),
        ("maroon", Rgb(0x80, 0x00, 0x00)),
        ("olive", Rgb(0x80, 0x80, 0x00)),
        ("cyan", Rgb(0x00, 0xFF, 0xFF)),
        ("magenta", Rgb(0xFF, 0x00, 0xFF)),
    ];
    NAMED.iter().find(|(name, _)| name.eq_ignore_ascii_case(text)).map(|(_, rgb)| *rgb)
}

/// An event of `CalendarEvent/get`, for an owner whose addresses are `me`, with `here` the zone the calendar is shown
/// in (for an event that names none, when the server gives no instants). See the module documentation.
pub fn event_of(value: &Value, me: &[String], here: Option<Zone>) -> Result<Read, Missing> {
    let id = text(value, "id");
    if !valid_id(id) {
        return Err(Missing::Id);
    }
    let all_day = value.get("showWithoutTime").and_then(Value::as_bool) == Some(true);
    let (start, end) = if all_day { days(value)? } else { instants(value, here)? };
    let named_zone = text(value, "timeZone").trim();
    let start_zone = if !all_day && Zone::named(named_zone).is_some() { named_zone.to_string() } else { String::new() };
    let (organizer, attendees, response, owner_role) = people(value, me);
    let is_organizer = organizer.as_ref().map(|organizer| mine(me, &organizer.email));
    let status = text(value, "status").trim().to_ascii_lowercase();
    let show_as = match (status.as_str(), text(value, "freeBusyStatus").trim().to_ascii_lowercase().as_str()) {
        ("tentative", _) => ShowAs::Tentative,
        (_, "free") => ShowAs::Free,
        _ => ShowAs::Busy,
    };
    let privacy = text(value, "privacy").trim().to_ascii_lowercase();
    let series = text(value, "baseEventId");
    let recurrence_id = local_date_time(text(value, "recurrenceId"));
    let occurrence = value.get("recurrenceId").is_some_and(|at| !at.is_null()) && valid_id(series);
    // A series itself: an event with a rule that is no occurrence of another.
    let master = !occurrence && value.get("recurrenceId").is_none_or(Value::is_null) && has_rule(value);
    let clock =
        local_date_time(text(value, "start")).or_else(|| mailcraft_time::Date::parse_iso(text(value, "start").trim()).map(DateTime::at_midnight));
    // When the series has an occurrence, as an instant: its recurrence id on the clock of the series' zone.
    let original = recurrence_id.filter(|_| occurrence).map(|at| {
        if all_day {
            DateTime::at_midnight(at.date).to_timestamp(0)
        } else {
            let zone = Zone::named(text(value, "recurrenceIdTimeZone")).or_else(|| Zone::named(named_zone)).or(here).unwrap_or(Zone::UTC);
            zone.instant(at)
        }
    });
    let updated = text(value, "updated");
    let change_key = if !updated.trim().is_empty() {
        bounded(updated.trim(), 64)
    } else {
        value.get("sequence").and_then(Value::as_u64).map(|n| n.to_string()).unwrap_or_default()
    };
    let uid = bounded(text(value, "uid").trim(), MAX_LINE);
    let mut event = RemoteEvent {
        id: id.to_string(),
        change_key,
        token: uid.clone(),
        uid,
        title: bounded(text(value, "title"), MAX_LINE),
        location: location(value),
        description: description(value),
        start,
        end,
        all_day,
        start_zone,
        organizer,
        attendees,
        response,
        show_as,
        private: matches!(privacy.as_str(), "private" | "secret"),
        cancelled: status == "cancelled",
        kind: match (occurrence, master) {
            (true, _) => RemoteEventKind::Occurrence,
            (false, true) => RemoteEventKind::SeriesMaster,
            (false, false) => RemoteEventKind::Single,
        },
        series: if occurrence { series.to_string() } else { String::new() },
        reminder_minutes: reminder(value),
        modified: utc_date(updated).unwrap_or(0),
        is_organizer,
        recurrence: clock.filter(|_| master).and_then(|clock| one_rule(value).and_then(|rule| rule_of(rule, clock, all_day))),
        series_day: clock.filter(|_| master).map(|clock| clock.date),
        original: original.filter(|at| *at != start),
        ..Default::default()
    };
    event.tidy();
    let calendars = value
        .get("calendarIds")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(id, on)| on.as_bool() == Some(true) && valid_id(id))
        .take(MAX_ENTRIES)
        .map(|(id, _)| id.clone())
        .collect();
    let changed = if master { changed_occurrences(value) } else { Vec::new() };
    Ok(Read { calendars, event, recurrence_id: recurrence_id.filter(|_| occurrence), changed, owner_role })
}

/// Whether an event says how it repeats (a rule, or a list of them that is not empty).
fn has_rule(value: &Value) -> bool {
    value.get("recurrenceRule").is_some_and(Value::is_object)
        || value.get("recurrenceRules").and_then(Value::as_array).is_some_and(|rules| !rules.is_empty())
}

/// The one rule a series repeats by: the newer list's when it has one entry, else the older single rule. `None`
/// when it has none, more than one, or rules that leave days out (`excludedRecurrenceRules`).
fn one_rule(value: &Value) -> Option<&Map<String, Value>> {
    if value.get("excludedRecurrenceRules").and_then(Value::as_array).is_some_and(|rules| !rules.is_empty()) {
        return None;
    }
    match value.get("recurrenceRules") {
        Some(Value::Array(rules)) if rules.len() == 1 => rules.first().and_then(Value::as_object),
        Some(Value::Array(rules)) if !rules.is_empty() => None,
        _ => value.get("recurrenceRule").and_then(Value::as_object),
    }
}

/// The occurrences a series changes beyond leaving them out (`recurrenceOverrides` whose patch says more than
/// `excluded`), by when the series has them; at most [`MAX_OVERRIDES`].
fn changed_occurrences(value: &Value) -> Vec<DateTime> {
    let Some(overrides) = value.get("recurrenceOverrides").and_then(Value::as_object) else { return Vec::new() };
    overrides
        .iter()
        .take(MAX_OVERRIDES)
        .filter_map(|(at, patch)| {
            let patch = patch.as_object()?;
            let left_out = patch.get("excluded").and_then(Value::as_bool) == Some(true);
            let more = patch.keys().any(|key| key != "excluded" && key != "@type");
            (!left_out && more).then(|| local_date_time(at)).flatten()
        })
        .collect()
}

/// A JSCalendar `RecurrenceRule` (RFC 8984 §4.3.3) as the mailbox says how an event repeats, for a series whose
/// first occurrence starts at `start` on its clock (`all_day`: a whole-day one). `None` for a rule the mailbox has
/// no way of saying: a frequency of hours, minutes or seconds; another calendar scale, or skipping otherwise than by
/// leaving a day out; the days of a year or the weeks of it, the nth of a set, hours and minutes other than the
/// start's; days of the month, or months, other than the start's; days of the week counted in a period they can't
/// be counted in here (every Monday of a month, the second Monday of a year); a count and an end together; anything
/// of the wrong shape.
pub fn rule_of(rule: &Map<String, Value>, start: DateTime, all_day: bool) -> Option<Recurrence> {
    let word = |key: &str| rule.get(key).filter(|value| !value.is_null()).map(|value| value.as_str().map(|text| text.trim().to_ascii_lowercase()));
    let list = |key: &str| -> Option<Vec<Value>> {
        match rule.get(key) {
            None | Some(Value::Null) => Some(Vec::new()),
            Some(Value::Array(items)) => Some(items.iter().take(MAX_RULE_DAYS + 1).cloned().collect()),
            Some(_) => None,
        }
    };
    let numbers = |key: &str| -> Option<Vec<i64>> { list(key)?.iter().map(Value::as_i64).collect() };
    if let Some(scale) = word("rscale") {
        scale.filter(|scale| scale == "gregorian")?;
    }
    if let Some(skip) = word("skip") {
        skip.filter(|skip| skip == "omit")?;
    }
    for unsaid in ["byYearDay", "byWeekNo", "bySetPosition"] {
        if !list(unsaid)?.is_empty() {
            return None;
        }
    }
    // Hours, minutes and seconds may only say the start's own.
    for (key, own) in [("byHour", start.hour), ("byMinute", start.minute), ("bySecond", start.second)] {
        let said = numbers(key)?;
        if !said.is_empty() && said != [i64::from(own)] {
            return None;
        }
    }
    let freq = match word("frequency")??.as_str() {
        "yearly" => Freq::Yearly,
        "monthly" => Freq::Monthly,
        "weekly" => Freq::Weekly,
        "daily" => Freq::Daily,
        _ => return None,
    };
    let interval = match rule.get("interval") {
        None | Some(Value::Null) => 1,
        Some(value) => u32::try_from(value.as_u64()?).ok().filter(|n| (1..=999).contains(n))?,
    };
    // The days of the week, each with which of them in its period is meant (none: every one).
    let mut days: Vec<(Weekday, Option<i64>)> = Vec::new();
    for entry in list("byDay")? {
        let entry = entry.as_object()?;
        let day = Weekday::from_ical(entry.get("day")?.as_str()?)?;
        let nth = match entry.get("nthOfPeriod") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_i64()?),
        };
        if days.len() > MAX_RULE_DAYS {
            return None;
        }
        days.push((day, nth));
    }
    let month_days = numbers("byMonthDay")?;
    if !month_days.is_empty() && (matches!(freq, Freq::Daily | Freq::Weekly) || month_days != [i64::from(start.date.day)]) {
        return None;
    }
    let months = list("byMonth")?;
    let own_month = start.date.month.to_string();
    let in_its_month =
        months.is_empty() || (months.len() == 1 && months.first().and_then(Value::as_str).is_some_and(|month| month.trim() == own_month));
    if !months.is_empty() && (freq != Freq::Yearly || !in_its_month) {
        return None;
    }
    // A weekly rule of several days, every other week or more, counts its weeks from Monday here.
    if let Some(first) = word("firstDayOfWeek") {
        let monday = first.as_deref().is_none_or(|first| first == "mo");
        if freq == Freq::Weekly && interval > 1 && days.len() > 1 && !monday {
            return None;
        }
    }
    let plain: Vec<Weekday> = days.iter().filter(|(_, nth)| nth.is_none()).map(|(day, _)| *day).collect();
    let counted: Vec<(Weekday, i64)> = days.iter().filter_map(|(day, nth)| nth.map(|nth| (*day, nth))).collect();
    let mut out = match (freq, plain.is_empty(), counted.as_slice()) {
        (Freq::Daily | Freq::Weekly, _, []) => Recurrence { freq, by_day: plain, ..Default::default() },
        (Freq::Monthly | Freq::Yearly, true, []) => Recurrence { freq, ..Default::default() },
        // The nth weekday of the month: of the start's month, for a yearly one.
        (Freq::Monthly | Freq::Yearly, true, [(day, nth)]) if month_days.is_empty() && matches!(*nth, 1..=5 | -1) => {
            if freq == Freq::Yearly && months.is_empty() {
                return None;
            }
            Recurrence { freq, by_day: vec![*day], nth: i8::try_from(*nth).ok(), ..Default::default() }
        }
        _ => return None,
    };
    out.interval = interval;
    let count = match rule.get("count") {
        None | Some(Value::Null) => None,
        Some(value) => Some(u32::try_from(value.as_u64()?).ok().filter(|n| *n > 0)?.min(9_999)),
    };
    let until = match rule.get("until") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let text = value.as_str()?;
            let at = local_date_time(text).or_else(|| Date::parse_iso(text.trim()).map(DateTime::at_midnight))?;
            // The last day it may happen on: the day before, when it ends before the start's time of day on its own.
            let early = !all_day && at.seconds_of_day() < start.seconds_of_day();
            Some(if early { at.date.add_days(-1) } else { at.date })
        }
    };
    if count.is_some() && until.is_some() {
        return None;
    }
    (out.count, out.until) = (count, until);
    Some(out)
}

/// A whole-day event's days: midnight UTC of the day of its `start`, and of the day after its last (a length that is
/// not whole days ends at the end of the day it ends on; no length is one day).
fn days(value: &Value) -> Result<(Timestamp, Timestamp), Missing> {
    let start =
        local_date_time(text(value, "start")).or_else(|| mailcraft_time::Date::parse_iso(text(value, "start").trim()).map(DateTime::at_midnight));
    let first = start.ok_or(Missing::Start)?.date;
    let length = duration(text(value, "duration")).map_or(1, |span| {
        let extra = if span.seconds > 0 { span.seconds.saturating_add(86_399) / 86_400 } else { 0 };
        span.days.saturating_add(extra)
    });
    let at = |date: mailcraft_time::Date| DateTime::at_midnight(date).to_timestamp(0);
    Ok((at(first), at(first.add_days(length.clamp(1, MAX_DAYS)))))
}

/// A timed event's instants: the server's own (`utcStart`, `utcEnd`), else worked out from its clock and its length.
fn instants(value: &Value, here: Option<Zone>) -> Result<(Timestamp, Timestamp), Missing> {
    let zone = Zone::named(text(value, "timeZone")).or(here).unwrap_or(Zone::UTC);
    let clock = local_date_time(text(value, "start"));
    let start = utc_date(text(value, "utcStart")).or_else(|| clock.map(|clock| zone.instant(clock))).ok_or(Missing::Start)?;
    let worked_out = || {
        let span = duration(text(value, "duration"))?;
        // The days on the clock of its zone, then the time that passes after them (RFC 8984 §1.4.6).
        let clock = clock.unwrap_or_else(|| zone.wall(start));
        let after_days = zone.instant(clock.add_days(span.days.clamp(0, MAX_DAYS)));
        Some(after_days.saturating_add(span.seconds))
    };
    let end = utc_date(text(value, "utcEnd")).or_else(worked_out).ok_or(Missing::End)?;
    Ok((start, end.max(start)))
}

/// The first location (by the order of their names) that has a name.
fn location(value: &Value) -> String {
    entries(value, "locations", MAX_ENTRIES)
        .filter_map(|place| place.get("name").and_then(Value::as_str))
        .map(str::trim)
        .find(|name| !name.is_empty())
        .map(|name| bounded(name, MAX_LINE))
        .unwrap_or_default()
}

/// The description as plain text: one the server says is HTML loses its tags, and its few common entities read.
fn description(value: &Value) -> String {
    let words = bounded(text(value, "description"), MAX_TEXT);
    if !text(value, "descriptionContentType").trim().to_ascii_lowercase().starts_with("text/html") {
        return words;
    }
    let mut plain = String::with_capacity(words.len());
    let mut in_tag = false;
    for c in words.chars() {
        match (c, in_tag) {
            ('<', _) => in_tag = true,
            ('>', true) => {
                in_tag = false;
                plain.push(' ');
            }
            (_, true) => {}
            (c, false) => plain.push(c),
        }
    }
    let plain =
        plain.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    plain.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// The minutes before the start of the reminder ([`reminder_alert`]).
fn reminder(value: &Value) -> Option<u32> {
    value.get("alerts").and_then(Value::as_object).and_then(reminder_alert).map(|(_, minutes)| minutes)
}

/// The alert of an event's `alerts` that is its reminder, its name and the minutes before the start: the first (by the
/// order of their names, of the first [`MAX_ENTRIES`] that are alerts at all) that shows itself (its `action` is
/// `display`, or says nothing) and whose trigger is an offset from the start that is not after it. The writer changes
/// that one alone ([`super::write`]).
pub fn reminder_alert(alerts: &Map<String, Value>) -> Option<(&String, u32)> {
    alerts.iter().filter_map(|(key, alert)| alert.as_object().map(|alert| (key, alert))).take(MAX_ENTRIES).find_map(|(key, alert)| {
        if alert.get("action").and_then(Value::as_str).unwrap_or("display") != "display" {
            return None;
        }
        let trigger = alert.get("trigger")?.as_object()?;
        let kind = trigger.get("@type").and_then(Value::as_str).unwrap_or("OffsetTrigger");
        let from = trigger.get("relativeTo").and_then(Value::as_str).unwrap_or("start");
        if kind != "OffsetTrigger" || from != "start" {
            return None;
        }
        let offset = signed_seconds(trigger.get("offset").and_then(Value::as_str).unwrap_or_default())?;
        (offset <= 0).then(|| (key, u32::try_from(offset.unsigned_abs() / 60).unwrap_or(u32::MAX)))
    })
}

/// A participant's address, in the order the server's own writers read it: `calendarAddress`, `email`,
/// `sendTo.imip`, `scheduleId`; without its `mailto:`. Empty when it has none.
fn address_of(participant: &Map<String, Value>) -> String {
    let said = |key: &str| participant.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty());
    let imip = participant.get("sendTo").and_then(|to| to.get("imip")).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty());
    said("calendarAddress").or_else(|| said("email")).or(imip).or_else(|| said("scheduleId")).map(mailto).unwrap_or_default()
}

/// Whether a participant's `roles` has `role`.
fn has_role(participant: &Map<String, Value>, role: &str) -> bool {
    participant.get("roles").and_then(|roles| roles.get(role)).and_then(Value::as_bool) == Some(true)
}

/// What a participant answered.
fn answer(participant: &Map<String, Value>) -> Response {
    match participant.get("participationStatus").and_then(Value::as_str).map(str::to_ascii_lowercase).as_deref() {
        Some("accepted") => Response::Accepted,
        Some("tentative") => Response::Tentative,
        Some("declined") => Response::Declined,
        _ => Response::None,
    }
}

/// The organizer, the people invited, what the owner answered, and whether the owner's own entry is an owner's (see the
/// module documentation).
fn people(value: &Value, me: &[String]) -> (Option<Address>, Vec<Attendee>, Response, bool) {
    let said = mailto(text(value, "organizerCalendarAddress"));
    let mut organizer = said.contains('@').then(|| Address { name: String::new(), email: bounded(&said, MAX_LINE) });
    let mut attendees = Vec::new();
    let mut response = Response::None;
    let (mut answered, mut owner_role) = (false, false);
    for participant in entries(value, "participants", MAX_PARTICIPANTS) {
        let email = bounded(&address_of(participant), MAX_LINE);
        let name = bounded(participant.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default(), MAX_LINE);
        let owner = has_role(participant, "owner") || organizer.as_ref().is_some_and(|o| !email.is_empty() && o.email.eq_ignore_ascii_case(&email));
        // What the person answered: their first own entry's, as the server finds it when it is told an answer.
        if !answered && own_entry(me, participant) {
            response = answer(participant);
            owner_role = has_role(participant, "owner");
            answered = true;
        }
        if owner {
            match &mut organizer {
                Some(o) if o.email.eq_ignore_ascii_case(&email) && o.name.is_empty() => o.name = name,
                None => organizer = Some(Address { name, email }),
                Some(_) => {}
            }
            continue;
        }
        let roles = participant.get("roles").and_then(Value::as_object).filter(|roles| roles.values().any(|on| on.as_bool() == Some(true)));
        let invited = roles.is_none() || has_role(participant, "attendee") || has_role(participant, "optional");
        if invited && (!email.is_empty() || !name.is_empty()) {
            attendees.push(Attendee { address: Address { name, email }, optional: has_role(participant, "optional"), response: answer(participant) });
        }
    }
    (organizer, attendees, response, owner_role)
}

#[cfg(test)]
mod tests;
