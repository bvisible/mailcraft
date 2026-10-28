// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): say which kind of sender wrote messages, for the tabs of an inbox.

//! `mail.setStreams`: the streams of messages (a person, a notification, a newsletter), as whoever hosts the window says them.
//!
//! Not an edit of the owner's: it changes nothing a server keeps, makes no undo step and does not mark the mailbox as edited. The
//! hosting page calls it with what the Neoffice server answered (`apps/mailcraft-web/src/streams.rs`); an agent or a script can
//! call it to try the tabs.

use mailcraft_model::{Id, Stream};
use serde_json::{Value, json};

use super::bad;
use crate::{CmdResult, CommandSpec, Filter, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("mail.setStreams", "Set Streams", "Folder › Actions", set)
            .params(r#"{"streams": [{"id": number, "stream": "person" | "notification" | "newsletter"}]} — the kind of sender of messages, for the inbox's tabs"#)
            .pure(),
    ]
}

fn set(s: &mut Session, v: &Value) -> CmdResult {
    p::only(v, &["streams"])?;
    let list = v.get("streams").and_then(Value::as_array).ok_or_else(|| bad("`streams` is a list of {id, stream}"))?;
    let mut said: Vec<(Id, Stream)> = Vec::with_capacity(list.len().min(mailcraft_model::stream::MAX_STREAMS_PER_SAY));
    for item in list.iter().take(mailcraft_model::stream::MAX_STREAMS_PER_SAY) {
        let id = p::req_id(item, "id")?;
        let stream = Stream::from_code(p::req_str(item, "stream")?).ok_or_else(|| bad("`stream` is person, notification or newsletter"))?;
        said.push((id, stream));
    }
    let inbox = s.mailbox.inbox();
    let had = inbox.is_some_and(|inbox| s.mailbox.has_streams(inbox));
    let changed = s.mailbox.set_streams(&said);
    let tabs = inbox.is_some_and(|inbox| s.mailbox.has_streams(inbox));
    // The tabs have just appeared over the inbox in view, listing all of it: open on the main one, as Gmail does. Said again, the
    // owner's choice (All, Unread, another tab) is left as it is.
    if tabs && !had && inbox == Some(s.view.folder) && s.view.filter == Filter::All {
        s.view.filter = Filter::Main;
    }
    Ok(json!({"changed": changed, "tabs": tabs}))
}
