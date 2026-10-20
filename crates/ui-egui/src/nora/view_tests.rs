// //// Neoffice — added file (no upstream equivalent): Nora in the window, through real frames.
//! Nora in the window, through real frames: the band under the subject of a message being written
//! (there only with a host, its pills each its command, greyed while she works, in both themes and
//! at every width), and what a hostile line of hers does to it: nothing.

use egui::{Pos2, Rect, vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;

use super::testing::*;
use super::{About, PROOFREAD};
use crate::MailApp;

type Window = egui_kittest::Harness<'static, MailApp>;

/// The whole window, through real frames.
fn window(app: MailApp, size: (f32, f32)) -> Window {
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(size.0, size.1)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(4);
    // The sample has reminders due, and their window would lie over the panes.
    let _ = harness.state_mut().execute("ui.dismissReminder", json!({"all": true}));
    harness.run_steps(3);
    harness
}

fn click_at(harness: &mut Window, pos: Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        harness.step();
    }
    harness.run_steps(3);
}

/// Where a pill of the band was drawn in the last frame.
fn pill(harness: &Window, kind: &str) -> Option<Rect> {
    harness.state().hit.rect(kind, 0)
}

fn run(harness: &mut Window, command: &str, params: serde_json::Value) {
    harness.state_mut().run(command, params).unwrap();
    harness.run_steps(3);
}

const PILLS: [&str; 3] = ["noraCorrect", "noraImprove", "noraTranslate"];

#[test]
fn the_band_is_there_with_a_host_and_a_message_being_written_only() {
    let (app, _calls, _) = sample(PLAIN);
    let mut harness = window(app, (1440.0, 900.0));
    assert!(PILLS.iter().all(|kind| pill(&harness, kind).is_none()), "nothing is being written");
    run(&mut harness, "mail.reply", json!({}));
    for kind in PILLS {
        let r = pill(&harness, kind).unwrap_or_else(|| panic!("no {kind}"));
        assert!(r.height() >= 20.0 && r.width() > 40.0, "{kind}: {r:?}");
    }
    // In a row, in that order, under the subject and above the body.
    let (correct, improve, translate) =
        (pill(&harness, "noraCorrect").unwrap(), pill(&harness, "noraImprove").unwrap(), pill(&harness, "noraTranslate").unwrap());
    assert!(correct.max.x <= improve.min.x && improve.max.x <= translate.min.x);
    assert!((correct.center().y - translate.center().y).abs() < 1.0);
    assert!(harness.query_all_by_label("Nora").next().is_some());
    run(&mut harness, "compose.discard", json!({}));
    assert!(PILLS.iter().all(|kind| pill(&harness, kind).is_none()));
    // Without a host, no band.
    let mut app = quiet(MailApp::new(mailcraft_engine::Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), crate::Services::default()));
    let id = app.session.mailbox.messages.values().find(|m| m.subject == PLAIN).map(|m| m.id).unwrap();
    app.execute("mail.select", json!({"id": id})).unwrap();
    let mut harness = window(app, (1440.0, 900.0));
    run(&mut harness, "mail.reply", json!({}));
    assert!(PILLS.iter().all(|kind| pill(&harness, kind).is_none()), "no host, no Nora");
}

#[test]
fn my_style_is_there_for_the_pages_mailboxes_only() {
    let (app, _calls, _) = sample(PLAIN);
    let mut harness = window(app, (1440.0, 900.0));
    run(&mut harness, "mail.reply", json!({}));
    assert!(pill(&harness, "noraCorrect").is_some() && pill(&harness, "noraStyle").is_none(), "the sample mailbox has no style on the server");
    let (app, _calls, account) = page(true);
    let mut harness = window(app, (1440.0, 900.0));
    run(&mut harness, "mail.new", json!({"account": account}));
    assert!(pill(&harness, "noraStyle").is_some());
}

#[test]
fn a_click_on_correct_asks_nora_and_the_pills_are_greyed_while_she_works() {
    let (app, calls, _) = sample(PLAIN);
    let mut harness = window(app, (1440.0, 900.0));
    run(&mut harness, "mail.reply", json!({}));
    typed(harness.state_mut(), "bonjour priya");
    harness.run_steps(2);
    let at = pill(&harness, "noraCorrect").unwrap().center();
    click_at(&mut harness, at);
    assert_eq!(calls.count(), 1);
    assert_eq!(calls.last().method, PROOFREAD);
    assert_eq!(calls.last().args["html_content"], "<p>bonjour priya</p>");
    // While she works: the pills that can't run are greyed, and the band says what she is doing.
    assert!(!harness.state().enabled("ui.noraImprove"));
    let improve = harness.query_all_by_label("Improve").collect::<Vec<_>>();
    assert!(!improve.is_empty() && improve.iter().all(|node| node.accesskit_node().is_disabled()));
    assert_eq!(super::band::working(harness.state()), Some("Nora is rewriting the text…"));
    assert_eq!(status(harness.state()), "Nora is rewriting the text…");
    // A click on a greyed pill asks nothing.
    let at = pill(&harness, "noraImprove").unwrap().center();
    click_at(&mut harness, at);
    assert_eq!(calls.count(), 1);
    // Her version comes: the band says so, and the pills are live again.
    calls.answer(200, json!({"message": {"corrected_text": "Bonjour Priya.", "has_changes": true}}));
    harness.run_steps(3);
    assert!(body(harness.state()).starts_with("Bonjour Priya.\n"));
    assert_eq!(said(harness.state()).map(|s| s.0), Some("Nora's version is in the message: Undo takes it back.".to_string()));
    assert_eq!(super::band::working(harness.state()), None);
    assert!(harness.state().enabled("ui.noraImprove"));
    assert!(harness.query_all_by_label("Improve").all(|node| !node.accesskit_node().is_disabled()));
}

#[test]
fn the_translate_pill_opens_its_languages() {
    let (app, calls, _) = sample(PLAIN);
    let mut harness = window(app, (1440.0, 900.0));
    run(&mut harness, "mail.reply", json!({}));
    typed(harness.state_mut(), "Thursday works.");
    harness.run_steps(2);
    let at = pill(&harness, "noraTranslate").unwrap().center();
    click_at(&mut harness, at);
    assert!(egui::Popup::is_any_open(&harness.ctx));
    harness.get_by_label_contains("German").click();
    harness.run_steps(3);
    assert_eq!(calls.count(), 1);
    assert_eq!(
        (calls.last().args["target_language"].as_str(), calls.last().args["html_content"].as_str()),
        (Some("de"), Some("<p>Thursday works.</p>"))
    );
}

#[test]
fn the_band_draws_in_light_and_dark_at_narrow_widths_without_panicking() {
    for theme in ["light", "dark"] {
        for size in [(1440.0, 900.0), (600.0, 800.0), (320.0, 560.0)] {
            let (mut app, calls, account) = page(true);
            app.execute("ui.theme", json!({"value": theme})).unwrap();
            let mut harness = window(app, size);
            run(&mut harness, "mail.new", json!({"account": account}));
            typed(harness.state_mut(), "Hello Grace");
            run(&mut harness, "ui.noraImprove", json!({}));
            harness.run_steps(3);
            calls.answer(200, json!({"message": {"improved_text": "Hello Grace, thank you.", "has_changes": true}}));
            harness.run_steps(3);
            run(&mut harness, "ui.noraLearnStyle", json!({}));
            harness.run_steps(2);
            // However narrow, the pills are all there, whole, and none lies over another (a narrow pane wraps them).
            let pills: Vec<Rect> = ["noraCorrect", "noraImprove", "noraTranslate", "noraStyle"]
                .iter()
                .map(|kind| pill(&harness, kind).unwrap_or_else(|| panic!("{theme} {size:?}: no {kind}")))
                .collect();
            for (i, a) in pills.iter().enumerate() {
                assert!((a.height() - super::look::PILL_H).abs() < 0.5, "{theme} {size:?}: {a:?}");
                for b in pills.iter().skip(i + 1) {
                    assert!(!a.shrink(0.5).intersects(b.shrink(0.5)), "{theme} {size:?}: {a:?} and {b:?}");
                }
            }
            if size.0 >= 1440.0 {
                assert!(pills.iter().all(|r| r.max.x <= size.0), "{theme} {size:?}: {pills:?}");
            }
        }
    }
}

#[test]
fn a_hostile_line_in_the_band_lays_out() {
    let hostile = [
        "x".repeat(20_000),
        "\u{202e}txet desrever\u{202c} and \u{2067}more\u{2069}".to_string(),
        "\u{0}\u{7}\u{1b}[2J\r\n\t\u{feff}".to_string(),
        "مرحبا بكم في نورا ".repeat(200),
        "🧪".repeat(3_000),
    ];
    for text in hostile {
        let (app, _calls, _) = sample(PLAIN);
        let mut harness = window(app, (600.0, 800.0));
        run(&mut harness, "mail.reply", json!({}));
        let error = text.len() % 2 == 0;
        harness.state_mut().nora.said = Some(super::flow::Said { text, error, at: crate::now_ms(), about: About::Writing });
        harness.run_steps(4);
        // The band and the body are still where they were: the band's pills are on screen.
        assert!(pill(&harness, "noraCorrect").is_some());
    }
}

/// Nora in the window, in light and dark, wide and narrow, as PNG files to look at:
/// `MAILCRAFT_SHOTS_DIR=<dir> cargo test -p mailcraft-ui-egui shots_of_nora -- --ignored`
#[test]
#[ignore = "writes screenshots, for looking at them"]
fn shots_of_nora() {
    let Ok(dir) = std::env::var("MAILCRAFT_SHOTS_DIR") else {
        eprintln!("set MAILCRAFT_SHOTS_DIR to the directory to write to");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    for theme in ["light", "dark"] {
        for (width, height) in [(1440.0, 900.0), (900.0, 720.0)] {
            let (mut app, calls, account) = page(true);
            app.execute("ui.theme", json!({"value": theme})).unwrap();
            // A narrow window without its folder pane: the message being written is under 500 points wide.
            if width < 1000.0 {
                app.execute("view.folderPane", json!({"value": false})).unwrap();
            }
            let mut harness = egui_kittest::Harness::builder()
                .with_size(vec2(width, height))
                .with_pixels_per_point(2.0)
                .with_step_dt(1.0 / 60.0)
                .wgpu()
                .build_ui_state(
                    |ui, app: &mut MailApp| {
                        let ctx = ui.ctx().clone();
                        app.logic(&ctx);
                        app.ui(ui);
                    },
                    app,
                );
            harness.input_mut().max_texture_side = Some(8192);
            harness.run_steps(4);
            let shot = |harness: &mut Window, name: &str| {
                harness.run_steps(10);
                let path = dir.join(format!("nora-{name}-{theme}-{width}.png"));
                harness.render().unwrap().save(&path).unwrap();
                println!("{}", path.display());
            };
            run(&mut harness, "mail.new", json!({"account": account}));
            typed(harness.state_mut(), "hello grace,\n\nthe proof is ready, i will send it tomorow");
            shot(&mut harness, "band-1-idle");
            run(&mut harness, "ui.noraCorrect", json!({}));
            shot(&mut harness, "band-2-working");
            calls.answer(
                200,
                json!({"message": {"corrected_text": "Hello Grace,\n\nThe proof is ready; I will send it tomorrow.", "has_changes": true}}),
            );
            shot(&mut harness, "band-3-done");
            run(&mut harness, "ui.noraTranslateDraft", json!({"language": "de"}));
            calls.answer(
                417,
                refusal("Nora refused to process this message: it contains instructions aimed at the assistant, or could not be verified."),
            );
            shot(&mut harness, "band-4-refused");
        }
    }
}
