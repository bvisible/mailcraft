// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a rule in words, in the interface's language.

//! What a rule does, as the Rules window says it under its name: `When from contains "x": move it to Inbox/Projects, flag it`.
//!
//! Upstream says it in English alone, whatever the interface's language (`mailcraft_model::Condition::describe_as`,
//! `Action::describe`, which the engine's commands and agents read, and which stay as they are). Each part is said here from a
//! phrase of its own, which the catalogues have; in English it reads as upstream's, word for word.

use mailcraft_model::{Action, Condition, FlagDue, Id, Importance, Mailbox, Rule, Sensitivity};

use crate::i18n::fmt;

/// The rule in words, naming the folder a move or a copy goes to.
pub(crate) fn describe(mb: &Mailbox, r: &Rule) -> String {
    let addresses = mb.rule_reads_addresses(r);
    let all = |conditions: &[Condition]| conditions.iter().map(|c| condition(c, addresses)).collect::<Vec<_>>();
    let when = joined(&all(&r.conditions), tl!("{first} and {second}"));
    let except = joined(&all(&r.exceptions), tl!("{first} or {second}"));
    let mut then: Vec<String> = r.actions.iter().map(|a| action(mb, r, a)).collect();
    if r.alert {
        then.push(tl!("show a desktop alert").to_string());
    }
    let then = then.join(", ");
    if except.is_empty() {
        fmt(tl!("When {when}: {then}"), &[("when", &when), ("then", &then)])
    } else {
        fmt(tl!("When {when}, except if {except}: {then}"), &[("when", &when), ("except", &except), ("then", &then)])
    }
}

/// `parts` said together, two by two, as `pair` says two.
fn joined(parts: &[String], pair: &str) -> String {
    let mut all = parts.iter();
    let first = all.next().cloned().unwrap_or_default();
    all.fold(first, |first, second| fmt(pair, &[("first", &first), ("second", second)]))
}

/// The words a condition looks for, as they are typed.
fn quoted(words: &[String]) -> String {
    let each: Vec<String> = words.iter().map(|w| format!("\"{}\"", w.replace('\\', "\\\\").replace('"', "\\\""))).collect();
    joined(&each, tl!("{first} or {second}"))
}

fn condition(c: &Condition, addresses_only: bool) -> String {
    let words = |phrase: &str, words: &[String]| fmt(phrase, &[("words", &quoted(words))]);
    match c {
        Condition::FromContains(w) if addresses_only => words(tl!("sender's address contains {words}"), w),
        Condition::ToContains(w) if addresses_only => words(tl!("recipient's address contains {words}"), w),
        Condition::FromContains(w) => words(tl!("from contains {words}"), w),
        Condition::ToContains(w) => words(tl!("recipient contains {words}"), w),
        Condition::SubjectContains(w) => words(tl!("subject contains {words}"), w),
        Condition::BodyContains(w) => words(tl!("body contains {words}"), w),
        Condition::HasAttachment => tl!("has an attachment").to_string(),
        Condition::Importance(Importance::High) => tl!("importance is high").to_string(),
        Condition::Importance(Importance::Normal) => tl!("importance is normal").to_string(),
        Condition::Importance(Importance::Low) => tl!("importance is low").to_string(),
        Condition::From(w) => words(tl!("from {words}"), w),
        Condition::SentTo(w) => words(tl!("sent to {words}"), w),
        Condition::SentOnlyToMe => tl!("sent only to me").to_string(),
        Condition::ToMe => tl!("my address is in To").to_string(),
        Condition::CcMe => tl!("my address is in Cc").to_string(),
        Condition::ToOrCcMe => tl!("my address is in To or Cc").to_string(),
        Condition::NotToMe => tl!("my address isn't in To").to_string(),
        Condition::SubjectOrBodyContains(w) => words(tl!("subject or body contains {words}"), w),
        Condition::Sensitivity(Sensitivity::Normal) => tl!("sensitivity is normal").to_string(),
        Condition::Sensitivity(Sensitivity::Personal) => tl!("sensitivity is personal").to_string(),
        Condition::Sensitivity(Sensitivity::Private) => tl!("sensitivity is private").to_string(),
        Condition::Sensitivity(Sensitivity::Confidential) => tl!("sensitivity is confidential").to_string(),
        Condition::SizeBetween { min, max } => fmt(tl!("size is {min} KB to {max} KB"), &[("min", &min.to_string()), ("max", &max.to_string())]),
        Condition::MeetingRequest => tl!("is a meeting invitation or update").to_string(),
        Condition::AutomaticReply => tl!("is an automatic reply").to_string(),
        Condition::Category(w) => words(tl!("category is {words}"), w),
        Condition::Flagged => tl!("is flagged").to_string(),
        Condition::ReceivedBetween { from, to } => match (from, to) {
            (Some(f), Some(t)) => fmt(tl!("received from {from} to {to}"), &[("from", &f.iso()), ("to", &t.iso())]),
            (Some(f), None) => fmt(tl!("received on or after {day}"), &[("day", &f.iso())]),
            (None, Some(t)) => fmt(tl!("received on or before {day}"), &[("day", &t.iso())]),
            (None, None) => tl!("received on any day").to_string(),
        },
    }
}

fn action(mb: &Mailbox, r: &Rule, a: &Action) -> String {
    match a {
        Action::MoveTo(folder) => match place(mb, r, *folder) {
            Some(folder) => fmt(tl!("move it to {folder}"), &[("folder", &folder)]),
            None => tl!("move it to a folder that is gone").to_string(),
        },
        Action::CopyTo(folder) => match place(mb, r, *folder) {
            Some(folder) => fmt(tl!("copy it to {folder}"), &[("folder", &folder)]),
            None => tl!("copy it to a folder that is gone").to_string(),
        },
        Action::Categorize(category) => fmt(tl!("categorize it as {category}"), &[("category", &quoted(std::slice::from_ref(category)))]),
        Action::Flag => tl!("flag it").to_string(),
        Action::MarkRead => tl!("mark it as read").to_string(),
        Action::SetImportance(Importance::High) => tl!("set importance to high").to_string(),
        Action::SetImportance(Importance::Normal) => tl!("set importance to normal").to_string(),
        Action::SetImportance(Importance::Low) => tl!("set importance to low").to_string(),
        Action::Delete => tl!("delete it").to_string(),
        Action::FlagFor(FlagDue::Today) => tl!("flag it for today").to_string(),
        Action::FlagFor(FlagDue::Tomorrow) => tl!("flag it for tomorrow").to_string(),
        Action::FlagFor(FlagDue::ThisWeek) => tl!("flag it for this week").to_string(),
        Action::FlagFor(FlagDue::NextWeek) => tl!("flag it for next week").to_string(),
        Action::ClearFlag => tl!("clear its flag").to_string(),
        Action::ClearCategories => tl!("clear its categories").to_string(),
    }
}

/// The folder `id`, with its account when it is another's than the rule's (a rule for every account's mail names it too: it is
/// shown under each).
fn place(mb: &Mailbox, r: &Rule, id: Id) -> Option<String> {
    let folder = mb.folder(id)?;
    let path = crate::nav::folder_path(mb, id);
    if folder.account == r.account {
        return Some(path);
    }
    let other = mb.account(folder.account)?;
    let who = if other.email.trim().is_empty() { other.name.trim() } else { other.email.trim() };
    Some(format!("{path} - {who}"))
}

#[cfg(test)]
#[path = "rule_words_tests.rs"]
mod tests;
