// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What a session that keeps nothing of its own between visits ([`Session::keeps_local`] false: a hosting page's tab)
//! refuses, because it would be gone at the next visit: what lives on this computer alone.
//!
//! - **Tasks** are kept on this computer only: such a session doesn't offer them at all (`view.tasks`, the module bar,
//!   `tasks.new`, Add to Tasks), as the old base hid Notes for the same reason ([`tasks_refusal`]).
//! - **Importing** a file puts its messages, events, tasks and contacts here, where nothing stays ([`import_refusal`]).
//! - **Public keys and certificates** (File › Options › Security, `keys.import`, `keys.remove`) are kept in the mailbox, where nothing
//!   stays between visits, and the keys of a contact's card are a card's, which a server's address book doesn't keep ([`keys_refusal`]).
//! - **A contact's categories and whether it is a favourite** are this computer's, whichever book the contact is in
//!   ([`marks_refusal`]).
//!
//! - **Send Later, Delay Delivery and follow-up reminders** (upstream's #950 and #948): the message waits in the Outbox of this
//!   computer and the reminder rings here, so neither goes nor rings once the tab is closed ([`later_refusal`]); and a reminder
//!   "if nobody replies" would be a second way beside Nora's follow-up, which the mailbox's server keeps. Undo Send stays: its
//!   wait is seconds, in the tab that is open.
//! - **Calendars subscribed to on the web** (upstream's #209): the subscription is kept in the mailbox, and only a host that
//!   reaches the web updates it; the browser build has none, so it would never be updated and be gone at the next visit
//!   ([`subscription_refusal`]). Unsubscribing stays.
//! - **The order of folders and of Favorites** (upstream's #1514): "kept on this computer", never sent to a server, so it would
//!   be lost at the next visit ([`order_refusal`]). The folders stay where the pane puts them, « Plus tard » right after Drafts.
//! - **A rule's desktop alert, a due day for the flag a rule sets, and a copy a rule makes of a JMAP account's mail** (upstream's
//!   #566, [`rule_refusal`]). The rules themselves are kept by the hosting page's server (`kept.rs` of the browser build), and
//!   run when the person runs them: a JMAP run puts no new mail through them, so an alert "as it arrives" would never ring; a
//!   flag's due date is not the server's to keep ([`due_refusal`]); and a JMAP server keeps a message in one mailbox, so a
//!   copy would be a message only this tab has ([`mailcraft_model::COPY_JMAP`], wherever the account is kept).
//! - **Shaded rows** (upstream's #1657): the list of such a tab is the design system's (variant D), which has no bands, and the
//!   choice would be lost at the next visit ([`own_list_refusal`]).
//!
//! The reasons are fixed words, which a greyed control says on hover too; the window reads them in its own language.

use mailcraft_model::{Action, Rule};

use crate::Session;

/// Why Tasks are not offered: they are kept on this computer only.
pub const NO_TASKS_HERE: &str = "This page keeps no tasks: they would be gone at the next visit";
/// Why nothing is imported: what a file brings is kept on this computer only.
pub const NO_IMPORT_HERE: &str = "This page keeps nothing of its own between visits: what is imported would be gone at the next one";
/// Why a contact's categories and whether it is a favourite can't be set: they are kept on this computer only.
pub const NO_MARKS_HERE: &str = "This page keeps no categories or favorites: they would be gone at the next visit";

/// Why a problem isn't reported to MailCraft's makers from such a page: it is a hosting page's, whose people tell theirs to whoever runs it.
pub const NO_SUPPORT_HERE: &str = "This page reports problems to the people who run it, not to MailCraft's makers";

/// Why Public keys are not imported or removed: they are kept in the mailbox, which a hosting page's tab doesn't keep.
pub const NO_KEYS_HERE: &str = "This page keeps no certificates or keys: they would be gone at the next visit";

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

/// Why Feedback and Get Support, which open MailCraft's own issue tracker, and Report a Problem, which starts a report to its makers,
/// are not offered, when they aren't: the session is a hosting page's tab (it keeps nothing of its own).
pub fn support_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_SUPPORT_HERE)
}

/// Why Public keys can't be imported or removed, when they can't.
pub fn keys_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_KEYS_HERE)
}

/// Why Send Later, Delay Delivery and the follow-up reminders are not offered: they wait on this computer alone.
pub const NO_LATER_HERE: &str =
    "This page keeps nothing of its own: a message to send later, or a reminder, would wait in this tab and neither go nor ring once it is closed";

/// Upstream's commands of Send Later and Delay Delivery (#950) and of follow-up reminders (#948): what they keep waits on this
/// computer. Undo Send (`compose.undoSend`, `sync.waitBeforeSending`) and Cancel Send (`mail.cancelSend`) are not among them.
pub const LATER: &[&str] = &[
    "compose.sendLater",
    "compose.delay",
    "mail.setSendTime",
    "compose.followUp",
    "mail.reminder",
    "mail.clearReminder",
    "mail.settleReminders",
    "mail.dueReminders",
    "mail.dismissReminder",
    "mail.postponeReminder",
];

/// Why Send Later, Delay Delivery and the follow-up reminders are not offered, when they aren't.
pub fn later_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_LATER_HERE)
}

/// Why no calendar is subscribed to on the web: it would not be updated here, and would be gone at the next visit.
pub const NO_SUBSCRIPTIONS_HERE: &str =
    "This page keeps nothing of its own: a calendar subscribed to here would not be updated and would be gone at the next visit";

/// Upstream's commands that subscribe to a calendar on the web or keep one up to date (#209). Unsubscribing is not among them.
pub const SUBSCRIBED: &[&str] = &["calendar.subscribe", "calendar.refreshSubscribed", "calendar.subscriptionSettings"];

/// Why no calendar is subscribed to on the web, when none is.
pub fn subscription_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_SUBSCRIPTIONS_HERE)
}

/// Why the order of folders and of Favorites isn't the person's to choose: it is kept on this computer alone.
pub const NO_ORDER_HERE: &str = "This page keeps no order of folders: it would be lost at the next visit";

/// Upstream's commands that put a folder, or one of the Favorites, in another place (#1514).
pub const ORDERED: &[&str] = &["folder.reorder", "folder.reorderFavorite"];

/// Why folders and Favorites aren't put in another order, when they aren't.
pub fn order_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_ORDER_HERE)
}

/// Why every other row isn't shaded: the list of a hosting page's tab is the design system's, which has no bands.
pub const OWN_LIST: &str = "This page's message list has a look of its own";

/// Upstream's command that shades every other row of its list (#1657).
pub const SHADED: &str = "view.shadeRows";

/// Why the rows of the list aren't shaded, when they aren't.
pub fn own_list_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(OWN_LIST)
}

/// Why a rule shows no desktop alert: a hosting page's rules run when the person runs them.
pub const NO_RULE_ALERT_HERE: &str = "This page's rules run when you run them, not as mail arrives: a rule shows no desktop alert here";

/// Why the rule `r` can't be made here, when it can't: it shows a desktop alert or flags with a due day in a hosting page's tab,
/// or copies the mail of a JMAP account, or into one.
pub fn rule_refusal(s: &Session, r: &Rule) -> Option<&'static str> {
    if r.alert && !s.keeps_local {
        return Some(NO_RULE_ALERT_HERE);
    }
    r.actions.iter().find_map(|a| match a {
        Action::FlagFor(_) => due_refusal(s),
        Action::CopyTo(folder) => {
            let into = s.mailbox.folder(*folder).map_or(0, |f| f.account);
            (s.mailbox.is_jmap(r.account) || s.mailbox.is_jmap(into)).then_some(mailcraft_model::COPY_JMAP)
        }
        _ => None,
    })
}

/// Why the command `id` is refused whoever asks (a script, an agent, a call that names its messages), when it is one of [`LATER`],
/// [`SUBSCRIBED`] or [`ORDERED`], or [`SHADED`].
pub fn later_refusal_of(s: &Session, id: &str) -> Option<&'static str> {
    if LATER.contains(&id) {
        later_refusal(s)
    } else if SUBSCRIBED.contains(&id) {
        subscription_refusal(s)
    } else if ORDERED.contains(&id) {
        order_refusal(s)
    } else if id == SHADED {
        own_list_refusal(s)
    } else {
        None
    }
}

/// [`later_refusal`], then upstream's own condition: a message being written.
pub(crate) fn later_while_writing(s: &Session) -> Option<&'static str> {
    later_refusal(s).or_else(|| super::needs_compose(s))
}

/// [`later_refusal`], then upstream's own condition: a selected message.
pub(crate) fn later_on_messages(s: &Session) -> Option<&'static str> {
    later_refusal(s).or_else(|| super::needs_message(s))
}

/// Why a contact's categories and whether it is a favourite can't be set, when they can't.
pub fn marks_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_MARKS_HERE)
}

/// Why a flag isn't given a due date: a JMAP server keeps a message's flag (`$flagged`) and nothing of its date, which the tab
/// would lose at the next visit. A flag without a date is set everywhere.
pub const NO_DUE_HERE: &str = "This page keeps no due date for a flag: its server keeps the flag alone, and the date would be gone at the next visit";

/// Why a flag isn't given a due date, when it isn't: a hosting page's tab.
pub fn due_refusal(s: &Session) -> Option<&'static str> {
    (!s.keeps_local).then_some(NO_DUE_HERE)
}

/// Why « Toutes les boîtes » and « Afficher les traités » are not offered: the list of a hosting page's tab has them (variant D),
/// the program that keeps its mail keeps upstream's list.
pub const HOSTED_ONLY: &str = "This view is offered in NeoMail only";

/// Why a view of the hosted list is not offered, when it isn't: the session keeps its mail.
pub fn hosted_refusal(s: &Session) -> Option<&'static str> {
    s.keeps_local.then_some(HOSTED_ONLY)
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
    fn feedback_and_get_support_open_no_tracker_in_a_tab_that_keeps_nothing() {
        let mut s = sample(false);
        for id in ["help.feedback", "help.support"] {
            let refused = s.run(id, &json!({})).unwrap_err().to_string();
            assert!(refused.contains(NO_SUPPORT_HERE), "{id}: {refused}");
            assert_eq!(s.registry.get(id).and_then(|spec| (spec.enabled)(&s)), Some(NO_SUPPORT_HERE), "{id}");
        }
        // The pages that only say what MailCraft is stay, and a session that keeps its own offers all.
        for id in ["help.help", "help.training", "help.whatsNew"] {
            assert!(s.run(id, &json!({})).is_ok(), "{id}");
        }
        let mut open = sample(true);
        for id in ["help.feedback", "help.support"] {
            assert!(open.run(id, &json!({})).is_ok(), "{id}");
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
        let data = mailcraft_charset::base64::STANDARD.encode(card);
        let refused = s.run("file.import", &json!({"name": "gus.vcf", "data": data})).unwrap_err().to_string();
        assert!(refused.contains(NO_IMPORT_HERE), "{refused}");
        // Kept here, they go on as before.
        let mut s = sample(true);
        s.run("people.favorite", &json!({"id": contact})).unwrap();
        s.run("file.import", &json!({"name": "gus.vcf", "data": data})).unwrap();
        assert!(s.mailbox.contacts.values().any(|c| c.first == "Gus" || c.display_name == "Gus Hale" || c.name() == "Gus Hale"));
    }

    #[test]
    fn the_order_of_folders_and_favorites_and_shaded_rows_are_refused_whoever_asks() {
        let mut s = sample(false);
        let account = s.mailbox.accounts[0].id;
        let folders: Vec<u64> = (0..2)
            .map(|n| s.run("folder.new", &json!({"name": format!("Project {n}"), "account": account})).unwrap()["id"].as_u64().unwrap())
            .collect();
        for id in &folders {
            s.run("folder.favorite", &json!({"id": id, "value": true})).unwrap();
        }
        let places = |s: &Session| folders.iter().map(|id| s.mailbox.folder(*id).map(|f| (f.place, f.favorite_place))).collect::<Vec<_>>();
        let before = places(&s);
        for id in ORDERED {
            let refused = s.run(id, &json!({"id": folders[1], "before": folders[0]})).unwrap_err().to_string();
            assert!(refused.contains(NO_ORDER_HERE), "{id}: {refused}");
            assert_eq!(s.registry.get(id).and_then(|spec| (spec.enabled)(&s)), Some(NO_ORDER_HERE), "{id}");
        }
        assert_eq!(places(&s), before, "nothing was put elsewhere");
        let refused = s.run(SHADED, &json!({"value": true})).unwrap_err().to_string();
        assert!(refused.contains(OWN_LIST) && !s.prefs.shade_rows, "{refused}");
        // Kept here, they run as upstream has them.
        s.keeps_local = true;
        for id in ORDERED {
            s.run(id, &json!({"id": folders[1], "before": folders[0]})).unwrap();
        }
        s.run(SHADED, &json!({"value": true})).unwrap();
        assert!(s.prefs.shade_rows);
    }

    /// A rule in a hosting page's tab: what its server keeps and what runs when the person runs it, and nothing else. A desktop
    /// alert, a flag's due day and a copy of a JMAP account's mail are refused, by `rules.add` and nothing of the rule is made.
    #[test]
    fn a_rule_shows_no_alert_flags_no_day_and_copies_nothing_in_a_hosting_pages_tab() {
        let mut s = sample(false);
        s.jmap_runs = true;
        let jmap = json!({"email": "ada@example.org", "kind": "jmap", "jmap": {"session": "https://jmap.example.org/session", "bearer": true}});
        let account = s.run("account.add", &jmap).unwrap()["id"].as_u64().unwrap();
        let inbox = s.mailbox.folder_of_kind(account, mailcraft_model::FolderKind::Inbox).unwrap();
        let rules = s.mailbox.rules.len();
        let add = |s: &mut Session, actions: serde_json::Value, alert: bool| {
            let rule = json!({"name": "Bills", "account": account, "conditions": [{"type": "subjectContains", "value": "invoice"}],
                "exceptions": [{"type": "subjectContains", "value": "reminder"}], "actions": actions, "alert": alert});
            s.run("rules.add", &rule)
        };
        for (actions, alert, why) in [
            (json!([{"type": "markRead"}]), true, NO_RULE_ALERT_HERE),
            (json!([]), true, NO_RULE_ALERT_HERE),
            (json!([{"type": "flagFor", "value": "today"}]), false, NO_DUE_HERE),
            (json!([{"type": "copyTo", "value": inbox}]), false, mailcraft_model::COPY_JMAP),
        ] {
            let refused = add(&mut s, actions.clone(), alert).unwrap_err().to_string();
            assert!(refused.contains(why), "{actions} {alert}: {refused}");
        }
        assert_eq!(s.mailbox.rules.len(), rules, "nothing was made");
        add(&mut s, json!([{"type": "flag"}, {"type": "markRead"}, {"type": "clearCategories"}]), false).unwrap();
        // The program that keeps its mail: an alert and a day too, and no copy of a JMAP account's mail all the same.
        s.keeps_local = true;
        add(&mut s, json!([{"type": "flagFor", "value": "today"}]), true).unwrap();
        assert!(add(&mut s, json!([{"type": "copyTo", "value": inbox}]), false).unwrap_err().to_string().contains(mailcraft_model::COPY_JMAP));
    }
}
