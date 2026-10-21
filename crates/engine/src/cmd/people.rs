// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! People: contacts.

use mailcraft_model::{Contact, Id, Labeled, PostalAddress, clean_name};
use mailcraft_time::Date;
use serde_json::{Value, json};

use super::{bad, done, fail, needs_message};
use crate::{CmdError, CmdResult, CommandSpec, Module, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        // //// Neoffice — refused, with the reason a greyed control says, where a new contact would be kept nowhere (`remote_contacts::adding_refusal`).
        CommandSpec::new("people.new", "New Contact", "Home › New", new)
            .key("Mod+Shift+C")
            .params(CONTACT)
            .when(super::remote_contacts::adding_refusal),
        // //// Neoffice — Edit and Delete say why the address book of the selected contact refuses them (`remote_contacts`).
        CommandSpec::new("people.edit", "Edit Contact", "Contact", edit)
            .params(r#"{"id"?: number, …the fields of people.new}"#)
            .selection(needs_changeable_contact),
        CommandSpec::new("people.delete", "Delete", "Home › Delete", delete).params(ONE).selection(needs_deletable_contact),
        CommandSpec::new("people.select", "Select Contact", "People", select).pure().params(r#"{"id": number | null}"#),
        CommandSpec::new("people.get", "Get Contact", "People", get).pure().params(ONE),
        CommandSpec::new("people.list", "List Contacts", "People", list).pure().params(r#"{"query"?: string}"#),
        CommandSpec::new("people.search", "Search People", "Home › Find", search).pure().params(r#"{"query": string}"#),
        // //// Neoffice — a contact's favourite and its categories are kept on this computer alone: refused, with the reason, where nothing is kept between visits (`kept_here`).
        CommandSpec::new("people.favorite", "Add to Favorites", "Home › Actions", favorite)
            .params(r#"{"value"?: bool (default: toggle), "id"?: number}"#)
            .selection(needs_marked_contact),
        CommandSpec::new("people.categorize", "Categorize", "Home › Tags", categorize)
            .params(r#"{"category": string, "value"?: bool, "id"?: number}"#)
            // //// Neoffice — see `people.favorite`.
            .selection(needs_marked_contact),
        CommandSpec::new("people.email", "Email", "Home › Communicate", email).pure().params(ONE).selection(needs_contact),
        // //// Neoffice — see `people.new`: the sender would be a contact kept nowhere; `book` as there.
        CommandSpec::new("people.fromMessage", "Add to Contacts", "Home › Actions", from_message)
            .params(r#"{"id"?: number (a message; default: the selected one), "book"?: number (as for people.new)}"#)
            .selection(needs_message_to_add),
    ]
}

const ONE: &str = r#"{"id"?: number} (default: the selected contact)"#;
// //// Neoffice — `book`: the address book a new contact goes to.
const CONTACT: &str = r#"{"book"?: number (an address book's id; 0: the mailbox's own contacts; default: a server's book the account may change, else the mailbox's own), "first"?: string, "last"?: string, "displayName"?: string, "company"?: string, "title"?: string, "department"?: string, "nickname"?: string, "email"?: string (primary) | "emails"?: [{"label", "value"}], "phone"?: string | "phones"?: [{"label", "value"}], "address"?: {"label", "street", "city", "region", "postalCode", "country"}, "website"?: string, "birthday"?: "YYYY-MM-DD" | null, "notes"?: string, "categories"?: [string], "favorite"?: bool}"#;

fn needs_contact(s: &Session) -> Option<&'static str> {
    if s.sel.contact.is_none() { Some("select a contact first") } else { None }
}

// //// Neoffice — what the address book of the selected contact says against changing or deleting it, and a session that keeps nothing against adding the sender.
/// [`needs_contact`], and a selected contact whose address book the account may change ([`super::remote_contacts`]).
fn needs_changeable_contact(s: &Session) -> Option<&'static str> {
    needs_contact(s)
        .or_else(|| s.sel.contact.and_then(|id| s.mailbox.contact(id)).and_then(|c| super::remote_contacts::change_refusal(&s.mailbox, c)))
}

/// [`needs_contact`], and a selected contact whose address book lets the account delete it.
fn needs_deletable_contact(s: &Session) -> Option<&'static str> {
    needs_contact(s)
        .or_else(|| s.sel.contact.and_then(|id| s.mailbox.contact(id)).and_then(|c| super::remote_contacts::delete_refusal(&s.mailbox, c)))
}

/// [`needs_contact`], where a contact's favourite and its categories are kept.
fn needs_marked_contact(s: &Session) -> Option<&'static str> {
    super::kept_here::marks_refusal(s).or_else(|| needs_contact(s))
}

/// [`needs_message`], and somewhere to keep the contact.
fn needs_message_to_add(s: &Session) -> Option<&'static str> {
    needs_message(s).or_else(|| super::remote_contacts::adding_refusal(s))
}

fn target(s: &Session, v: &Value) -> Result<Id, CmdError> {
    let id = p::id(v, "id").or(s.sel.contact).ok_or_else(|| fail("select a contact first"))?;
    s.mailbox.contact(id).map(|c| c.id).ok_or_else(|| fail(format!("no contact with id {id}")))
}

/// [`target`] for a command that changes or deletes the contact. A contact that an account's
/// server keeps is refused: changes are not sent back yet, and the next download would undo
/// them. (Its categories and whether it is a favourite are kept here, so those don't ask.)
// //// Neoffice — except a contact of a server's address book the account may change: that change goes to its server (`remote_contacts::written`); one of a book it may only read is refused with that reason.
fn own_target(s: &Session, v: &Value) -> Result<Id, CmdError> {
    let id = target(s, v)?;
    if let Some(c) = s.mailbox.contact(id) {
        // //// Neoffice — an address book the account may only read says so first, in the words of the greyed control; one it may change takes the change, which goes to its server (`remote_contacts`).
        if let Some(why) = super::remote_contacts::change_refusal(&s.mailbox, c) {
            return Err(fail(why));
        }
        if super::remote_contacts::writable_book_of(&s.mailbox, c).is_none() {
            super::remote::changeable(&s.mailbox, &c.origin, "contact")?;
        }
    }
    Ok(id)
}

fn line(v: &Value, k: &str) -> Option<String> {
    p::str(v, k).map(|t| t.replace(['\r', '\n'], " ").trim().chars().take(300).collect())
}

fn labeled(v: &Value, k: &str) -> Option<Vec<Labeled>> {
    let list = v.get(k)?.as_array()?;
    Some(
        list.iter()
            .take(100)
            .filter_map(|x| {
                let value = line(x, "value").or_else(|| x.as_str().map(|s| s.trim().to_string()))?;
                (!value.is_empty()).then(|| Labeled { label: line(x, "label").unwrap_or_default(), value })
            })
            .collect(),
    )
}

fn apply(c: &mut Contact, v: &Value) -> Result<(), CmdError> {
    for (key, field) in [
        ("first", &mut c.first),
        ("middle", &mut c.middle),
        ("last", &mut c.last),
        ("displayName", &mut c.display_name),
        ("company", &mut c.company),
        ("title", &mut c.title),
        ("department", &mut c.department),
        ("nickname", &mut c.nickname),
    ] {
        if let Some(t) = line(v, key) {
            *field = t;
        }
    }
    if let Some(n) = p::str(v, "notes") {
        c.notes = n.to_string();
    }
    if let Some(list) = labeled(v, "emails") {
        c.emails = list;
    }
    // `email` and `phone` set the first entry and leave the others.
    if let Some(e) = line(v, "email") {
        match (c.emails.first_mut(), e.is_empty()) {
            (Some(first), false) => first.value = e,
            (Some(_), true) => {
                c.emails.remove(0);
            }
            (None, false) => c.emails.push(Labeled::new("work", e)),
            (None, true) => {}
        }
    }
    if let Some(list) = labeled(v, "phones") {
        c.phones = list;
    }
    if let Some(ph) = line(v, "phone") {
        match (c.phones.first_mut(), ph.is_empty()) {
            (Some(first), false) => first.value = ph,
            (Some(_), true) => {
                c.phones.remove(0);
            }
            (None, false) => c.phones.push(Labeled::new("mobile", ph)),
            (None, true) => {}
        }
    }
    if let Some(a) = v.get("address").filter(|a| a.is_object()) {
        let addr = PostalAddress {
            label: line(a, "label").unwrap_or_else(|| "work".into()),
            street: p::str(a, "street").unwrap_or("").trim().chars().take(500).collect(),
            city: line(a, "city").unwrap_or_default(),
            region: line(a, "region").unwrap_or_default(),
            postal_code: line(a, "postalCode").unwrap_or_default(),
            country: line(a, "country").unwrap_or_default(),
        };
        c.addresses = if addr.is_empty() {
            c.addresses.iter().skip(1).cloned().collect()
        } else {
            std::iter::once(addr).chain(c.addresses.iter().skip(1).cloned()).collect()
        };
    }
    if let Some(w) = line(v, "website") {
        c.websites = if w.is_empty() {
            c.websites.iter().skip(1).cloned().collect()
        } else {
            std::iter::once(w).chain(c.websites.iter().skip(1).cloned()).collect()
        };
    }
    match v.get("birthday") {
        Some(Value::Null) => c.birthday = None,
        Some(_) => c.birthday = p::date(v, "birthday")?,
        None => {}
    }
    if let Some(cats) = p::strings(v, "categories") {
        c.categories = cats;
    }
    if let Some(f) = p::bool(v, "favorite") {
        c.favorite = f;
    }
    Ok(())
}

fn new(s: &mut Session, v: &Value) -> CmdResult {
    // //// Neoffice — into the address book it goes to (`book`, else a server's book the account may change), never where it would be kept nowhere (the spec's `when` says it first; this holds for any caller).
    let book = super::remote_contacts::target_book(s, v)?;
    let mut c = Contact::default();
    apply(&mut c, v)?;
    if c.name().is_empty() {
        return Err(bad("a contact needs a name, a company or an e-mail address"));
    }
    // //// Neoffice — made in that book, and noted for its server (`remote_contacts::add_contact`).
    let id = super::remote_contacts::add_contact(s, book, c)?;
    s.sel.contact = Some(id);
    Ok(json!({"id": id}))
}

fn edit(s: &mut Session, v: &Value) -> CmdResult {
    let id = own_target(s, v)?;
    let mut c = s.mailbox.contact(id).cloned().ok_or_else(|| fail("the contact is gone"))?;
    apply(&mut c, v)?;
    if c.name().is_empty() {
        return Err(fail("a contact needs a name, a company or an e-mail address"));
    }
    // //// Neoffice — in a server's address book, what its server would not keep is refused and the rest is noted for it (`remote_contacts::put_edited`).
    super::remote_contacts::put_edited(s, c, v)?;
    Ok(json!({"id": id}))
}

/// Contacts as the People list shows them: favourites first, then by name.
pub fn sorted(s: &Session, query: &str) -> Vec<Id> {
    let mut v: Vec<&Contact> = s.mailbox.contacts.values().map(|c| c.as_ref()).filter(|c| c.matches(query)).collect();
    v.sort_by(|a, b| b.favorite.cmp(&a.favorite).then_with(|| a.file_as().to_lowercase().cmp(&b.file_as().to_lowercase())).then(a.id.cmp(&b.id)));
    v.into_iter().map(|c| c.id).collect()
}

fn delete(s: &mut Session, v: &Value) -> CmdResult {
    // //// Neoffice — an address book whose contacts may be changed and not deleted refuses the deletion alone, with its reason.
    if let Some(why) = target(s, v).ok().and_then(|id| s.mailbox.contact(id)).and_then(|c| super::remote_contacts::delete_refusal(&s.mailbox, c)) {
        return Err(fail(why));
    }
    let id = own_target(s, v)?;
    // The selection moves to the neighbour in the list.
    let order = sorted(s, &s.view.people_search);
    let at = order.iter().position(|x| *x == id);
    // //// Neoffice — one of a server's address book is deleted there too (`remote_contacts`).
    let before = s.mailbox.contact(id).cloned();
    s.mailbox.remove_contact(id)?;
    if let Some(before) = before {
        super::remote_contacts::note_deleted(&mut s.mailbox, &before)?;
    }
    if s.sel.contact == Some(id) {
        s.sel.contact = at.and_then(|i| order.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| order.get(j)))).copied();
    }
    Ok(json!({"deleted": id}))
}

fn select(s: &mut Session, v: &Value) -> CmdResult {
    s.sel.contact = match p::id(v, "id") {
        Some(id) => Some(s.mailbox.contact(id).map(|c| c.id).ok_or_else(|| fail(format!("no contact with id {id}")))?),
        None => None,
    };
    done()
}

fn contact_json(c: &Contact) -> Value {
    json!({
        "id": c.id, "name": c.name(), "fileAs": c.file_as(), "first": c.first, "middle": c.middle, "last": c.last, "displayName": c.display_name,
        "nickname": c.nickname, "company": c.company, "title": c.title, "department": c.department,
        "emails": c.emails, "phones": c.phones, "addresses": c.addresses, "websites": c.websites,
        "birthday": c.birthday.map(Date::iso), "notes": c.notes, "categories": c.categories, "favorite": c.favorite,
    })
}

fn get(s: &mut Session, v: &Value) -> CmdResult {
    let id = target(s, v)?;
    s.mailbox.contact(id).map(contact_json).ok_or_else(|| fail("the contact is gone"))
}

fn list(s: &mut Session, v: &Value) -> CmdResult {
    let query = p::str(v, "query").unwrap_or(&s.view.people_search).to_string();
    let items: Vec<Value> = sorted(s, &query)
        .into_iter()
        .filter_map(|id| s.mailbox.contact(id))
        .map(|c| json!({"id": c.id, "name": c.name(), "email": c.primary_email(), "company": c.company, "favorite": c.favorite}))
        .collect();
    Ok(json!({"contacts": items}))
}

fn search(s: &mut Session, v: &Value) -> CmdResult {
    s.view.people_search = p::req_str(v, "query")?.chars().filter(|c| !c.is_control()).take(200).collect();
    Ok(json!({"query": s.view.people_search, "found": sorted(s, &s.view.people_search).len()}))
}

fn favorite(s: &mut Session, v: &Value) -> CmdResult {
    // //// Neoffice — also for a caller that names the contact (`needs_marked_contact`).
    if let Some(why) = super::kept_here::marks_refusal(s) {
        return Err(fail(why));
    }
    let id = target(s, v)?;
    let c = s.mailbox.contact_mut(id)?;
    c.favorite = p::bool(v, "value").unwrap_or(!c.favorite);
    Ok(json!({"id": id, "favorite": c.favorite}))
}

fn categorize(s: &mut Session, v: &Value) -> CmdResult {
    // //// Neoffice — see `favorite`.
    if let Some(why) = super::kept_here::marks_refusal(s) {
        return Err(fail(why));
    }
    let id = target(s, v)?;
    let name = clean_name(p::req_str(v, "category")?).ok_or_else(|| bad("`category` is empty"))?;
    let c = s.mailbox.contact_mut(id)?;
    let has = c.categories.iter().any(|x| x.eq_ignore_ascii_case(&name));
    let on = p::bool(v, "value").unwrap_or(!has);
    c.categories.retain(|x| !x.eq_ignore_ascii_case(&name));
    if on {
        c.categories.push(name);
    }
    Ok(json!({"id": id, "categories": c.categories}))
}

fn email(s: &mut Session, v: &Value) -> CmdResult {
    let id = target(s, v)?;
    let to = s.mailbox.contact(id).and_then(Contact::address).ok_or_else(|| fail("this contact has no e-mail address"))?;
    s.run("mail.new", &json!({"to": to.full()}))
}

fn from_message(s: &mut Session, v: &Value) -> CmdResult {
    let mid = p::id(v, "id").or(s.sel.focus()).ok_or_else(|| fail("select a message first"))?;
    let from = s.mailbox.message(mid).map(|m| m.from.clone()).ok_or_else(|| fail(format!("no message with id {mid}")))?;
    if !from.is_valid() {
        return Err(fail("the sender has no usable e-mail address"));
    }
    if let Some(existing) = s.mailbox.contact_by_email(&from.email).map(|c| c.id) {
        s.sel.contact = Some(existing);
        s.view.module = Module::People;
        return Ok(json!({"id": existing, "existing": true}));
    }
    // //// Neoffice — see `new`: into the address book it goes to, and never where it would be kept nowhere.
    let book = super::remote_contacts::target_book(s, v)?;
    // "First Last" splits at the last space; a single word is a first name.
    let (first, last) = match from.name.trim().rsplit_once(' ') {
        Some((f, l)) => (f.trim().to_string(), l.trim().to_string()),
        None => (from.name.trim().to_string(), String::new()),
    };
    // //// Neoffice — see `new`.
    let id =
        super::remote_contacts::add_contact(s, book, Contact { first, last, emails: vec![Labeled::new("work", from.email)], ..Default::default() })?;
    s.sel.contact = Some(id);
    s.view.module = Module::People;
    Ok(json!({"id": id, "existing": false}))
}
