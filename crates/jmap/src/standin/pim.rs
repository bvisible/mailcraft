// //// Neoffice — added file (no upstream equivalent)
//! What the stand-in offers besides mail: calendars (JMAP for Calendars) and address books (RFC 9610), when a test
//! turns them on ([`Data::pim`](super::Data::pim)).
//!
//! The session then names the capability, its account (the mail's, [`ACCOUNT`](super::ACCOUNT)) and what the
//! account says of it: for calendars the stretch the server expands repeating events over
//! (`maxExpandedQueryDuration`) and, when [`Pim::says_writable`], the properties an event keeps when it is written
//! (`writableProperties`, which a server may add to the draft's), and likewise for address books. A request may name
//! the capability in `using` only when the session offers it.

use serde_json::{Value, json};

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
}

impl Default for Pim {
    fn default() -> Pim {
        Pim { calendars: false, contacts: false, says_writable: true, span: "P500D".into() }
    }
}

impl Pim {
    /// Calendars and address books, as a server that keeps only the basics offers them, saying so.
    pub fn offered() -> Pim {
        Pim { calendars: true, contacts: true, ..Pim::default() }
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
}

#[cfg(test)]
mod tests;
