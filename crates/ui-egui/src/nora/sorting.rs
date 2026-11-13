// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): « Trier avec Nora », what the server says of a mailbox's filing and what
// the person asks of it.

//! « Trier avec Nora »: Nora files a mailbox the hosting page handed over into folders, by a plan its person in charge
//! validates once (one folder per client of the ERP, by activity, or notifications and newsletters only), then keeps it
//! filed on her own if asked; every run can be undone. The server does all of it (nora `nora.api.v2.mail_sorting`, its
//! contract in the vault, NORA/49 note 12); the window says what the server says and carries the person's clicks:
//! - [`poll`] reads the `state` of the mailbox in view now and then (the button above the Inbox's list, its banner, the
//!   switch of her pane) and the summary of its last run;
//! - the flow in her pane ([`Flow`]) reads what Nora read (`survey`), her plan, her journal and her questions, and asks for a
//!   plan (`propose`), validates it and tidies the inbox (`validate`, `tidy`), follows the tidying, undoes a run (`undo`, called
//!   again while the server says some are left), answers a question (`nora.api.asks.answer`, `later`); a mailbox she has
//!   never read (no secretary yet: every mailbox, the first time) is started on first (`begin`), and what she read is read
//!   again while she is still reading it (`survey.reading`);
//! - the activation card ([`Card`]) turns the filing on (`switch`) and, without a validated plan, opens the flow on a plan;
//! - Nora's questions to the person, of all their mailboxes ([`Questions`], `nora.api.asks.mine`, read when the window starts,
//!   every fifteen minutes and when her pane opens): the number on her button of the ribbon, the section « Nora vous demande »
//!   of her pane's home and of the flow's result, answered by a button, in the person's own words (Nora says what she would
//!   keep, the person confirms it), or « later »; an answer to a question of a family's folder moves what Nora had filed of
//!   it, a correction that « Annuler » undoes (`run` of the answer).
//!
//! Nothing of it exists in a window that keeps its mail on the computer, nor where the host doesn't carry these calls
//! ([`super::Host::sorting`]), nor where the server says it isn't available (`state.available`: a plan without Nora's
//! background assistants, or filing turned off on the site). Only the person in charge of the box changes anything
//! (`may_set`); the others read. These are the person's own clicks: Nora's conversation, which may not move mail
//! (`assist::REFUSED_FAMILIES`), plays none of them. Every answer is read as a stranger's bytes ([`super::answer`]): field by
//! field, each checked for its type, texts cut to one line of the window, lists cut short.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, TryRecvError};

use mailcraft_engine::Module;
use mailcraft_model::FolderKind;
use serde_json::{Value, json};

use super::answer;
use super::flow::page_mailbox;
use super::{DEADLINE_MS, NoraAnswer, NoraCall};
use crate::MailApp;
use crate::i18n::fmt;

/// The server's methods (nora `nora.api.v2.mail_sorting`, and the answers to Nora's questions, `nora.api.asks`).
pub const STATE: &str = "nora.api.v2.mail_sorting.state";
pub const SURVEY: &str = "nora.api.v2.mail_sorting.survey";
pub const BEGIN: &str = "nora.api.v2.mail_sorting.begin";
pub const PLAN: &str = "nora.api.v2.mail_sorting.plan";
pub const JOURNAL: &str = "nora.api.v2.mail_sorting.journal";
pub const RUN_MAILS: &str = "nora.api.v2.mail_sorting.run_mails";
pub const ASKS: &str = "nora.api.v2.mail_sorting.asks";
pub const SWITCH: &str = "nora.api.v2.mail_sorting.switch";
pub const PROPOSE: &str = "nora.api.v2.mail_sorting.propose";
pub const VALIDATE: &str = "nora.api.v2.mail_sorting.validate";
pub const TIDY: &str = "nora.api.v2.mail_sorting.tidy";
pub const STOP_TIDY: &str = "nora.api.v2.mail_sorting.stop_tidy";
pub const UNDO: &str = "nora.api.v2.mail_sorting.undo";
pub const MINE: &str = "nora.api.asks.mine";
pub const ANSWER: &str = "nora.api.asks.answer";
pub const CONFIRM: &str = "nora.api.asks.confirm";
pub const LATER: &str = "nora.api.asks.later";

/// The calls of « Trier avec Nora » are numbered apart from the window's other calls of Nora's.
const FIRST_ID: u64 = 3 << 40;
/// How often the window looks for an answer while a call is in flight, in milliseconds.
const LOOK_AGAIN_MS: u64 = 100;
/// How often the state of the mailbox in view is read again, in milliseconds; and while its inbox is being tidied.
pub(crate) const STATE_EVERY_MS: f64 = 60_000.0;
pub(crate) const TIDY_EVERY_MS: f64 = 2_000.0;
/// How often a plan made in the background (by activity: 6 to 25 seconds) is read again, and when it is given up.
pub(crate) const PLAN_EVERY_MS: f64 = 3_000.0;
pub(crate) const PLAN_GIVE_UP_MS: f64 = 120_000.0;
/// How often what Nora read (`survey`) is read again while she is still reading the mailbox (its catch-up `running`, or a
/// mailbox she has just started on and read nothing of yet), how long a mailbox just started on is taken to be read, and
/// when the reading is no longer followed.
pub(crate) const SURVEY_EVERY_MS: f64 = 12_000.0;
pub(crate) const SURVEY_FRESH_MS: f64 = 180_000.0;
pub(crate) const SURVEY_GIVE_UP_MS: f64 = 2.0 * 3_600_000.0;
/// How long a plan by activity is waited for before a failure it says is taken as this one's (the server keeps the last
/// one's until its job ends: 6 to 25 seconds).
pub(crate) const PLAN_ERROR_AFTER_MS: f64 = 30_000.0;
/// The rhythm a mailbox Nora has never read is started at (minutes).
const FIRST_INTERVAL: u64 = 15;
/// How often Nora's questions are read again, in milliseconds (each read shows the person at most two new ones a day), and
/// how old a read may be when her pane opens.
pub(crate) const QUESTIONS_EVERY_MS: f64 = 15.0 * 60_000.0;
const QUESTIONS_FRESH_MS: f64 = 60_000.0;
/// The longest answer in the person's own words sent, and the longest sentence of Nora's they confirm.
const MAX_SAID: usize = 1_000;
const MAX_RETAINED: usize = 500;
/// How many times an undo is called again while the server says some mails are left (500 a call).
const MAX_UNDO_ROUNDS: u32 = 20;
/// The days of the inbox « Ranger maintenant » tidies (the planche's « les 30 derniers jours »).
pub(crate) const TIDY_DAYS: u64 = 30;
/// The most runs of the journal read, and the most items of any list taken from an answer.
const JOURNAL_RUNS: u64 = 20;
const MAX_ITEMS: usize = 50;
/// The longest name of a run or of a question taken (Frappe's names are shorter).
const MAX_NAME: usize = 140;

/// One way to file the mailbox the server knows (`state.modes`): `clients`, `activity`, `streams`, and its words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mode {
    pub(crate) code: String,
    pub(crate) label: String,
}

/// A kind of the ERP's parties (`state.party_types`, and `survey.parties` with how many wrote): `clients`, `suppliers`,
/// `leads`, `employees`, its words in the site's language.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PartyType {
    pub(crate) code: String,
    pub(crate) label: String,
    pub(crate) default: bool,
    /// In `survey.parties`: how many parties of it wrote, and how many messages.
    pub(crate) entities: u64,
    pub(crate) mails: u64,
}

/// The tidying of the inbox (`state.tidy`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Tidy {
    /// "", `running`, `done`, `stopped`.
    pub(crate) status: String,
    pub(crate) run: String,
    pub(crate) filed: u64,
    pub(crate) left: u64,
    pub(crate) seconds: f64,
    pub(crate) error: String,
    /// `reading`: all she has sorted is filed, and she reads the rest of those days (it is `done` only once they are read).
    pub(crate) waiting_for: String,
    /// How far her reading of those days is, while it runs.
    pub(crate) reading: Option<Reading>,
}

impl Tidy {
    pub(crate) fn running(&self) -> bool {
        self.status == "running"
    }

    /// Whether it files as she reads: it waits for her reading of its days (or says it waits, `waiting`).
    pub(crate) fn as_she_reads(&self) -> bool {
        self.waiting_for == "reading" || self.status == "waiting"
    }
}

/// Where her reading of a mailbox's days stands (`survey.reading`, `state.tidy.reading`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Reading {
    /// "", `running`, `paused`, `done`, `off` (turned off on the site).
    pub(crate) status: String,
    /// Messages sorted, of how many in those days (`total` 0 while she counts them).
    pub(crate) read: u64,
    pub(crate) total: u64,
    pub(crate) counted: bool,
    /// Those days are read and sorted: the counts are final.
    pub(crate) done: bool,
    /// She reads them now; not done and not now: she carries on tonight.
    pub(crate) now: bool,
    pub(crate) error: String,
}

impl Reading {
    /// Whether she is still reading those days: not done, not turned off, nothing gone wrong.
    pub(crate) fn going(&self) -> bool {
        !self.done && self.status != "off" && self.error.is_empty()
    }

    /// How far she is, when she has counted what there is: messages read, of how many.
    pub(crate) fn progress(&self) -> Option<(u64, u64)> {
        (self.counted && self.total > 0).then(|| (self.read.min(self.total), self.total))
    }
}

/// The last run of the mailbox (`state.last_run`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LastRun {
    pub(crate) run: String,
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) started_at: String,
    pub(crate) filed: u64,
}

/// What the server says of a mailbox's filing (`state`, and `switch`'s answer).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BoxState {
    pub(crate) address: String,
    /// Whether Nora may file mail on this site: not where its plan has none of her background assistants
    /// (`unavailable: "plan"`) nor where filing is turned off (`"off"`).
    pub(crate) available: bool,
    pub(crate) unavailable: String,
    /// Whether the Inbox has its tabs by kind of sender on this site.
    pub(crate) streams: bool,
    pub(crate) modes: Vec<Mode>,
    /// The kinds of the ERP's parties a plan by party (`parties`) may keep, the server's choice ticked (`default`).
    pub(crate) party_types: Vec<PartyType>,
    pub(crate) intervals: Vec<u64>,
    pub(crate) secretary: bool,
    /// Whether the caller may change anything (the person in charge of the box).
    pub(crate) may_set: bool,
    /// « Nora range mon courrier »: she files the new mail.
    pub(crate) on: bool,
    /// The reading's rhythm in minutes, and the time of the next reading as the server says it (« 10:45 »).
    pub(crate) interval: u64,
    pub(crate) next_said: String,
    pub(crate) prepare_documents: bool,
    pub(crate) prepare_replies: bool,
    /// The plan's state: "", `proposed`, `validated`; and its mode.
    pub(crate) plan_status: String,
    pub(crate) plan_mode: String,
    pub(crate) tidy: Option<Tidy>,
    /// How many questions of Nora's wait for the caller.
    pub(crate) asks: u64,
    pub(crate) last_run: Option<LastRun>,
    /// `switch` turned the filing on without a validated plan: nothing moves until there is one.
    pub(crate) needs_plan: bool,
}

impl BoxState {
    pub(crate) fn validated(&self) -> bool {
        self.plan_status == "validated"
    }

    pub(crate) fn tidying(&self) -> bool {
        self.tidy.as_ref().is_some_and(Tidy::running)
    }

    /// How often the state is read again while the inbox is tidied: every two seconds, at the reading's pace while the
    /// tidying waits for her reading.
    pub(crate) fn tidy_every(&self) -> f64 {
        if self.tidy.as_ref().is_some_and(Tidy::as_she_reads) { SURVEY_EVERY_MS } else { TIDY_EVERY_MS }
    }
}

/// What Nora read of the mailbox (`survey`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Survey {
    pub(crate) days: u64,
    pub(crate) mails: u64,
    /// By kind of mail (`notification`, `newsletter`, `invoice`, … `unsorted`).
    pub(crate) kinds: BTreeMap<String, u64>,
    pub(crate) clients: u64,
    pub(crate) client_mails: u64,
    pub(crate) in_inbox: u64,
    pub(crate) read_since: String,
    /// Her reading of the mailbox's past (its catch-up): "", `running`, `paused`, `done`.
    pub(crate) catch_up: String,
    /// Where her reading of these days stands (`null` without a secretary, and from a server older than it).
    pub(crate) reading: Option<Reading>,
    /// Each kind of party that wrote: how many parties, how many messages.
    pub(crate) parties: Vec<PartyType>,
}

impl Survey {
    pub(crate) fn kind(&self, kind: &str) -> u64 {
        self.kinds.get(kind).copied().unwrap_or(0)
    }

    /// Whether Nora is still reading the mailbox, `read_from` being when something that may have started her reading was
    /// done (the flow opened, the mailbox started on, a plan asked for): as `reading` says (until `done`); from a server
    /// that doesn't say it, her catch-up running (or paused till its hours), or a mailbox she has read nothing of, for a few
    /// minutes after. Never after two hours.
    pub(crate) fn still_reading(&self, read_from: f64, now: f64) -> bool {
        if now - read_from >= SURVEY_GIVE_UP_MS {
            return false;
        }
        match (&self.reading, self.catch_up.as_str()) {
            (Some(reading), _) => reading.going(),
            (None, "done") => false,
            (None, "running" | "paused") => true,
            (None, _) => self.mails == 0 && now - read_from < SURVEY_FRESH_MS,
        }
    }

    /// Whether what she read is to be read again soon: she is still reading, now (not only tonight).
    pub(crate) fn read_again(&self, read_from: f64, now: f64) -> bool {
        self.still_reading(read_from, now) && self.reading.as_ref().is_none_or(|r| r.now)
    }
}

/// A folder of the plan.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PlanFolder {
    pub(crate) folder: String,
    pub(crate) holds: String,
    pub(crate) count: u64,
    /// What a rule sends there: `clients`, `suppliers`, `leads`, `employees`, `notifications`, `newsletters`, `invoices`,
    /// `spam`, or "".
    pub(crate) takes: String,
    /// A kind of party's folder: how many parties it would receive (« Fournisseurs › 8 entités »).
    pub(crate) entities: u64,
}

/// The clients whose mail a plan by client would file, each in a folder of its own (`plan.clients`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Clients {
    pub(crate) shown: Vec<(String, u64)>,
    pub(crate) more: u64,
    pub(crate) more_count: u64,
}

/// Nora's plan for the mailbox (`plan`, and the answers of `propose` and `validate`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Plan {
    /// "", `proposed`, `validated`.
    pub(crate) status: String,
    pub(crate) mode: String,
    pub(crate) keep_unread_hours: u64,
    pub(crate) may_set: bool,
    /// When it was validated, as the server says it (« 05.11.2026 »).
    pub(crate) since_said: String,
    pub(crate) folders: Vec<PlanFolder>,
    pub(crate) clients: Option<Clients>,
    /// The plan is being made in the background (`propose` by activity): read it again until it is proposed.
    pub(crate) queued: bool,
    /// `reading`: a plan by activity waits for her to have read enough (20 sorted messages); made as soon as she has.
    pub(crate) waiting: String,
    /// Why the last plan by activity could not be made: `too few`, `model`, `empty`, `error`.
    pub(crate) error: String,
    /// A plan by party: a folder of each party under the folder of its kind.
    pub(crate) per_entity: bool,
    /// The parties of each kind kept, as `clients` says the clients.
    pub(crate) parties: BTreeMap<String, Clients>,
    /// She still reads the mailbox: the counts grow (a plan by rules is counted again at each read).
    pub(crate) counting: bool,
}

impl Plan {
    /// The parties a folder that takes `takes` would get a folder each for: `parties`, or `clients` for the clients.
    pub(crate) fn parties_of(&self, takes: &str) -> Option<&Clients> {
        self.parties.get(takes).or(if takes == "clients" { self.clients.as_ref() } else { None })
    }
}

/// A folder a run filed mail into.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RunFolder {
    pub(crate) folder: String,
    /// The folder of the clients' folders, for a client's (« Nora test/Clients »).
    pub(crate) parent: String,
    pub(crate) leaf: String,
    pub(crate) count: u64,
}

impl RunFolder {
    /// As the window says it: « Clients › Atelier Favre » for a client's, the folder's name otherwise.
    pub(crate) fn said(&self) -> String {
        if self.parent.is_empty() || self.leaf.is_empty() { self.folder.clone() } else { format!("{} › {}", self.parent, self.leaf) }
    }
}

/// One run of Nora's on the mailbox (`journal.runs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Run {
    pub(crate) run: String,
    /// `reading` (a reading that moved mail), `tidy` (the inbox tidied) or `correction` (an answer to a question of a family's
    /// folder moved what was filed of it).
    pub(crate) kind: String,
    /// What a correction was about (the family of mails its question named).
    pub(crate) about: String,
    /// `running`, `done`, `undone`, `partly_undone`.
    pub(crate) status: String,
    pub(crate) started_at: String,
    pub(crate) seconds: f64,
    pub(crate) filed: u64,
    pub(crate) still_filed: u64,
    pub(crate) folders: Vec<RunFolder>,
    /// What it filed by kind: `clients`, `notifications`, `newsletters`, `invoices`, `other`.
    pub(crate) kinds: BTreeMap<String, u64>,
    pub(crate) can_undo: bool,
}

/// What Nora learnt from a mail a person moved themselves.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Learned {
    pub(crate) leaf: String,
    pub(crate) from: String,
}

/// Nora's journal of the mailbox (`journal`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Journal {
    pub(crate) runs: Vec<Run>,
    pub(crate) learned: Vec<Learned>,
    pub(crate) may_set: bool,
}

/// One of Nora's questions to the person (`nora.api.asks.mine`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Ask {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) context: String,
    pub(crate) choices: Vec<(String, String)>,
    /// The address of the mailbox it is about.
    pub(crate) mailbox: String,
}

/// What Nora would keep of the person's own words, for them to confirm (`answer` with `text`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Confirming {
    pub(crate) retained: String,
    pub(crate) choice: String,
    pub(crate) scope: String,
}

/// What an undo did (`undo`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Undone {
    pub(crate) undone: u64,
    /// Moved since by a person: left where they put it.
    pub(crate) moved: u64,
    /// Their folder is gone: left.
    pub(crate) gone: u64,
    pub(crate) failed: u64,
    /// Not done in this call: call again.
    pub(crate) remaining: u64,
}

// ---- reading the answers ----

/// A text of the answer, fit for one line of the window.
fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).map(answer::clean).unwrap_or_default()
}

/// A count of the answer (a number, or a number written as text); 0 for anything else.
fn count(v: &Value, key: &str) -> u64 {
    v.get(key).map_or(0, number)
}

/// A count: a number, or a number written as text; 0 for anything else.
fn number(v: &Value) -> u64 {
    match v {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f.min(1e15) as u64)).unwrap_or(0),
        Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Seconds of the answer; 0 for anything that isn't a finite, positive number.
fn seconds(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).filter(|f| f.is_finite() && *f >= 0.0).map_or(0.0, |f| f.min(1e9))
}

/// A flag as Frappe writes one: `true`, `1` or `"1"`.
fn flag(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// The items of a list of the answer, [`MAX_ITEMS`] at most.
fn items<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS)
}

/// The counts of an object of the answer (`kinds`), by key.
fn counts(v: &Value, key: &str) -> BTreeMap<String, u64> {
    v.get(key)
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .take(MAX_ITEMS)
        .filter(|(k, _)| is_code(k))
        .map(|(k, n)| (k.clone(), number(n)))
        .collect()
}

/// A code of the server's (`clients`, `notification`…): a few lower-case letters.
fn is_code(code: &str) -> bool {
    !code.is_empty() && code.len() <= 32 && code.chars().all(|c| c.is_ascii_lowercase() || c == '_')
}

/// A name of the server's (a run, a question), as it is to be sent back: one line, not too long.
fn name(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty() && n.len() <= MAX_NAME && !n.chars().any(char::is_control))
        .unwrap_or("")
        .to_string()
}

/// The status of a run or of the tidying, as one code (« Partly undone » is `partly_undone`).
fn status(v: &Value, key: &str) -> String {
    let said = text(v, key).to_lowercase().replace(' ', "_");
    if is_code(&said) { said } else { String::new() }
}

/// `state` (and `switch`'s answer: the state and `needs_plan`).
pub(crate) fn read_state(m: &Value) -> BoxState {
    let reading = m.get("reading").filter(|r| r.is_object());
    let plan = m.get("plan").filter(|p| p.is_object());
    let tidy = m.get("tidy").filter(|t| t.is_object()).map(read_tidy);
    let last_run = m.get("last_run").filter(|r| r.is_object()).map(|r| LastRun {
        run: name(r, "run"),
        kind: status(r, "kind"),
        status: status(r, "status"),
        started_at: text(r, "started_at"),
        filed: count(r, "filed"),
    });
    let modes = items(m, "modes")
        .map(|mode| Mode { code: text(mode, "code"), label: text(mode, "label") })
        .filter(|mode| is_code(&mode.code) && !mode.label.is_empty())
        .take(6)
        .collect();
    let intervals = items(m, "intervals").filter_map(|n| n.as_u64()).filter(|n| (1..=1440).contains(n)).take(6).collect();
    let party_types = read_party_types(m, "party_types");
    BoxState {
        address: text(m, "address"),
        available: flag(m, "available"),
        unavailable: status(m, "unavailable"),
        streams: flag(m, "streams"),
        modes,
        party_types,
        intervals,
        secretary: flag(m, "secretary"),
        may_set: flag(m, "may_set"),
        on: flag(m, "on"),
        interval: reading.map_or(0, |r| count(r, "interval")),
        next_said: reading.map(|r| text(r, "next_said")).unwrap_or_default(),
        prepare_documents: reading.is_some_and(|r| flag(r, "prepare_documents")),
        prepare_replies: reading.is_some_and(|r| flag(r, "prepare_replies")),
        plan_status: plan.map(|p| status(p, "status")).unwrap_or_default(),
        plan_mode: plan.map(|p| status(p, "mode")).unwrap_or_default(),
        tidy,
        asks: count(m, "asks"),
        last_run,
        needs_plan: flag(m, "needs_plan"),
    }
}

/// `state.party_types`, `survey.parties`: each a code, its words, whether the server ticks it, how many wrote.
fn read_party_types(m: &Value, key: &str) -> Vec<PartyType> {
    items(m, key)
        .map(|p| PartyType {
            code: text(p, "code"),
            label: text(p, "label"),
            default: flag(p, "default"),
            entities: count(p, "entities"),
            mails: count(p, "mails"),
        })
        .filter(|p| is_code(&p.code) && !p.label.is_empty())
        .take(8)
        .collect()
}

/// `state.tidy`, and the answers of `tidy` and `stop_tidy`.
pub(crate) fn read_tidy(t: &Value) -> Tidy {
    Tidy {
        status: status(t, "status"),
        run: name(t, "run"),
        filed: count(t, "filed"),
        left: count(t, "left"),
        seconds: seconds(t, "seconds"),
        error: text(t, "error"),
        waiting_for: status(t, "waiting_for"),
        reading: t.get("reading").filter(|r| r.is_object()).map(read_reading),
    }
}

/// `survey.reading`, `state.tidy.reading`.
pub(crate) fn read_reading(r: &Value) -> Reading {
    Reading {
        status: status(r, "status"),
        read: count(r, "read"),
        total: count(r, "total"),
        counted: flag(r, "counted"),
        done: flag(r, "done"),
        now: flag(r, "now"),
        error: text(r, "error"),
    }
}

/// `survey`.
pub(crate) fn read_survey(m: &Value) -> Survey {
    Survey {
        days: count(m, "days"),
        mails: count(m, "mails"),
        kinds: counts(m, "kinds"),
        clients: count(m, "clients"),
        client_mails: count(m, "client_mails"),
        in_inbox: count(m, "in_inbox"),
        read_since: text(m, "read_since"),
        catch_up: status(m, "catch_up"),
        reading: m.get("reading").filter(|r| r.is_object()).map(read_reading),
        parties: read_party_types(m, "parties"),
    }
}

/// The parties a plan names for a kind (`plan.clients`, each of `plan.parties`).
fn read_clients(c: &Value) -> Clients {
    Clients {
        shown: items(c, "shown").map(|s| (text(s, "name"), count(s, "count"))).filter(|(n, _)| !n.is_empty()).take(5).collect(),
        more: count(c, "more"),
        more_count: count(c, "more_count"),
    }
}

/// `plan`, and the answers of `propose` and `validate`.
pub(crate) fn read_plan(m: &Value) -> Plan {
    let folders = items(m, "folders")
        .map(|f| PlanFolder {
            folder: text(f, "folder"),
            holds: text(f, "holds"),
            count: count(f, "count"),
            takes: status(f, "takes"),
            entities: count(f, "entities"),
        })
        .filter(|f| !f.folder.is_empty())
        .collect();
    let clients = m.get("clients").filter(|c| c.is_object()).map(read_clients);
    let parties = m
        .get("parties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .take(8)
        .filter(|(k, c)| is_code(k) && c.is_object())
        .map(|(k, c)| (k.clone(), read_clients(c)))
        .collect();
    Plan {
        status: status(m, "status"),
        mode: status(m, "mode"),
        keep_unread_hours: count(m, "keep_unread_hours"),
        may_set: flag(m, "may_set"),
        since_said: text(m, "since_said"),
        folders,
        clients,
        queued: flag(m, "queued"),
        waiting: status(m, "waiting"),
        // « too few » is `too_few`.
        error: status(m, "error"),
        per_entity: flag(m, "per_entity"),
        parties,
        counting: flag(m, "counting"),
    }
}

/// `journal`.
pub(crate) fn read_journal(m: &Value) -> Journal {
    let runs = items(m, "runs")
        .map(|r| Run {
            run: name(r, "run"),
            kind: status(r, "kind"),
            about: text(r, "about"),
            status: status(r, "status"),
            started_at: text(r, "started_at"),
            seconds: seconds(r, "seconds"),
            filed: count(r, "filed"),
            still_filed: count(r, "still_filed"),
            folders: items(r, "folders")
                .map(|f| RunFolder { folder: text(f, "folder"), parent: text(f, "parent"), leaf: text(f, "leaf"), count: count(f, "count") })
                .filter(|f| !f.folder.is_empty())
                .collect(),
            kinds: counts(r, "kinds"),
            can_undo: flag(r, "can_undo"),
        })
        .filter(|r| !r.run.is_empty())
        .collect();
    let learned =
        items(m, "learned").map(|l| Learned { leaf: text(l, "leaf"), from: text(l, "from") }).filter(|l| !l.leaf.is_empty()).take(10).collect();
    Journal { runs, learned, may_set: flag(m, "may_set") }
}

/// `nora.api.asks.mine` (and the box's own `asks`): the questions, and how many answers Nora keeps.
pub(crate) fn read_asks(m: &Value) -> (Vec<Ask>, u64) {
    let asks = items(m, "asks")
        .map(|a| Ask {
            name: name(a, "name"),
            title: text(a, "title"),
            context: text(a, "context"),
            choices: items(a, "choices")
                .map(|c| (name(c, "code"), text(c, "label")))
                .filter(|(code, label)| !code.is_empty() && !label.is_empty())
                .take(6)
                .collect(),
            mailbox: text(a, "box"),
        })
        .filter(|a| !a.name.is_empty() && !a.title.is_empty())
        .take(10)
        .collect();
    (asks, count(m, "retained"))
}

/// The answer of `answer` in the person's own words: what Nora would keep, to confirm.
pub(crate) fn read_confirm(m: &Value) -> Option<Confirming> {
    let c = m.get("confirm").filter(|c| c.is_object())?;
    let retained = c.get("retained").and_then(Value::as_str).map(|r| r.split_whitespace().collect::<Vec<_>>().join(" "))?;
    let retained: String = retained.chars().filter(|ch| !ch.is_control()).take(MAX_RETAINED).collect();
    (!retained.is_empty()).then(|| Confirming { retained, choice: name(c, "choice"), scope: name(c, "scope") })
}

/// What Nora says once an answer is kept (`said`, « Noté : … »), and what she put right meanwhile (`corrected`).
pub(crate) fn read_noted(m: &Value) -> String {
    [text(m, "said"), text(m, "corrected")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

/// `undo`.
pub(crate) fn read_undone(m: &Value) -> Undone {
    Undone {
        undone: count(m, "undone"),
        moved: count(m, "moved"),
        gone: count(m, "gone"),
        failed: count(m, "failed"),
        remaining: count(m, "remaining"),
    }
}

// ---- what the window keeps ----

/// A call in flight.
struct Asked {
    id: u64,
    answer: Receiver<NoraAnswer>,
    since: f64,
    what: Asking,
}

/// What a call asked, for whom (the mailbox's name on the server).
#[derive(Clone, Debug, PartialEq)]
enum Asking {
    State(String),
    /// The last run, for the banner: the mailbox, the run.
    Latest(String, String),
    Survey(String),
    /// The mailbox Nora has never read, started on: her secretary made, her reading of its last days started.
    Begin(String),
    Plan(String),
    Journal(String),
    /// Nora's questions, of all the person's mailboxes.
    Mine,
    Switch(String, AfterSwitch),
    Propose(String),
    /// Validated, the inbox tidied too when `true`.
    Validate(String, bool),
    /// « Ensuite, je continue ? »: the plan files the new mail, then this rhythm (minutes).
    Continue(String, u64),
    Tidy(String),
    StopTidy(String),
    Undo(String, String),
    /// A question answered by a button, in the person's words, confirmed, put off: the question.
    Answer(String),
    Said(String),
    Confirm(String),
    Later(String),
    /// The correction an answer made, undone: the mailbox, the run.
    UndoCorrection(String, String),
}

/// What follows a switch.
#[derive(Clone, Debug, PartialEq)]
enum AfterSwitch {
    /// The pane's switch.
    Nothing,
    /// The activation card's « Activer et voir le plan », with the mode chosen there.
    FromCard(String),
    /// The rhythm asked by « Ensuite, je continue ? ».
    Rhythm(u64),
    /// The flow opened on a mailbox Nora has never read, through a host that refuses `begin`: started by the switch (its
    /// secretary made, her reading started), then read.
    Wake,
}

/// What the window knows of a mailbox's filing.
#[derive(Default)]
pub(crate) struct Known {
    pub(crate) state: Option<BoxState>,
    /// The last run, read for the banner.
    pub(crate) latest: Option<Run>,
    /// When its state was last asked (milliseconds): a refusal waits as long as an answer.
    asked_at: f64,
    /// The run the banner's summary was read for.
    latest_of: String,
}

impl Known {
    /// What the window knows of a mailbox, as if the server had just said it (for the tests that draw it).
    #[cfg(test)]
    pub(crate) fn said(state: BoxState, latest: Option<Run>) -> Known {
        let latest_of = latest.as_ref().map(|r| r.run.clone()).unwrap_or_default();
        Known { state: Some(state), latest, asked_at: crate::now_ms(), latest_of }
    }
}

/// Where the flow in her pane is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Reading what Nora read and her plan.
    Loading,
    /// « Comment voulez-vous que je range ? »
    Choose,
    /// A plan by party: which kinds of party, and a folder for each party or for its kind only?
    Parties,
    /// « Je vous explique »: the person's own words for a plan by activity.
    Explain,
    /// The plan is being made.
    Proposing,
    /// « Voici ce que je ferais »: the plan, to file now or change.
    Plan,
    /// The inbox is being tidied.
    Tidying,
    /// « C'est rangé », and the journal.
    Done,
}

/// A folder of the plan as the person leaves it (« Modifier »).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EditRow {
    pub(crate) folder: String,
    pub(crate) holds: String,
    pub(crate) takes: String,
    pub(crate) count: u64,
    pub(crate) kept: bool,
}

/// The flow of « Trier avec Nora » in her pane, for one mailbox.
pub(crate) struct Flow {
    /// The mailbox, by the name of the server's account for it.
    pub(crate) account: String,
    pub(crate) stage: Stage,
    pub(crate) survey: Option<Survey>,
    pub(crate) plan: Option<Plan>,
    pub(crate) journal: Option<Journal>,
    /// The way to file the person chose (its words are shown as theirs).
    pub(crate) chosen: Option<Mode>,
    /// A plan by party: the kinds of party ticked (the server's, at first), and the person's answer to « a folder for each
    /// party, or for its kind only? » (`per_entity`).
    pub(crate) party_types: Vec<String>,
    pub(crate) per_entity: Option<bool>,
    /// What the plan was last asked with (`propose`), to ask it again the same way.
    proposed: Option<Value>,
    /// Their own words for a plan by activity.
    pub(crate) wishes: String,
    /// The plan as they are changing it.
    pub(crate) editing: Option<Vec<EditRow>>,
    /// Why the last thing asked failed, in the server's words.
    pub(crate) error: Option<String>,
    /// What Nora says of what was just done.
    pub(crate) said: Option<String>,
    /// Opened by turning Nora on: the plan files the new mail once validated, and nothing is asked about it after.
    pub(crate) on_intent: bool,
    /// Opened from the banner's « Voir le journal »: the journal first, nothing read of the mailbox again.
    pub(crate) journal_first: bool,
    /// The run of the tidying this flow started.
    pub(crate) tidy_run: String,
    /// « Ensuite, je continue toute seule ? » was answered.
    pub(crate) continued: bool,
    /// When the stage began, and when to read again (milliseconds).
    pub(crate) since: f64,
    pub(crate) next: f64,
    undo_rounds: u32,
    /// What Nora read was asked; the mailbox, which she had never read, was started on (`switch`); it can't be: she has
    /// never read it and the person may not start her on it.
    surveyed: bool,
    woke: bool,
    pub(crate) unstarted: bool,
    /// When something that may have started her reading was done, and when to read what she read again (milliseconds).
    pub(crate) read_from: f64,
    pub(crate) survey_next: f64,
    /// « Ranger maintenant » was asked in this flow.
    pub(crate) filing: bool,
}

impl Flow {
    pub(crate) fn new(account: String, journal_first: bool, now: f64) -> Flow {
        Flow {
            account,
            stage: Stage::Loading,
            survey: None,
            plan: None,
            journal: None,
            chosen: None,
            wishes: String::new(),
            editing: None,
            error: None,
            said: None,
            on_intent: false,
            journal_first,
            tidy_run: String::new(),
            continued: false,
            since: now,
            next: now,
            undo_rounds: 0,
            surveyed: false,
            woke: false,
            unstarted: false,
            read_from: now,
            survey_next: now + SURVEY_EVERY_MS,
            filing: false,
            party_types: Vec::new(),
            per_entity: None,
            proposed: None,
        }
    }

    /// Whether Nora is still reading the mailbox, as what she read last said it.
    pub(crate) fn reading(&self, now: f64) -> bool {
        self.survey.as_ref().is_some_and(|s| s.still_reading(self.read_from, now))
    }

    fn enter(&mut self, stage: Stage, now: f64) {
        self.stage = stage;
        self.since = now;
        self.next = now;
    }
}

/// The activation card (« Nora range votre courrier »), while it is up.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Card {
    pub(crate) account: String,
    pub(crate) mode: String,
    pub(crate) interval: u64,
    /// « Préparer le travail »: a draft reply, an invoice read, a task.
    pub(crate) prepare: bool,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
}

/// Nora's questions to the person, of all their mailboxes, and what they are answering.
#[derive(Default)]
pub(crate) struct Questions {
    pub(crate) list: Vec<Ask>,
    /// How many answers Nora keeps.
    pub(crate) retained: u64,
    /// When they were last read (milliseconds).
    asked_at: f64,
    /// The person's own words for a question, by its name, as they type them.
    pub(crate) drafts: BTreeMap<String, String>,
    /// What Nora would keep of their words, by the question's name, as they change it before confirming.
    pub(crate) confirming: BTreeMap<String, Confirming>,
    /// What Nora said of the last answer (« Noté : … »), and the correction it made, which « Annuler » undoes.
    pub(crate) noted: Option<Noted>,
    pub(crate) error: Option<String>,
    undo_rounds: u32,
}

/// What Nora said of an answer, and the correction it made (`run`, "" when nothing moved), in which mailbox.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Noted {
    pub(crate) words: String,
    /// The mailbox, by the name of the server's account for it.
    pub(crate) account: String,
    pub(crate) run: String,
}

impl Questions {
    /// The questions as if the server had just said them (for the tests that draw them).
    #[cfg(test)]
    pub(crate) fn said(list: Vec<Ask>) -> Questions {
        Questions { list, asked_at: crate::now_ms(), ..Questions::default() }
    }

    /// A question is answered or set aside: it goes, with what was typed for it.
    fn settled(&mut self, ask: &str) {
        self.list.retain(|a| a.name != ask);
        self.drafts.remove(ask);
        self.confirming.remove(ask);
    }
}

/// What the window knows of « Trier avec Nora » while it runs. Nothing of it is kept between runs.
#[derive(Default)]
pub(crate) struct Sorting {
    last: u64,
    asked: Vec<Asked>,
    /// Each mailbox looked at, by the name of the server's account for it.
    pub(crate) boxes: BTreeMap<String, Known>,
    pub(crate) flow: Option<Flow>,
    pub(crate) card: Option<Card>,
    pub(crate) questions: Questions,
}

impl Sorting {
    /// The state of a mailbox where Nora files mail: `None` where she doesn't or isn't known to.
    pub(crate) fn available(&self, account: &str) -> Option<&BoxState> {
        self.boxes.get(account).and_then(|k| k.state.as_ref()).filter(|s| s.available)
    }

    fn asking(&self, what: &Asking) -> bool {
        self.asked.iter().any(|a| a.what == *what)
    }

    /// Whether a call that changes something is in flight.
    pub(crate) fn changing(&self) -> bool {
        self.asked.iter().any(|a| {
            matches!(
                a.what,
                Asking::Switch(..)
                    | Asking::Begin(_)
                    | Asking::Propose(_)
                    | Asking::Validate(..)
                    | Asking::Continue(..)
                    | Asking::Tidy(_)
                    | Asking::StopTidy(_)
                    | Asking::Undo(..)
                    | Asking::Answer(_)
                    | Asking::Said(_)
                    | Asking::Confirm(_)
                    | Asking::Later(_)
                    | Asking::UndoCorrection(..)
            )
        })
    }
}

/// What the person did, for [`act`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Intent {
    /// Open the flow in her pane for this mailbox, its journal first when `journal`.
    Open {
        account: String,
        journal: bool,
    },
    /// Leave the flow: her conversation again.
    Back,
    Choose(Mode),
    /// A plan by party: a kind of party ticked or unticked; the answer to « a folder for each party, or for its kind
    /// only? », which asks for the plan.
    PartyType(String),
    PerEntity(bool),
    /// « Je vous explique ».
    Explain,
    /// A plan by activity, after the person's words.
    ProposeWishes,
    /// « Modifier »; and back to Nora's plan.
    Edit,
    KeepPlan,
    /// « Refaire le plan ».
    NewPlan,
    /// « Ranger maintenant ».
    FileNow,
    StopTidy,
    Undo(String),
    /// « Ensuite, je continue ? »: a rhythm in minutes, or none.
    Continue(Option<u64>),
    /// A question answered by one of its buttons (its name, the choice's code); in the person's own words (what they typed
    /// for it); what Nora would keep of them confirmed, or put aside to say it again; « Plus tard ».
    Answer(String, String),
    SayAnswer(String),
    ConfirmAnswer(String),
    Rephrase(String),
    Later(String),
    /// « Annuler » under what Nora noted: the correction the answer made, undone.
    UndoNoted,
    /// The pane's switch.
    Switch {
        account: String,
        on: bool,
    },
    /// The card's « Activer et voir le plan », and « Plus tard ».
    Activate,
    CloseCard,
    Retry,
}

// ---- the calls ----

/// Whether « Trier avec Nora » may be offered in this window: a hosting page's tab (nothing kept here) whose host carries
/// these calls.
pub(crate) fn offered(app: &MailApp) -> bool {
    !app.session.keeps_local && app.services.nora.as_ref().is_some_and(|h| h.chat && h.sorting)
}

/// The mailbox of the folder in view, by the name of the server's account for it: one the hosting page handed over;
/// only its Inbox when `inbox`.
pub(crate) fn box_in_view(app: &MailApp, inbox: bool) -> Option<String> {
    if app.session.view.module != Module::Mail || !offered(app) {
        return None;
    }
    let folder = app.session.mailbox.folder(app.session.view.folder)?;
    if inbox && folder.kind != FolderKind::Inbox {
        return None;
    }
    page_mailbox(app, folder.account)
}

/// Ask the host; `false` without one, or while the same is asked already.
fn call(app: &mut MailApp, method: &'static str, args: Value, what: Asking) -> bool {
    if !offered(app) || app.nora.sorting.asking(&what) {
        return false;
    }
    let Some(host) = app.services.nora.as_ref() else { return false };
    let sorting = &mut app.nora.sorting;
    sorting.last = sorting.last.wrapping_add(1);
    let id = FIRST_ID + sorting.last;
    let answer = (host.call)(NoraCall { id, method, args });
    sorting.asked.push(Asked { id, answer, since: crate::now_ms(), what });
    true
}

/// Once a frame: the answers that came, the state of the mailbox in view now and then, what the flow follows.
pub(crate) fn poll(app: &mut MailApp) {
    if app.nora.sorting.asked.is_empty() && app.nora.sorting.flow.is_none() && !offered(app) {
        return;
    }
    let now = crate::now_ms();
    take_answers(app, now);
    let mut wake: Option<f64> = None;
    if offered(app) {
        let questions = &mut app.nora.sorting.questions;
        if questions.asked_at <= 0.0 || now - questions.asked_at >= QUESTIONS_EVERY_MS {
            questions.asked_at = now;
            call(app, MINE, json!({}), Asking::Mine);
        }
        wake = Some(app.nora.sorting.questions.asked_at + QUESTIONS_EVERY_MS);
    }
    if let Some(account) = box_in_view(app, false)
        && let Some(at) = look_at(app, &account, now)
    {
        wake = Some(wake.map_or(at, |w| w.min(at)));
    }
    if let Some(at) = follow_flow(app, now) {
        wake = Some(wake.map_or(at, |w| w.min(at)));
    }
    if let Some(ctx) = &app.ctx {
        if !app.nora.sorting.asked.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(LOOK_AGAIN_MS));
        } else if let Some(at) = wake {
            ctx.request_repaint_after(std::time::Duration::from_millis((at - now).clamp(50.0, STATE_EVERY_MS) as u64));
        }
    }
}

/// The state of the mailbox in view, read again when it is old (often while its inbox is tidied), and the summary of its
/// last run once it is a new one. When to look again.
fn look_at(app: &mut MailApp, account: &str, now: f64) -> Option<f64> {
    let known = app.nora.sorting.boxes.entry(account.to_string()).or_default();
    let every = known.state.as_ref().filter(|s| s.tidying()).map_or(STATE_EVERY_MS, BoxState::tidy_every);
    let due = known.asked_at + every;
    if known.asked_at <= 0.0 || now >= due {
        known.asked_at = now;
        call(app, STATE, json!({"account": account}), Asking::State(account.to_string()));
        return Some(now + every);
    }
    let latest =
        known.state.as_ref().filter(|s| s.available && s.on).and_then(|s| s.last_run.as_ref()).filter(|r| r.filed > 0).map(|r| r.run.clone());
    if let Some(run) = latest.filter(|run| *run != known.latest_of) {
        known.latest_of = run.clone();
        call(app, JOURNAL, json!({"account": account, "limit": 1}), Asking::Latest(account.to_string(), run));
    }
    Some(due)
}

/// What the flow waits for: what Nora read while she is still reading the mailbox, a plan made in the background, the
/// tidying of the inbox. When to look again.
fn follow_flow(app: &mut MailApp, now: f64) -> Option<f64> {
    let flow = app.nora.sorting.flow.as_mut()?;
    let account = flow.account.clone();
    // Read again while she reads now; not while the inbox is tidied (the state says how far her reading is then).
    // A plan by rules is counted again at each read while she reads (`counting`): read again at the same pace.
    let counting = flow.stage == Stage::Plan && flow.plan.as_ref().is_some_and(|p| p.counting) && now - flow.read_from < SURVEY_GIVE_UP_MS;
    let reading = flow.surveyed
        && !flow.journal_first
        && flow.stage != Stage::Tidying
        && (counting || flow.survey.as_ref().is_some_and(|s| s.read_again(flow.read_from, now)));
    let read_again = reading && now >= flow.survey_next;
    if read_again {
        flow.survey_next = now + SURVEY_EVERY_MS;
    }
    let survey_at = reading.then_some(flow.survey_next);
    // The plan shown while she reads has counts of what she has read so far: read again with what she read, its counts
    // brought up to date (not while the person changes it: their rows are theirs).
    let plan_again = read_again && flow.stage == Stage::Plan && flow.editing.is_none();
    if read_again {
        call(app, SURVEY, json!({"account": account, "days": TIDY_DAYS}), Asking::Survey(account.clone()));
    }
    if plan_again {
        call(app, PLAN, json!({"account": account}), Asking::Plan(account.clone()));
    }
    let staged = follow_stage(app, &account, now);
    match (survey_at, staged) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// What the flow's stage waits for: a plan made in the background, the tidying of the inbox. When to look again.
fn follow_stage(app: &mut MailApp, account: &str, now: f64) -> Option<f64> {
    let flow = app.nora.sorting.flow.as_mut()?;
    let account = account.to_string();
    // A plan by activity that waits for her reading (20 sorted messages) is made as soon as she has them: read again at
    // the reading's pace, and not given up while she reads.
    let waits = flow.plan.as_ref().is_some_and(|p| p.waiting == "reading");
    let plan_every = if waits { SURVEY_EVERY_MS } else { PLAN_EVERY_MS };
    let give_up = if waits { SURVEY_GIVE_UP_MS } else { PLAN_GIVE_UP_MS };
    // The tidying read again every two seconds, at the reading's pace while it waits for her reading.
    let tidy_every = app.nora.sorting.boxes.get(&account).and_then(|k| k.state.as_ref()).map_or(TIDY_EVERY_MS, BoxState::tidy_every);
    let flow = app.nora.sorting.flow.as_mut()?;
    match flow.stage {
        Stage::Proposing if now - flow.since > give_up => {
            flow.error = Some(tl!("Nora is taking too long to make the plan. Try again in a moment.").to_string());
            flow.enter(Stage::Choose, now);
            None
        }
        Stage::Proposing if now >= flow.next => {
            flow.next = now + plan_every;
            call(app, PLAN, json!({"account": account}), Asking::Plan(account));
            Some(now + plan_every)
        }
        Stage::Tidying if now >= flow.next => {
            flow.next = now + tidy_every;
            call(app, STATE, json!({"account": account}), Asking::State(account));
            Some(now + tidy_every)
        }
        Stage::Proposing | Stage::Tidying => Some(flow.next),
        _ => None,
    }
}

fn take_answers(app: &mut MailApp, now: f64) {
    let asked = std::mem::take(&mut app.nora.sorting.asked);
    let mut kept = Vec::with_capacity(asked.len());
    let mut landed = Vec::new();
    for a in asked {
        let result = match a.answer.try_recv() {
            Ok(answer) if answer.id == a.id => Some((answer::sorting(&answer), answer.status)),
            Ok(_) | Err(TryRecvError::Disconnected) => Some((Err(answer::not_reached(tl!("the connection stopped before Nora answered"))), 0)),
            Err(TryRecvError::Empty) if now - a.since > DEADLINE_MS => Some((Err(tl!("it took too long").to_string()), 0)),
            Err(TryRecvError::Empty) => None,
        };
        match result {
            None => kept.push(a),
            Some((result, status)) => landed.push((a.what, result, status)),
        }
    }
    kept.append(&mut app.nora.sorting.asked);
    app.nora.sorting.asked = kept;
    for (what, result, status) in landed {
        land(app, what, result, status, now);
    }
}

/// The flow of `account`, when it is the one in the pane.
fn flow_of<'a>(app: &'a mut MailApp, account: &str) -> Option<&'a mut Flow> {
    app.nora.sorting.flow.as_mut().filter(|f| f.account == account)
}

fn land(app: &mut MailApp, what: Asking, result: Result<Value, String>, status: u16, now: f64) {
    match what {
        Asking::State(account) => match result {
            Ok(v) => {
                let state = read_state(&v);
                took_state(app, &account, state, now);
            }
            Err(why) => {
                // A mailbox the person doesn't read, a session that ended: nothing of it is shown.
                if let Some(known) = app.nora.sorting.boxes.get_mut(&account) {
                    known.state = None;
                }
                if let Some(flow) = flow_of(app, &account) {
                    flow.error = Some(why);
                    if flow.stage == Stage::Loading {
                        flow.stage = Stage::Done;
                    }
                }
            }
        },
        Asking::Latest(account, _) => {
            if let (Ok(v), Some(known)) = (result, app.nora.sorting.boxes.get_mut(&account)) {
                known.latest = read_journal(&v).runs.into_iter().next();
            }
        }
        Asking::Survey(account) => read_now(app, &account, result, status, now),
        Asking::Begin(account) => begun(app, &account, result, status, now),
        Asking::Plan(account) => {
            let state = app.nora.sorting.boxes.get(&account).and_then(|k| k.state.clone());
            let Some(flow) = flow_of(app, &account) else { return };
            match result {
                Ok(v) => {
                    let plan = read_plan(&v);
                    let ready = !plan.status.is_empty();
                    // Why a plan by activity could not be made, once its job has had the time to end (the server keeps
                    // the last one's until then).
                    let failed =
                        (!ready && plan.waiting.is_empty() && now - flow.since >= PLAN_ERROR_AFTER_MS).then(|| plan_failed(&plan.error)).flatten();
                    flow.plan = Some(plan);
                    if flow.stage == Stage::Proposing {
                        if ready {
                            flow.enter(Stage::Plan, now);
                        } else if let Some(why) = failed {
                            flow.error = Some(why);
                            flow.enter(Stage::Choose, now);
                        }
                    }
                }
                // A plan being made, or shown and read again while she reads: a passing failure waits for the next read;
                // the server's refusal (no secretary, a plan that is not the person's to see) is said at once.
                Err(_) if matches!(flow.stage, Stage::Proposing | Stage::Plan) && !refusal(status) => {}
                Err(why) => {
                    flow.error = Some(why);
                    if flow.stage == Stage::Proposing {
                        flow.enter(Stage::Choose, now);
                    }
                }
            }
            settle(flow, state.as_ref(), now);
        }
        Asking::Journal(account) => {
            if let Some(flow) = flow_of(app, &account) {
                match result {
                    Ok(v) => flow.journal = Some(read_journal(&v)),
                    Err(why) => flow.error = Some(why),
                }
            }
        }
        Asking::Mine => {
            let questions = &mut app.nora.sorting.questions;
            match result {
                Ok(v) => {
                    let (list, retained) = read_asks(&v);
                    questions.drafts.retain(|name, _| list.iter().any(|a| a.name == *name));
                    questions.confirming.retain(|name, _| list.iter().any(|a| a.name == *name));
                    questions.list = list;
                    questions.retained = retained;
                }
                // Not the person's to answer (a portal account, a session that ended): none shown; anything else waits.
                Err(_) if matches!(status, 401 | 403) => questions.list.clear(),
                Err(_) => {}
            }
        }
        Asking::Switch(account, after) => switched(app, &account, after, result, now),
        Asking::Propose(account) => {
            let Some(flow) = flow_of(app, &account) else { return };
            match result {
                Ok(v) => {
                    // Asking for a plan may start her reading of a mailbox she had read nothing of.
                    flow.read_from = now;
                    let plan = read_plan(&v);
                    if plan.queued || plan.status.is_empty() {
                        // Made in the background: read again until it is proposed.
                        flow.enter(Stage::Proposing, now);
                        flow.next = now + PLAN_EVERY_MS;
                    } else {
                        flow.plan = Some(plan);
                        flow.enter(Stage::Plan, now);
                    }
                }
                Err(why) => {
                    flow.error = Some(why);
                    flow.enter(Stage::Choose, now);
                }
            }
        }
        Asking::Validate(account, tidied) => {
            let Some(flow) = flow_of(app, &account) else { return };
            match result {
                Ok(v) => {
                    flow.plan = Some(read_plan(&v));
                    flow.editing = None;
                    match v.get("tidy").filter(|t| t.is_object()).map(read_tidy) {
                        Some(tidy) if tidied => {
                            flow.tidy_run = tidy.run;
                            flow.enter(Stage::Tidying, now);
                            flow.next = now + TIDY_EVERY_MS;
                        }
                        _ => flow.enter(Stage::Plan, now),
                    }
                }
                Err(why) => {
                    flow.error = Some(why);
                    flow.enter(Stage::Plan, now);
                }
            }
            refresh(app, &account);
        }
        Asking::Continue(account, interval) => match result {
            Ok(_) => {
                call(app, SWITCH, json!({"account": account, "interval": interval}), Asking::Switch(account.clone(), AfterSwitch::Rhythm(interval)));
            }
            Err(why) => {
                if let Some(flow) = flow_of(app, &account) {
                    flow.error = Some(why);
                    flow.continued = false;
                }
            }
        },
        Asking::Tidy(account) => {
            let Some(flow) = flow_of(app, &account) else { return };
            match result {
                Ok(v) => {
                    flow.tidy_run = read_tidy(&v).run;
                    flow.enter(Stage::Tidying, now);
                    flow.next = now + TIDY_EVERY_MS;
                }
                Err(why) => flow.error = Some(why),
            }
        }
        Asking::StopTidy(account) => {
            if let Err(why) = result
                && let Some(flow) = flow_of(app, &account)
            {
                flow.error = Some(why);
            }
            refresh(app, &account);
        }
        Asking::Undo(account, run) => undone(app, &account, &run, result),
        Asking::Answer(ask) | Asking::Confirm(ask) => noted(app, &ask, result, status),
        Asking::Said(ask) => said_back(app, &ask, result, status),
        Asking::Later(ask) => match result {
            Ok(_) => app.nora.sorting.questions.settled(&ask),
            Err(why) => refused(app, &ask, why, status),
        },
        Asking::UndoCorrection(account, run) => correction_undone(app, &account, &run, result),
    }
}

/// Why the last plan by activity could not be made (`plan.error`), in words: `None` for none.
fn plan_failed(code: &str) -> Option<String> {
    let said = match code {
        "" => return None,
        "too_few" => tl!("This mailbox has too few messages for a plan by activity (fewer than 20 in 30 days): choose another way."),
        "model" => tl!("Nora could not reach her model to make the plan: try again in a moment, or choose another way."),
        "empty" => tl!("Nora found no folder to propose by activity: explain it otherwise, or choose another way."),
        _ => tl!("Nora could not make the plan by activity: try again in a moment, or choose another way."),
    };
    Some(said.to_string())
}

/// Whether a failure is the server's refusal (a session that ended, a box not the person's, `frappe.throw`): said at once,
/// never waited out as a passing one.
fn refusal(status: u16) -> bool {
    matches!(status, 401 | 403 | 417)
}

/// What Nora read came (`survey`). Read again while she is still reading, a passing failure waits for the next read. When
/// her reading moves on (her first messages read, or her reading done): a plan that had no folder yet is asked again in
/// the way chosen (by rules, at once: never the model's again), and the result's journal is read again.
fn read_now(app: &mut MailApp, account: &str, result: Result<Value, String>, status: u16, now: f64) {
    let state = app.nora.sorting.boxes.get(account).and_then(|k| k.state.clone());
    let Some(flow) = flow_of(app, account) else { return };
    flow.survey_next = now + SURVEY_EVERY_MS;
    let mut then = None;
    match result {
        Ok(v) => {
            let survey = read_survey(&v);
            let was = flow.survey.as_ref().map(|s| (s.mails, s.still_reading(flow.read_from, now)));
            let moved =
                was.is_some_and(|(mails, reading)| reading && ((mails == 0 && survey.mails > 0) || !survey.still_reading(flow.read_from, now)));
            flow.survey = Some(survey);
            if moved {
                let empty = flow.plan.as_ref().is_some_and(|p| p.folders.is_empty() && p.status != "validated");
                match (flow.stage, flow.chosen.as_ref()) {
                    (Stage::Plan, Some(mode)) if empty && mode.code != "activity" => then = Some((PROPOSE, String::new())),
                    (Stage::Done, _) => then = Some((JOURNAL, String::new())),
                    _ => {}
                }
            }
        }
        Err(_) if flow.survey.is_some() && !refusal(status) => {}
        Err(why) => flow.error = Some(why),
    }
    settle(flow, state.as_ref(), now);
    let account = account.to_string();
    match then {
        Some((PROPOSE, _)) => {
            // The same way as it was asked (a plan by party: its kinds and its depth).
            let mode = flow_of(app, &account).and_then(|f| f.chosen.as_ref()).map(|m| m.code.clone()).unwrap_or_default();
            let args = flow_of(app, &account)
                .and_then(|f| f.proposed.clone())
                .unwrap_or_else(|| json!({"account": account, "mode": mode, "keep_unread": 1}));
            propose(app, &account, args, now);
        }
        Some(_) => {
            call(app, JOURNAL, json!({"account": account, "limit": JOURNAL_RUNS}), Asking::Journal(account.clone()));
        }
        None => {}
    }
}

/// The mailbox Nora had never read, started on (`begin`): its answer is what she read so far, with how far her reading is
/// (`survey`). A host whose list of these calls is older than `begin` refuses it with nothing (Neodesk 1.3.6: a 403 and no
/// body): she is started by the switch instead (`switch` on, at a quarter of an hour), which makes her secretary and starts
/// the same reading; without a validated plan its answer says `needs_plan`, and the flow goes on to the choice.
fn begun(app: &mut MailApp, account: &str, result: Result<Value, String>, status: u16, now: f64) {
    let bare = answer::sorting(&NoraAnswer { id: 0, status: 403, body: String::new() }).err();
    match result {
        Err(why) if status == 403 && bare.as_deref() == Some(why.as_str()) => {
            let interval = app.nora.sorting.boxes.get(account).and_then(|k| k.state.as_ref()).map_or(FIRST_INTERVAL, |s| {
                if s.intervals.is_empty() || s.intervals.contains(&FIRST_INTERVAL) { FIRST_INTERVAL } else { s.intervals[0] }
            });
            let account = account.to_string();
            call(app, SWITCH, json!({"account": account, "on": 1, "interval": interval}), Asking::Switch(account.clone(), AfterSwitch::Wake));
        }
        result => {
            if let Some(flow) = flow_of(app, account) {
                flow.surveyed = true;
                flow.read_from = now;
            }
            read_now(app, account, result, status, now);
        }
    }
}

/// Ask for a plan (`propose`) with `args`, kept to ask it again the same way; the flow waits for it.
fn propose(app: &mut MailApp, account: &str, args: Value, now: f64) {
    if let Some(flow) = flow_of(app, account) {
        flow.proposed = Some(args.clone());
        flow.enter(Stage::Proposing, now);
        flow.next = now + PLAN_EVERY_MS;
    }
    call(app, PROPOSE, args, Asking::Propose(account.to_string()));
}

/// The flow's mailbox, read: what Nora read of it (`survey`) where she has a secretary for it; where she has none (the
/// first time, for every mailbox), started on (`begin`: her secretary made, her reading of its last 30 days started at
/// once; its answer is what she read so far); where the person may not start her, said so.
fn start_reading(app: &mut MailApp, account: &str, state: &BoxState, now: f64) {
    let Some(flow) = flow_of(app, account) else { return };
    if !state.available || flow.journal_first || flow.surveyed || flow.woke || flow.unstarted {
        return;
    }
    let account = account.to_string();
    if state.secretary {
        flow.surveyed = true;
        call(app, SURVEY, json!({"account": account, "days": TIDY_DAYS}), Asking::Survey(account.clone()));
    } else if state.may_set {
        flow.woke = true;
        call(app, BEGIN, json!({"account": account, "days": TIDY_DAYS}), Asking::Begin(account.clone()));
    } else {
        flow.unstarted = true;
        flow.enter(Stage::Done, now);
    }
}

/// A question answered (by a button, or the person's own words confirmed): it goes, and what Nora noted is said, with the
/// correction it made (a question of a family's folder: what she had filed of it moved where the answer says).
fn noted(app: &mut MailApp, ask: &str, result: Result<Value, String>, status: u16) {
    match result {
        Ok(v) => {
            let mailbox = app.nora.sorting.questions.list.iter().find(|a| a.name == ask).map(|a| a.mailbox.clone()).unwrap_or_default();
            let account = account_of(app, &mailbox);
            let questions = &mut app.nora.sorting.questions;
            questions.settled(ask);
            let words = read_noted(&v);
            let run = name(&v, "run");
            questions.noted = (!words.is_empty() || !run.is_empty()).then_some(Noted { words, account, run });
            questions.error = None;
        }
        Err(why) => refused(app, ask, why, status),
    }
}

/// The name of the server's account for the mailbox of `address`, as the hosting page handed it over (its address where it
/// said no other).
fn account_of(app: &MailApp, address: &str) -> String {
    let handed = app.services.nora.as_ref().and_then(|host| host.mailboxes.iter().find(|m| m.address.trim().eq_ignore_ascii_case(address.trim())));
    handed.map(|m| m.name.trim()).filter(|n| !n.is_empty()).unwrap_or(address.trim()).to_string()
}

/// The correction an answer made, undone: called again while the server says some are left, then said.
fn correction_undone(app: &mut MailApp, account: &str, run: &str, result: Result<Value, String>) {
    match result {
        Ok(v) => {
            let done = read_undone(&v);
            let questions = &mut app.nora.sorting.questions;
            if done.remaining > 0 && questions.undo_rounds < MAX_UNDO_ROUNDS {
                questions.undo_rounds += 1;
                call(app, UNDO, json!({"account": account, "run": run}), Asking::UndoCorrection(account.to_string(), run.to_string()));
                return;
            }
            questions.undo_rounds = 0;
            let mut said = vec![tln!(done.undone, "{n} message is back where it was.", "{n} messages are back where they were.")];
            if done.moved > 0 {
                said.push(tln!(
                    done.moved,
                    "{n} message moved by someone since stays where they put it.",
                    "{n} messages moved by someone since stay where they put them."
                ));
            }
            questions.noted = Some(Noted { words: said.join(" "), account: account.to_string(), run: String::new() });
            questions.error = None;
        }
        Err(why) => {
            let questions = &mut app.nora.sorting.questions;
            questions.undo_rounds = 0;
            questions.error = Some(why);
        }
    }
    // The flow of that mailbox reads its journal again.
    if flow_of(app, account).is_some() {
        let account = account.to_string();
        call(app, JOURNAL, json!({"account": account, "limit": JOURNAL_RUNS}), Asking::Journal(account));
    }
}

/// The person answered in their own words: what Nora would keep of them, for them to confirm or change.
fn said_back(app: &mut MailApp, ask: &str, result: Result<Value, String>, status: u16) {
    match result {
        Ok(v) => {
            let questions = &mut app.nora.sorting.questions;
            match read_confirm(&v) {
                Some(confirming) => {
                    questions.confirming.insert(ask.to_string(), confirming);
                    questions.error = None;
                }
                None => questions.error = Some(tl!("Nora could not make out your answer: say it again, or choose one of hers.").to_string()),
            }
        }
        Err(why) => refused(app, ask, why, status),
    }
}

/// A refusal of an answer: a question that isn't the person's to answer (403) goes without a word; anything else is said.
fn refused(app: &mut MailApp, ask: &str, why: String, status: u16) {
    let questions = &mut app.nora.sorting.questions;
    if status == 403 {
        questions.settled(ask);
    } else {
        questions.error = Some(why);
    }
}

/// Nora's questions are read again at the next frame, unless they were a moment ago (her pane opens, a run is over).
pub(crate) fn fresh_questions(app: &mut MailApp) {
    let questions = &mut app.nora.sorting.questions;
    if crate::now_ms() - questions.asked_at > QUESTIONS_FRESH_MS {
        questions.asked_at = 0.0;
    }
}

/// The state of `account` came: kept for the button, the banner and the switch; the flow moves on from it.
fn took_state(app: &mut MailApp, account: &str, state: BoxState, now: f64) {
    let ask_plan = !state.plan_status.is_empty();
    let known = app.nora.sorting.boxes.entry(account.to_string()).or_default();
    known.state = Some(state.clone());
    let Some(flow) = flow_of(app, account) else { return };
    if flow.stage == Stage::Tidying && !state.tidying() {
        // The tidying is over (done, stopped, or never started): its run in the journal.
        if let Some(error) = state.tidy.as_ref().map(|t| t.error.clone()).filter(|e| !e.is_empty()) {
            flow.error = Some(error);
        }
        flow.enter(Stage::Done, now);
        let account = account.to_string();
        call(app, JOURNAL, json!({"account": account, "limit": JOURNAL_RUNS}), Asking::Journal(account));
        app.nora.sorting.questions.asked_at = 0.0;
        return;
    }
    if flow.stage == Stage::Loading && state.tidying() {
        flow.tidy_run = state.tidy.as_ref().map(|t| t.run.clone()).unwrap_or_default();
        flow.enter(Stage::Tidying, now);
        flow.next = now + TIDY_EVERY_MS;
        return;
    }
    let wants_plan = flow.stage == Stage::Loading && ask_plan && flow.plan.is_none() && !flow.journal_first;
    let read = flow.stage == Stage::Loading;
    settle(flow, Some(&state), now);
    if read {
        start_reading(app, account, &state, now);
    }
    if wants_plan {
        let account = account.to_string();
        call(app, PLAN, json!({"account": account}), Asking::Plan(account));
    }
}

/// Where a flow that was reading goes once it has what it needs: the journal (opened from the banner), the plan there
/// is, or the choice of a way to file.
fn settle(flow: &mut Flow, state: Option<&BoxState>, now: f64) {
    if flow.stage != Stage::Loading {
        return;
    }
    let Some(state) = state else { return };
    if flow.journal_first {
        flow.enter(Stage::Done, now);
    } else if !state.plan_status.is_empty() {
        if flow.plan.as_ref().is_some_and(|p| !p.folders.is_empty() || !p.status.is_empty()) {
            flow.enter(Stage::Plan, now);
        }
    } else if flow.survey.is_some() {
        flow.enter(Stage::Choose, now);
    }
}

/// Read the state of `account` again at the next frame.
fn refresh(app: &mut MailApp, account: &str) {
    if let Some(known) = app.nora.sorting.boxes.get_mut(account) {
        known.asked_at = 0.0;
    }
    call(app, STATE, json!({"account": account}), Asking::State(account.to_string()));
}

fn switched(app: &mut MailApp, account: &str, after: AfterSwitch, result: Result<Value, String>, now: f64) {
    let state = match result {
        Ok(v) => read_state(&v),
        Err(why) => {
            match after {
                AfterSwitch::FromCard(_) => {
                    if let Some(card) = app.nora.sorting.card.as_mut() {
                        card.error = Some(why);
                        card.busy = false;
                    }
                }
                _ => match flow_of(app, account) {
                    Some(flow) => flow.error = Some(why),
                    None => app.status(why),
                },
            }
            return;
        }
    };
    let needs_plan = state.needs_plan;
    app.nora.sorting.boxes.entry(account.to_string()).or_default().state = Some(state.clone());
    match after {
        AfterSwitch::FromCard(mode) if needs_plan && mode == "parties" => {
            // A plan by party: Nora asks first, in the flow (which kinds, how deep).
            app.nora.sorting.card = None;
            open(app, account, false);
            if let Some(flow) = flow_of(app, account) {
                flow.on_intent = true;
            }
            if let Some(chosen) = state.modes.iter().find(|m| m.code == mode).cloned() {
                act(app, Intent::Choose(chosen));
            }
        }
        AfterSwitch::FromCard(mode) => {
            app.nora.sorting.card = None;
            if needs_plan {
                // No validated plan: the flow opens on one, made in the way chosen on the card.
                open(app, account, false);
                if let Some(flow) = flow_of(app, account) {
                    flow.on_intent = true;
                    flow.chosen = state.modes.iter().find(|m| m.code == mode).cloned();
                    flow.enter(Stage::Proposing, now);
                    flow.next = now + PLAN_EVERY_MS;
                }
                call(app, PROPOSE, json!({"account": account, "mode": mode, "keep_unread": 1}), Asking::Propose(account.to_string()));
            }
        }
        AfterSwitch::Wake => {
            // Started on: no plan yet is what comes next (`needs_plan`), what she read first.
            if let Some(flow) = flow_of(app, account) {
                flow.read_from = now;
                flow.surveyed = true;
            }
            let account = account.to_string();
            call(app, SURVEY, json!({"account": account, "days": TIDY_DAYS}), Asking::Survey(account.clone()));
        }
        AfterSwitch::Rhythm(minutes) => {
            if let Some(flow) = flow_of(app, account) {
                flow.said = Some(fmt(tl!("Noted: I carry on by myself ({every})."), &[("every", &every(minutes))]));
            }
        }
        AfterSwitch::Nothing => {
            // Turned on where the plan was no longer validated: the card asks how to file.
            if needs_plan {
                open_card(app, account);
            }
        }
    }
}

fn undone(app: &mut MailApp, account: &str, run: &str, result: Result<Value, String>) {
    let Some(flow) = flow_of(app, account) else { return };
    match result {
        Ok(v) => {
            let done = read_undone(&v);
            if done.remaining > 0 && flow.undo_rounds < MAX_UNDO_ROUNDS {
                // The server undoes 500 mails a call: the rest, at once.
                flow.undo_rounds += 1;
                call(app, UNDO, json!({"account": account, "run": run}), Asking::Undo(account.to_string(), run.to_string()));
                return;
            }
            flow.undo_rounds = 0;
            let mut said = vec![tln!(done.undone, "{n} message is back where it was.", "{n} messages are back where they were.")];
            if done.moved > 0 {
                said.push(tln!(
                    done.moved,
                    "{n} message moved by someone since stays where they put it.",
                    "{n} messages moved by someone since stay where they put them."
                ));
            }
            if done.gone > 0 {
                said.push(tln!(
                    done.gone,
                    "{n} message stays where it is: its folder is gone.",
                    "{n} messages stay where they are: their folder is gone."
                ));
            }
            if done.remaining > 0 {
                said.push(tln!(done.remaining, "{n} message is still to put back: undo again.", "{n} messages are still to put back: undo again."));
            }
            flow.said = Some(said.join(" "));
        }
        Err(why) => {
            flow.undo_rounds = 0;
            flow.error = Some(why);
        }
    }
    let account = account.to_string();
    call(app, JOURNAL, json!({"account": account, "limit": JOURNAL_RUNS}), Asking::Journal(account.clone()));
    refresh(app, &account);
}

/// Open the flow in her pane for `account`.
fn open(app: &mut MailApp, account: &str, journal: bool) {
    if !offered(app) {
        return;
    }
    let now = crate::now_ms();
    app.nora.sorting.flow = Some(Flow::new(account.to_string(), journal, now));
    // The pane opens on it (and the Échanges pane makes way).
    let _ = super::assist::toggle(app, &json!({"open": true}));
    let account = account.to_string();
    refresh(app, &account);
    call(app, JOURNAL, json!({"account": account, "limit": JOURNAL_RUNS}), Asking::Journal(account.clone()));
    fresh_questions(app);
    // A mailbox known to have her secretary is read at once; any other waits for its state, read again above: one she has
    // never read is started on first, and only on what the server says now.
    let known = app.nora.sorting.boxes.get(&account).and_then(|k| k.state.clone()).filter(|s| s.secretary);
    if let Some(state) = known {
        start_reading(app, &account, &state, now);
    }
}

/// Put the activation card up for `account`.
fn open_card(app: &mut MailApp, account: &str) {
    let Some(state) = app.nora.sorting.available(account) else { return };
    let mode = if state.modes.iter().any(|m| m.code == state.plan_mode) {
        state.plan_mode.clone()
    } else {
        state.modes.first().map(|m| m.code.clone()).unwrap_or_default()
    };
    let interval = if state.intervals.contains(&state.interval) {
        state.interval
    } else {
        state.intervals.iter().copied().find(|n| *n == 15).or(state.intervals.first().copied()).unwrap_or(15)
    };
    let prepare = state.prepare_documents || state.prepare_replies;
    app.nora.sorting.card = Some(Card { account: account.to_string(), mode, interval, prepare, error: None, busy: false });
}

/// The rhythm in the interface's language: « Toutes les 15 minutes », « Toutes les heures ».
pub(crate) fn every(minutes: u64) -> String {
    match minutes {
        60 => tl!("Every hour").to_string(),
        m if m > 60 && m % 60 == 0 => tln!(m / 60, "Every {n} hour", "Every {n} hours"),
        m => tln!(m, "Every {n} minute", "Every {n} minutes"),
    }
}

/// Do what the person asked.
pub(crate) fn act(app: &mut MailApp, intent: Intent) {
    let now = crate::now_ms();
    let flow_account = app.nora.sorting.flow.as_ref().map(|f| f.account.clone());
    match intent {
        Intent::Open { account, journal } => open(app, &account, journal),
        Intent::Back => app.nora.sorting.flow = None,
        Intent::Choose(mode) if mode.code == "parties" => {
            // Nora asks first: which kinds of party (the server's ticked), and how deep.
            let Some(account) = flow_account else { return };
            let ticked: Vec<String> = app
                .nora
                .sorting
                .available(&account)
                .map(|s| s.party_types.iter().filter(|p| p.default).map(|p| p.code.clone()).collect())
                .unwrap_or_default();
            if let Some(flow) = app.nora.sorting.flow.as_mut() {
                flow.error = None;
                flow.said = None;
                flow.chosen = Some(mode);
                flow.party_types = ticked;
                flow.per_entity = None;
                flow.enter(Stage::Parties, now);
            }
        }
        Intent::Choose(mode) => {
            let Some(account) = flow_account else { return };
            if let Some(flow) = app.nora.sorting.flow.as_mut() {
                flow.error = None;
                flow.said = None;
                flow.chosen = Some(mode.clone());
                flow.per_entity = None;
            }
            propose(app, &account, json!({"account": account, "mode": mode.code, "keep_unread": 1}), now);
        }
        Intent::PartyType(code) => {
            if let Some(flow) = app.nora.sorting.flow.as_mut().filter(|f| f.stage == Stage::Parties) {
                if let Some(at) = flow.party_types.iter().position(|c| *c == code) {
                    flow.party_types.remove(at);
                } else {
                    flow.party_types.push(code);
                }
            }
        }
        Intent::PerEntity(per_entity) => {
            let Some(account) = flow_account else { return };
            // The kinds ticked, in the server's order; none ticked: nothing asked.
            let order: Vec<String> =
                app.nora.sorting.available(&account).map(|s| s.party_types.iter().map(|p| p.code.clone()).collect()).unwrap_or_default();
            let Some(flow) = app.nora.sorting.flow.as_mut().filter(|f| f.stage == Stage::Parties) else { return };
            let types: Vec<String> = order.into_iter().filter(|c| flow.party_types.contains(c)).collect();
            if types.is_empty() {
                return;
            }
            flow.per_entity = Some(per_entity);
            flow.error = None;
            let args = json!({"account": account, "mode": "parties", "types": types, "per_entity": u8::from(per_entity), "keep_unread": 1});
            propose(app, &account, args, now);
        }
        Intent::Explain => {
            if let Some(flow) = app.nora.sorting.flow.as_mut() {
                flow.error = None;
                flow.enter(Stage::Explain, now);
            }
        }
        Intent::ProposeWishes => {
            let Some(account) = flow_account else { return };
            let Some(flow) = app.nora.sorting.flow.as_mut() else { return };
            let wishes: String = flow.wishes.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(600).collect();
            let label = if wishes.is_empty() { tl!("Let me explain").to_string() } else { wishes.clone() };
            flow.chosen = Some(Mode { code: "activity".into(), label });
            flow.error = None;
            propose(app, &account, json!({"account": account, "mode": "activity", "wishes": wishes, "keep_unread": 1}), now);
        }
        Intent::Edit => {
            if let Some(flow) = app.nora.sorting.flow.as_mut()
                && let Some(plan) = &flow.plan
            {
                flow.editing = Some(
                    plan.folders
                        .iter()
                        .map(|f| EditRow { folder: f.folder.clone(), holds: f.holds.clone(), takes: f.takes.clone(), count: f.count, kept: true })
                        .collect(),
                );
            }
        }
        Intent::KeepPlan => {
            if let Some(flow) = app.nora.sorting.flow.as_mut() {
                flow.editing = None;
                if flow.stage == Stage::Choose {
                    flow.enter(Stage::Plan, now);
                }
            }
        }
        Intent::NewPlan => {
            if let Some(flow) = app.nora.sorting.flow.as_mut() {
                flow.editing = None;
                flow.chosen = None;
                flow.per_entity = None;
                flow.enter(Stage::Choose, now);
            }
        }
        Intent::FileNow => file_now(app),
        Intent::StopTidy => {
            if let Some(account) = flow_account {
                call(app, STOP_TIDY, json!({"account": account}), Asking::StopTidy(account));
            }
        }
        Intent::Undo(run) => {
            if let Some(account) = flow_account {
                if let Some(flow) = app.nora.sorting.flow.as_mut() {
                    flow.error = None;
                    flow.said = None;
                    flow.undo_rounds = 0;
                }
                call(app, UNDO, json!({"account": account, "run": run}), Asking::Undo(account.clone(), run));
            }
        }
        Intent::Continue(rhythm) => {
            let Some(account) = flow_account else { return };
            if let Some(flow) = app.nora.sorting.flow.as_mut() {
                flow.continued = true;
                flow.error = None;
                if rhythm.is_none() {
                    flow.said = Some(tl!("Noted: I file only when you ask.").to_string());
                }
            }
            if let Some(minutes) = rhythm {
                call(app, VALIDATE, json!({"account": account, "file_mails": 1}), Asking::Continue(account.clone(), minutes));
            }
        }
        Intent::Answer(ask, choice) => {
            app.nora.sorting.questions.noted = None;
            call(app, ANSWER, json!({"name": ask, "choice": choice, "channel": "button"}), Asking::Answer(ask));
        }
        Intent::SayAnswer(ask) => {
            let questions = &mut app.nora.sorting.questions;
            let words: String = questions.drafts.get(&ask).map(|w| w.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
            let words: String = words.chars().take(MAX_SAID).collect();
            if words.is_empty() {
                return;
            }
            questions.noted = None;
            call(app, ANSWER, json!({"name": ask, "text": words, "channel": "text"}), Asking::Said(ask));
        }
        Intent::ConfirmAnswer(ask) => {
            let Some(confirming) = app.nora.sorting.questions.confirming.get(&ask).cloned() else { return };
            let retained: String = confirming.retained.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_RETAINED).collect();
            if retained.is_empty() {
                return;
            }
            let mut args = json!({"name": ask, "retained": retained});
            if !confirming.choice.is_empty() {
                args["choice"] = json!(confirming.choice);
            }
            if !confirming.scope.is_empty() {
                args["scope"] = json!(confirming.scope);
            }
            call(app, CONFIRM, args, Asking::Confirm(ask));
        }
        Intent::Rephrase(ask) => {
            app.nora.sorting.questions.confirming.remove(&ask);
        }
        Intent::Later(ask) => {
            call(app, LATER, json!({"name": ask}), Asking::Later(ask));
        }
        Intent::UndoNoted => {
            let Some(noted) = app.nora.sorting.questions.noted.clone().filter(|n| !n.run.is_empty() && !n.account.is_empty()) else { return };
            app.nora.sorting.questions.undo_rounds = 0;
            call(app, UNDO, json!({"account": noted.account, "run": noted.run}), Asking::UndoCorrection(noted.account.clone(), noted.run.clone()));
        }
        Intent::Switch { account, on } => {
            let Some((may_set, validated)) = app.nora.sorting.available(&account).map(|s| (s.may_set, s.validated())) else { return };
            if !may_set {
                return;
            }
            if on && !validated {
                // No validated plan: the card asks how to file before anything is turned on.
                open_card(app, &account);
            } else {
                call(app, SWITCH, json!({"account": account, "on": u8::from(on)}), Asking::Switch(account.clone(), AfterSwitch::Nothing));
            }
        }
        Intent::Activate => {
            let Some(card) = app.nora.sorting.card.as_mut() else { return };
            card.busy = true;
            card.error = None;
            let (account, mode, interval, prepare) = (card.account.clone(), card.mode.clone(), card.interval, u8::from(card.prepare));
            let args = json!({"account": account, "on": 1, "interval": interval, "prepare_documents": prepare, "prepare_replies": prepare});
            if !call(app, SWITCH, args, Asking::Switch(account, AfterSwitch::FromCard(mode)))
                && let Some(card) = app.nora.sorting.card.as_mut()
            {
                card.busy = false;
            }
        }
        Intent::CloseCard => app.nora.sorting.card = None,
        Intent::Retry => {
            let Some(account) = flow_account else { return };
            let journal = app.nora.sorting.flow.as_ref().is_some_and(|f| f.journal_first);
            open(app, &account, journal);
        }
    }
}

/// « Ranger maintenant »: a plan validated already (and not changed) tidies the inbox; otherwise the plan is validated as the
/// person left it, the inbox tidied with it.
fn file_now(app: &mut MailApp) {
    let Some(flow) = app.nora.sorting.flow.as_mut() else { return };
    let account = flow.account.clone();
    flow.error = None;
    flow.said = None;
    flow.filing = true;
    let validated = flow.plan.as_ref().is_some_and(|p| p.status == "validated");
    let edited = flow.editing.as_ref().map(|rows| {
        rows.iter()
            .filter(|r| r.kept && !r.folder.trim().is_empty())
            .map(|r| json!({"folder": r.folder.trim(), "holds": r.holds, "takes": r.takes}))
            .collect::<Vec<_>>()
    });
    if validated && edited.is_none() {
        call(app, TIDY, json!({"account": account, "days": TIDY_DAYS}), Asking::Tidy(account.clone()));
        return;
    }
    let file_mails = u8::from(flow.on_intent);
    let folders = edited.map_or(Value::Null, Value::Array);
    call(
        app,
        VALIDATE,
        json!({"account": account, "folders": folders, "file_mails": file_mails, "tidy_days": TIDY_DAYS}),
        Asking::Validate(account.clone(), true),
    );
}

/// The person closed her pane: the flow goes with it.
pub(crate) fn closed(app: &mut MailApp) {
    app.nora.sorting.flow = None;
}

#[cfg(test)]
#[path = "sorting_tests.rs"]
pub(crate) mod tests;
