// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The folder changes made to a JMAP account, told to its server with `Mailbox/set` (RFC 8621 §2.5), first in a run.
//!
//! The engine has worked them out as it does a Microsoft account's ([`FolderChange`], parents before what lies in
//! them); this makes them in one call and says what became of each ([`FolderChangeDone`]), in a piece of its own,
//! before the changes of messages go up.
//!
//! - **What is sent.** A folder made here is a `create`, keyed `f` and its id here, under its parent's server id, or
//!   under a folder made by the same call by a creation reference (`#f…`); one made under a folder that the call
//!   doesn't make waits. A rename and a move are an `update` of `name` and of `parentId`, one update for a folder that
//!   has both. A folder deleted for good is a `destroy`, with its mail (`onDestroyRemoveEmails`), as Outlook deletes a
//!   folder out of Deleted Items.
//! - **What became of each.** `created` and `updated` are done: the folder as the call left it, under the id the answer
//!   names. A server that names a mailbox by where it lies gives a renamed or moved folder another id (`updated:
//!   {old: {"id": new}}`), and a run that reads only some mailboxes reads those too, so that the messages in them are
//!   found again under their new ids. `destroyed` is done. A SetError is a refusal by its type, but for `notFound` of a
//!   folder that was there (gone: the listing settles it) and a failure of the server's own (the change waits for the
//!   next run). A call refused as a whole because the account can't be changed refuses every change in it; any other
//!   trouble of the call leaves them all for the next run, and the run goes on.
//!
//! Everything a server answers is hostile: only the keys that were sent are looked up in what it says, an id it names
//! is taken only when it could be one, and a SetError's text is never shown (only its type).

use std::collections::{HashMap, HashSet};

use mailcraft_model::sync::{ChangeOutcome, FolderChange, FolderChangeDone, GraphFolder, JmapMailResult, MAX_FOLDER_CHANGES};
use mailcraft_model::{FolderKind, Id};
use serde_json::{Map, Value, json};

use super::up::{NO_CHANGES, Verdict, token, verdict};
use super::{Outcome, Run, Stop, WINDOW_BYTES};
use crate::{Error, Http, valid_id};

/// The creation key of the folder `folder` (its id here).
fn key(folder: Id) -> String {
    format!("f{folder}")
}

fn refused(why: &str) -> ChangeOutcome {
    ChangeOutcome::Refused { why: why.to_string() }
}

/// What a SetError comes to; `None` for one that says the change waits (a failure of the server's own).
fn of_error(error: &Value, was_there: bool) -> Option<ChangeOutcome> {
    match verdict(error) {
        // A folder that isn't there any more is the listing's to settle; a parent that isn't is a refusal.
        Verdict::Gone if was_there => Some(ChangeOutcome::Gone),
        Verdict::Gone => Some(refused("notFound")),
        Verdict::Refused(why) => Some(ChangeOutcome::Refused { why }),
        Verdict::Later(_) => None,
    }
}

/// Put `value` under `name` in the update of the folder the server calls `id`, made or begun.
fn patch(update: &mut Map<String, Value>, id: &str, name: &str, value: Value) {
    let entry = update.entry(id.to_string()).or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(members) = entry {
        members.insert(name.to_string(), value);
    }
}

/// What the answer to the call says, as the server's words to each change: `created`, `updated` and the rest, read for
/// the keys that were sent only.
struct Answer<'a> {
    created: Option<&'a Map<String, Value>>,
    not_created: Option<&'a Map<String, Value>>,
    updated: Option<&'a Map<String, Value>>,
    not_updated: Option<&'a Map<String, Value>>,
    destroyed: HashSet<&'a str>,
    not_destroyed: Option<&'a Map<String, Value>>,
}

impl<'a> Answer<'a> {
    fn of(said: &'a Value, asked: usize) -> Answer<'a> {
        let members = |name: &str| said.get(name).and_then(Value::as_object);
        let destroyed = said
            .get("destroyed")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .take(asked.saturating_mul(2))
            .filter_map(Value::as_str)
            .collect();
        Answer {
            created: members("created"),
            not_created: members("notCreated"),
            updated: members("updated"),
            not_updated: members("notUpdated"),
            destroyed,
            not_destroyed: members("notDestroyed"),
        }
    }

    /// What became of the update of the folder `id`: the folder as it is now, `shaped` by what the change was, under the
    /// id the answer names (its own, when the answer names none); `None` when it says nothing of it.
    fn update(&self, id: &str, shaped: impl Fn(&mut GraphFolder)) -> Option<ChangeOutcome> {
        if let Some(error) = self.not_updated.and_then(|members| members.get(id)) {
            return of_error(error, true);
        }
        let entry = self.updated.and_then(|members| members.get(id))?;
        let now = entry.get("id").and_then(Value::as_str).filter(|named| valid_id(named)).unwrap_or(id);
        let mut folder = GraphFolder { id: now.to_string(), kind: FolderKind::Custom, ..Default::default() };
        shaped(&mut folder);
        Some(ChangeOutcome::Done { folder })
    }
}

impl<H: Http> Run<'_, H> {
    /// Make the folder changes of the request, and hand on what became of each in a piece of its own, before anything
    /// else goes up. Nothing is asked of the server, and no piece is handed on, when the request holds none. An error is
    /// what ends the run (a cancel, the credentials refused, a busy server); trouble that is the changes' alone is told
    /// in `out` and the run goes on.
    pub(super) async fn make_folder_changes(&self, out: &mut Outcome, emit: &mut dyn FnMut(JmapMailResult)) -> Result<(), Stop> {
        let changes: Vec<&FolderChange> = self.request.folders.iter().take(MAX_FOLDER_CHANGES).collect();
        if changes.is_empty() {
            return Ok(());
        }
        let mut settled: Vec<Option<ChangeOutcome>> = vec![None; changes.len()];
        // Which changes the call holds: only their answers are looked for.
        let mut sent = vec![false; changes.len()];
        let (mut create, mut update, mut destroy) = (Map::new(), Map::new(), Vec::<&str>::new());
        // The folders this call makes, by their ids here: a folder made under one of them goes in by a creation reference.
        let mut made: HashSet<Id> = HashSet::new();
        for (at, change) in changes.iter().enumerate() {
            let invalid = match change {
                FolderChange::Create { folder, name, parent, parent_folder } => {
                    let parent_id = match parent_folder {
                        Some(local) if made.contains(local) => Some(json!(format!("#{}", key(*local)))),
                        // Under a folder that this call doesn't make: it waits for the run after the one that makes it.
                        Some(_) => continue,
                        None if parent.is_empty() => Some(Value::Null),
                        None => valid_id(parent).then(|| json!(parent)),
                    };
                    match parent_id {
                        Some(parent_id) if !made.contains(folder) => {
                            create.insert(key(*folder), json!({"name": name, "parentId": parent_id}));
                            made.insert(*folder);
                            false
                        }
                        // (Asked twice: the first is the one made.)
                        Some(_) => continue,
                        None => true,
                    }
                }
                FolderChange::Rename { id, name, .. } if valid_id(id) => {
                    patch(&mut update, id, "name", json!(name));
                    false
                }
                FolderChange::Move { id, parent, .. } if valid_id(id) && (parent.is_empty() || valid_id(parent)) => {
                    let parent = if parent.is_empty() { Value::Null } else { json!(parent) };
                    patch(&mut update, id, "parentId", parent);
                    false
                }
                FolderChange::Delete { id } if valid_id(id) => {
                    if !destroy.contains(&id.as_str()) {
                        destroy.push(id.as_str());
                    }
                    false
                }
                _ => true,
            };
            if invalid && let Some(slot) = settled.get_mut(at) {
                *slot = Some(refused("invalidId"));
            } else if let Some(sent) = sent.get_mut(at) {
                *sent = true;
            }
        }
        let result = if create.is_empty() && update.is_empty() && destroy.is_empty() {
            Ok(())
        } else {
            self.check()?;
            let mut args = Map::new();
            args.insert("accountId".into(), json!(self.client.account_id()));
            if !create.is_empty() {
                args.insert("create".into(), Value::Object(create));
            }
            if !update.is_empty() {
                args.insert("update".into(), Value::Object(update));
            }
            if !destroy.is_empty() {
                args.insert("destroy".into(), json!(destroy));
                args.insert("onDestroyRemoveEmails".into(), Value::Bool(true));
            }
            self.set_folders(args, &changes, &sent, &mut settled, out).await
        };
        let answered: Vec<FolderChangeDone> = changes
            .iter()
            .zip(settled.iter())
            .filter_map(|(change, outcome)| outcome.as_ref().map(|outcome| FolderChangeDone { change: (*change).clone(), outcome: outcome.clone() }))
            .collect();
        let waiting = settled.iter().filter(|outcome| outcome.is_none()).count();
        if result.is_ok() && waiting > 0 {
            out.up.0 = out.up.0.saturating_add(waiting);
            out.up.1.get_or_insert_with(|| "the server did not take them".to_string());
        }
        log::debug!("jmap folder changes: {} of {} answered", answered.len(), changes.len());
        if !answered.is_empty() {
            emit(self.piece(JmapMailResult { folders: answered, partial: true, ..Default::default() }));
        }
        result
    }

    /// The one call, and what each change came to, in `settled`; the new ids of folders renamed or moved, in `out`.
    async fn set_folders(
        &self,
        args: Map<String, Value>,
        changes: &[&FolderChange],
        sent: &[bool],
        settled: &mut [Option<ChangeOutcome>],
        out: &mut Outcome,
    ) -> Result<(), Stop> {
        let told = |out: &mut Outcome, error: &Error| {
            out.up.1.get_or_insert_with(|| self.said(error));
        };
        let said = match self.client.mail_within(vec![("Mailbox/set", Value::Object(args))], WINDOW_BYTES).await {
            Ok(mut answers) => answers.take(0),
            Err(error) => {
                return match self.call_stop(error) {
                    Stop::Failed(error) | Stop::Down(error) => {
                        told(out, &error);
                        Ok(())
                    }
                    stop => Err(stop),
                };
            }
        };
        let said = match said {
            Ok(said) => said,
            // The account's folders can't be changed (a read-only account, a server that has no `Mailbox/set`): each is refused.
            Err(Error::Refused { kind, .. }) if NO_CHANGES.contains(&kind.as_str()) => {
                let why = token(&kind);
                for (slot, _) in settled.iter_mut().zip(sent).filter(|(slot, sent)| slot.is_none() && **sent) {
                    *slot = Some(refused(&why));
                }
                return Ok(());
            }
            Err(error) => {
                return match Self::method_stop(error) {
                    Stop::Failed(error) | Stop::Down(error) => {
                        told(out, &error);
                        Ok(())
                    }
                    stop => Err(stop),
                };
            }
        };
        let answer = Answer::of(&said, changes.len());
        // The server ids of the folders made, by their ids here: where those made under them lie.
        let mut made: HashMap<Id, String> = HashMap::new();
        for (at, change) in changes.iter().enumerate() {
            let Some(slot) = settled.get_mut(at).filter(|slot| slot.is_none() && sent.get(at) == Some(&true)) else { continue };
            *slot = match change {
                FolderChange::Create { folder, name, parent, parent_folder } => {
                    let key = key(*folder);
                    if let Some(error) = answer.not_created.and_then(|members| members.get(&key)) {
                        of_error(error, false)
                    } else if let Some(id) = answer
                        .created
                        .and_then(|members| members.get(&key))
                        .and_then(|entry| entry.get("id"))
                        .and_then(Value::as_str)
                        .filter(|id| valid_id(id))
                    {
                        let parent = match parent_folder {
                            Some(local) => made.get(local).cloned().unwrap_or_default(),
                            None => parent.clone(),
                        };
                        made.insert(*folder, id.to_string());
                        Some(ChangeOutcome::Done {
                            folder: GraphFolder { id: id.to_string(), parent, name: name.clone(), kind: FolderKind::Custom, ..Default::default() },
                        })
                    } else {
                        None
                    }
                }
                FolderChange::Rename { id, name, .. } => answer.update(id, |folder| folder.name.clone_from(name)),
                FolderChange::Move { id, parent, .. } => answer.update(id, |folder| folder.parent.clone_from(parent)),
                FolderChange::Delete { id } => {
                    if let Some(error) = answer.not_destroyed.and_then(|members| members.get(id)) {
                        of_error(error, true)
                    } else if answer.destroyed.contains(id.as_str()) {
                        Some(ChangeOutcome::Done { folder: GraphFolder::default() })
                    } else {
                        None
                    }
                }
            };
            // A folder that took another id: its window is read in this run.
            if let (FolderChange::Rename { id, .. } | FolderChange::Move { id, .. }, Some(ChangeOutcome::Done { folder })) = (change, &*slot)
                && folder.id != *id
                && !out.refolded.contains(&folder.id)
            {
                out.refolded.push(folder.id.clone());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "folder_changes/tests.rs"]
mod tests;
