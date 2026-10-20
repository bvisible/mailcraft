// //// Neoffice — added file (no upstream equivalent): Nora's band under the subject of the message being written.

//! Nora's band, under the subject of the message being written: her sphere and her name, then what
//! she does on the person's own text (Correct, Improve, Translate) and, when the message goes from a
//! mailbox the hosting page handed over, My style. Each pill runs its window command (`ui.nora*`),
//! as a script would. While she works, the pill that asked shows a spinner, the ones that can't run
//! now are greyed and say why when pointed at, and the band says what she is doing; what came of it
//! stays in the band for a while, as the status bar says it too.

use egui::{Margin, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use super::flow::page_mailbox;
use super::look::{self, Mark, State};
use super::{About, Kind, LANGUAGES, Rewrite};
use crate::MailApp;
use crate::theme::{Tokens, regular, semibold};

/// How long what came of something stays in the band, in milliseconds.
pub(crate) const SAID_MS: f64 = 15_000.0;

/// What Nora is doing for the message being written, in a few words; `None` when nothing.
pub(crate) fn working(app: &MailApp) -> Option<&'static str> {
    let nora = &app.nora;
    if nora.asking(Kind::Reply) {
        Some(tl!("Nora is writing the reply…"))
    } else if nora.asking(Kind::Rewrite) {
        Some(tl!("Nora is rewriting the text…"))
    } else if nora.asking(Kind::Style) {
        Some(tl!("Starting style analysis…"))
    } else {
        None
    }
}

/// Nora's band, its pills lined up with the text of the fields above it (`indent` from the left).
pub(crate) fn band(app: &mut MailApp, ui: &mut Ui, indent: f32) {
    let t = Tokens::get(ui.ctx());
    let tone = look::tone(&t);
    let now = crate::now_ms();
    let said = app.nora.said.clone().filter(|s| s.about == About::Writing && (0.0..SAID_MS).contains(&(now - s.at)));
    let doing = working(app);
    let rewriting = app.nora.rewriting();
    let style = app.session.compose.as_ref().is_some_and(|c| page_mailbox(app, c.account).is_some());
    let margin = Margin { left: indent.clamp(0.0, 127.0) as i8, right: 12, top: 4, bottom: 4 };
    let shown = egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
        // As wide as the fields above, whose lines its own line continues.
        ui.set_min_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            let (mark, _) = ui.allocate_exact_size(vec2(16.0, look::PILL_H), Sense::hover());
            look::orb(ui.painter(), mark.shrink2(vec2(0.0, 4.0)), tone.orb);
            ui.label(RichText::new(tl!("Nora")).font(semibold(12.5)).color(tone.text));
            let correct = Pill { label: tl!("Correct"), mark: Mark::Check, menu: false, kind: "noraCorrect", command: "ui.noraCorrect" };
            if correct.show(app, ui, &t, rewriting == Some(Rewrite::Correct), doing, tl!("Spelling and grammar of your text")) {
                let _ = app.run("ui.noraCorrect", json!({}));
            }
            let improve = Pill { label: tl!("Improve"), mark: Mark::Wand, menu: false, kind: "noraImprove", command: "ui.noraImprove" };
            if improve.show(app, ui, &t, rewriting == Some(Rewrite::Improve), doing, tl!("A clearer version of your text")) {
                let _ = app.run("ui.noraImprove", json!({}));
            }
            let translate = Pill { label: tl!("Translate"), mark: Mark::Globe, menu: true, kind: "noraTranslate", command: "ui.noraTranslateDraft" };
            let translating = matches!(rewriting, Some(Rewrite::Translate(_)));
            if let Some(resp) = translate.response(app, ui, &t, translating, doing, "") {
                egui::Popup::menu(&resp).show(|ui| {
                    for (code, name) in LANGUAGES {
                        crate::widgets::mi(ui, app, name, "ui.noraTranslateDraft", json!({"language": code}));
                    }
                });
            }
            if style {
                let mine = Pill { label: tl!("My style"), mark: Mark::Pen, menu: false, kind: "noraStyle", command: "ui.noraLearnStyle" };
                if mine.show(app, ui, &t, app.nora.asking(Kind::Style), doing, tl!("Learn my writing style from my sent emails")) {
                    let _ = app.run("ui.noraLearnStyle", json!({}));
                }
            }
            // What she is doing, or what came of it.
            if let Some(doing) = doing {
                if rewriting.is_none() && !app.nora.asking(Kind::Style) {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, look::PILL_H), Sense::hover());
                    egui::Spinner::new().size(12.0).color(tone.text).paint_at(ui, r.shrink2(vec2(0.0, 5.0)));
                }
                ui.add(egui::Label::new(RichText::new(doing).font(regular(11.5)).color(t.text_dim)).wrap());
            } else if let Some(said) = &said {
                let color = if said.error { t.red } else { t.text_dim };
                ui.add(egui::Label::new(RichText::new(said.text.as_str()).font(regular(11.5)).color(color)).wrap());
            }
        });
    });
    let r = shown.response.rect;
    ui.painter().line_segment([pos2(r.min.x + 16.0, r.max.y), pos2(r.max.x - 12.0, r.max.y)], Stroke::new(1.0, t.border));
    // The line goes when its time is up, without waiting for the pointer to move.
    if let Some(said) = said {
        let left = (SAID_MS - (now - said.at)).clamp(0.0, SAID_MS);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(left as u64 + 50));
    }
}

/// A pill of the band: what it shows, what the hit map calls it, the command it runs.
pub(crate) struct Pill<'a> {
    pub(crate) label: &'a str,
    pub(crate) mark: Mark,
    pub(crate) menu: bool,
    pub(crate) kind: &'static str,
    pub(crate) command: &'static str,
}

impl Pill<'_> {
    /// Draw it: greyed when its command can't run now, a spinner in place of its mark while it is
    /// `spinning`; pointed at, what it does (`tip`), or why it can't run now (`doing`). Its response
    /// when it can be clicked.
    pub(crate) fn response(
        &self,
        app: &mut MailApp,
        ui: &mut Ui,
        t: &Tokens,
        spinning: bool,
        doing: Option<&str>,
        tip: &str,
    ) -> Option<egui::Response> {
        let enabled = app.enabled(self.command);
        let state = match (spinning, enabled) {
            (true, _) => State::Spinning,
            (false, true) => State::Live,
            (false, false) => State::Greyed,
        };
        let resp = look::pill(ui, t, self.label, self.mark, self.menu, state);
        app.hit.add(self.kind, 0, resp.rect);
        let said = match (state, doing) {
            (State::Live, _) | (_, None) => tip,
            (_, Some(doing)) => doing,
        };
        let resp = if said.is_empty() { resp } else { resp.on_hover_text(said) };
        (state == State::Live).then_some(resp)
    }

    /// [`Pill::response`], and whether it was clicked.
    pub(crate) fn show(&self, app: &mut MailApp, ui: &mut Ui, t: &Tokens, spinning: bool, doing: Option<&str>, tip: &str) -> bool {
        self.response(app, ui, t, spinning, doing, tip).is_some_and(|resp| resp.clicked())
    }
}
