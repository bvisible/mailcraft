// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The sentences the crates write, as `run_words` reads them back: each row is the crates' English, the file that
//! writes it, its family, and the family each placeholder that holds a reason is read with. The French of a row is its
//! line in `fr.tsv`, found by the English as for any `tl!`; the rows are written with `run_word!`, so that a catalog
//! that lacks one fails its test. The tests hold the rows to the files they name (a census of their sentences).

use super::Family::{Banner, Cause, Doing, Doubt, Hold, Inner, Line, Server, Why};
use super::{Family, Row};

/// A row: the file that writes the sentence, its family, the sentence, and the family each reason in it is read with.
const fn row(at: &'static str, family: Family, english: &'static str, reasons: &'static [(&'static str, Family)]) -> Row {
    Row { at, family, english, reasons }
}

// The files that write the sentences, from the workspace's root.
const JMAP: &str = "crates/jmap/src/lib.rs";
const MAIL: &str = "crates/jmap/src/mail.rs";
const UP: &str = "crates/jmap/src/mail/up.rs";
const URL: &str = "crates/jmap/src/url.rs";
const SEND: &str = "crates/jmap/src/send.rs";
const ENGINE: &str = "crates/engine/src/cmd/jmap_mail.rs";
const SENDING: &str = "crates/engine/src/cmd/jmap_send.rs";
const CHANGES: &str = "crates/engine/src/cmd/jmap_up.rs";
const FETCH: &str = "crates/engine/src/cmd/jmap_fetch.rs";
const FOLDER: &str = "crates/engine/src/cmd/folder.rs";
const GRAPH_MAIL: &str = "crates/engine/src/cmd/graph_mail.rs";
const UNDO: &str = "crates/engine/src/undo.rs";
const MAILBOX: &str = "crates/model/src/mailbox.rs";
const SERVER: &str = "crates/model/src/jmap_server.rs";
const BROWSER: &str = "apps/mailcraft-web/src/fetch.rs";

/// Every row, family by family.
pub(super) static ROWS: &[Row] = &[
    // The frames a run puts round what went wrong (`jmap::mail`'s `Outcome::lines` and `failed_run`), and the reasons it
    // gives itself.
    row(MAIL, Line, run_word!("Couldn't reach the server: {why}"), &[("why", Why)]),
    row(MAIL, Line, run_word!("Couldn't list the mailboxes on the server: {why}"), &[("why", Why)]),
    row(MAIL, Line, run_word!("Couldn't bring the mail down from the server: {why}"), &[("why", Why)]),
    row(MAIL, Line, run_word!("Couldn't read the mail in 1 mailbox: {why}"), &[("why", Why)]),
    row(MAIL, Line, run_word!("Couldn't read the mail in {n} mailboxes: {why}"), &[("why", Why)]),
    row(MAIL, Line, run_word!("Couldn't read 1 message in full: {why}"), &[("why", Why)]),
    row(MAIL, Line, run_word!("Couldn't read {n} messages in full: {why}"), &[("why", Why)]),
    row(MAIL, Line, run_word!("Couldn't send 1 change to the server: {why}"), &[("why", Why)]),
    row(MAIL, Line, run_word!("Couldn't send {n} changes to the server: {why}"), &[("why", Why)]),
    row(MAIL, Line, run_word!("The server failed {n} mailboxes in a row; the rest wait for the next run."), &[]),
    row(MAIL, Line, run_word!("1 mailbox was not read: a run reads {max} at most."), &[]),
    row(MAIL, Line, run_word!("{n} mailboxes were not read: a run reads {max} at most."), &[]),
    row(MAIL, Why, run_word!("the server's answer did not have them"), &[]),
    row(UP, Why, run_word!("the server did not take them"), &[]),
    row(UP, Why, run_word!("the server could not make them just now ({kind})"), &[]),
    // What a failure says (`jmap::Error`), and the words under it: ours, or the transport's and the server's, which
    // stay as they came.
    row(JMAP, Why, run_word!("network: {text}"), &[("text", Inner)]),
    row(JMAP, Why, run_word!("secure connection: {text}"), &[("text", Inner)]),
    row(JMAP, Why, run_word!("unexpected server response: {text}"), &[("text", Inner)]),
    row(JMAP, Why, run_word!("the server refused: {said}"), &[]),
    row(JMAP, Why, run_word!("the server rejected the request: {said}"), &[]),
    row(JMAP, Why, run_word!("the server is busy (HTTP {status})"), &[]),
    row(JMAP, Why, run_word!("too large: {text}"), &[("text", Inner)]),
    row(BROWSER, Inner, run_word!("{browser} (the server may not let this page reach it)"), &[]),
    row(BROWSER, Inner, run_word!("the server took too long to answer"), &[]),
    row(JMAP, Inner, run_word!("the session address isn't https"), &[]),
    row(JMAP, Inner, run_word!("the JMAP {key} isn't https"), &[]),
    row(JMAP, Inner, run_word!("the JMAP {key} isn't on the server the session is served from"), &[]),
    row(JMAP, Inner, run_word!("the session address is too long"), &[]),
    row(JMAP, Inner, run_word!("the JMAP session has no capabilities"), &[]),
    row(JMAP, Inner, run_word!("this JMAP server doesn't offer mail"), &[]),
    row(JMAP, Inner, run_word!("no mail account in the JMAP session"), &[]),
    row(JMAP, Inner, run_word!("the JMAP session has no `{key}`"), &[]),
    row(JMAP, Inner, run_word!("the JMAP {key} can't be used"), &[]),
    row(JMAP, Inner, run_word!("a JMAP address is too long"), &[]),
    row(URL, Inner, run_word!("not a web address"), &[]),
    row(JMAP, Inner, run_word!("JMAP: {parser}"), &[]),
    row(JMAP, Inner, run_word!("JMAP: no methodResponses"), &[]),
    row(JMAP, Inner, run_word!("JMAP: the answer isn't an object"), &[]),
    row(JMAP, Inner, run_word!("JMAP: no answer to {method}"), &[]),
    row(MAIL, Inner, run_word!("the answer has no list of mailboxes"), &[]),
    row(MAIL, Inner, run_word!("the answer lists no mailbox"), &[]),
    row(MAIL, Inner, run_word!("the answer has no list of ids"), &[]),
    row(MAIL, Inner, run_word!("the answer has no list of emails"), &[]),
    row(JMAP, Inner, run_word!("the answer is larger than {max} bytes"), &[]),
    row(JMAP, Inner, run_word!("the request is over the server's limit {limit}"), &[]),
    row(JMAP, Inner, run_word!("the request is over the server's limit"), &[]),
    row(UP, Inner, run_word!("the server takes fewer objects in a call"), &[]),
    // What waits to go, when the server can't take it now (`jmap::send`).
    row(SEND, Line, run_word!("The server couldn't be reached. What waits to go will go with the next Send/Receive."), &[]),
    row(SEND, Line, run_word!("The server is busy. What waits to go will go with a later Send/Receive."), &[]),
    row(
        SEND,
        Line,
        run_word!("The server limits how many messages can be sent in a short time. What waits to go will go with a later Send/Receive."),
        &[],
    ),
    row(SEND, Line, run_word!("The server can't file mail just now. What waits to go will go with a later Send/Receive."), &[]),
    // How a run ended (`apply_jmap_mail`); how far it has got is read by `chrome::progress_in_language`.
    row(ENGINE, Line, run_word!("Up to date"), &[]),
    row(ENGINE, Line, run_word!("Folders up to date"), &[]),
    row(ENGINE, Line, run_word!("Stopped; carries on from here next time"), &[]),
    row(ENGINE, Line, run_word!("The server refused the sign-in"), &[]),
    row(ENGINE, Line, run_word!("1 moved message can't be found on the server yet, so changes to it wait."), &[]),
    row(ENGINE, Line, run_word!("{n} moved messages can't be found on the server yet, so changes to them wait."), &[]),
    // Sending and filing drafts: the engine's lines (`jmap_send::send_done`, `draft_done`) and the banner of a message held in
    // the Outbox (`Message::held`), round why a message is held (`jmap::send`'s words) or why it isn't known whether it went.
    row(SENDING, Line, run_word!("Couldn't send '{subject}': {why}. It is held in the Outbox."), &[("why", Hold)]),
    row(SENDING, Line, run_word!("'{subject}' may have been sent: {doubt}. It is held in the Outbox."), &[("doubt", Doubt)]),
    row(SENDING, Line, run_word!("'{subject}' was sent, although it was taken out of the Outbox meanwhile. Look in Sent."), &[]),
    row(SENDING, Line, run_word!("'{subject}' may have been sent, although it was taken out of the Outbox meanwhile. Look in Sent."), &[]),
    row(SENDING, Line, run_word!("A message was sent, although it was taken out of the Outbox meanwhile. Look in Sent."), &[]),
    row(SENDING, Line, run_word!("A message may have been sent, although it was taken out of the Outbox meanwhile. Look in Sent."), &[]),
    row(
        SENDING,
        Line,
        run_word!("Couldn't save the draft '{subject}' on the server: {why}. It stays here, and goes up the next time it is saved."),
        &[("why", Hold)],
    ),
    row(SENDING, Banner, run_word!("It wasn't sent: {why}. Change it, or send it again."), &[("why", Hold)]),
    row(
        SENDING,
        Banner,
        run_word!(
            "It may have been sent: {doubt}. Before you send it again, ask a recipient whether it arrived; the copy in Sent was filed first, so it doesn't tell."
        ),
        &[("doubt", Doubt)],
    ),
    row(SEND, Hold, run_word!("the message has no sender"), &[]),
    row(SEND, Hold, run_word!("\"{address}\" isn't an address mail can be sent from"), &[]),
    row(SEND, Hold, run_word!("\"{address}\" isn't an address mail can be sent to"), &[]),
    row(SEND, Hold, run_word!("the message has too many recipients"), &[]),
    row(SEND, Hold, run_word!("the message has no recipients"), &[]),
    row(SEND, Hold, run_word!("the server doesn't send mail"), &[]),
    row(SEND, Hold, run_word!("the server has no Sent folder to keep a copy of it in"), &[]),
    row(SEND, Hold, run_word!("the message is larger than the server takes ({size})"), &[]),
    row(SEND, Hold, run_word!("the server lists no identity to send as {address}"), &[]),
    row(SEND, Hold, run_word!("the server doesn't let this account send as {address}"), &[]),
    row(SEND, Hold, run_word!("the server doesn't let this account send as that address"), &[]),
    row(SEND, Hold, run_word!("the server doesn't let this account send mail just now"), &[]),
    row(SEND, Hold, run_word!("the server doesn't accept {addresses}"), &[]),
    row(SEND, Hold, run_word!("the server doesn't accept an address the message is for"), &[]),
    row(SEND, Hold, run_word!("the message has more recipients than the server allows ({most})"), &[]),
    row(SEND, Hold, run_word!("the message has more recipients than the server allows"), &[]),
    row(SEND, Hold, run_word!("the message is too large for the server"), &[]),
    row(SEND, Hold, run_word!("the mailbox on the server is full"), &[]),
    row(SEND, Hold, run_word!("the server found the message invalid"), &[]),
    row(SEND, Hold, run_word!("the server lost the message it was sent"), &[]),
    row(SEND, Hold, run_word!("the server refused it"), &[]),
    row(SEND, Hold, run_word!("the server refused it ({kind})"), &[]),
    row(SEND, Hold, run_word!("the server refused it: {description}"), &[]),
    row(SEND, Hold, run_word!("the server refused it ({kind}): {description}"), &[]),
    row(SEND, Hold, run_word!("the server wouldn't take it ({said})"), &[]),
    row(SEND, Hold, run_word!("the server's answer wasn't understood"), &[]),
    row(SENDING, Hold, run_word!("the server wouldn't take it"), &[]),
    row(SEND, Hold, run_word!("the server has no Drafts folder to keep it in"), &[]),
    row(SEND, Hold, run_word!("it is larger than the server takes ({size})"), &[]),
    row(SEND, Hold, run_word!("it is too large for the server"), &[]),
    row(SEND, Hold, run_word!("the server doesn't let this account keep drafts there"), &[]),
    row(SEND, Doubt, run_word!("the server failed after it was asked to send it (HTTP {status})"), &[]),
    row(SEND, Doubt, run_word!("the server's answer didn't say whether it was sent"), &[]),
    row(SEND, Doubt, run_word!("the server failed while it was sending it"), &[]),
    row(SEND, Doubt, run_word!("the server failed while it was sending it: {description}"), &[]),
    row(SEND, Doubt, run_word!("the server answered with something this program doesn't know"), &[]),
    row(SEND, Doubt, run_word!("the server answered with something this program doesn't know: {description}"), &[]),
    row(SEND, Doubt, run_word!("the server answered with something this program doesn't know ({kind})"), &[]),
    row(SEND, Doubt, run_word!("the server answered with something this program doesn't know ({kind}): {description}"), &[]),
    row(SENDING, Doubt, run_word!("the server's answer was lost"), &[]),
    // The changes of messages the server refused (`jmap_up::Refusals::lines`): a frame, what the change was to do (one form for
    // one message and one for several, so that a language can agree its words with them) and why.
    row(CHANGES, Line, run_word!("Couldn't {doing}: the server says {why}."), &[("doing", Doing), ("why", Cause)]),
    row(CHANGES, Line, run_word!("Couldn't {doing}: the server doesn't allow it."), &[("doing", Doing)]),
    row(CHANGES, Doing, run_word!("mark '{subject}' as read"), &[]),
    row(CHANGES, Doing, run_word!("mark {n} messages as read"), &[]),
    row(CHANGES, Doing, run_word!("mark '{subject}' as unread"), &[]),
    row(CHANGES, Doing, run_word!("mark {n} messages as unread"), &[]),
    row(CHANGES, Doing, run_word!("change the flag of '{subject}'"), &[]),
    row(CHANGES, Doing, run_word!("change the flag of {n} messages"), &[]),
    row(CHANGES, Doing, run_word!("change the replied or forwarded mark of '{subject}'"), &[]),
    row(CHANGES, Doing, run_word!("change the replied or forwarded mark of {n} messages"), &[]),
    row(CHANGES, Doing, run_word!("change '{subject}'"), &[]),
    row(CHANGES, Doing, run_word!("change {n} messages"), &[]),
    row(CHANGES, Doing, run_word!("move '{subject}'"), &[]),
    row(CHANGES, Doing, run_word!("move '{subject}' to {folder}"), &[]),
    row(CHANGES, Doing, run_word!("move {n} messages"), &[]),
    row(CHANGES, Doing, run_word!("move {n} messages to {folder}"), &[]),
    row(CHANGES, Doing, run_word!("delete a message for good"), &[]),
    row(CHANGES, Doing, run_word!("delete '{subject}' for good"), &[]),
    row(CHANGES, Doing, run_word!("delete {n} messages for good"), &[]),
    row(CHANGES, Cause, run_word!("it no longer exists"), &[]),
    row(CHANGES, Cause, run_word!("they no longer exist"), &[]),
    row(CHANGES, Cause, run_word!("this account may not change it"), &[]),
    row(CHANGES, Cause, run_word!("this account may not change them"), &[]),
    row(CHANGES, Cause, run_word!("the mailbox is full"), &[]),
    row(CHANGES, Cause, run_word!("it can't make that change"), &[]),
    // What Undo couldn't take back (`undo::Said::lines`), shown in the status bar.
    row(UNDO, Line, run_word!("'{subject}' was deleted for good: that can't be undone here."), &[]),
    row(UNDO, Line, run_word!("{n} messages were deleted for good: that can't be undone here."), &[]),
    row(UNDO, Line, run_word!("Sending '{subject}' can't be undone."), &[]),
    row(UNDO, Line, run_word!("Sending {n} messages can't be undone."), &[]),
    // A command's refusals for a JMAP account.
    row(
        MAILBOX,
        Line,
        run_word!(
            "Mail can't be moved between a JMAP account and another account or a local folder yet: this version can't put a message on a JMAP server, and the server's copy would stay where it is"
        ),
        &[],
    ),
    row(
        MAILBOX,
        Line,
        run_word!("Mail can't be copied to or from a JMAP account: its server keeps a message in one mailbox. Move it, or save it as a file"),
        &[],
    ),
    row(FOLDER, Line, run_word!("A JMAP account's folders are its server's: this version can't make, rename, move or delete them there"), &[]),
    // The engine's refusals of a Send/Receive or a download it can't make or take in (`jmap_mail`, `jmap_fetch`), and what is
    // wrong with the address of a JMAP server (`JmapServer::problem`).
    row(ENGINE, Line, run_word!("this Send/Receive was for another account, not for {account}"), &[]),
    row(ENGINE, Line, run_word!("no account with id {id}"), &[]),
    row(ENGINE, Line, run_word!("{account} isn't kept in step with a JMAP server"), &[]),
    row(ENGINE, Line, run_word!("{account} has no JMAP server"), &[]),
    row(ENGINE, Line, run_word!("{account}'s JMAP server can't be reached: {why}"), &[("why", Server)]),
    row(FETCH, Line, run_word!("no message with id {id} in {account}"), &[]),
    row(GRAPH_MAIL, Line, run_word!("these messages were for another account, not for {account}"), &[]),
    row(SERVER, Server, run_word!("is empty"), &[]),
    row(SERVER, Server, run_word!("is longer than 2048 characters"), &[]),
    row(SERVER, Server, run_word!("has a control character or a space in it"), &[]),
    row(SERVER, Server, run_word!("has a backslash in it"), &[]),
    row(SERVER, Server, run_word!("isn't an https:// address"), &[]),
    row(SERVER, Server, run_word!("names no server"), &[]),
    row(SERVER, Server, run_word!("carries a user name or a password: the secret is given apart from the address"), &[]),
    row(SERVER, Server, run_word!("has a port that isn't a number from 1 to 65535"), &[]),
    row(SERVER, Server, run_word!("is a plain http:// address to a server that isn't on this computer: use https://"), &[]),
    row(SERVER, Server, run_word!("names a server that isn't letters, digits, dots and dashes, or an IP address"), &[]),
];
