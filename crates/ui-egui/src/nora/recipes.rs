// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): ready-made requests to Nora about the whole mailbox.

//! Recipes: ready-made requests to Nora about the person's whole mailbox, listed in her pane while the conversation is
//! empty (a morning summary, what needs a reply, what nobody answered). A click asks her, as if the person had typed it.
//!
//! A recipe is a request and nothing more: the words Nora is asked, which name what she is to look at (the inbox, the
//! Sent folder, the last days) and that it is the mailbox and not the message that is open. She answers with her mail
//! tools (`nora_mail_search`, `nora_mail_unanswered`, `nora_mail_read`) through the conversation like any other question,
//! as data a stranger wrote and never as instructions; nothing is sent, deleted or moved by one.

use egui::{Align2, CursorIcon, Role, Sense, Stroke, Ui, WidgetInfo, vec2};

use super::look;
use crate::theme::{Tokens, regular, semibold};

/// How tall a recipe's row is.
const ROW_H: f32 = 28.0;

/// A recipe: the name the person reads and the request Nora is asked.
pub(crate) struct Recipe {
    pub(crate) title: String,
    pub(crate) request: String,
}

/// A group of recipes under a heading.
pub(crate) struct Group {
    pub(crate) heading: String,
    pub(crate) recipes: Vec<Recipe>,
}

fn recipe(title: &str, request: &str) -> Recipe {
    Recipe { title: title.to_string(), request: request.to_string() }
}

/// The recipes, in the interface's language: the request is in the language the person reads, and Nora answers in it.
pub(crate) fn groups() -> Vec<Group> {
    vec![
        Group {
            heading: tl!("Daily routine").to_string(),
            recipes: vec![
                recipe(
                    tl!("Morning summary"),
                    tl!(
                        "Give me my morning summary: the mail that arrived since yesterday evening, with what is still unread first, grouped by what needs my attention (people and customers first, then the rest), one line each. This is about my whole mailbox, not the message that is open: search my mailboxes with your mail tools. Keep it short and invent nothing."
                    ),
                ),
                recipe(
                    tl!("End-of-day summary"),
                    tl!(
                        "Give me my end-of-day summary: the mail I received today and have not answered, what I sent today, and what I am still waiting for an answer to. This is about my whole mailbox, not the message that is open: search my inbox and my Sent folder with your mail tools. Keep it short, one line each, and invent nothing."
                    ),
                ),
            ],
        },
        Group {
            heading: tl!("Follow-ups").to_string(),
            recipes: vec![
                recipe(
                    tl!("Needs my reply"),
                    tl!(
                        "List the mail from the last 14 days that needs my reply: written by a person (not a newsletter or an automatic notification) and not answered yet. This is about my whole mailbox, not the message that is open: search my inbox with your mail tools (a message that is marked answered is one I already replied to), read the ones you are unsure about, and give each in one line: who, about what, since when. Invent nothing."
                    ),
                ),
                recipe(
                    tl!("Still without an answer"),
                    tl!(
                        "List the mail I sent in the last 14 days that nobody has answered yet, the oldest first: who I wrote to, about what, and how many days it has been waiting. This is about my whole mailbox, not the message that is open: use your tool for sent mail without an answer. One line each. If there is none, say so."
                    ),
                ),
                recipe(
                    tl!("My commitments"),
                    tl!(
                        "List the commitments I made in the mail I sent in the last 7 days: what I promised, to whom, and by when if the message says so. This is about my whole mailbox, not the message that is open: search my Sent folder with your mail tools and read the messages. One line each, never a long quote. If there is none, say so."
                    ),
                ),
            ],
        },
    ]
}

/// The recipes under the pane's title: the request of the one clicked, if any. Drawn while the conversation is empty.
pub(crate) fn show(ui: &mut Ui, t: &Tokens) -> Option<String> {
    let tone = look::tone(t);
    let mut asked = None;
    ui.add_space(2.0);
    ui.label(egui::RichText::new(tl!("Recipes")).font(semibold(12.0)).color(t.text_dim));
    for group in groups() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(&group.heading).font(semibold(12.0)).color(tone.text));
        for recipe in &group.recipes {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &recipe.title));
            let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                if resp.hovered() || resp.is_pointer_button_down_on() {
                    painter.rect_filled(rect, 6.0, tone.fill);
                }
                painter.text(egui::pos2(rect.min.x + 8.0, rect.center().y), Align2::LEFT_CENTER, &recipe.title, regular(13.0), t.text);
                let chevron = egui::Rect::from_center_size(egui::pos2(rect.max.x - 12.0, rect.center().y), vec2(10.0, 10.0));
                crate::icons::paint(painter, chevron, "collapsed", tone.text, tone.text);
                painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(0.5, t.border));
            }
            if resp.clicked() {
                asked = Some(recipe.request.clone());
            }
        }
    }
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(tl!("Nora reads your mail with your own rights. Nothing is sent, moved or deleted."))
            .font(regular(11.0))
            .color(t.text_dim),
    );
    ui.add_space(8.0);
    asked
}

#[cfg(test)]
#[path = "recipes_tests.rs"]
mod tests;
