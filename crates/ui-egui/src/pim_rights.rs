// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What the owner may do with what a server keeps in its address books (JMAP for Contacts), said where it is offered:
//! the window's side of the engine's `remote_contacts`.
//!
//! - **The contact form** offers, for a contact of a server's address book (or a new one that goes to one), the rows
//!   the book keeps ([`contact_keeps`]): a server's card holds a contact's name, company, job title, e-mail addresses
//!   and phone numbers, and the rest (department, postal address, website, birthday, notes) is not offered; a book
//!   whose server keeps only the basics keeps no display name of its own either, and of a number's label no more than
//!   whether it is a mobile one. What is not offered is said once, under the rows ([`kept_note`]). In a tab that keeps
//!   nothing of its own between visits, a contact's categories and whether it is a favourite (kept here only) are
//!   greyed, with the reason ([`local_only`]).
//! - **A new contact** goes where the engine puts it (a server's book the account may change, else the mailbox's own);
//!   the form offers the books it may go to, with the mailbox's own where the session keeps it ([`book_choices`]).
//!
//! The engine refuses the same, in the same words ([`REASONS`]): its reasons are English, and the window says them
//! in the interface's language by the same literal here, which keeps their translations from looking stale.

use egui::Ui;
use mailcraft_engine::Session;
use mailcraft_engine::cmd::remote_contacts;
use mailcraft_model::Id;

/// The labels of a phone number in a book whose server keeps no more of a label than whether the number is a mobile one.
pub const PLAIN_PHONE_LABELS: [&str; 2] = ["mobile", "other"];

/// The engine's reasons that the window says, as the engine has them (they are translated by these literals).
pub const REASONS: [&str; 4] = [
    "Its address book keeps only a contact's name, company, job title, e-mail addresses and phone numbers",
    "Your rights do not let you add contacts",
    "Your rights do not let you change this contact",
    "Your rights do not let you delete this contact",
];

/// Why a contact's categories and whether it is a favourite are not offered in a tab that keeps nothing.
fn not_kept_here() -> &'static str {
    tl!("This page keeps no categories or favorites: they would be gone at the next visit")
}

/// The reason a row is not offered, in the interface's language.
fn keeps_only() -> &'static str {
    tl!("Its address book keeps only a contact's name, company, job title, e-mail addresses and phone numbers")
}

/// What the contact form offers for one contact ([`contact_keeps`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactKeeps {
    /// The server's address book the contact is in, or goes to (its id here); `None`: the mailbox's own contacts.
    pub book: Option<Id>,
    /// Every row: the department, the postal address, the website, the birthday and the notes too.
    pub everything: bool,
    /// A display name of its own.
    pub display: bool,
    /// Every label of a phone number (else: mobile, or not).
    pub labels: bool,
    /// Its categories and whether it is a favourite are kept from one visit to the next.
    pub local: bool,
    /// For a new contact: where it may go, by the book's id here (0: the mailbox's own), with its name.
    pub choices: Vec<(Id, String)>,
}

impl ContactKeeps {
    /// The labels the form offers for a phone number, of `all` it would offer otherwise.
    pub fn phone_labels<'a>(&self, all: &'a [&'static str]) -> &'a [&'static str] {
        if self.labels { all } else { &PLAIN_PHONE_LABELS }
    }
}

/// What the contact form offers for the contact `contact` (`None`: a new one, going to the book `chosen` names: its id
/// here, 0 for the mailbox's own, `None` for where the engine would put it).
pub fn contact_keeps(s: &Session, contact: Option<Id>, chosen: Option<Id>) -> ContactKeeps {
    let book = match contact {
        Some(id) => s.mailbox.contact(id).and_then(|contact| s.mailbox.book_of(contact)),
        None => match chosen {
            Some(0) => None,
            Some(id) => s.mailbox.address_book(id),
            None => remote_contacts::default_book(s),
        },
    };
    let full = book.is_none_or(|book| !book.plain);
    ContactKeeps {
        book: book.map(|book| book.id),
        everything: book.is_none(),
        display: full,
        labels: full,
        local: s.keeps_local,
        choices: if contact.is_none() { book_choices(s) } else { Vec::new() },
    }
}

/// Where a new contact may go: the servers' books whose contacts the account may change, by their ids here, then the
/// mailbox's own (0) where the session keeps what is made here. A book's name is said with its account's address when
/// another book has the same name.
pub fn book_choices(s: &Session) -> Vec<(Id, String)> {
    let mailbox = &s.mailbox;
    let books: Vec<&mailcraft_model::AddressBook> = mailbox.address_books.iter().filter(|book| remote_contacts::writable(mailbox, book)).collect();
    let mut choices: Vec<(Id, String)> = books
        .iter()
        .map(|book| {
            let twice = books.iter().filter(|other| other.name == book.name).count() > 1;
            let email = mailbox.account(book.origin.account).map(|account| account.email.trim()).unwrap_or_default();
            (book.id, if twice && !email.is_empty() { format!("{} ({email})", book.name) } else { book.name.clone() })
        })
        .collect();
    if s.keeps_local {
        choices.push((0, tl!("On this computer").to_string()));
    }
    choices
}

/// The menu of where a new contact goes ([`book_choices`]): `chosen` is what the owner picked (`None`: where it goes by
/// default, `shown`).
pub fn book_combo(ui: &mut Ui, keeps: &ContactKeeps, chosen: &mut Option<Id>) {
    let shown = chosen.or(Some(keeps.book.unwrap_or(0)));
    let name = |id: Option<Id>| keeps.choices.iter().find(|(book, _)| Some(*book) == id).map(|(_, name)| name.clone()).unwrap_or_default();
    egui::ComboBox::from_id_salt("contact_book").width(220.0).selected_text(name(shown)).truncate().show_ui(ui, |ui| {
        for (book, label) in &keeps.choices {
            if ui.selectable_label(shown == Some(*book), label.as_str()).clicked() {
                *chosen = Some(*book);
            }
        }
    });
}

/// Whether the form offers the menu of where a new contact goes: a server's book is among the choices.
pub fn offers_books(keeps: &ContactKeeps) -> bool {
    keeps.choices.iter().any(|(book, _)| *book != 0)
}

/// `add` (a contact's categories and whether it is a favourite), greyed with the reason where they would be gone at the
/// next visit.
pub fn local_only(ui: &mut Ui, keeps: &ContactKeeps, add: impl FnOnce(&mut Ui)) {
    let shown = ui.add_enabled_ui(keeps.local, add);
    if !keeps.local {
        shown.response.on_disabled_hover_text(not_kept_here());
    }
}

/// Under the rows: why some are not offered, when they aren't.
pub fn kept_note(ui: &mut Ui, keeps: &ContactKeeps) {
    if keeps.everything {
        return;
    }
    let dim = crate::theme::Tokens::get(ui.ctx()).text_dim;
    ui.add(egui::Label::new(egui::RichText::new(keeps_only()).color(dim).size(11.5)).wrap());
}

#[cfg(test)]
mod tests;
