// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's calendars (JMAP for Calendars, on RFC 8620): what a job is asked to do and what it
//! brings back.
//!
//! The counterpart for calendars of [`JmapMailRequest`](super::JmapMailRequest) and
//! [`JmapMailResult`](super::JmapMailResult), and a job of its own beside a Send/Receive rather than a
//! part of one: the calendar is looked at on a clock of its own, trouble with it must not end a mail
//! run, and what is read is small enough to come back in one answer. Plain data, as the rest of
//! [`crate::sync`]: the window works the request out from the engine's public functions (the
//! calendars on screen, the series it expects), a host with a way to reach the server does the
//! talking, and the window takes the answer in through the engine (`sync.applyCalendars`,
//! `sync.applyEvents`), as it takes in what Microsoft Graph brings.
//!
//! What comes back is made of the downloads Graph's calendar brings already ([`CalendarsDownload`],
//! [`EventsDownload`]): the engine knows them, and needs nothing new to keep a JMAP account's calendars.
//!
//! Neither type shows the server's address, the account's address, a name or a title in debug output:
//! counts only. The secret that opens the server goes beside the request, as for a mail run, and is in
//! neither.

use mailcraft_time::Timestamp;
use serde::{Deserialize, Serialize};

use super::is_false;
use crate::Id;
use crate::remote::{CalendarAsk, CalendarsDownload, EventsDownload, MAX_REMOTE_CALENDARS};

/// Most calendars one job asks about, and most of the owner's addresses it is told: more are left out.
pub const MAX_PIM_ASKS: usize = 20;
pub const MAX_PIM_ADDRESSES: usize = 20;
/// Most problem lines one answer carries, and the longest of them, in characters.
pub const MAX_PIM_ERRORS: usize = 50;
const MAX_PIM_LINE: usize = 600;

/// What a calendar job of a [`crate::AccountKind::Jmap`] account is asked to do: list the account's
/// calendars when `lists` says so, and bring the events of the calendars in `asks` between `from` and
/// `to`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapPimRequest {
    pub account: Id,
    /// The account's address: what the answer is checked against.
    pub email: String,
    /// The address of the server's session resource ([`crate::JmapServer::session`]). It may carry
    /// the account's address, so it is never logged.
    pub session: String,
    /// The secret that goes beside this request is a bearer token; otherwise it is the account's
    /// password, sent with its login name ([`crate::JmapServer::bearer`]).
    #[serde(skip_serializing_if = "is_false")]
    pub bearer: bool,
    /// The owner's own addresses (the mailbox's accounts): the participant of an event who is one of
    /// them is the owner, which says who organizes a meeting and what the owner answered.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub me: Vec<String>,
    /// The zone the calendar is shown in, by its name (`Europe/Zurich`); empty when it has none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub zone: String,
    /// When the window asked (Unix seconds, by its clock): a stretch longer than the server expands
    /// repeating events over is cut around it, in proportion.
    pub now: Timestamp,
    /// The stretch to bring (Unix seconds, UTC): from `from` up to, not including, `to`.
    pub from: Timestamp,
    pub to: Timestamp,
    /// List the account's calendars too: the first job of an account, the wide stretch, and any job
    /// whose window does not know the main calendar's name on the server.
    #[serde(skip_serializing_if = "is_false")]
    pub lists: bool,
    /// What the server calls the account's main calendar, as the last list said; empty when no list
    /// has said it yet (the host then lists the calendars to find it).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub main: String,
    /// The calendars whose events are brought, the main one as an empty name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub asks: Vec<CalendarAsk>,
    /// A server whose session does not say what it writes of an event keeps only the basics: set by
    /// a host that knows the server for one that does (a hosting page's), never by the window.
    #[serde(skip_serializing_if = "is_false")]
    pub plain_unless_said: bool,
}

/// What the session of a JMAP account offers besides mail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PimOffers {
    /// Calendars (`urn:ietf:params:jmap:calendars`).
    #[serde(skip_serializing_if = "is_false")]
    pub calendars: bool,
    /// Address books (`urn:ietf:params:jmap:contacts`, RFC 9610).
    #[serde(skip_serializing_if = "is_false")]
    pub contacts: bool,
}

impl PimOffers {
    /// Neither calendars nor address books: there is nothing to ask this account for.
    pub fn nothing(self) -> bool {
        !self.calendars && !self.contacts
    }
}

/// What a calendar job of a JMAP account brought back: one answer, whole.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapPimResult {
    pub account: Id,
    /// The address of the account the job was for: an answer is not taken in by another account that
    /// happens to have the id.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// What the server's session offers, when it could be read: an account whose session offers
    /// neither calendars nor address books is not asked again by itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offers: Option<PimOffers>,
    /// The account's calendars, when the job listed them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendars: Option<CalendarsDownload>,
    /// The stretch of the main calendar, when it was asked for and could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<EventsDownload>,
    /// The same stretch of the other calendars asked for, one download each.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub more_events: Vec<EventsDownload>,
    /// Problems worth telling the owner; empty when all went well. They name no event.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// The server said it was busy and to stay away this many seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy_for: Option<u32>,
    /// The server turned the secret down (the host could not get another that it accepts).
    #[serde(skip_serializing_if = "is_false")]
    pub auth_refused: bool,
    /// The job was stopped (its `cancel` was set) before it was done.
    #[serde(skip_serializing_if = "is_false")]
    pub stopped: bool,
}

impl JmapPimResult {
    /// Cut to what a window takes in, as everything from outside is: as many downloads as an account has
    /// calendars, each event cut to size ([`crate::remote::RemoteEvent::tidy`]), the list tidied, and a few
    /// problem lines of one line each. Doing it twice changes nothing more.
    pub fn tidy(&mut self) {
        if let Some(list) = &mut self.calendars {
            list.tidy();
        }
        let downloads = self.events.iter_mut().chain(self.more_events.iter_mut());
        for download in downloads {
            download.events.truncate(crate::remote::MAX_EVENTS);
            download.events.iter_mut().for_each(crate::remote::RemoteEvent::tidy);
            download.gone.truncate(crate::remote::MAX_EVENTS);
        }
        self.more_events.truncate(MAX_REMOTE_CALENDARS);
        self.errors =
            std::mem::take(&mut self.errors).iter().map(|line| one_line(line)).filter(|line| !line.is_empty()).take(MAX_PIM_ERRORS).collect();
        self.busy_for = self.busy_for.map(|seconds| seconds.min(3_600));
    }

    /// How many events came, in every download.
    pub fn event_count(&self) -> usize {
        self.events.iter().chain(&self.more_events).map(|download| download.events.len()).sum()
    }
}

/// A problem line from outside: one line, no control characters, [`MAX_PIM_LINE`] characters at most.
fn one_line(text: &str) -> String {
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let short: String = flat.split_whitespace().collect::<Vec<&str>>().join(" ").chars().take(MAX_PIM_LINE).collect();
    short.trim_end().to_string()
}

/// Counts only: the server's address, the account's, the owner's and every calendar's name stay out of
/// debug output.
impl std::fmt::Debug for JmapPimRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let series: usize = self.asks.iter().map(|ask| ask.series.len()).sum();
        f.debug_struct("JmapPimRequest")
            .field("account", &self.account)
            .field("bearer", &self.bearer)
            .field("me", &self.me.len())
            .field("zone", &!self.zone.is_empty())
            .field("days", &(self.to.saturating_sub(self.from).max(0) / 86_400))
            .field("lists", &self.lists)
            .field("main", &!self.main.is_empty())
            .field("asks", &self.asks.len())
            .field("series", &series)
            .field("plain_unless_said", &self.plain_unless_said)
            .finish()
    }
}

/// Counts only, as the request's: no title, no name, no address of anybody.
impl std::fmt::Debug for JmapPimResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JmapPimResult")
            .field("account", &self.account)
            .field("offers", &self.offers)
            .field("calendars", &self.calendars.as_ref().map(|list| list.calendars.len()))
            .field("downloads", &(usize::from(self.events.is_some()) + self.more_events.len()))
            .field("events", &self.event_count())
            .field("errors", &self.errors.len())
            .field("busy_for", &self.busy_for)
            .field("auth_refused", &self.auth_refused)
            .field("stopped", &self.stopped)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::remote::{RemoteCalendar, RemoteEvent};
    use crate::{Address, Attendee};

    const ME: &str = "ada@example.org";
    const SESSION: &str = "https://mail.example.org/jmap/session?account=ada@example.org";

    fn round_trip<T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> String {
        let json = serde_json::to_string(value).unwrap();
        assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), value, "{json}");
        json
    }

    /// A request with something in every part of it, and the owner's address wherever it could be.
    fn request() -> JmapPimRequest {
        JmapPimRequest {
            account: 3,
            email: ME.into(),
            session: SESSION.into(),
            bearer: true,
            me: vec![ME.into(), "ada.lovelace@example.org".into()],
            zone: "Europe/Zurich".into(),
            now: 1_791_993_600,
            from: 1_791_993_600 - 31 * 86_400,
            to: 1_791_993_600 + 366 * 86_400,
            lists: true,
            main: format!("C-{ME}"),
            asks: vec![CalendarAsk { calendar: String::new(), series: vec!["V1".into()] }, CalendarAsk { calendar: "C2".into(), series: Vec::new() }],
            plain_unless_said: true,
        }
    }

    /// A result with something in every part, and the owner's address and private words in it.
    fn result() -> JmapPimResult {
        let event = RemoteEvent {
            id: "O1".into(),
            title: format!("Budget review with {ME}"),
            location: "Room 4".into(),
            start: 1_791_993_600,
            end: 1_791_997_200,
            attendees: vec![Attendee { address: Address::new("Bob", "bob@example.org"), ..Default::default() }],
            ..Default::default()
        };
        JmapPimResult {
            account: 3,
            email: ME.into(),
            offers: Some(PimOffers { calendars: true, contacts: true }),
            calendars: Some(CalendarsDownload {
                account: 3,
                email: ME.into(),
                complete: true,
                calendars: vec![
                    RemoteCalendar { id: "C1".into(), name: "Ada's calendar".into(), main: true, plain: true, ..Default::default() },
                    RemoteCalendar { id: "C2".into(), name: "Team".into(), read_only: true, plain: true, ..Default::default() },
                ],
            }),
            events: Some(EventsDownload { account: 3, email: ME.into(), complete: true, events: vec![event.clone()], ..Default::default() }),
            more_events: vec![EventsDownload { account: 3, calendar: "C2".into(), events: vec![event], ..Default::default() }],
            errors: vec![format!("Couldn't read a calendar of {ME}")],
            busy_for: Some(30),
            auth_refused: true,
            stopped: true,
        }
    }

    #[test]
    fn a_request_and_a_result_read_and_write_as_json() {
        let json = round_trip(&request());
        for part in [
            r#""lists":true"#,
            r#""main":"C-ada@example.org""#,
            r#""asks":[{"series":["V1"]},{"calendar":"C2","series":[]}]"#,
            r#""plainUnlessSaid":true"#,
        ] {
            assert!(json.contains(part), "{part} in {json}");
        }
        let json = round_trip(&result());
        for part in [r#""offers":{"calendars":true,"contacts":true}"#, r#""moreEvents":[{"#, r#""authRefused":true"#, r#""busyFor":30"#] {
            assert!(json.contains(part), "{part} in {json}");
        }
        // What is not said reads as nothing, and nothing writes next to nothing.
        assert_eq!(serde_json::from_str::<JmapPimRequest>("{}").unwrap(), JmapPimRequest::default());
        assert_eq!(serde_json::from_str::<JmapPimResult>("{}").unwrap(), JmapPimResult::default());
        assert_eq!(round_trip(&JmapPimResult::default()), r#"{"account":0}"#);
        assert_eq!(round_trip(&PimOffers::default()), "{}");
        assert!(PimOffers::default().nothing() && !PimOffers { calendars: true, contacts: false }.nothing());
    }

    #[test]
    fn a_jmap_pim_request_prints_counts_only() {
        let (request, result) = (request(), result());
        for shown in [format!("{request:?}"), format!("{request:#?}"), format!("{result:?}"), format!("{result:#?}")] {
            for secret in ["@", "example.org", "jmap/session", "Ada", "Bob", "Budget", "Room", "Team", "Zurich", "C-", "V1"] {
                assert!(!shown.contains(secret), "{secret} in {shown}");
            }
        }
        assert_eq!(
            format!("{request:?}"),
            "JmapPimRequest { account: 3, bearer: true, me: 2, zone: true, days: 397, lists: true, main: true, asks: 2, series: 1, plain_unless_said: true }"
        );
        assert_eq!(
            format!("{result:?}"),
            "JmapPimResult { account: 3, offers: Some(PimOffers { calendars: true, contacts: true }), calendars: Some(2), downloads: 2, events: 2, errors: 1, busy_for: Some(30), auth_refused: true, stopped: true }"
        );
    }

    #[test]
    fn a_result_round_trips_and_a_hostile_one_is_tidied() {
        let mut hostile = result();
        // Two main calendars, an id that can be none, an event with a title of a thousand lines and an end before
        // its start, more downloads than any account has calendars, and problem lines of every shape.
        if let Some(list) = &mut hostile.calendars {
            list.calendars.push(RemoteCalendar { id: "C3".into(), name: "Second\nmain".into(), main: true, ..Default::default() });
            list.calendars.push(RemoteCalendar { id: "two words".into(), ..Default::default() });
        }
        if let Some(main) = &mut hostile.events {
            main.events.push(RemoteEvent { id: "O2".into(), title: "line\n".repeat(1_000), start: 500, end: 100, ..Default::default() });
        }
        hostile.more_events = (0..2 * MAX_REMOTE_CALENDARS).map(|n| EventsDownload { calendar: format!("C{n}"), ..Default::default() }).collect();
        hostile.errors = (0..200).map(|n| format!("line {n}\r\nwith\u{0}control  and   blanks")).chain([String::new(), "\n\t".into()]).collect();
        hostile.busy_for = Some(u32::MAX);
        hostile.tidy();
        let list = hostile.calendars.as_ref().unwrap();
        assert_eq!(list.calendars.iter().filter(|c| c.main).count(), 1, "one main calendar at most");
        assert!(list.calendars.iter().all(|c| !c.id.contains(' ') && !c.name.contains('\n')));
        let main = hostile.events.as_ref().unwrap();
        let cut = main.events.iter().find(|e| e.id == "O2").unwrap();
        assert!(!cut.title.contains('\n') && cut.title.chars().count() <= 500 && cut.end >= cut.start);
        assert_eq!(hostile.more_events.len(), MAX_REMOTE_CALENDARS);
        assert_eq!(hostile.errors.len(), MAX_PIM_ERRORS);
        assert_eq!(hostile.errors[0], "line 0 with control and blanks");
        assert_eq!(hostile.busy_for, Some(3_600));
        // Tidied once is tidied: it reads and writes as it is.
        let once = hostile.clone();
        hostile.tidy();
        assert_eq!(hostile, once);
        round_trip(&hostile);
    }

    #[test]
    fn a_calendar_from_before_plain_reads_as_before() {
        // Written before there was the flag: a server that keeps everything, and nothing about it is written back.
        let old: crate::Calendar = serde_json::from_str(r#"{"id":2,"name":"Calendar","visible":true,"origin":{"account":3}}"#).unwrap();
        assert!(!old.plain);
        assert!(!serde_json::to_string(&old).unwrap().contains("plain"));
        let plain = crate::Calendar { plain: true, ..old };
        assert!(round_trip(&plain).contains(r#""plain":true"#));
        let listed: RemoteCalendar = serde_json::from_str(r#"{"id":"C1","name":"Team","readOnly":true}"#).unwrap();
        assert!(!listed.plain && listed.read_only);
        assert!(round_trip(&RemoteCalendar { plain: true, ..listed }).contains(r#""plain":true"#));
    }

    proptest! {
        #[test]
        fn hostile_pim_results_never_panic(text in any::<String>(), n in any::<i64>(), m in any::<i64>(), busy in any::<u32>()) {
            let event = RemoteEvent { id: text.clone(), title: text.clone(), description: text.clone(), start: n, end: m, ..Default::default() };
            let mut result = JmapPimResult {
                email: text.clone(),
                calendars: Some(CalendarsDownload {
                    calendars: vec![RemoteCalendar { id: text.clone(), name: text.clone(), main: true, ..Default::default() }],
                    ..Default::default()
                }),
                events: Some(EventsDownload { calendar: text.clone(), from: n, to: m, events: vec![event.clone()], gone: vec![text.clone()], ..Default::default() }),
                more_events: vec![EventsDownload { events: vec![event], ..Default::default() }],
                errors: vec![text.clone(), text],
                busy_for: Some(busy),
                ..Default::default()
            };
            result.tidy();
            prop_assert!(result.errors.iter().all(|line| !line.chars().any(char::is_control) && line.chars().count() <= MAX_PIM_LINE));
            prop_assert!(result.events.iter().flat_map(|d| &d.events).all(|e| e.end >= e.start));
            let once = result.clone();
            result.tidy();
            prop_assert_eq!(&result, &once);
            let json = serde_json::to_string(&result).unwrap();
            prop_assert_eq!(serde_json::from_str::<JmapPimResult>(&json).unwrap(), result);
        }
    }
}
