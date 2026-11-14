// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a hosting page's mailboxes as the last visit left them, kept in this
// //// Neoffice — browser to be shown at once at the next visit.

//! A hosting page's mailboxes (Neoffice's `/neomail`, Neodesk's window) as the last visit left them: their folders, the
//! messages' headers and previews, the counts, and the bodies of the newest up to [`BODIES`], kept in the browser's
//! IndexedDB (never its `localStorage`, a few megabytes) so that the next visit shows them at once and its runners only
//! bring what changed since (the server has no `Email/changes`: they read their window again, the list shown
//! meanwhile).
//!
//! - **Whose**: the copy is the signed-in person's and these mailboxes' ([`key`]: the person the page names,
//!   `person`, and each mailbox's address and session), and it is read only when it holds exactly the mailboxes the page
//!   hands over ([`holds`]). At each start every other copy kept in this browser is deleted ([`stale`]): another person
//!   signed in on the same browser never finds the last one's.
//! - **How much**: never more than [`MAX_BYTES`] once written; of each folder the newest messages a run reads again
//!   (its window: what is in the copy is put right by the next run), the bodies of the newest until [`BODIES`] of them,
//!   the others headers and preview (fetched again when opened).
//! - **Nothing that would be done twice**: no Outbox (a message sent since would go again), no Drafts (the server's are
//!   read), no change still to tell the server (moves, deletions, folder changes): a copy shows, it never acts.
//! - **Then**: the window's first Send/Receive reads every mailbox (not the Inbox alone): what moved elsewhere since
//!   the last visit leaves the folders it was in. A message destroyed elsewhere in a folder larger than the window stays
//!   until the server can say what changed (`Email/changes`).
//! - **When**: read once as the window starts, [`READ_WAIT_MS`] at most (else the window starts without it); written
//!   every [`EVERY_MS`] while the mailbox changes, deleted when it can't be written, and none where the page says so
//!   (`cache: false`, which deletes any kept). A copy that can't be read is ignored (and written over).
//! - **Nothing secret**: a mailbox never holds a password or a token (the page hands them to this tab's memory).
//!
//! A page that signs the person out should delete the database ([`DATABASE`]): `indexedDB.deleteDatabase("neomail")`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use mailcraft_model::{FolderKind, Id, Mailbox, Message};

use crate::hosted::HostAccount;

/// The browser's database the copies are kept in, and its store.
pub const DATABASE: &str = "neomail";
pub const STORE: &str = "mailboxes";
/// What a key starts with: a copy of another shape is never read.
const FORMAT: &str = "v1";
/// The bodies kept, newest message first, up to this many bytes.
pub const BODIES: u64 = 8 << 20;
/// The most a copy weighs once written: a heavier one is written with no body, and not at all when still too heavy.
pub const MAX_BYTES: usize = 48 << 20;
/// How often a changing mailbox is written, in milliseconds.
pub const EVERY_MS: f64 = 15_000.0;
/// How long the window waits for the copy as it starts, in milliseconds: past it, it starts without.
pub const READ_WAIT_MS: i32 = 1_500;

/// The key of the copy of `accounts` for `person` (the page's `person`, none where it names nobody): the same whatever
/// the order of the mailboxes or the case of their addresses, another for another person or another set of mailboxes.
/// A digest: no address is written in clear in the key.
pub fn key(person: Option<&str>, accounts: &[HostAccount]) -> String {
    let mut boxes: Vec<String> = accounts.iter().map(|a| format!("{}\u{1f}{}", a.email.trim().to_lowercase(), a.session.trim())).collect();
    boxes.sort();
    boxes.dedup();
    let who = person.map(|p| p.trim().to_lowercase()).unwrap_or_default();
    let said = format!("{who}\u{1e}{}", boxes.join("\u{1e}"));
    format!("{FORMAT}:{:016x}{:016x}", fnv1a(said.as_bytes(), 0xcbf2_9ce4_8422_2325), fnv1a(said.as_bytes(), 0x84222325_cbf29ce4))
}

/// FNV-1a over `bytes` from `seed`: two of them make the key's 128 bits.
fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    bytes.iter().fold(seed, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

/// Of the keys kept in this browser, those to delete: every one but `current`.
pub fn stale(kept: &[String], current: &str) -> Vec<String> {
    kept.iter().filter(|k| k.as_str() != current).cloned().collect()
}

/// Whether `mailbox` holds exactly the mailboxes the page hands over: one JMAP account for each, at its session, no
/// other JMAP account, and no account of another kind but the empty local one every hosted session has.
pub fn holds(mailbox: &Mailbox, accounts: &[HostAccount]) -> bool {
    let foreign = mailbox
        .accounts
        .iter()
        .any(|a| a.jmap.is_none() && (!a.email.is_empty() || a.incoming.is_some() || a.outgoing.is_some() || a.ews.is_some() || a.oauth.is_some()));
    if foreign {
        return false;
    }
    let kept: BTreeSet<(String, String)> =
        mailbox.accounts.iter().filter_map(|a| a.jmap.as_ref().map(|j| (a.email.trim().to_lowercase(), j.session.trim().to_string()))).collect();
    let handed: BTreeSet<(String, String)> = accounts.iter().map(|a| (a.email.trim().to_lowercase(), a.session.trim().to_string())).collect();
    !handed.is_empty() && kept == handed
}

/// What is kept of `mailbox`: of each folder of its JMAP accounts the newest `window` messages (what the next run reads
/// again and puts right; older ones it would never remove once deleted elsewhere), the bodies of the newest up to
/// `bodies` bytes of them and the others headers and preview (fetched again from the server when opened); no Outbox, no
/// Drafts, no draft, no message being sent, and no change still to tell a server. The local account's, if any, whole.
pub fn snapshot(mailbox: &Mailbox, bodies: u64, window: usize) -> Mailbox {
    let mut kept = mailbox.clone();
    kept.pending.clear();
    kept.queued.clear();
    let served: HashSet<Id> = kept.accounts.iter().filter(|a| a.jmap.is_some()).map(|a| a.id).collect();
    let kinds: HashMap<Id, FolderKind> = kept.folders.iter().map(|f| (f.id, f.kind)).collect();
    let gone = |m: &Message| m.draft || m.sending || matches!(kinds.get(&m.folder), Some(FolderKind::Drafts | FolderKind::Outbox));
    // Of each folder of a server, the newest `window`.
    let mut by_folder: HashMap<Id, Vec<(i64, Id)>> = HashMap::new();
    for m in kept.messages.values().filter(|m| served.contains(&m.account)) {
        by_folder.entry(m.folder).or_default().push((m.received, m.id));
    }
    let mut out: HashSet<Id> = HashSet::new();
    for list in by_folder.values_mut() {
        list.sort_by(|a, b| b.cmp(a));
        out.extend(list.iter().skip(window).map(|(_, id)| *id));
    }
    kept.messages.retain(|id, m| !out.contains(id) && !(served.contains(&m.account) && gone(m.as_ref())));
    let mut newest: Vec<_> =
        kept.messages.values().filter(|m| served.contains(&m.account) && !m.unfetched).map(|m| (m.received, m.id, m.size())).collect();
    newest.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut used = 0u64;
    for (_, id, size) in newest {
        if used.saturating_add(size) <= bodies {
            used += size;
            continue;
        }
        if let Some(m) = kept.messages.get_mut(&id) {
            Arc::make_mut(m).to_fetch_again();
        }
    }
    for line in kept.repair() {
        log::info!("the copy of the mailboxes put right: {line}");
    }
    kept
}

/// The copy of `mailbox` as written: with the newest bodies up to `bodies` bytes, else with none, as long as it weighs
/// `max` bytes at most; `None` when even that is heavier.
pub fn encode_within(mailbox: &Mailbox, bodies: u64, window: usize, max: usize) -> Option<Vec<u8>> {
    for budget in [bodies, 0] {
        let bytes = mailcraft_engine::io::write_mailbox(&snapshot(mailbox, budget, window)).ok()?;
        if bytes.len() <= max {
            return Some(bytes);
        }
    }
    None
}

/// The copy of `mailbox` as written ([`encode_within`] [`BODIES`], [`MAX_BYTES`]), each folder's newest `window`.
pub fn encode(mailbox: &Mailbox, window: usize) -> Option<Vec<u8>> {
    encode_within(mailbox, BODIES, window, MAX_BYTES)
}

/// The mailbox a copy holds, when it can be read and holds exactly `accounts`; `None` otherwise (said in the console
/// when it can't be read, never what it holds).
pub fn decode(bytes: &[u8], accounts: &[HostAccount]) -> Option<Mailbox> {
    match mailcraft_engine::io::read_mailbox(bytes) {
        Ok(mailbox) if holds(&mailbox, accounts) => Some(mailbox),
        Ok(_) => None,
        // (Not why: the reason may quote what it holds.)
        Err(_) => {
            log::warn!("the copy of the mailboxes kept in this browser couldn't be read, and is set aside");
            None
        }
    }
}

/// When the copy is written next: once the mailbox has changed since the last copy, [`EVERY_MS`] at least after it.
#[derive(Clone, Debug, Default)]
pub struct Writer {
    pub key: String,
    written_rev: Option<u64>,
    written_at: f64,
    pub writing: bool,
}

impl Writer {
    /// The copy of `key`, the mailbox at `rev` as the window starts (what the copy read holds, or nothing yet).
    pub fn new(key: String, now: f64, rev: u64) -> Writer {
        Writer { key, written_rev: Some(rev), written_at: now, writing: false }
    }

    /// Whether the mailbox at `rev` is to be written now.
    pub fn due(&self, rev: u64, now: f64) -> bool {
        !self.writing && self.written_rev != Some(rev) && now - self.written_at >= EVERY_MS
    }

    /// The mailbox at `rev` is being written.
    pub fn started(&mut self, rev: u64, now: f64) {
        self.written_rev = Some(rev);
        self.written_at = now;
        self.writing = true;
    }
}

#[cfg(target_arch = "wasm32")]
pub use idb::{forget_all, read, start, tick};

/// The browser's IndexedDB: a copy read, written, and the others deleted. Each use opens the database and closes it
/// whatever happens, and closes it as soon as a page asks to delete it (`versionchange`), so that a sign-out's
/// `deleteDatabase` is never kept waiting by this tab.
#[cfg(target_arch = "wasm32")]
mod idb {
    use std::cell::RefCell;

    use mailcraft_ui_egui::MailApp;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_futures::JsFuture;

    use super::{DATABASE, READ_WAIT_MS, STORE, Writer, encode, stale};

    thread_local! {
        /// The copy this tab writes, once the window has its mailboxes.
        static WRITER: RefCell<Option<Writer>> = const { RefCell::new(None) };
        /// The window opened on a copy: its first Send/Receive reads every mailbox, not the Inbox alone, so that what
        /// moved elsewhere since the last visit leaves the folders it was in.
        static FULL_RUN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// A request's answer as a promise: its result, or its error.
    fn settle(request: &web_sys::IdbRequest) -> js_sys::Promise {
        let request = request.clone();
        js_sys::Promise::new(&mut |resolve, reject| {
            let done = request.clone();
            let ok = Closure::once_into_js(move || {
                let _ = resolve.call1(&JsValue::NULL, &done.result().unwrap_or(JsValue::NULL));
            });
            let failed = request.clone();
            let no = Closure::once_into_js(move || {
                let why = failed.error().ok().flatten().map(JsValue::from).unwrap_or(JsValue::NULL);
                let _ = reject.call1(&JsValue::NULL, &why);
            });
            request.set_onsuccess(Some(ok.unchecked_ref()));
            request.set_onerror(Some(no.unchecked_ref()));
        })
    }

    /// A transaction's end as a promise: kept (`complete`), or not (`abort`, `error`: a full store says so here).
    fn finished(tx: &web_sys::IdbTransaction) -> js_sys::Promise {
        let tx = tx.clone();
        js_sys::Promise::new(&mut |resolve, reject| {
            let ok = Closure::once_into_js(move || {
                let _ = resolve.call0(&JsValue::NULL);
            });
            let no = Closure::once_into_js(move || {
                let _ = reject.call0(&JsValue::NULL);
            });
            tx.set_oncomplete(Some(ok.unchecked_ref()));
            tx.set_onabort(Some(no.unchecked_ref()));
            tx.set_onerror(Some(no.unchecked_ref()));
        })
    }

    async fn open() -> Result<web_sys::IdbDatabase, JsValue> {
        let factory = web_sys::window().ok_or("no window")?.indexed_db()?.ok_or("no IndexedDB")?;
        let request = factory.open_with_u32(DATABASE, 1)?;
        let made = request.clone();
        let upgrade = Closure::once_into_js(move || {
            if let Ok(db) = made.result() {
                let db: web_sys::IdbDatabase = db.unchecked_into();
                if !db.object_store_names().contains(STORE) {
                    let _ = db.create_object_store(STORE);
                }
            }
        });
        request.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
        let db: web_sys::IdbDatabase = JsFuture::from(settle(&request)).await?.unchecked_into();
        // A page deleting the database (a sign-out) is never kept waiting by this tab.
        let closing = db.clone();
        let close = Closure::<dyn FnMut()>::new(move || closing.close());
        db.set_onversionchange(Some(close.as_ref().unchecked_ref()));
        close.forget();
        Ok(db)
    }

    /// `ms` milliseconds from now, as a promise.
    fn after(ms: i32) -> js_sys::Promise {
        js_sys::Promise::new(&mut |resolve, _| {
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
            }
        })
    }

    /// The keys kept, every other than `key` deleted, and the copy under `key`: in one transaction.
    async fn read_in(db: &web_sys::IdbDatabase, key: &str) -> Result<JsValue, JsValue> {
        let tx = db.transaction_with_str_and_mode(STORE, web_sys::IdbTransactionMode::Readwrite)?;
        let store = tx.object_store(STORE)?;
        // Both requests are made before anything is awaited: the transaction is still active for each.
        let keys = settle(&store.get_all_keys()?);
        let copy = settle(&store.get(&JsValue::from_str(key))?);
        let kept: Vec<String> = js_sys::Array::from(&JsFuture::from(keys).await?).iter().filter_map(|k| k.as_string()).collect();
        let value = JsFuture::from(copy).await?;
        // (Deleting from inside the answers' turn: the transaction is active again there.)
        for gone in stale(&kept, key) {
            let _ = store.delete(&JsValue::from_str(&gone));
        }
        Ok(value)
    }

    /// The copy kept under `key`, if any, within [`READ_WAIT_MS`]; and every other copy of this browser deleted.
    pub async fn read(key: &str) -> Option<Vec<u8>> {
        let key = key.to_string();
        let reading = wasm_bindgen_futures::future_to_promise(async move {
            let db = open().await?;
            let read = read_in(&db, &key).await;
            db.close();
            read
        });
        let raced = js_sys::Promise::race(&js_sys::Array::of2(&reading, &after(READ_WAIT_MS)));
        match JsFuture::from(raced).await {
            Ok(value) => value.dyn_into::<js_sys::Uint8Array>().ok().map(|bytes| bytes.to_vec()),
            Err(e) => {
                log::warn!("this browser gave no copy of the mailboxes: {e:?}");
                None
            }
        }
    }

    /// Put `bytes` under `key` (`None`: delete it), kept once the transaction is.
    async fn put(key: &str, bytes: Option<&[u8]>) -> Result<(), JsValue> {
        let db = open().await?;
        let done = async {
            let tx = db.transaction_with_str_and_mode(STORE, web_sys::IdbTransactionMode::Readwrite)?;
            let store = tx.object_store(STORE)?;
            let end = finished(&tx);
            match bytes {
                Some(bytes) => store.put_with_key(&js_sys::Uint8Array::from(bytes), &JsValue::from_str(key))?,
                None => store.delete(&JsValue::from_str(key))?,
            };
            JsFuture::from(end).await.map(|_| ())
        }
        .await;
        db.close();
        done
    }

    /// Every copy of this browser deleted: the page wants none kept (`cache: false`).
    pub fn forget_all() {
        wasm_bindgen_futures::spawn_local(async {
            let Ok(db) = open().await else { return };
            if let Ok(tx) = db.transaction_with_str_and_mode(STORE, web_sys::IdbTransactionMode::Readwrite)
                && let Ok(store) = tx.object_store(STORE)
            {
                let end = finished(&tx);
                let _ = store.clear();
                let _ = JsFuture::from(end).await;
            }
            db.close();
        });
    }

    /// The copy of this tab is written under `key` from now on; the mailbox is at `rev` as the window starts, from a copy
    /// when `kept`.
    pub fn start(key: String, rev: u64, kept: bool) {
        WRITER.with(|w| *w.borrow_mut() = Some(Writer::new(key, mailcraft_ui_egui::now_ms(), rev)));
        FULL_RUN.with(|full| full.set(kept));
    }

    /// Once a frame: the copy written when it is due (in the background, one write at a time), and deleted when it can't
    /// be: an old copy is never left to show what has changed since.
    pub fn tick(app: &mut MailApp) {
        if app.is_opening() {
            return;
        }
        if FULL_RUN.with(|full| full.replace(false)) {
            let _ = app.run("ui.sync", serde_json::json!({}));
        }
        let now = mailcraft_ui_egui::now_ms();
        let rev = app.session.rev();
        let Some(key) = WRITER.with(|w| {
            let mut w = w.borrow_mut();
            let writer = w.as_mut()?;
            writer.due(rev, now).then(|| {
                writer.started(rev, now);
                writer.key.clone()
            })
        }) else {
            return;
        };
        let window = match app.session.prefs.sync_limit {
            0 => mailcraft_model::sync::MAX_JMAP_WINDOW,
            n => n.min(mailcraft_model::sync::MAX_JMAP_WINDOW),
        };
        let bytes = encode(&app.session.mailbox, usize::try_from(window).unwrap_or(usize::MAX));
        if bytes.is_none() {
            log::warn!("the mailboxes are too large to be kept in this browser");
        }
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = put(&key, bytes.as_deref()).await {
                log::warn!("the copy of the mailboxes couldn't be kept in this browser: {e:?}");
                let _ = put(&key, None).await;
            }
            WRITER.with(|w| w.borrow_mut().as_mut().map(|w| w.writing = false));
        });
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
