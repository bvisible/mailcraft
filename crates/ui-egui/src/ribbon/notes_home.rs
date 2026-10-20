// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Notes' Home tab: a new note, and what may be done to the selected one (each button greyed by the engine where it
//! can't run, the refusal said when a note's rights refuse it).

use super::{Item, Tab, g, no};
use crate::MailApp;

pub(super) fn home(app: &MailApp) -> Tab {
    let note = app.session.sel.note.and_then(|id| app.session.mailbox.notes.note(id));
    let (pinned, archived) = (note.is_some_and(|note| note.pinned), note.is_some_and(|note| note.archived));
    Tab::of(vec![
        g("New", vec![Item::big("notes", "New\nNote", "notes.new", no()).at(0, 1)]),
        g("Delete", vec![Item::big("delete", "Delete", "notes.delete", no()).at(1, 2).plain()]),
        g(
            "Actions",
            vec![
                Item::big("pin", if pinned { "Unpin" } else { "Pin" }, "notes.pin", no()).checked(pinned).at(2, 2),
                Item::big("archive", if archived { "Unarchive" } else { "Archive" }, "notes.archive", no()).checked(archived).at(2, 3),
                Item::big("globe", "Open on\nthe Web", "ui.openNote", no()).at(2, 3),
            ],
        ),
    ])
}
