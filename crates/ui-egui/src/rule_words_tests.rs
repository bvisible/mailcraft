// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of `rule_words`.

#![allow(clippy::unwrap_used)]

use mailcraft_model::{Action, Condition, FlagDue, Importance, Mailbox, Rule, Sensitivity};
use mailcraft_time::Date;

/// One of each condition and each action a rule may have.
fn every() -> Rule {
    let words = |w: &str| vec![w.to_string(), "say \"hi\"".to_string()];
    let day = |d: u8| Date::new(2026, 10, d);
    Rule {
        name: "Every".into(),
        enabled: true,
        conditions: vec![
            Condition::FromContains(words("zoe")),
            Condition::ToContains(words("team")),
            Condition::SubjectContains(words("plans")),
            Condition::BodyContains(words("lunch")),
            Condition::HasAttachment,
            Condition::Importance(Importance::High),
            Condition::Importance(Importance::Normal),
            Condition::Importance(Importance::Low),
            Condition::From(words("zoe@example.org")),
            Condition::SentTo(words("team@example.org")),
            Condition::SentOnlyToMe,
            Condition::ToMe,
            Condition::CcMe,
            Condition::ToOrCcMe,
            Condition::NotToMe,
            Condition::SubjectOrBodyContains(words("either")),
            Condition::Sensitivity(Sensitivity::Normal),
            Condition::Sensitivity(Sensitivity::Personal),
            Condition::Sensitivity(Sensitivity::Private),
            Condition::Sensitivity(Sensitivity::Confidential),
            Condition::SizeBetween { min: 10, max: 20 },
            Condition::MeetingRequest,
            Condition::AutomaticReply,
            Condition::Category(words("Bills")),
            Condition::Flagged,
            Condition::ReceivedBetween { from: day(1), to: day(2) },
            Condition::ReceivedBetween { from: day(1), to: None },
            Condition::ReceivedBetween { from: None, to: day(2) },
            Condition::ReceivedBetween { from: None, to: None },
        ],
        exceptions: vec![Condition::Flagged, Condition::HasAttachment],
        actions: vec![
            Action::Categorize("Bills".into()),
            Action::Flag,
            Action::MarkRead,
            Action::SetImportance(Importance::High),
            Action::SetImportance(Importance::Normal),
            Action::SetImportance(Importance::Low),
            Action::Delete,
            Action::FlagFor(FlagDue::Today),
            Action::FlagFor(FlagDue::Tomorrow),
            Action::FlagFor(FlagDue::ThisWeek),
            Action::FlagFor(FlagDue::NextWeek),
            Action::ClearFlag,
            Action::ClearCategories,
        ],
        alert: true,
        ..Default::default()
    }
}

/// In English, word for word what upstream says (and the engine tells agents), so that a condition or an action it adds or
/// rewords is seen here at the next merge.
#[test]
fn a_rule_reads_as_upstreams_in_english() {
    let mb = Mailbox::with_local_account("Ada", "ada@example.org");
    let rule = every();
    assert_eq!(super::describe(&mb, &rule), rule.describe_as(false));
    let plain = Rule { exceptions: Vec::new(), alert: false, ..rule };
    assert_eq!(super::describe(&mb, &plain), plain.describe_as(false));
}

/// In French none of it is left in English.
#[test]
fn a_rule_reads_in_french() {
    let _held = crate::i18n::RestoreLanguage::hold();
    crate::i18n::set_current(crate::i18n::Lang::French);
    let mb = Mailbox::with_local_account("Ada", "ada@example.org");
    let mut rule = every();
    for c in &mut rule.conditions {
        if c.words().is_some() {
            *c = c.with_words(vec!["zz".into()]);
        }
    }
    let said = super::describe(&mb, &rule);
    for english in [
        "When",
        "except",
        "contains",
        " and ",
        " or ",
        "importance is",
        "sent",
        "address",
        "sensitivity",
        "size is",
        "received",
        "category",
        "flag it",
        "mark it",
        "set importance",
        "delete",
        "clear",
        "desktop",
        "attachment is",
        "automatic reply",
        "meeting invitation",
    ] {
        assert!(!said.contains(english), "{english}: {said}");
    }
    assert!(said.contains("\"zz\""), "{said}");
}
