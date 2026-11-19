// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what a hosting page's window keeps on its server between two
// visits (maintenance#1956).

//! What a hosting page's window keeps on its server between two visits: the rules, the categories, and each mailbox's
//! Safe Senders and Blocked Senders, and what was marked done in it (« Marqués comme traités »).
//!
//! A hosting page's tab keeps nothing of its own (hosted.rs): its mail comes back from the server at every visit, but
//! what MailCraft keeps in the mailbox itself was gone at the next one. The Neoffice server keeps them instead, under a
//! capability of its own, [`SETTINGS`] (`NeomailSettings/get` and `NeomailSettings/set`, one object, `singleton`, in each
//! mailbox): the rules and the categories are the person's (the same in every mailbox), the two lists each mailbox's.
//!
//! **Four parts and a base.** The rules, the categories, each mailbox's lists and each mailbox's done are parts of their own, each with
//! what the server holds as far as the window knows (its base). A part is written only once it was read (a mailbox
//! that didn't answer, rules still waiting for their folders: nothing of theirs is written), and only when the window's
//! differs from its base: the window's defaults never overwrite what the server keeps, and two windows open at once
//! (the desk's tab and Neodesk) each write only the parts they changed. Every [`REFRESH_MS`] what the server holds is
//! read again, and a part another window changed is put here, unless this window changed it too (then this one's goes).
//!
//! **Putting them.** The categories and the lists are put at once; the rules once the folders they file into are listed
//! (a rule names its folder by its mailbox and the folder's id on the server, never by an id of this visit). A rule
//! that files into a mailbox not handed over this time, or that this build can't read (a newer one wrote it), is kept
//! aside, unchanged, and written back with the others; one whose folder is gone from a mailbox that is there loses
//! that action (and goes, with none left). What is put can't be undone (the window's history is forgotten): an Undo
//! would bring back the window from before, and write it.
//!
//! **Done.** What was marked done in a mailbox ([`DoneEntry`]: by its Message-ID, never an id of its server, which names the
//! place and changes as the message goes to the Archive), [`MAX_DONE`] at most, of the last [`DONE_DAYS`]. At the next visit
//! each is put back ([`put_done_seen`]) once its message is in its account's Archive here; one whose message isn't (yet: the run
//! that brings the Archive hasn't come; or for good: it is past the newest messages a run reads of the Archive) waits, not
//! shown, is looked for again as the mailbox changes ([`refresh_done`]), and is written back unchanged until it is too old.
//! One whose message is back in a folder goes, here and on the server, once a run begun well after the mark has gone through
//! ([`Runs`]). A server that doesn't keep them (its answer has no `done`) is never sent them.
//!
//! **Writing.** Every couple of seconds a changed part is written, the rules and categories to the first mailbox that
//! answered, each mailbox's lists to it. One the server refuses is not tried again (said once in the console); one that
//! didn't go through is tried again half a minute later. While a change the person made waits (a rule, a category, a list of
//! senders: not what was marked done), the page is told that leaving would lose work (host_dirty.rs).
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::BTreeMap;

use mailcraft_engine::cmd::sync_progress::{AccountRun, RunState};
use mailcraft_model::{Action, Category, Folder, FolderKind, Id, JunkLists, MAX_JUNK_ENTRIES, MAX_RULES, Mailbox, Rule, junk_entry};
use mailcraft_ui_egui::mail_view::done::Item as DoneItem;
use serde_json::{Map, Value, json};

/// The capability: a data type of the Neoffice server (frappe_webmail's mailcraft_settings), named by it.
pub const SETTINGS: &str = "https://neoffice.me/jmap/neomail-settings";
pub const GET: &str = "NeomailSettings/get";
pub const SET: &str = "NeomailSettings/set";
pub const SINGLETON: &str = "singleton";
/// The most categories taken from the server (as a mailbox keeps).
pub const MAX_CATEGORIES: usize = 1_000;
/// How often what the server holds is read again.
pub const REFRESH_MS: f64 = 45_000.0;
/// The most messages marked done kept of a mailbox, and for how many days.
pub const MAX_DONE: usize = 200;
pub const DONE_DAYS: i64 = 7;
/// What is kept of a done row's sender and subject, in characters; a Message-ID longer than this many bytes is not kept.
const DONE_TEXT: usize = 200;
const MAX_MESSAGE_ID: usize = 250;

/// What the window holds, in the server's shape: the rules (a folder by its mailbox and its id on the server), the
/// categories, and the lists of each mailbox handed over (by its address in lower case).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kept {
    pub rules: Vec<Value>,
    pub categories: Vec<Category>,
    pub senders: BTreeMap<String, JunkLists>,
}

/// What the server keeps, as read: `None` for a part it never kept; what was marked done in a mailbox only for one whose
/// server keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub rules: Option<Vec<Value>>,
    pub categories: Option<Vec<Category>>,
    pub senders: BTreeMap<String, JunkLists>,
    pub done: BTreeMap<String, Vec<DoneEntry>>,
}

/// A message marked done, as its mailbox's server keeps it: its Message-ID, the folder it was in (its id on the server), when it
/// was marked (seconds), and what its row said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoneEntry {
    pub message_id: String,
    pub from: String,
    pub at: i64,
    pub who: String,
    pub subject: String,
}

/// A mailbox's address as a key.
pub fn key(email: &str) -> String {
    email.trim().to_lowercase()
}

/// A folder's name on its server: its id there (JMAP, Graph), else its path (IMAP).
fn folder_key(f: &Folder) -> Option<String> {
    let id = f.server.as_ref().map(|s| s.id.trim()).filter(|id| !id.is_empty());
    id.or_else(|| Some(f.remote.trim()).filter(|path| !path.is_empty())).map(str::to_string)
}

/// The accounts of the address `email` that keep their own lists (not a shared mailbox opened through another).
fn accounts_of(mb: &Mailbox, email: &str) -> Vec<Id> {
    mb.accounts.iter().filter(|a| a.shared_of.is_none() && !a.email.trim().is_empty() && key(&a.email) == email).map(|a| a.id).collect()
}

/// What the window holds now, for the mailboxes `handed` (addresses as [`key`] gives them) that it has.
pub fn take(mb: &Mailbox, handed: &[String]) -> Kept {
    let rules = mb.rules.iter().map(|r| rule_out(mb, r)).collect();
    let senders = handed
        .iter()
        .filter_map(|email| {
            let id = *accounts_of(mb, email).first()?;
            Some((email.clone(), mb.junk_lists(id).cloned().unwrap_or_default()))
        })
        .collect();
    Kept { rules, categories: mb.categories.clone(), senders }
}

/// A rule in the server's shape: without its id, its folders by their mailbox and their id on the server.
fn rule_out(mb: &Mailbox, rule: &Rule) -> Value {
    let mut v = serde_json::to_value(rule).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("id");
        if let Some(actions) = o.get_mut("actions").and_then(Value::as_array_mut) {
            for (out, action) in actions.iter_mut().zip(&rule.actions) {
                if let (Action::MoveTo(folder), Some(out)) = (action, out.as_object_mut()) {
                    out.insert("value".into(), folder_out(mb, *folder));
                }
            }
        }
    }
    v
}

fn folder_out(mb: &Mailbox, id: Id) -> Value {
    let Some(f) = mb.folder(id) else { return json!({}) };
    let mailbox = mb.account(f.account).map(|a| key(&a.email)).unwrap_or_default();
    json!({"mailbox": mailbox, "folder": folder_key(f).unwrap_or_default(), "name": f.name})
}

/// What a `NeomailSettings/get` answer says of its one object, for the mailbox `email`.
pub fn read_answer(answer: &Value, email: &str) -> Option<Saved> {
    let object = answer.get("list")?.as_array()?.iter().find(|o| o.get("id").and_then(Value::as_str) == Some(SINGLETON))?;
    let rules = object.get("rules").and_then(Value::as_array).map(|r| r.iter().take(MAX_RULES).cloned().collect());
    let categories = object.get("categories").and_then(Value::as_array).map(|list| {
        list.iter()
            .take(MAX_CATEGORIES)
            .filter_map(|c| serde_json::from_value::<Category>(c.clone()).ok())
            .filter(|c| !c.name.trim().is_empty())
            .collect()
    });
    let lists = JunkLists { safe: entries(object.get("safeSenders")), blocked: entries(object.get("blockedSenders")) };
    let done = object.get("done").and_then(Value::as_array).map(|list| list.iter().take(MAX_DONE * 2).filter_map(done_in).collect());
    Some(Saved { rules, categories, senders: BTreeMap::from([(key(email), lists)]), done: done.into_iter().map(|done| (key(email), done)).collect() })
}

/// A Message-ID as it names a message: without its brackets and spaces.
fn message_key(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').trim().to_string()
}

/// Whether what was marked done of a message of this Message-ID can be kept: it has one, not too long.
fn keepable(id: &str) -> bool {
    let key = message_key(id);
    !key.is_empty() && key.len() <= MAX_MESSAGE_ID
}

/// A done entry of the server, when it is one.
fn done_in(v: &Value) -> Option<DoneEntry> {
    let text = |name: &str| v.get(name).and_then(Value::as_str).unwrap_or_default();
    let message_id = message_key(text("messageId"));
    let at = v.get("at").and_then(Value::as_i64)?;
    keepable(&message_id).then(|| DoneEntry {
        message_id,
        from: text("from").trim().to_string(),
        at,
        who: text("who").chars().take(DONE_TEXT).collect(),
        subject: text("subject").chars().take(DONE_TEXT).collect(),
    })
}

fn done_out(e: &DoneEntry) -> Value {
    json!({"messageId": e.message_id, "from": e.from, "at": e.at, "who": e.who, "subject": e.subject})
}

/// `entries` as a mailbox keeps them: those of the last [`DONE_DAYS`] before `now`, the newest first, one for each message,
/// [`MAX_DONE`] at most.
pub fn tidy_done(mut entries: Vec<DoneEntry>, now: i64) -> Vec<DoneEntry> {
    let since = now.saturating_sub(DONE_DAYS * 86_400);
    entries.retain(|e| e.at >= since);
    // (Those of a same second by their Message-ID: the same entries are always in the same order, whichever were found here.)
    entries.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.message_id.cmp(&b.message_id)));
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| seen.insert(e.message_id.clone()));
    entries.truncate(MAX_DONE);
    entries
}

/// The account a done item is of: its message's, else the folder's it came from.
fn item_account(mb: &Mailbox, item: &DoneItem) -> Option<Id> {
    mb.message(item.id).map(|m| m.account).or_else(|| mb.folder(item.from).map(|f| f.account))
}

/// Whether `item`'s message is no longer done: it is here, out of its account's Archive (put back, moved elsewhere).
fn undone(mb: &Mailbox, item: &DoneItem) -> bool {
    mb.message(item.id).is_some_and(|m| mb.folder(m.folder).is_none_or(|f| f.kind != FolderKind::Archive))
}

/// What the window holds as done of the mailbox `email` (as [`key`] gives it), in the server's shape ([`tidy_done`]): what
/// was marked done here (`items`, whose message is still in its Archive, or no longer here: it may come back under another id),
/// and the server's entries that wait for their message (`waiting`).
pub fn take_done(mb: &Mailbox, items: &[DoneItem], waiting: &[DoneEntry], email: &str, now: i64) -> Vec<DoneEntry> {
    let accounts = accounts_of(mb, email);
    let mut out: Vec<DoneEntry> = items
        .iter()
        .filter(|i| item_account(mb, i).is_some_and(|a| accounts.contains(&a)) && !undone(mb, i) && keepable(&i.message_id))
        .map(|i| DoneEntry {
            message_id: message_key(&i.message_id),
            from: mb.folder(i.from).and_then(folder_key).unwrap_or_default(),
            at: i.at,
            who: i.who.chars().take(DONE_TEXT).collect(),
            subject: i.subject.chars().take(DONE_TEXT).collect(),
        })
        .collect();
    out.extend(waiting.iter().cloned());
    tidy_done(out, now)
}

/// When the last run of each account that went through began, as this window saw it (seconds): what it read is at least that
/// recent.
pub type RunsOver = BTreeMap<Id, i64>;

/// How long after a mark a run has to begin to say where its message is, in seconds: the window that marked it tells the server
/// of the move a little after it tells it of the mark, and another window's run may read the server in between.
const DONE_SETTLES: i64 = 120;

/// What this window saw of its accounts' runs: which are under way (since when), and when the last one that went through began.
#[derive(Clone, Debug, Default)]
pub struct Runs {
    under_way: BTreeMap<Id, i64>,
    pub over: RunsOver,
}

impl Runs {
    /// The rows of the run as they stand at `now`: a run that was seen under way and ended with nothing wrong went through.
    pub fn see(&mut self, rows: &[AccountRun], now: i64) {
        for row in rows {
            match row.state {
                RunState::Waiting | RunState::Running => {
                    self.under_way.entry(row.account).or_insert(now);
                }
                state => {
                    // (`kept`: it changed nothing, and shows what the last one came to, problems and all. It went through.)
                    if let Some(began) = self.under_way.remove(&row.account)
                        && (row.kept || (state == RunState::Done && row.problems.is_empty()))
                    {
                        self.over.insert(row.account, began);
                    }
                }
            }
        }
    }
}

/// [`put_done_seen`] before any run was seen.
#[cfg(test)]
pub fn put_done(mb: &Mailbox, email: &str, entries: &[DoneEntry], now: i64) -> (Vec<DoneItem>, Vec<DoneEntry>) {
    put_done_seen(mb, email, entries, now, &RunsOver::new())
}

/// What the window holds as done of the mailbox `email`, once the mailbox has changed: all its items (those of the other
/// mailboxes as they are) and the entries that wait. A message put back leaves; one that came back under another id of the
/// window is followed; one no longer here waits, not shown, until it comes back or is too old.
pub fn refresh_done(
    mb: &Mailbox,
    items: &[DoneItem],
    waiting: &[DoneEntry],
    email: &str,
    now: i64,
    runs: &RunsOver,
) -> (Vec<DoneItem>, Vec<DoneEntry>) {
    let entries = take_done(mb, items, waiting, email, now);
    let (put, still) = put_done_seen(mb, email, &entries, now, runs);
    (with_done(mb, items, email, put), still)
}

/// The server's done entries of the mailbox `email` put into the window: the items of those whose message is in its account's
/// Archive here (each to be put back in the folder it came from, else its account's Inbox), and the entries that wait for theirs.
/// One whose message is here out of the Archive (a copy in Sent or Drafts is another matter), when a run of its account that
/// began well after the mark went through (`runs`, [`DONE_SETTLES`]), is no longer done and is dropped: before such a run, what
/// is here may be older than the mark (the copy of a last visit, a move still on its way to the server).
pub fn put_done_seen(mb: &Mailbox, email: &str, entries: &[DoneEntry], now: i64, runs: &RunsOver) -> (Vec<DoneItem>, Vec<DoneEntry>) {
    let accounts = accounts_of(mb, email);
    let mut found: std::collections::HashMap<String, (Id, Id)> = std::collections::HashMap::new();
    for &account in &accounts {
        let Some(archive) = mb.folder_of_kind(account, FolderKind::Archive) else { continue };
        for m in mb.messages_in(archive) {
            let id = message_key(&m.message_id);
            if !id.is_empty() {
                found.entry(id).or_insert((m.id, account));
            }
        }
    }
    // (Only once a run was seen: nothing else reads it.)
    let elsewhere: std::collections::HashMap<String, Id> = if runs.is_empty() {
        std::collections::HashMap::new()
    } else {
        mb.messages
            .values()
            .filter(|m| {
                let elsewhere = |kind| !matches!(kind, FolderKind::Archive | FolderKind::Sent | FolderKind::Drafts | FolderKind::Outbox);
                accounts.contains(&m.account) && mb.folder(m.folder).is_none_or(|f| elsewhere(f.kind))
            })
            .map(|m| (message_key(&m.message_id), m.account))
            .filter(|(id, _)| !id.is_empty())
            .collect()
    };
    let (mut items, mut waiting) = (Vec::new(), Vec::new());
    for e in tidy_done(entries.to_vec(), now) {
        let Some(&(id, account)) = found.get(&e.message_id) else {
            let back =
                elsewhere.get(&e.message_id).and_then(|account| runs.get(account)).is_some_and(|began| *began > e.at.saturating_add(DONE_SETTLES));
            if !back {
                waiting.push(e);
            }
            continue;
        };
        let from = resolve(mb, email, &e.from).or_else(|| mb.folder_of_kind(account, FolderKind::Inbox));
        let Some(from) = from else {
            waiting.push(e);
            continue;
        };
        items.push(DoneItem { id, message_id: e.message_id, from, at: e.at, who: e.who, subject: e.subject });
    }
    (items, waiting)
}

/// The window's done items with those of the mailbox `email` that can be kept (a Message-ID) in place of `put`: those of the
/// other mailboxes, and those this one can't keep, stay.
pub fn with_done(mb: &Mailbox, items: &[DoneItem], email: &str, put: Vec<DoneItem>) -> Vec<DoneItem> {
    let accounts = accounts_of(mb, email);
    let ours = |i: &DoneItem| item_account(mb, i).is_some_and(|a| accounts.contains(&a)) && keepable(&i.message_id);
    let mut out: Vec<DoneItem> = items.iter().filter(|i| !ours(i)).cloned().collect();
    out.extend(put);
    out
}

/// [`merge3`] of a mailbox's done entries, in its shape ([`tidy_done`]): one for each message, this window's when both changed it.
pub fn merge_done(base: &[DoneEntry], server: &[DoneEntry], local: &[DoneEntry], now: i64) -> Vec<DoneEntry> {
    let mut merged = merge3(base, server, local);
    // (One for each message, this window's first: tidy keeps the newest, and this window's when they are as new.)
    merged.sort_by_key(|e| !local.contains(e));
    tidy_done(merged, now)
}

/// A list of addresses and domains as the window keeps them: each made a list's entry, the ones that can't be
/// dropped, each once, [`MAX_JUNK_ENTRIES`] at most.
fn entries(list: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in list.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).filter_map(junk_entry) {
        if out.len() >= MAX_JUNK_ENTRIES {
            break;
        }
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    out
}

/// Put the lists of the mailbox `email` into the window.
pub fn put_senders(mb: &mut Mailbox, email: &str, lists: &JunkLists) {
    for id in accounts_of(mb, email) {
        *mb.junk_lists_mut(id) = lists.clone();
    }
}

/// The mailboxes a rule's actions file into, as the server keeps them.
fn targets(rule: &Value) -> Vec<String> {
    let actions = rule.get("actions").and_then(Value::as_array).into_iter().flatten();
    actions
        .filter(|a| a.get("type").and_then(Value::as_str) == Some("moveTo"))
        .map(|a| a.get("value").and_then(|v| v.get("mailbox")).and_then(Value::as_str).map(key).unwrap_or_default())
        .collect()
}

/// Whether the folders of the account `id` are listed: one of them has its name on the server.
fn listed(mb: &Mailbox, id: Id) -> bool {
    mb.folders.iter().any(|f| f.account == id && folder_key(f).is_some())
}

/// The folder a rule names, by its mailbox and its id on the server.
fn resolve(mb: &Mailbox, mailbox: &str, folder: &str) -> Option<Id> {
    if folder.is_empty() {
        return None;
    }
    let accounts = accounts_of(mb, mailbox);
    mb.folders.iter().find(|f| accounts.contains(&f.account) && folder_key(f).as_deref() == Some(folder)).map(|f| f.id)
}

/// Put the rules the server keeps into the window, once the folders they file into are listed: `None` while one of
/// their mailboxes is there without its folders yet (nothing is put); else the rules kept aside, unchanged: those that
/// file into a mailbox not handed over this time, and those this build can't read.
pub fn put_rules(mb: &mut Mailbox, rules: &[Value]) -> Option<Vec<Value>> {
    let mut aside = Vec::new();
    let mut here = Vec::new();
    for rule in rules.iter().take(MAX_RULES) {
        let mut absent = false;
        for mailbox in targets(rule) {
            // (A folder of no mailbox's: the action goes, as for a folder that is gone.)
            if mailbox.is_empty() {
                continue;
            }
            let accounts = accounts_of(mb, &mailbox);
            if accounts.is_empty() {
                absent = true;
            } else if !accounts.iter().any(|&id| listed(mb, id)) {
                return None;
            }
        }
        if absent { aside.push(rule.clone()) } else { here.push(rule) }
    }
    let mut put = Vec::new();
    for rule in here {
        match rule_in(mb, rule) {
            Placed::Put(rule) => put.push(rule),
            Placed::Unread => aside.push(rule.clone()),
            Placed::Emptied => {}
        }
    }
    mb.rules = put;
    Some(aside)
}

/// What became of a rule of the server.
enum Placed {
    Put(Rule),
    /// This build can't read it (a condition or an action of a newer one).
    Unread,
    /// Every folder it filed into is gone.
    Emptied,
}

/// A rule of the server in the window's shape, with an id of its own.
fn rule_in(mb: &mut Mailbox, rule: &Value) -> Placed {
    let mut v = rule.clone();
    let Some(o) = v.as_object_mut() else { return Placed::Unread };
    o.remove("id");
    let mut had = false;
    if let Some(actions) = o.get_mut("actions").and_then(Value::as_array_mut) {
        had = !actions.is_empty();
        let mut kept = Vec::new();
        for action in actions.drain(..) {
            if action.get("type").and_then(Value::as_str) != Some("moveTo") {
                kept.push(action);
                continue;
            }
            let at = |name: &str| action.get("value").and_then(|v| v.get(name)).and_then(Value::as_str).unwrap_or_default().to_string();
            if let Some(folder) = resolve(mb, &key(&at("mailbox")), &at("folder")) {
                let mut moved = Map::new();
                moved.insert("type".into(), json!("moveTo"));
                moved.insert("value".into(), json!(folder));
                kept.push(Value::Object(moved));
            }
        }
        *actions = kept;
    }
    let Ok(mut rule) = serde_json::from_value::<Rule>(v) else { return Placed::Unread };
    if had && rule.actions.is_empty() {
        return Placed::Emptied;
    }
    rule.id = mb.alloc_id();
    Placed::Put(rule)
}

/// The `update` of a `NeomailSettings/set`, with the parts it writes.
pub fn update(rules: Option<&[Value]>, categories: Option<&[Category]>, lists: Option<&JunkLists>, done: Option<&[DoneEntry]>) -> Value {
    let mut patch = Map::new();
    if let Some(rules) = rules {
        patch.insert("rules".into(), Value::Array(rules.iter().take(MAX_RULES).cloned().collect()));
    }
    if let Some(categories) = categories {
        let categories: Vec<&Category> = categories.iter().take(MAX_CATEGORIES).collect();
        patch.insert("categories".into(), serde_json::to_value(categories).unwrap_or_else(|_| json!([])));
    }
    if let Some(lists) = lists {
        patch.insert("safeSenders".into(), json!(lists.safe));
        patch.insert("blockedSenders".into(), json!(lists.blocked));
    }
    if let Some(done) = done {
        patch.insert("done".into(), Value::Array(done.iter().take(MAX_DONE).map(done_out).collect()));
    }
    json!({ SINGLETON: Value::Object(patch) })
}

/// What a `NeomailSettings/set` answer says of the object: written, refused (the server won't take it as it is), or
/// neither (nothing that can be read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Written {
    Yes,
    Refused,
    Unknown,
}

pub fn written(answer: &Value) -> Written {
    let said = |name: &str| answer.get(name).and_then(Value::as_object).is_some_and(|o| o.contains_key(SINGLETON));
    if said("updated") {
        Written::Yes
    } else if said("notUpdated") {
        Written::Refused
    } else {
        Written::Unknown
    }
}

/// A three-way merge of two lists that changed since `base`: what `server` added since is added to `local` (after it),
/// what `server` took away since is taken away from it, and what `local` did stays. Order is `local`'s.
pub fn merge3<T: Clone + PartialEq>(base: &[T], server: &[T], local: &[T]) -> Vec<T> {
    let mut out: Vec<T> = local.iter().filter(|x| !(base.contains(x) && !server.contains(x))).cloned().collect();
    for x in server {
        if !base.contains(x) && !out.contains(x) {
            out.push(x.clone());
        }
    }
    out
}

/// [`merge3`] of the categories, one of a name (this window's, when both made one).
pub fn merge_categories(base: &[Category], server: &[Category], local: &[Category]) -> Vec<Category> {
    let mut out: Vec<Category> = Vec::new();
    for c in merge3(base, server, local) {
        if !out.iter().any(|o| o.name == c.name) {
            out.push(c);
        }
    }
    out
}

/// [`merge3`] of a mailbox's two lists.
pub fn merge_lists(base: &JunkLists, server: &JunkLists, local: &JunkLists) -> JunkLists {
    JunkLists { safe: merge3(&base.safe, &server.safe, &local.safe), blocked: merge3(&base.blocked, &server.blocked, &local.blocked) }
}

/// Whether two lists hold the same things, whatever their order (the rules kept aside go last here).
pub fn same<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    a.len() == b.len() && a.iter().all(|x| b.contains(x)) && b.iter().all(|x| a.contains(x))
}

/// Whether a refusal of the server is for good (a mailbox no longer open to the person, a call it won't take): not
/// tried again. Anything else is tried again later.
pub fn for_good(kind: &str) -> bool {
    matches!(kind, "forbidden" | "accountNotFound" | "accountNotSupportedByMethod" | "invalidArguments" | "unknownMethod" | "unknownCapability")
}

#[cfg(test)]
#[path = "kept_tests.rs"]
mod tests;

/// The parts a write sends, each as (its base, the window's).
#[derive(Clone, Default)]
pub struct Parts {
    pub rules: Option<(Vec<Value>, Vec<Value>)>,
    pub categories: Option<(Vec<Category>, Vec<Category>)>,
    pub senders: BTreeMap<String, (JunkLists, JunkLists)>,
    pub done: BTreeMap<String, (Vec<DoneEntry>, Vec<DoneEntry>)>,
}

impl Parts {
    fn is_empty(&self) -> bool {
        self.rules.is_none() && self.categories.is_none() && self.senders.is_empty() && self.done.is_empty()
    }

    /// Whether leaving the page while these changes wait to be written (or are being written) would lose something the person
    /// made, which the page asks about first (`pending`): a rule, a category, a list of senders. Not what was marked done: the
    /// message is in its Archive whatever happens to the mark, and asking for it made a reload right after a message was marked
    /// done raise the browser's « Leave site? » (19.11).
    pub fn asks_before_leaving(&self) -> bool {
        self.rules.is_some() || self.categories.is_some() || !self.senders.is_empty()
    }
}

#[cfg(target_arch = "wasm32")]
pub use run::{frame, pending, start};

/// The browser's part: reading at the start and again now and then, writing what changed.
#[cfg(target_arch = "wasm32")]
mod run {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;
    use std::time::Duration;

    use mailcraft_jmap::{CORE, Client, Error};
    use mailcraft_model::{Category, JunkLists, MAX_RULES};
    use mailcraft_ui_egui::MailApp;
    use serde_json::{Value, json};

    use super::{
        DoneEntry, GET, Parts, REFRESH_MS, Runs, SET, SETTINGS, SINGLETON, Saved, Written, for_good, key, merge_categories, merge_done, merge_lists,
        merge3, put_done_seen, put_rules, put_senders, read_answer, refresh_done, same, take, take_done, tidy_done, update, with_done, written,
    };
    use crate::credentials::{Credentials, Renewal};
    use crate::host::{HostAccount, HostCredentials};

    /// How often the window is compared with its base, and how long a write that didn't go through waits.
    const EVERY_MS: f64 = 2_000.0;
    const AGAIN_MS: f64 = 30_000.0;
    /// How long the rules wait for their folders before this read gives them up.
    const RULES_WAIT_MS: f64 = 180_000.0;
    /// How many times the start's reading of a mailbox is tried; how many times a write is made again after another
    /// window wrote in between.
    const READS: u32 = 3;
    const WRITES: u32 = 3;

    /// How a write ended: each part written (the window's as it was sent, and what was written: the window's with what
    /// another window wrote since its base) or refused for good, and whether some didn't go through.
    #[derive(Default)]
    struct Ended {
        written: Parts,
        refused: Parts,
        failed: bool,
    }

    #[derive(Default)]
    struct State {
        /// The mailboxes handed over: address (lower case), session address.
        handed: Vec<(String, String)>,
        credentials: Option<Rc<HostCredentials>>,
        ctx: Option<egui::Context>,
        /// What a reading brought, waiting to be put, and the mailboxes that answered it.
        read: Option<(Saved, Vec<String>)>,
        reading: bool,
        read_at: f64,
        /// The mailboxes whose reading answered, the first one taking the rules and categories.
        answered: Vec<String>,
        rules_waiting: Option<(Vec<Value>, f64)>,
        aside: Vec<Value>,
        /// What the server holds of each part, as far as this window knows; `None` (or no entry) until it was read.
        rules: Option<Vec<Value>>,
        categories: Option<Vec<Category>>,
        senders: BTreeMap<String, JunkLists>,
        /// What the server holds as done of each mailbox whose server keeps it; of its entries, those whose message isn't in
        /// its Archive here (yet); the runs this window saw; and what the mailbox was at when all that was last looked at
        /// (its revision, the runs that went through), and when.
        done: BTreeMap<String, Vec<DoneEntry>>,
        done_waiting: BTreeMap<String, Vec<DoneEntry>>,
        runs: Runs,
        done_looked: ((u64, super::RunsOver), f64),
        writing: bool,
        ended: Option<Ended>,
        changed: bool,
        /// What changed (or is being written) is something the person made ([`Parts::asks_before_leaving`]), and when that was
        /// last looked at during a write or before a write is tried again.
        asks: bool,
        asked_at: f64,
        next: f64,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    /// Whether a change the person made waits to be written (or is being written): leaving the page now would lose it
    /// ([`Parts::asks_before_leaving`]).
    pub fn pending() -> bool {
        STATE.with(|s| s.try_borrow().is_ok_and(|s| s.asks && (s.changed || s.writing)))
    }

    /// Read what the server keeps for the mailboxes the page handed over.
    pub fn start(accounts: &[HostAccount], credentials: Rc<HostCredentials>, ctx: egui::Context) {
        let handed: Vec<(String, String)> = accounts.iter().map(|a| (key(&a.email), a.session.trim().to_string())).collect();
        if handed.is_empty() {
            return;
        }
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.handed = handed.clone();
            s.credentials = Some(credentials.clone());
            s.ctx = Some(ctx);
            s.reading = true;
        });
        wasm_bindgen_futures::spawn_local(read(credentials, handed, READS));
    }

    /// Read the mailboxes `handed`, each tried `tries` times, and leave what came for the next frame.
    async fn read(credentials: Rc<HostCredentials>, handed: Vec<(String, String)>, tries: u32) {
        let mut saved = Saved::default();
        let mut answered = Vec::new();
        for (email, session) in &handed {
            for attempt in 0..tries {
                let got = match connect(&credentials, email, session).await {
                    Ok(Some(client)) => get(&client, email).await.map(Some),
                    Ok(None) => Ok(None),
                    Err(e) => Err(e),
                };
                match got {
                    Ok(Some((got, _))) => {
                        if answered.is_empty() {
                            saved.rules = got.rules;
                            saved.categories = got.categories;
                        }
                        saved.senders.extend(got.senders);
                        saved.done.extend(got.done);
                        answered.push(email.clone());
                        break;
                    }
                    // A server that doesn't keep them: nothing to read, nothing to write
                    Ok(None) => break,
                    Err(e) => {
                        log::warn!("kept settings: a mailbox's couldn't be read ({})", kind(&e));
                        if attempt + 1 < tries {
                            crate::fetch::sleep(5_000).await;
                        }
                    }
                }
            }
        }
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.read = Some((saved, answered));
            s.reading = false;
            if let Some(ctx) = &s.ctx {
                ctx.request_repaint();
            }
        });
    }

    /// Put what a reading brought, and write what changed since: called at every frame of a hosting page's window.
    pub fn frame(app: &mut MailApp) {
        let now = mailcraft_ui_egui::now_ms();
        STATE.with(|cell| {
            let Ok(mut guard) = cell.try_borrow_mut() else { return };
            let s = &mut *guard;
            let mut put = false;
            if let Some((saved, answered)) = s.read.take() {
                s.read_at = now;
                put |= merge(s, app, saved, &answered, now);
                for email in answered {
                    if !s.answered.contains(&email) {
                        s.answered.push(email);
                    }
                }
            }
            if let Some((rules, since)) = s.rules_waiting.take() {
                match put_rules(&mut app.session.mailbox, &rules) {
                    Some(aside) => {
                        s.aside = aside;
                        // The base as the window gives it back (the rules kept aside last): what it holds is no change
                        s.rules = Some(here_rules(s, app));
                        put = true;
                    }
                    None if now - since < RULES_WAIT_MS => s.rules_waiting = Some((rules, since)),
                    None => log::warn!("kept settings: the rules' folders never came; they are not written this time"),
                }
            }
            if let Some(ended) = s.ended.take() {
                s.writing = false;
                let failed = ended.failed;
                put |= ended_write(s, app, ended, now);
                // (Written, or refused for good: what waits now is what was changed since, if anything.)
                if !failed {
                    let changes = changes(s, app);
                    s.changed = !changes.is_empty();
                    s.asks = changes.asks_before_leaving();
                }
            }
            // What was marked done follows the mailbox as it changes, and as its runs go through.
            s.runs.see(&app.session.sync_progress.accounts, app.session.now());
            let seen = (app.session.rev(), s.runs.over.clone());
            if !s.done.is_empty() && seen != s.done_looked.0 && now - s.done_looked.1 >= EVERY_MS {
                s.done_looked = (seen, now);
                look_for_done(s, app);
            }
            // What is put can't be undone: an Undo would bring back the window from before, and write it.
            if put {
                app.session.forget_history();
            }
            // While a write is under way, or one that didn't go through waits to be tried again, a change the person makes
            // meanwhile asks before the page is left too.
            if (s.writing || s.changed) && now - s.asked_at >= EVERY_MS {
                s.asked_at = now;
                s.asks |= changes(s, app).asks_before_leaving();
            }
            if s.writing || s.reading || s.answered.is_empty() {
                return;
            }
            let ctx = s.ctx.clone();
            if now >= s.next {
                s.next = now + EVERY_MS;
                let changes = changes(s, app);
                s.changed = !changes.is_empty();
                s.asks = changes.asks_before_leaving();
                if s.changed {
                    if let Some(credentials) = s.credentials.clone() {
                        s.writing = true;
                        let at = app.session.now();
                        let handed = s.handed.clone();
                        let first = s.answered.first().cloned().unwrap_or_default();
                        let ctx = ctx.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let ended = write(&credentials, &handed, &first, changes, at).await;
                            STATE.with(|s| s.borrow_mut().ended = Some(ended));
                            if let Some(ctx) = ctx {
                                ctx.request_repaint();
                            }
                        });
                    }
                } else if s.rules_waiting.is_none() && now - s.read_at >= REFRESH_MS {
                    // What another window may have written since
                    if let Some(credentials) = s.credentials.clone() {
                        s.reading = true;
                        s.read_at = now;
                        let answered: Vec<(String, String)> = s.handed.iter().filter(|(e, _)| s.answered.contains(e)).cloned().collect();
                        wasm_bindgen_futures::spawn_local(read(credentials, answered, 1));
                    }
                }
            }
            // A change waits, or folders, or the next reading: the window looks again even when nobody moves
            if let Some(ctx) = ctx {
                ctx.request_repaint_after(Duration::from_millis(EVERY_MS as u64));
            }
        });
    }

    /// What the window holds as done of the mailbox `email`, in the server's shape.
    fn here_done(s: &State, app: &MailApp, email: &str) -> Vec<DoneEntry> {
        let waiting = s.done_waiting.get(email).map(Vec::as_slice).unwrap_or_default();
        take_done(&app.session.mailbox, app.done_items(), waiting, email, app.session.now())
    }

    /// Put `entries` as what was marked done in the mailbox `email`: each whose message is in its Archive here, the others to wait.
    fn put_done_of(s: &mut State, app: &mut MailApp, email: &str, entries: &[DoneEntry]) {
        let (items, waiting) = put_done_seen(&app.session.mailbox, email, entries, app.session.now(), &s.runs.over);
        let all = with_done(&app.session.mailbox, app.done_items(), email, items);
        app.set_done_items(all);
        s.done_waiting.insert(email.to_string(), waiting);
    }

    /// What was marked done in each mailbox, as the mailbox is now ([`refresh_done`]): the section's order stays while nothing
    /// in it changes.
    fn look_for_done(s: &mut State, app: &mut MailApp) {
        let now = app.session.now();
        for email in s.done.keys().cloned().collect::<Vec<_>>() {
            let waiting = s.done_waiting.get(&email).cloned().unwrap_or_default();
            let (all, still) = refresh_done(&app.session.mailbox, app.done_items(), &waiting, &email, now, &s.runs.over);
            if !same(&all, app.done_items()) {
                app.set_done_items(all);
            }
            s.done_waiting.insert(email, still);
        }
    }

    /// The window's rules in the server's shape, with those kept aside, as many as a mailbox keeps.
    fn here_rules(s: &State, app: &MailApp) -> Vec<Value> {
        let mut rules = take(&app.session.mailbox, &[]).rules;
        rules.extend(s.aside.iter().cloned());
        rules.truncate(MAX_RULES);
        rules
    }

    /// Put the rules `rules` into the window (their folders are listed by now), with those kept aside. Whether they were.
    fn put_all_rules(s: &mut State, app: &mut MailApp, rules: &[Value]) -> bool {
        match put_rules(&mut app.session.mailbox, rules) {
            Some(aside) => {
                s.aside = aside;
                true
            }
            None => false,
        }
    }

    /// Put what the server holds of each part into the window, unless the window changed that part since its base
    /// (then the next write merges them); and take it as the part's base. The rules wait for their folders the first
    /// time. Whether anything was put.
    fn merge(s: &mut State, app: &mut MailApp, saved: Saved, answered: &[String], now: f64) -> bool {
        let first = s.answered.first().or(answered.first()).cloned();
        let mut put = false;
        let here = take(&app.session.mailbox, answered);
        // The person's part comes from the first mailbox that answered (the same in every one)
        if first.as_ref().is_some_and(|f| answered.contains(f)) {
            match (saved.categories, s.categories.clone()) {
                (Some(server), Some(base)) if here.categories == base && server != base => {
                    app.session.mailbox.categories = server.clone();
                    s.categories = Some(server);
                    put = true;
                }
                (Some(server), None) => {
                    app.session.mailbox.categories = server.clone();
                    s.categories = Some(server);
                    put = true;
                }
                (None, None) => s.categories = Some(here.categories.clone()),
                _ => {}
            }
            let rules_here = here_rules(s, app);
            match (saved.rules, s.rules.clone()) {
                (Some(server), Some(base)) if same(&rules_here, &base) && !same(&server, &base) => {
                    if put_all_rules(s, app, &server) {
                        s.rules = Some(here_rules(s, app));
                        put = true;
                    }
                }
                (Some(server), None) => s.rules_waiting = Some((server, now)),
                (None, None) => s.rules = Some(rules_here),
                _ => {}
            }
        }
        for (email, server) in saved.senders {
            let local = here.senders.get(&email).cloned().unwrap_or_default();
            let fresh = match s.senders.get(&email) {
                None => true,
                Some(base) => local == *base && server != *base,
            };
            if fresh {
                put_senders(&mut app.session.mailbox, &email, &server);
                put = true;
            }
            if fresh || !s.senders.contains_key(&email) {
                // The base is the lists as the window keeps them (what take reads back)
                let kept = take(&app.session.mailbox, std::slice::from_ref(&email)).senders.remove(&email).unwrap_or_default();
                s.senders.insert(email, kept);
            }
        }
        // What was marked done: put back at the first reading with what was marked here meanwhile (written then, as it differs
        // from the base, the server's), and later what another window wrote, unless this one changed it too. Not an undo step:
        // the mailbox doesn't change.
        let at = app.session.now();
        for (email, server) in saved.done {
            let server = tidy_done(server, at);
            let local = here_done(s, app, &email);
            let entries = match s.done.get(&email) {
                None => Some(merge_done(&[], &server, &local, at)),
                Some(base) if local == *base && server != *base => Some(server.clone()),
                Some(_) => None,
            };
            if let Some(entries) = &entries {
                put_done_of(s, app, &email, entries);
            }
            if entries.is_some() || !s.done.contains_key(&email) {
                s.done.insert(email, server);
            }
        }
        put
    }

    /// The parts the window changed since their bases.
    fn changes(s: &State, app: &MailApp) -> Parts {
        let here = take(&app.session.mailbox, &s.answered);
        let rules = here_rules(s, app);
        let rules = match &s.rules {
            Some(base) if *base != rules => Some((base.clone(), rules)),
            _ => None,
        };
        let categories = match &s.categories {
            Some(base) if *base != here.categories => Some((base.clone(), here.categories)),
            _ => None,
        };
        let senders = here
            .senders
            .into_iter()
            .filter_map(|(email, lists)| {
                let base = s.senders.get(&email).filter(|base| **base != lists)?.clone();
                Some((email, (base, lists)))
            })
            .collect();
        let done = s
            .done
            .iter()
            .filter_map(|(email, base)| {
                let here = here_done(s, app, email);
                (here != *base).then(|| (email.clone(), (base.clone(), here)))
            })
            .collect();
        Parts { rules, categories, senders, done }
    }

    /// Take in how a write ended: what was written is the parts' base; what another window wrote since, and the window
    /// didn't change since it was sent, is put here. What was refused for good is the base too (not tried again).
    fn ended_write(s: &mut State, app: &mut MailApp, ended: Ended, now: f64) -> bool {
        let mut put = false;
        if let Some((sent, merged)) = ended.written.rules {
            if merged != sent {
                let now_here = here_rules(s, app);
                let rules = merge3(&sent, &merged, &now_here);
                if rules != now_here && put_all_rules(s, app, &rules) {
                    put = true;
                }
            }
            s.rules = Some(merged);
        }
        if let Some((sent, merged)) = ended.written.categories {
            if merged != sent {
                let now_here = app.session.mailbox.categories.clone();
                app.session.mailbox.categories = merge_categories(&sent, &merged, &now_here);
                put = true;
            }
            s.categories = Some(merged);
        }
        for (email, (sent, merged)) in ended.written.senders {
            if merged != sent {
                let now_here = take(&app.session.mailbox, std::slice::from_ref(&email)).senders.remove(&email).unwrap_or_default();
                put_senders(&mut app.session.mailbox, &email, &merge_lists(&sent, &merged, &now_here));
                put = true;
            }
            s.senders.insert(email, merged);
        }
        for (email, (sent, merged)) in ended.written.done {
            if merged != sent {
                let now_here = here_done(s, app, &email);
                let entries = merge_done(&sent, &merged, &now_here, app.session.now());
                if entries != now_here {
                    put_done_of(s, app, &email, &entries);
                }
            }
            s.done.insert(email, merged);
        }
        let refused = ended.refused;
        if !refused.is_empty() {
            log::warn!("kept settings: the server refused some of them for good; they are not tried again");
        }
        if let Some((_, sent)) = refused.rules {
            s.rules = Some(sent);
        }
        if let Some((_, sent)) = refused.categories {
            s.categories = Some(sent);
        }
        for (email, (_, sent)) in refused.senders {
            s.senders.insert(email, sent);
        }
        for (email, (_, sent)) in refused.done {
            s.done.insert(email, sent);
        }
        if ended.failed {
            s.next = now + AGAIN_MS;
        }
        put
    }

    /// Write the rules and categories to the mailbox `first` and each mailbox's lists to it. Each is read again first:
    /// what another window wrote since the base is merged with this window's ([`merge3`]), and written only if the
    /// server still holds what was read (`ifInState`), else read and merged again.
    async fn write(credentials: &HostCredentials, handed: &[(String, String)], first: &str, parts: Parts, now: i64) -> Ended {
        let mut ended = Ended::default();
        let mut emails: Vec<String> = parts.senders.keys().cloned().collect();
        for email in parts.done.keys() {
            if !emails.contains(email) {
                emails.push(email.clone());
            }
        }
        let person = parts.rules.is_some() || parts.categories.is_some();
        if person && !emails.iter().any(|e| e == first) {
            emails.insert(0, first.to_string());
        }
        for email in emails {
            let Some((_, session)) = handed.iter().find(|(e, _)| *e == email) else { continue };
            let mine = email == first;
            let rules = if mine { parts.rules.clone() } else { None };
            let categories = if mine { parts.categories.clone() } else { None };
            let lists = parts.senders.get(&email).cloned();
            let done = parts.done.get(&email).cloned();
            let refuse = |ended: &mut Ended| {
                if let Some(rules) = &rules {
                    ended.refused.rules = Some(rules.clone());
                }
                if let Some(categories) = &categories {
                    ended.refused.categories = Some(categories.clone());
                }
                if let Some(lists) = &lists {
                    ended.refused.senders.insert(email.clone(), lists.clone());
                }
                if let Some(done) = &done {
                    ended.refused.done.insert(email.clone(), done.clone());
                }
            };
            let client = match connect(credentials, &email, session).await {
                Ok(Some(client)) => client,
                // The server no longer keeps them
                Ok(None) => {
                    refuse(&mut ended);
                    continue;
                }
                Err(e) => {
                    log::warn!("kept settings: couldn't be written ({})", kind(&e));
                    ended.failed = true;
                    continue;
                }
            };
            let mut tries = 0;
            let outcome = loop {
                tries += 1;
                let outcome = write_one(&client, &email, rules.as_ref(), categories.as_ref(), lists.as_ref(), done.as_ref(), now).await;
                let again = matches!(&outcome, Err(Error::Refused { kind, .. }) if kind == "stateMismatch");
                if !again || tries >= WRITES {
                    break outcome;
                }
            };
            match outcome {
                Ok((Written::Yes, merged)) => {
                    if let (Some((_, sent)), Some(merged)) = (&rules, merged.rules) {
                        ended.written.rules = Some((sent.clone(), merged));
                    }
                    if let (Some((_, sent)), Some(merged)) = (&categories, merged.categories) {
                        ended.written.categories = Some((sent.clone(), merged));
                    }
                    if let (Some((_, sent)), Some(merged)) = (&lists, merged.senders.get(&email)) {
                        ended.written.senders.insert(email.clone(), (sent.clone(), merged.clone()));
                    }
                    if let (Some((_, sent)), Some(merged)) = (&done, merged.done.get(&email)) {
                        ended.written.done.insert(email.clone(), (sent.clone(), merged.clone()));
                    }
                }
                Ok((Written::Refused, _)) => refuse(&mut ended),
                Err(Error::Refused { kind, .. }) if for_good(&kind) => refuse(&mut ended),
                Ok((Written::Unknown, _)) | Err(_) => {
                    log::warn!("kept settings: couldn't be written");
                    ended.failed = true;
                }
            }
        }
        ended
    }

    /// One write to a mailbox: read it, merge, write if it still holds what was read. What was written (as a [`Saved`]
    /// of the parts sent), and what the server said of it. What was marked done goes only to a server that keeps it still (a
    /// server that no longer does takes it as written: it is not tried again).
    async fn write_one(
        client: &Client<crate::fetch::Fetch>,
        email: &str,
        rules: Option<&(Vec<Value>, Vec<Value>)>,
        categories: Option<&(Vec<Category>, Vec<Category>)>,
        lists: Option<&(JunkLists, JunkLists)>,
        done: Option<&(Vec<DoneEntry>, Vec<DoneEntry>)>,
        now: i64,
    ) -> Result<(Written, Saved), Error> {
        let (server, state) = get(client, email).await?;
        let mut merged = Saved::default();
        if let Some((base, here)) = rules {
            merged.rules = Some(merge3(base, server.rules.as_deref().unwrap_or(base), here));
        }
        if let Some((base, here)) = categories {
            merged.categories = Some(merge_categories(base, server.categories.as_deref().unwrap_or(base), here));
        }
        if let Some((base, here)) = lists {
            let theirs = server.senders.get(&key(email)).cloned().unwrap_or_default();
            merged.senders.insert(key(email), merge_lists(base, &theirs, here));
        }
        let keeps_done = server.done.contains_key(&key(email));
        if let Some((base, here)) = done {
            let merged_done = match server.done.get(&key(email)) {
                Some(theirs) => merge_done(base, theirs, here, now),
                None => here.clone(),
            };
            merged.done.insert(key(email), merged_done);
        }
        let done_sent = merged.done.get(&key(email)).filter(|_| keeps_done);
        let patch = update(merged.rules.as_deref(), merged.categories.as_deref(), merged.senders.get(&key(email)), done_sent.map(Vec::as_slice));
        let mut args = json!({"accountId": client.account_id(), "update": patch});
        if let (Some(state), Some(o)) = (state, args.as_object_mut()) {
            o.insert("ifInState".into(), json!(state));
        }
        let mut answers = client.call(&[CORE, SETTINGS], vec![(SET, args)]).await?;
        let answer = answers.take(0)?;
        Ok((written(&answer), merged))
    }

    /// What the mailbox's server keeps, and the state it is at.
    async fn get(client: &Client<crate::fetch::Fetch>, email: &str) -> Result<(Saved, Option<String>), Error> {
        let args = json!({"accountId": client.account_id(), "ids": [SINGLETON]});
        let mut answers = client.call(&[CORE, SETTINGS], vec![(GET, args)]).await?;
        let answer = answers.take(0)?;
        let state = answer.get("state").and_then(Value::as_str).map(str::to_string);
        let saved = read_answer(&answer, email).ok_or_else(|| Error::Protocol("no settings in the answer".into()))?;
        Ok((saved, state))
    }

    /// The mailbox `email`'s server, signed in; `None` when it doesn't offer the capability. A credential the server
    /// refuses is renewed by the page, once.
    async fn connect(credentials: &HostCredentials, email: &str, session: &str) -> Result<Option<Client<crate::fetch::Fetch>>, Error> {
        let mut renewed = false;
        loop {
            let Some(auth) = credentials.auth(email) else { return Err(Error::Auth("no credential".into())) };
            match Client::connect(crate::fetch::Fetch, session, &auth).await {
                Ok(client) => return Ok(client.has_capability(SETTINGS).then_some(client)),
                Err(Error::Auth(_)) if !renewed => {
                    renewed = true;
                    match credentials.renew(email).await {
                        Renewal::Given(_) => continue,
                        Renewal::Later | Renewal::Reload => return Err(Error::Auth("refused".into())),
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// What kind of trouble it was, never the server's words (they may name the mailbox).
    fn kind(e: &Error) -> &'static str {
        match e {
            Error::Io(_) => "network",
            Error::Tls(_) => "secure connection",
            Error::Auth(_) => "sign-in",
            Error::Busy { .. } => "busy",
            _ => "refused",
        }
    }
}
