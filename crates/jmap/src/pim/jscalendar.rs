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
//!   invited when their roles say `attendee` or `optional`, or say nothing. The owner is the participant who is one of
//!   the owner's addresses: what they answered is the event's answer, and whether the organizer is one of them says
//!   whether the owner organizes it (`None` when nobody does).
//! - **The rest.** Free or Busy from `freeBusyStatus`, Tentative from a `status` of `tentative`; private when its
//!   `privacy` is `private` or `secret`; cancelled from its `status`; the reminder is the first alert (in the order of
//!   their names) with an offset before the start; the place is the first location with a name; a description the
//!   server says is HTML loses its tags. An occurrence of a series (it has a `recurrenceId`) names the series by its
//!   `baseEventId`, and is an occurrence the engine keeps as an event of its own while it has no series to tell it
//!   from.
//!
//! Everything is hostile: a value of the wrong type is no value, lists are looked at up to a bound, and the event is
//! cut to size afterwards ([`RemoteEvent::tidy`]).

use mailcraft_model::remote::{RemoteCalendar, RemoteEvent, RemoteEventKind};
use mailcraft_model::{Address, Attendee, Response, Rgb, ShowAs};
use mailcraft_time::{DateTime, Timestamp, Zone};
use serde_json::{Map, Value};

use super::{duration, local_date_time, mailto, mine, signed_seconds, utc_date};
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
    let (organizer, attendees, response) = people(value, me);
    let is_organizer = organizer.as_ref().map(|organizer| mine(me, &organizer.email));
    let status = text(value, "status").trim().to_ascii_lowercase();
    let show_as = match (status.as_str(), text(value, "freeBusyStatus").trim().to_ascii_lowercase().as_str()) {
        ("tentative", _) => ShowAs::Tentative,
        (_, "free") => ShowAs::Free,
        _ => ShowAs::Busy,
    };
    let privacy = text(value, "privacy").trim().to_ascii_lowercase();
    let series = text(value, "baseEventId");
    let occurrence = value.get("recurrenceId").is_some_and(|at| !at.is_null()) && valid_id(series);
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
        kind: if occurrence { RemoteEventKind::Occurrence } else { RemoteEventKind::Single },
        series: if occurrence { series.to_string() } else { String::new() },
        reminder_minutes: reminder(value),
        modified: utc_date(updated).unwrap_or(0),
        is_organizer,
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
    Ok(Read { calendars, event })
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

/// The minutes before the start of the first alert (by the order of their names) whose trigger is an offset from the
/// start that is not after it.
fn reminder(value: &Value) -> Option<u32> {
    entries(value, "alerts", MAX_ENTRIES).find_map(|alert| {
        let trigger = alert.get("trigger")?.as_object()?;
        let kind = trigger.get("@type").and_then(Value::as_str).unwrap_or("OffsetTrigger");
        let from = trigger.get("relativeTo").and_then(Value::as_str).unwrap_or("start");
        if kind != "OffsetTrigger" || from != "start" {
            return None;
        }
        let offset = signed_seconds(trigger.get("offset").and_then(Value::as_str).unwrap_or_default())?;
        (offset <= 0).then(|| u32::try_from(offset.unsigned_abs() / 60).unwrap_or(u32::MAX))
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

/// The organizer, the people invited, and what the owner answered (see the module documentation).
fn people(value: &Value, me: &[String]) -> (Option<Address>, Vec<Attendee>, Response) {
    let said = mailto(text(value, "organizerCalendarAddress"));
    let mut organizer = said.contains('@').then(|| Address { name: String::new(), email: bounded(&said, MAX_LINE) });
    let mut attendees = Vec::new();
    let mut response = Response::None;
    for participant in entries(value, "participants", MAX_PARTICIPANTS) {
        let email = bounded(&address_of(participant), MAX_LINE);
        let name = bounded(participant.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default(), MAX_LINE);
        let owner = has_role(participant, "owner") || organizer.as_ref().is_some_and(|o| !email.is_empty() && o.email.eq_ignore_ascii_case(&email));
        if response == Response::None && mine(me, &email) {
            response = answer(participant);
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
    (organizer, attendees, response)
}

#[cfg(test)]
mod tests;
