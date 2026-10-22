// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): Nora's pane, a conversation with her beside the window.

//! Nora's pane: a conversation with Neoffice's assistant at the right of the window, the one the editors
//! of Neoffice show beside a document (the same client core, `craft-nora`, and the same panel,
//! `craft-nora-ui`). The person asks her about the message being read (« résume ce fil », « qu'est-ce
//! qu'on me demande ? », « qui est ce client ? ») or to write (« réponds-lui que… », « traduis ma réponse
//! en allemand »).
//!
//! Nora (the agent, her memory and her tools) lives on the server. The pane keeps her conversation
//! ([`NoraSession`]) and hands its calls to the host that reaches her ([`super::Host`], the page
//! `/neomail` with its own session), one of [`CHAT_METHODS`] each, and only when the host takes them
//! ([`super::Host::chat`]). Her conversation stays open from one message to the next until New
//! conversation: she remembers what was said in it.
//!
//! What she asks of the window is played here, and nothing else:
//! - she reads it ([`super::observe`]): the message being read and its conversation, the message being
//!   written, its selection; all of it written by other people but the person's own text, so data, which
//!   the server reads through its guard against instructions aimed at her before any model does;
//! - she writes into the message being written, and only there: at the caret (`doc_type_text`), at its
//!   top, after the person's text or over it (`draft.write`), into the reply she opens to the message
//!   being read (`draft.reply`) or into a new message she opens (`draft.new`); she selects in it
//!   (`doc_select_text`). Each write is one step that the composer's Undo takes back, and the pane's
//!   Undo takes back only hers, while the message is still as she left it;
//! - nothing else: she sends nothing (Send stays the person's click), deletes, moves, files or flags
//!   nothing, saves nothing, touches no account, setting or file, opens no dialog. Any other command is
//!   refused, by the conversation's policy for the window's own families and by the pane for the rest.
//!
//! While she works, the panel's Ask button is a Stop button; a failure of the connection to her is said
//! in the interface's language.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use craft_nora::{
    AppKind, CommandPolicy, DocActionKind, DocumentInfo, HttpRequest, HttpResponse, Input, NoraError, NoraSession, Output, PageContext, Selection,
    SessionConfig,
};
use craft_nora_ui::{NoraLabels, NoraPanel, NoraStyle, PanelAction};
use egui::{Margin, RichText, Sense, Stroke, Ui, vec2};
use mailcraft_engine::ComposeMode;
use mailcraft_model::Id;
use mailcraft_model::RichText;
use serde_json::{Value, json};

use super::{CHAT_METHODS, DEADLINE_MS, MAX_REPLY, NoraAnswer, NoraCall, look, observe, own_text};
use crate::i18n::fmt;
use crate::theme::{Tokens, semibold};
use crate::{MailApp, now_ms};

/// The commands of hers the pane plays (`doc_execute`): they write into a message being written, and
/// only there. Everything else is refused.
pub const COMMANDS: [&str; 3] = ["draft.reply", "draft.new", "draft.write"];

/// The window's own command families, refused by the conversation before an action reaches the pane
/// (besides the default policy's `file.` and `app.`): Nora plays the pane's [`COMMANDS`], never the
/// window's commands, which send, delete, move, file, flag or change settings.
const REFUSED_FAMILIES: &[&str] = &[
    "ui.",
    "mail.",
    "compose.",
    "edit.",
    "folder.",
    "account.",
    "calendar.",
    "people.",
    "tasks.",
    "notes.",
    "sync.",
    "signature.",
    "rules.",
    "format.",
    "view.",
    "options.",
    "categories.",
    "search.",
    "engine.",
];

/// The longest recipient line and subject taken from Nora for a new message.
const MAX_FIELD: usize = 1_000;
/// The longest text Nora may ask to select (the search costs the message's length times it).
const MAX_SELECT: usize = 2_000;
/// The longest server's sentence shown in the pane.
const MAX_SAID: usize = 300;
/// The most writes of hers the pane's Undo can take back, the oldest forgotten first (the composer keeps 100 steps).
const MAX_CHANGES: usize = 50;

/// What the pane keeps while the window runs: whether it is open, the conversation, the panel that draws
/// it, the calls in flight, and what Nora changed. Nothing of it is kept between runs.
pub(crate) struct Assist {
    pub(crate) open: bool,
    /// Whether the pane's text box had the keyboard when it was last drawn: the window's shortcuts are then
    /// left alone ([`has_keys`]).
    typing: bool,
    session: NoraSession,
    panel: NoraPanel,
    calls: Vec<Call>,
    /// Nora's writes that the pane's Undo can take back, the newest last.
    changes: Vec<Change>,
}

/// A call the host is answering.
struct Call {
    id: u64,
    since: f64,
    answer: Receiver<NoraAnswer>,
}

/// One write of Nora's: the message it went into (by its kind and source, the way the window tells one
/// message being written from another), and its text, formatting included, before and once written.
#[derive(Clone, Debug, PartialEq)]
struct Change {
    mode: ComposeMode,
    source: Option<Id>,
    before: RichText,
    after: RichText,
}

impl Default for Assist {
    fn default() -> Self {
        let config = SessionConfig { policy: policy(), ..SessionConfig::default() };
        Assist {
            open: false,
            typing: false,
            session: NoraSession::new(config, PageContext::editor(AppKind::Mail, DocumentInfo::named(""))),
            panel: NoraPanel::new().with_error_words(error_words),
            calls: Vec::new(),
            changes: Vec::new(),
        }
    }
}

impl Assist {
    /// Whether Nora is working on the last message.
    #[cfg(test)]
    pub(crate) fn busy(&self) -> bool {
        self.session.is_busy()
    }
}

/// What the conversation refuses before an action reaches the pane.
fn policy() -> CommandPolicy {
    REFUSED_FAMILIES.iter().fold(CommandPolicy::default(), |policy, family| policy.deny_prefix(*family))
}

/// Whether the pane can be opened: a host reaches Nora and takes the calls of her conversation.
pub(crate) fn available(app: &MailApp) -> bool {
    app.services.nora.as_ref().is_some_and(|host| host.chat)
}

/// Whether what is typed goes into the pane's text box: the window's shortcuts don't run from it (Ctrl+Enter
/// would send the message being written, Escape close it).
pub(crate) fn has_keys(app: &MailApp) -> bool {
    app.nora.assist.open && app.nora.assist.typing
}

/// `ui.noraAssistant {"open"?: bool}`: open or close the pane (without `open`, the other way round).
pub(crate) fn toggle(app: &mut MailApp, p: &Value) -> Result<Value, String> {
    if !available(app) {
        return Err("Nora's pane needs a hosting page that reaches her conversation (NeoMail in Neoffice)".to_string());
    }
    let open = match p.get("open") {
        None | Some(Value::Null) => !app.nora.assist.open,
        Some(open) => open.as_bool().ok_or_else(|| "`open` is true or false".to_string())?,
    };
    if open && !app.nora.assist.open {
        app.nora.assist.panel.focus_input();
    }
    if !open {
        close(app);
    }
    app.nora.assist.open = open;
    Ok(json!({"open": open}))
}

/// The pane closes: what Nora was doing stops with it (she would go on writing into a message the person
/// no longer watches her write into).
fn close(app: &mut MailApp) {
    app.nora.assist.open = false;
    app.nora.assist.typing = false;
    if app.nora.assist.session.is_busy() {
        let outputs = app.nora.assist.session.handle(Input::Cancel);
        perform(app, outputs);
    }
}

/// The pane at the right of the window, while it is open: its title with a close button, then the
/// conversation. What the person did in it goes to the conversation at once.
pub(crate) fn side(app: &mut MailApp, ui: &mut Ui) {
    if !app.nora.assist.open || !available(app) {
        app.nora.assist.typing = false;
        return;
    }
    let t = Tokens::get(ui.ctx());
    let mut actions = Vec::new();
    let mut closing = false;
    let mut typing = false;
    egui::Panel::right("nora_assistant")
        .default_size(330.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(Margin::same(10)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            closing = title(ui, &t);
            ui.add_space(4.0);
            // The panel's text box, as craft-nora-ui names it in the `Ui` it is drawn in.
            let input = ui.id().with("nora").with("input");
            let assist = &mut app.nora.assist;
            assist.panel.labels = labels();
            assist.panel.style = style(&t);
            actions = assist.panel.show(ui, &assist.session);
            typing = ui.ctx().memory(|m| m.has_focus(input));
        });
    app.nora.assist.typing = typing;
    if closing {
        close(app);
        return;
    }
    // The conversation's clock first: a message is stamped with it.
    let now = now_ms().max(0.0) as u64;
    let mut outputs = app.nora.assist.session.handle(Input::Tick { now_ms: now });
    for action in actions {
        if matches!(action, PanelAction::Send(_)) {
            // Where the person is, as of the message: told to Nora with it.
            let context = context_of(app);
            outputs.extend(app.nora.assist.session.handle(Input::SetContext(context)));
        }
        if action == PanelAction::NewConversation {
            app.nora.assist.changes.clear();
        }
        outputs.extend(app.nora.assist.session.handle(action.into_input()));
    }
    perform(app, outputs);
}

/// The pane's title: Nora's sphere and her name, and a button that closes the pane. Whether it was clicked.
fn title(ui: &mut Ui, t: &Tokens) -> bool {
    let tone = look::tone(t);
    let mut close = false;
    ui.horizontal(|ui| {
        let (mark, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        look::orb(ui.painter(), mark, tone.orb);
        ui.label(RichText::new(tl!("Nora")).font(semibold(13.0)).color(tone.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = crate::widgets::icon_button(ui, "close", tl!("Close Nora's pane"), 12.0, true).clicked();
        });
    });
    close
}

/// The panel's words, in the interface's language. Its button says Ask, not Send: in a mail program, Send
/// is the message's.
fn labels() -> NoraLabels {
    NoraLabels {
        title: String::new(),
        new_conversation: tl!("New conversation").into(),
        undo: tl!("Undo").into(),
        undo_hint: tl!("Take back the last thing Nora wrote in your message").into(),
        placeholder: tl!("Ask Nora…").into(),
        send: tl!("Ask").into(),
        stop: tl!("Stop").into(),
        working: tl!("Nora is working…").into(),
        stopped: tl!("Stopped.").into(),
        empty: tl!("Ask Nora about the message you are reading, or to write your reply. Nothing she writes is sent: you read it and send it.").into(),
        confirm_heading: tl!("Nora wants to:").into(),
        allow: tl!("Allow").into(),
        refuse: tl!("Refuse").into(),
        describe,
    }
}

/// What an action that waits for the person's yes would do (none does in the pane today: she sends and
/// saves nothing), in the interface's language.
fn describe(kind: &DocActionKind) -> String {
    match kind {
        DocActionKind::Save => tl!("Save the message").to_string(),
        _ => tl!("Change the message you are writing").to_string(),
    }
}

/// The panel's colours: the window's own, with Nora's clay for her bubbles.
fn style(t: &Tokens) -> NoraStyle {
    let tone = look::tone(t);
    NoraStyle { user_fill: Some(t.selection), user_text: Some(t.text), nora_fill: Some(tone.fill), nora_text: Some(t.text), ..NoraStyle::default() }
}

/// A failure of the connection to Nora, in the interface's language.
fn error_words(error: &NoraError) -> String {
    if error.is_auth() {
        return tl!("Your Neoffice session has ended: reload the page, then ask Nora again.").to_string();
    }
    match error {
        NoraError::Timeout => tl!("Nora did not answer in time: ask her again.").to_string(),
        NoraError::Unreachable(why) => fmt(tl!("Nora couldn't be reached: {reason}"), &[("reason", &said(why))]),
        other => fmt(tl!("Nora couldn't answer: {reason}"), &[("reason", &said(&other.to_string()))]),
    }
}

/// A server's sentence as the pane shows it: on one line, [`MAX_SAID`] characters at most.
fn said(text: &str) -> String {
    let flat: String = text.chars().take(MAX_SAID + 1).map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.trim();
    if flat.chars().count() > MAX_SAID { format!("{}…", flat.chars().take(MAX_SAID).collect::<String>()) } else { flat.to_string() }
}

/// Where the person is, as Nora is told with each message: NeoMail, the subject of the message being
/// written or read, and the selection in the message being written.
fn context_of(app: &MailApp) -> PageContext {
    let mut document = DocumentInfo::named(observe::document_name(app));
    document.selection = observe::draft_selection(app).map(|text| Selection { text });
    PageContext::editor(AppKind::Mail, document)
}

/// Once a frame, open or not: the answers that came are handed to the conversation, its clock moves on,
/// and what it asks is done.
pub(crate) fn poll(app: &mut MailApp) {
    let now = now_ms();
    let mut outputs = deliver(app, now);
    outputs.extend(app.nora.assist.session.handle(Input::Tick { now_ms: now.max(0.0) as u64 }));
    perform(app, outputs);
    // The host wakes the window when an answer comes; the conversation's own clock says when to read again.
    let wake = app.nora.assist.session.next_wake_ms().map(|at| at.saturating_sub(now.max(0.0) as u64));
    if let (Some(ms), Some(ctx)) = (wake, &app.ctx) {
        ctx.request_repaint_after(Duration::from_millis(ms.max(50)));
    }
}

/// The answers the host gave since the last look, as the conversation reads them; a call the host let go,
/// or that took too long, is an answer with no status and why.
fn deliver(app: &mut MailApp, now: f64) -> Vec<Output> {
    let calls = std::mem::take(&mut app.nora.assist.calls);
    let mut kept = Vec::with_capacity(calls.len());
    let mut outputs = Vec::new();
    for call in calls {
        let response = match call.answer.try_recv() {
            Ok(answer) if answer.id == call.id => {
                // No status: the host's reason, as craft-nora reads one.
                let body = if answer.status == 0 { json!({"exception": answer.body}).to_string() } else { answer.body };
                HttpResponse { id: call.id, status: answer.status, body }
            }
            Ok(_) | Err(TryRecvError::Disconnected) => no_answer(call.id, tl!("the connection stopped before Nora answered")),
            Err(TryRecvError::Empty) if now - call.since > DEADLINE_MS => no_answer(call.id, tl!("it took too long")),
            Err(TryRecvError::Empty) => {
                kept.push(call);
                continue;
            }
        };
        outputs.extend(app.nora.assist.session.handle(Input::Http(response)));
    }
    kept.append(&mut app.nora.assist.calls);
    app.nora.assist.calls = kept;
    outputs
}

fn no_answer(id: u64, why: &str) -> HttpResponse {
    HttpResponse { id, status: 0, body: json!({"exception": why}).to_string() }
}

/// Does what the conversation asks: a call goes to the host, an action is played in the window, a change
/// repaints. Results and refusals come back as inputs and may ask for more, so this goes round a few times
/// at most.
fn perform(app: &mut MailApp, outputs: Vec<Output>) {
    let mut queue = outputs;
    for _ in 0..8 {
        if queue.is_empty() {
            return;
        }
        let mut next = Vec::new();
        for output in std::mem::take(&mut queue) {
            match output {
                Output::Http(request) => next.extend(send(app, request)),
                Output::Document(call) => {
                    let result = play(app, &call.kind);
                    next.extend(app.nora.assist.session.handle(Input::DocumentResult { id: call.id, result }));
                }
                Output::Changed => {
                    if let Some(ctx) = &app.ctx {
                        ctx.request_repaint();
                    }
                }
            }
        }
        // A save always waits for the person's yes (craft-nora): it is answered at once, and refused there
        // (`play`), rather than shown as a question whose yes would change nothing.
        for waiting in app.nora.assist.session.confirmations() {
            next.extend(app.nora.assist.session.handle(Input::Confirm { id: waiting.id, accepted: true }));
        }
        queue = next;
    }
}

/// One call of the conversation, to the host: only one of [`CHAT_METHODS`], only to a host that takes them.
/// One that can't go is answered at once, with why.
fn send(app: &mut MailApp, request: HttpRequest) -> Vec<Output> {
    let method = request.path.strip_prefix("/api/method/").and_then(|asked| CHAT_METHODS.iter().copied().find(|m| *m == asked));
    let host = app.services.nora.as_ref().filter(|host| host.chat);
    match (method, host) {
        (Some(method), Some(host)) => {
            let answer = (host.call)(NoraCall { id: request.id, method, args: request.body });
            app.nora.assist.calls.push(Call { id: request.id, since: now_ms(), answer });
            Vec::new()
        }
        (None, _) => {
            let refused = no_answer(request.id, "that is not one of the calls of Nora's conversation");
            app.nora.assist.session.handle(Input::Http(refused))
        }
        (Some(_), None) => {
            let refused = no_answer(request.id, tl!("nothing hosting this window reaches Nora's conversation"));
            app.nora.assist.session.handle(Input::Http(refused))
        }
    }
}

/// Plays one action of Nora's in the window. One that wrote answers what it wrote and where; one that
/// changed nothing says so (`"changed": false`), and leaves nothing to undo.
fn play(app: &mut MailApp, kind: &DocActionKind) -> Result<Value, String> {
    // While a signature is open in File › Signatures, the composer's commands work in it.
    if app.session.signature_edit.is_some() && *kind != DocActionKind::Observe {
        return Err("a signature is open in File › Signatures: what Nora does would go into it; the person closes it first".to_string());
    }
    match kind {
        DocActionKind::Observe => Ok(observe::observation(app)),
        DocActionKind::TypeText { text, .. } => write(app, Place::Caret, text),
        DocActionKind::SelectText { text, occurrence } => select(app, text, *occurrence),
        DocActionKind::Execute { command, params } => execute(app, command, params),
        DocActionKind::Batch { .. } => {
            Err("NeoMail takes one command at a time: draft.reply, draft.new or draft.write, each in a call of its own".to_string())
        }
        DocActionKind::RenderPage { .. } => Err("a message has no pages to look at: read it with doc_observe".to_string()),
        DocActionKind::Save => Err("saving or sending the message is the person's: tell them it is ready in their window".to_string()),
        DocActionKind::Undo => undo(app),
    }
}

/// One of Nora's [`COMMANDS`], with its parameters.
fn execute(app: &mut MailApp, command: &str, params: &Value) -> Result<Value, String> {
    let text = || params.get("text").and_then(Value::as_str).ok_or_else(|| format!("`{command}` takes the text to write as `text`"));
    match command {
        "draft.reply" => {
            let all = match params.get("all") {
                None | Some(Value::Null) => false,
                Some(all) => all.as_bool().ok_or_else(|| "`all` is true or false".to_string())?,
            };
            reply(app, text()?, all)
        }
        "draft.new" => new_message(app, text()?, params),
        "draft.write" => {
            let place = match params.get("at").and_then(Value::as_str).unwrap_or("top") {
                "top" | "start" => Place::Top,
                "end" => Place::End,
                "replace" => Place::Replace,
                other => return Err(format!("`at` is \"top\", \"end\" or \"replace\", not {other:?}")),
            };
            write(app, place, text()?)
        }
        other => Err(format!(
            "`{other}` is not one of Nora's commands in NeoMail: she reads the window and writes into the message being written \
             ({}); sending, deleting, moving, filing and flagging are the person's",
            COMMANDS.join(", ")
        )),
    }
}

/// Where Nora writes in the message being written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// At the caret, over the selection.
    Caret,
    /// Above everything: the person's text, the signature and the original.
    Top,
    /// After the person's own text, above the signature and the original.
    End,
    /// Over the person's own text (above the signature and the original).
    Replace,
}

impl Place {
    fn name(self) -> &'static str {
        match self {
            Place::Caret => "caret",
            Place::Top => "top",
            Place::End => "end",
            Place::Replace => "replace",
        }
    }
}

const NO_DRAFT: &str = "no message is being written: open the reply with draft.reply (it writes the text too), or a new message with draft.new";

/// `text` as it goes into a message: line breaks and tabs, no other control characters, and not too much.
fn written(text: &str) -> Result<String, String> {
    let text = observe::clean(text);
    let text = text.trim_matches('\n');
    if text.trim().is_empty() {
        return Err("say what to write: `text` is empty".to_string());
    }
    if text.chars().nth(MAX_REPLY).is_some() {
        return Err(format!("that is too long to write in one go: {MAX_REPLY} characters at most"));
    }
    Ok(text.to_string())
}

/// Nora's text into the message being written, at `place`, as one step Undo takes back.
fn write(app: &mut MailApp, place: Place, text: &str) -> Result<Value, String> {
    let text = written(text)?;
    if app.session.signature_edit.is_some() {
        return Err("a signature is open in File › Signatures: what Nora writes would go into it; the person closes it first".to_string());
    }
    let Some(c) = app.session.compose.as_ref() else { return Err(NO_DRAFT.to_string()) };
    let (mode, source) = (c.mode, c.source);
    let before = c.body.text().clone();
    let kept = c.body.selection();
    let own = own_text::own_text(c.body.text());
    let first = c.body.text().paragraphs().first().map(|p| (p.signature, p.is_empty()));
    let has_own = !own.text.trim().is_empty();
    // Where it goes, and what it is put over: (from, to, the text with the blank line that keeps it apart).
    let (from, to, put) = match place {
        Place::Caret => {
            let s = c.body.selection();
            (s.start(), s.end(), text)
        }
        Place::End if has_own => (own.chars, own.chars, format!("\n\n{text}")),
        Place::Replace if has_own => (0, own.chars, text),
        // At the top (or at the end, or over, a text that isn't there): above all of it.
        _ => match first {
            // Pasted into a paragraph of the signature, its first line would be the signature's.
            Some((true, _)) => {
                return Err("the message starts with its signature: the person leaves a line above it for Nora's text".to_string());
            }
            Some((false, false)) => (0, 0, format!("{text}\n\n")),
            _ => (0, 0, text),
        },
    };
    app.execute("compose.select", json!({"from": from, "to": to}))?;
    app.execute("edit.paste", json!({"text": put}))?;
    let Some(after) = app.session.compose.as_ref().map(|c| c.body.text().clone()) else { return Err(NO_DRAFT.to_string()) };
    if after == before {
        return Ok(json!({"written": 0, "where": place.name(), "changed": false}));
    }
    // The person's caret stays in their text where she wrote elsewhere: past what went in when it was past
    // what it replaced, at its end when it was in it. At the caret, it is after her text, as after typing.
    if place != Place::Caret {
        let (was, now) = (before.len(), after.len());
        let moved = |at: usize| match at {
            at if at >= to => (at + now).saturating_sub(was),
            at if at >= from => (to + now).saturating_sub(was),
            at => at,
        };
        let _ = app.execute("compose.select", json!({"from": moved(kept.anchor), "to": moved(kept.head)}));
    }
    let changes = &mut app.nora.assist.changes;
    changes.push(Change { mode, source, before, after });
    if changes.len() > MAX_CHANGES {
        changes.remove(0);
    }
    let c = app.session.compose.as_ref();
    Ok(json!({
        "written": put.chars().count(),
        "where": place.name(),
        "message": {"mode": c.map(|c| mode_name(c.mode)), "subject": c.map(|c| c.subject.clone()), "to": c.map(|c| c.to.clone())},
        "note": "It is in the message being written, nothing is sent: the person reads it and sends it.",
    }))
}

fn mode_name(mode: ComposeMode) -> &'static str {
    match mode {
        ComposeMode::New => "new",
        ComposeMode::Reply => "reply",
        ComposeMode::ReplyAll => "replyAll",
        ComposeMode::Forward => "forward",
        ComposeMode::Draft => "draft",
    }
}

/// `draft.reply`: Nora's reply at the top of the reply to the message being read, opened when it isn't yet
/// (Reply, or Reply All with `all`). A reply already being written takes it at its top.
fn reply(app: &mut MailApp, text: &str, all: bool) -> Result<Value, String> {
    let text = written(text)?;
    if let Some(c) = app.session.compose.as_ref() {
        if matches!(c.mode, ComposeMode::Reply | ComposeMode::ReplyAll) {
            return write(app, Place::Top, &text);
        }
        return Err("another message is being written: write into it with draft.write, or the person closes it first".to_string());
    }
    let Some(message) = app.session.sel.focus() else {
        return Err("no message is open: the person opens the message to answer first".to_string());
    };
    let opened = app.run(if all { "mail.replyAll" } else { "mail.reply" }, json!({"id": message}))?;
    let replying =
        app.session.compose.as_ref().is_some_and(|c| matches!(c.mode, ComposeMode::Reply | ComposeMode::ReplyAll) && c.source == Some(message));
    if replying {
        let mut answer = write(app, Place::Top, &text)?;
        if let Some(o) = answer.as_object_mut() {
            o.insert("opened".into(), json!(if all { "replyAll" } else { "reply" }));
        }
        return Ok(answer);
    }
    match opened.get("pending").and_then(Value::as_str) {
        Some(_) => Err("the message is still being downloaded: ask again in a moment".to_string()),
        None => Err("the reply could not be opened".to_string()),
    }
}

/// `draft.new`: a new message, with the recipients and subject Nora gives (`to`, `cc`, `subject`), and her
/// text in it. Nothing is sent.
fn new_message(app: &mut MailApp, text: &str, params: &Value) -> Result<Value, String> {
    let text = written(text)?;
    if app.session.compose.is_some() {
        return Err("a message is being written already: write into it with draft.write, or the person closes it first".to_string());
    }
    let mut fields = serde_json::Map::new();
    for key in ["to", "cc", "subject"] {
        match params.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::String(value)) => {
                let one_line: String = value.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
                let one_line = one_line.trim();
                if one_line.chars().nth(MAX_FIELD).is_some() {
                    return Err(format!("`{key}` is too long"));
                }
                if !one_line.is_empty() {
                    fields.insert(key.to_string(), json!(one_line));
                }
            }
            Some(_) => return Err(format!("`{key}` is text")),
        }
    }
    app.run("mail.new", Value::Object(fields))?;
    if app.session.compose.as_ref().is_none_or(|c| c.mode != ComposeMode::New) {
        return Err("the new message could not be opened".to_string());
    }
    let mut answer = write(app, Place::Top, &text)?;
    if let Some(o) = answer.as_object_mut() {
        o.insert("opened".into(), json!("new"));
    }
    Ok(answer)
}

/// Select the `occurrence`-th time `text` is in the message being written, so that her next write goes over it.
fn select(app: &mut MailApp, text: &str, occurrence: u32) -> Result<Value, String> {
    let Some(c) = app.session.compose.as_ref() else {
        return Err("selecting works in the message being written, and none is: Nora reads a message with doc_observe".to_string());
    };
    let body: Vec<char> = c.body.plain().chars().collect();
    let wanted: Vec<char> = text.chars().take(MAX_SELECT + 1).collect();
    if wanted.is_empty() {
        return Err("say which text to select".to_string());
    }
    if wanted.len() > MAX_SELECT {
        return Err(format!("select at most {MAX_SELECT} characters at a time"));
    }
    let mut seen = 0;
    let mut at = None;
    for start in 0..body.len().saturating_sub(wanted.len() - 1) {
        if body.get(start..start + wanted.len()) == Some(wanted.as_slice()) {
            seen += 1;
            if seen == occurrence.max(1) {
                at = Some(start);
                break;
            }
        }
    }
    let Some(from) = at else {
        return Err(format!("{text:?} is not in the message being written ({seen} time(s))"));
    };
    app.execute("compose.select", json!({"from": from, "to": from + wanted.len()}))?;
    Ok(json!({"selected": text, "from": from, "changed": false}))
}

/// The pane's Undo: Nora's last write taken back, only while the message it went into is still as she left it
/// (the person's typing since is theirs, and the composer's own Undo is there for it).
fn undo(app: &mut MailApp) -> Result<Value, String> {
    let Some(change) = app.nora.assist.changes.pop() else { return Err(tl!("Nora has written nothing here to take back.").to_string()) };
    let still = app.session.compose.as_ref().filter(|c| c.mode == change.mode && c.source == change.source).map(|c| c.body.text().clone());
    if still.as_ref() != Some(&change.after) {
        // What she wrote before lies under the person's typing too.
        app.nora.assist.changes.clear();
        return Err(tl!("The message changed since Nora wrote in it: undo your own way with the message's Undo.").to_string());
    }
    app.execute("compose.undo", json!({}))?;
    // The step taken back must be hers: anything else is put back as it was.
    let back = app.session.compose.as_ref().map(|c| c.body.text().clone());
    if back.as_ref() != Some(&change.before) {
        let _ = app.execute("compose.redo", json!({}));
        app.nora.assist.changes.clear();
        return Err(tl!("The message changed since Nora wrote in it: undo your own way with the message's Undo.").to_string());
    }
    Ok(json!({"undone": true}))
}

#[cfg(test)]
#[path = "assist_tests.rs"]
mod tests;
