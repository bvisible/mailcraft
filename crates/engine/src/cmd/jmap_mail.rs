// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! An account kept in step with a JMAP server (RFC 8620, RFC 8621).
//!
//! As for IMAP and Graph, the engine never talks to a server: [`jmap_mail_request`]
//! (`sync.jmapRequest`) says what a run should do, a host does it (in a browser, over `fetch`),
//! and [`apply_jmap_mail`] (`sync.applyJmap`) takes in what it brought back. The request and the
//! answer are JMAP's own (`mailcraft_model::sync::JmapMailRequest`), beside the IMAP and Graph
//! ones rather than bent into them.
//!
//! **What a run reads.** A JMAP server says which messages a mailbox holds, newest first, but not
//! necessarily what changed since the last run (`Email/changes` is optional to offer): a run reads
//! each mailbox's newest messages, up to the account's window (File › Options, "messages to keep
//! per folder", 500 at most). A window that holds all of a mailbox is complete, and the mailbox
//! here is made the same as the server's; a window cut short adds and updates only
//! ([`super::jmap_down`]). The ids this mailbox already has go with the request, so that the host
//! asks the server only for the keywords of those, and for the headers of the others.
//!
//! **Folders** come down by server id as a Graph account's do ([`super::graph_mail`]): the
//! account's standard folders are tied to the server's by role, the others are made here, and a
//! complete listing removes what the server no longer has.
//!
//! **Changes up.** What the owner changed here since the server last said (read marks, flags, a move to
//! another folder, a delete, Deleted Items emptied) goes up first in a run, as the difference between each
//! message and the server's last word on it ([`super::jmap_up`]); the host says what became of each change,
//! in a piece of its own before the mailboxes are read, and the pieces after it show the server after them. A
//! run that only reads the Inbox and the folders on screen also reads the folders where a message waits to
//! be found again: some servers name a message by where it lies, so a moved message has no id until a window
//! of the mailbox it went to shows it ([`super::jmap_down`]).

//!
//! **Sending and drafts** go up with the run ([`super::jmap_send`]): after the changes, what waits in the Outbox
//! is sent and the drafts saved here are filed in the server's Drafts before any window is read, and the
//! mailboxes those copies go to (Sent, Drafts) are read besides, so that what was sent shows in Sent at once.

use std::collections::HashMap;

use mailcraft_model::sync::{
    GraphMessage, JmapChange, JmapKnown, JmapMailRequest, JmapMailResult, JmapOutcome, JmapScope, MAX_JMAP_WINDOW, MAX_MESSAGE_CHANGES, MessagePage,
    SendOutcome, SyncPhase,
};
use mailcraft_model::time::Timestamp;
use mailcraft_model::{AccountKind, FolderKind, Id, Mailbox};
use serde_json::{Value, json};

use super::graph_down::{self, Down};
use super::graph_mail::{Taken, take_listing};
use super::jmap_down::{self, plausible_id};
use super::jmap_send;
use super::jmap_up::{self, Refusals};
use super::sync_progress::{self, Piece};
use super::{bad, fail};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The most mailboxes whose known ids a request names, and the most ids it names in all.
const MAX_KNOWN_MAILBOXES: usize = 500;
const MAX_KNOWN_IDS: usize = 50_000;
/// The most mailboxes a light run reads besides the Inbox (the folder on screen, mostly).
const MAX_ALSO: usize = 8;
/// The most mailboxes it reads besides those, to find again messages that were moved.
const MAX_FOLLOWED: usize = 8;
/// The most windows and error lines an answer is read for: what is past them waits for the next run.
const MAX_PAGES: usize = 1_000;
const MAX_ERRORS: usize = 50;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("sync.jmapRequest", "Send/Receive with JMAP", "Send / Receive", request)
            .pure()
            .params(r#"{"account"?: number, "full"?: bool (every mailbox and the folder list; else the Inbox and "also"), "also"?: [string] (mailbox ids to read besides the Inbox), "background"?: bool} — for an account kept in step with a JMAP server: answers what a host should read and change there"#)
            .when(has_jmap_account),
        CommandSpec::new("sync.applyJmap", "Apply Send/Receive with JMAP", "Send / Receive", apply)
            .pure()
            .params(r#"{"result": JmapMailResult}"#),
    ]
}

fn has_jmap_account(s: &Session) -> Option<&'static str> {
    if s.mailbox.accounts.iter().any(|a| a.kind == AccountKind::Jmap) { None } else { Some("no account is kept in step with a JMAP server") }
}

/// What a run of `account` should do. `full`: every mailbox, and the folder list; else the Inbox
/// and the mailboxes named in `also` (the folder on screen). The folder list is also read when no
/// folder of the account is tied to the server yet. The host adds the credential.
pub fn jmap_mail_request(s: &Session, account: Id, full: bool, also: &[String], background: bool) -> Result<JmapMailRequest, CmdError> {
    let a = s.mailbox.account(account).ok_or_else(|| fail(format!("no account with id {account}")))?;
    if a.kind != AccountKind::Jmap {
        return Err(fail(format!("{} isn't kept in step with a JMAP server", a.name)));
    }
    let server = a.jmap.as_ref().ok_or_else(|| fail(format!("{} has no JMAP server", a.name)))?;
    // The mailbox file may say anything: the address is looked at again here.
    if let Some(why) = server.problem() {
        return Err(fail(format!("{}'s JMAP server can't be reached: {why}", a.name)));
    }
    let window = match s.prefs.sync_limit {
        0 => MAX_JMAP_WINDOW,
        n => n.min(MAX_JMAP_WINDOW),
    };
    let tied = s.mailbox.folders.iter().any(|f| f.account == account && f.server.is_some());
    // What was changed here and the server hasn't heard of.
    let changes = jmap_up::message_changes(s, account, MAX_MESSAGE_CHANGES).0;
    // What waits to go: the Outbox, and the drafts saved here.
    let (sends, drafts) = (jmap_send::sends(s, account), jmap_send::drafts(s, account));
    // Where the copies go is read too, so that a message that was sent is in Sent when the run is over, and a
    // draft that was filed is in Drafts: a light run would not read them otherwise.
    let copies_to = |kind: FolderKind| {
        s.mailbox.folder_of_kind(account, kind).and_then(|f| s.mailbox.folder(f)).and_then(|f| f.server.as_ref()).map(|server| server.id.clone())
    };
    let wanted: Vec<String> = [(!sends.is_empty()).then(|| copies_to(FolderKind::Sent)), (!drafts.is_empty()).then(|| copies_to(FolderKind::Drafts))]
        .into_iter()
        .flatten()
        .flatten()
        .collect();
    let scope = if full || !tied {
        JmapScope::All
    } else {
        let mut named: Vec<String> = Vec::new();
        for id in wanted.iter().chain(also).filter(|id| plausible_id(id)) {
            if named.len() < MAX_ALSO && !named.contains(id) {
                named.push(id.clone());
            }
        }
        // The mailboxes the moved messages go to, and those where a message waits to be found again, are read in
        // this run too, so that a message the server gave a new id is the same message here when the run ends.
        // (Enough of them that the ones already named leave `MAX_FOLLOWED`.)
        let followed = jmap_up::to_find_again(s, account, &changes, MAX_ALSO + MAX_FOLLOWED);
        let followed: Vec<String> = followed.into_iter().filter(|id| !named.contains(id)).take(MAX_FOLLOWED).collect();
        named.extend(followed);
        JmapScope::Light { also: named }
    };
    Ok(JmapMailRequest {
        account,
        email: a.email.clone(),
        session: server.session.clone(),
        bearer: server.bearer,
        window,
        list_folders: full || !tied,
        scope,
        known: known(&s.mailbox, account, window),
        changes,
        sends,
        drafts,
        background,
        // (A Send/Receive reads windows: no email by its id alone.)
        emails: Vec::new(),
    })
}

/// The server ids this mailbox has of `account`'s messages, by mailbox, newest first: at most
/// `window` a mailbox, so that the host asks the server for the keywords alone of those.
fn known(mb: &Mailbox, account: Id, window: u32) -> Vec<JmapKnown> {
    let mut by_mailbox: HashMap<&str, Vec<(Timestamp, &str)>> = HashMap::new();
    for m in mb.messages.values().filter(|m| m.account == account) {
        if let Some(s) = m.server.as_ref().filter(|s| !s.gone && plausible_id(&s.id) && plausible_id(&s.folder)) {
            by_mailbox.entry(s.folder.as_str()).or_default().push((m.received, s.id.as_str()));
        }
    }
    let mut mailboxes: Vec<(&str, Vec<(Timestamp, &str)>)> = by_mailbox.into_iter().collect();
    mailboxes.sort_unstable_by(|a, b| a.0.cmp(b.0));
    let per = usize::try_from(window).unwrap_or(usize::MAX);
    let mut named = 0usize;
    let mut out = Vec::new();
    for (mailbox, mut ids) in mailboxes.into_iter().take(MAX_KNOWN_MAILBOXES) {
        ids.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        let room = MAX_KNOWN_IDS.saturating_sub(named).min(per);
        let ids: Vec<String> = ids.into_iter().take(room).map(|(_, id)| id.to_string()).collect();
        named = named.saturating_add(ids.len());
        if !ids.is_empty() {
            out.push(JmapKnown { mailbox: mailbox.to_string(), ids });
        }
    }
    out
}

/// How many changes made in `account` its server hasn't heard of yet ([`super::account::unpushed`] for an
/// account kept in step with a JMAP server): the marks and moves that differ from what the server last said, the
/// messages deleted for good, and the drafts saved here that it hasn't got in their latest form. (The Outbox is
/// counted by the window, as for any account that sends.)
pub fn unpushed(s: &Session, account: Id) -> usize {
    jmap_up::message_changes(s, account, 0).1.saturating_add(jmap_send::pending_drafts(&s.mailbox, account))
}

fn request(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::id(v, "account")
        .or_else(|| s.mailbox.accounts.iter().find(|a| a.kind == AccountKind::Jmap).map(|a| a.id))
        .ok_or_else(|| fail("no account is kept in step with a JMAP server"))?;
    let also: Vec<String> = match v.get("also") {
        Some(Value::Array(ids)) => ids.iter().take(MAX_ALSO).filter_map(|id| id.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    let full = v.get("full").and_then(Value::as_bool).unwrap_or(false);
    let background = v.get("background").and_then(Value::as_bool).unwrap_or(false);
    serde_json::to_value(jmap_mail_request(s, account, full, &also, background)?).map_err(|e| fail(e.to_string()))
}

fn apply(s: &mut Session, v: &Value) -> CmdResult {
    let result: JmapMailResult =
        serde_json::from_value(v.get("result").cloned().ok_or_else(|| bad("`result` is required"))?).map_err(|e| bad(format!("`result`: {e}")))?;
    apply_jmap_mail(s, result)
}

/// How many messages of `account` the server has but nothing says under what id: they were moved, and no window of the
/// mailbox they went to has shown them since (some servers name a message by where it lies, and a mailbox bigger than
/// the window shows only its newest). Nothing can be sent for them.
fn unplaced(mb: &Mailbox, account: Id) -> usize {
    mb.messages.values().filter(|m| m.account == account && m.server.as_ref().is_some_and(|server| server.id.is_empty() && !server.gone)).count()
}

/// `said` and a sentence more, each ending in a full stop.
fn then(said: String, more: &str) -> String {
    if said.is_empty() || said.ends_with('.') { format!("{said} {more}").trim().to_string() } else { format!("{said}. {more}") }
}

/// A line a server sent, as it is shown: one line, no control characters, not too long.
fn said(line: &str) -> String {
    line.chars().map(|c| if c.is_control() { ' ' } else { c }).take(300).collect::<String>().trim().to_string()
}

/// Take in what a JMAP run brought back: the folder list, then each mailbox's window. Not an undo
/// step: the server is the source of truth, but for what was changed here and hasn't reached it.
pub fn apply_jmap_mail(s: &mut Session, result: JmapMailResult) -> CmdResult {
    let account = result.account;
    let a = s.mailbox.account(account).ok_or_else(|| fail(format!("no account with id {account}")))?;
    if a.kind != AccountKind::Jmap {
        return Err(fail(format!("{} isn't kept in step with a JMAP server", a.name)));
    }
    // An answer for another account (the mailbox was replaced while the run was under way) would
    // tie this one's folders to a stranger's server.
    if !result.email.is_empty() && !a.email.trim().eq_ignore_ascii_case(result.email.trim()) {
        return Err(fail(format!("this Send/Receive was for another account, not for {}", a.name)));
    }
    if s.file_is_current() {
        s.note_saved();
    }
    let mut taken = Taken::default();
    let mut errors: Vec<String> = result.errors.iter().take(MAX_ERRORS).map(|e| said(e)).filter(|e| !e.is_empty()).collect();
    if result.auth_refused && !result.partial {
        errors.push("The server refused the sign-in".to_string());
    }
    // What became of the changes made here (RFC 8621 §4.6): first, so that what the windows say is merged with the
    // server's last word as it is now. A refusal is said once per kind of change and reason, and the status says it
    // at the end of the run.
    let mut refusals = Refusals::default();
    let (mut moved, mut told) = (false, false);
    // (The deletes for good are taken in together, after the others: there may be thousands.)
    let mut destroys = Vec::new();
    for done in result.changes.iter().take(MAX_MESSAGE_CHANGES) {
        let took = if matches!(done.change, JmapChange::Destroy { .. }) {
            destroys.push(done);
            true
        } else {
            jmap_up::message_done(&mut s.mailbox, account, done, &mut refusals)
        };
        if took {
            match done.outcome {
                JmapOutcome::Done { .. } => taken.pushed += 1,
                _ => taken.updated += 1,
            }
            // The server was told to move or delete: what was there before can't be brought back by an undo.
            let reached = matches!(done.outcome, JmapOutcome::Done { .. } | JmapOutcome::Gone);
            moved |= reached && matches!(done.change, JmapChange::Move { .. });
            told |= reached && matches!(done.change, JmapChange::Move { .. } | JmapChange::Destroy { .. });
        }
    }
    jmap_up::destroys_done(&mut s.mailbox, account, &destroys, &mut refusals);
    if told {
        s.forget_history();
    }
    // A moved message may have another id now: where each message is, is looked at afresh.
    if moved {
        s.graph_index = Default::default();
    }
    let changes_refused = refusals.lines();
    errors.extend(changes_refused.iter().cloned());
    s.graph_said.extend(changes_refused);
    // The drafts filed and the messages sent (RFC 8621 §4.8, §7): what became of each. A message that went
    // leaves the Outbox, one that was refused or whose answer was lost is held there, with why.
    let mut refused: Vec<String> = Vec::new();
    let waiting_before = jmap_send::destroys_waiting(&s.mailbox, account);
    for done in result.drafts.iter().take(MAX_PAGES) {
        if jmap_send::draft_done(&mut s.mailbox, account, done, &mut refused) {
            taken.updated += 1;
            // A draft's server id is new: where each message is, is looked at afresh.
            s.graph_index = Default::default();
        }
    }
    let mut sent = 0;
    for done in result.sent.iter().take(MAX_PAGES) {
        if jmap_send::send_done(&mut s.mailbox, account, done, &mut refused) {
            sent += usize::from(matches!(done.outcome, SendOutcome::Sent));
            taken.updated += 1;
            s.graph_index = Default::default();
        }
    }
    // The copies the server has that nobody wants and that the host could not destroy wait to be destroyed.
    if jmap_send::unwanted(&mut s.mailbox, account, &result.unwanted) {
        taken.updated += 1;
    }
    // (A copy filed for a draft that is gone here waits to be destroyed too.)
    let queued = jmap_send::destroys_waiting(&s.mailbox, account) > waiting_before;
    // Mail that reached the server, or may have, can't be taken back by Undo; and neither can a message held for
    // that doubt be let go by it (an older state of the mailbox has the message in the Outbox, not held, and the
    // next run would send it again). Whether the message is still in the Outbox or not. The same for a copy that
    // waits to be destroyed: an older state has no such queue, and the copy would come down as mail.
    if queued || result.sent.iter().any(|done| !matches!(done.outcome, SendOutcome::Waits)) {
        s.forget_history();
    }
    errors.splice(0..0, refused.into_iter().map(|line| said(&line)).filter(|line| !line.is_empty()).take(MAX_ERRORS));
    if let Some(listing) = &result.listing {
        take_listing(&mut s.mailbox, account, listing, &mut taken);
    }
    let mut down = Down::default();
    let mut index = std::mem::take(&mut s.graph_index);
    for page in result.pages.iter().take(MAX_PAGES) {
        jmap_down::take_window(&mut s.mailbox, &mut index, account, page, &mut down);
    }
    s.graph_index = index;
    // Every window was read to its end: what is still gone is gone here too.
    if result.whole {
        graph_down::sweep(&mut s.mailbox, account, &mut down);
    }
    if taken != Taken::default() || down != Down::default() || !result.pages.is_empty() {
        s.mailbox_changed();
    }
    // The view stays on a folder that is there.
    if s.mailbox.folder(s.view.folder).is_none() {
        s.view.folder = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap_or(0);
        s.sel.messages.clear();
    }
    // Send/Receive Progress: where the run is, what it brought, what went wrong.
    let step = match (&result.progress, &result.listing, result.sent.is_empty()) {
        (Some(progress), ..) => {
            let name = s.mailbox.folder_by_server(account, &progress.folder).and_then(|f| s.mailbox.folder(f)).map(|f| f.name.as_str());
            Some((SyncPhase::NewMail, sync_progress::shown(name.unwrap_or_default()), progress.read.min(progress.total), progress.total))
        }
        (None, Some(_), _) => Some((SyncPhase::Folders, String::new(), 0, 0)),
        (None, None, false) => Some((SyncPhase::Sending, String::new(), 0, 0)),
        (None, None, true) => None,
    };
    let folders = result.progress.as_ref().map(|p| (p.number, p.folders));
    let piece = Piece { step, folders, new: down.new, sent, problems: &errors, last: !result.partial, stopped: result.stopped };
    s.sync_progress.take(account, piece);
    let summary = match (errors.first(), taken.pushed + taken.added + taken.updated + taken.removed + down.new + down.changed + down.removed) {
        (Some(e), _) => e.clone(),
        (None, 0) => "Up to date".to_string(),
        (None, _) => "Folders up to date".to_string(),
    };
    match (&result.progress, result.partial) {
        // Under way: how far it has got.
        (Some(progress), true) => s.sync_status = graph_down::progress_words(&s.mailbox, account, progress),
        (_, true) => {}
        (_, false) => {
            let t = s.local_now();
            // What the server refused in this run, said once, ahead of anything else.
            let refused = std::mem::take(&mut s.graph_said);
            let said = if !refused.is_empty() {
                refused.join(" ")
            } else if result.stopped && errors.is_empty() {
                "Stopped; carries on from here next time".to_string()
            } else {
                summary.clone()
            };
            // Messages that were moved and are not found again can't be written to: said, as they are not changes that wait.
            let said = match unplaced(&s.mailbox, account) {
                0 => said,
                1 => then(said, "1 moved message can't be found on the server yet, so changes to it wait."),
                n => then(said, &format!("{n} moved messages can't be found on the server yet, so changes to them wait.")),
            };
            s.sync_status = format!("{said} — {}", t.time_short(s.prefs.twelve_hour));
        }
    }
    Ok(json!({
        "pushed": taken.pushed,
        "added": taken.added,
        "updated": taken.updated,
        "removed": taken.removed,
        "errors": errors,
        "busyFor": result.busy_for,
        "complete": result.listing.as_ref().map(|l| l.complete),
        "new": down.new,
        "changed": down.changed,
        "gone": down.gone,
        "removedMessages": down.removed,
        "summary": summary,
        "authRefused": result.auth_refused,
    }))
}

/// Take in the emails a run read by their ids alone (`JmapMailRequest::emails`: a message a hosting page opens that
/// no window has brought yet), after the folder listing that run brought when it has one (an account whose folders
/// aren't known yet): each email into the folder whose window lists it, as a window that is not the whole of its
/// mailbox takes an email in, and only one this mailbox hasn't got: one that a Send/Receive brought meanwhile has
/// what that run read, which is no older than this. Nothing else of the result is read: it is no Send/Receive, so the
/// account's progress and status are left as they are, and nothing is taken for gone. Not an undo step.
///
/// The pages it could not take in, because their folder isn't here (yet: a Send/Receive lists it): the caller may
/// hand them in again once it is.
pub fn apply_jmap_opened(s: &mut Session, result: &JmapMailResult) -> Result<Vec<MessagePage>, CmdError> {
    let account = result.account;
    let a = s.mailbox.account(account).ok_or_else(|| fail(format!("no account with id {account}")))?;
    if a.kind != AccountKind::Jmap {
        return Err(fail(format!("{} isn't kept in step with a JMAP server", a.name)));
    }
    // (As for a run: an answer for another account would put a stranger's mail in this one.)
    if !result.email.is_empty() && !a.email.trim().eq_ignore_ascii_case(result.email.trim()) {
        return Err(fail(format!("this Send/Receive was for another account, not for {}", a.name)));
    }
    let mut taken = Taken::default();
    if let Some(listing) = &result.listing {
        take_listing(&mut s.mailbox, account, listing, &mut taken);
    }
    let mut down = Down::default();
    let mut index = std::mem::take(&mut s.graph_index);
    let mut waiting = Vec::new();
    // (A page that would be the whole of its mailbox is no email read by its id: it would take the rest for gone.)
    for page in result.pages.iter().filter(|page| !page.afresh).take(MAX_PAGES) {
        if s.mailbox.folder_by_server(account, &page.folder).is_none() {
            waiting.push(page.clone());
            continue;
        }
        let new: Vec<GraphMessage> = page.messages.iter().filter(|m| index.find(&s.mailbox, account, &m.server.id).is_none()).cloned().collect();
        if !new.is_empty() {
            jmap_down::take_window(&mut s.mailbox, &mut index, account, &MessagePage { messages: new, ..page.clone() }, &mut down);
        }
    }
    s.graph_index = index;
    if down != Down::default() || taken != Taken::default() {
        s.mailbox_changed();
    }
    // The view stays on a folder that is there (a listing may have taken one away).
    if s.mailbox.folder(s.view.folder).is_none() {
        s.view.folder = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap_or(0);
        s.sel.messages.clear();
    }
    Ok(waiting)
}

#[cfg(test)]
#[path = "jmap_mail_tests.rs"]
mod tests;
