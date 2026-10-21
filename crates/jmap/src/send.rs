// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Sending a message and filing a draft (RFC 8620 §6.1, RFC 8621 §4.8, §6 and §7): what a run does with the
//! Outbox and with the drafts that wait to go up.
//!
//! **A message** goes in four requests, in this order, and in no other:
//!
//! 1. `Identity/get`, once for the run: who the account may send as. The identity whose address is the
//!    message's `From` is the one; a message from an address the server lists no identity for is not sent as
//!    another (it is held).
//! 2. The message is **uploaded** (§6.1 of RFC 8620) and **filed in the Sent mailbox** with `$seen`
//!    (`Email/import`). The copy in Sent is the one the owner keeps; the server sends the message from it.
//! 3. `EmailSubmission/set` makes the **submission**: the identity, the email, and an envelope that names
//!    every recipient once, To, Cc and Bcc alike. The message itself is written *without* its Bcc field, so
//!    that no server can send it on with one.
//! 4. A message that went and was written as a draft the server has: the draft's copy is destroyed (and not
//!    before: a message that does not go leaves its draft where it was).
//!
//! **What may be retried, and what may not.** Until the submission is asked for, nothing has gone, whatever
//! happens: a message whose upload or filing fails is *not sent*, and a failure of the network, the sign-in,
//! or the server's patience (and, before anything is filed, a gateway that is down) leaves it waiting for the
//! next run; a server that refuses it, says so, or fails on it at the filing holds it, so that a message the
//! server always fails on is not tried again at every run, shutting out those behind it. From the moment the
//! submission is asked for, an answer that does not say whether it went (the
//! connection lost, a status that is a server's failure, an answer that is no JMAP) is **lost**: the message
//! is held, "it may have been sent", and is never sent again by itself. So is a submission the server says
//! it failed to make (`serverFail`) or refused with a type that the standard does not give for a refusal made
//! before anything was sent: a server that fails after it handed the message on says just that. The copy in
//! Sent stays then, since it may be the only record of a message that went. A submission the server says it did
//! not make, with one of the types the standard gives for that, takes the copy out of Sent again (`Email/set`,
//! destroy), so that Sent never shows a message that was not sent. A copy that cannot be taken out, and the
//! copy of a draft that cannot be destroyed, are handed back ([`Outgoing::take_unwanted`]) for the engine to
//! see to.
//!
//! **Drafts** are filed in the Drafts mailbox with `$draft` and `$seen`, with their Bcc (a draft is for its
//! owner), and the copy an earlier save made is destroyed once the new one is in, never before. Nothing of a
//! draft goes anywhere. Trouble that is not the draft's (the network, the sign-in, a busy or unavailable server,
//! a gateway that is down) leaves it for the next run; a server that fails on this draft, or says it will not
//! have it, settles it: the owner is told once, and it is tried again when it is saved again. A draft that the
//! server always fails on would otherwise be uploaded again at every run, and a server may keep what was
//! uploaded for a while and limit how much, which would shut out the messages that wait to be sent.
//!
//! **Everything a server says is hostile.** Ids are held to [`valid_id`], answers are capped, the details a
//! server gives of a refusal are read by name and type, and what reaches the owner is words of ours (a
//! server's own text is shown only for a refusal this code has no word for, and for why it is not known whether
//! a message went, cleaned first). The
//! credentials, the addresses and the mail stay out of every line this writes, as in the rest of the crate.

use std::borrow::Cow;

use mailcraft_mime::WriteOptions;
use mailcraft_model::Message;
use mailcraft_model::sync::{JmapDraftDone, JmapDraftUp, JmapSend, SendDone, SendOutcome};
use serde_json::{Map, Value, json};

use crate::{Answers, CORE, Client, Error, Http, MAIL, Request, Result, SUBMISSION, Secrets, check_hiding, json_body, tidy, valid_id};

/// The type a message is uploaded as (RFC 5322 messages, RFC 2046 §5.2.1).
const MESSAGE_TYPE: &str = "message/rfc822";
/// The most the answer to an upload, to a filing, to a submission or to a destroy may be, in bytes: a few
/// small objects.
const MAX_ANSWER: usize = 256 << 10;
/// The most identities read (an account has a few) and the longest address and id kept of one.
const MAX_IDENTITIES: usize = 100;
const MAX_EMAIL: usize = 320;
/// The most addresses a refusal may name, kept.
const MAX_NAMED: usize = 100;
/// The longest address an envelope names (RFC 5321's longest path, less its angle brackets).
const MAX_ADDRESS: usize = 254;
/// The longest line of ours that is made of what a server said.
const MAX_SAID: usize = 160;
/// The most recipients an envelope has: past it, a server would refuse the message in any case.
const MAX_RECIPIENTS: usize = 1_000;
/// The most copies a run hands back to be destroyed.
const MAX_UNWANTED: usize = 100;
/// The longest `type` and `description` of a refusal that are kept whole: far more than a line of ours says of them,
/// so that a credential in what a server says is found and hidden (`Outgoing::said`) before it is cut.
const MAX_KIND: usize = 512;
const MAX_DETAIL: usize = 2_048;

const NO_SUBMISSION: &str = "the server doesn't send mail";
const NO_SENT_FOLDER: &str = "the server has no Sent folder to keep a copy of it in";
const NO_DRAFTS_FOLDER: &str = "the server has no Drafts folder to keep it in";
pub(crate) const UNREACHABLE: &str = "The server couldn't be reached. What waits to go will go with the next Send/Receive.";
pub(crate) const BUSY: &str = "The server is busy. What waits to go will go with a later Send/Receive.";
pub(crate) const RATE_LIMITED: &str =
    "The server limits how many messages can be sent in a short time. What waits to go will go with a later Send/Receive.";
pub(crate) const UNAVAILABLE: &str = "The server can't file mail just now. What waits to go will go with a later Send/Receive.";

// ---------------------------------------------------------------------------------------------
// The requests

/// Who the account may send as (RFC 8621 §6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub id: String,
    pub email: String,
}

/// Who a message goes from and to, as the server is told (not as the message's headers say).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Envelope {
    pub from: String,
    pub recipients: Vec<String>,
}

/// `local@domain` in visible ASCII, safe to put in an envelope.
fn is_address(email: &str) -> bool {
    let plain = email.len() <= MAX_ADDRESS && email.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'<' | b'>' | b',' | b';' | b'"'));
    plain && email.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty() && !domain.contains('@'))
}

impl Envelope {
    /// The message's `From`, and To + Cc + Bcc, each address once (compared without regard to case). Leaving
    /// someone out silently would be worse than not sending, so an address that is none is a refusal; so is a
    /// message that has no one to go to. The refusal is in plain words, for the owner.
    pub(crate) fn of(message: &Message) -> std::result::Result<Envelope, String> {
        let from = message.from.email.trim();
        if from.is_empty() {
            return Err("the message has no sender".into());
        }
        if !is_address(from) {
            return Err(format!("\"{}\" isn't an address mail can be sent from", shown(from)));
        }
        let mut recipients: Vec<String> = Vec::new();
        for address in message.recipients() {
            let email = address.email.trim();
            if email.is_empty() {
                continue;
            }
            if !is_address(email) {
                return Err(format!("\"{}\" isn't an address mail can be sent to", shown(email)));
            }
            if !recipients.iter().any(|known| known.eq_ignore_ascii_case(email)) {
                recipients.push(email.to_string());
            }
            if recipients.len() > MAX_RECIPIENTS {
                return Err("the message has too many recipients".into());
            }
        }
        if recipients.is_empty() {
            return Err("the message has no recipients".into());
        }
        Ok(Envelope { from: from.to_string(), recipients })
    }
}

/// An address, or any text of the owner's, for a line: short, on one line.
fn shown(text: &str) -> String {
    tidy(text, 80)
}

/// The HTTP status an [`Error::Bad`] was made of (it reads "HTTP 502 …"), when it says one.
fn status_of(text: &str) -> Option<u16> {
    text.strip_prefix("HTTP ")?.split(' ').next()?.parse().ok()
}

/// A message as it is uploaded, ready to go: dated now if it never was, in the zone of the machine it is
/// written on. A message that is sent has no Bcc field (`bcc: false`: the server is told of those it names in
/// the envelope), a draft keeps it (`bcc: true`).
fn octets_of(message: &Message, bcc: bool) -> Vec<u8> {
    let dated: Cow<'_, Message> =
        if message.date == 0 { Cow::Owned(Message { date: mailcraft_time::now(), ..message.clone() }) } else { Cow::Borrowed(message) };
    let offset_minutes = mailcraft_time::local_offset_minutes(dated.date);
    mailcraft_mime::write(&dated, &WriteOptions { include_bcc: bcc, offset_minutes })
}

/// Upload a message (RFC 8620 §6.1): the id of the blob the server made of it. A server that says it kept
/// another number of octets than were sent is one that cut the message.
async fn upload<H: Http>(client: &Client<H>, octets: Vec<u8>) -> Result<String> {
    let url = client.upload_url()?;
    // The address names the account (a person's address, for some servers): hidden as the session's is.
    let mut secrets = client.secrets.clone();
    secrets.add(&url);
    let sent = octets.len();
    let request = Request::post(&url, MESSAGE_TYPE, octets, MAX_ANSWER).header("Authorization", &client.auth).header("Accept", "application/json");
    let resp = client.http.send(request).await.map_err(|e| e.hidden(&secrets))?;
    if resp.body.len() > MAX_ANSWER {
        return Err(Error::TooLarge(format!("the answer is larger than {MAX_ANSWER} bytes")));
    }
    if resp.status == 413 {
        return Err(Error::TooLarge("the server takes no message as large as this".into()));
    }
    check_hiding(&resp, &secrets)?;
    let doc = json_body(&resp)?;
    let blob = doc
        .get("blobId")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or_else(|| Error::Protocol("the server's answer to an upload names no blob".into()))?;
    if doc.get("size").and_then(Value::as_u64).is_some_and(|kept| usize::try_from(kept) != Ok(sent)) {
        return Err(Error::Protocol("the server kept another number of octets than it was sent".into()));
    }
    Ok(blob.to_string())
}

/// What the server made of the one object a `/set` or `import` call was asked to create (RFC 8620 §5.3): the
/// created object, or the refusal as an [`Error::Refused`] with the `type` and `description` of its SetError.
fn created<'a>(answer: &'a Value, key: &str) -> Result<&'a Value> {
    if let Some(made) = answer.get("created").and_then(|c| c.get(key)).filter(|made| made.is_object()) {
        return Ok(made);
    }
    match answer.get("notCreated").and_then(|c| c.get(key)).and_then(Value::as_object) {
        Some(error) => Err(Error::Refused {
            kind: error.get("type").and_then(Value::as_str).map(|t| tidy(t, MAX_KIND)).unwrap_or_default(),
            description: error.get("description").and_then(Value::as_str).map(|t| tidy(t, MAX_DETAIL)).unwrap_or_default(),
        }),
        None => Err(Error::Protocol("the server's answer doesn't say what it made of the message".into())),
    }
}

/// File an uploaded message in the mailbox `mailbox` with `keywords` (`Email/import`, RFC 8621 §4.8): the id
/// of the email the server made of it.
async fn import<H: Http>(client: &Client<H>, blob: &str, mailbox: &str, keywords: &[&str]) -> Result<String> {
    let keywords: Map<String, Value> = keywords.iter().map(|k| ((*k).to_string(), Value::Bool(true))).collect();
    let args = json!({"accountId": client.account_id(), "emails": {"m": {"blobId": blob, "mailboxIds": {mailbox: true}, "keywords": keywords}}});
    let answers = client.mail_within(vec![("Email/import", args)], MAX_ANSWER).await?;
    let made = created(answers.get(0)?, "m")?;
    made.get("id")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .map(str::to_string)
        .ok_or_else(|| Error::Protocol("the server's answer to a filing names no email".into()))
}

/// Who the account may send as (`Identity/get`, RFC 8621 §6.1).
async fn identities<H: Http>(client: &Client<H>) -> Result<Vec<Identity>> {
    let args = json!({"accountId": client.account_id(), "ids": null});
    let answers = client.call_within(&[CORE, MAIL, SUBMISSION], vec![("Identity/get", args)], MAX_ANSWER).await?;
    let list = answers.get(0)?.get("list").and_then(Value::as_array).ok_or_else(|| Error::Protocol("the answer has no list of identities".into()))?;
    Ok(list
        .iter()
        .take(MAX_IDENTITIES)
        .filter_map(|identity| {
            let id = identity.get("id").and_then(Value::as_str).filter(|id| valid_id(id))?;
            let email = identity.get("email").and_then(Value::as_str).map(str::trim).filter(|e| !e.is_empty() && e.len() <= MAX_EMAIL)?;
            Some(Identity { id: id.to_string(), email: email.to_string() })
        })
        .collect())
}

/// Destroy an email (`Email/set`, RFC 8621 §4.6): whether the server no longer has it (it is gone now, or
/// was gone already).
async fn destroy<H: Http>(client: &Client<H>, id: &str) -> Result<bool> {
    let args = json!({"accountId": client.account_id(), "destroy": [id]});
    let answers = client.mail_within(vec![("Email/set", args)], MAX_ANSWER).await?;
    let answer = answers.get(0)?;
    let gone = answer.get("destroyed").and_then(Value::as_array).is_some_and(|ids| ids.iter().any(|d| d.as_str() == Some(id)));
    let was_not_there = answer.get("notDestroyed").and_then(|n| n.get(id)).and_then(|e| e.get("type")).and_then(Value::as_str) == Some("notFound");
    Ok(gone || was_not_there)
}

/// What a submission came to, as far as the answer shows.
#[derive(Debug)]
pub(crate) enum Submitted {
    /// The server made it: the message is sent.
    Made,
    /// The server says it did not make it, and why: nothing was sent.
    Refused(Refusal),
    /// The request did not get as far as sending (the server refused it as a whole, was busy, or turned the
    /// sign-in down): nothing was sent.
    NotTaken(Error),
    /// The server may have sent it, and why that is so.
    Unknown(Doubt),
}

/// Why it is not known whether a message went.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Doubt {
    /// No answer came: the connection was lost.
    NoAnswer,
    /// The server's answer to the request as a whole was a failure of the server's (this HTTP status), which may
    /// have come after the work was done.
    Failed(u16),
    /// The answer was no JMAP answer, or did not say what became of the submission.
    Unclear,
    /// The server said it failed, or said a type of refusal that the standard does not define (and that no
    /// one can take to be a refusal before anything was sent).
    Said { kind: String, description: String },
}

/// The SetError types that say a submission was refused before anything was sent (RFC 8621 §7.5, and the
/// standard ones of RFC 8620 §5.3). Any other type is a doubt: so is `serverUnavailable`, which the standard gives
/// for the error of a whole call and not for one object, and which a submission that has gone on its way could
/// as well say.
const REFUSALS: [&str; 13] = [
    "invalidEmail",
    "tooManyRecipients",
    "noRecipients",
    "invalidRecipients",
    "forbiddenMailFrom",
    "forbiddenFrom",
    "forbiddenToSend",
    "forbidden",
    "overQuota",
    "tooLarge",
    "rateLimit",
    "notFound",
    "invalidProperties",
];

/// The errors of a whole call that a server raises before it has run the call (RFC 8620 §3.6.2: it could not
/// be understood, is not allowed, or the server could not serve it just now). `serverFail`, `serverPartialFail`
/// and a type no one knows are doubts.
const NO_CHANGES: [&str; 10] = [
    "serverUnavailable",
    "unknownMethod",
    "invalidArguments",
    "invalidResultReference",
    "forbidden",
    "accountNotFound",
    "accountNotSupportedByMethod",
    "accountReadOnly",
    "requestTooLarge",
    "stateMismatch",
];

/// A submission the server did not make (RFC 8621 §7.5): the `type` of its SetError, or of the error of the
/// whole call, and what else it says.
#[derive(Debug, Default)]
pub(crate) struct Refusal {
    pub kind: String,
    pub description: String,
    /// `invalidRecipients`: the addresses it names.
    pub invalid: Vec<String>,
    /// `maxRecipients`: how many recipients it takes.
    pub most: Option<u64>,
}

/// Ask the server to send the email `email` as `identity` to the envelope's recipients
/// (`EmailSubmission/set`, RFC 8621 §7.5).
async fn submit<H: Http>(client: &Client<H>, email: &str, identity: &str, from: &str, recipients: &[String]) -> Submitted {
    let to: Vec<Value> = recipients.iter().map(|r| json!({"email": r, "parameters": null})).collect();
    let args = json!({
        "accountId": client.account_id(),
        "create": {"s": {"identityId": identity, "emailId": email, "envelope": {"mailFrom": {"email": from, "parameters": null}, "rcptTo": to}}},
    });
    let (status, outcome) = client.call_with_status(&[CORE, MAIL, SUBMISSION], vec![("EmailSubmission/set", args)], MAX_ANSWER).await;
    classify(status, outcome)
}

/// What the answer to a submission says of it, with the care a message that may have gone asks for.
///
/// - **Not sent**: the server said so with one of the types the standard gives for that (a SetError for the
///   submission, [`REFUSALS`]; or an error of the whole call that "made no changes", RFC 8620 §3.6.2,
///   [`NO_CHANGES`]), or it did not get as far as running the call (its answer is a refusal of the whole request:
///   4xx, or 503 and 429 for a server that is busy; or the connection could not be made secure).
/// - **Sent**: the server created the submission.
/// - **Unknown**: everything else, and the reason ([`Doubt`]). The network lost the answer; the server's answer
///   is a failure that may have come after the sending (5xx but 503); it says `serverFail`, or a type that is
///   none of the standard's for a refusal (a server that fails after it handed the message on says just that,
///   and the standard's "made no changes" is not a promise such a server keeps); the answer is not JMAP; it says
///   neither that it made the submission nor that it did not (or says both).
fn classify(status: Option<u16>, outcome: Result<Answers>) -> Submitted {
    let answers = match outcome {
        Ok(answers) => answers,
        Err(e @ (Error::Auth(_) | Error::Busy { .. } | Error::Tls(_))) => return Submitted::NotTaken(e),
        Err(e) => {
            // A refusal of the request as a whole that is its client's fault (4xx): the call did not run. Anything
            // else (no answer, a failure of the server's, an answer that is none) may have come after it did.
            return match (status, &e) {
                (Some(400..=499), _) => Submitted::NotTaken(e),
                (None, Error::Io(_)) => Submitted::Unknown(Doubt::NoAnswer),
                (Some(status), Error::Bad(_)) if status >= 500 => Submitted::Unknown(Doubt::Failed(status)),
                _ => Submitted::Unknown(Doubt::Unclear),
            };
        }
    };
    let answer = match answers.get(0) {
        Ok(answer) => answer,
        Err(Error::Refused { kind, description }) => {
            // The standard says these made no changes. `serverFail` and `serverPartialFail` are the failures that may
            // have come after the sending, and so is a type no one knows.
            return if NO_CHANGES.contains(&kind.as_str()) {
                Submitted::Refused(Refusal { kind, description, ..Default::default() })
            } else {
                Submitted::Unknown(Doubt::Said { kind, description })
            };
        }
        Err(_) => return Submitted::Unknown(Doubt::Unclear),
    };
    let (made, refused) = (answer.get("created").and_then(|c| c.get("s")), answer.get("notCreated").and_then(|c| c.get("s")));
    match (made.is_some_and(Value::is_object), refused.and_then(Value::as_object)) {
        (true, None) => Submitted::Made,
        (false, Some(error)) => {
            let kind = error.get("type").and_then(Value::as_str).map(|t| tidy(t, MAX_KIND)).unwrap_or_default();
            let description = error.get("description").and_then(Value::as_str).map(|t| tidy(t, MAX_DETAIL)).unwrap_or_default();
            if !REFUSALS.contains(&kind.as_str()) {
                return Submitted::Unknown(Doubt::Said { kind, description });
            }
            Submitted::Refused(Refusal {
                kind,
                description,
                invalid: error
                    .get("invalidRecipients")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .take(MAX_NAMED)
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|a| a.len() <= MAX_ADDRESS)
                    .map(str::to_string)
                    .collect(),
                most: error.get("maxRecipients").and_then(Value::as_u64),
            })
        }
        _ => Submitted::Unknown(Doubt::Unclear),
    }
}

/// The `type` of an error, as a word to put in a line for the owner: what a server said, taken through `said`
/// (the credentials hidden, the addresses too), and then only its letters and digits, as many as a type has.
fn type_words(kind: &str, said: &dyn Fn(&str) -> String) -> String {
    said(kind).chars().filter(char::is_ascii_alphanumeric).take(40).collect()
}

/// What the owner is told of a doubt, in plain words: why it is not known whether the message went. Empty when
/// the answer was lost, which is what the owner is told when nothing else is said.
fn doubt_words(doubt: &Doubt, said: &dyn Fn(&str) -> String) -> String {
    match doubt {
        Doubt::NoAnswer => String::new(),
        Doubt::Failed(status) => format!("the server failed after it was asked to send it (HTTP {status})"),
        Doubt::Unclear => "the server's answer didn't say whether it was sent".to_string(),
        Doubt::Said { kind, description } => {
            let what = if matches!(kind.as_str(), "serverFail" | "serverPartialFail") {
                "the server failed while it was sending it".to_string()
            } else {
                match type_words(kind, said) {
                    kind if kind.is_empty() => "the server answered with something this program doesn't know".to_string(),
                    kind => format!("the server answered with something this program doesn't know ({kind})"),
                }
            };
            match said(description) {
                text if text.is_empty() => what,
                text => format!("{what}: {text}"),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The words

/// What a server's refusal to file or send a message is in plain words, for the owner. `kind` is the `type` of
/// a SetError (RFC 8620 §5.3, RFC 8621 §4.8 and §7.5) or of an error of a call (RFC 8620 §3.6.2). A refusal
/// this has no word for is said with what the server said, cleaned (`said`).
fn words(refusal: &Refusal, envelope: Option<&Envelope>, said: &dyn Fn(&str) -> String) -> String {
    match refusal.kind.as_str() {
        "forbiddenFrom" | "forbiddenMailFrom" => match envelope {
            Some(envelope) => format!("the server doesn't let this account send as {}", shown(&envelope.from)),
            None => "the server doesn't let this account send as that address".to_string(),
        },
        "forbiddenToSend" | "forbidden" => "the server doesn't let this account send mail just now".to_string(),
        "noRecipients" => "the message has no recipients".to_string(),
        "invalidRecipients" => {
            // The addresses it names (a list, or, for a server that has none, the words of the refusal): only
            // those the message does name are shown, since what else a server says is not shown as an address.
            let named: Vec<&str> =
                if refusal.invalid.is_empty() { vec![refusal.description.trim()] } else { refusal.invalid.iter().map(String::as_str).collect() };
            let ours: Vec<&str> =
                envelope.map(|e| named.into_iter().filter(|a| e.recipients.iter().any(|r| r.eq_ignore_ascii_case(a))).collect()).unwrap_or_default();
            if ours.is_empty() {
                "the server doesn't accept an address the message is for".to_string()
            } else {
                format!("the server doesn't accept {}", ours.join(", "))
            }
        }
        "tooManyRecipients" => match refusal.most {
            Some(most) => format!("the message has more recipients than the server allows ({most})"),
            None => "the message has more recipients than the server allows".to_string(),
        },
        "tooLarge" => "the message is too large for the server".to_string(),
        "overQuota" => "the mailbox on the server is full".to_string(),
        "invalidEmail" => "the server found the message invalid".to_string(),
        "unknownMethod" | "accountNotSupportedByMethod" | "accountReadOnly" => NO_SUBMISSION.to_string(),
        "invalidProperties" | "blobNotFound" => "the server lost the message it was sent".to_string(),
        _ => {
            let kind = type_words(&refusal.kind, said);
            let description = said(&refusal.description);
            match (kind.is_empty(), description.is_empty()) {
                (true, true) => "the server refused it".to_string(),
                (false, true) => format!("the server refused it ({kind})"),
                (true, false) => format!("the server refused it: {description}"),
                (false, false) => format!("the server refused it ({kind}): {description}"),
            }
        }
    }
}

/// The size a message may be, in words.
fn size_words(octets: usize) -> String {
    mailcraft_model::size_label(u64::try_from(octets).unwrap_or(u64::MAX))
}

// ---------------------------------------------------------------------------------------------
// A run's sending

/// The step before a submission that failed. A gateway that fails (502, 504) is the gateway's trouble and not the
/// message's while nothing has been filed: asking who the account is changes nothing, and an upload changes nothing
/// that matters. At the filing the call may have run and left a copy, so a message that the gateway keeps failing
/// at is settled and not tried again at every run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Identity,
    Upload,
    Import,
}

/// What became of one item of a run's sending.
pub(crate) struct Went<T> {
    /// What became of the item, when that is known: not when it waits for the next run.
    pub done: Option<T>,
    /// A line for the owner, for trouble that is the run's and not the item's.
    pub line: Option<String>,
    /// Nothing more is tried in this run: the server can't be had for it just now.
    pub stop: bool,
}

impl<T> Went<T> {
    fn of(done: T) -> Went<T> {
        Went { done: Some(done), line: None, stop: false }
    }

    /// The item waits for the next run, and so does everything after it.
    fn waits(line: Option<&str>) -> Went<T> {
        Went { done: None, line: line.map(str::to_string), stop: true }
    }
}

/// What a run needs to send what waits in the Outbox and to file its drafts: the client, the mailboxes the
/// copies go to, and what is learnt once for the run (who the account may send as).
pub(crate) struct Outgoing<'a, H> {
    client: &'a Client<H>,
    /// The server ids of the Sent and the Drafts mailboxes, when the server lists them.
    sent: Option<&'a str>,
    drafts: Option<&'a str>,
    /// What the server said of who the account may send as; not asked for until a message needs it.
    identities: Option<Vec<Identity>>,
    secrets: Secrets,
    /// The server ids of copies it has that nobody wants and that could not be destroyed (see
    /// [`Outgoing::take_unwanted`]).
    unwanted: Vec<String>,
}

impl<'a, H: Http> Outgoing<'a, H> {
    pub(crate) fn new(client: &'a Client<H>, sent: Option<&'a str>, drafts: Option<&'a str>, secrets: Secrets) -> Outgoing<'a, H> {
        Outgoing { client, sent, drafts, identities: None, secrets, unwanted: Vec::new() }
    }

    /// The copies the server has that nobody wants and that this could not destroy, since the last call: a copy
    /// filed in Sent for a message that was not sent, the copy of a draft whose message has gone, the copy an
    /// earlier save of a draft made. They are handed to the engine, which sees that they are destroyed (and that
    /// they do not come back as mail in the meantime).
    pub(crate) fn take_unwanted(&mut self) -> Vec<String> {
        std::mem::take(&mut self.unwanted)
    }

    /// Destroy a copy nobody wants (`what` says which, for the log). When the server does not say it is gone,
    /// the copy is handed back. Whether the server can't be had for more just now (no answer, a sign-in it
    /// turned down, a busy server).
    async fn destroy_unwanted(&mut self, id: &str, what: &str) -> bool {
        if !valid_id(id) {
            return false;
        }
        match destroy(self.client, id).await {
            Ok(true) => false,
            outcome => {
                // (Nothing of the copy is logged.)
                log::warn!("jmap sending: {what} couldn't be destroyed on the server; the engine is told");
                if self.unwanted.len() < MAX_UNWANTED {
                    self.unwanted.push(id.to_string());
                }
                matches!(outcome, Err(Error::Io(_) | Error::Auth(_) | Error::Busy { .. } | Error::Tls(_)))
            }
        }
    }

    /// What a server said, as one line of ours: the credentials and the addresses of the session hidden, any
    /// word with an `@` too, and cut.
    fn said(&self, text: &str) -> String {
        let cut = tidy(&self.secrets.hide(text), MAX_SAID);
        cut.split(' ').map(|word| if crate::names_an_address(word) { "***" } else { word }).collect::<Vec<&str>>().join(" ")
    }

    /// Who the account may send as, asked for once.
    async fn identity_for(&mut self, from: &str) -> Result<Option<Identity>> {
        if self.identities.is_none() {
            self.identities = Some(identities(self.client).await?);
        }
        Ok(self.identities.as_deref().unwrap_or_default().iter().find(|i| i.email.eq_ignore_ascii_case(from)).cloned())
    }

    /// Why a step before the submission failed: not a word on whether the message is any good for the
    /// network, the sign-in, a busy or unavailable server, a server that fails to say who the account is, or a
    /// gateway that is down while nothing is filed (it waits for the next run, and nothing more is tried now), a
    /// refusal in plain words for the server saying it won't have it, or failing on it (the message is held: a
    /// message the server always fails on would otherwise be tried again at every run, and shut out those behind
    /// it).
    /// `Err` is a refusal to hold the message with; `Ok` is the trouble that makes it wait.
    fn trouble(&self, e: &Error, step: Step) -> std::result::Result<Option<&'static str>, String> {
        match e {
            Error::Io(_) | Error::Tls(_) => Ok(Some(UNREACHABLE)),
            Error::Auth(_) => Ok(None),
            Error::Busy { .. } => Ok(Some(BUSY)),
            Error::TooLarge(_) => Err("the message is too large for the server".to_string()),
            Error::Refused { kind, .. } if kind == "serverUnavailable" => Ok(Some(UNAVAILABLE)),
            Error::Refused { kind, description } => {
                Err(words(&Refusal { kind: kind.clone(), description: description.clone(), ..Default::default() }, None, &|t| self.said(t)))
            }
            Error::Bad(text) => match (step, status_of(text)) {
                (Step::Identity, Some(500..=599)) | (Step::Upload, Some(502 | 504)) => Ok(Some(UNAVAILABLE)),
                _ => Err(format!("the server wouldn't take it ({})", self.said(text))),
            },
            Error::Protocol(_) => Err("the server's answer wasn't understood".to_string()),
        }
    }

    /// Take a copy filed in Sent out again, for a message that was not sent. The server may fail to; the copy
    /// then stays, which is a record of a message that did not go and no more, and is handed back to be
    /// destroyed later.
    async fn take_out(&mut self, copy: &str) {
        self.destroy_unwanted(copy, "a copy filed in Sent for a message that was not sent").await;
    }

    /// Send the message `send`: see the module documentation for the steps and what may be retried.
    pub(crate) async fn send(&mut self, send: &JmapSend) -> Went<SendDone> {
        let done = |outcome: SendOutcome| SendDone { message: send.message, outcome, draft: None };
        let held = |why: String| Went::of(done(SendOutcome::Held { lost: false, why }));
        let message: &Message = &send.mime;
        // What needs no server.
        let envelope = match Envelope::of(message) {
            Ok(envelope) => envelope,
            Err(why) => return held(why),
        };
        if !self.client.has_capability(SUBMISSION) {
            return held(NO_SUBMISSION.to_string());
        }
        let Some(sent) = self.sent else { return held(NO_SENT_FOLDER.to_string()) };
        let octets = octets_of(message, false);
        if octets.len() > self.client.max_size_upload() {
            return held(format!("the message is larger than the server takes ({})", size_words(self.client.max_size_upload())));
        }
        // Who it is sent as: the identity of its From, and no other.
        let identity = match self.identity_for(&envelope.from).await {
            Ok(Some(identity)) => identity,
            Ok(None) => return held(format!("the server lists no identity to send as {}", shown(&envelope.from))),
            Err(e) => return self.before_submission(&e, Step::Identity, &done),
        };
        // Up, and into Sent. Nothing has gone yet, whatever goes wrong here.
        let blob = match upload(self.client, octets).await {
            Ok(blob) => blob,
            Err(e) => return self.before_submission(&e, Step::Upload, &done),
        };
        let copy = match import(self.client, &blob, sent, &["$seen"]).await {
            Ok(copy) => copy,
            Err(e) => return self.before_submission(&e, Step::Import, &done),
        };
        // Sent from here, if the server says so.
        match submit(self.client, &copy, &identity.id, &identity.email, &envelope.recipients).await {
            Submitted::Made => {
                self.destroy_draft(&send.draft_copy).await;
                log::info!("jmap sending: a message was sent");
                Went::of(done(SendOutcome::Sent))
            }
            Submitted::Refused(refusal) => {
                self.take_out(&copy).await;
                match refusal.kind.as_str() {
                    "rateLimit" => {
                        log::info!("jmap sending: the server limits how fast mail is sent; the rest waits");
                        Went::waits(Some(RATE_LIMITED))
                    }
                    "serverUnavailable" => Went::waits(Some(UNAVAILABLE)),
                    _ => {
                        log::info!("jmap sending: the server refused a message");
                        held(words(&refusal, Some(&envelope), &|t| self.said(t)))
                    }
                }
            }
            Submitted::NotTaken(e) => {
                self.take_out(&copy).await;
                self.before_submission(&e, Step::Import, &done)
            }
            Submitted::Unknown(doubt) => {
                // The copy stays: if the message went, it is the owner's record of it.
                log::warn!("jmap sending: it is not known whether a message went; it is held, not sent again by itself");
                let why = doubt_words(&doubt, &|t| self.said(t));
                Went { done: Some(done(SendOutcome::Held { lost: true, why })), line: None, stop: true }
            }
        }
    }

    /// What an error before the message was sent makes of it: it waits, or it is held.
    fn before_submission(&self, e: &Error, step: Step, done: &dyn Fn(SendOutcome) -> SendDone) -> Went<SendDone> {
        match self.trouble(e, step) {
            Ok(line) => Went::waits(line),
            Err(why) => Went::of(done(SendOutcome::Held { lost: false, why })),
        }
    }

    /// The message went: the copy of the draft it was written as is destroyed. If that fails, the draft stays
    /// on the server until the engine has it destroyed (it is handed back).
    async fn destroy_draft(&mut self, id: &str) {
        self.destroy_unwanted(id, "the draft a message was written as").await;
    }

    /// File the draft `draft` in Drafts, and destroy the copy an earlier save made.
    pub(crate) async fn draft(&mut self, draft: &JmapDraftUp) -> Went<JmapDraftDone> {
        let refused = |why: String| Went::of(JmapDraftDone { message: draft.message, saves: draft.saves, refused: Some(why), ..Default::default() });
        let Some(mailbox) = self.drafts else { return refused(NO_DRAFTS_FOLDER.to_string()) };
        let octets = octets_of(&draft.mime, true);
        if octets.len() > self.client.max_size_upload() {
            return refused(format!("it is larger than the server takes ({})", size_words(self.client.max_size_upload())));
        }
        let blob = match upload(self.client, octets).await {
            Ok(blob) => blob,
            Err(e) => return self.draft_failed(&e, Step::Upload, &refused),
        };
        let id = match import(self.client, &blob, mailbox, &["$draft", "$seen"]).await {
            Ok(id) => id,
            Err(e) => return self.draft_failed(&e, Step::Import, &refused),
        };
        // The new copy is in: the earlier one goes. If it can't, it is handed back to be destroyed, and a second
        // draft shows until then.
        let stop = draft.replaces != id && self.destroy_unwanted(&draft.replaces, "the earlier copy of a draft").await;
        Went {
            done: Some(JmapDraftDone { message: draft.message, saves: draft.saves, id, folder: mailbox.to_string(), refused: None }),
            line: None,
            stop,
        }
    }

    /// A draft that could not be filed. It is the owner's work and costs nothing to try again, so trouble that is
    /// not the draft's (the network, the sign-in, a busy or unavailable server, a gateway that is down while
    /// nothing is filed) leaves it for the next run. A server that fails on this draft (`serverFail`, any other
    /// failure of its own, a gateway that fails at the filing), says it will not have it, or answers with what is
    /// no JMAP settles it: said once, and tried again when the draft is saved again (what the owner is writing is
    /// saved every few minutes). Trying it at every run instead would upload it again each time, which a server
    /// may limit, and would keep the drafts behind it waiting.
    fn draft_failed(&self, e: &Error, step: Step, refused: &dyn Fn(String) -> Went<JmapDraftDone>) -> Went<JmapDraftDone> {
        match e {
            Error::Io(_) | Error::Tls(_) => Went::waits(Some(UNREACHABLE)),
            Error::Auth(_) => Went::waits(None),
            Error::Busy { .. } => Went::waits(Some(BUSY)),
            Error::Refused { kind, .. } if matches!(kind.as_str(), "serverUnavailable" | "blobNotFound") => Went::waits(Some(UNAVAILABLE)),
            Error::Bad(text) if step == Step::Upload && matches!(status_of(text), Some(502 | 504)) => Went::waits(Some(UNAVAILABLE)),
            Error::TooLarge(_) => refused("it is too large for the server".to_string()),
            // (Not a word about sending: a draft is only kept.)
            Error::Refused { kind, .. } if kind.starts_with("forbidden") => {
                refused("the server doesn't let this account keep drafts there".to_string())
            }
            Error::Refused { kind, description } => {
                refused(words(&Refusal { kind: kind.clone(), description: description.clone(), ..Default::default() }, None, &|t| self.said(t)))
            }
            Error::Bad(text) => refused(format!("the server wouldn't take it ({})", self.said(text))),
            Error::Protocol(_) => refused("the server's answer wasn't understood".to_string()),
        }
    }
}

#[cfg(test)]
mod tests;
