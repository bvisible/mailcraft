// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of `input_order`.
//! A click and what is typed never share a frame: what was typed before the click, the click, what was typed after it,
//! each in a frame of its own, in order. Nothing is lost or reordered; a frame without both is as it came.

use egui::{Event, Modifiers, PointerButton, RawInput, pos2};

use super::split;
use crate::MailApp;

fn press(pressed: bool) -> Event {
    Event::PointerButton { pos: pos2(10.0, 10.0), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn text(t: &str) -> Event {
    Event::Text(t.into())
}

fn raw(events: Vec<Event>) -> RawInput {
    RawInput { events, ..Default::default() }
}

/// The frames `came` makes, one input after the other, as long as some is held.
fn frames(came: Vec<Event>) -> Vec<Vec<Event>> {
    let (mut held, mut out) = (Vec::new(), Vec::new());
    let mut frame = raw(came);
    loop {
        let more = split(&mut held, &mut frame);
        out.push(std::mem::take(&mut frame.events));
        if !more {
            return out;
        }
        frame = raw(Vec::new());
    }
}

#[test]
fn what_was_typed_the_click_and_what_is_typed_after_it_are_three_frames() {
    let came = vec![text("Essai A"), Event::PointerMoved(pos2(10.0, 10.0)), press(true), press(false), text("Corps")];
    let got = frames(came.clone());
    assert_eq!(got, vec![vec![text("Essai A"), Event::PointerMoved(pos2(10.0, 10.0))], vec![press(true), press(false)], vec![text("Corps")]]);
    assert_eq!(got.concat(), came, "nothing lost, nothing reordered");
}

#[test]
fn several_clicks_with_keys_between_are_a_frame_each() {
    let came = vec![text("ada@example.org"), press(true), press(false), text("Body"), press(true), press(false), text("Subject")];
    let got = frames(came.clone());
    assert_eq!(
        got,
        vec![
            vec![text("ada@example.org")],
            vec![press(true), press(false)],
            vec![text("Body")],
            vec![press(true), press(false)],
            vec![text("Subject")]
        ]
    );
    assert_eq!(got.concat(), came);
}

#[test]
fn what_was_held_comes_first_in_the_next_frame() {
    let mut held = Vec::new();
    let mut frame = raw(vec![press(true), press(false), text("w")]);
    assert!(split(&mut held, &mut frame));
    assert_eq!((frame.events, held.clone()), (vec![press(true), press(false)], vec![text("w")]));
    let mut next = raw(vec![text("x")]);
    assert!(!split(&mut held, &mut next));
    assert_eq!(next.events, vec![text("w"), text("x")]);
    assert!(held.is_empty());
}

#[test]
fn a_frame_without_both_a_click_and_keys_is_as_it_came() {
    // A double click, the pointer moving on.
    let came = vec![press(true), press(false), press(true), press(false), Event::PointerMoved(pos2(12.0, 10.0))];
    assert_eq!(frames(came.clone()), vec![came]);
    // Keys alone, Ctrl+A among them.
    let came = vec![text("abc"), Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }];
    assert_eq!(frames(came.clone()), vec![came]);
}

#[test]
fn a_button_still_down_ends_its_frame_where_it_went_down() {
    assert_eq!(frames(vec![press(true), text("x")]), vec![vec![press(true)], vec![text("x")]]);
}

#[test]
fn the_window_holds_them_and_hands_them_over_frame_by_frame() {
    let mut app = MailApp::new(mailcraft_engine::Session::empty(), crate::Services::default());
    let mut got = Vec::new();
    let mut frame = raw(vec![text("q"), press(true), press(false), text("w")]);
    for _ in 0..4 {
        app.raw_input_hook(&mut frame);
        got.push(std::mem::take(&mut frame.events));
    }
    assert_eq!(got, vec![vec![text("q")], vec![press(true), press(false)], vec![text("w")], vec![]]);
    assert!(app.held_input.is_empty());
}
