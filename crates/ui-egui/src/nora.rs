// //// Neoffice — added file (no upstream equivalent): Nora, the assistant of the Neoffice server, in MailCraft.

//! Nora, the assistant of the Neoffice server, in MailCraft. Reply with Nora (or Reply All) opens the
//! reply and has her write its draft; Translate shows the message being read in French, German or
//! English, with the way back to the original; and the band under the subject of a message being
//! written has her correct, improve or translate the person's own text, and learn the writing style
//! of the mailbox it goes from (My style).
//!
//! Nora lives on the server. The window reaches her only through a host that says it can
//! ([`crate::Services::nora`]): the browser build when a Neoffice page hosts it. A call names one of
//! six methods of the server ([`METHODS`], the six Neodesk's shell lets through) with its arguments,
//! and the host hands the answer back through a channel, a frame or more later. Nothing here opens a
//! connection.
//!
//! Nothing Nora writes is sent: a draft goes into the reply being written, above the signature and
//! the quoted original, and her version of the person's text goes over that text, each as one step
//! that Undo takes back; a translation is shown in place of the message's body, as plain text. What
//! comes back from the server is read as warily as anything from outside ([`answer`]).
//!
//! What leaves the window: for a message of a mailbox the hosting page handed over, its id on the
//! server and the mailbox's address, never its text (the server reads it itself, and runs its guard
//! against instructions aimed at the assistant before any model does); for the sample mailbox,
//! which the server can't read, the message's sender, subject and text; and for the band, the
//! person's own text, above the signature and the quoted original ([`own_text`]).

mod answer;
mod band;
mod flow;
pub(crate) mod look;
mod own_text;
pub(crate) mod pane;
pub(crate) mod ribbon;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod view_tests;

use std::sync::mpsc::Receiver;

use serde_json::Value;

pub(crate) use band::band;
pub(crate) use flow::{About, Kind, Nora, Rewrite, enabled, ui_command};

use crate::MailApp;

/// The server method that drafts the reply to a message of a mailbox of the server, which it reads
/// itself by the message's id (the webmail's Reply with Nora).
pub const QUICK_REPLY: &str = "nora.api.nora_webmail.quick_reply";
/// The server method that writes an email from an instruction and the email it answers (the sample
/// mailbox, which the server can't read).
pub const GENERATE_EMAIL: &str = "nora.api.nora_webmail.generate_email";
/// The server method that translates a message, or the person's text (French, German, English).
pub const TRANSLATE: &str = "nora.api.nora_webmail.translate";
/// The server methods that correct and improve the person's text.
pub const PROOFREAD: &str = "nora.api.nora_webmail.proofread";
pub const IMPROVE: &str = "nora.api.nora_webmail.improve";
/// The server method that learns a mailbox's writing style from its sent mail, in the background.
pub const SCAN_STYLE: &str = "nora.api.nora_webmail.scan_style";
/// Every method the window calls: Neodesk's shell lets these six through and no other, so a new one
/// waits for a release of Neodesk.
pub const METHODS: [&str; 6] = [QUICK_REPLY, GENERATE_EMAIL, TRANSLATE, PROOFREAD, IMPROVE, SCAN_STYLE];

/// The longest wait for an answer, in milliseconds: above the 180 s after which Neodesk's shell
/// answers by itself that Nora could not be reached (each of the server's model calls may take a
/// minute, and a reply or a translation makes up to three), below the point where the person has
/// given up.
pub(crate) const DEADLINE_MS: f64 = 200_000.0;
/// The longest own text sent to be rewritten, in characters: the server cuts at 10 000 and sends
/// the cut text back with a mark, which would then replace the whole of the person's text.
pub(crate) const MAX_OWN_TEXT: usize = 8_000;
/// Most characters of a message of the sample mailbox sent along (the server keeps 4 000 of them
/// for a reply).
pub(crate) const CONTEXT_MAX: usize = 6_000;
/// The largest answer read, in bytes; a host's own reading of the answer stops there too.
pub const MAX_ANSWER: usize = 1 << 20;
/// The longest draft, rewrite or translation taken, in characters (a text of 10 000 characters and
/// the server's limit of 4 000 tokens give less).
pub(crate) const MAX_REPLY: usize = 20_000;
/// What the server puts after a text it cut (`_truncate_text`): a rewrite that carries it is not
/// put over the person's text.
pub(crate) const TRUNCATED: &str = "[... text truncated for processing]";

/// The languages Nora translates into: the code the server takes, and the English name the
/// interface translates.
pub(crate) const LANGUAGES: [(&str, &str); 3] = [("fr", "French"), ("de", "German"), ("en", "English")];

/// One call to the Neoffice server: one of [`METHODS`] and its arguments, a plain JSON object of the
/// server method's own keys.
#[derive(Clone, Debug, PartialEq)]
pub struct NoraCall {
    pub id: u64,
    pub method: &'static str,
    pub args: Value,
}

/// The server's answer to a call: its HTTP status, 0 when the network gave none (the body then says
/// why, in a few words), and its body.
#[derive(Clone, Debug, PartialEq)]
pub struct NoraAnswer {
    pub id: u64,
    pub status: u16,
    pub body: String,
}

/// A host that reaches Nora ([`crate::Services::nora`]).
pub struct Host {
    /// Make one call. Its answer comes through the channel, and the host wakes the window when it
    /// does: a sender dropped before an answer reads as a connection that stopped, and no answer
    /// within [`DEADLINE_MS`] as one that took too long. Called for the person's own clicks only.
    pub call: Box<dyn Fn(NoraCall) -> Receiver<NoraAnswer>>,
    /// The addresses of the mailboxes the hosting page handed over (in any case): a message of one
    /// of them is named to Nora by its id on the server, and only their style can be learnt there.
    pub mailboxes: Vec<String>,
}

impl MailApp {
    /// Nora's answers that came, the reply that waits for its message, and the calls that took too
    /// long: once a frame ([`flow`]).
    pub(crate) fn poll_nora(&mut self) {
        flow::poll(self);
    }
}

/// One of [`LANGUAGES`], by its code (`fr`, `de`, `en`); `None` for anything else.
pub(crate) fn language(code: &str) -> Option<&'static str> {
    LANGUAGES.iter().find(|(c, _)| *c == code).map(|(c, _)| *c)
}

/// The name of a language Nora translates into, in the interface's language.
pub(crate) fn language_name(code: &str) -> &'static str {
    let english = LANGUAGES.iter().find(|(c, _)| *c == code).map_or("English", |(_, name)| *name);
    tl!(english)
}
