// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! `CalendarEvent/set` as a server that keeps only the basics of an event takes it, as strictly as it does.
//!
//! - **One call** makes, changes and deletes at most as many events as the session says (`requestTooLarge`), and at
//!   most [`MAX_EVENTS_PER_SET`] of them are written: the others are answered `rateLimit`, to be tried again in a
//!   minute. `ifInState` is refused. Creates go first, then changes, then deletions, each answered on its own.
//! - **What is written** is a whole property, never a part of one (a key with a `/` is `invalidProperties`), of the
//!   properties it writes ([`WRITABLE_EVENT`]); what it sets or keeps for itself ([`SERVER_SET`]) may be sent back as
//!   it was read and is left as it is; anything else is `invalidProperties` ("Not written here"). The organizer and
//!   the draft state are the event's own: another organizer is `forbidden`, another draft state `invalidProperties`.
//!   An event goes in calendars it has whose events the account may all write (`forbidden` otherwise), in the
//!   default one when none is named.
//! - **A new event** needs its start and its length; it is known again by the `uid` it was made with (an answer lost
//!   on the way makes no second one), and is never one occurrence of a series. Its organizer is the person.
//! - **A change** of a series or of an event of its own replaces the properties it names. A change of one occurrence
//!   (by its `O` id) is that occurrence's alone: what the series has instead, there, and never another zone or another
//!   calendar for it (those two are dropped). A deletion of one occurrence leaves it out of its series.
//! - **Mail.** The server writes to the people of an event only when the call asks for it (`sendSchedulingMessages`)
//!   and the person organizes the event and it is not a draft: invitations, updates and cancellations
//!   ([`Pim::scheduled`]). A change that is nothing but the person's own answer to somebody else's invitation, asked
//!   to be sent from a calendar that sends answers, is that answer: kept on the person's copy and told to the
//!   organizer ([`Pim::tells_the_organizer`]); one occurrence of somebody else's invitation is not answered alone
//!   (`forbidden`, "answered as a whole").
//! - **Trouble on demand.** An event (by its id, or a new one by its `uid`) answered with the SetError a test names
//!   ([`Pim::refused_events`]); events whose time a change says it took and doesn't keep ([`Pim::keeps_old_times`]).

use mailcraft_time::Zone;
use serde_json::{Map, Value, json};

use super::super::{ACCOUNT, Outcome, USER, account, allow_only, invalid, refuse};
use super::{Event, Pim, WRITABLE_EVENT, unb64};
use crate::pim::{duration, local_date_time};

/// What such a server sets or keeps for itself: a client may send it back as it read it.
pub const SERVER_SET: [&str; 18] = [
    "@type",
    "id",
    "uid",
    "created",
    "updated",
    "sequence",
    "prodId",
    "method",
    "utcStart",
    "utcEnd",
    "baseEventId",
    "recurrenceIdTimeZone",
    "isOrigin",
    "mayInviteSelf",
    "mayInviteOthers",
    "hideAttendees",
    "organizerCalendarAddress",
    "isDraft",
];
/// Most events written in one call.
pub const MAX_EVENTS_PER_SET: usize = 50;
/// The answers a person gives an invitation.
const ANSWERS: [&str; 3] = ["accepted", "tentative", "declined"];
/// The longest `uid` kept, and most changes kept as they came.
const MAX_UID: usize = 255;
const MAX_WRITTEN: usize = 10_000;

/// Mail the server sent about an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheduled {
    /// `invitation`, `update`, `cancellation` or `reply`.
    pub what: &'static str,
    /// The event, by what the store calls it, and the occurrence, by its start on the series' clock, for one alone.
    pub event: String,
    pub occurrence: Option<String>,
    /// Who it went to.
    pub to: Vec<String>,
}

/// A SetError.
fn set_error(kind: &str, description: impl Into<String>) -> Value {
    json!({"type": kind, "description": description.into()})
}

/// An address without its `mailto:`, in lower case.
fn address(value: Option<&Value>) -> String {
    let text = value.and_then(Value::as_str).unwrap_or_default().trim();
    let bare = text.get(..7).filter(|scheme| scheme.eq_ignore_ascii_case("mailto:")).map_or(text, |_| text.get(7..).unwrap_or_default());
    bare.trim().to_ascii_lowercase()
}

/// A participant's address, as the server reads it: its calendar address, else its email, else where invitations go,
/// else its scheduling id.
fn participant_address(entry: &Map<String, Value>) -> String {
    ["calendarAddress", "email"]
        .iter()
        .map(|key| address(entry.get(*key)))
        .chain([address(entry.get("sendTo").and_then(|to| to.get("imip"))), address(entry.get("scheduleId"))])
        .find(|found| !found.is_empty())
        .unwrap_or_default()
}

/// A participant as it is compared when telling whether a change is only an answer: without what the server works
/// out again (where invitations go, its scheduling id, whether a reply is expected), its address and its answer in
/// lower case, its roles those that are on.
fn compared(entry: &Value) -> Value {
    let Some(entry) = entry.as_object() else { return Value::Null };
    let mut out: Map<String, Value> = entry
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "sendTo" | "scheduleId" | "expectReply" | "calendarAddress" | "email"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    out.insert("address".into(), json!(participant_address(entry)));
    let mut roles: Vec<&str> = entry
        .get("roles")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, on)| on.as_bool() == Some(true))
        .map(|(role, _)| role.as_str())
        .collect();
    roles.sort_unstable();
    out.insert("roles".into(), json!(roles));
    let status = entry.get("participationStatus").and_then(Value::as_str).map(str::to_ascii_lowercase);
    out.insert("participationStatus".into(), json!(status));
    Value::Object(out)
}

/// The writable properties a create or a change names, held to what the server writes, and the calendars it names
/// (by what the store calls them).
struct Fields {
    named: Map<String, Value>,
    calendars: Option<Vec<String>>,
}

/// What a change is about: an event (its place in the store), or one occurrence of a series (by its start on the
/// series' clock).
struct Addressed {
    at: usize,
    occurrence: Option<String>,
}

impl Pim {
    /// `CalendarEvent/set`, with at most `most` objects in a call.
    pub(in crate::standin) fn event_set(&mut self, args: &Map<String, Value>, most: usize) -> Outcome<Value> {
        allow_only(args, &["accountId", "ifInState", "create", "update", "destroy", "sendSchedulingMessages"])?;
        account(args)?;
        let object = |key: &str| match args.get(key) {
            None | Some(Value::Null) => Ok(Map::new()),
            Some(Value::Object(map)) => Ok(map.clone()),
            Some(_) => Err(invalid("create, update, destroy")),
        };
        let (creates, updates) = (object("create")?, object("update")?);
        let destroys: Vec<Value> = match args.get("destroy") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(ids)) => ids.clone(),
            Some(_) => return Err(invalid("create, update, destroy")),
        };
        if creates.len().saturating_add(updates.len()).saturating_add(destroys.len()) > most {
            return Err(refuse("requestTooLarge", "maxObjectsInSet"));
        }
        if !matches!(args.get("ifInState"), None | Some(Value::Null)) {
            return Err(invalid("ifInState"));
        }
        let notify = args.get("sendSchedulingMessages") == Some(&Value::Bool(true));
        let mut budget = MAX_EVENTS_PER_SET;
        let mut spend = || {
            if budget == 0 {
                return false;
            }
            budget -= 1;
            true
        };
        let limited = || set_error("rateLimit", "Too many events at once: try again in a minute.");
        let (mut created, mut not_created) = (Map::new(), Map::new());
        for (key, event) in &creates {
            match if spend() { self.create(event, notify) } else { Err(limited()) } {
                Ok((id, uid)) => {
                    created.insert(key.clone(), json!({"id": id, "uid": uid}));
                }
                Err(error) => {
                    not_created.insert(key.clone(), error);
                }
            }
        }
        let (mut updated, mut not_updated) = (Map::new(), Map::new());
        for (id, patch) in &updates {
            match if spend() { self.update(id, patch, notify) } else { Err(limited()) } {
                Ok(()) => {
                    updated.insert(id.clone(), Value::Null);
                }
                Err(error) => {
                    not_updated.insert(id.clone(), error);
                }
            }
        }
        let (mut destroyed, mut not_destroyed) = (Vec::new(), Map::new());
        for id in destroys.iter().filter_map(Value::as_str) {
            match if spend() { self.destroy(id, notify) } else { Err(limited()) } {
                Ok(()) => destroyed.push(json!(id)),
                Err(error) => {
                    not_destroyed.insert(id.to_string(), error);
                }
            }
        }
        Ok(json!({
            "accountId": ACCOUNT,
            "oldState": "0",
            "newState": "0",
            "created": created,
            "updated": updated,
            "destroyed": destroyed,
            "notCreated": not_created,
            "notUpdated": not_updated,
            "notDestroyed": not_destroyed,
        }))
    }

    /// The person's own addresses, in lower case: those a test names, else the stand-in's user.
    fn own(&self) -> Vec<String> {
        let own: Vec<String> = self.own.iter().map(|a| a.trim().to_ascii_lowercase()).filter(|a| !a.is_empty()).collect();
        if own.is_empty() { vec![USER.to_string()] } else { own }
    }

    /// The writable properties of an event or a change, checked as the server checks them.
    fn fields(&self, event: &Map<String, Value>) -> Result<Fields, Value> {
        let wrong = |why: &str| set_error("invalidProperties", why);
        if event.keys().any(|key| key.contains('/')) {
            return Err(wrong("A whole property replaces it: no patch of a part."));
        }
        let mut unknown: Vec<&str> =
            event.keys().map(String::as_str).filter(|key| !WRITABLE_EVENT.contains(key) && !SERVER_SET.contains(key)).collect();
        if !unknown.is_empty() {
            unknown.sort_unstable();
            let listed: Vec<String> = unknown.iter().take(10).map(|key| key.chars().take(40).collect()).collect();
            return Err(wrong(&format!("Not written here: {}", listed.join(", "))));
        }
        let mut named = Map::new();
        let mut calendars = None;
        for (key, value) in event.iter().filter(|(key, _)| WRITABLE_EVENT.contains(&key.as_str())) {
            let fits = match key.as_str() {
                "calendarIds" => {
                    let raws: Option<Vec<String>> = value.as_object().filter(|ids| ids.values().any(|on| on.as_bool() == Some(true))).map(|ids| {
                        ids.iter().filter(|(_, on)| on.as_bool() == Some(true)).filter_map(|(id, _)| id.strip_prefix('C').and_then(unb64)).collect()
                    });
                    let named_count = value.as_object().map_or(0, |ids| ids.values().filter(|on| on.as_bool() == Some(true)).count());
                    let raws =
                        raws.filter(|raws| raws.len() == named_count && raws.iter().all(|raw| self.calendar_list.iter().any(|c| &c.raw == raw)));
                    let Some(raws) = raws else { return Err(wrong("An event goes in one of your calendars.")) };
                    if !raws.iter().all(|raw| self.calendar_list.iter().any(|c| &c.raw == raw && c.write_all)) {
                        return Err(set_error("forbidden", "Your rights do not allow it."));
                    }
                    calendars = Some(raws);
                    continue;
                }
                "start" => value.as_str().is_some_and(|start| local_date_time(start).is_some()),
                "duration" => value.as_str().is_some_and(|length| duration(length).is_some()),
                "timeZone" => value.is_null() || value.as_str().is_some_and(|zone| Zone::named(zone).is_some()),
                "title" | "description" => value.is_null() || value.is_string(),
                "showWithoutTime" | "useDefaultAlerts" => value.is_boolean(),
                "status" => value.is_null() || value.as_str().is_some_and(|s| matches!(s, "confirmed" | "tentative" | "cancelled")),
                "privacy" => value.as_str().is_some_and(|s| matches!(s, "public" | "private" | "secret")),
                "freeBusyStatus" => value.as_str().is_some_and(|s| matches!(s, "free" | "busy")),
                "recurrenceRule" => value.is_null() || value.get("frequency").and_then(Value::as_str).is_some(),
                _ => value.is_null() || value.is_object(),
            };
            if !fits {
                return Err(wrong(&format!("{key} can't be written as it is.")));
            }
            let mut value = value.clone();
            // People without an address are no participants.
            if key == "participants"
                && let Value::Object(people) = &mut value
            {
                people.retain(|_, entry| entry.as_object().is_some_and(|entry| !participant_address(entry).is_empty()));
            }
            named.insert(key.clone(), value);
        }
        Ok(Fields { named, calendars })
    }

    /// The organizer and the draft state are the event's own (`stored`: the event as it is; `None` for a new one).
    fn keep_own(&self, given: &Map<String, Value>, stored: Option<&Map<String, Value>>) -> Result<(), Value> {
        if let (Some(organizer), Some(stored)) = (given.get("organizerCalendarAddress"), stored)
            && address(Some(organizer)) != address(stored.get("organizerCalendarAddress"))
        {
            return Err(set_error("forbidden", "The organizer of an event is not changed here."));
        }
        let draft = |of: Option<&Map<String, Value>>| of.and_then(|of| of.get("isDraft")).and_then(Value::as_bool).unwrap_or(false);
        if given.contains_key("isDraft") && draft(Some(given)) != draft(stored) {
            return Err(set_error("invalidProperties", "A draft is sent from the desk."));
        }
        Ok(())
    }

    /// Whether the person organizes an event: its organizer is one of their addresses, or it names none.
    fn organizes(&self, props: &Map<String, Value>) -> bool {
        let organizer = address(props.get("organizerCalendarAddress"));
        organizer.is_empty() || self.own().contains(&organizer)
    }

    /// The people an event's mail goes to: its participants but the person.
    fn guests(&self, props: &Map<String, Value>) -> Vec<String> {
        let own = self.own();
        let organizer = address(props.get("organizerCalendarAddress"));
        props
            .get("participants")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(_, entry)| entry.as_object().map(participant_address))
            .filter(|who| !who.is_empty() && !own.contains(who) && *who != organizer)
            .collect()
    }

    /// A refusal a test asked for, of the event `id` (or a new one's uid).
    fn refused_on_demand(&self, id: &str) -> Result<(), Value> {
        match self.refused_events.get(id) {
            Some(kind) => Err(set_error(kind, "refused on demand")),
            None => Ok(()),
        }
    }

    /// A new event: its id and its uid.
    fn create(&mut self, event: &Value, notify: bool) -> Result<(String, String), Value> {
        let Some(event) = event.as_object() else { return Err(set_error("invalidProperties", "An event must be an object.")) };
        if event.get("recurrenceId").is_some_and(|at| !at.is_null()) {
            return Err(set_error("invalidProperties", "An occurrence is changed through its series, not created."));
        }
        self.keep_own(event, None)?;
        let organizer = address(event.get("organizerCalendarAddress"));
        if !organizer.is_empty() && !self.own().contains(&organizer) {
            return Err(set_error("forbidden", "An event is organized by one of your own addresses."));
        }
        let fields = self.fields(event)?;
        let said = |key: &str| fields.named.get(key).and_then(Value::as_str).map(str::to_string);
        let (Some(start), Some(length)) = (said("start"), said("duration")) else {
            return Err(set_error("invalidProperties", "An event needs its start and its duration."));
        };
        let uid: String = event.get("uid").and_then(Value::as_str).unwrap_or_default().chars().take(MAX_UID).collect();
        // Sent again, its answer lost on the way: the event it made.
        if let Some(raw) = self.made.get(&uid).filter(|_| !uid.is_empty())
            && self.events.iter().any(|e| &e.raw == raw)
        {
            return Ok((Pim::event_id(raw), uid));
        }
        self.refused_on_demand(&uid)?;
        let calendars = match fields.calendars {
            Some(calendars) => calendars,
            None => match self.default_calendar().filter(|raw| self.calendar_list.iter().any(|c| &c.raw == raw && c.write_all)) {
                Some(raw) => vec![raw.to_string()],
                None => return Err(set_error("forbidden", "Your rights do not allow it.")),
            },
        };
        self.made_count = self.made_count.saturating_add(1);
        let raw = format!("made-{}", self.made_count);
        let uid = if uid.is_empty() { format!("{raw}@standin.example") } else { uid };
        let mut made = Event {
            raw: raw.clone(),
            calendars,
            start,
            zone: None,
            duration: length,
            all_day: false,
            rule: None,
            excluded: Vec::new(),
            overrides: Default::default(),
            props: Map::new(),
        };
        made.props.insert("uid".into(), json!(uid));
        made.props.insert("organizerCalendarAddress".into(), json!(format!("mailto:{}", self.own().first().cloned().unwrap_or_default())));
        apply(&mut made, &fields.named);
        if notify {
            let to = self.guests(&made.props);
            if !to.is_empty() {
                self.scheduled.push(Scheduled { what: "invitation", event: raw.clone(), occurrence: None, to });
            }
        }
        self.events.push(made);
        self.made.insert(uid.clone(), raw.clone());
        Ok((Pim::event_id(&raw), uid))
    }

    /// The event an id of the API names for a change: `notFound` for one it doesn't have now.
    fn addressed(&self, id: &str) -> Result<Addressed, Value> {
        let missing = || set_error("notFound", "");
        if let Some(raw) = id.strip_prefix('V').and_then(unb64) {
            let at = self.events.iter().position(|e| e.raw == raw).ok_or_else(missing)?;
            return Ok(Addressed { at, occurrence: None });
        }
        // One occurrence: still one of its series now.
        let served = self.read(id).ok_or_else(missing)?;
        let series = served.get("baseEventId").and_then(Value::as_str).and_then(|base| base.strip_prefix('V')).and_then(unb64).ok_or_else(missing)?;
        let at = self.events.iter().position(|e| e.raw == series).ok_or_else(missing)?;
        let occurrence = served.get("recurrenceId").and_then(Value::as_str).map(str::to_string).ok_or_else(missing)?;
        Ok(Addressed { at, occurrence: Some(occurrence) })
    }

    /// An event of a calendar the account may not write all of is not its to change.
    fn check_writable(&self, event: &Event) -> Result<(), Value> {
        let known: Vec<&super::Calendar> = event.calendars.iter().filter_map(|raw| self.calendar_list.iter().find(|c| &c.raw == raw)).collect();
        if known.is_empty() || known.len() != event.calendars.len() {
            return Err(set_error("notFound", ""));
        }
        if !known.iter().all(|c| c.write_all) {
            return Err(set_error("forbidden", "Your rights do not allow it."));
        }
        Ok(())
    }

    /// The person's answer that a change gives, when it is nothing else (see the module documentation): accepted,
    /// tentative or declined. `now` is the event (or the occurrence) as the server has it.
    fn answer_of(&self, now: &Map<String, Value>, patch: &Map<String, Value>) -> Option<String> {
        if patch.keys().any(|key| key != "participants" && !SERVER_SET.contains(&key.as_str())) {
            return None;
        }
        let given = patch.get("participants")?.as_object()?;
        let kept = now.get("participants")?.as_object()?;
        let own = self.own();
        let first =
            kept.iter().find(|(_, entry)| entry.as_object().is_some_and(|entry| own.contains(&participant_address(entry)))).map(|(key, _)| key)?;
        let status = given.get(first)?.get("participationStatus")?.as_str()?.to_ascii_lowercase();
        let was = kept.get(first)?.get("participationStatus").and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase();
        if !ANSWERS.contains(&status.as_str()) || status == was || given.len() != kept.len() {
            return None;
        }
        let without_status = |entry: &Value| {
            let mut entry = compared(entry);
            if let Some(map) = entry.as_object_mut() {
                map.remove("participationStatus");
            }
            entry
        };
        for (key, entry) in kept {
            let other = given.get(key)?;
            let same = if key == first { without_status(entry) == without_status(other) } else { compared(entry) == compared(other) };
            if !same {
                return None;
            }
        }
        Some(status)
    }

    /// Whether the person's answers go to organizers from an event's calendars.
    fn sends_answers(&self, event: &Event) -> bool {
        !event.calendars.is_empty()
            && event.calendars.iter().all(|raw| self.calendar_list.iter().any(|c| &c.raw == raw && c.sends_answers && (c.write_all || c.write_own)))
    }

    /// A change of the event (or occurrence) `id`.
    fn update(&mut self, id: &str, patch: &Value, notify: bool) -> Result<(), Value> {
        if self.written.len() < MAX_WRITTEN {
            self.written.push((id.chars().take(MAX_UID).collect(), patch.clone()));
        }
        let addressed = self.addressed(id)?;
        self.refused_on_demand(id)?;
        let stored = self.events.get(addressed.at).cloned().ok_or_else(|| set_error("notFound", ""))?;
        self.check_writable(&stored)?;
        let Some(patch) = patch.as_object() else { return Err(set_error("invalidProperties", "A change must be an object.")) };
        self.keep_own(patch, Some(&stored.props))?;
        let mut fields = self.fields(patch)?;
        let draft = stored.props.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
        let asked = notify && !draft;
        let organizes = asked && self.organizes(&stored.props);
        // As the server has it now: the occurrence's own, for one.
        let now = match self.read(id) {
            Some(Value::Object(now)) => now,
            _ => stored.props.clone(),
        };
        let answer = if asked && !organizes && self.sends_answers(&stored) { self.answer_of(&now, patch) } else { None };
        if answer.is_some() && addressed.occurrence.is_some() {
            return Err(set_error("forbidden", "An invitation is answered as a whole: answer its series."));
        }
        if let Some(answer) = answer {
            let own = self.own();
            let organizer = address(stored.props.get("organizerCalendarAddress"));
            if let Some(event) = self.events.get_mut(addressed.at)
                && let Some(Value::Object(people)) = event.props.get_mut("participants")
                && let Some(Value::Object(entry)) =
                    people.values_mut().find(|entry| entry.as_object().is_some_and(|entry| own.contains(&participant_address(entry))))
            {
                entry.insert("participationStatus".into(), json!(answer));
            }
            if self.tells_the_organizer && !organizer.is_empty() {
                self.scheduled.push(Scheduled { what: "reply", event: stored.raw.clone(), occurrence: None, to: vec![organizer] });
            }
            return Ok(());
        }
        let old_times = self.keeps_old_times.contains(id);
        if old_times {
            for key in ["start", "duration", "timeZone", "showWithoutTime"] {
                fields.named.remove(key);
            }
        }
        let guests = self.guests(&stored.props);
        match &addressed.occurrence {
            Some(at) => {
                // Never another zone or another calendar for one occurrence.
                fields.named.remove("timeZone");
                let Some(event) = self.events.get_mut(addressed.at) else { return Err(set_error("notFound", "")) };
                let patch = event.overrides.entry(at.clone()).or_default();
                for (key, value) in fields.named {
                    patch.insert(key, value);
                }
            }
            None => {
                let Some(event) = self.events.get_mut(addressed.at) else { return Err(set_error("notFound", "")) };
                apply(event, &fields.named);
                if let Some(calendars) = fields.calendars {
                    event.calendars = calendars;
                }
            }
        }
        if organizes && !guests.is_empty() {
            self.scheduled.push(Scheduled { what: "update", event: stored.raw.clone(), occurrence: addressed.occurrence.clone(), to: guests });
        }
        Ok(())
    }

    /// The deletion of the event (or occurrence) `id`.
    fn destroy(&mut self, id: &str, notify: bool) -> Result<(), Value> {
        let addressed = self.addressed(id)?;
        self.refused_on_demand(id)?;
        let stored = self.events.get(addressed.at).cloned().ok_or_else(|| set_error("notFound", ""))?;
        self.check_writable(&stored)?;
        let draft = stored.props.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
        let guests = self.guests(&stored.props);
        if notify && !draft && self.organizes(&stored.props) && !guests.is_empty() {
            self.scheduled.push(Scheduled { what: "cancellation", event: stored.raw.clone(), occurrence: addressed.occurrence.clone(), to: guests });
        }
        match addressed.occurrence {
            Some(at) => {
                if let Some(event) = self.events.get_mut(addressed.at) {
                    event.overrides.remove(&at);
                    if !event.excluded.contains(&at) {
                        event.excluded.push(at);
                    }
                }
            }
            None => {
                self.events.remove(addressed.at);
            }
        }
        Ok(())
    }
}

/// Put the properties a create or a change names into the store's event: its times and its rule where the store
/// keeps them, the rest as they are (null takes one away).
fn apply(event: &mut Event, named: &Map<String, Value>) {
    for (key, value) in named {
        match key.as_str() {
            "start" => event.start = value.as_str().unwrap_or_default().to_string(),
            "duration" => event.duration = value.as_str().unwrap_or_default().to_string(),
            "timeZone" => event.zone = value.as_str().map(str::to_string),
            "showWithoutTime" => event.all_day = value.as_bool().unwrap_or(false),
            "recurrenceRule" => {
                event.rule = (!value.is_null()).then(|| value.clone());
                if event.rule.is_none() {
                    event.excluded.clear();
                    event.overrides.clear();
                }
            }
            "calendarIds" => {}
            _ if value.is_null() => {
                event.props.remove(key);
            }
            _ => {
                event.props.insert(key.clone(), value.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests;
