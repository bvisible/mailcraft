// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The sentences that a JMAP Send/Receive and the engine write in English, read back in the interface's language where
//! they are drawn: the status bar, Send/Receive Progress, the banner of a message held in the Outbox, a command's
//! refusal. The crates and the engine are left as they are: they write English for the log, the control channel and
//! agents, and a mailbox file never holds anything else (a message's `held` is saved with it).
//!
//! A line is matched against a table of the sentences the crates write ([`rows`]), each the crates' English with
//! `{placeholders}`. A row reads a line whole or not at all; a placeholder holds a value (a number, a subject, a
//! server's words), and one that holds a reason (`{why}`) is read again with the rows of its own family, so a line
//! made of a frame round a reason round the transport's words is followed down. What a server said is never
//! translated, and a line that no row reads is shown as it came.
//!
//! What a placeholder may hold follows its name ([`Kind`]); the families and the rows of each are in [`rows`].
//!
//! A line may end with sentences that the engine puts after many of its lines (`It is kept on this computer only.`,
//! `It is back in the calendar.`): each is a row of its own ([`Family::Tail`]), and a line is read as what comes before
//! them and them, when it ends with one ([`explain`]). An event or a contact with no name is one the engine calls
//! `an event` or `a contact`, which are words of the interface's ([`NAMES`]).

mod pattern;
mod rows;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::sync::OnceLock;

use mailcraft_model::{FolderKind, Mailbox};

use self::pattern::{Budget, Pattern};
use super::{Lang, catalog, current, fmt, t_in};

/// The longest text that is read; a longer one is shown as it came.
const MAX_TEXT: usize = 1_024;
/// How deep reasons are read inside reasons (a line, its reason, the words under that: three).
const MAX_DEPTH: usize = 4;
/// How many ways of reading a line may be tried (rows, and places where a placeholder may end) before it is left as it came.
const MAX_STEPS: usize = 10_000;
/// How many sentences of [`Family::Tail`] are read off the end of a line.
const MAX_TAILS: usize = 3;

/// The word the engine puts where a line had an address ([`Kind::Addresses`]).
pub const ADDRESS: &str = run_word!("(address)");
/// The word the engine gives a message with no subject ([`Kind::Subject`]).
pub const NO_SUBJECT: &str = run_word!("(no subject)");
/// What the browser's runner says in place of the browser's own words when it gave none ([`Kind::Text`]).
pub const NO_REASON: &str = run_word!("no reason given");
/// The words of ours that a value may be, which are the interface's.
pub const WORDS: [&str; 3] = [ADDRESS, NO_SUBJECT, NO_REASON];
/// What the engine calls an event or a contact that has no name, where a line names one (`{name}`): words of the
/// interface's.
pub const NAMES: [&str; 3] = [run_word!("an event"), run_word!("a contact"), run_word!("A contact")];

/// What a text is read as: a whole line, or a part of one that a placeholder of a line holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Family {
    /// A whole line: a problem of a run, its status, a command's refusal, what Undo couldn't do.
    Line,
    /// What a message held in the Outbox says of itself (`Message::held`).
    Banner,
    /// What went wrong, under the frame of a line of a run (`Couldn't reach the server: {why}`).
    Why,
    /// The words under a failure (`network: {text}`).
    Inner,
    /// Why a message or a draft is held or refused.
    Hold,
    /// Why it isn't known whether a message went.
    Doubt,
    /// What a change of a message the server refused was to do.
    Doing,
    /// Why the server refused a change.
    Cause,
    /// What is wrong with the address of a JMAP server.
    Server,
    /// A sentence the engine puts after a line, about what became of the item (`It is back in the calendar.`).
    Tail,
    /// What of an event a server shows otherwise than it was sent (`its time and its title`).
    Parts,
}

impl Family {
    /// Every family, in order.
    pub const ALL: [Family; 11] = [
        Family::Line,
        Family::Banner,
        Family::Why,
        Family::Inner,
        Family::Hold,
        Family::Doubt,
        Family::Doing,
        Family::Cause,
        Family::Server,
        Family::Tail,
        Family::Parts,
    ];

    fn index(self) -> usize {
        self as usize
    }
}

/// A sentence the crates write, as the table holds it.
#[derive(Debug)]
pub struct Row {
    /// The source file that writes it, from the workspace's root (the census of the tests holds the two to each other).
    pub at: &'static str,
    pub family: Family,
    /// The crates' English, with a `{name}` for each value (written with `run_word!`, so that every catalog has it).
    pub english: &'static str,
    /// The family that each placeholder holding a reason ([`Kind::Reason`]) is read with.
    pub reasons: &'static [(&'static str, Family)],
}

impl Row {
    fn reason(&self, name: &str) -> Option<Family> {
        self.reasons.iter().find(|(slot, _)| *slot == name).map(|(_, family)| *family)
    }
}

/// Every row of the table.
pub fn rows() -> &'static [Row] {
    rows::ROWS
}

/// What a text reads as in a language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub text: String,
    /// The reasons inside it that no row of their family reads, kept as they came (a server's words, the transport's).
    pub unread: Vec<(Family, String)>,
    /// The English of every row that read a part of it, the outermost first.
    pub rows: Vec<&'static str>,
}

/// What a placeholder may hold, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Digits.
    Number,
    /// A JMAP type word: letters and digits, 40 at most.
    Token,
    /// A message's subject as a line shows it; [`NO_SUBJECT`] is a word of the interface's.
    Subject,
    /// Addresses; [`ADDRESS`] is a word of the interface's.
    Addresses,
    /// A folder's name; a standard folder's is the one the folder pane gives it.
    Folder,
    /// A reason, read again with the family the row names.
    Reason,
    /// Text shown as it came: a server's words, a name, a size.
    Text,
}

impl Kind {
    pub(crate) fn of(name: &str) -> Option<Kind> {
        Some(match name {
            "n" | "max" | "most" | "status" | "done" | "total" | "number" | "folders" => Kind::Number,
            "kind" => Kind::Token,
            "subject" => Kind::Subject,
            "address" | "addresses" => Kind::Addresses,
            "folder" => Kind::Folder,
            "why" | "doubt" | "doing" | "text" | "parts" | "one" | "two" | "three" | "four" => Kind::Reason,
            "said" | "description" | "browser" | "parser" | "method" | "key" | "limit" | "id" | "account" | "size" | "name" | "title" | "copy" => {
                Kind::Text
            }
            _ => return None,
        })
    }

    /// Whether `value` may be what a placeholder of this kind holds.
    fn fits(self, value: &str) -> bool {
        match self {
            // Digits, or digits in groups of three as English writes a large number (`100,000`).
            Kind::Number => !value.is_empty() && value.len() <= 20 && grouped(value),
            Kind::Token => !value.is_empty() && value.len() <= 40 && value.bytes().all(|b| b.is_ascii_alphanumeric()),
            _ => !value.is_empty() && !value.chars().any(char::is_control),
        }
    }
}

/// A thousand as English writes it: a catalog writes it as its language does (`1'000`), which says how its thousands
/// are set apart.
const THOUSANDS: &str = "1,000";

/// Whether `value` is digits, or groups of three digits after one to three, set apart by commas.
fn grouped(value: &str) -> bool {
    let mut parts = value.split(',');
    let first = parts.next().unwrap_or_default();
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !value.contains(',') {
        return digits(value);
    }
    digits(first) && first.len() <= 3 && parts.all(|part| digits(part) && part.len() == 3)
}

/// What sets thousands apart in `lang`, as its catalog writes [`THOUSANDS`].
fn thousands(lang: Lang) -> String {
    let written = t_in(lang, THOUSANDS);
    written.chars().nth(1).filter(|c| !c.is_ascii_digit()).map_or_else(|| ",".to_string(), |c| c.to_string())
}

/// A row and its template's parts.
struct Compiled {
    row: &'static Row,
    pattern: Pattern,
}

/// The rows by family, each family's with the most text of its own first (the table's order between equals).
struct Table {
    families: Vec<Vec<Compiled>>,
}

impl Table {
    /// The table of `rows`; a row whose template can't be read is left out (the tests refuse it).
    fn new(rows: &'static [Row]) -> Table {
        let mut families: Vec<Vec<Compiled>> = Family::ALL.iter().map(|_| Vec::new()).collect();
        for row in rows {
            match Pattern::parse(row.english) {
                Ok(pattern) => {
                    if let Some(family) = families.get_mut(row.family.index()) {
                        family.push(Compiled { row, pattern });
                    }
                }
                Err(e) => log::warn!("run_words: a row is left out: {e}"),
            }
        }
        for family in &mut families {
            family.sort_by_key(|compiled| std::cmp::Reverse(compiled.pattern.literal_len()));
        }
        Table { families }
    }

    fn family(&self, family: Family) -> &[Compiled] {
        self.families.get(family.index()).map_or(&[], Vec::as_slice)
    }
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| Table::new(rows::ROWS))
}

/// What `text` reads as in `lang`, read as a `family`; None when no row reads it (it is shown as it came), when it is
/// longer than a line can be, or when reading it would cost too much. A line that ends with sentences of
/// [`Family::Tail`] is read as what comes before them and them, when both can be read; as a whole otherwise.
pub fn explain(lang: Lang, family: Family, text: &str, mb: &Mailbox) -> Option<Reading> {
    if family == Family::Line
        && let Some(reading) = with_tails(table(), lang, text, mb, MAX_TAILS)
    {
        return Some(reading);
    }
    read_with(table(), lang, family, text, mb)
}

/// `text` read as a line that ends with up to `tails` sentences of [`Family::Tail`] after a full stop and a space,
/// each read by its row and what comes before them by a row of [`Family::Line`]; None when it doesn't end with one, or
/// what comes before can't be read.
fn with_tails(table: &Table, lang: Lang, text: &str, mb: &Mailbox, tails: usize) -> Option<Reading> {
    if tails == 0 || text.len() > MAX_TEXT {
        return None;
    }
    let at = text.rfind(". ")?;
    let (head, tail) = (text.get(..=at)?, text.get(at.checked_add(2)?..)?);
    let end = read_with(table, lang, Family::Tail, tail, mb)?;
    let head = with_tails(table, lang, head, mb, tails - 1).or_else(|| read_with(table, lang, Family::Line, head, mb))?;
    Some(Reading {
        text: format!("{} {}", head.text, end.text),
        unread: head.unread.into_iter().chain(end.unread).collect(),
        rows: head.rows.into_iter().chain(end.rows).collect(),
    })
}

fn read_with(table: &Table, lang: Lang, family: Family, text: &str, mb: &Mailbox) -> Option<Reading> {
    read_steps(table, lang, family, text, mb, MAX_STEPS)
}

/// [`read_with`], at most `steps` ways of reading tried.
fn read_steps(table: &Table, lang: Lang, family: Family, text: &str, mb: &Mailbox, steps: usize) -> Option<Reading> {
    if text.len() > MAX_TEXT {
        return None;
    }
    let reader = Reader { table, lang, mb, budget: Budget::new(steps) };
    let found = reader.read(family, text, 0);
    if reader.budget.spent() {
        return None;
    }
    found.map(|found| Reading { text: found.text, unread: found.unread, rows: found.rows })
}

/// One line of the problems of a run, in the interface's language: as it came in English, or when no row reads it.
pub(crate) fn line<'a>(text: &'a str, mb: &Mailbox) -> Cow<'a, str> {
    in_language(current(), Family::Line, text, mb)
}

/// [`line`] for what is drawn at every frame (the status bar's message): read once for each text and language, and the
/// same reading given back while they stay.
pub(crate) fn line_shown(text: &str, mb: &Mailbox) -> String {
    thread_local! {
        static LAST: std::cell::RefCell<Option<(Lang, String, String)>> = const { std::cell::RefCell::new(None) };
    }
    let lang = current();
    let kept = LAST.with(|last| last.borrow().as_ref().filter(|(was, said, _)| *was == lang && said == text).map(|(.., shown)| shown.clone()));
    if let Some(shown) = kept {
        return shown;
    }
    let shown = line(text, mb).into_owned();
    LAST.with(|last| *last.borrow_mut() = Some((lang, text.to_string(), shown.clone())));
    shown
}

/// A reason the window puts after words of its own (`A calendar change couldn't be sent yet…: {why}`), in the interface's
/// language: as it came when no row reads it.
pub(crate) fn reason<'a>(text: &'a str, mb: &Mailbox) -> Cow<'a, str> {
    in_language(current(), Family::Why, text, mb)
}

/// A command's refusal, in the interface's language.
pub(crate) fn refusal<'a>(text: &'a str, mb: &Mailbox) -> Cow<'a, str> {
    // (The engine's refusal of received mail for a Microsoft account is an interface string of its own.)
    if text == mailcraft_model::STAYS_LOCAL {
        return Cow::Borrowed(tl!(mailcraft_model::STAYS_LOCAL));
    }
    line(text, mb)
}

/// What a message held in the Outbox says of itself, in the interface's language: the mailbox keeps it in English.
pub(crate) fn held<'a>(text: &'a str, mb: &Mailbox) -> Cow<'a, str> {
    in_language(current(), Family::Banner, text, mb)
}

/// The status of Send/Receive, in the interface's language: a line (how a run ended, or its first problem), with what
/// the engine may add to it (`1 moved message can't be found…`) and the time it ended (` — 15:05`).
pub(crate) fn status<'a>(text: &'a str, mb: &Mailbox) -> Cow<'a, str> {
    let lang = current();
    if lang == Lang::English {
        return Cow::Borrowed(text);
    }
    explain_status(lang, text, mb).map_or(Cow::Borrowed(text), |reading| Cow::Owned(reading.text))
}

fn in_language<'a>(lang: Lang, family: Family, text: &'a str, mb: &Mailbox) -> Cow<'a, str> {
    if lang == Lang::English {
        return Cow::Borrowed(text);
    }
    explain(lang, family, text, mb).map_or(Cow::Borrowed(text), |reading| Cow::Owned(reading.text))
}

/// What the status of Send/Receive reads as in `lang` ([`status`]); None when it is shown as it came.
pub fn explain_status(lang: Lang, text: &str, mb: &Mailbox) -> Option<Reading> {
    let (said, at) = match text.rsplit_once(" — ") {
        Some((said, at)) if is_time(at) => (said, Some(at)),
        _ => (text, None),
    };
    // `{said}. {more}`, or `{said} {more}` when it ends with a full stop already: read as the two, when they are.
    let mut reading = match moved_said(said).and_then(|(head, more)| joined(lang, head, more, mb)) {
        Some(reading) => reading,
        None => explain(lang, Family::Line, said, mb)?,
    };
    if let Some(at) = at {
        reading.text = format!("{} — {at}", reading.text);
    }
    Some(reading)
}

/// A line and the sentence of the moved messages after it, each read.
fn joined(lang: Lang, head: &str, more: &str, mb: &Mailbox) -> Option<Reading> {
    let more = explain(lang, Family::Line, more, mb)?;
    if head.is_empty() {
        return Some(more);
    }
    let head = explain(lang, Family::Line, head, mb).or_else(|| {
        let read = explain(lang, Family::Line, head.strip_suffix('.')?, mb)?;
        Some(Reading { text: format!("{}.", read.text), ..read })
    })?;
    Some(Reading {
        text: format!("{} {}", head.text, more.text),
        unread: head.unread.into_iter().chain(more.unread).collect(),
        rows: head.rows.into_iter().chain(more.rows).collect(),
    })
}

/// Whether the end of a status is the time the engine writes after it (`15:05`, `3:05 PM`).
fn is_time(text: &str) -> bool {
    (4..=8).contains(&text.len())
        && text.starts_with(|c: char| c.is_ascii_digit())
        && text.contains(':')
        && text.chars().all(|c| c.is_ascii_digit() || matches!(c, ':' | ' ' | 'A' | 'P' | 'M'))
}

/// A status that the engine made of a line and of the messages it can't find again (`Up to date. 3 moved messages
/// can't be found…`): the line (with its full stop) and the sentence after it.
fn moved_said(said: &str) -> Option<(&str, &str)> {
    let at = said.rfind(" moved message")?;
    let digits = said.get(..at)?.bytes().rev().take_while(u8::is_ascii_digit).count();
    let start = at.checked_sub(digits).filter(|_| digits > 0)?;
    let more = said.get(start..)?;
    match start {
        0 => Some(("", more)),
        _ => Some((said.get(..start)?.strip_suffix(' ')?, more)),
    }
}

/// A reading in progress: the line's language, its mailbox (for the names of folders) and what it may still cost.
struct Reader<'a> {
    table: &'a Table,
    lang: Lang,
    mb: &'a Mailbox,
    budget: Budget,
}

/// What a text was read as, and how well: how many reasons inside it were read.
struct Found {
    text: String,
    unread: Vec<(Family, String)>,
    rows: Vec<&'static str>,
    read: usize,
}

impl Found {
    /// Whether this reading is better than `other`: fewer reasons left unread, then more read (the earlier wins a tie).
    fn better_than(&self, other: Option<&Found>) -> bool {
        other.is_none_or(|other| self.unread.len() < other.unread.len() || (self.unread.len() == other.unread.len() && self.read > other.read))
    }
}

impl Reader<'_> {
    /// The best reading of `text` by a row of `family`.
    fn read(&self, family: Family, text: &str, depth: usize) -> Option<Found> {
        if depth >= MAX_DEPTH {
            return None;
        }
        let fits = |name: &str, value: &str| Kind::of(name).is_some_and(|kind| kind.fits(value));
        let mut best: Option<Found> = None;
        for compiled in self.table.family(family) {
            if !self.budget.take() {
                return None;
            }
            for values in compiled.pattern.splits(text, &fits, &self.budget) {
                let found = self.rewrite(compiled, &values, depth);
                if self.budget.spent() {
                    return None;
                }
                if found.better_than(best.as_ref()) {
                    best = Some(found);
                }
            }
        }
        best
    }

    /// The row's sentence in the reader's language, with the values a line gave its placeholders.
    fn rewrite(&self, compiled: &Compiled, values: &[&str], depth: usize) -> Found {
        let row = compiled.row;
        let mut found = Found { text: String::new(), unread: Vec::new(), rows: vec![row.english], read: 0 };
        let mut args: Vec<(&str, String)> = Vec::new();
        for (name, value) in compiled.pattern.slots().zip(values.iter().copied()) {
            let shown = match Kind::of(name) {
                Some(Kind::Reason) => {
                    let family = row.reason(name).unwrap_or(row.family);
                    match self.read(family, value, depth + 1) {
                        Some(inner) => {
                            found.read = found.read.saturating_add(inner.read).saturating_add(1);
                            found.unread.extend(inner.unread);
                            found.rows.extend(inner.rows);
                            inner.text
                        }
                        None => {
                            found.unread.push((family, value.to_string()));
                            value.to_string()
                        }
                    }
                }
                Some(Kind::Subject) if value == NO_SUBJECT => t_in(self.lang, NO_SUBJECT).to_string(),
                Some(Kind::Subject | Kind::Addresses) => value.replace(ADDRESS, t_in(self.lang, ADDRESS)),
                Some(Kind::Folder) => standard_folder_name(self.mb, value, self.lang).unwrap_or_else(|| value.to_string()),
                Some(Kind::Text) if value == NO_REASON => t_in(self.lang, NO_REASON).to_string(),
                // (A large number with its thousands as the reader's language writes them.)
                Some(Kind::Number) => value.replace(',', &thousands(self.lang)),
                // (An event or a contact with no name.)
                Some(Kind::Text) if name == "name" && NAMES.contains(&value) => t_in(self.lang, value).to_string(),
                Some(Kind::Token | Kind::Text) | None => value.to_string(),
            };
            args.push((name, shown));
        }
        let args: Vec<(&str, &str)> = args.iter().map(|(name, shown)| (*name, shown.as_str())).collect();
        found.text = fmt(t_in(self.lang, row.english), &args);
        found
    }
}

/// The name the folder pane gives the standard folder that the engine names `name` (its name on the server, `INBOX`),
/// in `lang`; None for a folder that isn't a standard one.
pub(crate) fn standard_folder_name(mb: &Mailbox, name: &str, lang: Lang) -> Option<String> {
    let folder = mb.folders.iter().find(|f| f.kind != FolderKind::Custom && f.name == name)?;
    let english = folder.kind.default_name();
    // (A folder's name has a context of its own in a catalog: `nav::folder_name`.)
    let shown = match lang {
        Lang::Catalog(id) => catalog::translate(id, "folder", english).unwrap_or_else(|| t_in(lang, english)),
        _ => t_in(lang, english),
    };
    Some(shown.to_string())
}
