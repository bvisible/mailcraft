// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The quirks of a server that names a message by where it is.
//!
//! Some JMAP servers are a thin layer in front of a mailbox that is not a JMAP store at all (an IMAP
//! server, say). What they serve is what the layer can make of that: an id says where its message lies,
//! an email lies in one mailbox only, the folders are the mailbox's own to make or not, and the header
//! fields come raw (RFC 8621 §4.1.2.2) instead of parsed into `subject`, `from` and the rest. A client
//! that must work against such a server as well as against a store of the standard kind is tried
//! against both by setting [`Data::profile`](super::Data::profile).
//!
//! A move gives a message a new id and blob here, whether another client makes it
//! ([`Data::move_to`](super::Data::move_to)) or the client under test does, by `Email/set`; what that
//! answers of the new id is [`Profile::says_moved_id`]. Such a server takes the patches of `Email/set` of the
//! two forms a layer over a mailbox can follow (`keywords/<name>` and `mailboxIds/<id>`, each `true` or `null`),
//! and answers `{}` and `[]` for what it did nothing with. It does not look whether the message it is asked to change
//! is still there: the mailbox does not mind, and the answer is that it was made ([`Profile::unchecked_ids`]).
//!
//! **Sending** is held to what such a server does ([`Profile::strict_submission`]): the one identity of the
//! mailbox, named after its address; a message filed from an upload and sent once, within the hour it was
//! filed; a message read back before it is sent, which must be from the mailbox and from it alone; an
//! envelope that is the mailbox's own sender and recipients that are addresses with a dotted domain, at
//! most a hundred; two submissions to a call, five filings, ten uploads kept at once (and sixty megabytes of
//! them), a limit to the messages sent in a few minutes (`rateLimit`), and the flags a message can be filed
//! with. It sends nothing but what is filed, and no copy of it goes to Sent.

use base64::Engine as _;
use mailcraft_model::Address;
use serde_json::{Value, json};

use super::outgoing::{SetError, header_fields};
use super::{Data, Outcome, account, allow_only, refuse};

/// Most changes of one kind that a `Mailbox/set` is answered for.
const MAX_KEYS: usize = 10_000;

/// Most uploads kept at once, and most octets of them.
pub(super) const MAX_UPLOADS: usize = 10;
pub(super) const MAX_UPLOAD_BYTES: usize = 60 << 20;
/// Most emails an `Email/import` files, and most submissions an `EmailSubmission/set` makes, in one call.
pub(super) const MAX_IMPORTS: usize = 5;
pub(super) const MAX_SUBMISSIONS: usize = 2;
/// Most messages sent in the time such a server counts.
pub(super) const SEND_LIMIT: u32 = 30;
/// The only flags a message is filed with.
pub(super) const KEYWORDS: [&str; 5] = ["$seen", "$flagged", "$answered", "$draft", "$forwarded"];

/// What a server of one kind serves and refuses, beyond what every stand-in does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    /// `Email/get` serves the fields a store parses out of the header: `subject`, `from`, `to`, `cc`,
    /// `messageId`, `sentAt`, and `threadId`. A server that does not leaves a client the raw `headers`
    /// to read them from, and refuses (here: `invalidArguments`) a property it does not have.
    pub parsed_fields: bool,
    /// `Mailbox/set` is a method that refuses every change it is asked for (`forbidden`). Without it
    /// the method does not exist, as in every stand-in so far.
    pub fixed_mailboxes: bool,
    /// An email lies in one mailbox only (`maxMailboxesPerEmail` is 1), so [`Data::add_to_mailbox`](super::Data::add_to_mailbox)
    /// has nothing to do, and its id says where it lies, so that [`Data::move_to`](super::Data::move_to) gives
    /// it (and its blob) a new one.
    pub one_mailbox: bool,
    /// Sending is held to the rules of a server that is a layer in front of an IMAP server and an SMTP server (see
    /// the module): one identity named after the address, an email sent once and only within an hour of being
    /// filed, a message read back and checked before it is sent, a limit on how many are sent, a few at a time.
    pub strict_submission: bool,
    /// What `Email/set` makes of a patch. A server that follows the standard takes any path of the
    /// patch grammar (RFC 8620 §5.3), the whole of `keywords` or `mailboxIds` among them, and leaves an
    /// empty `updated`, `destroyed`, `notUpdated` or `notDestroyed` as `null`. A layer over a mailbox takes
    /// `keywords/<name>` and `mailboxIds/<id>` with `true` or `null` and nothing else (anything more is
    /// `invalidPatch`), and always answers `{}` and `[]`.
    pub plain_patches: bool,
    /// With [`Profile::one_mailbox`]: a move names the email's new id in the answer (`updated: {old: {"id": new}}`).
    /// Without it the answer says `null`, as a server does that does not know the id its own move gave.
    pub says_moved_id: bool,
    /// An update or a destroy of an id that names no email (a message that was moved or deleted since) is answered
    /// as if it was made, not as `notFound`: a layer over a mailbox asks the mailbox to change a message it may
    /// not have any more, and the mailbox does not mind.
    pub unchecked_ids: bool,
}

impl Profile {
    /// A store of the standard kind: parsed fields, an email in as many mailboxes as it is put in.
    pub fn standard() -> Profile {
        Profile {
            parsed_fields: true,
            fixed_mailboxes: false,
            one_mailbox: false,
            strict_submission: false,
            plain_patches: false,
            says_moved_id: false,
            unchecked_ids: false,
        }
    }

    /// A server that names a message by where it is: raw headers only, one mailbox for an email, and
    /// mailboxes that are not to be made, renamed or removed. It takes only the plain patches, does not
    /// say the new id of a message it moved, and answers a change of a message that is no more as made.
    pub fn by_place() -> Profile {
        Profile {
            parsed_fields: false,
            fixed_mailboxes: true,
            one_mailbox: true,
            strict_submission: true,
            plain_patches: true,
            says_moved_id: false,
            unchecked_ids: true,
        }
    }
}

impl Default for Profile {
    fn default() -> Profile {
        Profile::standard()
    }
}

/// `Mailbox/set` of a server whose folders are fixed: it answers (RFC 8620 §5.3) and refuses all.
pub(super) fn mailbox_set(args: &serde_json::Map<String, Value>, state: &str) -> Outcome<Value> {
    allow_only(args, &["accountId", "ifInState", "create", "update", "destroy", "onDestroyRemoveEmails"])?;
    account(args)?;
    let refused = |what: &str| -> Outcome<Value> {
        let keys: Vec<String> = match args.get(what) {
            Some(Value::Object(members)) => members.keys().take(MAX_KEYS).cloned().collect(),
            Some(Value::Array(ids)) => ids.iter().filter_map(Value::as_str).take(MAX_KEYS).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        let reason = json!({"type": "forbidden", "description": "the mailboxes of this server are not changed through JMAP"});
        Ok(Value::Object(keys.into_iter().map(|k| (k, reason.clone())).collect()))
    };
    if !matches!(args.get("create"), None | Some(Value::Null) | Some(Value::Object(_)))
        || !matches!(args.get("update"), None | Some(Value::Null) | Some(Value::Object(_)))
        || !matches!(args.get("destroy"), None | Some(Value::Null) | Some(Value::Array(_)))
    {
        return Err(refuse("invalidArguments", "create and update are objects, destroy is a list"));
    }
    Ok(json!({
        "accountId": super::ACCOUNT,
        "oldState": state,
        "newState": state,
        "notCreated": refused("create")?,
        "notUpdated": refused("update")?,
        "notDestroyed": refused("destroy")?,
    }))
}

/// A submission such a server makes, in the order it checks it: the identity and the sender are the mailbox's
/// own (both are `forbiddenFrom`), the recipients are addresses (at most a hundred), the email is one just filed
/// here, it has not sent too many lately, and the message read back is from the mailbox and from it alone.
pub(super) fn submit(data: &mut Data, spec: &serde_json::Map<String, Value>) -> std::result::Result<Value, SetError> {
    let own = data.identities.first().map(|identity| identity.email.clone()).unwrap_or_default();
    if spec.get("identityId").and_then(Value::as_str) != Some(identity_id(&own).as_str()) {
        return Err(SetError::new("forbiddenFrom", "This mailbox can only send as itself."));
    }
    let envelope = spec.get("envelope").filter(|envelope| envelope.is_object());
    let sender = envelope.and_then(|e| e.get("mailFrom")).and_then(|m| m.get("email")).and_then(Value::as_str).unwrap_or_default();
    if !sender.eq_ignore_ascii_case(&own) {
        return Err(SetError::new("forbiddenFrom", "This mailbox can only send as itself."));
    }
    let mut recipients: Vec<String> = Vec::new();
    for rcpt in envelope.and_then(|e| e.get("rcptTo")).and_then(Value::as_array).into_iter().flatten() {
        let address = rcpt.get("email").and_then(Value::as_str).map(str::trim).unwrap_or_default();
        if !address_ok(address) {
            // (It names the address in the words of its refusal, as it is, and in no list.)
            return Err(SetError::new("invalidRecipients", address.chars().take(200).collect::<String>()));
        }
        recipients.push(address.to_string());
    }
    if recipients.is_empty() {
        return Err(SetError::new("noRecipients", "Add at least one recipient."));
    }
    if recipients.len() > super::outgoing::MAX_RECIPIENTS {
        return Err(SetError::new("tooManyRecipients", super::outgoing::MAX_RECIPIENTS.to_string()));
    }
    let email_id = spec.get("emailId").and_then(Value::as_str).unwrap_or_default().to_string();
    if !data.filed.contains(&email_id) {
        return Err(SetError::new("invalidEmail", "Only a message just written here can be sent."));
    }
    if data.sent_lately >= SEND_LIMIT {
        return Err(SetError::new("rateLimit", "Too many messages sent in a short time; try again in a few minutes."));
    }
    let Some(email) = data.emails.iter().find(|e| e.id == email_id).cloned() else {
        return Err(SetError::new("invalidEmail", "The message to send was not found."));
    };
    check_message(&email.raw, &own).map_err(|(kind, why)| SetError::new(kind, why))?;
    // Sent: counted, and it can't be sent again.
    data.sent_lately = data.sent_lately.saturating_add(1);
    data.filed.remove(&email_id);
    Ok(data.send_out(&email, own, recipients))
}

/// The id such a server gives the identity of the mailbox at `email`.
pub(super) fn identity_id(email: &str) -> String {
    format!("I{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(email))
}

/// Whether `text` is an address as such a server reads one: one `@`, and on each side something that has no
/// blank, no bracket, no comma, no colon and no quote, the domain with a dot in it that has something on
/// both sides.
pub(super) fn address_ok(text: &str) -> bool {
    let side = |part: &str| !part.is_empty() && !part.chars().any(|c| c.is_whitespace() || "<>()[],;:\"@".contains(c));
    text.split_once('@').is_some_and(|(local, domain)| {
        side(local) && side(domain) && domain.find('.').is_some_and(|dot| dot > 0 && dot + 1 < domain.len()) && !domain.ends_with('.')
    })
}

/// What such a server finds when it reads the message back before sending it: exactly one `From`, with one
/// address, the mailbox's own, and a `Sender` that is absent or the same.
pub(super) fn check_message(raw: &[u8], own: &str) -> std::result::Result<(), (&'static str, String)> {
    let mine = |value: &String| {
        let list = Address::parse_list(value);
        list.len() == 1 && list.first().is_some_and(|a| a.email.eq_ignore_ascii_case(own))
    };
    let froms = header_fields(raw, "from");
    if froms.len() != 1 || !froms.iter().all(mine) || !header_fields(raw, "sender").iter().all(mine) {
        return Err(("forbiddenFrom", "The message must be from this mailbox.".to_string()));
    }
    Ok(())
}
