// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! `ContactCard/set` as a server that keeps only the basics of a contact takes it, as strictly as it does.
//!
//! - **One call** makes, changes and deletes at most as many cards as the session says (`requestTooLarge`).
//!   `ifInState` is refused. Creates go first, then changes, then deletions, each answered on its own: one refused
//!   leaves the others as they go.
//! - **What is written** is a whole property, never a part of one (a key with a `/` is `invalidProperties`, "A whole
//!   property replaces it: no patch of a part."), of the properties it writes ([`WRITABLE_CARD`]); what it sets or
//!   keeps for itself (`@type`, `version`, `addressBookIds`, `uid`, `kind`) may be sent and is left as it is; anything
//!   else (`notes`, `addresses`…) is `invalidProperties` ("Only name, emails, phones, organizations and titles can be
//!   written."). A card is in one book: `addressBookIds` names one the account has, or is left out (its default book).
//! - **A name** is its parts (`given`, `surname`); a name with no parts is read from its full form, cut at the first
//!   space; a name emptied by a change is not written (a contact keeps one), nor one whose full form is the contact's
//!   own. **Addresses** are those with an `@`, each once; the one the card prefers (`pref` 1) is the primary, else the
//!   one that was stays while it is there. **Numbers** keep what they were, and only the primary mobile number says it
//!   is one; the **company** and the **job title** are the first organization and the first title that have a name.
//!   A text longer than [`MAX_FIELD`] characters is refused (`invalidProperties`), never cut.
//! - **A new card** needs a name, an address or a company (one with no name is named from its address:
//!   `jeanne.muster@…` is Jeanne Muster; else by its company); it is called by its contact's name, which is its id
//!   (`Gus Hale`, then `Gus Hale-1`), and known again by the `uid` it was made with: made again, it is that card, with
//!   what was sent this time written to it.
//! - **Every card says** when it was made and last changed (`created`, `updated`): a card made under the name of one
//!   that was deleted is told apart from it so.
//! - **Rights** are the book's: making or changing a card in a book the account may only read is `forbidden` ("Your
//!   rights do not allow it."), and so is deleting one where it may not delete; a card it doesn't have is `notFound`.
//!   A card a document names is not deleted: `forbidden`, with the words of the store ([`Pim::linked_cards`]).
//! - **Trouble on demand**: a card (by its id, or a new one by its `uid`) answered with the SetError a test names
//!   ([`Pim::refused_cards`]). What it was sent is kept, in order ([`Pim::card_writes`]).

use serde_json::{Map, Value, json};

use super::super::{ACCOUNT, Outcome, account, allow_only, invalid, refuse};
use super::{Card, Pim, WRITABLE_CARD};

/// What such a server sets or keeps for itself: a client may send it.
const IGNORED: [&str; 5] = ["@type", "version", "addressBookIds", "uid", "kind"];
/// The longest text a field takes, in characters (a full name with no parts: twice that and a space), and the
/// longest words of a refusal.
const MAX_FIELD: usize = 140;
const MAX_FULL: usize = 281;
const MAX_WORDS: usize = 200;
/// The longest `uid` kept, and most changes kept as they came.
const MAX_UID: usize = 255;
const MAX_WRITTEN: usize = 10_000;

/// A SetError.
fn set_error(kind: &str, description: impl Into<String>) -> Value {
    json!({"type": kind, "description": description.into()})
}

fn forbidden() -> Value {
    set_error("forbidden", "Your rights do not allow it.")
}

fn not_found() -> Value {
    json!({"type": "notFound"})
}

fn invalid_properties(description: &str) -> Value {
    set_error("invalidProperties", description)
}

/// A value as such a server reads text: a string as it is, a number or a truth as it is written, nothing as empty;
/// trimmed. Longer than `max` characters: refused.
fn text(value: Option<&Value>, max: usize) -> Result<String, Value> {
    let raw = match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    };
    let text = raw.trim();
    if text.chars().count() > max {
        return Err(invalid_properties(&format!("A text is longer than {max} characters.")));
    }
    Ok(text.to_string())
}

/// The entries of a map property of a card (`emails`, `phones`…): its values that are objects; `null` is none.
fn entries<'a>(card: &'a Map<String, Value>, key: &str) -> Result<Vec<&'a Map<String, Value>>, Value> {
    match card.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => Ok(map.values().filter_map(Value::as_object).collect()),
        Some(_) => Err(invalid_properties(&format!("{key} must be an object."))),
    }
}

/// What a card (or a change of whole properties) gives, for the properties it names.
#[derive(Default)]
struct Fields {
    /// The first and last name, when it names a name; and the full form it was read from, when it had no parts.
    name: Option<(String, String)>,
    full: Option<String>,
    emails: Option<Vec<String>>,
    /// The address the card prefers (`pref` 1), when one is.
    preferred: Option<String>,
    phones: Option<Vec<(String, bool)>>,
    company: Option<String>,
    title: Option<String>,
}

/// Read a card or a change as such a server does; `book_ok` says whether the book it names is one it has.
fn fields(card: &Map<String, Value>, books: &[String]) -> Result<Fields, Value> {
    if card.keys().any(|key| !WRITABLE_CARD.contains(&key.as_str()) && !IGNORED.contains(&key.as_str())) {
        return Err(invalid_properties("Only name, emails, phones, organizations and titles can be written."));
    }
    if let Some(named) = card.get("addressBookIds") {
        let one = named.as_object().filter(|map| map.len() == 1).and_then(|map| map.iter().next());
        if !one.is_some_and(|(book, on)| on == &Value::Bool(true) && books.contains(book)) {
            return Err(invalid_properties("A card goes in one address book of the account."));
        }
    }
    let mut out = Fields::default();
    if let Some(name) = card.get("name") {
        let name = match name {
            Value::Null => Map::new(),
            Value::Object(name) => name.clone(),
            _ => return Err(invalid_properties("name must be an object.")),
        };
        let components = match name.get("components") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(list)) => list.clone(),
            Some(_) => return Err(invalid_properties("name.components must be a list.")),
        };
        let part = |kind: &str| -> Result<String, Value> {
            let named = components.iter().filter_map(Value::as_object).filter(|part| part.get("kind").and_then(Value::as_str) == Some(kind));
            Ok(named.map(|part| text(part.get("value"), MAX_FIELD)).collect::<Result<Vec<String>, Value>>()?.pop().unwrap_or_default())
        };
        let (mut first, mut last) = (part("given")?, part("surname")?);
        if first.is_empty() && last.is_empty() {
            // Only the full name (a client that keeps no parts of it).
            let full = text(name.get("full"), MAX_FULL)?;
            let (a, b) = full.split_once(' ').unwrap_or((full.as_str(), ""));
            if a.chars().count() > MAX_FIELD || b.trim().chars().count() > MAX_FIELD {
                return Err(invalid_properties(&format!("A text is longer than {MAX_FIELD} characters.")));
            }
            (first, last) = (a.to_string(), b.trim().to_string());
            out.full = Some(full);
        }
        out.name = Some((first, last));
    }
    if card.contains_key("emails") {
        let mut seen: Vec<String> = Vec::new();
        let mut addresses = Vec::new();
        for entry in entries(card, "emails")? {
            let address = text(entry.get("address"), MAX_FIELD)?;
            if address.contains('@') && !seen.contains(&address.to_lowercase()) {
                seen.push(address.to_lowercase());
                if entry.get("pref").and_then(Value::as_u64) == Some(1) && out.preferred.is_none() {
                    out.preferred = Some(address.clone());
                }
                addresses.push(address);
            }
        }
        out.emails = Some(addresses);
    }
    if card.contains_key("phones") {
        let mut numbers = Vec::new();
        for entry in entries(card, "phones")? {
            let mobile = entry.get("features").and_then(|features| features.get("mobile")).is_some_and(|on| on.as_bool() == Some(true));
            let number = text(entry.get("number"), MAX_FIELD)?;
            if !number.is_empty() {
                numbers.push((number, mobile));
            }
        }
        out.phones = Some(numbers);
    }
    let first_name = |key: &str| -> Result<String, Value> {
        let names = entries(card, key)?.into_iter().map(|entry| text(entry.get("name"), MAX_FIELD)).collect::<Result<Vec<String>, Value>>()?;
        Ok(names.into_iter().find(|name| !name.is_empty()).unwrap_or_default())
    };
    if card.contains_key("organizations") {
        out.company = Some(first_name("organizations")?);
    }
    if card.contains_key("titles") {
        out.title = Some(first_name("titles")?);
    }
    Ok(out)
}

/// Numbers as such a server keeps them: one it had keeps what it was; the primary mobile number of a kind that went goes
/// to the first new mobile number; one primary mobile at most.
fn numbers(new: Vec<(String, bool)>, old: &[(String, bool)], fresh: bool) -> Vec<(String, bool)> {
    let mut rows: Vec<(String, bool)> =
        new.iter().map(|(number, _)| (number.clone(), old.iter().any(|(had, primary)| had == number && *primary))).collect();
    let mut flagged = false;
    for row in &mut rows {
        row.1 = row.1 && !flagged;
        flagged |= row.1;
    }
    if !flagged {
        let first_new = new.iter().position(|(number, mobile)| *mobile && (fresh || !old.iter().any(|(had, _)| had == number)));
        if let Some(row) = first_new.and_then(|at| rows.get_mut(at)) {
            row.1 = true;
        }
    }
    rows
}

/// Addresses as such a server keeps them: the one the card prefers first; without one, the one that was the primary
/// (`old`) stays it while it is there.
fn addresses(mut new: Vec<String>, preferred: Option<String>, old: Option<&String>) -> Vec<String> {
    let primary = preferred.or_else(|| old.cloned());
    if let Some(primary) = primary
        && let Some(at) = new.iter().position(|address| *address == primary)
    {
        let first = new.remove(at);
        new.insert(0, first);
    }
    new
}

impl Pim {
    /// The books a card may be in, by their ids.
    fn book_ids(&self) -> Vec<String> {
        self.books.iter().map(|book| book.id.clone()).collect()
    }

    /// The default book: the one flagged, else the first.
    fn default_book(&self) -> Option<&super::Book> {
        self.books.iter().find(|book| book.default).or_else(|| self.books.first())
    }

    /// Keep what was sent, in order.
    fn wrote(&mut self, id: &str, what: Value) {
        if self.card_writes.len() < MAX_WRITTEN {
            self.card_writes.push((id.to_string(), what));
        }
    }

    /// `ContactCard/set`, with at most `most` objects in a call.
    pub(in crate::standin) fn card_set(&mut self, args: &Map<String, Value>, most: usize) -> Outcome<Value> {
        allow_only(args, &["accountId", "ifInState", "create", "update", "destroy"])?;
        account(args)?;
        let object = |key: &str| match args.get(key) {
            None | Some(Value::Null) => Ok(Map::new()),
            Some(Value::Object(map)) => Ok(map.clone()),
            Some(_) => Err(invalid("create, update, destroy")),
        };
        let (creates, updates) = (object("create")?, object("update")?);
        let destroys: Vec<Value> = match args.get("destroy") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(ids)) => ids.clone(),
            Some(_) => return Err(invalid("create, update, destroy")),
        };
        if creates.len().saturating_add(updates.len()).saturating_add(destroys.len()) > most {
            return Err(refuse("requestTooLarge", "maxObjectsInSet"));
        }
        if !matches!(args.get("ifInState"), None | Some(Value::Null)) {
            return Err(invalid("ifInState"));
        }
        let old_state = self.cards_state();
        let (mut created, mut not_created) = (Map::new(), Map::new());
        for (key, card) in &creates {
            match self.create_card(card) {
                Ok(id) => {
                    created.insert(key.clone(), json!({"id": id, "uid": id}));
                }
                Err(error) => {
                    not_created.insert(key.clone(), error);
                }
            }
        }
        let (mut updated, mut not_updated) = (Map::new(), Map::new());
        for (id, patch) in &updates {
            match self.update_card(id, patch) {
                Ok(()) => {
                    updated.insert(id.clone(), Value::Null);
                }
                Err(error) => {
                    not_updated.insert(id.clone(), error);
                }
            }
        }
        let (mut destroyed, mut not_destroyed) = (Vec::new(), Map::new());
        for id in destroys.iter().filter_map(Value::as_str) {
            match self.destroy_card(id) {
                Ok(()) => destroyed.push(json!(id)),
                Err(error) => {
                    not_destroyed.insert(id.to_string(), error);
                }
            }
        }
        Ok(json!({
            "accountId": ACCOUNT,
            "oldState": old_state,
            "newState": self.cards_state(),
            "created": created,
            "updated": updated,
            "destroyed": destroyed,
            "notCreated": not_created,
            "notUpdated": not_updated,
            "notDestroyed": not_destroyed,
        }))
    }

    /// A free id for a card called `name`: the name, else the name and a number.
    fn free_id(&self, name: &str) -> String {
        let taken = |id: &str| self.cards.iter().any(|card| card.id == id);
        if !taken(name) {
            return name.to_string();
        }
        (1u32..).map(|n| format!("{name}-{n}")).find(|id| !taken(id)).unwrap_or_else(|| format!("{name}-x"))
    }

    fn create_card(&mut self, card: &Value) -> Result<String, Value> {
        // The account must be allowed to add cards at all (its default book), before anything of the card is read.
        if !self.default_book().is_some_and(|book| book.may_write) {
            return Err(forbidden());
        }
        let Some(card) = card.as_object() else {
            return Err(invalid_properties("Only name, emails, phones, organizations and titles can be written."));
        };
        let fields = fields(card, &self.book_ids())?;
        // Its book: the one named (one the account has), else the default one; and to that one too.
        let named = card.get("addressBookIds").and_then(Value::as_object).and_then(|map| map.keys().next().cloned());
        let book = named.or_else(|| self.default_book().map(|book| book.id.clone())).unwrap_or_default();
        if !self.books.iter().any(|b| b.id == book && b.may_write) {
            return Err(forbidden());
        }
        let uid = card.get("uid").and_then(Value::as_str).map(|uid| uid.chars().take(MAX_UID).collect::<String>()).unwrap_or_default();
        if let Some(error) = self.refused_cards.get(&uid).filter(|_| !uid.is_empty()) {
            // (A stricter server says which card is there already.)
            let mut refusal = set_error(error, "refused on demand");
            if let Some(existing) = self.made_cards.get(&uid).filter(|_| *error == "alreadyExists") {
                refusal["existingId"] = json!(existing);
            }
            return Err(refusal);
        }
        // A card sent again after its answer was lost: the one made the first time, with what was sent this time.
        if let Some(made) = self.made_cards.get(&uid).filter(|id| !uid.is_empty() && self.cards.iter().any(|card| &card.id == *id)).cloned() {
            if let Some(at) = self.cards.iter().position(|card| card.id == made) {
                self.write_fields(at, fields);
            }
            self.wrote(&made, Value::Object(card.clone()));
            return Ok(made);
        }
        let company = fields.company.clone().unwrap_or_default();
        let addresses = addresses(fields.emails.clone().unwrap_or_default(), fields.preferred.clone(), None);
        let (mut first, mut last) = fields.name.clone().unwrap_or_default();
        let mut full = String::new();
        if first.is_empty() && last.is_empty() {
            // Named from its address when no name is given, else by its company.
            match addresses.first() {
                Some(address) => {
                    let local = address.split('@').next().unwrap_or_default();
                    let words: Vec<String> = local
                        .split(['.', '_', '-'])
                        .filter(|word| !word.is_empty())
                        .map(|word| {
                            let mut chars = word.chars();
                            chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
                        })
                        .collect();
                    first = if words.is_empty() { address.clone() } else { words.join(" ").chars().take(MAX_FIELD).collect() };
                    last = String::new();
                }
                None if !company.is_empty() => full.clone_from(&company),
                None => return Err(invalid_properties("A contact needs a name, an address or a company.")),
            }
        }
        let name = if full.is_empty() {
            [first.as_str(), last.as_str()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<&str>>().join(" ")
        } else {
            full.clone()
        };
        let id = self.free_id(&name);
        let phones = numbers(fields.phones.unwrap_or_default(), &[], true);
        let now = self.tick();
        self.cards.push(Card {
            id: id.clone(),
            books: vec![book],
            first,
            last,
            full,
            company,
            title: fields.title.unwrap_or_default(),
            emails: addresses,
            phones,
            created: now,
            updated: now,
        });
        if !uid.is_empty() {
            self.made_cards.insert(uid, id.clone());
        }
        self.wrote(&id, Value::Object(card.clone()));
        Ok(id)
    }

    fn update_card(&mut self, id: &str, patch: &Value) -> Result<(), Value> {
        let Some(at) = self.cards.iter().position(|card| card.id == id) else { return Err(not_found()) };
        if let Some(error) = self.refused_cards.get(id) {
            return Err(set_error(error, "refused on demand"));
        }
        if let Some(patch) = patch.as_object()
            && patch.keys().any(|key| key.contains('/'))
        {
            return Err(invalid_properties("A whole property replaces it: no patch of a part."));
        }
        let Some(patch) = patch.as_object() else {
            return Err(invalid_properties("Only name, emails, phones, organizations and titles can be written."));
        };
        let fields = fields(patch, &self.book_ids())?;
        let card = self.cards.get(at).ok_or_else(not_found)?;
        if !card.books.iter().any(|book| self.books.iter().any(|b| &b.id == book && b.may_write)) {
            return Err(forbidden());
        }
        self.write_fields(at, fields);
        self.wrote(id, Value::Object(patch.clone()));
        Ok(())
    }

    /// Write what a change (or a card sent again) names to the card at `at`, as such a server writes it.
    fn write_fields(&mut self, at: usize, fields: Fields) {
        let now = self.tick();
        let Some(card) = self.cards.get_mut(at) else { return };
        if let Some((first, last)) = fields.name {
            // A name emptied is not written; nor is the full name as the contact has it.
            let own = fields.full.as_ref().is_some_and(|full| *full == card.full_name());
            if !(first.is_empty() && last.is_empty()) && !own {
                (card.first, card.last, card.full) = (first, last, String::new());
            }
        }
        if let Some(company) = fields.company {
            card.company = company;
        }
        if let Some(title) = fields.title {
            card.title = title;
        }
        if let Some(new) = fields.emails {
            card.emails = addresses(new, fields.preferred, card.emails.first());
        }
        if let Some(phones) = fields.phones {
            card.phones = numbers(phones, &card.phones, false);
        }
        card.updated = now;
    }

    fn destroy_card(&mut self, id: &str) -> Result<(), Value> {
        let Some(at) = self.cards.iter().position(|card| card.id == id) else { return Err(not_found()) };
        if let Some(error) = self.refused_cards.get(id) {
            return Err(set_error(error, "refused on demand"));
        }
        let card = self.cards.get(at).cloned().ok_or_else(not_found)?;
        if !card.books.iter().any(|book| self.books.iter().any(|b| &b.id == book && b.may_write && b.may_remove)) {
            return Err(forbidden());
        }
        if let Some(document) = self.linked_cards.get(id) {
            let words = format!("Cannot delete or cancel because Contact {id} is linked with {document}");
            return Err(set_error("forbidden", words.chars().take(MAX_WORDS).collect::<String>()));
        }
        self.cards.remove(at);
        self.wrote(id, json!("destroy"));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
