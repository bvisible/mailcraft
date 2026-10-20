// //// Neoffice — added file (no upstream equivalent): what Nora's answers say, read as warily as anything from outside.

//! What an answer of the server says: the text Nora wrote, or the sentence that tells the person why
//! there is none, in the interface's language.
//!
//! An answer is a stranger's bytes: it is read only when it is small enough ([`MAX_ANSWER`]), through
//! `serde_json` (which refuses nesting past its limit rather than overflow), field by field, each
//! field checked for its type; a text Nora wrote is taken up to [`MAX_REPLY`] characters, and what
//! the server says of a refusal goes into one line of the window: no markup, no control characters,
//! a few hundred characters at most.
//!
//! The server is Frappe: a good answer is `{"message": {…}}` with status 200; a refusal
//! (`frappe.throw`) is status 417 with `_server_messages`, a list written as JSON text of messages
//! that are each JSON text of an object with a `message`; a crash is status 500 with `exception`.

use serde_json::Value;

use super::{MAX_ANSWER, MAX_REPLY, NoraAnswer, TRUNCATED};
use crate::i18n::fmt;

/// The longest reason from the server shown, in characters.
const MAX_REASON: usize = 300;
/// Most messages of `_server_messages` looked through.
const MAX_SERVER_MESSAGES: usize = 20;

/// What was asked of Nora, for the sentence that says it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Task {
    Reply,
    Translate,
    Rewrite,
    Style,
}

impl Task {
    /// "Nora couldn't …: `reason`", in the interface's language.
    pub(crate) fn failed(self, reason: &str) -> String {
        let template = match self {
            Task::Reply => tl!("Nora couldn't write the reply: {reason}"),
            Task::Translate => tl!("Nora couldn't translate the message: {reason}"),
            Task::Rewrite => tl!("Nora couldn't rewrite the text: {reason}"),
            Task::Style => tl!("Nora couldn't learn your style: {reason}"),
        };
        fmt(template, &[("reason", reason)])
    }
}

/// No answer came from the network: why, in a few words.
pub(crate) fn not_reached(reason: &str) -> String {
    fmt(tl!("Nora couldn't be reached: {reason}"), &[("reason", &clean(reason))])
}

/// The body of a good answer (status 200, a `message` object), or the sentence that says what went
/// wrong.
fn parse(answer: &NoraAnswer, task: Task) -> Result<Value, String> {
    if answer.status == 0 {
        return Err(not_reached(&answer.body));
    }
    if answer.body.len() > MAX_ANSWER {
        return Err(task.failed(tl!("the answer is too long")));
    }
    let body: Value = serde_json::from_str(&answer.body).unwrap_or(Value::Null);
    if answer.status == 200 && body.get("message").is_some_and(Value::is_object) { Ok(body) } else { Err(task.failed(&reason(&body, answer.status))) }
}

/// A text field of the answer's `message`, trimmed: `None` when it is missing, not text, or empty.
fn text<'a>(body: &'a Value, field: &str) -> Option<&'a str> {
    body.get("message")?.get(field)?.as_str().map(str::trim).filter(|t| !t.is_empty())
}

/// A flag of the answer's `message`, when it is one.
fn flag(body: &Value, field: &str) -> Option<bool> {
    body.get("message")?.get(field)?.as_bool()
}

/// A text Nora wrote, within the longest that is taken: line breaks and tabs, and no other control or
/// direction-changing character (it goes into the person's mail, whose readers' programs would follow them).
fn bounded(text: &str, task: Task) -> Result<String, String> {
    if text.chars().nth(MAX_REPLY).is_some() {
        return Err(task.failed(tl!("the answer is too long")));
    }
    let kept: String = text.replace("\r\n", "\n").chars().filter(|c| !unseen(*c) && (!c.is_control() || matches!(c, '\n' | '\t'))).collect();
    let kept = kept.trim();
    if kept.is_empty() { Err(task.failed(tl!("the answer had no text"))) } else { Ok(kept.to_string()) }
}

/// The draft of a reply an answer carries (`quick_reply`, `generate_email`), or what to tell the
/// person.
pub(crate) fn draft(answer: &NoraAnswer) -> Result<String, String> {
    let task = Task::Reply;
    let body = parse(answer, task)?;
    match text(&body, "draft_text") {
        Some(draft) => bounded(draft, task),
        None => Err(task.failed(&reason(&body, answer.status))),
    }
}

/// The translation an answer carries (`translate`): `None` when the text is already in `language`,
/// as the server says when it changed nothing; or what to tell the person.
pub(crate) fn translation(answer: &NoraAnswer, language: &str) -> Result<Option<String>, String> {
    let task = Task::Translate;
    let body = parse(answer, task)?;
    let source = body.get("message").and_then(|m| m.get("source_language")).and_then(Value::as_str);
    if flag(&body, "has_changes") == Some(false) && source == Some(language) {
        return Ok(None);
    }
    match text(&body, "translated_text") {
        Some(translated) => bounded(translated, task).map(Some),
        None => Err(task.failed(&reason(&body, answer.status))),
    }
}

/// The new version of the person's text an answer carries in `field` (`corrected_text`,
/// `improved_text`, `translated_text`): `None` when Nora changed nothing; or what to tell the person.
/// A version the server cut (its mark is in it) is refused: put over the person's text, it would
/// take the rest of that text away.
pub(crate) fn rewritten(answer: &NoraAnswer, field: &str) -> Result<Option<String>, String> {
    let task = Task::Rewrite;
    let body = parse(answer, task)?;
    if flag(&body, "has_changes") == Some(false) {
        return Ok(None);
    }
    match text(&body, field) {
        Some(new) if new.contains(TRUNCATED) => Err(task.failed(tl!("the server cut the text short"))),
        Some(new) => bounded(new, task).map(Some),
        None => Err(task.failed(&reason(&body, answer.status))),
    }
}

/// Whether the analysis of a mailbox's style that an answer speaks of (`scan_style`) was already
/// running (`true`) or has just started (`false`); or what to tell the person.
pub(crate) fn style_started(answer: &NoraAnswer) -> Result<bool, String> {
    let task = Task::Style;
    let body = parse(answer, task)?;
    match flag(&body, "success") {
        Some(true) => Ok(flag(&body, "already_running").unwrap_or(false)),
        _ => Err(task.failed(&reason(&body, answer.status))),
    }
}

/// Why the server said no: the first message it shows (Frappe's `_server_messages`), else the last
/// part of its exception, else the HTTP status.
pub(crate) fn reason(body: &Value, status: u16) -> String {
    let exception = || {
        let said = body.get("exception")?.as_str()?;
        let last = said.rsplit(": ").next().unwrap_or(said);
        Some(clean(last)).filter(|e| !e.is_empty())
    };
    // A good status with nothing in it (no text, a page that isn't the server's answer) is no success.
    let fallback = || if status == 200 { tl!("the answer had no text").to_string() } else { format!("HTTP {status}") };
    server_message(body).or_else(exception).unwrap_or_else(fallback)
}

/// The first of Frappe's `_server_messages` that has text, without its markup.
fn server_message(body: &Value) -> Option<String> {
    let parsed: Vec<Value>;
    let list: &[Value] = match body.get("_server_messages")? {
        Value::String(written) => {
            parsed = serde_json::from_str(written).ok()?;
            &parsed
        }
        Value::Array(items) => items,
        _ => return None,
    };
    list.iter().take(MAX_SERVER_MESSAGES).find_map(|item| {
        let decoded: Value;
        let item = match item {
            Value::String(written) => {
                decoded = serde_json::from_str(written).ok()?;
                &decoded
            }
            other => other,
        };
        Some(clean(&plain(item.get("message")?.as_str()?))).filter(|said| !said.is_empty())
    })
}

/// A server's message without its HTML: its tags left out, its character references read.
fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in html.chars().take(MAX_REASON * 16) {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            c if !tag => out.push(c),
            _ => {}
        }
    }
    mailcraft_html::unescape(&out)
}

/// Text from outside, fit for one line of the window: no control or direction-changing characters
/// (a line break becomes a space), runs of spaces as one, at most [`MAX_REASON`] characters.
pub(crate) fn clean(text: &str) -> String {
    let flat: String = text.chars().take(MAX_REASON * 4).filter(|c| !unseen(*c)).map(|c| if c.is_control() { ' ' } else { c }).collect();
    let words = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.chars().nth(MAX_REASON).is_some() { words.chars().take(MAX_REASON - 1).chain(['…']).collect() } else { words }
}

/// A character that changes the direction text is shown in, or marks it: left out of what Nora says
/// and writes.
fn unseen(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}
