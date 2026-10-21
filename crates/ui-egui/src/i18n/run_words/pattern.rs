// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A sentence template (`Couldn't send '{subject}': {why}. It is held in the Outbox.`) split into the literal parts and
//! the slots it is made of, and the ways a line can be read as it: which text each slot holds. Pure, with no table and
//! no language: [`super`] says what a slot may hold and which way of reading a line is the right one.

use std::cell::Cell;

/// The most places a slot that a literal follows may end at: the first ones where that literal is found.
const MAX_ENDS: usize = 16;
/// The most ways of reading one line as one template.
const MAX_SPLITS: usize = 64;

/// A piece of a template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    /// Text that the line has as it is.
    Literal(&'static str),
    /// A `{name}` placeholder: text the line has in its place.
    Slot(&'static str),
}

/// A template, as its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pattern {
    parts: Vec<Part>,
}

/// How much work a line may cost before it is left as it came: each way of reading tried takes one.
#[derive(Debug)]
pub(super) struct Budget {
    left: Cell<usize>,
    spent: Cell<bool>,
}

impl Budget {
    pub(super) fn new(steps: usize) -> Budget {
        Budget { left: Cell::new(steps), spent: Cell::new(false) }
    }

    /// One step; false (for good) once there are none left.
    pub(super) fn take(&self) -> bool {
        match self.left.get().checked_sub(1) {
            Some(left) => {
                self.left.set(left);
                true
            }
            None => {
                self.spent.set(true);
                false
            }
        }
    }

    /// Whether a step was refused.
    pub(super) fn spent(&self) -> bool {
        self.spent.get()
    }
}

impl Pattern {
    /// The parts of `template`: a `{name}` whose name is letters, digits and `_` is a slot (as the catalogs read a
    /// placeholder, `census::placeholders`), any other brace is text. Two slots side by side can't be told apart in a
    /// line, and a template with no text can't be told from anything: both are refused.
    pub(super) fn parse(template: &'static str) -> Result<Pattern, String> {
        let mut parts = Vec::new();
        let mut literal_start = 0;
        let mut at = 0;
        while let Some(open) = template.get(at..).and_then(|rest| rest.find('{')).map(|offset| at + offset) {
            let after = open + 1;
            let name = template.get(after..).and_then(|rest| rest.split_once('}')).map(|(name, _)| name);
            match name {
                Some(name) if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                    if let Some(text) = template.get(literal_start..open).filter(|text| !text.is_empty()) {
                        parts.push(Part::Literal(text));
                    }
                    if matches!(parts.last(), Some(Part::Slot(_))) {
                        return Err(format!("{template:?}: two placeholders side by side can't be told apart"));
                    }
                    parts.push(Part::Slot(name));
                    literal_start = after + name.len() + 1;
                    at = literal_start;
                }
                _ => at = after,
            }
        }
        if let Some(text) = template.get(literal_start..).filter(|text| !text.is_empty()) {
            parts.push(Part::Literal(text));
        }
        if !parts.iter().any(|part| matches!(part, Part::Literal(_))) {
            return Err(format!("{template:?}: a template with no text of its own would read any line"));
        }
        Ok(Pattern { parts })
    }

    #[cfg(test)]
    pub(super) fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// The names of its slots, in order.
    pub(super) fn slots(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.parts.iter().filter_map(|part| match part {
            Part::Slot(name) => Some(*name),
            Part::Literal(_) => None,
        })
    }

    /// How many characters of text of its own it has: the more, the more a line must have to be read as it.
    pub(super) fn literal_len(&self) -> usize {
        self.parts
            .iter()
            .map(|part| match part {
                Part::Literal(text) => text.chars().count(),
                Part::Slot(_) => 0,
            })
            .sum()
    }

    /// Every way `text` reads as this template, the earliest first: for each, the text of each slot, in order. A slot
    /// holds what `fits` lets it (by its name); the last slot takes what is left before the last literal, and any other
    /// ends at one of the first places where the literal after it is found. None once `budget` is spent.
    pub(super) fn splits<'t>(&self, text: &'t str, fits: &dyn Fn(&str, &str) -> bool, budget: &Budget) -> Vec<Vec<&'t str>> {
        let mut out = Vec::new();
        let mut held = Vec::new();
        split(&self.parts, text, fits, budget, &mut held, &mut out);
        out
    }
}

/// The ways `text` reads as `parts`, appended to `out`, with what the slots before took in `held`.
fn split<'t>(
    parts: &[Part],
    text: &'t str,
    fits: &dyn Fn(&str, &str) -> bool,
    budget: &Budget,
    held: &mut Vec<&'t str>,
    out: &mut Vec<Vec<&'t str>>,
) {
    if out.len() >= MAX_SPLITS || !budget.take() {
        return;
    }
    match parts {
        [] => {
            if text.is_empty() {
                out.push(held.clone());
            }
        }
        [Part::Literal(literal), rest @ ..] => {
            if let Some(after) = text.strip_prefix(literal) {
                split(rest, after, fits, budget, held, out);
            }
        }
        [Part::Slot(name)] => {
            if fits(name, text) {
                held.push(text);
                out.push(held.clone());
                held.pop();
            }
        }
        // The last literal ends the line: the slot before it holds all the rest.
        [Part::Slot(name), Part::Literal(last)] => {
            if let Some(value) = text.strip_suffix(last)
                && fits(name, value)
            {
                held.push(value);
                out.push(held.clone());
                held.pop();
            }
        }
        [Part::Slot(name), rest @ ..] => {
            let Some(Part::Literal(next)) = rest.first() else { return };
            for (end, _) in text.match_indices(next).filter(|(end, _)| *end > 0).take(MAX_ENDS) {
                let (Some(value), Some(after)) = (text.get(..end), text.get(end..)) else { continue };
                if !fits(name, value) {
                    continue;
                }
                held.push(value);
                split(rest, after, fits, budget, held, out);
                held.pop();
                if out.len() >= MAX_SPLITS || budget.spent() {
                    return;
                }
            }
        }
    }
}
