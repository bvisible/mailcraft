// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's calendars (JMAP for Calendars, on RFC 8620) and address books (JMAP for Contacts,
//! RFC 9610): what a job is asked to do and what it brings back.
//!
//! The counterpart for calendars and contacts of [`JmapMailRequest`](super::JmapMailRequest) and
//! [`JmapMailResult`](super::JmapMailResult), and a job of its own beside a Send/Receive rather than a
//! part of one: they are looked at on a clock of their own, trouble with them must not end a mail
//! run, and what is read is small enough to come back in one answer. Plain data, as the rest of
//! [`crate::sync`]: the window works the request out from the engine's public functions (the
//! calendars on screen, the series it expects), a host with a way to reach the server does the
//! talking, and the window takes the answer in through the engine (`sync.applyCalendars`,
//! `sync.applyEvents`, `sync.applyBooks`, `sync.applyContacts`), as it takes in what Microsoft Graph
//! brings.
//!
//! What comes back is made of the downloads Graph's calendar and contacts bring already
//! ([`CalendarsDownload`], [`EventsDownload`], [`ContactsDownload`]) and of the address books
//! ([`BooksDownload`]): the engine knows them all.
//!
//! **Reading the contacts again costs one question when nothing changed.** A server says a state of
//! its contacts with every query of them (`queryState`). The window keeps the state of the last read
//! that brought them all and hands it back with the next request ([`JmapPimRequest::contacts_state`]);
//! the host asks for that state alone first, and reads everything only when it changed or is an hour
//! old ([`CONTACTS_STATE_FOR`]).
//!
//! **What was changed here goes first.** A request may carry the changes made here to the account's calendars
//! that its server is yet to be told ([`JmapPimRequest::changes`], as the engine says them: `sync.eventChanges`);
//! the host writes them before it reads, so that what is read shows them, and says what the server made of each
//! ([`JmapPimResult::uploaded`], for `sync.applyEventChanges`). Likewise for the contacts of its address books
//! ([`JmapPimRequest::contact_changes`], as `sync.contactChanges` says them; [`JmapPimResult::contacts_uploaded`], for
//! `sync.applyContactChanges`): written before they are read.
//!
//! Neither type shows the server's address, the account's address, a name or a title in debug output:
//! counts only. The secret that opens the server goes beside the request, as for a mail run, and is in
//! neither.

use mailcraft_time::Timestamp;
use serde::{Deserialize, Serialize};

use super::is_false;
use crate::Id;
use crate::book::{BooksDownload, ContactsUpload, ContactsUploaded, MAX_CONTACT_OPS};
use crate::remote::{
    CalendarAsk, CalendarsDownload, ContactsDownload, EventsDownload, EventsUpload, EventsUploaded, MAX_CONTACTS, MAX_EVENT_OPS,
    MAX_REMOTE_CALENDARS, clean_key,
};

/// How long the state of a whole read of the contacts is trusted, in seconds: after that they are read again in
/// full, whatever the server says of them (a state can stay the same while what one may read of them changes).
pub const CONTACTS_STATE_FOR: i64 = 3_600;
/// The longest state of the contacts that is kept, in bytes: a longer one is none.
const MAX_STATE: usize = 256;

/// Most calendars one job asks about, and most of the owner's addresses it is told: more are left out.
pub const MAX_PIM_ASKS: usize = 20;
pub const MAX_PIM_ADDRESSES: usize = 20;
/// Most problem lines one answer carries, and the longest of them, in characters.
pub const MAX_PIM_ERRORS: usize = 50;
const MAX_PIM_LINE: usize = 600;
/// The longest address of the person's identities that is taken, in characters.
const MAX_ADDRESS: usize = 1_000;

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
    /// Bring the account's address books and their contacts too.
    #[serde(skip_serializing_if = "is_false")]
    pub contacts: bool,
    /// The state of the contacts at the last read that brought them all, as the server said it; empty when the
    /// window holds none. While the server still says it, nothing of the contacts is read but that.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub contacts_state: String,
    /// When that read was (Unix seconds, by the window's clock, as `now`): a state [`CONTACTS_STATE_FOR`] old or more
    /// is not asked about, and everything is read.
    pub contacts_read: Timestamp,
    /// A server whose session does not say what it writes of an event or a card keeps only the basics:
    /// set by a host that knows the server for one that does (a hosting page's), never by the window.
    #[serde(skip_serializing_if = "is_false")]
    pub plain_unless_said: bool,
    /// What was made, changed and deleted here of the account's calendars that its server is yet to be told, oldest
    /// first: written before anything is read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<EventsUpload>,
    /// What was made, changed and deleted here of the contacts of the account's address books that its server is yet
    /// to be told, oldest first: written before the contacts are read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_changes: Option<ContactsUpload>,
    /// The person's notes, when the session offers them: read them (or ask whether the state held is still theirs),
    /// after writing what was changed of them here ([`crate::note::NotesAsk`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<crate::note::NotesAsk>,
}

impl JmapPimRequest {
    /// The state of the contacts that the host may ask about instead of reading them all: the one held, while it is
    /// less than [`CONTACTS_STATE_FOR`] old by `now` (and not from after it).
    pub fn contacts_state_to_ask(&self) -> Option<&str> {
        let age = self.now.saturating_sub(self.contacts_read);
        (!self.contacts_state.is_empty() && self.contacts_read > 0 && (0..CONTACTS_STATE_FOR).contains(&age)).then_some(self.contacts_state.as_str())
    }
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
    /// The account's address books, when the job read them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub books: Option<BooksDownload>,
    /// The contacts in them, when the job read them; none when they were not asked for, could not be read, or are
    /// as they were at the state the request held ([`JmapPimResult::contacts_state`] says which).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contacts: Option<ContactsDownload>,
    /// The state of the contacts as the server says it, when the host vouches for it: the state they were all read
    /// at (with `contacts`), or the one the request held, which the server still says (without). None after any
    /// trouble with them: the next job reads them all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contacts_state: Option<String>,
    /// Problems worth telling the owner; empty when all went well. They name no event and no contact.
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
    /// What the server made of the changes the request carried: an outcome for each that was tried, in the order they
    /// were; one without an outcome was not tried, and waits as before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded: Option<EventsUploaded>,
    /// The addresses the server knows the person by in events (JMAP for Calendars' participant identities: an alias,
    /// the one what they make is organized under), in lower case, when it said; `None` when it was not asked, or said
    /// nothing. They are the person's as the accounts' addresses are ([`JmapPimRequest::me`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identities: Option<Vec<String>>,
    /// What the server made of the changes of contacts the request carried, as [`JmapPimResult::uploaded`] for events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contacts_uploaded: Option<ContactsUploaded>,
    /// What came of the person's notes, when they were asked for and the session offers them: what the server made of
    /// the changes, and the notes read ([`crate::note::NotesAnswer`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<crate::note::NotesAnswer>,
}

impl JmapPimResult {
    /// Cut to what a window takes in, as everything from outside is: as many downloads as an account has
    /// calendars, each event cut to size ([`crate::remote::RemoteEvent::tidy`]), the lists tidied, the
    /// contacts cut to size ([`crate::remote::RemoteContact::tidy`]) and to as many as a download holds, a
    /// state that can be one, and a few problem lines of one line each. Doing it twice changes nothing more.
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
        if let Some(books) = &mut self.books {
            books.tidy();
        }
        if let Some(download) = &mut self.contacts {
            download.contacts.truncate(MAX_CONTACTS);
            download.contacts.iter_mut().for_each(crate::remote::RemoteContact::tidy);
        }
        self.contacts_state = self.contacts_state.take().map(|state| clean_key(&state)).filter(|state| !state.is_empty() && state.len() <= MAX_STATE);
        self.errors =
            std::mem::take(&mut self.errors).iter().map(|line| one_line(line)).filter(|line| !line.is_empty()).take(MAX_PIM_ERRORS).collect();
        self.busy_for = self.busy_for.map(|seconds| seconds.min(3_600));
        if let Some(uploaded) = &mut self.uploaded {
            uploaded.outcomes.truncate(MAX_EVENT_OPS);
            uploaded.outcomes.iter_mut().for_each(crate::remote::EventOutcome::tidy);
        }
        if let Some(identities) = &mut self.identities {
            let mut kept: Vec<String> = Vec::new();
            for address in identities.iter().map(|address| address.trim().to_lowercase()) {
                let fits =
                    address.contains('@') && !address.chars().any(|c| c.is_whitespace() || c.is_control()) && address.chars().count() <= MAX_ADDRESS;
                if fits && !kept.contains(&address) && kept.len() < MAX_PIM_ADDRESSES {
                    kept.push(address);
                }
            }
            *identities = kept;
        }
        if let Some(uploaded) = &mut self.contacts_uploaded {
            uploaded.outcomes.truncate(MAX_CONTACT_OPS);
            uploaded.outcomes.iter_mut().for_each(crate::book::ContactOutcome::tidy);
        }
        if let Some(notes) = &mut self.notes {
            notes.tidy();
        }
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
            .field("contacts", &self.contacts)
            .field("contacts_state", &!self.contacts_state.is_empty())
            .field("plain_unless_said", &self.plain_unless_said)
            .field("changes", &self.changes.as_ref().map(|changes| changes.ops.len()))
            .field("contact_changes", &self.contact_changes.as_ref().map(|changes| changes.ops.len()))
            .field("notes", &self.notes)
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
            .field("books", &self.books.as_ref().map(|list| list.books.len()))
            .field("contacts", &self.contacts.as_ref().map(|download| download.contacts.len()))
            .field("contacts_state", &self.contacts_state.is_some())
            .field("errors", &self.errors.len())
            .field("busy_for", &self.busy_for)
            .field("auth_refused", &self.auth_refused)
            .field("stopped", &self.stopped)
            .field("uploaded", &self.uploaded.as_ref().map(|uploaded| uploaded.outcomes.len()))
            .field("identities", &self.identities.as_ref().map(Vec::len))
            .field("contacts_uploaded", &self.contacts_uploaded.as_ref().map(|uploaded| uploaded.outcomes.len()))
            .field("notes", &self.notes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::book::{ContactChangeKind, ContactFields, ContactName, ContactOp, ContactOutcome, RemoteBook};
    use crate::note::{NoteFields, NoteOp, NoteOutcome, NotesAnswer, NotesAsk, NotesDownload, NotesUpload, NotesUploaded, RemoteNote};
    use crate::remote::{EventChangeKind, EventFields, EventOp, EventOutcome, EventResult, RemoteCalendar, RemoteContact, RemoteEvent};
    use crate::{Address, Attendee, Labeled};

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
            contacts: true,
            contacts_state: "q-ada@example.org".into(),
            contacts_read: 1_791_993_600 - 600,
            plain_unless_said: true,
            changes: Some(EventsUpload {
                account: 3,
                email: ME.into(),
                ops: vec![EventOp {
                    kind: EventChangeKind::Update,
                    event: 9,
                    remote: "V1".into(),
                    fields: EventFields { title: Some(format!("Budget review with {ME}")), ..Default::default() },
                    ..Default::default()
                }],
                zone: "Europe/Zurich".into(),
            }),
            contact_changes: Some(ContactsUpload {
                account: 3,
                email: ME.into(),
                ops: vec![ContactOp {
                    kind: ContactChangeKind::Update,
                    contact: 11,
                    remote: "Grace-Okafor".into(),
                    book: "neoffice".into(),
                    fields: ContactFields {
                        name: Some(ContactName { first: "Grace".into(), last: "Okafor".into(), ..Default::default() }),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
            }),
            notes: Some(NotesAsk {
                read: true,
                state: "q-notes".into(),
                read_at: 1_791_993_600 - 60,
                changes: Some(NotesUpload {
                    account: 3,
                    email: ME.into(),
                    ops: vec![NoteOp {
                        note: 4,
                        token: "t-1".into(),
                        fields: NoteFields { title: Some(format!("Budget for {ME}")), ..Default::default() },
                        ..Default::default()
                    }],
                }),
            }),
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
                refused: Vec::new(),
                calendars: vec![
                    RemoteCalendar { id: "C1".into(), name: "Ada's calendar".into(), main: true, plain: true, ..Default::default() },
                    RemoteCalendar { id: "C2".into(), name: "Team".into(), rights: crate::Rights::Read, plain: true, ..Default::default() },
                ],
            }),
            events: Some(EventsDownload { account: 3, email: ME.into(), complete: true, events: vec![event.clone()], ..Default::default() }),
            more_events: vec![EventsDownload { account: 3, calendar: "C2".into(), events: vec![event], ..Default::default() }],
            books: Some(BooksDownload {
                account: 3,
                email: ME.into(),
                complete: true,
                books: vec![RemoteBook {
                    id: "neoffice".into(),
                    name: "Neoffice of Ada".into(),
                    read_only: true,
                    main: true,
                    plain: true,
                    ..Default::default()
                }],
            }),
            contacts: Some(ContactsDownload {
                account: 3,
                email: ME.into(),
                complete: true,
                contacts: vec![RemoteContact {
                    id: "Grace-Okafor".into(),
                    book: "neoffice".into(),
                    first: "Grace".into(),
                    last: "Okafor".into(),
                    emails: vec![Labeled::new("", "grace@example.org")],
                    ..Default::default()
                }],
            }),
            contacts_state: Some("a1b2c3d4".into()),
            errors: vec![format!("Couldn't read a calendar of {ME}")],
            busy_for: Some(30),
            auth_refused: true,
            stopped: true,
            uploaded: Some(EventsUploaded {
                account: 3,
                email: ME.into(),
                outcomes: vec![EventOutcome { result: EventResult::Refused, error: format!("Not written here for {ME}"), ..Default::default() }],
                token_refused: false,
            }),
            identities: None,
            contacts_uploaded: Some(ContactsUploaded {
                account: 3,
                email: ME.into(),
                outcomes: vec![ContactOutcome { result: EventResult::Refused, error: format!("Grace is linked for {ME}"), ..Default::default() }],
                token_refused: true,
            }),
            notes: Some(NotesAnswer {
                uploaded: Some(NotesUploaded {
                    account: 3,
                    email: ME.into(),
                    outcomes: vec![NoteOutcome { error: format!("Not saved for {ME}"), ..Default::default() }],
                    ..Default::default()
                }),
                download: Some(NotesDownload {
                    account: 3,
                    email: ME.into(),
                    offered: true,
                    notes: vec![RemoteNote { id: "VN-1".into(), title: format!("Budget for {ME}"), ..Default::default() }],
                    ..Default::default()
                }),
                state: Some("a1b2-notes".into()),
            }),
        }
    }

    #[test]
    fn a_request_and_a_result_read_and_write_as_json() {
        let json = round_trip(&request());
        for part in [
            r#""lists":true"#,
            r#""main":"C-ada@example.org""#,
            r#""asks":[{"series":["V1"]},{"calendar":"C2","series":[]}]"#,
            r#""contacts":true"#,
            r#""contactsState":"q-ada@example.org""#,
            r#""plainUnlessSaid":true"#,
            r#""changes":{"account":3"#,
            r#""contactChanges":{"account":3"#,
            r#""notes":{"read":true,"state":"q-notes""#,
        ] {
            assert!(json.contains(part), "{part} in {json}");
        }
        let json = round_trip(&result());
        for part in [
            r#""offers":{"calendars":true,"contacts":true}"#,
            r#""moreEvents":[{"#,
            r#""books":{"account":3"#,
            r#""contacts":{"account":3"#,
            r#""contactsState":"a1b2c3d4""#,
            r#""authRefused":true"#,
            r#""busyFor":30"#,
            r#""uploaded":{"account":3"#,
            r#""contactsUploaded":{"account":3"#,
            r#""notes":{"uploaded":{"account":3"#,
        ] {
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
            for secret in
                ["@", "example.org", "jmap/session", "Ada", "Bob", "Budget", "Room", "Team", "Zurich", "C-", "V1", "Grace", "neoffice", "q-", "a1b2"]
            {
                assert!(!shown.contains(secret), "{secret} in {shown}");
            }
        }
        assert_eq!(
            format!("{request:?}"),
            "JmapPimRequest { account: 3, bearer: true, me: 2, zone: true, days: 397, lists: true, main: true, asks: 2, series: 1, contacts: true, contacts_state: true, plain_unless_said: true, changes: Some(1), contact_changes: Some(1), notes: Some(NotesAsk { read: true, state: true, changes: Some(1) }) }"
        );
        assert_eq!(
            format!("{result:?}"),
            "JmapPimResult { account: 3, offers: Some(PimOffers { calendars: true, contacts: true }), calendars: Some(2), downloads: 2, events: 2, books: Some(1), contacts: Some(1), contacts_state: true, errors: 1, busy_for: Some(30), auth_refused: true, stopped: true, uploaded: Some(1), identities: None, contacts_uploaded: Some(1), notes: Some(NotesAnswer { uploaded: Some(1), download: Some(1), state: true }) }"
        );
        // The person's identities are counted, never shown.
        let known = JmapPimResult { identities: Some(vec!["hello@example.org".into()]), ..JmapPimResult::default() };
        let shown = format!("{known:?}");
        assert!(shown.contains("identities: Some(1)") && !shown.contains("hello"), "{shown}");
    }

    #[test]
    fn the_persons_identities_are_addresses_in_lower_case_each_once_and_few() {
        let mut said = JmapPimResult {
            identities: Some(
                ["Hello@Example.org", " hello@example.org ", "no address", "two words@example.org", "x\u{0}@example.org", ""]
                    .into_iter()
                    .map(String::from)
                    .chain((0..2 * MAX_PIM_ADDRESSES).map(|n| format!("alias{n}@example.org")))
                    .collect(),
            ),
            ..JmapPimResult::default()
        };
        said.tidy();
        let kept = said.identities.clone().unwrap();
        assert_eq!(kept.first().map(String::as_str), Some("hello@example.org"));
        assert_eq!(kept.len(), MAX_PIM_ADDRESSES);
        assert!(kept.iter().all(|address| address.contains('@') && !address.contains(' ') && address == &address.to_lowercase()));
        said.tidy();
        assert_eq!(said.identities, Some(kept), "tidy twice changes nothing more");
        assert!(round_trip(&said).contains(r#""identities":["hello@example.org""#));
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
        // A book said twice and two main ones, a card with a name of many lines and an address that is none, more cards
        // than a download holds, and a state that can't be one.
        if let Some(books) = &mut hostile.books {
            books.books.push(RemoteBook { id: "neoffice".into(), name: "Again".into(), ..Default::default() });
            books.books.push(RemoteBook { id: "shared".into(), main: true, ..Default::default() });
        }
        if let Some(download) = &mut hostile.contacts {
            download.contacts.push(RemoteContact {
                id: "Cy".into(),
                first: "Cy\nril".into(),
                emails: vec![Labeled::new("", "not an address")],
                ..Default::default()
            });
            download.contacts.extend((0..MAX_CONTACTS).map(|n| RemoteContact { id: format!("c{n}"), first: "N".into(), ..Default::default() }));
        }
        hostile.contacts_state = Some("two words".into());
        // More outcomes than an upload has changes, one with a line of a thousand lines.
        if let Some(uploaded) = &mut hostile.uploaded {
            uploaded.outcomes =
                (0..2 * MAX_EVENT_OPS).map(|n| EventOutcome { error: format!("line {n}\n").repeat(1_000), ..Default::default() }).collect();
        }
        if let Some(uploaded) = &mut hostile.contacts_uploaded {
            uploaded.outcomes = (0..2 * MAX_CONTACT_OPS)
                .map(|n| ContactOutcome { remote: "two words".into(), error: format!("line {n}\n").repeat(1_000), ..Default::default() })
                .collect();
        }
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
        let books = hostile.books.as_ref().unwrap();
        assert_eq!(books.books.iter().map(|b| (b.id.as_str(), b.main)).collect::<Vec<_>>(), [("neoffice", true), ("shared", false)]);
        let contacts = &hostile.contacts.as_ref().unwrap().contacts;
        assert_eq!(contacts.len(), MAX_CONTACTS);
        let cy = contacts.iter().find(|c| c.id == "Cy").unwrap();
        assert!(cy.first == "Cy ril" && cy.emails.is_empty(), "{cy:?}");
        assert_eq!(hostile.contacts_state, None, "a state that can't be one is none");
        let outcomes = &hostile.uploaded.as_ref().unwrap().outcomes;
        assert!(outcomes.len() == MAX_EVENT_OPS && outcomes.iter().all(|o| !o.error.contains('\n') && o.error.chars().count() <= 300));
        let outcomes = &hostile.contacts_uploaded.as_ref().unwrap().outcomes;
        assert!(outcomes.len() == MAX_CONTACT_OPS && outcomes.iter().all(|o| o.remote.is_empty() && !o.error.contains('\n')));
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
        let listed: RemoteCalendar = serde_json::from_str(r#"{"id":"C1","name":"Team","rights":"read"}"#).unwrap();
        assert!(!listed.plain && listed.rights == crate::Rights::Read);
        assert!(round_trip(&RemoteCalendar { plain: true, ..listed }).contains(r#""plain":true"#));
    }

    #[test]
    fn the_state_of_the_contacts_is_asked_about_while_it_is_less_than_an_hour_old() {
        let held = request();
        assert_eq!(held.contacts_state_to_ask(), Some("q-ada@example.org"));
        let at = |read: i64| JmapPimRequest { contacts_read: read, ..request() };
        assert_eq!(at(held.now - CONTACTS_STATE_FOR + 1).contacts_state_to_ask(), Some("q-ada@example.org"));
        assert_eq!(at(held.now - CONTACTS_STATE_FOR).contacts_state_to_ask(), None, "an hour old: read through");
        assert_eq!(at(held.now + 60).contacts_state_to_ask(), None, "from after the job: not believed");
        assert_eq!(at(0).contacts_state_to_ask(), None, "never read");
        assert_eq!(JmapPimRequest { contacts_state: String::new(), ..request() }.contacts_state_to_ask(), None);
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
                books: Some(BooksDownload { books: vec![RemoteBook { id: text.clone(), name: text.clone(), main: true, ..Default::default() }], ..Default::default() }),
                contacts: Some(ContactsDownload {
                    contacts: vec![RemoteContact { id: text.clone(), book: text.clone(), first: text.clone(), emails: vec![Labeled::new(&text, &text)], ..Default::default() }],
                    ..Default::default()
                }),
                contacts_state: Some(text.clone()),
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
