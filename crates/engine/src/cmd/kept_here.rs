// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What a session that keeps nothing of its own between visits ([`Session::keeps_local`] false: a hosting page's tab)
//! refuses, because it would be gone at the next visit: what lives on this computer alone.
//!
//! - **Tasks** are kept on this computer only: such a session doesn't offer them at all (`view.tasks`, the module bar,
//!   `tasks.new`, Add to Tasks), as the old base hid Notes for the same reason ([`tasks_refusal`]).
//! - **Importing** a file puts its messages, events, tasks and contacts here, where nothing stays ([`import_refusal`]).
//! - **A contact's categories and whether it is a favourite** are this computer's, whichever book the contact is in
//!   ([`marks_refusal`]).
//!
//! The reasons are fixed words, which a greyed control says on hover too; the window reads them in its own language.

use crate::Session;

/// Why Tasks are not offered: they are kept on this computer only.
pub const NO_TASKS_HERE: &str = "This page keeps no tasks: they would be gone at the next visit";
/// Why nothing is imported: what a file brings is kept on this computer only.
pub const NO_IMPORT_HERE: &str = "This page keeps nothing of its own between visits: what is imported would be gone at the next one";
/// Why a contact's categories and whether it is a favourite can't be set: they are kept on this computer only.
pub const NO_MARKS_HERE: &str = "This page keeps no categories or favorites: they would be gone at the next visit";

/// Whether Tasks are offered: where what is made here is kept.
pub fn tasks_offered(s: &Session) -> bool {
    s.keeps_local
}

/// Why Tasks are not offered, when they aren't.
pub fn tasks_refusal(s: &Session) -> Option<&'static str> {
    (!tasks_offered(s)).then_some(NO_TASKS_HERE)
}

/// Why a file is not imported, when it isn't.
pub fn import_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_IMPORT_HERE)
}

/// Why a contact's categories and whether it is a favourite can't be set, when they can't.
pub fn marks_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_MARKS_HERE)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Wednesday 2026-10-14 16:00 UTC.
    const NOW: i64 = 1_791_993_600;

    fn sample(keeps_local: bool) -> Session {
        let mut s = Session::at(crate::sample::mailbox(NOW, 0), NOW, 0);
        s.keeps_local = keeps_local;
        s
    }

    #[test]
    fn tasks_are_not_offered_in_a_tab_that_keeps_nothing() {
        let mut s = sample(false);
        // Not in front, not made, not made of a message; and the key does nothing.
        let refused = s.run("view.tasks", &json!({})).unwrap_err().to_string();
        assert!(refused.contains(NO_TASKS_HERE), "{refused}");
        assert_ne!(s.view.module, crate::Module::Tasks);
        let before = s.mailbox.tasks.len();
        assert!(s.run("tasks.new", &json!({"title": "Water the plants"})).unwrap_err().to_string().contains(NO_TASKS_HERE));
        let message = s.mailbox.messages.keys().next().copied().unwrap();
        assert!(s.run("tasks.fromMessage", &json!({"id": message})).unwrap_err().to_string().contains(NO_TASKS_HERE));
        assert_eq!((s.mailbox.tasks.len(), s.view.module == crate::Module::Tasks), (before, false));
        for id in ["view.tasks", "tasks.new"] {
            assert_eq!(s.registry.get(id).and_then(|spec| (spec.enabled)(&s)), Some(NO_TASKS_HERE), "{id}");
        }
        // The other modules open as ever.
        for id in ["view.mail", "view.calendar", "view.people"] {
            s.run(id, &json!({})).unwrap();
        }
    }

    #[test]
    fn an_app_that_keeps_tasks_offers_them() {
        let mut s = sample(true);
        s.run("view.tasks", &json!({})).unwrap();
        assert_eq!(s.view.module, crate::Module::Tasks);
        assert!(s.run("tasks.new", &json!({"title": "Water the plants"})).is_ok());
        assert_eq!(tasks_refusal(&s), None);
    }

    #[test]
    fn a_contacts_categories_favourite_and_an_import_are_refused_where_nothing_is_kept() {
        let mut s = sample(false);
        let contact = s.mailbox.contacts.keys().next().copied().unwrap();
        s.sel.contact = Some(contact);
        for (id, params) in [("people.favorite", json!({"id": contact})), ("people.categorize", json!({"id": contact, "category": "Red category"}))] {
            assert!(s.run(id, &params).unwrap_err().to_string().contains(NO_MARKS_HERE), "{id}");
            assert_eq!(s.registry.get(id).and_then(|spec| (spec.enabled)(&s)), Some(NO_MARKS_HERE), "{id}");
        }
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Gus Hale\r\nEMAIL:gus@example.org\r\nEND:VCARD\r\n";
        use base64::Engine as _;
        let data = base64::engine::general_purpose::STANDARD.encode(card);
        let refused = s.run("file.import", &json!({"name": "gus.vcf", "data": data})).unwrap_err().to_string();
        assert!(refused.contains(NO_IMPORT_HERE), "{refused}");
        // Kept here, they go on as before.
        let mut s = sample(true);
        s.run("people.favorite", &json!({"id": contact})).unwrap();
        s.run("file.import", &json!({"name": "gus.vcf", "data": data})).unwrap();
        assert!(s.mailbox.contacts.values().any(|c| c.first == "Gus" || c.display_name == "Gus Hale" || c.name() == "Gus Hale"));
    }
}
