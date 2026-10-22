// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): Nora's place in the ribbon.

//! Nora's place in the ribbon, when a host reaches her, in both its layouts: a Nora button in the
//! Respond group of Mail's Home tab, whose menu is the reading pane's button's (her pane, reply with
//! her, translate the message), and a Nora group in the tab of a message being written (her pane,
//! Correct, Improve, Translate and, for a mailbox the hosting page handed over, My style). So she can be reached with the
//! reading pane off, and from the ribbon's keys. The ribbon describes its controls as data; this adds
//! hers to that description once it is made, so that the ribbon's own tabs stay as they are.

use egui::Ui;
use mailcraft_engine::Module;
use serde_json::json;

use super::LANGUAGES;
use super::flow::page_mailbox;
use crate::MailApp;
use crate::ribbon::{Group, Item, Menu, Size, Tab};

/// The group of a message's tab hers comes after, and the group of Mail's Home tab her button is in.
const AFTER: &str = "Include";
const RESPOND: &str = "Respond";

/// A control of hers, before what it does and its place in the simplified row are said.
fn control(key: &'static str, icon: &'static str, label: &'static str, tip: &'static str, size: Size) -> Item {
    Item { key, icon, label, tip, command: None, menu: None, size, checked: false, section: 9, rank: 5, plain: false, under: None }
}

/// Add Nora's controls to `tab`, the ribbon's description of the tab in view, when a host reaches her.
pub(crate) fn add(app: &MailApp, tab: &mut Tab) {
    if app.services.nora.is_none() {
        return;
    }
    if app.ui.tab == "Message" {
        let Some(c) = app.session.compose.as_ref() else { return };
        let mut items = vec![
            Item {
                command: Some(("ui.noraCorrect", json!({}))),
                section: 3,
                rank: 4,
                ..control("ui.noraCorrect", "check", "Correct", "Spelling and grammar of your text", Size::Small)
            },
            Item {
                command: Some(("ui.noraImprove", json!({}))),
                section: 3,
                rank: 4,
                ..control("ui.noraImprove", "noraWand", "Improve", "A clearer version of your text", Size::Small)
            },
            Item { menu: Some(Menu::NoraTranslate), section: 3, rank: 4, ..control("noraTranslate", "globe", "Translate", "", Size::Small) },
        ];
        if super::assist::available(app) {
            items.insert(
                0,
                Item {
                    command: Some(("ui.noraAssistant", json!({"open": true}))),
                    section: 3,
                    rank: 4,
                    ..control("ui.noraAssistant", "nora", "Ask Nora", "Nora, the Neoffice assistant", Size::Small)
                },
            );
        }
        if page_mailbox(app, c.account).is_some() {
            items.push(Item {
                command: Some(("ui.noraLearnStyle", json!({}))),
                section: 3,
                rank: 5,
                ..control("ui.noraLearnStyle", "drafts", "My style", "Learn my writing style from my sent emails", Size::Small)
            });
        }
        let at = tab.groups.iter().position(|g| g.title == AFTER).map_or(tab.groups.len(), |i| i + 1);
        tab.groups.insert(at, Group { title: "Nora", items });
        return;
    }
    // Mail's Home tab is the one with a Respond group in the mail module.
    if app.session.view.module == Module::Mail
        && let Some(respond) = tab.groups.iter_mut().find(|g| g.title == RESPOND)
    {
        respond.items.push(Item {
            menu: Some(Menu::Nora),
            section: 2,
            rank: 3,
            ..control("nora", "nora", "Nora", "Nora, the Neoffice assistant", Size::Big)
        });
    }
}

/// What Nora's menus of the ribbon hold.
pub(crate) fn menu(ui: &mut Ui, app: &mut MailApp, menu: Menu) {
    match menu {
        Menu::Nora => {
            let Some(message) = app.session.sel.focus() else {
                // Her pane needs no message to be opened.
                if super::assist::available(app) {
                    crate::widgets::mi(ui, app, "Ask Nora", "ui.noraAssistant", json!({"open": true}));
                    ui.separator();
                }
                ui.label(tl!("Select a message first"));
                return;
            };
            super::pane::menu(ui, app, message);
        }
        Menu::NoraTranslate => {
            for (code, name) in LANGUAGES {
                crate::widgets::mi(ui, app, name, "ui.noraTranslateDraft", json!({"language": code}));
            }
        }
        _ => {}
    }
}
