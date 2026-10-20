// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! An address book and a card as JMAP for Contacts serves them (RFC 9610; cards as JSContact, RFC 9553), into what the
//! engine takes: [`RemoteBook`] and [`RemoteContact`].
//!
//! - **Names.** The first and last name are the card's name components of the kinds `given` and `surname` (several of
//!   a kind joined by a space); the name the lists show is the card's `full` name when it is not just those two (a
//!   company's card reads as its company, a name with a middle name as it is written).
//! - **Addresses and numbers**, in the order the server prefers them (`pref`, the lowest first, then those without
//!   one in the order of their names: `e2` before `e10`). An address is labelled `work` or `home` (JSContact's
//!   `private`) by its `contexts`; a number `mobile` when its `features` say so, else by its contexts, else not.
//! - **The company and the job title** are the first organization and the first title that have a name.
//! - **Its book** is the first of the books it is in (`addressBookIds`, in the order of their ids) that the account
//!   may write, else the first.
//! - **Its name for the engine** is its id, or, for a server that names its cards outside the JMAP alphabet (by their
//!   contacts' names), a key made of that name ([`super::card_key`]): the engine keeps no other kind. Its `uid`, when
//!   it is not its id, is the mark it was made with (a card made here carries the engine's): by it the engine knows
//!   again a card whose answer was lost.
//!
//! **Written** ([`card_json`]): a new card whole, in its book and with its mark as its `uid`; a change as the whole
//! properties it names, the name always with its parts and its full form (a server that keeps only the basics reads a
//! name with no parts as the contact's own, and keeps the first and last it has). An address is said `work` or
//! `private` by its label and the first is preferred (`pref` 1); a number is a mobile, a fax or a pager by its label,
//! else a voice one.
//!
//! What a server that keeps only the basics doesn't serve (middle names, nicknames, departments, postal addresses,
//! web sites, birthdays, notes) is not read. A card that is no person (a group's) or has nothing to show (no name,
//! company or address) is passed over, as the engine would pass it over. Everything is hostile: a value of the wrong
//! type is no value, lists are looked at up to a bound, and the contact is cut to size afterwards
//! ([`RemoteContact::tidy`]).

use std::collections::HashSet;

use mailcraft_model::Labeled;
use mailcraft_model::book::{ContactChangeKind, ContactName, ContactOp, RemoteBook};
use mailcraft_model::remote::RemoteContact;
use serde_json::{Map, Value, json};

use super::{card_id, card_key};
use crate::valid_id;

/// Most entries of one card's map that are looked at (addresses, numbers, organizations, books), and most name
/// components.
const MAX_ENTRIES: usize = 100;
/// The longest text taken of a name, an address or a number, in characters (what the model keeps, and a little more
/// for the tidying to cut); and the longest time a card was made that is kept (a longer one is not kept at all: a
/// part of it would tell nothing apart).
const MAX_LINE: usize = 1_000;
const MAX_MARK: usize = 64;

/// What a card of a server's answer was passed over for lacking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// An id that is one.
    Id,
    /// A person to show: it is a group's card, or has no name, company or address.
    Person,
}

/// An address book as it was read: the engine's, and what the account may do with the cards in it there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub book: RemoteBook,
    /// The account may write its cards (`myRights.mayWrite`).
    pub writable: bool,
    /// And delete them: the server does not narrow that (`myRights.mayRemoveItems` is not false).
    pub removes: bool,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Text from outside, no longer than `max` characters.
fn bounded(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// An address book of `AddressBook/get`, as the engine takes it (its rights as the server says them); `None` without an
/// id that is one.
pub fn book_of(value: &Value) -> Option<Listed> {
    let id = text(value, "id");
    if !valid_id(id) {
        return None;
    }
    let right = |key: &str| value.get("myRights").and_then(|rights| rights.get(key)).and_then(Value::as_bool);
    let writable = right("mayWrite") == Some(true);
    let book = RemoteBook {
        id: id.to_string(),
        name: bounded(text(value, "name"), MAX_LINE),
        main: value.get("isDefault").and_then(Value::as_bool) == Some(true),
        ..Default::default()
    };
    Some(Listed { book, writable, removes: writable && right("mayRemoveItems") != Some(false) })
}

/// The order of a map's entries by their names: `e2` before `e10`, as people number them.
fn natural(name: &str) -> (&str, u64, &str) {
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let number = name.get(stem.len()..).and_then(|digits| digits.parse::<u64>().ok()).unwrap_or(u64::MAX);
    (stem, number, name)
}

/// The entries of a JSContact map (`emails`, `phones`, `organizations`, `titles`) that are objects, at most `most` of
/// them, in the order the server prefers them: by `pref` (1 is the most preferred; none comes after every one), then
/// by their names.
fn entries<'a>(value: &'a Value, key: &str, most: usize) -> Vec<&'a Map<String, Value>> {
    let mut listed: Vec<(&String, &Map<String, Value>)> = value
        .get(key)
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| map.iter())
        .filter_map(|(name, entry)| Some((name, entry.as_object()?)))
        .take(most)
        .collect();
    let pref = |entry: &Map<String, Value>| entry.get("pref").and_then(Value::as_u64).filter(|n| (1..=100).contains(n)).unwrap_or(u64::MAX);
    listed.sort_by(|a, b| (pref(a.1), natural(a.0)).cmp(&(pref(b.1), natural(b.0))));
    listed.into_iter().map(|(_, entry)| entry).collect()
}

/// Whether a JSContact `contexts` (or `features`) object says `key`.
fn says(entry: &Map<String, Value>, set: &str, key: &str) -> bool {
    entry.get(set).and_then(|set| set.get(key)).and_then(Value::as_bool) == Some(true)
}

/// The label of an address or a number by its `contexts`: `work`, `home` for `private`, else none.
fn context(entry: &Map<String, Value>) -> &'static str {
    if says(entry, "contexts", "work") {
        "work"
    } else if says(entry, "contexts", "private") {
        "home"
    } else {
        ""
    }
}

/// The first entry of a map that has a name, by the server's order.
fn first_named(value: &Value, key: &str) -> String {
    entries(value, key, MAX_ENTRIES)
        .into_iter()
        .map(|entry| entry.get("name").and_then(Value::as_str).unwrap_or_default().trim())
        .find(|name| !name.is_empty())
        .map(|name| bounded(name, MAX_LINE))
        .unwrap_or_default()
}

/// A card of `ContactCard/get`, as the engine takes it (see the module), in the first of its books that `writable` has,
/// else the first of them.
pub fn card_of(value: &Value, writable: &HashSet<String>) -> Result<RemoteContact, Missing> {
    let id = text(value, "id");
    if !card_id(id) {
        return Err(Missing::Id);
    }
    if text(value, "kind") == "group" {
        return Err(Missing::Person);
    }
    let name = value.get("name");
    let components = name.and_then(|name| name.get("components")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let part = |kind: &str| {
        let words: Vec<String> = components
            .iter()
            .take(MAX_ENTRIES)
            .filter(|component| text(component, "kind") == kind)
            .map(|component| text(component, "value").trim())
            .filter(|word| !word.is_empty())
            .map(|word| bounded(word, MAX_LINE))
            .collect();
        words.join(" ")
    };
    let (first, last) = (part("given"), part("surname"));
    let full = bounded(name.map(|name| text(name, "full")).unwrap_or_default().trim(), MAX_LINE);
    let parts = [first.as_str(), last.as_str()].into_iter().filter(|p| !p.is_empty()).collect::<Vec<&str>>().join(" ");
    let display_name = if full == parts { String::new() } else { full };
    let emails: Vec<Labeled> = entries(value, "emails", MAX_ENTRIES).into_iter().map(email_of).collect();
    let phones: Vec<Labeled> = entries(value, "phones", MAX_ENTRIES).into_iter().map(phone_of).collect();
    // The books it is in, in the order of their ids; the first the account may write is its own.
    let books: Vec<&str> = value
        .get("addressBookIds")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| map.iter())
        .filter(|(_, member)| member.as_bool() == Some(true))
        .map(|(book, _)| book.as_str())
        .filter(|book| valid_id(book))
        .take(MAX_ENTRIES)
        .collect();
    let book = books.iter().find(|book| writable.contains(**book)).or_else(|| books.first()).map_or_else(String::new, |book| book.to_string());
    // The mark it was made with, when it is not its id (a server that names a card by its uid has said nothing more).
    let uid = text(value, "uid").trim();
    let uid = if uid == id || !valid_id(uid) { String::new() } else { uid.to_string() };
    // When the server made it: what tells the contact behind a name from another a server gave the same name later.
    // Whole, or nothing.
    let created = text(value, "created").trim();
    let created = if created.chars().count() <= MAX_MARK { created.to_string() } else { String::new() };
    let contact = RemoteContact {
        id: card_key(id),
        change_key: created,
        book,
        uid,
        display_name,
        first,
        last,
        company: first_named(value, "organizations"),
        title: first_named(value, "titles"),
        emails,
        phones,
        ..Default::default()
    };
    let shown = [&contact.display_name, &contact.first, &contact.last, &contact.company].iter().any(|name| !name.is_empty())
        || contact.emails.iter().any(|email| email.value.contains('@'));
    if shown { Ok(contact) } else { Err(Missing::Person) }
}

/// An address of a card as the engine takes it: labelled by its contexts.
fn email_of(entry: &Map<String, Value>) -> Labeled {
    Labeled::new(context(entry), bounded(entry.get("address").and_then(Value::as_str).unwrap_or_default().trim(), MAX_LINE))
}

/// A number of a card as the engine takes it: a mobile one says so, another is labelled by its contexts.
fn phone_of(entry: &Map<String, Value>) -> Labeled {
    let label = if says(entry, "features", "mobile") { "mobile" } else { context(entry) };
    Labeled::new(label, bounded(entry.get("number").and_then(Value::as_str).unwrap_or_default().trim(), MAX_LINE))
}

/// The contexts of an address or a number, by its label: `work`, or `private` for `home`.
fn contexts(label: &str) -> Value {
    match label.trim().to_ascii_lowercase().as_str() {
        "work" => json!({"work": true}),
        "home" => json!({"private": true}),
        _ => json!({}),
    }
}

/// A name as a card holds it: its parts and its full form (the display name, else the first and last name).
fn name_json(name: &ContactName) -> Value {
    let parts: Vec<Value> = [("given", name.first.trim()), ("surname", name.last.trim())]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(kind, value)| json!({"@type": "NameComponent", "kind": kind, "value": value}))
        .collect();
    let joined = [name.first.trim(), name.last.trim()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<&str>>().join(" ");
    let full = if name.display.trim().is_empty() { joined } else { name.display.trim().to_string() };
    json!({"@type": "Name", "full": full, "components": parts})
}

/// An address as a card holds it.
fn email_entry(email: &Labeled) -> Map<String, Value> {
    let mut entry = Map::new();
    entry.insert("@type".into(), json!("EmailAddress"));
    entry.insert("address".into(), json!(email.value.trim()));
    entry.insert("contexts".into(), contexts(&email.label));
    entry
}

/// A number as a card holds it: what it is (a mobile, a fax, a pager; else a voice one), by its label.
fn phone_entry(phone: &Labeled) -> Map<String, Value> {
    let features = match phone.label.trim().to_ascii_lowercase().as_str() {
        "mobile" | "cell" => json!({"mobile": true}),
        "fax" => json!({"fax": true}),
        "pager" => json!({"pager": true}),
        _ => json!({"voice": true}),
    };
    let mut entry = Map::new();
    entry.insert("@type".into(), json!("Phone"));
    entry.insert("number".into(), json!(phone.value.trim()));
    entry.insert("features".into(), features);
    entry.insert("contexts".into(), contexts(&phone.label));
    entry
}

/// The addresses as a card holds them, each as it is (`kept`) or made from what it is here; the first preferred, and
/// only it.
fn emails_map(emails: Vec<(Labeled, Option<Map<String, Value>>)>) -> Value {
    let entries: Map<String, Value> = emails
        .into_iter()
        .filter(|(email, _)| email.value.contains('@'))
        .enumerate()
        .map(|(n, (email, kept))| {
            let mut entry = kept.unwrap_or_else(|| email_entry(&email));
            if n == 0 {
                entry.insert("pref".into(), json!(1));
            } else if entry.get("pref").and_then(Value::as_u64) == Some(1) {
                entry.remove("pref");
            }
            (format!("e{}", n + 1), Value::Object(entry))
        })
        .collect();
    Value::Object(entries)
}

/// The numbers as a card holds them, each as it is (`kept`) or made from what it is here.
fn phones_map(phones: Vec<(Labeled, Option<Map<String, Value>>)>) -> Value {
    let entries: Map<String, Value> = phones
        .into_iter()
        .filter(|(phone, _)| !phone.value.trim().is_empty())
        .enumerate()
        .map(|(n, (phone, kept))| (format!("p{}", n + 1), Value::Object(kept.unwrap_or_else(|| phone_entry(&phone)))))
        .collect();
    Value::Object(entries)
}

/// The addresses as a card holds them, the first preferred.
fn emails_json(emails: &[Labeled]) -> Value {
    emails_map(emails.iter().map(|email| (email.clone(), None)).collect())
}

/// The numbers as a card holds them.
fn phones_json(phones: &[Labeled]) -> Value {
    phones_map(phones.iter().map(|phone| (phone.clone(), None)).collect())
}

/// One named entry of `key` (`o1` of `organizations`, `t1` of `titles`), or none for an empty name.
fn one_named(letter: &str, name: &str, kind: &str) -> Value {
    let name = name.trim();
    if name.is_empty() {
        return json!({});
    }
    let mut entry = json!({"@type": kind, "name": name});
    if kind == "Title" {
        entry["kind"] = json!("title");
    }
    json!({ format!("{letter}1"): entry })
}

/// What a change of a contact (`op`) writes, as JSContact (RFC 9553): for a new card the card whole, in its book
/// (`op.book`, when it names one) and with the mark it was made with as its `uid`; for a change, the whole properties
/// it names, never a part of one. Nothing for a deletion.
///
/// A new card names every property it says, an empty one as empty (`emails: {}`): sent again after its answer was
/// lost, it is the card made the first time, and what was taken out here since (a number, the company) goes there
/// too. Only a name with nothing in it is left out: a card with no name is named by its server, from its company or
/// its address.
pub fn card_json(op: &ContactOp) -> Value {
    let fields = &op.fields;
    let create = op.kind == ContactChangeKind::Create;
    let mut card = Map::new();
    if create {
        card.insert("@type".into(), json!("Card"));
        card.insert("version".into(), json!("1.0"));
        card.insert("uid".into(), json!(op.token));
        if valid_id(&op.book) {
            card.insert("addressBookIds".into(), json!({ op.book.clone(): true }));
        }
    }
    if let Some(name) = &fields.name
        && (!create || [&name.first, &name.last, &name.display].iter().any(|part| !part.trim().is_empty()))
    {
        card.insert("name".into(), name_json(name));
    }
    if let Some(emails) = &fields.emails {
        card.insert("emails".into(), emails_json(emails));
    }
    if let Some(phones) = &fields.phones {
        card.insert("phones".into(), phones_json(phones));
    }
    if let Some(company) = &fields.company {
        card.insert("organizations".into(), one_named("o", company, "Organization"));
    }
    if let Some(title) = &fields.title {
        card.insert("titles".into(), one_named("t", title, "Title"));
    }
    Value::Object(card)
}

/// What a change of a contact (`op`) writes to the server's card as it is now (`card`, as read just before): only what
/// was changed here (`op.base` is what the parts it names were before), so that what someone else changed there
/// meanwhile stays. A name, a company and a job title are the ones set here. Of the addresses and the numbers, those
/// taken out here go, those added here are added, a label changed here is that, and the others stay as the server
/// has them (a fax, a pager, their contexts and labels); the first is the one made the first here, when the order was
/// changed here. Without `op.base`, the parts named are written as they are ([`card_json`]).
pub fn merged_json(op: &ContactOp, card: &Value) -> Value {
    let Some(base) = &op.base else { return card_json(op) };
    let fields = &op.fields;
    let mut out = match card_json(op) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if let Some(now) = &fields.emails {
        let server = entries(card, "emails", MAX_ENTRIES).into_iter().map(|entry| (email_of(entry), entry.clone())).collect();
        let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
        out.insert("emails".into(), emails_map(merged(server, base.emails.as_deref().unwrap_or_default(), now, &same)));
    }
    if let Some(now) = &fields.phones {
        let server = entries(card, "phones", MAX_ENTRIES).into_iter().map(|entry| (phone_of(entry), entry.clone())).collect();
        let same = |a: &str, b: &str| a.trim() == b.trim();
        out.insert("phones".into(), phones_map(merged(server, base.phones.as_deref().unwrap_or_default(), now, &same)));
    }
    Value::Object(out)
}

/// A list of the server's (`server`: each entry as the engine takes it, and as the server has it) with what was done
/// to it here (`was` → `now`) made to it: what was taken out here goes, what was added here is added, a label changed
/// here is that (the entry made anew), and the others stay as they are. The first here goes first when the order was
/// changed here: the first of `was` that is still there is another, or none of `was` is left (all of it replaced);
/// taking out the first alone is no new order, and adding one where there was none puts it after what the server has.
fn merged(
    server: Vec<(Labeled, Map<String, Value>)>,
    was: &[Labeled],
    now: &[Labeled],
    same: &dyn Fn(&str, &str) -> bool,
) -> Vec<(Labeled, Option<Map<String, Value>>)> {
    let had = |list: &[Labeled], value: &str| list.iter().find(|entry| same(&entry.value, value)).cloned();
    let mut out: Vec<(Labeled, Option<Map<String, Value>>)> = server
        .into_iter()
        .filter(|(entry, _)| had(was, &entry.value).is_none() || had(now, &entry.value).is_some())
        .map(|(entry, kept)| (entry, Some(kept)))
        .collect();
    for entry in now {
        let before = had(was, &entry.value);
        match out.iter_mut().find(|(kept, _)| same(&kept.value, &entry.value)) {
            // A label changed here.
            Some((kept, json)) => {
                if before.as_ref().is_none_or(|before| before.label != entry.label) && kept.label != entry.label {
                    kept.label.clone_from(&entry.label);
                    *json = None;
                }
            }
            // Added here (one that was there before and isn't any more was taken out elsewhere: it stays out).
            None if before.is_none() => out.push((entry.clone(), None)),
            None => {}
        }
    }
    let first_kept = was.iter().find(|entry| had(now, &entry.value).is_some());
    if let Some(first) = now.first()
        && first_kept.map_or(!was.is_empty(), |before| !same(&before.value, &first.value))
        && let Some(at) = out.iter().position(|(entry, _)| same(&entry.value, &first.value))
    {
        let entry = out.remove(at);
        out.insert(0, entry);
    }
    out
}

#[cfg(test)]
mod tests;
