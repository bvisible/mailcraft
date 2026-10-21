// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The sentences the crates write, as `run_words` reads them back: each row is the crates' English, the file that
//! writes it, its family, and the family each placeholder that holds a reason is read with. The French of a row is its
//! line in `fr.tsv`, found by the English as for any `tl!`; the rows are written with `run_word!`, so that a catalog
//! that lacks one fails its test. The tests hold the rows to the files they name (a census of their sentences).

use super::Family::{Banner, Cause, Doing, Doubt, Hold, Inner, Line, Parts, Server, Tail, Why};
use super::{Family, Row};

/// A row: the file that writes the sentence, its family, the sentence, and the family each reason in it is read with.
const fn row(at: &'static str, family: Family, english: &'static str, reasons: &'static [(&'static str, Family)]) -> Row {
    Row { at, family, english, reasons }
}

// The files that write the sentences, from the workspace's root.
const JMAP: &str = "crates/jmap/src/lib.rs";
const HTTP: &str = "crates/jmap/src/http.rs";
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
const BROWSER_RULES: &str = "apps/mailcraft-web/src/fetch_rules.rs";
const CONTACTS: &str = "crates/engine/src/cmd/remote_contacts/written.rs";
const MEETING: &str = "crates/engine/src/cmd/calendar/meeting.rs";
const REMOTE: &str = "crates/engine/src/cmd/remote.rs";
const NOTES: &str = "crates/engine/src/cmd/notes/words.rs";
const NOTES_JOB: &str = "crates/jmap/src/pim/notes.rs";
const PIM: &str = "crates/jmap/src/pim.rs";
const CALENDARS: &str = "crates/jmap/src/pim/calendars.rs";
const CARDS: &str = "crates/jmap/src/pim/contacts.rs";
const EVENTS_UP: &str = "crates/jmap/src/pim/push.rs";
const CARDS_UP: &str = "crates/jmap/src/pim/contacts/push.rs";
const WRITE: &str = "crates/jmap/src/pim/write.rs";
const BOOKS: &str = "crates/engine/src/cmd/remote_contacts.rs";
const CALENDAR: &str = "crates/engine/src/cmd/calendar.rs";
const KEPT_HERE: &str = "crates/engine/src/cmd/kept_here.rs";
const COMMANDS: &str = "crates/engine/src/lib.rs";

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
    row(HTTP, Why, run_word!("network: {text}"), &[("text", Inner)]),
    row(HTTP, Why, run_word!("secure connection: {text}"), &[("text", Inner)]),
    row(HTTP, Why, run_word!("unexpected server response: {text}"), &[("text", Inner)]),
    row(HTTP, Why, run_word!("the server refused: {said}"), &[]),
    row(HTTP, Why, run_word!("the server rejected the request: {said}"), &[]),
    row(HTTP, Why, run_word!("the server is busy (HTTP {status})"), &[]),
    row(HTTP, Why, run_word!("too large: {text}"), &[("text", Inner)]),
    // The browser's transport (`fetch.rs`, `fetch_rules.rs`); what the browser itself says stays as it came.
    row(BROWSER_RULES, Inner, run_word!("timed out"), &[]),
    row(BROWSER_RULES, Inner, run_word!("the request failed"), &[]),
    row(BROWSER_RULES, Inner, run_word!("the response body is over {max} bytes"), &[]),
    row(BROWSER, Inner, run_word!("no window to fetch from"), &[]),
    row(BROWSER, Inner, run_word!("the response body cannot be read"), &[]),
    row(BROWSER, Inner, run_word!("the response body is not bytes"), &[]),
    row(BROWSER, Inner, run_word!("the browser refused a request header"), &[]),
    row(BROWSER, Inner, run_word!("the browser gave no response"), &[]),
    row(BROWSER, Inner, run_word!("the browser could not build the request"), &[]),
    row(BROWSER, Inner, run_word!("the browser cannot limit the request's time"), &[]),
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
    // An address that is none (`jmap::url`).
    row(URL, Inner, run_word!("the address does not start with http:// or https://"), &[]),
    row(URL, Inner, run_word!("the address cannot be split"), &[]),
    row(URL, Inner, run_word!("the address holds user information"), &[]),
    row(URL, Inner, run_word!("the link leads to a scheme other than http or https"), &[]),
    row(URL, Inner, run_word!("the address is too long"), &[]),
    row(URL, Inner, run_word!("the address holds a space or a control character"), &[]),
    row(URL, Inner, run_word!("the address holds a backslash"), &[]),
    row(URL, Inner, run_word!("the IPv6 address is not closed"), &[]),
    row(URL, Inner, run_word!("unexpected characters after the host"), &[]),
    row(URL, Inner, run_word!("the address has no host"), &[]),
    row(URL, Inner, run_word!("the host is too long"), &[]),
    row(URL, Inner, run_word!("the IPv6 address holds characters it cannot"), &[]),
    row(URL, Inner, run_word!("the IPv6 address is not valid"), &[]),
    row(URL, Inner, run_word!("the host holds characters a host cannot"), &[]),
    row(URL, Inner, run_word!("the IPv4 address is not four decimal numbers"), &[]),
    row(URL, Inner, run_word!("the port is not valid"), &[]),
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
    // A new contact, a change or a deletion the server has no room for (`remote_contacts::written`): it waits, and the
    // owner is told once.
    row(CONTACTS, Line, run_word!("The server's mailbox is full: {name} waits here and goes as soon as there is room."), &[]),
    row(CONTACTS, Line, run_word!("The server's mailbox is full: the change to {name} waits here and goes as soon as there is room."), &[]),
    row(CONTACTS, Line, run_word!("The server's mailbox is full: deleting {name} waits here and goes as soon as there is room."), &[]),
    // An online meeting in a JMAP account's calendar: a command's refusals (`calendar::meeting`), and what became of one
    // its server was asked to make (`remote::settle_online`, round why: the server's words).
    row(MEETING, Line, run_word!("This calendar's account can't make online meetings."), &[]),
    row(MEETING, Line, run_word!("The account's server has made this an online meeting, and it stays one: that can't be taken back from here."), &[]),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} couldn't be made an online meeting: the account's server wouldn't have it ({why}). The meeting itself is saved, without a link to join."
        ),
        &[("why", Why)],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} couldn't be made an online meeting: the account's server wouldn't have it. The meeting itself is saved, without a link to join."
        ),
        &[],
    ),
    // (Its reasons said in words of their own: `remote::unmade_words`.)
    row(
        REMOTE,
        Line,
        run_word!("{name}: an online meeting is the whole series', so it was not made for that day alone. Turn it on for the whole series."),
        &[],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} couldn't be made an online meeting: its account's server no longer makes them. The meeting itself is saved, without a link to join."
        ),
        &[],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} couldn't be made an online meeting: the account's server can't make more meeting rooms just now. The meeting itself is saved, without a link to join; turn it on again later."
        ),
        &[],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} couldn't be made an online meeting: the account's server doesn't let you make meeting rooms. The meeting itself is saved, without a link to join."
        ),
        &[],
    ),
    // Why a note's command is refused (`notes/words.rs`).
    row(NOTES, Line, run_word!("Notes aren't kept here: this page's server offers none."), &[]),
    row(NOTES, Line, run_word!("Your rights do not let you change this note"), &[]),
    row(NOTES, Line, run_word!("Only its owner deletes a note: this one was shared with you"), &[]),
    row(NOTES, Line, run_word!("This note's text is changed where it was written: open it on the web"), &[]),
    row(NOTES, Line, run_word!("A note holds {max} characters at most"), &[]),
    row(NOTES, Line, run_word!("A note's title holds {max} characters at most"), &[]),
    row(NOTES, Line, run_word!("This note has no web address"), &[]),
    row(NOTES, Line, run_word!("Too many note changes are waiting to be sent. Let them get through first."), &[]),
    // What became of a change of a note a server was told (`notes/server.rs`, in the words of `notes/words.rs`).
    row(NOTES, Line, run_word!("{title} was changed elsewhere meanwhile: your version is kept as a note of its own, {copy}."), &[]),
    row(NOTES, Line, run_word!("{title} was deleted elsewhere meanwhile: your version is kept as a note of its own."), &[]),
    row(NOTES, Line, run_word!("{title} is no longer shared with you: your version is kept as a note of its own."), &[]),
    row(NOTES, Line, run_word!("{title} couldn't be saved on the server: {said}"), &[]),
    row(NOTES, Line, run_word!("The change to {title} was put back as the server has it: {said}"), &[]),
    row(NOTES, Line, run_word!("The server wouldn't take the change to {title}: {said}. Your version is kept as a note of its own, {copy}."), &[]),
    row(
        NOTES,
        Line,
        run_word!("{title} can no longer be changed here ({why}): your version is kept as a note of its own, {copy}."),
        &[("why", Line)],
    ),
    row(
        NOTES,
        Line,
        run_word!("{title} couldn't be saved on the server ({said}), and there is no room for a note of its own: your version stays here, unsent."),
        &[],
    ),
    row(NOTES, Line, run_word!("The server wouldn't delete {title}, which is back in the notes: {said}"), &[]),
    row(NOTES, Line, run_word!("{title} was already deleted on the server when its deletion was undone here: it is made there anew."), &[]),
    row(NOTES, Line, run_word!("The server no longer keeps notes here: the notes that waited to go are kept on this computer."), &[]),
    row(
        NOTES,
        Line,
        run_word!("The server no longer keeps notes here: the notes that waited to go stay on this page, unsent, until it is left."),
        &[],
    ),
    row(NOTES, Line, run_word!("The undo couldn't keep a note in step with its server: too many note changes are waiting to be sent."), &[]),
    // What a job's notes say went wrong (`jmap::pim::notes`): they name no note.
    row(NOTES_JOB, Line, run_word!("Couldn't send the note changes: {why}"), &[("why", Why)]),
    row(NOTES_JOB, Line, run_word!("Couldn't read the notes: {why}"), &[("why", Why)]),
    row(NOTES_JOB, Line, run_word!("1 note couldn't be read: {why}"), &[("why", Why)]),
    row(NOTES_JOB, Line, run_word!("{n} notes couldn't be read: {why}"), &[("why", Why)]),
    row(NOTES_JOB, Why, run_word!("the notes hold more text than one reading takes, and these are not shown"), &[]),
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
    // A JMAP account's calendars and address books: what a job says went wrong (`jmap::pim`), and why.
    row(PIM, Line, run_word!("Couldn't reach the server of the calendars and contacts: {why}"), &[("why", Why)]),
    row(CALENDARS, Line, run_word!("Couldn't list the calendars on the server: {why}"), &[("why", Why)]),
    row(CALENDARS, Line, run_word!("Couldn't bring the calendar down from the server: {why}"), &[("why", Why)]),
    row(CALENDARS, Line, run_word!("Couldn't read 1 event of the calendar: {why}"), &[("why", Why)]),
    row(CALENDARS, Line, run_word!("Couldn't read {n} events of the calendar: {why}"), &[("why", Why)]),
    row(CALENDARS, Line, run_word!("1 event from the server couldn't be read and was left out."), &[]),
    row(CALENDARS, Line, run_word!("{n} events from the server couldn't be read and were left out."), &[]),
    row(CALENDARS, Why, run_word!("the answer has no list of events"), &[]),
    row(CALENDARS, Why, run_word!("the answer has no list of calendars"), &[]),
    row(CALENDARS, Why, run_word!("the server's answer did not have the series"), &[]),
    row(CARDS, Line, run_word!("Couldn't list the address books on the server: {why}"), &[("why", Why)]),
    row(CARDS, Line, run_word!("Couldn't bring the contacts down from the server: {why}"), &[("why", Why)]),
    row(CARDS, Line, run_word!("Couldn't read 1 contact of the address books: {why}"), &[("why", Why)]),
    row(CARDS, Line, run_word!("Couldn't read {n} contacts of the address books: {why}"), &[("why", Why)]),
    row(CARDS, Why, run_word!("the answer has no list of address books"), &[]),
    row(CARDS, Why, run_word!("the answer has no list of cards"), &[]),
    // Why a server didn't take a change of an event (`jmap::pim::push`) or of a contact (`jmap::pim::contacts::push`):
    // what a line about it says in brackets, and after the window's words for a change that waits.
    row(
        WRITE,
        Why,
        run_word!("this calendar's server keeps no categories, importance or proposed times, and everyone invited there is asked for an answer"),
        &[],
    ),
    row(EVENTS_UP, Why, run_word!("{why}, and forwards no meetings"), &[("why", Why)]),
    row(EVENTS_UP, Why, run_word!("{why}: answer without another time"), &[("why", Why)]),
    row(EVENTS_UP, Why, run_word!("the server says the event is one occurrence of a series, and doesn't say of which"), &[]),
    row(EVENTS_UP, Why, run_word!("the server has too many events around that time to find the occurrence among them just now"), &[]),
    row(EVENTS_UP, Why, run_word!("the server has more than 1,000 people for this event, more than are rewritten from here"), &[]),
    row(EVENTS_UP, Why, run_word!("the event has no mark or no time to be made with"), &[]),
    row(EVENTS_UP, Why, run_word!("the change says nothing that could be sent"), &[]),
    row(EVENTS_UP, Why, run_word!("the server's answer did not have it"), &[]),
    row(EVENTS_UP, Why, run_word!("the server no longer has the event"), &[]),
    row(EVENTS_UP, Why, run_word!("the server's copy of it couldn't be read"), &[]),
    row(EVENTS_UP, Why, run_word!("the event has no time to be made with"), &[]),
    row(EVENTS_UP, Why, run_word!("the server no longer has the series"), &[]),
    row(EVENTS_UP, Why, run_word!("the series no longer has that occurrence"), &[]),
    row(EVENTS_UP, Why, run_word!("the server no longer has its series"), &[]),
    row(EVENTS_UP, Why, run_word!("you are not among the people invited to it"), &[]),
    row(EVENTS_UP, Why, run_word!("the server's answer said nothing of it"), &[]),
    row(CARDS_UP, Why, run_word!("the server has another contact under its name now"), &[]),
    row(CARDS_UP, Why, run_word!("the contact has no mark to be made with"), &[]),
    row(CARDS_UP, Why, run_word!("the contact's name on the server can't be read"), &[]),
    row(CARDS_UP, Why, run_word!("the server's answer to the look for its card can't be read"), &[]),
    row(CARDS_UP, Why, run_word!("the server no longer has the contact"), &[]),
    row(CARDS_UP, Why, run_word!("the server's card couldn't be read"), &[]),
    row(CARDS_UP, Why, run_word!("the card the server said it had is gone"), &[]),
    row(CARDS_UP, Why, run_word!("too many changes at once"), &[]),
    row(CARDS_UP, Why, run_word!("the server no longer has the card it made of it"), &[]),
    // The engine's refusals of a change of a server's contact, event or calendar (`remote_contacts`, `calendar`, `remote`),
    // and of what a window that keeps nothing of its own between visits doesn't keep (`kept_here`). A greyed control
    // says them too, by these rows' lines.
    row(BOOKS, Line, run_word!("Your rights do not let you add contacts"), &[]),
    row(BOOKS, Line, run_word!("This page keeps no contacts of its own, and its server has no address book to put new ones in"), &[]),
    row(BOOKS, Line, run_word!("The address books are still being read from the server"), &[]),
    row(BOOKS, Line, run_word!("Your rights do not let you change this contact"), &[]),
    row(BOOKS, Line, run_word!("Your rights do not let you delete this contact"), &[]),
    row(BOOKS, Line, run_word!("Its address book keeps only a contact's name, company, job title, e-mail addresses and phone numbers"), &[]),
    row(BOOKS, Line, run_word!("A contact can't be moved to another address book here"), &[]),
    row(CONTACTS, Line, run_word!("Too many contact changes are waiting to be sent to the server. Let Send/Receive get them through first."), &[]),
    row(
        CALENDAR,
        Line,
        run_word!("This calendar's server keeps no categories, importance or proposed times, and everyone invited there is asked for an answer."),
        &[],
    ),
    row(CALENDAR, Line, run_word!("This calendar can only be read"), &[]),
    row(CALENDAR, Line, run_word!("This calendar's server doesn't forward meetings."), &[]),
    row(CALENDAR, Line, run_word!("This calendar's server keeps only Free and Busy."), &[]),
    row(
        CALENDAR,
        Line,
        run_word!("This calendar's server answers an invitation as a whole: answer the whole series, or keep this answer here without sending it."),
        &[],
    ),
    row(REMOTE, Line, run_word!("Too many calendar changes are waiting to be sent to the server. Let Send/Receive get them through first."), &[]),
    row(REMOTE, Line, run_word!("The calendar \"{name}\" can only be read: its owner hasn't let this account change it."), &[]),
    row(KEPT_HERE, Line, run_word!("This page keeps no tasks: they would be gone at the next visit"), &[]),
    row(KEPT_HERE, Line, run_word!("This page keeps nothing of its own between visits: what is imported would be gone at the next one"), &[]),
    row(KEPT_HERE, Line, run_word!("This page keeps no categories or favorites: they would be gone at the next visit"), &[]),
    row(COMMANDS, Line, run_word!("not available: {why}"), &[("why", Line)]),
    // What became of a change of a contact a server was told, or that waited for it (`remote_contacts::written`).
    row(CONTACTS, Line, run_word!("{name}: its address book can't be changed any more, so the change made here was undone."), &[]),
    row(CONTACTS, Line, run_word!("{name} was not sent to the server: its address book can't be changed any more."), &[]),
    row(
        CONTACTS,
        Line,
        run_word!(
            "{name}: its address book can't be changed any more, so it was not deleted there. It comes back with the next read if the server still has it."
        ),
        &[],
    ),
    row(CONTACTS, Line, run_word!("A change to {name} was not sent: it is no longer in the contacts."), &[]),
    row(CONTACTS, Line, run_word!("A contact change that was waiting to be sent no longer matched the contacts. It was not sent."), &[]),
    row(CONTACTS, Line, run_word!("{name} could not be sent to the server, which keeps failing with it."), &[]),
    row(CONTACTS, Line, run_word!("{name} could not be sent to the server, which keeps failing with it ({why})."), &[("why", Why)]),
    row(
        CONTACTS,
        Line,
        run_word!("The change to {name} could not be sent: the server keeps failing with it. It was put back as the server has it."),
        &[],
    ),
    row(
        CONTACTS,
        Line,
        run_word!("The change to {name} could not be sent: the server keeps failing with it ({why}). It was put back as the server has it."),
        &[("why", Why)],
    ),
    row(
        CONTACTS,
        Line,
        run_word!(
            "Deleting {name} could not be sent: the server keeps failing with it. It comes back with the next read if the server still has it."
        ),
        &[],
    ),
    row(
        CONTACTS,
        Line,
        run_word!(
            "Deleting {name} could not be sent: the server keeps failing with it ({why}). It comes back with the next read if the server still has it."
        ),
        &[("why", Why)],
    ),
    row(CONTACTS, Line, run_word!("{name} was made on the server, which doesn't say what it calls it."), &[]),
    row(CONTACTS, Line, run_word!("The server wouldn't take the change to {name}. It was put back as it was."), &[]),
    row(CONTACTS, Line, run_word!("The server wouldn't take the change to {name} ({why}). It was put back as it was."), &[("why", Why)]),
    row(CONTACTS, Line, run_word!("The server wouldn't take {name}."), &[]),
    row(CONTACTS, Line, run_word!("The server wouldn't take {name} ({why})."), &[("why", Why)]),
    row(CONTACTS, Line, run_word!("{name} is no longer on the server, so it couldn't be changed there. It was removed here too."), &[]),
    row(
        CONTACTS,
        Line,
        run_word!(
            "{name} was already deleted on the server when the deletion was undone here, so it is gone here too. The server may keep a copy of what was deleted."
        ),
        &[],
    ),
    row(CONTACTS, Line, run_word!("The server wouldn't delete {name}."), &[]),
    row(CONTACTS, Line, run_word!("The server wouldn't delete {name} ({why})."), &[("why", Why)]),
    // The sentences the engine puts after such a line, read off its end.
    row(CONTACTS, Tail, run_word!("It is kept on this computer only."), &[]),
    row(CONTACTS, Tail, run_word!("It stays here until this tab is closed: copy what you need of it."), &[]),
    row(CONTACTS, Tail, run_word!("The server may have it already: then the next read brings it too, as a contact of its own."), &[]),
    row(CONTACTS, Tail, run_word!("The next read brings the server's card too."), &[]),
    row(CONTACTS, Tail, run_word!("It is back in the contacts."), &[]),
    row(REMOTE, Tail, run_word!("It is back in the calendar."), &[]),
    row(REMOTE, Tail, run_word!("It is kept on this device only."), &[]),
    // What became of a change of an event a server was told, or that waited for it (`remote`): for a JMAP account's
    // calendar and a Microsoft one's alike.
    row(REMOTE, Why, run_word!("it would have made the server write to the people invited, and nobody said to send that"), &[]),
    row(REMOTE, Line, run_word!("The change to one day of {name} was not sent: {why}. It was put back to what the server has."), &[("why", Why)]),
    row(REMOTE, Line, run_word!("The change to {name} was not sent: {why}. It was put back to what the server has."), &[("why", Why)]),
    row(REMOTE, Line, run_word!("{name} was not sent to the server: {why}."), &[("why", Why)]),
    row(
        REMOTE,
        Line,
        run_word!("Deleting {name} was not sent to the server: {why}. It is still there, and comes back into the calendar."),
        &[("why", Why)],
    ),
    row(REMOTE, Line, run_word!("{name} was not forwarded: that writes to other people, and nobody said to send it. Nobody was written to."), &[]),
    row(
        REMOTE,
        Line,
        run_word!(
            "The other time proposed for {name} was not sent: that writes to its organizer, and nobody said to send it. The next Send/Receive shows the meeting as the server has it."
        ),
        &[],
    ),
    row(REMOTE, Line, run_word!("{name} was not forwarded: it is no longer in the calendar."), &[]),
    row(REMOTE, Line, run_word!("A change to {name} was not sent: it is no longer in the calendar."), &[]),
    row(REMOTE, Line, run_word!("A calendar change that was waiting to be sent no longer matched the calendar. It was not sent."), &[]),
    row(
        REMOTE,
        Line,
        run_word!(
            "The invitations to \"{name}\" were already sent when it was taken back here. It is still on the server and comes back into the calendar: delete it there to call it off."
        ),
        &[],
    ),
    row(REMOTE, Line, run_word!("The server wouldn't take {name} into the other calendar, so it wasn't moved: it is back where it was."), &[]),
    row(
        REMOTE,
        Line,
        run_word!("The server wouldn't take {name} into the other calendar ({why}), so it wasn't moved: it is back where it was."),
        &[("why", Why)],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} couldn't be made a Teams meeting: the account's server wouldn't have it. The meeting itself is saved, without a link to join."
        ),
        &[],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} couldn't be made a Teams meeting: the account's server wouldn't have it ({why}). The meeting itself is saved, without a link to join."
        ),
        &[("why", Why)],
    ),
    row(REMOTE, Line, run_word!("{name} is no longer on the server, so it wasn't forwarded."), &[]),
    row(REMOTE, Line, run_word!("The server wouldn't forward {name}. Nobody was written to."), &[]),
    row(REMOTE, Line, run_word!("The server wouldn't forward {name} ({why}). Nobody was written to."), &[("why", Why)]),
    row(REMOTE, Line, run_word!("The server wouldn't take deleting one occurrence of {name}. It was put back as it was."), &[]),
    row(REMOTE, Line, run_word!("The server wouldn't take deleting one occurrence of {name} ({why}). It was put back as it was."), &[("why", Why)]),
    row(REMOTE, Line, run_word!("The server wouldn't take your answer to one occurrence of {name}. It was put back as it was."), &[]),
    row(
        REMOTE,
        Line,
        run_word!("The server wouldn't take your answer to one occurrence of {name} ({why}). It was put back as it was."),
        &[("why", Why)],
    ),
    row(REMOTE, Line, run_word!("The server wouldn't take the change to one occurrence of {name}. It was put back as it was."), &[]),
    row(
        REMOTE,
        Line,
        run_word!("The server wouldn't take the change to one occurrence of {name} ({why}). It was put back as it was."),
        &[("why", Why)],
    ),
    row(REMOTE, Line, run_word!("The server wouldn't take your answer to {name}. It was put back as it was."), &[]),
    row(REMOTE, Line, run_word!("The server wouldn't take your answer to {name} ({why}). It was put back as it was."), &[("why", Why)]),
    row(
        REMOTE,
        Line,
        run_word!(
            "The account's server doesn't let it be changed, once a meeting is made, whether another time may be proposed for {name}. That option is as it was."
        ),
        &[],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "The change to {name} had already reached the server, and the people invited, when it was taken back here. It is shown as the server has it."
        ),
        &[],
    ),
    row(REMOTE, Line, run_word!("{name} was already deleted on the server when the deletion was undone here; it couldn't be brought back."), &[]),
    row(
        REMOTE,
        Line,
        run_word!(
            "The server wouldn't take {name} out of the calendar it was in, so it wasn't moved: it is back there, and the copy in the other calendar was removed."
        ),
        &[],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "The server wouldn't take {name} out of the calendar it was in ({why}), so it wasn't moved: it is back there, and the copy in the other calendar was removed."
        ),
        &[("why", Why)],
    ),
    row(REMOTE, Line, run_word!("The server wouldn't take {name} out of the calendar it was in. It is in both calendars now."), &[]),
    row(
        REMOTE,
        Line,
        run_word!("The server wouldn't take {name} out of the calendar it was in ({why}). It is in both calendars now."),
        &[("why", Why)],
    ),
    // What the server shows of a change it said it took, when that is not what it was sent (`remote`'s checks), and what
    // of the event that is.
    row(
        REMOTE,
        Line,
        run_word!("One day of {name} was deleted here and {why}, but it still has it. That day is back in the calendar."),
        &[("why", Why)],
    ),
    row(REMOTE, Line, run_word!("{name} was deleted here and {why}, but it still has it. It is back in the calendar."), &[("why", Why)]),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} was moved to another calendar here, and when it was to be taken out of the one it was in {why}, but it still has it there. It wasn't moved: it is back there, and the copy in the other calendar was removed."
        ),
        &[("why", Why)],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} was moved to another calendar here, and when it was to be taken out of the one it was in {why}, but it still has it there. It is in both calendars now."
        ),
        &[("why", Why)],
    ),
    row(REMOTE, Why, run_word!("the server answered that it has no such day"), &[]),
    row(REMOTE, Why, run_word!("the server answered that it has no such item"), &[]),
    row(REMOTE, Why, run_word!("the server said it deleted it too"), &[]),
    row(
        REMOTE,
        Line,
        run_word!("The server took the change to {name} but has {parts} otherwise than it was sent. It is shown here as the server has it."),
        &[("parts", Parts)],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} was changed here and the server said it took the change, but it still has {parts} as before. It was put back to what the server has."
        ),
        &[("parts", Parts)],
    ),
    row(REMOTE, Parts, run_word!("its time"), &[]),
    row(REMOTE, Parts, run_word!("its title"), &[]),
    row(REMOTE, Parts, run_word!("how it shows"), &[]),
    row(REMOTE, Parts, run_word!("whether it is private"), &[]),
    row(REMOTE, Parts, run_word!("{one} and {two}"), &[("one", Parts), ("two", Parts)]),
    row(REMOTE, Parts, run_word!("{one}, {two} and {three}"), &[("one", Parts), ("two", Parts), ("three", Parts)]),
    row(REMOTE, Parts, run_word!("{one}, {two}, {three} and {four}"), &[("one", Parts), ("two", Parts), ("three", Parts), ("four", Parts)]),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} was moved to another calendar here and the server said it took it there, but that calendar doesn't have it. It wasn't moved: it is back where it was."
        ),
        &[],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} was made here and the server said it took it, but it is not in the server's calendar. It is kept on this computer only: it is not in your Outlook calendar."
        ),
        &[],
    ),
    row(
        REMOTE,
        Line,
        run_word!(
            "{name} was changed here and the server said it took the change, but the server's calendar doesn't have it where it is here. It was taken out here; the next Send/Receive shows it where the server has it."
        ),
        &[],
    ),
];
