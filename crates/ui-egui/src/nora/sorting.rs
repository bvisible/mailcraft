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
//!   again while the server says some are left), answers a question (`nora.api.asks.answer`, `later`);
//! - the activation card ([`Card`]) turns the filing on (`switch`) and, without a validated plan, opens the flow on a plan.
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
pub const ANSWER: &str = "nora.api.asks.answer";
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
}

impl Tidy {
    pub(crate) fn running(&self) -> bool {
        self.status == "running"
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
}

impl Survey {
    pub(crate) fn kind(&self, kind: &str) -> u64 {
        self.kinds.get(kind).copied().unwrap_or(0)
    }
}

/// A folder of the plan.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PlanFolder {
    pub(crate) folder: String,
    pub(crate) holds: String,
    pub(crate) count: u64,
    /// What a rule sends there: `clients`, `notifications`, `newsletters`, `invoices`, `spam`, or "".
    pub(crate) takes: String,
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
    /// `reading` (a reading that moved mail) or `tidy` (the inbox tidied).
    pub(crate) kind: String,
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

/// One of Nora's questions about the mailbox (`asks`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Ask {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) context: String,
    pub(crate) choices: Vec<(String, String)>,
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
    BoxState {
        address: text(m, "address"),
        available: flag(m, "available"),
        unavailable: status(m, "unavailable"),
        streams: flag(m, "streams"),
        modes,
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

/// `state.tidy`, and the answers of `tidy` and `stop_tidy`.
pub(crate) fn read_tidy(t: &Value) -> Tidy {
    Tidy {
        status: status(t, "status"),
        run: name(t, "run"),
        filed: count(t, "filed"),
        left: count(t, "left"),
        seconds: seconds(t, "seconds"),
        error: text(t, "error"),
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
    }
}

/// `plan`, and the answers of `propose` and `validate`.
pub(crate) fn read_plan(m: &Value) -> Plan {
    let folders = items(m, "folders")
        .map(|f| PlanFolder { folder: text(f, "folder"), holds: text(f, "holds"), count: count(f, "count"), takes: status(f, "takes") })
        .filter(|f| !f.folder.is_empty())
        .collect();
    let clients = m.get("clients").filter(|c| c.is_object()).map(|c| Clients {
        shown: items(c, "shown").map(|s| (text(s, "name"), count(s, "count"))).filter(|(n, _)| !n.is_empty()).take(5).collect(),
        more: count(c, "more"),
        more_count: count(c, "more_count"),
    });
    Plan {
        status: status(m, "status"),
        mode: status(m, "mode"),
        keep_unread_hours: count(m, "keep_unread_hours"),
        may_set: flag(m, "may_set"),
        since_said: text(m, "since_said"),
        folders,
        clients,
        queued: flag(m, "queued"),
    }
}

/// `journal`.
pub(crate) fn read_journal(m: &Value) -> Journal {
    let runs = items(m, "runs")
        .map(|r| Run {
            run: name(r, "run"),
            kind: status(r, "kind"),
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

/// `asks`.
pub(crate) fn read_asks(m: &Value) -> Vec<Ask> {
    items(m, "asks")
        .map(|a| Ask {
            name: name(a, "name"),
            title: text(a, "title"),
            context: text(a, "context"),
            choices: items(a, "choices")
                .map(|c| (name(c, "code"), text(c, "label")))
                .filter(|(code, label)| !code.is_empty() && !label.is_empty())
                .take(6)
                .collect(),
        })
        .filter(|a| !a.name.is_empty() && !a.title.is_empty())
        .take(5)
        .collect()
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
    Plan(String),
    Journal(String),
    Asks(String),
    Switch(String, AfterSwitch),
    Propose(String),
    /// Validated, the inbox tidied too when `true`.
    Validate(String, bool),
    /// « Ensuite, je continue ? »: the plan files the new mail, then this rhythm (minutes).
    Continue(String, u64),
    Tidy(String),
    StopTidy(String),
    Undo(String, String),
    Answer(String, String),
    Later(String, String),
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
    pub(crate) asks: Vec<Ask>,
    /// The way to file the person chose (its words are shown as theirs).
    pub(crate) chosen: Option<Mode>,
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
    next: f64,
    undo_rounds: u32,
}

impl Flow {
    pub(crate) fn new(account: String, journal_first: bool, now: f64) -> Flow {
        Flow {
            account,
            stage: Stage::Loading,
            survey: None,
            plan: None,
            journal: None,
            asks: Vec::new(),
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
        }
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

/// What the window knows of « Trier avec Nora » while it runs. Nothing of it is kept between runs.
#[derive(Default)]
pub(crate) struct Sorting {
    last: u64,
    asked: Vec<Asked>,
    /// Each mailbox looked at, by the name of the server's account for it.
    pub(crate) boxes: BTreeMap<String, Known>,
    pub(crate) flow: Option<Flow>,
    pub(crate) card: Option<Card>,
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
                    | Asking::Propose(_)
                    | Asking::Validate(..)
                    | Asking::Continue(..)
                    | Asking::Tidy(_)
                    | Asking::StopTidy(_)
                    | Asking::Undo(..)
                    | Asking::Answer(..)
                    | Asking::Later(..)
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
    Answer(String, String),
    Later(String),
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
    if let Some(account) = box_in_view(app, false) {
        wake = look_at(app, &account, now);
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
    let every = if known.state.as_ref().is_some_and(BoxState::tidying) { TIDY_EVERY_MS } else { STATE_EVERY_MS };
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

/// What the flow waits for: a plan made in the background, the tidying of the inbox. When to look again.
fn follow_flow(app: &mut MailApp, now: f64) -> Option<f64> {
    let flow = app.nora.sorting.flow.as_mut()?;
    let account = flow.account.clone();
    match flow.stage {
        Stage::Proposing if now - flow.since > PLAN_GIVE_UP_MS => {
            flow.error = Some(tl!("Nora is taking too long to make the plan. Try again in a moment.").to_string());
            flow.enter(Stage::Choose, now);
            None
        }
        Stage::Proposing if now >= flow.next => {
            flow.next = now + PLAN_EVERY_MS;
            call(app, PLAN, json!({"account": account}), Asking::Plan(account));
            Some(now + PLAN_EVERY_MS)
        }
        Stage::Tidying if now >= flow.next => {
            flow.next = now + TIDY_EVERY_MS;
            call(app, STATE, json!({"account": account}), Asking::State(account));
            Some(now + TIDY_EVERY_MS)
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
            Ok(answer) if answer.id == a.id => Some(answer::sorting(&answer)),
            Ok(_) | Err(TryRecvError::Disconnected) => Some(Err(answer::not_reached(tl!("the connection stopped before Nora answered")))),
            Err(TryRecvError::Empty) if now - a.since > DEADLINE_MS => Some(Err(tl!("it took too long").to_string())),
            Err(TryRecvError::Empty) => None,
        };
        match result {
            None => kept.push(a),
            Some(result) => landed.push((a.what, result)),
        }
    }
    kept.append(&mut app.nora.sorting.asked);
    app.nora.sorting.asked = kept;
    for (what, result) in landed {
        land(app, what, result, now);
    }
}

/// The flow of `account`, when it is the one in the pane.
fn flow_of<'a>(app: &'a mut MailApp, account: &str) -> Option<&'a mut Flow> {
    app.nora.sorting.flow.as_mut().filter(|f| f.account == account)
}

fn land(app: &mut MailApp, what: Asking, result: Result<Value, String>, now: f64) {
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
        Asking::Survey(account) => {
            let state = app.nora.sorting.boxes.get(&account).and_then(|k| k.state.clone());
            if let Some(flow) = flow_of(app, &account) {
                match result {
                    Ok(v) => flow.survey = Some(read_survey(&v)),
                    Err(why) => flow.error = Some(why),
                }
                settle(flow, state.as_ref(), now);
            }
        }
        Asking::Plan(account) => {
            let state = app.nora.sorting.boxes.get(&account).and_then(|k| k.state.clone());
            let Some(flow) = flow_of(app, &account) else { return };
            match result {
                Ok(v) => {
                    let plan = read_plan(&v);
                    let ready = !plan.status.is_empty();
                    flow.plan = Some(plan);
                    if flow.stage == Stage::Proposing && ready {
                        flow.enter(Stage::Plan, now);
                    }
                }
                // A plan being made, read again: its errors wait for the next read.
                Err(_) if flow.stage == Stage::Proposing => {}
                Err(why) => flow.error = Some(why),
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
        Asking::Asks(account) => {
            if let (Some(flow), Ok(v)) = (flow_of(app, &account), result) {
                flow.asks = read_asks(&v);
            }
        }
        Asking::Switch(account, after) => switched(app, &account, after, result, now),
        Asking::Propose(account) => {
            let Some(flow) = flow_of(app, &account) else { return };
            match result {
                Ok(v) => {
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
        Asking::Answer(account, ask) | Asking::Later(account, ask) => {
            let Some(flow) = flow_of(app, &account) else { return };
            match result {
                Ok(v) => {
                    flow.asks.retain(|a| a.name != ask);
                    let said = text(&v, "said");
                    if !said.is_empty() {
                        flow.said = Some(said);
                    }
                }
                Err(why) => flow.error = Some(why),
            }
        }
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
        call(app, JOURNAL, json!({"account": account, "limit": JOURNAL_RUNS}), Asking::Journal(account.clone()));
        call(app, ASKS, json!({"account": account}), Asking::Asks(account));
        return;
    }
    if flow.stage == Stage::Loading && state.tidying() {
        flow.tidy_run = state.tidy.as_ref().map(|t| t.run.clone()).unwrap_or_default();
        flow.enter(Stage::Tidying, now);
        flow.next = now + TIDY_EVERY_MS;
        return;
    }
    let wants_plan = flow.stage == Stage::Loading && ask_plan && flow.plan.is_none() && !flow.journal_first;
    settle(flow, Some(&state), now);
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
    call(app, ASKS, json!({"account": account}), Asking::Asks(account.clone()));
    if !journal {
        call(app, SURVEY, json!({"account": account, "days": TIDY_DAYS}), Asking::Survey(account));
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
        Intent::Choose(mode) => {
            let Some(account) = flow_account else { return };
            if let Some(flow) = app.nora.sorting.flow.as_mut() {
                flow.error = None;
                flow.said = None;
                flow.chosen = Some(mode.clone());
                flow.enter(Stage::Proposing, now);
                flow.next = now + PLAN_EVERY_MS;
            }
            call(app, PROPOSE, json!({"account": account, "mode": mode.code, "keep_unread": 1}), Asking::Propose(account));
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
            flow.enter(Stage::Proposing, now);
            flow.next = now + PLAN_EVERY_MS;
            call(app, PROPOSE, json!({"account": account, "mode": "activity", "wishes": wishes, "keep_unread": 1}), Asking::Propose(account));
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
            if let Some(account) = flow_account {
                call(app, ANSWER, json!({"name": ask, "choice": choice, "channel": "button"}), Asking::Answer(account, ask));
            }
        }
        Intent::Later(ask) => {
            if let Some(account) = flow_account {
                call(app, LATER, json!({"name": ask}), Asking::Later(account, ask));
            }
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
