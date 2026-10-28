// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A mailbox kept in step over JMAP (RFC 8620 and 8621): what a run is asked to do and what it
//! brings back.
//!
//! The counterpart of [`GraphMailRequest`](super::GraphMailRequest) and
//! [`GraphMailResult`](super::GraphMailResult), beside them rather than bent into them. Plain data,
//! so the pieces stay apart: the engine builds a [`JmapMailRequest`] and takes in a
//! [`JmapMailResult`] (both pure), and a host with a way to reach the server (`fetch` in the
//! browser, an HTTP client where there are sockets) does the talking in between, usually as a task
//! that hands the answer over in pieces.
//!
//! A JMAP account's folders and messages are known to its server by the ids the server hands out
//! ([`crate::AccountKind::server_ids`]), as a Microsoft account's are. What carries no Graph
//! meaning is used again here instead of being copied: [`FolderListing`], [`MessagePage`],
// //// Neoffice — not `SendUp`: a message to send goes as a `JmapSend` here (see the `use` below).
//! [`SendDone`], [`GraphProgress`] and, for a message fetched whole, what that brings
//! ([`GraphFetchResult`](super::GraphFetchResult)). What is asked for it is a [`JmapFetchRequest`]
//! of its own, since a host needs to know where the server is.
//!
//! A run reads a **window** of each mailbox it reads: the newest messages, [`JmapMailRequest::window`]
//! of them. A mailbox with no more messages than that is read whole, and its page says so
//! ([`MessagePage::afresh`]); a bigger one is read in part, and what is not in the window is
//! neither added nor removed by it.
//!
//! Neither type shows the server's address, the account's address or a message in debug output:
//! counts only. The secret that opens the server (a password, or a bearer token) is handed to the
//! host beside the request and is in neither.
//!
// //// Neoffice — the sending and the drafts of a JMAP account are here: this fork sends from one (`mailcraft-jmap`'s
// `send`), where upstream has them still to come.
//! **Sending** (RFC 8621 §7): what waits in the Outbox goes with the run as [`JmapSend`], each message
//! whole, and what became of each comes back as a [`SendDone`] (sent, held with a reason, or not
//! tried). **Drafts** (RFC 8621 §4.8): a draft saved here goes up as a [`JmapDraftUp`], to be filed in
//! the server's Drafts in place of the copy an earlier save made; [`JmapDraftDone`] says what the
//! server filed. A copy the server has that nobody wants and that the run could not destroy comes back in
//! [`JmapMailResult::unwanted`]. All of these read a missing field as empty, so the files and answers
//! written before them stay good.

use serde::{Deserialize, Serialize};

// //// Neoffice — a message to send goes whole as a `JmapSend` (with the copy of its draft the server has), not as
// Graph's `SendUp`: sending from a JMAP account works here (`mailcraft-jmap`'s `send`).
use super::{FolderListing, GraphProgress, MessagePage, SendDone, is_false};
use crate::{Id, Message};

/// Most messages of one mailbox that a run reads (the newest ones): what [`JmapMailRequest::window`]
/// is held to, here and by the host.
pub const MAX_JMAP_WINDOW: u32 = 500;

// //// Neoffice — how much one run sends and files (see `JmapSend`, `JmapDraftUp`).
/// Most messages one run sends, and most drafts it files: more go with the next run. A server may
/// limit how many messages a person sends in a few minutes, and a run that sends a few at a time
/// stays well under it.
pub const MAX_JMAP_SENDS: usize = 10;
pub const MAX_JMAP_DRAFTS: usize = 10;

/// What a run for a [`crate::AccountKind::Jmap`] account is asked to do: the changes made here go
/// up, what waits in the Outbox is sent, the server's mailboxes are listed if they should be, and
/// the newest messages of the mailboxes in `scope` are read.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapMailRequest {
    pub account: Id,
    /// The account's address: what the answer is checked against, and the mailbox whose requests
    /// are counted together.
    pub email: String,
    /// The address of the server's session resource ([`crate::JmapServer::session`]). It may carry
    /// the account's address, so it is never logged.
    pub session: String,
    /// The secret that goes beside this request is a bearer token. Otherwise it is the account's
    /// password, sent with the account's address as its login name ([`crate::JmapServer::bearer`]).
    #[serde(skip_serializing_if = "is_false")]
    pub bearer: bool,
    /// How many of the newest messages of a mailbox are read at most, from 1 to [`MAX_JMAP_WINDOW`].
    pub window: u32,
    /// List the server's mailboxes in this run: the first run, a run that reads every mailbox, and
    /// the run after one that brought no listing.
    #[serde(skip_serializing_if = "is_false")]
    pub list_folders: bool,
    /// Which mailboxes are read.
    pub scope: JmapScope,
    /// The messages each mailbox has here, so that the host asks for the whole of only those it
    /// hasn't got, and for the marks alone of the rest. A mailbox that isn't here has none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub known: Vec<JmapKnown>,
    /// The changes of messages made here, after which the mailboxes are read.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<JmapChange>,
    /// What waits in the Outbox, sent after the changes (RFC 8621 §7). Each message goes whole.
    // //// Neoffice — as a `JmapSend` (see the `use` above), and the drafts filed with the run.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<JmapSend>,
    /// The drafts saved here that the server doesn't have yet in their latest form: filed in its
    /// Drafts after the messages are sent (RFC 8621 §4.8).
    // //// Neoffice — (see `JmapDraftUp`)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drafts: Vec<JmapDraftUp>,
    /// Nobody is waiting for this run: it gets the share of requests background work has.
    #[serde(skip_serializing_if = "is_false")]
    pub background: bool,
    // //// Neoffice — a message a hosting page opens that no window has brought yet (a link from a notification) is read
    // by its id alone, in a run of its own (`mailcraft-jmap`'s `sync_mail`, the window's `host_menu.rs`).
    /// Emails to read by their server ids, [`MAX_JMAP_EMAILS`] at most: a run that names any lists the mailboxes and
    /// reads those emails, each into the mailbox whose window would list it, and does nothing else (no change goes up,
    /// nothing is sent, no window is read).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<String>,
}

// //// Neoffice — see `JmapMailRequest::emails`.
/// Most emails a run reads by their ids alone ([`JmapMailRequest::emails`]).
pub const MAX_JMAP_EMAILS: usize = 10;

/// Messages of a JMAP account to fetch whole (their raw octets, which an email's blob holds), by
/// server id, in this order: the counterpart of [`GraphFetchRequest`](super::GraphFetchRequest).
///
/// It says where the server is, as a [`JmapMailRequest`] does, because a host that has to connect
/// for it has no other way to know. What it brings back is the same [`GraphFetchResult`](super::GraphFetchResult)
/// a Microsoft account's fetch brings, so that taking it in is the same for both. The secret that
/// opens the server goes beside the request, as for a run, and is not in it.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapFetchRequest {
    pub account: Id,
    /// The account's address: what the answer is checked against.
    pub email: String,
    /// The address of the server's session resource ([`crate::JmapServer::session`]). It may carry
    /// the account's address, so it is never logged.
    pub session: String,
    /// The secret that goes beside this request is a bearer token. Otherwise it is the account's
    /// password, sent with the account's address as its login name ([`crate::JmapServer::bearer`]).
    #[serde(skip_serializing_if = "is_false")]
    pub bearer: bool,
    /// The server ids of the emails to fetch, in the order they are wanted.
    pub ids: Vec<String>,
    /// Nobody is waiting: it gets the share of requests background work has.
    #[serde(skip_serializing_if = "is_false")]
    pub background: bool,
}

// //// Neoffice — a message to send, a draft to file and what became of it: sending and drafts of a JMAP account work
// here (`mailcraft-jmap`'s `send`), where upstream has none yet.
/// A message of a JMAP account's Outbox to send, whole (RFC 8621 §7): the host files it in the
/// server's Sent and has the server send it to everyone it names.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapSend {
    /// The message in the Outbox.
    pub message: Id,
    /// The server id of the copy of the draft this message was written as, when the server has one:
    /// destroyed once the message has gone, and not before. Empty: there is none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub draft_copy: String,
    /// The message as it goes out, with its attachments. Its Bcc is for the envelope alone: the host
    /// writes the message without that header.
    pub mime: Box<Message>,
}

/// A draft saved here, to be filed in the server's Drafts (RFC 8621 §4.8): the whole message, with its
/// Bcc. The copy an earlier save made is destroyed once the new one is in, never before.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapDraftUp {
    /// The draft here.
    pub message: Id,
    /// How many times it was saved here ([`ServerDraft::saves`](super::ServerDraft)) when this was
    /// worked out: an answer settles the draft only when it wasn't saved again meanwhile.
    pub saves: u32,
    /// The server id of the copy an earlier save made, when the server has one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub replaces: String,
    /// The draft as it is saved, with its attachments and its Bcc.
    pub mime: Box<Message>,
}

/// What became of a [`JmapDraftUp`]. A draft that got no answer at all isn't here: it goes up again
/// with the next run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapDraftDone {
    pub message: Id,
    /// Its [`JmapDraftUp::saves`].
    pub saves: u32,
    /// The server id of the copy just filed; empty when the server filed none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The server id of the mailbox it is in (the server's Drafts).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub folder: String,
    /// The server won't have it, and it isn't a matter of trying again: why, in plain words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

/// Which mailboxes a run reads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JmapScope {
    /// The Inbox and the mailboxes named (the server ids of those on screen): the cheap run that
    /// keeps what the owner is looking at fresh.
    Light {
        #[serde(default)]
        also: Vec<String>,
    },
    /// Every mailbox of the listing.
    #[default]
    All,
}

impl JmapScope {
    /// A word for it that names no mailbox: `all`, or `light` and how many are named besides the
    /// Inbox.
    fn describe(&self) -> String {
        match self {
            JmapScope::All => "all".into(),
            JmapScope::Light { also } => format!("light+{}", also.len()),
        }
    }
}

/// The messages one mailbox has here, as the server's ids for them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapKnown {
    /// The mailbox's server id.
    pub mailbox: String,
    /// The server ids of its messages that are here, newest first. The engine sends the newest
    /// ones, and no more than a host can use.
    pub ids: Vec<String>,
}

/// A change made here to a message of a JMAP account, for its server to take on: worked out from
/// the difference between the message and what the server last said of it
/// ([`ServerMessage`](super::ServerMessage)), or waiting in [`crate::Mailbox::queued`]. Messages
/// are named by their server ids, mailboxes by theirs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JmapChange {
    /// The marks of the message `message` (the server's `id`) were changed here: only those are
    /// named, and they go as one change. `read` is `$seen`, `flagged` is `$flagged`, `answered`
    /// is `$answered` and `forwarded` is `$forwarded`; other keywords are never touched.
    #[serde(rename_all = "camelCase")]
    Keywords {
        message: Id,
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flagged: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answered: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        forwarded: Option<bool>,
    },
    /// The message lies in another mailbox here: `to` instead of `from`. Some servers give a
    // //// Neoffice — a server that gives a moved message a new id may not say it: this fork finds the message again
    // (`cmd/jmap_down.rs`), so the change says so.
    /// message a new id when it moves ([`JmapOutcome::Done`] says it), and may not say which: then
    /// the message has none until a window of the mailbox it was moved to shows it.
    Move { message: Id, id: String, from: String, to: String },
    /// The message was deleted for good here: the server destroys it.
    Destroy { id: String },
}

impl JmapChange {
    /// The server id of the message it is about.
    pub fn id(&self) -> &str {
        match self {
            JmapChange::Keywords { id, .. } | JmapChange::Move { id, .. } | JmapChange::Destroy { id } => id,
        }
    }
}

/// What the server made of a [`JmapChange`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JmapOutcome {
    // //// Neoffice — what an empty `id` means here: for a move, that the id changed and nobody said to what (the engine
    // finds the message again), where upstream reads it as "the id stays"; this fork's host says the id that stays.
    /// Done. For a move, `id` is the message's server id now: the new one when the server said it, or the one it
    /// had when the host found that the id stays (a server that does not name a message by where it lies); empty
    /// when the id has changed and nobody said to what, and the engine finds the message again in the mailbox it
    /// went to. For a change of marks and a delete there is no id to say, and it is empty.
    Done {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        id: String,
    },
    /// The server no longer has the message.
    Gone,
    /// The server won't: `why` says so in plain words.
    Refused { why: String },
}

/// A change and what became of it. A change that isn't answered (the server was busy, the
/// connection went) isn't here: it is worked out again for the next run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapChangeDone {
    pub change: JmapChange,
    pub outcome: JmapOutcome,
}

/// What a run for a JMAP account brought back, or a piece of it.
///
/// A run arrives in pieces, in the order things were done (`partial` on all but the last), each
/// whole in itself: taking the pieces in one after the other, as far as they come, leaves the
/// mailbox as far along as the run got.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapMailResult {
    pub account: Id,
    /// The address of the account the run was for: an answer is not taken in by another account
    /// that happens to have the id.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// The address of the server's session resource the run read (its request's): an answer is
    /// not taken in once the account names another server. It may carry the account's address, so
    /// it is never logged.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session: String,
    /// What became of the changes of messages sent.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<JmapChangeDone>,
    /// What became of the messages sent.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sent: Vec<SendDone>,
    // //// Neoffice — what became of the drafts filed, and the copies nobody wants (see `JmapDraftUp`).
    /// What became of the drafts filed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drafts: Vec<JmapDraftDone>,
    /// The server ids of copies the server has that nobody wants and that the run could not destroy (the
    /// draft of a message that went, the copy a new save of a draft replaced, the copy filed in Sent for a
    /// message that was not sent): the engine has them destroyed, and keeps them out of the windows until then.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unwanted: Vec<String>,
    /// Every mailbox of the server, when it could be listed at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listing: Option<FolderListing>,
    /// The windows read, in the order they came: each is taken in by itself.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<MessagePage>,
    /// Problems worth telling the owner; empty when all went well.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// The server said it was busy and to stay away this many seconds: all work for the account
    /// waits that long.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy_for: Option<u32>,
    /// More of this run comes after this piece.
    #[serde(skip_serializing_if = "is_false")]
    pub partial: bool,
    // //// Neoffice — what counts against a run being whole, as this fork's run of the changes made here goes.
    /// Nothing went wrong in the run and it was not stopped: every mailbox it meant to read was read,
    /// if in part (a change of marks, or a delete, that the server did not take does not count against it: it waits
    /// for the next run and no window says otherwise; a move that was not answered does, for it may have been
    /// made). A window that does not hold all of its mailbox marks nothing gone (only a page with
    /// [`MessagePage::afresh`] does), so what the pages left out of the windows that do is gone from
    /// those mailboxes (deleted, or moved to a mailbox this run did not read), and the engine removes it now.
    #[serde(skip_serializing_if = "is_false")]
    pub whole: bool,
    /// The owner stopped the run (Cancel) after the piece in hand: the next one goes on from there.
    #[serde(skip_serializing_if = "is_false")]
    pub stopped: bool,
    /// How far the run has got, for the status bar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<GraphProgress>,
    /// The server turned the secret down (the host could not get another that it accepts): the
    /// owner is told to sign in again, and the run is not repeated by itself.
    #[serde(skip_serializing_if = "is_false")]
    pub auth_refused: bool,
}

impl JmapMailResult {
    /// Take in the next piece of the same run, for a caller that takes a run in at once: the pages
    /// and the rest in the order they came, and what the last piece says of the end.
    pub fn absorb(&mut self, piece: JmapMailResult) {
        self.account = piece.account;
        if !piece.email.is_empty() {
            self.email = piece.email;
        }
        if !piece.session.is_empty() {
            self.session = piece.session;
        }
        self.changes.extend(piece.changes);
        self.sent.extend(piece.sent);
        // //// Neoffice — (the drafts and the copies nobody wants, likewise)
        self.drafts.extend(piece.drafts);
        self.unwanted.extend(piece.unwanted);
        if piece.listing.is_some() {
            self.listing = piece.listing;
        }
        self.pages.extend(piece.pages);
        self.errors.extend(piece.errors);
        self.busy_for = piece.busy_for.or(self.busy_for);
        (self.partial, self.whole, self.stopped) = (piece.partial, piece.whole, piece.stopped);
        if piece.progress.is_some() {
            self.progress = piece.progress;
        }
        self.auth_refused |= piece.auth_refused;
    }
}

/// Counts only: the server's address, the account's, and every id, name and word of a message stay
/// out of debug output.
impl std::fmt::Debug for JmapMailRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let known: usize = self.known.iter().map(|k| k.ids.len()).sum();
        f.debug_struct("JmapMailRequest")
            .field("account", &self.account)
            .field("bearer", &self.bearer)
            .field("window", &self.window)
            .field("list_folders", &self.list_folders)
            .field("scope", &self.scope.describe())
            .field("mailboxes", &self.known.len())
            .field("known", &known)
            .field("changes", &self.changes.len())
            .field("sends", &self.sends.len())
            // //// Neoffice — (the drafts filed with the run)
            .field("drafts", &self.drafts.len())
            .field("background", &self.background)
            // //// Neoffice — (the emails read by their ids alone)
            .field("emails", &self.emails.len())
            .finish()
    }
}

// //// Neoffice — the debug lines of a message to send and of a draft to file: counts only.
/// Counts only: a message is private, even in a debug line.
impl std::fmt::Debug for JmapSend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JmapSend")
            .field("message", &self.message)
            .field("draft_copy", &!self.draft_copy.is_empty())
            .field("recipients", &self.mime.recipients().count())
            .field("attachments", &self.mime.attachments.len())
            .finish()
    }
}

/// Counts only, as a message to send's.
impl std::fmt::Debug for JmapDraftUp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JmapDraftUp")
            .field("message", &self.message)
            .field("saves", &self.saves)
            .field("replaces", &!self.replaces.is_empty())
            .field("attachments", &self.mime.attachments.len())
            .finish()
    }
}

/// Counts only, as a run's request: no address, and no id of a message.
impl std::fmt::Debug for JmapFetchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JmapFetchRequest")
            .field("account", &self.account)
            .field("bearer", &self.bearer)
            .field("ids", &self.ids.len())
            .field("background", &self.background)
            .finish()
    }
}

/// Counts only, as the request's.
impl std::fmt::Debug for JmapMailResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let messages: usize = self.pages.iter().map(|page| page.messages.len()).sum();
        f.debug_struct("JmapMailResult")
            .field("account", &self.account)
            .field("changes", &self.changes.len())
            .field("sent", &self.sent.len())
            // //// Neoffice — (the drafts and the copies nobody wants)
            .field("drafts", &self.drafts.len())
            .field("unwanted", &self.unwanted.len())
            .field("listed", &self.listing.as_ref().map(|listing| listing.folders.len()))
            .field("pages", &self.pages.len())
            .field("messages", &messages)
            .field("errors", &self.errors.len())
            .field("busy_for", &self.busy_for)
            .field("partial", &self.partial)
            .field("whole", &self.whole)
            .field("stopped", &self.stopped)
            .field("progress", &self.progress.is_some())
            .field("auth_refused", &self.auth_refused)
            .finish()
    }
}

#[cfg(test)]
#[path = "jmap_tests.rs"]
mod tests;
