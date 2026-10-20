// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account's notes, kept current by the job that keeps its calendars and address books ([`crate::jmap_pim`]): the
//! window says what the job asks of them ([`NotesAsk`]), and takes in what it brought.
//!
//! **When.** The notes are read with the wide job (every ten minutes, and the first of an account: a session that
//! offers none answers so, and costs the server nothing); at once when Notes comes to the front, and then every two
//! minutes while it is in front, for an account whose server keeps notes. What was made, changed or deleted here goes
//! three seconds after the last change (`sync.noteChanges`), and the very same changes are not sent again within half
//! a minute. A job for the days around an invitation asks nothing of the notes.
//!
//! **The state.** The window keeps, in memory, the state the job said the account's notes are at, and when it asked;
//! it hands it back with the next job, which asks the server whether it still says it before reading them all (the
//! host trusts it an hour). A job that read the notes and brought no state makes the window forget it.
//!
//! **The answer** is taken in in this order: what the server made of the changes (`sync.applyNoteChanges`), then the
//! notes read (`sync.applyNotes`); what the engine says of them (which names a note) goes to the status bar and the
//! Send/Receive problems report, not to the log. A job that carried changes and says nothing of them came to nothing.

use std::collections::HashMap;

use mailcraft_engine::Module;
use mailcraft_engine::cmd::notes::server::{apply_note_outcomes, merge_notes, note_upload, notes_waiting, settle_note_changes};
use mailcraft_model::Id;
use mailcraft_model::note::{NotesAsk, NotesUpload};
use mailcraft_model::sync::JmapPimResult;
use mailcraft_time::Timestamp;

use crate::MailApp;

/// How long after the last change a change of notes goes, how soon the very same changes may go again, and how often
/// the notes are read while Notes is in front (milliseconds).
const QUIET_MS: f64 = 3_000.0;
const SAME_AGAIN_MS: f64 = 30_000.0;
const IN_FRONT_MS: f64 = 120_000.0;

/// What the window remembers of an account's notes.
#[derive(Default)]
struct NotesJob {
    /// The state the last job said its notes are at, and when that job was asked for (Unix seconds, by the session's
    /// clock).
    state: Option<(String, Timestamp)>,
    /// Read at the next job: Notes came to the front.
    due: bool,
    /// When the notes were last asked to be read (milliseconds).
    read: Option<f64>,
    /// What was last sent, and when (milliseconds).
    sent: Option<(NotesUpload, f64)>,
}

/// The job in flight's notes: whether it read them and carried changes, and when it was asked for.
struct Flight {
    account: Id,
    read: bool,
    carried: bool,
    asked_at: Timestamp,
}

/// What the window remembers of keeping JMAP accounts' notes current.
#[derive(Default)]
pub(crate) struct NotesJobs {
    accounts: HashMap<Id, NotesJob>,
    /// Whether Notes was in front, and the window had the focus, at the last look.
    in_front: bool,
    focused: bool,
    /// The state of the mailbox at the last look, and when it last changed (milliseconds): a change of notes goes once
    /// the mailbox has been quiet a moment.
    rev: Option<u64>,
    changed: f64,
    flight: Option<Flight>,
}

impl MailApp {
    /// Look at the window at `now` (milliseconds): Notes came to the front, or the window came back with it in front (the
    /// notes are read at the next job of each account); and when the mailbox last changed.
    pub(crate) fn notes_look(&mut self, focused: bool, now: f64) {
        let in_front = self.session.view.module == Module::Notes && !self.ui.backstage;
        let jobs = &mut self.jmap_pim.notes;
        let came = in_front && (!jobs.in_front || (focused && !jobs.focused));
        (jobs.in_front, jobs.focused) = (in_front, focused);
        if came {
            let accounts: Vec<Id> = self.session.mailbox.notes.servers.iter().map(|server| server.account).collect();
            for account in accounts {
                self.jmap_pim.notes.accounts.entry(account).or_default().due = true;
            }
        }
        let rev = self.session.rev();
        if self.jmap_pim.notes.rev != Some(rev) {
            (self.jmap_pim.notes.rev, self.jmap_pim.notes.changed) = (Some(rev), now);
            // What waits and can't go any more (its note, account or server gone) is given up; the engine logs it.
            settle_note_changes(&mut self.session);
            self.jmap_pim.notes.rev = Some(self.session.rev());
        }
    }

    /// What of `account`'s notes is to go now, when anything: due, the mailbox quiet a moment, and not the very same as
    /// what went a moment ago.
    fn notes_push(&self, account: Id, now: f64) -> Option<NotesUpload> {
        if notes_waiting(&self.session, account).due == 0 || now - self.jmap_pim.notes.changed < QUIET_MS {
            return None;
        }
        let upload = note_upload(&self.session, account).ok().filter(|upload| !upload.ops.is_empty())?;
        let again = self.jmap_pim.notes.accounts.get(&account).and_then(|job| job.sent.as_ref());
        let repeated = again.is_some_and(|(sent, at)| *sent == upload && now - at < SAME_AGAIN_MS);
        (!repeated).then_some(upload)
    }

    /// Whether `account`'s notes are to be read by a job that is not the wide one: Notes came to the front, or has been
    /// in front long enough, and its server keeps notes.
    fn notes_read_due(&self, account: Id, now: f64) -> bool {
        let job = self.jmap_pim.notes.accounts.get(&account);
        let kept = self.session.mailbox.notes.server(account).is_some();
        let every = self.jmap_pim.notes.in_front && job.and_then(|job| job.read).is_none_or(|at| now - at >= IN_FRONT_MS);
        kept && (job.is_some_and(|job| job.due) || every)
    }

    /// Whether a job of `account` is wanted for its notes alone: a change to send, or a read that is due.
    pub(crate) fn notes_want(&self, account: Id, now: f64) -> bool {
        self.notes_read_due(account, now) || self.notes_push(account, now).is_some()
    }

    /// Whether a change of notes waits to go to `account`'s server (the window looks sooner).
    pub(crate) fn notes_waiting(&self, account: Id) -> bool {
        notes_waiting(&self.session, account).due > 0
    }

    /// What the job of `account` about to start asks of its notes (none for the days `around` an invitation), noted as
    /// asked: read with the `wide` job and when due; the changes to send.
    pub(crate) fn notes_ask(&mut self, account: Id, wide: bool, around: bool, now: f64) -> Option<NotesAsk> {
        if around {
            return None;
        }
        let read = wide || self.notes_read_due(account, now);
        let changes = self.notes_push(account, now);
        if !read && changes.is_none() {
            return None;
        }
        let asked_at = self.session.now();
        let job = self.jmap_pim.notes.accounts.entry(account).or_default();
        // (The state goes with changes alone too: the host keeps it when the server took them from that very state.)
        let (state, read_at) = job.state.clone().unwrap_or_default();
        if read {
            (job.read, job.due) = (Some(now), false);
        }
        if let Some(changes) = &changes {
            job.sent = Some((changes.clone(), now));
        }
        self.jmap_pim.notes.flight = Some(Flight { account, read, carried: changes.is_some(), asked_at });
        Some(NotesAsk { read, state, read_at, changes })
    }

    /// Take in what the job of `account` brought of its notes (taken out of `result`), as the module says. Whether it went
    /// well: a job that carried changes and says nothing of them came to nothing.
    pub(crate) fn take_in_notes(&mut self, account: Id, result: &mut JmapPimResult, errors: &mut Vec<String>) -> bool {
        let flight = self.jmap_pim.notes.flight.take().filter(|flight| flight.account == account);
        let Some(flight) = flight else { return true };
        let answer = result.notes.take();
        let ended_early = result.stopped || result.auth_refused || result.busy_for.is_some();
        let mut messages: Vec<String> = Vec::new();
        let mut taken = true;
        let mut answered = false;
        if let Some(uploaded) = answer.as_ref().and_then(|answer| answer.uploaded.as_ref()) {
            answered = !uploaded.outcomes.is_empty();
            match apply_note_outcomes(&mut self.session, uploaded) {
                Ok(applied) => {
                    log::info!(
                        "notes: of {} changes sent, {} were taken, {} refused, {} stale, {} are to be tried again; {} still wait",
                        uploaded.outcomes.len(),
                        applied.done,
                        applied.refused,
                        applied.stale,
                        applied.failed,
                        applied.waiting
                    );
                    messages.extend(applied.messages);
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", tl!("The notes couldn't be taken in")));
                    taken = false;
                }
            }
        }
        if let Some(download) = answer.as_ref().and_then(|answer| answer.download.as_ref()) {
            match merge_notes(&mut self.session, download) {
                Ok(merged) => messages.extend(merged.messages),
                Err(e) => {
                    errors.push(format!("{}: {e}", tl!("The notes couldn't be taken in")));
                    taken = false;
                }
            }
        }
        if let Some(first) = messages.first() {
            self.status(first.clone());
        }
        self.keep_sync_problems(account, &messages);
        // The state: the one the job vouched for; the one held when the job ended before the notes (nothing was learned
        // of them); none otherwise.
        let job = self.jmap_pim.notes.accounts.entry(account).or_default();
        match answer.as_ref().and_then(|answer| answer.state.clone()) {
            Some(state) if taken => job.state = Some((state, flight.asked_at)),
            None if answer.is_none() && ended_early => {}
            _ => job.state = None,
        }
        if flight.read && answer.is_none() {
            job.due = true;
        }
        !flight.carried || answered
    }

    /// The state of `account`'s notes that the window holds, and when the job that said it was asked for. (Public for a
    /// host's tests.)
    pub fn jmap_notes_state(&self, account: Id) -> Option<(String, Timestamp)> {
        self.jmap_pim.notes.accounts.get(&account).and_then(|job| job.state.clone())
    }
}

#[cfg(test)]
mod tests;
