// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the whole messages a person opened that are too heavy for the copy of
// //// Neoffice — their mailboxes, kept apart in this browser so that opening one again doesn't download it again.

//! The heavy messages a person opened, whole, as their server gave them, in a store of their own of the browser's IndexedDB
//! ([`STORE`], beside the copy of the mailboxes: [`super::cache`]).
//!
//! The copy is one value written again every few seconds: it can't hold what a heavy message weighs, and a message past its
//! bounds was downloaded again at every opening. Here each message is one record, written once, in its bytes (no base64),
//! read only when it is asked for.
//!
//! - **Where it comes in**: between the window and the server, in the runner of a fetch ([`answered`]). What is asked for is
//!   looked for here first; only the rest is asked of the server, and a message found here needs no sign-in and no network.
//!   The window can't tell the difference: it takes the same MIME the server gave.
//! - **What is kept**: what the person opened ([`JmapFetchRequest::background`] false) and weighs [`HEAVY`] at least; a lighter
//!   one is the copy's. The background reads what is here but keeps nothing: this is what was opened.
//! - **How much**: [`MAX_BYTES`], and never more than half of what the browser says it still has for this site ([`room`]). Past
//!   it, the least recently opened go first ([`Index::keep`]).
//! - **Whose**: a key names the copy's key ([`super::cache::key`], the person and their mailboxes), the mailbox and the
//!   message's id on the server. Kept only where the page names the person; at each start everything else is deleted
//!   ([`foreign`]), and a sign-out's `deleteDatabase` takes it with the copy.
//! - **Until when**: a message the server says is gone is forgotten when it says so, and one that leaves its mailbox during a
//!   visit (deleted, or moved: it has another id then) as it leaves ([`left_the_mailbox`]). One that is merely not among the
//!   messages a run reads of its folder is still on the server, and stays until it is the least recently opened. An id names
//!   one message and its content for good (RFC 8621; the Neoffice facade's carries the folder's UIDVALIDITY), so what is kept
//!   is never out of date.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use mailcraft_model::Mailbox;
use mailcraft_model::sync::{GraphFetchResult, GraphFetched, JmapFetchRequest};
use mailcraft_ui_egui::jmap_job::JmapFetchJob;
use serde_json::{Value, json};

/// The store of the browser's database ([`super::cache::DATABASE`]).
pub const STORE: &str = "contents";
/// The most that is kept, in bytes: five messages of the largest a fetch brings (50 MB).
pub const MAX_BYTES: u64 = 256 << 20;
/// What a message opened weighs at least to be kept here, in bytes.
pub const HEAVY: usize = 1 << 20;
/// What separates the parts of a key.
const SEPARATOR: char = '\u{1f}';

/// What names a mailbox in a key: a digest of its address and its session, so that no address is written in clear.
pub fn mailbox_tag(email: &str, session: &str) -> String {
    super::cache::digest(&format!("{}{SEPARATOR}{}", email.trim().to_lowercase(), session.trim()))
}

/// The key of the message `id` of the mailbox `tag`, for the copy `scope`.
pub fn record_key(scope: &str, tag: &str, id: &str) -> String {
    format!("{scope}{SEPARATOR}{tag}{SEPARATOR}{id}")
}

/// The key of what is known of the records of `scope`.
pub fn index_key(scope: &str) -> String {
    format!("{scope}{SEPARATOR}index")
}

/// A record: its key, its bytes, and when it was last opened (seconds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub size: u64,
    pub at: i64,
}

/// What is kept for a copy: read and written in the same transaction as the records it names, so that two tabs never lose each
/// other's, and so that a record's size and age are known without reading its bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index(pub Vec<Entry>);

impl Index {
    /// The index as written; nothing when it can't be read (its records are then swept: [`foreign`]).
    pub fn parse(text: &str) -> Index {
        let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else { return Index::default() };
        Index(
            list.iter()
                .filter_map(|e| {
                    let (key, size, at) = (e.get(0)?.as_str()?, e.get(1)?.as_u64()?, e.get(2)?.as_i64()?);
                    Some(Entry { key: key.to_string(), size, at })
                })
                .collect(),
        )
    }

    pub fn to_text(&self) -> String {
        Value::Array(self.0.iter().map(|e| json!([e.key, e.size, e.at])).collect()).to_string()
    }

    pub fn has(&self, key: &str) -> bool {
        self.0.iter().any(|e| e.key == key)
    }

    /// What the records weigh.
    pub fn used(&self) -> u64 {
        self.0.iter().fold(0, |sum, e| sum.saturating_add(e.size))
    }

    /// The record `key` was opened at `at`.
    pub fn touch(&mut self, key: &str, at: i64) {
        if let Some(e) = self.0.iter_mut().find(|e| e.key == key) {
            e.at = e.at.max(at);
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.0.retain(|e| e.key != key);
    }

    /// `key`, of `size` bytes, opened at `at`, within `room` bytes: the records to delete for it, the least recently opened
    /// first. `None` when it can't be kept: it is heavier than all the room, and nothing is deleted for it.
    pub fn keep(&mut self, key: &str, size: u64, at: i64, room: u64) -> Option<Vec<String>> {
        if size > room {
            return None;
        }
        self.remove(key);
        self.0.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.key.cmp(&b.key)));
        let mut gone = Vec::new();
        while self.used().saturating_add(size) > room && !self.0.is_empty() {
            gone.push(self.0.remove(0).key);
        }
        self.0.push(Entry { key: key.to_string(), size, at });
        Some(gone)
    }
}

/// How many bytes may be kept, `ours` being kept now, with what the browser says of this site (`usage` and `quota`, when it
/// says): [`MAX_BYTES`], and never more than half of what it still has, so that the copy and the page keep their room.
pub fn room(ours: u64, estimate: Option<(u64, u64)>) -> u64 {
    match estimate {
        Some((usage, quota)) if quota > 0 => MAX_BYTES.min(ours.saturating_add(quota.saturating_sub(usage) / 2)),
        _ => MAX_BYTES,
    }
}

/// Of the keys of the store, those to delete: all those of another copy (all of them without a `scope`), and those of this one
/// that its index doesn't name.
pub fn foreign(keys: &[String], scope: Option<&str>, index: &Index) -> Vec<String> {
    let Some(scope) = scope else { return keys.to_vec() };
    let (prefix, own_index) = (format!("{scope}{SEPARATOR}"), index_key(scope));
    keys.iter().filter(|k| !k.starts_with(&prefix) || (**k != own_index && !index.has(k))).cloned().collect()
}

/// Each mailbox of a server in `mailbox`, with the ids on that server of the messages it holds.
pub fn live(mailbox: &Mailbox) -> Vec<(String, HashSet<String>)> {
    mailbox
        .accounts
        .iter()
        .filter_map(|a| {
            let server = a.jmap.as_ref()?;
            let ids = mailbox
                .messages
                .values()
                .filter(|m| m.account == a.id)
                .filter_map(|m| m.server.as_ref().filter(|s| !s.gone).map(|s| s.id.clone()))
                .collect();
            Some((mailbox_tag(&a.email, &server.session), ids))
        })
        .collect()
}

/// The records of `scope` for the messages that were in their mailbox (`before`: [`live`]) and no longer are (`now`): deleted or
/// moved. One that was never seen here says nothing, and a mailbox not looked at now neither.
pub fn left_the_mailbox(scope: &str, before: &[(String, HashSet<String>)], now: &[(String, HashSet<String>)]) -> Vec<String> {
    let mut gone = Vec::new();
    for (tag, was) in before {
        if let Some((_, is)) = now.iter().find(|(t, _)| t == tag) {
            gone.extend(was.difference(is).map(|id| record_key(scope, tag, id)));
        }
    }
    gone
}

/// Of `keys`, those the browser's store may hold: all of them until what it keeps is known (`known`).
pub fn worth_asking(keys: &[String], known: Option<&HashSet<String>>) -> Vec<String> {
    keys.iter().filter(|key| known.is_none_or(|known| known.contains(*key))).cloned().collect()
}

/// Where the records are kept: the browser's store, or a test's.
pub trait Shelf {
    /// The bytes of those of `keys` that are kept; each is marked opened at `opened`, when it says.
    fn take(&self, keys: &[String], opened: Option<i64>) -> impl Future<Output = Vec<(String, Vec<u8>)>>;
    /// Keep `mime` under `key`, opened at `at`. Nobody waits for it.
    fn keep(&self, key: &str, mime: &[u8], at: i64);
    /// Forget `keys`. Nobody waits for it.
    fn forget(&self, keys: &[String]);
}

/// What a fetch answers, from `shelf` first and from `server` for the rest (with the same request, for those alone; not at all
/// when every message is here). What the person opened and is heavy is kept; what the server says is gone is forgotten.
pub async fn answered<S, F, W>(shelf: &S, scope: &str, job: JmapFetchJob, now: i64, server: F) -> GraphFetchResult
where
    S: Shelf,
    F: FnOnce(JmapFetchJob) -> W,
    W: Future<Output = GraphFetchResult>,
{
    let JmapFetchJob { request, secret, cancel } = job;
    let tag = mailbox_tag(&request.email, &request.session);
    let key_of = |id: &str| record_key(scope, &tag, id);
    let opened = !request.background;
    let keys: Vec<String> = request.ids.iter().map(|id| key_of(id)).collect();
    let mut here: HashMap<String, Vec<u8>> = shelf.take(&keys, opened.then_some(now)).await.into_iter().collect();
    let mut found = Vec::new();
    let mut rest = Vec::new();
    for (id, key) in request.ids.iter().zip(&keys) {
        match here.remove(key) {
            Some(mime) => found.push(GraphFetched { id: id.clone(), mime }),
            None => rest.push(id.clone()),
        }
    }
    if rest.is_empty() && !found.is_empty() {
        return GraphFetchResult { account: request.account, email: request.email, session: request.session, fetched: found, ..Default::default() };
    }
    let asked = JmapFetchRequest { ids: rest, ..request };
    let mut answer = server(JmapFetchJob { request: asked, secret, cancel }).await;
    if opened {
        for message in answer.fetched.iter().filter(|m| m.mime.len() >= HEAVY) {
            shelf.keep(&key_of(&message.id), &message.mime, now);
        }
    }
    let gone: Vec<String> = answer.missing.iter().map(|id| key_of(id)).collect();
    if !gone.is_empty() {
        shelf.forget(&gone);
    }
    answer.fetched.splice(0..0, found);
    answer
}

#[cfg(target_arch = "wasm32")]
pub use browser::{Browser, forget_others, scope, set_scope};

/// The browser's store. Each use opens the database and closes it (as the copy does), and does all it does in one transaction.
#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;
    use std::future::Future;
    use std::rc::Rc;

    use wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_futures::{JsFuture, future_to_promise, spawn_local};

    use super::super::cache::idb::{after, finished, open, settle};
    use super::{Index, STORE, Shelf, foreign, index_key, room, worth_asking};

    /// How long a fetch waits for the store, in milliseconds: past it, the server is asked, and the store is left alone for the
    /// rest of the visit (a browser whose database doesn't answer must not hold up the mail).
    const TAKE_WAIT_MS: i32 = 5_000;

    thread_local! {
        /// The copy whose records this tab reads and writes: none where the page names nobody, or keeps no copy.
        static SCOPE: RefCell<Option<String>> = const { RefCell::new(None) };
        /// The keys kept, once they are known: a message that isn't among them costs no visit to the database.
        static KNOWN: RefCell<Option<HashSet<String>>> = const { RefCell::new(None) };
        static STALLED: Cell<bool> = const { Cell::new(false) };
    }

    pub fn set_scope(scope: Option<String>) {
        SCOPE.with(|s| *s.borrow_mut() = scope);
    }

    pub fn scope() -> Option<String> {
        SCOPE.with(|s| s.borrow().clone())
    }

    fn in_store(db: &web_sys::IdbDatabase, mode: web_sys::IdbTransactionMode) -> Result<(web_sys::IdbTransaction, web_sys::IdbObjectStore), JsValue> {
        let tx = db.transaction_with_str_and_mode(STORE, mode)?;
        let store = tx.object_store(STORE)?;
        Ok((tx, store))
    }

    fn writing(db: &web_sys::IdbDatabase) -> Result<(web_sys::IdbTransaction, web_sys::IdbObjectStore), JsValue> {
        in_store(db, web_sys::IdbTransactionMode::Readwrite)
    }

    async fn index_in(store: &web_sys::IdbObjectStore, key: &str) -> Result<Index, JsValue> {
        let value = JsFuture::from(settle(&store.get(&JsValue::from_str(key))?)).await?;
        Ok(value.as_string().map(|text| Index::parse(&text)).unwrap_or_default())
    }

    fn put_index(store: &web_sys::IdbObjectStore, key: &str, index: &Index) -> Result<(), JsValue> {
        store.put_with_key(&JsValue::from_str(&index.to_text()), &JsValue::from_str(key)).map(|_| ())
    }

    /// (usage, quota) of this site, when the browser says (`navigator.storage`: none on a page that isn't secure).
    async fn estimate() -> Option<(u64, u64)> {
        let navigator = web_sys::window()?.navigator();
        let storage = js_sys::Reflect::get(navigator.as_ref(), &"storage".into()).ok().filter(|s| !s.is_undefined())?;
        let ask: js_sys::Function = js_sys::Reflect::get(&storage, &"estimate".into()).ok()?.dyn_into().ok()?;
        let promise: js_sys::Promise = ask.call0(&storage).ok()?.dyn_into().ok()?;
        let answer = JsFuture::from(promise).await.ok()?;
        let number = |name: &str| js_sys::Reflect::get(&answer, &name.into()).ok().and_then(|v| v.as_f64());
        Some((number("usage")? as u64, number("quota")? as u64))
    }

    /// The records of `keys` that are kept, as soon as they are read: marking them opened (`opened`) goes on behind, and what
    /// was read is answered even if that doesn't go through. Reading alone (the background) takes no write lock.
    async fn take_in(db: &web_sys::IdbDatabase, index: &str, keys: &[String], opened: Option<i64>) -> Result<Vec<(String, Vec<u8>)>, JsValue> {
        let mode = if opened.is_some() { web_sys::IdbTransactionMode::Readwrite } else { web_sys::IdbTransactionMode::Readonly };
        let (tx, store) = in_store(db, mode)?;
        let mut kept = index_in(&store, index).await?;
        let wanted: Vec<&String> = keys.iter().filter(|k| kept.has(k)).collect();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        // All asked before anything is awaited: the transaction is active for each.
        let mut answers = Vec::with_capacity(wanted.len());
        for key in wanted {
            answers.push((key, settle(&store.get(&JsValue::from_str(key))?)));
        }
        let (mut out, mut changed) = (Vec::new(), false);
        for (key, answer) in answers {
            match JsFuture::from(answer).await.map(|value| value.dyn_into::<js_sys::Uint8Array>()) {
                Ok(Ok(bytes)) => {
                    out.push((key.clone(), bytes.to_vec()));
                    if let Some(at) = opened {
                        kept.touch(key, at);
                        changed = true;
                    }
                }
                // Its bytes are gone: it is fetched again, and forgotten when the store may be written.
                Ok(Err(_)) => {
                    kept.remove(key);
                    changed = opened.is_some();
                }
                Err(e) => log::warn!("a message kept in this browser couldn't be read: {e:?}"),
            }
        }
        if changed {
            match put_index(&store, index, &kept) {
                Ok(()) => {
                    let end = finished(&tx);
                    spawn_local(async move {
                        if let Err(e) = JsFuture::from(end).await {
                            log::warn!("the messages kept in this browser couldn't be marked opened: {e:?}");
                        }
                    });
                }
                Err(e) => log::warn!("the messages kept in this browser couldn't be marked opened: {e:?}"),
            }
        }
        Ok(out)
    }

    async fn keep_in(
        db: &web_sys::IdbDatabase,
        index: &str,
        key: &str,
        bytes: &js_sys::Uint8Array,
        at: i64,
        said: Option<(u64, u64)>,
    ) -> Result<(), JsValue> {
        let (tx, store) = writing(db)?;
        let mut kept = index_in(&store, index).await?;
        let Some(gone) = kept.keep(key, u64::from(bytes.length()), at, room(kept.used(), said)) else { return Ok(()) };
        for old in gone {
            store.delete(&JsValue::from_str(&old))?;
        }
        store.put_with_key(bytes, &JsValue::from_str(key))?;
        put_index(&store, index, &kept)?;
        JsFuture::from(finished(&tx)).await.map(|_| ())
    }

    async fn forget_in(db: &web_sys::IdbDatabase, index: &str, keys: &[String]) -> Result<(), JsValue> {
        let (tx, store) = writing(db)?;
        let mut kept = index_in(&store, index).await?;
        for key in keys {
            kept.remove(key);
            store.delete(&JsValue::from_str(key))?;
        }
        put_index(&store, index, &kept)?;
        JsFuture::from(finished(&tx)).await.map(|_| ())
    }

    /// Run `work` on the database, and say in the console (never what it holds) when it didn't go through.
    fn in_the_background<W: Future<Output = Result<(), JsValue>> + 'static>(work: W) {
        spawn_local(async move {
            if let Err(e) = work.await {
                log::warn!("the messages kept in this browser couldn't be changed: {e:?}");
            }
        });
    }

    /// The browser's store, for the copy `scope`.
    pub struct Browser {
        pub scope: String,
    }

    impl Shelf for Browser {
        async fn take(&self, keys: &[String], opened: Option<i64>) -> Vec<(String, Vec<u8>)> {
            if STALLED.get() {
                return Vec::new();
            }
            // What the person opens is always looked for (another tab may have kept it); the background, only if it is known here.
            let wanted = if opened.is_some() { keys.to_vec() } else { KNOWN.with(|known| worth_asking(keys, known.borrow().as_ref())) };
            if wanted.is_empty() {
                return Vec::new();
            }
            // Raced with the clock, as the copy is read: a database that doesn't answer never holds up a message.
            let answer = Rc::new(RefCell::new(None));
            let (given, index) = (answer.clone(), index_key(&self.scope));
            let reading = future_to_promise(async move {
                let db = open().await?;
                let read = take_in(&db, &index, &wanted, opened).await;
                db.close();
                *given.borrow_mut() = Some(read?);
                Ok(JsValue::NULL)
            });
            let raced = js_sys::Promise::race(&js_sys::Array::of2(&reading, &after(TAKE_WAIT_MS)));
            if let Err(e) = JsFuture::from(raced).await {
                log::warn!("the messages kept in this browser couldn't be read: {e:?}");
                return Vec::new();
            }
            let read = answer.borrow_mut().take();
            read.unwrap_or_else(|| {
                log::warn!("the messages kept in this browser didn't come in time: they are left alone for this visit");
                STALLED.set(true);
                Vec::new()
            })
        }

        fn keep(&self, key: &str, mime: &[u8], at: i64) {
            if STALLED.get() {
                return;
            }
            KNOWN.with(|known| {
                if let Some(known) = known.borrow_mut().as_mut() {
                    known.insert(key.to_string());
                }
            });
            // (Copied now: the window takes the message as it is.)
            let (index, key, bytes) = (index_key(&self.scope), key.to_string(), js_sys::Uint8Array::from(mime));
            in_the_background(async move {
                // (Before the transaction: awaiting anything else would end it.)
                let said = estimate().await;
                let db = open().await?;
                let kept = keep_in(&db, &index, &key, &bytes, at, said).await;
                db.close();
                kept
            });
        }

        fn forget(&self, keys: &[String]) {
            if STALLED.get() {
                return;
            }
            // (What this tab doesn't know of is another's to forget: it sees the same mailbox.)
            let keys = KNOWN.with(|known| {
                let mut known = known.borrow_mut();
                let keys = worth_asking(keys, known.as_ref());
                if let Some(known) = known.as_mut() {
                    keys.iter().for_each(|key| {
                        known.remove(key);
                    });
                }
                keys
            });
            if keys.is_empty() {
                return;
            }
            let index = index_key(&self.scope);
            in_the_background(async move {
                let db = open().await?;
                let done = forget_in(&db, &index, &keys).await;
                db.close();
                done
            });
        }
    }

    /// Everything but what the current scope names is deleted (another person's, a set of mailboxes no longer handed over), and
    /// what it keeps is known from now on.
    pub fn forget_others() {
        let scope = scope();
        in_the_background(async move {
            let db = open().await?;
            let done = async {
                let (tx, store) = writing(&db)?;
                let keys = settle(&store.get_all_keys()?);
                let index = match &scope {
                    Some(scope) => settle(&store.get(&JsValue::from_str(&index_key(scope)))?),
                    None => js_sys::Promise::resolve(&JsValue::UNDEFINED),
                };
                // (Both awaited before either answer is looked at: neither is left without a taker.)
                let (keys, index) = (JsFuture::from(keys).await, JsFuture::from(index).await);
                let keys: Vec<String> = js_sys::Array::from(&keys?).iter().filter_map(|k| k.as_string()).collect();
                let kept = index?.as_string().map(|text| Index::parse(&text)).unwrap_or_default();
                for key in foreign(&keys, scope.as_deref(), &kept) {
                    store.delete(&JsValue::from_str(&key))?;
                }
                JsFuture::from(finished(&tx)).await?;
                KNOWN.with(|known| *known.borrow_mut() = Some(kept.0.into_iter().map(|e| e.key).collect()));
                Ok(())
            }
            .await;
            db.close();
            done
        });
    }
}

#[cfg(test)]
#[path = "contents_tests.rs"]
mod tests;
