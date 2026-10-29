// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): Nora's ready replies under the message being read, and « My style ».

//! Nora's ready replies: under the header of a message being read, up to two short replies and a full draft, written
//! the way the person writes, with a pill for each thing Nora could not know (a date, an amount). The person fills the
//! pills in, then Sends (after five seconds they can take back), Edits (the reply opens, the text at its top) or
//! Dismisses. Nothing goes out without their click, and a message never goes out with a field of Nora's still empty
//! in it ([`unfilled_in_compose`], which `compose.send` checks).
//!
//! When: a message of a mailbox the hosting page handed over, written by a person (its kind of sender, as the Inbox's
//! tabs know it), in a folder mail arrives in (not Sent, Drafts, Junk, Deleted Items, the Outbox), not from the mailbox
//! itself, not answered yet, read in the pane for a second: then one call, `ready_replies`, names it by its id on the
//! server (the server reads it, through its guard). Its answer is kept by the server: opening the message again costs
//! nothing. The first time, the server asks for the person's yes: a card says what Nora reads and keeps, and « My
//! style » changes it later.
//!
//! « My style » shows what Nora keeps of the person's writing (in plain sentences, with the excerpts of their sent mail
//! it uses as examples), and turns ready replies on or off, learns the style again, or forgets it (`my_style`).
//!
//! Both methods go out where Nora's conversation does ([`super::Host::chat`]): from the page `/neomail`; Neodesk's shell
//! lets neither through yet. What is drawn is `craft-nora-ui`'s card of a proposal, in Nora's colours.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, TryRecvError};

use craft_nora::proposal::{self, FieldKind, Proposal, Ready as Answered, StyleState};
use craft_nora_ui::proposal::{Choice, ConsentWords, Context, ProposalAction, ProposalCard, ProposalTone, ProposalWords, consent};
use egui::{Margin, RichText, Sense, Ui, vec2};
use mailcraft_model::{FolderKind, Id, Message};
use serde_json::{Value, json};

use super::answer;
use super::flow::page_mailbox;
use super::look;
use super::{DEADLINE_MS, MY_STYLE, NoraAnswer, NoraCall, READY_REPLIES};
use crate::MailApp;
use crate::i18n::fmt;
use crate::reading::Place;
use crate::theme::{Tokens, regular, semibold};

/// How long a message is read before Nora is asked, in milliseconds: no call for the messages gone through with the
/// arrow keys.
pub(crate) const WAIT_MS: f64 = 1_000.0;
/// How often the window looks for an answer while a call is in flight, in milliseconds.
const LOOK_AGAIN_MS: u64 = 100;
/// The calls of the ready replies are numbered apart from the window's other calls of Nora's.
const FIRST_ID: u64 = 1 << 40;
/// How long a reply waits for its message to download before it is let go, in milliseconds.
const PENDING_MAX_MS: f64 = 330_000.0;

/// What the card shows for the message.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Shown {
    /// Nothing: none wanted, the person said no, or asking came to nothing.
    Nothing,
    Proposal {
        proposal: Proposal,
        style: StyleState,
    },
    /// The card that asks for the person's yes.
    Consent,
    /// The guard held the message back: what the server said.
    Blocked(String),
}

/// A call of the ready replies in flight.
struct Asked {
    id: u64,
    answer: Receiver<NoraAnswer>,
    since: f64,
    what: Asking,
}

#[derive(Clone, Debug, PartialEq)]
enum Asking {
    /// The ready replies to this message.
    Replies(Id),
    /// An action of « My style » (`get`, `enable`…).
    Style(&'static str),
}

/// A reply whose message is being downloaded: the text goes in once it opens.
struct Pending {
    message: Id,
    text: String,
    since: f64,
}

/// « My style », while it is open.
#[derive(Default)]
pub(crate) struct StylePanel {
    pub(crate) open: bool,
    /// What the server said of the person's style (`my_style`'s answer).
    pub(crate) view: Option<Value>,
    pub(crate) error: Option<String>,
    show_examples: bool,
}

/// What the window knows of the ready replies while it runs. Nothing of it is kept between runs.
#[derive(Default)]
pub(crate) struct ReadyState {
    /// The message the card is about, and since when it is read (milliseconds).
    pub(crate) message: Option<Id>,
    since: f64,
    pub(crate) shown: Option<Shown>,
    pub(crate) card: ProposalCard,
    asked: Vec<Asked>,
    last: u64,
    /// The messages whose card the person dismissed.
    dismissed: HashSet<Id>,
    /// The person said not now, or turned them off: nothing more is asked this run (« My style » asks again).
    pub(crate) quiet: bool,
    pending: Option<Pending>,
    pub(crate) style: StylePanel,
}

impl ReadyState {
    /// Pretend the message was opened `ms` milliseconds earlier.
    #[cfg(test)]
    pub(crate) fn age(&mut self, ms: f64) {
        self.since -= ms;
    }

    fn asking(&self, what: &Asking) -> bool {
        self.asked.iter().any(|a| a.what == *what)
    }
}

/// Whether the ready replies may be offered for `m`, in this window, now.
pub(crate) fn wanted(app: &MailApp, m: &Message) -> bool {
    let Some(host) = app.services.nora.as_ref() else { return false };
    if !host.chat || page_mailbox(app, m.account).is_none() || app.nora.ready.dismissed.contains(&m.id) {
        return false;
    }
    let on_server = m.server.as_ref().is_some_and(|s| !s.id.is_empty() && s.id.len() <= 512);
    let arrives =
        app.session.mailbox.folder(m.folder).is_some_and(|f| matches!(f.kind, FolderKind::Inbox | FolderKind::Archive | FolderKind::Custom));
    let own = app.session.mailbox.account(m.account).is_some_and(|a| a.email.trim().eq_ignore_ascii_case(m.from.email.trim()));
    on_server && arrives && !own && !m.answered && !m.draft && m.stream_or_person() == mailcraft_model::stream::Stream::Person
}

/// The message the reading pane shows, when it is one.
fn read_now(app: &MailApp) -> Option<std::sync::Arc<Message>> {
    if app.session.compose.is_some() || app.session.view.module != mailcraft_engine::Module::Mail {
        return None;
    }
    app.session.sel.focus().and_then(|id| app.session.mailbox.messages.get(&id).cloned())
}

/// Ask the host; `None` without one.
fn call(app: &mut MailApp, method: &'static str, args: Value, what: Asking) -> Option<u64> {
    let host = app.services.nora.as_ref()?;
    let state = &mut app.nora.ready;
    state.last = state.last.wrapping_add(1);
    let id = FIRST_ID + state.last;
    let answer = (host.call)(NoraCall { id, method, args });
    state.asked.push(Asked { id, answer, since: crate::now_ms(), what });
    Some(id)
}

/// Ask for « My style »'s `action` (`get`, `enable`, `disable`, `decline`, `relearn`, `forget`).
pub(crate) fn style_action(app: &mut MailApp, action: &'static str) {
    if !app.nora.ready.asking(&Asking::Style(action)) {
        let _ = call(app, MY_STYLE, json!({"action": action}), Asking::Style(action));
    }
}

/// Open « My style », and read what the server keeps.
pub(crate) fn open_style(app: &mut MailApp) -> Result<Value, String> {
    if app.services.nora.as_ref().is_none_or(|h| !h.chat) {
        return Err(tl!("Nora's ready replies aren't offered here.").to_string());
    }
    app.nora.ready.style.open = true;
    app.nora.ready.style.error = None;
    style_action(app, "get");
    Ok(json!({"open": true}))
}

/// Once a frame: follow the message being read, ask for its ready replies when it has been read for a second, take in
/// the answers, write a reply that waited for its message.
pub(crate) fn poll(app: &mut MailApp) {
    let now = crate::now_ms();
    let read = read_now(app);
    let id = read.as_ref().map(|m| m.id);
    if id != app.nora.ready.message {
        let state = &mut app.nora.ready;
        state.message = id;
        state.since = now;
        state.shown = None;
        state.card = ProposalCard::default();
    }
    if let Some(m) = read.as_deref()
        && app.nora.ready.shown.is_none()
        && !app.nora.ready.quiet
        && !app.nora.ready.asking(&Asking::Replies(m.id))
        && wanted(app, m)
    {
        let left = WAIT_MS - (now - app.nora.ready.since);
        if left <= 0.0 {
            if let (Some(mailbox), Some(server)) = (page_mailbox(app, m.account), m.server.as_ref().map(|s| s.id.clone())) {
                let _ = call(app, READY_REPLIES, json!({"account_name": mailbox, "email_id": server}), Asking::Replies(m.id));
            }
        } else if let Some(ctx) = &app.ctx {
            ctx.request_repaint_after(std::time::Duration::from_millis(left as u64 + 20));
        }
    }
    take_answers(app, now);
    write_pending(app, now);
    if !app.nora.ready.asked.is_empty()
        && let Some(ctx) = &app.ctx
    {
        ctx.request_repaint_after(std::time::Duration::from_millis(LOOK_AGAIN_MS));
    }
}

fn take_answers(app: &mut MailApp, now: f64) {
    let asked = std::mem::take(&mut app.nora.ready.asked);
    let mut kept = Vec::with_capacity(asked.len());
    for a in asked {
        let landed = match a.answer.try_recv() {
            Ok(answer) if answer.id == a.id => Some(answer::ready(&answer)),
            Ok(_) | Err(TryRecvError::Disconnected) => Some(Err(answer::not_reached(tl!("the connection stopped before Nora answered")))),
            Err(TryRecvError::Empty) if now - a.since > DEADLINE_MS => Some(Err(tl!("it took too long").to_string())),
            Err(TryRecvError::Empty) => None,
        };
        match landed {
            None => kept.push(a),
            Some(result) => land(app, a.what, result),
        }
    }
    kept.append(&mut app.nora.ready.asked);
    app.nora.ready.asked = kept;
}

fn land(app: &mut MailApp, what: Asking, result: Result<Value, String>) {
    match what {
        Asking::Replies(message) => {
            // Another message is read now: this answer is no longer the card's (the server keeps it for the next visit).
            if app.nora.ready.message != Some(message) {
                return;
            }
            let shown = match result.and_then(|v| proposal::parse_ready(&v)) {
                Ok((Answered::Proposal(proposal), style)) => Shown::Proposal { proposal, style },
                Ok((Answered::ConsentNeeded, _)) => Shown::Consent,
                Ok((Answered::Blocked(said), _)) => Shown::Blocked(said),
                Ok((Answered::Declined | Answered::Disabled, _)) => {
                    app.nora.ready.quiet = true;
                    Shown::Nothing
                }
                Ok((Answered::Skip(_), _)) => Shown::Nothing,
                // Asking came to nothing: the card stays out of the way (the person reads their mail as before).
                Err(why) => {
                    app.status(why);
                    Shown::Nothing
                }
            };
            app.nora.ready.shown = Some(shown);
        }
        Asking::Style(action) => match result {
            Ok(view) => {
                let on = view.get("ready_replies").and_then(Value::as_bool).unwrap_or(false);
                app.nora.ready.quiet = !on;
                app.nora.ready.style.view = Some(view);
                app.nora.ready.style.error = None;
                // Turned on (the card's yes, or « My style »): the message being read is asked again.
                if action == "enable" && on {
                    app.nora.ready.shown = None;
                    app.nora.ready.since -= WAIT_MS;
                }
            }
            Err(why) => app.nora.ready.style.error = Some(why),
        },
    }
}

/// Open the reply to `message` with `text` at its top, as Reply with Nora puts her draft; and send it at once when
/// `send` and it opened at once (a reply that waits for its message to download is only written: the person sends it).
fn reply_with(app: &mut MailApp, message: Id, text: &str, send: bool) {
    let opened = match app.run("mail.reply", json!({"id": message})) {
        Ok(opened) => opened,
        Err(_) => return,
    };
    if super::flow::replying_to(app, message) {
        super::flow::put_draft(app, message, text);
        if send {
            let _ = app.run("compose.send", json!({}));
        }
        return;
    }
    if opened.get("pending").is_some() {
        app.nora.ready.pending = Some(Pending { message, text: text.to_string(), since: crate::now_ms() });
        app.status(tl!("The reply opens once the message is here: read it, then send it."));
    }
}

fn write_pending(app: &mut MailApp, now: f64) {
    let Some(p) = &app.nora.ready.pending else { return };
    if super::flow::replying_to(app, p.message) {
        if let Some(p) = app.nora.ready.pending.take() {
            super::flow::put_draft(app, p.message, &p.text);
        }
    } else if now - p.since > PENDING_MAX_MS || (app.session.compose.is_some() && !app.fetching.fetching(p.message)) {
        app.nora.ready.pending = None;
    }
}

/// What the person decided on the card of `message`.
pub(crate) fn act(app: &mut MailApp, message: Id, action: ProposalAction) {
    match action {
        ProposalAction::Send(text) => {
            app.nora.ready.dismissed.insert(message);
            reply_with(app, message, &text, true);
        }
        ProposalAction::Edit(text) => reply_with(app, message, &text, false),
        ProposalAction::Dismiss => {
            app.nora.ready.dismissed.insert(message);
            app.nora.ready.shown = Some(Shown::Nothing);
        }
        ProposalAction::OpenStyle => {
            let _ = open_style(app);
        }
    }
}

/// The person's answer to the card that asks: yes turns ready replies on (and lets Nora learn their style), not now
/// keeps them off.
pub(crate) fn consented(app: &mut MailApp, yes: bool) {
    app.nora.ready.shown = Some(Shown::Nothing);
    if yes {
        style_action(app, "enable");
    } else {
        app.nora.ready.quiet = true;
        style_action(app, "decline");
    }
}

/// When the message being written still has a field of Nora's empty in the person's own text: what to tell them
/// (`compose.send` is refused then, whoever asks for it).
pub(crate) fn unfilled_in_compose(app: &MailApp) -> Option<String> {
    let c = app.session.compose.as_ref()?;
    let own = super::own_text::own_text(c.body.text());
    let left = proposal::unfilled(&own.text);
    let first = left.first()?;
    Some(fmt(tl!("A field of Nora's is still empty: \"{label}\". Fill it in or delete it before sending."), &[("label", first)]))
}

/// The card's words, in the interface's language.
fn words() -> ProposalWords {
    ProposalWords {
        heading: tl!("Ready replies").to_string(),
        draft: tl!("NORA'S DRAFT").to_string(),
        to: tl!("to").to_string(),
        fields_left: |n| tln!(n, "{n} field to fill in", "{n} fields to fill in"),
        send: tl!("Send").to_string(),
        edit: tl!("Edit").to_string(),
        dismiss: tl!("Dismiss").to_string(),
        sending: |n| tln!(n, "Sending in {n} second…", "Sending in {n} seconds…"),
        cancel: tl!("Cancel").to_string(),
        footer: tl!("Your signature and the quoted message are added when it is sent.").to_string(),
        other_value: tl!("Something else").to_string(),
        keep: tl!("OK").to_string(),
        keys: tl!("Enter: keep · Tab: next field · Escape: close").to_string(),
        style_link: tl!("In your style · My style").to_string(),
        learning: tl!("Nora is learning your style from your sent emails.").to_string(),
    }
}

fn consent_words() -> ConsentWords {
    ConsentWords {
        heading: tl!("Ready replies").to_string(),
        text: tl!(
            "Nora can propose replies in your own style. To do so she reads the message you open and learns from the emails you sent. Everything stays on your Neoffice server, for you alone; nothing is sent without your click."
        )
        .to_string(),
        yes: tl!("Turn on").to_string(),
        not_now: tl!("Not now").to_string(),
        later: tl!("You can change this in My style.").to_string(),
    }
}

/// Nora's colours, and the window's for the reply.
fn tone(t: &Tokens) -> ProposalTone {
    let nora = look::tone(t);
    ProposalTone {
        accent: nora.orb,
        text: nora.text,
        fill: nora.fill,
        hover: nora.hover,
        border: nora.border,
        card: t.surface,
        card_border: t.border,
        body: t.text,
        dim: t.text_dim,
        send_fill: t.accent,
        send_text: t.on_accent,
        mark: Some(look::orb),
    }
}

/// The shortcuts of a date field: tomorrow, the coming Friday, next Monday, in two weeks; each in words, in the reply's
/// language.
fn date_choices(app: &MailApp, language: &str) -> Vec<Choice> {
    let today = app.session.today();
    let weekday = |d: mailcraft_time::Date| d.weekday() as u32;
    let mut days = vec![today.add_days(1)];
    let to_friday = (4 + 7 - i64::from(weekday(today))) % 7;
    if to_friday > 1 {
        days.push(today.add_days(to_friday));
    }
    days.push(today.add_days(7 - i64::from(weekday(today))));
    days.push(today.add_days(14));
    days.dedup();
    days.into_iter()
        .map(|d| {
            let said = proposal::date_words(language, weekday(d), u32::from(d.day), u32::from(d.month));
            Choice { shown: said.clone(), value: said }
        })
        .collect()
}

/// The card under the header of `m` in the reading pane: preparing, the proposal, the card that asks, or what the guard
/// said.
pub(crate) fn card(app: &mut MailApp, ui: &mut Ui, m: &Message, place: Place) {
    if place != Place::Pane || app.nora.ready.message != Some(m.id) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let tone = tone(&t);
    let preparing = app.nora.ready.asking(&Asking::Replies(m.id));
    let shown = app.nora.ready.shown.clone();
    if !preparing && !matches!(shown, Some(Shown::Proposal { .. } | Shown::Consent | Shown::Blocked(_))) {
        return;
    }
    egui::Frame::NONE.inner_margin(Margin { left: 20, right: 12, top: 6, bottom: 6 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match shown {
            Some(Shown::Proposal { proposal, style }) => {
                let to = m.reply_to.first().unwrap_or(&m.from).email.clone();
                let language = if proposal.language.is_empty() { "fr".to_string() } else { proposal.language.clone() };
                let dates = date_choices(app, &language);
                let choices = move |kind: FieldKind| if kind == FieldKind::Date { dates.clone() } else { Vec::new() };
                let context = Context { to: &to, choices: &choices, learning: style.learning };
                let mut card = std::mem::take(&mut app.nora.ready.card);
                let actions = card.show(ui, &proposal, &words(), &tone, &context);
                app.nora.ready.card = card;
                for action in actions {
                    act(app, m.id, action);
                }
            }
            Some(Shown::Consent) => {
                if let Some(yes) = consent(ui, &consent_words(), &tone) {
                    consented(app, yes);
                }
            }
            Some(Shown::Blocked(said)) => {
                crate::reading::remote::notice(tone.accent, &t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
                        let (mark, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        look::orb(ui.painter(), mark, tone.accent);
                        ui.add(egui::Label::new(RichText::new(said.as_str()).font(regular(11.5)).color(t.text)).wrap());
                    });
                });
            }
            _ => {
                ui.horizontal(|ui| {
                    let (mark, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    egui::Spinner::new().size(14.0).color(tone.text).paint_at(ui, mark);
                    ui.label(RichText::new(tl!("Nora is preparing a reply in your style…")).font(semibold(12.0)).color(tone.text));
                });
            }
        }
    });
}

/// « My style », over the window while it is open.
pub(crate) fn style_panel(app: &mut MailApp, ctx: &egui::Context) {
    if !app.nora.ready.style.open {
        return;
    }
    let t = Tokens::get(ctx);
    let nora = look::tone(&t);
    let frame = egui::Frame::window(&ctx.global_style()).fill(t.surface).inner_margin(Margin::same(18));
    let width = (ctx.content_rect().width() - 48.0).clamp(200.0, 520.0);
    let id = egui::Id::unique("nora-my-style");
    let view = app.nora.ready.style.view.clone();
    let error = app.nora.ready.style.error.clone();
    let busy = app.nora.ready.asked.iter().any(|a| matches!(a.what, Asking::Style(_)));
    let mut close = false;
    let mut action: Option<&'static str> = None;
    let mut show_examples = app.nora.ready.style.show_examples;
    egui::Modal::new(id).frame(frame).backdrop_color(egui::Color32::from_black_alpha(if t.dark { 110 } else { 60 })).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
        ui.horizontal(|ui| {
            let (mark, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            look::orb(ui.painter(), mark, nora.orb);
            ui.label(RichText::new(tl!("My style")).font(semibold(16.0)).color(t.text));
            if busy {
                ui.spinner();
            }
        });
        let Some(view) = view.as_ref() else {
            if let Some(error) = &error {
                ui.add(egui::Label::new(RichText::new(error.as_str()).color(t.red)).wrap());
            }
            close = ui.button(tl!("Close")).clicked();
            return;
        };
        let learnt = view.pointer("/style/learnt").and_then(Value::as_bool).unwrap_or(false);
        let learning = view.pointer("/style/learning").and_then(Value::as_bool).unwrap_or(false);
        let own = view.get("own_mailbox").and_then(Value::as_bool).unwrap_or(false);
        let said = if learning {
            tl!("Nora is learning your style from your sent emails.").to_string()
        } else if learnt {
            let n = view.get("emails_scanned").and_then(Value::as_u64).unwrap_or(0);
            let on = view.get("learnt_on").and_then(Value::as_str).unwrap_or("");
            fmt(&tln!(n, "Learnt on {date} from {n} sent email.", "Learnt on {date} from {n} sent emails."), &[("date", on)])
        } else if own {
            tl!("Nora hasn't learnt your style yet: she learns it from the emails you sent.").to_string()
        } else {
            tl!("Nora learns your style from a mailbox of yours: you have none here.").to_string()
        };
        ui.add(egui::Label::new(RichText::new(said).color(t.text_dim)).wrap());
        let points: Vec<String> =
            view.get("points").and_then(Value::as_array).into_iter().flatten().filter_map(|p| p.as_str().map(answer::clean)).collect();
        if !points.is_empty() {
            ui.label(RichText::new(tl!("WHAT NORA KEEPS")).font(semibold(11.5)).color(nora.text));
            for point in &points {
                ui.add(egui::Label::new(RichText::new(format!("•  {point}")).color(t.text)).wrap());
            }
        }
        let examples: Vec<String> =
            view.get("examples").and_then(Value::as_array).into_iter().flatten().filter_map(|p| p.as_str().map(answer::clean)).collect();
        if !examples.is_empty() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(tln!(
                        examples.len(),
                        "{n} excerpt of your replies, kept as an example",
                        "{n} excerpts of your replies, kept as examples"
                    ))
                    .color(t.text),
                );
                let word = if show_examples { tl!("Hide") } else { tl!("Show") };
                if ui.link(word).clicked() {
                    show_examples = !show_examples;
                }
            });
            if show_examples {
                for example in &examples {
                    ui.add(egui::Label::new(RichText::new(format!("« {example} »")).font(regular(12.5)).color(t.text_dim)).wrap());
                }
            }
        }
        ui.separator();
        let mut on = view.get("ready_replies").and_then(Value::as_bool).unwrap_or(false);
        if ui.add_enabled(!busy, egui::Checkbox::new(&mut on, tl!("Propose ready replies"))).changed() {
            action = Some(if on { "enable" } else { "disable" });
        }
        ui.add(
            egui::Label::new(
                RichText::new(tl!("With each person, Nora writes \"tu\" or \"vous\" as your last email to them did."))
                    .font(regular(12.0))
                    .color(t.text_dim),
            )
            .wrap(),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!busy && own && !learning, egui::Button::new(tl!("Learn again now"))).clicked() {
                action = Some("relearn");
            }
            if ui.add_enabled(!busy && learnt, egui::Button::new(tl!("Forget my style"))).clicked() {
                action = Some("forget");
            }
        });
        if let Some(error) = &error {
            ui.add(egui::Label::new(RichText::new(error.as_str()).color(t.red)).wrap());
        }
        ui.add(
            egui::Label::new(
                RichText::new(tl!("Kept on your Neoffice server, for you alone: neither your colleagues nor Nora's memory see it."))
                    .font(regular(12.0))
                    .color(t.text_dim),
            )
            .wrap(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = ui.button(tl!("Close")).clicked();
        });
    });
    app.nora.ready.style.show_examples = show_examples;
    let layer = crate::keys::modal_layer(id);
    if crate::keys::take_escape(ctx, layer) {
        close = true;
    }
    if let Some(action) = action {
        style_action(app, action);
    }
    if close {
        app.nora.ready.style.open = false;
    }
}

#[cfg(test)]
#[path = "ready_tests.rs"]
mod tests;
