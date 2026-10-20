// //// Neoffice — added file (no upstream equivalent)
//! What the stand-in offers besides mail: calendars (JMAP for Calendars) and address books (RFC 9610), when a test
//! turns them on ([`Data::pim`](super::Data::pim)), held to what a server that keeps only the basics of an event
//! serves, as strictly as it does.
//!
//! - **The session** then names the capability, its account (the mail's, [`ACCOUNT`](super::ACCOUNT)) and what the
//!   account says of it: for calendars the stretch the server expands repeating events over
//!   (`maxExpandedQueryDuration`) and, when [`Pim::says_writable`], the properties an event keeps when it is written
//!   (`writableProperties`, which a server may add to the draft's); likewise for address books. A request may name
//!   the capability in `using` only when the session offers it, and a method of calendars needs it there.
//! - **`Calendar/get`** lists every calendar (`ids: null`) or those named, each with the eight rights of the draft
//!   (`myRights`: `mayWriteAll` says whether its events may be written; a calendar where only the account's own
//!   events may be is not one), one of them the default (the first flagged among those whose events may be written,
//!   else the first of those, else the first flagged, else the first), its colour as `#rrggbb` or `null`.
//! - **`CalendarEvent/query`** takes a stretch it requires (`after` and `before`, UTC, 500 days at most) and the
//!   calendars to look in (`inCalendars`, by id; names it doesn't have find nothing), and no other filter, no sort
//!   and no anchor; it lists the events that overlap the stretch by their start, each occurrence of a series apart
//!   with `expandRecurrences`, 5,000 at a time at most (the limit said back when it set it), from a `position` that
//!   may be negative, with the total when asked. An event's id is `V` and its id inside, an occurrence's `O` and its
//!   series and start (`["<series>", "<start>"]`, JSON), a calendar's `C` and its id inside, each in URL-safe base64:
//!   ids a server makes up for its clients, which name an occurrence even when the store behind them renumbers it.
//! - **`CalendarEvent/get`** reads the events named (never all of them: `requestTooLarge`), an occurrence by its `O`
//!   id, a series by its `V` id with its rule and the occurrences it leaves out (`recurrenceOverrides`); each timed
//!   one with its start and end in UTC worked out (`utcStart`, `utcEnd`: the clock of its zone, or of the person's for
//!   one that names none, then its length, the days on the clock and the hours after them); `properties` keeps those
//!   named (and the id).
//! - **Trouble on demand**, besides the mail part's (`refuse_next`, `drop_token_after`, `lost_answers`, method errors
//!   through `faults`): events whose `get` fails ([`Pim::broken_events`]).

use std::collections::BTreeSet;

use base64::Engine as _;
use mailcraft_time::{Date, DateTime, Timestamp, Weekday, Zone};
use serde_json::{Map, Value, json};

use super::{ACCOUNT, Outcome, account, allow_only, invalid, refuse};
use crate::pim::{duration, local_date_time, local_text, utc_date, utc_text};

/// The properties a server that keeps only the basics of an event writes, and of a card: what its session says
/// when it says.
pub const WRITABLE_EVENT: [&str; 16] = [
    "alerts",
    "calendarIds",
    "description",
    "duration",
    "freeBusyStatus",
    "links",
    "locations",
    "participants",
    "privacy",
    "recurrenceRule",
    "showWithoutTime",
    "start",
    "status",
    "timeZone",
    "title",
    "useDefaultAlerts",
];
pub const WRITABLE_CARD: [&str; 5] = ["emails", "name", "organizations", "phones", "titles"];

/// The longest stretch a query takes, in seconds, and most events it lists at a time.
const MAX_STRETCH: i64 = 500 * 86_400;
const MAX_LISTED: usize = 5_000;
/// Most calendars a query may name, and most steps of a rule that are walked.
const MAX_NAMED: usize = 500;
const MAX_STEPS: usize = 20_000;

/// A calendar the stand-in has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calendar {
    /// What the store behind the server calls it: the API names it `C` and that ([`Pim::calendar_id`]).
    pub raw: String,
    pub name: String,
    /// Its colour as the store keeps it: served as `#rrggbb`, or `null` for what is no colour.
    pub color: Option<String>,
    /// The store flags it as a default (it may flag several, or none: the server serves one).
    pub flagged: bool,
    /// The account may write every event of it; only its own.
    pub write_all: bool,
    pub write_own: bool,
}

impl Calendar {
    /// A calendar whose events the account may write, or (`writable: false`) only read.
    pub fn new(raw: &str, name: &str, writable: bool) -> Calendar {
        Calendar { raw: raw.into(), name: name.into(), color: None, flagged: false, write_all: writable, write_own: writable }
    }
}

/// An event the stand-in has: one of its own, or a series with its rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// What the store calls it: the API names it `V` and that ([`Pim::event_id`]).
    pub raw: String,
    /// The calendars it is in, by what the store calls them.
    pub calendars: Vec<String>,
    /// Its start on the clock (`2026-10-19T09:00:00`), the zone of that clock (`None`: a floating event, read in the
    /// person's), its length, and whether it is whole days.
    pub start: String,
    pub zone: Option<String>,
    pub duration: String,
    pub all_day: bool,
    /// For a series: its rule as JSCalendar has it (`{"frequency": "weekly", "count": 3}`), and the starts on the
    /// clock of the occurrences it leaves out.
    pub rule: Option<Value>,
    pub excluded: Vec<String>,
    /// Everything else it is served with, as it is (`uid`, `title`, `participants`, `alerts`…).
    pub props: Map<String, Value>,
}

impl Event {
    /// A timed event in `calendar`, at `start` on the clock of `zone` (`None`: floating), lasting `duration`.
    pub fn timed(raw: &str, calendar: &str, title: &str, start: &str, zone: Option<&str>, duration: &str) -> Event {
        let mut props = Map::new();
        props.insert("uid".into(), json!(format!("{raw}@standin.example")));
        props.insert("title".into(), json!(title));
        Event {
            raw: raw.into(),
            calendars: vec![calendar.into()],
            start: start.into(),
            zone: zone.map(str::to_string),
            duration: duration.into(),
            all_day: false,
            rule: None,
            excluded: Vec::new(),
            props,
        }
    }

    /// A whole-day event in `calendar` from `day` (`2026-10-20`), `days` long.
    pub fn whole_days(raw: &str, calendar: &str, title: &str, day: &str, days: u32) -> Event {
        Event { all_day: true, zone: None, ..Event::timed(raw, calendar, title, &format!("{day}T00:00:00"), None, &format!("P{days}D")) }
    }

    /// The same, served with `key` as `value` too.
    pub fn with(mut self, key: &str, value: Value) -> Event {
        self.props.insert(key.into(), value);
        self
    }

    /// The same, as a series repeating by `rule`.
    pub fn repeating(mut self, rule: Value) -> Event {
        self.rule = Some(rule);
        self
    }
}

/// What the stand-in offers besides mail. Nothing, at first: a stand-in for mail stays one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pim {
    /// The session offers calendars.
    pub calendars: bool,
    /// The session offers address books.
    pub contacts: bool,
    /// The session says what an event and a card keep when they are written (`writableProperties`).
    pub says_writable: bool,
    /// What the session says of the stretch the server expands repeating events over in one query.
    pub span: String,
    /// The person's zone, where a floating event and a whole day are read.
    pub zone: String,
    /// The calendars, and the events in them.
    pub calendar_list: Vec<Calendar>,
    pub events: Vec<Event>,
    /// Events (their ids in the API) whose `CalendarEvent/get` fails as a whole with `serverFail`.
    pub broken_events: BTreeSet<String>,
}

impl Default for Pim {
    fn default() -> Pim {
        Pim {
            calendars: false,
            contacts: false,
            says_writable: true,
            span: "P500D".into(),
            zone: "Europe/Zurich".into(),
            calendar_list: Vec::new(),
            events: Vec::new(),
            broken_events: BTreeSet::new(),
        }
    }
}

/// URL-safe base64 without padding: the alphabet of ids.
fn b64(text: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text)
}

fn unb64(text: &str) -> Option<String> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text).ok().and_then(|bytes| String::from_utf8(bytes).ok())
}

/// One occurrence of an event: its start on the clock, and (for a series) the start that names it.
struct Occurrence {
    start: DateTime,
    recurrence: Option<String>,
}

impl Pim {
    /// Calendars and address books, as a server that keeps only the basics offers them, saying so.
    pub fn offered() -> Pim {
        Pim { calendars: true, contacts: true, ..Pim::default() }
    }

    /// A calendar's id in the API.
    pub fn calendar_id(raw: &str) -> String {
        format!("C{}", b64(raw))
    }

    /// An event's id in the API.
    pub fn event_id(raw: &str) -> String {
        format!("V{}", b64(raw))
    }

    /// An occurrence's id in the API: its series and its start on the clock (`2026-10-19T09:00:00`).
    pub fn occurrence_id(series: &str, start: &str) -> String {
        format!("O{}", b64(&json!([series, start]).to_string()))
    }

    /// Add a calendar; its id in the API.
    pub fn add_calendar(&mut self, calendar: Calendar) -> String {
        let id = Pim::calendar_id(&calendar.raw);
        self.calendar_list.push(calendar);
        id
    }

    /// Add an event; its id in the API.
    pub fn add_event(&mut self, event: Event) -> String {
        let id = Pim::event_id(&event.raw);
        self.events.push(event);
        id
    }

    /// What the session adds for them: the capabilities, and the account's own.
    pub(super) fn add_to(&self, session: &mut Value, account: &str) {
        if self.calendars {
            session["capabilities"][crate::CALENDARS] = json!({});
            let mut said = json!({
                "maxCalendarsPerEvent": null,
                "minDateTime": "1970-01-01T00:00:00Z",
                "maxDateTime": "2100-01-01T00:00:00Z",
                "maxExpandedQueryDuration": self.span,
                "maxParticipantsPerEvent": null,
                "mayCreateCalendar": false,
            });
            if self.says_writable {
                said["writableProperties"] = json!(WRITABLE_EVENT);
            }
            session["accounts"][account]["accountCapabilities"][crate::CALENDARS] = said;
            session["primaryAccounts"][crate::CALENDARS] = json!(account);
        }
        if self.contacts {
            session["capabilities"][crate::CONTACTS] = json!({});
            let mut said = json!({"maxAddressBooksPerCard": 1, "mayCreateAddressBook": false});
            if self.says_writable {
                said["writableProperties"] = json!(WRITABLE_CARD);
            }
            session["accounts"][account]["accountCapabilities"][crate::CONTACTS] = said;
            session["primaryAccounts"][crate::CONTACTS] = json!(account);
        }
    }

    /// Whether a request may name `capability` in `using`: the session offers it.
    pub(super) fn knows(&self, capability: &str) -> bool {
        (capability == crate::CALENDARS && self.calendars) || (capability == crate::CONTACTS && self.contacts)
    }

    /// The person's zone.
    fn person_zone(&self) -> Zone {
        Zone::named(&self.zone).unwrap_or(Zone::UTC)
    }

    /// The default calendar, by what the store calls it (see the module documentation).
    fn default_calendar(&self) -> Option<&str> {
        let list = &self.calendar_list;
        let writable = || list.iter().filter(|c| c.write_all);
        writable()
            .find(|c| c.flagged)
            .or_else(|| writable().next())
            .or_else(|| list.iter().find(|c| c.flagged))
            .or_else(|| list.first())
            .map(|c| c.raw.as_str())
    }

    /// `Calendar/get`.
    pub(super) fn calendar_get(&self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "ids", "properties"])?;
        account(args)?;
        let wanted: Option<Vec<String>> = match args.get("ids") {
            None | Some(Value::Null) => None,
            Some(Value::Array(ids)) if ids.len() > MAX_NAMED => return Err(refuse("requestTooLarge", "too many ids")),
            Some(Value::Array(ids)) => Some(ids.iter().filter_map(Value::as_str).map(str::to_string).collect()),
            Some(_) => return Err(invalid("ids is a list, or null")),
        };
        let default = self.default_calendar();
        let mut list = Vec::new();
        for calendar in &self.calendar_list {
            let id = Pim::calendar_id(&calendar.raw);
            if wanted.as_ref().is_some_and(|wanted| !wanted.contains(&id)) {
                continue;
            }
            list.push(json!({
                "id": id,
                "name": calendar.name,
                "description": null,
                "color": calendar.color.as_deref().and_then(hex_color),
                "sortOrder": 0,
                "isSubscribed": true,
                "isVisible": true,
                "isDefault": default == Some(calendar.raw.as_str()),
                "includeInAvailability": "none",
                "myRights": {
                    "mayReadFreeBusy": true,
                    "mayReadItems": true,
                    "mayWriteAll": calendar.write_all,
                    "mayWriteOwn": calendar.write_own,
                    "mayUpdatePrivate": calendar.write_all,
                    "mayRSVP": calendar.write_all || calendar.write_own,
                    "mayAdmin": false,
                    "mayDelete": false,
                },
            }));
        }
        let found: Vec<&str> = list.iter().filter_map(|c| c["id"].as_str()).collect();
        let not_found: Vec<&String> = wanted.iter().flatten().filter(|id| !found.contains(&id.as_str())).collect();
        Ok(json!({"accountId": ACCOUNT, "state": "0", "list": list, "notFound": not_found}))
    }

    /// The UTC start and end of an occurrence that starts at `start` on the clock of `event`.
    fn instants(&self, event: &Event, start: DateTime) -> (Timestamp, Timestamp) {
        let zone = if event.all_day { self.person_zone() } else { event.zone.as_deref().and_then(Zone::named).unwrap_or_else(|| self.person_zone()) };
        let span = duration(&event.duration).unwrap_or_default();
        let from = zone.instant(start);
        let to = zone.instant(start.add_days(span.days.clamp(0, 3_660))).saturating_add(span.seconds);
        (from, to.max(from))
    }

    /// The occurrences of `event` that begin before `before` (all of a series' rule, as far as it is walked; the one
    /// of an event of its own), but the ones it leaves out.
    fn occurrences(&self, event: &Event, before: Timestamp) -> Vec<Occurrence> {
        let Some(first) = local_date_time(&event.start) else { return Vec::new() };
        let Some(rule) = &event.rule else { return vec![Occurrence { start: first, recurrence: None }] };
        let frequency = rule.get("frequency").and_then(Value::as_str).unwrap_or("daily");
        let interval = rule.get("interval").and_then(Value::as_i64).unwrap_or(1).clamp(1, 1_000);
        let count = rule.get("count").and_then(Value::as_u64).map(|n| usize::try_from(n).unwrap_or(usize::MAX));
        let until = rule.get("until").and_then(Value::as_str).and_then(local_date_time);
        let by_day: Vec<Weekday> = rule
            .get("byDay")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| d.get("day").and_then(Value::as_str))
            .filter_map(Weekday::from_ical)
            .collect();
        let mut out = Vec::new();
        let mut made = 0usize;
        let at = |date: Date| DateTime { date, ..first };
        let mut step = 0i64;
        while out.len() < MAX_LISTED && usize::try_from(step).unwrap_or(usize::MAX) < MAX_STEPS {
            let dates: Vec<Date> = match frequency {
                "weekly" if !by_day.is_empty() => {
                    let monday = first.date.add_days(-i64::from(first.date.weekday().index())).add_days(step * 7 * interval);
                    let mut days: Vec<Date> = by_day.iter().map(|day| monday.add_days(i64::from(day.index()))).collect();
                    days.sort();
                    days.into_iter().filter(|d| *d >= first.date).collect()
                }
                "weekly" => vec![first.date.add_days(step * 7 * interval)],
                "monthly" => {
                    let month = first.date.add_months(i32::try_from(step * interval).unwrap_or(i32::MAX));
                    Date::new(month.year, month.month, first.date.day).into_iter().collect()
                }
                "yearly" => {
                    let year = first.date.add_years(i32::try_from(step * interval).unwrap_or(i32::MAX));
                    Date::new(year.year, first.date.month, first.date.day).into_iter().collect()
                }
                _ => vec![first.date.add_days(step * interval)],
            };
            step += 1;
            let mut past = false;
            for date in dates {
                let start = at(date);
                if count.is_some_and(|count| made >= count) || until.is_some_and(|until| start > until) {
                    return out;
                }
                made += 1;
                if self.instants(event, start).0 >= before {
                    past = true;
                    break;
                }
                let recurrence = local_text(start);
                if !event.excluded.contains(&recurrence) {
                    out.push(Occurrence { start, recurrence: Some(recurrence) });
                }
            }
            if past {
                break;
            }
        }
        out
    }

    /// An event as the API serves it: of its own, a series (its rule and what it leaves out), or one occurrence.
    fn served(&self, event: &Event, occurrence: Option<&Occurrence>) -> Value {
        let mut out = event.props.clone();
        let calendars: Map<String, Value> = event.calendars.iter().map(|raw| (Pim::calendar_id(raw), Value::Bool(true))).collect();
        let start = occurrence.map_or_else(|| local_date_time(&event.start), |o| Some(o.start));
        out.insert("@type".into(), json!("Event"));
        out.insert("calendarIds".into(), Value::Object(calendars));
        out.insert("start".into(), json!(start.map_or_else(|| event.start.clone(), local_text)));
        out.insert("timeZone".into(), json!(event.zone));
        out.insert("duration".into(), json!(event.duration));
        out.insert("showWithoutTime".into(), json!(event.all_day));
        match occurrence.and_then(|o| o.recurrence.as_deref()) {
            Some(recurrence) => {
                out.insert("id".into(), json!(Pim::occurrence_id(&event.raw, recurrence)));
                out.insert("recurrenceId".into(), json!(recurrence));
                out.insert("baseEventId".into(), json!(Pim::event_id(&event.raw)));
            }
            None => {
                out.insert("id".into(), json!(Pim::event_id(&event.raw)));
                if let Some(rule) = &event.rule {
                    out.insert("recurrenceRule".into(), rule.clone());
                    let left_out: Map<String, Value> = event.excluded.iter().map(|at| (at.clone(), json!({"excluded": true}))).collect();
                    out.insert("recurrenceOverrides".into(), Value::Object(left_out));
                }
            }
        }
        if !event.all_day
            && let Some(start) = start
        {
            let (from, to) = self.instants(event, start);
            out.insert("utcStart".into(), json!(utc_text(from)));
            out.insert("utcEnd".into(), json!(utc_text(to)));
        }
        Value::Object(out)
    }

    /// `CalendarEvent/query`.
    pub(super) fn event_query(&self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "filter", "sort", "position", "anchor", "anchorOffset", "limit", "calculateTotal", "expandRecurrences"])?;
        account(args)?;
        if !matches!(args.get("sort"), None | Some(Value::Null)) {
            return Err(refuse("unsupportedSort", "events are listed by their start"));
        }
        if !matches!(args.get("anchor"), None | Some(Value::Null)) || !matches!(args.get("anchorOffset"), None | Some(Value::Null)) {
            return Err(invalid("anchor"));
        }
        let unsupported = |why: &str| refuse("unsupportedFilter", why);
        let filter = match args.get("filter") {
            Some(Value::Object(filter)) => filter,
            _ => return Err(unsupported("A window (after and before) is required.")),
        };
        if filter.keys().any(|key| !matches!(key.as_str(), "after" | "before" | "inCalendars")) {
            return Err(unsupported("Only after, before and inCalendars are supported."));
        }
        let date = |key: &str| -> Outcome<Option<Timestamp>> {
            match filter.get(key) {
                None => Ok(None),
                Some(value) => value.as_str().and_then(utc_date).map(Some).ok_or_else(|| invalid(key.to_string())),
            }
        };
        let (Some(after), Some(before)) = (date("after")?, date("before")?) else {
            return Err(unsupported("A window (after and before) is required."));
        };
        if before <= after || before - after > MAX_STRETCH {
            return Err(unsupported("The window must be of 500 days at most."));
        }
        let named: Option<Vec<String>> = match filter.get("inCalendars") {
            None | Some(Value::Null) => None,
            Some(Value::Array(ids)) if ids.len() <= MAX_NAMED => {
                Some(ids.iter().filter_map(Value::as_str).filter_map(|id| id.strip_prefix('C')).filter_map(unb64).collect())
            }
            Some(_) => return Err(invalid("inCalendars")),
        };
        let number = |key: &str| -> Outcome<Option<i64>> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(value) => value.as_i64().map(Some).ok_or_else(|| invalid(key.to_string())),
            }
        };
        let position = number("position")?.unwrap_or(0);
        let asked = number("limit")?;
        if asked.is_some_and(|limit| limit < 0) {
            return Err(invalid("limit"));
        }
        let limit = asked.map_or(MAX_LISTED, |limit| usize::try_from(limit).unwrap_or(usize::MAX).min(MAX_LISTED));
        let expand = args.get("expandRecurrences").and_then(Value::as_bool) == Some(true);
        let total = args.get("calculateTotal").and_then(Value::as_bool) == Some(true);
        // Every event and occurrence that overlaps the window, by its start.
        let mut found: Vec<(Timestamp, String)> = Vec::new();
        for event in &self.events {
            if named.as_ref().is_some_and(|named| !event.calendars.iter().any(|c| named.contains(c))) {
                continue;
            }
            let overlaps = |start: DateTime| {
                let (from, to) = self.instants(event, start);
                from < before && (to > after || (to == from && from >= after))
            };
            let mut occurrences = self.occurrences(event, before).into_iter().filter(|o| overlaps(o.start)).peekable();
            if expand && event.rule.is_some() {
                for occurrence in occurrences {
                    let id = Pim::occurrence_id(&event.raw, occurrence.recurrence.as_deref().unwrap_or_default());
                    found.push((self.instants(event, occurrence.start).0, id));
                }
            } else if let Some(first) = occurrences.peek() {
                found.push((self.instants(event, first.start).0, Pim::event_id(&event.raw)));
            }
        }
        found.sort();
        let count = found.len();
        let position = if position < 0 {
            count.saturating_sub(usize::try_from(position.unsigned_abs()).unwrap_or(usize::MAX))
        } else {
            usize::try_from(position).unwrap_or(usize::MAX)
        };
        let ids: Vec<&str> = found.iter().skip(position).take(limit).map(|(_, id)| id.as_str()).collect();
        let state = format!("q{}", ids.len().wrapping_mul(31).wrapping_add(count));
        let mut out = json!({"accountId": ACCOUNT, "queryState": state, "canCalculateChanges": false, "position": position, "ids": ids});
        if total {
            out["total"] = json!(count);
        }
        if asked.is_none_or(|asked| asked > MAX_LISTED as i64) {
            out["limit"] = json!(MAX_LISTED);
        }
        Ok(out)
    }

    /// `CalendarEvent/get`, the ids at most `most`.
    pub(super) fn event_get(&self, args: &Map<String, Value>, most: usize) -> Outcome<Value> {
        allow_only(args, &["accountId", "ids", "properties"])?;
        account(args)?;
        let ids: Vec<String> = match args.get("ids") {
            None | Some(Value::Null) => return Err(refuse("requestTooLarge", "Ask for the events by id (CalendarEvent/query).")),
            Some(Value::Array(ids)) if ids.len() > most => return Err(refuse("requestTooLarge", "too many ids")),
            Some(Value::Array(ids)) => ids.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            Some(_) => return Err(invalid("ids is a list")),
        };
        let properties: Option<Vec<String>> = match args.get("properties") {
            None | Some(Value::Null) => None,
            Some(Value::Array(names)) => {
                let names: Option<Vec<String>> = names.iter().map(|n| n.as_str().map(str::to_string)).collect();
                Some(names.ok_or_else(|| invalid("properties"))?)
            }
            Some(_) => return Err(invalid("properties")),
        };
        if ids.iter().any(|id| self.broken_events.contains(id)) {
            return Err(refuse("serverFail", "an event can't be read just now"));
        }
        let (mut list, mut not_found) = (Vec::new(), Vec::new());
        for id in &ids {
            match self.read(id) {
                Some(mut event) => {
                    if let (Some(keep), Value::Object(map)) = (&properties, &mut event) {
                        map.retain(|key, _| key == "id" || key == "@type" || keep.contains(key));
                    }
                    list.push(event);
                }
                None => not_found.push(id.clone()),
            }
        }
        Ok(json!({"accountId": ACCOUNT, "state": "0", "list": list, "notFound": not_found}))
    }

    /// The event an id of the API names, as it is served.
    fn read(&self, id: &str) -> Option<Value> {
        if let Some(raw) = id.strip_prefix('V').and_then(unb64) {
            let event = self.events.iter().find(|e| e.raw == raw)?;
            return Some(self.served(event, None));
        }
        let named = id.strip_prefix('O').and_then(unb64)?;
        let (series, start) = match serde_json::from_str::<Value>(&named).ok()? {
            Value::Array(parts) if parts.len() == 2 => (parts.first()?.as_str()?.to_string(), parts.get(1)?.as_str()?.to_string()),
            _ => return None,
        };
        let event = self.events.iter().find(|e| e.raw == series && e.rule.is_some())?;
        let clock = local_date_time(&start)?;
        let before = self.instants(event, clock).0.saturating_add(1);
        let occurrence = self.occurrences(event, before).into_iter().find(|o| o.recurrence.as_deref() == Some(start.as_str()))?;
        Some(self.served(event, Some(&occurrence)))
    }
}

/// A colour as the store keeps it, as `#rrggbb`: `#rgb`, `#rrggbb` and a few names; `None` for anything else.
fn hex_color(text: &str) -> Option<String> {
    crate::pim::jscalendar::color(text).map(|rgb| format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2))
}

#[cfg(test)]
mod tests;
