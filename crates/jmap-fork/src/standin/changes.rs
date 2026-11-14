// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! `Email/changes` of the in-process server (RFC 8620 §5.2, RFC 8621 §4.3): what became of the emails since a
//! state it handed out, as a server that keeps a journal of them says.
//!
//! - **The journal.** Every change to an email that the stand-in makes, whoever asks for it (a test's
//!   [`Data::deliver`], [`Data::move_to`]…, an `Email/set`, an `Email/import`), is written under the state it
//!   made, with when each email was made and when it went. An email a test pushes into [`Data::emails`] by hand
//!   is in none: it was there from the start.
//! - **The call.** `accountId`, `sinceState` (a string) and `maxChanges` (a number above 0, or `null`), and no
//!   other argument (`invalidArguments`). A state it never handed out, or one older than
//!   [`Data::changes_from`] (a journal that lost its oldest entries), is `cannotCalculateChanges`.
//! - **The answer.** The emails made since (`created`), changed (`updated`) and gone (`destroyed`), each once, as
//!   they stand at `newState` (one made and gone between the two is in none). The changes of one state are
//!   never split: when they are more than `maxChanges` the call is refused (`cannotCalculateChanges`), and when
//!   those of later states would make more than it, the answer stops before them, says `hasMoreChanges`, and its
//!   `newState` is where it stopped.
//! - **A server without it.** [`Data::changes_unknown`] makes the method unknown (`unknownMethod`), as on a
//!   server that never had it.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use super::{ACCOUNT, Data, Outcome, account, allow_only, invalid, refuse};

/// What became of the emails, state by state.
#[derive(Default)]
pub(super) struct Journal {
    /// Each change to an email, oldest first: the state it made, and the email.
    entries: Vec<(u32, String)>,
    /// The state that made each email, and the one it went at.
    born: BTreeMap<String, u32>,
    died: BTreeMap<String, u32>,
}

impl Journal {
    /// Whether the email `id` was there at state `at`.
    fn there(&self, id: &str, at: u32) -> bool {
        self.born.get(id).is_none_or(|born| *born <= at) && self.died.get(id).is_none_or(|died| *died > at)
    }
}

impl Data {
    /// The email `id` was made, at the state the stand-in is at now.
    pub(super) fn made(&mut self, id: &str) {
        self.journal.born.insert(id.to_string(), self.version);
        self.journal.entries.push((self.version, id.to_string()));
    }

    /// The email `id` was changed (its keywords, its mailboxes).
    pub(super) fn touched(&mut self, id: &str) {
        self.journal.entries.push((self.version, id.to_string()));
    }

    /// The email `id` went (destroyed, or another id given it).
    pub(super) fn went(&mut self, id: &str) {
        self.journal.died.insert(id.to_string(), self.version);
        self.journal.entries.push((self.version, id.to_string()));
    }

    /// From now on, no change is calculated from a state handed out before: a journal lost.
    pub fn forget_changes(&mut self) {
        self.changes_from = self.version;
    }

    /// `Email/changes` (RFC 8620 §5.2).
    pub(super) fn email_changes(&self, args: &Map<String, Value>) -> Outcome<Value> {
        allow_only(args, &["accountId", "sinceState", "maxChanges"])?;
        account(args)?;
        let since = args.get("sinceState").and_then(Value::as_str).ok_or_else(|| invalid("sinceState is a string"))?;
        let most = match args.get("maxChanges") {
            None | Some(Value::Null) => usize::MAX,
            Some(value) => value
                .as_u64()
                .filter(|n| *n > 0)
                .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
                .ok_or_else(|| invalid("maxChanges is a number above 0"))?,
        };
        let cannot = || refuse("cannotCalculateChanges", "this state is not known here: read the emails again");
        let from = since.strip_prefix('s').and_then(|n| n.parse::<u32>().ok()).filter(|n| *n <= self.version).ok_or_else(cannot)?;
        if from < self.changes_from {
            return Err(cannot());
        }
        // The emails each state after `from` changed, in order.
        let mut states: BTreeMap<u32, BTreeSet<&str>> = BTreeMap::new();
        for (at, id) in self.journal.entries.iter().filter(|(at, _)| *at > from) {
            states.entry(*at).or_default().insert(id.as_str());
        }
        let mut taken: BTreeSet<&str> = BTreeSet::new();
        let (mut to, mut more) = (self.version, false);
        for (at, ids) in &states {
            let with = taken.union(ids).count();
            if with > most {
                // (The changes of one state are never split.)
                if taken.is_empty() {
                    return Err(cannot());
                }
                more = true;
                break;
            }
            taken.extend(ids.iter().copied());
            to = *at;
        }
        if !more {
            to = self.version;
        }
        let (mut created, mut updated, mut destroyed) = (Vec::new(), Vec::new(), Vec::new());
        for id in taken {
            match (self.journal.there(id, from), self.journal.there(id, to)) {
                (false, true) => created.push(id),
                (true, true) => updated.push(id),
                (true, false) => destroyed.push(id),
                (false, false) => {}
            }
        }
        Ok(json!({
            "accountId": ACCOUNT,
            "oldState": since,
            "newState": format!("s{to}"),
            "hasMoreChanges": more,
            "created": created,
            "updated": updated,
            "destroyed": destroyed,
        }))
    }
}
