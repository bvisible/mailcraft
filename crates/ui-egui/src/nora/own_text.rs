// //// Neoffice — added file (no upstream equivalent): what Nora is given of the person's text and of a message.

//! What Nora is given: the person's own text in a message being written (what is above the signature
//! and the quoted original, which are not theirs to have rewritten), and for the sample mailbox, a
//! message's sender, subject and text. Both go to the server as text the server can't take for
//! markup.

use mailcraft_model::rich::Paragraph;
use mailcraft_model::{Address, RichText};

use super::{CONTEXT_MAX, MAX_OWN_TEXT};
use crate::i18n::fmt;

/// The line a plain-text original is quoted under in a reply or a forward (`mail.reply` writes it).
const QUOTE_LINE: &str = "-----Original Message-----";
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
/// paragraph that is a signature's `-- ` (a draft opened again has no marks), the line a plain-text
/// original is quoted under. An original in HTML is not in the body at all.
pub(crate) fn own_text(body: &RichText) -> OwnText {
    let paragraphs = body.paragraphs();
    let end = paragraphs.iter().position(ends_own_text).unwrap_or(paragraphs.len());
    let upto = paragraphs.iter().take(end).map(Paragraph::chars).sum::<usize>().saturating_add(end.saturating_sub(1));
    let own: String = body.text().chars().take(upto).collect();
    let own = own.trim_end();
    let chars = own.chars().count();
    let given = body.slice(0, chars).map(|part| part.to_plain()).unwrap_or_else(|_| own.to_string());
    OwnText { text: given.trim_end().to_string(), chars }
}

fn ends_own_text(p: &Paragraph) -> bool {
    if p.signature {
        return true;
    }
    let text = p.text();
    matches!(text.as_str(), "-- " | "--") || text.trim() == QUOTE_LINE
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
