// //// Neoffice — added file (no upstream equivalent): what the person asks of Nora, and what becomes of her answers.

//! The window's commands for Nora (`ui.nora*`, listed in `UI_COMMANDS`), the calls they make, and
//! what is done with her answers when they come: a draft into the reply it was asked for, her version
//! over the person's text, a translation in place of the message's body, a word on how the learning
//! of a style began.
//!
//! One call of each kind at a time: a reply, a rewrite, a translation, a style. A call is asked of
//! the host at once, except a reply whose message has to be downloaded first: it is asked when the
//! reply opens (no model call without a reply to put the draft in). Every call ends: with its answer,
//! with the host gone ([`TryRecvError::Disconnected`]), or at [`DEADLINE_MS`].
//!
//! The writes are the composer's own commands, so that Undo takes each back in one step:
//! `compose.select`, then `edit.paste` over what is selected. A draft goes at the top of the reply,
//! above the signature and the quoted original; her version of the person's text goes over that text
//! only, and only if it is still what was sent. A compose has no identity of its own: a draft is for
//! the reply (or reply all) to its message, a rewrite for the message being written of the same kind
//! and source, and both check what they go over. While a signature is open in File › Signatures those
//! commands would write in it: what writes waits until it is closed.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use mailcraft_engine::ComposeMode;
use mailcraft_model::{AccountKind, Body, Id, Message};
use serde_json::{Value, json};

use super::answer::{self, Task};
use super::own_text::{self, OwnText};
use super::{DEADLINE_MS, GENERATE_EMAIL, IMPROVE, NoraAnswer, NoraCall, PROOFREAD, QUICK_REPLY, SCAN_STYLE, TRANSLATE, language, language_name};
use crate::i18n::fmt;
use crate::{MailApp, now_ms};

/// What Nora is asked along with a message of the sample mailbox. The model reads instructions in
/// English; the reply is written in the language of the message it answers.
const INSTRUCTION: &str = "Write the reply to the email being replied to, in the language that email is written in. \
Answer what it asks; when information is missing, say that it will be checked.";
/// The longest id of a message on the server taken (JMAP's are at most 255 characters).
const MAX_ID: usize = 512;
/// Most calls in flight: one of each kind.
const MAX_FLIGHTS: usize = 4;
/// How long a reply that waits for its message is kept once nothing is downloading it any more, in
/// milliseconds: the reply opens in the frame its download lands.
const WANTED_GRACE_MS: f64 = 2_000.0;
/// How often the window looks for answers while a call is in flight, in milliseconds.
const LOOK_AGAIN_MS: u64 = 100;

/// How Nora rewrites the person's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rewrite {
    Correct,
    Improve,
    /// Into this language.
    Translate(&'static str),
}

impl Rewrite {
    fn method(self) -> &'static str {
        match self {
            Rewrite::Correct => PROOFREAD,
            Rewrite::Improve => IMPROVE,
            Rewrite::Translate(_) => TRANSLATE,
        }
    }

    /// The field of the answer her version is in.
    fn field(self) -> &'static str {
        match self {
            Rewrite::Correct => "corrected_text",
            Rewrite::Improve => "improved_text",
            Rewrite::Translate(_) => "translated_text",
        }
    }
}

/// The kinds of call, one of each at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Reply,
    Rewrite,
    Translation,
    Style,
}

/// What a call is for.
#[derive(Clone, Debug, PartialEq)]
enum What {
    /// The draft of the reply to `message`.
    Reply { message: Id },
    /// A new version of the person's text in the message being written: the text that was sent (her
    /// version goes in only over that same text), and what was being written (its kind and source).
    Rewrite { how: Rewrite, sent: OwnText, mode: ComposeMode, source: Option<Id> },
    /// `message` in `language`.
    Translation { message: Id, language: &'static str },
    /// The start of the learning of a mailbox's style.
    Style,
}

impl What {
    fn kind(&self) -> Kind {
        match self {
            What::Reply { .. } => Kind::Reply,
            What::Rewrite { .. } => Kind::Rewrite,
            What::Translation { .. } => Kind::Translation,
            What::Style => Kind::Style,
        }
    }

    fn task(&self) -> Task {
        match self {
            What::Reply { .. } => Task::Reply,
            What::Rewrite { .. } => Task::Rewrite,
            What::Translation { .. } => Task::Translate,
            What::Style => Task::Style,
        }
    }

    /// Its answer writes into the message being written.
    fn writes(&self) -> bool {
        matches!(self, What::Reply { .. } | What::Rewrite { .. })
    }

    /// Where what is said of it belongs.
    fn about(&self) -> About {
        match self {
            What::Translation { message, .. } => About::Message(*message),
            _ => About::Writing,
        }
    }
}

/// A call the host is answering.
struct Flight {
    what: What,
    /// When it was asked (milliseconds).
    since: f64,
    answer: Receiver<NoraAnswer>,
}

/// A reply whose message is being downloaded: Nora is asked for its draft when it opens.
struct Wanted {
    message: Id,
    method: &'static str,
    args: Value,
    since: f64,
}

/// A message as Nora translated it, shown in place of its body (the header, the files and the rest
/// are the message's own).
// The reading pane shows it (the next step).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Translation {
    pub(crate) message: Id,
    pub(crate) language: &'static str,
    /// The message with the translation for its body, as plain text: made once, when it came.
    pub(crate) shown: Arc<Message>,
}

/// What a line about Nora is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum About {
    /// The message being written (the band under its subject shows it).
    Writing,
    /// A message being read (the strip above its body shows it).
    Message(Id),
}

/// The last thing said of what Nora did.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Said {
    pub(crate) text: String,
    pub(crate) error: bool,
    /// When (milliseconds).
    pub(crate) at: f64,
    pub(crate) about: About,
}

/// What the window knows of Nora while it runs. Nothing of it is kept between runs.
#[derive(Default)]
pub(crate) struct Nora {
    last: u64,
    flights: Vec<Flight>,
    wanted: Option<Wanted>,
    pub(crate) translation: Option<Translation>,
    pub(crate) said: Option<Said>,
}

impl Nora {
    /// Whether a call is in flight or a reply waits to ask for one.
    pub(crate) fn busy(&self) -> bool {
        !self.flights.is_empty() || self.wanted.is_some()
    }

    /// Whether a call of this kind is in flight (a reply that waits for its message counts).
    pub(crate) fn asking(&self, kind: Kind) -> bool {
        self.flights.iter().any(|f| f.what.kind() == kind) || (kind == Kind::Reply && self.wanted.is_some())
    }

    /// The rewrite in flight, when there is one.
    pub(crate) fn rewriting(&self) -> Option<Rewrite> {
        self.flights.iter().find_map(|f| match f.what {
            What::Rewrite { how, .. } => Some(how),
            _ => None,
        })
    }

    /// The message being translated, when one is.
    // The reading pane shows it (the next step).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn translating(&self) -> Option<Id> {
        self.flights.iter().find_map(|f| match f.what {
            What::Translation { message, .. } => Some(message),
            _ => None,
        })
    }

    /// Pretend the calls in flight, and a reply that waits, were asked `ms` milliseconds earlier.
    #[cfg(test)]
    pub(crate) fn age(&mut self, ms: f64) {
        for flight in &mut self.flights {
            flight.since -= ms;
        }
        if let Some(wanted) = &mut self.wanted {
            wanted.since -= ms;
        }
    }
}

/// Where a message goes to Nora.
enum Route {
    /// A mailbox the hosting page handed over: the message goes by its id on the server, which reads
    /// it itself; the mailbox by its address, which is the name of the server's account for it.
    Server { mailbox: String, id: String },
    /// The sample mailbox (any account kept on this computer only): the server can't read it, so its
    /// text goes along.
    Text,
}

/// The address of the page's mailbox `account` is, as the page named it; `None` for any other account.
pub(crate) fn page_mailbox(app: &MailApp, account: Id) -> Option<String> {
    let host = app.services.nora.as_ref()?;
    let a = app.session.mailbox.account(account)?;
    let email = a.email.trim();
    let handed = host.mailboxes.iter().any(|m| m.trim().eq_ignore_ascii_case(email));
    (a.kind == AccountKind::Jmap && !email.is_empty() && handed).then(|| email.to_string())
}

/// An id a server gave a message, as JMAP writes them, within what is taken.
fn plausible_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn route(app: &MailApp, m: &Message) -> Result<Route, String> {
    if let Some(mailbox) = page_mailbox(app, m.account) {
        // Without its id (moved and not found again yet, say), its text is not sent instead.
        return match m.server.as_ref().map(|s| s.id.as_str()).filter(|id| plausible_id(id)) {
            Some(id) => Ok(Route::Server { mailbox, id: id.to_string() }),
            None => Err(tl!("Nora couldn't find this message on the server: refresh the list and try again.").to_string()),
        };
    }
    match app.session.mailbox.account(m.account) {
        Some(a) if a.kind == AccountKind::Local => Ok(Route::Text),
        // Another server's mail (an account the person added in the page) is not sent to this one.
        _ => Err(tl!("Nora works on the mailboxes of your Neoffice account only.").to_string()),
    }
}

/// A message's text for the server, its body read into memory when the mailbox keeps it on disk.
fn text_of(app: &MailApp, m: &Message) -> String {
    let loaded = app.session.loaded_message_where(m.id, |_| false).ok();
    let body = loaded.as_ref().map_or(&m.body, |m| &m.body);
    match (&body.html, body.text.trim().is_empty()) {
        (Some(html), true) => mailcraft_html::to_text(html),
        _ => body.text.clone(),
    }
}

/// Whether a reply (or a reply to everyone) to `message` is being written.
fn replying_to(app: &MailApp, message: Id) -> bool {
    app.session.compose.as_ref().is_some_and(|c| matches!(c.mode, ComposeMode::Reply | ComposeMode::ReplyAll) && c.source == Some(message))
}

/// The message a command is about: the one it names, else the one being read.
fn subject(app: &MailApp, p: &Value) -> Result<Arc<Message>, String> {
    let id = match p.get("message") {
        None | Some(Value::Null) => app.session.sel.focus().ok_or_else(|| tl!("Select a message first").to_string())?,
        Some(id) => id.as_u64().ok_or_else(|| "`message` is a message's id".to_string())?,
    };
    app.session.mailbox.messages.get(&id).cloned().ok_or_else(|| format!("no message with id {id}"))
}

/// The language a command names.
fn chosen_language(p: &Value) -> Result<&'static str, String> {
    p.get("language").and_then(Value::as_str).and_then(language).ok_or_else(|| "`language` is \"fr\", \"de\" or \"en\"".to_string())
}

/// A call that can't be asked: as many are in flight as are taken (one of each kind makes that impossible).
fn full() -> String {
    "Nora has as many calls under way as she takes".to_string()
}

fn on(app: &MailApp) -> Result<(), String> {
    if app.services.nora.is_some() { Ok(()) } else { Err("Nora isn't here: nothing hosting this window reaches her".to_string()) }
}

/// Ask the host; `None` when there is no host, or too many calls are in flight.
fn send(app: &mut MailApp, method: &'static str, args: Value, what: What) -> Option<u64> {
    if app.nora.flights.len() >= MAX_FLIGHTS {
        return None;
    }
    let host = app.services.nora.as_ref()?;
    app.nora.last = app.nora.last.wrapping_add(1);
    let id = app.nora.last;
    let answer = (host.call)(NoraCall { id, method, args });
    app.nora.flights.push(Flight { what, since: now_ms(), answer });
    Some(id)
}

/// Say what came of something Nora did: where it is about (the band, the strip) and in the status bar.
fn say(app: &mut MailApp, about: About, text: String, error: bool) {
    app.status(text.clone());
    app.nora.said = Some(Said { text, error, at: now_ms(), about });
}

/// A refusal the person is to read in the band too (the status bar has it from `run`).
fn refused(app: &mut MailApp, text: String) -> Result<Value, String> {
    app.nora.said = Some(Said { text: text.clone(), error: true, at: now_ms(), about: About::Writing });
    Err(text)
}

/// The window commands for Nora (`UI_COMMANDS` lists them).
pub(crate) fn ui_command(app: &mut MailApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "ui.noraReply" => reply(app, p),
        "ui.noraTranslate" => translate(app, p),
        "ui.noraOriginal" => original(app, p),
        "ui.noraCorrect" => rewrite(app, Rewrite::Correct),
        "ui.noraImprove" => rewrite(app, Rewrite::Improve),
        "ui.noraTranslateDraft" => chosen_language(p).and_then(|l| rewrite(app, Rewrite::Translate(l))),
        "ui.noraLearnStyle" => learn_style(app),
        _ => return None,
    })
}

/// Whether a window command for Nora can run now (`MailApp::enabled`).
pub(crate) fn enabled(app: &MailApp, id: &str) -> bool {
    if app.services.nora.is_none() {
        return false;
    }
    let nora = &app.nora;
    let writing = app.session.compose.as_ref();
    let reading = app.session.sel.focus().is_some();
    match id {
        // Whatever Reply refuses, Nora's reply refuses.
        "ui.noraReply" => reading && app.enabled("mail.reply") && !nora.asking(Kind::Reply),
        // A translation shows in the reading pane, which a message being written takes the place of.
        "ui.noraTranslate" => reading && writing.is_none() && !nora.asking(Kind::Translation),
        "ui.noraOriginal" => nora.translation.is_some(),
        // While a reply's draft is coming, the text it goes into is not to be rewritten.
        "ui.noraCorrect" | "ui.noraImprove" | "ui.noraTranslateDraft" => {
            writing.is_some() && app.session.signature_edit.is_none() && !nora.asking(Kind::Rewrite) && !nora.asking(Kind::Reply)
        }
        "ui.noraLearnStyle" => writing.is_some_and(|c| page_mailbox(app, c.account).is_some()) && !nora.asking(Kind::Style),
        _ => false,
    }
}

/// Reply with Nora: open the reply, and ask her for its draft. Every refusal comes before the reply
/// opens, so that a refusal leaves nothing behind.
fn reply(app: &mut MailApp, p: &Value) -> Result<Value, String> {
    on(app)?;
    let all = match p.get("all") {
        None | Some(Value::Null) => false,
        Some(all) => all.as_bool().ok_or_else(|| "`all` is true or false".to_string())?,
    };
    let m = subject(app, p)?;
    if app.nora.asking(Kind::Reply) {
        return Err(tl!("Nora is already writing a reply.").to_string());
    }
    let (method, args) = match route(app, &m)? {
        Route::Server { mailbox, id } => (QUICK_REPLY, json!({"account_name": mailbox, "email_id": id})),
        Route::Text => {
            let context = own_text::reply_context(&m.from, &m.subject, &text_of(app, &m));
            (GENERATE_EMAIL, json!({"prompt": INSTRUCTION, "account_name": "", "reply_context": context}))
        }
    };
    // As Reply does it: a message not downloaded whole yet is downloaded first (`MailApp::fetch_first`).
    let opened = app.run(if all { "mail.replyAll" } else { "mail.reply" }, json!({"id": m.id}))?;
    if replying_to(app, m.id) {
        let Some(asked) = send(app, method, args, What::Reply { message: m.id }) else { return Err(full()) };
        app.status(tl!("Nora is writing the reply…"));
        return Ok(json!({"composing": true, "asked": asked, "method": method}));
    }
    match opened.get("pending").and_then(Value::as_str) {
        // The reply opens once the message is here: Nora is asked then ([`poll`]).
        Some(pending) => {
            app.nora.wanted = Some(Wanted { message: m.id, method, args, since: now_ms() });
            Ok(json!({"pending": pending, "message": m.id}))
        }
        None => Ok(json!({"composing": false})),
    }
}

/// Translate the message being read.
fn translate(app: &mut MailApp, p: &Value) -> Result<Value, String> {
    on(app)?;
    let language = chosen_language(p)?;
    let m = subject(app, p)?;
    if app.session.compose.is_some() {
        return Err("a message is being written: a translation shows in the reading pane, which it takes the place of".to_string());
    }
    if app.nora.asking(Kind::Translation) {
        return Err(tl!("Nora is already translating a message.").to_string());
    }
    let args = match route(app, &m)? {
        Route::Server { mailbox, id } => json!({"target_language": language, "account_name": mailbox, "email_id": id}),
        Route::Text => json!({"html_content": own_text::message_html(&text_of(app, &m)), "target_language": language, "account_name": ""}),
    };
    let Some(asked) = send(app, TRANSLATE, args, What::Translation { message: m.id, language }) else { return Err(full()) };
    if app.nora.said.as_ref().is_some_and(|s| s.about == About::Message(m.id)) {
        app.nora.said = None;
    }
    app.status(tl!("Nora is translating the message…"));
    Ok(json!({"asked": asked, "message": m.id, "language": language}))
}

/// Show the original: the message as it came, in place of Nora's translation.
fn original(app: &mut MailApp, p: &Value) -> Result<Value, String> {
    let id = match p.get("message") {
        None | Some(Value::Null) => app.nora.translation.as_ref().map(|t| t.message).or(app.session.sel.focus()).unwrap_or(0),
        Some(id) => id.as_u64().ok_or_else(|| "`message` is a message's id".to_string())?,
    };
    let was = app.nora.translation.as_ref().is_some_and(|t| t.message == id);
    if was {
        app.nora.translation = None;
    }
    if app.nora.said.as_ref().is_some_and(|s| s.about == About::Message(id)) {
        app.nora.said = None;
    }
    Ok(json!({"message": id, "translated": false, "was": was}))
}

/// Correct, Improve or Translate the person's own text in the message being written.
fn rewrite(app: &mut MailApp, how: Rewrite) -> Result<Value, String> {
    on(app)?;
    let Some(c) = app.session.compose.as_ref() else { return Err("no message is being written".to_string()) };
    if app.session.signature_edit.is_some() {
        return Err("a signature is open in File › Signatures: what Nora writes would go into it; close it first".to_string());
    }
    if app.nora.asking(Kind::Reply) {
        let busy = tl!("Nora is already writing a reply.").to_string();
        return refused(app, busy);
    }
    if app.nora.asking(Kind::Rewrite) {
        let busy = tl!("Nora is already rewriting the text.").to_string();
        return refused(app, busy);
    }
    let own = match own_text::to_rewrite(c.body.text()) {
        Ok(own) => own,
        Err(why) => return refused(app, why),
    };
    let mailbox = page_mailbox(app, c.account).unwrap_or_default();
    let html = own_text::as_html(&own.text);
    let args = match how {
        Rewrite::Translate(l) => json!({"html_content": html, "target_language": l, "account_name": mailbox}),
        Rewrite::Correct | Rewrite::Improve => json!({"html_content": html, "account_name": mailbox}),
    };
    let what = What::Rewrite { how, sent: own, mode: c.mode, source: c.source };
    let Some(asked) = send(app, how.method(), args, what) else { return Err(full()) };
    app.nora.said = None;
    app.status(tl!("Nora is rewriting the text…"));
    Ok(json!({"asked": asked, "method": how.method()}))
}

/// My style: have Nora learn the writing style of the mailbox the message goes from, from its sent
/// mail, on the server (only a mailbox the hosting page handed over has one there).
fn learn_style(app: &mut MailApp) -> Result<Value, String> {
    on(app)?;
    let Some(c) = app.session.compose.as_ref() else { return Err("no message is being written".to_string()) };
    let Some(mailbox) = page_mailbox(app, c.account) else {
        let why = tl!("Nora works on the mailboxes of your Neoffice account only.").to_string();
        return refused(app, why);
    };
    if app.nora.asking(Kind::Style) {
        let busy = tl!("Nora is already learning your writing style.").to_string();
        return refused(app, busy);
    }
    let Some(asked) = send(app, SCAN_STYLE, json!({"account_name": mailbox}), What::Style) else { return Err(full()) };
    app.nora.said = None;
    app.status(tl!("Starting style analysis…"));
    Ok(json!({"asked": asked}))
}

/// How a call ended.
enum Landed {
    Answer(NoraAnswer),
    /// Nothing came within [`DEADLINE_MS`].
    TooLate,
    /// The host let the call go without an answer.
    Dropped,
}

/// Take in Nora's answers, ask for the draft of a reply that has opened, and give up on what took too
/// long. Once a frame.
pub(crate) fn poll(app: &mut MailApp) {
    if !app.nora.busy() {
        return;
    }
    let now = now_ms();
    wanted(app, now);
    let flights = std::mem::take(&mut app.nora.flights);
    let mut kept = Vec::with_capacity(flights.len());
    for flight in flights {
        // While a signature is open in File › Signatures, the composer's commands would write in it.
        if flight.what.writes() && app.session.signature_edit.is_some() {
            kept.push(flight);
            continue;
        }
        let landed = match flight.answer.try_recv() {
            Ok(answer) => Landed::Answer(answer),
            Err(TryRecvError::Empty) if now - flight.since > DEADLINE_MS => Landed::TooLate,
            Err(TryRecvError::Empty) => {
                kept.push(flight);
                continue;
            }
            Err(TryRecvError::Disconnected) => Landed::Dropped,
        };
        land(app, flight.what, landed);
    }
    kept.append(&mut app.nora.flights);
    app.nora.flights = kept;
    if app.nora.busy()
        && let Some(ctx) = &app.ctx
    {
        ctx.request_repaint_after(std::time::Duration::from_millis(LOOK_AGAIN_MS));
    }
}

/// A reply whose message was being downloaded: Nora is asked once it has opened; it is let go when the
/// download ended without one (the download said why) or it all took too long.
fn wanted(app: &mut MailApp, now: f64) {
    let Some(w) = &app.nora.wanted else { return };
    if replying_to(app, w.message) {
        if let Some(w) = app.nora.wanted.take()
            && send(app, w.method, w.args, What::Reply { message: w.message }).is_some()
        {
            app.status(tl!("Nora is writing the reply…"));
        }
        return;
    }
    let downloading = app.fetching.fetching(w.message);
    if now - w.since > DEADLINE_MS || (!downloading && now - w.since > WANTED_GRACE_MS) {
        app.nora.wanted = None;
    }
}

fn land(app: &mut MailApp, what: What, landed: Landed) {
    let about = what.about();
    let answer = match landed {
        Landed::Answer(answer) => answer,
        Landed::TooLate => return say(app, about, what.task().failed(tl!("it took too long")), true),
        Landed::Dropped => return say(app, about, answer::not_reached(tl!("the connection stopped before Nora answered")), true),
    };
    match what {
        What::Reply { message } => match answer::draft(&answer) {
            Ok(text) => put_draft(app, message, &text),
            Err(why) => say(app, about, why, true),
        },
        What::Rewrite { how, sent, mode, source } => match answer::rewritten(&answer, how.field()) {
            Ok(Some(text)) => put_over_own_text(app, &sent, (mode, source), &text),
            Ok(None) => {
                let said = match how {
                    Rewrite::Translate(l) => fmt(tl!("The message is already in {language}."), &[("language", language_name(l))]),
                    Rewrite::Correct | Rewrite::Improve => tl!("Nora found nothing to change.").to_string(),
                };
                say(app, about, said, false);
            }
            Err(why) => say(app, about, why, true),
        },
        What::Translation { message, language } => match answer::translation(&answer, language) {
            Ok(Some(text)) => show_translation(app, message, language, text),
            Ok(None) => say(app, about, fmt(tl!("The message is already in {language}."), &[("language", language_name(language))]), false),
            Err(why) => say(app, about, why, true),
        },
        What::Style => match answer::style_started(&answer) {
            Ok(true) => say(app, about, tl!("Nora is already learning your writing style.").to_string(), false),
            Ok(false) => say(app, about, tl!("Nora is learning your writing style from your sent emails. This takes a minute.").to_string(), false),
            Err(why) => say(app, about, why, true),
        },
    }
}

/// Write `text` over the first `chars` characters of the message being written (none: at its top), as
/// one step Undo takes back.
fn write(app: &mut MailApp, chars: usize, text: &str) -> Result<(), String> {
    app.execute("compose.select", json!({"from": 0, "to": chars}))?;
    app.execute("edit.paste", json!({"text": text}))?;
    app.body_focus_wanted = true;
    Ok(())
}

/// Nora's draft at the top of the reply it was asked for, while that reply is being written: above
/// the signature and the original, and above anything the person has typed meanwhile, a blank line
/// between.
fn put_draft(app: &mut MailApp, message: Id, text: &str) {
    if !replying_to(app, message) {
        return say(app, About::Writing, tl!("The reply was closed before Nora's draft came back.").to_string(), false);
    }
    let first = app.session.compose.as_ref().and_then(|c| c.body.text().paragraphs().first().map(|p| (p.signature, p.is_empty())));
    let text = match first {
        // Pasted into a paragraph of the signature, the draft's first line would be the signature's and
        // the signature's first line the draft's.
        Some((true, _)) => {
            let why = tl!("Your reply starts with its signature: leave a line above it for Nora's draft, and ask her again.");
            return say(app, About::Writing, why.to_string(), true);
        }
        Some((false, false)) => format!("{text}\n\n"),
        _ => text.to_string(),
    };
    match write(app, 0, &text) {
        Ok(()) => say(app, About::Writing, tl!("Nora's draft is in the reply: read it before sending.").to_string(), false),
        Err(why) => say(app, About::Writing, Task::Reply.failed(&answer::clean(&why)), true),
    }
}

/// Nora's version over the person's own text, if that text is still what was sent (anything typed
/// meanwhile wins), in the message it was sent from.
fn put_over_own_text(app: &mut MailApp, sent: &OwnText, (mode, source): (ComposeMode, Option<Id>), text: &str) {
    let Some(c) = app.session.compose.as_ref().filter(|c| c.mode == mode && c.source == source) else {
        return say(app, About::Writing, tl!("The message was closed before Nora's version came back.").to_string(), false);
    };
    if own_text::own_text(c.body.text()) != *sent {
        return say(app, About::Writing, tl!("Your text changed meanwhile: Nora's version wasn't put in.").to_string(), false);
    }
    match write(app, sent.chars, text) {
        Ok(()) => say(app, About::Writing, tl!("Nora's version is in the message: Undo takes it back.").to_string(), false),
        Err(why) => say(app, About::Writing, Task::Rewrite.failed(&answer::clean(&why)), true),
    }
}

/// Keep a translation to show in place of its message's body, made into a message once.
fn show_translation(app: &mut MailApp, message: Id, language: &'static str, text: String) {
    let Some(original) = app.session.mailbox.messages.get(&message) else { return };
    let mut shown = Message::clone(original);
    shown.body = Body { text, html: None };
    // Its body is the translation, whole: nothing of it waits on disk.
    shown.kept = None;
    app.nora.translation = Some(Translation { message, language, shown: Arc::new(shown) });
    if app.nora.said.as_ref().is_some_and(|s| s.about == About::Message(message)) {
        app.nora.said = None;
    }
    app.status(fmt(tl!("Translated by Nora into {language}"), &[("language", language_name(language))]));
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
