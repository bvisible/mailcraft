// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): each field of a message being written takes the keys typed in it.
//! A browser that draws late hands a frame keys typed in one field and the click that moves the keyboard to another. The
//! window gives the click and the keys around it frames of their own (`crate::input_order`): what is typed in the Subject
//! or To stays there, what is typed in the body stays in the body, whatever order the fields are typed in. And the body,
//! given such a frame whole, takes only its own keys: none of those before the click that gave it the keyboard, all of
//! those before the click that took the keyboard away.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;

use super::editor;
use super::tests::{Window, app, plain, type_in, window};

/// A new message to Maya whose Subject is `subject` and whose body is empty, the keyboard in the body when `in_body`.
fn writing(subject: &str, in_body: bool) -> Window {
    let mut a = app();
    a.execute("mail.new", json!({"to": "maya@lumen.example", "subject": subject})).unwrap();
    a.execute("compose.set", json!({"body": ""})).unwrap();
    a.execute("compose.select", json!({"from": 0})).unwrap();
    if in_body {
        a.want_body_focus();
    }
    let mut h = window(a);
    h.run_steps(2);
    h
}

fn subject(h: &Window) -> String {
    h.state().session.compose.as_ref().unwrap().subject.clone()
}

fn to(h: &Window) -> String {
    h.state().session.compose.as_ref().unwrap().to.clone()
}

/// The text field of the message's lines that holds `holds`.
fn field(h: &Window, holds: &str) -> Rect {
    h.query_all_by_value(holds).find(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput).expect("the field").rect()
}

/// Where a click goes at the end of the field holding `holds` (past its text: the caret goes to its end).
fn end_of(h: &Window, holds: &str) -> Pos2 {
    field(h, holds).right_center() - vec2(4.0, 0.0)
}

fn body_at(h: &Window) -> Pos2 {
    h.state().hit.rect("composeBody", 0).unwrap().min + vec2(8.0, 8.0)
}

fn click(at: Pos2) -> [Event; 3] {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    [Event::PointerMoved(at), button(true), button(false)]
}

/// `events` in ONE frame, as egui gets it (`Harness::event` makes a frame of each event): the body's own reading.
fn one_frame(h: &mut Window, events: impl IntoIterator<Item = Event>) {
    h.input_mut().events.extend(events);
    h.step();
    h.run_steps(2);
}

/// `events` come at once, and the window takes them as a browser hands them over: through its input hook, a frame at a time
/// for as long as it holds some (`crate::input_order`).
fn through_the_window(h: &mut Window, events: impl IntoIterator<Item = Event>) {
    h.input_mut().events.extend(events);
    for _ in 0..16 {
        let mut raw = egui::RawInput { events: std::mem::take(&mut h.input_mut().events), ..Default::default() };
        h.state_mut().raw_input_hook(&mut raw);
        h.input_mut().events = raw.events;
        h.step();
        if h.state().held_input.is_empty() {
            break;
        }
    }
    h.run_steps(2);
}

#[test]
fn the_body_takes_no_key_of_the_frame_before_the_click_that_gave_it_the_keyboard() {
    let mut h = writing("Plan", false);
    let line = end_of(&h, "Plan");
    one_frame(&mut h, click(line));
    assert!(!editor::has_focus(&h.ctx), "the keyboard is in the Subject");
    let at = body_at(&h);
    one_frame(&mut h, [Event::PointerMoved(at)]);
    one_frame(&mut h, std::iter::once(Event::Text(" A".into())).chain(click(at)));
    assert!(editor::has_focus(&h.ctx), "the click gave the body the keyboard");
    assert_eq!(plain(&h), "", "the Subject's keys are not the body's");
    type_in(&mut h, "Corps");
    assert_eq!(plain(&h), "Corps");
}

#[test]
fn the_body_keeps_the_keys_of_the_frame_before_the_click_that_took_the_keyboard() {
    let mut h = writing("Plan", true);
    assert!(editor::has_focus(&h.ctx), "the keyboard is in the body");
    let at = end_of(&h, "Plan");
    one_frame(&mut h, [Event::PointerMoved(at)]);
    one_frame(&mut h, std::iter::once(Event::Text("Corps".into())).chain(click(at)));
    assert!(!editor::has_focus(&h.ctx), "the click gave the Subject the keyboard");
    assert_eq!((subject(&h), plain(&h)), ("Plan".to_string(), "Corps".to_string()), "the body's keys are not lost");
}

#[test]
fn subject_then_body_each_takes_its_own() {
    let mut h = writing("Plan", false);
    let (line, body) = (end_of(&h, "Plan"), body_at(&h));
    one_frame(&mut h, click(line));
    through_the_window(&mut h, std::iter::once(Event::Text(" A".into())).chain(click(body)).chain([Event::Text("Corps".into())]));
    assert_eq!((subject(&h), plain(&h)), ("Plan A".to_string(), "Corps".to_string()));
}

#[test]
fn body_then_subject_each_takes_its_own() {
    let mut h = writing("Plan", true);
    let line = end_of(&h, "Plan");
    through_the_window(&mut h, std::iter::once(Event::Text("Corps".into())).chain(click(line)).chain([Event::Text(" B".into())]));
    assert_eq!((subject(&h), plain(&h)), ("Plan B".to_string(), "Corps".to_string()));
}

#[test]
fn to_then_body_then_subject_each_takes_its_own() {
    let mut h = writing("Plan", false);
    let (to_line, subject_line, body) = (end_of(&h, "maya@lumen.example"), end_of(&h, "Plan"), body_at(&h));
    one_frame(&mut h, click(to_line));
    let came = [Event::Text("; scan@example.org".into())]
        .into_iter()
        .chain(click(body))
        .chain([Event::Text("Message d'essai.".into())])
        .chain(click(subject_line))
        .chain([Event::Text(" envoi".into())]);
    through_the_window(&mut h, came);
    assert!(to(&h).contains("scan@example.org"), "To: {}", to(&h));
    assert_eq!((subject(&h), plain(&h)), ("Plan envoi".to_string(), "Message d'essai.".to_string()));
}
