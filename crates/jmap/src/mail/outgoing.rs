// //// Neoffice — added file (no upstream equivalent)
//! The sending part of a run: what waits in the Outbox goes first, then the drafts that wait to go up, and
//! then the windows are read, so that the copy of a message that went is in the window of Sent.
//!
//! The steps, and what may be tried again, are in [`crate::send`]. Here is what belongs to a run:
//!
//! - **Nothing is asked of the server when there is nothing to send or file.** A run with no Outbox and no
//!   draft makes the requests it always did, and no others.
//! - **The mailboxes the copies go to** are the server's Sent and Drafts, as the run's own list of mailboxes
//!   has them. Without that list (the server could not give it) nothing is tried, and what waits waits.
//! - **A piece for each message** and each draft, as soon as it is done: the engine takes in what became of it
//!   before the next one is started, so that a run cut short after a message went does not leave the engine
//!   believing it did not.
//! - **A message is looked at once** whatever the request says, and at most [`MAX_JMAP_SENDS`] messages and
//!   [`MAX_JMAP_DRAFTS`] drafts are tried in a run: the rest wait for the next. A server may limit how many
//!   messages are sent in a few minutes, and says so when it does (the Outbox waits, and the owner is told).
//! - **Stop** (the owner's Cancel) is looked at before each message and each draft, not inside one: a
//!   message whose copy is filed is submitted or taken out again, never left half done.
//! - **Trouble with the server** (no answer, a busy server, a sign-in refused) ends the sending of this run
//!   and nothing else: the windows are read all the same, and meet the same trouble if it lasts.

use std::collections::HashSet;

use mailcraft_model::FolderKind;
use mailcraft_model::sync::{JmapMailResult, MAX_JMAP_DRAFTS, MAX_JMAP_SENDS};

use super::{Places, Run, Stop};
use crate::Http;
use crate::send::Outgoing;

impl<H: Http> Run<'_, H> {
    /// Send what waits in the Outbox and file the drafts, handing a piece to `emit` for each, before a
    /// window is read. `places` is the run's list of mailboxes, when it could be had.
    pub(super) async fn outgoing(&self, places: Option<&Places>, emit: &mut dyn FnMut(JmapMailResult)) -> Result<(), Stop> {
        let (sends, drafts) = (&self.request.sends, &self.request.drafts);
        if sends.is_empty() && drafts.is_empty() {
            return Ok(());
        }
        let Some(places) = places else {
            log::debug!("jmap mail: {} messages and {} drafts wait: the list of mailboxes was not had", sends.len(), drafts.len());
            return Ok(());
        };
        let mailbox = |kind: FolderKind| places.listing.folders.iter().find(|folder| folder.kind == kind).map(|folder| folder.id.as_str());
        let mut outgoing = Outgoing::new(self.client, mailbox(FolderKind::Sent), mailbox(FolderKind::Drafts), self.secrets.clone());

        let mut seen = HashSet::new();
        let mut stopped = false;
        for send in sends.iter().filter(|send| seen.insert(send.message)).take(MAX_JMAP_SENDS) {
            self.check()?;
            let went = outgoing.send(send).await;
            if went.done.is_some() || went.line.is_some() {
                emit(self.piece(JmapMailResult {
                    sent: went.done.into_iter().collect(),
                    errors: went.line.into_iter().collect(),
                    partial: true,
                    ..Default::default()
                }));
            }
            if went.stop {
                stopped = true;
                break;
            }
        }
        // (A server that can't be had for the messages is no better for the drafts.)
        let mut seen = HashSet::new();
        for draft in drafts.iter().filter(|draft| seen.insert(draft.message)).take(MAX_JMAP_DRAFTS) {
            if stopped {
                break;
            }
            self.check()?;
            let went = outgoing.draft(draft).await;
            if went.done.is_some() || went.line.is_some() {
                emit(self.piece(JmapMailResult {
                    drafts: went.done.into_iter().collect(),
                    errors: went.line.into_iter().collect(),
                    partial: true,
                    ..Default::default()
                }));
            }
            stopped = went.stop;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
