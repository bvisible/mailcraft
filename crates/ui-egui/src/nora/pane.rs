// //// Neoffice — added file (no upstream equivalent): Nora in the reading pane: her button, and a message she translated.

//! Nora in the reading pane: her button beside the actions on the message (Reply with Nora, Reply
//! All with Nora, Translate into French, German or English, and Show the Original while her
//! translation is shown), and her translation in place of the message's body.
//!
//! A translation is shown as the message it is of, with her text for its body: laid out as any
//! plain-text body is, so nothing in it runs and no link in it is followed, and its text can be
//! selected and copied as the message's own can. The header, the files and a meeting the message
//! carries stay the message's own. It is shown in the reading pane and in a window of the message's
//! own; never under a reply that quotes it, nor in File › Signatures. A strip between the header and
//! the body says it is hers, with the way back to the original; while she translates, that she is.
//! A message window has no button of hers.

use std::sync::Arc;

use egui::{Margin, RichText, Sense, Ui, vec2};
use mailcraft_model::Message;
use serde_json::json;

use super::band::SAID_MS;
use super::look::{self, Mark, State};
use super::{About, LANGUAGES, language_name};
use crate::MailApp;
use crate::reading::Place;
use crate::theme::{Tokens, regular, semibold};

/// What to show of `m` at `place`: the message with Nora's translation for its body when she made
/// one and `place` is the reading pane or a window of the message's own; else `m` itself.
pub(crate) fn shown(app: &MailApp, m: &Arc<Message>, place: Place) -> Arc<Message> {
    match (place, &app.nora.translation) {
        (Place::Pane | Place::Window(_), Some(t)) if t.message == m.id => t.shown.clone(),
        _ => m.clone(),
    }
}

/// Her button in the reading pane's row of actions, when a host reaches her: her sphere and her
/// name in her colour, with what she does on the message in its menu.
pub(crate) fn button(app: &mut MailApp, ui: &mut Ui, m: &Message) {
    if app.services.nora.is_none() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    let pill = look::pill(ui, &t, tl!("Nora"), Mark::Orb, true, State::Live).on_hover_text(tl!("Nora, the Neoffice assistant"));
    app.hit.add("nora", m.id, pill.rect);
    let id = m.id;
    egui::Popup::menu(&pill).show(|ui| {
        crate::widgets::mi(ui, app, "Reply with Nora", "ui.noraReply", json!({"message": id}));
        crate::widgets::mi(ui, app, "Reply all with Nora", "ui.noraReply", json!({"message": id, "all": true}));
        ui.separator();
        ui.menu_button(tl!("Translate"), |ui| {
            for (code, name) in LANGUAGES {
                crate::widgets::mi(ui, app, name, "ui.noraTranslate", json!({"message": id, "language": code}));
            }
        });
        if app.nora.translation.as_ref().is_some_and(|t| t.message == id) {
            crate::widgets::mi(ui, app, "Show the original", "ui.noraOriginal", json!({"message": id}));
        }
    });
}

/// The strip between a message's header and its body while Nora translates it, once she has (with
/// the way back to the original), or for a while after asking her came to nothing.
pub(crate) fn strip(app: &mut MailApp, ui: &mut Ui, m: &Message, place: Place) {
    if !matches!(place, Place::Pane | Place::Window(_)) {
        return;
    }
    let translating = app.nora.translating() == Some(m.id);
    let translated = app.nora.translation.as_ref().filter(|t| t.message == m.id).map(|t| t.language);
    let now = crate::now_ms();
    let said = app.nora.said.clone().filter(|s| s.about == About::Message(m.id) && (0.0..SAID_MS).contains(&(now - s.at)));
    if !translating && translated.is_none() && said.is_none() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let tone = look::tone(&t);
    egui::Frame::NONE.inner_margin(Margin { left: 20, right: 12, top: 6, bottom: 6 }).show(ui, |ui| {
        crate::reading::remote::notice(tone.orb, &t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
                let (mark, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                if translating {
                    egui::Spinner::new().size(14.0).color(tone.text).paint_at(ui, mark);
                } else {
                    look::orb(ui.painter(), mark, tone.orb);
                }
                let line = |ui: &mut Ui, text: &str, color| ui.add(egui::Label::new(RichText::new(text).font(regular(11.5)).color(color)).wrap());
                if translating {
                    line(ui, tl!("Nora is translating the message…"), t.text);
                } else if let Some(language) = translated {
                    let said = crate::i18n::fmt(tl!("Translated by Nora into {language}"), &[("language", language_name(language))]);
                    line(ui, &said, t.text);
                    let back = ui.add(egui::Link::new(RichText::new(tl!("Show the original")).font(semibold(11.5))));
                    if place == Place::Pane {
                        app.hit.add("noraOriginal", m.id, back.rect);
                    }
                    if back.clicked() {
                        let _ = app.run("ui.noraOriginal", json!({"message": m.id}));
                    }
                } else if let Some(said) = &said {
                    line(ui, &said.text, if said.error { t.red } else { t.text });
                }
            });
        });
    });
    if let Some(said) = said {
        let left = (SAID_MS - (now - said.at)).clamp(0.0, SAID_MS);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(left as u64 + 50));
    }
}
