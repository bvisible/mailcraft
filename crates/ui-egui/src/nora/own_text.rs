// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what Nora is given of the person's text and of a message.

//! What Nora is given: the person's own text in a message being written (what is above the signature
//! and the quoted original, which are not theirs to have rewritten), and for the sample mailbox, a
//! message's sender, subject and text. Both go to the server as text the server can't take for
//! markup.

use mailcraft_model::rich::Paragraph;
use mailcraft_model::{Address, RichText};

use super::{CONTEXT_MAX, MAX_OWN_TEXT};
use crate::i18n::fmt;

/// The longest sender or subject sent along with a message, in characters (the server keeps 300).
const MAX_LINE: usize = 300;

/// The person's own text in a message being written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OwnText {
    /// As Nora is given it: a list with its bullets and numbers, a link with its address
    /// (`RichText::to_plain`), without what ends it of blank lines and spaces.
    pub text: String,
    /// How many characters of the body it takes, from the start: what her version goes over.
    pub chars: usize,
}

/// The text before the first of: the signature put in the message (its paragraphs are marked), a
/// paragraph that is a signature's `-- `, and what starts another's mail quoted under the person's:
/// the line a plain-text original is quoted under, and in a draft opened again (whose HTML MailCraft
/// didn't write, or from before it marked its quote) the ways other programs start a quote. An original
/// in HTML that MailCraft carries is not in the body at all.
pub(crate) fn own_text(body: &RichText) -> OwnText {
    let paragraphs = body.paragraphs();
    let end = own_end(paragraphs);
    let upto = paragraphs.iter().take(end).map(Paragraph::chars).sum::<usize>().saturating_add(end.saturating_sub(1));
    let own: String = body.text().chars().take(upto).collect();
    let own = own.trim_end();
    let chars = own.chars().count();
    let given = body.slice(0, chars).map(|part| part.to_plain()).unwrap_or_else(|_| own.to_string());
    OwnText { text: given.trim_end().to_string(), chars }
}

/// The first paragraph that isn't the person's own text, or the number of paragraphs.
fn own_end(paragraphs: &[Paragraph]) -> usize {
    for (i, p) in paragraphs.iter().enumerate() {
        if p.signature {
            return i;
        }
        let text = p.text();
        if starts_another_text(&text) {
            return i;
        }
        // A header block of the mail quoted: `From:`, and `Sent:` or `Date:` within the next lines
        // (MailCraft's own, Outlook's, and theirs in other languages), as the server reads it.
        if header(&text, &FROM) && paragraphs.iter().skip(i + 1).take(3).any(|next| header(&next.text(), &SENT)) {
            return i;
        }
    }
    paragraphs.len()
}

/// The names a quoted mail's sender and date lines start with, as the server strips them from what it reads.
const FROM: [&str; 5] = ["from", "de", "von", "da", "van"];
const SENT: [&str; 9] = ["sent", "date", "envoyé", "envoye", "gesendet", "datum", "inviato", "data", "verzonden"];

/// A line `<name>:` (spaces before the colon allowed) of one of `names`, in any case.
fn header(text: &str, names: &[&str]) -> bool {
    let low = text.trim().to_lowercase();
    names.iter().any(|name| low.strip_prefix(name).is_some_and(|rest| rest.trim_start().starts_with(':')))
}

/// A paragraph that starts what isn't the person's: a signature's separator, a quoted line, a rule
/// other programs draw over the mail they quote or forward, or the line that says who wrote it.
fn starts_another_text(text: &str) -> bool {
    let line = text.trim();
    let low = line.to_lowercase();
    let rule = |l: &str| {
        let dashes = l.starts_with("--") && l.ends_with("--") && l.trim_matches('-').trim().chars().any(char::is_alphanumeric);
        dashes || (l.chars().count() >= 10 && l.chars().all(|c| c == '_'))
    };
    matches!(text, "-- " | "--")
        || line.starts_with('>')
        || rule(line)
        || low.starts_with("begin forwarded message")
        || (low.starts_with("on ") && low.ends_with("wrote:"))
        || (low.starts_with("le ") && (low.ends_with("a écrit :") || low.ends_with("a écrit:")))
        || (low.starts_with("am ") && low.contains(" schrieb") && low.ends_with(':'))
}

/// The own text Nora is asked to work on, or why she isn't: there is none, or more than she takes.
pub(crate) fn to_rewrite(body: &RichText) -> Result<OwnText, String> {
    let own = own_text(body);
    if own.text.trim().is_empty() {
        return Err(tl!("Write your text first: Nora works on it.").to_string());
    }
    if own.text.chars().nth(MAX_OWN_TEXT).is_some() {
        return Err(fmt(tl!("Your text is too long for Nora: {n} characters at most."), &[("n", &MAX_OWN_TEXT.to_string())]));
    }
    Ok(own)
}

/// Plain text as the HTML the server reads: a paragraph for each run of lines between blank ones, a
/// line break for each other, and nothing it could take for markup.
pub(crate) fn as_html(text: &str) -> String {
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", p.lines().map(mailcraft_html::escape).collect::<Vec<_>>().join("<br>")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A message of the sample mailbox as the server reads it to answer it: who wrote it, its subject
/// (each on one line), and its text, [`CONTEXT_MAX`] characters at most.
pub(crate) fn reply_context(from: &Address, subject: &str, text: &str) -> String {
    let name = one_line(&from.name);
    let email = one_line(&from.email);
    let sender = if name.is_empty() { email } else { format!("{name} <{email}>") };
    let text: String = text.chars().take(CONTEXT_MAX).collect();
    format!("From: {sender}\nSubject: {}\n\n{}", one_line(subject), text.trim())
}

/// A message's text, [`CONTEXT_MAX`] characters at most, as HTML for the server to translate.
pub(crate) fn message_html(text: &str) -> String {
    as_html(&text.chars().take(CONTEXT_MAX).collect::<String>())
}

/// A header's value on one line: control characters as spaces, trimmed, [`MAX_LINE`] characters at most.
fn one_line(text: &str) -> String {
    let flat: String = text.chars().take(MAX_LINE * 2).map(|c| if c.is_control() { ' ' } else { c }).collect();
    flat.trim().chars().take(MAX_LINE).collect()
}
