// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the kind of sender of a message, for the tabs of an inbox.

//! The stream of a message: which kind of sender wrote it, for the tabs of an inbox (Principal, Notifications, Newsletters), as Gmail
//! and Spark have them.
//!
//! A person (a message written by a person to a person), a notification (a system wrote it: a receipt, an alert, the activity of an
//! application) or a newsletter (a sender wrote to many). It is **not** a category: it is not the owner's to give, it is not written
//! to the server's mailbox, and it is not kept in the mailbox file — whoever hosts the window says it (the Neoffice server's
//! `NeomailStream/get`, `apps/mailcraft-web/src/streams.rs`), and a message nobody has said anything of is in the main tab, so that
//! the tab the owner reads first never hides mail nobody was sure about.

use serde::{Deserialize, Serialize};

use crate::{FolderKind, Id, Mailbox};

/// Which kind of sender wrote a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stream {
    Person,
    Notification,
    Newsletter,
}

impl Stream {
    pub const ALL: [Stream; 3] = [Stream::Person, Stream::Notification, Stream::Newsletter];

    /// The word the server says it with.
    pub const fn code(self) -> &'static str {
        match self {
            Stream::Person => "person",
            Stream::Notification => "notification",
            Stream::Newsletter => "newsletter",
        }
    }

    /// The stream a server's word names, or `None` for any other word (a newer server may know more kinds than this window).
    pub fn from_code(code: &str) -> Option<Stream> {
        Stream::ALL.into_iter().find(|s| s.code().eq_ignore_ascii_case(code.trim()))
    }

    fn index(self) -> usize {
        match self {
            Stream::Person => 0,
            Stream::Notification => 1,
            Stream::Newsletter => 2,
        }
    }
}

/// What a folder holds of each stream: the messages, and the unread ones. A message nobody said anything of is a person's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamCounts {
    total: [usize; 3],
    unread: [usize; 3],
}

impl StreamCounts {
    pub fn total(&self, stream: Stream) -> usize {
        self.total.get(stream.index()).copied().unwrap_or(0)
    }

    pub fn unread(&self, stream: Stream) -> usize {
        self.unread.get(stream.index()).copied().unwrap_or(0)
    }
}

/// How many streams one say hands over at most (a window of a list holds 200 messages, the server's answer at most as many).
pub const MAX_STREAMS_PER_SAY: usize = 5_000;

impl Mailbox {
    /// Put the streams a server said: `(message, stream)`, at most [`MAX_STREAMS_PER_SAY`]. A message that is not here, or is a draft
    /// (a draft is nobody's tab), is left; a message already in that stream is not touched. Returns how many messages changed.
    /// Not an edit of the owner's: nothing is written to the server, and what was put is not undone by an Undo.
    pub fn set_streams(&mut self, streams: &[(Id, Stream)]) -> usize {
        let mut changed = 0;
        for &(id, stream) in streams.iter().take(MAX_STREAMS_PER_SAY) {
            let Ok(message) = self.message_mut(id) else { continue };
            if message.draft || message.stream == Some(stream) {
                continue;
            }
            message.stream = Some(stream);
            changed += 1;
        }
        changed
    }

    /// Whether the tabs are worth showing in `folder`: it is an Inbox and at least one of its messages has been given a stream.
    pub fn has_streams(&self, folder: Id) -> bool {
        self.folder(folder).is_some_and(|f| f.kind == FolderKind::Inbox) && self.messages_in(folder).any(|m| m.stream.is_some())
    }

    /// The messages and the unread ones of each stream in `folder`.
    pub fn stream_counts(&self, folder: Id) -> StreamCounts {
        let mut counts = StreamCounts::default();
        for m in self.messages_in(folder).filter(|m| !m.draft) {
            let at = m.stream_or_person().index();
            if let Some(n) = counts.total.get_mut(at) {
                *n += 1;
            }
            if !m.read
                && let Some(n) = counts.unread.get_mut(at)
            {
                *n += 1;
            }
        }
        counts
    }
}
