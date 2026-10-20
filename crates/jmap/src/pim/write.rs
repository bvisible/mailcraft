// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What the engine says was made, changed or answered here of a server's event ([`EventOp`]), as JMAP for Calendars
//! writes it (JSCalendar, RFC 8984): a new event whole ([`create`]), a change as the whole properties it names
//! ([`patch`]), an answer as the people of the event with the person's own answer ([`answer`]).
//!
//! - **Times.** A whole-day event is its days: its first at midnight, `P{n}D`, with no zone. A timed one is its start
//!   on the clock of its own zone when it has one the server can be told (a name of the zone database), else of the
//!   zone the calendar is shown in, with that zone's name; a series by the clock it repeats by. Its length is the
//!   days on that clock and the time that passes after them, as RFC 8984 counts a `Duration` (an event from nine on
//!   Saturday to nine on Sunday is `P1D`, whatever the clocks did that night), or the time that passes when the days
//!   don't add up. A start in the hour the clocks show twice, the night they go back, cannot be told on that clock:
//!   such an event is written in UTC, and loses its own zone. One occurrence of a series keeps its series' zone: its
//!   start is on that clock, and no zone goes with it ([`Clock::Series`]).
//! - **Places, people and alerts** are written whole, as the server takes them: the server's own map, read just before
//!   ([`Copy`]), with the person's change made to it, so that what somebody else changed there meanwhile stays. A
//!   change of who is invited is who came and who went since the event it was made to (`EventOp::invited_before`):
//!   anybody else of the server's stays as the server has them. A copy of more than [`MAX_COPY_PEOPLE`] is not
//!   rewritten. A reminder is the one alert the reader takes for it, set; turned off, every alert the reader would take
//!   for it goes; the other alerts stay.
//! - **What a server that keeps only the basics doesn't keep** (categories, importance, response options, online
//!   meetings) is not written: it would refuse the whole event, and the person's other changes with it. A change of
//!   nothing else is none ([`patch`] says so). But an online meeting is asked of a server that writes virtual locations,
//!   by the request ([`online_request`]): the server makes a room and gives the event its link.

use mailcraft_model::remote::{EventFields, EventOp, EventTime};
use mailcraft_model::{Attendee, Freq, Recurrence, Response, ShowAs};
use mailcraft_time::{Date, DateTime, Timestamp, Zone};
use serde_json::{Map, Value, json};

use super::{Span, duration_text, jscalendar, local_text, mailto, mine, own_entry};

/// The zone a time is written in that can't be told on its own clock.
pub const UTC_ZONE: &str = "Etc/UTC";
/// The address of a virtual location by which a client asks the server to make an event an online meeting (a room of
/// its own, whose link it then gives the event).
pub const ONLINE_MEETING_REQUEST: &str = "urn:x-online-meeting:new";
/// The property an event's online meeting is in.
pub const ONLINE: &str = "virtualLocations";
/// What the person is told when only what such a server doesn't keep was changed.
pub const KEEPS_NO: &str = "this calendar's server keeps no categories, importance, response options, online meetings or proposed times";
/// Most people of an event that are written.
const MAX_PEOPLE: usize = 500;
/// Most people of the server's copy of an event that are rewritten: as many as are read.
pub const MAX_COPY_PEOPLE: usize = 1_000;
/// The longest a whole-day event is written to last, in days.
const MAX_DAYS: i64 = 3_660;

/// The server's copy of an event, read just before it is written to: its places and its people, for a change to be
/// made to them.
pub type Copy = Map<String, Value>;

/// On which clock a time is written.
#[derive(Clone, Copy, Debug)]
pub enum Clock {
    /// An event's own: its own zone, or the zone the calendar is shown in.
    Own(Zone),
    /// One occurrence of a series: the series' zone (`None`: a series of no zone, read on the clock of the zone the
    /// calendar is shown in, which is the second). No zone is written with it.
    Series(Option<Zone>, Zone),
}

/// The JSCalendar time of an event (`start`, `duration`, `showWithoutTime`, and `timeZone` but for an occurrence),
/// and the zone it was written in (`""` for whole days).
pub fn times(fields: &EventFields, time: EventTime, clock: Clock) -> (Map<String, Value>, String) {
    let mut out = Map::new();
    let occurrence = matches!(clock, Clock::Series(..));
    if time.all_day {
        let first = DateTime::from_timestamp(time.start, 0).date;
        let days = (time.end.saturating_sub(time.start) / 86_400).clamp(1, MAX_DAYS);
        out.insert("start".into(), json!(local_text(DateTime::at_midnight(first))));
        out.insert("duration".into(), json!(format!("P{days}D")));
        out.insert("showWithoutTime".into(), json!(true));
        if !occurrence {
            out.insert("timeZone".into(), Value::Null);
        }
        return (out, String::new());
    }
    // The zone, and the name it is written by.
    let (zone, name) = match clock {
        Clock::Series(own, shown) => (own.unwrap_or(shown), None),
        Clock::Own(shown) => {
            let own = fields.zones.as_ref().and_then(|zones| Zone::named(&zones.start));
            let zone = own.unwrap_or(shown);
            (zone, Some(zone_name(zone)))
        }
    };
    // A series by the clock it repeats by, when it is this zone's.
    let wall = fields.wall.filter(|_| fields.zones.as_ref().and_then(|zones| Zone::named(&zones.start)).is_none_or(|own| own == zone));
    let (start, end) = match wall {
        Some(wall) => (wall.start, wall.end.max(wall.start)),
        None => (zone.wall(time.start), zone.wall(time.end.max(time.start))),
    };
    // A zone the server can't be told, or a start the clock shows twice: in UTC.
    let twice = wall.is_none() && zone.instant(start) != time.start;
    let in_utc = !occurrence && (name.as_deref() == Some(UTC_ZONE) || twice);
    let (zone, start, end, name, wall) = if in_utc {
        (Zone::UTC, Zone::UTC.wall(time.start), Zone::UTC.wall(time.end.max(time.start)), Some(UTC_ZONE.to_string()), None)
    } else {
        (zone, start, end, name, wall)
    };
    out.insert("start".into(), json!(local_text(start)));
    out.insert("duration".into(), json!(length(zone, start, end, time, wall.is_some())));
    out.insert("showWithoutTime".into(), json!(false));
    if let Some(name) = &name {
        out.insert("timeZone".into(), json!(name));
    }
    (out, name.unwrap_or_else(|| zone.id()))
}

/// The name a zone is written by: its name in the zone database, or [`UTC_ZONE`] for UTC and for a zone that has
/// none (an offset).
fn zone_name(zone: Zone) -> String {
    let id = zone.id();
    if id.contains('/') && !id.starts_with("Etc/") { id } else { UTC_ZONE.to_string() }
}

/// How long an event lasts on the clock of `zone` from `start` to `end`: the days on that clock and the time that
/// passes after them, when that comes out at its end; else the time that passes. `by_clock`: the times are a
/// series' clock times, whose length is the clock's.
fn length(zone: Zone, start: DateTime, end: DateTime, time: EventTime, by_clock: bool) -> String {
    let exact = || duration_text(Span { days: 0, seconds: time.end.saturating_sub(time.start).max(0) });
    if by_clock {
        let seconds = end.civil_seconds().saturating_sub(start.civil_seconds()).max(0);
        return duration_text(Span { days: seconds / 86_400, seconds: seconds % 86_400 });
    }
    let mut days = start.date.days_until(end.date).clamp(0, MAX_DAYS);
    if start.add_days(days) > end {
        days = days.saturating_sub(1);
    }
    if days == 0 {
        return exact();
    }
    let after_days: Timestamp = zone.instant(start.add_days(days));
    let rest = time.end.saturating_sub(after_days);
    if rest < 0 {
        return exact();
    }
    duration_text(Span { days, seconds: rest })
}

/// How an event repeats, as JSCalendar says it, counted from `first`.
pub fn rule_json(rule: &Recurrence, first: Date) -> Value {
    let mut out = Map::new();
    out.insert("@type".into(), json!("RecurrenceRule"));
    out.insert(
        "frequency".into(),
        json!(match rule.freq {
            Freq::Daily => "daily",
            Freq::Weekly => "weekly",
            Freq::Monthly => "monthly",
            Freq::Yearly => "yearly",
        }),
    );
    let interval = rule.interval.clamp(1, 999);
    if interval > 1 {
        out.insert("interval".into(), json!(interval));
    }
    let day = |day: mailcraft_time::Weekday| day.ical().to_ascii_lowercase();
    match (rule.freq, rule.nth.filter(|nth| matches!(nth, 1..=5 | -1))) {
        (Freq::Daily | Freq::Weekly, _) if !rule.by_day.is_empty() => {
            let days: Vec<Value> = rule.by_day.iter().map(|d| json!({"@type": "NDay", "day": day(*d)})).collect();
            out.insert("byDay".into(), json!(days));
        }
        (Freq::Monthly | Freq::Yearly, Some(nth)) => {
            let counted = rule.by_day.first().copied().unwrap_or_else(|| first.weekday());
            out.insert("byDay".into(), json!([{"@type": "NDay", "day": day(counted), "nthOfPeriod": nth}]));
            if rule.freq == Freq::Yearly {
                out.insert("byMonth".into(), json!([first.month.to_string()]));
            }
        }
        _ => {}
    }
    match (rule.count, rule.until) {
        (Some(count), _) => {
            out.insert("count".into(), json!(count.clamp(1, 9_999)));
        }
        // The last day it may happen on, to its end.
        (None, Some(until)) => {
            let last = DateTime { date: until.max(first), hour: 23, minute: 59, second: 59 };
            out.insert("until".into(), json!(local_text(last)));
        }
        (None, None) => {}
    }
    Value::Object(out)
}

/// The trigger of a reminder that many minutes before the start.
fn trigger(minutes: u32) -> Value {
    let offset = if minutes == 0 { "PT0S".to_string() } else { format!("-{}", duration_text(Span { days: 0, seconds: i64::from(minutes) * 60 })) };
    json!({"@type": "OffsetTrigger", "offset": offset, "relativeTo": "start"})
}

/// The server's alerts (of `copy`; none for a new event) with the reminder as the person has it: the alert that is the
/// reminder as it is read ([`jscalendar::reminder_alert`]) set to that many minutes before the start, or a new one when
/// there is none; when it is off, every alert that would be read as the reminder taken away, one after the other, so
/// that the next read finds none. Every other alert stays as the server has it.
fn alerts(copy: Option<&Copy>, on: bool, minutes: u32) -> Value {
    let mut map: Map<String, Value> = copy.and_then(|copy| copy.get("alerts")).and_then(Value::as_object).cloned().unwrap_or_default();
    if !on {
        // (Each turn takes one away: no more turns than there are alerts.)
        while let Some(key) = jscalendar::reminder_alert(&map).map(|(key, _)| key.clone()) {
            map.remove(&key);
        }
        return Value::Object(map);
    }
    match jscalendar::reminder_alert(&map).map(|(key, _)| key.clone()) {
        Some(key) => {
            if let Some(Value::Object(alert)) = map.get_mut(&key) {
                alert.insert("trigger".into(), trigger(minutes));
            }
        }
        None => {
            let key = free_key(&map, "a");
            map.insert(key, json!({"@type": "Alert", "trigger": trigger(minutes), "action": "display"}));
        }
    }
    Value::Object(map)
}

/// One person invited, as such a server takes a participant who is to answer.
fn invited(who: &Attendee) -> Value {
    let email = who.address.email.trim();
    let mut roles = Map::new();
    roles.insert("attendee".into(), json!(true));
    if who.optional {
        roles.insert("optional".into(), json!(true));
    }
    let mut out = json!({
        "@type": "Participant",
        "email": email,
        "sendTo": {"imip": format!("mailto:{email}")},
        "roles": roles,
        "participationStatus": "needs-action",
        "expectReply": true,
    });
    if !who.address.name.trim().is_empty() {
        out["name"] = json!(who.address.name.trim());
    }
    out
}

/// A participant's address, in the order the server's writers read it, without its `mailto:`.
fn address_of(entry: &Value) -> String {
    let said = |key: &str| entry.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty());
    let imip = entry.get("sendTo").and_then(|to| to.get("imip")).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty());
    said("calendarAddress").or_else(|| said("email")).or(imip).or_else(|| said("scheduleId")).map(mailto).unwrap_or_default()
}

fn has_role(entry: &Value, role: &str) -> bool {
    entry.get("roles").and_then(|roles| roles.get(role)).and_then(Value::as_bool) == Some(true)
}

/// A key `prefix1`, `prefix2`… that `map` doesn't have yet.
fn free_key(map: &Map<String, Value>, prefix: &str) -> String {
    (1..=map.len().saturating_add(1)).map(|n| format!("{prefix}{n}")).find(|key| !map.contains_key(key)).unwrap_or_else(|| format!("{prefix}0"))
}

/// The server's places with `place` as the first of them with a name: that one renamed, or taken away when `place`
/// is empty, or a new one when it has none.
fn locations(copy: Option<&Copy>, place: &str) -> Value {
    let mut map: Map<String, Value> = copy.and_then(|copy| copy.get("locations")).and_then(Value::as_object).cloned().unwrap_or_default();
    let place = place.trim();
    let first = map
        .iter()
        .find(|(_, entry)| entry.get("name").and_then(Value::as_str).is_some_and(|name| !name.trim().is_empty()))
        .map(|(key, _)| key.clone());
    match (first, place.is_empty()) {
        (Some(key), true) => {
            map.remove(&key);
        }
        (Some(key), false) => {
            if let Some(Value::Object(entry)) = map.get_mut(&key) {
                entry.insert("name".into(), json!(place));
            }
        }
        (None, true) => {}
        (None, false) => {
            let key = free_key(&map, "l");
            map.insert(key, json!({"@type": "Location", "name": place}));
        }
    }
    Value::Object(map)
}

/// Whether the server's copy of an event has more people than are rewritten here ([`MAX_COPY_PEOPLE`]).
pub fn too_many_people(copy: &Copy) -> bool {
    copy.get("participants").and_then(Value::as_object).is_some_and(|map| map.len() > MAX_COPY_PEOPLE)
}

/// The server's people with the person's change of who is invited made to them: those invited in the event the change
/// was made to (`before`) and not now go, those invited now and not before come, those whose being optional changed
/// are so; everybody else of the server's stays as the server has them (one added there meanwhile, one the window
/// never shows: a room, somebody informed, a chair). The person and the organizer always stay. Without `before`, nobody
/// goes and nothing is changed of those there: those invited now who are not there come.
fn participants(copy: Option<&Copy>, invited_now: &[Attendee], before: Option<&[Attendee]>, me: &[String]) -> Value {
    let mut map: Map<String, Value> = copy.and_then(|copy| copy.get("participants")).and_then(Value::as_object).cloned().unwrap_or_default();
    let organizer = copy.map(|copy| mailto(copy.get("organizerCalendarAddress").and_then(Value::as_str).unwrap_or_default())).unwrap_or_default();
    let same = |one: &Attendee, email: &str| !email.is_empty() && one.address.email.trim().eq_ignore_ascii_case(email);
    let now_of = |email: &str| invited_now.iter().find(|who| same(who, email));
    let before_of = |email: &str| before.and_then(|before| before.iter().find(|was| same(was, email)));
    // Gone: invited before, and not now. Optional or not: as now says, of those it changed for.
    let gone = |email: &str| before_of(email).is_some() && now_of(email).is_none();
    let reoptioned = |email: &str| match (before_of(email), now_of(email)) {
        (Some(was), Some(now)) if was.optional != now.optional => Some(now.optional),
        _ => None,
    };
    map.retain(|_, entry| {
        let email = address_of(entry);
        let keeps = has_role(entry, "owner") || mine(me, &email) || (!organizer.is_empty() && organizer.eq_ignore_ascii_case(&email));
        keeps || email.is_empty() || !gone(&email)
    });
    for entry in map.values_mut() {
        let email = address_of(entry);
        if let (Some(optional), Some(roles)) =
            (reoptioned(&email), entry.as_object_mut().and_then(|entry| entry.get_mut("roles")).and_then(Value::as_object_mut))
        {
            if optional {
                roles.insert("optional".into(), json!(true));
            } else {
                roles.remove("optional");
            }
        }
    }
    // Come: invited now, and not before (any of them, when before is not known), and not there already.
    for who in invited_now.iter().take(MAX_PEOPLE) {
        let email = who.address.email.trim();
        if email.is_empty() || before_of(email).is_some() || map.values().any(|entry| address_of(entry).eq_ignore_ascii_case(email)) {
            continue;
        }
        let key = free_key(&map, "p");
        map.insert(key, invited(who));
    }
    Value::Object(map)
}

/// A new event whole (`op.fields`), with the mark it is made with as its `uid`, in the calendar it names (none for
/// the main one: the server's default). Whether the server is to invite the people in it.
pub fn create(op: &EventOp, clock: Clock) -> Option<(Value, bool, String)> {
    let fields = &op.fields;
    let time = fields.time?;
    let mut out = Map::new();
    out.insert("@type".into(), json!("Event"));
    out.insert("uid".into(), json!(op.token));
    if !op.calendar.is_empty() {
        out.insert("calendarIds".into(), json!({op.calendar.clone(): true}));
    }
    out.insert("title".into(), json!(fields.title.clone().unwrap_or_default()));
    if let Some(words) = fields.description.as_deref().filter(|words| !words.trim().is_empty()) {
        out.insert("description".into(), json!(words));
    }
    if let Some(place) = fields.location.as_deref().filter(|place| !place.trim().is_empty()) {
        out.insert("locations".into(), locations(None, place));
    }
    let (when, zone) = times(fields, time, clock);
    out.extend(when);
    out.insert("freeBusyStatus".into(), json!(free_busy(fields.show_as.unwrap_or_default())));
    out.insert("privacy".into(), json!(if fields.private == Some(true) { "private" } else { "public" }));
    let reminder = fields.reminder.unwrap_or_default();
    out.insert("alerts".into(), alerts(None, reminder.on, reminder.minutes));
    if let Some(rule) = fields.repeat.as_ref().and_then(|repeat| repeat.rule.as_ref().map(|rule| (rule, repeat.first))) {
        out.insert("recurrenceRule".into(), rule_json(rule.0, rule.1));
    }
    let invites = fields.attendees.as_ref().is_some_and(|people| !people.is_empty());
    if let Some(people) = fields.attendees.as_ref().filter(|people| !people.is_empty()) {
        out.insert("participants".into(), participants(None, people, None, &[]));
    }
    Some((Value::Object(out), invites, zone))
}

/// The virtual locations by which an event asks to be made an online meeting ([`ONLINE_MEETING_REQUEST`]), for a change
/// that asks for one (`EventFields::online`) of an event that is not one occurrence of a series (an online meeting is
/// the whole series'); `None` otherwise. Only for a server that writes virtual locations (its session says so).
pub fn online_request(op: &EventOp, clock: Clock) -> Option<Value> {
    (op.fields.online.is_some() && !matches!(clock, Clock::Series(..)))
        .then(|| json!({"online": {"@type": "VirtualLocation", "uri": ONLINE_MEETING_REQUEST}}))
}

/// Whether the virtual locations of a change (or of a new event) are the request for an online meeting.
pub fn is_online_request(places: &Value) -> bool {
    places.as_object().is_some_and(|places| {
        !places.is_empty() && places.values().all(|place| place.get("uri").and_then(Value::as_str) == Some(ONLINE_MEETING_REQUEST))
    })
}

/// The link to join by of the first of `places` (an event's virtual locations, by the order of their names) whose
/// address is one ([`mailcraft_model::OnlineMeeting::clean_link`]).
pub fn join_link(places: &Value) -> Option<String> {
    places.as_object()?.values().find_map(|place| mailcraft_model::OnlineMeeting::clean_link(place.get("uri").and_then(Value::as_str)?))
}

/// How an event shows, as such a server keeps it: free or busy.
fn free_busy(show_as: ShowAs) -> &'static str {
    if show_as == ShowAs::Free { "free" } else { "busy" }
}

/// What changed of an event (`op.fields`), as the whole properties it names, made to the server's copy `copy` (for
/// the places and the people); `me` are the person's addresses. `None` when nothing that such a server keeps is
/// left to write. The zone the time was written in, when one was.
pub fn patch(op: &EventOp, copy: Option<&Copy>, clock: Clock, me: &[String]) -> Option<(Value, String)> {
    let fields = &op.fields;
    let mut out = Map::new();
    if let Some(title) = &fields.title {
        out.insert("title".into(), json!(title));
    }
    if let Some(words) = &fields.description {
        out.insert("description".into(), json!(words));
    }
    if let Some(place) = &fields.location {
        out.insert("locations".into(), locations(copy, place));
    }
    let mut zone = String::new();
    if let Some(time) = fields.time {
        let (when, written) = times(fields, time, clock);
        out.extend(when);
        zone = written;
    }
    if let Some(show_as) = fields.show_as {
        out.insert("freeBusyStatus".into(), json!(free_busy(show_as)));
    }
    if let Some(private) = fields.private {
        out.insert("privacy".into(), json!(if private { "private" } else { "public" }));
    }
    if let Some(reminder) = fields.reminder {
        out.insert("alerts".into(), alerts(copy, reminder.on, reminder.minutes));
    }
    if let Some(repeat) = &fields.repeat
        && !matches!(clock, Clock::Series(..))
    {
        out.insert("recurrenceRule".into(), repeat.rule.as_ref().map_or(Value::Null, |rule| rule_json(rule, repeat.first)));
    }
    if let Some(people) = &fields.attendees {
        out.insert("participants".into(), participants(copy, people, op.invited_before.as_deref(), me));
    }
    (!out.is_empty()).then_some((Value::Object(out), zone))
}

/// Whether a change says anything that the people of a meeting would see (and be written to about): anything but how
/// it shows in the person's own calendar and its reminder.
pub fn seen_by_others(fields: &EventFields) -> bool {
    let quiet = EventFields { show_as: fields.show_as, reminder: fields.reminder, categories: fields.categories.clone(), ..Default::default() };
    *fields != quiet
}

/// The person's answer to somebody else's meeting, as the server takes one: the server's people, read just before,
/// with the person's own answer set: in the first of them that is the person's own entry, found by its calendar address
/// or its email as the server finds it ([`own_entry`]). `None` when the person is not among them, or the answer is no
/// answer.
pub fn answer(copy: &Copy, response: Response, me: &[String]) -> Option<Value> {
    let status = match response {
        Response::Accepted => "accepted",
        Response::Tentative => "tentative",
        Response::Declined => "declined",
        Response::None => return None,
    };
    let mut map: Map<String, Value> = copy.get("participants").and_then(Value::as_object).cloned()?;
    let own = map.iter().find(|(_, entry)| entry.as_object().is_some_and(|entry| own_entry(me, entry))).map(|(key, _)| key.clone())?;
    if let Some(Value::Object(entry)) = map.get_mut(&own) {
        entry.insert("participationStatus".into(), json!(status));
    }
    Some(json!({"participants": map}))
}

/// Whether the person organizes the event the server's copy is of (its organizer is one of their addresses), and
/// whether others are invited to it; and whether it is a draft, which writes to nobody.
pub fn meeting(copy: &Copy, me: &[String]) -> (bool, bool, bool) {
    let organizer = mailto(copy.get("organizerCalendarAddress").and_then(Value::as_str).unwrap_or_default());
    let people: Vec<String> = copy.get("participants").and_then(Value::as_object).into_iter().flatten().map(|(_, entry)| address_of(entry)).collect();
    let owner = copy
        .get("participants")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .find(|(_, entry)| has_role(entry, "owner"))
        .map(|(_, entry)| address_of(entry));
    let organizes = if organizer.is_empty() { owner.is_none_or(|owner| owner.is_empty() || mine(me, &owner)) } else { mine(me, &organizer) };
    let others = people.iter().any(|who| !who.is_empty() && !mine(me, who) && !who.eq_ignore_ascii_case(&organizer));
    let draft = copy.get("isDraft").and_then(Value::as_bool) == Some(true);
    (organizes, others, draft)
}

#[cfg(test)]
mod tests;
