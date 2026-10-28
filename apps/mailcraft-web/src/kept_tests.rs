// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the tests of what a hosting page's window keeps on its server.

//! What a hosting page's window keeps on its server between two visits (kept.rs): the rules, the categories and each
//! mailbox's lists, taken from one visit's window and put into the next one's, where the ids are not the same.

use std::collections::BTreeMap;

use mailcraft_model::{Account, AccountKind, Action, Category, Condition, Folder, Id, JunkLists, Mailbox, Rgb, Rule, ServerFolder};
use serde_json::{Value, json};

use super::{Kept, Written, for_good, merge_categories, merge_lists, merge3, put_rules, put_senders, read_answer, same, take, update, written};

const ADA: &str = "ada@example.org";
const BOB: &str = "bob@example.org";

/// A hosting page's window with the mailboxes `emails`, each with the folders `folders` (their ids on the server) when
/// `listed`. The ids differ with `shift`, as they do from one visit to the next.
fn window(emails: &[&str], folders: &[&str], listed: bool, shift: Id) -> Mailbox {
    let mut mb = Mailbox::with_local_account("", "");
    for _ in 0..shift {
        mb.alloc_id();
    }
    for email in emails {
        let account = mb.add_account(Account { email: (*email).into(), name: (*email).into(), kind: AccountKind::Jmap, ..Default::default() });
        if listed {
            for folder in folders {
                let id = mb.alloc_id();
                mb.folders.push(Folder {
                    id,
                    account,
                    name: format!("Folder {folder}"),
                    server: Some(ServerFolder { id: (*folder).into(), ..Default::default() }),
                    ..Default::default()
                });
            }
        }
    }
    mb
}

fn folder(mb: &Mailbox, email: &str, server: &str) -> Id {
    let account = account(mb, email);
    mb.folders.iter().find(|f| f.account == account && f.server.as_ref().is_some_and(|s| s.id == server)).map(|f| f.id).unwrap()
}

fn account(mb: &Mailbox, email: &str) -> Id {
    mb.accounts.iter().find(|a| a.email == email).map(|a| a.id).unwrap()
}

fn rule(name: &str, actions: Vec<Action>) -> Rule {
    Rule { id: 0, name: name.into(), enabled: true, conditions: vec![Condition::SubjectContains("invoice".into())], actions, stop: false }
}

fn handed() -> Vec<String> {
    vec![ADA.to_string(), BOB.to_string()]
}

/// What the server would answer, for `email`, after `kept` was written.
fn answer(kept: &Kept, email: &str) -> Value {
    let lists = kept.senders.get(email).cloned().unwrap_or_default();
    json!({"list": [{
        "id": "singleton",
        "rules": kept.rules,
        "categories": kept.categories,
        "safeSenders": lists.safe,
        "blockedSenders": lists.blocked,
    }]})
}

#[test]
fn rules_categories_and_lists_come_back_at_the_next_visit_where_the_ids_differ() {
    let mut first = window(&[ADA, BOB], &["Minbox", "Minvoices"], true, 0);
    let invoices = folder(&first, BOB, "Minvoices");
    first.rules = vec![rule("Invoices", vec![Action::MoveTo(invoices), Action::Categorize("Bills".into()), Action::Flag])];
    first.categories = vec![Category { name: "Bills".into(), color: Rgb(1, 2, 3) }];
    let (ada, bob) = (account(&first, ADA), account(&first, BOB));
    first.junk_lists_mut(ada).blocked = vec!["@spam.example".into()];
    first.junk_lists_mut(bob).safe = vec!["boss@example.com".into()];
    let kept = take(&first, &handed());
    assert_eq!(kept.rules[0]["actions"][0]["value"]["mailbox"], BOB);
    assert_eq!(kept.rules[0]["actions"][0]["value"]["folder"], "Minvoices");
    assert!(kept.rules[0].get("id").is_none(), "an id of this visit is not kept");

    // The next visit: other ids, the folders listed later
    let saved = read_answer(&answer(&kept, ADA), ADA).unwrap();
    let bobs = read_answer(&answer(&kept, BOB), BOB).unwrap();
    let mut next = window(&[ADA, BOB], &["Minbox", "Minvoices"], false, 40);
    assert_eq!(put_rules(&mut next, saved.rules.as_deref().unwrap()), None, "their folders are not listed yet");
    assert!(next.rules.is_empty());

    let mut next = window(&[ADA, BOB], &["Minbox", "Minvoices"], true, 40);
    next.categories = saved.categories.clone().unwrap();
    put_senders(&mut next, ADA, saved.senders.get(ADA).unwrap());
    put_senders(&mut next, BOB, bobs.senders.get(BOB).unwrap());
    assert_eq!(next.junk_lists(account(&next, ADA)).unwrap().blocked, vec!["@spam.example".to_string()]);
    assert_eq!(next.junk_lists(account(&next, BOB)).unwrap().safe, vec!["boss@example.com".to_string()]);
    assert_eq!(put_rules(&mut next, saved.rules.as_deref().unwrap()), Some(vec![]));
    assert_eq!(next.rules.len(), 1);
    assert_eq!(next.rules[0].actions[0], Action::MoveTo(folder(&next, BOB, "Minvoices")));
    assert_eq!(&next.rules[0].actions[1..], &first.rules[0].actions[1..]);
    assert!(next.rules[0].id > 40, "an id of this visit");
    assert_eq!(take(&next, &handed()), kept, "read back, the same");
}

#[test]
fn a_rule_into_a_mailbox_not_handed_over_waits_aside_unchanged() {
    let mut first = window(&[ADA, BOB], &["Minvoices"], true, 0);
    let invoices = folder(&first, BOB, "Minvoices");
    first.rules = vec![rule("Bob's", vec![Action::MoveTo(invoices)]), rule("Flag", vec![Action::Flag])];
    let kept = take(&first, &handed());
    let mut next = window(&[ADA], &["Minvoices"], true, 7);
    let aside = put_rules(&mut next, &kept.rules).unwrap();
    assert_eq!(aside, vec![kept.rules[0].clone()]);
    assert_eq!(next.rules.len(), 1);
    assert_eq!(next.rules[0].name, "Flag");
}

#[test]
fn a_rule_this_build_cant_read_waits_aside_unchanged() {
    let mut next = window(&[ADA], &["Minvoices"], true, 0);
    let newer = json!({"name": "Newer", "enabled": true, "conditions": [{"type": "sentOnFriday"}], "actions": [{"type": "flag"}], "stop": false});
    let plain = json!({"name": "Plain", "enabled": true, "conditions": [{"type": "hasAttachment"}], "actions": [{"type": "flag"}], "stop": false});
    let aside = put_rules(&mut next, &[newer.clone(), plain, json!("not a rule")]).unwrap();
    assert_eq!(aside, vec![newer, json!("not a rule")]);
    assert_eq!(next.rules.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Plain"]);
}

#[test]
fn a_rule_whose_folder_is_gone_loses_that_action_and_goes_with_none_left() {
    let mut first = window(&[ADA], &["Minvoices", "Mold"], true, 0);
    let (invoices, old) = (folder(&first, ADA, "Minvoices"), folder(&first, ADA, "Mold"));
    first.rules = vec![
        rule("Both", vec![Action::MoveTo(old), Action::MarkRead]),
        rule("Gone", vec![Action::MoveTo(old)]),
        rule("Kept", vec![Action::MoveTo(invoices)]),
    ];
    let kept = take(&first, &[ADA.to_string()]);
    let mut next = window(&[ADA], &["Minvoices"], true, 3);
    assert_eq!(put_rules(&mut next, &kept.rules), Some(vec![]));
    let names: Vec<&str> = next.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Both", "Kept"]);
    assert_eq!(next.rules[0].actions, vec![Action::MarkRead]);
}

#[test]
fn what_the_server_never_kept_reads_as_nothing() {
    let saved = read_answer(&json!({"list": [{"id": "singleton", "rules": null, "categories": null, "safeSenders": [], "blockedSenders": []}]}), ADA)
        .unwrap();
    assert_eq!((saved.rules, saved.categories), (None, None));
    assert_eq!(saved.senders, BTreeMap::from([(ADA.to_string(), JunkLists::default())]));
}

#[test]
fn the_servers_lists_are_read_as_the_window_keeps_them() {
    let read = read_answer(
        &json!({"list": [{"id": "singleton", "safeSenders": ["  Boss@Example.COM ", "boss@example.com", "not an address", 3], "blockedSenders": ["*@spam.example"]}]}),
        "Ada@Example.org",
    )
    .unwrap();
    let lists = read.senders.get(ADA).unwrap();
    assert_eq!(lists.safe, vec!["boss@example.com".to_string()]);
    assert_eq!(lists.blocked, vec!["@spam.example".to_string()]);
    assert_eq!(read_answer(&json!({"list": []}), ADA), None);
    assert_eq!(read_answer(&json!({"notFound": ["singleton"]}), ADA), None);
}

#[test]
fn a_category_without_a_name_or_a_colour_is_not_taken() {
    let read = read_answer(
        &json!({"list": [{"id": "singleton", "categories": [{"name": "Bills", "color": [1, 2, 3]}, {"name": " ", "color": [1, 2, 3]}, {"name": "Red", "color": "red"}]}]}),
        ADA,
    )
    .unwrap();
    assert_eq!(read.categories, Some(vec![Category { name: "Bills".into(), color: Rgb(1, 2, 3) }]));
}

#[test]
fn a_write_names_the_parts_it_changes_and_its_answer_is_read() {
    let rules = vec![json!({"name": "x"})];
    let categories = vec![Category { name: "Bills".into(), color: Rgb(1, 2, 3) }];
    let lists = JunkLists { safe: vec!["a@b.ch".into()], blocked: vec![] };
    assert_eq!(
        update(Some(rules.as_slice()), Some(categories.as_slice()), Some(&lists)),
        json!({"singleton": {"rules": [{"name": "x"}], "categories": [{"name": "Bills", "color": [1, 2, 3]}], "safeSenders": ["a@b.ch"], "blockedSenders": []}})
    );
    assert_eq!(update(None, None, Some(&lists)), json!({"singleton": {"safeSenders": ["a@b.ch"], "blockedSenders": []}}));
    assert_eq!(update(None, Some(categories.as_slice()), None), json!({"singleton": {"categories": [{"name": "Bills", "color": [1, 2, 3]}]}}));
    assert_eq!(written(&json!({"updated": {"singleton": null}})), Written::Yes);
    assert_eq!(written(&json!({"updated": null, "notUpdated": {"singleton": {"type": "invalidProperties"}}})), Written::Refused);
    assert_eq!(written(&json!({"something": "else"})), Written::Unknown);
}

#[test]
fn the_lists_of_a_mailbox_are_its_own_and_a_mailbox_the_window_lacks_has_none() {
    let mut mb = window(&[ADA, BOB], &[], true, 0);
    put_senders(&mut mb, BOB, &JunkLists { safe: vec![], blocked: vec!["@x.example".into()] });
    assert_eq!(mb.junk_lists(account(&mb, ADA)), None);
    let kept = take(&mb, &[ADA.to_string(), BOB.to_string(), "gone@example.org".to_string()]);
    assert_eq!(kept.senders.get(BOB).unwrap().blocked, vec!["@x.example".to_string()]);
    assert_eq!(kept.senders.get(ADA), Some(&JunkLists::default()));
    assert!(!kept.senders.contains_key("gone@example.org"), "no lists are written for it");
}

#[test]
fn what_another_window_wrote_since_the_base_is_merged_with_this_ones() {
    // Base [a, b]; the other window added x and took b away; this one added y
    assert_eq!(merge3(&["a", "b"], &["a", "x"], &["a", "b", "y"]), ["a", "y", "x"]);
    // Nothing written elsewhere: this window's, as it is, in its order
    assert_eq!(merge3(&["a", "b"], &["a", "b"], &["b", "a"]), ["b", "a"]);
    let red = |r| Category { name: "Red".into(), color: Rgb(r, 0, 0) };
    assert_eq!(merge_categories(&[red(1)], &[red(2)], &[red(3)]), [red(3)], "one of a name, this window's");
    let lists = |blocked: &[&str]| JunkLists { safe: vec![], blocked: blocked.iter().map(|b| b.to_string()).collect() };
    assert_eq!(merge_lists(&lists(&["@a.ch"]), &lists(&["@a.ch", "@b.ch"]), &lists(&["@c.ch"])), lists(&["@c.ch", "@b.ch"]));
    assert!(same(&[1, 2], &[2, 1]) && !same(&[1, 2], &[1, 3]));
    assert!(for_good("forbidden") && !for_good("stateMismatch") && !for_good("serverFail"));
}
