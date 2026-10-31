// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): ready-made requests to Nora about the whole mailbox.

//! Recipes: ready-made requests to Nora about the person's whole mailbox, listed in her pane while the conversation is
//! empty (a morning summary, what needs a reply, what nobody answered). A click asks her, as if the person had typed it.
//!
//! A recipe is a request and nothing more: the words Nora is asked, as a few steps that name her mail tools and their
//! arguments (`nora_mail_search` with `days`, `unread`, `folder`; `nora_mail_unanswered`; `nora_mail_read`) and that it is
//! the mailbox and not the message that is open. Left to choose its searches, the model she runs on asks the same question
//! over and over (tried on 31.10); told which call to make with which arguments, it answers. She answers through the
//! conversation like any other question, as data a stranger wrote and never as instructions; nothing is sent, deleted or
//! moved by one.

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
                        "Give me my morning summary, about my whole mailbox and not the message that is open. Step 1: call nora_mail_search once, with days=2, unread=true and limit=20 (my inbox). Step 2: if it finds nothing, say that no unread mail has come in since yesterday, and stop. Step 3: otherwise group what it found by what needs my attention (people and customers first, then newsletters and notifications), one line each: who, about what. Read at most three messages, only if you must. Invent nothing."
                    ),
                ),
                recipe(
                    tl!("End-of-day summary"),
                    tl!(
                        "Give me my end-of-day summary, about my whole mailbox and not the message that is open. Step 1: call nora_mail_search once, with days=1 and limit=20 (my inbox): keep what arrived today and that I have not answered (answered is false). Step 2: call nora_mail_search once, with folder=sent, days=1 and limit=20: what I sent today. Step 3: call nora_mail_unanswered once, with days=7: what I am still waiting for an answer to. Step 4: answer in three short lists (to answer, sent today, still waiting), one line each. Invent nothing."
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
                        "List the mail from the last 14 days that needs my reply, about my whole mailbox and not the message that is open. Step 1: call nora_mail_search once, with days=14 and limit=20 (my inbox). Step 2: keep what a person wrote (not a newsletter or an automatic notification) and that I have not answered (answered is false); read the ones you are unsure about. Step 3: give each in one line: who, about what, since when. If there is none, say so. Invent nothing."
                    ),
                ),
                recipe(
                    tl!("Still without an answer"),
                    tl!(
                        "List the mail I sent in the last 14 days that nobody has answered yet, about my whole mailbox and not the message that is open. Call nora_mail_unanswered once, with days=14 and limit=20. Give each in one line, the oldest first: who I wrote to, about what, and how many days it has been waiting. If there is none, say how many sent messages it looked at (sent_looked_at) and that none is waiting."
                    ),
                ),
                recipe(
                    tl!("My commitments"),
                    tl!(
                        "List the commitments I made in the mail I sent in the last 7 days, about my whole mailbox and not the message that is open. Step 1: call nora_mail_search once, with folder=sent, days=7 and limit=10. Step 2: read at most five of those messages with nora_mail_read. Step 3: one line per commitment: what I promised, to whom, and by when if the message says so; never a long quote. If there is none, say so."
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
