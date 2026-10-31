// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Rename Folder and New Folder run by hand without a name (the ribbon's buttons, F2) ask for one: Rename Folder for the
//! folder in view's new name, under its own name, and New Folder for a folder in it.

use mailcraft_engine::Session;
use mailcraft_model::FolderKind;
use serde_json::json;

use crate::dialogs::{Dialog, FolderMode};
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

#[test]
fn rename_folder_asks_for_the_folder_s_new_name_and_new_folder_for_one_in_it() {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    let account = app.session.mailbox.accounts[0].id;
    let inbox = app.session.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let reports = app.session.run("folder.new", &json!({"name": "Quarterly reports", "parent": inbox})).unwrap()["id"].as_u64().unwrap();
    app.session.run("folder.select", &json!({"id": reports})).unwrap();
    for params in [json!({}), serde_json::Value::Null] {
        app.dialog = None;
        assert_eq!(app.run("folder.rename", params).unwrap()["pending"], "folder");
        match app.dialog.as_ref() {
            Some(Dialog::Folder { mode: FolderMode::Rename { id }, name, .. }) => assert_eq!((*id, name.as_str()), (reports, "Quarterly reports")),
            _ => panic!("Rename Folder didn't ask for the folder's new name"),
        }
    }
    app.dialog = None;
    assert_eq!(app.run("folder.new", json!({})).unwrap()["pending"], "folder");
    assert!(matches!(app.dialog, Some(Dialog::Folder { mode: FolderMode::New { parent: Some(p) }, .. }) if p == reports));
}
