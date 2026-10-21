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
//!   waits to go to a server, made here and not told to it yet (a change of an event or of a contact), and mail that
//!   waits in an Outbox. The next visit starts from what the servers have, and these would not be there.
//! - What the notes' own question before leaving counts ([`MailApp::notes_unsent`]): a change of a note that waits to
//!   go, a note here alone, words typed into a note and not in it yet.

use mailcraft_model::FolderKind;

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
    /// or mail in an Outbox.
    fn waits_to_go(&self) -> bool {
        let mb = &self.session.mailbox;
        let outbox = mb.folders.iter().filter(|f| f.kind == FolderKind::Outbox).any(|f| mb.count(f.id) > 0);
        !mb.event_changes.is_empty() || !mb.contact_changes.is_empty() || outbox
    }
}

#[cfg(test)]
mod tests {
    use mailcraft_engine::Session;
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
            let mut app = window(keeps_local);
            // Mail that waits in the Outbox.
            let outbox = app.session.mailbox.folders.iter().find(|f| f.kind == mailcraft_model::FolderKind::Outbox).map(|f| f.id);
            if let Some(outbox) = outbox {
                let message = app.session.mailbox.messages.values().next().map(|m| (**m).clone()).unwrap();
                app.session.mailbox.add_message(mailcraft_model::Message { id: 0, folder: outbox, ..message }).unwrap();
                assert_eq!(app.unsent_work(), !keeps_local, "keeps_local {keeps_local}: the Outbox");
            }
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
        }
    }
}
