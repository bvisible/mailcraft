// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the tests of what a hosting page's window keeps on its server.

//! What a hosting page's window keeps on its server between two visits (kept.rs): the rules, the categories, each
//! mailbox's lists and what was marked done in it, taken from one visit's window and put into the next one's, where the ids
//! are not the same.

use std::collections::BTreeMap;

use mailcraft_model::{
    Account, AccountKind, Action, Category, Condition, Folder, FolderKind, Id, JunkLists, Mailbox, Message, Rgb, Rule, ServerFolder,
};
use mailcraft_ui_egui::mail_view::done::Item;
use serde_json::{Value, json};

use super::{
    DONE_DAYS, DoneEntry, Kept, MAX_DONE, Parts, Runs, RunsOver, Written, for_good, merge_categories, merge_done, merge_lists, merge3, put_done,
    put_done_seen, put_rules, put_senders, read_answer, refresh_done, same, take, take_done, tidy_done, update, with_done, written,
};

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
    Rule {
        id: 0,
        name: name.into(),
        enabled: true,
        conditions: vec![Condition::SubjectContains("invoice".into())],
        actions,
        stop: false,
        ..Default::default()
    }
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
        update(Some(rules.as_slice()), Some(categories.as_slice()), Some(&lists), None),
        json!({"singleton": {"rules": [{"name": "x"}], "categories": [{"name": "Bills", "color": [1, 2, 3]}], "safeSenders": ["a@b.ch"], "blockedSenders": []}})
    );
    assert_eq!(update(None, None, Some(&lists), None), json!({"singleton": {"safeSenders": ["a@b.ch"], "blockedSenders": []}}));
    assert_eq!(update(None, Some(categories.as_slice()), None, None), json!({"singleton": {"categories": [{"name": "Bills", "color": [1, 2, 3]}]}}));
    let done = vec![DoneEntry { message_id: "a.1@example.org".into(), from: "Minbox".into(), at: NOW, who: "Grace".into(), subject: "Plans".into() }];
    assert_eq!(
        update(None, None, None, Some(done.as_slice())),
        json!({"singleton": {"done": [{"messageId": "a.1@example.org", "from": "Minbox", "at": NOW, "who": "Grace", "subject": "Plans"}]}})
    );
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

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// A hosting page's window with the mailboxes `emails`, each with its standard folders, its Inbox, Archive and Projects with
/// their ids on the server (`Minbox`, `Marchive`, `Mprojects`). The ids differ with `shift`, as they do from one visit to the next.
fn with_archive(emails: &[&str], shift: Id) -> Mailbox {
    let mut mb = Mailbox::with_local_account("", "");
    for _ in 0..shift {
        mb.alloc_id();
    }
    for email in emails {
        let account = mb.add_account(Account { email: (*email).into(), name: (*email).into(), kind: AccountKind::Jmap, ..Default::default() });
        for f in mb.folders.iter_mut().filter(|f| f.account == account) {
            let server = match f.kind {
                FolderKind::Inbox => "Minbox",
                FolderKind::Archive => "Marchive",
                _ => continue,
            };
            f.server = Some(ServerFolder { id: server.into(), ..Default::default() });
        }
        let id = mb.alloc_id();
        mb.folders.push(Folder {
            id,
            account,
            name: "Projects".into(),
            server: Some(ServerFolder { id: "Mprojects".into(), ..Default::default() }),
            ..Default::default()
        });
    }
    mb
}

/// A message of `email`'s folder `server` (its id on the server), with the Message-ID `<{name}@example.org>`.
fn mail(mb: &mut Mailbox, email: &str, server: &str, name: &str) -> Id {
    let folder = folder(mb, email, server);
    let m = Message {
        folder,
        message_id: format!("{name}@example.org"),
        subject: format!("About {name}"),
        from: mailcraft_model::Address::new("Grace", "grace@example.org"),
        received: NOW - 3_600,
        ..Message::default()
    };
    mb.add_message(m).unwrap()
}

/// `ids` marked done at `at`, as the window does: each moved to its account's Archive, remembered with the folder it was in.
fn marked(mb: &mut Mailbox, ids: &[Id], at: i64) -> Vec<Item> {
    ids.iter()
        .map(|&id| {
            let m = mb.message(id).unwrap().clone();
            let archive = mb.folder_of_kind(m.account, FolderKind::Archive).unwrap();
            mb.message_mut(id).unwrap().folder = archive;
            Item { id, message_id: m.message_id.clone(), from: m.folder, at, who: m.from.display().to_string(), subject: m.subject.clone() }
        })
        .collect()
}

fn entry(name: &str, from: &str, at: i64) -> DoneEntry {
    DoneEntry { message_id: format!("{name}@example.org"), from: from.into(), at, who: "Grace".into(), subject: format!("About {name}") }
}

#[test]
fn what_was_marked_done_comes_back_at_the_next_visit_by_its_message_id_where_the_ids_differ() {
    let mut first = with_archive(&[ADA, BOB], 0);
    let (plans, offer, notes) =
        (mail(&mut first, ADA, "Minbox", "plans"), mail(&mut first, ADA, "Mprojects", "offer"), mail(&mut first, BOB, "Minbox", "notes"));
    let items = marked(&mut first, &[plans, offer, notes], NOW);
    let adas = take_done(&first, &items, &[], ADA, NOW);
    let names: Vec<(&str, &str)> = adas.iter().map(|e| (e.message_id.as_str(), e.from.as_str())).collect();
    assert_eq!(
        names,
        [("offer@example.org", "Mprojects"), ("plans@example.org", "Minbox")],
        "Ada's, by Message-ID and the folder's id on the server, those of a same second by their Message-ID"
    );
    assert_eq!((adas[1].at, adas[1].who.as_str(), adas[1].subject.as_str()), (NOW, "Grace", "About plans"));
    assert_eq!(take_done(&first, &items, &[], BOB, NOW).len(), 1, "Bob's, his own");

    // The next visit: other ids; Ada's Archive has the plans, but not yet the offer (the run that brings it hasn't come).
    let mut next = with_archive(&[ADA, BOB], 40);
    let plans_next = mail(&mut next, ADA, "Marchive", "plans");
    let (items_next, waiting) = put_done(&next, ADA, &adas, NOW);
    assert_eq!(items_next.len(), 1);
    assert_eq!((items_next[0].id, items_next[0].from), (plans_next, folder(&next, ADA, "Minbox")), "found again, to go back to its Inbox");
    assert_eq!((items_next[0].at, items_next[0].subject.as_str()), (NOW, "About plans"));
    assert_eq!(waiting, vec![adas[0].clone()], "the offer waits for its message");
    // What the window gives back is what the server holds: nothing to write.
    assert_eq!(take_done(&next, &items_next, &waiting, ADA, NOW), adas);
    // The offer comes into the Archive: put back, to go back to Projects.
    let offer_next = mail(&mut next, ADA, "Marchive", "offer");
    let (came, still) = put_done(&next, ADA, &waiting, NOW);
    assert_eq!((came[0].id, came[0].from, still.len()), (offer_next, folder(&next, ADA, "Mprojects"), 0));
}

#[test]
fn a_message_put_back_or_moved_out_of_the_archive_is_no_longer_done_and_one_no_longer_here_is_kept() {
    let mut mb = with_archive(&[ADA], 0);
    let (a, b, c) = (mail(&mut mb, ADA, "Minbox", "a"), mail(&mut mb, ADA, "Minbox", "b"), mail(&mut mb, ADA, "Minbox", "c"));
    let items = marked(&mut mb, &[a, b, c], NOW);
    // `a` moved to Projects by hand; `b` gone from the window (a run that brought it back under another id, say).
    mb.message_mut(a).unwrap().folder = folder(&mb, ADA, "Mprojects");
    mb.messages.remove(&b);
    let kept: Vec<String> = take_done(&mb, &items, &[], ADA, NOW).into_iter().map(|e| e.message_id).collect();
    assert_eq!(kept, ["b@example.org", "c@example.org"]);
    // A message out of the Archive is not put back as done; nor one whose account isn't the mailbox's.
    let (items, waiting) = put_done(&mb, ADA, &[entry("a", "Minbox", NOW)], NOW);
    assert!(items.is_empty() && waiting.len() == 1);
    assert_eq!(put_done(&mb, BOB, &[entry("c", "Minbox", NOW)], NOW).0, vec![]);
}

#[test]
fn a_message_already_back_is_dropped_once_a_run_begun_well_after_the_mark_has_gone_through() {
    let mut mb = with_archive(&[ADA], 0);
    mail(&mut mb, ADA, "Minbox", "back");
    let ada = account(&mb, ADA);
    // A message sent to oneself: its copy in Sent is not the one that was marked.
    let sent = mb.folder_of_kind(ada, FolderKind::Sent).unwrap();
    mb.add_message(Message { folder: sent, message_id: "self@example.org".into(), received: NOW - 3_600, ..Message::default() }).unwrap();
    let at = NOW - 100;
    let (back, unseen, own) = (entry("back", "Minbox", at), entry("unseen", "Minbox", at), entry("self", "Minbox", at));
    // No run seen in this visit: what is here may be older than the mark (a copy of last week, another window's mark).
    let (items, waiting) = put_done_seen(&mb, ADA, std::slice::from_ref(&back), NOW, &RunsOver::new());
    assert!(items.is_empty());
    assert_eq!(waiting, std::slice::from_ref(&back), "it waits");
    // A run begun before the mark, or just after it (the window that marked it may not have told the server yet), says nothing.
    for started in [at - 100, at + 50] {
        let (_, waiting) = put_done_seen(&mb, ADA, std::slice::from_ref(&back), NOW, &RunsOver::from([(ada, started)]));
        assert_eq!(waiting.len(), 1, "a run begun at {started}");
    }
    // One begun well after: its message is where the server has it, out of the Archive: not done any more.
    let (items, waiting) = put_done_seen(&mb, ADA, &[back, unseen.clone(), own.clone()], NOW, &RunsOver::from([(ada, at + 200)]));
    assert!(items.is_empty());
    let mut names: Vec<&str> = waiting.iter().map(|e| e.message_id.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["self@example.org", "unseen@example.org"], "one not here at all may be past what a run reads of the Archive");
}

#[test]
fn the_runs_this_window_saw_go_through_or_dont() {
    use mailcraft_engine::cmd::sync_progress::{AccountRun, RunState};
    let row =
        |state, problems: &[&str]| AccountRun { account: 7, state, problems: problems.iter().map(|p| p.to_string()).collect(), ..Default::default() };
    let mut runs = Runs::default();
    runs.see(&[row(RunState::Done, &[])], NOW);
    assert!(runs.over.is_empty(), "the end of a run it didn't see under way");
    runs.see(&[row(RunState::Waiting, &[])], NOW + 1);
    runs.see(&[row(RunState::Running, &[])], NOW + 2);
    assert!(runs.over.is_empty());
    runs.see(&[row(RunState::Done, &[])], NOW + 3);
    assert_eq!(runs.over.get(&7), Some(&(NOW + 1)), "when it began: what it read is at least that recent");
    runs.see(&[row(RunState::Done, &[])], NOW + 9);
    assert_eq!(runs.over.get(&7), Some(&(NOW + 1)), "the same run, seen again");
    for ended in [row(RunState::Done, &["the server didn't answer"]), row(RunState::Stopped, &[]), row(RunState::Skipped, &[])] {
        runs.see(&[row(RunState::Running, &[])], NOW + 10);
        runs.see(&[ended], NOW + 11);
        assert_eq!(runs.over.get(&7), Some(&(NOW + 1)), "one that didn't go through says nothing");
    }
    // A run that changed nothing shows what the last one came to (`kept`), problems and all: it went through.
    runs.see(&[row(RunState::Running, &[])], NOW + 20);
    runs.see(&[AccountRun { kept: true, ..row(RunState::Stopped, &["the server didn't answer"]) }], NOW + 21);
    assert_eq!(runs.over.get(&7), Some(&(NOW + 20)));
}

#[test]
fn what_was_marked_done_follows_the_mailbox_as_it_changes() {
    let mut mb = with_archive(&[ADA, BOB], 0);
    let (a, b, c) = (mail(&mut mb, ADA, "Minbox", "a"), mail(&mut mb, ADA, "Minbox", "b"), mail(&mut mb, ADA, "Mprojects", "c"));
    let bobs = mail(&mut mb, BOB, "Minbox", "d");
    let items = marked(&mut mb, &[a, b, c, bobs], NOW);
    let none = RunsOver::new();
    // Nothing changed: the same items, nothing waits.
    let (all, waiting) = refresh_done(&mb, &items, &[], ADA, NOW, &none);
    assert!(same(&all, &items) && waiting.is_empty(), "{all:?}");
    // `a` was put back from elsewhere: it leaves, and is not kept. `b` came back as another message of the window (a run
    // brought it under another id): the item follows it. `c` is gone from here: it waits for its message, not shown.
    mb.message_mut(a).unwrap().folder = folder(&mb, ADA, "Minbox");
    let old = mb.messages.remove(&b).unwrap();
    let again = mb.add_message(Message { id: 0, ..(*old).clone() }).unwrap();
    assert_ne!(again, b);
    mb.messages.remove(&c);
    let (all, waiting) = refresh_done(&mb, &items, &[], ADA, NOW, &none);
    let mut ids: Vec<Id> = all.iter().map(|i| i.id).collect();
    ids.sort_unstable();
    let mut expected = vec![again, bobs];
    expected.sort_unstable();
    assert_eq!(ids, expected, "Bob's stays as it was");
    assert_eq!(all.iter().find(|i| i.id == again).unwrap().from, folder(&mb, ADA, "Minbox"), "and still goes back where it came from");
    assert_eq!(waiting.iter().map(|e| e.message_id.as_str()).collect::<Vec<_>>(), ["c@example.org"]);
    // Its message comes back: it shows again, for the folder it came from.
    let back = mail(&mut mb, ADA, "Marchive", "c");
    let (all, waiting) = refresh_done(&mb, &all, &waiting, ADA, NOW, &none);
    assert!(waiting.is_empty());
    assert_eq!(all.iter().find(|i| i.id == back).unwrap().from, folder(&mb, ADA, "Mprojects"));
}

#[test]
fn done_is_kept_seven_days_two_hundred_at_most_one_for_each_message_newest_first() {
    let mut entries: Vec<DoneEntry> = (0..250).map(|n| entry(&format!("m{n}"), "Minbox", NOW - n)).collect();
    entries.push(entry("old", "Minbox", NOW - DONE_DAYS * 86_400 - 1));
    entries.push(entry("m3", "Minbox", NOW - 10_000));
    let tidy = tidy_done(entries, NOW);
    assert_eq!(tidy.len(), MAX_DONE);
    assert_eq!(tidy[0].message_id, "m0@example.org");
    assert!(tidy.windows(2).all(|w| w[0].at >= w[1].at), "newest first");
    assert!(!tidy.iter().any(|e| e.message_id == "old@example.org"), "older than a week");
    assert_eq!(
        tidy.iter().filter(|e| e.message_id == "m3@example.org").map(|e| e.at).collect::<Vec<_>>(),
        [NOW - 3],
        "one for each message, the newest"
    );
    // A sender or a subject is kept short; a Message-ID too long, or none, is not kept.
    let mut mb = with_archive(&[ADA], 0);
    let (a, b, c) = (mail(&mut mb, ADA, "Minbox", "a"), mail(&mut mb, ADA, "Minbox", "b"), mail(&mut mb, ADA, "Minbox", "c"));
    let mut items = marked(&mut mb, &[a, b, c], NOW);
    items[0].subject = "s".repeat(1_000);
    items[1].message_id = format!("{}@example.org", "x".repeat(300));
    items[2].message_id = String::new();
    let kept = take_done(&mb, &items, &[], ADA, NOW);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].subject.chars().count(), 200);
}

#[test]
fn a_server_that_keeps_no_done_is_never_sent_it() {
    let old = read_answer(&json!({"list": [{"id": "singleton", "rules": null, "safeSenders": [], "blockedSenders": []}]}), ADA).unwrap();
    assert!(old.done.is_empty(), "no `done`: a server that doesn't keep it");
    let new = read_answer(&json!({"list": [{"id": "singleton", "done": []}]}), ADA).unwrap();
    assert_eq!(new.done.get(ADA), Some(&vec![]), "nothing kept yet, but kept");
    let read = read_answer(
        &json!({"list": [{"id": "singleton", "done": [
            {"messageId": " <a.1@example.org> ", "from": "Minbox", "at": NOW, "who": "Grace", "subject": "Plans"},
            {"messageId": "", "from": "Minbox", "at": NOW},
            {"messageId": "b.2@example.org", "at": "today"},
            "not an entry"
        ]}]}),
        ADA,
    )
    .unwrap();
    assert_eq!(
        read.done.get(ADA).unwrap(),
        &vec![DoneEntry { message_id: "a.1@example.org".into(), from: "Minbox".into(), at: NOW, who: "Grace".into(), subject: "Plans".into() }]
    );
}

#[test]
fn what_another_window_marked_done_or_put_back_is_merged_with_this_ones() {
    let (a, b, c) = (entry("a", "Minbox", NOW - 30), entry("b", "Minbox", NOW - 20), entry("c", "Minbox", NOW - 10));
    // Base [a, b]; the other window put b back and marked c; this one marked d.
    let d = entry("d", "Minbox", NOW - 5);
    assert_eq!(
        merge_done(&[a.clone(), b.clone()], &[a.clone(), c.clone()], &[a.clone(), b.clone(), d.clone()], NOW),
        [d.clone(), c.clone(), a.clone()]
    );
    // Both marked the same message: one entry, the newest.
    let later = entry("a", "Mprojects", NOW);
    assert_eq!(merge_done(&[], std::slice::from_ref(&a), std::slice::from_ref(&later), NOW), std::slice::from_ref(&later));
    // The window's items of a mailbox replaced, the others' and those it can't keep staying.
    let mut mb = with_archive(&[ADA, BOB], 0);
    let (x, y, z) = (mail(&mut mb, ADA, "Minbox", "x"), mail(&mut mb, BOB, "Minbox", "y"), mail(&mut mb, ADA, "Minbox", "z"));
    let mut items = marked(&mut mb, &[x, y, z], NOW);
    items[2].message_id = String::new();
    let put = vec![Item { id: 999, message_id: "w@example.org".into(), from: items[0].from, at: NOW, who: String::new(), subject: String::new() }];
    let all: Vec<Id> = with_done(&mb, &items, ADA, put).iter().map(|i| i.id).collect();
    assert_eq!(all, [y, z, 999]);
}

/// A change of a rule, a category or a list of senders that waits to be written is the person's own: the page asks before it is
/// left. What was marked done is not (the message is in its Archive whatever happens to the mark): a reload right after a message
/// was marked done raised the browser's « Leave site? » for it (19.11).
#[test]
fn the_page_asks_before_it_is_left_for_the_persons_rules_categories_and_lists_not_for_what_was_marked_done() {
    let entry = DoneEntry { message_id: "m1@example.org".into(), from: "Min".into(), at: 1_800_000_000, who: "Ada".into(), subject: "Hello".into() };
    let done = Parts { done: BTreeMap::from([(ADA.to_string(), (Vec::new(), vec![entry]))]), ..Default::default() };
    assert!(!done.asks_before_leaving(), "what was marked done");
    assert!(!Parts::default().asks_before_leaving(), "nothing");
    let rules = Parts { rules: Some((Vec::new(), vec![json!({"name": "Invoices"})])), ..Default::default() };
    assert!(rules.asks_before_leaving(), "a rule");
    let categories = Parts { categories: Some((Vec::new(), vec![Category::default()])), ..Default::default() };
    assert!(categories.asks_before_leaving(), "a category");
    let senders = Parts { senders: BTreeMap::from([(ADA.to_string(), (JunkLists::default(), JunkLists::default()))]), ..Default::default() };
    assert!(senders.asks_before_leaving(), "a list of senders");
    // A done mark with a rule: the rule asks.
    let both = Parts { rules: rules.rules.clone(), ..done };
    assert!(both.asks_before_leaving());
}
