// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what a hosting page's window keeps on its server between two
// visits (maintenance#1956).

//! What a hosting page's window keeps on its server between two visits: the rules, the categories, and each mailbox's
//! Safe Senders and Blocked Senders.
//!
//! A hosting page's tab keeps nothing of its own (hosted.rs): its mail comes back from the server at every visit, but
//! what MailCraft keeps in the mailbox itself was gone at the next one. The Neoffice server keeps them instead, under a
//! capability of its own, [`SETTINGS`] (`NeomailSettings/get` and `NeomailSettings/set`, one object, `singleton`, in each
//! mailbox): the rules and the categories are the person's (the same in every mailbox), the two lists each mailbox's.
//!
//! **Three parts and a base.** The rules, the categories and each mailbox's lists are parts of their own, each with
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
//! **Writing.** Every couple of seconds a changed part is written, the rules and categories to the first mailbox that
//! answered, each mailbox's lists to it. One the server refuses is not tried again (said once in the console); one that
//! didn't go through is tried again half a minute later. While a change waits, the page is told that leaving would lose
//! work (host_dirty.rs).
// Outside the browser only the tests use it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::BTreeMap;

use mailcraft_model::{Action, Category, Folder, Id, JunkLists, MAX_JUNK_ENTRIES, MAX_RULES, Mailbox, Rule, junk_entry};
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

/// What the window holds, in the server's shape: the rules (a folder by its mailbox and its id on the server), the
/// categories, and the lists of each mailbox handed over (by its address in lower case).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kept {
    pub rules: Vec<Value>,
    pub categories: Vec<Category>,
    pub senders: BTreeMap<String, JunkLists>,
}

/// What the server keeps, as read: `None` for a part it never kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub rules: Option<Vec<Value>>,
    pub categories: Option<Vec<Category>>,
    pub senders: BTreeMap<String, JunkLists>,
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
    Some(Saved { rules, categories, senders: BTreeMap::from([(key(email), lists)]) })
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
pub fn update(rules: Option<&[Value]>, categories: Option<&[Category]>, lists: Option<&JunkLists>) -> Value {
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
        GET, REFRESH_MS, SET, SETTINGS, SINGLETON, Saved, Written, for_good, key, merge_categories, merge_lists, merge3, put_rules, put_senders,
        read_answer, same, take, update, written,
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

    /// The parts a write sends, each as (its base, the window's).
    #[derive(Clone, Default)]
    struct Parts {
        rules: Option<(Vec<Value>, Vec<Value>)>,
        categories: Option<(Vec<Category>, Vec<Category>)>,
        senders: BTreeMap<String, (JunkLists, JunkLists)>,
    }

    impl Parts {
        fn is_empty(&self) -> bool {
            self.rules.is_none() && self.categories.is_none() && self.senders.is_empty()
        }
    }

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
        writing: bool,
        ended: Option<Ended>,
        changed: bool,
        next: f64,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    /// Whether a change waits to be written (or is being written): leaving the page now would lose it.
    pub fn pending() -> bool {
        STATE.with(|s| s.try_borrow().is_ok_and(|s| s.changed || s.writing))
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
                put |= ended_write(s, app, ended, now);
            }
            // What is put can't be undone: an Undo would bring back the window from before, and write it.
            if put {
                app.session.forget_history();
            }
            if s.writing || s.reading || s.answered.is_empty() {
                return;
            }
            let ctx = s.ctx.clone();
            if now >= s.next {
                s.next = now + EVERY_MS;
                let changes = changes(s, app);
                s.changed = !changes.is_empty();
                if s.changed {
                    if let Some(credentials) = s.credentials.clone() {
                        s.writing = true;
                        let handed = s.handed.clone();
                        let first = s.answered.first().cloned().unwrap_or_default();
                        let ctx = ctx.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let ended = write(&credentials, &handed, &first, changes).await;
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
        Parts { rules, categories, senders }
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
        if ended.failed {
            s.next = now + AGAIN_MS;
        }
        put
    }

    /// Write the rules and categories to the mailbox `first` and each mailbox's lists to it. Each is read again first:
    /// what another window wrote since the base is merged with this window's ([`merge3`]), and written only if the
    /// server still holds what was read (`ifInState`), else read and merged again.
    async fn write(credentials: &HostCredentials, handed: &[(String, String)], first: &str, parts: Parts) -> Ended {
        let mut ended = Ended::default();
        let mut emails: Vec<String> = parts.senders.keys().cloned().collect();
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
                let outcome = write_one(&client, &email, rules.as_ref(), categories.as_ref(), lists.as_ref()).await;
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
    /// of the parts sent), and what the server said of it.
    async fn write_one(
        client: &Client<crate::fetch::Fetch>,
        email: &str,
        rules: Option<&(Vec<Value>, Vec<Value>)>,
        categories: Option<&(Vec<Category>, Vec<Category>)>,
        lists: Option<&(JunkLists, JunkLists)>,
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
        let patch = update(merged.rules.as_deref(), merged.categories.as_deref(), merged.senders.get(&key(email)));
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
