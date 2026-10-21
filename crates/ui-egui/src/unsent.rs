// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Whether the window holds work that leaving it now would lose: what a hosting page needs to know to ask before it
//! closes (Neodesk's window asks only when the page says so, and leaves the browser's own question aside; the web build
//! tells it, `apps/mailcraft-web/src/host_dirty.rs`).
//!
//! - A message being written that has changes not saved, as closing the desktop window asks about
//!   ([`MailApp::close_requested`]).
//! - An appointment or a meeting whose window has changes not saved ([`crate::windows`]), and a form a dialog holds
//!   with something typed into it: an appointment's, a contact's.
//! - In a tab that keeps nothing of its own between visits ([`mailcraft_engine::Session::keeps_local`] false): what
//!   waits to go to a server, made here and not told to it yet: a change of an event or of a contact, mail that waits
//!   in an Outbox, a draft saved here that the next run files on the server, a move or a delete for good the server is
//!   yet to be told of (`Mailbox::pending`, `Mailbox::queued`), and a JMAP account's messages read, flagged or moved here
//!   (`jmap_up::message_changes`, worked out once for each state of the mailbox). The next visit starts from what the
//!   servers have, and these would not be there.
//! - What the notes' own question before leaving counts ([`MailApp::notes_unsent`]): a change of a note that waits to
//!   go, a note here alone, words typed into a note and not in it yet.

use mailcraft_engine::cmd::{jmap_send, jmap_up};
use mailcraft_model::{AccountKind, FolderKind};

use crate::MailApp;
use crate::dialogs::Dialog;

impl MailApp {
    /// Whether leaving the window now would lose work done here (see the module's documentation).
    pub fn unsent_work(&self) -> bool {
        let writing = self.session.compose.as_ref().is_some_and(|c| c.dirty);
        let forms = self.windows.unsaved() > 0 || matches!(&self.dialog, Some(Dialog::Form(form)) if form.unsaved());
        writing || forms || (!self.session.keeps_local && self.waits_to_go()) || self.notes_unsent()
    }

    /// Whether something made here waits to go to a server: a change of an event or of a contact not yet told to it,
    /// mail in an Outbox, a draft to file, a move or a delete to tell, a change of a JMAP account's messages.
    fn waits_to_go(&self) -> bool {
        let mb = &self.session.mailbox;
        let outbox = mb.folders.iter().filter(|f| f.kind == FolderKind::Outbox).any(|f| mb.count(f.id) > 0);
        let drafts = mb.accounts.iter().any(|a| jmap_send::pending_drafts(mb, a.id) > 0);
        !mb.event_changes.is_empty()
            || !mb.contact_changes.is_empty()
            || outbox
            || drafts
            || !mb.pending.is_empty()
            || !mb.queued.is_empty()
            || self.mail_changes_wait()
    }

    /// Whether a JMAP account's messages were changed here (read, flagged, moved) and its server is yet to be told: worked
    /// out once for each state of the mailbox, as it reads every message.
    fn mail_changes_wait(&self) -> bool {
        let at = (self.session.openings(), self.session.rev());
        if let Some((openings, rev, waits)) = self.unsent_marks.get()
            && (openings, rev) == at
        {
            return waits;
        }
        let mut jmap = self.session.mailbox.accounts.iter().filter(|a| a.kind == AccountKind::Jmap);
        let waits = jmap.any(|account| jmap_up::message_changes(&self.session, account.id, 1).1 > 0);
        self.unsent_marks.set(Some((at.0, at.1, waits)));
        waits
    }
}

#[cfg(test)]
mod tests {
    use mailcraft_engine::Session;
    use mailcraft_model::{FolderKind, Id, Message};
    use serde_json::json;

    use crate::{MailApp, Services};

    /// Wednesday 2026-10-14 16:00 UTC.
    const NOW: i64 = 1_791_993_600;

    fn window(keeps_local: bool) -> MailApp {
        let mut s = Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0);
        s.keeps_local = keeps_local;
        let mut app = MailApp::new(s, Services::default());
        app.autosave = false;
        app
    }

    #[test]
    fn a_message_being_written_with_changes_is_unsent_work_until_it_is_saved_or_closed() {
        let mut app = window(false);
        assert!(!app.unsent_work(), "nothing yet");
        app.execute("mail.new", json!({})).unwrap();
        // Opened and nothing typed: nothing to lose.
        assert!(!app.unsent_work());
        app.execute("compose.set", json!({"subject": "Plans for Friday"})).unwrap();
        assert!(app.unsent_work(), "typed into");
        app.execute("compose.discard", json!({})).unwrap();
        assert!(!app.unsent_work(), "thrown away: nothing left");
    }

    #[test]
    fn an_appointment_or_a_contact_form_with_something_typed_is_unsent_work() {
        let mut app = window(false);
        // A new contact's form, typed into.
        app.execute("ui.dialog", json!({"name": "contact"})).unwrap();
        assert!(!app.unsent_work(), "an empty form");
        app.dialog = None;
        app.execute("ui.dialog", json!({"name": "contact", "first": "Gus"})).unwrap();
        assert!(app.unsent_work(), "a name typed in");
        app.dialog = None;
        // An existing contact's, opened and not changed.
        let contact = app.session.mailbox.contacts.keys().next().copied().unwrap();
        app.execute("ui.dialog", json!({"name": "editContact", "id": contact})).unwrap();
        assert!(!app.unsent_work(), "as it was");
        app.dialog = None;
        // An appointment's window with another time in its fields, not saved.
        let event = app.session.mailbox.events.values().find(|e| e.recurrence.is_none() && !e.all_day).map(|e| e.id).unwrap();
        app.execute("ui.openWindow", json!({"event": event})).unwrap();
        assert!(!app.unsent_work(), "opened, as it is");
        let other = json!({"start": "2026-10-20T14:00:00", "end": "2026-10-20T15:00:00"});
        app.execute("ui.openWindow", json!({"event": event, "proposed": other})).unwrap();
        assert_eq!(app.windows.unsaved(), 1);
        assert!(app.unsent_work());
        app.execute("ui.closeWindow", json!({"all": true})).unwrap();
        assert!(!app.unsent_work());
        // And the same in a dialog's form.
        app.execute("ui.dialog", json!({"name": "editEvent", "id": event})).unwrap();
        assert!(!app.unsent_work());
        if let Some(crate::dialogs::Dialog::Form(form)) = &mut app.dialog
            && let Some(form) = form.event_mut()
        {
            let at = |text: &str| mailcraft_time::parse_iso(text).map(|(at, _)| at).unwrap();
            form.take_proposal(at("2026-10-20T14:00:00"), at("2026-10-20T15:00:00"), false);
        }
        assert!(app.unsent_work());
    }

    #[test]
    fn what_waits_to_go_to_a_server_is_unsent_work_in_a_tab_that_keeps_nothing() {
        for keeps_local in [false, true] {
            // Mail that waits in the Outbox.
            let mut app = window(keeps_local);
            let outbox = app.session.mailbox.folders.iter().find(|f| f.kind == FolderKind::Outbox).map(|f| f.id).expect("the sample has an Outbox");
            let message = app.session.mailbox.messages.values().next().map(|m| (**m).clone()).unwrap();
            app.session.mailbox.add_message(mailcraft_model::Message { id: 0, folder: outbox, ..message }).unwrap();
            assert_eq!(app.unsent_work(), !keeps_local, "keeps_local {keeps_local}: the Outbox");
            // A change of an event noted for a server.
            let mut app = window(keeps_local);
            app.session.mailbox.event_changes.push(mailcraft_model::remote::EventChange::default());
            assert_eq!(app.unsent_work(), !keeps_local, "keeps_local {keeps_local}: an event's change");
            // A change of a contact noted for a server.
            let mut app = window(keeps_local);
            app.session.mailbox.contact_changes.push(mailcraft_model::book::ContactChange::default());
            assert_eq!(app.unsent_work(), !keeps_local, "keeps_local {keeps_local}: a contact's change");
            // And what the notes' own question counts: a change of a note that waits to go.
            let mut app = window(keeps_local);
            app.session.mailbox.notes.changes.push(mailcraft_model::note::NoteChange::default());
            assert_eq!(app.unsent_work(), !keeps_local, "keeps_local {keeps_local}: a note's change");
            // A delete for good the server is yet to be told of.
            let mut app = window(keeps_local);
            let delete = mailcraft_model::Pending::Delete { account: 1, from: "INBOX".into(), uid: 7, message_id: String::new() };
            app.session.mailbox.pending.push(delete);
            assert_eq!(app.unsent_work(), !keeps_local, "keeps_local {keeps_local}: a delete for good");
        }
    }

    /// A JMAP account with its Inbox and Drafts: the window, the account, its Inbox and its Drafts.
    fn jmap_window() -> (MailApp, Id, Id, Id) {
        let mut app = window(false);
        let added = app
            .session
            .run("account.add", &json!({"email": "ada@example.org", "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap/session"}}));
        let account = added.unwrap()["id"].as_u64().unwrap();
        // (Those the account has already, else made.)
        let mut folder = |kind: FolderKind, name: &str| {
            if let Some(id) = app.session.mailbox.folder_of_kind(account, kind) {
                return id;
            }
            let id = app.session.mailbox.alloc_id();
            app.session.mailbox.folders.push(mailcraft_model::Folder { id, account, kind, name: name.into(), ..Default::default() });
            id
        };
        let (inbox, drafts) = (folder(FolderKind::Inbox, "Inbox"), folder(FolderKind::Drafts, "Drafts"));
        app.session.mailbox_changed();
        (app, account, inbox, drafts)
    }

    #[test]
    fn a_draft_saved_here_and_a_message_marked_here_wait_to_go_in_a_tab_that_keeps_nothing() {
        // A draft saved here, which the next run files on the server.
        let (mut app, account, _, drafts) = jmap_window();
        assert!(!app.unsent_work(), "nothing yet");
        let draft = Message {
            account,
            folder: drafts,
            draft: true,
            subject: "Plans".into(),
            server_draft: Some(mailcraft_model::sync::ServerDraft { pending: true, ..Default::default() }),
            ..Default::default()
        };
        app.session.mailbox.add_message(draft).unwrap();
        app.session.mailbox_changed();
        assert!(app.unsent_work(), "a draft to file");
        // A message read here that its server still has unread.
        let (mut app, account, inbox, _) = jmap_window();
        let server = mailcraft_model::sync::ServerMessage { id: "M1".into(), folder: "f-inbox".into(), read: false, ..Default::default() };
        let read = Message { account, folder: inbox, subject: "Hello".into(), read: false, server: Some(server), ..Default::default() };
        let id = app.session.mailbox.add_message(read).unwrap();
        app.session.mailbox_changed();
        assert!(!app.unsent_work(), "as the server has it");
        app.session.run("mail.markRead", &json!({"ids": [id]})).unwrap();
        assert!(app.unsent_work(), "read here, unread there");
        // Worked out once for that state, again for the next.
        app.session.run("mail.markUnread", &json!({"ids": [id]})).unwrap();
        assert!(!app.unsent_work(), "as the server has it again");
    }
}
