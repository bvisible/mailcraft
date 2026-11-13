// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): ready-made requests to Nora about the whole mailbox.

//! Recipes: ready-made requests to Nora about the person's whole mailbox, listed in her pane while the conversation is
//! empty (a morning summary, what needs a reply, what nobody answered), under the pane's greeting and in place of its hint
//! (`NoraPanel::show_with`). A click asks her, as if the person had typed it.
//!
//! A recipe is a request and nothing more: the words Nora is asked, as a few steps that name her mail tools (`nora_mail_news`,
//! `nora_mail_sent`, `nora_mail_unanswered`, `nora_mail_search`) and say that it is the mailbox and not the message that is
//! open. The model she runs on leaves out the arguments it is told to give (tried on 31.10: five identical calls, then
//! nothing), so the steps ask for the call that needs none, whose defaults are the right ones: what came in since yesterday
//! evening, what was sent in the last 7 days, the sent mail nobody answered in the last 14, and for the rest the latest
//! 20 messages of the inbox. Each of the first three says by itself when there is nothing. She answers through the
//! conversation like any other question, as data a stranger wrote and never as instructions; nothing is sent, deleted or
//! moved by one.

use egui::{Align2, CursorIcon, FontFamily, FontId, Role, Sense, Stroke, TextStyle, Ui, WidgetInfo, vec2};

use super::look;
use crate::theme::Tokens;

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
                        "Give me my morning summary, about my whole mailbox and not the message that is open. Call nora_mail_news once, with no argument: it lists what came in since yesterday evening, the unread first, and says so itself when nothing did: then say that no mail has come in since yesterday evening, and stop. Otherwise group it by what needs my attention (people and customers first, then newsletters and notifications), one line each: who, about what. Invent nothing."
                    ),
                ),
                recipe(
                    tl!("End-of-day summary"),
                    tl!(
                        "Give me my end-of-day summary, about my whole mailbox and not the message that is open. Step 1: call nora_mail_news once, with no argument: keep what arrived today and that I have not answered (answered is false). Step 2: call nora_mail_sent once, with no argument: keep what I sent today. Step 3: call nora_mail_unanswered once, with no argument: what I am still waiting for an answer to. Step 4: answer in three short lists (to answer, sent today, still waiting), one line each, and do not search again. Invent nothing."
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
                        "List the mail from the last 14 days that needs my reply, about my whole mailbox and not the message that is open. Step 1: call nora_mail_search once, with no argument (my latest 20 inbox messages). Step 2: keep what arrived in the last 14 days, was written by a person (not a newsletter or an automatic notification) and that I have not answered (answered is false); read the ones you are unsure about. Step 3: give each in one line: who, about what, since when. If there is none, say so. Invent nothing."
                    ),
                ),
                recipe(
                    tl!("Still without an answer"),
                    tl!(
                        "List the mail I sent in the last 14 days that nobody has answered yet, about my whole mailbox and not the message that is open. Call nora_mail_unanswered once, with no argument. Give each in one line, the oldest first: who I wrote to, about what, and how many days it has been waiting. If there is none, say how many sent messages it looked at (sent_looked_at) and that none is waiting."
                    ),
                ),
                recipe(
                    tl!("My commitments"),
                    tl!(
                        "List the commitments I made in the mail I sent in the last 7 days, about my whole mailbox and not the message that is open. Step 1: call nora_mail_sent once, with no argument: my sent messages of the last 7 days, each with the start of my own words. Step 2: one line per commitment: what I promised, to whom, and by when if the text says so; never a long quote. If there is none, say so."
                    ),
                ),
            ],
        },
    ]
}

/// What the pane shows while the conversation is empty, under its greeting: the hint it always had (`hint`), what `first`
/// draws (« Trier ma boîte », [`super::sorting_view::home_chip`]), then the recipes. The request of the one clicked, if any
/// (the panel sends it as if the person had typed it).
///
/// In the panel's own look, which it sets on the `Ui` it hands over (craft-nora-ui): its ink and its weak words, Karla for the
/// words (its families, or the window's own until they are installed), ink for the ring of a row reached by the keyboard;
/// Nora's clay only for a row under the pointer and its chevron.
pub(crate) fn show(ui: &mut Ui, t: &Tokens, hint: &str, first: &mut dyn FnMut(&mut Ui)) -> Option<String> {
    let tone = look::tone(t);
    let ink = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    // The family the panel set for each kind of text (no panic for one that isn't there: the window's own family).
    let sized =
        |style: TextStyle, size: f32| FontId::new(size, ui.style().text_styles.get(&style).map_or(FontFamily::Proportional, |f| f.family.clone()));
    let (title, heading, note) = (sized(TextStyle::Body, 13.5), sized(TextStyle::Button, 12.5), sized(TextStyle::Small, 11.5));
    let mut asked = None;
    ui.vertical_centered(|ui| ui.weak(hint));
    ui.add_space(14.0);
    first(ui);
    ui.label(egui::RichText::new(tl!("Recipes")).font(heading.clone()).color(weak));
    for group in groups() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(&group.heading).font(heading.clone()).color(ink));
        for recipe in &group.recipes {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &recipe.title));
            let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                if resp.hovered() || resp.is_pointer_button_down_on() {
                    painter.rect_filled(rect, 6.0, tone.fill);
                }
                painter.text(egui::pos2(rect.min.x + 8.0, rect.center().y), Align2::LEFT_CENTER, &recipe.title, title.clone(), ink);
                let chevron = egui::Rect::from_center_size(egui::pos2(rect.max.x - 12.0, rect.center().y), vec2(10.0, 10.0));
                crate::icons::paint(painter, chevron, "collapsed", tone.text, tone.text);
                painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(0.5, t.border));
                // Reached by the keyboard, a row says so.
                if resp.has_focus() {
                    painter.rect_stroke(rect, 6.0, Stroke::new(1.5, ink), egui::StrokeKind::Inside);
                }
            }
            if resp.clicked() {
                asked = Some(recipe.request.clone());
            }
        }
    }
    ui.add_space(8.0);
    ui.label(egui::RichText::new(tl!("Nora only reads your mail, with your own rights.")).font(note).color(weak));
    ui.add_space(8.0);
    asked
}

#[cfg(test)]
#[path = "recipes_tests.rs"]
mod tests;
