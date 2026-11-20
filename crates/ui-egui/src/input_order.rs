// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a click and what is typed never share a frame.

//! egui hands a frame all the input that came since the last one, and a widget reads the keys of a frame when it has the
//! keyboard. When a frame holds a click that moves the keyboard and keys typed around it, every field reads them as the
//! frame finds it: one that had the keyboard at the start of the frame takes all of them, one that took it by that click
//! took them too (the message's body read the frame's keys after its click), and one that lost it took none. A browser
//! makes such frames whenever it draws late: a page in the background, a busy frame, a test that drives the page. What was
//! typed in the Subject then came into the body (« Essai ACorps »), and a body's text typed before a click in the Subject
//! was lost (neoffice-maintenance, 20.11.2026).
//!
//! So a click and what is typed never share a frame ([`split`]): what was typed before a click has a frame of its own, read
//! by the field that has the keyboard; the click has its own, which moves the keyboard; what was typed after it waits for
//! the next frame, read by the field the click gave the keyboard to, as when the browser keeps up. Nothing is dropped or
//! reordered; a frame with no click, or with a click and nothing typed, is as it came. (The message's body also reads, of
//! a frame that holds both, only the keys that are its own: `compose::editor`.)

use egui::{Event, RawInput};

/// Whether `event` is something typed: a key, text, what an input method sends, the clipboard's.
fn typed(event: &Event) -> bool {
    matches!(event, Event::Text(_) | Event::Key { .. } | Event::Ime(_) | Event::Paste(_) | Event::Copy | Event::Cut | Event::PasteImage(_))
}

/// Where the first click of `events` ends: after its button is let go, else (still held down) after it went down.
fn first_click_end(events: &[Event]) -> Option<usize> {
    let down = events.iter().position(|e| matches!(e, Event::PointerButton { pressed: true, .. }))?;
    let up = events.iter().skip(down + 1).position(|e| matches!(e, Event::PointerButton { pressed: false, .. }));
    Some(up.map_or(down + 1, |n| down + 1 + n + 1))
}

/// The input of a frame: what `held` kept from the last one first, then what came. When it holds a click and something
/// typed, only its events up to the first click (when something was typed before it), or up to the end of that click (its
/// button let go: when something typed comes after it), and the rest is kept in `held` for the next frame. Whether some is
/// held.
pub fn split(held: &mut Vec<Event>, raw: &mut RawInput) -> bool {
    if !held.is_empty() {
        let came = std::mem::take(&mut raw.events);
        raw.events = std::mem::take(held);
        raw.events.extend(came);
    }
    let Some(down) = raw.events.iter().position(|e| matches!(e, Event::PointerButton { pressed: true, .. })) else { return false };
    let cut = if raw.events[..down].iter().any(typed) {
        down
    } else {
        let end = first_click_end(&raw.events).unwrap_or(down + 1);
        if !raw.events[end..].iter().any(typed) {
            return false;
        }
        end
    };
    *held = raw.events.split_off(cut);
    true
}

#[cfg(test)]
#[path = "input_order_tests.rs"]
mod tests;
