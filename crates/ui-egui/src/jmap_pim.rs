// //// Neoffice — added file (no upstream equivalent)
//! Keeping a JMAP account's calendars current (JMAP for Calendars), by a job of their own beside its Send/Receive.
//!
//! The window works the job out from the engine's public functions, as it does for Microsoft Graph's calendar
//! ([`crate::calendar_sync`]): the calendars to read (the main one, the others that are ticked, all of them on the wide
//! stretch, and any that holds something sent and not yet seen) and the stretch; the host does the talking
//! ([`crate::Services::jmap_pim`]: `fetch` in the browser) and answers once, with what the engine takes in through
//! `sync.applyCalendars` and `sync.applyEvents`. Nothing new is asked of the engine.
//!
//! **When.** An account is asked for its list of calendars and the wide stretch (a month back, a year ahead) at its
//! first job and every ten minutes after; for the days on screen and a week either side every two minutes while the
//! calendar or People is in front, and every ten minutes otherwise; at once when the window comes back to the front
//! or the owner switches to the calendar or to People, when other days come into view, and after the owner's
//! Send/Receive of the account. A job that came to nothing is followed by a wait of half a minute, twice as long
//! after each one more, ten minutes at the most. (The cadences are the server's to bear: a server of the kind that
//! answers from a layer over another store pays for each read.)
//!
//! **Which accounts.** A JMAP account the window can ask without asking the owner anything: the host holds its
//! credential, or a password is at hand; not one whose server turned the secret down (until the owner asks, as for
//! mail), nor one whose server asked to be left alone, nor one whose session offers neither calendars nor address
//! books (asked again at the owner's Send/Receive), nor one whose mail is being read just now.
//!
//! **One at a time**, and none while a Microsoft Graph calendar job is under way: an answer asked for before a change
//! went up must never be taken in after it. A job not answered in two minutes is given up (its `cancel` is set and
//! its answer, should it come, is never read), and counts as one that came to nothing.
//!
//! **The answer** is taken in in this order: the list of calendars, then the main calendar's events, then the
//! others'. A download of a calendar the mailbox doesn't have is not taken in (the engine would make the calendar
//! for it). What the host says went wrong goes to the Send/Receive problems report and the log; what the engine says
//! of an event (which names it) to the report alone. A server that turned the secret down stops the account's jobs
//! and its mail runs alike; one that is busy is left alone as long as it asked.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

use mailcraft_engine::Module;
use mailcraft_model::remote::{CalendarAsk, EventsDownload};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult, MAX_PIM_ADDRESSES, PimOffers};
use mailcraft_model::{AccountKind, Id};
use mailcraft_time::{DateTime, Timestamp};

use crate::{MailApp, now_ms};

/// How often the days on screen are asked for while the calendar or People is in front of the owner, and otherwise;
/// how often the list of calendars and the wide stretch are (milliseconds).
const IN_FRONT_MS: f64 = 120_000.0;
const BEHIND_MS: f64 = 600_000.0;
const WIDE_MS: f64 = 600_000.0;
/// How long the server is left alone after a job that came to nothing: this long after the first, twice as long
/// after each one more, and never longer than the second.
const FIRST_RETRY_MS: f64 = 30_000.0;
const LONGEST_RETRY_MS: f64 = 600_000.0;
/// How long a job may take before it is given up.
const DEADLINE_MS: f64 = 120_000.0;
/// The longest a server's wish to be left alone is followed, in milliseconds.
const MAX_CALM_MS: f64 = 900_000.0;
/// The wide stretch: a month back, a year ahead (seconds).
const WIDE_BEHIND: i64 = 31 * 86_400;
const WIDE_AHEAD: i64 = 366 * 86_400;

/// What the host is asked to do for one calendar job of a JMAP account ([`crate::Services::jmap_pim`]).
pub struct JmapPimJob {
    /// What the job reads, and from where.
    pub request: JmapPimRequest,
    /// The secret that opens the server: the account's password, or a bearer token when `request.bearer` is set.
    /// `None` when the host holds the credential itself ([`crate::Services::jmap_host_credentials`]).
    pub secret: Option<String>,
    /// Set when the window gives the job up (it took too long): the host stops at the next request and answers with
    /// what it has, which nobody reads.
    pub cancel: Arc<AtomicBool>,
}

/// Counts and flags only: never the secret, nor the server's address, nor the account's.
impl std::fmt::Debug for JmapPimJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JmapPimJob")
            .field("request", &self.request)
            .field("secret", &self.secret.is_some())
            .field("cancel", &self.cancel.load(Ordering::Relaxed))
            .finish()
    }
}

/// A job in flight.
struct InFlight {
    account: Id,
    /// When it started (milliseconds), and whether it was for the list and the wide stretch.
    started: f64,
    wide: bool,
    cancel: Arc<AtomicBool>,
    rx: Receiver<JmapPimResult>,
}

/// What the window remembers of keeping JMAP accounts' calendars current.
#[derive(Default)]
pub(crate) struct JmapPim {
    rx: Option<InFlight>,
    /// When each account's days on screen were last asked for, and its list with the wide stretch (milliseconds).
    checked: HashMap<Id, f64>,
    wide: HashMap<Id, f64>,
    /// How many jobs in a row came to nothing for an account, and when the last one did.
    failures: HashMap<Id, (u32, f64)>,
    /// What each account's session offers, once a job has said.
    offers: HashMap<Id, PimOffers>,
    /// What each account's server calls its main calendar, as its last list said.
    mains: HashMap<Id, String>,
    /// Ask at once: the window came back to the front, or the owner switched to the calendar or to People.
    pub(crate) due: bool,
    /// Whether the window had the focus, and the calendar or People was in front, at the last look.
    focused: bool,
    in_front: bool,
    /// The stretch of each account's calendar that was last asked for.
    shown: HashMap<Id, (Timestamp, Timestamp)>,
    /// The account asked for last: the next look starts after it, so that accounts take turns.
    last: Option<Id>,
}

impl MailApp {
    /// Whether a calendar job of a JMAP account is in flight.
    pub fn jmap_pim_running(&self) -> bool {
        self.jmap_pim.rx.is_some()
    }

    /// Look whether a JMAP account's calendar is due, and start a job when one is. Called every frame.
    pub(crate) fn jmap_pim_tick(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        self.jmap_pim_tick_at(focused, now_ms());
        if self.services.jmap_pim.is_none() || !self.auto_sync {
            return;
        }
        if self.jmap_pim.rx.is_some() {
            // An answer is on its way (the host wakes the window when it is there, too), or the job's time runs out.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else if !self.jmap_pim_accounts().is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_secs(5));
        }
    }

    /// The JMAP accounts, in turn: those after the one asked for last first.
    fn jmap_pim_accounts(&self) -> Vec<Id> {
        let all: Vec<Id> = self.session.mailbox.accounts.iter().filter(|a| a.kind == AccountKind::Jmap).map(|a| a.id).collect();
        let after = self.jmap_pim.last.and_then(|last| all.iter().position(|id| *id == last)).map_or(0, |at| at + 1);
        let (first, rest) = all.split_at(after.min(all.len()));
        rest.iter().chain(first).copied().collect()
    }

    /// Whether the window may ask `account`'s server for its calendar now, without asking the owner anything.
    fn may_ask_pim(&self, account: Id, now: f64) -> bool {
        let mail_run = self.jmap_rx.as_ref().is_some_and(|(running, ..)| *running == account);
        let calm = now >= self.jmap_calm.get(&account).copied().unwrap_or(0.0);
        let offers_nothing = self.jmap_pim.offers.get(&account).is_some_and(|offers| offers.nothing());
        self.is_jmap(account) && self.can_sync_quietly(account) && !self.jmap_refused.contains(&account) && calm && !offers_nothing && !mail_run
    }

    /// [`jmap_pim_tick`](Self::jmap_pim_tick) at `now` (milliseconds), with the window in front or not. Whether a job
    /// was started. (Public for a host's tests, which have no window.)
    pub fn jmap_pim_tick_at(&mut self, focused: bool, now: f64) -> bool {
        let in_front = matches!(self.session.view.module, Module::Calendar | Module::People) && !self.ui.backstage;
        let came_back = (focused && !self.jmap_pim.focused) || (in_front && !self.jmap_pim.in_front);
        (self.jmap_pim.focused, self.jmap_pim.in_front) = (focused, in_front);
        if came_back && focused && in_front {
            self.jmap_pim.due = true;
        }
        // A job that took too long is given up: its answer, should it come, is never read.
        if let Some(job) = self.jmap_pim.rx.as_ref().filter(|job| now - job.started >= DEADLINE_MS) {
            log::warn!("calendar, account {}: the server didn't answer within two minutes; the job was given up", job.account);
            job.cancel.store(true, Ordering::Relaxed);
            let (account, wide) = (job.account, job.wide);
            self.jmap_pim.rx = None;
            self.pim_failed(account, now, wide);
        }
        if !self.auto_sync || self.services.jmap_pim.is_none() {
            return false;
        }
        // One at a time; and none beside a Microsoft Graph calendar job.
        if self.jmap_pim.rx.is_some() || self.calendar.rx.is_some() {
            return false;
        }
        for account in self.jmap_pim_accounts() {
            if !self.may_ask_pim(account, now) {
                continue;
            }
            let wide = self.jmap_pim.wide.get(&account).is_none_or(|at| now - at >= WIDE_MS);
            let every = if in_front { IN_FRONT_MS } else { BEHIND_MS };
            // Other days came into view: they are asked for without waiting.
            let elsewhere = in_front && self.jmap_pim.shown.get(&account).is_some_and(|shown| *shown != self.calendar_window());
            let asked = self.jmap_pim.checked.get(&account).copied();
            let refresh = wide || ((self.jmap_pim.due || elsewhere) && in_front) || asked.is_none_or(|at| now - at >= every);
            if !refresh {
                continue;
            }
            // A server that didn't answer the last time is given longer each time.
            let (failures, at) = self.jmap_pim.failures.get(&account).copied().unwrap_or((0, 0.0));
            let patience = (FIRST_RETRY_MS * f64::from(1u32 << failures.saturating_sub(1).min(8))).min(LONGEST_RETRY_MS);
            if failures > 0 && now - at < patience {
                continue;
            }
            if self.start_jmap_pim(account, wide, now, None) {
                return true;
            }
        }
        // Nobody to ask: the wish to ask at once doesn't outlive the look.
        self.jmap_pim.due = false;
        false
    }

    /// Hand the host a calendar job for `account`: its list and the wide stretch when `wide`, else the days on screen;
    /// or, for `around`, those days alone (which says nothing of when the days on screen were last asked for). `false`
    /// when there is no service, or no credential at hand.
    fn start_jmap_pim(&mut self, account: Id, wide: bool, now: f64, around: Option<(Timestamp, Timestamp)>) -> bool {
        let Some(owner) = self.session.mailbox.account(account).cloned() else { return false };
        let Some(server) = owner.jmap.clone() else { return false };
        if self.services.jmap_pim.is_none() {
            return false;
        }
        let secret = if self.services.jmap_host_credentials {
            None
        } else {
            match self.jmap_secret(account, &owner.email, server.bearer) {
                Some(secret) => Some(secret),
                None => return false,
            }
        };
        let today = self.session.now();
        let (from, to) = around.unwrap_or_else(|| if wide { (today - WIDE_BEHIND, today + WIDE_AHEAD) } else { self.calendar_window() });
        let main = self.jmap_pim.mains.get(&account).cloned().unwrap_or_default();
        let has_main = self.session.mailbox.calendars.iter().any(|c| c.origin.account == account && c.origin.id.is_empty());
        let mut asks = self.calendar_asks(account, wide && around.is_none(), Some((from, to)));
        // The main calendar always, even before a list has made it here.
        if !asks.iter().any(|ask| ask.calendar.is_empty()) {
            asks.insert(0, CalendarAsk { calendar: String::new(), series: Vec::new() });
        }
        let me: Vec<String> =
            self.session.mailbox.accounts.iter().map(|a| a.email.trim().to_string()).filter(|e| e.contains('@')).take(MAX_PIM_ADDRESSES).collect();
        let request = JmapPimRequest {
            account,
            email: owner.email.trim().to_string(),
            session: server.session.clone(),
            bearer: server.bearer,
            me,
            zone: self.session.zone().id(),
            now: today,
            from,
            to,
            lists: wide || main.is_empty() || !has_main,
            main,
            asks,
            plain_unless_said: false,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let Some(start) = &self.services.jmap_pim else { return false };
        let rx = start(JmapPimJob { request, secret, cancel: cancel.clone() });
        self.jmap_pim.rx = Some(InFlight { account, started: now, wide: wide && around.is_none(), cancel, rx });
        self.jmap_pim.last = Some(account);
        if around.is_none() {
            self.jmap_pim.checked.insert(account, now);
            if wide {
                self.jmap_pim.wide.insert(account, now);
            }
            self.jmap_pim.shown.insert(account, self.calendar_window());
            self.jmap_pim.due = false;
        }
        true
    }

    /// Ask the server of the JMAP account `account` for the days round `day` now, in a job of its own: for a meeting
    /// that should be on the calendar and hasn't come down (an invitation in the mail, [`crate::reading`]). Why not,
    /// when it can't be asked now.
    pub(crate) fn jmap_pim_around(&mut self, account: Id, day: mailcraft_time::Date) -> Result<(), String> {
        if self.services.jmap_pim.is_none() {
            return Err(tl!("This build can't reach the account's calendar.").to_string());
        }
        if self.jmap_pim.rx.is_some() || self.calendar.rx.is_some() {
            return Err(tl!("The calendar is being updated already.").to_string());
        }
        let now = now_ms();
        if !self.may_ask_pim(account, now) {
            return Err(tl!("This account's calendar can't be asked yet: Send/Receive signs it in first.").to_string());
        }
        let instant = |day: mailcraft_time::Date| {
            let midnight = DateTime::at_midnight(day);
            midnight.to_timestamp(self.session.offset_at(midnight.to_timestamp(0)))
        };
        let from = instant(day.add_days(-1));
        let round = (from, instant(day.add_days(2)).max(from.saturating_add(mailcraft_time::DAY)));
        if self.start_jmap_pim(account, false, now, Some(round)) {
            Ok(())
        } else {
            Err(tl!("This account's calendar can't be asked yet: Send/Receive signs it in first.").to_string())
        }
    }

    /// The owner asked for a Send/Receive of `account` (or of everything): its calendar is asked for at the next look,
    /// the wide stretch with the list, whatever its session said it offers last.
    pub(crate) fn jmap_pim_asked_for(&mut self, account: Id) {
        self.jmap_pim.wide.remove(&account);
        self.jmap_pim.offers.remove(&account);
        self.jmap_pim.failures.remove(&account);
        self.jmap_pim.due = true;
    }

    /// The calendar job answered: take in what it brought, in order. (Public for a host's tests.)
    pub fn poll_jmap_pim(&mut self) {
        let Some(job) = &self.jmap_pim.rx else { return };
        let (account, started, wide) = (job.account, job.started, job.wide);
        let result = match job.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                log::warn!("calendar, account {account}: the job ended without an answer (its task ended early)");
                self.jmap_pim.rx = None;
                self.pim_failed(account, started, wide);
                return;
            }
        };
        self.jmap_pim.rx = None;
        self.take_in_pim(account, (started, wide), result);
    }

    /// What one job brought, for `account` whatever it names (the engine still checks the address it carries).
    fn take_in_pim(&mut self, account: Id, (started, wide): (f64, bool), mut result: JmapPimResult) {
        result.account = account;
        result.tidy();
        if let Some(offers) = result.offers {
            self.jmap_pim.offers.insert(account, offers);
        }
        if result.auth_refused {
            log::warn!("calendar, account {account}: the server refused the sign-in; nothing is asked of it until the owner asks");
            self.jmap_refused.insert(account);
        }
        if let Some(seconds) = result.busy_for {
            self.jmap_calm.insert(account, now_ms() + (f64::from(seconds) * 1_000.0).min(MAX_CALM_MS));
        }
        if let Some(main) = result.calendars.as_ref().and_then(|list| list.calendars.iter().find(|c| c.main)) {
            self.jmap_pim.mains.insert(account, main.id.clone());
        }
        let mut errors = std::mem::take(&mut result.errors);
        self.take_in_calendar_list(account, result.calendars.as_ref(), None, false, &mut errors);
        // A download of a calendar the mailbox doesn't have is not taken in: the engine would make it.
        let here = |app: &MailApp, download: &EventsDownload| {
            let kept = app.session.mailbox.calendars.iter().any(|c| c.origin.account == account && c.origin.id == download.calendar);
            if !kept {
                log::debug!("calendar, account {account}: a download of a calendar that isn't here was left out");
            }
            kept
        };
        let main = result.events.take().filter(|download| here(self, download));
        self.take_in_calendar(account, None, main.as_ref(), &mut errors);
        let more: Vec<EventsDownload> = std::mem::take(&mut result.more_events).into_iter().filter(|download| here(self, download)).collect();
        self.take_in_more_calendars(&more, &mut errors);
        let well = result.offers.is_some() && errors.is_empty() && !result.auth_refused && result.busy_for.is_none() && !result.stopped;
        if well {
            self.jmap_pim.failures.remove(&account);
        } else {
            self.pim_failed(account, started, wide);
        }
        self.note_sync_problems(account, &errors, false);
    }

    /// A job of `account` came to nothing (`at`: when it started, or when it was given up): the next one waits longer
    /// each time, and is due once it has waited, not at the next look the cadence would make (the wide one again, when
    /// this one was `wide`).
    fn pim_failed(&mut self, account: Id, at: f64, wide: bool) {
        let earlier = self.jmap_pim.failures.get(&account).map_or(0, |(n, _)| *n);
        self.jmap_pim.failures.insert(account, (earlier.saturating_add(1), at));
        self.jmap_pim.checked.remove(&account);
        if wide {
            self.jmap_pim.wide.remove(&account);
        }
    }
}

#[cfg(test)]
#[path = "jmap_pim_tests.rs"]
mod tests;
