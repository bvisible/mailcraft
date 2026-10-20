// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The sentences the crates write, as `run_words` reads them back: each row is the crates' English, the file that
//! writes it, its family, and the family each placeholder that holds a reason is read with. The French of a row is its
//! line in `fr.tsv`, found by the English as for any `tl!`; the rows are written with `run_word!`, so that a catalog
//! that lacks one fails its test. The tests hold the rows to the files they name (a census of their sentences).

use super::Family::{Inner, Line, Why};
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
];
